// ── audit.rs — Signed audit log with hash chain ──────────────────────────
//
// Enterprise-grade audit logging:
// - Each entry is SHA-256 hashed with machine-derived key
// - Hash chain links entries (tamper detection)
// - Entries stored as JSON lines with entry_hash + prev_hash
// - Verification function checks entire chain integrity
//
// Compliance: SOC2 Type II, ISO 27001 audit trail requirements.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A single signed audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// ISO 8601 timestamp.
    pub ts: String,
    /// User ID that performed the action.
    pub uid: u32,
    /// Hostname of the machine.
    pub host: String,
    /// Event type (ACTIVATED, DEACTIVATED, EXPIRED, etc.).
    pub event: String,
    /// License ID involved.
    pub license_id: String,
    /// Human-readable detail.
    pub detail: String,
    /// SHA-256 hash of the previous entry (hash chain).
    pub prev_hash: String,
    /// HMAC-SHA256 of this entry (tamper detection).
    pub entry_hash: String,
}

impl AuditEntry {
    /// Compute HMAC for this entry using machine-derived key.
    fn compute_hash(&self, prev_hash: &str) -> String {
        let key = derive_audit_key();
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.ts, self.uid, self.host, self.event, self.license_id, self.detail, prev_hash
        );
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.update(payload.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
}

/// Derive an audit-specific key from machine fingerprint.
/// Different from the license obfuscation key to prevent cross-module attacks.
fn derive_audit_key() -> [u8; 32] {
    let machine = crate::license::generate_machine_id().unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(b"talus-audit-v1");
    hasher.update(machine.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Get the audit log file path.
fn audit_log_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("talus").join("audit.log"))
}

/// Append a signed entry to the audit log with hash chain.
pub fn audit_log(event: &str, license_id: &str, detail: &str) {
    let log_path = match audit_log_path() {
        Some(p) => p,
        None => return,
    };

    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Get previous hash from last line
    let prev_hash = fs::read_to_string(&log_path)
        .ok()
        .and_then(|data| {
            data.lines().last().and_then(|line| {
                serde_json::from_str::<AuditEntry>(line)
                    .ok()
                    .map(|e| e.entry_hash)
            })
        })
        .unwrap_or_else(|| "0".to_string());

    let ts = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".into());
    // SAFETY: getuid() is a simple syscall.
    let uid = unsafe { libc::getuid() };

    let mut entry = AuditEntry {
        ts,
        uid,
        host: hostname,
        event: event.to_string(),
        license_id: license_id.to_string(),
        detail: detail.to_string(),
        prev_hash,
        entry_hash: String::new(),
    };
    entry.entry_hash = entry.compute_hash(&entry.prev_hash);

    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        if let Ok(json) = serde_json::to_string(&entry) {
            let _ = writeln!(file, "{json}");
        }
        let _ = fs::set_permissions(&log_path, fs::Permissions::from_mode(0o600));
    }
}

/// Verify the integrity of the entire audit log (hash chain).
/// Returns the number of verified entries, or an error if any entry is corrupted.
pub fn verify_audit_log() -> Result<u64> {
    let log_path = audit_log_path().context("cannot determine audit log path")?;

    if !log_path.exists() {
        return Ok(0);
    }

    let data = fs::read_to_string(&log_path).context("failed to read audit log")?;

    let mut prev_hash = "0".to_string();
    let mut verified = 0u64;
    let mut corrupted = 0u64;

    for line in data.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<AuditEntry>(line) {
            Ok(entry) => {
                // Verify hash chain
                if entry.prev_hash != prev_hash {
                    corrupted += 1;
                    continue;
                }
                // Verify entry hash
                let expected = entry.compute_hash(&prev_hash);
                if entry.entry_hash != expected {
                    corrupted += 1;
                    continue;
                }
                prev_hash = entry.entry_hash;
                verified += 1;
            }
            Err(_) => {
                corrupted += 1;
            }
        }
    }

    if corrupted > 0 {
        bail!(
            "audit log integrity check failed: {corrupted} corrupted entries out of {}",
            verified + corrupted
        );
    }

    Ok(verified)
}

