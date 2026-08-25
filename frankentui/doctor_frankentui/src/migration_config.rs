// SPDX-License-Identifier: Apache-2.0
//! Profile-driven configuration system with override DSL.
//!
//! Provides three migration workflow profiles (`strict`, `parity`,
//! `aggressive`) plus a fine-grained override DSL for policy and threshold
//! controls. Configuration is resolved through a deterministic precedence
//! chain:
//!
//! 1. **Defaults** — hard-coded sensible values
//! 2. **Profile** — workflow-mode presets (strict / parity / aggressive)
//! 3. **Config file** — TOML/JSON file at `--config-file` path
//! 4. **CLI overrides** — highest precedence, individual flags
//!
//! The effective configuration snapshot is serializable and emitted in every
//! run manifest for auditability.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{DoctorError, Result};

// ── Migration Profiles ───────────────────────────────────────────────

/// Built-in migration workflow profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationProfile {
    /// Minimal changes, highest confidence gates, safest output.
    Strict,
    /// Focus on semantic equivalence with balanced risk tolerance.
    Parity,
    /// Quality and performance improvement beyond parity.
    Aggressive,
}

impl MigrationProfile {
    /// Parse a profile name string (case-insensitive).
    pub fn from_name(name: &str) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "strict" => Ok(Self::Strict),
            "parity" => Ok(Self::Parity),
            "aggressive" => Ok(Self::Aggressive),
            _ => Err(DoctorError::InvalidArgument {
                message: format!(
                    "unknown migration profile '{name}'; valid profiles: strict, parity, aggressive"
                ),
            }),
        }
    }

    /// Profile name as a string.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Parity => "parity",
            Self::Aggressive => "aggressive",
        }
    }

    /// All available profile names.
    #[must_use]
    pub fn all_names() -> &'static [&'static str] {
        &["strict", "parity", "aggressive"]
    }
}

// ── Effective Configuration ──────────────────────────────────────────

/// The resolved, effective migration configuration.
///
/// This is the single source of truth for all pipeline stages. It is
/// computed once via [`ConfigResolver`] and then passed immutably to
/// every component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    /// Which profile was used as the base.
    pub profile: MigrationProfile,
    /// Planner settings.
    pub planner: PlannerSettings,
    /// Confidence model settings.
    pub confidence: ConfidenceSettings,
    /// Code emission settings.
    pub emission: EmissionSettings,
    /// Optimization settings.
    pub optimization: OptimizationSettings,
    /// Synthesis settings.
    pub synthesis: SynthesisSettings,
    /// Overrides applied (for audit trail).
    pub overrides_applied: Vec<ConfigOverride>,
    /// Config file path that was loaded (if any).
    pub config_file_path: Option<String>,
}

/// Planner-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerSettings {
    /// Deterministic seed for tie-breaking.
    pub seed: u64,
    /// Minimum confidence threshold for auto-approval.
    pub min_confidence_threshold: f64,
    /// Whether to use intent-inference signals.
    pub use_intent_signals: bool,
    /// Whether to use effect-model signals.
    pub use_effect_signals: bool,
}

/// Confidence model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceSettings {
    /// Auto-approve threshold (above this = auto).
    pub auto_approve_threshold: f64,
    /// Human-review lower bound (below this = reject).
    pub human_review_lower: f64,
    /// Credible interval width for posteriors.
    pub credible_interval_width: f64,
    /// Minimum samples before calibration kicks in.
    pub min_calibration_samples: usize,
}

/// Code emission configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionSettings {
    /// Generated crate name.
    pub crate_name: String,
    /// Whether to emit migration manifest alongside code.
    pub emit_manifest: bool,
    /// Whether to emit provenance links in generated code.
    pub emit_provenance: bool,
}

