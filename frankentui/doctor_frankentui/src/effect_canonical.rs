// SPDX-License-Identifier: Apache-2.0
//! Canonicalize effects into deterministic command/subscription model.
//!
//! Transforms extracted effect semantics from the IR into a deterministic
//! representation aligned with FrankenTUI's Elm-inspired execution model:
//!
//! - **Commands** (one-shot): executed once per trigger, return result as message.
//! - **Subscriptions** (continuous): run until deactivated, emit messages on events.
//!
//! Each canonical effect has explicit trigger conditions, message protocol,
//! cleanup semantics, and async boundaries — enabling deterministic replay
//! and certification diffing.
//!
//! # Pipeline position
//! ```text
//!   EffectRegistry (raw IR) → canonicalize_effects() → CanonicalEffectModel
//! ```

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::migration_ir::{EffectDecl, EffectKind, EffectRegistry, IrNodeId, Provenance};

// ── Core Types ───────────────────────────────────────────────────────────

/// The execution model for a canonical effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ExecutionModel {
    /// One-shot command: spawned by `update()`, returns a single message.
    /// Maps to `Cmd::Task` in ftui-runtime.
    Command,
    /// Continuous subscription: declared by `subscriptions()`, emits messages
    /// until deactivated. Maps to `Subscription<M>` in ftui-runtime.
    Subscription,
    /// Fire-and-forget: no result message needed (logging, analytics).
    /// Maps to `Cmd::Task` with unit return or `Cmd::Log`.
    FireAndForget,
}

/// When the effect should trigger or re-trigger.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TriggerCondition {
    /// Triggered once on mount (empty dependency array `[]`).
    OnMount,
    /// Re-triggered when specific state dependencies change.
    OnDependencyChange(BTreeSet<IrNodeId>),
    /// Re-triggered on every render (no dependency array).
    OnEveryRender,
    /// Triggered by explicit user action (event handler dispatch).
    OnUserAction(String),
}

/// How the effect's cleanup phase maps to the runtime model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CleanupStrategy {
    /// No cleanup needed (pure command, fire-and-forget).
    None,
    /// Subscription stops when deactivated (runtime calls `StopSignal`).
    SubscriptionStop,
    /// Explicit cleanup action required (e.g., remove DOM node, close handle).
    ExplicitAction,
    /// Abort in-flight command (cancel network request, clear timer).
    AbortInFlight,
}

/// The async execution boundary for this effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AsyncBoundary {
    /// Runs in the thread pool (short-lived, returns quickly).
    ThreadPool,
    /// Runs on a dedicated background thread (long-lived subscription).
    DedicatedThread,
    /// Synchronous (no async boundary, runs in update cycle).
    Synchronous,
}

/// Describes what message the effect produces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MessageProtocol {
    /// Returns typed data (fetch result, measurement, etc.).
    DataResult(String),
    /// Emits periodic tick messages.
    Tick,
    /// Emits event-driven messages (listener, subscription).
    EventStream(String),
    /// No message produced (fire-and-forget).
    Silent,
    /// Multiple message types possible.
    Polymorphic(Vec<String>),
}

/// Confidence in the classification (0.0–1.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationConfidence {
    pub score: f64,
    pub rationale: String,
}

/// A canonicalized effect with full execution semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalEffect {
    /// Original IR effect ID.
    pub id: IrNodeId,
    /// Human-readable name.
    pub name: String,
    /// Original effect kind (preserved for traceability).
    pub original_kind: EffectKind,
    /// Canonical execution model.
    pub execution_model: ExecutionModel,
    /// When this effect triggers.
    pub trigger: TriggerCondition,
    /// What message protocol it uses.
    pub message_protocol: MessageProtocol,
    /// How cleanup is handled.
    pub cleanup: CleanupStrategy,
    /// Async execution boundary.
    pub async_boundary: AsyncBoundary,
    /// State dependencies that trigger re-execution.
    pub dependencies: BTreeSet<IrNodeId>,
    /// State variables read by this effect.
    pub reads: BTreeSet<IrNodeId>,
    /// State variables written by this effect.
    pub writes: BTreeSet<IrNodeId>,
    /// Whether this effect is idempotent (safe to re-execute).
    pub idempotent: bool,
    /// Whether this effect is deterministic (same inputs → same outputs).
    pub deterministic: bool,
    /// Classification confidence.
    pub confidence: ClassificationConfidence,
    /// Provenance link to source.
    pub provenance: Provenance,
}

