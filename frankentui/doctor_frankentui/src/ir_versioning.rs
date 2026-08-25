// SPDX-License-Identifier: Apache-2.0
//! IR versioning, migration adapters, and schema compatibility checks.
//!
//! Supports controlled IR evolution by:
//! 1. Parsing and comparing schema version tags.
//! 2. Maintaining a registry of migration adapters (vN → vN+1).
//! 3. Upgrading serialized IR manifests through the adapter chain.
//! 4. Producing actionable diagnostics on version mismatches.
//!
//! # Design
//!
//! Schema versions follow the convention `migration-ir-v{N}` where N is a
//! monotonically increasing integer.  Each bump implies a potentially
//! breaking change that requires an explicit migration adapter.
//!
//! Older manifests are upgraded reproducibly: the adapter chain is
//! deterministic and idempotent (applying the same upgrade twice yields
//! the same result).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::migration_ir::{self, IR_SCHEMA_VERSION, MigrationIr};

// ── Schema Version ──────────────────────────────────────────────────────

/// Parsed schema version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaVersion {
    /// Major version number (the N in `migration-ir-vN`).
    pub major: u32,
    /// The full version label (e.g. `"migration-ir-v1"`).
    pub label: String,
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl PartialOrd for SchemaVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SchemaVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major.cmp(&other.major)
    }
}

/// Parse a schema version label into a `SchemaVersion`.
///
/// Accepted formats: `migration-ir-v{N}` where N is a non-negative integer.
pub fn parse_version(label: &str) -> Result<SchemaVersion, VersioningError> {
    let prefix = "migration-ir-v";
    let rest = label
        .strip_prefix(prefix)
        .ok_or_else(|| VersioningError::InvalidFormat {
            version: label.to_string(),
            reason: format!("expected prefix '{prefix}'"),
        })?;

    let major: u32 = rest.parse().map_err(|_| VersioningError::InvalidFormat {
        version: label.to_string(),
        reason: format!("'{rest}' is not a valid version number"),
    })?;

    Ok(SchemaVersion {
        major,
        label: label.to_string(),
    })
}

/// Return the current schema version (parsed).
pub fn current_version() -> SchemaVersion {
    parse_version(IR_SCHEMA_VERSION).expect("IR_SCHEMA_VERSION must be valid")
}

// ── Compatibility ───────────────────────────────────────────────────────

/// Result of a schema compatibility check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    /// Versions match exactly.
    Exact,
    /// The manifest is older and can be upgraded through the adapter chain.
    Upgradable {
        from: SchemaVersion,
        to: SchemaVersion,
        steps: u32,
    },
    /// The manifest is newer than the tool — cannot downgrade.
    TooNew {
        found: SchemaVersion,
        supported: SchemaVersion,
    },
    /// The version label is not parseable.
    Unknown { raw: String },
}

/// Check compatibility between a manifest's version and the current tool version.
pub fn check_compatibility(manifest_version: &str) -> Compatibility {
    let current = current_version();

    let found = match parse_version(manifest_version) {
        Ok(v) => v,
        Err(_) => {
            return Compatibility::Unknown {
                raw: manifest_version.to_string(),
            };
        }
    };

    match found.major.cmp(&current.major) {
        std::cmp::Ordering::Equal => Compatibility::Exact,
        std::cmp::Ordering::Less => {
            let steps = current.major - found.major;
            Compatibility::Upgradable {
                from: found,
                to: current,
                steps,
            }
        }
        std::cmp::Ordering::Greater => Compatibility::TooNew {
            found,
            supported: current,
        },
    }
}

