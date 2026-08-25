#![forbid(unsafe_code)]

//! Doctor workflow cost profiling (bd-qbbkv).
//!
//! Maps workflow-level cost centers in `doctor_frankentui` across capture,
//! replay, report, and failure triage paths. Produces structured evidence
//! for downstream optimization beads (bd-tsgb4, bd-mn4a0, bd-vb2b7).
//!
//! # Cost model
//!
//! Doctor workflows decompose into five cost lanes:
//!
//! | Lane | Dominant cost | Example |
//! |------|--------------|---------|
//! | **Subprocess** | Wall-clock blocking on VHS/docker/tmux | Capture recording |
//! | **Network** | RPC latency, retry backoff, readiness polling | Seed bootstrap |
//! | **FileIO** | Artifact writes, directory creation, log appends | Evidence ledger |
//! | **Computation** | Report generation, manifest building, comparison | Suite aggregation |
//! | **Orchestration** | Scheduling, timeout enforcement, fallback logic | Doctor certification |
//!
//! # Workflow stages
//!
//! ```text
//! seed-demo → capture → suite → report → triage
//!    │            │         │        │         │
//!    └─ RPC       └─ VHS   └─ fan   └─ HTML  └─ failure
//!       retry       docker    out      JSON     signature
//!       probe       tmux     recur    manifest  remediation
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use ftui_harness::doctor_cost_profile::*;
//!
//! let mut profile = DoctorCostProfile::new();
//! profile.record(CostEntry::new(WorkflowStage::Capture, CostLane::Subprocess)
//!     .operation("vhs_host_record")
//!     .wall_clock_ms(45000)
//!     .blocking(true)
//!     .rationale("VHS recording is the dominant cost in capture workflow"));
//!
//! let report = profile.finalize();
//! println!("{}", report.to_json());
//! ```

// ============================================================================
// Workflow Stages
// ============================================================================

/// Top-level doctor workflow stages, ordered by execution sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WorkflowStage {
    /// MCP bootstrap: readiness polling, agent registration, message seeding.
    Seed,
    /// Capture recording: VHS/docker subprocess, tape generation, snapshot.
    Capture,
    /// Suite fan-out: per-profile orchestration with recursive doctor calls.
    Suite,
    /// Report generation: HTML/JSON synthesis, artifact linking.
    Report,
    /// Failure triage: signature matching, remediation hints, replay helpers.
    Triage,
}

impl WorkflowStage {
    /// Human-readable label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Capture => "capture",
            Self::Suite => "suite",
            Self::Report => "report",
            Self::Triage => "triage",
        }
    }

    /// All stages in execution order.
    pub const ALL: &'static [Self] = &[
        Self::Seed,
        Self::Capture,
        Self::Suite,
        Self::Report,
        Self::Triage,
    ];
}

// ============================================================================
// Cost Lanes
// ============================================================================

/// Resource class that dominates the cost of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CostLane {
    /// Blocking on child process (VHS, docker, tmux, ffmpeg, ffprobe).
    Subprocess,
    /// Network I/O: HTTP/RPC calls, readiness polling, retry backoff.
    Network,
    /// File system: artifact writes, directory creation, log appends.
    FileIo,
    /// CPU-bound: report synthesis, manifest building, comparison ops.
    Computation,
    /// Scheduling overhead: timeout tracking, fallback decisions, fan-out.
    Orchestration,
}

impl CostLane {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Subprocess => "subprocess",
            Self::Network => "network",
            Self::FileIo => "file_io",
            Self::Computation => "computation",
            Self::Orchestration => "orchestration",
        }
    }

    /// All lanes.
    pub const ALL: &'static [Self] = &[
        Self::Subprocess,
        Self::Network,
        Self::FileIo,
        Self::Computation,
        Self::Orchestration,
    ];
}

// ============================================================================
// Optimization Impact
// ============================================================================

/// Expected optimization impact level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptimizationImpact {
    /// Tail spike only: p99 improvement, no median change.
    TailOnly,
    /// Moderate: measurable improvement in p50-p95.
    Moderate,
    /// High: dominant cost center, improvement directly visible to operators.
    High,
    /// Critical: blocking the workflow, improvement unlocks new use cases.
    Critical,
}

