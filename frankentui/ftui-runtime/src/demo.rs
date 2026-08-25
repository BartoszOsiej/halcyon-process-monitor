#![forbid(unsafe_code)]

//! Demo definition parser and runner (bd-2xj.3).
//!
//! Parses `demo.yaml` into structured demo definitions and provides
//! validation for demo schemas.
//!
//! # demo.yaml schema
//!
//! ```yaml
//! demos:
//!   - demo_id: widget_gallery
//!     title: "Widget Gallery"
//!     claim: "Renders 12+ widgets correctly"
//!     timeout_seconds: 10
//!     terminal_size: [120, 40]
//!     tags: [widgets, rendering]
//!     steps:
//!       - type: render
//!         widget: block
//!       - type: assert_content
//!         contains: ["Block"]
//!       - type: measure_timing
//!         metric: render_frame_us
//!         max_us: 4000
//! ```

use std::collections::HashSet;

// ============================================================================
// Types
// ============================================================================

/// A parsed demo definition.
#[derive(Debug, Clone)]
pub struct DemoDefinition {
    pub demo_id: String,
    pub title: String,
    pub claim: String,
    pub timeout_seconds: u32,
    pub terminal_width: u16,
    pub terminal_height: u16,
    pub tags: Vec<String>,
    pub steps: Vec<DemoStep>,
}

/// A single step in a demo.
#[derive(Debug, Clone)]
pub enum DemoStep {
    /// Render a widget.
    Render {
        widget: String,
        description: String,
        level: Option<String>,
        signal: Option<String>,
        seed: Option<u64>,
    },
    /// Resize the terminal.
    Resize {
        width: u16,
        height: u16,
        description: String,
    },
    /// Assert a BLAKE3 checksum matches.
    AssertChecksum { description: String },
    /// Assert rendered content contains strings.
    AssertContent {
        contains: Vec<String>,
        description: String,
    },
    /// Measure timing of an operation.
    MeasureTiming {
        metric: String,
        max_us: Option<u64>,
        description: String,
    },
}

/// Demo parsing error.
#[derive(Debug, Clone, PartialEq)]
pub enum DemoParseError {
    /// Missing required field.
    MissingField { demo_id: String, field: String },
    /// Invalid value.
    InvalidValue {
        demo_id: String,
        field: String,
        reason: String,
    },
    /// Duplicate demo_id.
    DuplicateId(String),
    /// No demos found.
    NoDemos,
    /// Structural error.
    StructuralError(String),
}

impl std::fmt::Display for DemoParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField { demo_id, field } => {
                write!(f, "demo '{demo_id}': missing required field '{field}'")
            }
            Self::InvalidValue {
                demo_id,
                field,
                reason,
            } => {
                write!(f, "demo '{demo_id}': invalid '{field}': {reason}")
            }
            Self::DuplicateId(id) => write!(f, "duplicate demo_id: '{id}'"),
            Self::NoDemos => write!(f, "no demos defined"),
            Self::StructuralError(msg) => write!(f, "structural error: {msg}"),
        }
    }
}

impl std::error::Error for DemoParseError {}

// ============================================================================
// Parser
// ============================================================================

