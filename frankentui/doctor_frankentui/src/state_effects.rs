// SPDX-License-Identifier: Apache-2.0
//! State machine and effects semantic extraction.
//!
//! Recovers state transitions, reducer patterns, derived state, and effect
//! dependencies from hooks and event handlers. Produces a serializable
//! state-effects model that maps into FrankenTUI Model/update/subscriptions.
//!
//! Consumes output from `tsx_parser` and `module_graph`.

use std::collections::{BTreeMap, BTreeSet};

use regex_lite::Regex;
use serde::{Deserialize, Serialize};

use crate::migration_ir::{Capability, PlatformAssumption};
use crate::tsx_parser::{ComponentDecl, FileParse, HookCall, JsxElement};

// ── Types ────────────────────────────────────────────────────────────────

/// Complete state-effects analysis result for a component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentStateModel {
    pub component_name: String,
    pub file: String,
    pub state_vars: Vec<StateVar>,
    pub reducers: Vec<ReducerPattern>,
    pub derived: Vec<DerivedComputation>,
    pub effects: Vec<EffectBinding>,
    pub event_transitions: Vec<EventStateTransition>,
    pub context_consumers: Vec<ContextConsumer>,
    pub context_providers: Vec<ContextProvider>,
}

/// A state variable extracted from hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateVar {
    pub name: String,
    pub setter: Option<String>,
    pub hook: String,
    pub initial_value: Option<String>,
    pub scope: StateVarScope,
    pub type_hint: Option<String>,
    pub line: usize,
}

/// Scope of a state variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateVarScope {
    /// Component-local useState.
    Local,
    /// useReducer pattern.
    Reducer,
    /// useRef (mutable ref, not reactive).
    Ref,
    /// useContext consumer.
    Context,
    /// External store (Zustand, Redux, Jotai, etc.).
    ExternalStore,
    /// URL/search params.
    Url,
    /// Server state (React Query, SWR).
    Server,
}

/// A reducer pattern (useReducer or dispatch-based).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReducerPattern {
    pub state_name: String,
    pub dispatch_name: String,
    pub reducer_name: Option<String>,
    pub initial_state: Option<String>,
    pub action_types: Vec<String>,
    pub line: usize,
}

/// A derived computation (useMemo, useCallback).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedComputation {
    pub name: Option<String>,
    pub hook: String,
    pub dependencies: Vec<String>,
    pub expression_snippet: String,
    pub is_callback: bool,
    pub line: usize,
}

/// An effect binding (useEffect, useLayoutEffect).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectBinding {
    pub hook: String,
    pub dependencies: Vec<String>,
    pub has_cleanup: bool,
    pub kind: EffectClassification,
    pub required_capabilities: Vec<Capability>,
    pub optional_capabilities: Vec<Capability>,
    pub platform_assumptions: Vec<PlatformAssumption>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub line: usize,
}

/// Classification of an effect's purpose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectClassification {
    /// Data fetching (fetch, axios, etc.).
    DataFetch,
    /// DOM manipulation or measurement.
    DomManipulation,
    /// Event listener setup.
    EventListener,
    /// Timer/interval setup.
    Timer,
    /// Subscription setup.
    Subscription,
    /// Synchronization with external state.
    Sync,
    /// Logging or analytics.
    Telemetry,
    /// Unknown/unclassified.
    Unknown,
}

/// A state transition triggered by an event handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStateTransition {
    pub event_name: String,
    pub handler_name: Option<String>,
    pub state_writes: Vec<String>,
    pub is_async: bool,
    pub line: usize,
}

/// A context consumer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConsumer {
    pub context_name: String,
    pub binding: Option<String>,
    pub line: usize,
}

/// A context provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextProvider {
    pub context_name: String,
    pub value_expression: Option<String>,
    pub line: usize,
}

/// Project-level state-effects analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStateModel {
    pub components: BTreeMap<String, ComponentStateModel>,
    pub global_state_stores: Vec<GlobalStoreInfo>,
    pub context_graph: Vec<ContextEdge>,
    pub required_capabilities: BTreeSet<Capability>,
    pub optional_capabilities: BTreeSet<Capability>,
    pub platform_assumptions: Vec<PlatformAssumption>,
    pub risk_flags: Vec<CapabilityRiskFlag>,
    pub stats: StateEffectsStats,
}

/// Risk level for capability/platform assumptions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityRiskLevel {
    Blocking,
    WarnOnly,
}

/// A capability/platform risk with origin metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRiskFlag {
    pub level: CapabilityRiskLevel,
    pub summary: String,
    pub capability: Option<Capability>,
    pub assumption: Option<String>,
    pub file: String,
    pub component: String,
    pub line: usize,
}

/// Information about a detected global state store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalStoreInfo {
    pub kind: GlobalStoreKind,
    pub name: String,
    pub file: String,
}

/// Kind of global state management.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GlobalStoreKind {
    Redux,
    Zustand,
    Jotai,
    Recoil,
    MobX,
    Context,
    Custom,
}

/// An edge in the context provider→consumer graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEdge {
    pub provider_component: String,
    pub consumer_component: String,
    pub context_name: String,
}

/// Summary statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEffectsStats {
    pub total_state_vars: usize,
    pub total_reducers: usize,
    pub total_effects: usize,
    pub total_derived: usize,
    pub total_event_transitions: usize,
    pub total_context_consumers: usize,
    pub total_context_providers: usize,
    pub effect_classification_counts: BTreeMap<String, usize>,
}

// ── Extraction ───────────────────────────────────────────────────────────

/// Extract state-effects model from a parsed file.
pub fn extract_state_effects(file_parse: &FileParse) -> Vec<ComponentStateModel> {
    let mut models = Vec::new();

    for (index, component) in file_parse.components.iter().enumerate() {
        let component_jsx =
            jsx_elements_for_component(&file_parse.jsx_elements, &file_parse.components, index);
        let model = extract_component_state(component, &file_parse.file, &component_jsx);
        models.push(model);
    }

    models
}

/// Extract state-effects model from a single component.
fn extract_component_state(
    component: &ComponentDecl,
    file: &str,
    jsx_elements: &[&JsxElement],
) -> ComponentStateModel {
    let mut state_vars = Vec::new();
    let mut reducers = Vec::new();
    let mut derived = Vec::new();
    let mut effects = Vec::new();
    let mut context_consumers = Vec::new();
    let context_providers = extract_context_providers(jsx_elements);

    for hook in &component.hooks {
        match hook.name.as_str() {
            "useState" => {
                state_vars.push(extract_use_state(hook));
            }
            "useReducer" => {
                if let Some((sv, rp)) = extract_use_reducer(hook) {
                    state_vars.push(sv);
                    reducers.push(rp);
                }
            }
            "useRef" => {
                state_vars.push(extract_use_ref(hook));
            }
            "useMemo" => {
                derived.push(extract_use_memo(hook, false));
            }
            "useCallback" => {
                derived.push(extract_use_memo(hook, true));
            }
            "useEffect" | "useLayoutEffect" | "useInsertionEffect" => {
                effects.push(extract_use_effect(hook));
            }
            "useContext" => {
                context_consumers.push(extract_use_context(hook));
            }
            name if name.starts_with("use") => {
                // Custom hooks — try to classify.
                if let Some(sv) = try_classify_custom_hook(hook) {
                    state_vars.push(sv);
                }
            }
            _ => {}
        }
    }

    // Event transitions from event handlers.
    let event_transitions = extract_event_transitions(component);

    ComponentStateModel {
        component_name: component.name.clone(),
        file: file.to_string(),
        state_vars,
        reducers,
        derived,
        effects,
        event_transitions,
        context_consumers,
        context_providers,
    }
}