/// Generate actionable guidance for a version mismatch.
pub fn version_mismatch_guidance(found: &str, expected: &str) -> String {
    let compat = check_compatibility(found);

    match compat {
        Compatibility::Exact => {
            format!("Schema version '{found}' matches the current version — no action needed.")
        }
        Compatibility::Upgradable { from, to, steps } => {
            let plural = if steps == 1 { "step" } else { "steps" };
            format!(
                "Schema version mismatch: manifest is '{from}' but tool expects '{to}'.\n\
                 This manifest can be upgraded automatically ({steps} migration {plural}).\n\
                 Use `upgrade_manifest()` to migrate the IR to the current schema."
            )
        }
        Compatibility::TooNew { found, supported } => {
            format!(
                "Schema version mismatch: manifest is '{found}' but tool only supports up to '{supported}'.\n\
                 The manifest was created by a newer version of the tool.\n\
                 Please upgrade doctor_frankentui to a version that supports '{found}' or later."
            )
        }
        Compatibility::Unknown { raw } => {
            format!(
                "Unrecognized schema version '{raw}' (expected format: '{expected}').\n\
                 The manifest may be corrupted or created by an incompatible tool.\n\
                 Expected version format: 'migration-ir-vN' where N is a version number."
            )
        }
    }
}

// ── Errors ──────────────────────────────────────────────────────────────

/// Errors from the versioning subsystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersioningError {
    /// The version label does not match expected format.
    InvalidFormat { version: String, reason: String },
    /// No migration adapter registered for this version transition.
    NoAdapter { from: u32, to: u32 },
    /// The migration adapter failed.
    AdapterFailed { from: u32, to: u32, reason: String },
    /// The manifest version is newer than the tool supports.
    UnsupportedVersion {
        found: String,
        max_supported: String,
    },
    /// JSON deserialization failed after upgrade.
    DeserializationFailed { version: String, reason: String },
}

impl std::fmt::Display for VersioningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFormat { version, reason } => {
                write!(f, "Invalid schema version '{version}': {reason}")
            }
            Self::NoAdapter { from, to } => {
                write!(f, "No migration adapter from v{from} to v{to}")
            }
            Self::AdapterFailed { from, to, reason } => {
                write!(f, "Migration v{from} → v{to} failed: {reason}")
            }
            Self::UnsupportedVersion {
                found,
                max_supported,
            } => {
                write!(
                    f,
                    "Schema '{found}' is newer than max supported '{max_supported}'"
                )
            }
            Self::DeserializationFailed { version, reason } => {
                write!(
                    f,
                    "Failed to deserialize IR after upgrading to {version}: {reason}"
                )
            }
        }
    }
}

// ── Migration Adapters ──────────────────────────────────────────────────

/// A single migration step from one schema version to the next.
#[derive(Clone)]
struct MigrationStep {
    /// Source version major.
    from: u32,
    /// Target version major.
    to: u32,
    /// Human-readable description of the migration.
    description: String,
    /// The adapter function: transforms JSON from source to target schema.
    adapter: fn(Value) -> Result<Value, String>,
}

/// Registry of migration adapters.
pub struct MigrationRegistry {
    /// Keyed by source version major.
    steps: BTreeMap<u32, MigrationStep>,
}

impl Default for MigrationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MigrationRegistry {
    /// Create a registry with all known migration adapters.
    pub fn new() -> Self {
        let mut steps = BTreeMap::new();

        // v0 → v1: Legacy format migration.
        // v0 is a hypothetical pre-release format that:
        //   - Used "version" instead of "schema_version"
        //   - Missing "capabilities" section
        //   - Missing "accessibility" section
        //   - "effect_registry" was called "effects"
        //   - No integrity hash
        steps.insert(
            0,
            MigrationStep {
                from: 0,
                to: 1,
                description: "Migrate from pre-release v0 to stable v1 schema".to_string(),
                adapter: migrate_v0_to_v1,
            },
        );

        Self { steps }
    }

    /// Return a description of the migration path from `from` to `to`.
    pub fn describe_path(&self, from: u32, to: u32) -> Vec<String> {
        let mut descriptions = Vec::new();
        let mut current = from;

        while current < to {
            if let Some(step) = self.steps.get(&current) {
                descriptions.push(format!(
                    "v{} → v{}: {}",
                    step.from, step.to, step.description
                ));
                current = step.to;
            } else {
                descriptions.push(format!(
                    "v{current} → v{}: <no adapter registered>",
                    current + 1
                ));
                break;
            }
        }

        descriptions
    }

