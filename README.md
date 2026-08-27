<img src="https://capsule-render.vercel.app/api?type=waving&color=gradient&customColorList=6,11,20&height=130&section=header&text=halcyon&fontSize=32&animation=fadeIn" width="100%" />

<div align="center">

[![Typing SVG](https://readme-typing-svg.demolab.com/?font=JetBrains+Mono&weight=600&size=18&duration=3000&pause=1200&color=58A6FF&center=true&vCenter=true&width=600&height=45&lines=eBPF+endpoint+security+agent+%E2%80%94+detect+and+respond+at+the+kernel+edge)](https://github.com/BartoszOsiej/halcyon-process-monitor)

</div>

# 🛡️ Halcyon — Endpoint Security Agent

![License](https://img.shields.io/badge/License-MIT-green?style=flat-square)
![Rust](https://img.shields.io/badge/Rust-2021-DEA584?style=flat-square&logo=rust)
![eBPF](https://img.shields.io/badge/eBPF-Linux%205.8+-FCD900?style=flat-square&logo=linux)
![Go](https://img.shields.io/badge/Go-1.22-00ADD8?style=flat-square&logo=go)
![Docker](https://img.shields.io/badge/Docker-GHCR-2496ED?style=flat-square&logo=docker)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/BartoszOsiej/halcyon-process-monitor/badge)](https://scorecard.dev/viewer/?uri=github.com/BartoszOsiej/halcyon-process-monitor)

**eBPF-based endpoint security agent for Linux — detect ransomware behaviour, respond at the kernel edge.**

Halcyon is not a passive monitor. It is a **detect-and-respond** agent that hooks syscalls at the kernel level via eBPF tracepoints, scores per-process file-open rates in real-time, and **terminates** offending processes the instant a heuristic verdict fires. It processes ~500k events/sec through per-CPU perf buffers with zero-copy handoff to a userspace detection engine built in Rust.

> 🇵🇱 [Wersja polska](README.pl.md) · [Architecture](ARCHITECTURE.md)

---

## Table of Contents

- [What It Does](#what-it-does)
- [Architecture / Data Flow](#architecture--data-flow)
- [Network Visibility](#network-visibility)
- [Detection & Response](#detection--response)
- [Storage & Pipeline](#storage--pipeline)
- [Requirements](#requirements)
- [Quick Start](#quick-start)
- [Usage](#usage)
- [Build Variants](#build-variants)
- [TUI Controls](#tui-controls)
- [Web Dashboard](#web-dashboard)
- [Operator View (TUI + Web)](#operator-view-tui--web)
- [Project Structure](#project-structure)
- [Tested Live on Linux](#tested-live-on-linux)
- [Docker / Kubernetes](#docker--kubernetes)
- [License](#license)

---

## What It Does

| Capability | How |
|---|---|
| **Kernel-level tracing** | eBPF tracepoints on `execve`, `openat`, `connect`, `accept`, `sendto`, `recvfrom`, `mkdir`, `unlinkat`, `kill`, `fchmodat` |
| **Ransomware detection** | 1-second sliding window per PID; alerts when file-open rate exceeds configurable threshold |
| **Automated response** | `--auto-kill` sends `SIGKILL` to the offending process on alert verdict |
| **Network egress tracking** | Parses `sockaddr` in-kernel — captures IPv4/IPv6/Unix addresses on connect/accept/send/recv |
| **Event pipeline** | Kernel perf buffer → zero-copy ring → detection engine → TUI / JSON / WebSocket / Prometheus |
| **Process tree** | Resolves PPID from `/proc`, builds hierarchical view with per-process alert counts |
| **File ranking** | Most-opened files with Shannon entropy scoring (detects encrypted/randomised filenames) |
| **Single binary** | Full LTO, `panic = "abort"`, symbol-stripped — 1.7 MB TUI, 2.5 MB with web |
| **Kafka streaming** | Events → Kafka topics with lz4 compression, partitioned by PID |
| **ClickHouse storage** | Batch inserts into MergeTree for analytics retention |
| **MemGraph graph** | Process trees + file access as a graph (Cypher queries) |

---

## Architecture / Data Flow

Halcyon follows a **pipeline architecture** — kernel ingestion → userspace detection → operator response:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                      KERNEL SPACE (eBPF programs)                       │
│                                                                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐                  │
│  │ sys_enter_   │  │ sys_enter_   │  │ sys_enter_   │                  │
│  │ execve       │  │ openat       │  │ connect      │ ... 10 total     │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘                  │
│         │                 │                 │                            │
│         ▼                 ▼                 ▼                            │
│  ┌─────────────────────────────────────────────────────┐               │
│  │  ProcessEvent { pid, uid, comm, filename, argv }    │               │
│  │  PerfEventArray (per-CPU, zero-copy)                │               │
│  └──────────────────────────┬──────────────────────────┘               │
└─────────────────────────────┼───────────────────────────────────────────┘
                              │
┌─────────────────────────────┼───────────────────────────────────────────┐
│                      USERSPACE (Rust)                                   │
│                              │                                           │
│  ┌───────────────────────────▼──────────────────────────┐              │
│  │  Reader thread — reads perf buffers per CPU          │              │
│  │  MPSC channel → Monitor event loop                   │              │
│  └───────────────────────────┬──────────────────────────┘              │
│                              │                                           │
│  ┌───────────────────────────▼──────────────────────────┐              │
│  │  DETECTION ENGINE                                    │              │
│  │  • Sliding window per PID (1s rolling)               │              │
│  │  • File-extension frequency tracking                 │              │
│  │  • Shannon entropy scoring on filenames              │              │
│  │  • Per-process stats (opens, execs, alerts, PPID)    │              │
│  └───────────┬─────────────────────────┬───────────────┘              │
│              │ VERDICT                  │                              │
│              ▼                          ▼                               │
│  ┌─────────────────────┐  ┌────────────────────────────┐              │
│  │  RESPONSE           │  │  OUTPUT                     │              │
│  │  kill(pid, SIGKILL) │  │  TUI (7 panels)            │              │
│  │  cgroup freeze      │  │  JSON / WebSocket           │              │
│  │  (extensible)       │  │  Prometheus /metrics        │              │
│  └─────────────────────┘  │  REST API                   │              │
│                           └────────────────────────────┘              │
└─────────────────────────────────────────────────────────────────────────┘
```

### Pipeline stages

| Stage | Component | Throughput | Mechanism |
|---|---|---|---|
| **1. Ingest** | eBPF tracepoints | ~500k events/s | `bpf_perf_event_output` per-CPU |
| **2. Transport** | PerfEventArray | zero-copy | `PerfEventArrayBuffer::read_events` |
| **3. Detect** | Sliding window engine | real-time | 1s rolling window, configurable threshold |
| **4. Respond** | `kill(2)` / cgroup | < 1ms latency | `SIGKILL` on heuristic verdict |
| **5. Persist** | TUI / JSON / WebSocket | live stream | REST API + Prometheus for retention |

This maps directly to a **Kafka-style** event pipeline: kernel perf buffer = topic, reader thread = consumer, detection engine = stream processor, TUI/API = sink.

---

## Storage & Pipeline

Halcyon supports pluggable storage backends for event persistence and downstream analytics:

```bash
# Stream events to Kafka
sudo halcyon --kafka-brokers localhost:9092 --kafka-topic halcyon-events

# Store events in ClickHouse for analytics
sudo halcyon --clickhouse http://localhost:8123

# Build process relationship graph in MemGraph
sudo halcyon --memgraph http://localhost:7474

# Combine all backends
sudo halcyon \
  --kafka-brokers localhost:9092 --kafka-topic halcyon-events \
  --clickhouse http://localhost:8123 \
  --memgraph http://localhost:7474
```

### Kafka

Events are sent to a configurable topic with `lz4` compression and partitioned by PID for ordering per-process:

| Config | Default | Description |
|---|---|---|
| `--kafka-brokers` | — | Broker address (e.g. `localhost:9092`) |
| `--kafka-topic` | `halcyon-events` | Topic name |

### ClickHouse

Events are batch-inserted into a `MergeTree` table partitioned by date:

```sql
CREATE TABLE halcyon.events (
    ts DateTime64(3),
    kind LowCardinality(String),
    pid UInt32, uid UInt32,
    comm LowCardinality(String),
    file Nullable(String),
    extension LowCardinality(Nullable(String))
) ENGINE = MergeTree()
PARTITION BY toYYYYMMDD(ts)
ORDER BY (ts, kind, pid)
```

### MemGraph

Process trees and file access patterns are stored as a graph:

```cypher
// Find all processes that opened .enc files
MATCH (p:Process)-[r:OPENED]->(f:File)
WHERE f.path ENDS WITH '.enc'
RETURN p.pid, p.comm, f.path, r.count
ORDER BY r.count DESC

// Find exfiltration candidates (file opens + external network)
MATCH (p:Process)-[:OPENED]->(f:File), (p)-[:CONNECTED_TO]->(n:NetworkTarget)
WHERE NOT n.addr STARTS WITH '10.'
RETURN p.pid, p.comm, collect(f.path), collect(n.addr)
```

---

## Network Visibility

Halcyon traces network syscalls at the kernel level — not just file operations. This provides **full egress visibility** for detecting data exfiltration, C2 communication, and lateral movement.

| Syscall | Event Type | What's Captured | How |
|---|---|---|---|
| `connect` | `Connect` | Remote IPv4/IPv6/Unix address + port | `sockaddr` parsed via `bpf_probe_read_user` |
| `accept` | `Accept` | Remote address of incoming connection | Same mechanism |
| `sendto` | `SendTo` | Destination address | `sockaddr` at arg index 4 |
| `recvfrom` | `RecvFrom` | Source address | `sockaddr` at arg index 4 |

### In-kernel sockaddr parsing

The eBPF program reads raw `sockaddr` structures byte-by-byte from userspace:

```c
// Read AF_INET address from sockaddr_in
bpf_probe_read_user(&family, 2, sockaddr_ptr);      // sa_family
bpf_probe_read_user(&port_be, 2, ptr + 2);          // sin_port (big-endian)
bpf_probe_read_user(&a0, 1, ptr + 4);               // sin_addr[0]
// ... formats as "192.168.1.1:443"
```

This runs in the kernel with **zero userspace round-trips** — addresses are resolved before the event even reaches userspace.

### Example: detecting exfiltration

```jsonc
{"ts":"14:09:17.100","type":"event","kind":"Connect","pid":1234,"comm":"curl","file":"93.184.216.34:443"}
{"ts":"14:09:17.205","type":"event","kind":"SendTo","pid":1234,"comm":"curl","file":"93.184.216.34:443"}
{"ts":"14:09:17.502","type":"event","kind":"Open","pid":1234,"comm":"curl","file":"/home/user/Documents/backup.tar.gz"}
```

---

## Detection & Response

### Detection: sliding-window heuristic

Each PID maintains a **1-second rolling window** of `openat` events. When the count hits the threshold (default: 50 opens/s), a verdict fires:

```
PID 2126 ("Cache2 I/O") opened 50 files in 1.0s  →  VERDICT: SUSPICIOUS
```

The threshold is configurable at runtime via the API or CLI:

```bash
# Lower threshold for high-security environments
sudo process-monitor --alert-threshold 20

# Filter by extension (e.g. detect .enc/.pdf mass opens)
sudo process-monitor --filter-ext enc
```

### Response: automated termination

With `--auto-kill`, Halcyon sends `SIGKILL` to the offending process immediately on verdict:

```bash
# EDR mode: detect + respond
sudo process-monitor --alert-threshold 50 --auto-kill
```

```rust
// The response layer — ~30 lines of Rust
fn kill_process(pid: u32) -> bool {
    let rc = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    rc == 0
}

// Fired inside the detection engine on verdict:
if self.auto_kill {
    let result = kill_process(ev.pid);
    outputs.push(Output::Action(ResponseAction {
        ts: ev.ts.clone(),
        pid: ev.pid,
        action: format!("SIGKILL sent to PID {}", ev.pid),
        success: result,
    }));
}
```

This is extensible — the `ResponseAction` interface supports `kill`, cgroup freeze, network quarantine, or any custom response.

---

## Requirements

| Requirement | Notes |
|---|---|
| Linux kernel **5.8+** | eBPF + tracepoint support |
| **root** (`CAP_BPF` / `CAP_SYS_ADMIN`) | Required to load eBPF programs |
| Rust **nightly** + `rust-src` | Builds eBPF with `-Z build-std` |
| `bpf-linker`, `clang` | eBPF toolchain |
| BTF (`/sys/kernel/btf/vmlinux`) | Recommended for CO-RE |

---

## Quick Start

```bash
# Distro-aware installer
./install.sh --system    # System-wide to /usr/local
./install.sh             # User-local to ~/.local

# Or build manually
./build.sh

# Run in EDR mode (detect + auto-respond)
sudo target/release/process-monitor --auto-kill

# Run in monitor-only mode (no auto-kill)
sudo target/release/process-monitor
```

---

## Usage

```bash
# EDR mode — detect and auto-kill
sudo process-monitor --auto-kill

# Lower threshold for stricter detection
sudo process-monitor --auto-kill --alert-threshold 20

# Monitor only (no kill)
sudo process-monitor

# Filter by extension
sudo process-monitor --filter-ext pdf

# JSON output for external pipelines
sudo process-monitor --json | jq .

# Plain text log
sudo process-monitor --plain

# Web dashboard (requires --features web build)
sudo process-monitor --web 0.0.0.0:8080

# Self-diagnostic
sudo process-monitor --diagnose
```

### CLI Reference

| Flag | Default | Description |
|---|---|---|
| `-b, --bpf <PATH>` | auto | Path to compiled eBPF object |
| `--alert-threshold <N>` | `50` | Alert when N+ files opened within 1s |
| `--auto-kill` | off | **Send SIGKILL to processes that trigger alerts** |
| `--filter-ext <EXT>` | all | Filter by file extension |
| `--top-files <N>` | `8` | Top files in TUI |
| `--json` | off | Newline-delimited JSON output |
| `--plain` | off | Plain text log |
| `--diagnose` | off | 5-second self-diagnostic |
| `--web <ADDR>` | off | Start web server (requires `--features web`) |

---

## Build Variants

```bash
# TUI-only (default, 1.7MB)
./build.sh

# Web-featured (2.5MB) — REST API, WebSocket, Prometheus
./build.sh --web

# Both variants
./build.sh --all
```

| Variant | Size | Dependencies |
|---|---|---|
| `process-monitor-tui` | 1.7MB | aya, ratatui, chrono, crossterm |
| `process-monitor-web` | 2.5MB | +axum, tokio, tower-http, prometheus-client |

---

## TUI Controls

| Key | Action |
|---|---|
| `q` / `Esc` | Quit |
| `p` | Pause / resume |
| `c` | Clear all panels |
| `↑`/`↓` / `k`/`j` | Scroll |
| `Tab` | Next panel |
| `1`-`7` | Jump to panel |
| `/` | Search mode |
| `?` / `h` | Help overlay |

### TUI Panels (7)

| # | Panel | Description |
|---|---|---|
| 1 | **EVENTS** | Live event log with search/filter |
| 2 | **PROCESSES** | Hierarchical process tree with alert counts |
| 3 | **NETWORK** | Real-time connections (connect/accept/send/recv + IP:port) |
| 4 | **TOP FILES** | Most-opened files with Shannon entropy |
| 5 | **FILE TYPES** | Extension frequency with coloured bars |
| 6 | **ALERTS** | Alert history + response actions |
| 7 | **HEATMAP** | Syscall frequency visualisation |

---

## Web Dashboard

Optional build with `--features web`:

```bash
cargo build --release --features web
sudo process-monitor --web 0.0.0.0:8080
```

| Endpoint | Method | Description |
|---|---|---|
| `/` | GET | Dashboard UI |
| `/ws` | WebSocket | Live event stream |
| `/api/v1/stats` | GET | Global statistics |
| `/api/v1/processes` | GET | Tracked processes |
| `/api/v1/files` | GET | Top opened files |
| `/api/v1/extensions` | GET | Extension frequency |
| `/api/v1/threshold` | POST | Update threshold at runtime |
| `/metrics` | GET | Prometheus metrics |

---

## Operator View (TUI + Web + Desktop)

Halcyon provides three operator interfaces:

- **TUI** — 7-panel terminal interface for local investigation. Cyberpunk aesthetic, process trees, heatmaps, sparklines. Runs anywhere, no browser needed.
- **Web Dashboard** — browser-based UI with WebSocket live stream, REST API for integration, and Prometheus metrics for Grafana/monitoring stacks.
- **Desktop App (Tauri + React)** — native desktop GUI built with Tauri 2 + React 19 + Recharts. Connects to the halcyon backend via WebSocket and REST API. See [`halcyon-tauri/`](halcyon-tauri/) for source.

All three consume the same detection engine — the agent is **headless-capable** and can run as a background daemon with JSON output piped to external SIEM/storage.

---

## Project Structure

```
halcyon-process-monitor/
├── process-monitor/          # Userspace: detection engine + TUI + web + FFI
│   └── src/
│       ├── main.rs           # CLI, mode selection, signal handling
│       ├── monitor.rs        # eBPF loading, perf reader, detection, response
│       ├── tui.rs            # 7-panel ratatui cyberpunk interface
│       ├── web.rs            # axum web server (--features web)
│       └── ffi.rs            # C FFI bindings (libhalcyon)
├── process-monitor-ebpf/     # Kernel side (#![no_std], aya-ebpf)
│   └── src/
│       ├── main.rs           # execve/openat → PerfEventArray
│       ├── network.rs        # connect/accept/sendto/recvfrom + sockaddr
│       └── fs.rs             # mkdir/unlink/kill/chmod tracepoints
├── go-agent/                 # Go CLI agent (HTTP/WebSocket client)
├── halcyon-tauri/            # Tauri desktop dashboard (React + Rust)
├── c-api/                    # C header for libhalcyon
├── k8s/                      # Kubernetes manifests (DaemonSet, Service)
├── proto/                    # Protobuf schema (gRPC)
├── build.sh                  # Build script
├── install.sh                # Distro-aware installer
└── Cargo.toml                # Workspace definition
```

---

## Tested Live on Linux

Halcyon has been **deployed and tested on real hardware** running Linux:

```bash
# Verify eBPF tracepoints exist
ls /sys/kernel/tracing/events/syscalls/sys_enter_execve/id

# Load and attach eBPF programs
sudo process-monitor --diagnose

# Watch live events in another terminal
ls -la /tmp
# → Halcyon shows: 14:09:16 OPEN [29645] bash → /tmp

# Test auto-kill
sudo process-monitor --alert-threshold 3 --auto-kill
# In another terminal: for i in $(seq 1 100); do touch /tmp/f$i; done
# → Halcyon kills the process after 3 opens in 1s

# Verify with bpftool
bpftool prog list      # shows attached tracepoints
bpftool map dump name events  # shows perf event array
```

---

## Docker / Kubernetes

```bash
# Docker
docker build -t halcyon .
docker run --privileged -v /sys/kernel/btf:/sys/kernel/btf halcyon

# Kubernetes (DaemonSet on every node)
kubectl apply -f k8s/
```

---

## License

MIT

---

## 📺 Demo

![halcyon Demo](assets/halcyon-demo.gif)
