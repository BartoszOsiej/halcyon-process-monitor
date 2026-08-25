#![forbid(unsafe_code)]

//! Doctor supervision topology for Asupersync migration (bd-1889t).
//!
//! This module defines the supervised topology for `doctor_frankentui` workflows.
//! Each workflow is decomposed into supervised tasks with explicit cancellation,
//! timeout, retry, and evidence boundaries. The topology doubles as the migration
//! roadmap — each node identifies what to migrate, why, and what to preserve.
//!
//! # Topology overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    DoctorSupervisor (root)                     │
//! │  Owns: global deadline, trace_id, run_dir, evidence_ledger    │
//! ├─────────────┬──────────────┬────────────────┬─────────────────┤
//! │ SeedFlow    │ CaptureFlow  │ SuiteFlow      │ ReportFlow      │
//! │ (network)   │ (subprocess) │ (fan-out)       │ (file I/O)     │
//! └──────┬──────┴──────┬───────┴────────┬───────┴─────────────────┘
//!        │             │                │
//!   ┌────┴────┐   ┌───┴────────┐  ┌───┴────────┐
//!   │ RPC     │   │ VhsHost    │  │ Profile[n] │
//!   │ Stage[n]│   │ VhsDocker  │  │ (recursive │
//!   │ (retry) │   │ TmuxObs    │  │  doctor)   │
//!   └─────────┘   │ Snapshot   │  └────────────┘
//!                 │ SeedThread │
//!                 └────────────┘
//! ```
//!
//! # Migration value per flow
//!
//! | Flow | Value | Why |
//! |------|-------|-----|
//! | SeedFlow | **High** | Replaces ad-hoc retry/polling with structured deadlines and cancellation |
//! | CaptureFlow | **Critical** | Replaces process group management, timeout polling, fallback logic |
//! | SuiteFlow | **Medium** | Replaces sequential subprocess fan-out with supervised parallel tracks |
//! | ReportFlow | **Low** | Already deterministic file I/O; minimal supervision benefit |
//!
//! # Preservation invariants
//!
//! - `trace_id` propagation through all supervised tasks
//! - `run_meta.json` 57-field contract (status, exit codes, fallback metadata)
//! - `evidence_ledger.jsonl` append-only JSONL with `DecisionRecord` schema
//! - Process group termination semantics (SIGTERM → 1s → SIGKILL)
//! - Failure classification pipeline (`classify_capture_failure`)
//! - Output mode detection (`OutputIntegration::detect()`)

/// Migration priority for a supervised workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MigrationPriority {
    /// Critical path — migrate first for maximum impact.
    Critical,
    /// High value — migrate after critical path.
    High,
    /// Medium value — migrate when bandwidth allows.
    Medium,
    /// Low value — defer or skip (already well-structured).
    Low,
}

/// A supervised concern in the doctor topology.
#[derive(Debug, Clone)]
pub struct SupervisedNode {
    /// Human-readable name for this node.
    pub name: &'static str,
    /// Source file containing this workflow.
    pub source: &'static str,
    /// Migration priority.
    pub priority: MigrationPriority,
    /// User/operator value of migrating this node.
    pub value: &'static str,
    /// Timeout boundary (if any).
    pub timeout: Option<TimeoutSpec>,
    /// Retry policy (if any).
    pub retry: Option<RetrySpec>,
    /// Subprocess management (if any).
    pub subprocess: Option<SubprocessSpec>,
    /// Network operations (if any).
    pub network: Option<NetworkSpec>,
    /// Evidence artifacts produced.
    pub evidence: &'static [&'static str],
    /// Failure modes to preserve.
    pub failure_modes: &'static [&'static str],
    /// Child nodes in the supervision tree.
    pub children: &'static [&'static str],
}

/// Timeout specification for a supervised node.
#[derive(Debug, Clone, Copy)]
pub struct TimeoutSpec {
    /// Default timeout in seconds.
    pub default_secs: u64,
    /// Whether configurable via CLI/env.
    pub configurable: bool,
    /// Polling interval in milliseconds.
    pub poll_interval_ms: u64,
}