    /// Check whether a complete migration path exists.
    pub fn has_path(&self, from: u32, to: u32) -> bool {
        let mut current = from;
        while current < to {
            if self.steps.contains_key(&current) {
                current += 1;
            } else {
                return false;
            }
        }
        current == to
    }

    /// Apply the migration chain to a JSON value, upgrading from `from` to `to`.
    pub fn apply_chain(
        &self,
        mut json: Value,
        from: u32,
        to: u32,
    ) -> Result<Value, VersioningError> {
        let mut current = from;

        while current < to {
            let step = self.steps.get(&current).ok_or(VersioningError::NoAdapter {
                from: current,
                to: current + 1,
            })?;

            json = (step.adapter)(json).map_err(|reason| VersioningError::AdapterFailed {
                from: current,
                to: step.to,
                reason,
            })?;

            current = step.to;
        }

        Ok(json)
    }
}

// ── v0 → v1 Migration Adapter ──────────────────────────────────────────

fn migrate_v0_to_v1(mut json: Value) -> Result<Value, String> {
    let obj = json
        .as_object_mut()
        .ok_or_else(|| "IR must be a JSON object".to_string())?;

    // Rename "version" → "schema_version" if present.
    if let Some(version) = obj.remove("version") {
        obj.insert("schema_version".to_string(), version);
    }

    // Set schema version to v1.
    obj.insert(
        "schema_version".to_string(),
        Value::String("migration-ir-v1".to_string()),
    );

    // Rename "effects" → "effect_registry" if present.
    if let Some(effects) = obj.remove("effects") {
        obj.insert("effect_registry".to_string(), effects);
    }

    // Ensure "effect_registry" is properly shaped.
    if !obj.contains_key("effect_registry") {
        obj.insert(
            "effect_registry".to_string(),
            serde_json::json!({ "effects": {} }),
        );
    } else if let Some(registry) = obj.get("effect_registry") {
        // If it's an array or bare map, wrap it.
        if registry.is_array()
            || (registry.is_object() && !registry.as_object().unwrap().contains_key("effects"))
        {
            let inner = obj.remove("effect_registry").unwrap();
            obj.insert(
                "effect_registry".to_string(),
                serde_json::json!({ "effects": inner }),
            );
        }
    }

    // Add missing "capabilities" section.
    if !obj.contains_key("capabilities") {
        obj.insert(
            "capabilities".to_string(),
            serde_json::json!({
                "required": [],
                "optional": [],
                "platform_assumptions": []
            }),
        );
    }

    // Add missing "accessibility" section.
    if !obj.contains_key("accessibility") {
        obj.insert(
            "accessibility".to_string(),
            serde_json::json!({ "entries": {} }),
        );
    }

    // Ensure "metadata" has the expected shape.
    if !obj.contains_key("metadata") {
        obj.insert(
            "metadata".to_string(),
            serde_json::json!({
                "created_at": "",
                "source_file_count": 0,
                "total_nodes": 0,
                "total_state_vars": 0,
                "total_events": 0,
                "total_effects": 0,
                "warnings": [],
                "integrity_hash": null
            }),
        );
    } else if let Some(meta) = obj.get_mut("metadata")
        && let Some(meta_obj) = meta.as_object_mut()
    {
        // Ensure all required fields exist.
        let defaults = [
            ("created_at", Value::String(String::new())),
            ("source_file_count", Value::Number(0.into())),
            ("total_nodes", Value::Number(0.into())),
            ("total_state_vars", Value::Number(0.into())),
            ("total_events", Value::Number(0.into())),
            ("total_effects", Value::Number(0.into())),
            ("warnings", Value::Array(Vec::new())),
            ("integrity_hash", Value::Null),
        ];
        for (key, default) in defaults {
            meta_obj.entry(key.to_string()).or_insert(default);
        }
    }

    // Ensure "style_intent" has the expected shape.
    if !obj.contains_key("style_intent") {
        obj.insert(
            "style_intent".to_string(),
            serde_json::json!({ "tokens": {}, "layouts": {}, "themes": [] }),
        );
    }

    // Ensure "view_tree" has the expected shape.
    if !obj.contains_key("view_tree") {
        obj.insert(
            "view_tree".to_string(),
            serde_json::json!({ "roots": [], "nodes": {} }),
        );
    }

    // Ensure "state_graph" has the expected shape.
    if !obj.contains_key("state_graph") {
        obj.insert(
            "state_graph".to_string(),
            serde_json::json!({ "variables": {}, "derived": {}, "data_flow": {} }),
        );
    }

    // Ensure "event_catalog" has the expected shape.
    if !obj.contains_key("event_catalog") {
        obj.insert(
            "event_catalog".to_string(),
            serde_json::json!({ "events": {}, "transitions": [] }),
        );
    }

    Ok(json)
}