fn jsx_elements_for_component<'a>(
    jsx_elements: &'a [JsxElement],
    components: &[ComponentDecl],
    component_index: usize,
) -> Vec<&'a JsxElement> {
    let component = &components[component_index];
    let component_end = components
        .iter()
        .skip(component_index + 1)
        .map(|next| next.line)
        .min()
        .unwrap_or(usize::MAX);

    jsx_elements
        .iter()
        .filter(|element| element.line >= component.line && element.line < component_end)
        .collect()
}

fn extract_context_providers(jsx_elements: &[&JsxElement]) -> Vec<ContextProvider> {
    jsx_elements
        .iter()
        .filter_map(|element| {
            let context_name = element.tag.strip_suffix(".Provider")?;
            if context_name.is_empty() {
                return None;
            }
            let value_expression = element
                .props
                .iter()
                .find(|prop| !prop.is_spread && prop.name == "value")
                .and_then(|prop| prop.value_snippet.clone());
            Some(ContextProvider {
                context_name: context_name.to_string(),
                value_expression,
                line: element.line,
            })
        })
        .collect()
}

fn extract_use_state(hook: &HookCall) -> StateVar {
    let (name, setter) = if let Some(ref binding) = hook.binding {
        parse_destructured_pair(binding)
    } else {
        (None, None)
    };

    StateVar {
        name: name.unwrap_or_else(|| "state".to_string()),
        setter,
        hook: "useState".to_string(),
        initial_value: if hook.args_snippet.is_empty() {
            None
        } else {
            Some(hook.args_snippet.clone())
        },
        scope: StateVarScope::Local,
        type_hint: None,
        line: hook.line,
    }
}

fn extract_use_reducer(hook: &HookCall) -> Option<(StateVar, ReducerPattern)> {
    let (state_name, dispatch_name) = if let Some(ref binding) = hook.binding {
        parse_destructured_pair(binding)
    } else {
        (None, None)
    };

    let state_name = state_name.unwrap_or_else(|| "state".to_string());
    let dispatch_name = dispatch_name.unwrap_or_else(|| "dispatch".to_string());

    // Parse args: reducer, initialState
    let parts: Vec<&str> = hook.args_snippet.splitn(2, ',').collect();
    let reducer_name = parts.first().map(|s| s.trim().to_string());
    let initial_state = parts.get(1).map(|s| s.trim().to_string());

    let sv = StateVar {
        name: state_name.clone(),
        setter: Some(dispatch_name.clone()),
        hook: "useReducer".to_string(),
        initial_value: initial_state.clone(),
        scope: StateVarScope::Reducer,
        type_hint: None,
        line: hook.line,
    };

    let rp = ReducerPattern {
        state_name,
        dispatch_name,
        reducer_name,
        initial_state,
        action_types: Vec::new(), // Would need deeper analysis.
        line: hook.line,
    };

    Some((sv, rp))
}

fn extract_use_ref(hook: &HookCall) -> StateVar {
    let name = hook
        .binding
        .as_ref()
        .map(|b| b.trim().to_string())
        .unwrap_or_else(|| "ref".to_string());

    StateVar {
        name,
        setter: None,
        hook: "useRef".to_string(),
        initial_value: if hook.args_snippet.is_empty() {
            None
        } else {
            Some(hook.args_snippet.clone())
        },
        scope: StateVarScope::Ref,
        type_hint: None,
        line: hook.line,
    }
}

fn extract_use_memo(hook: &HookCall, is_callback: bool) -> DerivedComputation {
    let name = hook.binding.as_ref().map(|b| b.trim().to_string());

    // Dependencies are in the second argument (array).
    let deps = extract_dependency_array(&hook.args_snippet);

    DerivedComputation {
        name,
        hook: hook.name.clone(),
        dependencies: deps,
        expression_snippet: hook.args_snippet.clone(),
        is_callback,
        line: hook.line,
    }
}

fn extract_use_effect(hook: &HookCall) -> EffectBinding {
    let deps = extract_dependency_array(&hook.args_snippet);
    let has_cleanup = has_cleanup_return(&hook.args_snippet);
    let kind = classify_effect(&hook.args_snippet);
    let (required_caps, optional_caps, assumptions) =
        classify_effect_capabilities(&kind, &hook.args_snippet);

    // Approximate reads/writes by looking for setter patterns.
    let reads = extract_reads_from_snippet(&hook.args_snippet);
    let writes = extract_writes_from_snippet(&hook.args_snippet);

    EffectBinding {
        hook: hook.name.clone(),
        dependencies: deps,
        has_cleanup,
        kind,
        required_capabilities: required_caps.into_iter().collect(),
        optional_capabilities: optional_caps.into_iter().collect(),
        platform_assumptions: assumptions,
        reads,
        writes,
        line: hook.line,
    }
}

fn has_cleanup_return(snippet: &str) -> bool {
    let mut search_start = 0usize;
    while search_start < snippet.len() {
        let Some(relative) = snippet[search_start..].find("return") else {
            return false;
        };
        let pos = search_start + relative;

        let prev_ok = pos == 0
            || snippet[..pos]
                .chars()
                .next_back()
                .is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'));

        let after = &snippet[pos + "return".len()..];
        if prev_ok && is_cleanup_return_expression(after) {
            return true;
        }

        search_start = pos + "return".len();
    }
    false
}

fn is_cleanup_return_expression(after_return: &str) -> bool {
    let expr = after_return.trim_start();
    if expr.is_empty() || expr.starts_with(';') || expr.starts_with('{') {
        return false;
    }

    if expr.starts_with("function") || expr.starts_with("async function") {
        return true;
    }

    if is_arrow_function_expression(expr) {
        return true;
    }

    if let Some((head, rest)) = split_identifier_chain(expr) {
        if is_non_cleanup_literal(head) {
            return false;
        }
        let tail = rest.trim_start();
        return tail.is_empty() || matches!(tail.chars().next(), Some(';' | ',' | ')' | ']'));
    }

    false
}

fn is_arrow_function_expression(expr: &str) -> bool {
    if let Some(rest) = expr.strip_prefix("async") {
        return is_arrow_function_expression(rest.trim_start());
    }

    if let Some((_, rest)) = split_identifier(expr) {
        return rest.trim_start().starts_with("=>");
    }

    let Some(after_open) = expr.strip_prefix('(') else {
        return false;
    };

    let mut depth = 1usize;
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    for (idx, ch) in after_open.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => in_string = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let tail = after_open[idx + 1..].trim_start();
                    return tail.starts_with("=>");
                }
            }
            _ => {}
        }
    }

    false
}

