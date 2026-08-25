//! Standalone tests for the drift visualization widget (bd-1lgz8.2).

use ftui_core::geometry::Rect;
use ftui_render::cell::PackedRgba;
use ftui_render::frame::Frame;
use ftui_render::grapheme_pool::GraphemePool;
use ftui_runtime::transparency::TrafficLight;
use ftui_runtime::unified_evidence::DecisionDomain;
use ftui_style::Style;
use ftui_widgets::Widget;
use ftui_widgets::borders::BorderType;
use ftui_widgets::drift_visualization::{
    DomainSnapshot, DriftSnapshot, DriftTimeline, DriftVisualization,
};

fn make_domain_snapshot(
    domain: DecisionDomain,
    confidence: f64,
    in_fallback: bool,
) -> DomainSnapshot {
    DomainSnapshot {
        domain,
        confidence,
        signal: if confidence >= 0.7 {
            TrafficLight::Green
        } else if confidence >= 0.3 {
            TrafficLight::Yellow
        } else {
            TrafficLight::Red
        },
        in_fallback,
        regime_label: if in_fallback {
            "deterministic"
        } else {
            "bayesian"
        },
    }
}

fn make_snapshot(frame_id: u64, domains: Vec<DomainSnapshot>) -> DriftSnapshot {
    DriftSnapshot { domains, frame_id }
}

fn extract_row(frame: &Frame, y: u16, width: u16) -> String {
    let mut row = String::new();
    for x in 0..width {
        if let Some(cell) = frame.buffer.get(x, y) {
            if let Some(ch) = cell.content.as_char() {
                row.push(ch);
            } else {
                row.push(' ');
            }
        }
    }
    row
}

fn extract_all_text(frame: &Frame, width: u16, height: u16) -> String {
    let mut text = String::new();
    for y in 0..height {
        text.push_str(&extract_row(frame, y, width));
        text.push('\n');
    }
    text
}

// --- Timeline tests ---

#[test]
fn standalone_timeline_capacity_one() {
    let mut tl = DriftTimeline::new(1);
    tl.push(make_snapshot(
        0,
        vec![make_domain_snapshot(
            DecisionDomain::DiffStrategy,
            0.8,
            false,
        )],
    ));
    tl.push(make_snapshot(
        1,
        vec![make_domain_snapshot(
            DecisionDomain::DiffStrategy,
            0.5,
            false,
        )],
    ));
    assert_eq!(tl.len(), 1);
    assert_eq!(tl.latest().unwrap().frame_id, 1);
}

#[test]
fn standalone_confidence_series_multi_domain() {
    let mut tl = DriftTimeline::new(5);
    for i in 0..5 {
        tl.push(make_snapshot(
            i,
            vec![
                make_domain_snapshot(DecisionDomain::DiffStrategy, 0.5 + i as f64 * 0.1, false),
                make_domain_snapshot(
                    DecisionDomain::ResizeCoalescing,
                    0.9 - i as f64 * 0.1,
                    false,
                ),
            ],
        ));
    }

    let diff_series = tl.confidence_series(DecisionDomain::DiffStrategy);
    assert_eq!(diff_series.len(), 5);
    assert!((diff_series[0] - 0.5).abs() < 0.01);
    assert!((diff_series[4] - 0.9).abs() < 0.01);

    let resize_series = tl.confidence_series(DecisionDomain::ResizeCoalescing);
    assert_eq!(resize_series.len(), 5);
    assert!((resize_series[0] - 0.9).abs() < 0.01);
    assert!((resize_series[4] - 0.5).abs() < 0.01);
}

#[test]
fn standalone_fallback_trigger_finds_last_edge() {
    let mut tl = DriftTimeline::new(20);
    // Normal, then fallback, then normal, then fallback again
    for i in 0..5 {
        tl.push(make_snapshot(
            i,
            vec![make_domain_snapshot(
                DecisionDomain::DiffStrategy,
                0.8,
                false,
            )],
        ));
    }
    for i in 5..10 {
        tl.push(make_snapshot(
            i,
            vec![make_domain_snapshot(
                DecisionDomain::DiffStrategy,
                0.1,
                true,
            )],
        ));
    }
    for i in 10..15 {
        tl.push(make_snapshot(
            i,
            vec![make_domain_snapshot(
                DecisionDomain::DiffStrategy,
                0.8,
                false,
            )],
        ));
    }
    for i in 15..20 {
        tl.push(make_snapshot(
            i,
            vec![make_domain_snapshot(
                DecisionDomain::DiffStrategy,
                0.1,
                true,
            )],
        ));
    }

    // Should find the LAST rising edge at index 15
    let trigger = tl.last_fallback_trigger(DecisionDomain::DiffStrategy);
    assert_eq!(trigger, Some(15));
}

// --- Rendering tests ---

