// SPDX-License-Identifier: Apache-2.0
//! Translate side effects and async workflows into Cmd/subscription orchestration.
//!
//! Consumes a [`CanonicalEffectModel`] (classified effects with execution semantics)
//! and produces an [`EffectOrchestrationPlan`] — a structured description of the
//! generated async orchestration code:
//!
//! - **Command effects**: one-shot tasks with timeout, retry, and error handling
//! - **Subscription effects**: long-lived streams with cancellation policies
//! - **Fire-and-forget effects**: logging/telemetry with best-effort delivery
//! - **Ordering constraints**: dependency-aware sequencing of effect execution
//!
//! Design invariants:
//! - **Explicit timeouts**: every effect has a concrete timeout value — no implicit
//!   "wait forever" behavior.
//! - **Deterministic retries**: backoff strategies use formula-based delays (no
//!   randomness) so replay produces identical behavior.
//! - **Auditable errors**: every error path produces a structured log entry for
//!   the certification pipeline.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::effect_canonical::{
    AsyncBoundary, CanonicalEffect, CanonicalEffectModel, CleanupStrategy, ExecutionModel,
    MessageProtocol, OrderingConstraint,
};
use crate::migration_ir::{EffectKind, Provenance};

// ── Constants ──────────────────────────────────────────────────────────

/// Module version tag.
pub const EFFECT_TRANSLATOR_VERSION: &str = "effect-translator-v1";

// Default timeouts by effect kind (milliseconds).
const DEFAULT_TIMEOUT_NETWORK_MS: u64 = 30_000;
const DEFAULT_TIMEOUT_STORAGE_MS: u64 = 10_000;
const DEFAULT_TIMEOUT_DOM_MS: u64 = 5_000;
const DEFAULT_TIMEOUT_TIMER_MS: u64 = 60_000;
const DEFAULT_TIMEOUT_SUBSCRIPTION_MS: u64 = 300_000;
const DEFAULT_TIMEOUT_PROCESS_MS: u64 = 60_000;
const DEFAULT_TIMEOUT_TELEMETRY_MS: u64 = 5_000;
const DEFAULT_TIMEOUT_OTHER_MS: u64 = 30_000;

// Retry defaults.
const DEFAULT_MAX_RETRY_ATTEMPTS: u32 = 3;
const DEFAULT_RETRY_BASE_MS: u64 = 1000;
const DEFAULT_RETRY_MULTIPLIER: f64 = 2.0;

// ── Core Output Types ──────────────────────────────────────────────────

/// The complete effect orchestration plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectOrchestrationPlan {
    /// Schema version.
    pub version: String,
    /// Per-effect orchestration descriptors.
    pub orchestrations: BTreeMap<String, EffectOrchestration>,
    /// Ordering constraints preserved from canonical model.
    pub ordering_constraints: Vec<OrderingConstraint>,
    /// Diagnostics from translation.
    pub diagnostics: Vec<EffectDiagnostic>,
    /// Statistics.
    pub stats: EffectTranslationStats,
}

/// Orchestration descriptor for a single effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectOrchestration {
    /// Effect id from canonical model.
    pub effect_id: String,
    /// Human-readable name.
    pub name: String,
    /// What kind of runtime construct to generate.
    pub runtime_construct: RuntimeConstruct,
    /// Timeout in milliseconds.
    pub timeout_ms: u64,
    /// Cancellation policy.
    pub cancellation: CancellationPolicy,
    /// Retry configuration (if applicable).
    pub retry_config: Option<RetryConfig>,
    /// Error handling strategy.
    pub error_handling: ErrorStrategy,
    /// Async boundary description.
    pub async_boundary: String,
    /// Message type(s) produced.
    pub message_types: Vec<String>,
    /// Certification metadata for audit trail.
    pub certification: CertificationMetadata,
    /// Confidence in the orchestration.
    pub confidence: f64,
    /// Source provenance.
    pub provenance: Option<Provenance>,
}

/// What runtime construct to generate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeConstruct {
    /// `Cmd::Task` — one-shot async command.
    CmdTask,
    /// `Subscription<M>` — long-lived event stream.
    Subscription,
    /// `Cmd::Task` with unit return — fire-and-forget.
    CmdFireAndForget,
}

/// How to handle cancellation for this effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancellationPolicy {
    /// Kind of cancellation.
    pub kind: CancellationKind,
    /// Whether the cancellation is observable in the audit trail.
    pub observable: bool,
    /// Cleanup code description (if any).
    pub cleanup_description: Option<String>,
}