/// Optimization pass configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationSettings {
    /// Whether to run dead-branch elimination.
    pub dead_branch_elimination: bool,
    /// Whether to run style constant folding.
    pub style_constant_folding: bool,
    /// Whether to run helper extraction.
    pub helper_extraction: bool,
    /// Whether to run import deduplication.
    pub import_deduplication: bool,
    /// Whether to run e-graph saturation optimizer.
    pub egraph_optimization: bool,
    /// E-graph node budget.
    pub egraph_max_nodes: usize,
}

/// CEGIS synthesis configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisSettings {
    /// Whether synthesis is enabled for unmapped holes.
    pub enabled: bool,
    /// Maximum holes to attempt per run.
    pub max_holes: usize,
    /// Maximum CEGIS iterations per hole.
    pub max_iterations_per_hole: usize,
}

/// A single configuration override (for audit trail).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOverride {
    /// Dotted key path (e.g. "planner.min_confidence_threshold").
    pub key: String,
    /// Value that was set.
    pub value: String,
    /// Source of the override.
    pub source: OverrideSource,
}

/// Where a config override came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverrideSource {
    /// From the profile preset.
    Profile,
    /// From a config file.
    ConfigFile,
    /// From a CLI argument.
    Cli,
}

// ── Default configurations per profile ───────────────────────────────

impl MigrationConfig {
    /// Build default configuration for a given profile.
    #[must_use]
    pub fn from_profile(profile: MigrationProfile) -> Self {
        match profile {
            MigrationProfile::Strict => Self::strict_defaults(),
            MigrationProfile::Parity => Self::parity_defaults(),
            MigrationProfile::Aggressive => Self::aggressive_defaults(),
        }
    }

    fn strict_defaults() -> Self {
        Self {
            profile: MigrationProfile::Strict,
            planner: PlannerSettings {
                seed: 0xF7A4_D12B,
                min_confidence_threshold: 0.7,
                use_intent_signals: false,
                use_effect_signals: true,
            },
            confidence: ConfidenceSettings {
                auto_approve_threshold: 0.9,
                human_review_lower: 0.6,
                credible_interval_width: 0.95,
                min_calibration_samples: 50,
            },
            emission: EmissionSettings {
                crate_name: "migrated-app".into(),
                emit_manifest: true,
                emit_provenance: true,
            },
            optimization: OptimizationSettings {
                dead_branch_elimination: true,
                style_constant_folding: true,
                helper_extraction: false,
                import_deduplication: true,
                egraph_optimization: false,
                egraph_max_nodes: 5_000,
            },
            synthesis: SynthesisSettings {
                enabled: false,
                max_holes: 10,
                max_iterations_per_hole: 5,
            },
            overrides_applied: Vec::new(),
            config_file_path: None,
        }
    }

    fn parity_defaults() -> Self {
        Self {
            profile: MigrationProfile::Parity,
            planner: PlannerSettings {
                seed: 0xF7A4_D12B,
                min_confidence_threshold: 0.3,
                use_intent_signals: true,
                use_effect_signals: true,
            },
            confidence: ConfidenceSettings {
                auto_approve_threshold: 0.7,
                human_review_lower: 0.4,
                credible_interval_width: 0.90,
                min_calibration_samples: 20,
            },
            emission: EmissionSettings {
                crate_name: "migrated-app".into(),
                emit_manifest: true,
                emit_provenance: true,
            },
            optimization: OptimizationSettings {
                dead_branch_elimination: true,
                style_constant_folding: true,
                helper_extraction: true,
                import_deduplication: true,
                egraph_optimization: true,
                egraph_max_nodes: 10_000,
            },
            synthesis: SynthesisSettings {
                enabled: true,
                max_holes: 50,
                max_iterations_per_hole: 10,
            },
            overrides_applied: Vec::new(),
            config_file_path: None,
        }
    }