#[test]
fn standalone_render_all_domains() {
    let mut tl = DriftTimeline::new(30);
    for i in 0..30 {
        tl.push(make_snapshot(
            i,
            vec![
                make_domain_snapshot(DecisionDomain::DiffStrategy, 0.8, false),
                make_domain_snapshot(DecisionDomain::ResizeCoalescing, 0.6, false),
                make_domain_snapshot(DecisionDomain::FrameBudget, 0.3, false),
            ],
        ));
    }

    let viz = DriftVisualization::new(&tl);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 20, &mut pool);
    viz.render(Rect::new(0, 0, 80, 20), &mut frame);

    let text = extract_all_text(&frame, 80, 20);
    assert!(text.contains("Drift Monitor"), "should have title");
    assert!(text.contains("diff_strategy"), "should show DiffStrategy");
    assert!(
        text.contains("resize_coalescing"),
        "should show ResizeCoalescing"
    );
    assert!(text.contains("frame_budget"), "should show FrameBudget");
}

#[test]
fn standalone_render_confidence_percentages() {
    let mut tl = DriftTimeline::new(5);
    tl.push(make_snapshot(
        0,
        vec![make_domain_snapshot(
            DecisionDomain::DiffStrategy,
            0.85,
            false,
        )],
    ));

    let viz = DriftVisualization::new(&tl).domains(vec![DecisionDomain::DiffStrategy]);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(60, 8, &mut pool);
    viz.render(Rect::new(0, 0, 60, 8), &mut frame);

    let text = extract_all_text(&frame, 60, 8);
    assert!(text.contains("85%"), "should show confidence percentage");
}

#[test]
fn standalone_drift_sequence_rendering() {
    let mut tl = DriftTimeline::new(20);

    // Normal
    for i in 0..10 {
        tl.push(make_snapshot(
            i,
            vec![make_domain_snapshot(
                DecisionDomain::DiffStrategy,
                0.9,
                false,
            )],
        ));
    }
    // Drift
    for i in 10..15 {
        let conf = 0.9 - (i - 10) as f64 * 0.15;
        tl.push(make_snapshot(
            i,
            vec![make_domain_snapshot(
                DecisionDomain::DiffStrategy,
                conf,
                conf < 0.3,
            )],
        ));
    }
    // Fallback
    for i in 15..20 {
        tl.push(make_snapshot(
            i,
            vec![make_domain_snapshot(
                DecisionDomain::DiffStrategy,
                0.1,
                true,
            )],
        ));
    }

    let viz = DriftVisualization::new(&tl).domains(vec![DecisionDomain::DiffStrategy]);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 10, &mut pool);
    viz.render(Rect::new(0, 0, 80, 10), &mut frame);

    let text = extract_all_text(&frame, 80, 10);
    assert!(text.contains("FALLBACK"), "should show fallback badge");
    assert!(text.contains("REGIME"), "should show regime banner");
}

#[test]
fn standalone_compose_with_border_types() {
    let mut tl = DriftTimeline::new(5);
    tl.push(make_snapshot(
        0,
        vec![make_domain_snapshot(
            DecisionDomain::DiffStrategy,
            0.7,
            false,
        )],
    ));

    for border in [
        BorderType::Square,
        BorderType::Rounded,
        BorderType::Double,
        BorderType::Heavy,
    ] {
        let viz = DriftVisualization::new(&tl).border_type(border);
        let mut pool = GraphemePool::new();
        let mut frame = Frame::new(40, 8, &mut pool);
        viz.render(Rect::new(0, 0, 40, 8), &mut frame);
        // Should not panic with any border type
    }
}

#[test]
fn standalone_degenerate_areas_safe() {
    let tl = DriftTimeline::new(1);
    let viz = DriftVisualization::new(&tl);
    let mut pool = GraphemePool::new();

    // Zero area
    let mut frame = Frame::new(1, 1, &mut pool);
    viz.render(Rect::new(0, 0, 0, 0), &mut frame);

    // Very narrow
    let mut frame = Frame::new(3, 3, &mut pool);
    viz.render(Rect::new(0, 0, 3, 3), &mut frame);

    // Very short
    let mut frame = Frame::new(80, 2, &mut pool);
    viz.render(Rect::new(0, 0, 80, 2), &mut frame);
}

#[test]
fn standalone_custom_thresholds() {
    let mut tl = DriftTimeline::new(5);
    tl.push(make_snapshot(
        0,
        vec![make_domain_snapshot(
            DecisionDomain::DiffStrategy,
            0.5,
            false,
        )],
    ));

    let viz = DriftVisualization::new(&tl)
        .fallback_threshold(0.4)
        .caution_threshold(0.6);
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 10, &mut pool);
    viz.render(Rect::new(0, 0, 80, 10), &mut frame);
    // Confidence 0.5 is between 0.4 and 0.6, so yellow zone — should not crash
}

#[test]
fn standalone_style_customization() {
    let mut tl = DriftTimeline::new(5);
    tl.push(make_snapshot(
        0,
        vec![make_domain_snapshot(
            DecisionDomain::DiffStrategy,
            0.8,
            false,
        )],
    ));

    let viz = DriftVisualization::new(&tl).style(Style::new().bg(PackedRgba::rgb(20, 20, 40)));
    let mut pool = GraphemePool::new();
    let mut frame = Frame::new(80, 10, &mut pool);
    viz.render(Rect::new(0, 0, 80, 10), &mut frame);
}