impl OptimizationImpact {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::TailOnly => "tail_only",
            Self::Moderate => "moderate",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

// ============================================================================
// Cost Entry
// ============================================================================

/// A single cost observation for a doctor workflow operation.
#[derive(Debug, Clone)]
pub struct CostEntry {
    /// Which workflow stage this cost belongs to.
    pub stage: WorkflowStage,
    /// Dominant resource class.
    pub lane: CostLane,
    /// Specific operation name (e.g., "vhs_host_record", "rpc_send_message").
    pub operation: String,
    /// Wall-clock time in milliseconds.
    pub wall_clock_ms: u64,
    /// Whether this operation blocks the workflow pipeline.
    pub blocking: bool,
    /// Whether this is essential evidence work or potentially redundant.
    pub essential: bool,
    /// Expected optimization impact.
    pub impact: OptimizationImpact,
    /// Why this cost matters or doesn't — ties to operator stories.
    pub rationale: String,
    /// Subprocess involved, if any (e.g., "vhs", "docker", "ffmpeg").
    pub subprocess: Option<String>,
    /// Estimated fraction of total stage time (0.0-1.0).
    pub stage_fraction: f64,
}

impl CostEntry {
    /// Create a new cost entry with defaults.
    #[must_use]
    pub fn new(stage: WorkflowStage, lane: CostLane) -> Self {
        Self {
            stage,
            lane,
            operation: String::new(),
            wall_clock_ms: 0,
            blocking: false,
            essential: true,
            impact: OptimizationImpact::Moderate,
            rationale: String::new(),
            subprocess: None,
            stage_fraction: 0.0,
        }
    }

    #[must_use]
    pub fn operation(mut self, op: &str) -> Self {
        self.operation = op.to_string();
        self
    }

    #[must_use]
    pub fn wall_clock_ms(mut self, ms: u64) -> Self {
        self.wall_clock_ms = ms;
        self
    }

    #[must_use]
    pub fn blocking(mut self, b: bool) -> Self {
        self.blocking = b;
        self
    }

    #[must_use]
    pub fn essential(mut self, e: bool) -> Self {
        self.essential = e;
        self
    }

    #[must_use]
    pub fn impact(mut self, i: OptimizationImpact) -> Self {
        self.impact = i;
        self
    }

    #[must_use]
    pub fn rationale(mut self, r: &str) -> Self {
        self.rationale = r.to_string();
        self
    }

    #[must_use]
    pub fn subprocess(mut self, s: &str) -> Self {
        self.subprocess = Some(s.to_string());
        self
    }

    #[must_use]
    pub fn stage_fraction(mut self, f: f64) -> Self {
        self.stage_fraction = f;
        self
    }
}

// ============================================================================
// Cost Profile
// ============================================================================

/// Builder for accumulating doctor workflow cost observations.
pub struct DoctorCostProfile {
    entries: Vec<CostEntry>,
}

impl DoctorCostProfile {
    /// Create an empty cost profile.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Record a cost observation.
    pub fn record(&mut self, entry: CostEntry) {
        self.entries.push(entry);
    }

    /// Populate the canonical doctor workflow cost model.
    ///
    /// This encodes the known cost structure of `doctor_frankentui` based on
    /// codebase analysis. Values represent typical timing from CI/headless runs.
    #[must_use]
    pub fn canonical() -> Self {
        let mut p = Self::new();

        // ================================================================
        // SEED STAGE
        // ================================================================

        p.record(
            CostEntry::new(WorkflowStage::Seed, CostLane::Network)
                .operation("server_readiness_poll")
                .wall_clock_ms(3000)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::Moderate)
                .stage_fraction(0.30)
                .rationale(
                    "Readiness polling blocks seed start. Operators wait for MCP server \
                     health before any useful work begins.",
                ),
        );

