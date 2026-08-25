#![forbid(unsafe_code)]

//! Presenter equivalence rules and state-churn minimization model (bd-ddruy).
//!
//! Defines what counts as "terminal-visible equivalence" for presenter output
//! optimization. Two ANSI byte streams are equivalent if they produce identical
//! visible cell content, colors, and attributes on any conforming terminal.
//!
//! # Core principle
//!
//! Fewer bytes and fewer state transitions are only beneficial if terminal-visible
//! behavior remains intact. This module makes the equivalence boundary explicit
//! so presenter optimizations have a crisp correctness contract.
//!
//! # Equivalence classes
//!
//! | Class | Rule | Example |
//! |-------|------|---------|
//! | `CursorPath` | Different cursor paths producing same cell writes are equivalent | `\x1b[H\x1b[3C` vs `\x1b[1;4H` |
//! | `ResetVariant` | Different SGR reset forms producing same attribute state are equivalent | `\x1b[0m` vs `\x1b[m` |
//! | `StyleOrder` | SGR parameters in any order within one sequence are equivalent | `\x1b[1;31m` vs `\x1b[31;1m` |
//! | `RedundantState` | Redundant state-set sequences can be suppressed | `\x1b[31m\x1b[31m` → `\x1b[31m` |
//! | `BatchedWrites` | Multiple small writes coalesced into one are equivalent | two `write()` calls vs one |
//!
//! # Non-equivalent variations (must NOT be suppressed)
//!
//! | Variation | Why it matters |
//! |-----------|---------------|
//! | Missing reset at row end | Leaks style into scrollback |
//! | Wrong cursor position | Writes to wrong cell |
//! | Dropped attributes | Visual corruption |
//! | Color depth mismatch | Wrong colors on limited terminals |

// ============================================================================
// Equivalence Classes
// ============================================================================

/// Categories of ANSI output variations that are terminal-visible equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EquivalenceClass {
    /// Different cursor movement sequences that reach the same position.
    CursorPath,
    /// Different forms of SGR reset that produce the same attribute state.
    ResetVariant,
    /// SGR parameters in different order within one escape sequence.
    StyleOrder,
    /// Duplicate state-set sequences where the state is already active.
    RedundantState,
    /// Multiple write syscalls coalesced into fewer (or one) write.
    BatchedWrites,
    /// Trailing whitespace differences that don't affect visible content.
    TrailingWhitespace,
}

impl EquivalenceClass {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::CursorPath => "cursor-path",
            Self::ResetVariant => "reset-variant",
            Self::StyleOrder => "style-order",
            Self::RedundantState => "redundant-state",
            Self::BatchedWrites => "batched-writes",
            Self::TrailingWhitespace => "trailing-whitespace",
        }
    }

    /// Whether this class is safe to exploit for optimization.
    #[must_use]
    pub const fn safe_to_optimize(&self) -> bool {
        // All defined equivalence classes are safe by definition —
        // if they weren't safe, they wouldn't be equivalence classes.
        true
    }

    /// Typical byte savings from exploiting this equivalence.
    #[must_use]
    pub const fn typical_savings(&self) -> TypicalSavings {
        match self {
            Self::CursorPath => TypicalSavings::Medium,
            Self::ResetVariant => TypicalSavings::Small,
            Self::StyleOrder => TypicalSavings::Negligible,
            Self::RedundantState => TypicalSavings::Large,
            Self::BatchedWrites => TypicalSavings::Medium, // syscall reduction
            Self::TrailingWhitespace => TypicalSavings::Small,
        }
    }

    pub const ALL: &'static [EquivalenceClass] = &[
        Self::CursorPath,
        Self::ResetVariant,
        Self::StyleOrder,
        Self::RedundantState,
        Self::BatchedWrites,
        Self::TrailingWhitespace,
    ];
}

/// Rough savings magnitude from an optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TypicalSavings {
    /// < 1% byte reduction.
    Negligible,
    /// 1-5% byte reduction.
    Small,
    /// 5-20% byte reduction.
    Medium,
    /// > 20% byte reduction.
    Large,
}

impl TypicalSavings {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Negligible => "negligible",
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

// ============================================================================
// Non-Equivalent Variations (Safety Boundary)
// ============================================================================

/// Variations that are NOT equivalent and must never be suppressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NonEquivalentVariation {
    /// Missing style reset at end of row (leaks into scrollback).
    MissingRowReset,
    /// Cursor positioned at wrong cell.
    WrongCursorPosition,
    /// SGR attribute dropped or changed.
    DroppedAttribute,
    /// Color rendered at wrong depth (e.g., 256-color vs true-color).
    ColorDepthMismatch,
    /// Character content differs.
    ContentMismatch,
    /// Wide character continuation cell misaligned.
    WideCharMisalignment,
    /// Hyperlink (OSC 8) state leaked or dropped.
    HyperlinkStateLeak,
}

