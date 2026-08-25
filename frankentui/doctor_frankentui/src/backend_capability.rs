// SPDX-License-Identifier: Apache-2.0
//! Backend capability detection, caching, and deterministic fallback resolution.
//!
//! When migrating an OpenTUI application, certain source behaviors depend on
//! terminal capabilities that may or may not be present at runtime (TrueColor,
//! mouse input, alternate screen, unicode width, etc.).
//!
//! This module provides:
//! - [`TerminalCapabilities`]: a cached snapshot of detected terminal features
//! - [`CapabilityProbe`]: individual probe results with confidence
//! - [`FallbackPolicy`]: per-capability fallback strategy
//! - [`resolve_capabilities`]: entry point that probes, caches, and resolves
//!   the full set of backend capabilities needed by a migration

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::migration_ir::{Capability, CapabilityProfile};

// ── Constants ──────────────────────────────────────────────────────────

pub const BACKEND_CAPABILITY_VERSION: &str = "backend-capability-v1";

// ── Core Types ─────────────────────────────────────────────────────────

/// Complete terminal capability snapshot with probe results and fallbacks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalCapabilities {
    /// Module version for forward compatibility.
    pub version: String,
    /// Individual probe results keyed by capability name.
    pub probes: BTreeMap<String, CapabilityProbe>,
    /// Resolved fallback policies for missing/degraded capabilities.
    pub fallbacks: BTreeMap<String, FallbackPolicy>,
    /// Diagnostic messages produced during detection.
    pub diagnostics: Vec<CapabilityDiagnostic>,
    /// Aggregate statistics.
    pub stats: CapabilityStats,
}

/// Result of probing a single terminal capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityProbe {
    /// The capability being probed.
    pub capability: String,
    /// Whether the capability is available.
    pub available: bool,
    /// Confidence in the probe result (0.0–1.0).
    pub confidence: f64,
    /// Detection method used.
    pub method: DetectionMethod,
    /// Additional notes about the probe result.
    pub notes: String,
}

/// How a capability was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionMethod {
    /// Detected from environment variables (TERM, COLORTERM, etc.).
    Environment,
    /// Detected from terminfo/termcap database.
    Terminfo,
    /// Detected via terminal query (DA1, DA2, etc.).
    TerminalQuery,
    /// Assumed from platform defaults.
    PlatformDefault,
    /// Manually configured by user.
    UserConfig,
    /// Inferred from other probe results.
    Inferred,
}

/// Fallback strategy when a capability is missing or degraded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackPolicy {
    /// The capability this fallback addresses.
    pub capability: String,
    /// The fallback strategy to use.
    pub strategy: FallbackStrategy,
    /// Whether this fallback is reversible (can upgrade if capability appears).
    pub reversible: bool,
    /// Risk level of the fallback.
    pub risk: FallbackRisk,
    /// Description of what changes with this fallback.
    pub description: String,
}

/// Fallback strategy kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackStrategy {
    /// Downgrade gracefully (e.g., TrueColor → 256-color → 16-color).
    Downgrade,
    /// Disable the feature entirely (e.g., mouse input → keyboard only).
    Disable,
    /// Emulate the feature in software (e.g., alternate screen → clear + redraw).
    Emulate,
    /// Warn and proceed with partial functionality.
    WarnAndProceed,
    /// Block migration if the capability is required.
    Block,
}

/// Risk level of a fallback path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackRisk {
    /// No visible impact to the user.
    None,
    /// Minor cosmetic differences.
    Cosmetic,
    /// Functionality degraded but usable.
    Degraded,
    /// Significant functionality loss.
    Significant,
    /// Migration should not proceed without this capability.
    Blocking,
}

/// Diagnostic produced during capability detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDiagnostic {
    /// Severity level.
    pub level: DiagLevel,
    /// The capability involved.
    pub capability: String,
    /// Human-readable message.
    pub message: String,
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagLevel {
    Info,
    Warning,
    Error,
}

/// Aggregate statistics for capability resolution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityStats {
    /// Total capabilities probed.
    pub total_probed: usize,
    /// Capabilities found available.
    pub available: usize,
    /// Capabilities using fallback paths.
    pub using_fallback: usize,
    /// Capabilities that block migration.
    pub blocking: usize,
    /// Mean probe confidence.
    pub mean_confidence: f64,
    /// Counts by fallback risk level.
    pub by_risk: BTreeMap<String, usize>,
}

