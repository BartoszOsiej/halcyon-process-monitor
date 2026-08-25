//! Secret detection and artifact redaction pipeline.
//!
//! Detects and redacts sensitive tokens/credentials in source-derived artifacts,
//! logs, and manifests while preserving debugging utility.
//!
//! # Design Principles
//!
//! 1. **Deterministic**: same input always produces same redacted output.
//! 2. **Reversible**: redacted values can be recovered via a secure local mapping.
//! 3. **Traceable**: every redaction is logged with location and pattern metadata.
//! 4. **Layered**: scanners run on both raw inputs and generated outputs.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Secret Pattern Definitions ───────────────────────────────────────────

/// A pattern that matches a class of secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretPattern {
    /// Unique identifier for this pattern.
    pub pattern_id: String,
    /// Human-readable description.
    pub description: String,
    /// Regex pattern string (applied per line).
    pub regex: String,
    /// Severity: critical, high, medium, low.
    pub severity: String,
    /// Category: api_key, token, password, private_key, connection_string, etc.
    pub category: String,
    /// Whether this pattern produces high false-positive rates.
    pub high_entropy_only: bool,
}

/// Built-in secret patterns covering common credential formats.
pub fn builtin_patterns() -> Vec<SecretPattern> {
    vec![
        SecretPattern {
            pattern_id: "aws-access-key".into(),
            description: "AWS Access Key ID".into(),
            regex: r"(?i)(?:AKIA|ASIA)[0-9A-Z]{16}".into(),
            severity: "critical".into(),
            category: "api_key".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "aws-secret-key".into(),
            description: "AWS Secret Access Key".into(),
            regex: r"(?i)aws[_\-]?secret[_\-]?access[_\-]?key\s*[:=]\s*[A-Za-z0-9/+=]{40}".into(),
            severity: "critical".into(),
            category: "api_key".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "github-token".into(),
            description: "GitHub Personal Access Token or Fine-Grained Token".into(),
            regex: r"(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{36,255}".into(),
            severity: "critical".into(),
            category: "token".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "generic-api-key".into(),
            description: "Generic API key assignment".into(),
            regex: r#"(?i)(?:api[_\-]?key|apikey)\s*[:=]\s*["']?[A-Za-z0-9\-_.]{20,}"#.into(),
            severity: "high".into(),
            category: "api_key".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "generic-secret".into(),
            description: "Generic secret assignment".into(),
            regex: r#"(?i)(?:secret|password|passwd|pwd)\s*[:=]\s*["']?[^\s"']{8,}"#.into(),
            severity: "high".into(),
            category: "password".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "private-key-header".into(),
            description: "PEM private key header".into(),
            regex: r"-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----".into(),
            severity: "critical".into(),
            category: "private_key".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "jwt-token".into(),
            description: "JSON Web Token".into(),
            regex: r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}".into(),
            severity: "high".into(),
            category: "token".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "connection-string".into(),
            description: "Database connection string with credentials".into(),
            regex: r"(?i)(?:mysql|postgres|mongodb|redis|amqp)://[^\s@]+:[^\s@]+@[^\s]+".into(),
            severity: "critical".into(),
            category: "connection_string".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "npm-token".into(),
            description: "NPM auth token".into(),
            regex: r"(?i)//registry\.npmjs\.org/:_authToken\s*=\s*[A-Za-z0-9\-._]{20,}".into(),
            severity: "critical".into(),
            category: "token".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "slack-token".into(),
            description: "Slack Bot or User OAuth Token".into(),
            regex: r"xox[bpors]-[0-9A-Za-z\-]{10,}".into(),
            severity: "critical".into(),
            category: "token".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "stripe-key".into(),
            description: "Stripe API key".into(),
            regex: r"(?:sk|pk|rk)_(?:test|live)_[0-9a-zA-Z]{20,}".into(),
            severity: "critical".into(),
            category: "api_key".into(),
            high_entropy_only: false,
        },
        SecretPattern {
            pattern_id: "env-file-secret".into(),
            description: "Secret-like assignment in .env file".into(),
            regex: r#"(?i)^[A-Z_]*(?:SECRET|TOKEN|KEY|PASSWORD|CREDENTIAL)[A-Z_]*\s*=\s*.{8,}"#
                .into(),
            severity: "medium".into(),
            category: "password".into(),
            high_entropy_only: false,
        },
    ]
}

// ── Scanner ──────────────────────────────────────────────────────────────

