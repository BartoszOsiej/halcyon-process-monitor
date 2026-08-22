use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use aya::maps::perf::PerfEventArrayBuffer;
use aya::maps::{MapData, PerfEventArray};
use aya::programs::TracePoint;
use aya::util::online_cpus;
use aya::Ebpf;
use bytes::BytesMut;
use chrono::Local;

pub const EVENT_EXECVE: u8 = 0;
pub const EVENT_OPENAT: u8 = 1;
pub const EVENT_CONNECT: u8 = 2;
pub const EVENT_ACCEPT: u8 = 3;
pub const EVENT_SENDTO: u8 = 4;
pub const EVENT_RECVFROM: u8 = 5;

const EVENT_COMM_LEN: usize = 16;
const EVENT_FILENAME_LEN: usize = 64;
const EVENT_ARGV_LEN: usize = 128;
const WINDOW_SECS: u64 = 1;
const OUT_BUFS: usize = 128;
const OUT_BUF_CAP: usize = 4096;

// ── eBPF event record (must match kernel-side #[repr(C)]) ────────────────

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProcessEvent {
    pub event_type: u8,
    pub pid: u32,
    pub uid: u32,
    pub comm: [u8; EVENT_COMM_LEN],
    pub filename: [u8; EVENT_FILENAME_LEN],
    pub argv: [u8; EVENT_ARGV_LEN],
}

unsafe impl aya::Pod for ProcessEvent {}

impl ProcessEvent {
    fn comm_str(&self) -> String {
        cstr_to_string(&self.comm)
    }
    fn filename_str(&self) -> String {
        cstr_to_string(&self.filename)
    }
    fn argv_str(&self) -> String {
        cstr_to_string(&self.argv)
    }
}

// ── Public types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Exec,
    Open,
    Connect,
    Accept,
    SendTo,
    RecvFrom,
}