/// Retry specification for a supervised node.
#[derive(Debug, Clone, Copy)]
pub struct RetrySpec {
    /// Maximum retry attempts.
    pub max_attempts: u32,
    /// Base backoff in milliseconds.
    pub base_backoff_ms: u64,
    /// Whether backoff is exponential.
    pub exponential: bool,
}

/// Subprocess management specification.
#[derive(Debug, Clone, Copy)]
pub struct SubprocessSpec {
    /// Process name or command.
    pub command: &'static str,
    /// Whether process group leadership is used.
    pub process_group: bool,
    /// Grace period before force kill (ms).
    pub kill_grace_ms: u64,
}

/// Network operation specification.
#[derive(Debug, Clone, Copy)]
pub struct NetworkSpec {
    /// Protocol used.
    pub protocol: &'static str,
    /// Per-request timeout in seconds.
    pub request_timeout_secs: u64,
}

/// Build the complete doctor supervision topology.
///
/// Returns all nodes in the tree. The root is always `doctor_supervisor`.
#[must_use]
pub fn topology() -> Vec<SupervisedNode> {
    vec![
        // ================================================================
        // Root supervisor
        // ================================================================
        SupervisedNode {
            name: "doctor_supervisor",
            source: "doctor.rs",
            priority: MigrationPriority::Critical,
            value: "Root orchestrator — owns global deadline, trace_id, \
                    and evidence ledger lifecycle",
            timeout: None,
            retry: None,
            subprocess: None,
            network: None,
            evidence: &["run_meta.json", "evidence_ledger.jsonl", "run_summary.txt"],
            failure_modes: &[
                "missing required commands (vhs, ffmpeg)",
                "smoke test failure with app fallback",
                "terminal initialization error",
            ],
            children: &["seed_flow", "capture_flow", "suite_flow", "report_flow"],
        },
        // ================================================================
        // Seed flow (MCP bootstrap via JSON-RPC)
        // ================================================================
        SupervisedNode {
            name: "seed_flow",
            source: "seed.rs",
            priority: MigrationPriority::High,
            value: "Replaces ad-hoc retry/polling with structured deadlines. \
                    Fewer mystery waits, clearer failure evidence, better \
                    timeout handling for operators.",
            timeout: Some(TimeoutSpec {
                default_secs: 30,
                configurable: true,
                poll_interval_ms: 1000,
            }),
            retry: Some(RetrySpec {
                max_attempts: 3,
                base_backoff_ms: 100,
                exponential: true,
            }),
            subprocess: None,
            network: Some(NetworkSpec {
                protocol: "HTTP JSON-RPC",
                request_timeout_secs: 10,
            }),
            evidence: &["seed.log", "seed.stdout.log", "seed.stderr.log"],
            failure_modes: &[
                "unreachable server (connection refused)",
                "server probe non-result (empty/non-JSON)",
                "RPC error response",
                "deadline exhaustion mid-run",
                "backoff exhaustion (3 retries failed)",
                "send_message permanent failure",
                "stage lifecycle failure (seed_stage_failed)",
            ],
            // RPC stages are internal to the seed flow, not separate supervised
            // nodes. They share the flow's deadline and retry policy.
            // Stages: ensure_project, register_agent, fetch_inbox,
            // search_messages, send_message, file_reservation_paths.
            children: &[],
        },
        // ================================================================
        // Capture flow (VHS/ttyd subprocess orchestration)
        // ================================================================
        SupervisedNode {
            name: "capture_flow",
            source: "capture.rs",
            priority: MigrationPriority::Critical,
            value: "Replaces process group management, timeout polling, and \
                    Docker fallback logic with structured supervision. Reduces \
                    hung captures, improves failure diagnostics, and makes \
                    fallback decisions explicit and auditable.",
            timeout: Some(TimeoutSpec {
                default_secs: 300,
                configurable: true,
                poll_interval_ms: 200,
            }),
            retry: None,
            subprocess: Some(SubprocessSpec {
                command: "vhs",
                process_group: true,
                kill_grace_ms: 1000,
            }),
            network: None,
            evidence: &[
                "vhs.log",
                "run_meta.json",
                "snapshot.png",
                "ttyd_shim.log",
                "ttyd_runtime.log",
            ],
            failure_modes: &[
                "missing vhs binary",
                "ttyd EOF handshake failure",
                "VHS timeout (exit 124)",
                "VHS non-zero exit",
                "fatal stream reason detected",
                "defunct ttyd process",
                "Docker fallback required",
                "conservative mode triggered",
                "snapshot extraction failure",
            ],
            children: &[
                "vhs_host_subprocess",
                "vhs_docker_fallback",
                "tmux_observer",
                "seed_demo_thread",
                "log_pump_threads",
            ],
        },
        // ================================================================
        // Capture children
        // ================================================================
        SupervisedNode {
            name: "vhs_host_subprocess",
            source: "capture.rs",
            priority: MigrationPriority::Critical,
            value: "Primary capture path — process group with SIGTERM/SIGKILL \
                    lifecycle. Most complex subprocess management in the crate.",
            timeout: Some(TimeoutSpec {
                default_secs: 300,
                configurable: true,
                poll_interval_ms: 200,
            }),
            retry: None,
            subprocess: Some(SubprocessSpec {
                command: "vhs",
                process_group: true,
                kill_grace_ms: 1000,
            }),
            network: None,
            evidence: &["vhs.log"],
            failure_modes: &[
                "exit code 124 (timeout)",
                "non-zero exit with fatal reason",
                "process group termination failure",
            ],
            children: &[],
        },
        SupervisedNode {
            name: "vhs_docker_fallback",
            source: "capture.rs",
            priority: MigrationPriority::High,
            value: "Fallback capture when host VHS fails. Docker container \
                    lifecycle adds complexity but improves reliability.",
            timeout: Some(TimeoutSpec {
                default_secs: 300,
                configurable: true,
                poll_interval_ms: 200,
            }),
            retry: None,
            subprocess: Some(SubprocessSpec {
                command: "docker",
                process_group: false,
                kill_grace_ms: 2000,
            }),
            network: None,
            evidence: &["vhs.log", "run_meta.json (fallback_reason)"],
            failure_modes: &[
                "Docker not available",
                "image pull failure",
                "container timeout",
            ],
            children: &[],
        },
        SupervisedNode {
            name: "tmux_observer",
            source: "capture.rs",
            priority: MigrationPriority::Medium,
            value: "Optional terminal state capture for debugging. \
                    Cleanup reliability improves with structured supervision.",
            timeout: None,
            retry: None,
            subprocess: Some(SubprocessSpec {
                command: "tmux",
                process_group: false,
                kill_grace_ms: 0,
            }),
            network: None,
            evidence: &["tmux pane captures"],
            failure_modes: &["session cleanup failure (kill-session)"],
            children: &[],
        },
        SupervisedNode {
            name: "seed_demo_thread",
            source: "capture.rs",
            priority: MigrationPriority::High,
            value: "Thread-spawned seed demo with delayed start and 30s timeout. \
                    Structured supervision would replace thread::spawn + join.",
            timeout: Some(TimeoutSpec {
                default_secs: 30,
                configurable: false,
                poll_interval_ms: 0,
            }),
            retry: None,
            subprocess: None,
            network: None,
            evidence: &["seed.log"],
            failure_modes: &["thread join timeout", "seed failure propagation"],
            children: &[],
        },
        SupervisedNode {
            name: "log_pump_threads",
            source: "capture.rs",
            priority: MigrationPriority::Medium,
            value: "Per-stream log forwarding with fatal pattern detection. \
                    Structured supervision would make cancellation cleaner.",
            timeout: None,
            retry: None,
            subprocess: None,
            network: None,
            evidence: &["ttyd_runtime.log"],
            failure_modes: &["fatal pattern triggers early termination"],
            children: &[],
        },
        // ================================================================
        // Suite flow (multi-profile fan-out)
        // ================================================================
        SupervisedNode {
            name: "suite_flow",
            source: "suite.rs",
            priority: MigrationPriority::Medium,
            value: "Sequential profile fan-out that could become parallel with \
                    structured supervision. Reduces total suite time and improves \
                    failure isolation between profiles.",
            timeout: None,
            retry: None,
            subprocess: None,
            network: None,
            evidence: &["suite_manifest.json", "suite_summary.txt"],
            failure_modes: &[
                "invalid profile specification",
                "per-profile capture failure",
                "report generation failure",
                "missing metadata for aggregation",
            ],
            children: &["profile_run"],
        },
        SupervisedNode {
            name: "profile_run",
            source: "suite.rs",
            priority: MigrationPriority::Medium,
            value: "Individual profile execution — recursive doctor invocation. \
                    With supervision, failures are isolated per-profile.",
            timeout: None,
            retry: None,
            subprocess: Some(SubprocessSpec {
                command: "doctor_frankentui (recursive)",
                process_group: false,
                kill_grace_ms: 0,
            }),
            network: None,
            evidence: &["per-profile run_meta.json"],
            failure_modes: &["subprocess exit code non-zero", "missing artifacts"],
            children: &[],
        },
        // ================================================================
        // Report flow (post-processing)
        // ================================================================
        SupervisedNode {
            name: "report_flow",
            source: "report.rs",
            priority: MigrationPriority::Low,
            value: "Deterministic file I/O and HTML generation. Minimal \
                    supervision benefit — already well-structured.",
            timeout: None,
            retry: None,
            subprocess: None,
            network: None,
            evidence: &["report.json", "report.html", "doctor_summary.json"],
            failure_modes: &["file I/O error", "link resolution failure"],
            children: &[],
        },
    ]
}

