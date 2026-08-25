#![forbid(unsafe_code)]

//! Render-skip certificate design and correctness model (bd-2dlqr).
//!
//! Defines when render, diff, or present work can be safely skipped or
//! narrowed. This module is the shared correctness story that implementation
//! beads (bd-i71od, bd-6b9nr) must follow.
//!
//! # Certificate model
//!
//! A **certificate** is a proof that a render stage can be skipped without
//! changing visible output. Certificates have:
//!
//! - **Inputs**: what state they observe (dirty rows, cell content, layout, style).
//! - **Outputs**: what guarantee they provide (exact match, bounded deviation).
//! - **Invalidation causes**: what events revoke the certificate.
//! - **Fallback**: what happens when the certificate cannot be issued.
//!
//! # Safety invariant
//!
//! **A certificate must never suppress work that would produce visibly
//! different terminal output.** If in doubt, the certificate MUST fall back
//! to full work. This is the non-negotiable correctness constraint.
//!
//! # Certificate levels
//!
//! | Level | What it skips | Safety requirement |
//! |-------|--------------|-------------------|
//! | `FrameSkip` | Entire frame (view+diff+present) | No model state changed since last frame |
//! | `DiffSkip` | Buffer diff computation | Old and new buffers are identical |
//! | `RegionSkip` | Diff for a rectangular region | Region cells unchanged since last diff |
//! | `PresentNarrow` | Present outside dirty region | Only dirty cells need ANSI emission |
//! | `WidgetSkip` | Individual widget re-render | Widget inputs unchanged since last render |
//!
//! # Usage
//!
//! ```ignore
//! use ftui_harness::render_certificate::*;
//!
//! // Check if a frame can be skipped
//! let inputs = CertificateInputs {
//!     dirty_row_count: 0,
//!     dirty_cell_count: 0,
//!     model_generation: 5,
//!     last_certified_generation: 5,
//!     viewport_changed: false,
//!     style_epoch: 3,
//!     last_certified_style_epoch: 3,
//!     layout_displacement: 0.0,
//!     degradation_changed: false,
//! };
//!
//! let cert = CertificateEvaluator::evaluate(&inputs);
//! assert_eq!(cert.level, CertificateLevel::FrameSkip);
//! assert!(cert.is_safe());
//! ```

// ============================================================================
// Certificate Levels
// ============================================================================

/// The level of render work that a certificate allows skipping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CertificateLevel {
    /// No skip possible — full render required.
    None,
    /// Individual widget render can be skipped.
    WidgetSkip,
    /// Present can be narrowed to dirty region only.
    PresentNarrow,
    /// Diff for a rectangular region can be skipped.
    RegionSkip,
    /// Buffer diff computation can be skipped entirely.
    DiffSkip,
    /// Entire frame can be skipped (most aggressive).
    FrameSkip,
}

impl CertificateLevel {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::WidgetSkip => "widget-skip",
            Self::PresentNarrow => "present-narrow",
            Self::RegionSkip => "region-skip",
            Self::DiffSkip => "diff-skip",
            Self::FrameSkip => "frame-skip",
        }
    }

    /// How much work is saved (rough multiplier).
    #[must_use]
    pub const fn savings_estimate(&self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::WidgetSkip => 0.1,    // saves one widget's render
            Self::PresentNarrow => 0.3, // skip clean-region ANSI emission
            Self::RegionSkip => 0.4,    // skip region diff
            Self::DiffSkip => 0.5,      // skip entire diff pass
            Self::FrameSkip => 1.0,     // skip everything
        }
    }

    /// Whether this level is conservative (safe for correctness-critical paths).
    #[must_use]
    pub const fn is_conservative(&self) -> bool {
        matches!(self, Self::None | Self::PresentNarrow)
    }
}

// ============================================================================
// Invalidation Causes
// ============================================================================

