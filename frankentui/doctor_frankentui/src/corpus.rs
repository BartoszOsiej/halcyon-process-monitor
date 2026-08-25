// SPDX-License-Identifier: Apache-2.0
//! Curated OpenTUI corpus management.
//!
//! Maintains a registry of representative OpenTUI projects with pinned
//! snapshots, metadata, license records, and complexity tags for regression
//! testing and migration coverage analysis.
//!
//! Each corpus entry is fully reproducible from its manifest definition.
//! The pipeline:
//!   1. Define entries in a JSONL manifest.
//!   2. `acquire_corpus()` materializes snapshots via the import pipeline.
//!   3. `verify_corpus()` checks integrity hashes and snapshot presence.
//!   4. `diff_corpus()` produces a changelog between two manifest versions.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DoctorError, Result};
use crate::util::{copy_tree_snapshot_materialized, join_validated_child_path};

// ── Configuration ────────────────────────────────────────────────────────

/// Default root directory for corpus snapshots.
pub const DEFAULT_CORPUS_ROOT: &str = "/tmp/doctor_frankentui/corpus";

// ── Types ────────────────────────────────────────────────────────────────

/// A single project entry in the corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusEntry {
    /// Unique slug for this project (e.g., "opentui-dashboard-basic").
    pub slug: String,
    /// Human-readable description.
    pub description: String,
    /// Source URL (Git URL or local path reference).
    pub source_url: String,
    /// Pinned commit SHA for reproducibility.
    pub pinned_commit: String,
    /// SPDX license identifier.
    pub license: String,
    /// License verification status.
    pub license_verified: bool,
    /// Provenance metadata (how/when this entry was sourced).
    pub provenance: CorpusProvenance,
    /// Complexity classification tags.
    pub complexity_tags: Vec<ComplexityTag>,
    /// Feature coverage tags (which migration features this exercises).
    pub feature_tags: Vec<String>,
    /// Expected metrics from last successful acquisition.
    pub expected_metrics: Option<CorpusMetrics>,
    /// Whether this entry is active in the regression suite.
    pub active: bool,
}

/// Provenance metadata for a corpus entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusProvenance {
    /// Who added this entry.
    pub added_by: String,
    /// ISO timestamp of addition.
    pub added_at: String,
    /// Rationale for inclusion in the corpus.
    pub rationale: String,
    /// Source of the project (e.g., "github-public", "synthetic", "curated").
    pub source_type: ProvenanceSourceType,
    /// Any notes about licensing or attribution.
    pub attribution_notes: Option<String>,
}

/// How the project was sourced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenanceSourceType {
    /// Public GitHub repository.
    GithubPublic,
    /// Synthetic test fixture.
    Synthetic,
    /// Manually curated from documentation/examples.
    Curated,
    /// Open-source project with explicit license.
    OpenSource,
}

/// Complexity classification.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ComplexityTag {
    /// Trivial single-component app.
    Trivial,
    /// Small app (<10 components, simple state).
    Small,
    /// Medium app (10-50 components, mixed state patterns).
    Medium,
    /// Large app (50+ components, complex state, routing).
    Large,
    /// Uses TypeScript.
    TypeScript,
    /// Uses global state management (Redux, Zustand, etc.).
    GlobalState,
    /// Has server-side rendering patterns.
    ServerRendering,
    /// Uses code splitting / dynamic imports.
    CodeSplitting,
    /// Has complex routing (nested, dynamic).
    ComplexRouting,
    /// Uses custom hooks extensively.
    CustomHooks,
    /// Has accessibility features.
    Accessibility,
    /// Uses CSS-in-JS or theme systems.
    ThemedStyling,
    /// Has form handling with validation.
    FormValidation,
    /// Uses real-time/WebSocket patterns.
    RealTime,
    /// Monorepo/workspace structure.
    Monorepo,
}

/// Expected metrics for a corpus entry (for regression detection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusMetrics {
    /// Number of source files.
    pub file_count: usize,
    /// Number of detected components.
    pub component_count: usize,
    /// Number of detected hooks.
    pub hook_count: usize,
    /// Number of modules in the graph.
    pub module_count: usize,
    /// Source hash from intake.
    pub source_hash: String,
    /// Number of effects detected.
    pub effect_count: usize,
    /// Total lines of code (approximate).
    pub loc_approx: usize,
}

/// Full corpus manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusManifest {
    /// Manifest schema version.
    pub schema_version: String,
    /// When this manifest was last updated.
    pub updated_at: String,
    /// Manifest content hash for integrity.
    pub manifest_hash: String,
    /// All corpus entries, keyed by slug.
    pub entries: BTreeMap<String, CorpusEntry>,
}

impl CorpusManifest {
    /// Compute the manifest hash from entries (deterministic).
    pub fn compute_hash(entries: &BTreeMap<String, CorpusEntry>) -> String {
        let mut hasher = Sha256::new();
        // Serialize entries deterministically (BTreeMap is sorted).
        let canonical = serde_json::to_string(entries).unwrap_or_default();
        hasher.update(canonical.as_bytes());
        let digest = hasher.finalize();
        hex_encode(digest.as_slice())
    }