        p.record(
            CostEntry::new(WorkflowStage::Seed, CostLane::Network)
                .operation("rpc_ensure_project")
                .wall_clock_ms(500)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.05)
                .rationale("Single RPC; fast on healthy server."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Seed, CostLane::Network)
                .operation("rpc_register_agent")
                .wall_clock_ms(500)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.05)
                .rationale("Single RPC; fast on healthy server."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Seed, CostLane::Network)
                .operation("rpc_send_messages")
                .wall_clock_ms(2000)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::Moderate)
                .stage_fraction(0.20)
                .rationale(
                    "Multiple message sends for demo content. Sequential RPC calls \
                     with per-call timeout. Parallelization possible.",
                ),
        );

        p.record(
            CostEntry::new(WorkflowStage::Seed, CostLane::Network)
                .operation("rpc_file_reservations")
                .wall_clock_ms(500)
                .blocking(true)
                .essential(false)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.05)
                .rationale("Advisory reservations; skip in --fast mode."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Seed, CostLane::Network)
                .operation("rpc_fetch_inbox")
                .wall_clock_ms(500)
                .blocking(true)
                .essential(false)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.05)
                .rationale("Verification step; skip in --fast mode."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Seed, CostLane::Orchestration)
                .operation("retry_backoff_waits")
                .wall_clock_ms(3000)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::High)
                .stage_fraction(0.30)
                .rationale(
                    "Exponential backoff waits dominate seed time when server is slow. \
                     Tail spikes matter: a single retry failure can add 10s+.",
                ),
        );

        // ================================================================
        // CAPTURE STAGE (dominant cost center)
        // ================================================================

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::Subprocess)
                .operation("vhs_host_record")
                .wall_clock_ms(45000)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::Critical)
                .subprocess("vhs")
                .stage_fraction(0.75)
                .rationale(
                    "VHS recording is the single largest cost in doctor. The 300s \
                     default timeout means even healthy runs take 30-60s. Operators \
                     experience this as the primary wait.",
                ),
        );

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::Subprocess)
                .operation("vhs_docker_fallback")
                .wall_clock_ms(90000)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::Critical)
                .subprocess("docker")
                .stage_fraction(0.0) // only when host fails
                .rationale(
                    "Docker fallback adds container startup overhead (15-30s) on top \
                     of VHS time. Only triggered when host VHS fails.",
                ),
        );

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::Subprocess)
                .operation("snapshot_extraction")
                .wall_clock_ms(2000)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .subprocess("ffmpeg")
                .stage_fraction(0.03)
                .rationale("Single ffmpeg frame extraction; fast when video exists."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::Subprocess)
                .operation("video_metadata_query")
                .wall_clock_ms(500)
                .blocking(true)
                .essential(false)
                .impact(OptimizationImpact::TailOnly)
                .subprocess("ffprobe")
                .stage_fraction(0.01)
                .rationale("Duration metadata; could be deferred or cached."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::Orchestration)
                .operation("timeout_poll_loop")
                .wall_clock_ms(100)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.01)
                .rationale("Polling overhead is negligible in healthy runs."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::FileIo)
                .operation("tape_generation")
                .wall_clock_ms(50)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.001)
                .rationale("VHS tape script is small text; negligible I/O."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::FileIo)
                .operation("log_pump_writes")
                .wall_clock_ms(200)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.003)
                .rationale("Background log pumping; low overhead."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::FileIo)
                .operation("evidence_ledger_append")
                .wall_clock_ms(100)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.002)
                .rationale("Append-only JSONL; low I/O cost per decision record."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::FileIo)
                .operation("run_meta_write")
                .wall_clock_ms(20)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.0003)
                .rationale("Single JSON write; contract-critical but fast."),
        );

        // ================================================================
        // SUITE STAGE
        // ================================================================

        p.record(
            CostEntry::new(WorkflowStage::Suite, CostLane::Subprocess)
                .operation("per_profile_doctor_invocation")
                .wall_clock_ms(60000)
                .blocking(true)
                .essential(true)
                .impact(OptimizationImpact::High)
                .stage_fraction(0.90)
                .rationale(
                    "Sequential per-profile doctor invocations multiply the capture cost. \
                     Parallelizing profiles would reduce wall-clock by ~N×.",
                ),
        );

        p.record(
            CostEntry::new(WorkflowStage::Suite, CostLane::FileIo)
                .operation("manifest_index_generation")
                .wall_clock_ms(100)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.002)
                .rationale("Walking run directories and writing index; fast."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Suite, CostLane::Orchestration)
                .operation("profile_fanout_scheduling")
                .wall_clock_ms(50)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.001)
                .rationale("Minimal scheduling overhead."),
        );

        // ================================================================
        // REPORT STAGE
        // ================================================================

        p.record(
            CostEntry::new(WorkflowStage::Report, CostLane::Computation)
                .operation("html_report_synthesis")
                .wall_clock_ms(500)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.50)
                .rationale("Template rendering + artifact path resolution; fast."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Report, CostLane::Computation)
                .operation("json_summary_generation")
                .wall_clock_ms(200)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.20)
                .rationale("JSON serialization of summary data; fast."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Report, CostLane::FileIo)
                .operation("artifact_directory_walk")
                .wall_clock_ms(200)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.20)
                .rationale("Scanning for run_meta.json files across runs."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Report, CostLane::FileIo)
                .operation("report_file_writes")
                .wall_clock_ms(100)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.10)
                .rationale("Writing HTML/JSON files; small."),
        );

        // ================================================================
        // TRIAGE STAGE
        // ================================================================

        p.record(
            CostEntry::new(WorkflowStage::Triage, CostLane::Computation)
                .operation("failure_signature_matching")
                .wall_clock_ms(100)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.40)
                .rationale(
                    "Pattern matching against known failure signatures; fast \
                     unless the signature catalog grows large.",
                ),
        );

        p.record(
            CostEntry::new(WorkflowStage::Triage, CostLane::Computation)
                .operation("remediation_hint_generation")
                .wall_clock_ms(50)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.20)
                .rationale("Lookup-based hint generation; negligible cost."),
        );

        p.record(
            CostEntry::new(WorkflowStage::Triage, CostLane::FileIo)
                .operation("replay_artifact_collection")
                .wall_clock_ms(100)
                .blocking(false)
                .essential(true)
                .impact(OptimizationImpact::TailOnly)
                .stage_fraction(0.40)
                .rationale("Gathering log files and evidence for replay bundle."),
        );

        p
    }

    /// Finalize the profile into a structured cost report.
    #[must_use]
    pub fn finalize(self) -> CostReport {
        let mut stage_totals: Vec<(WorkflowStage, u64)> = Vec::new();
        let mut lane_totals: Vec<(CostLane, u64)> = Vec::new();

        for stage in WorkflowStage::ALL {
            let total: u64 = self
                .entries
                .iter()
                .filter(|e| e.stage == *stage)
                .map(|e| e.wall_clock_ms)
                .sum();
            stage_totals.push((*stage, total));
        }

        for lane in CostLane::ALL {
            let total: u64 = self
                .entries
                .iter()
                .filter(|e| e.lane == *lane)
                .map(|e| e.wall_clock_ms)
                .sum();
            lane_totals.push((*lane, total));
        }

        let grand_total: u64 = self.entries.iter().map(|e| e.wall_clock_ms).sum();

        let blocking_total: u64 = self
            .entries
            .iter()
            .filter(|e| e.blocking)
            .map(|e| e.wall_clock_ms)
            .sum();

        let redundant: Vec<&CostEntry> = self.entries.iter().filter(|e| !e.essential).collect();
        let redundant_total: u64 = redundant.iter().map(|e| e.wall_clock_ms).sum();

        let mut optimization_targets: Vec<OptimizationTarget> = Vec::new();
        for entry in &self.entries {
            if entry.impact >= OptimizationImpact::Moderate {
                optimization_targets.push(OptimizationTarget {
                    stage: entry.stage,
                    lane: entry.lane,
                    operation: entry.operation.clone(),
                    impact: entry.impact,
                    wall_clock_ms: entry.wall_clock_ms,
                    blocking: entry.blocking,
                    rationale: entry.rationale.clone(),
                });
            }
        }
        optimization_targets.sort_by_key(|t| std::cmp::Reverse(t.impact));

        CostReport {
            entries: self.entries,
            stage_totals,
            lane_totals,
            grand_total_ms: grand_total,
            blocking_total_ms: blocking_total,
            redundant_total_ms: redundant_total,
            optimization_targets,
        }
    }
}