/// Events that invalidate a render-skip certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvalidationCause {
    /// Model state changed (update() produced new state).
    ModelStateChange,
    /// Viewport dimensions changed (resize).
    ViewportResize,
    /// Theme or style epoch changed.
    StyleChange,
    /// Terminal capabilities changed (redetection).
    CapabilityChange,
    /// Layout displacement exceeds threshold.
    LayoutThrash,
    /// Degradation level changed.
    DegradationChange,
    /// Focus state changed (cursor position, focus ring).
    FocusChange,
    /// Subscription delivered new data.
    SubscriptionData,
    /// Timer tick required UI update.
    TimerTick,
    /// Mouse/input state changed hover/press visuals.
    InputStateChange,
    /// Explicit invalidation request from application code.
    ExplicitInvalidation,
}

impl InvalidationCause {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::ModelStateChange => "model-state-change",
            Self::ViewportResize => "viewport-resize",
            Self::StyleChange => "style-change",
            Self::CapabilityChange => "capability-change",
            Self::LayoutThrash => "layout-thrash",
            Self::DegradationChange => "degradation-change",
            Self::FocusChange => "focus-change",
            Self::SubscriptionData => "subscription-data",
            Self::TimerTick => "timer-tick",
            Self::InputStateChange => "input-state-change",
            Self::ExplicitInvalidation => "explicit-invalidation",
        }
    }

    /// Whether this cause always forces full render (no partial skip possible).
    #[must_use]
    pub const fn forces_full_render(&self) -> bool {
        matches!(
            self,
            Self::ViewportResize | Self::CapabilityChange | Self::DegradationChange
        )
    }

    /// Whether this cause can be handled by partial region invalidation.
    #[must_use]
    pub const fn allows_partial_skip(&self) -> bool {
        matches!(
            self,
            Self::ModelStateChange
                | Self::FocusChange
                | Self::InputStateChange
                | Self::TimerTick
                | Self::SubscriptionData
        )
    }

    pub const ALL: &'static [InvalidationCause] = &[
        Self::ModelStateChange,
        Self::ViewportResize,
        Self::StyleChange,
        Self::CapabilityChange,
        Self::LayoutThrash,
        Self::DegradationChange,
        Self::FocusChange,
        Self::SubscriptionData,
        Self::TimerTick,
        Self::InputStateChange,
        Self::ExplicitInvalidation,
    ];
}

// ============================================================================
// Certificate Inputs
// ============================================================================

/// Observable state used to evaluate whether a certificate can be issued.
///
/// These inputs are derived from the existing Buffer dirty tracking,
/// layout CoherenceCache, and runtime model generation counter.
#[derive(Debug, Clone)]
pub struct CertificateInputs {
    /// Number of dirty rows in the buffer (from `Buffer::dirty_rows`).
    pub dirty_row_count: u32,
    /// Number of dirty cells (from `Buffer::dirty_cells`).
    pub dirty_cell_count: u32,
    /// Current model generation (incremented on every `update()` call).
    pub model_generation: u64,
    /// Generation when the last certificate was issued.
    pub last_certified_generation: u64,
    /// Whether the viewport dimensions changed since last frame.
    pub viewport_changed: bool,
    /// Current style/theme epoch.
    pub style_epoch: u64,
    /// Style epoch when last certificate was issued.
    pub last_certified_style_epoch: u64,
    /// Layout displacement magnitude from CoherenceCache (0.0 = stable).
    pub layout_displacement: f64,
    /// Whether degradation level changed since last frame.
    pub degradation_changed: bool,
}

impl CertificateInputs {
    /// Whether any state has changed since the last certified frame.
    #[must_use]
    pub fn has_any_change(&self) -> bool {
        self.dirty_row_count > 0
            || self.dirty_cell_count > 0
            || self.model_generation != self.last_certified_generation
            || self.viewport_changed
            || self.style_epoch != self.last_certified_style_epoch
            || self.layout_displacement > 0.0
            || self.degradation_changed
    }