impl NonEquivalentVariation {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::MissingRowReset => "missing-row-reset",
            Self::WrongCursorPosition => "wrong-cursor-position",
            Self::DroppedAttribute => "dropped-attribute",
            Self::ColorDepthMismatch => "color-depth-mismatch",
            Self::ContentMismatch => "content-mismatch",
            Self::WideCharMisalignment => "wide-char-misalignment",
            Self::HyperlinkStateLeak => "hyperlink-state-leak",
        }
    }

    /// Severity if this violation occurs.
    #[must_use]
    pub const fn severity(&self) -> ViolationSeverity {
        match self {
            Self::MissingRowReset => ViolationSeverity::Major,
            Self::WrongCursorPosition => ViolationSeverity::Critical,
            Self::DroppedAttribute => ViolationSeverity::Major,
            Self::ColorDepthMismatch => ViolationSeverity::Minor,
            Self::ContentMismatch => ViolationSeverity::Critical,
            Self::WideCharMisalignment => ViolationSeverity::Major,
            Self::HyperlinkStateLeak => ViolationSeverity::Minor,
        }
    }

    pub const ALL: &'static [NonEquivalentVariation] = &[
        Self::MissingRowReset,
        Self::WrongCursorPosition,
        Self::DroppedAttribute,
        Self::ColorDepthMismatch,
        Self::ContentMismatch,
        Self::WideCharMisalignment,
        Self::HyperlinkStateLeak,
    ];
}

/// Severity of an equivalence violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViolationSeverity {
    /// Visual oddity but no data corruption.
    Minor,
    /// Visible corruption or style leakage.
    Major,
    /// Wrong cell content or position — terminal output is incorrect.
    Critical,
}

impl ViolationSeverity {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::Major => "major",
            Self::Critical => "critical",
        }
    }
}

// ============================================================================
// State Transition Model
// ============================================================================

/// Presenter state that is tracked to suppress redundant transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackedState {
    /// Foreground color.
    Foreground,
    /// Background color.
    Background,
    /// Bold attribute.
    Bold,
    /// Italic attribute.
    Italic,
    /// Underline attribute.
    Underline,
    /// Strikethrough attribute.
    Strikethrough,
    /// Dim attribute.
    Dim,
    /// Reverse video attribute.
    Reverse,
    /// Hidden attribute.
    Hidden,
    /// Blink attribute.
    Blink,
    /// Cursor position (row, column).
    CursorPosition,
}

impl TrackedState {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Foreground => "fg",
            Self::Background => "bg",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Underline => "underline",
            Self::Strikethrough => "strikethrough",
            Self::Dim => "dim",
            Self::Reverse => "reverse",
            Self::Hidden => "hidden",
            Self::Blink => "blink",
            Self::CursorPosition => "cursor",
        }
    }

    /// Whether this state can be suppressed when unchanged.
    #[must_use]
    pub const fn suppressible(&self) -> bool {
        // All tracked states can be suppressed when the new value
        // equals the current tracked value.
        true
    }

    /// Typical byte cost of emitting this state transition.
    #[must_use]
    pub const fn transition_cost_bytes(&self) -> u32 {
        match self {
            Self::Foreground | Self::Background => 16, // \x1b[38;2;R;G;Bm
            Self::CursorPosition => 8,                 // \x1b[R;CH
            _ => 4,                                    // \x1b[Nm
        }
    }
}

// ============================================================================
// Suppression Decision
// ============================================================================

/// Result of evaluating whether a state transition can be suppressed.
#[derive(Debug, Clone)]
pub struct SuppressionDecision {
    /// Which state was evaluated.
    pub state: TrackedState,
    /// Whether the transition was suppressed.
    pub suppressed: bool,
    /// Bytes saved by suppression (0 if not suppressed).
    pub bytes_saved: u32,
    /// Reason for the decision.
    pub reason: SuppressionReason,
}

/// Why a suppression decision was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionReason {
    /// New value equals tracked value — safe to suppress.
    Redundant,
    /// Value changed — must emit transition.
    Changed,
    /// First emission in frame — no tracked state to compare.
    Initial,
    /// Conservative: terminal mode requires explicit state.
    ConservativeMode,
}

impl SuppressionReason {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Redundant => "redundant",
            Self::Changed => "changed",
            Self::Initial => "initial",
            Self::ConservativeMode => "conservative-mode",
        }
    }
}