/// Kind of cancellation behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancellationKind {
    /// No cancellation needed.
    None,
    /// Runtime stops subscription automatically.
    SubscriptionStop,
    /// Explicit abort of in-flight work.
    AbortInFlight,
    /// Custom cleanup action required.
    ExplicitCleanup,
}

/// Deterministic retry configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_attempts: u32,
    /// Backoff strategy.
    pub backoff: BackoffStrategy,
    /// Whether the retry loop is deterministic (no randomness).
    pub deterministic: bool,
    /// Conditions that disable retry (e.g., 4xx HTTP status).
    pub non_retryable_conditions: Vec<String>,
}

/// Backoff strategy for retries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackoffStrategy {
    /// Fixed delay between retries.
    Fixed { delay_ms: u64 },
    /// Exponential backoff: `base_ms * multiplier^attempt`.
    Exponential { base_ms: u64, multiplier_x100: u64 },
    /// Linear backoff: `base_ms * (attempt + 1)`.
    Linear { base_ms: u64 },
}

/// Error handling strategy for an effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorStrategy {
    /// What message variant to emit on error.
    pub error_message_variant: String,
    /// Recovery action to take.
    pub recovery: RecoveryAction,
    /// Whether errors are logged to the certification audit trail.
    pub audit_logged: bool,
    /// Risk level for certification purposes.
    pub risk_level: String,
}

/// Recovery action when an effect fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryAction {
    /// Propagate the error to the update function.
    Propagate,
    /// Use a fallback value.
    Fallback { description: String },
    /// Silently ignore the error (fire-and-forget).
    Ignore,
    /// Retry via the retry configuration.
    RetryThenPropagate,
}

/// Certification metadata for the audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificationMetadata {
    /// State variables impacted by this effect.
    pub state_impact: Vec<String>,
    /// Whether timeout behavior is observable.
    pub timeout_observable: bool,
    /// Whether cancellation is observable.
    pub cancellation_observable: bool,
    /// Whether error behavior is observable.
    pub error_observable: bool,
    /// Effect category for risk classification.
    pub risk_category: String,
}

/// A diagnostic from effect translation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectDiagnostic {
    /// Diagnostic code.
    pub code: String,
    /// Severity: info, warning, error.
    pub severity: String,
    /// Diagnostic message.
    pub message: String,
    /// Related effect id.
    pub effect_id: Option<String>,
}

/// Statistics for effect translation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EffectTranslationStats {
    /// Total effects processed.
    pub total_effects: usize,
    /// Commands generated.
    pub commands: usize,
    /// Subscriptions generated.
    pub subscriptions: usize,
    /// Fire-and-forget effects generated.
    pub fire_and_forget: usize,
    /// Effects with retry configuration.
    pub with_retry: usize,
    /// Effects with explicit cancellation.
    pub with_cancellation: usize,
    /// Non-deterministic effects flagged.
    pub nondeterministic_flagged: usize,
}

// ── Public API ─────────────────────────────────────────────────────────

/// Translate a canonical effect model into an orchestration plan.
pub fn translate_effects(model: &CanonicalEffectModel) -> EffectOrchestrationPlan {
    let mut orchestrations = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut stats = EffectTranslationStats::default();

    for (id, effect) in &model.effects {
        stats.total_effects += 1;
        let orch = translate_single_effect(effect, &mut diagnostics, &mut stats);
        orchestrations.insert(id.0.clone(), orch);
    }

    EffectOrchestrationPlan {
        version: EFFECT_TRANSLATOR_VERSION.to_string(),
        orchestrations,
        ordering_constraints: model.ordering_constraints.clone(),
        diagnostics,
        stats,
    }
}

// ── Single Effect Translation ──────────────────────────────────────────

fn translate_single_effect(
    effect: &CanonicalEffect,
    diagnostics: &mut Vec<EffectDiagnostic>,
    stats: &mut EffectTranslationStats,
) -> EffectOrchestration {
    let runtime_construct = classify_runtime_construct(effect);
    let timeout_ms = infer_timeout(effect);
    let cancellation = build_cancellation_policy(effect);
    let retry_config = build_retry_config(effect, diagnostics);
    let error_handling = build_error_strategy(effect);
    let async_boundary = describe_async_boundary(effect);
    let message_types = extract_message_types(effect);
    let certification = build_certification_metadata(effect);

    // Update stats.
    match runtime_construct {
        RuntimeConstruct::CmdTask => stats.commands += 1,
        RuntimeConstruct::Subscription => stats.subscriptions += 1,
        RuntimeConstruct::CmdFireAndForget => stats.fire_and_forget += 1,
    }
    if retry_config.is_some() {
        stats.with_retry += 1;
    }
    if cancellation.kind != CancellationKind::None {
        stats.with_cancellation += 1;
    }

    // Flag non-deterministic effects.
    if !effect.deterministic {
        stats.nondeterministic_flagged += 1;
        diagnostics.push(EffectDiagnostic {
            code: "ET001".into(),
            severity: "warning".into(),
            message: format!(
                "Effect '{}' is non-deterministic; replay may diverge",
                effect.name
            ),
            effect_id: Some(effect.id.0.clone()),
        });
    }

    EffectOrchestration {
        effect_id: effect.id.0.clone(),
        name: effect.name.clone(),
        runtime_construct,
        timeout_ms,
        cancellation,
        retry_config,
        error_handling,
        async_boundary,
        message_types,
        certification,
        confidence: effect.confidence.score,
        provenance: Some(effect.provenance.clone()),
    }
}