    fn aggressive_defaults() -> Self {
        Self {
            profile: MigrationProfile::Aggressive,
            planner: PlannerSettings {
                seed: 0xF7A4_D12B,
                min_confidence_threshold: 0.15,
                use_intent_signals: true,
                use_effect_signals: true,
            },
            confidence: ConfidenceSettings {
                auto_approve_threshold: 0.5,
                human_review_lower: 0.2,
                credible_interval_width: 0.80,
                min_calibration_samples: 10,
            },
            emission: EmissionSettings {
                crate_name: "migrated-app".into(),
                emit_manifest: true,
                emit_provenance: false,
            },
            optimization: OptimizationSettings {
                dead_branch_elimination: true,
                style_constant_folding: true,
                helper_extraction: true,
                import_deduplication: true,
                egraph_optimization: true,
                egraph_max_nodes: 50_000,
            },
            synthesis: SynthesisSettings {
                enabled: true,
                max_holes: 200,
                max_iterations_per_hole: 20,
            },
            overrides_applied: Vec::new(),
            config_file_path: None,
        }
    }
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self::from_profile(MigrationProfile::Parity)
    }
}

// ── Config Resolver ──────────────────────────────────────────────────

/// Resolves configuration through the precedence chain:
/// defaults → profile → config file → CLI overrides.
pub struct ConfigResolver {
    config: MigrationConfig,
}

impl ConfigResolver {
    /// Start with a profile's defaults.
    #[must_use]
    pub fn from_profile(profile: MigrationProfile) -> Self {
        Self {
            config: MigrationConfig::from_profile(profile),
        }
    }

    /// Apply overrides from a config file (JSON format).
    ///
    /// Only keys present in the file are overridden. Unknown keys trigger
    /// a validation error.
    pub fn apply_config_file(&mut self, path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(path).map_err(DoctorError::Io)?;
        let overrides: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(&content).map_err(DoctorError::Json)?;

        self.config.config_file_path = Some(path.display().to_string());

        for (key, value) in &overrides {
            self.apply_override(key, value, OverrideSource::ConfigFile)?;
        }

        Ok(())
    }

    /// Apply CLI-level overrides from a key=value list.
    ///
    /// Format: `"planner.seed=0xDEAD"`, `"synthesis.enabled=true"`, etc.
    pub fn apply_cli_overrides(&mut self, overrides: &[String]) -> Result<()> {
        for entry in overrides {
            let (key, value_str) =
                entry
                    .split_once('=')
                    .ok_or_else(|| DoctorError::InvalidArgument {
                        message: format!("invalid override format '{entry}'; expected key=value"),
                    })?;

            let value: serde_json::Value = parse_value_str(value_str);
            self.apply_override(key.trim(), &value, OverrideSource::Cli)?;
        }
        Ok(())
    }

    /// Finalize and return the resolved configuration.
    #[must_use]
    pub fn build(self) -> MigrationConfig {
        self.config
    }

    /// Validate the current configuration (called before build).
    pub fn validate(&self) -> Result<()> {
        let c = &self.config;

        if c.planner.min_confidence_threshold < 0.0 || c.planner.min_confidence_threshold > 1.0 {
            return Err(DoctorError::InvalidArgument {
                message: format!(
                    "planner.min_confidence_threshold must be in [0.0, 1.0], got {}",
                    c.planner.min_confidence_threshold
                ),
            });
        }

        if c.confidence.auto_approve_threshold <= c.confidence.human_review_lower {
            return Err(DoctorError::InvalidArgument {
                message: format!(
                    "confidence.auto_approve_threshold ({}) must be > human_review_lower ({})",
                    c.confidence.auto_approve_threshold, c.confidence.human_review_lower
                ),
            });
        }

        if c.confidence.credible_interval_width <= 0.0
            || c.confidence.credible_interval_width >= 1.0
        {
            return Err(DoctorError::InvalidArgument {
                message: format!(
                    "confidence.credible_interval_width must be in (0.0, 1.0), got {}",
                    c.confidence.credible_interval_width
                ),
            });
        }

        if c.synthesis.enabled && c.synthesis.max_holes == 0 {
            return Err(DoctorError::InvalidArgument {
                message: "synthesis.enabled=true but max_holes=0".into(),
            });
        }

        Ok(())
    }

