//! Choreographic Programming for multi-widget interactions (bd-14k2m).
//!
//! Defines coordinated multi-widget behavior from a **global** specification
//! and automatically projects it to per-widget message handlers.
//!
//! # Motivation
//!
//! In Elm/Bubbletea architecture, coordinated behavior across multiple widgets
//! requires manually routing messages. Missing a message means inconsistent
//! state. Choreographic programming (Montesi 2013) eliminates this class of
//! bugs by:
//!
//! 1. Define the choreography once (global specification).
//! 2. Project to individual widgets automatically.
//! 3. Guarantee completeness (no missing messages).
//!
//! # Example
//!
//! ```
//! use ftui_widgets::choreography::*;
//!
//! let mut choreo = Choreography::new("filter_update");
//!
//! // Global specification: when filter changes, update list + status + header
//! choreo.step("filter", Action::Emit("filter_changed".to_string()));
//! choreo.step(
//!     "list",
//!     Action::Handle("filter_changed".to_string(), "scroll_to_top".to_string()),
//! );
//! choreo.step(
//!     "status",
//!     Action::Handle("filter_changed".to_string(), "update_count".to_string()),
//! );
//! choreo.step(
//!     "header",
//!     Action::Handle("filter_changed".to_string(), "highlight_filter".to_string()),
//! );
//!
//! // Project to per-widget handlers
//! let projection = choreo.project();
//! assert_eq!(projection.handlers("filter").len(), 1);
//! assert_eq!(projection.handlers("list").len(), 1);
//! assert_eq!(projection.handlers("status").len(), 1);
//! assert_eq!(projection.handlers("header").len(), 1);
//!
//! // Verify deadlock freedom
//! assert!(choreo.verify_deadlock_free());
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

/// A participant in a choreography (typically a widget).
pub type ParticipantId = String;

/// A message type exchanged between participants.
pub type MessageType = String;

/// An action in a choreography step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Emit a message to the choreography.
    Emit(MessageType),
    /// Handle a received message by executing a named handler.
    Handle(MessageType, String),
    /// Internal computation (no message exchange).
    Local(String),
}

/// A single step in a choreography.
#[derive(Debug, Clone)]
pub struct ChoreographyStep {
    /// Which participant performs this step.
    pub participant: ParticipantId,
    /// What the participant does.
    pub action: Action,
    /// Step index in the choreography sequence.
    pub sequence: usize,
}

/// A global choreography specification.
#[derive(Debug, Clone)]
pub struct Choreography {
    /// Name of this choreography (e.g., "filter_update").
    pub name: String,
    /// Ordered sequence of steps.
    steps: Vec<ChoreographyStep>,
}