#[derive(Debug, Clone)]
pub struct RecordedEvent {
    pub ts: String,
    pub kind: Kind,
    pub pid: u32,
    pub uid: u32,
    pub comm: String,
    pub file: Option<String>,
    pub extension: Option<String>,
    /// Full command path from argv[0] (execve events only).
    pub argv: Option<String>,
    /// Bytes count (for network send/recv events).
    pub bytes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub ts: String,
    pub pid: u32,
    pub uid: u32,
    pub comm: String,
    pub opens: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ProcStats {
    pub pid: u32,
    #[allow(dead_code)]
    pub ppid: u32,
    pub comm: String,
    pub window_opens: u64,
    pub total_opens: u64,
    pub total_execs: u64,
    pub alerts: u64,
    pub extensions: HashMap<String, u64>,
}

/// Ranked file access entry for the TUI "Top Files" panel.
#[derive(Debug, Clone)]
pub struct FileRank {
    pub path: String,
    pub count: u64,
    pub extension: String,
    /// Shannon entropy of the filename (0.0–1.0 normalised).
    pub entropy: f64,
}

/// Event-rate sample for sparkline visualisation.
#[derive(Debug, Clone, Copy)]
pub struct RateSample {
    pub exec_count: u64,
    pub open_count: u64,
    pub alert_count: u64,
}

/// A node in the process tree.
#[derive(Debug, Clone)]
pub struct ProcessNode {
    pub pid: u32,
    #[allow(dead_code)]
    pub ppid: u32,
    pub comm: String,
    pub alerts: u64,
    pub total_opens: u64,
    pub children: Vec<ProcessNode>,
}

pub enum Output {
    Event(RecordedEvent),
    Alert(Alert),
}

enum Msg {
    Event(RecordedEvent),
    Lost(u64),
}

// ── Monitor core ──────────────────────────────────────────────────────────

pub struct Monitor {
    rx: Receiver<Msg>,
    pub threshold: u64,
    stats: HashMap<u32, ProcStats>,
    windows: HashMap<u32, VecDeque<Instant>>,
    /// PID → PPID mapping (resolved from /proc/<pid>/status).
    pid_to_ppid: HashMap<u32, u32>,
    /// Per-path open count for the "Top Files" panel.
    file_counts: HashMap<String, u64>,
    /// Extension → open count for extension-frequency tracking.
    ext_counts: HashMap<String, u64>,
    /// Sliding window of per-second event counts (for sparklines).
    rate_history: VecDeque<RateSample>,
    /// Counters accumulated since the last rate-history tick.
    tick_execs: u64,
    tick_opens: u64,
    tick_alerts: u64,
    tick_start: Instant,
    pub total_events: u64,
    pub total_lost: u64,
    pub started: Instant,
    _reader: thread::JoinHandle<()>,
    _bpf: Option<Ebpf>,
}

impl Monitor {
    pub fn start(bpf_path: &Path, threshold: u64) -> Result<Self> {
        if unsafe { libc::geteuid() } != 0 {
            bail!("must be run as root: loading eBPF programs requires CAP_BPF / CAP_SYS_ADMIN");
        }

        eprintln!("[halcyon] loading eBPF object: {}", bpf_path.display());
        let mut bpf = Ebpf::load_file(bpf_path).context("failed to load eBPF program")?;
        eprintln!(
            "[halcyon] object parsed OK; programs: {:?}",
            bpf.programs().map(|(n, _)| n).collect::<Vec<_>>()
        );

        let program: &mut TracePoint = bpf
            .program_mut("sys_enter_execve")
            .context("failed to find 'sys_enter_execve' program")?
            .try_into()
            .context("'sys_enter_execve' is not a tracepoint program")?;
        program.load().context("failed to load execve program")?;
        program
            .attach("syscalls", "sys_enter_execve")
            .context("failed to attach execve tracepoint")?;
        eprintln!("[halcyon] attached tracepoint syscalls/sys_enter_execve");

        let program: &mut TracePoint = bpf
            .program_mut("sys_enter_openat")
            .context("failed to find 'sys_enter_openat' program")?
            .try_into()
            .context("'sys_enter_openat' is not a tracepoint program")?;
        program.load().context("failed to load openat program")?;
        program
            .attach("syscalls", "sys_enter_openat")
            .context("failed to attach openat tracepoint")?;
        eprintln!("[halcyon] attached tracepoint syscalls/sys_enter_openat");

        let perf_array: PerfEventArray<MapData> = bpf
            .take_map("EVENTS")
            .context("failed to find 'EVENTS' map")?
            .try_into()
            .context("'EVENTS' is not a PerfEventArray")?;

        let (tx, rx) = mpsc::channel();
        let reader = spawn_reader(perf_array, tx)?;

        let now = Instant::now();
        Ok(Self {
            rx,
            threshold,
            stats: HashMap::new(),
            windows: HashMap::new(),
            pid_to_ppid: HashMap::new(),
            file_counts: HashMap::new(),
            ext_counts: HashMap::new(),
            rate_history: VecDeque::new(),
            tick_execs: 0,
            tick_opens: 0,
            tick_alerts: 0,
            tick_start: now,
            total_events: 0,
            total_lost: 0,
            started: now,
            _reader: reader,
            _bpf: Some(bpf),
        })
    }