impl Default for DoctorCostProfile {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Optimization Target
// ============================================================================

/// A prioritized optimization target from the cost analysis.
#[derive(Debug, Clone)]
pub struct OptimizationTarget {
    pub stage: WorkflowStage,
    pub lane: CostLane,
    pub operation: String,
    pub impact: OptimizationImpact,
    pub wall_clock_ms: u64,
    pub blocking: bool,
    pub rationale: String,
}

// ============================================================================
// Cost Report
// ============================================================================

/// Finalized cost report with totals, breakdowns, and optimization targets.
#[derive(Debug)]
pub struct CostReport {
    /// All recorded cost entries.
    pub entries: Vec<CostEntry>,
    /// Total wall-clock per stage.
    pub stage_totals: Vec<(WorkflowStage, u64)>,
    /// Total wall-clock per cost lane.
    pub lane_totals: Vec<(CostLane, u64)>,
    /// Grand total wall-clock across all entries.
    pub grand_total_ms: u64,
    /// Total wall-clock for blocking operations only.
    pub blocking_total_ms: u64,
    /// Total wall-clock for non-essential (potentially redundant) operations.
    pub redundant_total_ms: u64,
    /// Prioritized optimization targets (moderate+ impact).
    pub optimization_targets: Vec<OptimizationTarget>,
}

impl CostReport {
    /// Entries for a specific stage.
    #[must_use]
    pub fn by_stage(&self, stage: WorkflowStage) -> Vec<&CostEntry> {
        self.entries.iter().filter(|e| e.stage == stage).collect()
    }