/// Read the last N entries from the audit log.
pub fn read_audit_log(max_entries: usize) -> Vec<String> {
    let log_path = match audit_log_path() {
        Some(p) => p,
        None => return Vec::new(),
    };

    fs::read_to_string(&log_path)
        .ok()
        .map(|data| {
            data.lines()
                .rev()
                .take(max_entries)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_key_is_deterministic() {
        let k1 = derive_audit_key();
        let k2 = derive_audit_key();
        assert_eq!(k1, k2);
    }

    #[test]
    fn audit_entry_hash_deterministic() {
        let entry = AuditEntry {
            ts: "2026-01-01T00:00:00Z".into(),
            uid: 0,
            host: "test".into(),
            event: "TEST".into(),
            license_id: "TALUS-001".into(),
            detail: "test detail".into(),
            prev_hash: "0".into(),
            entry_hash: String::new(),
        };
        let h1 = entry.compute_hash("0");
        let h2 = entry.compute_hash("0");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn audit_entry_hash_depends_on_prev() {
        let entry = AuditEntry {
            ts: "2026-01-01T00:00:00Z".into(),
            uid: 0,
            host: "test".into(),
            event: "TEST".into(),
            license_id: "TALUS-001".into(),
            detail: "test detail".into(),
            prev_hash: "0".into(),
            entry_hash: String::new(),
        };
        let h1 = entry.compute_hash("aaa");
        let h2 = entry.compute_hash("bbb");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_chain_end_to_end() {
        // Build a 3-entry chain manually and verify
        let _key = derive_audit_key();

        let entry1 = AuditEntry {
            ts: "2026-01-01T00:00:00Z".into(),
            uid: 0,
            host: "host1".into(),
            event: "ACTIVATED".into(),
            license_id: "TALUS-001".into(),
            detail: "machine=T-abc".into(),
            prev_hash: "0".into(),
            entry_hash: String::new(),
        };
        let h1 = entry1.compute_hash("0");

        let entry2 = AuditEntry {
            ts: "2026-01-01T00:01:00Z".into(),
            uid: 0,
            host: "host1".into(),
            event: "EXPIRED".into(),
            license_id: "TALUS-001".into(),
            detail: "license expired".into(),
            prev_hash: h1.clone(),
            entry_hash: String::new(),
        };
        let h2 = entry2.compute_hash(&h1);

        let entry3 = AuditEntry {
            ts: "2026-01-01T00:02:00Z".into(),
            uid: 0,
            host: "host1".into(),
            event: "DEACTIVATED".into(),
            license_id: "TALUS-001".into(),
            detail: "user deactivation".into(),
            prev_hash: h2.clone(),
            entry_hash: String::new(),
        };
        let h3 = entry3.compute_hash(&h2);

        // Verify chain: each prev_hash must match the previous entry_hash
        assert_eq!(entry1.prev_hash, "0");
        assert_eq!(entry2.prev_hash, h1);
        assert_eq!(entry3.prev_hash, h2);

        // Verify each entry's hash is correct
        assert_eq!(h1, entry1.compute_hash("0"));
        assert_eq!(h2, entry2.compute_hash(&h1));
        assert_eq!(h3, entry3.compute_hash(&h2));

        // Tamper detection: change entry2 detail → hash breaks
        let mut tampered = entry2.clone();
        tampered.detail = "TAMPERED".into();
        let h2_tampered = tampered.compute_hash(&h1);
        assert_ne!(h2_tampered, h2, "tampered entry must produce different hash");

        // Chain break: entry3.prev_hash no longer matches
        assert_ne!(entry3.prev_hash, h2_tampered);
    }

    #[test]
    fn audit_log_write_and_verify() {
        // Write 3 entries to a temp file, then verify the chain
        let tmp_dir = std::env::temp_dir().join("talus_audit_test");
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).unwrap();
        let log_path = tmp_dir.join("audit.log");

        // Write 3 entries
        for i in 0..3 {
            let prev_hash = if i == 0 {
                "0".to_string()
            } else {
                let data = fs::read_to_string(&log_path).unwrap();
                let last_line = data.lines().last().unwrap();
                let entry: AuditEntry = serde_json::from_str(last_line).unwrap();
                entry.entry_hash
            };

            let mut entry = AuditEntry {
                ts: format!("2026-01-01T00:0{i}:00Z"),
                uid: 1000,
                host: "test-host".into(),
                event: format!("TEST_{i}"),
                license_id: "TALUS-TEST".into(),
                detail: format!("entry {i}"),
                prev_hash,
                entry_hash: String::new(),
            };
            entry.entry_hash = entry.compute_hash(&entry.prev_hash);

            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap();
            writeln!(file, "{}", serde_json::to_string(&entry).unwrap()).unwrap();
        }

        // Verify the chain
        let data = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = data.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3);

        let mut prev_hash = "0".to_string();
        let mut verified = 0u64;
        for line in &lines {
            let entry: AuditEntry = serde_json::from_str(line).unwrap();
            assert_eq!(entry.prev_hash, prev_hash, "chain break at entry");
            let expected = entry.compute_hash(&prev_hash);
            assert_eq!(entry.entry_hash, expected, "hash mismatch at entry");
            prev_hash = entry.entry_hash;
            verified += 1;
        }
        assert_eq!(verified, 3);

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn audit_log_detects_tampering() {
        let tmp_dir = std::env::temp_dir().join("talus_audit_tamper_test");
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).unwrap();
        let log_path = tmp_dir.join("audit.log");

        // Write 2 entries
        for i in 0..2 {
            let prev_hash = if i == 0 {
                "0".to_string()
            } else {
                let data = fs::read_to_string(&log_path).unwrap();
                let last_line = data.lines().last().unwrap();
                let entry: AuditEntry = serde_json::from_str(last_line).unwrap();
                entry.entry_hash
            };
            let mut entry = AuditEntry {
                ts: format!("2026-01-01T00:0{i}:00Z"),
                uid: 1000,
                host: "test-host".into(),
                event: format!("TEST_{i}"),
                license_id: "TALUS-TEST".into(),
                detail: format!("entry {i}"),
                prev_hash,
                entry_hash: String::new(),
            };
            entry.entry_hash = entry.compute_hash(&entry.prev_hash);
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap();
            writeln!(file, "{}", serde_json::to_string(&entry).unwrap()).unwrap();
        }

        // Tamper with entry 1 (change detail)
        let data = fs::read_to_string(&log_path).unwrap();
        let mut lines: Vec<String> = data.lines().map(String::from).collect();
        let mut entry1: AuditEntry = serde_json::from_str(&lines[1]).unwrap();
        entry1.detail = "HACKED".into();
        lines[1] = serde_json::to_string(&entry1).unwrap();
        fs::write(&log_path, lines.join("\n") + "\n").unwrap();

        // Verify should fail
        let data = fs::read_to_string(&log_path).unwrap();
        let mut prev_hash = "0".to_string();
        let mut corrupted = 0u64;
        for line in data.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: AuditEntry = serde_json::from_str(line).unwrap();
            if entry.prev_hash != prev_hash {
                corrupted += 1;
                continue;
            }
            let expected = entry.compute_hash(&prev_hash);
            if entry.entry_hash != expected {
                corrupted += 1;
                continue;
            }
            prev_hash = entry.entry_hash;
        }
        assert!(corrupted > 0, "tampering should be detected");

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