    /// Validate manifest internal consistency.
    pub fn validate(&self) -> Vec<ManifestWarning> {
        let mut warnings = Vec::new();

        // Check hash integrity.
        let computed = Self::compute_hash(&self.entries);
        if computed != self.manifest_hash {
            warnings.push(ManifestWarning {
                kind: WarningKind::IntegrityMismatch,
                message: format!(
                    "manifest_hash mismatch: stored={}, computed={}",
                    self.manifest_hash, computed
                ),
                entry_slug: None,
            });
        }

        // Check each entry.
        for (slug, entry) in &self.entries {
            if slug != &entry.slug {
                warnings.push(ManifestWarning {
                    kind: WarningKind::SlugMismatch,
                    message: format!("key '{}' does not match entry slug '{}'", slug, entry.slug),
                    entry_slug: Some(slug.clone()),
                });
            }

            if entry.pinned_commit.is_empty() {
                warnings.push(ManifestWarning {
                    kind: WarningKind::MissingPin,
                    message: "pinned_commit is empty".to_string(),
                    entry_slug: Some(slug.clone()),
                });
            }

            if entry.license.is_empty() {
                warnings.push(ManifestWarning {
                    kind: WarningKind::MissingLicense,
                    message: "license is empty".to_string(),
                    entry_slug: Some(slug.clone()),
                });
            }

            if !entry.license_verified {
                warnings.push(ManifestWarning {
                    kind: WarningKind::UnverifiedLicense,
                    message: "license has not been verified".to_string(),
                    entry_slug: Some(slug.clone()),
                });
            }

            if entry.complexity_tags.is_empty() {
                warnings.push(ManifestWarning {
                    kind: WarningKind::MissingTags,
                    message: "no complexity tags assigned".to_string(),
                    entry_slug: Some(slug.clone()),
                });
            }
        }

        warnings
    }
}

/// A warning from manifest validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestWarning {
    pub kind: WarningKind,
    pub message: String,
    pub entry_slug: Option<String>,
}

/// Kind of manifest warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarningKind {
    IntegrityMismatch,
    SlugMismatch,
    MissingPin,
    MissingLicense,
    UnverifiedLicense,
    MissingTags,
    MissingSnapshot,
    MetricsDrift,
}

// ── Acquisition ──────────────────────────────────────────────────────────

/// Result of acquiring a corpus entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquisitionResult {
    pub slug: String,
    pub status: AcquisitionStatus,
    pub snapshot_path: Option<String>,
    pub source_hash: Option<String>,
    pub metrics: Option<CorpusMetrics>,
    pub error_message: Option<String>,
    pub duration_ms: u64,
}

/// Status of an acquisition attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcquisitionStatus {
    /// Successfully acquired and verified.
    Ok,
    /// Acquired but metrics differ from expected.
    Drifted,
    /// Acquisition failed.
    Failed,
    /// Skipped (entry is inactive).
    Skipped,
}

/// Acquire all active entries in a corpus manifest.
pub fn acquire_corpus(manifest: &CorpusManifest, corpus_root: &Path) -> Vec<AcquisitionResult> {
    let mut results = Vec::new();

    for (slug, entry) in &manifest.entries {
        if !entry.active {
            results.push(AcquisitionResult {
                slug: slug.clone(),
                status: AcquisitionStatus::Skipped,
                snapshot_path: None,
                source_hash: None,
                metrics: None,
                error_message: None,
                duration_ms: 0,
            });
            continue;
        }

        let result = acquire_entry(entry, corpus_root);
        results.push(result);
    }

    results
}