    /// Entries for a specific lane.
    #[must_use]
    pub fn by_lane(&self, lane: CostLane) -> Vec<&CostEntry> {
        self.entries.iter().filter(|e| e.lane == lane).collect()
    }

    /// Total for a specific stage.
    #[must_use]
    pub fn stage_total(&self, stage: WorkflowStage) -> u64 {
        self.stage_totals
            .iter()
            .find(|(s, _)| *s == stage)
            .map(|(_, t)| *t)
            .unwrap_or(0)
    }

    /// Total for a specific lane.
    #[must_use]
    pub fn lane_total(&self, lane: CostLane) -> u64 {
        self.lane_totals
            .iter()
            .find(|(l, _)| *l == lane)
            .map(|(_, t)| *t)
            .unwrap_or(0)
    }

    /// Percentage of grand total that is blocking.
    #[must_use]
    pub fn blocking_pct(&self) -> f64 {
        if self.grand_total_ms == 0 {
            return 0.0;
        }
        (self.blocking_total_ms as f64 / self.grand_total_ms as f64) * 100.0
    }

    /// Percentage of grand total that is redundant.
    #[must_use]
    pub fn redundant_pct(&self) -> f64 {
        if self.grand_total_ms == 0 {
            return 0.0;
        }
        (self.redundant_total_ms as f64 / self.grand_total_ms as f64) * 100.0
    }