impl Choreography {
    /// Create a new choreography with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
        }
    }

    /// Add a step to the choreography.
    pub fn step(&mut self, participant: impl Into<String>, action: Action) -> &mut Self {
        let seq = self.steps.len();
        self.steps.push(ChoreographyStep {
            participant: participant.into(),
            action,
            sequence: seq,
        });
        self
    }

    /// Get all steps.
    pub fn steps(&self) -> &[ChoreographyStep] {
        &self.steps
    }

    /// Get all unique participants.
    pub fn participants(&self) -> BTreeSet<ParticipantId> {
        self.steps.iter().map(|s| s.participant.clone()).collect()
    }

    /// Get all message types used.
    pub fn message_types(&self) -> BTreeSet<MessageType> {
        let mut types = BTreeSet::new();
        for step in &self.steps {
            match &step.action {
                Action::Emit(msg) => {
                    types.insert(msg.clone());
                }
                Action::Handle(msg, _) => {
                    types.insert(msg.clone());
                }
                Action::Local(_) => {}
            }
        }
        types
    }

    /// Project the choreography to per-participant handlers.
    pub fn project(&self) -> Projection {
        let mut handlers: BTreeMap<ParticipantId, Vec<ProjectedHandler>> = BTreeMap::new();

        for step in &self.steps {
            let handler = match &step.action {
                Action::Emit(msg) => ProjectedHandler {
                    kind: HandlerKind::Emit,
                    message: msg.clone(),
                    handler_name: format!("emit_{msg}"),
                    sequence: step.sequence,
                },
                Action::Handle(msg, name) => ProjectedHandler {
                    kind: HandlerKind::Receive,
                    message: msg.clone(),
                    handler_name: name.clone(),
                    sequence: step.sequence,
                },
                Action::Local(name) => ProjectedHandler {
                    kind: HandlerKind::Local,
                    message: String::new(),
                    handler_name: name.clone(),
                    sequence: step.sequence,
                },
            };
            handlers
                .entry(step.participant.clone())
                .or_default()
                .push(handler);
        }

        Projection { handlers }
    }

    /// Verify that the choreography is deadlock-free.
    ///
    /// A choreography is deadlock-free if:
    /// 1. Every emitted message has at least one handler.
    /// 2. Every handled message has at least one emitter.
    /// 3. The message dependency graph is acyclic.
    pub fn verify_deadlock_free(&self) -> bool {
        let mut emitters: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut receivers: HashMap<&str, Vec<usize>> = HashMap::new();

        for (i, step) in self.steps.iter().enumerate() {
            match &step.action {
                Action::Emit(msg) => emitters.entry(msg.as_str()).or_default().push(i),
                Action::Handle(msg, _) => receivers.entry(msg.as_str()).or_default().push(i),
                Action::Local(_) => {}
            }
        }

        // Check: every emitted message has at least one receiver.
        for msg in emitters.keys() {
            if !receivers.contains_key(msg) {
                return false;
            }
        }

        // Check: every received message has at least one emitter.
        for msg in receivers.keys() {
            if !emitters.contains_key(msg) {
                return false;
            }
        }

        // Check: causal ordering is respected (emit before handle in sequence).
        for (msg, emit_indices) in &emitters {
            if let Some(recv_indices) = receivers.get(msg) {
                let min_emit = emit_indices.iter().min().copied().unwrap_or(usize::MAX);
                let min_recv = recv_indices.iter().min().copied().unwrap_or(0);
                if min_recv < min_emit {
                    return false; // Handler runs before any emitter
                }
            }
        }

        // Check: no circular dependencies.
        // Build a graph: participant A depends on B if A handles a message emitted by B.
        let mut dep_graph: HashMap<&str, HashSet<&str>> = HashMap::new();
        for step in &self.steps {
            if let Action::Handle(msg, _) = &step.action {
                // Find emitter of this message.
                for emit_step in &self.steps {
                    if let Action::Emit(emsg) = &emit_step.action
                        && emsg == msg
                    {
                        dep_graph
                            .entry(step.participant.as_str())
                            .or_default()
                            .insert(emit_step.participant.as_str());
                    }
                }
            }
        }

        // Cycle detection via BFS/coloring.
        !has_cycle(&dep_graph)
    }

    /// Completeness check: are there unhandled messages or unmatched handlers?
    pub fn completeness_report(&self) -> CompletenessReport {
        let mut emitted: BTreeSet<String> = BTreeSet::new();
        let mut handled: BTreeSet<String> = BTreeSet::new();

        for step in &self.steps {
            match &step.action {
                Action::Emit(msg) => {
                    emitted.insert(msg.clone());
                }
                Action::Handle(msg, _) => {
                    handled.insert(msg.clone());
                }
                Action::Local(_) => {}
            }
        }

        let unhandled: Vec<String> = emitted.difference(&handled).cloned().collect();
        let orphaned: Vec<String> = handled.difference(&emitted).cloned().collect();

        CompletenessReport {
            is_complete: unhandled.is_empty() && orphaned.is_empty(),
            unhandled_messages: unhandled,
            orphaned_handlers: orphaned,
            participant_count: self.participants().len(),
            message_count: self.message_types().len(),
            step_count: self.steps.len(),
        }
    }
}

/// Check if a directed graph has a cycle.
fn has_cycle<'a>(graph: &HashMap<&'a str, HashSet<&'a str>>) -> bool {
    let mut visited = HashSet::new();
    let mut in_stack = HashSet::new();

    for &node in graph.keys() {
        if !visited.contains(node) && dfs_cycle(node, graph, &mut visited, &mut in_stack) {
            return true;
        }
    }
    false
}

fn dfs_cycle<'a>(
    node: &'a str,
    graph: &HashMap<&'a str, HashSet<&'a str>>,
    visited: &mut HashSet<&'a str>,
    in_stack: &mut HashSet<&'a str>,
) -> bool {
    visited.insert(node);
    in_stack.insert(node);

    if let Some(neighbors) = graph.get(node) {
        for &next in neighbors {
            if !visited.contains(next) {
                if dfs_cycle(next, graph, visited, in_stack) {
                    return true;
                }
            } else if in_stack.contains(next) {
                return true;
            }
        }
    }

    in_stack.remove(node);
    false
}

