#![forbid(unsafe_code)]

//! Simulated data generation and deterministic data sources for the demo showcase.
//!
//! All data generation is deterministic for a given `tick_count` so that snapshot
//! tests produce reproducible output. No system time or external randomness is used.

use std::collections::VecDeque;

use crate::determinism;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CPU_HISTORY_CAP: usize = 60;
const MEMORY_HISTORY_CAP: usize = 60;
const NETWORK_HISTORY_CAP: usize = 60;
const CHART_HISTORY_CAP: usize = 60;
const TARGET_PROCESS_COUNT: usize = 20;
const MAX_ALERTS: usize = 50;
const ALERT_INTERVAL: u64 = 20;
const DISK_CATEGORIES: [&str; 5] = ["System", "Applications", "Documents", "Media", "Cache"];

// ---------------------------------------------------------------------------
// Deterministic pseudo-random
// ---------------------------------------------------------------------------

/// Simple deterministic hash for reproducible "random" values.
/// Uses a splitmix64-style scramble of `seed`.
fn det_hash(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Returns a deterministic f64 in `[0.0, 1.0)` for the given seed.
fn det_float(seed: u64) -> f64 {
    (det_hash(seed) >> 11) as f64 / ((1u64 << 53) as f64)
}

/// Returns a deterministic f64 in `[lo, hi)` for the given seed.
fn det_range(seed: u64, lo: f64, hi: f64) -> f64 {
    lo + det_float(seed) * (hi - lo)
}

// ---------------------------------------------------------------------------
// Alert types
// ---------------------------------------------------------------------------

/// Severity level for system alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
}

/// A simulated system alert.
#[derive(Debug, Clone)]
pub struct Alert {
    pub severity: AlertSeverity,
    pub message: String,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Process info
// ---------------------------------------------------------------------------

/// A simulated running process.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f64,
    pub mem_mb: f64,
}

// ---------------------------------------------------------------------------
// SimulatedData
// ---------------------------------------------------------------------------

/// System-monitor style simulated data, updated each tick.
pub struct SimulatedData {
    seed: u64,
    pub cpu_history: VecDeque<f64>,
    pub memory_history: VecDeque<f64>,
    pub network_in: VecDeque<f64>,
    pub network_out: VecDeque<f64>,
    pub disk_usage: Vec<(String, f64)>,
    pub processes: Vec<ProcessInfo>,
    pub alerts: VecDeque<Alert>,
    pub events_per_second: f64,
}

impl Default for SimulatedData {
    fn default() -> Self {
        let disk_usage = DISK_CATEGORIES
            .iter()
            .map(|name| ((*name).to_owned(), 0.0))
            .collect();
        Self {
            seed: determinism::demo_seed(0),
            cpu_history: VecDeque::with_capacity(CPU_HISTORY_CAP + 1),
            memory_history: VecDeque::with_capacity(MEMORY_HISTORY_CAP + 1),
            network_in: VecDeque::with_capacity(NETWORK_HISTORY_CAP + 1),
            network_out: VecDeque::with_capacity(NETWORK_HISTORY_CAP + 1),
            disk_usage,
            processes: Vec::with_capacity(TARGET_PROCESS_COUNT + 2),
            alerts: VecDeque::with_capacity(MAX_ALERTS + 1),
            events_per_second: 0.0,
        }
    }
}