/// Parse demo.yaml content into structured definitions.
///
/// This is a lightweight line-based parser (no serde_yaml dependency).
pub fn parse_demo_yaml(yaml: &str) -> Result<Vec<DemoDefinition>, Vec<DemoParseError>> {
    let mut demos = Vec::new();
    let mut errors = Vec::new();
    let mut seen_ids = HashSet::new();

    let mut current_demo: Option<DemoBuilder> = None;
    let mut in_steps = false;
    let mut in_contains = false;
    let mut in_tags = false;
    let mut in_terminal_size = false;
    let mut current_step: Option<StepBuilder> = None;

    for line in yaml.lines() {
        let trimmed = line.trim();

        // Skip blanks and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        let _ = indent; // used for terminal_size list items

        // New demo entry
        if trimmed == "- demo_id:" || trimmed.starts_with("- demo_id:") {
            // Flush previous demo
            if let Some(mut builder) = current_demo.take() {
                flush_step(&mut current_step, &mut builder.steps);
                match builder.build() {
                    Ok(demo) => demos.push(demo),
                    Err(errs) => errors.extend(errs),
                }
            }
            let id = trimmed
                .strip_prefix("- demo_id:")
                .unwrap_or("")
                .trim()
                .to_string();
            if !id.is_empty() && !seen_ids.insert(id.clone()) {
                errors.push(DemoParseError::DuplicateId(id.clone()));
            }
            current_demo = Some(DemoBuilder::new(id));
            in_steps = false;
            in_contains = false;
            in_tags = false;
            in_terminal_size = false;
            current_step = None;
            continue;
        }

        let Some(ref mut demo) = current_demo else {
            continue;
        };

        // Parse demo-level fields
        if let Some(val) = trimmed.strip_prefix("title:") {
            demo.title = Some(unquote(val.trim()));
            in_steps = false;
            in_contains = false;
            in_tags = false;
            in_terminal_size = false;
        } else if let Some(val) = trimmed.strip_prefix("claim:") {
            demo.claim = Some(unquote(val.trim()));
            in_steps = false;
            in_contains = false;
            in_tags = false;
            in_terminal_size = false;
        } else if let Some(val) = trimmed.strip_prefix("timeout_seconds:") {
            demo.timeout_seconds = val.trim().parse().ok();
            in_steps = false;
            in_contains = false;
            in_tags = false;
            in_terminal_size = false;
        } else if trimmed.starts_with("terminal_size:") {
            let val = trimmed.strip_prefix("terminal_size:").unwrap().trim();
            // Inline array: [120, 40]
            if let Some(inner) = val.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                let parts: Vec<&str> = inner.split(',').collect();
                if parts.len() == 2 {
                    demo.terminal_width = parts[0].trim().parse().ok();
                    demo.terminal_height = parts[1].trim().parse().ok();
                }
            } else {
                in_terminal_size = true;
            }
            in_steps = false;
            in_contains = false;
            in_tags = false;
        } else if trimmed.starts_with("tags:") {
            let val = trimmed.strip_prefix("tags:").unwrap().trim();
            // Inline array: [widgets, rendering]
            if let Some(inner) = val.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                demo.tags = inner.split(',').map(|s| s.trim().to_string()).collect();
            } else {
                in_tags = true;
                demo.tags.clear();
            }
            in_steps = false;
            in_contains = false;
            in_terminal_size = false;
        } else if trimmed == "steps:" {
            in_steps = true;
            in_contains = false;
            in_tags = false;
            in_terminal_size = false;
        } else if in_tags && trimmed.starts_with("- ") {
            demo.tags.push(trimmed[2..].trim().to_string());
        } else if in_terminal_size && indent >= 6 {
            // YAML list items for terminal_size
            if let Some(val) = trimmed.strip_prefix("- ") {
                if demo.terminal_width.is_none() {
                    demo.terminal_width = val.trim().parse().ok();
                } else {
                    demo.terminal_height = val.trim().parse().ok();
                }
            }
        } else if in_steps {
            // Step parsing
            if trimmed.starts_with("- type:") {
                // Flush previous step
                flush_step(&mut current_step, &mut demo.steps);
                let step_type = trimmed.strip_prefix("- type:").unwrap().trim();
                current_step = Some(StepBuilder::new(step_type));
            } else if let Some(ref mut step) = current_step {
                if let Some(val) = trimmed.strip_prefix("widget:") {
                    step.widget = Some(val.trim().to_string());
                } else if let Some(val) = trimmed.strip_prefix("description:") {
                    step.description = Some(unquote(val.trim()));
                } else if let Some(val) = trimmed.strip_prefix("level:") {
                    step.level = Some(val.trim().to_string());
                } else if let Some(val) = trimmed.strip_prefix("signal:") {
                    step.signal = Some(val.trim().to_string());
                } else if let Some(val) = trimmed.strip_prefix("seed:") {
                    step.seed = val.trim().parse().ok();
                } else if let Some(val) = trimmed.strip_prefix("metric:") {
                    step.metric = Some(val.trim().to_string());
                } else if let Some(val) = trimmed.strip_prefix("max_us:") {
                    step.max_us = val.trim().parse().ok();
                } else if let Some(val) = trimmed.strip_prefix("to:") {
                    // Inline array: [80, 24]
                    let val = val.trim();
                    if let Some(inner) = val.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                        let parts: Vec<&str> = inner.split(',').collect();
                        if parts.len() == 2 {
                            step.to_width = parts[0].trim().parse().ok();
                            step.to_height = parts[1].trim().parse().ok();
                        }
                    }
                } else if trimmed.starts_with("contains:") {
                    let val = trimmed.strip_prefix("contains:").unwrap().trim();
                    if let Some(inner) = val.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                        step.contains = inner.split(',').map(|s| unquote(s.trim())).collect();
                    } else {
                        in_contains = true;
                        step.contains.clear();
                    }
                } else if in_contains && trimmed.starts_with("- ") {
                    step.contains.push(unquote(trimmed[2..].trim()));
                }
            }
        }
    }

    // Flush last demo
    if let Some(mut builder) = current_demo.take() {
        flush_step(&mut current_step, &mut builder.steps);
        match builder.build() {
            Ok(demo) => demos.push(demo),
            Err(errs) => errors.extend(errs),
        }
    }

    if demos.is_empty() && errors.is_empty() {
        errors.push(DemoParseError::NoDemos);
    }

    if errors.is_empty() {
        Ok(demos)
    } else {
        Err(errors)
    }
}