/// The complete canonical effect model for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalEffectModel {
    /// All canonicalized effects, keyed by their IR node ID.
    pub effects: BTreeMap<IrNodeId, CanonicalEffect>,
    /// Effects classified as commands.
    pub commands: BTreeSet<IrNodeId>,
    /// Effects classified as subscriptions.
    pub subscriptions: BTreeSet<IrNodeId>,
    /// Effects classified as fire-and-forget.
    pub fire_and_forget: BTreeSet<IrNodeId>,
    /// Ordering constraints between effects (effect A must complete before B).
    pub ordering_constraints: Vec<OrderingConstraint>,
    /// Diagnostics from canonicalization.
    pub diagnostics: Vec<CanonDiagnostic>,
}

/// An ordering constraint between two effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderingConstraint {
    /// Effect that must complete first.
    pub before: IrNodeId,
    /// Effect that depends on the first.
    pub after: IrNodeId,
    /// Reason for the ordering.
    pub reason: String,
}

/// Diagnostic from the canonicalization pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonDiagnostic {
    pub effect_id: IrNodeId,
    pub code: String,
    pub message: String,
    pub severity: DiagnosticSeverity,
}

/// Severity of a canonicalization diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

// ── Public API ───────────────────────────────────────────────────────────

/// Canonicalize all effects in an EffectRegistry into the command/subscription model.
pub fn canonicalize_effects(registry: &EffectRegistry) -> CanonicalEffectModel {
    let mut effects = BTreeMap::new();
    let mut commands = BTreeSet::new();
    let mut subscriptions = BTreeSet::new();
    let mut fire_and_forget = BTreeSet::new();
    let mut diagnostics = Vec::new();

    for (id, decl) in &registry.effects {
        let canonical = canonicalize_single(decl, &mut diagnostics);

        match canonical.execution_model {
            ExecutionModel::Command => {
                commands.insert(id.clone());
            }
            ExecutionModel::Subscription => {
                subscriptions.insert(id.clone());
            }
            ExecutionModel::FireAndForget => {
                fire_and_forget.insert(id.clone());
            }
        }

        effects.insert(id.clone(), canonical);
    }

    let ordering_constraints = infer_ordering_constraints(&effects);

    CanonicalEffectModel {
        effects,
        commands,
        subscriptions,
        fire_and_forget,
        ordering_constraints,
        diagnostics,
    }
}

/// Verify that a canonical model satisfies replay determinism invariants.
pub fn verify_determinism(model: &CanonicalEffectModel) -> Vec<CanonDiagnostic> {
    let mut diagnostics = Vec::new();

    for (id, effect) in &model.effects {
        // Non-deterministic effects need special handling for replay.
        if !effect.deterministic {
            diagnostics.push(CanonDiagnostic {
                effect_id: id.clone(),
                code: "C001".to_string(),
                message: format!(
                    "Effect '{}' is non-deterministic; replay requires recorded outputs",
                    effect.name,
                ),
                severity: DiagnosticSeverity::Warning,
            });
        }

        // Effects that write state without reading create implicit ordering.
        if !effect.writes.is_empty() && effect.reads.is_empty() {
            diagnostics.push(CanonDiagnostic {
                effect_id: id.clone(),
                code: "C002".to_string(),
                message: format!(
                    "Effect '{}' writes state without reading; may create ordering ambiguity",
                    effect.name,
                ),
                severity: DiagnosticSeverity::Info,
            });
        }

        // Subscriptions without cleanup risk resource leaks.
        if effect.execution_model == ExecutionModel::Subscription
            && effect.cleanup == CleanupStrategy::None
        {
            diagnostics.push(CanonDiagnostic {
                effect_id: id.clone(),
                code: "C003".to_string(),
                message: format!(
                    "Subscription '{}' has no cleanup strategy; may leak resources",
                    effect.name,
                ),
                severity: DiagnosticSeverity::Warning,
            });
        }
    }

    diagnostics
}