impl SimulatedData {
    /// Create deterministic data seeded with the given value.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            seed,
            ..Default::default()
        }
    }

    /// Advance the simulation by one tick. All values are deterministic given
    /// `tick_count`.
    pub fn tick(&mut self, tick_count: u64) {
        let tick = tick_count.wrapping_add(self.seed);
        self.update_cpu(tick);
        self.update_memory(tick);
        self.update_network(tick);
        self.update_disk(tick);
        self.update_processes(tick);
        self.update_alerts(tick);
        self.events_per_second = 800.0
            + 400.0 * ((tick as f64) / 30.0).sin()
            + det_range(tick.wrapping_mul(7), -50.0, 50.0);
    }

    fn update_cpu(&mut self, tick: u64) {
        let base = 30.0 + 20.0 * ((tick as f64) / 50.0).sin();
        let noise = det_range(tick.wrapping_mul(3), -5.0, 5.0);
        let value = (base + noise).clamp(0.0, 100.0);
        self.cpu_history.push_back(value);
        if self.cpu_history.len() > CPU_HISTORY_CAP {
            self.cpu_history.pop_front();
        }
    }

    fn update_memory(&mut self, tick: u64) {
        // Memory slowly climbs with a sawtooth pattern, resetting periodically.
        let cycle = (tick % 500) as f64 / 500.0;
        let base = 40.0 + 30.0 * cycle;
        let noise = det_range(tick.wrapping_mul(5), -2.0, 2.0);
        let value = (base + noise).clamp(0.0, 100.0);
        self.memory_history.push_back(value);
        if self.memory_history.len() > MEMORY_HISTORY_CAP {
            self.memory_history.pop_front();
        }
    }

    fn update_network(&mut self, tick: u64) {
        // Network with random bursts.
        let burst_in = if det_hash(tick.wrapping_mul(11)).is_multiple_of(10) {
            det_range(tick.wrapping_mul(13), 500.0, 2000.0)
        } else {
            det_range(tick.wrapping_mul(13), 10.0, 200.0)
        };
        let burst_out = if det_hash(tick.wrapping_mul(17)).is_multiple_of(8) {
            det_range(tick.wrapping_mul(19), 300.0, 1500.0)
        } else {
            det_range(tick.wrapping_mul(19), 5.0, 150.0)
        };
        self.network_in.push_back(burst_in);
        self.network_out.push_back(burst_out);
        if self.network_in.len() > NETWORK_HISTORY_CAP {
            self.network_in.pop_front();
        }
        if self.network_out.len() > NETWORK_HISTORY_CAP {
            self.network_out.pop_front();
        }
    }

    fn update_disk(&mut self, tick: u64) {
        for (i, (_name, usage)) in self.disk_usage.iter_mut().enumerate() {
            let base = match i {
                0 => 65.0, // System
                1 => 45.0, // Applications
                2 => 30.0, // Documents
                3 => 55.0, // Media
                _ => 20.0, // Cache
            };
            let drift = 5.0 * ((tick as f64) / (100.0 + i as f64 * 30.0)).sin();
            let noise = det_range(tick.wrapping_mul(23).wrapping_add(i as u64), -1.0, 1.0);
            *usage = (base + drift + noise).clamp(0.0, 100.0);
        }
    }

    fn update_processes(&mut self, tick: u64) {
        // Rebuild process list deterministically each tick.
        self.processes.clear();
        let process_names = [
            "systemd",
            "ftui-demo",
            "sshd",
            "postgres",
            "nginx",
            "redis-server",
            "node",
            "cargo",
            "rustc",
            "python3",
            "dockerd",
            "containerd",
            "tmux",
            "zsh",
            "htop",
            "git",
            "rg",
            "fd",
            "bat",
            "asupersync-rt",
            "journald",
            "dbus-daemon",
        ];

        // Determine how many processes to show (approximately TARGET_PROCESS_COUNT).
        let count = TARGET_PROCESS_COUNT.min(process_names.len());
        // Occasionally one process spawns or dies.
        let jitter = (det_hash(tick.wrapping_mul(29)) % 3) as usize;
        let active_count = if det_hash(tick.wrapping_mul(31)).is_multiple_of(2) {
            count.saturating_sub(jitter).max(TARGET_PROCESS_COUNT - 2)
        } else {
            (count + jitter).min(process_names.len())
        };

        for (i, &pname) in process_names.iter().enumerate().take(active_count) {
            let seed_base = tick.wrapping_mul(37).wrapping_add(i as u64 * 41);
            let pid = 1000 + (det_hash(seed_base) % 50000) as u32;
            let cpu = det_range(seed_base.wrapping_add(1), 0.0, 25.0);
            let mem = det_range(seed_base.wrapping_add(2), 5.0, 500.0);
            self.processes.push(ProcessInfo {
                pid,
                name: pname.to_owned(),
                cpu_percent: (cpu * 10.0).round() / 10.0,
                mem_mb: (mem * 10.0).round() / 10.0,
            });
        }
    }

    fn update_alerts(&mut self, tick: u64) {
        if tick.is_multiple_of(ALERT_INTERVAL) && tick > 0 {
            let severity_seed = det_hash(tick.wrapping_mul(43)) % 10;
            let severity = match severity_seed {
                0..=1 => AlertSeverity::Error,
                2..=4 => AlertSeverity::Warning,
                _ => AlertSeverity::Info,
            };
            let messages = [
                "CPU spike detected on core 3",
                "Memory allocation pool expanded",
                "Network latency increased to 45ms",
                "Disk I/O throughput nominal",
                "Service health check passed",
                "Cache eviction rate elevated",
                "Connection pool at 80% capacity",
                "Garbage collection completed",
                "TLS certificate renewal scheduled",
                "Rate limiter threshold adjusted",
            ];
            let msg_idx = (det_hash(tick.wrapping_mul(47)) % messages.len() as u64) as usize;
            self.alerts.push_back(Alert {
                severity,
                message: messages[msg_idx].to_owned(),
                timestamp: tick,
            });
            if self.alerts.len() > MAX_ALERTS {
                self.alerts.pop_front();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ChartData (for DataViz screen)
// ---------------------------------------------------------------------------

/// Mathematical series data for the data-visualization screen.
pub struct ChartData {
    pub sine_series: VecDeque<f64>,
    pub cosine_series: VecDeque<f64>,
    pub random_series: VecDeque<f64>,
}

impl Default for ChartData {
    fn default() -> Self {
        Self {
            sine_series: VecDeque::with_capacity(CHART_HISTORY_CAP + 1),
            cosine_series: VecDeque::with_capacity(CHART_HISTORY_CAP + 1),
            random_series: VecDeque::with_capacity(CHART_HISTORY_CAP + 1),
        }
    }
}

impl ChartData {
    /// Push new data points for the given tick. Deterministic.
    pub fn tick(&mut self, tick_count: u64) {
        let t = tick_count as f64;
        self.sine_series.push_back(t.sin());
        self.cosine_series.push_back(t.cos());
        self.random_series
            .push_back(det_range(tick_count.wrapping_mul(53), -1.0, 1.0));

        if self.sine_series.len() > CHART_HISTORY_CAP {
            self.sine_series.pop_front();
        }
        if self.cosine_series.len() > CHART_HISTORY_CAP {
            self.cosine_series.pop_front();
        }
        if self.random_series.len() > CHART_HISTORY_CAP {
            self.random_series.pop_front();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn data_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join(name)
    }

    fn read_fixture(name: &str) -> (String, PathBuf) {
        let path = data_path(name);
        let text = fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read fixture {name} at {}: {err}", path.display())
        });
        (text, path)
    }

    #[test]
    fn det_float_in_unit_range() {
        for seed in 0..200 {
            let value = det_float(seed);
            assert!(
                (0.0..1.0).contains(&value),
                "det_float out of range for seed {seed}: {value}"
            );
        }
    }

    #[test]
    fn det_range_within_bounds() {
        for seed in 0..200 {
            let value = det_range(seed, -5.0, 3.5);
            assert!(
                (-5.0..3.5).contains(&value),
                "det_range out of bounds for seed {seed}: {value}"
            );
        }
    }

    #[test]
    fn simulated_data_deterministic() {
        let mut a = SimulatedData::default();
        let mut b = SimulatedData::default();
        for tick in 0..100 {
            a.tick(tick);
            b.tick(tick);
        }
        // CPU histories must match exactly.
        assert_eq!(a.cpu_history.len(), b.cpu_history.len());
        for (va, vb) in a.cpu_history.iter().zip(b.cpu_history.iter()) {
            assert!(
                (va - vb).abs() < f64::EPSILON,
                "CPU mismatch at some point: {va} vs {vb}"
            );
        }
        // Memory histories must match.
        for (va, vb) in a.memory_history.iter().zip(b.memory_history.iter()) {
            assert!(
                (va - vb).abs() < f64::EPSILON,
                "Memory mismatch: {va} vs {vb}"
            );
        }
        // Process lists must match.
        assert_eq!(a.processes.len(), b.processes.len());
        for (pa, pb) in a.processes.iter().zip(b.processes.iter()) {
            assert_eq!(pa.name, pb.name);
            assert_eq!(pa.pid, pb.pid);
        }
        // Alerts must match.
        assert_eq!(a.alerts.len(), b.alerts.len());
        for (aa, ab) in a.alerts.iter().zip(b.alerts.iter()) {
            assert_eq!(aa.severity, ab.severity);
            assert_eq!(aa.message, ab.message);
            assert_eq!(aa.timestamp, ab.timestamp);
        }
    }

    #[test]
    fn cpu_history_bounded() {
        let mut data = SimulatedData::default();
        for tick in 0..200 {
            data.tick(tick);
        }
        assert!(
            data.cpu_history.len() <= CPU_HISTORY_CAP,
            "CPU history exceeded cap: {}",
            data.cpu_history.len()
        );
        // Values must be in [0, 100].
        for &v in &data.cpu_history {
            assert!((0.0..=100.0).contains(&v), "CPU value out of range: {v}");
        }
    }

    #[test]
    fn alert_generation() {
        let mut data = SimulatedData::default();
        for tick in 0..100 {
            data.tick(tick);
        }
        // With ALERT_INTERVAL=20, ticks 20,40,60,80 should generate alerts = 4.
        assert_eq!(
            data.alerts.len(),
            4,
            "Expected 4 alerts at interval {ALERT_INTERVAL}, got {}",
            data.alerts.len()
        );
        // Verify timestamps are at expected intervals.
        for (i, alert) in data.alerts.iter().enumerate() {
            let expected_tick = (i as u64 + 1) * ALERT_INTERVAL;
            assert_eq!(alert.timestamp, expected_tick);
        }
    }

    #[test]
    fn process_list_bounded() {
        let mut data = SimulatedData::default();
        for tick in 0..200 {
            data.tick(tick);
            assert!(
                data.processes.len() <= TARGET_PROCESS_COUNT + 2,
                "Too many processes at tick {tick}: {}",
                data.processes.len()
            );
            assert!(
                data.processes.len() >= TARGET_PROCESS_COUNT - 2,
                "Too few processes at tick {tick}: {}",
                data.processes.len()
            );
        }
    }

    #[test]
    fn chart_data_deterministic() {
        let mut a = ChartData::default();
        let mut b = ChartData::default();
        for tick in 0..100 {
            a.tick(tick);
            b.tick(tick);
        }
        assert_eq!(a.sine_series.len(), b.sine_series.len());
        for (va, vb) in a.sine_series.iter().zip(b.sine_series.iter()) {
            assert!((va - vb).abs() < f64::EPSILON);
        }
        for (va, vb) in a.random_series.iter().zip(b.random_series.iter()) {
            assert!((va - vb).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn chart_data_bounded() {
        let mut chart = ChartData::default();
        for tick in 0..200 {
            chart.tick(tick);
        }
        assert!(chart.sine_series.len() <= CHART_HISTORY_CAP);
        assert!(chart.cosine_series.len() <= CHART_HISTORY_CAP);
        assert!(chart.random_series.len() <= CHART_HISTORY_CAP);
    }

    #[test]
    fn shakespeare_fixture_invariants() {
        let (text, path) = read_fixture("shakespeare.txt");
        let line_count = text.lines().count();
        assert!(
            line_count > 100_000,
            "shakespeare fixture too small: path={}, lines={line_count}",
            path.display()
        );
        assert!(
            text.contains("THE TRAGEDY OF HAMLET"),
            "shakespeare fixture missing Hamlet header: path={}",
            path.display()
        );
        assert!(
            text.contains("ROMEO AND JULIET"),
            "shakespeare fixture missing Romeo and Juliet header: path={}",
            path.display()
        );
    }

    #[test]
    fn sqlite_fixture_invariants() {
        let (text, path) = read_fixture("sqlite3.c");
        let line_count = text.lines().count();
        assert!(
            line_count > 100_000,
            "sqlite3.c fixture too small: path={}, lines={line_count}",
            path.display()
        );
        assert!(
            text.contains("sqlite3_open"),
            "sqlite3.c fixture missing sqlite3_open: path={}",
            path.display()
        );
        assert!(
            text.contains("SQLite"),
            "sqlite3.c fixture missing SQLite header text: path={}",
            path.display()
        );
    }

    #[test]
    fn sqlite_header_fixture_invariants() {
        let (text, path) = read_fixture("sqlite3.h");
        let line_count = text.lines().count();
        assert!(
            line_count > 5_000,
            "sqlite3.h fixture too small: path={}, lines={line_count}",
            path.display()
        );
        assert!(
            text.contains("SQLITE_VERSION"),
            "sqlite3.h fixture missing SQLITE_VERSION: path={}",
            path.display()
        );
    }

    #[test]
    fn notice_fixture_mentions_assets() {
        let (text, path) = read_fixture("NOTICE");
        assert!(
            text.contains("shakespeare.txt"),
            "NOTICE missing shakespeare.txt entry: path={}",
            path.display()
        );
        assert!(
            text.contains("sqlite3.c"),
            "NOTICE missing sqlite3.c entry: path={}",
            path.display()
        );
        assert!(
            text.contains("sqlite3.h"),
            "NOTICE missing sqlite3.h entry: path={}",
            path.display()
        );
    }
}
