//! Galaxy-brain transparency layer with progressive disclosure (bd-xox.5).
//!
//! Provides 4 levels of decision transparency:
//!
//! - **Level 0 (Traffic Light)**: Green/yellow/red indicator for confidence.
//! - **Level 1 (Plain English)**: One-sentence human-readable explanation.
//! - **Level 2 (Evidence Terms)**: Posterior probabilities and confidence intervals.
//! - **Level 3 (Full Bayesian)**: Complete factor breakdown with prior/posterior comparison.
//!
//! Each level includes all information from lower levels.

use crate::decision_core::{Action, Decision};
use crate::unified_evidence::DecisionDomain;
use std::fmt;

/// Progressive disclosure level for decision transparency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum DisclosureLevel {
    /// Traffic light indicator only.
    TrafficLight = 0,
    /// Plain English one-sentence explanation.
    PlainEnglish = 1,
    /// Evidence terms with probabilities and confidence intervals.
    EvidenceTerms = 2,
    /// Full Bayesian factor breakdown.
    FullBayesian = 3,
}

impl DisclosureLevel {
    /// Cycle to the next level, wrapping around.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::TrafficLight => Self::PlainEnglish,
            Self::PlainEnglish => Self::EvidenceTerms,
            Self::EvidenceTerms => Self::FullBayesian,
            Self::FullBayesian => Self::TrafficLight,
        }
    }
}

/// Traffic light signal for quick confidence assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficLight {
    /// High confidence in the chosen action.
    Green,
    /// Moderate confidence — decision is reasonable but uncertain.
    Yellow,
    /// Low confidence — near-fallback territory.
    Red,
}

impl TrafficLight {
    /// Determine signal from log-posterior odds and confidence interval width.
    #[must_use]
    pub fn from_decision<A: Action>(decision: &Decision<A>) -> Self {
        let ci_width = decision.confidence_interval.1 - decision.confidence_interval.0;
        let loss_margin = decision.loss_avoided();

        if decision.log_posterior > 1.0 && ci_width < 0.3 && loss_margin > 0.1 {
            Self::Green
        } else if decision.log_posterior > 0.0 && ci_width < 0.6 {
            Self::Yellow
        } else {
            Self::Red
        }
    }

    /// Emoji-free label for terminal display.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Green => "OK",
            Self::Yellow => "WARN",
            Self::Red => "ALERT",
        }
    }
}

impl fmt::Display for TrafficLight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A disclosure snapshot at a specific level for a single decision.
#[derive(Debug, Clone)]
pub struct Disclosure {
    /// The domain this decision belongs to.
    pub domain: DecisionDomain,
    /// The disclosure level.
    pub level: DisclosureLevel,
    /// Traffic light signal (always available).
    pub signal: TrafficLight,
    /// Action label chosen.
    pub action_label: String,
    /// Plain English explanation (level >= 1).
    pub explanation: Option<String>,
    /// Evidence terms with Bayes factors (level >= 2).
    pub evidence_terms: Option<Vec<DisclosureEvidence>>,
    /// Full Bayesian details (level >= 3).
    pub bayesian_details: Option<BayesianDetails>,
}

/// An evidence term exposed at disclosure level 2+.
#[derive(Debug, Clone)]
pub struct DisclosureEvidence {
    /// Human-readable label for this evidence factor.
    pub label: &'static str,
    /// Bayes factor (likelihood ratio) for this evidence.
    pub bayes_factor: f64,
    /// Direction: positive means supporting the chosen action.
    pub direction: EvidenceDirection,
}

/// Whether evidence supports or opposes the chosen action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceDirection {
    /// Evidence supports the chosen action.
    Supporting,
    /// Evidence opposes the chosen action.
    Opposing,
    /// Evidence is neutral (Bayes factor near 1.0).
    Neutral,
}

impl EvidenceDirection {
    /// Classify from a Bayes factor.
    #[must_use]
    pub fn from_bayes_factor(bf: f64) -> Self {
        if bf > 1.1 {
            Self::Supporting
        } else if bf < 0.9 {
            Self::Opposing
        } else {
            Self::Neutral
        }
    }
}

/// Full Bayesian details exposed at disclosure level 3.
#[derive(Debug, Clone)]
pub struct BayesianDetails {
    /// Log-posterior odds.
    pub log_posterior: f64,
    /// Confidence interval (lower, upper) on posterior probability.
    pub confidence_interval: (f64, f64),
    /// Expected loss of chosen action.
    pub expected_loss: f64,
    /// Expected loss of next-best action.
    pub next_best_loss: f64,
    /// Loss avoided by the chosen action.
    pub loss_avoided: f64,
}