    pub fn poll(&mut self) -> Vec<Output> {
        let mut outputs = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Lost(n) => self.total_lost += n,
                Msg::Event(ev) => {
                    self.total_events += 1;
                    self.handle_event(&ev, &mut outputs);
                }
            }
        }
        // Accumulate rate samples every ~1 second.
        if self.tick_start.elapsed() >= Duration::from_secs(1) {
            self.rate_history.push_back(RateSample {
                exec_count: self.tick_execs,
                open_count: self.tick_opens,
                alert_count: self.tick_alerts,
            });
            // Keep last 120 seconds of history.
            if self.rate_history.len() > 120 {
                self.rate_history.pop_front();
            }
            self.tick_execs = 0;
            self.tick_opens = 0;
            self.tick_alerts = 0;
            self.tick_start = Instant::now();
        }
        outputs
    }

    pub(crate) fn handle_event(&mut self, ev: &RecordedEvent, outputs: &mut Vec<Output>) {
        let stats = self.stats.entry(ev.pid).or_default();
        stats.pid = ev.pid;
        stats.comm = ev.comm.clone();

        // Resolve PPID from /proc on first encounter.
        if let std::collections::hash_map::Entry::Vacant(e) = self.pid_to_ppid.entry(ev.pid) {
            if let Some(ppid) = read_ppid_from_proc(ev.pid) {
                e.insert(ppid);
                stats.ppid = ppid;
            }
        }

        match ev.kind {
            Kind::Exec => {
                stats.total_execs += 1;
                self.tick_execs += 1;
            }
            Kind::Connect | Kind::Accept | Kind::SendTo | Kind::RecvFrom => {
                // Network events — count as exec for stats purposes
                stats.total_execs += 1;
                self.tick_execs += 1;
            }
            Kind::Open => {
                // Track file extensions.
                if let Some(ref ext) = ev.extension {
                    if !ext.is_empty() {
                        *stats.extensions.entry(ext.clone()).or_insert(0) += 1;
                        *self.ext_counts.entry(ext.clone()).or_insert(0) += 1;
                    }
                }

                // Track per-file open count.
                if let Some(ref file) = ev.file {
                    *self.file_counts.entry(file.clone()).or_insert(0) += 1;
                }

                let now = Instant::now();
                let window = self.windows.entry(ev.pid).or_default();
                let cutoff = now - Duration::from_secs(WINDOW_SECS);
                while window.front().is_some_and(|t| *t < cutoff) {
                    window.pop_front();
                }
                window.push_back(now);

                stats.total_opens += 1;
                stats.window_opens = window.len() as u64;
                self.tick_opens += 1;

                if self.threshold > 0 && stats.window_opens == self.threshold {
                    stats.alerts += 1;
                    self.tick_alerts += 1;
                    outputs.push(Output::Alert(Alert {
                        ts: ev.ts.clone(),
                        pid: ev.pid,
                        uid: ev.uid,
                        comm: ev.comm.clone(),
                        opens: stats.window_opens,
                    }));
                }
            }
        }
        outputs.push(Output::Event(ev.clone()));
    }

    pub fn stats_sorted(&self) -> Vec<ProcStats> {
        let mut all: Vec<ProcStats> = self.stats.values().cloned().collect();
        all.sort_by_key(|s| std::cmp::Reverse(s.window_opens));
        all
    }

    /// Top-N most-opened files, sorted by count descending.
    pub fn top_files(&self, n: usize) -> Vec<FileRank> {
        let mut files: Vec<FileRank> = self
            .file_counts
            .iter()
            .map(|(path, &count)| {
                let ext = extract_extension(path);
                let entropy = shannon_entropy(path);
                FileRank {
                    path: path.clone(),
                    count,
                    extension: ext,
                    entropy,
                }
            })
            .collect();
        files.sort_by_key(|b| std::cmp::Reverse(b.count));
        files.truncate(n);
        files
    }

    /// Extension frequency map (extension → total open count across all processes).
    pub fn extension_counts(&self) -> &HashMap<String, u64> {
        &self.ext_counts
    }

    /// Sparkline-ready rate history (last N samples).
    pub fn rate_history(&self) -> &VecDeque<RateSample> {
        &self.rate_history
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// Build a hierarchical process tree from the flat PID→PPID stats.
    pub fn build_process_tree(&self) -> Vec<ProcessNode> {
        // Collect all known PIDs and their children.
        let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut all_pids: Vec<u32> = Vec::new();

        for &pid in self.stats.keys() {
            all_pids.push(pid);
            let ppid = self.pid_to_ppid.get(&pid).copied().unwrap_or(0);
            children_map.entry(ppid).or_default().push(pid);
        }

        // Sort children by name for stable display.
        for children in children_map.values_mut() {
            children.sort_by_key(|pid| {
                self.stats
                    .get(pid)
                    .map(|s| s.comm.clone())
                    .unwrap_or_default()
            });
        }

        // Find roots: PIDs whose parent is not in our stats (or is 0).
        let mut roots: Vec<u32> = all_pids
            .iter()
            .filter(|&&pid| {
                let ppid = self.pid_to_ppid.get(&pid).copied().unwrap_or(0);
                ppid == 0 || !self.stats.contains_key(&ppid)
            })
            .copied()
            .collect();
        roots.sort_by_key(|pid| {
            self.stats
                .get(pid)
                .map(|s| s.comm.clone())
                .unwrap_or_default()
        });

        // Build recursively.
        fn build(
            pid: u32,
            stats: &HashMap<u32, ProcStats>,
            children_map: &HashMap<u32, Vec<u32>>,
        ) -> ProcessNode {
            let s = stats.get(&pid);
            let children_ids = children_map.get(&pid).cloned().unwrap_or_default();
            let children: Vec<ProcessNode> = children_ids
                .into_iter()
                .map(|child_pid| build(child_pid, stats, children_map))
                .collect();
            ProcessNode {
                pid,
                ppid: s.map(|s| s.ppid).unwrap_or(0),
                comm: s
                    .map(|s| s.comm.clone())
                    .unwrap_or_else(|| "<unknown>".into()),
                alerts: s.map(|s| s.alerts).unwrap_or(0),
                total_opens: s.map(|s| s.total_opens).unwrap_or(0),
                children,
            }
        }

        roots
            .into_iter()
            .map(|pid| build(pid, &self.stats, &children_map))
            .collect()
    }

    /// Flatten a process tree into a Vec of (depth, node) for display.
    pub fn flatten_tree(tree: &[ProcessNode]) -> Vec<(usize, &ProcessNode)> {
        fn walk<'a>(node: &'a ProcessNode, depth: usize, out: &mut Vec<(usize, &'a ProcessNode)>) {
            out.push((depth, node));
            for child in &node.children {
                walk(child, depth + 1, out);
            }
        }
        let mut result = Vec::new();
        for root in tree {
            walk(root, 0, &mut result);
        }
        result
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Extract file extension (without the dot). Empty string for dotfiles / no ext.
fn extract_extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    if let Some(pos) = name.rfind('.') {
        if pos > 0 && pos < name.len() - 1 {
            return name[pos + 1..].to_lowercase();
        }
    }
    String::new()
}

/// Read the parent PID from /proc/<pid>/status (PPid line).
/// Returns None if the file cannot be read (process exited, permission denied, etc.).
fn read_ppid_from_proc(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("PPid:\t") {
            return rest.trim().parse::<u32>().ok();
        }
    }
    None
}