    /// Serialize to structured JSON for machine consumption.
    #[must_use]
    pub fn to_json(&self) -> String {
        let stage_json: Vec<String> = self
            .stage_totals
            .iter()
            .map(|(s, t)| {
                let pct = if self.grand_total_ms > 0 {
                    (*t as f64 / self.grand_total_ms as f64) * 100.0
                } else {
                    0.0
                };
                format!(
                    r#"    {{"stage": "{}", "total_ms": {}, "pct": {:.1}}}"#,
                    s.label(),
                    t,
                    pct
                )
            })
            .collect();

        let lane_json: Vec<String> = self
            .lane_totals
            .iter()
            .map(|(l, t)| {
                let pct = if self.grand_total_ms > 0 {
                    (*t as f64 / self.grand_total_ms as f64) * 100.0
                } else {
                    0.0
                };
                format!(
                    r#"    {{"lane": "{}", "total_ms": {}, "pct": {:.1}}}"#,
                    l.label(),
                    t,
                    pct
                )
            })
            .collect();

        let target_json: Vec<String> = self
            .optimization_targets
            .iter()
            .map(|t| {
                format!(
                    r#"    {{
      "stage": "{}",
      "lane": "{}",
      "operation": "{}",
      "impact": "{}",
      "wall_clock_ms": {},
      "blocking": {},
      "rationale": "{}"
    }}"#,
                    t.stage.label(),
                    t.lane.label(),
                    t.operation,
                    t.impact.label(),
                    t.wall_clock_ms,
                    t.blocking,
                    t.rationale.replace('"', "\\\""),
                )
            })
            .collect();

