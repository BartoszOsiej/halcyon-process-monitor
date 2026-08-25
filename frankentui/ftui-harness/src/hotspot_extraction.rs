#![forbid(unsafe_code)]

//! Hotspot extraction and profiling infrastructure (bd-p8i4s).
//!
//! Converts raw profiler output into a stable, comparable hotspot table
//! that later optimization beads consume for ranking and decision-making.
//!
//! # Profiling modes
//!
//! | Mode | What it measures | When to use |
//! |------|-----------------|-------------|
//! | CPU | Instruction-level time | Render inner loops, diff algorithms |
//! | Allocation | Heap churn, allocation count/size | Text/layout, widget creation |
//! | Syscall | Kernel transitions, I/O waits | Presenter output, file evidence writes |
//! | Workflow | Stage-level wall-clock timing | Doctor capture, seed orchestration |
//!
//! # Hotspot table schema
//!
//! Each hotspot is a normalized record with:
//! - **Location**: crate, module, function, line
//! - **Category**: which resource class dominates
//! - **Contribution**: percentage of total in that category
//! - **Confidence**: whether the finding is stable across runs
//! - **Linkage**: fixture ID and baseline ID for traceability
//!
//! # Usage
//!
//! ```ignore
//! use ftui_harness::hotspot_extraction::*;
//!
//! let mut table = HotspotTable::new("render_80x24", "baseline-001");
//! table.add(Hotspot::new("ftui_render::diff::compute", ProfilingMode::Cpu)
//!     .contribution_pct(34.2)
//!     .location("ftui-render", "src/diff.rs", 142));
//! table.add(Hotspot::new("ftui_text::wrap::reflow", ProfilingMode::Allocation)
//!     .contribution_pct(18.5)
//!     .location("ftui-text", "src/wrap.rs", 87));
//!
//! let ranked = table.ranked();
//! ```

use std::collections::HashSet;

/// Profiling mode / resource class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfilingMode {
    /// CPU instruction-level time (flamegraph, perf, samply).
    Cpu,
    /// Heap allocation count and size (DHAT, heaptrack).
    Allocation,
    /// Kernel syscall transitions and I/O waits (strace, perf-syscall).
    Syscall,
    /// Stage-level wall-clock timing (tracing spans, custom timers).
    Workflow,
}

impl ProfilingMode {
    /// Human-readable label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Allocation => "allocation",
            Self::Syscall => "syscall",
            Self::Workflow => "workflow",
        }
    }

    /// Typical profiling tool for this mode.
    #[must_use]
    pub const fn typical_tool(&self) -> &'static str {
        match self {
            Self::Cpu => "perf/samply/flamegraph",
            Self::Allocation => "DHAT/heaptrack",
            Self::Syscall => "strace/perf-syscall",
            Self::Workflow => "tracing spans",
        }
    }

    /// All profiling modes.
    pub const ALL: &'static [ProfilingMode] =
        &[Self::Cpu, Self::Allocation, Self::Syscall, Self::Workflow];
}

/// Confidence level for a hotspot finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HotspotConfidence {
    /// Reproduced across multiple runs with low variance.
    High,
    /// Appears in most runs but with moderate variance.
    Medium,
    /// Appeared in some runs or has high variance — may be noise.
    Low,
}

/// A single hotspot extracted from profiling data.
#[derive(Debug, Clone)]
pub struct Hotspot {
    /// Fully qualified function/symbol name.
    pub symbol: String,
    /// Which resource class this hotspot dominates.
    pub mode: ProfilingMode,
    /// Percentage contribution within its profiling mode.
    pub contribution_pct: f64,
    /// Confidence that this is a real bottleneck.
    pub confidence: HotspotConfidence,
    /// Crate containing this hotspot.
    pub crate_name: String,
    /// Source file path (relative to crate root).
    pub file: String,
    /// Source line number (0 = unknown).
    pub line: u32,
    /// Whether this is likely a measurement artifact rather than genuine EV.
    pub likely_artifact: bool,
    /// Human-readable note about why this hotspot matters or doesn't.
    pub note: String,
}