/// Compute normalised Shannon entropy of a byte string (0.0 = uniform, 1.0 = max).
fn shannon_entropy(s: &str) -> f64 {
    let bytes = s.as_bytes();
    let len = bytes.len() as f64;
    if len == 0.0 {
        return 0.0;
    }
    let mut freq = [0u64; 256];
    for &b in bytes {
        freq[b as usize] += 1;
    }
    let mut entropy = 0.0f64;
    for &f in &freq {
        if f > 0 {
            let p = f as f64 / len;
            entropy -= p * p.log2();
        }
    }
    // Normalise to [0, 1] range (max entropy for 256 symbols = 8 bits).
    entropy / 8.0
}

fn spawn_reader(
    mut perf_array: PerfEventArray<MapData>,
    tx: mpsc::Sender<Msg>,
) -> Result<thread::JoinHandle<()>> {
    let cpus = online_cpus()
        .map_err(|(err, io)| anyhow::anyhow!("failed to enumerate CPUs: {err}: {io}"))?;
    let mut buffers: Vec<PerfEventArrayBuffer<MapData>> = Vec::with_capacity(cpus.len());
    for cpu in cpus {
        let buf = perf_array
            .open(cpu, Some(8))
            .context("failed to open perf buffer")?;
        buffers.push(buf);
    }

    eprintln!("[halcyon] opening perf buffers on {} CPUs", buffers.len());

    thread::Builder::new()
        .name("halcyon-reader".into())
        .spawn(move || {
            let mut out = vec![BytesMut::with_capacity(OUT_BUF_CAP); OUT_BUFS];
            loop {
                let mut idle = true;
                for buf in buffers.iter_mut() {
                    match buf.read_events(&mut out) {
                        Ok(events) => {
                            if events.read > 0 {
                                idle = false;
                            }
                            if events.lost > 0 && tx.send(Msg::Lost(events.lost as u64)).is_err() {
                                return;
                            }
                            for raw in out.iter().take(events.read) {
                                let evt = unsafe {
                                    std::ptr::read_unaligned(raw.as_ptr() as *const ProcessEvent)
                                };
                                if tx.send(Msg::Event(to_recorded(&evt))).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[halcyon] perf buffer error: {e}");
                            idle = true;
                        }
                    }
                }
                if idle {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        })
        .context("failed to spawn reader thread")
}
fn to_recorded(raw: &ProcessEvent) -> RecordedEvent {
    let ts = Local::now().format("%H:%M:%S%.3f").to_string();
    let argv = raw.argv_str();
    let argv_opt = if argv.is_empty() { None } else { Some(argv) };
    match raw.event_type {
        EVENT_EXECVE => RecordedEvent {
            ts,
            kind: Kind::Exec,
            pid: raw.pid,
            uid: raw.uid,
            comm: raw.comm_str(),
            file: None,
            extension: None,
            argv: argv_opt,
            bytes: None,
        },
        EVENT_OPENAT => {
            let filename = raw.filename_str();
            let ext = extract_extension(&filename);
            RecordedEvent {
                ts,
                kind: Kind::Open,
                pid: raw.pid,
                uid: raw.uid,
                comm: raw.comm_str(),
                file: Some(filename),
                extension: Some(ext),
                argv: None,
                bytes: None,
            }
        }
        EVENT_CONNECT => {
            let addr = raw.filename_str();
            RecordedEvent {
                ts,
                kind: Kind::Connect,
                pid: raw.pid,
                uid: raw.uid,
                comm: raw.comm_str(),
                file: if addr.is_empty() { None } else { Some(addr) },
                extension: None,
                argv: None,
                bytes: None,
            }
        }
        EVENT_ACCEPT => {
            let addr = raw.filename_str();
            RecordedEvent {
                ts,
                kind: Kind::Accept,
                pid: raw.pid,
                uid: raw.uid,
                comm: raw.comm_str(),
                file: if addr.is_empty() { None } else { Some(addr) },
                extension: None,
                argv: None,
                bytes: None,
            }
        }
        EVENT_SENDTO => {
            let addr = raw.filename_str();
            let bytes_str = raw.argv_str();
            RecordedEvent {
                ts,
                kind: Kind::SendTo,
                pid: raw.pid,
                uid: raw.uid,
                comm: raw.comm_str(),
                file: if addr.is_empty() { None } else { Some(addr) },
                extension: None,
                argv: None,
                bytes: if bytes_str.is_empty() {
                    None
                } else {
                    Some(bytes_str)
                },
            }
        }
        EVENT_RECVFROM => {
            let addr = raw.filename_str();
            let bytes_str = raw.argv_str();
            RecordedEvent {
                ts,
                kind: Kind::RecvFrom,
                pid: raw.pid,
                uid: raw.uid,
                comm: raw.comm_str(),
                file: if addr.is_empty() { None } else { Some(addr) },
                extension: None,
                argv: None,
                bytes: if bytes_str.is_empty() {
                    None
                } else {
                    Some(bytes_str)
                },
            }
        }
        _ => RecordedEvent {
            ts,
            kind: Kind::Exec,
            pid: raw.pid,
            uid: raw.uid,
            comm: raw.comm_str(),
            file: None,
            extension: None,
            argv: None,
            bytes: None,
        },
    }
}

fn cstr_to_string(arr: &[u8]) -> String {
    let bytes: Vec<u8> = arr.iter().take_while(|&&c| c != 0).copied().collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
impl Monitor {
    pub(crate) fn dummy() -> Self {
        let (_tx, rx) = mpsc::channel();
        let handle = thread::spawn(|| loop {
            thread::sleep(Duration::from_secs(60));
        });
        let now = Instant::now();
        Self {
            rx,
            threshold: 3,
            stats: HashMap::new(),
            windows: HashMap::new(),
            pid_to_ppid: HashMap::new(),
            file_counts: HashMap::new(),
            ext_counts: HashMap::new(),
            rate_history: VecDeque::new(),
            tick_execs: 0,
            tick_opens: 0,
            tick_alerts: 0,
            tick_start: now,
            total_events: 0,
            total_lost: 0,
            started: now,
            _reader: handle,
            _bpf: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(pid: u32, uid: u32, file: &str) -> RecordedEvent {
        RecordedEvent {
            ts: "00:00:00.000".into(),
            kind: Kind::Open,
            pid,
            uid,
            comm: "probe".into(),
            file: Some(file.into()),
            extension: Some(extract_extension(file)),
            argv: None,
            bytes: None,
        }
    }

    #[test]
    fn cstr_to_string_stops_at_nul() {
        let mut buf = [b'a'; EVENT_FILENAME_LEN];
        buf[3] = 0;
        let s = cstr_to_string(&buf);
        assert_eq!(s, "aaa");
        assert!(!s.contains('\0'));
    }

    #[test]
    fn process_event_string_helpers_trim_nul_padding() {
        let mut ev = ProcessEvent {
            event_type: 1,
            pid: 7,
            uid: 1000,
            comm: [0u8; EVENT_COMM_LEN],
            filename: [0u8; EVENT_FILENAME_LEN],
            argv: [0u8; EVENT_ARGV_LEN],
        };
        ev.comm[..4].copy_from_slice(b"bash");
        ev.filename[..11].copy_from_slice(b"/etc/passwd");
        assert_eq!(ev.comm_str(), "bash");
        assert_eq!(ev.filename_str(), "/etc/passwd");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // miri has no /proc filesystem
    fn exec_events_update_stats_without_alerts() {
        let mut monitor = Monitor::dummy();
        let mut outputs = Vec::new();
        monitor.handle_event(
            &RecordedEvent {
                ts: "00:00:00.000".into(),
                kind: Kind::Exec,
                pid: 9,
                uid: 0,
                comm: "init".into(),
                file: None,
                extension: None,
                argv: None,
                bytes: None,
            },
            &mut outputs,
        );
        let stats = monitor.stats.get(&9).expect("stats recorded");
        assert_eq!(stats.total_execs, 1);
        assert_eq!(stats.total_opens, 0);
        assert_eq!(stats.alerts, 0);
        assert!(matches!(outputs[0], Output::Event(_)));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // calls read_ppid_from_proc → /proc
    fn opens_trigger_alert_at_threshold() {
        let mut monitor = Monitor::dummy(); // threshold = 3
        let mut outputs = Vec::new();
        for _ in 0..3 {
            monitor.handle_event(&open(42, 1000, "/tmp/x"), &mut outputs);
        }
        let alerts: Vec<_> = outputs
            .iter()
            .filter(|o| matches!(o, Output::Alert(_)))
            .collect();
        assert_eq!(alerts.len(), 1, "exactly one alert at the threshold");
        if let Output::Alert(a) = &outputs[2] {
            assert_eq!(a.pid, 42);
            assert_eq!(a.opens, 3);
        } else {
            panic!("third output must be the alert");
        }
        let stats = monitor.stats.get(&42).unwrap();
        assert_eq!(stats.alerts, 1);
        assert_eq!(stats.window_opens, 3);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // calls read_ppid_from_proc → /proc
    fn no_second_alert_above_threshold() {
        let mut monitor = Monitor::dummy(); // threshold = 3
        let mut outputs = Vec::new();
        for _ in 0..5 {
            monitor.handle_event(&open(42, 1000, "/tmp/x"), &mut outputs);
        }
        let alerts = outputs
            .iter()
            .filter(|o| matches!(o, Output::Alert(_)))
            .count();
        assert_eq!(alerts, 1, "alert fires once, not repeatedly");
        assert_eq!(monitor.stats.get(&42).unwrap().alerts, 1);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // calls read_ppid_from_proc → /proc
    fn stats_sorted_orders_by_window_opens_desc() {
        let mut monitor = Monitor::dummy();
        let mut outputs = Vec::new();
        for _ in 0..2 {
            monitor.handle_event(&open(1, 0, "/a"), &mut outputs);
        }
        for _ in 0..4 {
            monitor.handle_event(&open(2, 0, "/b"), &mut outputs);
        }
        let sorted = monitor.stats_sorted();
        assert_eq!(sorted[0].pid, 2);
        assert_eq!(sorted[0].window_opens, 4);
        assert_eq!(sorted[1].pid, 1);
        assert_eq!(sorted[1].window_opens, 2);
    }

    #[test]
    fn extract_extension_basic() {
        assert_eq!(extract_extension("/etc/passwd"), "");
        assert_eq!(extract_extension("/tmp/doc.pdf"), "pdf");
        assert_eq!(extract_extension("/home/user/file.tar.gz"), "gz");
        assert_eq!(extract_extension("/a/.hidden"), "");
    }

    #[test]
    fn shannon_entropy_range() {
        assert_eq!(shannon_entropy(""), 0.0);
        let e = shannon_entropy("abcdefghij");
        assert!(e > 0.0 && e <= 1.0);
        // Uniform single-char string has 0 entropy.
        assert_eq!(shannon_entropy("aaaa"), 0.0);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // calls read_ppid_from_proc → /proc
    fn top_files_sorted_by_count() {
        let mut monitor = Monitor::dummy();
        let mut outputs = Vec::new();
        for _ in 0..3 {
            monitor.handle_event(&open(1, 0, "/a.txt"), &mut outputs);
        }
        for _ in 0..5 {
            monitor.handle_event(&open(1, 0, "/b.log"), &mut outputs);
        }
        let top = monitor.top_files(10);
        assert_eq!(top[0].path, "/b.log");
        assert_eq!(top[0].count, 5);
        assert_eq!(top[1].path, "/a.txt");
        assert_eq!(top[1].count, 3);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // calls read_ppid_from_proc → /proc
    fn extension_counts_aggregate() {
        let mut monitor = Monitor::dummy();
        let mut outputs = Vec::new();
        monitor.handle_event(&open(1, 0, "/a.pdf"), &mut outputs);
        monitor.handle_event(&open(1, 0, "/b.pdf"), &mut outputs);
        monitor.handle_event(&open(1, 0, "/c.rs"), &mut outputs);
        let exts = monitor.extension_counts();
        assert_eq!(exts.get("pdf"), Some(&2));
        assert_eq!(exts.get("rs"), Some(&1));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // calls read_ppid_from_proc → /proc
    fn process_tree_builds_hierarchy() {
        let mut monitor = Monitor::dummy();
        let mut outputs = Vec::new();
        // Manually inject PPID mappings (bypass /proc since test PIDs don't exist).
        monitor.pid_to_ppid.insert(1, 0); // systemd → root
        monitor.pid_to_ppid.insert(100, 1); // sshd → systemd
        monitor.pid_to_ppid.insert(200, 1); // bash → systemd
        monitor.pid_to_ppid.insert(300, 200); // vim → bash

        for pid in &[1, 100, 200, 300] {
            monitor.handle_event(
                &RecordedEvent {
                    ts: "00:00:00.000".into(),
                    kind: Kind::Exec,
                    pid: *pid,
                    uid: 0,
                    comm: match pid {
                        1 => "systemd".into(),
                        100 => "sshd".into(),
                        200 => "bash".into(),
                        300 => "vim".into(),
                        _ => unreachable!(),
                    },
                    file: None,
                    extension: None,
                    argv: None,
                    bytes: None,
                },
                &mut outputs,
            );
        }

        let tree = monitor.build_process_tree();
        // Should have 1 root (systemd)
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].pid, 1);
        assert_eq!(tree[0].comm, "systemd");
        // systemd has 2 children (sshd, bash)
        assert_eq!(tree[0].children.len(), 2);
        // bash has 1 child (vim)
        let bash = tree[0].children.iter().find(|c| c.comm == "bash").unwrap();
        assert_eq!(bash.children.len(), 1);
        assert_eq!(bash.children[0].comm, "vim");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // calls read_ppid_from_proc → /proc
    fn flatten_tree_depth_ordering() {
        let mut monitor = Monitor::dummy();
        let mut outputs = Vec::new();
        monitor.pid_to_ppid.insert(1, 0);
        monitor.pid_to_ppid.insert(2, 1);
        monitor.pid_to_ppid.insert(3, 2);

        for pid in &[1, 2, 3] {
            monitor.handle_event(
                &RecordedEvent {
                    ts: "00:00:00.000".into(),
                    kind: Kind::Exec,
                    pid: *pid,
                    uid: 0,
                    comm: format!("proc{pid}"),
                    file: None,
                    extension: None,
                    argv: None,
                    bytes: None,
                },
                &mut outputs,
            );
        }

        let tree = monitor.build_process_tree();
        let flat = Monitor::flatten_tree(&tree);
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].0, 0); // depth 0
        assert_eq!(flat[1].0, 1); // depth 1
        assert_eq!(flat[2].0, 2); // depth 2
    }

    #[test]
    #[cfg_attr(miri, ignore)] // calls read_ppid_from_proc → /proc
    fn multiple_roots() {
        let mut monitor = Monitor::dummy();
        let mut outputs = Vec::new();
        // Two unrelated roots.
        monitor.pid_to_ppid.insert(10, 0);
        monitor.pid_to_ppid.insert(20, 0);

        for pid in &[10, 20] {
            monitor.handle_event(
                &RecordedEvent {
                    ts: "00:00:00.000".into(),
                    kind: Kind::Exec,
                    pid: *pid,
                    uid: 0,
                    comm: format!("root{pid}"),
                    file: None,
                    extension: None,
                    argv: None,
                    bytes: None,
                },
                &mut outputs,
            );
        }

        let tree = monitor.build_process_tree();
        assert_eq!(tree.len(), 2);
    }
}