/// Compute execution order for a set of effects respecting ordering constraints.
pub fn compute_execution_order(model: &CanonicalEffectModel) -> Vec<IrNodeId> {
    // Topological sort based on ordering constraints.
    let mut in_degree: BTreeMap<IrNodeId, usize> = BTreeMap::new();
    let mut graph: BTreeMap<IrNodeId, Vec<IrNodeId>> = BTreeMap::new();

    for id in model.effects.keys() {
        in_degree.entry(id.clone()).or_insert(0);
        graph.entry(id.clone()).or_default();
    }

    for constraint in &model.ordering_constraints {
        graph
            .entry(constraint.before.clone())
            .or_default()
            .push(constraint.after.clone());
        *in_degree.entry(constraint.after.clone()).or_insert(0) += 1;
    }

    let mut queue: Vec<IrNodeId> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(id, _)| id.clone())
        .collect();

    // BTreeMap iteration is already sorted, so queue starts sorted.
    let mut result = Vec::new();
    while let Some(id) = queue.first().cloned() {
        queue.remove(0);
        result.push(id.clone());

        if let Some(neighbors) = graph.get(&id) {
            for neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(neighbor) {
                    *deg -= 1;
                    if *deg == 0 {
                        // Insert in sorted position for determinism.
                        let pos = queue.binary_search(neighbor).unwrap_or_else(|pos| pos);
                        queue.insert(pos, neighbor.clone());
                    }
                }
            }
        }
    }

    result
}

// ── Internal Classification ──────────────────────────────────────────────

fn canonicalize_single(
    decl: &EffectDecl,
    diagnostics: &mut Vec<CanonDiagnostic>,
) -> CanonicalEffect {
    let execution_model = classify_execution_model(decl);
    let trigger = classify_trigger(decl);
    let message_protocol = classify_message_protocol(decl, &execution_model);
    let cleanup = classify_cleanup(decl, &execution_model);
    let async_boundary = classify_async_boundary(&execution_model);
    let idempotent = classify_idempotent(decl);
    let deterministic = classify_deterministic(decl);
    let confidence = compute_confidence(decl, &execution_model);

    // Emit diagnostics for ambiguous cases.
    if confidence.score < 0.5 {
        diagnostics.push(CanonDiagnostic {
            effect_id: decl.id.clone(),
            code: "C010".to_string(),
            message: format!(
                "Low confidence ({:.2}) classifying '{}' as {:?}: {}",
                confidence.score, decl.name, execution_model, confidence.rationale,
            ),
            severity: DiagnosticSeverity::Warning,
        });
    }

    CanonicalEffect {
        id: decl.id.clone(),
        name: decl.name.clone(),
        original_kind: decl.kind.clone(),
        execution_model,
        trigger,
        message_protocol,
        cleanup,
        async_boundary,
        dependencies: decl.dependencies.clone(),
        reads: decl.reads.clone(),
        writes: decl.writes.clone(),
        idempotent,
        deterministic,
        confidence,
        provenance: decl.provenance.clone(),
    }
}