impl Hotspot {
    /// Create a new hotspot with minimal required fields.
    #[must_use]
    pub fn new(symbol: &str, mode: ProfilingMode) -> Self {
        Self {
            symbol: symbol.to_string(),
            mode,
            contribution_pct: 0.0,
            confidence: HotspotConfidence::Medium,
            crate_name: String::new(),
            file: String::new(),
            line: 0,
            likely_artifact: false,
            note: String::new(),
        }
    }

    /// Set the contribution percentage.
    #[must_use]
    pub fn contribution_pct(mut self, pct: f64) -> Self {
        self.contribution_pct = pct;
        self
    }

    /// Set the source location.
    #[must_use]
    pub fn location(mut self, crate_name: &str, file: &str, line: u32) -> Self {
        self.crate_name = crate_name.to_string();
        self.file = file.to_string();
        self.line = line;
        self
    }

    /// Set confidence level.
    #[must_use]
    pub fn confidence(mut self, confidence: HotspotConfidence) -> Self {
        self.confidence = confidence;
        self
    }

    /// Mark as likely a measurement artifact.
    #[must_use]
    pub fn artifact(mut self) -> Self {
        self.likely_artifact = true;
        self
    }

    /// Add an explanatory note.
    #[must_use]
    pub fn note(mut self, note: &str) -> Self {
        self.note = note.to_string();
        self
    }

    /// Whether this hotspot is actionable (high confidence, not an artifact,
    /// contribution > 5%).
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        !self.likely_artifact
            && self.confidence != HotspotConfidence::Low
            && self.contribution_pct >= 5.0
    }
}

/// A normalized hotspot table for one fixture/baseline pair.
#[derive(Debug, Clone)]
pub struct HotspotTable {
    /// Fixture ID this table was profiled against.
    pub fixture_id: String,
    /// Baseline ID for traceability.
    pub baseline_id: String,
    /// All extracted hotspots.
    pub hotspots: Vec<Hotspot>,
}

impl HotspotTable {
    /// Create a new empty hotspot table.
    #[must_use]
    pub fn new(fixture_id: &str, baseline_id: &str) -> Self {
        Self {
            fixture_id: fixture_id.to_string(),
            baseline_id: baseline_id.to_string(),
            hotspots: Vec::new(),
        }
    }

    /// Add a hotspot to the table.
    pub fn add(&mut self, hotspot: Hotspot) {
        self.hotspots.push(hotspot);
    }

