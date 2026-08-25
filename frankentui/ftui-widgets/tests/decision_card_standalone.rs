//! bd-1lg.14: E2E test — Galaxy-brain decision card standalone embedding.
//!
//! Proves the decision card widget renders correctly in isolation:
//! 1. All 4 progressive disclosure levels produce correct output.
//! 2. Evidence data can be injected externally (no runtime loop needed).
//! 3. Traffic light signals produce distinct visual output.
//! 4. Card handles degenerate areas gracefully.
//!
//! Run:
//!   cargo test -p ftui-widgets --test decision_card_standalone

use ftui_core::geometry::Rect;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_runtime::transparency::{
    BayesianDetails, Disclosure, DisclosureEvidence, DisclosureLevel, EvidenceDirection,
    TrafficLight,
};
use ftui_runtime::unified_evidence::DecisionDomain;
use ftui_widgets::Widget;
use ftui_widgets::decision_card::DecisionCard;

// ============================================================================
// Helpers
// ============================================================================

fn row_text(frame: &Frame, y: u16) -> String {
    let mut out = String::new();
    for x in 0..frame.buffer.width() {
        let ch = frame
            .buffer
            .get(x, y)
            .and_then(|cell| cell.content.as_char())
            .unwrap_or(' ');
        out.push(ch);
    }
    out
}

fn all_rows(frame: &Frame) -> Vec<String> {
    (0..frame.buffer.height())
        .map(|y| row_text(frame, y))
        .collect()
}

fn contains_any(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|r| r.contains(needle))
}

/// Build a disclosure at any level with externally injected data.
fn build_disclosure(
    level: DisclosureLevel,
    signal: TrafficLight,
    domain: DecisionDomain,
) -> Disclosure {
    let explanation = if level >= DisclosureLevel::PlainEnglish {
        Some(format!(
            "{}: chose 'incremental_diff' with {} confidence.",
            match domain {
                DecisionDomain::DiffStrategy => "Diff strategy",
                DecisionDomain::ResizeCoalescing => "Resize coalescing",
                DecisionDomain::FrameBudget => "Frame budget",
                _ => "Decision",
            },
            match signal {
                TrafficLight::Green => "high",
                TrafficLight::Yellow => "moderate",
                TrafficLight::Red => "low",
            }
        ))
    } else {
        None
    };

    let evidence_terms = if level >= DisclosureLevel::EvidenceTerms {
        Some(vec![
            DisclosureEvidence {
                label: "change_density",
                bayes_factor: 5.2,
                direction: EvidenceDirection::Supporting,
            },
            DisclosureEvidence {
                label: "scroll_velocity",
                bayes_factor: 0.6,
                direction: EvidenceDirection::Opposing,
            },
            DisclosureEvidence {
                label: "cursor_proximity",
                bayes_factor: 1.05,
                direction: EvidenceDirection::Neutral,
            },
        ])
    } else {
        None
    };

    let bayesian_details = if level >= DisclosureLevel::FullBayesian {
        Some(BayesianDetails {
            log_posterior: 1.8,
            confidence_interval: (0.65, 0.92),
            expected_loss: 0.12,
            next_best_loss: 0.45,
            loss_avoided: 0.33,
        })
    } else {
        None
    };

    Disclosure {
        domain,
        level,
        signal,
        action_label: "incremental_diff".to_string(),
        explanation,
        evidence_terms,
        bayesian_details,
    }
}

// ============================================================================
// Level 0: Traffic Light — badge + action only
// ============================================================================

#[test]
fn standalone_level0_renders_badge_and_action() {
    let disc = build_disclosure(
        DisclosureLevel::TrafficLight,
        TrafficLight::Green,
        DecisionDomain::DiffStrategy,
    );
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(50, 5, &mut pool);
    DecisionCard::new(&disc).render(Rect::new(0, 0, 50, 5), &mut frame);

    let rows = all_rows(&frame);
    assert!(
        contains_any(&rows, "OK"),
        "Level 0 should show OK badge: {rows:?}"
    );
    assert!(
        contains_any(&rows, "incremental_diff"),
        "Level 0 should show action label: {rows:?}"
    );
    // Level 0 should NOT show explanation or evidence
    assert!(
        !contains_any(&rows, "Diff strategy"),
        "Level 0 should not show explanation"
    );
    assert!(
        !contains_any(&rows, "Evidence:"),
        "Level 0 should not show evidence"
    );
}

// ============================================================================
// Level 1: Plain English — explanation added
// ============================================================================