    fn apply_override(
        &mut self,
        key: &str,
        value: &serde_json::Value,
        source: OverrideSource,
    ) -> Result<()> {
        let value_str = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };

        match key {
            // Planner
            "planner.seed" => {
                self.config.planner.seed = parse_u64_value(value, key)?;
            }
            "planner.min_confidence_threshold" => {
                self.config.planner.min_confidence_threshold = parse_f64_value(value, key)?;
            }
            "planner.use_intent_signals" => {
                self.config.planner.use_intent_signals = parse_bool_value(value, key)?;
            }
            "planner.use_effect_signals" => {
                self.config.planner.use_effect_signals = parse_bool_value(value, key)?;
            }

            // Confidence
            "confidence.auto_approve_threshold" => {
                self.config.confidence.auto_approve_threshold = parse_f64_value(value, key)?;
            }
            "confidence.human_review_lower" => {
                self.config.confidence.human_review_lower = parse_f64_value(value, key)?;
            }
            "confidence.credible_interval_width" => {
                self.config.confidence.credible_interval_width = parse_f64_value(value, key)?;
            }
            "confidence.min_calibration_samples" => {
                self.config.confidence.min_calibration_samples =
                    parse_u64_value(value, key)? as usize;
            }

            // Emission
            "emission.crate_name" => {
                self.config.emission.crate_name = parse_string_value(value, key)?;
            }
            "emission.emit_manifest" => {
                self.config.emission.emit_manifest = parse_bool_value(value, key)?;
            }
            "emission.emit_provenance" => {
                self.config.emission.emit_provenance = parse_bool_value(value, key)?;
            }

            // Optimization
            "optimization.dead_branch_elimination" => {
                self.config.optimization.dead_branch_elimination = parse_bool_value(value, key)?;
            }
            "optimization.style_constant_folding" => {
                self.config.optimization.style_constant_folding = parse_bool_value(value, key)?;
            }
            "optimization.helper_extraction" => {
                self.config.optimization.helper_extraction = parse_bool_value(value, key)?;
            }
            "optimization.import_deduplication" => {
                self.config.optimization.import_deduplication = parse_bool_value(value, key)?;
            }
            "optimization.egraph_optimization" => {
                self.config.optimization.egraph_optimization = parse_bool_value(value, key)?;
            }
            "optimization.egraph_max_nodes" => {
                self.config.optimization.egraph_max_nodes = parse_u64_value(value, key)? as usize;
            }

            // Synthesis
            "synthesis.enabled" => {
                self.config.synthesis.enabled = parse_bool_value(value, key)?;
            }
            "synthesis.max_holes" => {
                self.config.synthesis.max_holes = parse_u64_value(value, key)? as usize;
            }
            "synthesis.max_iterations_per_hole" => {
                self.config.synthesis.max_iterations_per_hole =
                    parse_u64_value(value, key)? as usize;
            }

            unknown => {
                return Err(DoctorError::InvalidArgument {
                    message: format!("unknown config key: '{unknown}'"),
                });
            }
        }

        self.config.overrides_applied.push(ConfigOverride {
            key: key.to_string(),
            value: value_str,
            source,
        });

        Ok(())
    }
}

// ── Schema ───────────────────────────────────────────────────────────

/// All valid configuration keys for schema validation.
pub const VALID_CONFIG_KEYS: &[&str] = &[
    "planner.seed",
    "planner.min_confidence_threshold",
    "planner.use_intent_signals",
    "planner.use_effect_signals",
    "confidence.auto_approve_threshold",
    "confidence.human_review_lower",
    "confidence.credible_interval_width",
    "confidence.min_calibration_samples",
    "emission.crate_name",
    "emission.emit_manifest",
    "emission.emit_provenance",
    "optimization.dead_branch_elimination",
    "optimization.style_constant_folding",
    "optimization.helper_extraction",
    "optimization.import_deduplication",
    "optimization.egraph_optimization",
    "optimization.egraph_max_nodes",
    "synthesis.enabled",
    "synthesis.max_holes",
    "synthesis.max_iterations_per_hole",
];