fn split_identifier(input: &str) -> Option<(&str, &str)> {
    let mut chars = input.char_indices();
    let (_, first) = chars.next()?;
    if !is_identifier_start(first) {
        return None;
    }

    let mut end = first.len_utf8();
    for (idx, ch) in chars {
        if is_identifier_part(ch) {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }

    Some((&input[..end], &input[end..]))
}

fn split_identifier_chain(input: &str) -> Option<(&str, &str)> {
    let (head, mut rest) = split_identifier(input)?;
    let mut consumed = head.len();

    while let Some(after_dot) = rest.strip_prefix('.')
        && let Some((_, chain_rest)) = split_identifier(after_dot)
    {
        consumed = input.len() - chain_rest.len();
        rest = &input[consumed..];
    }

    Some((head, &input[consumed..]))
}

fn is_identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch == '$'
}

fn is_identifier_part(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

fn is_non_cleanup_literal(identifier: &str) -> bool {
    matches!(
        identifier,
        "null" | "undefined" | "true" | "false" | "NaN" | "Infinity" | "this"
    )
}

fn extract_use_context(hook: &HookCall) -> ContextConsumer {
    let binding = hook.binding.as_ref().map(|b| b.trim().to_string());
    let context_name = hook.args_snippet.trim().to_string();

    ContextConsumer {
        context_name,
        binding,
        line: hook.line,
    }
}

fn try_classify_custom_hook(hook: &HookCall) -> Option<StateVar> {
    let name_lower = hook.name.to_lowercase();

    // Common patterns for state-like custom hooks.
    let scope = if name_lower.contains("query")
        || name_lower.contains("fetch")
        || name_lower.contains("swr")
    {
        StateVarScope::Server
    } else if name_lower.contains("store")
        || name_lower.contains("atom")
        || name_lower.contains("selector")
    {
        StateVarScope::ExternalStore
    } else if name_lower.contains("param") || name_lower.contains("router") {
        StateVarScope::Url
    } else {
        return None;
    };

    let name = hook
        .binding
        .as_ref()
        .map(|b| b.trim().to_string())
        .unwrap_or_else(|| hook.name.clone());

    Some(StateVar {
        name,
        setter: None,
        hook: hook.name.clone(),
        initial_value: None,
        scope,
        type_hint: None,
        line: hook.line,
    })
}

// ── Effect Classification ────────────────────────────────────────────────

fn classify_effect(snippet: &str) -> EffectClassification {
    let lower = snippet.to_lowercase();

    if lower.contains("fetch(")
        || lower.contains("axios")
        || lower.contains(".get(")
        || lower.contains(".post(")
        || lower.contains("graphql")
    {
        EffectClassification::DataFetch
    } else if lower.contains("addeventlistener")
        || lower.contains("removeeventlistener")
        || lower.contains("window.on")
    {
        EffectClassification::EventListener
    } else if lower.contains("settimeout")
        || lower.contains("setinterval")
        || lower.contains("requestanimationframe")
    {
        EffectClassification::Timer
    } else if lower.contains("subscribe") || lower.contains("observable") {
        EffectClassification::Subscription
    } else if lower.contains("document.")
        || lower.contains("getelementby")
        || lower.contains(".focus(")
    {
        EffectClassification::DomManipulation
    } else if lower.contains("console.") || lower.contains("analytics") || lower.contains("track(")
    {
        EffectClassification::Telemetry
    } else if lower.contains("localstorage")
        || lower.contains("sessionstorage")
        || lower.contains("sync")
    {
        EffectClassification::Sync
    } else {
        EffectClassification::Unknown
    }
}

fn classify_effect_capabilities(
    kind: &EffectClassification,
    snippet: &str,
) -> (
    BTreeSet<Capability>,
    BTreeSet<Capability>,
    Vec<PlatformAssumption>,
) {
    let mut required = BTreeSet::new();
    let mut optional = BTreeSet::new();
    let mut assumptions = Vec::new();
    let lower = snippet.to_lowercase();

    match kind {
        EffectClassification::DataFetch => {
            required.insert(Capability::NetworkAccess);
        }
        EffectClassification::Timer => {
            required.insert(Capability::Timers);
        }
        EffectClassification::EventListener => {
            let keyboard =
                lower.contains("keydown") || lower.contains("keyup") || lower.contains("keypress");
            let pointer = lower.contains("mouse")
                || lower.contains("pointer")
                || lower.contains("click")
                || lower.contains("drag")
                || lower.contains("wheel");
            let touch = lower.contains("touch");

            if keyboard {
                required.insert(Capability::KeyboardInput);
            }
            if pointer {
                required.insert(Capability::MouseInput);
            }
            if touch {
                required.insert(Capability::TouchInput);
            }
            if !keyboard && !pointer && !touch {
                optional.insert(Capability::KeyboardInput);
                optional.insert(Capability::MouseInput);
            }
        }
        EffectClassification::Subscription => {
            optional.insert(Capability::NetworkAccess);
        }
        EffectClassification::Telemetry => {
            optional.insert(Capability::NetworkAccess);
        }
        EffectClassification::Sync => {
            if lower.contains("localstorage") || lower.contains("sessionstorage") {
                optional.insert(Capability::FileSystem);
            }
        }
        EffectClassification::DomManipulation | EffectClassification::Unknown => {}
    }

    if lower.contains("navigator.clipboard") {
        required.insert(Capability::Clipboard);
    }
    if lower.contains("document.") || lower.contains("window.") {
        assumptions.push(PlatformAssumption {
            assumption: "Browser DOM APIs are available at runtime".to_string(),
            evidence: truncate_evidence(snippet, 120),
            impact_if_wrong: "DOM/event effects cannot execute; UI behavior may break".to_string(),
        });
    }
    if lower.contains("process.env") {
        assumptions.push(PlatformAssumption {
            assumption: "Environment variables are available in runtime context".to_string(),
            evidence: truncate_evidence(snippet, 120),
            impact_if_wrong: "Configuration-dependent effects may use wrong defaults".to_string(),
        });
    }
    if lower.contains("localstorage") || lower.contains("sessionstorage") {
        assumptions.push(PlatformAssumption {
            assumption: "Web storage APIs are available".to_string(),
            evidence: truncate_evidence(snippet, 120),
            impact_if_wrong: "Persistence/sync flows may fail or lose state".to_string(),
        });
    }

    (required, optional, assumptions)
}

fn truncate_evidence(snippet: &str, max: usize) -> String {
    let collapsed = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let prefix = collapsed.chars().take(max).collect::<String>();
        format!("{prefix}...")
    }
}

fn build_effect_risk_flags(
    file: &str,
    component: &str,
    effect: &EffectBinding,
) -> Vec<CapabilityRiskFlag> {
    let mut flags = Vec::new();

    for cap in &effect.required_capabilities {
        flags.push(CapabilityRiskFlag {
            level: CapabilityRiskLevel::Blocking,
            summary: format!("Required capability {:?} inferred from effect", cap),
            capability: Some(cap.clone()),
            assumption: None,
            file: file.to_string(),
            component: component.to_string(),
            line: effect.line,
        });
    }

    for cap in &effect.optional_capabilities {
        flags.push(CapabilityRiskFlag {
            level: CapabilityRiskLevel::WarnOnly,
            summary: format!("Optional capability {:?} may improve fidelity", cap),
            capability: Some(cap.clone()),
            assumption: None,
            file: file.to_string(),
            component: component.to_string(),
            line: effect.line,
        });
    }

    for assumption in &effect.platform_assumptions {
        flags.push(CapabilityRiskFlag {
            level: CapabilityRiskLevel::WarnOnly,
            summary: format!("Platform assumption: {}", assumption.assumption),
            capability: None,
            assumption: Some(assumption.assumption.clone()),
            file: file.to_string(),
            component: component.to_string(),
            line: effect.line,
        });
    }

    flags
}