/// A projected handler for a single participant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedHandler {
    /// What kind of handler this is.
    pub kind: HandlerKind,
    /// The message type involved (empty for Local).
    pub message: String,
    /// The handler function name.
    pub handler_name: String,
    /// Original sequence number from the choreography.
    pub sequence: usize,
}

/// Kind of projected handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerKind {
    /// This participant emits the message.
    Emit,
    /// This participant receives and handles the message.
    Receive,
    /// This participant does local computation.
    Local,
}

/// Projected per-participant handlers from a choreography.
#[derive(Debug, Clone)]
pub struct Projection {
    handlers: BTreeMap<ParticipantId, Vec<ProjectedHandler>>,
}

impl Projection {
    /// Get handlers for a specific participant.
    pub fn handlers(&self, participant: &str) -> &[ProjectedHandler] {
        self.handlers
            .get(participant)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get all participants in the projection.
    pub fn participants(&self) -> Vec<&str> {
        self.handlers.keys().map(|s| s.as_str()).collect()
    }

    /// Check if the projection is complete (all messages are balanced).
    pub fn is_complete(&self) -> bool {
        let mut emitted: HashSet<&str> = HashSet::new();
        let mut received: HashSet<&str> = HashSet::new();

        for handlers in self.handlers.values() {
            for h in handlers {
                match h.kind {
                    HandlerKind::Emit => {
                        emitted.insert(h.message.as_str());
                    }
                    HandlerKind::Receive => {
                        received.insert(h.message.as_str());
                    }
                    HandlerKind::Local => {}
                }
            }
        }

        emitted == received
    }
}

/// Report on choreography completeness.
#[derive(Debug, Clone)]
pub struct CompletenessReport {
    /// Whether the choreography is complete.
    pub is_complete: bool,
    /// Messages that are emitted but never handled.
    pub unhandled_messages: Vec<String>,
    /// Messages that are handled but never emitted.
    pub orphaned_handlers: Vec<String>,
    /// Number of participants.
    pub participant_count: usize,
    /// Number of distinct message types.
    pub message_count: usize,
    /// Total number of choreography steps.
    pub step_count: usize,
}

/// Execute a choreography against a set of participant state callbacks.
///
/// Returns the ordered list of messages delivered during execution.
pub fn execute_choreography(choreo: &Choreography) -> Vec<ExecutionEvent> {
    let mut events = Vec::new();
    let mut pending_messages: VecDeque<(String, usize)> = VecDeque::new();

    for step in choreo.steps() {
        match &step.action {
            Action::Emit(msg) => {
                pending_messages.push_back((msg.clone(), step.sequence));
                events.push(ExecutionEvent {
                    sequence: step.sequence,
                    participant: step.participant.clone(),
                    kind: EventKind::Emit(msg.clone()),
                });
            }
            Action::Handle(msg, handler) => {
                // Check if this message has been emitted.
                let delivered = pending_messages.iter().any(|(m, _)| m == msg);
                events.push(ExecutionEvent {
                    sequence: step.sequence,
                    participant: step.participant.clone(),
                    kind: if delivered {
                        EventKind::Deliver(msg.clone(), handler.clone())
                    } else {
                        EventKind::Deadlock(msg.clone())
                    },
                });
            }
            Action::Local(name) => {
                events.push(ExecutionEvent {
                    sequence: step.sequence,
                    participant: step.participant.clone(),
                    kind: EventKind::Local(name.clone()),
                });
            }
        }
    }

    events
}

/// An event from choreography execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionEvent {
    /// Step sequence number.
    pub sequence: usize,
    /// Which participant was involved.
    pub participant: ParticipantId,
    /// What happened.
    pub kind: EventKind,
}

/// Kind of execution event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// Message was emitted.
    Emit(String),
    /// Message was delivered to handler.
    Deliver(String, String),
    /// Local computation.
    Local(String),
    /// Deadlock: message needed but not yet emitted.
    Deadlock(String),
}

// ============================================================================
// Example Choreographies
// ============================================================================