    /// Identify which invalidation causes apply.
    #[must_use]
    pub fn active_causes(&self) -> Vec<InvalidationCause> {
        let mut causes = Vec::new();
        if self.model_generation != self.last_certified_generation {
            causes.push(InvalidationCause::ModelStateChange);
        }
        if self.viewport_changed {
            causes.push(InvalidationCause::ViewportResize);
        }
        if self.style_epoch != self.last_certified_style_epoch {
            causes.push(InvalidationCause::StyleChange);
        }
        if self.layout_displacement > LAYOUT_THRASH_THRESHOLD {
            causes.push(InvalidationCause::LayoutThrash);
        }
        if self.degradation_changed {
            causes.push(InvalidationCause::DegradationChange);
        }
        causes
    }
}

/// Layout displacement above this value triggers full invalidation.
pub const LAYOUT_THRASH_THRESHOLD: f64 = 5.0;

/// Dirty cell fraction above which region-skip is not worthwhile.
pub const REGION_SKIP_DENSITY_LIMIT: f64 = 0.25;

// ============================================================================
// Certificate
// ============================================================================

/// A render-skip certificate: the result of evaluating inputs.
#[derive(Debug, Clone)]
pub struct Certificate {
    /// What level of work can be safely skipped.
    pub level: CertificateLevel,
    /// Confidence in the certificate (1.0 = certain, lower = more risk).
    pub confidence: f64,
    /// Active invalidation causes (empty for FrameSkip).
    pub causes: Vec<InvalidationCause>,
    /// Whether a conservative fallback was applied.
    pub fell_back: bool,
    /// Human-readable explanation.
    pub reason: String,
}

impl Certificate {
    /// Whether the certificate allows any skip at all.
    #[must_use]
    pub fn is_safe(&self) -> bool {
        self.level != CertificateLevel::None
    }

    /// Serialize to JSON for evidence logging.
    #[must_use]
    pub fn to_json(&self) -> String {
        let causes: Vec<String> = self
            .causes
            .iter()
            .map(|c| format!("\"{}\"", c.label()))
            .collect();
        format!(
            r#"{{
  "level": "{}",
  "confidence": {:.3},
  "causes": [{}],
  "fell_back": {},
  "reason": "{}"
}}"#,
            self.level.label(),
            self.confidence,
            causes.join(", "),
            self.fell_back,
            self.reason.replace('"', "\\\""),
        )
    }
}

// ============================================================================
// Certificate Evaluator
// ============================================================================

/// Evaluates certificate inputs and issues the highest safe certificate level.
///
/// # Decision tree
///
/// ```text
/// viewport_changed OR degradation_changed?
///   YES → None (full render required)
///
/// model_generation == last_certified AND no dirty cells?
///   YES → FrameSkip
///
/// style_epoch changed?
///   YES → None (style changes are global)
///
/// layout_displacement > threshold?
///   YES → None (layout thrash → conservative full render)
///
/// dirty_cell_count == 0?
///   YES → DiffSkip (buffer unchanged, but model state changed)
///
/// dirty fraction < region_skip_limit?
///   YES → RegionSkip or PresentNarrow
///
/// Otherwise → None
/// ```
pub struct CertificateEvaluator;