/// Configuration for capability resolution.
#[derive(Debug, Clone)]
pub struct CapabilityConfig {
    /// Environment variables to check (overrides for testing).
    pub env_overrides: BTreeMap<String, String>,
    /// Whether to trust environment detection (default true).
    pub trust_environment: bool,
    /// Whether to allow blocking fallbacks (default true).
    pub allow_blocking: bool,
}

impl Default for CapabilityConfig {
    fn default() -> Self {
        Self {
            env_overrides: BTreeMap::new(),
            trust_environment: true,
            allow_blocking: true,
        }
    }
}

// ── Public API ─────────────────────────────────────────────────────────

/// Resolve terminal capabilities needed by a migration profile.
///
/// Probes the terminal environment (or uses config overrides), determines
/// which required/optional capabilities are available, and assigns
/// fallback policies for any missing features.
pub fn resolve_capabilities(
    profile: &CapabilityProfile,
    config: &CapabilityConfig,
) -> TerminalCapabilities {
    let mut probes = BTreeMap::new();
    let mut fallbacks = BTreeMap::new();
    let mut diagnostics = Vec::new();

    // Probe all required capabilities
    for cap in &profile.required {
        let name = capability_name(cap);
        let probe = probe_capability(cap, config);
        if !probe.available {
            let fallback = resolve_fallback(cap, true);
            diagnostics.push(CapabilityDiagnostic {
                level: if fallback.risk >= FallbackRisk::Significant {
                    DiagLevel::Error
                } else {
                    DiagLevel::Warning
                },
                capability: name.clone(),
                message: format!(
                    "Required capability '{}' not available; fallback: {}",
                    name, fallback.description
                ),
            });
            fallbacks.insert(name.clone(), fallback);
        } else {
            diagnostics.push(CapabilityDiagnostic {
                level: DiagLevel::Info,
                capability: name.clone(),
                message: format!("Capability '{}' detected via {:?}", name, probe.method),
            });
        }
        probes.insert(name, probe);
    }

    // Probe optional capabilities
    for cap in &profile.optional {
        let name = capability_name(cap);
        let probe = probe_capability(cap, config);
        if !probe.available {
            let fallback = resolve_fallback(cap, false);
            diagnostics.push(CapabilityDiagnostic {
                level: DiagLevel::Info,
                capability: name.clone(),
                message: format!(
                    "Optional capability '{}' not available; fallback: {}",
                    name, fallback.description
                ),
            });
            fallbacks.insert(name.clone(), fallback);
        }
        probes.insert(name, probe);
    }

    // Compute stats
    let total_probed = probes.len();
    let available = probes.values().filter(|p| p.available).count();
    let using_fallback = fallbacks.len();
    let blocking = fallbacks
        .values()
        .filter(|f| f.strategy == FallbackStrategy::Block)
        .count();
    let mean_confidence = if total_probed > 0 {
        probes.values().map(|p| p.confidence).sum::<f64>() / total_probed as f64
    } else {
        1.0
    };

    let mut by_risk = BTreeMap::new();
    for fb in fallbacks.values() {
        *by_risk.entry(format!("{:?}", fb.risk)).or_insert(0) += 1;
    }

    TerminalCapabilities {
        version: BACKEND_CAPABILITY_VERSION.to_string(),
        probes,
        fallbacks,
        diagnostics,
        stats: CapabilityStats {
            total_probed,
            available,
            using_fallback,
            blocking,
            mean_confidence,
            by_risk,
        },
    }
}

/// Resolve capabilities with default configuration.
pub fn resolve_capabilities_simple(profile: &CapabilityProfile) -> TerminalCapabilities {
    resolve_capabilities(profile, &CapabilityConfig::default())
}

/// Check if all blocking capabilities are satisfied.
pub fn migration_can_proceed(caps: &TerminalCapabilities) -> bool {
    caps.stats.blocking == 0
}

// ── Internal probing ───────────────────────────────────────────────────