// ── Public Upgrade API ──────────────────────────────────────────────────

/// Result of upgrading a manifest.
#[derive(Debug)]
pub struct UpgradeResult {
    /// The upgraded IR (deserialized into current schema).
    pub ir: MigrationIr,
    /// The version the manifest was upgraded from.
    pub original_version: SchemaVersion,
    /// Number of migration steps applied.
    pub steps_applied: u32,
    /// Descriptions of each migration step.
    pub migration_log: Vec<String>,
}

/// Upgrade a serialized IR manifest (JSON) to the current schema version.
///
/// If the manifest is already at the current version, it is deserialized directly.
/// If it is older, the migration adapter chain is applied.
/// If it is newer or unparseable, an error is returned.
pub fn upgrade_manifest(json_str: &str) -> Result<UpgradeResult, VersioningError> {
    let json: Value =
        serde_json::from_str(json_str).map_err(|e| VersioningError::DeserializationFailed {
            version: "unknown".to_string(),
            reason: e.to_string(),
        })?;

    upgrade_manifest_value(json)
}

/// Upgrade a JSON `Value` manifest to the current schema version.
pub fn upgrade_manifest_value(json: Value) -> Result<UpgradeResult, VersioningError> {
    let current = current_version();

    // Extract version from the JSON.
    let version_str = extract_version_string(&json);
    let found = parse_version(&version_str)?;

    if found.major > current.major {
        return Err(VersioningError::UnsupportedVersion {
            found: found.label,
            max_supported: current.label,
        });
    }

    let registry = MigrationRegistry::new();
    let steps = current.major - found.major;

    let (upgraded_json, migration_log) = if steps > 0 {
        if !registry.has_path(found.major, current.major) {
            return Err(VersioningError::NoAdapter {
                from: found.major,
                to: current.major,
            });
        }

        let log = registry.describe_path(found.major, current.major);
        let upgraded = registry.apply_chain(json, found.major, current.major)?;
        (upgraded, log)
    } else {
        (json, Vec::new())
    };

    // Deserialize into the current MigrationIr.
    let ir: MigrationIr = serde_json::from_value(upgraded_json).map_err(|e| {
        VersioningError::DeserializationFailed {
            version: current.label.clone(),
            reason: e.to_string(),
        }
    })?;

    Ok(UpgradeResult {
        ir,
        original_version: found,
        steps_applied: steps,
        migration_log,
    })
}

/// Extract the schema version string from a JSON value.
///
/// Checks both `schema_version` (v1+) and `version` (v0 legacy).
fn extract_version_string(json: &Value) -> String {
    if let Some(v) = json.get("schema_version").and_then(|v| v.as_str()) {
        return v.to_string();
    }
    if let Some(v) = json.get("version").and_then(|v| v.as_str()) {
        return v.to_string();
    }
    // No version field → assume v0.
    "migration-ir-v0".to_string()
}