impl CertificateEvaluator {
    /// Evaluate inputs and return the highest safe certificate.
    #[must_use]
    pub fn evaluate(inputs: &CertificateInputs) -> Certificate {
        let causes = inputs.active_causes();

        // Force-full-render causes: viewport resize, degradation change
        if inputs.viewport_changed {
            return Certificate {
                level: CertificateLevel::None,
                confidence: 1.0,
                causes,
                fell_back: false,
                reason: "Viewport changed — full render required".to_string(),
            };
        }

        if inputs.degradation_changed {
            return Certificate {
                level: CertificateLevel::None,
                confidence: 1.0,
                causes,
                fell_back: false,
                reason: "Degradation level changed — full render required".to_string(),
            };
        }

        // FrameSkip: no state change at all
        if !inputs.has_any_change() {
            return Certificate {
                level: CertificateLevel::FrameSkip,
                confidence: 1.0,
                causes: Vec::new(),
                fell_back: false,
                reason: "No state change since last certified frame".to_string(),
            };
        }

        // Style change: global invalidation (cannot safely skip anything)
        if inputs.style_epoch != inputs.last_certified_style_epoch {
            return Certificate {
                level: CertificateLevel::None,
                confidence: 1.0,
                causes,
                fell_back: false,
                reason: "Style epoch changed — global re-render required".to_string(),
            };
        }

        // Layout thrash: conservative fallback
        if inputs.layout_displacement > LAYOUT_THRASH_THRESHOLD {
            return Certificate {
                level: CertificateLevel::None,
                confidence: 0.8,
                causes,
                fell_back: true,
                reason: format!(
                    "Layout displacement {:.1} exceeds threshold {:.1} — conservative fallback",
                    inputs.layout_displacement, LAYOUT_THRASH_THRESHOLD
                ),
            };
        }

        // DiffSkip: model state changed but no buffer cells are dirty
        // This happens when update() changes internal state but view()
        // produces identical output (common for timers, background work).
        if inputs.dirty_cell_count == 0 && inputs.dirty_row_count == 0 {
            return Certificate {
                level: CertificateLevel::DiffSkip,
                confidence: 0.95,
                causes,
                fell_back: false,
                reason: "Model changed but no buffer cells dirty — diff skip safe".to_string(),
            };
        }

        // PresentNarrow: few dirty cells, can narrow ANSI emission
        // We need total_cells to compute density, but we don't have it.
        // Use dirty_row_count as a proxy: few dirty rows = narrow present.
        if inputs.dirty_row_count <= 3 {
            return Certificate {
                level: CertificateLevel::PresentNarrow,
                confidence: 0.9,
                causes,
                fell_back: false,
                reason: format!(
                    "Only {} dirty row(s) — narrow present to dirty region",
                    inputs.dirty_row_count
                ),
            };
        }

        // Too many changes for safe partial skip
        Certificate {
            level: CertificateLevel::None,
            confidence: 1.0,
            causes,
            fell_back: false,
            reason: format!(
                "{} dirty rows, {} dirty cells — full render required",
                inputs.dirty_row_count, inputs.dirty_cell_count
            ),
        }
    }

    /// Conservative evaluator: only issues certificates when 100% safe.
    /// Used during migration and shadow-run comparison.
    #[must_use]
    pub fn evaluate_conservative(inputs: &CertificateInputs) -> Certificate {
        if !inputs.has_any_change() {
            Certificate {
                level: CertificateLevel::FrameSkip,
                confidence: 1.0,
                causes: Vec::new(),
                fell_back: false,
                reason: "Conservative: no change detected".to_string(),
            }
        } else {
            Certificate {
                level: CertificateLevel::None,
                confidence: 1.0,
                causes: inputs.active_causes(),
                fell_back: true,
                reason: "Conservative: any change forces full render".to_string(),
            }
        }
    }
}

// ============================================================================
// Proof Obligations
// ============================================================================

/// Proof obligations that must be satisfied before a certificate level
/// can be trusted in production.
#[derive(Debug, Clone)]
pub struct ProofObligation {
    /// Certificate level this obligation applies to.
    pub level: CertificateLevel,
    /// What must be proven.
    pub description: String,
    /// How to test it.
    pub test_method: String,
    /// What evidence to log on failure.
    pub failure_evidence: String,
}