// ── Runtime Construct Classification ───────────────────────────────────

fn classify_runtime_construct(effect: &CanonicalEffect) -> RuntimeConstruct {
    match effect.execution_model {
        ExecutionModel::Command => RuntimeConstruct::CmdTask,
        ExecutionModel::Subscription => RuntimeConstruct::Subscription,
        ExecutionModel::FireAndForget => RuntimeConstruct::CmdFireAndForget,
    }
}

// ── Timeout Inference ──────────────────────────────────────────────────

fn infer_timeout(effect: &CanonicalEffect) -> u64 {
    match effect.original_kind {
        EffectKind::Network => DEFAULT_TIMEOUT_NETWORK_MS,
        EffectKind::Storage => DEFAULT_TIMEOUT_STORAGE_MS,
        EffectKind::Dom => DEFAULT_TIMEOUT_DOM_MS,
        EffectKind::Timer => DEFAULT_TIMEOUT_TIMER_MS,
        EffectKind::Subscription => DEFAULT_TIMEOUT_SUBSCRIPTION_MS,
        EffectKind::Process => DEFAULT_TIMEOUT_PROCESS_MS,
        EffectKind::Telemetry => DEFAULT_TIMEOUT_TELEMETRY_MS,
        EffectKind::Other => DEFAULT_TIMEOUT_OTHER_MS,
    }
}

// ── Cancellation Policy ────────────────────────────────────────────────

fn build_cancellation_policy(effect: &CanonicalEffect) -> CancellationPolicy {
    let (kind, cleanup_description) = match effect.cleanup {
        CleanupStrategy::None => (CancellationKind::None, None),
        CleanupStrategy::SubscriptionStop => (
            CancellationKind::SubscriptionStop,
            Some("Runtime sends StopSignal to subscription".into()),
        ),
        CleanupStrategy::ExplicitAction => (
            CancellationKind::ExplicitCleanup,
            Some("Custom cleanup function invoked on deactivation".into()),
        ),
        CleanupStrategy::AbortInFlight => (
            CancellationKind::AbortInFlight,
            Some(abort_description(&effect.original_kind)),
        ),
    };

    let observable = kind != CancellationKind::None;

    CancellationPolicy {
        kind,
        observable,
        cleanup_description,
    }
}

fn abort_description(kind: &EffectKind) -> String {
    match kind {
        EffectKind::Network => "Cancel in-flight HTTP request via AbortController".into(),
        EffectKind::Timer => "Clear timer via clearTimeout/clearInterval".into(),
        EffectKind::Process => "Kill spawned process".into(),
        EffectKind::Subscription => "Close event stream".into(),
        _ => "Abort in-flight operation".into(),
    }
}

// ── Retry Configuration ────────────────────────────────────────────────

