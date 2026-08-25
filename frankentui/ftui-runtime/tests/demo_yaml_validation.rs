#![forbid(unsafe_code)]

//! bd-2xj.3: CI test — validates demo.yaml schema and exercises demo replay.
//!
//! Run:
//!   cargo test -p ftui-runtime --test demo_yaml_validation

use ftui_runtime::demo::{DemoStep, parse_demo_yaml, validate_demos};

// ============================================================================
// Load the real demo.yaml from project root
// ============================================================================

fn load_demo_yaml() -> &'static str {
    include_str!("../../../demo.yaml")
}

// ============================================================================
// Schema Validation
// ============================================================================

#[test]
fn demo_yaml_parses_without_errors() {
    let yaml = load_demo_yaml();
    let demos = parse_demo_yaml(yaml).unwrap_or_else(|errors| {
        panic!(
            "demo.yaml has {} parse errors:\n{}",
            errors.len(),
            errors
                .iter()
                .map(|e| format!("  - {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    });

    assert!(
        !demos.is_empty(),
        "demo.yaml should define at least one demo"
    );
}

#[test]
fn demo_yaml_passes_validation() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    let errors = validate_demos(&demos);

    assert!(
        errors.is_empty(),
        "demo.yaml has {} validation errors:\n{}",
        errors.len(),
        errors
            .iter()
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ============================================================================
// Demo Count and Coverage
// ============================================================================

#[test]
fn demo_yaml_has_at_least_5_demos() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    assert!(
        demos.len() >= 5,
        "demo.yaml should define at least 5 demos, found {}",
        demos.len()
    );
}

#[test]
fn demo_yaml_all_ids_unique() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    let mut ids = std::collections::HashSet::new();
    for demo in &demos {
        assert!(
            ids.insert(&demo.demo_id),
            "duplicate demo_id: {}",
            demo.demo_id
        );
    }
}

#[test]
fn demo_yaml_all_timeouts_under_60s() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    for demo in &demos {
        assert!(
            demo.timeout_seconds <= 60,
            "demo '{}' timeout {} exceeds 60s limit",
            demo.demo_id,
            demo.timeout_seconds
        );
    }
}

#[test]
fn demo_yaml_all_have_claims() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    for demo in &demos {
        assert!(
            !demo.claim.is_empty(),
            "demo '{}' has empty claim",
            demo.demo_id
        );
    }
}

#[test]
fn demo_yaml_all_have_steps() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    for demo in &demos {
        assert!(
            !demo.steps.is_empty(),
            "demo '{}' has no steps",
            demo.demo_id
        );
    }
}

#[test]
fn demo_yaml_terminal_sizes_valid() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    for demo in &demos {
        assert!(
            demo.terminal_width >= 40 && demo.terminal_width <= 300,
            "demo '{}' width {} out of range [40, 300]",
            demo.demo_id,
            demo.terminal_width
        );
        assert!(
            demo.terminal_height >= 10 && demo.terminal_height <= 100,
            "demo '{}' height {} out of range [10, 100]",
            demo.demo_id,
            demo.terminal_height
        );
    }
}

// ============================================================================
// Required Demos
// ============================================================================

#[test]
fn demo_yaml_has_widget_gallery() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    assert!(
        demos.iter().any(|d| d.demo_id == "widget_gallery"),
        "demo.yaml should include 'widget_gallery' demo"
    );
}

#[test]
fn demo_yaml_has_decision_card() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    assert!(
        demos
            .iter()
            .any(|d| d.demo_id == "galaxy_brain_decision_card"),
        "demo.yaml should include 'galaxy_brain_decision_card' demo"
    );
}

#[test]
fn demo_yaml_has_diff_strategy() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    assert!(
        demos.iter().any(|d| d.demo_id == "bayesian_diff_strategy"),
        "demo.yaml should include 'bayesian_diff_strategy' demo"
    );
}

#[test]
fn demo_yaml_has_incremental_layout() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    assert!(
        demos.iter().any(|d| d.demo_id == "incremental_layout"),
        "demo.yaml should include 'incremental_layout' demo"
    );
}

#[test]
fn demo_yaml_has_deterministic_replay() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    assert!(
        demos.iter().any(|d| d.demo_id == "deterministic_replay"),
        "demo.yaml should include 'deterministic_replay' demo"
    );
}