/// A finding from the secret scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    /// Which pattern matched.
    pub pattern_id: String,
    /// File where the secret was found.
    pub file_path: String,
    /// Line number (1-indexed).
    pub line_number: usize,
    /// Column range (0-indexed start, exclusive end).
    pub col_start: usize,
    pub col_end: usize,
    /// Severity of the finding.
    pub severity: String,
    /// Category of the finding.
    pub category: String,
    /// SHA-256 of the matched secret value (for deterministic tracking without exposing it).
    pub value_hash: String,
    /// Length of the matched secret value.
    pub value_length: usize,
}

/// Scan a single string content for secrets.
pub fn scan_content(
    content: &str,
    file_path: &str,
    patterns: &[SecretPattern],
) -> Vec<SecretFinding> {
    let mut findings = Vec::new();

    for pattern in patterns {
        let Ok(re) = regex_lite::Regex::new(&pattern.regex) else {
            continue;
        };

        for (line_idx, line) in content.lines().enumerate() {
            for mat in re.find_iter(line) {
                let matched = mat.as_str();
                let hash = sha256_hex(matched.as_bytes());
                findings.push(SecretFinding {
                    pattern_id: pattern.pattern_id.clone(),
                    file_path: file_path.to_string(),
                    line_number: line_idx + 1,
                    col_start: mat.start(),
                    col_end: mat.end(),
                    severity: pattern.severity.clone(),
                    category: pattern.category.clone(),
                    value_hash: hash,
                    value_length: matched.len(),
                });
            }
        }
    }

    findings
}

/// Scan a file on disk for secrets.
pub fn scan_file(path: &Path, patterns: &[SecretPattern]) -> std::io::Result<Vec<SecretFinding>> {
    let content = std::fs::read_to_string(path)?;
    let path_str = path.to_string_lossy();
    Ok(scan_content(&content, &path_str, patterns))
}

// ── Redaction ────────────────────────────────────────────────────────────

/// A single redaction applied to content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionRecord {
    /// Unique redaction placeholder token (e.g., `[REDACTED:a1b2c3d4]`).
    pub placeholder: String,
    /// SHA-256 of the original secret value.
    pub value_hash: String,
    /// Length of the original secret value.
    pub value_length: usize,
    /// Pattern that triggered the redaction.
    pub pattern_id: String,
    /// File where the redaction was applied.
    pub file_path: String,
    /// Line number (1-indexed).
    pub line_number: usize,
}

/// Mapping from placeholder tokens to original secret values.
/// This is the only structure that stores raw secrets and must be
/// kept in a secure local location, never committed or transmitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionMap {
    /// Run ID for traceability.
    pub run_id: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Mapping: placeholder → original value.
    pub entries: BTreeMap<String, String>,
}

impl RedactionMap {
    fn new(run_id: &str) -> Self {
        Self {
            run_id: run_id.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            entries: BTreeMap::new(),
        }
    }