fn build_retry_config(
    effect: &CanonicalEffect,
    diagnostics: &mut Vec<EffectDiagnostic>,
) -> Option<RetryConfig> {
    // Only retryable effect kinds get retry configs.
    let retryable = matches!(
        effect.original_kind,
        EffectKind::Network | EffectKind::Storage | EffectKind::Process
    );

    if !retryable {
        return None;
    }

    // Commands get retry; subscriptions reconnect via their own mechanism.
    if effect.execution_model != ExecutionModel::Command {
        return None;
    }

    // Non-deterministic + retry = warning.
    if !effect.deterministic {
        diagnostics.push(EffectDiagnostic {
            code: "ET002".into(),
            severity: "warning".into(),
            message: format!(
                "Effect '{}' is non-deterministic with retry enabled; \
                 replay divergence is likely on retry paths",
                effect.name
            ),
            effect_id: Some(effect.id.0.clone()),
        });
    }

    // Idempotent effects get more aggressive retry.
    let max_attempts = if effect.idempotent {
        DEFAULT_MAX_RETRY_ATTEMPTS
    } else {
        1 // Non-idempotent: only one retry to avoid side-effect duplication.
    };

    let backoff = if effect.idempotent {
        BackoffStrategy::Exponential {
            base_ms: DEFAULT_RETRY_BASE_MS,
            multiplier_x100: (DEFAULT_RETRY_MULTIPLIER * 100.0) as u64,
        }
    } else {
        BackoffStrategy::Fixed {
            delay_ms: DEFAULT_RETRY_BASE_MS,
        }
    };

    let non_retryable_conditions = match effect.original_kind {
        EffectKind::Network => vec![
            "HTTP 4xx client error".into(),
            "Authentication failure".into(),
        ],
        EffectKind::Storage => vec!["Quota exceeded".into(), "Permission denied".into()],
        _ => Vec::new(),
    };

    Some(RetryConfig {
        max_attempts,
        backoff,
        deterministic: true, // All backoff strategies are formula-based.
        non_retryable_conditions,
    })
}

/// Compute the delay for a given retry attempt using the backoff strategy.
pub fn compute_retry_delay(backoff: &BackoffStrategy, attempt: u32) -> u64 {
    match backoff {
        BackoffStrategy::Fixed { delay_ms } => *delay_ms,
        BackoffStrategy::Exponential {
            base_ms,
            multiplier_x100,
        } => {
            let multiplier = *multiplier_x100 as f64 / 100.0;
            let delay = *base_ms as f64 * multiplier.powf(f64::from(attempt));
            round_nonnegative_f64_to_u64_saturating(delay)
        }
        BackoffStrategy::Linear { base_ms } => {
            base_ms.saturating_mul(u64::from(attempt).saturating_add(1))
        }
    }
}

fn round_nonnegative_f64_to_u64_saturating(value: f64) -> u64 {
    if value.is_nan() || value <= 0.0 {
        0
    } else if value >= u64::MAX as f64 {
        u64::MAX
    } else {
        value.round() as u64
    }
}

// ── Error Strategy ─────────────────────────────────────────────────────