/// Filter update: filter dropdown → list + status bar + header.
pub fn example_filter_update() -> Choreography {
    let mut c = Choreography::new("filter_update");
    c.step("filter", Action::Emit("filter_changed".into()));
    c.step(
        "list",
        Action::Handle("filter_changed".into(), "scroll_to_top".into()),
    );
    c.step(
        "status",
        Action::Handle("filter_changed".into(), "update_count".into()),
    );
    c.step(
        "header",
        Action::Handle("filter_changed".into(), "highlight_filter".into()),
    );
    c
}

/// Selection sync: list selection → detail panel + status bar.
pub fn example_selection_sync() -> Choreography {
    let mut c = Choreography::new("selection_sync");
    c.step("list", Action::Emit("selection_changed".into()));
    c.step(
        "detail",
        Action::Handle("selection_changed".into(), "show_details".into()),
    );
    c.step(
        "status",
        Action::Handle("selection_changed".into(), "update_selection_info".into()),
    );
    c
}

/// Tab navigation: tab bar → content panel + breadcrumb + status.
pub fn example_tab_navigation() -> Choreography {
    let mut c = Choreography::new("tab_navigation");
    c.step("tabs", Action::Emit("tab_changed".into()));
    c.step(
        "content",
        Action::Handle("tab_changed".into(), "switch_view".into()),
    );
    c.step(
        "breadcrumb",
        Action::Handle("tab_changed".into(), "update_path".into()),
    );
    c.step(
        "status",
        Action::Handle("tab_changed".into(), "update_tab_info".into()),
    );
    c
}

/// Search flow: search input → results list → status + preview.
pub fn example_search_flow() -> Choreography {
    let mut c = Choreography::new("search_flow");
    c.step("search_input", Action::Emit("query_changed".into()));
    c.step("search_input", Action::Local("debounce".into()));
    c.step(
        "results",
        Action::Handle("query_changed".into(), "filter_results".into()),
    );
    c.step("results", Action::Emit("results_updated".into()));
    c.step(
        "status",
        Action::Handle("results_updated".into(), "update_match_count".into()),
    );
    c.step(
        "preview",
        Action::Handle("results_updated".into(), "update_preview".into()),
    );
    c
}

