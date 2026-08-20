# Halcyon Process Monitor — Architecture

This document describes the internal architecture of Halcyon Process Monitor:
the kernel-side eBPF programs, the userspace event pipeline, the sliding-window
alerting heuristic, and the output layer. It is intended for developers
extending or debugging the project.

---

## 1. System overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              KERNEL SPACE                                    │
│                                                                              │
│   syscall entry           eBPF tracepoint programs            map            │
│  ┌───────────┐   ┌──────────────────────────────────┐   ┌──────────────┐    │
│  │ execve    │──►│ process-monitor-ebpf             │──►│   EVENTS     │    │
│  │ openat    │   │  #[tracepoint] sys_enter_execve  │   │ PerfEventArray│   │
│  └───────────┘   │  #[tracepoint] sys_enter_openat  │   └──────┬───────┘    │
│                  └──────────────────────────────────┘          │             │
└─────────────────────────────────────────────────────────────────┼───────────┘
                                                                  │ per-CPU
                                                                  │ perf buffers
┌─────────────────────────────────────────────────────────────────▼───────────┐
│                             USERSPACE                                        │
│                                                                              │
│   ┌────────────────────┐   MPSC   ┌───────────────────────────────────────┐  │
│   │ halcyon-reader     │ channel  │ Monitor                               │  │
│   │ thread             │─────────►│  • sliding window per PID (1 s)       │  │
│   │  • open perf buf   │          │  • threshold alerting                 │  │
│   │  • decode events   │          │  • per-process stats                  │  │
│   └────────────────────┘          └──────────────┬────────────────────────┘  │
│                                                   │ Output::Event/Alert     │
│                         ┌─────────────────────────┴──────────────────┐       │
│                         │        Output layer (main thread)          │       │
│                         │  TUI (ratatui) │ JSON │ Plain │ Diagnose    │       │
│                         └────────────────────────────────────────────┘       │
└───────────────────────────────────────────────────────────────────────────────┘
```

Two crates form the workspace:

| Crate | Role | Toolchain |
|---|---|---|
| `process-monitor-ebpf` | Kernel-side tracepoint programs, `#![no_std]`, aya-ebpf | Rust **nightly** (`-Z build-std`) |
| `process-monitor` | Userspace: loader, reader thread, monitor core, output modes | Rust **stable** |

---

## 2. Kernel side — `process-monitor-ebpf`

### 2.1 Program layout

`src/main.rs` defines two tracepoint programs and one map:

```
sys_enter_execve ──┐
                   ├──► EVENTS: PerfEventArray<ProcessEvent>
sys_enter_openat ──┘
```

Both programs run in **tracepoint context** on syscall entry, before the
kernel copies arguments, so all pointers to userspace memory are read with
`bpf_probe_read_user` — never dereferenced. This is what keeps the code
verifier-safe and immune to TOCTOU-style kernel pointer hazards.

### 2.2 Event record

The kernel and userspace agree on a fixed, `#[repr(C)]` layout so records can
be memcpy'd across the perf buffer without serialization:

```rust
pub struct ProcessEvent {
    pub event_type: u8,             // 0 = EXEC, 1 = OPEN
    pub pid: u32,
    pub uid: u32,
    pub comm: [u8; 16],             // process comm (truncated)
    pub filename: [u8; 64],         // target path (truncated)
}
```

> **Size note:** `u8 + u32 + u32 + [u8;16] + [u8;64]` — an **85-byte payload**
> that occupies **92 bytes on the wire** (3 alignment-padding bytes before the
> first `u32`). Keeping the record small and fixed-size is what makes per-CPU
> perf buffering cheap — no allocation, no variable-length encoding in kernel
> context.

`event_type` distinguishes `execve` from `openat`; the userspace side maps it
to the `Kind::{Exec, Open}` enum.

### 2.3 Map

The `EVENTS` map is a `PerfEventArray` (one ring buffer per CPU). The kernel
program selects the current CPU's buffer implicitly; the userspace side opens
one reader per online CPU.

---

## 3. Userspace — `process-monitor`

### 3.1 Startup sequence (`Monitor::start`)

1. **Privilege check** — bails unless `geteuid() == 0`
   (`CAP_BPF` / `CAP_SYS_ADMIN` required).
2. **Object load** — `aya::Ebpf::load_file` parses the compiled eBPF object.
3. **Program load + attach** — each `TracePoint` program is loaded and attached
   to `syscalls/sys_enter_execve` and `syscalls/sys_enter_openat`.
4. **Map hand-off** — the `EVENTS` `PerfEventArray` is taken from the object
   and moved into the reader thread.
5. **Channel** — an MPSC channel connects the reader thread to the monitor.

### 3.2 Reader thread (`spawn_reader`, named `halcyon-reader`)

- Enumerates online CPUs and opens a `PerfEventArrayBuffer` per CPU.
- Loop: `read_events` on every buffer; batches are decoded into pre-allocated
  `BytesMut` pools (`OUT_BUFS` × `OUT_BUF_CAP`) to avoid per-event allocation.
- Counts `events.lost` (perf-buffer overruns) and forwards `Msg::Lost`.
- Decodes raw bytes into `RecordedEvent` via `to_recorded` (unsafe
  `read_unaligned` + `cstr_to_string` for the fixed-size byte arrays).
- Idles 1 ms when no buffer had events — a polling design with ~1 ms latency
  and near-zero CPU when idle.

> The workspace release profile sets `panic = "abort"`, so a panic in this
> thread aborts the process loudly instead of silently degrading to
> "events 0" forever.

### 3.3 Monitor core

Holds:

```
stats:        HashMap<u32, ProcStats>        // pid → cumulative stats
windows:      HashMap<u32, VecDeque<Instant>> // pid → open timestamps (1 s window)
file_counts:  HashMap<String, u64>            // path → total open count
ext_counts:   HashMap<String, u64>            // extension → total open count
rate_history: VecDeque<RateSample>            // per-second event counts (sparklines)
tick_execs/opens/alerts: u64                 // accumulated since last tick
```

`handle_event`:

1. Records the event into `ProcStats` (totals per kind, extension map).
2. For `Open` events, pushes the timestamp onto the PID's sliding window and
   evicts entries older than `WINDOW_SECS` (1 s).
3. Increments `file_counts` and `ext_counts` for file-extension tracking.
4. **Alert trigger:** when the window length reaches exactly `threshold`, an
   `Alert` is emitted (once per crossing; further opens keep counting). A
   threshold of `0` disables alerting entirely.

Every ~1 second, `poll()` flushes the tick counters into a `RateSample`
appended to `rate_history` (capped at 120 seconds). This powers the TUI's
sparkline rate charts.

`stats_sorted` returns processes ranked by current `opens/s` for the TUI's
"Top processes" panel. `top_files(n)` returns the N most-opened files with
normalised Shannon entropy scores. `extension_counts` feeds the "File Types"
panel. `uptime` and `total_lost` feed the status bar.

### 3.4 Output layer

`Monitor::poll` returns a `Vec<Output>` per tick, where

```rust
enum Output {
    Event(RecordedEvent),
    Alert(Alert),
}
```

The main thread routes these by mode:

| Mode | Implementation |
|---|---|
| **TUI** | `tui.rs` — cyberpunk-themed `ratatui` layout: event log (scrollable), sparkline rate charts, top-processes table with mini-bars, file-type frequency panel, top-files leaderboard with entropy, alerts panel, status bar |
| **JSON** | `run_json` — one JSON object per line: events and alerts (see schema in `README.md`) |
| **Plain** | `run_plain` — human-readable timestamped lines |
| **Diagnose** | `run_diagnose` — verifies tracepoint IDs under `/sys/kernel/tracing/events`, loads + attaches, listens 5 s, prints counters |

Mode selection: `--tui` forces the TUI; otherwise `--json` / `--plain` select
their modes; with no flags and a TTY stdout, the TUI is the default (non-TTY
stdout defaults to plain, keeping pipes clean).

The TUI uses a 3-column layout:
- **Left (40%)**: scrollable event log with colour-coded EXEC/OPEN/ALERT tags
- **Middle (30%)**: top-processes table (top) + file-type frequency bars (bottom)
- **Right (30%)**: top-files leaderboard with entropy scores (top) + alerts (bottom)

Sparkline charts for exec/s, open/s, and alert/s are rendered above the body,
using a 120-second rolling window of per-second rate samples.

Signal handling (`install_signal_handler`) sets an atomic `QUIT` flag on
`SIGINT`/`SIGTERM` so loops exit cleanly and buffers flush.

---

## 4. Alerting heuristic

The ransomware heuristic is deliberately simple and observable:

> **For every PID, keep a 1-second sliding window of `openat` calls. If the
> window contains ≥ N opens (default 50), emit an alert.**

Design decisions:

- **Sliding window, not a rate counter** — a burst of 50 opens in 100 ms is
  just as alarming as 50 opens spread over a full second, and the window
  captures both without hysteresis or smoothing artifacts.
- **Per-process isolation** — `Cache2 I/O` hammering the Firefox cache does
  not alert because of what the browser is doing; it alerts on its *own* rate
  crossing the threshold.
- **Threshold `0` disables** the heuristic entirely for quiet deployments.
- **Fixed 1 s window** keeps memory bounded: each tracked PID holds at most a
  handful of `Instant`s, and eviction is amortized O(1).

---

## 5. Data flow summary

```
kernel                  userspace reader            monitor core                        output
──────────              ─────────────────          ─────────────                        ──────
openat entry   ──► EVENTS map ──► perf buffer ──► Msg::Event ──► sliding window     ──► TUI / JSON / plain
                                  (per CPU)           │          ├─ file_counts     ──► top-files panel
                                                      │          ├─ ext_counts      ──► file-types panel
                                                      │          ├─ rate_history    ──► sparkline charts
                                                      │          └─ Alert (on threshold) ──► alerts panel
                                                      └─ Msg::Lost ──► lost counter ──► status bar
```

---

## 6. Performance characteristics

| Aspect | Design |
|---|---|
| Kernel overhead | Two tracepoint programs; fixed-size record; no allocation |
| Userspace decode | Pre-allocated `BytesMut` pools; zero per-event allocation in the hot loop |
| Latency | Reader polls perf buffers continuously; events typically visible in <1 ms |
| Idle CPU | Reader sleeps 1 ms when no buffers have data |
| Memory | Sliding window evicts old entries every poll; maps bounded by live PIDs; rate_history capped at 120 samples |
| Binary | Full LTO + `strip = "symbols"` + `panic = "abort"` release profile |

---

## 7. Extending the project

### Add a syscall

1. Add a `#[tracepoint]` program in `process-monitor-ebpf/src/main.rs`,
   reading the new syscall's arguments with `bpf_probe_read_user` and pushing
   a `ProcessEvent` (reuse or extend `event_type`).
2. Load + attach it in `Monitor::start`.
3. Map the new `event_type` in `to_recorded` and, if needed, extend
   `RecordedEvent` / JSON output.

### Change the alert heuristic

- Threshold: CLI flag `--alert-threshold` (default 50).
- Window length: `WINDOW_SECS` in `monitor.rs`.
- Trigger semantics: the exact `== threshold` comparison in `handle_event`.

### Add an output mode

Add a variant to `Output` handling in `main.rs`, mirror `run_json` /
`run_plain`, and extend the mode-selection logic plus `Args` in the CLI.