fn unquote(s: &str) -> String {
    s.trim_matches('"').trim_matches('\'').to_string()
}

fn flush_step(current_step: &mut Option<StepBuilder>, steps: &mut Vec<DemoStep>) {
    if let Some(step) = current_step.take()
        && let Some(built) = step.build()
    {
        steps.push(built);
    }
}

// ============================================================================
// Builders
// ============================================================================

struct DemoBuilder {
    demo_id: String,
    title: Option<String>,
    claim: Option<String>,
    timeout_seconds: Option<u32>,
    terminal_width: Option<u16>,
    terminal_height: Option<u16>,
    tags: Vec<String>,
    steps: Vec<DemoStep>,
}

impl DemoBuilder {
    fn new(demo_id: String) -> Self {
        Self {
            demo_id,
            title: None,
            claim: None,
            timeout_seconds: None,
            terminal_width: None,
            terminal_height: None,
            tags: Vec::new(),
            steps: Vec::new(),
        }
    }

    fn build(self) -> Result<DemoDefinition, Vec<DemoParseError>> {
        let mut errors = Vec::new();
        let id = &self.demo_id;

        if self.title.is_none() {
            errors.push(DemoParseError::MissingField {
                demo_id: id.clone(),
                field: "title".into(),
            });
        }
        if self.claim.is_none() {
            errors.push(DemoParseError::MissingField {
                demo_id: id.clone(),
                field: "claim".into(),
            });
        }
        if self.timeout_seconds.is_none() {
            errors.push(DemoParseError::MissingField {
                demo_id: id.clone(),
                field: "timeout_seconds".into(),
            });
        }
        if let Some(t) = self.timeout_seconds
            && t > 60
        {
            errors.push(DemoParseError::InvalidValue {
                demo_id: id.clone(),
                field: "timeout_seconds".into(),
                reason: format!("{t} exceeds 60-second limit"),
            });
        }
        if self.terminal_width.is_none() || self.terminal_height.is_none() {
            errors.push(DemoParseError::MissingField {
                demo_id: id.clone(),
                field: "terminal_size".into(),
            });
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(DemoDefinition {
            demo_id: self.demo_id,
            title: self.title.unwrap(),
            claim: self.claim.unwrap(),
            timeout_seconds: self.timeout_seconds.unwrap(),
            terminal_width: self.terminal_width.unwrap(),
            terminal_height: self.terminal_height.unwrap(),
            tags: self.tags,
            steps: self.steps,
        })
    }
}

struct StepBuilder {
    step_type: String,
    widget: Option<String>,
    description: Option<String>,
    level: Option<String>,
    signal: Option<String>,
    seed: Option<u64>,
    metric: Option<String>,
    max_us: Option<u64>,
    to_width: Option<u16>,
    to_height: Option<u16>,
    contains: Vec<String>,
}

impl StepBuilder {
    fn new(step_type: &str) -> Self {
        Self {
            step_type: step_type.to_string(),
            widget: None,
            description: None,
            level: None,
            signal: None,
            seed: None,
            metric: None,
            max_us: None,
            to_width: None,
            to_height: None,
            contains: Vec::new(),
        }
    }