/// Build a disclosure snapshot from a decision at the requested level.
pub fn disclose<A: Action>(
    decision: &Decision<A>,
    domain: DecisionDomain,
    level: DisclosureLevel,
) -> Disclosure {
    let signal = TrafficLight::from_decision(decision);
    let action_label = decision.action.label().to_string();

    let explanation = if level >= DisclosureLevel::PlainEnglish {
        Some(build_explanation(decision, domain, signal))
    } else {
        None
    };

    let evidence_terms = if level >= DisclosureLevel::EvidenceTerms {
        Some(
            decision
                .evidence
                .iter()
                .map(|t| DisclosureEvidence {
                    label: t.label,
                    bayes_factor: t.bayes_factor,
                    direction: EvidenceDirection::from_bayes_factor(t.bayes_factor),
                })
                .collect(),
        )
    } else {
        None
    };

    let bayesian_details = if level >= DisclosureLevel::FullBayesian {
        Some(BayesianDetails {
            log_posterior: decision.log_posterior,
            confidence_interval: decision.confidence_interval,
            expected_loss: decision.expected_loss,
            next_best_loss: decision.next_best_loss,
            loss_avoided: decision.loss_avoided(),
        })
    } else {
        None
    };

    Disclosure {
        domain,
        level,
        signal,
        action_label,
        explanation,
        evidence_terms,
        bayesian_details,
    }
}

/// Build a plain-English explanation for a decision.
fn build_explanation<A: Action>(
    decision: &Decision<A>,
    domain: DecisionDomain,
    signal: TrafficLight,
) -> String {
    let domain_name = domain_display_name(domain);
    let action = decision.action.label();
    let confidence = match signal {
        TrafficLight::Green => "high confidence",
        TrafficLight::Yellow => "moderate confidence",
        TrafficLight::Red => "low confidence",
    };

    let loss_info = if decision.loss_avoided() > 0.01 {
        format!(
            ", saving {:.1}% over the alternative",
            decision.loss_avoided() * 100.0
        )
    } else {
        String::new()
    };

    format!("{domain_name}: chose '{action}' with {confidence}{loss_info}.")
}

/// Human-readable domain name for explanations.
fn domain_display_name(domain: DecisionDomain) -> &'static str {
    match domain {
        DecisionDomain::DiffStrategy => "Diff strategy",
        DecisionDomain::ResizeCoalescing => "Resize coalescing",
        DecisionDomain::FrameBudget => "Frame budget",
        DecisionDomain::Degradation => "Degradation",
        DecisionDomain::VoiSampling => "VOI sampling",
        DecisionDomain::HintRanking => "Hint ranking",
        DecisionDomain::PaletteScoring => "Palette scoring",
    }
}