#[test]
fn standalone_level1_adds_explanation() {
    let disc = build_disclosure(
        DisclosureLevel::PlainEnglish,
        TrafficLight::Green,
        DecisionDomain::DiffStrategy,
    );
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(70, 6, &mut pool);
    DecisionCard::new(&disc).render(Rect::new(0, 0, 70, 6), &mut frame);

    let rows = all_rows(&frame);
    assert!(contains_any(&rows, "OK"), "Level 1 still shows badge");
    assert!(
        contains_any(&rows, "Diff strategy"),
        "Level 1 should show explanation: {rows:?}"
    );
    assert!(
        contains_any(&rows, "high confidence"),
        "Level 1 should show confidence: {rows:?}"
    );
    // Level 1 should NOT show evidence
    assert!(
        !contains_any(&rows, "Evidence:"),
        "Level 1 should not show evidence"
    );
}

// ============================================================================
// Level 2: Evidence Terms — Bayes factors added
// ============================================================================

#[test]
fn standalone_level2_adds_evidence_terms() {
    let disc = build_disclosure(
        DisclosureLevel::EvidenceTerms,
        TrafficLight::Yellow,
        DecisionDomain::ResizeCoalescing,
    );
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(70, 12, &mut pool);
    DecisionCard::new(&disc).render(Rect::new(0, 0, 70, 12), &mut frame);

    let rows = all_rows(&frame);
    assert!(
        contains_any(&rows, "WARN"),
        "Level 2 should show WARN for yellow"
    );
    assert!(
        contains_any(&rows, "Evidence:"),
        "Level 2 should show evidence header: {rows:?}"
    );
    assert!(
        contains_any(&rows, "change_density"),
        "Level 2 should show evidence term: {rows:?}"
    );
    assert!(
        contains_any(&rows, "scroll_velocity"),
        "Level 2 should show opposing term: {rows:?}"
    );
    assert!(
        contains_any(&rows, "BF="),
        "Level 2 should show Bayes factor: {rows:?}"
    );
    // Level 2 should NOT show full Bayesian stats
    assert!(
        !contains_any(&rows, "log_post"),
        "Level 2 should not show Bayesian details"
    );
}

// ============================================================================
// Level 3: Full Bayesian — complete stats
// ============================================================================

#[test]
fn standalone_level3_adds_bayesian_details() {
    let disc = build_disclosure(
        DisclosureLevel::FullBayesian,
        TrafficLight::Red,
        DecisionDomain::FrameBudget,
    );
    let card = DecisionCard::new(&disc);
    let min_h = card.min_height();

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(70, min_h.max(12), &mut pool);
    card.render(Rect::new(0, 0, 70, min_h.max(12)), &mut frame);

    let rows = all_rows(&frame);
    assert!(
        contains_any(&rows, "ALERT"),
        "Level 3 should show ALERT for red"
    );
    assert!(
        contains_any(&rows, "log_post"),
        "Level 3 should show log-posterior: {rows:?}"
    );
    assert!(
        contains_any(&rows, "loss="),
        "Level 3 should show expected loss: {rows:?}"
    );
    assert!(
        contains_any(&rows, "avoided="),
        "Level 3 should show loss avoided: {rows:?}"
    );
    // Also includes everything from lower levels
    assert!(
        contains_any(&rows, "Evidence:"),
        "Level 3 includes level 2 evidence"
    );
    assert!(
        contains_any(&rows, "Frame budget"),
        "Level 3 includes level 1 explanation"
    );
}

// ============================================================================
// External evidence injection — custom domain + terms
// ============================================================================

#[test]
fn standalone_external_evidence_injection() {
    let disc = Disclosure {
        domain: DecisionDomain::PaletteScoring,
        level: DisclosureLevel::EvidenceTerms,
        signal: TrafficLight::Green,
        action_label: "custom_action".to_string(),
        explanation: Some(
            "Palette scoring: chose 'custom_action' with high confidence.".to_string(),
        ),
        evidence_terms: Some(vec![
            DisclosureEvidence {
                label: "user_preference",
                bayes_factor: 12.0,
                direction: EvidenceDirection::Supporting,
            },
            DisclosureEvidence {
                label: "latency_penalty",
                bayes_factor: 0.3,
                direction: EvidenceDirection::Opposing,
            },
        ]),
        bayesian_details: None,
    };

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(70, 10, &mut pool);
    DecisionCard::new(&disc).render(Rect::new(0, 0, 70, 10), &mut frame);

    let rows = all_rows(&frame);
    assert!(
        contains_any(&rows, "custom_action"),
        "Should render externally injected action"
    );
    assert!(
        contains_any(&rows, "user_preference"),
        "Should render externally injected evidence: {rows:?}"
    );
    assert!(
        contains_any(&rows, "latency_penalty"),
        "Should render opposing evidence: {rows:?}"
    );
}

// ============================================================================
// All 7 domains render correctly
// ============================================================================