// ============================================================================
// Transcript Comparison
// ============================================================================

/// A transcript entry for comparing presenter output.
#[derive(Debug, Clone)]
pub struct TranscriptEntry {
    /// Byte offset in the output stream.
    pub offset: usize,
    /// ANSI escape sequence or literal text.
    pub content: String,
    /// What this entry does.
    pub effect: TranscriptEffect,
}

/// What a transcript entry does to terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptEffect {
    /// Moves cursor to a position.
    CursorMove,
    /// Sets one or more SGR attributes.
    StyleSet,
    /// Resets attributes.
    StyleReset,
    /// Writes visible character(s).
    Content,
    /// Other control sequence.
    Control,
}

impl TranscriptEffect {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::CursorMove => "cursor-move",
            Self::StyleSet => "style-set",
            Self::StyleReset => "style-reset",
            Self::Content => "content",
            Self::Control => "control",
        }
    }
}

/// Result of comparing two presenter transcripts.
#[derive(Debug, Clone)]
pub struct TranscriptComparison {
    /// Whether the transcripts are terminal-visible equivalent.
    pub equivalent: bool,
    /// Byte count of the first (baseline) transcript.
    pub baseline_bytes: usize,
    /// Byte count of the second (optimized) transcript.
    pub optimized_bytes: usize,
    /// Bytes saved (positive = smaller optimized output).
    pub bytes_saved: i64,
    /// Equivalence classes exploited.
    pub exploited_classes: Vec<EquivalenceClass>,
    /// Violations found (empty if equivalent).
    pub violations: Vec<NonEquivalentVariation>,
}

impl TranscriptComparison {
    /// Byte reduction percentage.
    #[must_use]
    pub fn reduction_pct(&self) -> f64 {
        if self.baseline_bytes == 0 {
            return 0.0;
        }
        (self.bytes_saved as f64 / self.baseline_bytes as f64) * 100.0
    }

    /// Serialize to JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let classes: Vec<String> = self
            .exploited_classes
            .iter()
            .map(|c| format!("\"{}\"", c.label()))
            .collect();
        let violations: Vec<String> = self
            .violations
            .iter()
            .map(|v| format!("\"{}\"", v.label()))
            .collect();
        format!(
            r#"{{
  "equivalent": {},
  "baseline_bytes": {},
  "optimized_bytes": {},
  "bytes_saved": {},
  "reduction_pct": {:.2},
  "exploited_classes": [{}],
  "violations": [{}]
}}"#,
            self.equivalent,
            self.baseline_bytes,
            self.optimized_bytes,
            self.bytes_saved,
            self.reduction_pct(),
            classes.join(", "),
            violations.join(", "),
        )
    }
}

// ============================================================================
// Conservative Terminal Mode Rules
// ============================================================================

/// Terminal modes that require conservative presenter behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConservativeMode {
    /// Dumb terminal: no SGR optimization, explicit resets everywhere.
    DumbTerminal,
    /// Inside tmux/screen: extra reset sequences needed for passthrough.
    Multiplexer,
    /// Sixel or image mode: cursor state unpredictable after image output.
    ImageMode,
    /// Bracketed paste mode active: careful about escape sequences.
    BracketedPaste,
}