fn classify_execution_model(decl: &EffectDecl) -> ExecutionModel {
    match decl.kind {
        // Continuous: timers, subscriptions, event listeners, long-running processes.
        EffectKind::Timer => ExecutionModel::Subscription,
        EffectKind::Subscription => ExecutionModel::Subscription,
        EffectKind::Process => ExecutionModel::Subscription,

        // One-shot: network requests, DOM measurements, storage operations.
        EffectKind::Network => ExecutionModel::Command,
        EffectKind::Dom => ExecutionModel::Command,
        EffectKind::Storage => ExecutionModel::Command,

        // Fire-and-forget: telemetry, logging.
        EffectKind::Telemetry => ExecutionModel::FireAndForget,

        // Other: default to command (safest).
        EffectKind::Other => {
            if decl.has_cleanup {
                // Cleanup suggests ongoing lifecycle → subscription.
                ExecutionModel::Subscription
            } else if decl.writes.is_empty() && decl.reads.is_empty() {
                // No state interaction → fire-and-forget.
                ExecutionModel::FireAndForget
            } else {
                ExecutionModel::Command
            }
        }
    }
}

fn classify_trigger(decl: &EffectDecl) -> TriggerCondition {
    if decl.dependencies.is_empty() {
        // Empty deps → mount-only (like `useEffect(() => ..., [])`)
        TriggerCondition::OnMount
    } else {
        TriggerCondition::OnDependencyChange(decl.dependencies.clone())
    }
}

fn classify_message_protocol(decl: &EffectDecl, model: &ExecutionModel) -> MessageProtocol {
    match model {
        ExecutionModel::FireAndForget => MessageProtocol::Silent,
        ExecutionModel::Command => match decl.kind {
            EffectKind::Network => MessageProtocol::DataResult("FetchResponse".to_string()),
            EffectKind::Dom => MessageProtocol::DataResult("DomMeasurement".to_string()),
            EffectKind::Storage => MessageProtocol::DataResult("StorageResult".to_string()),
            _ => MessageProtocol::DataResult("CommandResult".to_string()),
        },
        ExecutionModel::Subscription => match decl.kind {
            EffectKind::Timer => MessageProtocol::Tick,
            EffectKind::Subscription => {
                MessageProtocol::EventStream("SubscriptionEvent".to_string())
            }
            EffectKind::Process => MessageProtocol::EventStream("ProcessEvent".to_string()),
            _ => MessageProtocol::EventStream("EffectEvent".to_string()),
        },
    }
}

fn classify_cleanup(decl: &EffectDecl, model: &ExecutionModel) -> CleanupStrategy {
    if !decl.has_cleanup {
        return CleanupStrategy::None;
    }

    match model {
        ExecutionModel::Subscription => CleanupStrategy::SubscriptionStop,
        ExecutionModel::Command => match decl.kind {
            EffectKind::Network => CleanupStrategy::AbortInFlight,
            _ => CleanupStrategy::ExplicitAction,
        },
        ExecutionModel::FireAndForget => CleanupStrategy::None,
    }
}

fn classify_async_boundary(model: &ExecutionModel) -> AsyncBoundary {
    match model {
        ExecutionModel::Command => AsyncBoundary::ThreadPool,
        ExecutionModel::Subscription => AsyncBoundary::DedicatedThread,
        ExecutionModel::FireAndForget => AsyncBoundary::Synchronous,
    }
}

fn classify_idempotent(decl: &EffectDecl) -> bool {
    match decl.kind {
        // Network requests may not be idempotent (POST, DELETE).
        EffectKind::Network => false,
        // DOM reads are idempotent, writes may not be.
        EffectKind::Dom => decl.writes.is_empty(),
        // Storage writes are idempotent (overwrite semantics).
        EffectKind::Storage => true,
        // Telemetry is fire-and-forget, considered idempotent.
        EffectKind::Telemetry => true,
        // Timers and subscriptions: re-subscribing is idempotent if properly cleaned up.
        EffectKind::Timer | EffectKind::Subscription => decl.has_cleanup,
        // Process spawn is not idempotent.
        EffectKind::Process => false,
        // Unknown: assume not idempotent (conservative).
        EffectKind::Other => false,
    }
}