// ── Event Transition Extraction ──────────────────────────────────────────

fn extract_event_transitions(component: &ComponentDecl) -> Vec<EventStateTransition> {
    let mut transitions = Vec::new();

    for handler in &component.event_handlers {
        let handler_text = handler.handler_name.as_deref().unwrap_or("");

        // Look for state setter patterns (setXxx, dispatch).
        let re_setter = Regex::new(r"set[A-Z]\w*").expect("setter regex");
        let mut writes: Vec<String> = Vec::new();
        for m in re_setter.find_iter(handler_text) {
            writes.push(m.as_str().to_string());
        }

        let is_async = handler_text.contains("async")
            || handler_text.contains("await")
            || handler_text.contains(".then(");

        transitions.push(EventStateTransition {
            event_name: handler.event_name.clone(),
            handler_name: handler.handler_name.clone(),
            state_writes: writes,
            is_async,
            line: handler.line,
        });
    }

    transitions
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Parse a destructured pair like `[foo, setFoo]` into (Some("foo"), Some("setFoo")).
fn parse_destructured_pair(binding: &str) -> (Option<String>, Option<String>) {
    let trimmed = binding.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        let first = parts.first().map(|s| s.trim().to_string());
        let second = parts.get(1).map(|s| s.trim().to_string());
        (first, second)
    } else {
        (Some(trimmed.to_string()), None)
    }
}

/// Extract dependency array items from a hook args snippet.
fn extract_dependency_array(args: &str) -> Vec<String> {
    let re = Regex::new(r"\[([^\]]*)\]").expect("dep array regex");
    if let Some(caps) = re.captures(args) {
        caps[1]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    }
}

/// Approximate state reads from effect body.
fn extract_reads_from_snippet(snippet: &str) -> Vec<String> {
    let re = Regex::new(r"\b([a-z]\w*)\b").expect("reads regex");
    let mut reads: BTreeSet<String> = BTreeSet::new();
    for m in re.find_iter(snippet) {
        let name = m.as_str();
        // Skip common non-state identifiers.
        if !matches!(
            name,
            "const"
                | "let"
                | "var"
                | "return"
                | "if"
                | "else"
                | "true"
                | "false"
                | "null"
                | "undefined"
                | "function"
                | "async"
                | "await"
                | "new"
        ) {
            reads.insert(name.to_string());
        }
    }
    reads.into_iter().collect()
}

/// Approximate state writes from effect body (setter calls).
fn extract_writes_from_snippet(snippet: &str) -> Vec<String> {
    let re = Regex::new(r"(set[A-Z]\w*|dispatch)\s*\(").expect("writes regex");
    let mut writes = Vec::new();
    for caps in re.captures_iter(snippet) {
        writes.push(caps[1].to_string());
    }
    writes
}

/// Detect global state store patterns in file content.
pub fn detect_global_stores(content: &str, file: &str) -> Vec<GlobalStoreInfo> {
    let mut stores = Vec::new();
    let recoil_atom_re = Regex::new(r"atom\(\s*\{").expect("recoil atom regex");
    let recoil_selector_re = Regex::new(r"selector\(\s*\{").expect("recoil selector regex");
    let declarations = detect_store_declarations(content);

    let patterns: &[(&str, GlobalStoreKind)] = &[
        ("createStore", GlobalStoreKind::Redux),
        ("configureStore", GlobalStoreKind::Redux),
        ("create(", GlobalStoreKind::Zustand),
        ("atom(", GlobalStoreKind::Jotai),
        ("atomWithStorage", GlobalStoreKind::Jotai),
        ("atom({", GlobalStoreKind::Recoil),
        ("selector({", GlobalStoreKind::Recoil),
        ("makeAutoObservable", GlobalStoreKind::MobX),
        ("makeObservable", GlobalStoreKind::MobX),
        ("createContext", GlobalStoreKind::Context),
    ];

    for (pattern, kind) in patterns {
        let pattern_present = match (kind, *pattern) {
            (GlobalStoreKind::Recoil, "atom({") => recoil_atom_re.is_match(content),
            (GlobalStoreKind::Recoil, "selector({") => recoil_selector_re.is_match(content),
            _ => content.contains(pattern),
        };
        if pattern_present {
            let matched_names = declarations
                .iter()
                .enumerate()
                .filter_map(|(idx, (_, initializer_start, name))| {
                    let initializer_end = declarations
                        .get(idx + 1)
                        .map(|(next_decl_start, _, _)| *next_decl_start)
                        .unwrap_or(content.len());
                    let initializer = content[*initializer_start..initializer_end]
                        .trim()
                        .trim_end_matches(';')
                        .trim();
                    let matches_pattern = match (kind, *pattern) {
                        (GlobalStoreKind::Jotai, "atom(") => {
                            initializer.contains("atom(") && !recoil_atom_re.is_match(initializer)
                        }
                        (GlobalStoreKind::Recoil, "atom({") => recoil_atom_re.is_match(initializer),
                        (GlobalStoreKind::Recoil, "selector({") => {
                            recoil_selector_re.is_match(initializer)
                        }
                        _ => initializer.contains(pattern),
                    };
                    matches_pattern.then(|| name.clone())
                })
                .collect::<BTreeSet<_>>();
            let requires_structural_match = matches!(
                (kind, *pattern),
                (GlobalStoreKind::Jotai, "atom(")
                    | (GlobalStoreKind::Recoil, "atom({")
                    | (GlobalStoreKind::Recoil, "selector({")
            );
            if requires_structural_match && matched_names.is_empty() {
                continue;
            }

            if matched_names.is_empty() {
                stores.push(GlobalStoreInfo {
                    kind: kind.clone(),
                    name: format!("{kind:?}Store"),
                    file: file.to_string(),
                });
                continue;
            }

            for name in matched_names {
                stores.push(GlobalStoreInfo {
                    kind: kind.clone(),
                    name,
                    file: file.to_string(),
                });
            }
        }
    }

    stores
}

fn detect_store_declarations(content: &str) -> Vec<(usize, usize, String)> {
    let declaration_head_re = Regex::new(r"(?m)(?:^|[;\n])\s*(?:export\s+)?(?:const|let)\s+(\w+)")
        .expect("store declaration regex");
    declaration_head_re
        .captures_iter(content)
        .filter_map(|captures| {
            let declaration = captures.get(0)?;
            let name = captures.get(1)?;
            let initializer_start = find_declaration_initializer_start(content, name.end())?;
            Some((
                declaration.start(),
                initializer_start,
                name.as_str().to_string(),
            ))
        })
        .collect()
}