/// Count nodes by migration priority.
#[must_use]
pub fn priority_summary(nodes: &[SupervisedNode]) -> Vec<(MigrationPriority, usize)> {
    let mut counts = std::collections::BTreeMap::new();
    for node in nodes {
        *counts.entry(node.priority).or_insert(0) += 1;
    }
    counts.into_iter().collect()
}

/// Return the names of nodes that have subprocess management.
#[must_use]
pub fn subprocess_nodes(nodes: &[SupervisedNode]) -> Vec<&'static str> {
    nodes
        .iter()
        .filter(|n| n.subprocess.is_some())
        .map(|n| n.name)
        .collect()
}

/// Return the names of nodes that have network operations.
#[must_use]
pub fn network_nodes(nodes: &[SupervisedNode]) -> Vec<&'static str> {
    nodes
        .iter()
        .filter(|n| n.network.is_some())
        .map(|n| n.name)
        .collect()
}

/// Return the names of nodes that have retry policies.
#[must_use]
pub fn retry_nodes(nodes: &[SupervisedNode]) -> Vec<&'static str> {
    nodes
        .iter()
        .filter(|n| n.retry.is_some())
        .map(|n| n.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_has_root() {
        let nodes = topology();
        assert!(
            nodes.iter().any(|n| n.name == "doctor_supervisor"),
            "topology must have a root supervisor node"
        );
    }

    #[test]
    fn topology_covers_all_major_flows() {
        let nodes = topology();
        let names: Vec<&str> = nodes.iter().map(|n| n.name).collect();

        assert!(names.contains(&"seed_flow"), "missing seed_flow");
        assert!(names.contains(&"capture_flow"), "missing capture_flow");
        assert!(names.contains(&"suite_flow"), "missing suite_flow");
        assert!(names.contains(&"report_flow"), "missing report_flow");
    }

    #[test]
    fn all_children_resolve_to_named_nodes() {
        let nodes = topology();
        let names: std::collections::HashSet<&str> = nodes.iter().map(|n| n.name).collect();

        for node in &nodes {
            for child in node.children {
                assert!(
                    names.contains(child),
                    "child '{}' of '{}' not found in topology",
                    child,
                    node.name
                );
            }
        }
    }

    #[test]
    fn critical_nodes_have_timeout_or_subprocess() {
        let nodes = topology();
        for node in &nodes {
            if node.priority == MigrationPriority::Critical && node.name != "doctor_supervisor" {
                assert!(
                    node.timeout.is_some() || node.subprocess.is_some(),
                    "critical node '{}' should have timeout or subprocess management",
                    node.name
                );
            }
        }
    }

    #[test]
    fn all_nodes_have_evidence() {
        let nodes = topology();
        for node in &nodes {
            assert!(
                !node.evidence.is_empty(),
                "node '{}' should produce at least one evidence artifact",
                node.name
            );
        }
    }

    #[test]
    fn all_nodes_have_failure_modes() {
        let nodes = topology();
        for node in &nodes {
            assert!(
                !node.failure_modes.is_empty(),
                "node '{}' should document at least one failure mode",
                node.name
            );
        }
    }

    #[test]
    fn priority_summary_covers_all_levels() {
        let nodes = topology();
        let summary = priority_summary(&nodes);
        let priorities: Vec<MigrationPriority> = summary.iter().map(|(p, _)| *p).collect();

        assert!(
            priorities.contains(&MigrationPriority::Critical),
            "should have critical nodes"
        );
        assert!(
            priorities.contains(&MigrationPriority::High),
            "should have high nodes"
        );
    }

    #[test]
    fn subprocess_nodes_identified() {
        let nodes = topology();
        let subs = subprocess_nodes(&nodes);

        assert!(
            subs.contains(&"vhs_host_subprocess"),
            "VHS host should be a subprocess node"
        );
        assert!(
            subs.contains(&"vhs_docker_fallback"),
            "VHS docker should be a subprocess node"
        );
    }

    #[test]
    fn network_nodes_identified() {
        let nodes = topology();
        let nets = network_nodes(&nodes);

        assert!(
            nets.contains(&"seed_flow"),
            "seed_flow should be a network node"
        );
    }

    #[test]
    fn retry_nodes_identified() {
        let nodes = topology();
        let retries = retry_nodes(&nodes);

        assert!(
            retries.contains(&"seed_flow"),
            "seed_flow should have retry policy"
        );
        assert_eq!(retries.len(), 1, "only seed_flow should have retry");
    }

    #[test]
    fn capture_flow_is_critical_priority() {
        let nodes = topology();
        let capture = nodes.iter().find(|n| n.name == "capture_flow").unwrap();
        assert_eq!(
            capture.priority,
            MigrationPriority::Critical,
            "capture flow must be critical — it's the most complex subprocess orchestration"
        );
    }

    #[test]
    fn report_flow_is_low_priority() {
        let nodes = topology();
        let report = nodes.iter().find(|n| n.name == "report_flow").unwrap();
        assert_eq!(
            report.priority,
            MigrationPriority::Low,
            "report flow is deterministic I/O — low migration value"
        );
    }

    #[test]
    fn seed_flow_has_retry_and_network_specs() {
        let nodes = topology();
        let seed = nodes.iter().find(|n| n.name == "seed_flow").unwrap();

        assert!(seed.retry.is_some(), "seed_flow must have retry policy");
        assert!(seed.network.is_some(), "seed_flow must have network spec");

        let retry = seed.retry.unwrap();
        assert_eq!(retry.max_attempts, 3, "seed uses 3 retry attempts");
        assert!(retry.exponential, "seed uses exponential backoff");

        let network = seed.network.unwrap();
        assert_eq!(network.protocol, "HTTP JSON-RPC");
    }
}