// ── Value parsers ────────────────────────────────────────────────────

fn parse_value_str(s: &str) -> serde_json::Value {
    let trimmed = s.trim();
    if let Ok(b) = trimmed.parse::<bool>() {
        return serde_json::Value::Bool(b);
    }
    if let Ok(n) = trimmed.parse::<u64>() {
        return serde_json::Value::Number(n.into());
    }
    if (trimmed.starts_with("0x") || trimmed.starts_with("0X"))
        && let Ok(n) = u64::from_str_radix(&trimmed[2..], 16)
    {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(f) = trimmed.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return serde_json::Value::Number(n);
    }
    serde_json::Value::String(trimmed.to_string())
}

fn parse_bool_value(value: &serde_json::Value, key: &str) -> Result<bool> {
    match value {
        serde_json::Value::Bool(b) => Ok(*b),
        serde_json::Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(DoctorError::InvalidArgument {
                message: format!("'{key}': cannot parse '{s}' as bool"),
            }),
        },
        _ => Err(DoctorError::InvalidArgument {
            message: format!("'{key}': expected bool, got {value}"),
        }),
    }
}

fn parse_f64_value(value: &serde_json::Value, key: &str) -> Result<f64> {
    match value {
        serde_json::Value::Number(n) => n.as_f64().ok_or_else(|| DoctorError::InvalidArgument {
            message: format!("'{key}': cannot convert {n} to f64"),
        }),
        serde_json::Value::String(s) => {
            s.parse::<f64>().map_err(|_| DoctorError::InvalidArgument {
                message: format!("'{key}': cannot parse '{s}' as f64"),
            })
        }
        _ => Err(DoctorError::InvalidArgument {
            message: format!("'{key}': expected number, got {value}"),
        }),
    }
}

fn parse_u64_value(value: &serde_json::Value, key: &str) -> Result<u64> {
    match value {
        serde_json::Value::Number(n) => n.as_u64().ok_or_else(|| DoctorError::InvalidArgument {
            message: format!("'{key}': cannot convert {n} to u64"),
        }),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                u64::from_str_radix(&trimmed[2..], 16).map_err(|_| DoctorError::InvalidArgument {
                    message: format!("'{key}': cannot parse '{s}' as hex u64"),
                })
            } else {
                trimmed
                    .parse::<u64>()
                    .map_err(|_| DoctorError::InvalidArgument {
                        message: format!("'{key}': cannot parse '{s}' as u64"),
                    })
            }
        }
        _ => Err(DoctorError::InvalidArgument {
            message: format!("'{key}': expected integer, got {value}"),
        }),
    }
}