/// Modal dialog: trigger → overlay + dimmer + focus trap.
pub fn example_modal_dialog() -> Choreography {
    let mut c = Choreography::new("modal_dialog");
    c.step("trigger", Action::Emit("open_modal".into()));
    c.step(
        "overlay",
        Action::Handle("open_modal".into(), "show_overlay".into()),
    );
    c.step(
        "dimmer",
        Action::Handle("open_modal".into(), "dim_background".into()),
    );
    c.step(
        "focus_trap",
        Action::Handle("open_modal".into(), "capture_focus".into()),
    );
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_update_is_deadlock_free() {
        let c = example_filter_update();
        assert!(c.verify_deadlock_free());
    }

    #[test]
    fn filter_update_projection() {
        let c = example_filter_update();
        let p = c.project();
        assert_eq!(p.participants().len(), 4);
        assert_eq!(p.handlers("filter").len(), 1);
        assert_eq!(p.handlers("list").len(), 1);
        assert_eq!(p.handlers("status").len(), 1);
        assert_eq!(p.handlers("header").len(), 1);
        assert!(p.is_complete());
    }

    #[test]
    fn filter_update_completeness() {
        let c = example_filter_update();
        let report = c.completeness_report();
        assert!(report.is_complete);
        assert_eq!(report.participant_count, 4);
        assert_eq!(report.message_count, 1);
        assert_eq!(report.step_count, 4);
    }

    #[test]
    fn selection_sync_works() {
        let c = example_selection_sync();
        assert!(c.verify_deadlock_free());
        let p = c.project();
        assert!(p.is_complete());
        assert_eq!(p.participants().len(), 3);
    }

    #[test]
    fn tab_navigation_works() {
        let c = example_tab_navigation();
        assert!(c.verify_deadlock_free());
        assert!(c.project().is_complete());
    }

    #[test]
    fn search_flow_chained_messages() {
        let c = example_search_flow();
        assert!(c.verify_deadlock_free());
        let p = c.project();
        assert!(p.is_complete());
        assert_eq!(c.message_types().len(), 2); // query_changed + results_updated
    }

    #[test]
    fn modal_dialog_works() {
        let c = example_modal_dialog();
        assert!(c.verify_deadlock_free());
        assert!(c.project().is_complete());
    }

    #[test]
    fn orphaned_handler_detected() {
        let mut c = Choreography::new("broken");
        c.step(
            "widget_a",
            Action::Handle("nonexistent".into(), "do_stuff".into()),
        );
        let report = c.completeness_report();
        assert!(!report.is_complete);
        assert_eq!(report.orphaned_handlers, vec!["nonexistent"]);
    }

    #[test]
    fn unhandled_message_detected() {
        let mut c = Choreography::new("incomplete");
        c.step("widget_a", Action::Emit("fire_and_forget".into()));
        let report = c.completeness_report();
        assert!(!report.is_complete);
        assert_eq!(report.unhandled_messages, vec!["fire_and_forget"]);
    }

    #[test]
    fn deadlock_from_missing_emitter() {
        let mut c = Choreography::new("deadlock");
        c.step(
            "widget_a",
            Action::Handle("missing".into(), "handler".into()),
        );
        assert!(!c.verify_deadlock_free());
    }

    #[test]
    fn deadlock_from_handle_before_emit() {
        let mut c = Choreography::new("out_of_order");
        c.step("widget_b", Action::Handle("msg".into(), "process".into()));
        c.step("widget_a", Action::Emit("msg".into()));
        assert!(!c.verify_deadlock_free());
    }

    #[test]
    fn circular_dependency_detected() {
        let mut c = Choreography::new("circular");
        c.step("a", Action::Emit("msg_ab".into()));
        c.step("b", Action::Handle("msg_ab".into(), "handle_ab".into()));
        c.step("b", Action::Emit("msg_ba".into()));
        c.step("a", Action::Handle("msg_ba".into(), "handle_ba".into()));
        assert!(!c.verify_deadlock_free());
    }

    #[test]
    fn execute_filter_update() {
        let c = example_filter_update();
        let events = execute_choreography(&c);
        assert_eq!(events.len(), 4);
        assert!(matches!(&events[0].kind, EventKind::Emit(m) if m == "filter_changed"));
        assert!(matches!(&events[1].kind, EventKind::Deliver(m, _) if m == "filter_changed"));
        assert!(matches!(&events[2].kind, EventKind::Deliver(m, _) if m == "filter_changed"));
        assert!(matches!(&events[3].kind, EventKind::Deliver(m, _) if m == "filter_changed"));
    }

    #[test]
    fn execute_detects_deadlock_event() {
        let mut c = Choreography::new("deadlock_exec");
        c.step(
            "widget_b",
            Action::Handle("missing".into(), "handler".into()),
        );
        let events = execute_choreography(&c);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0].kind, EventKind::Deadlock(m) if m == "missing"));
    }

    #[test]
    fn empty_choreography() {
        let c = Choreography::new("empty");
        assert!(c.verify_deadlock_free());
        assert!(c.project().is_complete());
        let report = c.completeness_report();
        assert!(report.is_complete);
        assert_eq!(report.step_count, 0);
    }

    #[test]
    fn local_actions_preserved() {
        let mut c = Choreography::new("with_local");
        c.step("widget", Action::Local("compute".into()));
        let p = c.project();
        assert_eq!(p.handlers("widget").len(), 1);
        assert_eq!(p.handlers("widget")[0].kind, HandlerKind::Local);
    }

    #[test]
    fn choreography_determinism() {
        let c1 = example_search_flow();
        let c2 = example_search_flow();
        let p1 = c1.project();
        let p2 = c2.project();
        assert_eq!(p1.participants(), p2.participants());
        for part in p1.participants() {
            assert_eq!(p1.handlers(part), p2.handlers(part));
        }
    }

    #[test]
    fn comparison_manual_vs_choreographic() {
        // Manual approach: 4 separate message routes
        // Choreographic approach: 1 choreography, 4 projected handlers
        let c = example_filter_update();
        let p = c.project();

        // The choreographic version produces the same handlers
        // but guarantees completeness
        assert_eq!(p.handlers("filter")[0].kind, HandlerKind::Emit);
        assert_eq!(p.handlers("list")[0].kind, HandlerKind::Receive);
        assert_eq!(p.handlers("status")[0].kind, HandlerKind::Receive);
        assert_eq!(p.handlers("header")[0].kind, HandlerKind::Receive);

        // Completeness is verified
        assert!(c.completeness_report().is_complete);
    }
}