#[test]
fn standalone_all_domains_render() {
    let domains = [
        DecisionDomain::DiffStrategy,
        DecisionDomain::ResizeCoalescing,
        DecisionDomain::FrameBudget,
        DecisionDomain::Degradation,
        DecisionDomain::VoiSampling,
        DecisionDomain::HintRanking,
        DecisionDomain::PaletteScoring,
    ];

    for &domain in &domains {
        let disc = build_disclosure(DisclosureLevel::PlainEnglish, TrafficLight::Green, domain);
        let mut pool = GraphemePool::new();
        let mut frame = Frame::new(70, 6, &mut pool);
        DecisionCard::new(&disc).render(Rect::new(0, 0, 70, 6), &mut frame);

        let rows = all_rows(&frame);
        assert!(
            contains_any(&rows, "OK"),
            "Domain {domain:?} should render badge"
        );
        assert!(
            contains_any(&rows, "incremental_diff"),
            "Domain {domain:?} should render action"
        );
    }
}

// ============================================================================
// All 3 traffic light signals render distinct labels
// ============================================================================

#[test]
fn standalone_traffic_lights_distinct() {
    let signals = [
        (TrafficLight::Green, "OK"),
        (TrafficLight::Yellow, "WARN"),
        (TrafficLight::Red, "ALERT"),
    ];

    for &(signal, expected_label) in &signals {
        let disc = build_disclosure(
            DisclosureLevel::TrafficLight,
            signal,
            DecisionDomain::DiffStrategy,
        );
        let mut pool = GraphemePool::new();
        let mut frame = Frame::new(40, 5, &mut pool);
        DecisionCard::new(&disc).render(Rect::new(0, 0, 40, 5), &mut frame);

        let rows = all_rows(&frame);
        assert!(
            contains_any(&rows, expected_label),
            "{signal:?} should show {expected_label}: {rows:?}"
        );
    }
}

// ============================================================================
// Degenerate areas — no panics
// ============================================================================

#[test]
fn standalone_degenerate_areas_safe() {
    let disc = build_disclosure(
        DisclosureLevel::FullBayesian,
        TrafficLight::Green,
        DecisionDomain::DiffStrategy,
    );

    let test_areas = [
        (0, 0),
        (1, 1),
        (2, 2),
        (3, 3), // Below min 4x3
        (4, 3), // Exact minimum
        (5, 3),
        (100, 1), // Wide but too short
        (1, 100), // Tall but too narrow
    ];

    for (w, h) in test_areas {
        let mut pool = GraphemePool::new();
        let actual_w = w.max(1);
        let actual_h = h.max(1);
        let mut frame = Frame::new(actual_w, actual_h, &mut pool);
        DecisionCard::new(&disc).render(Rect::new(0, 0, w, h), &mut frame);
        // If we get here without panic, the test passes
    }
}

// ============================================================================
// min_height is accurate for all levels
// ============================================================================

#[test]
fn standalone_min_height_matches_content() {
    for level in [
        DisclosureLevel::TrafficLight,
        DisclosureLevel::PlainEnglish,
        DisclosureLevel::EvidenceTerms,
        DisclosureLevel::FullBayesian,
    ] {
        let disc = build_disclosure(level, TrafficLight::Green, DecisionDomain::DiffStrategy);
        let card = DecisionCard::new(&disc);
        let min_h = card.min_height();

        // Rendering at min_height should not truncate critical content
        let mut pool = GraphemePool::new();
        let mut frame = Frame::new(70, min_h, &mut pool);
        card.render(Rect::new(0, 0, 70, min_h), &mut frame);

        let rows = all_rows(&frame);
        assert!(
            contains_any(&rows, "incremental_diff"),
            "At min_height for {level:?}, action should still render: {rows:?}"
        );
    }
}

// ============================================================================
// Compose with other widgets — no runtime needed
// ============================================================================

#[test]
fn standalone_compose_with_block() {
    use ftui_widgets::block::Block;
    use ftui_widgets::borders::Borders;

    let disc = build_disclosure(
        DisclosureLevel::PlainEnglish,
        TrafficLight::Green,
        DecisionDomain::DiffStrategy,
    );

    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(72, 8, &mut pool);

    // Render a containing block
    let block = Block::default().title("Dashboard").borders(Borders::ALL);
    let inner = block.inner(Rect::new(0, 0, 72, 8));
    Widget::render(&block, Rect::new(0, 0, 72, 8), &mut frame);

    // Render decision card inside
    DecisionCard::new(&disc).render(inner, &mut frame);

    let rows = all_rows(&frame);
    assert!(
        contains_any(&rows, "Dashboard"),
        "Containing block should render: {rows:?}"
    );
    assert!(
        contains_any(&rows, "OK"),
        "Decision card badge should render inside block: {rows:?}"
    );
}