        format!(
            r#"{{
  "schema_version": 1,
  "grand_total_ms": {},
  "blocking_total_ms": {},
  "blocking_pct": {:.1},
  "redundant_total_ms": {},
  "redundant_pct": {:.1},
  "entry_count": {},
  "optimization_target_count": {},
  "stage_breakdown": [
{}
  ],
  "lane_breakdown": [
{}
  ],
  "optimization_targets": [
{}
  ]
}}"#,
            self.grand_total_ms,
            self.blocking_total_ms,
            self.blocking_pct(),
            self.redundant_total_ms,
            self.redundant_pct(),
            self.entries.len(),
            self.optimization_targets.len(),
            stage_json.join(",\n"),
            lane_json.join(",\n"),
            target_json.join(",\n"),
        )
    }

    /// Human-readable summary for operator consumption.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "Doctor Workflow Cost Profile ({} entries, {} targets)",
            self.entries.len(),
            self.optimization_targets.len()
        ));
        lines.push(format!(
            "Total: {}ms | Blocking: {}ms ({:.0}%) | Redundant: {}ms ({:.0}%)",
            self.grand_total_ms,
            self.blocking_total_ms,
            self.blocking_pct(),
            self.redundant_total_ms,
            self.redundant_pct(),
        ));
        lines.push(String::new());

        lines.push("Stage breakdown:".to_string());
        for (stage, total) in &self.stage_totals {
            if *total > 0 {
                let pct = (*total as f64 / self.grand_total_ms as f64) * 100.0;
                lines.push(format!(
                    "  {:<10} {:>8}ms ({:.0}%)",
                    stage.label(),
                    total,
                    pct
                ));
            }
        }
        lines.push(String::new());

        lines.push("Top optimization targets:".to_string());
        for target in self.optimization_targets.iter().take(5) {
            lines.push(format!(
                "  [{:>8}] {}/{}: {} ({}ms{})",
                target.impact.label(),
                target.stage.label(),
                target.lane.label(),
                target.operation,
                target.wall_clock_ms,
                if target.blocking { ", blocking" } else { "" },
            ));
        }

        lines.join("\n")
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_profile_has_all_stages() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();

        for stage in WorkflowStage::ALL {
            assert!(
                !report.by_stage(*stage).is_empty(),
                "canonical profile missing stage {}",
                stage.label()
            );
        }
    }

    #[test]
    fn canonical_profile_has_all_lanes() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();

        for lane in CostLane::ALL {
            assert!(
                !report.by_lane(*lane).is_empty(),
                "canonical profile missing lane {}",
                lane.label()
            );
        }
    }

    #[test]
    fn canonical_capture_dominates() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();

        let capture_total = report.stage_total(WorkflowStage::Capture);
        let other_total: u64 = report
            .stage_totals
            .iter()
            .filter(|(s, _)| *s != WorkflowStage::Capture)
            .map(|(_, t)| *t)
            .sum();

        assert!(
            capture_total > other_total,
            "capture should dominate: capture={}ms, others={}ms",
            capture_total,
            other_total
        );
    }

    #[test]
    fn canonical_subprocess_is_largest_lane() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();

        let subprocess_total = report.lane_total(CostLane::Subprocess);
        for lane in CostLane::ALL {
            if *lane != CostLane::Subprocess {
                assert!(
                    subprocess_total >= report.lane_total(*lane),
                    "subprocess should be largest lane: subprocess={}ms, {}={}ms",
                    subprocess_total,
                    lane.label(),
                    report.lane_total(*lane)
                );
            }
        }
    }

    #[test]
    fn blocking_percentage_is_high() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();

        assert!(
            report.blocking_pct() > 80.0,
            "blocking should be dominant: {:.1}%",
            report.blocking_pct()
        );
    }

    #[test]
    fn optimization_targets_are_ranked() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();

        assert!(
            !report.optimization_targets.is_empty(),
            "should have optimization targets"
        );

        // Targets should be sorted by impact (highest first)
        for window in report.optimization_targets.windows(2) {
            assert!(
                window[0].impact >= window[1].impact,
                "targets not sorted: {:?} should be >= {:?}",
                window[0].impact,
                window[1].impact
            );
        }
    }

    #[test]
    fn vhs_recording_is_critical_target() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();

        let vhs_targets: Vec<&OptimizationTarget> = report
            .optimization_targets
            .iter()
            .filter(|t| t.operation == "vhs_host_record")
            .collect();

        assert!(!vhs_targets.is_empty(), "VHS recording should be a target");
        assert_eq!(
            vhs_targets[0].impact,
            OptimizationImpact::Critical,
            "VHS recording should be critical impact"
        );
    }

    #[test]
    fn report_json_is_valid() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();
        let json = report.to_json();

        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"grand_total_ms\":"));
        assert!(json.contains("\"blocking_pct\":"));
        assert!(json.contains("\"stage_breakdown\":"));
        assert!(json.contains("\"lane_breakdown\":"));
        assert!(json.contains("\"optimization_targets\":"));
    }

    #[test]
    fn report_summary_is_readable() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();
        let summary = report.summary();

        assert!(summary.contains("Doctor Workflow Cost Profile"));
        assert!(summary.contains("Blocking:"));
        assert!(summary.contains("Stage breakdown:"));
        assert!(summary.contains("Top optimization targets:"));
    }

    #[test]
    fn custom_profile_works() {
        let mut profile = DoctorCostProfile::new();
        profile.record(
            CostEntry::new(WorkflowStage::Capture, CostLane::Subprocess)
                .operation("test_op")
                .wall_clock_ms(1000)
                .blocking(true)
                .impact(OptimizationImpact::High),
        );

        let report = profile.finalize();
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.grand_total_ms, 1000);
        assert_eq!(report.blocking_total_ms, 1000);
    }

    #[test]
    fn empty_profile_produces_valid_report() {
        let profile = DoctorCostProfile::new();
        let report = profile.finalize();
        assert_eq!(report.grand_total_ms, 0);
        assert_eq!(report.blocking_pct(), 0.0);
        assert_eq!(report.redundant_pct(), 0.0);
    }

    #[test]
    fn redundant_operations_tracked() {
        let profile = DoctorCostProfile::canonical();
        let report = profile.finalize();

        let non_essential: Vec<&CostEntry> =
            report.entries.iter().filter(|e| !e.essential).collect();
        assert!(
            !non_essential.is_empty(),
            "canonical profile should have some non-essential entries"
        );
        assert!(report.redundant_total_ms > 0, "should have redundant time");
    }

    #[test]
    fn stage_labels_unique() {
        let labels: Vec<&str> = WorkflowStage::ALL.iter().map(|s| s.label()).collect();
        let mut seen = std::collections::HashSet::new();
        for label in &labels {
            assert!(seen.insert(*label), "duplicate stage label: {label}");
        }
    }

    #[test]
    fn lane_labels_unique() {
        let labels: Vec<&str> = CostLane::ALL.iter().map(|l| l.label()).collect();
        let mut seen = std::collections::HashSet::new();
        for label in &labels {
            assert!(seen.insert(*label), "duplicate lane label: {label}");
        }
    }
}