fn find_declaration_initializer_start(content: &str, mut index: usize) -> Option<usize> {
    index = skip_whitespace_and_comments(content, index);
    if content[index..].starts_with(':') {
        index += ':'.len_utf8();
        index = skip_type_annotation(content, index)?;
    }
    index = skip_whitespace_and_comments(content, index);
    if !content[index..].starts_with('=') {
        return None;
    }
    index += '='.len_utf8();
    Some(skip_whitespace_and_comments(content, index))
}

fn skip_type_annotation(content: &str, mut index: usize) -> Option<usize> {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut angle_depth = 0usize;
    let mut string_delimiter: Option<char> = None;
    let mut escaped = false;

    while index < content.len() {
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }

        let ch = content[index..].chars().next()?;
        if let Some(delimiter) = string_delimiter {
            index += ch.len_utf8();
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == delimiter {
                string_delimiter = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => {
                string_delimiter = Some(ch);
                index += ch.len_utf8();
            }
            '(' => {
                paren_depth += 1;
                index += ch.len_utf8();
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                index += ch.len_utf8();
            }
            '[' => {
                bracket_depth += 1;
                index += ch.len_utf8();
            }
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                index += ch.len_utf8();
            }
            '{' => {
                brace_depth += 1;
                index += ch.len_utf8();
            }
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                index += ch.len_utf8();
            }
            '<' => {
                angle_depth += 1;
                index += ch.len_utf8();
            }
            '>' => {
                angle_depth = angle_depth.saturating_sub(1);
                index += ch.len_utf8();
            }
            '=' if paren_depth == 0
                && bracket_depth == 0
                && brace_depth == 0
                && angle_depth == 0
                && !content[index..].starts_with("=>") =>
            {
                return Some(index);
            }
            ';' | '\n'
                if paren_depth == 0
                    && bracket_depth == 0
                    && brace_depth == 0
                    && angle_depth == 0 =>
            {
                return None;
            }
            _ => {
                index += ch.len_utf8();
            }
        }
    }

    None
}

fn skip_whitespace_and_comments(content: &str, mut index: usize) -> usize {
    while index < content.len() {
        let remaining = &content[index..];
        let Some(ch) = remaining.chars().next() else {
            break;
        };
        if ch.is_whitespace() {
            index += ch.len_utf8();
            continue;
        }
        if let Some(next_index) = skip_comment(content, index) {
            index = next_index;
            continue;
        }
        break;
    }
    index
}

fn skip_comment(content: &str, index: usize) -> Option<usize> {
    let remaining = &content[index..];
    if remaining.starts_with("//") {
        return Some(
            remaining
                .find('\n')
                .map(|offset| index + offset)
                .unwrap_or(content.len()),
        );
    }
    if remaining.starts_with("/*") {
        return Some(
            remaining
                .find("*/")
                .map(|offset| index + offset + 2)
                .unwrap_or(content.len()),
        );
    }
    None
}