    fn build(self) -> Option<DemoStep> {
        let desc = self.description.unwrap_or_default();
        match self.step_type.as_str() {
            "render" => Some(DemoStep::Render {
                widget: self.widget.unwrap_or_default(),
                description: desc,
                level: self.level,
                signal: self.signal,
                seed: self.seed,
            }),
            "resize" => Some(DemoStep::Resize {
                width: self.to_width.unwrap_or(80),
                height: self.to_height.unwrap_or(24),
                description: desc,
            }),
            "assert_checksum" => Some(DemoStep::AssertChecksum { description: desc }),
            "assert_content" => Some(DemoStep::AssertContent {
                contains: self.contains,
                description: desc,
            }),
            "measure_timing" => Some(DemoStep::MeasureTiming {
                metric: self.metric.unwrap_or_default(),
                max_us: self.max_us,
                description: desc,
            }),
            _ => None,
        }
    }
}

// ============================================================================
// Validation
// ============================================================================

/// Validate demo definitions for consistency.
pub fn validate_demos(demos: &[DemoDefinition]) -> Vec<DemoParseError> {
    let mut errors = Vec::new();

    for demo in demos {
        if demo.steps.is_empty() {
            errors.push(DemoParseError::MissingField {
                demo_id: demo.demo_id.clone(),
                field: "steps".into(),
            });
        }

        if demo.terminal_width == 0 || demo.terminal_height == 0 {
            errors.push(DemoParseError::InvalidValue {
                demo_id: demo.demo_id.clone(),
                field: "terminal_size".into(),
                reason: "width and height must be > 0".into(),
            });
        }

        // Check that render steps have widget names
        for (i, step) in demo.steps.iter().enumerate() {
            if let DemoStep::Render { widget, .. } = step
                && widget.is_empty()
            {
                errors.push(DemoParseError::MissingField {
                    demo_id: demo.demo_id.clone(),
                    field: format!("steps[{i}].widget"),
                });
            }
        }
    }

    errors
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_YAML: &str = r#"
demos:
  - demo_id: test_demo
    title: "Test"
    claim: "It works"
    timeout_seconds: 5
    terminal_size: [80, 24]
    tags: [test]
    steps:
      - type: render
        widget: block
        description: "Render a block"
"#;

    #[test]
    fn parse_minimal_demo() {
        let demos = parse_demo_yaml(MINIMAL_YAML).unwrap();
        assert_eq!(demos.len(), 1);
        assert_eq!(demos[0].demo_id, "test_demo");
        assert_eq!(demos[0].title, "Test");
        assert_eq!(demos[0].claim, "It works");
        assert_eq!(demos[0].timeout_seconds, 5);
        assert_eq!(demos[0].terminal_width, 80);
        assert_eq!(demos[0].terminal_height, 24);
        assert_eq!(demos[0].tags, vec!["test"]);
        assert_eq!(demos[0].steps.len(), 1);
    }

    #[test]
    fn parse_multiple_demos() {
        let yaml = r#"
demos:
  - demo_id: a
    title: "A"
    claim: "Claim A"
    timeout_seconds: 10
    terminal_size: [120, 40]
    tags: [x]
    steps:
      - type: render
        widget: block
        description: "block"
  - demo_id: b
    title: "B"
    claim: "Claim B"
    timeout_seconds: 15
    terminal_size: [80, 24]
    tags: [y]
    steps:
      - type: assert_checksum
        description: "check"
"#;
        let demos = parse_demo_yaml(yaml).unwrap();
        assert_eq!(demos.len(), 2);
        assert_eq!(demos[0].demo_id, "a");
        assert_eq!(demos[1].demo_id, "b");
    }

    #[test]
    fn parse_all_step_types() {
        let yaml = r#"
demos:
  - demo_id: steps
    title: "Steps"
    claim: "All step types"
    timeout_seconds: 10
    terminal_size: [80, 24]
    tags: [test]
    steps:
      - type: render
        widget: block
        level: full_bayesian
        signal: red
        seed: 42
        description: "render"
      - type: resize
        to: [120, 40]
        description: "resize"
      - type: assert_checksum
        description: "checksum"
      - type: assert_content
        contains: ["hello", "world"]
        description: "content"
      - type: measure_timing
        metric: render_frame_us
        max_us: 4000
        description: "timing"
"#;
        let demos = parse_demo_yaml(yaml).unwrap();
        let steps = &demos[0].steps;
        assert_eq!(steps.len(), 5);

        assert!(matches!(&steps[0], DemoStep::Render { widget, seed, .. }
            if widget == "block" && *seed == Some(42)));
        assert!(matches!(
            &steps[1],
            DemoStep::Resize {
                width: 120,
                height: 40,
                ..
            }
        ));
        assert!(matches!(&steps[2], DemoStep::AssertChecksum { .. }));
        assert!(matches!(&steps[3], DemoStep::AssertContent { contains, .. }
            if contains == &["hello", "world"]));
        assert!(
            matches!(&steps[4], DemoStep::MeasureTiming { metric, max_us, .. }
            if metric == "render_frame_us" && *max_us == Some(4000))
        );
    }

    #[test]
    fn reject_duplicate_ids() {
        let yaml = r#"
demos:
  - demo_id: dup
    title: "A"
    claim: "A"
    timeout_seconds: 5
    terminal_size: [80, 24]
    tags: [x]
    steps:
      - type: render
        widget: block
        description: "r"
  - demo_id: dup
    title: "B"
    claim: "B"
    timeout_seconds: 5
    terminal_size: [80, 24]
    tags: [x]
    steps:
      - type: render
        widget: block
        description: "r"
"#;
        let errors = parse_demo_yaml(yaml).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, DemoParseError::DuplicateId(id) if id == "dup"))
        );
    }

    #[test]
    fn reject_timeout_over_60() {
        let yaml = r#"
demos:
  - demo_id: slow
    title: "Slow"
    claim: "Too slow"
    timeout_seconds: 90
    terminal_size: [80, 24]
    tags: [x]
    steps:
      - type: render
        widget: block
        description: "r"
"#;
        let errors = parse_demo_yaml(yaml).unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            DemoParseError::InvalidValue { field, .. } if field == "timeout_seconds"
        )));
    }

    #[test]
    fn reject_missing_title() {
        let yaml = r#"
demos:
  - demo_id: notitle
    claim: "C"
    timeout_seconds: 5
    terminal_size: [80, 24]
    tags: [x]
    steps:
      - type: render
        widget: block
        description: "r"
"#;
        let errors = parse_demo_yaml(yaml).unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            DemoParseError::MissingField { field, .. } if field == "title"
        )));
    }

    #[test]
    fn reject_empty_yaml() {
        let errors = parse_demo_yaml("").unwrap_err();
        assert!(errors.iter().any(|e| matches!(e, DemoParseError::NoDemos)));
    }

    #[test]
    fn validate_empty_steps() {
        let demo = DemoDefinition {
            demo_id: "empty".into(),
            title: "E".into(),
            claim: "C".into(),
            timeout_seconds: 5,
            terminal_width: 80,
            terminal_height: 24,
            tags: vec![],
            steps: vec![],
        };
        let errors = validate_demos(&[demo]);
        assert!(errors.iter().any(|e| matches!(
            e,
            DemoParseError::MissingField { field, .. } if field == "steps"
        )));
    }

    #[test]
    fn validate_zero_terminal_size() {
        let demo = DemoDefinition {
            demo_id: "zero".into(),
            title: "Z".into(),
            claim: "C".into(),
            timeout_seconds: 5,
            terminal_width: 0,
            terminal_height: 24,
            tags: vec![],
            steps: vec![DemoStep::Render {
                widget: "block".into(),
                description: "r".into(),
                level: None,
                signal: None,
                seed: None,
            }],
        };
        let errors = validate_demos(&[demo]);
        assert!(errors.iter().any(|e| matches!(
            e,
            DemoParseError::InvalidValue { field, .. } if field == "terminal_size"
        )));
    }

    #[test]
    fn error_display() {
        let err = DemoParseError::MissingField {
            demo_id: "test".into(),
            field: "title".into(),
        };
        assert!(err.to_string().contains("title"));
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let yaml = r#"
# This is a comment
demos:
  # Demo comment
  - demo_id: commented
    title: "C"
    claim: "C"
    timeout_seconds: 5
    terminal_size: [80, 24]

    tags: [x]
    steps:
      - type: render
        widget: block
        description: "r"
"#;
        let demos = parse_demo_yaml(yaml).unwrap();
        assert_eq!(demos.len(), 1);
    }
}