    /// Return hotspots ranked by contribution percentage (descending).
    #[must_use]
    pub fn ranked(&self) -> Vec<&Hotspot> {
        let mut sorted: Vec<&Hotspot> = self.hotspots.iter().collect();
        sorted.sort_by(|a, b| {
            b.contribution_pct
                .partial_cmp(&a.contribution_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted
    }

    /// Return only actionable hotspots (high confidence, not artifacts, >= 5%).
    #[must_use]
    pub fn actionable(&self) -> Vec<&Hotspot> {
        self.ranked()
            .into_iter()
            .filter(|h| h.is_actionable())
            .collect()
    }

    /// Return hotspots filtered by profiling mode.
    #[must_use]
    pub fn by_mode(&self, mode: ProfilingMode) -> Vec<&Hotspot> {
        self.hotspots.iter().filter(|h| h.mode == mode).collect()
    }

    /// Total contribution percentage across all hotspots for a given mode.
    /// Should not exceed 100% for well-formed data.
    #[must_use]
    pub fn total_contribution(&self, mode: ProfilingMode) -> f64 {
        self.hotspots
            .iter()
            .filter(|h| h.mode == mode)
            .map(|h| h.contribution_pct)
            .sum()
    }

    /// Profiling modes that have at least one hotspot.
    #[must_use]
    pub fn covered_modes(&self) -> HashSet<ProfilingMode> {
        self.hotspots.iter().map(|h| h.mode).collect()
    }

    /// Serialize to JSON for artifact storage.
    #[must_use]
    pub fn to_json(&self) -> String {
        let entries: Vec<String> = self
            .hotspots
            .iter()
            .map(|h| {
                format!(
                    r#"    {{
      "symbol": "{}",
      "mode": "{}",
      "contribution_pct": {:.1},
      "confidence": "{}",
      "crate": "{}",
      "file": "{}",
      "line": {},
      "likely_artifact": {},
      "actionable": {}
    }}"#,
                    h.symbol,
                    h.mode.label(),
                    h.contribution_pct,
                    match h.confidence {
                        HotspotConfidence::High => "high",
                        HotspotConfidence::Medium => "medium",
                        HotspotConfidence::Low => "low",
                    },
                    h.crate_name,
                    h.file,
                    h.line,
                    h.likely_artifact,
                    h.is_actionable(),
                )
            })
            .collect();

        format!(
            r#"{{
  "fixture_id": "{}",
  "baseline_id": "{}",
  "hotspot_count": {},
  "actionable_count": {},
  "covered_modes": [{}],
  "hotspots": [
{}
  ]
}}"#,
            self.fixture_id,
            self.baseline_id,
            self.hotspots.len(),
            self.actionable().len(),
            self.covered_modes()
                .iter()
                .map(|m| format!("\"{}\"", m.label()))
                .collect::<Vec<_>>()
                .join(", "),
            entries.join(",\n"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiling_modes_all_have_labels() {
        for mode in ProfilingMode::ALL {
            assert!(!mode.label().is_empty());
            assert!(!mode.typical_tool().is_empty());
        }
    }

    #[test]
    fn hotspot_builder() {
        let h = Hotspot::new("ftui_render::diff::compute", ProfilingMode::Cpu)
            .contribution_pct(34.2)
            .location("ftui-render", "src/diff.rs", 142)
            .confidence(HotspotConfidence::High)
            .note("Inner loop dominates");

        assert_eq!(h.symbol, "ftui_render::diff::compute");
        assert_eq!(h.mode, ProfilingMode::Cpu);
        assert!((h.contribution_pct - 34.2).abs() < 0.01);
        assert_eq!(h.crate_name, "ftui-render");
        assert_eq!(h.file, "src/diff.rs");
        assert_eq!(h.line, 142);
        assert_eq!(h.confidence, HotspotConfidence::High);
        assert!(!h.likely_artifact);
        assert!(h.is_actionable());
    }

    #[test]
    fn hotspot_actionable_criteria() {
        // Actionable: high confidence, not artifact, >= 5%
        let actionable = Hotspot::new("f", ProfilingMode::Cpu)
            .contribution_pct(10.0)
            .confidence(HotspotConfidence::High);
        assert!(actionable.is_actionable());

        // Not actionable: low confidence
        let low_conf = Hotspot::new("f", ProfilingMode::Cpu)
            .contribution_pct(10.0)
            .confidence(HotspotConfidence::Low);
        assert!(!low_conf.is_actionable());

        // Not actionable: artifact
        let artifact = Hotspot::new("f", ProfilingMode::Cpu)
            .contribution_pct(10.0)
            .artifact();
        assert!(!artifact.is_actionable());

        // Not actionable: too small contribution
        let small = Hotspot::new("f", ProfilingMode::Cpu)
            .contribution_pct(2.0)
            .confidence(HotspotConfidence::High);
        assert!(!small.is_actionable());
    }

    #[test]
    fn hotspot_table_ranking() {
        let mut table = HotspotTable::new("fixture-1", "baseline-001");
        table.add(Hotspot::new("a", ProfilingMode::Cpu).contribution_pct(10.0));
        table.add(Hotspot::new("b", ProfilingMode::Cpu).contribution_pct(30.0));
        table.add(Hotspot::new("c", ProfilingMode::Cpu).contribution_pct(20.0));

        let ranked = table.ranked();
        assert_eq!(ranked[0].symbol, "b"); // 30%
        assert_eq!(ranked[1].symbol, "c"); // 20%
        assert_eq!(ranked[2].symbol, "a"); // 10%
    }

    #[test]
    fn hotspot_table_by_mode() {
        let mut table = HotspotTable::new("f", "b");
        table.add(Hotspot::new("cpu1", ProfilingMode::Cpu).contribution_pct(30.0));
        table.add(Hotspot::new("alloc1", ProfilingMode::Allocation).contribution_pct(20.0));
        table.add(Hotspot::new("cpu2", ProfilingMode::Cpu).contribution_pct(15.0));

        let cpu = table.by_mode(ProfilingMode::Cpu);
        assert_eq!(cpu.len(), 2);

        let alloc = table.by_mode(ProfilingMode::Allocation);
        assert_eq!(alloc.len(), 1);
    }

    #[test]
    fn hotspot_table_total_contribution() {
        let mut table = HotspotTable::new("f", "b");
        table.add(Hotspot::new("a", ProfilingMode::Cpu).contribution_pct(30.0));
        table.add(Hotspot::new("b", ProfilingMode::Cpu).contribution_pct(25.0));
        table.add(Hotspot::new("c", ProfilingMode::Allocation).contribution_pct(10.0));

        let cpu_total = table.total_contribution(ProfilingMode::Cpu);
        assert!((cpu_total - 55.0).abs() < 0.01);

        let alloc_total = table.total_contribution(ProfilingMode::Allocation);
        assert!((alloc_total - 10.0).abs() < 0.01);
    }

    #[test]
    fn hotspot_table_covered_modes() {
        let mut table = HotspotTable::new("f", "b");
        table.add(Hotspot::new("a", ProfilingMode::Cpu).contribution_pct(30.0));
        table.add(Hotspot::new("b", ProfilingMode::Syscall).contribution_pct(10.0));

        let modes = table.covered_modes();
        assert!(modes.contains(&ProfilingMode::Cpu));
        assert!(modes.contains(&ProfilingMode::Syscall));
        assert!(!modes.contains(&ProfilingMode::Allocation));
    }

    #[test]
    fn hotspot_table_actionable_filters() {
        let mut table = HotspotTable::new("f", "b");
        table.add(
            Hotspot::new("real", ProfilingMode::Cpu)
                .contribution_pct(25.0)
                .confidence(HotspotConfidence::High),
        );
        table.add(
            Hotspot::new("noise", ProfilingMode::Cpu)
                .contribution_pct(3.0)
                .confidence(HotspotConfidence::Low),
        );
        table.add(
            Hotspot::new("artifact", ProfilingMode::Cpu)
                .contribution_pct(15.0)
                .artifact(),
        );

        let actionable = table.actionable();
        assert_eq!(actionable.len(), 1);
        assert_eq!(actionable[0].symbol, "real");
    }

    #[test]
    fn hotspot_table_to_json() {
        let mut table = HotspotTable::new("render_80x24", "baseline-001");
        table.add(
            Hotspot::new("ftui_render::diff::compute", ProfilingMode::Cpu)
                .contribution_pct(34.2)
                .location("ftui-render", "src/diff.rs", 142)
                .confidence(HotspotConfidence::High),
        );

        let json = table.to_json();
        assert!(json.contains("\"fixture_id\": \"render_80x24\""));
        assert!(json.contains("\"baseline_id\": \"baseline-001\""));
        assert!(json.contains("\"hotspot_count\": 1"));
        assert!(json.contains("\"symbol\": \"ftui_render::diff::compute\""));
        assert!(json.contains("\"contribution_pct\": 34.2"));
        assert!(json.contains("\"confidence\": \"high\""));
    }

    #[test]
    fn empty_table() {
        let table = HotspotTable::new("f", "b");
        assert!(table.ranked().is_empty());
        assert!(table.actionable().is_empty());
        assert!(table.covered_modes().is_empty());
        assert!((table.total_contribution(ProfilingMode::Cpu)).abs() < 0.01);
    }

    #[test]
    fn confidence_ordering() {
        assert!(HotspotConfidence::High < HotspotConfidence::Medium);
        assert!(HotspotConfidence::Medium < HotspotConfidence::Low);
    }

    #[test]
    fn all_profiling_modes_count() {
        assert_eq!(ProfilingMode::ALL.len(), 4);
    }
}