/// Acquire a single corpus entry.
fn acquire_entry(entry: &CorpusEntry, corpus_root: &Path) -> AcquisitionResult {
    let start = std::time::Instant::now();
    let entry_dir = match join_validated_child_path(corpus_root, &entry.slug, "slug") {
        Ok(path) => path,
        Err(error) => {
            return AcquisitionResult {
                slug: entry.slug.clone(),
                status: AcquisitionStatus::Failed,
                snapshot_path: None,
                source_hash: None,
                metrics: None,
                error_message: Some(error.to_string()),
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    // Create entry directory.
    if let Err(e) = fs::create_dir_all(&entry_dir) {
        return AcquisitionResult {
            slug: entry.slug.clone(),
            status: AcquisitionStatus::Failed,
            snapshot_path: None,
            source_hash: None,
            metrics: None,
            error_message: Some(format!("failed to create directory: {e}")),
            duration_ms: start.elapsed().as_millis() as u64,
        };
    }

    let snapshot_dir = entry_dir.join("snapshot");

    // If snapshot already exists and hash matches, skip re-acquisition.
    if snapshot_dir.exists()
        && let Some(ref expected) = entry.expected_metrics
    {
        let current_hash = compute_directory_hash(&snapshot_dir);
        if current_hash.as_deref() == Some(expected.source_hash.as_str()) {
            return AcquisitionResult {
                slug: entry.slug.clone(),
                status: AcquisitionStatus::Ok,
                snapshot_path: Some(snapshot_dir.to_string_lossy().to_string()),
                source_hash: Some(expected.source_hash.clone()),
                metrics: entry.expected_metrics.clone(),
                error_message: None,
                duration_ms: start.elapsed().as_millis() as u64,
            };
        }
    }

    // Determine acquisition method.
    let is_git = entry.source_url.starts_with("https://")
        || entry.source_url.starts_with("git@")
        || entry.source_url.starts_with("ssh://")
        || entry.source_url.ends_with(".git");

    let result = if is_git {
        acquire_from_git(entry, &entry_dir, &snapshot_dir)
    } else {
        acquire_from_local(entry, &snapshot_dir)
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok((source_hash, metrics)) => {
            let status = if let Some(ref expected) = entry.expected_metrics {
                if expected.source_hash != source_hash {
                    AcquisitionStatus::Drifted
                } else {
                    AcquisitionStatus::Ok
                }
            } else {
                AcquisitionStatus::Ok
            };

            // Write entry metadata alongside snapshot.
            let _ = write_entry_metadata(entry, &entry_dir, &source_hash, &metrics);

            AcquisitionResult {
                slug: entry.slug.clone(),
                status,
                snapshot_path: Some(snapshot_dir.to_string_lossy().to_string()),
                source_hash: Some(source_hash),
                metrics: Some(metrics),
                error_message: None,
                duration_ms,
            }
        }
        Err(msg) => AcquisitionResult {
            slug: entry.slug.clone(),
            status: AcquisitionStatus::Failed,
            snapshot_path: None,
            source_hash: None,
            metrics: None,
            error_message: Some(msg),
            duration_ms,
        },
    }
}

fn acquire_from_git(
    entry: &CorpusEntry,
    entry_dir: &Path,
    snapshot_dir: &Path,
) -> std::result::Result<(String, CorpusMetrics), String> {
    let clone_dir = entry_dir.join("_clone");

    // Clean previous clone if present.
    if clone_dir.exists() {
        fs::remove_dir_all(&clone_dir).map_err(|e| format!("cleanup failed: {e}"))?;
    }

    let result = (|| {
        // Shallow clone at pinned commit.
        let output = std::process::Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "--single-branch",
                &entry.source_url,
                &clone_dir.to_string_lossy(),
            ])
            .current_dir(entry_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git clone failed: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git clone failed: {stderr}"));
        }

        // Checkout pinned commit if not HEAD.
        if !entry.pinned_commit.is_empty() {
            // Fetch the specific commit (depth=1 may not have it).
            let _ = std::process::Command::new("git")
                .args(["fetch", "origin", &entry.pinned_commit])
                .current_dir(&clone_dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output();

            let checkout = std::process::Command::new("git")
                .args(["checkout", &entry.pinned_commit])
                .current_dir(&clone_dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .map_err(|e| format!("git checkout failed: {e}"))?;

            if !checkout.status.success() {
                let stderr = String::from_utf8_lossy(&checkout.stderr);
                return Err(format!(
                    "git checkout {} failed: {stderr}",
                    entry.pinned_commit
                ));
            }
        }

        // Materialize snapshot via archive (deterministic, excludes .git).
        if snapshot_dir.exists() {
            fs::remove_dir_all(snapshot_dir).map_err(|e| format!("snapshot cleanup: {e}"))?;
        }
        fs::create_dir_all(snapshot_dir).map_err(|e| format!("create snapshot dir: {e}"))?;

        let archive = std::process::Command::new("git")
            .args(["archive", "--format=tar", "HEAD"])
            .current_dir(&clone_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git archive failed: {e}"))?;

        if !archive.status.success() {
            let stderr = String::from_utf8_lossy(&archive.stderr);
            let trimmed = stderr.trim();
            if trimmed.is_empty() {
                return Err("git archive failed".to_string());
            }
            return Err(format!("git archive failed: {trimmed}"));
        }

        extract_tar_archive(snapshot_dir, &archive.stdout)?;

        // Compute metrics.
        let source_hash =
            compute_directory_hash(snapshot_dir).unwrap_or_else(|| "hash_failed".to_string());
        let metrics = compute_snapshot_metrics(snapshot_dir);

        Ok((source_hash, metrics))
    })();

    let _ = fs::remove_dir_all(&clone_dir);

    result
}

fn extract_tar_archive(
    snapshot_dir: &Path,
    archive_bytes: &[u8],
) -> std::result::Result<(), String> {
    let mut child = std::process::Command::new("tar")
        .args(["xf", "-"])
        .current_dir(snapshot_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("tar extract failed: {e}"))?;

    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| "tar extract failed: stdin pipe unavailable".to_string())
        .and_then(|mut stdin| {
            stdin
                .write_all(archive_bytes)
                .map_err(|e| format!("tar extract stdin write failed: {e}"))
        });

    let output = child
        .wait_with_output()
        .map_err(|e| format!("tar extract wait failed: {e}"))?;

    if let Err(error) = write_result {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        if trimmed.is_empty() {
            return Err(error);
        }
        return Err(format!("{error}; tar stderr: {trimmed}"));
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        if trimmed.is_empty() {
            return Err("tar extract failed".to_string());
        }
        return Err(format!("tar extract failed: {trimmed}"));
    }

    Ok(())
}

fn acquire_from_local(
    entry: &CorpusEntry,
    snapshot_dir: &Path,
) -> std::result::Result<(String, CorpusMetrics), String> {
    let source = Path::new(&entry.source_url);
    if !source.exists() {
        return Err(format!("local source does not exist: {}", entry.source_url));
    }

    if snapshot_dir.exists() {
        fs::remove_dir_all(snapshot_dir).map_err(|e| format!("snapshot cleanup: {e}"))?;
    }
    fs::create_dir_all(snapshot_dir).map_err(|e| format!("create snapshot dir: {e}"))?;

    copy_tree(source, snapshot_dir).map_err(|e| format!("copy failed: {e}"))?;

    let source_hash =
        compute_directory_hash(snapshot_dir).unwrap_or_else(|| "hash_failed".to_string());
    let metrics = compute_snapshot_metrics(snapshot_dir);

    Ok((source_hash, metrics))
}

// ── Verification ─────────────────────────────────────────────────────────

/// Verification result for the entire corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub total: usize,
    pub ok: usize,
    pub missing: usize,
    pub drifted: usize,
    pub inactive: usize,
    pub entries: Vec<EntryVerification>,
}

/// Verification status of a single entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryVerification {
    pub slug: String,
    pub status: VerificationStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationStatus {
    Ok,
    Missing,
    HashDrifted,
    MetricsDrifted,
    Inactive,
}

/// Verify all corpus entries against their expected state.
pub fn verify_corpus(manifest: &CorpusManifest, corpus_root: &Path) -> VerificationReport {
    let mut entries = Vec::new();
    let mut ok = 0usize;
    let mut missing = 0usize;
    let mut drifted = 0usize;
    let mut inactive = 0usize;

    for (slug, entry) in &manifest.entries {
        if !entry.active {
            entries.push(EntryVerification {
                slug: slug.clone(),
                status: VerificationStatus::Inactive,
                message: None,
            });
            inactive += 1;
            continue;
        }

        let snapshot_dir = match join_validated_child_path(corpus_root, slug, "slug") {
            Ok(path) => path.join("snapshot"),
            Err(error) => {
                entries.push(EntryVerification {
                    slug: slug.clone(),
                    status: VerificationStatus::Missing,
                    message: Some(error.to_string()),
                });
                missing += 1;
                continue;
            }
        };
        if !snapshot_dir.exists() {
            entries.push(EntryVerification {
                slug: slug.clone(),
                status: VerificationStatus::Missing,
                message: Some("snapshot directory not found".to_string()),
            });
            missing += 1;
            continue;
        }

        if let Some(ref expected) = entry.expected_metrics {
            let current_hash = compute_directory_hash(&snapshot_dir);
            if current_hash.as_deref() != Some(expected.source_hash.as_str()) {
                entries.push(EntryVerification {
                    slug: slug.clone(),
                    status: VerificationStatus::HashDrifted,
                    message: Some(format!(
                        "hash mismatch: expected={}, actual={}",
                        expected.source_hash,
                        current_hash.as_deref().unwrap_or("none")
                    )),
                });
                drifted += 1;
                continue;
            }
        }

        entries.push(EntryVerification {
            slug: slug.clone(),
            status: VerificationStatus::Ok,
            message: None,
        });
        ok += 1;
    }

    VerificationReport {
        total: manifest.entries.len(),
        ok,
        missing,
        drifted,
        inactive,
        entries,
    }
}

// ── Changelog ────────────────────────────────────────────────────────────

/// A change between two manifest versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusChange {
    pub slug: String,
    pub kind: ChangeKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Removed,
    PinUpdated,
    MetadataChanged,
    Activated,
    Deactivated,
}

/// Compute the diff between two corpus manifests.
pub fn diff_corpus(old: &CorpusManifest, new: &CorpusManifest) -> Vec<CorpusChange> {
    let mut changes = Vec::new();

    // Detect additions and modifications.
    for (slug, new_entry) in &new.entries {
        if let Some(old_entry) = old.entries.get(slug) {
            if old_entry.pinned_commit != new_entry.pinned_commit {
                changes.push(CorpusChange {
                    slug: slug.clone(),
                    kind: ChangeKind::PinUpdated,
                    detail: format!("{} -> {}", old_entry.pinned_commit, new_entry.pinned_commit),
                });
            }
            if old_entry.active && !new_entry.active {
                changes.push(CorpusChange {
                    slug: slug.clone(),
                    kind: ChangeKind::Deactivated,
                    detail: String::new(),
                });
            } else if !old_entry.active && new_entry.active {
                changes.push(CorpusChange {
                    slug: slug.clone(),
                    kind: ChangeKind::Activated,
                    detail: String::new(),
                });
            }
            if old_entry.license != new_entry.license
                || old_entry.description != new_entry.description
                || old_entry.feature_tags != new_entry.feature_tags
            {
                changes.push(CorpusChange {
                    slug: slug.clone(),
                    kind: ChangeKind::MetadataChanged,
                    detail: "description, license, or feature tags changed".to_string(),
                });
            }
        } else {
            changes.push(CorpusChange {
                slug: slug.clone(),
                kind: ChangeKind::Added,
                detail: new_entry.description.clone(),
            });
        }
    }

    // Detect removals.
    for slug in old.entries.keys() {
        if !new.entries.contains_key(slug) {
            changes.push(CorpusChange {
                slug: slug.clone(),
                kind: ChangeKind::Removed,
                detail: String::new(),
            });
        }
    }

    changes
}

// ── Manifest I/O ─────────────────────────────────────────────────────────

/// Load a corpus manifest from a JSONL file.
/// Each line is a CorpusEntry JSON; the manifest wrapper is synthesized.
pub fn load_manifest_from_jsonl(path: &Path) -> Result<CorpusManifest> {
    let content = fs::read_to_string(path)
        .map_err(|e| DoctorError::exit(50, format!("failed to read corpus manifest: {e}")))?;

    let mut entries = BTreeMap::new();
    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let entry: CorpusEntry = serde_json::from_str(trimmed).map_err(|e| {
            DoctorError::exit(51, format!("corpus manifest line {}: {e}", line_no + 1))
        })?;
        entries.insert(entry.slug.clone(), entry);
    }

    let manifest_hash = CorpusManifest::compute_hash(&entries);

    Ok(CorpusManifest {
        schema_version: "corpus-manifest-v1".to_string(),
        updated_at: now_iso(),
        manifest_hash,
        entries,
    })
}