fn parse_string_value(value: &serde_json::Value, key: &str) -> Result<String> {
    match value {
        serde_json::Value::String(s) => Ok(s.clone()),
        _ => Err(DoctorError::InvalidArgument {
            message: format!("'{key}': expected string, got {value}"),
        }),
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_from_name_valid() {
        assert_eq!(
            MigrationProfile::from_name("strict").unwrap(),
            MigrationProfile::Strict
        );
        assert_eq!(
            MigrationProfile::from_name("PARITY").unwrap(),
            MigrationProfile::Parity
        );
        assert_eq!(
            MigrationProfile::from_name("Aggressive").unwrap(),
            MigrationProfile::Aggressive
        );
    }

    #[test]
    fn profile_from_name_invalid() {
        assert!(MigrationProfile::from_name("unknown").is_err());
    }

    #[test]
    fn profile_all_names() {
        let names = MigrationProfile::all_names();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"strict"));
        assert!(names.contains(&"parity"));
        assert!(names.contains(&"aggressive"));
    }

    #[test]
    fn strict_profile_has_high_thresholds() {
        let cfg = MigrationConfig::from_profile(MigrationProfile::Strict);
        assert!(cfg.planner.min_confidence_threshold >= 0.5);
        assert!(cfg.confidence.auto_approve_threshold >= 0.8);
        assert!(!cfg.synthesis.enabled);
        assert!(!cfg.optimization.egraph_optimization);
    }

    #[test]
    fn parity_profile_is_balanced() {
        let cfg = MigrationConfig::from_profile(MigrationProfile::Parity);
        assert!(cfg.planner.min_confidence_threshold <= 0.5);
        assert!(cfg.synthesis.enabled);
        assert!(cfg.optimization.egraph_optimization);
    }

    #[test]
    fn aggressive_profile_has_low_thresholds() {
        let cfg = MigrationConfig::from_profile(MigrationProfile::Aggressive);
        assert!(cfg.planner.min_confidence_threshold <= 0.2);
        assert!(cfg.confidence.auto_approve_threshold <= 0.6);
        assert!(cfg.synthesis.max_holes >= 100);
    }

    #[test]
    fn default_config_is_parity() {
        let cfg = MigrationConfig::default();
        assert_eq!(cfg.profile, MigrationProfile::Parity);
    }

    #[test]
    fn resolver_cli_overrides() {
        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Parity);
        resolver
            .apply_cli_overrides(&[
                "planner.min_confidence_threshold=0.8".into(),
                "synthesis.enabled=false".into(),
                "emission.crate_name=my-app".into(),
            ])
            .unwrap();

        let cfg = resolver.build();
        assert_eq!(cfg.planner.min_confidence_threshold, 0.8);
        assert!(!cfg.synthesis.enabled);
        assert_eq!(cfg.emission.crate_name, "my-app");
        assert_eq!(cfg.overrides_applied.len(), 3);
        assert!(
            cfg.overrides_applied
                .iter()
                .all(|o| o.source == OverrideSource::Cli)
        );
    }

    #[test]
    fn resolver_rejects_unknown_keys() {
        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Strict);
        let result = resolver.apply_cli_overrides(&["nonexistent.key=42".into()]);
        assert!(result.is_err());
    }

    #[test]
    fn resolver_rejects_bad_format() {
        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Strict);
        let result = resolver.apply_cli_overrides(&["no_equals_sign".into()]);
        assert!(result.is_err());
    }

    #[test]
    fn validate_catches_invalid_threshold() {
        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Parity);
        resolver
            .apply_cli_overrides(&["planner.min_confidence_threshold=1.5".into()])
            .unwrap();
        assert!(resolver.validate().is_err());
    }

    #[test]
    fn validate_catches_inverted_boundaries() {
        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Parity);
        resolver
            .apply_cli_overrides(&[
                "confidence.auto_approve_threshold=0.3".into(),
                "confidence.human_review_lower=0.5".into(),
            ])
            .unwrap();
        assert!(resolver.validate().is_err());
    }

    #[test]
    fn validate_catches_synthesis_contradiction() {
        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Parity);
        resolver
            .apply_cli_overrides(&[
                "synthesis.enabled=true".into(),
                "synthesis.max_holes=0".into(),
            ])
            .unwrap();
        assert!(resolver.validate().is_err());
    }

    #[test]
    fn validate_passes_on_defaults() {
        for &profile in &[
            MigrationProfile::Strict,
            MigrationProfile::Parity,
            MigrationProfile::Aggressive,
        ] {
            let resolver = ConfigResolver::from_profile(profile);
            resolver.validate().unwrap_or_else(|e| {
                panic!("default {} config should validate: {e}", profile.name())
            });
        }
    }

    #[test]
    fn hex_seed_override() {
        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Parity);
        resolver
            .apply_cli_overrides(&["planner.seed=0xDEADBEEF".into()])
            .unwrap();
        let cfg = resolver.build();
        assert_eq!(cfg.planner.seed, 0xDEAD_BEEF);
    }

    #[test]
    fn config_file_override() {
        let dir = std::env::temp_dir().join("ftui-test-config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_config.json");
        std::fs::write(
            &path,
            r#"{
                "planner.seed": 42,
                "synthesis.enabled": false,
                "emission.crate_name": "from-file"
            }"#,
        )
        .unwrap();

        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Aggressive);
        resolver.apply_config_file(&path).unwrap();
        let cfg = resolver.build();

        assert_eq!(cfg.planner.seed, 42);
        assert!(!cfg.synthesis.enabled);
        assert_eq!(cfg.emission.crate_name, "from-file");
        assert!(cfg.config_file_path.is_some());

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn config_file_rejects_unknown_keys() {
        let dir = std::env::temp_dir().join("ftui-test-config-bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad_config.json");
        std::fs::write(&path, r#"{ "nonexistent.key": 42 }"#).unwrap();

        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Parity);
        let result = resolver.apply_config_file(&path);
        assert!(result.is_err());

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn precedence_cli_overrides_file() {
        let dir = std::env::temp_dir().join("ftui-test-precedence");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prec_config.json");
        std::fs::write(&path, r#"{ "planner.min_confidence_threshold": 0.5 }"#).unwrap();

        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Strict);
        resolver.apply_config_file(&path).unwrap();
        resolver
            .apply_cli_overrides(&["planner.min_confidence_threshold=0.99".into()])
            .unwrap();
        let cfg = resolver.build();

        // CLI should win
        assert_eq!(cfg.planner.min_confidence_threshold, 0.99);

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn config_serialization_roundtrip() {
        let cfg = MigrationConfig::from_profile(MigrationProfile::Parity);
        let json = serde_json::to_string(&cfg).expect("should serialize");
        let deser: MigrationConfig = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deser.profile, MigrationProfile::Parity);
        assert_eq!(
            deser.planner.min_confidence_threshold,
            cfg.planner.min_confidence_threshold
        );
    }

    #[test]
    fn overrides_audit_trail() {
        let mut resolver = ConfigResolver::from_profile(MigrationProfile::Parity);
        resolver
            .apply_cli_overrides(&["planner.seed=123".into(), "synthesis.enabled=true".into()])
            .unwrap();
        let cfg = resolver.build();

        assert_eq!(cfg.overrides_applied.len(), 2);
        assert_eq!(cfg.overrides_applied[0].key, "planner.seed");
        assert_eq!(cfg.overrides_applied[0].source, OverrideSource::Cli);
        assert_eq!(cfg.overrides_applied[1].key, "synthesis.enabled");
    }

    #[test]
    fn parse_value_str_types() {
        assert_eq!(parse_value_str("true"), serde_json::Value::Bool(true));
        assert_eq!(parse_value_str("false"), serde_json::Value::Bool(false));
        assert_eq!(parse_value_str("42"), serde_json::json!(42));
        assert_eq!(parse_value_str("0xFF"), serde_json::json!(255));
        assert_eq!(
            parse_value_str("hello"),
            serde_json::Value::String("hello".into())
        );
    }

    #[test]
    fn valid_config_keys_covers_all_sections() {
        assert!(VALID_CONFIG_KEYS.iter().any(|k| k.starts_with("planner.")));
        assert!(
            VALID_CONFIG_KEYS
                .iter()
                .any(|k| k.starts_with("confidence."))
        );
        assert!(VALID_CONFIG_KEYS.iter().any(|k| k.starts_with("emission.")));
        assert!(
            VALID_CONFIG_KEYS
                .iter()
                .any(|k| k.starts_with("optimization."))
        );
        assert!(
            VALID_CONFIG_KEYS
                .iter()
                .any(|k| k.starts_with("synthesis."))
        );
        assert_eq!(VALID_CONFIG_KEYS.len(), 20);
    }
}
