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

const EVENT_COMM_LEN: usize = 16;
const EVENT_FILENAME_LEN: usize = 64;
const WINDOW_SECS: u64 = 1;
const OUT_BUFS: usize = 128;
const OUT_BUF_CAP: usize = 4096;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProcessEvent {
    pub event_type: u8,
    pub pid: u32,
    pub uid: u32,
    pub comm: [u8; EVENT_COMM_LEN],
    pub filename: [u8; EVENT_FILENAME_LEN],
}

unsafe impl aya::Pod for ProcessEvent {}

impl ProcessEvent {
    fn comm_str(&self) -> String {
        cstr_to_string(&self.comm)
    }
    fn filename_str(&self) -> String {
        cstr_to_string(&self.filename)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Exec,
    Open,
}

#[derive(Debug, Clone)]
pub struct RecordedEvent {
    pub ts: String,
    pub kind: Kind,
    pub pid: u32,
    pub uid: u32,
    pub comm: String,
    pub file: Option<String>,
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
    pub comm: String,
    pub window_opens: u64,
    pub total_opens: u64,
    pub total_execs: u64,
    pub alerts: u64,
}

pub enum Output {
    Event(RecordedEvent),
    Alert(Alert),
}

enum Msg {
    Event(RecordedEvent),
    Lost(u64),
}

pub struct Monitor {
    rx: Receiver<Msg>,
    pub threshold: u64,
    stats: HashMap<u32, ProcStats>,
    windows: HashMap<u32, VecDeque<Instant>>,
    pub total_events: u64,
    pub total_lost: u64,
    pub started: Instant,
    _reader: thread::JoinHandle<()>,
    /// The loaded eBPF object MUST stay alive for the whole monitor lifetime:
    /// when it is dropped, its programs and their attach links are dropped too,
    /// which detaches the tracepoints and silently stops all events.
    _bpf: Option<Ebpf>,
}

impl Monitor {
    pub fn start(bpf_path: &Path, threshold: u64) -> Result<Self> {
        if unsafe { libc::geteuid() } != 0 {
            bail!(
                "must be run as root: loading eBPF programs requires CAP_BPF / CAP_SYS_ADMIN"
            );
        }

        eprintln!("[halcyon] loading eBPF object: {}", bpf_path.display());
        let mut bpf = Ebpf::load_file(bpf_path).context("failed to load eBPF program")?;
        eprintln!("[halcyon] object parsed OK; programs: {:?}", bpf.programs().map(|(n, _)| n).collect::<Vec<_>>());

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

        Ok(Self {
            rx,
            threshold,
            stats: HashMap::new(),
            windows: HashMap::new(),
            total_events: 0,
            total_lost: 0,
            started: Instant::now(),
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
        outputs
    }

    pub(crate) fn handle_event(&mut self, ev: &RecordedEvent, outputs: &mut Vec<Output>) {
        let stats = self.stats.entry(ev.pid).or_default();
        stats.pid = ev.pid;
        stats.comm = ev.comm.clone();
        match ev.kind {
            Kind::Exec => {
                stats.total_execs += 1;
            }
            Kind::Open => {
                let now = Instant::now();
                let window = self.windows.entry(ev.pid).or_default();
                let cutoff = now - Duration::from_secs(WINDOW_SECS);
                while window.front().is_some_and(|t| *t < cutoff) {
                    window.pop_front();
                }
                window.push_back(now);

                stats.total_opens += 1;
                stats.window_opens = window.len() as u64;

                if self.threshold > 0 && stats.window_opens == self.threshold {
                    stats.alerts += 1;
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

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }
}

fn spawn_reader(
    mut perf_array: PerfEventArray<MapData>,
    tx: mpsc::Sender<Msg>,
) -> Result<thread::JoinHandle<()>> {
    let cpus = online_cpus().map_err(|(err, io)| anyhow::anyhow!("failed to enumerate CPUs: {err}: {io}"))?;
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
            // NOTE: the workspace release profile sets `panic = "abort"`, so a
            // panic here aborts the whole process loudly (visible in the
            // terminal) instead of dying silently with "events 0" forever.
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
    match raw.event_type {
        EVENT_EXECVE => RecordedEvent {
            ts,
            kind: Kind::Exec,
            pid: raw.pid,
            uid: raw.uid,
            comm: raw.comm_str(),
            file: None,
        },
        EVENT_OPENAT => RecordedEvent {
            ts,
            kind: Kind::Open,
            pid: raw.pid,
            uid: raw.uid,
            comm: raw.comm_str(),
            file: Some(raw.filename_str()),
        },
        _ => RecordedEvent {
            ts,
            kind: Kind::Exec,
            pid: raw.pid,
            uid: raw.uid,
            comm: raw.comm_str(),
            file: None,
        },
    }
}

fn cstr_to_string(arr: &[u8]) -> String {
    let bytes: Vec<u8> = arr
        .iter()
        .take_while(|&&c| c != 0)
        .copied()
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
impl Monitor {
    pub(crate) fn dummy() -> Self {
        let (_tx, rx) = mpsc::channel();
        let handle = thread::spawn(|| loop {
            thread::sleep(Duration::from_secs(60));
        });
        Self {
            rx,
            threshold: 3,
            stats: HashMap::new(),
            windows: HashMap::new(),
            total_events: 0,
            total_lost: 0,
            started: Instant::now(),
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
        }
    }

    #[test]
    fn cstr_to_string_stops_at_nul() {
        let mut buf = [b'a'; EVENT_FILENAME_LEN];
        buf[3] = 0; // terminate early
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
        };
        ev.comm[..4].copy_from_slice(b"bash");
        ev.filename[..11].copy_from_slice(b"/etc/passwd");
        assert_eq!(ev.comm_str(), "bash");
        assert_eq!(ev.filename_str(), "/etc/passwd");
    }

    #[test]
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
    fn opens_trigger_alert_at_threshold() {
        let mut monitor = Monitor::dummy(); // threshold = 3
        let mut outputs = Vec::new();
        for _ in 0..3 {
            monitor.handle_event(&open(42, 1000, "/tmp/x"), &mut outputs);
        }
        let alerts: Vec<_> = outputs.iter().filter(|o| matches!(o, Output::Alert(_))).collect();
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
    fn no_second_alert_above_threshold() {
        let mut monitor = Monitor::dummy(); // threshold = 3
        let mut outputs = Vec::new();
        for _ in 0..5 {
            monitor.handle_event(&open(42, 1000, "/tmp/x"), &mut outputs);
        }
        let alerts = outputs.iter().filter(|o| matches!(o, Output::Alert(_))).count();
        assert_eq!(alerts, 1, "alert fires once, not repeatedly");
        assert_eq!(monitor.stats.get(&42).unwrap().alerts, 1);
    }

    #[test]
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
}