    /// Number of redacted values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Result of redacting content.
#[derive(Debug, Clone)]
pub struct RedactionResult {
    /// The redacted content (secrets replaced with placeholders).
    pub content: String,
    /// Records of all redactions applied.
    pub records: Vec<RedactionRecord>,
    /// The reverse mapping (placeholder → original value).
    pub map: RedactionMap,
}

/// Redact all secrets from content, producing deterministic placeholder tokens.
///
/// The placeholder format is `[REDACTED:<8-char-hex>]` where the hex is derived
/// from the SHA-256 of the original value + file path + line context. This makes
/// redaction deterministic: the same secret at the same location always produces
/// the same placeholder.
pub fn redact_content(
    content: &str,
    file_path: &str,
    patterns: &[SecretPattern],
    run_id: &str,
) -> RedactionResult {
    let mut result_content = content.to_string();
    let mut records = Vec::new();
    let mut map = RedactionMap::new(run_id);

    // Collect all findings first, sorted by position (reverse order for replacement).
    let mut findings = scan_content(content, file_path, patterns);

    // Sort by line_number desc, then col_start desc — so we can replace from end to start
    // without invalidating earlier positions.
    findings.sort_by(|a, b| {
        b.line_number
            .cmp(&a.line_number)
            .then(b.col_start.cmp(&a.col_start))
    });

    // Deduplicate overlapping findings (keep the one with higher severity).
    let mut applied_ranges: Vec<(usize, usize, usize)> = Vec::new(); // (line, start, end)

    for finding in &findings {
        // Check for overlap with already-applied redactions.
        let overlaps = applied_ranges.iter().any(|(line, start, end)| {
            *line == finding.line_number && finding.col_start < *end && finding.col_end > *start
        });
        if overlaps {
            continue;
        }

        // Generate deterministic placeholder.
        let placeholder_hash = sha256_hex(
            format!(
                "{}:{}:{}:{}",
                file_path, finding.line_number, finding.col_start, finding.value_hash
            )
            .as_bytes(),
        );
        let placeholder = format!("[REDACTED:{}]", &placeholder_hash[..8]);

        // Extract the original value from the content.
        let lines: Vec<&str> = result_content.lines().collect();
        if finding.line_number <= lines.len() {
            let line = lines[finding.line_number - 1];
            if finding.col_end <= line.len() {
                let original_value = &line[finding.col_start..finding.col_end];

                // Store in the reverse map.
                map.entries
                    .insert(placeholder.clone(), original_value.to_string());

                // Apply the replacement.
                let mut new_lines: Vec<String> = result_content.lines().map(String::from).collect();
                let target_line = &mut new_lines[finding.line_number - 1];
                *target_line = format!(
                    "{}{}{}",
                    &target_line[..finding.col_start],
                    placeholder,
                    &target_line[finding.col_end..]
                );
                result_content = new_lines.join("\n");
                // Preserve trailing newline if original had one.
                if content.ends_with('\n') && !result_content.ends_with('\n') {
                    result_content.push('\n');
                }

                records.push(RedactionRecord {
                    placeholder: placeholder.clone(),
                    value_hash: finding.value_hash.clone(),
                    value_length: finding.value_length,
                    pattern_id: finding.pattern_id.clone(),
                    file_path: file_path.to_string(),
                    line_number: finding.line_number,
                });

                applied_ranges.push((finding.line_number, finding.col_start, finding.col_end));
            }
        }
    }

    // Sort records by line number ascending for readability.
    records.sort_by_key(|r| r.line_number);

    RedactionResult {
        content: result_content,
        records,
        map,
    }
}

/// Restore redacted content using the reverse mapping.
pub fn unredact_content(redacted: &str, map: &RedactionMap) -> String {
    let mut result = redacted.to_string();
    for (placeholder, original) in &map.entries {
        result = result.replace(placeholder, original);
    }
    result
}

// ── Scan Report ──────────────────────────────────────────────────────────

/// Summary of a scan/redaction run across multiple files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    /// Run identifier.
    pub run_id: String,
    /// Total files scanned.
    pub files_scanned: usize,
    /// Total findings across all files.
    pub total_findings: usize,
    /// Findings by severity.
    pub by_severity: BTreeMap<String, usize>,
    /// Findings by category.
    pub by_category: BTreeMap<String, usize>,
    /// Findings by pattern.
    pub by_pattern: BTreeMap<String, usize>,
    /// Files with findings.
    pub affected_files: Vec<String>,
    /// Whether any critical findings were detected.
    pub has_critical: bool,
}