/// Load a corpus manifest from a JSON file.
pub fn load_manifest_from_json(path: &Path) -> Result<CorpusManifest> {
    let content = fs::read_to_string(path)
        .map_err(|e| DoctorError::exit(50, format!("failed to read corpus manifest: {e}")))?;

    let manifest: CorpusManifest = serde_json::from_str(&content)
        .map_err(|e| DoctorError::exit(51, format!("invalid corpus manifest JSON: {e}")))?;

    Ok(manifest)
}

/// Save a corpus manifest to a JSON file.
pub fn save_manifest(manifest: &CorpusManifest, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| DoctorError::exit(52, format!("failed to serialize manifest: {e}")))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            DoctorError::exit(52, format!("failed to create manifest directory: {e}"))
        })?;
    }

    fs::write(path, json)
        .map_err(|e| DoctorError::exit(52, format!("failed to write manifest: {e}")))?;

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn write_entry_metadata(
    entry: &CorpusEntry,
    entry_dir: &Path,
    source_hash: &str,
    metrics: &CorpusMetrics,
) -> std::result::Result<(), String> {
    let meta = serde_json::json!({
        "slug": entry.slug,
        "source_url": entry.source_url,
        "pinned_commit": entry.pinned_commit,
        "license": entry.license,
        "source_hash": source_hash,
        "metrics": metrics,
        "acquired_at": now_iso(),
    });

    let path = entry_dir.join("entry_meta.json");
    let json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// Compute a deterministic hash of a directory's contents.
fn compute_directory_hash(dir: &Path) -> Option<String> {
    let mut paths = Vec::new();
    collect_files(dir, &mut paths);
    paths.sort();

    let mut hasher = Sha256::new();
    for path in &paths {
        let rel = path.strip_prefix(dir).ok()?;
        hasher.update(rel.to_string_lossy().as_bytes());
        if let Ok(content) = fs::read(path) {
            hasher.update(&content);
        }
    }

    let digest = hasher.finalize();
    Some(hex_encode(digest.as_slice()))
}

/// Collect all regular files in a directory (recursive).
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip .git and node_modules.
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && (name == ".git" || name == "node_modules")
            {
                continue;
            }
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Compute basic metrics from a snapshot directory.
fn compute_snapshot_metrics(dir: &Path) -> CorpusMetrics {
    let mut files = Vec::new();
    collect_files(dir, &mut files);

    let source_extensions: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "mts", "cjs", "cts"];

    let mut source_files = 0usize;
    let mut loc = 0usize;

    for file in &files {
        if let Some(ext) = file.extension().and_then(|e| e.to_str())
            && source_extensions.contains(&ext)
        {
            source_files += 1;
            if let Ok(content) = fs::read_to_string(file) {
                loc += content.lines().count();
            }
        }
    }

    let source_hash = compute_directory_hash(dir).unwrap_or_else(|| "hash_failed".to_string());

    CorpusMetrics {
        file_count: source_files,
        component_count: 0, // Filled by analysis pass.
        hook_count: 0,      // Filled by analysis pass.
        module_count: 0,    // Filled by analysis pass.
        source_hash,
        effect_count: 0, // Filled by analysis pass.
        loc_approx: loc,
    }
}