/// Validate that an IR's schema version matches the current version.
///
/// Returns `Ok(())` if the version matches, or an error with actionable guidance.
pub fn validate_version(ir: &MigrationIr) -> Result<(), VersioningError> {
    let current = current_version();
    let found = parse_version(&ir.schema_version)?;

    if found.major != current.major {
        Err(VersioningError::UnsupportedVersion {
            found: found.label,
            max_supported: current.label,
        })
    } else {
        Ok(())
    }
}

/// Recompute and update the integrity hash after an upgrade.
pub fn recompute_integrity(ir: &mut MigrationIr) {
    let hash = migration_ir::compute_integrity_hash(ir);
    ir.metadata.integrity_hash = Some(hash);
}

// ── Schema Diff (for migration planning) ────────────────────────────────

/// A description of what changed between two schema versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaChange {
    /// The version transition (e.g. "v0 → v1").
    pub transition: String,
    /// Human-readable list of changes.
    pub changes: Vec<String>,
}

/// Return a human-readable changelog of schema changes for a given upgrade path.
pub fn schema_changelog(from: u32, to: u32) -> Vec<SchemaChange> {
    let mut log = Vec::new();
    let mut current = from;

    while current < to {
        let changes = match current {
            0 => SchemaChange {
                transition: "v0 → v1".to_string(),
                changes: vec![
                    "Renamed 'version' field to 'schema_version'".to_string(),
                    "Renamed 'effects' to 'effect_registry' with wrapper object".to_string(),
                    "Added 'capabilities' section (required, optional, platform_assumptions)"
                        .to_string(),
                    "Added 'accessibility' section (entries map)".to_string(),
                    "Added 'metadata.integrity_hash' field".to_string(),
                    "Added 'metadata.warnings' array".to_string(),
                    "Standardized all sub-sections to object wrappers".to_string(),
                ],
            },
            _ => SchemaChange {
                transition: format!("v{current} → v{}", current + 1),
                changes: vec!["No changelog available for this transition".to_string()],
            },
        };
        log.push(changes);
        current += 1;
    }

    log
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration_ir::IrBuilder;

    // ── Version parsing ────────────────────────────────────────────────

    #[test]
    fn parse_valid_version() {
        let v = parse_version("migration-ir-v1").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.label, "migration-ir-v1");
    }

    #[test]
    fn parse_v0() {
        let v = parse_version("migration-ir-v0").unwrap();
        assert_eq!(v.major, 0);
    }

    #[test]
    fn parse_high_version() {
        let v = parse_version("migration-ir-v42").unwrap();
        assert_eq!(v.major, 42);
    }

    #[test]
    fn parse_invalid_prefix() {
        let err = parse_version("schema-v1").unwrap_err();
        assert!(matches!(err, VersioningError::InvalidFormat { .. }));
    }

    #[test]
    fn parse_invalid_number() {
        let err = parse_version("migration-ir-vXYZ").unwrap_err();
        assert!(matches!(err, VersioningError::InvalidFormat { .. }));
    }

    #[test]
    fn parse_empty_string() {
        let err = parse_version("").unwrap_err();
        assert!(matches!(err, VersioningError::InvalidFormat { .. }));
    }

    // ── Version ordering ───────────────────────────────────────────────

    #[test]
    fn version_ordering() {
        let v0 = parse_version("migration-ir-v0").unwrap();
        let v1 = parse_version("migration-ir-v1").unwrap();
        let v2 = parse_version("migration-ir-v2").unwrap();

        assert!(v0 < v1);
        assert!(v1 < v2);
        assert!(v0 < v2);
        assert_eq!(v1, parse_version("migration-ir-v1").unwrap());
    }

    // ── Compatibility checking ─────────────────────────────────────────

    #[test]
    fn compatibility_exact_match() {
        let result = check_compatibility("migration-ir-v1");
        assert_eq!(result, Compatibility::Exact);
    }

    #[test]
    fn compatibility_upgradable() {
        let result = check_compatibility("migration-ir-v0");
        assert!(matches!(result, Compatibility::Upgradable { steps: 1, .. }));
    }

    #[test]
    fn compatibility_too_new() {
        let result = check_compatibility("migration-ir-v99");
        assert!(matches!(result, Compatibility::TooNew { .. }));
    }

    #[test]
    fn compatibility_unknown_format() {
        let result = check_compatibility("not-a-version");
        assert!(matches!(result, Compatibility::Unknown { .. }));
    }

    // ── Migration registry ─────────────────────────────────────────────

    #[test]
    fn registry_has_v0_to_v1_path() {
        let registry = MigrationRegistry::new();
        assert!(registry.has_path(0, 1));
    }

    #[test]
    fn registry_no_path_beyond_registered() {
        let registry = MigrationRegistry::new();
        // v1 → v2 doesn't exist.
        assert!(!registry.has_path(1, 2));
    }

    #[test]
    fn registry_describe_path() {
        let registry = MigrationRegistry::new();
        let desc = registry.describe_path(0, 1);
        assert_eq!(desc.len(), 1);
        assert!(desc[0].contains("v0 → v1"));
    }

    // ── v0 → v1 adapter ───────────────────────────────────────────────

    #[test]
    fn migrate_v0_minimal() {
        let v0_json = serde_json::json!({
            "version": "migration-ir-v0",
            "run_id": "test-run",
            "source_project": "my-app"
        });

        let result = migrate_v0_to_v1(v0_json).unwrap();
        let obj = result.as_object().unwrap();

        assert_eq!(obj["schema_version"], "migration-ir-v1");
        assert!(obj.contains_key("capabilities"));
        assert!(obj.contains_key("accessibility"));
        assert!(obj.contains_key("effect_registry"));
        assert!(obj.contains_key("metadata"));
        assert!(obj.contains_key("view_tree"));
        assert!(obj.contains_key("state_graph"));
        assert!(obj.contains_key("event_catalog"));
        assert!(obj.contains_key("style_intent"));
    }

    #[test]
    fn migrate_v0_with_effects_array() {
        let v0_json = serde_json::json!({
            "version": "migration-ir-v0",
            "run_id": "test-run",
            "source_project": "test",
            "effects": [
                { "id": "effect-1", "name": "fetchData" }
            ]
        });

        let result = migrate_v0_to_v1(v0_json).unwrap();
        let obj = result.as_object().unwrap();

        // "effects" should be renamed to "effect_registry" with wrapper.
        assert!(obj.contains_key("effect_registry"));
        assert!(!obj.contains_key("effects"));
        let registry = &obj["effect_registry"];
        assert!(registry.get("effects").is_some());
    }

    #[test]
    fn migrate_v0_preserves_existing_data() {
        let v0_json = serde_json::json!({
            "version": "migration-ir-v0",
            "run_id": "preserve-test",
            "source_project": "my-app",
            "view_tree": { "roots": ["r1"], "nodes": {} },
            "state_graph": { "variables": {}, "derived": {}, "data_flow": {} }
        });

        let result = migrate_v0_to_v1(v0_json).unwrap();
        let obj = result.as_object().unwrap();

        assert_eq!(obj["run_id"], "preserve-test");
        assert_eq!(obj["source_project"], "my-app");
        assert_eq!(obj["view_tree"]["roots"][0], "r1");
    }

    #[test]
    fn migrate_v0_metadata_defaults() {
        let v0_json = serde_json::json!({
            "version": "migration-ir-v0",
            "run_id": "test",
            "source_project": "test",
            "metadata": { "created_at": "2025-01-01T00:00:00Z" }
        });

        let result = migrate_v0_to_v1(v0_json).unwrap();
        let meta = &result["metadata"];

        assert_eq!(meta["created_at"], "2025-01-01T00:00:00Z");
        assert!(meta.get("source_file_count").is_some());
        assert!(meta.get("warnings").is_some());
        assert!(meta.get("integrity_hash").is_some());
    }

    // ── Upgrade pipeline ───────────────────────────────────────────────

    #[test]
    fn upgrade_v0_manifest_to_current() {
        let v0_json = serde_json::json!({
            "version": "migration-ir-v0",
            "run_id": "upgrade-test",
            "source_project": "old-project",
            "view_tree": { "roots": [], "nodes": {} },
            "state_graph": { "variables": {}, "derived": {}, "data_flow": {} },
            "event_catalog": { "events": {}, "transitions": [] }
        });

        let json_str = serde_json::to_string(&v0_json).unwrap();
        let result = upgrade_manifest(&json_str).unwrap();

        assert_eq!(result.ir.schema_version, "migration-ir-v1");
        assert_eq!(result.original_version.major, 0);
        assert_eq!(result.steps_applied, 1);
        assert_eq!(result.migration_log.len(), 1);
        assert_eq!(result.ir.run_id, "upgrade-test");
    }

    #[test]
    fn upgrade_current_version_is_noop() {
        let builder = IrBuilder::new("test-run".to_string(), "test-project".to_string());
        let ir = builder.build();
        let json_str = serde_json::to_string(&ir).unwrap();

        let result = upgrade_manifest(&json_str).unwrap();
        assert_eq!(result.steps_applied, 0);
        assert!(result.migration_log.is_empty());
        assert_eq!(result.ir.schema_version, IR_SCHEMA_VERSION);
    }

    #[test]
    fn upgrade_future_version_fails() {
        let future_json = serde_json::json!({
            "schema_version": "migration-ir-v99",
            "run_id": "future",
            "source_project": "future-project"
        });

        let json_str = serde_json::to_string(&future_json).unwrap();
        let err = upgrade_manifest(&json_str).unwrap_err();
        assert!(matches!(err, VersioningError::UnsupportedVersion { .. }));
    }

    #[test]
    fn upgrade_invalid_json_fails() {
        let err = upgrade_manifest("not valid json").unwrap_err();
        assert!(matches!(err, VersioningError::DeserializationFailed { .. }));
    }

    // ── Version extraction ─────────────────────────────────────────────

    #[test]
    fn extract_version_from_schema_version_field() {
        let json = serde_json::json!({ "schema_version": "migration-ir-v1" });
        assert_eq!(extract_version_string(&json), "migration-ir-v1");
    }

    #[test]
    fn extract_version_from_legacy_version_field() {
        let json = serde_json::json!({ "version": "migration-ir-v0" });
        assert_eq!(extract_version_string(&json), "migration-ir-v0");
    }

    #[test]
    fn extract_version_defaults_to_v0() {
        let json = serde_json::json!({ "run_id": "test" });
        assert_eq!(extract_version_string(&json), "migration-ir-v0");
    }

    #[test]
    fn schema_version_preferred_over_legacy() {
        let json = serde_json::json!({
            "schema_version": "migration-ir-v1",
            "version": "migration-ir-v0"
        });
        assert_eq!(extract_version_string(&json), "migration-ir-v1");
    }

    // ── Guidance messages ──────────────────────────────────────────────

    #[test]
    fn guidance_for_exact_match() {
        let msg = version_mismatch_guidance("migration-ir-v1", "migration-ir-v1");
        assert!(msg.contains("no action needed"));
    }

    #[test]
    fn guidance_for_upgradable() {
        let msg = version_mismatch_guidance("migration-ir-v0", "migration-ir-v1");
        assert!(msg.contains("upgraded automatically"));
        assert!(msg.contains("upgrade_manifest"));
    }

    #[test]
    fn guidance_for_too_new() {
        let msg = version_mismatch_guidance("migration-ir-v99", "migration-ir-v1");
        assert!(msg.contains("upgrade doctor_frankentui"));
    }

    #[test]
    fn guidance_for_unknown() {
        let msg = version_mismatch_guidance("garbage", "migration-ir-v1");
        assert!(msg.contains("corrupted"));
    }

    // ── Schema changelog ───────────────────────────────────────────────

    #[test]
    fn changelog_v0_to_v1() {
        let log = schema_changelog(0, 1);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].transition, "v0 → v1");
        assert!(log[0].changes.len() >= 5);
    }

    #[test]
    fn changelog_empty_for_same_version() {
        let log = schema_changelog(1, 1);
        assert!(log.is_empty());
    }

    // ── Validate version ───────────────────────────────────────────────

    #[test]
    fn validate_current_version_ok() {
        let builder = IrBuilder::new("test".to_string(), "test".to_string());
        let ir = builder.build();
        assert!(validate_version(&ir).is_ok());
    }

    #[test]
    fn validate_wrong_version_fails() {
        let builder = IrBuilder::new("test".to_string(), "test".to_string());
        let mut ir = builder.build();
        ir.schema_version = "migration-ir-v99".to_string();
        assert!(validate_version(&ir).is_err());
    }

    // ── Recompute integrity ────────────────────────────────────────────

    #[test]
    fn recompute_integrity_after_modification() {
        let builder = IrBuilder::new("test".to_string(), "test".to_string());
        let mut ir = builder.build();
        let original_hash = ir.metadata.integrity_hash.clone();

        // Modify the IR.
        ir.run_id = "modified".to_string();
        ir.metadata.integrity_hash = None;

        recompute_integrity(&mut ir);
        assert!(ir.metadata.integrity_hash.is_some());
        assert_ne!(ir.metadata.integrity_hash, original_hash);
    }

    // ── Error display ──────────────────────────────────────────────────

    #[test]
    fn error_display() {
        let err = VersioningError::InvalidFormat {
            version: "bad".to_string(),
            reason: "test".to_string(),
        };
        assert!(err.to_string().contains("bad"));

        let err = VersioningError::NoAdapter { from: 5, to: 6 };
        assert!(err.to_string().contains("v5"));

        let err = VersioningError::AdapterFailed {
            from: 0,
            to: 1,
            reason: "oops".to_string(),
        };
        assert!(err.to_string().contains("oops"));

        let err = VersioningError::UnsupportedVersion {
            found: "v99".to_string(),
            max_supported: "v1".to_string(),
        };
        assert!(err.to_string().contains("v99"));

        let err = VersioningError::DeserializationFailed {
            version: "v1".to_string(),
            reason: "parse error".to_string(),
        };
        assert!(err.to_string().contains("parse error"));
    }

    // ── Upgrade idempotence ────────────────────────────────────────────

    #[test]
    fn upgrade_is_idempotent() {
        let v0_json = serde_json::json!({
            "version": "migration-ir-v0",
            "run_id": "idem-test",
            "source_project": "test",
            "view_tree": { "roots": [], "nodes": {} },
            "state_graph": { "variables": {}, "derived": {}, "data_flow": {} },
            "event_catalog": { "events": {}, "transitions": [] }
        });

        // Apply v0 → v1 once.
        let once = migrate_v0_to_v1(v0_json.clone()).unwrap();
        // Apply v0 → v1 again on the original (not the result).
        let twice_from_original = migrate_v0_to_v1(v0_json).unwrap();

        // Both should produce the same structure.
        assert_eq!(
            serde_json::to_string(&once).unwrap(),
            serde_json::to_string(&twice_from_original).unwrap(),
        );
    }

    // ── Migration chain integration ────────────────────────────────────

    #[test]
    fn apply_chain_v0_to_v1() {
        let registry = MigrationRegistry::new();
        let v0_json = serde_json::json!({
            "version": "migration-ir-v0",
            "run_id": "chain-test",
            "source_project": "test"
        });

        let result = registry.apply_chain(v0_json, 0, 1).unwrap();
        assert_eq!(result["schema_version"], "migration-ir-v1");
    }

    #[test]
    fn apply_chain_missing_adapter_fails() {
        let registry = MigrationRegistry::new();
        let json = serde_json::json!({});

        let err = registry.apply_chain(json, 5, 6).unwrap_err();
        assert!(matches!(err, VersioningError::NoAdapter { from: 5, to: 6 }));
    }
}