fn capability_name(cap: &Capability) -> String {
    match cap {
        Capability::MouseInput => "mouse_input".to_string(),
        Capability::KeyboardInput => "keyboard_input".to_string(),
        Capability::TouchInput => "touch_input".to_string(),
        Capability::NetworkAccess => "network_access".to_string(),
        Capability::FileSystem => "file_system".to_string(),
        Capability::Clipboard => "clipboard".to_string(),
        Capability::Timers => "timers".to_string(),
        Capability::AlternateScreen => "alternate_screen".to_string(),
        Capability::TrueColor => "true_color".to_string(),
        Capability::Unicode => "unicode".to_string(),
        Capability::InlineMode => "inline_mode".to_string(),
        Capability::ProcessSpawn => "process_spawn".to_string(),
        Capability::Custom(name) => format!("custom:{name}"),
    }
}

fn probe_capability(cap: &Capability, config: &CapabilityConfig) -> CapabilityProbe {
    let name = capability_name(cap);

    // Check config overrides first
    if let Some(val) = config.env_overrides.get(&name) {
        return CapabilityProbe {
            capability: name,
            available: val == "true" || val == "1",
            confidence: 1.0,
            method: DetectionMethod::UserConfig,
            notes: "Configured via override".to_string(),
        };
    }

    // Environment-based detection
    if config.trust_environment
        && let Some(probe) = probe_from_environment(cap)
    {
        return probe;
    }

    // Platform defaults
    probe_from_defaults(cap)
}

fn probe_from_environment(cap: &Capability) -> Option<CapabilityProbe> {
    let name = capability_name(cap);
    match cap {
        Capability::TrueColor => {
            // COLORTERM=truecolor or 24bit indicates TrueColor support
            let colorterm = std::env::var("COLORTERM").unwrap_or_default();
            let available = colorterm == "truecolor" || colorterm == "24bit";
            Some(CapabilityProbe {
                capability: name,
                available,
                confidence: if available { 0.95 } else { 0.7 },
                method: DetectionMethod::Environment,
                notes: format!("COLORTERM={colorterm}"),
            })
        }
        Capability::Unicode => {
            // Check LANG/LC_ALL for UTF-8
            let lang = std::env::var("LANG").unwrap_or_default();
            let available = lang.contains("UTF-8") || lang.contains("utf-8");
            Some(CapabilityProbe {
                capability: name,
                available,
                confidence: if available { 0.9 } else { 0.6 },
                method: DetectionMethod::Environment,
                notes: format!("LANG={lang}"),
            })
        }
        Capability::AlternateScreen => {
            // Most modern terminals support alternate screen
            let term = std::env::var("TERM").unwrap_or_default();
            let available = term.contains("xterm")
                || term.contains("screen")
                || term.contains("tmux")
                || term.contains("rxvt")
                || term.contains("alacritty")
                || term.contains("kitty")
                || term.contains("ghostty");
            Some(CapabilityProbe {
                capability: name,
                available,
                confidence: 0.85,
                method: DetectionMethod::Environment,
                notes: format!("TERM={term}"),
            })
        }
        Capability::MouseInput => {
            // SGR mouse is widely supported
            let term = std::env::var("TERM").unwrap_or_default();
            let available = !term.is_empty() && term != "dumb";
            Some(CapabilityProbe {
                capability: name,
                available,
                confidence: 0.8,
                method: DetectionMethod::Environment,
                notes: format!("TERM={term}"),
            })
        }
        _ => None,
    }
}

fn probe_from_defaults(cap: &Capability) -> CapabilityProbe {
    let name = capability_name(cap);
    match cap {
        // These are always available in a Rust process
        Capability::KeyboardInput
        | Capability::Timers
        | Capability::NetworkAccess
        | Capability::FileSystem
        | Capability::ProcessSpawn => CapabilityProbe {
            capability: name,
            available: true,
            confidence: 1.0,
            method: DetectionMethod::PlatformDefault,
            notes: "Always available in Rust runtime".to_string(),
        },
        // These require specific terminal support
        Capability::Clipboard => CapabilityProbe {
            capability: name,
            available: false,
            confidence: 0.5,
            method: DetectionMethod::PlatformDefault,
            notes: "Clipboard requires OSC 52 or platform integration".to_string(),
        },
        Capability::TouchInput => CapabilityProbe {
            capability: name,
            available: false,
            confidence: 0.9,
            method: DetectionMethod::PlatformDefault,
            notes: "Touch input not available in standard terminals".to_string(),
        },
        Capability::InlineMode => CapabilityProbe {
            capability: name,
            available: true,
            confidence: 0.95,
            method: DetectionMethod::PlatformDefault,
            notes: "FrankenTUI is inline-mode-first".to_string(),
        },
        _ => CapabilityProbe {
            capability: name,
            available: false,
            confidence: 0.5,
            method: DetectionMethod::PlatformDefault,
            notes: "No detection method available; assuming unavailable".to_string(),
        },
    }
}