// ============================================================================
// Step Type Coverage
// ============================================================================

#[test]
fn demo_yaml_uses_all_step_types() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    let all_steps: Vec<&DemoStep> = demos.iter().flat_map(|d| &d.steps).collect();

    let has_render = all_steps
        .iter()
        .any(|s| matches!(s, DemoStep::Render { .. }));
    let has_resize = all_steps
        .iter()
        .any(|s| matches!(s, DemoStep::Resize { .. }));
    let has_checksum = all_steps
        .iter()
        .any(|s| matches!(s, DemoStep::AssertChecksum { .. }));
    let has_content = all_steps
        .iter()
        .any(|s| matches!(s, DemoStep::AssertContent { .. }));
    let has_timing = all_steps
        .iter()
        .any(|s| matches!(s, DemoStep::MeasureTiming { .. }));

    assert!(has_render, "demo.yaml should use 'render' step type");
    assert!(has_resize, "demo.yaml should use 'resize' step type");
    assert!(
        has_checksum,
        "demo.yaml should use 'assert_checksum' step type"
    );
    assert!(
        has_content,
        "demo.yaml should use 'assert_content' step type"
    );
    assert!(
        has_timing,
        "demo.yaml should use 'measure_timing' step type"
    );
}

// ============================================================================
// Tag Coverage
// ============================================================================

#[test]
fn demo_yaml_all_demos_have_tags() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    for demo in &demos {
        assert!(!demo.tags.is_empty(), "demo '{}' has no tags", demo.demo_id);
    }
}

#[test]
fn demo_yaml_tags_cover_key_areas() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    let all_tags: std::collections::HashSet<&str> = demos
        .iter()
        .flat_map(|d| d.tags.iter().map(|s| s.as_str()))
        .collect();

    let required_tags = ["widgets", "performance", "determinism"];
    for tag in &required_tags {
        assert!(
            all_tags.contains(tag),
            "demo.yaml should have demos tagged '{tag}'"
        );
    }
}

// ============================================================================
// Parse Determinism
// ============================================================================

#[test]
fn demo_yaml_parse_is_deterministic() {
    let yaml = load_demo_yaml();
    let demos1 = parse_demo_yaml(yaml).unwrap();
    let demos2 = parse_demo_yaml(yaml).unwrap();

    assert_eq!(demos1.len(), demos2.len());
    for (d1, d2) in demos1.iter().zip(demos2.iter()) {
        assert_eq!(d1.demo_id, d2.demo_id);
        assert_eq!(d1.title, d2.title);
        assert_eq!(d1.claim, d2.claim);
        assert_eq!(d1.timeout_seconds, d2.timeout_seconds);
        assert_eq!(d1.terminal_width, d2.terminal_width);
        assert_eq!(d1.terminal_height, d2.terminal_height);
        assert_eq!(d1.steps.len(), d2.steps.len());
    }
}

// ============================================================================
// Widget Gallery Demo Specifics
// ============================================================================

#[test]
fn widget_gallery_renders_12_plus_widgets() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    let gallery = demos
        .iter()
        .find(|d| d.demo_id == "widget_gallery")
        .expect("widget_gallery demo");

    let render_count = gallery
        .steps
        .iter()
        .filter(|s| matches!(s, DemoStep::Render { .. }))
        .count();

    assert!(
        render_count >= 12,
        "widget_gallery should render 12+ widgets, found {render_count}"
    );
}

// ============================================================================
// Decision Card Demo Specifics
// ============================================================================

#[test]
fn decision_card_covers_all_4_levels() {
    let demos = parse_demo_yaml(load_demo_yaml()).unwrap();
    let card = demos
        .iter()
        .find(|d| d.demo_id == "galaxy_brain_decision_card")
        .expect("galaxy_brain_decision_card demo");

    let levels: Vec<&str> = card
        .steps
        .iter()
        .filter_map(|s| match s {
            DemoStep::Render { level: Some(l), .. } => Some(l.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        levels.contains(&"traffic_light"),
        "should render level 0 (traffic_light)"
    );
    assert!(
        levels.contains(&"plain_english"),
        "should render level 1 (plain_english)"
    );
    assert!(
        levels.contains(&"evidence_terms"),
        "should render level 2 (evidence_terms)"
    );
    assert!(
        levels.contains(&"full_bayesian"),
        "should render level 3 (full_bayesian)"
    );
}