fn classify_deterministic(decl: &EffectDecl) -> bool {
    match decl.kind {
        // Network: non-deterministic (depends on server state).
        EffectKind::Network => false,
        // DOM: non-deterministic (depends on layout engine).
        EffectKind::Dom => false,
        // Timer: deterministic if intervals are fixed.
        EffectKind::Timer => true,
        // Storage: deterministic (same key → same value).
        EffectKind::Storage => true,
        // Subscription: non-deterministic (event timing varies).
        EffectKind::Subscription => false,
        // Process: non-deterministic.
        EffectKind::Process => false,
        // Telemetry: fire-and-forget, considered deterministic (no observable output).
        EffectKind::Telemetry => true,
        // Unknown: assume non-deterministic (conservative).
        EffectKind::Other => false,
    }
}

fn compute_confidence(decl: &EffectDecl, model: &ExecutionModel) -> ClassificationConfidence {
    let mut score = 0.7_f64; // Base confidence from kind-based classification.
    let mut reasons = Vec::new();

    // Higher confidence for well-understood kinds.
    match decl.kind {
        EffectKind::Timer => {
            score += 0.2;
            reasons.push("Timer effects map directly to Subscription".to_string());
        }
        EffectKind::Network => {
            score += 0.15;
            reasons.push("Network effects map to Cmd::Task".to_string());
        }
        EffectKind::Telemetry => {
            score += 0.2;
            reasons.push("Telemetry is clearly fire-and-forget".to_string());
        }
        EffectKind::Other => {
            score -= 0.3;
            reasons.push("Unknown effect kind, classification uncertain".to_string());
        }
        _ => {
            reasons.push(format!("{:?} kind has standard mapping", decl.kind));
        }
    }

    // Cleanup presence strengthens subscription classification.
    if decl.has_cleanup && *model == ExecutionModel::Subscription {
        score += 0.1;
        reasons.push("Cleanup function confirms subscription lifecycle".to_string());
    }

    // Dependencies strengthen trigger classification confidence.
    if !decl.dependencies.is_empty() {
        score += 0.05;
        reasons.push(format!("{} explicit dependencies", decl.dependencies.len()));
    }

    let score = score.min(1.0);

    ClassificationConfidence {
        score,
        rationale: reasons.join("; "),
    }
}