/// Returns the proof obligations for each certificate level.
#[must_use]
pub fn proof_obligations() -> Vec<ProofObligation> {
    vec![
        ProofObligation {
            level: CertificateLevel::FrameSkip,
            description: "Skipped frame must produce byte-identical ANSI output to a fully-rendered frame".to_string(),
            test_method: "Shadow-run: render both paths and compare presenter output checksums".to_string(),
            failure_evidence: "Buffer hex dump, ANSI diff, frame index, model state snapshot".to_string(),
        },
        ProofObligation {
            level: CertificateLevel::DiffSkip,
            description: "Skipped diff must not miss any changed cells in the new buffer".to_string(),
            test_method: "Compare skipped-diff output against full compute_into() output".to_string(),
            failure_evidence: "Missed cell positions, old/new cell content at each miss".to_string(),
        },
        ProofObligation {
            level: CertificateLevel::RegionSkip,
            description: "Skipped region must contain zero changed cells".to_string(),
            test_method: "Full cell-by-cell comparison of skipped region after render".to_string(),
            failure_evidence: "Region bounds, changed cell positions within region".to_string(),
        },
        ProofObligation {
            level: CertificateLevel::PresentNarrow,
            description: "Narrowed present must emit identical ANSI for dirty region AND not miss any dirty cells outside the narrowed region".to_string(),
            test_method: "Compare narrowed present output against full present output".to_string(),
            failure_evidence: "ANSI byte diff, dirty region bounds, missed dirty cells outside region".to_string(),
        },
        ProofObligation {
            level: CertificateLevel::WidgetSkip,
            description: "Skipped widget must produce identical buffer cells as a fully-rendered widget".to_string(),
            test_method: "Render widget, compare cell checksums against cached version".to_string(),
            failure_evidence: "Widget area, changed cell positions, input state diff".to_string(),
        },
    ]
}

// ============================================================================
// Conservative Fallback Rules
// ============================================================================

/// Conditions under which certificates must fall back to full render,
/// even if the evaluator would otherwise issue a skip.
#[derive(Debug, Clone)]
pub struct FallbackRule {
    /// Rule identifier.
    pub id: String,
    /// When this rule triggers.
    pub condition: String,
    /// Why the fallback is necessary.
    pub rationale: String,
}