fn resolve_fallback(cap: &Capability, required: bool) -> FallbackPolicy {
    let name = capability_name(cap);
    match cap {
        Capability::TrueColor => FallbackPolicy {
            capability: name,
            strategy: FallbackStrategy::Downgrade,
            reversible: true,
            risk: FallbackRisk::Cosmetic,
            description: "Downgrade to 256-color or 16-color palette".to_string(),
        },
        Capability::MouseInput => FallbackPolicy {
            capability: name,
            strategy: FallbackStrategy::Disable,
            reversible: true,
            risk: FallbackRisk::Degraded,
            description: "Disable mouse; use keyboard navigation only".to_string(),
        },
        Capability::AlternateScreen => FallbackPolicy {
            capability: name,
            strategy: FallbackStrategy::Emulate,
            reversible: true,
            risk: FallbackRisk::Cosmetic,
            description: "Emulate via clear-screen + redraw in inline mode".to_string(),
        },
        Capability::Unicode => FallbackPolicy {
            capability: name,
            strategy: FallbackStrategy::Downgrade,
            reversible: true,
            risk: FallbackRisk::Degraded,
            description: "Fall back to ASCII-only rendering".to_string(),
        },
        Capability::Clipboard => FallbackPolicy {
            capability: name,
            strategy: FallbackStrategy::Disable,
            reversible: true,
            risk: FallbackRisk::Degraded,
            description: "Disable clipboard integration; use terminal paste".to_string(),
        },
        Capability::TouchInput => FallbackPolicy {
            capability: name,
            strategy: FallbackStrategy::Disable,
            reversible: false,
            risk: if required {
                FallbackRisk::Significant
            } else {
                FallbackRisk::None
            },
            description: "Touch input not available in terminal environment".to_string(),
        },
        Capability::InlineMode => FallbackPolicy {
            capability: name,
            strategy: FallbackStrategy::Emulate,
            reversible: true,
            risk: FallbackRisk::Cosmetic,
            description: "FrankenTUI inline mode is native; fallback not needed".to_string(),
        },
        _ => {
            if required {
                FallbackPolicy {
                    capability: name,
                    strategy: FallbackStrategy::Block,
                    reversible: false,
                    risk: FallbackRisk::Blocking,
                    description: "Required capability has no fallback path".to_string(),
                }
            } else {
                FallbackPolicy {
                    capability: name,
                    strategy: FallbackStrategy::WarnAndProceed,
                    reversible: true,
                    risk: FallbackRisk::Cosmetic,
                    description: "Optional capability unavailable; proceeding without it"
                        .to_string(),
                }
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::*;

    #[derive(Debug, Clone)]
    struct CapabilityScenario {
        scenario_id: &'static str,
        capability: Capability,
        required: bool,
        expected_outcome: &'static str,
        expected_fallback: Option<FallbackStrategy>,
    }

    fn classify_outcome(caps: &TerminalCapabilities, capability_id: &str) -> &'static str {
        let probe = caps
            .probes
            .get(capability_id)
            .expect("scenario capability must be probed");
        if probe.available {
            return "available";
        }

        match caps
            .fallbacks
            .get(capability_id)
            .map(|fallback| fallback.strategy)
        {
            Some(FallbackStrategy::Block) => "blocked",
            Some(_) => "fallback",
            None => "missing",
        }
    }

    fn profile_for_scenario(scenario: &CapabilityScenario) -> CapabilityProfile {
        let mut required = BTreeSet::new();
        let mut optional = BTreeSet::new();
        if scenario.required {
            required.insert(scenario.capability.clone());
        } else {
            optional.insert(scenario.capability.clone());
        }
        CapabilityProfile {
            required,
            optional,
            platform_assumptions: vec![],
        }
    }

    fn outcome_map(caps: &TerminalCapabilities) -> BTreeMap<String, String> {
        let mut outcomes = BTreeMap::new();
        for capability_id in caps.probes.keys() {
            outcomes.insert(
                capability_id.clone(),
                classify_outcome(caps, capability_id).to_string(),
            );
        }
        outcomes
    }

    fn empty_profile() -> CapabilityProfile {
        CapabilityProfile {
            required: BTreeSet::new(),
            optional: BTreeSet::new(),
            platform_assumptions: vec![],
        }
    }

    fn simple_profile() -> CapabilityProfile {
        let mut required = BTreeSet::new();
        required.insert(Capability::KeyboardInput);
        required.insert(Capability::TrueColor);
        let mut optional = BTreeSet::new();
        optional.insert(Capability::MouseInput);
        optional.insert(Capability::Clipboard);
        CapabilityProfile {
            required,
            optional,
            platform_assumptions: vec![],
        }
    }

    fn test_config() -> CapabilityConfig {
        CapabilityConfig {
            env_overrides: BTreeMap::new(),
            trust_environment: false,
            allow_blocking: true,
        }
    }

    #[test]
    fn capability_scenarios_cover_happy_boundary_and_failure_modes() {
        let scenarios = vec![
            CapabilityScenario {
                scenario_id: "happy_required_keyboard_input",
                capability: Capability::KeyboardInput,
                required: true,
                expected_outcome: "available",
                expected_fallback: None,
            },
            CapabilityScenario {
                scenario_id: "happy_required_network_access",
                capability: Capability::NetworkAccess,
                required: true,
                expected_outcome: "available",
                expected_fallback: None,
            },
            CapabilityScenario {
                scenario_id: "happy_required_file_system",
                capability: Capability::FileSystem,
                required: true,
                expected_outcome: "available",
                expected_fallback: None,
            },
            CapabilityScenario {
                scenario_id: "happy_required_timers",
                capability: Capability::Timers,
                required: true,
                expected_outcome: "available",
                expected_fallback: None,
            },
            CapabilityScenario {
                scenario_id: "happy_required_process_spawn",
                capability: Capability::ProcessSpawn,
                required: true,
                expected_outcome: "available",
                expected_fallback: None,
            },
            CapabilityScenario {
                scenario_id: "happy_required_inline_mode",
                capability: Capability::InlineMode,
                required: true,
                expected_outcome: "available",
                expected_fallback: None,
            },
            CapabilityScenario {
                scenario_id: "boundary_required_mouse_input_fallback",
                capability: Capability::MouseInput,
                required: true,
                expected_outcome: "fallback",
                expected_fallback: Some(FallbackStrategy::Disable),
            },
            CapabilityScenario {
                scenario_id: "boundary_required_alternate_screen_fallback",
                capability: Capability::AlternateScreen,
                required: true,
                expected_outcome: "fallback",
                expected_fallback: Some(FallbackStrategy::Emulate),
            },
            CapabilityScenario {
                scenario_id: "boundary_required_truecolor_fallback",
                capability: Capability::TrueColor,
                required: true,
                expected_outcome: "fallback",
                expected_fallback: Some(FallbackStrategy::Downgrade),
            },
            CapabilityScenario {
                scenario_id: "boundary_required_unicode_fallback",
                capability: Capability::Unicode,
                required: true,
                expected_outcome: "fallback",
                expected_fallback: Some(FallbackStrategy::Downgrade),
            },
            CapabilityScenario {
                scenario_id: "boundary_optional_clipboard_fallback",
                capability: Capability::Clipboard,
                required: false,
                expected_outcome: "fallback",
                expected_fallback: Some(FallbackStrategy::Disable),
            },
            CapabilityScenario {
                scenario_id: "boundary_required_touch_input_fallback",
                capability: Capability::TouchInput,
                required: true,
                expected_outcome: "fallback",
                expected_fallback: Some(FallbackStrategy::Disable),
            },
            CapabilityScenario {
                scenario_id: "failure_required_custom_capability_blocked",
                capability: Capability::Custom("gpu-accel".to_string()),
                required: true,
                expected_outcome: "blocked",
                expected_fallback: Some(FallbackStrategy::Block),
            },
        ];

        let mut structured_logs = Vec::new();
        for scenario in &scenarios {
            let capability_id = capability_name(&scenario.capability);
            let caps = resolve_capabilities(&profile_for_scenario(scenario), &test_config());
            let outcome = classify_outcome(&caps, &capability_id);
            assert_eq!(
                outcome, scenario.expected_outcome,
                "Scenario {} expected {}, got {}",
                scenario.scenario_id, scenario.expected_outcome, outcome
            );

            if let Some(expected_fallback) = scenario.expected_fallback {
                let fallback = caps
                    .fallbacks
                    .get(&capability_id)
                    .expect("expected fallback policy must be present");
                assert_eq!(fallback.strategy, expected_fallback);
            }

            structured_logs.push(json!({
                "capability_id": capability_id,
                "scenario_id": scenario.scenario_id,
                "outcome_classification": outcome,
            }));
        }

        for entry in &structured_logs {
            let capability_id = entry
                .get("capability_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let scenario_id = entry
                .get("scenario_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let outcome_classification = entry
                .get("outcome_classification")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            assert!(
                !capability_id.is_empty(),
                "structured log missing capability_id"
            );
            assert!(
                !scenario_id.is_empty(),
                "structured log missing scenario_id"
            );
            assert!(
                matches!(
                    outcome_classification,
                    "available" | "fallback" | "blocked" | "missing"
                ),
                "unexpected outcome_classification: {outcome_classification}"
            );
        }
    }

    #[test]
    fn migration_fixture_capability_regression_matrix_is_stable() {
        let dashboard_fixture = CapabilityProfile {
            required: BTreeSet::from([
                Capability::KeyboardInput,
                Capability::AlternateScreen,
                Capability::TrueColor,
            ]),
            optional: BTreeSet::from([Capability::MouseInput, Capability::Unicode]),
            platform_assumptions: vec![],
        };
        let dashboard_caps = resolve_capabilities(&dashboard_fixture, &test_config());
        let dashboard_outcomes = outcome_map(&dashboard_caps);
        assert_eq!(dashboard_caps.stats.total_probed, 5);
        assert_eq!(dashboard_caps.stats.using_fallback, 4);
        assert_eq!(dashboard_caps.stats.blocking, 0);
        assert!(migration_can_proceed(&dashboard_caps));
        assert_eq!(dashboard_outcomes["keyboard_input"], "available");
        assert_eq!(dashboard_outcomes["alternate_screen"], "fallback");
        assert_eq!(dashboard_outcomes["true_color"], "fallback");
        assert_eq!(dashboard_outcomes["mouse_input"], "fallback");
        assert_eq!(dashboard_outcomes["unicode"], "fallback");

        let worker_fixture = CapabilityProfile {
            required: BTreeSet::from([
                Capability::KeyboardInput,
                Capability::NetworkAccess,
                Capability::FileSystem,
                Capability::ProcessSpawn,
                Capability::Timers,
            ]),
            optional: BTreeSet::from([Capability::Clipboard]),
            platform_assumptions: vec![],
        };
        let worker_caps = resolve_capabilities(&worker_fixture, &test_config());
        let worker_outcomes = outcome_map(&worker_caps);
        assert_eq!(worker_caps.stats.total_probed, 6);
        assert_eq!(worker_caps.stats.using_fallback, 1);
        assert_eq!(worker_caps.stats.blocking, 0);
        assert!(migration_can_proceed(&worker_caps));
        assert_eq!(worker_outcomes["clipboard"], "fallback");
        for capability_id in [
            "keyboard_input",
            "network_access",
            "file_system",
            "process_spawn",
            "timers",
        ] {
            assert_eq!(worker_outcomes[capability_id], "available");
        }

        let blocking_fixture = CapabilityProfile {
            required: BTreeSet::from([
                Capability::KeyboardInput,
                Capability::Custom("device-sensor".to_string()),
            ]),
            optional: BTreeSet::new(),
            platform_assumptions: vec![],
        };
        let blocking_caps = resolve_capabilities(&blocking_fixture, &test_config());
        let blocking_outcomes = outcome_map(&blocking_caps);
        assert_eq!(blocking_caps.stats.total_probed, 2);
        assert_eq!(blocking_caps.stats.blocking, 1);
        assert!(!migration_can_proceed(&blocking_caps));
        assert_eq!(blocking_outcomes["keyboard_input"], "available");
        assert_eq!(blocking_outcomes["custom:device-sensor"], "blocked");
    }

    #[test]
    fn empty_profile_resolves_cleanly() {
        let profile = empty_profile();
        let caps = resolve_capabilities(&profile, &test_config());
        assert_eq!(caps.version, BACKEND_CAPABILITY_VERSION);
        assert!(caps.probes.is_empty());
        assert!(caps.fallbacks.is_empty());
        assert_eq!(caps.stats.total_probed, 0);
        assert!(migration_can_proceed(&caps));
    }

    #[test]
    fn simple_profile_probes_all_capabilities() {
        let profile = simple_profile();
        let caps = resolve_capabilities(&profile, &test_config());
        assert_eq!(caps.stats.total_probed, 4);
        assert!(caps.probes.contains_key("keyboard_input"));
        assert!(caps.probes.contains_key("true_color"));
        assert!(caps.probes.contains_key("mouse_input"));
        assert!(caps.probes.contains_key("clipboard"));
    }

    #[test]
    fn keyboard_input_always_available() {
        let profile = simple_profile();
        let caps = resolve_capabilities(&profile, &test_config());
        let probe = &caps.probes["keyboard_input"];
        assert!(probe.available);
        assert_eq!(probe.confidence, 1.0);
    }

    #[test]
    fn truecolor_uses_fallback_without_env() {
        let mut config = test_config();
        config.trust_environment = false;
        let profile = simple_profile();
        let caps = resolve_capabilities(&profile, &config);

        // Without environment, TrueColor should have fallback
        if !caps.probes["true_color"].available {
            assert!(caps.fallbacks.contains_key("true_color"));
            let fb = &caps.fallbacks["true_color"];
            assert_eq!(fb.strategy, FallbackStrategy::Downgrade);
            assert!(fb.reversible);
        }
    }

    #[test]
    fn config_overrides_force_capability() {
        let mut config = test_config();
        config
            .env_overrides
            .insert("true_color".to_string(), "true".to_string());

        let profile = simple_profile();
        let caps = resolve_capabilities(&profile, &config);

        let probe = &caps.probes["true_color"];
        assert!(probe.available);
        assert_eq!(probe.method, DetectionMethod::UserConfig);
        assert_eq!(probe.confidence, 1.0);
    }

    #[test]
    fn config_overrides_disable_capability() {
        let mut config = test_config();
        config
            .env_overrides
            .insert("keyboard_input".to_string(), "false".to_string());

        let profile = simple_profile();
        let caps = resolve_capabilities(&profile, &config);

        let probe = &caps.probes["keyboard_input"];
        assert!(!probe.available);
        // Should generate a fallback since it's required
        assert!(caps.fallbacks.contains_key("keyboard_input"));
    }

    #[test]
    fn required_unknown_capability_blocks() {
        let mut required = BTreeSet::new();
        required.insert(Capability::Custom("exotic-feature".to_string()));
        let profile = CapabilityProfile {
            required,
            optional: BTreeSet::new(),
            platform_assumptions: vec![],
        };

        let caps = resolve_capabilities(&profile, &test_config());
        let fb = &caps.fallbacks["custom:exotic-feature"];
        assert_eq!(fb.strategy, FallbackStrategy::Block);
        assert_eq!(fb.risk, FallbackRisk::Blocking);
        assert!(!migration_can_proceed(&caps));
    }

    #[test]
    fn optional_unknown_capability_warns() {
        let mut optional = BTreeSet::new();
        optional.insert(Capability::Custom("nice-to-have".to_string()));
        let profile = CapabilityProfile {
            required: BTreeSet::new(),
            optional,
            platform_assumptions: vec![],
        };

        let caps = resolve_capabilities(&profile, &test_config());
        let fb = &caps.fallbacks["custom:nice-to-have"];
        assert_eq!(fb.strategy, FallbackStrategy::WarnAndProceed);
        assert!(migration_can_proceed(&caps));
    }

    #[test]
    fn stats_are_consistent() {
        let profile = simple_profile();
        let caps = resolve_capabilities(&profile, &test_config());
        assert_eq!(
            caps.stats.available + caps.stats.using_fallback,
            caps.stats.total_probed
        );
        assert!(caps.stats.mean_confidence > 0.0);
        assert!(caps.stats.mean_confidence <= 1.0);
    }

    #[test]
    fn diagnostics_produced_for_all_probes() {
        let profile = simple_profile();
        let caps = resolve_capabilities(&profile, &test_config());
        // Each probe produces at least one diagnostic
        assert!(caps.diagnostics.len() >= caps.probes.len());
    }

    #[test]
    fn resolve_is_deterministic() {
        let profile = simple_profile();
        let config = test_config();
        let caps1 = resolve_capabilities(&profile, &config);
        let caps2 = resolve_capabilities(&profile, &config);

        assert_eq!(caps1.probes.len(), caps2.probes.len());
        for (name, p1) in &caps1.probes {
            let p2 = &caps2.probes[name];
            assert_eq!(p1.available, p2.available);
            assert_eq!(p1.confidence, p2.confidence);
            assert_eq!(p1.method, p2.method);
        }
    }

    #[test]
    fn fallback_risk_ordering() {
        assert!(FallbackRisk::None < FallbackRisk::Cosmetic);
        assert!(FallbackRisk::Cosmetic < FallbackRisk::Degraded);
        assert!(FallbackRisk::Degraded < FallbackRisk::Significant);
        assert!(FallbackRisk::Significant < FallbackRisk::Blocking);
    }

    #[test]
    fn clipboard_has_disable_fallback() {
        let mut optional = BTreeSet::new();
        optional.insert(Capability::Clipboard);
        let profile = CapabilityProfile {
            required: BTreeSet::new(),
            optional,
            platform_assumptions: vec![],
        };

        let caps = resolve_capabilities(&profile, &test_config());
        if caps.fallbacks.contains_key("clipboard") {
            let fb = &caps.fallbacks["clipboard"];
            assert_eq!(fb.strategy, FallbackStrategy::Disable);
        }
    }

    #[test]
    fn touch_input_unavailable_in_terminal() {
        let mut optional = BTreeSet::new();
        optional.insert(Capability::TouchInput);
        let profile = CapabilityProfile {
            required: BTreeSet::new(),
            optional,
            platform_assumptions: vec![],
        };

        let caps = resolve_capabilities(&profile, &test_config());
        assert!(!caps.probes["touch_input"].available);
    }

    #[test]
    fn inline_mode_is_native() {
        let mut required = BTreeSet::new();
        required.insert(Capability::InlineMode);
        let profile = CapabilityProfile {
            required,
            optional: BTreeSet::new(),
            platform_assumptions: vec![],
        };

        let caps = resolve_capabilities(&profile, &test_config());
        assert!(caps.probes["inline_mode"].available);
    }

    #[test]
    fn simple_resolve_uses_defaults() {
        let profile = empty_profile();
        let caps = resolve_capabilities_simple(&profile);
        assert_eq!(caps.version, BACKEND_CAPABILITY_VERSION);
    }

    #[test]
    fn all_platform_capabilities_have_names() {
        let caps = [
            Capability::MouseInput,
            Capability::KeyboardInput,
            Capability::TouchInput,
            Capability::NetworkAccess,
            Capability::FileSystem,
            Capability::Clipboard,
            Capability::Timers,
            Capability::AlternateScreen,
            Capability::TrueColor,
            Capability::Unicode,
            Capability::InlineMode,
            Capability::ProcessSpawn,
        ];
        for cap in &caps {
            let name = capability_name(cap);
            assert!(!name.is_empty(), "Capability {:?} has empty name", cap);
            assert!(
                !name.starts_with("custom:"),
                "Platform capability {:?} should not use custom prefix",
                cap
            );
        }
    }

    #[test]
    fn custom_capability_name_has_prefix() {
        let name = capability_name(&Capability::Custom("my-feature".to_string()));
        assert_eq!(name, "custom:my-feature");
    }
}