fn infer_ordering_constraints(
    effects: &BTreeMap<IrNodeId, CanonicalEffect>,
) -> Vec<OrderingConstraint> {
    let mut constraints = Vec::new();

    // Build write→read dependency: if effect A writes state X and effect B reads X,
    // then A should execute before B.
    let mut writers: BTreeMap<&IrNodeId, Vec<&IrNodeId>> = BTreeMap::new();
    for (id, effect) in effects {
        for write in &effect.writes {
            writers.entry(write).or_default().push(id);
        }
    }

    for (id, effect) in effects {
        for read in &effect.reads {
            if let Some(write_effects) = writers.get(read) {
                for writer_id in write_effects {
                    if *writer_id != id {
                        constraints.push(OrderingConstraint {
                            before: (*writer_id).clone(),
                            after: id.clone(),
                            reason: format!(
                                "Effect '{}' writes state that '{}' reads",
                                effects
                                    .get(*writer_id)
                                    .map(|e| e.name.as_str())
                                    .unwrap_or("?"),
                                effect.name,
                            ),
                        });
                    }
                }
            }
        }
    }

    constraints
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration_ir::{EffectDecl, EffectKind, EffectRegistry, IrNodeId, Provenance};

    fn test_provenance() -> Provenance {
        Provenance {
            file: "test.tsx".into(),
            line: 1,
            column: None,
            source_name: None,
            policy_category: None,
        }
    }

    fn make_effect(name: &str, kind: EffectKind, has_cleanup: bool) -> EffectDecl {
        EffectDecl {
            id: IrNodeId(format!("ir-{name}")),
            name: name.to_string(),
            kind,
            dependencies: BTreeSet::new(),
            has_cleanup,
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            provenance: test_provenance(),
        }
    }

    fn make_registry(effects: Vec<EffectDecl>) -> EffectRegistry {
        let mut map = BTreeMap::new();
        for e in effects {
            map.insert(e.id.clone(), e);
        }
        EffectRegistry { effects: map }
    }

    #[test]
    fn timer_classified_as_subscription() {
        let reg = make_registry(vec![make_effect("timer", EffectKind::Timer, true)]);
        let model = canonicalize_effects(&reg);

        assert_eq!(model.subscriptions.len(), 1);
        assert_eq!(model.commands.len(), 0);

        let effect = &model.effects[&IrNodeId("ir-timer".into())];
        assert_eq!(effect.execution_model, ExecutionModel::Subscription);
        assert_eq!(effect.message_protocol, MessageProtocol::Tick);
        assert_eq!(effect.cleanup, CleanupStrategy::SubscriptionStop);
        assert_eq!(effect.async_boundary, AsyncBoundary::DedicatedThread);
    }

    #[test]
    fn network_classified_as_command() {
        let reg = make_registry(vec![make_effect("fetch", EffectKind::Network, false)]);
        let model = canonicalize_effects(&reg);

        assert_eq!(model.commands.len(), 1);
        let effect = &model.effects[&IrNodeId("ir-fetch".into())];
        assert_eq!(effect.execution_model, ExecutionModel::Command);
        assert_eq!(
            effect.message_protocol,
            MessageProtocol::DataResult("FetchResponse".to_string()),
        );
        assert_eq!(effect.cleanup, CleanupStrategy::None);
        assert_eq!(effect.async_boundary, AsyncBoundary::ThreadPool);
    }

    #[test]
    fn network_with_cleanup_has_abort() {
        let reg = make_registry(vec![make_effect("fetch-abort", EffectKind::Network, true)]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-fetch-abort".into())];
        assert_eq!(effect.cleanup, CleanupStrategy::AbortInFlight);
    }

    #[test]
    fn telemetry_classified_as_fire_and_forget() {
        let reg = make_registry(vec![make_effect("analytics", EffectKind::Telemetry, false)]);
        let model = canonicalize_effects(&reg);

        assert_eq!(model.fire_and_forget.len(), 1);
        let effect = &model.effects[&IrNodeId("ir-analytics".into())];
        assert_eq!(effect.execution_model, ExecutionModel::FireAndForget);
        assert_eq!(effect.message_protocol, MessageProtocol::Silent);
        assert_eq!(effect.async_boundary, AsyncBoundary::Synchronous);
    }

    #[test]
    fn subscription_kind_classified_as_subscription() {
        let reg = make_registry(vec![make_effect("resize", EffectKind::Subscription, true)]);
        let model = canonicalize_effects(&reg);

        assert_eq!(model.subscriptions.len(), 1);
        let effect = &model.effects[&IrNodeId("ir-resize".into())];
        assert_eq!(effect.execution_model, ExecutionModel::Subscription);
        assert_eq!(
            effect.message_protocol,
            MessageProtocol::EventStream("SubscriptionEvent".to_string()),
        );
    }

    #[test]
    fn dom_classified_as_command() {
        let reg = make_registry(vec![make_effect("measure", EffectKind::Dom, false)]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-measure".into())];
        assert_eq!(effect.execution_model, ExecutionModel::Command);
        assert!(effect.idempotent); // DOM reads are idempotent.
    }

    #[test]
    fn storage_classified_as_command() {
        let reg = make_registry(vec![make_effect("persist", EffectKind::Storage, false)]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-persist".into())];
        assert_eq!(effect.execution_model, ExecutionModel::Command);
        assert!(effect.idempotent); // Storage writes are idempotent.
        assert!(effect.deterministic); // Same key → same result.
    }

    #[test]
    fn process_classified_as_subscription() {
        let reg = make_registry(vec![make_effect("worker", EffectKind::Process, true)]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-worker".into())];
        assert_eq!(effect.execution_model, ExecutionModel::Subscription);
        assert!(!effect.idempotent);
        assert!(!effect.deterministic);
    }

    #[test]
    fn other_with_cleanup_becomes_subscription() {
        let reg = make_registry(vec![make_effect("custom", EffectKind::Other, true)]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-custom".into())];
        assert_eq!(effect.execution_model, ExecutionModel::Subscription);
    }

    #[test]
    fn other_without_state_becomes_fire_and_forget() {
        let reg = make_registry(vec![make_effect("noop", EffectKind::Other, false)]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-noop".into())];
        assert_eq!(effect.execution_model, ExecutionModel::FireAndForget);
    }

    #[test]
    fn other_with_writes_becomes_command() {
        let mut decl = make_effect("writer", EffectKind::Other, false);
        decl.writes.insert(IrNodeId("ir-state-x".into()));
        let reg = make_registry(vec![decl]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-writer".into())];
        assert_eq!(effect.execution_model, ExecutionModel::Command);
    }

    #[test]
    fn empty_deps_trigger_on_mount() {
        let reg = make_registry(vec![make_effect("mount", EffectKind::Network, false)]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-mount".into())];
        assert_eq!(effect.trigger, TriggerCondition::OnMount);
    }

    #[test]
    fn deps_trigger_on_change() {
        let mut decl = make_effect("watcher", EffectKind::Network, false);
        decl.dependencies.insert(IrNodeId("ir-dep-a".into()));
        let reg = make_registry(vec![decl]);
        let model = canonicalize_effects(&reg);

        let effect = &model.effects[&IrNodeId("ir-watcher".into())];
        assert!(matches!(
            effect.trigger,
            TriggerCondition::OnDependencyChange(_)
        ));
        if let TriggerCondition::OnDependencyChange(deps) = &effect.trigger {
            assert!(deps.contains(&IrNodeId("ir-dep-a".into())));
        }
    }

    #[test]
    fn ordering_constraints_from_write_read() {
        let mut writer = make_effect("writer", EffectKind::Network, false);
        writer.writes.insert(IrNodeId("ir-shared".into()));

        let mut reader = make_effect("reader", EffectKind::Dom, false);
        reader.reads.insert(IrNodeId("ir-shared".into()));

        let reg = make_registry(vec![writer, reader]);
        let model = canonicalize_effects(&reg);

        assert_eq!(model.ordering_constraints.len(), 1);
        assert_eq!(
            model.ordering_constraints[0].before,
            IrNodeId("ir-writer".into())
        );
        assert_eq!(
            model.ordering_constraints[0].after,
            IrNodeId("ir-reader".into())
        );
    }

    #[test]
    fn no_self_ordering_constraint() {
        let mut both = make_effect("both", EffectKind::Storage, false);
        both.writes.insert(IrNodeId("ir-x".into()));
        both.reads.insert(IrNodeId("ir-x".into()));

        let reg = make_registry(vec![both]);
        let model = canonicalize_effects(&reg);

        assert!(model.ordering_constraints.is_empty());
    }

    #[test]
    fn execution_order_deterministic() {
        let e1 = make_effect("alpha", EffectKind::Network, false);
        let e2 = make_effect("beta", EffectKind::Timer, true);
        let e3 = make_effect("gamma", EffectKind::Telemetry, false);

        let reg = make_registry(vec![e1, e2, e3]);
        let model = canonicalize_effects(&reg);

        let order1 = compute_execution_order(&model);
        let order2 = compute_execution_order(&model);
        assert_eq!(order1, order2);
        assert_eq!(order1.len(), 3);
    }

    #[test]
    fn execution_order_respects_constraints() {
        let mut writer = make_effect("aaa-writer", EffectKind::Network, false);
        writer.writes.insert(IrNodeId("ir-shared".into()));

        let mut reader = make_effect("bbb-reader", EffectKind::Dom, false);
        reader.reads.insert(IrNodeId("ir-shared".into()));

        let reg = make_registry(vec![reader, writer]);
        let model = canonicalize_effects(&reg);
        let order = compute_execution_order(&model);

        let writer_pos = order.iter().position(|id| id.0 == "ir-aaa-writer").unwrap();
        let reader_pos = order.iter().position(|id| id.0 == "ir-bbb-reader").unwrap();
        assert!(writer_pos < reader_pos, "Writer must come before reader");
    }

    #[test]
    fn verify_determinism_warns_on_nondeterministic() {
        let reg = make_registry(vec![make_effect("fetch", EffectKind::Network, false)]);
        let model = canonicalize_effects(&reg);
        let diags = verify_determinism(&model);

        assert!(diags.iter().any(|d| d.code == "C001"));
    }

    #[test]
    fn verify_determinism_warns_on_subscription_no_cleanup() {
        let reg = make_registry(vec![make_effect("sub", EffectKind::Subscription, false)]);
        let model = canonicalize_effects(&reg);
        let diags = verify_determinism(&model);

        assert!(diags.iter().any(|d| d.code == "C003"));
    }

    #[test]
    fn low_confidence_emits_diagnostic() {
        let reg = make_registry(vec![make_effect("unknown", EffectKind::Other, false)]);
        let model = canonicalize_effects(&reg);

        assert!(model.diagnostics.iter().any(|d| d.code == "C010"));
    }

    #[test]
    fn mixed_model_classification() {
        let reg = make_registry(vec![
            make_effect("fetch", EffectKind::Network, false),
            make_effect("timer", EffectKind::Timer, true),
            make_effect("log", EffectKind::Telemetry, false),
            make_effect("listen", EffectKind::Subscription, true),
            make_effect("measure", EffectKind::Dom, false),
        ]);

        let model = canonicalize_effects(&reg);

        assert_eq!(model.commands.len(), 2); // fetch + measure
        assert_eq!(model.subscriptions.len(), 2); // timer + listen
        assert_eq!(model.fire_and_forget.len(), 1); // log
    }

    #[test]
    fn confidence_higher_for_known_kinds() {
        let timer = make_effect("timer", EffectKind::Timer, true);
        let unknown = make_effect("unknown", EffectKind::Other, false);

        let reg = make_registry(vec![timer, unknown]);
        let model = canonicalize_effects(&reg);

        let timer_conf = model.effects[&IrNodeId("ir-timer".into())].confidence.score;
        let unknown_conf = model.effects[&IrNodeId("ir-unknown".into())]
            .confidence
            .score;

        assert!(
            timer_conf > unknown_conf,
            "Timer ({timer_conf}) should have higher confidence than Other ({unknown_conf})"
        );
    }

    #[test]
    fn all_effects_preserve_provenance() {
        let reg = make_registry(vec![
            make_effect("e1", EffectKind::Network, false),
            make_effect("e2", EffectKind::Timer, true),
        ]);

        let model = canonicalize_effects(&reg);

        for effect in model.effects.values() {
            assert_eq!(effect.provenance.file, "test.tsx");
        }
    }

    #[test]
    fn empty_registry_produces_empty_model() {
        let reg = EffectRegistry {
            effects: BTreeMap::new(),
        };
        let model = canonicalize_effects(&reg);

        assert!(model.effects.is_empty());
        assert!(model.commands.is_empty());
        assert!(model.subscriptions.is_empty());
        assert!(model.fire_and_forget.is_empty());
        assert!(model.ordering_constraints.is_empty());
        assert!(model.diagnostics.is_empty());
    }

    #[test]
    fn network_is_not_deterministic() {
        let reg = make_registry(vec![make_effect("fetch", EffectKind::Network, false)]);
        let model = canonicalize_effects(&reg);
        let effect = &model.effects[&IrNodeId("ir-fetch".into())];
        assert!(!effect.deterministic);
        assert!(!effect.idempotent);
    }

    #[test]
    fn timer_is_deterministic() {
        let reg = make_registry(vec![make_effect("tick", EffectKind::Timer, true)]);
        let model = canonicalize_effects(&reg);
        let effect = &model.effects[&IrNodeId("ir-tick".into())];
        assert!(effect.deterministic);
        assert!(effect.idempotent); // has cleanup
    }
}