/// Build project-level state model from all file parses.
pub fn build_project_state_model(
    file_parses: &BTreeMap<String, FileParse>,
    file_contents: &BTreeMap<String, String>,
) -> ProjectStateModel {
    let mut components = BTreeMap::new();
    let mut global_stores = Vec::new();
    let mut context_graph = Vec::new();
    let mut required_capabilities = BTreeSet::new();
    let mut optional_capabilities = BTreeSet::new();
    let mut platform_assumptions = Vec::new();
    let mut platform_assumption_dedup = BTreeSet::new();
    let mut risk_flags = Vec::new();
    let mut context_providers_by_name: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut unresolved_context_consumers = Vec::new();

    let mut total_state_vars = 0usize;
    let mut total_reducers = 0usize;
    let mut total_effects = 0usize;
    let mut total_derived = 0usize;
    let mut total_event_transitions = 0usize;
    let mut total_context_consumers = 0usize;
    let mut total_context_providers = 0usize;
    let mut effect_counts: BTreeMap<String, usize> = BTreeMap::new();

    for (file, parse) in file_parses {
        let models = extract_state_effects(parse);

        for model in models {
            let component_key = format!("{}::{}", file, model.component_name);
            total_state_vars += model.state_vars.len();
            total_reducers += model.reducers.len();
            total_effects += model.effects.len();
            total_derived += model.derived.len();
            total_event_transitions += model.event_transitions.len();
            total_context_consumers += model.context_consumers.len();
            total_context_providers += model.context_providers.len();

            for effect in &model.effects {
                let key = format!("{:?}", effect.kind);
                *effect_counts.entry(key).or_default() += 1;

                for cap in &effect.required_capabilities {
                    required_capabilities.insert(cap.clone());
                    optional_capabilities.remove(cap);
                }
                for cap in &effect.optional_capabilities {
                    if !required_capabilities.contains(cap) {
                        optional_capabilities.insert(cap.clone());
                    }
                }
                for assumption in &effect.platform_assumptions {
                    let dedup_key = format!(
                        "{}|{}|{}",
                        assumption.assumption, assumption.evidence, assumption.impact_if_wrong
                    );
                    if platform_assumption_dedup.insert(dedup_key) {
                        platform_assumptions.push(assumption.clone());
                    }
                }

                risk_flags.extend(build_effect_risk_flags(file, &model.component_name, effect));
            }

            for provider in &model.context_providers {
                context_providers_by_name
                    .entry(provider.context_name.clone())
                    .or_default()
                    .insert(component_key.clone());
            }
            for consumer in &model.context_consumers {
                unresolved_context_consumers
                    .push((component_key.clone(), consumer.context_name.clone()));
            }

            components.insert(component_key, model);
        }

        // Detect global stores.
        if let Some(content) = file_contents.get(file) {
            global_stores.extend(detect_global_stores(content, file));
        }
    }

    for (consumer_component, context_name) in unresolved_context_consumers {
        let Some(provider_components) = context_providers_by_name.get(&context_name) else {
            continue;
        };
        // React resolves useContext() against an ancestor provider, not a provider
        // rendered later by the same component. Without ancestry information we
        // also refuse ambiguous multi-provider fanout instead of asserting false
        // data-flow edges.
        let candidate_providers = provider_components
            .iter()
            .filter(|provider_component| *provider_component != &consumer_component)
            .cloned()
            .collect::<Vec<_>>();
        if let [provider_component] = candidate_providers.as_slice() {
            context_graph.push(ContextEdge {
                provider_component: provider_component.clone(),
                consumer_component: consumer_component.clone(),
                context_name: context_name.clone(),
            });
        }
    }
    context_graph.sort_by(|left, right| {
        left.context_name
            .cmp(&right.context_name)
            .then_with(|| left.provider_component.cmp(&right.provider_component))
            .then_with(|| left.consumer_component.cmp(&right.consumer_component))
    });
    context_graph.dedup_by(|left, right| {
        left.context_name == right.context_name
            && left.provider_component == right.provider_component
            && left.consumer_component == right.consumer_component
    });

    ProjectStateModel {
        components,
        global_state_stores: global_stores,
        context_graph,
        required_capabilities,
        optional_capabilities,
        platform_assumptions,
        risk_flags,
        stats: StateEffectsStats {
            total_state_vars,
            total_reducers,
            total_effects,
            total_derived,
            total_event_transitions,
            total_context_consumers,
            total_context_providers,
            effect_classification_counts: effect_counts,
        },
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsx_parser::parse_file;

    #[test]
    fn extract_use_state_basic() {
        let src = r#"
function Counter() {
    const [count, setCount] = useState(0);
    return <div>{count}</div>;
}
"#;
        let parse = parse_file(src, "Counter.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].state_vars.len(), 1);
        assert_eq!(models[0].state_vars[0].name, "count");
        assert_eq!(models[0].state_vars[0].setter.as_deref(), Some("setCount"));
        assert_eq!(models[0].state_vars[0].scope, StateVarScope::Local);
        assert_eq!(models[0].state_vars[0].initial_value.as_deref(), Some("0"));
    }

    #[test]
    fn extract_use_reducer_pattern() {
        let src = r#"
function TodoList() {
    const [state, dispatch] = useReducer(todoReducer, initialState);
    return <div />;
}
"#;
        let parse = parse_file(src, "TodoList.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models[0].reducers.len(), 1);
        assert_eq!(models[0].reducers[0].state_name, "state");
        assert_eq!(models[0].reducers[0].dispatch_name, "dispatch");
        assert_eq!(
            models[0].reducers[0].reducer_name.as_deref(),
            Some("todoReducer")
        );
    }

    #[test]
    fn extract_use_ref() {
        let src = r#"
function Input() {
    const inputRef = useRef(null);
    return <input ref={inputRef} />;
}
"#;
        let parse = parse_file(src, "Input.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models[0].state_vars.len(), 1);
        assert_eq!(models[0].state_vars[0].scope, StateVarScope::Ref);
        assert_eq!(models[0].state_vars[0].name, "inputRef");
    }

    #[test]
    fn extract_use_memo_and_callback() {
        let src = r#"
function Expensive() {
    const memoized = useMemo(() => compute(x), [x]);
    const handler = useCallback(() => doStuff(), [dep]);
    return <div />;
}
"#;
        let parse = parse_file(src, "Expensive.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models[0].derived.len(), 2);
        assert!(!models[0].derived[0].is_callback);
        assert!(models[0].derived[1].is_callback);
    }

    #[test]
    fn extract_use_effect_presence() {
        // tsx_parser's hook regex stops at the first ')' in arrow params,
        // so args_snippet is empty for useEffect(()=>...). We just verify
        // the effect is extracted; classification is tested directly below.
        let src = r#"
function DataLoader() {
    const [data, setData] = useState(null);
    useEffect(() => { fetch('/api'); }, []);
    return <div />;
}
"#;
        let parse = parse_file(src, "DataLoader.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models[0].effects.len(), 1);
        assert_eq!(models[0].effects[0].hook, "useEffect");
    }

    #[test]
    fn classify_effect_data_fetch() {
        // Test classification directly with a realistic snippet.
        assert_eq!(
            classify_effect("() => { fetch('/api/data').then(setData); }"),
            EffectClassification::DataFetch
        );
    }

    #[test]
    fn extract_use_context() {
        let src = r#"
function ThemeButton() {
    const theme = useContext(ThemeContext);
    return <button />;
}
"#;
        let parse = parse_file(src, "ThemeButton.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models[0].context_consumers.len(), 1);
        assert_eq!(models[0].context_consumers[0].context_name, "ThemeContext");
    }

    #[test]
    fn extract_context_provider_from_jsx() {
        let src = r#"
function ThemeProvider() {
    return <ThemeContext.Provider value={theme}><button /></ThemeContext.Provider>;
}
"#;
        let parse = parse_file(src, "ThemeProvider.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models[0].context_providers.len(), 1);
        assert_eq!(models[0].context_providers[0].context_name, "ThemeContext");
        assert_eq!(
            models[0].context_providers[0].value_expression.as_deref(),
            Some("theme")
        );
    }

    #[test]
    fn classify_effect_timer() {
        assert_eq!(
            classify_effect("() => { const id = setInterval(() => tick(), 1000); }"),
            EffectClassification::Timer
        );
    }

    #[test]
    fn classify_effect_event_listener() {
        assert_eq!(
            classify_effect("() => { window.addEventListener('resize', handler); }"),
            EffectClassification::EventListener
        );
    }

    #[test]
    fn classify_effect_subscription() {
        assert_eq!(
            classify_effect("() => { const sub = observable.subscribe(cb); }"),
            EffectClassification::Subscription
        );
    }

    #[test]
    fn classify_effect_dom() {
        assert_eq!(
            classify_effect("() => { document.title = 'New Title'; }"),
            EffectClassification::DomManipulation
        );
    }

    #[test]
    fn classify_effect_unknown() {
        assert_eq!(
            classify_effect("() => { doSomething(); }"),
            EffectClassification::Unknown
        );
    }

    #[test]
    fn classify_effect_capabilities_data_fetch_and_dom_assumption() {
        let (required, optional, assumptions) = classify_effect_capabilities(
            &EffectClassification::DataFetch,
            "() => { window.fetch('/api/data'); }",
        );
        assert!(required.contains(&Capability::NetworkAccess));
        assert!(!optional.contains(&Capability::NetworkAccess));
        assert!(!assumptions.is_empty());
        assert!(
            assumptions
                .iter()
                .any(|a| a.assumption.contains("Browser DOM APIs"))
        );
    }

    #[test]
    fn classify_effect_capabilities_event_listener_keyboard() {
        let (required, _optional, assumptions) = classify_effect_capabilities(
            &EffectClassification::EventListener,
            "() => { window.addEventListener('keydown', onKey); }",
        );
        assert!(required.contains(&Capability::KeyboardInput));
        assert!(!assumptions.is_empty());
    }

    #[test]
    fn truncate_evidence_handles_unicode_boundaries() {
        assert_eq!(truncate_evidence("ééé document.title", 1), "é...");
        assert_eq!(
            truncate_evidence("window.document.title = 'ééé';", 26),
            "window.document.title = 'é..."
        );
    }

    #[test]
    fn parse_destructured_pair_array() {
        let (a, b) = parse_destructured_pair("[count, setCount]");
        assert_eq!(a.as_deref(), Some("count"));
        assert_eq!(b.as_deref(), Some("setCount"));
    }

    #[test]
    fn parse_destructured_pair_single() {
        let (a, b) = parse_destructured_pair("value");
        assert_eq!(a.as_deref(), Some("value"));
        assert!(b.is_none());
    }

    #[test]
    fn extract_dependency_array_basic() {
        let deps = extract_dependency_array("() => {}, [a, b, c]");
        assert_eq!(deps, vec!["a", "b", "c"]);
    }

    #[test]
    fn extract_dependency_array_empty() {
        let deps = extract_dependency_array("() => {}, []");
        assert!(deps.is_empty());
    }

    #[test]
    fn extract_dependency_array_none() {
        let deps = extract_dependency_array("() => {}");
        assert!(deps.is_empty());
    }

    #[test]
    fn detect_global_stores_redux() {
        let content = "const store = configureStore({ reducer: rootReducer });";
        let stores = detect_global_stores(content, "store.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Redux);
    }

    #[test]
    fn detect_global_stores_zustand() {
        let content = "export const useStore = create((set) => ({ count: 0 }));";
        let stores = detect_global_stores(content, "store.ts");
        assert!(stores.iter().any(|s| s.kind == GlobalStoreKind::Zustand));
        assert!(
            stores
                .iter()
                .any(|s| s.kind == GlobalStoreKind::Zustand && s.name == "useStore")
        );
    }

    #[test]
    fn detect_global_stores_jotai_atom() {
        let content = "export const countAtom = atom(0);";
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Jotai);
        assert_eq!(stores[0].name, "countAtom");
    }

    #[test]
    fn detect_global_stores_multiline_jotai_atom() {
        let content = r#"
export const countAtom = atom(
    0
);
"#;
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Jotai);
        assert_eq!(stores[0].name, "countAtom");
    }

    #[test]
    fn detect_global_stores_recoil_atom_is_not_misclassified_as_jotai() {
        let content = "export const themeAtom = atom({ key: 'theme', default: 'light' });";
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Recoil);
        assert_eq!(stores[0].name, "themeAtom");
    }

    #[test]
    fn detect_global_stores_multiline_recoil_atom() {
        let content = r#"
export const themeAtom = atom({
    key: 'theme',
    default: 'light',
});
"#;
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Recoil);
        assert_eq!(stores[0].name, "themeAtom");
    }

    #[test]
    fn detect_global_stores_multiline_recoil_atom_with_delayed_object_start() {
        let content = r#"
export const themeAtom = atom(
    {
        key: 'theme',
        default: 'light',
    }
);
"#;
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Recoil);
        assert_eq!(stores[0].name, "themeAtom");
    }

    #[test]
    fn detect_global_stores_recoil_atom_with_whitespace_after_paren() {
        let content = r#"
export const themeAtom = atom(
{
    key: 'theme',
    default: 'light',
});
"#;
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Recoil);
        assert_eq!(stores[0].name, "themeAtom");
    }

    #[test]
    fn detect_global_stores_recoil_selector_with_whitespace_after_paren() {
        let content = r#"
export const themeSelector = selector(
{
    key: 'themeSelector',
    get: () => 'light',
});
"#;
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Recoil);
        assert_eq!(stores[0].name, "themeSelector");
    }

    #[test]
    fn detect_global_stores_skips_prior_unrelated_assignment_names() {
        let content = r#"
const noise = 1;
export const realStore = configureStore({ reducer: rootReducer });
"#;
        let stores = detect_global_stores(content, "store.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Redux);
        assert_eq!(stores[0].name, "realStore");
    }

    #[test]
    fn detect_global_stores_collects_multiple_jotai_atoms() {
        let content = r#"
export const firstAtom = atom(0);
export const secondAtom = atom(1);
"#;
        let stores = detect_global_stores(content, "state.ts");
        let mut names = stores
            .iter()
            .filter(|store| store.kind == GlobalStoreKind::Jotai)
            .map(|store| store.name.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(names, vec!["firstAtom", "secondAtom"]);
    }

    #[test]
    fn detect_global_stores_redux_slice_is_not_misclassified_as_store() {
        let content = r#"
export const todosSlice = createSlice({
    name: 'todos',
    initialState: [],
    reducers: {},
});
"#;
        let stores = detect_global_stores(content, "store.ts");
        assert!(stores.is_empty());
    }

    #[test]
    fn detect_global_stores_typed_jotai_atom_preserves_name() {
        let content = "export const countAtom: PrimitiveAtom<number> = atom(0);";
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Jotai);
        assert_eq!(stores[0].name, "countAtom");
    }

    #[test]
    fn detect_global_stores_typed_recoil_atom_preserves_name() {
        let content = "export const themeAtom: RecoilState<string> = atom({ key: 'theme', default: 'light' });";
        let stores = detect_global_stores(content, "state.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Recoil);
        assert_eq!(stores[0].name, "themeAtom");
    }

    #[test]
    fn detect_global_stores_export_let_with_typed_initializer_preserves_name() {
        let content = r#"
type StoreState = { count: number };
export let useStore: StoreApi<StoreState> = create((set) => ({ count: 0 }));
"#;
        let stores = detect_global_stores(content, "store.ts");
        assert_eq!(stores.len(), 1);
        assert_eq!(stores[0].kind, GlobalStoreKind::Zustand);
        assert_eq!(stores[0].name, "useStore");
    }

    #[test]
    fn detect_global_stores_context() {
        let content = "export const ThemeContext = createContext('light');";
        let stores = detect_global_stores(content, "context.ts");
        assert!(stores.iter().any(|s| s.kind == GlobalStoreKind::Context));
    }

    #[test]
    fn custom_hook_server_state() {
        let src = r#"
function UserProfile() {
    const data = useQuery('user', fetchUser);
    return <div />;
}
"#;
        let parse = parse_file(src, "UserProfile.tsx");
        let models = extract_state_effects(&parse);
        assert!(!models[0].state_vars.is_empty());
        assert_eq!(models[0].state_vars[0].scope, StateVarScope::Server);
    }

    #[test]
    fn multiple_hooks_per_component() {
        let src = r#"
function Dashboard() {
    const [count, setCount] = useState(0);
    const [name, setName] = useState('');
    const ref = useRef(null);
    const memo = useMemo(() => count * 2, [count]);
    useEffect(() => { document.title = name; }, [name]);
    return <div />;
}
"#;
        let parse = parse_file(src, "Dashboard.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models[0].state_vars.len(), 3); // 2 useState + 1 useRef
        assert_eq!(models[0].derived.len(), 1);
        assert_eq!(models[0].effects.len(), 1);
    }

    #[test]
    fn project_state_model_from_files() {
        let src = r#"
function App() {
    const [x, setX] = useState(0);
    return <div />;
}
"#;
        let parse = parse_file(src, "App.tsx");
        let mut file_parses = BTreeMap::new();
        file_parses.insert("App.tsx".to_string(), parse);

        let model = build_project_state_model(&file_parses, &BTreeMap::new());
        assert_eq!(model.stats.total_state_vars, 1);
    }

    #[test]
    fn project_state_model_detects_global_stores_when_contents_are_available() {
        let src = r#"
export const appStore = createStore({ reducer });
function App() {
    return <div />;
}
"#;
        let parse = parse_file(src, "App.tsx");
        let mut file_parses = BTreeMap::new();
        file_parses.insert("App.tsx".to_string(), parse);
        let mut file_contents = BTreeMap::new();
        file_contents.insert("App.tsx".to_string(), src.to_string());

        let model = build_project_state_model(&file_parses, &file_contents);
        assert_eq!(model.global_state_stores.len(), 1);
        assert_eq!(model.global_state_stores[0].name, "appStore");
        assert_eq!(model.global_state_stores[0].kind, GlobalStoreKind::Redux);
    }

    #[test]
    fn project_state_model_resolves_context_provider_edges() {
        let provider_src = r#"
function App() {
    return <ThemeContext.Provider value={theme}><ThemeButton /></ThemeContext.Provider>;
}
"#;
        let consumer_src = r#"
function ThemeButton() {
    const theme = useContext(ThemeContext);
    return <button />;
}
"#;
        let mut file_parses = BTreeMap::new();
        file_parses.insert(
            "src/App.tsx".to_string(),
            parse_file(provider_src, "src/App.tsx"),
        );
        file_parses.insert(
            "src/ThemeButton.tsx".to_string(),
            parse_file(consumer_src, "src/ThemeButton.tsx"),
        );

        let model = build_project_state_model(&file_parses, &BTreeMap::new());
        assert_eq!(model.context_graph.len(), 1);
        assert_eq!(
            model.context_graph[0].provider_component,
            "src/App.tsx::App"
        );
        assert_eq!(
            model.context_graph[0].consumer_component,
            "src/ThemeButton.tsx::ThemeButton"
        );
        assert_eq!(model.context_graph[0].context_name, "ThemeContext");
    }

    #[test]
    fn project_state_model_deduplicates_repeated_context_reads_in_one_component() {
        let provider_src = r#"
function App() {
    return <ThemeContext.Provider value={theme}><ThemeButton /></ThemeContext.Provider>;
}
"#;
        let consumer_src = r#"
function ThemeButton() {
    const theme = useContext(ThemeContext);
    const inheritedTheme = useContext(ThemeContext);
    return <button data-theme={theme} data-alt-theme={inheritedTheme} />;
}
"#;
        let mut file_parses = BTreeMap::new();
        file_parses.insert(
            "src/App.tsx".to_string(),
            parse_file(provider_src, "src/App.tsx"),
        );
        file_parses.insert(
            "src/ThemeButton.tsx".to_string(),
            parse_file(consumer_src, "src/ThemeButton.tsx"),
        );

        let model = build_project_state_model(&file_parses, &BTreeMap::new());
        assert_eq!(
            model.context_graph.len(),
            1,
            "graph should contain unique provider→consumer edges rather than duplicating repeated reads"
        );
        assert_eq!(model.context_graph[0].context_name, "ThemeContext");
    }

    #[test]
    fn project_state_model_does_not_link_consumer_to_provider_in_same_component() {
        let src = r#"
function ThemeBridge() {
    const theme = useContext(ThemeContext);
    return <ThemeContext.Provider value={theme}><button /></ThemeContext.Provider>;
}
"#;
        let mut file_parses = BTreeMap::new();
        file_parses.insert(
            "src/ThemeBridge.tsx".to_string(),
            parse_file(src, "src/ThemeBridge.tsx"),
        );

        let model = build_project_state_model(&file_parses, &BTreeMap::new());
        assert!(
            model.context_graph.is_empty(),
            "same-component providers should not satisfy useContext in that component"
        );
    }

    #[test]
    fn project_state_model_avoids_ambiguous_context_provider_edges() {
        let provider_a_src = r#"
function ThemeShell() {
    return <ThemeContext.Provider value={theme}><Outlet /></ThemeContext.Provider>;
}
"#;
        let provider_b_src = r#"
function ThemeModal() {
    return <ThemeContext.Provider value={theme}><Dialog /></ThemeContext.Provider>;
}
"#;
        let consumer_src = r#"
function ThemeButton() {
    const theme = useContext(ThemeContext);
    return <button />;
}
"#;
        let mut file_parses = BTreeMap::new();
        file_parses.insert(
            "src/ThemeShell.tsx".to_string(),
            parse_file(provider_a_src, "src/ThemeShell.tsx"),
        );
        file_parses.insert(
            "src/ThemeModal.tsx".to_string(),
            parse_file(provider_b_src, "src/ThemeModal.tsx"),
        );
        file_parses.insert(
            "src/ThemeButton.tsx".to_string(),
            parse_file(consumer_src, "src/ThemeButton.tsx"),
        );

        let model = build_project_state_model(&file_parses, &BTreeMap::new());
        assert!(
            model.context_graph.is_empty(),
            "multiple candidate providers should stay unresolved until ancestry is known"
        );
    }

    #[test]
    fn project_state_model_aggregates_capabilities_and_risk_flags() {
        let src = r#"
function App() {
    useEffect(graphqlClientQuery, []);
    useEffect(setInterval, []);
    useEffect(window.addEventListener, []);
    return <div />;
}
"#;
        let parse = parse_file(src, "App.tsx");
        let mut file_parses = BTreeMap::new();
        file_parses.insert("App.tsx".to_string(), parse);

        let model = build_project_state_model(&file_parses, &BTreeMap::new());
        assert!(
            model
                .required_capabilities
                .contains(&Capability::NetworkAccess)
        );
        assert!(model.required_capabilities.contains(&Capability::Timers));
        assert!(
            model
                .optional_capabilities
                .contains(&Capability::KeyboardInput)
        );
        assert!(!model.platform_assumptions.is_empty());
        assert!(
            model
                .risk_flags
                .iter()
                .any(|f| f.level == CapabilityRiskLevel::Blocking)
        );
    }

    #[test]
    fn model_serializes_to_json() {
        let src = r#"
function App() {
    const [x, setX] = useState(0);
    return <div />;
}
"#;
        let parse = parse_file(src, "App.tsx");
        let models = extract_state_effects(&parse);
        let json = serde_json::to_string_pretty(&models).unwrap();
        assert!(json.contains("useState"));
        assert!(json.contains("Local"));
    }

    #[test]
    fn writes_extraction_from_snippet() {
        let writes =
            extract_writes_from_snippet("setCount(prev => prev + 1); dispatch({ type: 'INC' })");
        assert!(writes.contains(&"setCount".to_string()));
        assert!(writes.contains(&"dispatch".to_string()));
    }

    #[test]
    fn effect_cleanup_and_timer_classification_direct() {
        // Test cleanup detection and timer classification directly
        // (tsx_parser can't capture nested-paren args for useEffect).
        let snippet =
            "() => { const id = setInterval(tick, 1000); return () => clearInterval(id); }";
        assert!(has_cleanup_return(snippet));
        assert_eq!(classify_effect(snippet), EffectClassification::Timer);
    }

    #[test]
    fn effect_cleanup_detection_handles_compact_arrow_cleanup() {
        let snippet = "() => { return()=>unsubscribe(); }";
        assert!(has_cleanup_return(snippet));
    }

    #[test]
    fn effect_cleanup_detection_ignores_return_prefixed_identifiers() {
        let snippet = "() => { const returnValue = 1; returnValue.toString(); }";
        assert!(!has_cleanup_return(snippet));
    }

    #[test]
    fn effect_cleanup_detection_ignores_plain_return_expression() {
        let snippet = "() => { const n = 1; return n + 1; }";
        assert!(!has_cleanup_return(snippet));
    }

    #[test]
    fn effect_cleanup_detection_accepts_returned_cleanup_reference() {
        let snippet = "() => { const unsubscribe = listen(); return unsubscribe; }";
        assert!(has_cleanup_return(snippet));
    }

    #[test]
    fn effect_extracted_from_parsed_source() {
        let src = r#"
function Timer() {
    useEffect(() => { setInterval(t, 1000); }, []);
    return <div />;
}
"#;
        let parse = parse_file(src, "Timer.tsx");
        let models = extract_state_effects(&parse);
        assert_eq!(models[0].effects.len(), 1);
        assert_eq!(models[0].effects[0].hook, "useEffect");
    }
}