/// Copy a directory tree, skipping .git and node_modules.
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    copy_tree_snapshot_materialized(src, dst, |relative| {
        relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::Normal(name)
                    if name == std::ffi::OsStr::new(".git")
                        || name == std::ffi::OsStr::new("node_modules")
            )
        })
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(slug: &str, commit: &str) -> CorpusEntry {
        CorpusEntry {
            slug: slug.to_string(),
            description: format!("Test entry {slug}"),
            source_url: format!("https://github.com/test/{slug}"),
            pinned_commit: commit.to_string(),
            license: "MIT".to_string(),
            license_verified: true,
            provenance: CorpusProvenance {
                added_by: "test".to_string(),
                added_at: "2026-01-01T00:00:00Z".to_string(),
                rationale: "test fixture".to_string(),
                source_type: ProvenanceSourceType::Synthetic,
                attribution_notes: None,
            },
            complexity_tags: vec![ComplexityTag::Small],
            feature_tags: vec!["hooks".to_string()],
            expected_metrics: None,
            active: true,
        }
    }

    fn make_manifest(entries: Vec<CorpusEntry>) -> CorpusManifest {
        let mut map = BTreeMap::new();
        for e in entries {
            map.insert(e.slug.clone(), e);
        }
        let hash = CorpusManifest::compute_hash(&map);
        CorpusManifest {
            schema_version: "corpus-manifest-v1".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            manifest_hash: hash,
            entries: map,
        }
    }

    #[test]
    fn manifest_hash_is_deterministic() {
        let e1 = make_entry("alpha", "abc123");
        let e2 = make_entry("beta", "def456");
        let m1 = make_manifest(vec![e1.clone(), e2.clone()]);
        let m2 = make_manifest(vec![e1, e2]);
        assert_eq!(m1.manifest_hash, m2.manifest_hash);
    }

    #[test]
    fn manifest_hash_changes_with_content() {
        let e1 = make_entry("alpha", "abc123");
        let e2 = make_entry("alpha", "xyz789");
        let m1 = make_manifest(vec![e1]);
        let m2 = make_manifest(vec![e2]);
        assert_ne!(m1.manifest_hash, m2.manifest_hash);
    }

    #[test]
    fn validate_detects_empty_commit() {
        let mut entry = make_entry("test", "");
        entry.pinned_commit = String::new();
        let manifest = make_manifest(vec![entry]);
        let warnings = manifest.validate();
        assert!(warnings.iter().any(|w| w.kind == WarningKind::MissingPin));
    }

    #[test]
    fn validate_detects_empty_license() {
        let mut entry = make_entry("test", "abc");
        entry.license = String::new();
        let manifest = make_manifest(vec![entry]);
        let warnings = manifest.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.kind == WarningKind::MissingLicense)
        );
    }

    #[test]
    fn validate_detects_unverified_license() {
        let mut entry = make_entry("test", "abc");
        entry.license_verified = false;
        let manifest = make_manifest(vec![entry]);
        let warnings = manifest.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.kind == WarningKind::UnverifiedLicense)
        );
    }

    #[test]
    fn validate_detects_missing_tags() {
        let mut entry = make_entry("test", "abc");
        entry.complexity_tags = Vec::new();
        let manifest = make_manifest(vec![entry]);
        let warnings = manifest.validate();
        assert!(warnings.iter().any(|w| w.kind == WarningKind::MissingTags));
    }

    #[test]
    fn validate_detects_slug_mismatch() {
        let entry = make_entry("actual", "abc");
        let mut map = BTreeMap::new();
        map.insert("wrong_key".to_string(), entry);
        let hash = CorpusManifest::compute_hash(&map);
        let manifest = CorpusManifest {
            schema_version: "v1".to_string(),
            updated_at: "now".to_string(),
            manifest_hash: hash,
            entries: map,
        };
        let warnings = manifest.validate();
        assert!(warnings.iter().any(|w| w.kind == WarningKind::SlugMismatch));
    }

    #[test]
    fn validate_detects_hash_mismatch() {
        let manifest = CorpusManifest {
            schema_version: "v1".to_string(),
            updated_at: "now".to_string(),
            manifest_hash: "bad_hash".to_string(),
            entries: BTreeMap::new(),
        };
        let warnings = manifest.validate();
        assert!(
            warnings
                .iter()
                .any(|w| w.kind == WarningKind::IntegrityMismatch)
        );
    }

    #[test]
    fn validate_clean_manifest_no_errors() {
        let manifest = make_manifest(vec![
            make_entry("alpha", "abc123"),
            make_entry("beta", "def456"),
        ]);
        let warnings = manifest.validate();
        assert!(warnings.is_empty());
    }

    #[test]
    fn diff_detects_addition() {
        let old = make_manifest(vec![make_entry("alpha", "abc")]);
        let new = make_manifest(vec![make_entry("alpha", "abc"), make_entry("beta", "def")]);
        let changes = diff_corpus(&old, &new);
        assert!(
            changes
                .iter()
                .any(|c| c.slug == "beta" && c.kind == ChangeKind::Added)
        );
    }

    #[test]
    fn diff_detects_removal() {
        let old = make_manifest(vec![make_entry("alpha", "abc"), make_entry("beta", "def")]);
        let new = make_manifest(vec![make_entry("alpha", "abc")]);
        let changes = diff_corpus(&old, &new);
        assert!(
            changes
                .iter()
                .any(|c| c.slug == "beta" && c.kind == ChangeKind::Removed)
        );
    }

    #[test]
    fn diff_detects_pin_update() {
        let old = make_manifest(vec![make_entry("alpha", "abc")]);
        let new = make_manifest(vec![make_entry("alpha", "xyz")]);
        let changes = diff_corpus(&old, &new);
        assert!(
            changes
                .iter()
                .any(|c| c.slug == "alpha" && c.kind == ChangeKind::PinUpdated)
        );
    }

    #[test]
    fn diff_detects_activation_change() {
        let mut entry_old = make_entry("alpha", "abc");
        entry_old.active = true;
        let mut entry_new = make_entry("alpha", "abc");
        entry_new.active = false;

        let old = make_manifest(vec![entry_old]);
        let new = make_manifest(vec![entry_new]);
        let changes = diff_corpus(&old, &new);
        assert!(
            changes
                .iter()
                .any(|c| c.slug == "alpha" && c.kind == ChangeKind::Deactivated)
        );
    }

    #[test]
    fn diff_no_changes_for_identical() {
        let m = make_manifest(vec![make_entry("alpha", "abc")]);
        let changes = diff_corpus(&m, &m);
        assert!(changes.is_empty());
    }

    #[test]
    fn verify_reports_missing_snapshots() {
        let manifest = make_manifest(vec![make_entry("nonexistent", "abc")]);
        let report = verify_corpus(&manifest, Path::new("/tmp/corpus_test_nonexistent"));
        assert_eq!(report.missing, 1);
        assert_eq!(report.entries[0].status, VerificationStatus::Missing);
    }

    #[test]
    fn verify_rejects_unsafe_slug_paths() {
        let manifest = make_manifest(vec![make_entry("../escape", "abc")]);
        let report = verify_corpus(&manifest, Path::new("/tmp/corpus_test_nonexistent"));
        assert_eq!(report.missing, 1);
        assert_eq!(report.entries[0].status, VerificationStatus::Missing);
        assert!(
            report.entries[0].message.as_deref().is_some_and(
                |message| message.contains("slug must be a single safe path component")
            )
        );
    }

    #[test]
    fn verify_skips_inactive_entries() {
        let mut entry = make_entry("inactive_test", "abc");
        entry.active = false;
        let manifest = make_manifest(vec![entry]);
        let report = verify_corpus(&manifest, Path::new("/tmp/doesnt_matter"));
        assert_eq!(report.inactive, 1);
        assert_eq!(report.entries[0].status, VerificationStatus::Inactive);
    }

    #[test]
    fn acquire_skips_inactive() {
        let mut entry = make_entry("skip_me", "abc");
        entry.active = false;
        let manifest = make_manifest(vec![entry]);
        let results = acquire_corpus(&manifest, Path::new("/tmp/corpus_skip_test"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, AcquisitionStatus::Skipped);
    }

    #[test]
    fn manifest_serialization_roundtrip() {
        let manifest = make_manifest(vec![
            make_entry("alpha", "abc123"),
            make_entry("beta", "def456"),
        ]);
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let deserialized: CorpusManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest.manifest_hash, deserialized.manifest_hash);
        assert_eq!(manifest.entries.len(), deserialized.entries.len());
    }

    #[test]
    fn manifest_from_jsonl_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpus.jsonl");
        let entry = make_entry("test", "abc123");
        let line = serde_json::to_string(&entry).unwrap();
        fs::write(&path, format!("{line}\n")).unwrap();

        let manifest = load_manifest_from_jsonl(&path).unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert!(manifest.entries.contains_key("test"));
    }

    #[test]
    fn jsonl_skips_comments_and_blanks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpus.jsonl");
        let entry = make_entry("test", "abc");
        let line = serde_json::to_string(&entry).unwrap();
        let content = format!("# Comment\n\n{line}\n\n# Another comment\n");
        fs::write(&path, content).unwrap();

        let manifest = load_manifest_from_jsonl(&path).unwrap();
        assert_eq!(manifest.entries.len(), 1);
    }

    #[test]
    fn save_and_reload_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let manifest = make_manifest(vec![make_entry("test", "abc")]);
        save_manifest(&manifest, &path).unwrap();
        let reloaded = load_manifest_from_json(&path).unwrap();
        assert_eq!(manifest.entries.len(), reloaded.entries.len());
        assert_eq!(manifest.manifest_hash, reloaded.manifest_hash);
    }

    #[test]
    fn directory_hash_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("a.txt"), "hello").unwrap();
        fs::write(sub.join("b.txt"), "world").unwrap();

        let h1 = compute_directory_hash(dir.path()).unwrap();
        let h2 = compute_directory_hash(dir.path()).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn directory_hash_changes_with_content() {
        let dir1 = tempfile::tempdir().unwrap();
        fs::write(dir1.path().join("a.txt"), "hello").unwrap();
        let h1 = compute_directory_hash(dir1.path()).unwrap();

        let dir2 = tempfile::tempdir().unwrap();
        fs::write(dir2.path().join("a.txt"), "world").unwrap();
        let h2 = compute_directory_hash(dir2.path()).unwrap();

        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_metrics_counts_source_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("app.tsx"), "function App() {}").unwrap();
        fs::write(dir.path().join("util.ts"), "export const x = 1;").unwrap();
        fs::write(dir.path().join("readme.md"), "# Readme").unwrap();

        let metrics = compute_snapshot_metrics(dir.path());
        assert_eq!(metrics.file_count, 2); // tsx + ts, not md
        assert!(metrics.loc_approx > 0);
    }

    #[test]
    fn copy_tree_skips_git_and_node_modules() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        fs::write(src.path().join("index.ts"), "export default 1;").unwrap();
        fs::create_dir_all(src.path().join(".git")).unwrap();
        fs::write(src.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        fs::create_dir_all(src.path().join("node_modules/pkg")).unwrap();
        fs::write(src.path().join("node_modules/pkg/index.js"), "//").unwrap();

        copy_tree(src.path(), dst.path()).unwrap();

        assert!(dst.path().join("index.ts").exists());
        assert!(!dst.path().join(".git").exists());
        assert!(!dst.path().join("node_modules").exists());
    }

    #[cfg(unix)]
    #[test]
    fn acquire_local_entry_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let src = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(src.path().join("index.tsx"), "function App() {}").unwrap();
        fs::write(
            outside.path().join("outside.ts"),
            "export const outside_flag = true;\n",
        )
        .unwrap();
        symlink(
            outside.path().join("outside.ts"),
            src.path().join("escape.ts"),
        )
        .unwrap();

        let entry = CorpusEntry {
            slug: "escape_test".to_string(),
            description: "test".to_string(),
            source_url: src.path().to_string_lossy().to_string(),
            pinned_commit: "local".to_string(),
            license: "MIT".to_string(),
            license_verified: true,
            provenance: CorpusProvenance {
                added_by: "test".to_string(),
                added_at: "now".to_string(),
                rationale: "test".to_string(),
                source_type: ProvenanceSourceType::Synthetic,
                attribution_notes: None,
            },
            complexity_tags: vec![ComplexityTag::Trivial],
            feature_tags: vec![],
            expected_metrics: None,
            active: true,
        };

        let corpus_dir = tempfile::tempdir().unwrap();
        let result = acquire_entry(&entry, corpus_dir.path());
        assert_eq!(result.status, AcquisitionStatus::Failed);
        assert!(
            result
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("escapes source root")),
            "unexpected error: {:?}",
            result.error_message
        );
    }

    #[test]
    fn extract_tar_archive_materializes_valid_archive() {
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "hello").unwrap();

        let archive = std::process::Command::new("tar")
            .args(["cf", "-", "."])
            .current_dir(source.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap();
        assert!(archive.status.success());

        let dest = tempfile::tempdir().unwrap();
        super::extract_tar_archive(dest.path(), &archive.stdout).unwrap();

        assert_eq!(
            fs::read_to_string(dest.path().join("file.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn extract_tar_archive_reports_invalid_archive_bytes() {
        let dest = tempfile::tempdir().unwrap();
        let error = super::extract_tar_archive(dest.path(), b"not a tar archive")
            .expect_err("invalid tar input should fail");

        assert!(error.contains("tar extract failed"));
        assert!(!dest.path().join("file.txt").exists());
    }

    #[test]
    fn acquire_from_git_cleans_up_clone_dir_on_checkout_failure() {
        let repo = tempfile::tempdir().unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(repo.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(init.success());

        let config_email = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(config_email.success());

        let config_name = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(config_name.success());

        fs::write(repo.path().join("README.md"), "hello").unwrap();
        let add = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(repo.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(add.success());

        let commit = std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(commit.success());

        let entry_dir = tempfile::tempdir().unwrap();
        let snapshot_dir = entry_dir.path().join("snapshot");
        let entry = CorpusEntry {
            slug: "git-cleanup".to_string(),
            description: "cleanup regression".to_string(),
            source_url: repo.path().to_string_lossy().to_string(),
            pinned_commit: "deadbeef".to_string(),
            license: "MIT".to_string(),
            license_verified: true,
            provenance: CorpusProvenance {
                added_by: "test".to_string(),
                added_at: "2026-01-01T00:00:00Z".to_string(),
                rationale: "regression".to_string(),
                source_type: ProvenanceSourceType::Synthetic,
                attribution_notes: None,
            },
            complexity_tags: vec![],
            feature_tags: vec![],
            expected_metrics: None,
            active: true,
        };

        let error = super::acquire_from_git(&entry, entry_dir.path(), &snapshot_dir)
            .expect_err("invalid pinned commit should fail checkout");
        assert!(error.contains("git checkout deadbeef failed"));
        assert!(!entry_dir.path().join("_clone").exists());
    }

    #[test]
    fn acquire_local_entry() {
        let src = tempfile::tempdir().unwrap();
        fs::write(
            src.path().join("index.tsx"),
            "function App() { return <div />; }",
        )
        .unwrap();
        fs::write(src.path().join("package.json"), r#"{"name":"test"}"#).unwrap();

        let entry = CorpusEntry {
            slug: "local_test".to_string(),
            description: "test".to_string(),
            source_url: src.path().to_string_lossy().to_string(),
            pinned_commit: "local".to_string(),
            license: "MIT".to_string(),
            license_verified: true,
            provenance: CorpusProvenance {
                added_by: "test".to_string(),
                added_at: "now".to_string(),
                rationale: "test".to_string(),
                source_type: ProvenanceSourceType::Synthetic,
                attribution_notes: None,
            },
            complexity_tags: vec![ComplexityTag::Trivial],
            feature_tags: vec![],
            expected_metrics: None,
            active: true,
        };

        let corpus_dir = tempfile::tempdir().unwrap();
        let result = acquire_entry(&entry, corpus_dir.path());
        assert_eq!(result.status, AcquisitionStatus::Ok);
        assert!(result.snapshot_path.is_some());
        assert!(result.source_hash.is_some());
        assert!(result.metrics.is_some());
        assert_eq!(result.metrics.as_ref().unwrap().file_count, 1);
    }

    #[test]
    fn acquire_entry_rejects_unsafe_slug() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("index.tsx"), "function App() {}").unwrap();

        let entry = CorpusEntry {
            slug: "../escape".to_string(),
            description: "test".to_string(),
            source_url: src.path().to_string_lossy().to_string(),
            pinned_commit: "local".to_string(),
            license: "MIT".to_string(),
            license_verified: true,
            provenance: CorpusProvenance {
                added_by: "test".to_string(),
                added_at: "now".to_string(),
                rationale: "test".to_string(),
                source_type: ProvenanceSourceType::Synthetic,
                attribution_notes: None,
            },
            complexity_tags: vec![ComplexityTag::Trivial],
            feature_tags: vec![],
            expected_metrics: None,
            active: true,
        };

        let corpus_dir = tempfile::tempdir().unwrap();
        let result = acquire_entry(&entry, corpus_dir.path());
        assert_eq!(result.status, AcquisitionStatus::Failed);
        assert!(
            result.error_message.as_deref().is_some_and(
                |message| message.contains("slug must be a single safe path component")
            ),
            "unexpected error: {:?}",
            result.error_message
        );
        assert!(!corpus_dir.path().join("escape").exists());
    }

    #[test]
    fn acquire_detects_drift() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("index.tsx"), "function App() {}").unwrap();

        let entry = CorpusEntry {
            slug: "drift_test".to_string(),
            description: "test".to_string(),
            source_url: src.path().to_string_lossy().to_string(),
            pinned_commit: "local".to_string(),
            license: "MIT".to_string(),
            license_verified: true,
            provenance: CorpusProvenance {
                added_by: "test".to_string(),
                added_at: "now".to_string(),
                rationale: "test".to_string(),
                source_type: ProvenanceSourceType::Synthetic,
                attribution_notes: None,
            },
            complexity_tags: vec![ComplexityTag::Trivial],
            feature_tags: vec![],
            expected_metrics: Some(CorpusMetrics {
                file_count: 1,
                component_count: 0,
                hook_count: 0,
                module_count: 0,
                source_hash: "stale_hash_that_wont_match".to_string(),
                effect_count: 0,
                loc_approx: 1,
            }),
            active: true,
        };

        let corpus_dir = tempfile::tempdir().unwrap();
        let result = acquire_entry(&entry, corpus_dir.path());
        assert_eq!(result.status, AcquisitionStatus::Drifted);
    }
}