/// Format a disclosure for terminal display at any level.
impl fmt::Display for Disclosure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Level 0: traffic light
        write!(f, "[{}] {}", self.signal, self.action_label)?;

        // Level 1: explanation
        if let Some(ref explanation) = self.explanation {
            write!(f, "\n  {explanation}")?;
        }

        // Level 2: evidence terms
        if let Some(ref terms) = self.evidence_terms
            && !terms.is_empty()
        {
            write!(f, "\n  Evidence:")?;
            for t in terms {
                let dir = match t.direction {
                    EvidenceDirection::Supporting => "+",
                    EvidenceDirection::Opposing => "-",
                    EvidenceDirection::Neutral => "~",
                };
                write!(f, "\n    {dir} {}: BF={:.2}", t.label, t.bayes_factor)?;
            }
        }

        // Level 3: full Bayesian
        if let Some(ref details) = self.bayesian_details {
            write!(
                f,
                "\n  Bayesian: log_post={:.3} CI=[{:.3}, {:.3}] E[loss]={:.4} avoided={:.4}",
                details.log_posterior,
                details.confidence_interval.0,
                details.confidence_interval.1,
                details.expected_loss,
                details.loss_avoided,
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unified_evidence::EvidenceTerm;

    // Minimal action type for testing.
    #[derive(Debug, Clone)]
    struct TestAction(&'static str);
    impl Action for TestAction {
        fn label(&self) -> &'static str {
            self.0
        }
    }

    fn sample_decision(
        log_posterior: f64,
        ci: (f64, f64),
        expected_loss: f64,
        next_best_loss: f64,
    ) -> Decision<TestAction> {
        Decision {
            action: TestAction("full_redraw"),
            expected_loss,
            next_best_loss,
            log_posterior,
            confidence_interval: ci,
            evidence: vec![
                EvidenceTerm {
                    label: "change_rate",
                    bayes_factor: 3.5,
                },
                EvidenceTerm {
                    label: "frame_cost",
                    bayes_factor: 0.8,
                },
                EvidenceTerm {
                    label: "stability",
                    bayes_factor: 1.0,
                },
            ],
        }
    }

    #[test]
    fn traffic_light_green() {
        let d = sample_decision(2.0, (0.7, 0.95), 0.1, 0.5);
        assert_eq!(TrafficLight::from_decision(&d), TrafficLight::Green);
    }

    #[test]
    fn traffic_light_yellow() {
        let d = sample_decision(0.5, (0.3, 0.7), 0.3, 0.35);
        assert_eq!(TrafficLight::from_decision(&d), TrafficLight::Yellow);
    }

    #[test]
    fn traffic_light_red() {
        let d = sample_decision(-0.5, (0.1, 0.9), 0.4, 0.42);
        assert_eq!(TrafficLight::from_decision(&d), TrafficLight::Red);
    }

    #[test]
    fn disclosure_level_0() {
        let d = sample_decision(2.0, (0.7, 0.95), 0.1, 0.5);
        let disc = disclose(
            &d,
            DecisionDomain::DiffStrategy,
            DisclosureLevel::TrafficLight,
        );
        assert_eq!(disc.signal, TrafficLight::Green);
        assert!(disc.explanation.is_none());
        assert!(disc.evidence_terms.is_none());
        assert!(disc.bayesian_details.is_none());
    }

    #[test]
    fn disclosure_level_1() {
        let d = sample_decision(2.0, (0.7, 0.95), 0.1, 0.5);
        let disc = disclose(
            &d,
            DecisionDomain::DiffStrategy,
            DisclosureLevel::PlainEnglish,
        );
        assert!(disc.explanation.is_some());
        let expl = disc.explanation.unwrap();
        assert!(expl.contains("Diff strategy"));
        assert!(expl.contains("full_redraw"));
        assert!(expl.contains("high confidence"));
    }

    #[test]
    fn disclosure_level_2() {
        let d = sample_decision(2.0, (0.7, 0.95), 0.1, 0.5);
        let disc = disclose(
            &d,
            DecisionDomain::DiffStrategy,
            DisclosureLevel::EvidenceTerms,
        );
        let terms = disc.evidence_terms.unwrap();
        assert_eq!(terms.len(), 3);
        assert_eq!(terms[0].label, "change_rate");
        assert_eq!(terms[0].direction, EvidenceDirection::Supporting);
        assert_eq!(terms[1].direction, EvidenceDirection::Opposing);
        assert_eq!(terms[2].direction, EvidenceDirection::Neutral);
    }

    #[test]
    fn disclosure_level_3() {
        let d = sample_decision(2.0, (0.7, 0.95), 0.1, 0.5);
        let disc = disclose(
            &d,
            DecisionDomain::DiffStrategy,
            DisclosureLevel::FullBayesian,
        );
        let details = disc.bayesian_details.unwrap();
        assert!((details.log_posterior - 2.0).abs() < 1e-10);
        assert!((details.expected_loss - 0.1).abs() < 1e-10);
        assert!((details.loss_avoided - 0.4).abs() < 1e-10);
    }

    #[test]
    fn disclosure_level_ordering() {
        assert!(DisclosureLevel::TrafficLight < DisclosureLevel::PlainEnglish);
        assert!(DisclosureLevel::PlainEnglish < DisclosureLevel::EvidenceTerms);
        assert!(DisclosureLevel::EvidenceTerms < DisclosureLevel::FullBayesian);
    }

    #[test]
    fn disclosure_level_cycle() {
        let mut l = DisclosureLevel::TrafficLight;
        l = l.next();
        assert_eq!(l, DisclosureLevel::PlainEnglish);
        l = l.next();
        assert_eq!(l, DisclosureLevel::EvidenceTerms);
        l = l.next();
        assert_eq!(l, DisclosureLevel::FullBayesian);
        l = l.next();
        assert_eq!(l, DisclosureLevel::TrafficLight);
    }

    #[test]
    fn display_formats_correctly() {
        let d = sample_decision(2.0, (0.7, 0.95), 0.1, 0.5);
        let disc = disclose(
            &d,
            DecisionDomain::DiffStrategy,
            DisclosureLevel::FullBayesian,
        );
        let output = disc.to_string();
        assert!(output.contains("[OK]"));
        assert!(output.contains("full_redraw"));
        assert!(output.contains("Evidence:"));
        assert!(output.contains("Bayesian:"));
    }

    #[test]
    fn loss_avoided_in_explanation() {
        let d = sample_decision(2.0, (0.7, 0.95), 0.1, 0.5);
        let disc = disclose(
            &d,
            DecisionDomain::DiffStrategy,
            DisclosureLevel::PlainEnglish,
        );
        let expl = disc.explanation.unwrap();
        assert!(expl.contains("saving"), "should mention savings: {expl}");
    }

    #[test]
    fn no_savings_when_margin_tiny() {
        let d = sample_decision(2.0, (0.7, 0.95), 0.1, 0.105);
        let disc = disclose(
            &d,
            DecisionDomain::DiffStrategy,
            DisclosureLevel::PlainEnglish,
        );
        let expl = disc.explanation.unwrap();
        assert!(
            !expl.contains("saving"),
            "should not mention savings when margin < 1%: {expl}"
        );
    }
}