impl ConservativeMode {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::DumbTerminal => "dumb-terminal",
            Self::Multiplexer => "multiplexer",
            Self::ImageMode => "image-mode",
            Self::BracketedPaste => "bracketed-paste",
        }
    }

    /// Which equivalence classes are NOT safe in this mode.
    #[must_use]
    pub const fn restricted_classes(&self) -> &'static [EquivalenceClass] {
        match self {
            Self::DumbTerminal => &[
                EquivalenceClass::CursorPath,
                EquivalenceClass::RedundantState,
            ],
            Self::Multiplexer => &[EquivalenceClass::RedundantState],
            Self::ImageMode => &[EquivalenceClass::CursorPath],
            Self::BracketedPaste => &[],
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_equivalence_classes_safe() {
        for class in EquivalenceClass::ALL {
            assert!(
                class.safe_to_optimize(),
                "{} should be safe to optimize",
                class.label()
            );
        }
    }

    #[test]
    fn equivalence_class_labels() {
        for class in EquivalenceClass::ALL {
            assert!(!class.label().is_empty());
            assert!(!class.typical_savings().label().is_empty());
        }
        assert_eq!(EquivalenceClass::ALL.len(), 6);
    }

    #[test]
    fn redundant_state_has_largest_savings() {
        assert_eq!(
            EquivalenceClass::RedundantState.typical_savings(),
            TypicalSavings::Large
        );
    }

    #[test]
    fn non_equivalent_variations_all_labeled() {
        for var in NonEquivalentVariation::ALL {
            assert!(!var.label().is_empty());
            assert!(!var.severity().label().is_empty());
        }
        assert_eq!(NonEquivalentVariation::ALL.len(), 7);
    }

    #[test]
    fn critical_violations_identified() {
        assert_eq!(
            NonEquivalentVariation::WrongCursorPosition.severity(),
            ViolationSeverity::Critical
        );
        assert_eq!(
            NonEquivalentVariation::ContentMismatch.severity(),
            ViolationSeverity::Critical
        );
    }

    #[test]
    fn severity_ordering() {
        assert!(ViolationSeverity::Minor < ViolationSeverity::Major);
        assert!(ViolationSeverity::Major < ViolationSeverity::Critical);
    }

    #[test]
    fn tracked_state_suppression() {
        for state in [
            TrackedState::Foreground,
            TrackedState::Background,
            TrackedState::Bold,
            TrackedState::CursorPosition,
        ] {
            assert!(state.suppressible());
            assert!(state.transition_cost_bytes() > 0);
        }
    }

    #[test]
    fn color_transitions_most_expensive() {
        assert!(
            TrackedState::Foreground.transition_cost_bytes()
                > TrackedState::Bold.transition_cost_bytes()
        );
    }

    #[test]
    fn transcript_comparison_reduction() {
        let comp = TranscriptComparison {
            equivalent: true,
            baseline_bytes: 1000,
            optimized_bytes: 700,
            bytes_saved: 300,
            exploited_classes: vec![EquivalenceClass::RedundantState],
            violations: vec![],
        };
        assert!((comp.reduction_pct() - 30.0).abs() < 0.01);
    }

    #[test]
    fn transcript_comparison_zero_baseline() {
        let comp = TranscriptComparison {
            equivalent: true,
            baseline_bytes: 0,
            optimized_bytes: 0,
            bytes_saved: 0,
            exploited_classes: vec![],
            violations: vec![],
        };
        assert!((comp.reduction_pct()).abs() < 0.01);
    }

    #[test]
    fn transcript_comparison_with_violations() {
        let comp = TranscriptComparison {
            equivalent: false,
            baseline_bytes: 1000,
            optimized_bytes: 800,
            bytes_saved: 200,
            exploited_classes: vec![],
            violations: vec![NonEquivalentVariation::DroppedAttribute],
        };
        assert!(!comp.equivalent);
        assert_eq!(comp.violations.len(), 1);
    }

    #[test]
    fn transcript_comparison_to_json_valid() {
        let comp = TranscriptComparison {
            equivalent: true,
            baseline_bytes: 500,
            optimized_bytes: 350,
            bytes_saved: 150,
            exploited_classes: vec![
                EquivalenceClass::RedundantState,
                EquivalenceClass::CursorPath,
            ],
            violations: vec![],
        };
        let json = comp.to_json();
        assert!(json.contains("\"equivalent\": true"));
        assert!(json.contains("\"bytes_saved\": 150"));
        assert!(json.contains("\"redundant-state\""));
        assert!(json.contains("\"reduction_pct\":"));
    }

    #[test]
    fn conservative_modes_restrict_classes() {
        let dumb = ConservativeMode::DumbTerminal;
        assert!(!dumb.restricted_classes().is_empty());
        assert!(
            dumb.restricted_classes()
                .contains(&EquivalenceClass::CursorPath)
        );

        let mux = ConservativeMode::Multiplexer;
        assert!(
            mux.restricted_classes()
                .contains(&EquivalenceClass::RedundantState)
        );

        let paste = ConservativeMode::BracketedPaste;
        assert!(paste.restricted_classes().is_empty());
    }

    #[test]
    fn suppression_reasons_labeled() {
        for reason in [
            SuppressionReason::Redundant,
            SuppressionReason::Changed,
            SuppressionReason::Initial,
            SuppressionReason::ConservativeMode,
        ] {
            assert!(!reason.label().is_empty());
        }
    }

    #[test]
    fn transcript_effects_labeled() {
        for effect in [
            TranscriptEffect::CursorMove,
            TranscriptEffect::StyleSet,
            TranscriptEffect::StyleReset,
            TranscriptEffect::Content,
            TranscriptEffect::Control,
        ] {
            assert!(!effect.label().is_empty());
        }
    }

    #[test]
    fn typical_savings_ordered() {
        assert!(TypicalSavings::Negligible < TypicalSavings::Small);
        assert!(TypicalSavings::Small < TypicalSavings::Medium);
        assert!(TypicalSavings::Medium < TypicalSavings::Large);
    }
}