impl ScanReport {
    /// Build a report from a set of findings.
    pub fn from_findings(run_id: &str, findings: &[SecretFinding]) -> Self {
        let mut by_severity: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_pattern: BTreeMap<String, usize> = BTreeMap::new();
        let mut affected: BTreeMap<String, ()> = BTreeMap::new();
        let mut has_critical = false;

        for f in findings {
            *by_severity.entry(f.severity.clone()).or_default() += 1;
            *by_category.entry(f.category.clone()).or_default() += 1;
            *by_pattern.entry(f.pattern_id.clone()).or_default() += 1;
            affected.insert(f.file_path.clone(), ());
            if f.severity == "critical" {
                has_critical = true;
            }
        }

        Self {
            run_id: run_id.to_string(),
            files_scanned: 0, // Caller must set this.
            total_findings: findings.len(),
            by_severity,
            by_category,
            by_pattern,
            affected_files: affected.into_keys().collect(),
            has_critical,
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex_encode(&result)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Pattern coverage ─────────────────────────────────────────────────

    #[test]
    fn builtin_patterns_are_nonempty() {
        let patterns = builtin_patterns();
        assert!(!patterns.is_empty());
        assert!(patterns.len() >= 10);
    }

    #[test]
    fn all_builtin_patterns_have_valid_regex() {
        for p in builtin_patterns() {
            assert!(
                regex_lite::Regex::new(&p.regex).is_ok(),
                "pattern '{}' has invalid regex: {}",
                p.pattern_id,
                p.regex
            );
        }
    }

    #[test]
    fn all_builtin_patterns_have_unique_ids() {
        let patterns = builtin_patterns();
        let mut seen = std::collections::HashSet::new();
        for p in &patterns {
            assert!(
                seen.insert(&p.pattern_id),
                "duplicate pattern_id: {}",
                p.pattern_id
            );
        }
    }

    // ── Secret detection ─────────────────────────────────────────────────

    #[test]
    fn detects_aws_access_key() {
        let content = "aws_key = AKIAIOSFODNN7EXAMPLE";
        let findings = scan_content(content, "config.txt", &builtin_patterns());
        assert!(
            findings.iter().any(|f| f.pattern_id == "aws-access-key"),
            "should detect AWS access key"
        );
    }

    #[test]
    fn detects_github_token() {
        let content = "GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
        let findings = scan_content(content, ".env", &builtin_patterns());
        assert!(
            findings.iter().any(|f| f.pattern_id == "github-token"),
            "should detect GitHub token"
        );
    }

    #[test]
    fn detects_private_key_header() {
        let content = "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----";
        let findings = scan_content(content, "key.pem", &builtin_patterns());
        assert!(
            findings
                .iter()
                .any(|f| f.pattern_id == "private-key-header"),
            "should detect private key header"
        );
    }

    #[test]
    fn detects_connection_string() {
        let content = "DATABASE_URL=postgres://admin:supersecret@db.example.com:5432/mydb";
        let findings = scan_content(content, ".env", &builtin_patterns());
        assert!(
            findings.iter().any(|f| f.pattern_id == "connection-string"),
            "should detect connection string"
        );
    }

    #[test]
    fn detects_jwt_token() {
        let content = "token = eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let findings = scan_content(content, "auth.ts", &builtin_patterns());
        assert!(
            findings.iter().any(|f| f.pattern_id == "jwt-token"),
            "should detect JWT token"
        );
    }

    #[test]
    fn detects_stripe_key() {
        let content = "const key = 'sk_test_FAKE_KEY_FOR_TESTING_ONLY';";
        let findings = scan_content(content, "payment.js", &builtin_patterns());
        assert!(
            findings.iter().any(|f| f.pattern_id == "stripe-key"),
            "should detect Stripe key"
        );
    }

    #[test]
    fn detects_npm_token() {
        let content = "//registry.npmjs.org/:_authToken=npm_ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let findings = scan_content(content, ".npmrc", &builtin_patterns());
        assert!(
            findings.iter().any(|f| f.pattern_id == "npm-token"),
            "should detect NPM token"
        );
    }

    #[test]
    fn no_false_positive_on_clean_code() {
        let content = r#"
fn main() {
    let x = 42;
    println!("Hello, world!");
}
"#;
        let findings = scan_content(content, "main.rs", &builtin_patterns());
        assert!(
            findings.is_empty(),
            "clean code should produce no findings, got: {findings:?}"
        );
    }

    // ── Findings metadata ────────────────────────────────────────────────

    #[test]
    fn finding_has_correct_line_and_column() {
        let content = "line1\napi_key = AKIAIOSFODNN7EXAMPLE\nline3";
        let findings = scan_content(content, "test.txt", &builtin_patterns());
        let aws = findings
            .iter()
            .find(|f| f.pattern_id == "aws-access-key")
            .expect("should find AWS key");
        assert_eq!(aws.line_number, 2);
        assert!(aws.col_start > 0);
        assert!(aws.col_end > aws.col_start);
    }

    #[test]
    fn finding_value_hash_is_deterministic() {
        let content = "key = AKIAIOSFODNN7EXAMPLE";
        let f1 = scan_content(content, "a.txt", &builtin_patterns());
        let f2 = scan_content(content, "a.txt", &builtin_patterns());
        assert_eq!(f1.len(), f2.len());
        for (a, b) in f1.iter().zip(f2.iter()) {
            assert_eq!(a.value_hash, b.value_hash);
        }
    }

    // ── Redaction ────────────────────────────────────────────────────────

    #[test]
    fn redaction_replaces_secrets_with_placeholders() {
        let content = "token = ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
        let result = redact_content(content, "test.env", &builtin_patterns(), "run-1");
        assert!(
            result.content.contains("[REDACTED:"),
            "redacted content should contain placeholder"
        );
        assert!(
            !result
                .content
                .contains("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789"),
            "original secret must not appear in redacted output"
        );
    }

    #[test]
    fn redaction_is_deterministic() {
        let content = "secret = ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
        let r1 = redact_content(content, "test.env", &builtin_patterns(), "run-1");
        let r2 = redact_content(content, "test.env", &builtin_patterns(), "run-1");
        assert_eq!(r1.content, r2.content);
        assert_eq!(r1.records.len(), r2.records.len());
    }

    #[test]
    fn redaction_records_are_produced() {
        let content = "GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
        let result = redact_content(content, ".env", &builtin_patterns(), "run-1");
        assert!(
            !result.records.is_empty(),
            "should produce redaction records"
        );
        for record in &result.records {
            assert!(record.placeholder.starts_with("[REDACTED:"));
            assert!(record.placeholder.ends_with(']'));
            assert!(!record.value_hash.is_empty());
            assert!(record.value_length > 0);
        }
    }

    #[test]
    fn redaction_map_stores_original_values() {
        let content = "tok = ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
        let result = redact_content(content, ".env", &builtin_patterns(), "run-1");
        assert!(!result.map.is_empty(), "redaction map should not be empty");
        for (placeholder, original) in &result.map.entries {
            assert!(placeholder.starts_with("[REDACTED:"), "placeholder format");
            assert!(!original.is_empty(), "original value should not be empty");
        }
    }

    // ── Round-trip: redact then unredact ──────────────────────────────────

    #[test]
    fn unredact_restores_original_content() {
        let content = "db = postgres://admin:supersecret@db.example.com:5432/mydb";
        let result = redact_content(content, "config.env", &builtin_patterns(), "run-1");
        assert_ne!(result.content, content, "redaction should change content");
        let restored = unredact_content(&result.content, &result.map);
        assert_eq!(restored, content, "unredact must restore original");
    }

    #[test]
    fn unredact_is_noop_on_clean_content() {
        let content = "no secrets here";
        let result = redact_content(content, "clean.txt", &builtin_patterns(), "run-1");
        assert_eq!(result.content, content);
        let restored = unredact_content(&result.content, &result.map);
        assert_eq!(restored, content);
    }

    // ── Multiple secrets in one file ─────────────────────────────────────

    #[test]
    fn redacts_multiple_secrets_in_same_file() {
        let content = "\
GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789
AWS_KEY=AKIAIOSFODNN7EXAMPLE
DB_URL=postgres://admin:supersecret@db.example.com:5432/mydb";
        let result = redact_content(content, ".env", &builtin_patterns(), "run-1");
        assert!(
            result.records.len() >= 3,
            "should redact at least 3 secrets, got {}",
            result.records.len()
        );
        // Verify none of the originals appear.
        assert!(!result.content.contains("ghp_"));
        assert!(!result.content.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!result.content.contains("supersecret"));
    }

    #[test]
    fn round_trip_with_multiple_secrets() {
        let content = "\
GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789
AWS_KEY=AKIAIOSFODNN7EXAMPLE";
        let result = redact_content(content, ".env", &builtin_patterns(), "run-1");
        let restored = unredact_content(&result.content, &result.map);
        assert_eq!(restored, content);
    }

    // ── Scan report ──────────────────────────────────────────────────────

    #[test]
    fn scan_report_from_findings() {
        let content = "\
token=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789
key=AKIAIOSFODNN7EXAMPLE";
        let findings = scan_content(content, ".env", &builtin_patterns());
        let report = ScanReport::from_findings("run-1", &findings);
        assert!(report.total_findings >= 2);
        assert!(report.has_critical);
        assert!(report.affected_files.contains(&".env".to_string()));
    }

    #[test]
    fn scan_report_serializes_to_json() {
        let findings = scan_content("key=AKIAIOSFODNN7EXAMPLE", "test.txt", &builtin_patterns());
        let report = ScanReport::from_findings("run-1", &findings);
        let json = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-1");
    }

    // ── Edge cases ───────────────────────────────────────────────────────

    #[test]
    fn empty_content_produces_no_findings() {
        let findings = scan_content("", "empty.txt", &builtin_patterns());
        assert!(findings.is_empty());
    }

    #[test]
    fn empty_patterns_produces_no_findings() {
        let findings = scan_content("AKIAIOSFODNN7EXAMPLE", "test.txt", &[]);
        assert!(findings.is_empty());
    }

    #[test]
    fn redaction_map_serializes_to_json() {
        let content = "tok = ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123456789";
        let result = redact_content(content, ".env", &builtin_patterns(), "run-1");
        let json = serde_json::to_string_pretty(&result.map).unwrap();
        let parsed: RedactionMap = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.run_id, "run-1");
        assert_eq!(parsed.entries.len(), result.map.entries.len());
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        let h1 = sha256_hex(b"test");
        let h2 = sha256_hex(b"test");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // 256 bits = 32 bytes = 64 hex chars
    }

    #[test]
    fn finding_preserves_file_path() {
        let findings = scan_content(
            "key = AKIAIOSFODNN7EXAMPLE",
            "/project/src/config.ts",
            &builtin_patterns(),
        );
        for f in &findings {
            assert_eq!(f.file_path, "/project/src/config.ts");
        }
    }
}