fn build_error_strategy(effect: &CanonicalEffect) -> ErrorStrategy {
    let error_variant = format!("{}Error", to_pascal_case(&effect.name));

    let (recovery, risk_level) = match effect.execution_model {
        ExecutionModel::FireAndForget => (RecoveryAction::Ignore, "low"),
        ExecutionModel::Command => {
            let recovery = if effect.writes.is_empty() {
                // Read-only effect: safe to propagate.
                RecoveryAction::Propagate
            } else {
                // Write effect: retry first if available.
                RecoveryAction::RetryThenPropagate
            };
            let risk = match effect.original_kind {
                EffectKind::Network => "high",
                EffectKind::Storage | EffectKind::Process => "medium",
                _ => "low",
            };
            (recovery, risk)
        }
        ExecutionModel::Subscription => (
            RecoveryAction::Fallback {
                description: "Emit disconnection message, allow reconnect".into(),
            },
            match effect.original_kind {
                EffectKind::Network => "high",
                _ => "medium",
            },
        ),
    };

    ErrorStrategy {
        error_message_variant: error_variant,
        recovery,
        audit_logged: true,
        risk_level: risk_level.into(),
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split(['_', '-', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect()
}

// ── Async Boundary Description ─────────────────────────────────────────

fn describe_async_boundary(effect: &CanonicalEffect) -> String {
    match effect.async_boundary {
        AsyncBoundary::ThreadPool => "ThreadPool (short-lived async task)".into(),
        AsyncBoundary::DedicatedThread => "DedicatedThread (long-lived background worker)".into(),
        AsyncBoundary::Synchronous => "Synchronous (runs in update cycle)".into(),
    }
}

// ── Message Type Extraction ────────────────────────────────────────────

fn extract_message_types(effect: &CanonicalEffect) -> Vec<String> {
    match &effect.message_protocol {
        MessageProtocol::DataResult(name) => vec![name.clone()],
        MessageProtocol::Tick => vec!["Tick".into()],
        MessageProtocol::EventStream(name) => vec![name.clone()],
        MessageProtocol::Silent => vec![],
        MessageProtocol::Polymorphic(names) => names.clone(),
    }
}

// ── Certification Metadata ─────────────────────────────────────────────

fn build_certification_metadata(effect: &CanonicalEffect) -> CertificationMetadata {
    let state_impact: Vec<String> = effect.writes.iter().map(|id| id.0.clone()).collect();

    let risk_category = match effect.original_kind {
        EffectKind::Network => "network-io",
        EffectKind::Storage => "storage-io",
        EffectKind::Process => "process-spawn",
        EffectKind::Dom => "dom-mutation",
        EffectKind::Timer => "timer",
        EffectKind::Subscription => "event-stream",
        EffectKind::Telemetry => "telemetry",
        EffectKind::Other => "unknown",
    };

    CertificationMetadata {
        state_impact,
        timeout_observable: true,
        cancellation_observable: effect.cleanup != CleanupStrategy::None,
        error_observable: effect.execution_model != ExecutionModel::FireAndForget,
        risk_category: risk_category.into(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect_canonical::{ClassificationConfidence, TriggerCondition};
    use crate::migration_ir::{IrNodeId, Provenance};
    use std::collections::BTreeSet;

    fn make_effect(
        name: &str,
        kind: EffectKind,
        exec: ExecutionModel,
        cleanup: CleanupStrategy,
    ) -> CanonicalEffect {
        CanonicalEffect {
            id: IrNodeId(format!("eff-{name}")),
            name: name.into(),
            original_kind: kind,
            execution_model: exec,
            trigger: TriggerCondition::OnMount,
            message_protocol: MessageProtocol::DataResult("Result".into()),
            cleanup,
            async_boundary: AsyncBoundary::ThreadPool,
            dependencies: BTreeSet::new(),
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            idempotent: true,
            deterministic: true,
            confidence: ClassificationConfidence {
                score: 0.9,
                rationale: "test".into(),
            },
            provenance: Provenance {
                file: "test.tsx".into(),
                line: 1,
                column: None,
                source_name: None,
                policy_category: None,
            },
        }
    }

    fn make_model(effects: Vec<CanonicalEffect>) -> CanonicalEffectModel {
        let mut model = CanonicalEffectModel {
            effects: BTreeMap::new(),
            commands: BTreeSet::new(),
            subscriptions: BTreeSet::new(),
            fire_and_forget: BTreeSet::new(),
            ordering_constraints: Vec::new(),
            diagnostics: Vec::new(),
        };
        for eff in effects {
            match eff.execution_model {
                ExecutionModel::Command => {
                    model.commands.insert(eff.id.clone());
                }
                ExecutionModel::Subscription => {
                    model.subscriptions.insert(eff.id.clone());
                }
                ExecutionModel::FireAndForget => {
                    model.fire_and_forget.insert(eff.id.clone());
                }
            }
            model.effects.insert(eff.id.clone(), eff);
        }
        model
    }

    #[test]
    fn empty_model_produces_empty_plan() {
        let model = make_model(vec![]);
        let plan = translate_effects(&model);
        assert_eq!(plan.version, EFFECT_TRANSLATOR_VERSION);
        assert!(plan.orchestrations.is_empty());
        assert_eq!(plan.stats.total_effects, 0);
    }

    #[test]
    fn command_effect_generates_cmd_task() {
        let eff = make_effect(
            "fetch_data",
            EffectKind::Network,
            ExecutionModel::Command,
            CleanupStrategy::AbortInFlight,
        );
        let model = make_model(vec![eff]);
        let plan = translate_effects(&model);

        let orch = &plan.orchestrations["eff-fetch_data"];
        assert_eq!(orch.runtime_construct, RuntimeConstruct::CmdTask);
        assert_eq!(orch.timeout_ms, DEFAULT_TIMEOUT_NETWORK_MS);
        assert_eq!(plan.stats.commands, 1);
    }

    #[test]
    fn subscription_effect_generates_subscription() {
        let eff = make_effect(
            "websocket_stream",
            EffectKind::Subscription,
            ExecutionModel::Subscription,
            CleanupStrategy::SubscriptionStop,
        );
        let model = make_model(vec![eff]);
        let plan = translate_effects(&model);

        let orch = &plan.orchestrations["eff-websocket_stream"];
        assert_eq!(orch.runtime_construct, RuntimeConstruct::Subscription);
        assert_eq!(orch.timeout_ms, DEFAULT_TIMEOUT_SUBSCRIPTION_MS);
        assert_eq!(plan.stats.subscriptions, 1);
    }

    #[test]
    fn fire_and_forget_generates_cmd_fire_and_forget() {
        let eff = make_effect(
            "log_event",
            EffectKind::Telemetry,
            ExecutionModel::FireAndForget,
            CleanupStrategy::None,
        );
        let model = make_model(vec![eff]);
        let plan = translate_effects(&model);

        let orch = &plan.orchestrations["eff-log_event"];
        assert_eq!(orch.runtime_construct, RuntimeConstruct::CmdFireAndForget);
        assert_eq!(plan.stats.fire_and_forget, 1);
    }

    // ── Timeout Tests ──────────────────────────────────────────────────

    #[test]
    fn timeout_defaults_by_kind() {
        let kinds = vec![
            (EffectKind::Network, DEFAULT_TIMEOUT_NETWORK_MS),
            (EffectKind::Storage, DEFAULT_TIMEOUT_STORAGE_MS),
            (EffectKind::Dom, DEFAULT_TIMEOUT_DOM_MS),
            (EffectKind::Timer, DEFAULT_TIMEOUT_TIMER_MS),
            (EffectKind::Subscription, DEFAULT_TIMEOUT_SUBSCRIPTION_MS),
            (EffectKind::Process, DEFAULT_TIMEOUT_PROCESS_MS),
            (EffectKind::Telemetry, DEFAULT_TIMEOUT_TELEMETRY_MS),
            (EffectKind::Other, DEFAULT_TIMEOUT_OTHER_MS),
        ];

        for (kind, expected) in kinds {
            let eff = make_effect("test", kind, ExecutionModel::Command, CleanupStrategy::None);
            assert_eq!(infer_timeout(&eff), expected);
        }
    }

    // ── Cancellation Tests ─────────────────────────────────────────────

    #[test]
    fn cancellation_none_for_no_cleanup() {
        let eff = make_effect(
            "pure_cmd",
            EffectKind::Dom,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        let policy = build_cancellation_policy(&eff);
        assert_eq!(policy.kind, CancellationKind::None);
        assert!(!policy.observable);
    }

    #[test]
    fn cancellation_subscription_stop() {
        let eff = make_effect(
            "stream",
            EffectKind::Subscription,
            ExecutionModel::Subscription,
            CleanupStrategy::SubscriptionStop,
        );
        let policy = build_cancellation_policy(&eff);
        assert_eq!(policy.kind, CancellationKind::SubscriptionStop);
        assert!(policy.observable);
        assert!(policy.cleanup_description.is_some());
    }

    #[test]
    fn cancellation_abort_in_flight() {
        let eff = make_effect(
            "fetch",
            EffectKind::Network,
            ExecutionModel::Command,
            CleanupStrategy::AbortInFlight,
        );
        let policy = build_cancellation_policy(&eff);
        assert_eq!(policy.kind, CancellationKind::AbortInFlight);
        assert!(policy.observable);
        assert!(
            policy
                .cleanup_description
                .unwrap()
                .contains("AbortController")
        );
    }

    #[test]
    fn cancellation_explicit_cleanup() {
        let eff = make_effect(
            "dom_mutation",
            EffectKind::Dom,
            ExecutionModel::Command,
            CleanupStrategy::ExplicitAction,
        );
        let policy = build_cancellation_policy(&eff);
        assert_eq!(policy.kind, CancellationKind::ExplicitCleanup);
        assert!(policy.observable);
    }

    // ── Retry Tests ────────────────────────────────────────────────────

    #[test]
    fn retry_config_for_network_command() {
        let eff = make_effect(
            "api_call",
            EffectKind::Network,
            ExecutionModel::Command,
            CleanupStrategy::AbortInFlight,
        );
        let mut diags = Vec::new();
        let config = build_retry_config(&eff, &mut diags);
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.max_attempts, DEFAULT_MAX_RETRY_ATTEMPTS);
        assert!(config.deterministic);
        assert!(!config.non_retryable_conditions.is_empty());
    }

    #[test]
    fn no_retry_for_non_retryable_kinds() {
        let eff = make_effect(
            "dom_update",
            EffectKind::Dom,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        let mut diags = Vec::new();
        let config = build_retry_config(&eff, &mut diags);
        assert!(config.is_none());
    }

    #[test]
    fn no_retry_for_subscriptions() {
        let eff = make_effect(
            "ws_stream",
            EffectKind::Network,
            ExecutionModel::Subscription,
            CleanupStrategy::SubscriptionStop,
        );
        let mut diags = Vec::new();
        let config = build_retry_config(&eff, &mut diags);
        assert!(config.is_none());
    }

    #[test]
    fn non_idempotent_effect_gets_single_retry() {
        let mut eff = make_effect(
            "write_data",
            EffectKind::Storage,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        eff.idempotent = false;
        let mut diags = Vec::new();
        let config = build_retry_config(&eff, &mut diags).unwrap();
        assert_eq!(config.max_attempts, 1);
        assert_eq!(
            config.backoff,
            BackoffStrategy::Fixed {
                delay_ms: DEFAULT_RETRY_BASE_MS
            }
        );
    }

    #[test]
    fn nondeterministic_retry_emits_warning() {
        let mut eff = make_effect(
            "random_fetch",
            EffectKind::Network,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        eff.deterministic = false;
        let mut diags = Vec::new();
        build_retry_config(&eff, &mut diags);
        assert!(diags.iter().any(|d| d.code == "ET002"));
    }

    #[test]
    fn exponential_backoff_formula() {
        let backoff = BackoffStrategy::Exponential {
            base_ms: 1000,
            multiplier_x100: 200,
        };
        assert_eq!(compute_retry_delay(&backoff, 0), 1000); // 1000 * 2^0
        assert_eq!(compute_retry_delay(&backoff, 1), 2000); // 1000 * 2^1
        assert_eq!(compute_retry_delay(&backoff, 2), 4000); // 1000 * 2^2
        assert_eq!(compute_retry_delay(&backoff, 3), 8000); // 1000 * 2^3
    }

    #[test]
    fn linear_backoff_formula() {
        let backoff = BackoffStrategy::Linear { base_ms: 500 };
        assert_eq!(compute_retry_delay(&backoff, 0), 500); // 500 * 1
        assert_eq!(compute_retry_delay(&backoff, 1), 1000); // 500 * 2
        assert_eq!(compute_retry_delay(&backoff, 2), 1500); // 500 * 3
    }

    #[test]
    fn linear_backoff_saturates_on_overflow() {
        let backoff = BackoffStrategy::Linear { base_ms: u64::MAX };
        assert_eq!(compute_retry_delay(&backoff, 1), u64::MAX);
    }

    #[test]
    fn exponential_backoff_large_attempt_saturates_without_wrapping() {
        let backoff = BackoffStrategy::Exponential {
            base_ms: 1,
            multiplier_x100: 200,
        };
        assert_eq!(compute_retry_delay(&backoff, u32::MAX), u64::MAX);
    }

    #[test]
    fn fixed_backoff_formula() {
        let backoff = BackoffStrategy::Fixed { delay_ms: 1000 };
        assert_eq!(compute_retry_delay(&backoff, 0), 1000);
        assert_eq!(compute_retry_delay(&backoff, 5), 1000);
    }

    // ── Error Strategy Tests ───────────────────────────────────────────

    #[test]
    fn error_strategy_command_read_only() {
        let eff = make_effect(
            "get_data",
            EffectKind::Network,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        let strategy = build_error_strategy(&eff);
        assert_eq!(strategy.error_message_variant, "GetDataError");
        assert_eq!(strategy.recovery, RecoveryAction::Propagate);
        assert_eq!(strategy.risk_level, "high");
        assert!(strategy.audit_logged);
    }

    #[test]
    fn error_strategy_command_with_writes() {
        let mut eff = make_effect(
            "save_data",
            EffectKind::Storage,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        eff.writes.insert(IrNodeId("state-1".into()));
        let strategy = build_error_strategy(&eff);
        assert_eq!(strategy.recovery, RecoveryAction::RetryThenPropagate);
        assert_eq!(strategy.risk_level, "medium");
    }

    #[test]
    fn error_strategy_fire_and_forget() {
        let eff = make_effect(
            "track_event",
            EffectKind::Telemetry,
            ExecutionModel::FireAndForget,
            CleanupStrategy::None,
        );
        let strategy = build_error_strategy(&eff);
        assert_eq!(strategy.recovery, RecoveryAction::Ignore);
        assert_eq!(strategy.risk_level, "low");
    }

    #[test]
    fn error_strategy_subscription_fallback() {
        let eff = make_effect(
            "live_updates",
            EffectKind::Network,
            ExecutionModel::Subscription,
            CleanupStrategy::SubscriptionStop,
        );
        let strategy = build_error_strategy(&eff);
        assert!(matches!(strategy.recovery, RecoveryAction::Fallback { .. }));
        assert_eq!(strategy.risk_level, "high");
    }

    // ── Certification Metadata Tests ───────────────────────────────────

    #[test]
    fn certification_metadata_with_writes() {
        let mut eff = make_effect(
            "write_effect",
            EffectKind::Storage,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        eff.writes.insert(IrNodeId("var-a".into()));
        eff.writes.insert(IrNodeId("var-b".into()));
        let cert = build_certification_metadata(&eff);
        assert_eq!(cert.state_impact.len(), 2);
        assert!(cert.timeout_observable);
        assert!(cert.error_observable);
        assert_eq!(cert.risk_category, "storage-io");
    }

    #[test]
    fn certification_metadata_fire_and_forget() {
        let eff = make_effect(
            "log",
            EffectKind::Telemetry,
            ExecutionModel::FireAndForget,
            CleanupStrategy::None,
        );
        let cert = build_certification_metadata(&eff);
        assert!(!cert.error_observable);
        assert!(!cert.cancellation_observable);
        assert_eq!(cert.risk_category, "telemetry");
    }

    // ── Non-Determinism Flagging ───────────────────────────────────────

    #[test]
    fn nondeterministic_effect_flagged() {
        let mut eff = make_effect(
            "random_effect",
            EffectKind::Other,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        eff.deterministic = false;
        let model = make_model(vec![eff]);
        let plan = translate_effects(&model);
        assert_eq!(plan.stats.nondeterministic_flagged, 1);
        assert!(plan.diagnostics.iter().any(|d| d.code == "ET001"));
    }

    // ── Message Type Extraction ────────────────────────────────────────

    #[test]
    fn message_types_data_result() {
        let eff = make_effect(
            "fetch",
            EffectKind::Network,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        let types = extract_message_types(&eff);
        assert_eq!(types, vec!["Result".to_string()]);
    }

    #[test]
    fn message_types_silent() {
        let mut eff = make_effect(
            "log",
            EffectKind::Telemetry,
            ExecutionModel::FireAndForget,
            CleanupStrategy::None,
        );
        eff.message_protocol = MessageProtocol::Silent;
        let types = extract_message_types(&eff);
        assert!(types.is_empty());
    }

    #[test]
    fn message_types_polymorphic() {
        let mut eff = make_effect(
            "multi",
            EffectKind::Other,
            ExecutionModel::Command,
            CleanupStrategy::None,
        );
        eff.message_protocol = MessageProtocol::Polymorphic(vec!["TypeA".into(), "TypeB".into()]);
        let types = extract_message_types(&eff);
        assert_eq!(types, vec!["TypeA".to_string(), "TypeB".to_string()]);
    }

    // ── PascalCase Conversion ──────────────────────────────────────────

    #[test]
    fn pascal_case_conversion() {
        assert_eq!(to_pascal_case("fetch_data"), "FetchData");
        assert_eq!(to_pascal_case("api-call"), "ApiCall");
        assert_eq!(to_pascal_case("simple"), "Simple");
        assert_eq!(to_pascal_case("multi word name"), "MultiWordName");
    }

    // ── Full Pipeline Tests ────────────────────────────────────────────

    #[test]
    fn full_pipeline_mixed_effects() {
        let effects = vec![
            make_effect(
                "api_call",
                EffectKind::Network,
                ExecutionModel::Command,
                CleanupStrategy::AbortInFlight,
            ),
            make_effect(
                "ws_stream",
                EffectKind::Subscription,
                ExecutionModel::Subscription,
                CleanupStrategy::SubscriptionStop,
            ),
            make_effect(
                "analytics",
                EffectKind::Telemetry,
                ExecutionModel::FireAndForget,
                CleanupStrategy::None,
            ),
        ];
        let model = make_model(effects);
        let plan = translate_effects(&model);

        assert_eq!(plan.stats.total_effects, 3);
        assert_eq!(plan.stats.commands, 1);
        assert_eq!(plan.stats.subscriptions, 1);
        assert_eq!(plan.stats.fire_and_forget, 1);
        assert_eq!(plan.stats.with_retry, 1); // Only network command.
        assert_eq!(plan.stats.with_cancellation, 2); // Abort + subscription stop.
    }

    #[test]
    fn translation_is_deterministic() {
        let effects = vec![
            make_effect(
                "effect_a",
                EffectKind::Network,
                ExecutionModel::Command,
                CleanupStrategy::None,
            ),
            make_effect(
                "effect_b",
                EffectKind::Timer,
                ExecutionModel::Command,
                CleanupStrategy::None,
            ),
        ];
        let model = make_model(effects);
        let p1 = translate_effects(&model);
        let p2 = translate_effects(&model);

        let j1 = serde_json::to_string(&p1).unwrap();
        let j2 = serde_json::to_string(&p2).unwrap();
        assert_eq!(j1, j2);
    }

    #[test]
    fn ordering_constraints_preserved() {
        let mut model = make_model(vec![make_effect(
            "eff1",
            EffectKind::Dom,
            ExecutionModel::Command,
            CleanupStrategy::None,
        )]);
        model.ordering_constraints.push(OrderingConstraint {
            before: IrNodeId("a".into()),
            after: IrNodeId("b".into()),
            reason: "test".into(),
        });
        let plan = translate_effects(&model);
        assert_eq!(plan.ordering_constraints.len(), 1);
        assert_eq!(plan.ordering_constraints[0].before, IrNodeId("a".into()));
    }
}