/// Returns the conservative fallback rules.
#[must_use]
pub fn fallback_rules() -> Vec<FallbackRule> {
    vec![
        FallbackRule {
            id: "fb-resize".to_string(),
            condition: "Viewport dimensions changed".to_string(),
            rationale: "Buffer dimensions are immutable after creation; resize requires a new buffer pair and full re-render. No certificate can bridge a resize.".to_string(),
        },
        FallbackRule {
            id: "fb-style-epoch".to_string(),
            condition: "Theme or style epoch changed".to_string(),
            rationale: "Style changes affect all cells globally. Partial certificates cannot determine which cells are visually affected without full re-render.".to_string(),
        },
        FallbackRule {
            id: "fb-capability".to_string(),
            condition: "Terminal capabilities re-detected".to_string(),
            rationale: "Capability changes affect presenter output format (color depth, attribute support). Full re-render ensures ANSI output matches new capabilities.".to_string(),
        },
        FallbackRule {
            id: "fb-degradation".to_string(),
            condition: "Degradation level changed".to_string(),
            rationale: "Degradation affects which widgets are rendered and at what quality. Certificates from a different degradation level are invalid.".to_string(),
        },
        FallbackRule {
            id: "fb-layout-thrash".to_string(),
            condition: "Layout displacement exceeds threshold".to_string(),
            rationale: "High layout displacement indicates the CoherenceCache is not stabilizing. Skip certificates in unstable layouts risk stale frames.".to_string(),
        },
        FallbackRule {
            id: "fb-scissor-stack".to_string(),
            condition: "Scissor stack depth changed since last frame".to_string(),
            rationale: "Scissor changes alter clipping regions, potentially exposing previously hidden cells. Region certificates must be invalidated.".to_string(),
        },
        FallbackRule {
            id: "fb-first-frame".to_string(),
            condition: "No previous frame exists for comparison".to_string(),
            rationale: "The first frame has no baseline to compare against. Full render is mandatory.".to_string(),
        },
    ]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn no_change_inputs() -> CertificateInputs {
        CertificateInputs {
            dirty_row_count: 0,
            dirty_cell_count: 0,
            model_generation: 5,
            last_certified_generation: 5,
            viewport_changed: false,
            style_epoch: 3,
            last_certified_style_epoch: 3,
            layout_displacement: 0.0,
            degradation_changed: false,
        }
    }

    #[test]
    fn no_change_issues_frame_skip() {
        let cert = CertificateEvaluator::evaluate(&no_change_inputs());
        assert_eq!(cert.level, CertificateLevel::FrameSkip);
        assert!(cert.is_safe());
        assert!((cert.confidence - 1.0).abs() < 0.01);
        assert!(cert.causes.is_empty());
    }

    #[test]
    fn viewport_change_forces_full_render() {
        let mut inputs = no_change_inputs();
        inputs.viewport_changed = true;
        let cert = CertificateEvaluator::evaluate(&inputs);
        assert_eq!(cert.level, CertificateLevel::None);
        assert!(!cert.is_safe());
    }

    #[test]
    fn degradation_change_forces_full_render() {
        let mut inputs = no_change_inputs();
        inputs.degradation_changed = true;
        let cert = CertificateEvaluator::evaluate(&inputs);
        assert_eq!(cert.level, CertificateLevel::None);
    }

    #[test]
    fn style_change_forces_full_render() {
        let mut inputs = no_change_inputs();
        inputs.style_epoch = 4; // changed
        let cert = CertificateEvaluator::evaluate(&inputs);
        assert_eq!(cert.level, CertificateLevel::None);
    }

    #[test]
    fn model_change_no_dirty_cells_issues_diff_skip() {
        let mut inputs = no_change_inputs();
        inputs.model_generation = 6; // model changed
        // but no dirty cells
        let cert = CertificateEvaluator::evaluate(&inputs);
        assert_eq!(cert.level, CertificateLevel::DiffSkip);
        assert!(cert.is_safe());
    }

    #[test]
    fn few_dirty_rows_issues_present_narrow() {
        let mut inputs = no_change_inputs();
        inputs.model_generation = 6;
        inputs.dirty_row_count = 2;
        inputs.dirty_cell_count = 10;
        let cert = CertificateEvaluator::evaluate(&inputs);
        assert_eq!(cert.level, CertificateLevel::PresentNarrow);
    }

    #[test]
    fn many_dirty_rows_forces_full_render() {
        let mut inputs = no_change_inputs();
        inputs.model_generation = 6;
        inputs.dirty_row_count = 20;
        inputs.dirty_cell_count = 500;
        let cert = CertificateEvaluator::evaluate(&inputs);
        assert_eq!(cert.level, CertificateLevel::None);
    }

    #[test]
    fn layout_thrash_triggers_fallback() {
        let mut inputs = no_change_inputs();
        inputs.model_generation = 6;
        inputs.layout_displacement = 10.0; // above threshold
        let cert = CertificateEvaluator::evaluate(&inputs);
        assert_eq!(cert.level, CertificateLevel::None);
        assert!(cert.fell_back);
    }

    #[test]
    fn conservative_evaluator_only_frame_skip() {
        // No change → FrameSkip
        let cert = CertificateEvaluator::evaluate_conservative(&no_change_inputs());
        assert_eq!(cert.level, CertificateLevel::FrameSkip);

        // Any change → None
        let mut inputs = no_change_inputs();
        inputs.model_generation = 6;
        let cert = CertificateEvaluator::evaluate_conservative(&inputs);
        assert_eq!(cert.level, CertificateLevel::None);
        assert!(cert.fell_back);
    }

    #[test]
    fn certificate_levels_ordered() {
        assert!(CertificateLevel::None < CertificateLevel::WidgetSkip);
        assert!(CertificateLevel::WidgetSkip < CertificateLevel::PresentNarrow);
        assert!(CertificateLevel::PresentNarrow < CertificateLevel::RegionSkip);
        assert!(CertificateLevel::RegionSkip < CertificateLevel::DiffSkip);
        assert!(CertificateLevel::DiffSkip < CertificateLevel::FrameSkip);
    }

    #[test]
    fn certificate_savings_monotonic() {
        let levels = [
            CertificateLevel::None,
            CertificateLevel::WidgetSkip,
            CertificateLevel::PresentNarrow,
            CertificateLevel::RegionSkip,
            CertificateLevel::DiffSkip,
            CertificateLevel::FrameSkip,
        ];
        for pair in levels.windows(2) {
            assert!(
                pair[0].savings_estimate() <= pair[1].savings_estimate(),
                "{} should save <= {} but got {} > {}",
                pair[0].label(),
                pair[1].label(),
                pair[0].savings_estimate(),
                pair[1].savings_estimate(),
            );
        }
    }

    #[test]
    fn invalidation_cause_labels() {
        for cause in InvalidationCause::ALL {
            assert!(!cause.label().is_empty());
        }
    }

    #[test]
    fn force_full_render_causes() {
        assert!(InvalidationCause::ViewportResize.forces_full_render());
        assert!(InvalidationCause::CapabilityChange.forces_full_render());
        assert!(InvalidationCause::DegradationChange.forces_full_render());
        assert!(!InvalidationCause::ModelStateChange.forces_full_render());
        assert!(!InvalidationCause::TimerTick.forces_full_render());
    }

    #[test]
    fn partial_skip_causes() {
        assert!(InvalidationCause::ModelStateChange.allows_partial_skip());
        assert!(InvalidationCause::FocusChange.allows_partial_skip());
        assert!(!InvalidationCause::ViewportResize.allows_partial_skip());
        assert!(!InvalidationCause::StyleChange.allows_partial_skip());
    }

    #[test]
    fn proof_obligations_cover_all_skip_levels() {
        let obligations = proof_obligations();
        let covered_levels: Vec<CertificateLevel> = obligations.iter().map(|o| o.level).collect();
        // All skip levels (not None) should have proof obligations
        for level in [
            CertificateLevel::FrameSkip,
            CertificateLevel::DiffSkip,
            CertificateLevel::RegionSkip,
            CertificateLevel::PresentNarrow,
            CertificateLevel::WidgetSkip,
        ] {
            assert!(
                covered_levels.contains(&level),
                "missing proof obligation for {}",
                level.label()
            );
        }
    }

    #[test]
    fn fallback_rules_exist() {
        let rules = fallback_rules();
        assert!(rules.len() >= 7, "expected at least 7 fallback rules");
        for rule in &rules {
            assert!(!rule.id.is_empty());
            assert!(!rule.condition.is_empty());
            assert!(!rule.rationale.is_empty());
        }
    }

    #[test]
    fn certificate_to_json_valid() {
        let cert = CertificateEvaluator::evaluate(&no_change_inputs());
        let json = cert.to_json();
        assert!(json.contains("\"level\": \"frame-skip\""));
        assert!(json.contains("\"confidence\":"));
        assert!(json.contains("\"fell_back\": false"));
    }

    #[test]
    fn active_causes_detection() {
        let mut inputs = no_change_inputs();
        inputs.model_generation = 6;
        inputs.layout_displacement = 10.0;
        let causes = inputs.active_causes();
        assert!(causes.contains(&InvalidationCause::ModelStateChange));
        assert!(causes.contains(&InvalidationCause::LayoutThrash));
    }

    #[test]
    fn conservative_levels_identified() {
        assert!(CertificateLevel::None.is_conservative());
        assert!(CertificateLevel::PresentNarrow.is_conservative());
        assert!(!CertificateLevel::FrameSkip.is_conservative());
        assert!(!CertificateLevel::DiffSkip.is_conservative());
    }
}
