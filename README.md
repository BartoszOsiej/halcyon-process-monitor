<img src="https://capsule-render.vercel.app/api?type=waving&color=gradient&customColorList=6,11,20&height=130&section=header&text=halcyon-process-monitor&fontSize=32&animation=fadeIn" width="100%" />

<div align="center">

[![Typing SVG](https://readme-typing-svg.demolab.com/?font=JetBrains+Mono&weight=600&size=18&duration=3000&pause=1200&color=58A6FF&center=true&vCenter=true&width=600&height=45&lines=eBPF%20ransomware%20tracker%20%E2%80%94%20kernel%20execve%2Fopenat%20tracing%2C%20per-CPU%20perf%20buffers%2C%20ratatui%20TUI%2C%20sliding-window%20alerts)](https://github.com/BartoszOsiej/halcyon-process-monitor)

</div># 🔬 Halcyon Process Monitor

![License](https://img.shields.io/badge/License-MIT-green?style=flat-square)
![Rust](https://img.shields.io/badge/Rust-2021-DEA584?style=flat-square&logo=rust)
![crates.io](https://img.shields.io/crates/v/process-monitor?style=flat-square&label=process-monitor&logo=rust)
![eBPF](https://img.shields.io/badge/eBPF-Linux%205.8+-FCD900?style=flat-square&logo=linux)
![Go](https://img.shields.io/badge/Go-1.22-00ADD8?style=flat-square&logo=go)
![Docker](https://img.shields.io/badge/Docker-GHCR-2496ED?style=flat-square&logo=docker)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/BartoszOsiej/halcyon-process-monitor/badge)](https://scorecard.dev/viewer/?uri=github.com/BartoszOsiej/halcyon-process-monitor)

**Real-time, eBPF-based process, file-operation, and network telemetry for Linux.**

Halcyon Process Monitor traces `execve`, `openat`, `connect`, `accept`, `sendto`, and `recvfrom` syscalls at the kernel level using eBPF tracepoints, streams the events into userspace through per-CPU perf buffers, and surfaces them in a live terminal TUI — while continuously scoring per-process file-open rates against a sliding window to flag ransomware-style mass file access.

> 🇵🇱 [Wersja polska](README.pl.md) · [Documentation](https://bartoszosiej.github.io/Docs/projects/halcyon-process-monitor/) · [Architecture](ARCHITECTURE.md) · [New Features](NEW_FEATURES.md)

---

## Table of Contents

- [Features](#features)
- [Architecture](#architecture)
- [Requirements](#requirements)
- [Quick Start](#quick-start)
- [Usage](#usage)
- [Build Variants](#build-variants)
- [TUI Controls](#tui-controls)
- [Network Tracing](#network-tracing)
- [Web Dashboard](#web-dashboard)
- [Go Agent](#go-agent)
- [C FFI Library](#c-ffi-library)
- [Kubernetes](#kubernetes)
- [JSON Output](#json-output)
- [How It Works](#how-it-works)
- [Project Structure](#project-structure)
- [Security](#security)
- [Docker](#docker)
- [Troubleshooting](#troubleshooting)
- [License](#license)

---

## Features

| Capability | Description |
|---|---|
| **Kernel-level tracing** | `execve`, `openat`, `connect`, `accept`, `sendto`, `recvfrom` tracepoints |
| **Verifier-safe** | Userspace pointers read via `bpf_probe_read_user` — never dereferenced |
| **Zero-copy pipeline** | Fixed-size `ProcessEvent` through per-CPU `PerfEventArray` |
| **Ultra-advanced TUI** | 7 panels: Events, Processes, Network, TopFiles, Extensions, Alerts, Heatmap |
| **Search & filter** | Real-time text search across all events (`/` to activate) |
| **Process detail view** | Full hierarchical process tree with stats (`Enter` to open) |
| **Help overlay** | Complete keybinding reference (`?` to show) |
| **Pane resize** | Drag column borders with `[`/`]` and `{`/`}` |
| **Network panel** | Live view of connect/accept/sendto/recvfrom with IP:port |
| **Sparkline charts** | Real-time exec/s, open/s, net/s, alert/s (120s rolling window) |
| **Heatmap** | Syscall frequency visualization (exec, open, network, alerts) |
| **File-type tracking** | Per-extension open frequency with colour-coded categories |
| **Top-files leaderboard** | Most-accessed files with Shannon entropy scores |
| **Sliding-window heuristic** | 1-second rolling window per PID; alerts at configurable threshold |
| **Multiple output modes** | Human TUI, JSON, plain text, self-diagnostic, web dashboard |
| **Web dashboard** | REST API + WebSocket + Prometheus metrics (optional build) |
| **Go agent** | Lightweight CLI that connects to daemon via HTTP/WebSocket |
| **C FFI library** | `libhalcyon.so` with C bindings for cross-language integration |
| **Kubernetes** | DaemonSet + Service + ConfigMap + ServiceMonitor manifests |
| **Protobuf schema** | gRPC service definition for inter-component communication |
| **Lost-event accounting** | Perf-buffer overruns counted and reported |
| **Single static binary** | Full LTO, `panic = "abort"`, symbol-stripped |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        KERNEL SPACE (eBPF)                      │
│                                                                 │
│  sys_enter_execve ──┐                                           │
│  sys_enter_openat  ──┤                                           │
│  sys_enter_connect ──┤   ProcessEvent    PerfEventArray         │
│  sys_enter_accept  ──┼──► (map)    ────► (per-CPU buffers)     │
│  sys_enter_sendto  ──┤                                           │
│  sys_enter_recvfrom ─┘                                           │
└──────────────────────────────────┬──────────────────────────────┘
                                   │
┌──────────────────────────────────▼──────────────────────────────┐
│                     USERSPACE (Rust)                            │
│                                                                 │
│  reader thread ──► channel ──► Monitor ──► TUI / JSON / Web    │
│       │                  │         │                            │
│       │                  │    sliding window                    │
│       │                  │    + alerting                        │
│       │                  │    + process tree                    │
│       │                  │    + file ranking                    │
│       │                  │    + network tracking                │
│       │                  │    + heatmap                         │
│       └──► perf buffer   └──► search/filter                    │
└─────────────────────────────────────────────────────────────────┘
```

---

## Requirements

| Requirement | Notes |
|---|---|
| Linux kernel **5.8+** | eBPF + tracepoint support |
| **root** (`CAP_BPF` / `CAP_SYS_ADMIN`) | Required to load eBPF programs |
| Rust **nightly** + `rust-src` | Builds eBPF program with `-Z build-std` |
| `bpf-linker`, `clang` | eBPF linking toolchain |
| BTF (`/sys/kernel/btf/vmlinux`) | Recommended for CO-RE |

---

## Quick Start

```bash
# Distro-aware installer (detects apt/dnf/pacman/zypper/apk/xbps)
./install.sh --system    # System-wide to /usr/local
./install.sh             # User-local to ~/.local

# Or build manually
./build.sh               # TUI-only (1.7MB)
./build.sh --all         # Both TUI + web variants

# Run
sudo target/release/process-monitor-tui
```

---

## Build Variants

```bash
# TUI-only (default, 1.7MB) — recommended
./build.sh
# or
cargo build --release

# Web-featured (2.5MB) — with REST API, WebSocket, Prometheus
./build.sh --web
# or
cargo build --release --features web

# Both variants
./build.sh --all
```

### Binary sizes (release)

| Variant | Size | Dependencies |
|---|---|---|
| `process-monitor-tui` | 1.7MB | aya, ratatui, chrono, crossterm |
| `process-monitor-web` | 2.5MB | +axum, tokio, tower-http, prometheus-client |

---

## Usage

```bash
# TUI (default when stdout is a terminal)
sudo process-monitor

# Raise alert threshold
sudo process-monitor --alert-threshold 100

# Filter by file extension
sudo process-monitor --filter-ext pdf

# JSON output for pipelines
sudo process-monitor --json | jq .

# Plain text log (no TUI)
sudo process-monitor --plain

# Self-diagnostic
sudo process-monitor --diagnose

# Web dashboard (requires --features web build)
sudo process-monitor --web 0.0.0.0:8080
```

### CLI Reference

| Flag | Default | Description |
|---|---|---|
| `-b, --bpf <PATH>` | auto | Path to compiled eBPF object |
| `--alert-threshold <N>` | `50` | Alert when N+ files opened within 1s |
| `--filter-ext <EXT>` | all | Filter by file extension |
| `--top-files <N>` | `8` | Top files in TUI |
| `--json` | off | Newline-delimited JSON output |
| `--plain` | off | Plain text log |
| `--diagnose` | off | 5-second self-diagnostic |
| `--web <ADDR>` | off | Start web server (requires `--features web`) |

---

## TUI Controls

### Navigation

| Key | Action |
|---|---|
| `q` / `Esc` | Quit (or clear scroll) |
| `p` | Pause / resume |
| `c` | Clear all panels |
| `↑`/`↓` / `k`/`j` | Scroll up/down |
| `PgUp`/`PgDn` | Page up/down |
| `g` / `Home` | Jump to top |
| `G` / `End` | Jump to bottom |

### Panels

| Key | Action |
|---|---|
| `Tab` | Next panel |
| `Shift+Tab` | Previous panel |
| `1`-`7` | Jump to panel by number |
| `Enter` | Open process detail view |

### Search & Filter

| Key | Action |
|---|---|
| `/` | Start search mode |
| `Esc` | Cancel search |
| `Enter` | Apply filter |

### Layout

| Key | Action |
|---|---|
| `[` / `]` | Resize left pane |
| `{` / `}` | Resize middle pane |

### Other

| Key | Action |
|---|---|
| `?` / `h` | Show help overlay |
| `Ctrl+C` | Force quit |

### TUI Panels (7)

| # | Panel | Description |
|---|---|---|
| 1 | **EVENTS** | Live event log with search/filter |
| 2 | **PROCESSES** | Hierarchical process tree with mini-bars |
| 3 | **NETWORK** | Real-time network connections (connect/accept/send/recv) |
| 4 | **TOP FILES** | Most-opened files with Shannon entropy |
| 5 | **FILE TYPES** | Extension frequency with colored bars |
| 6 | **ALERTS** | Alert history with timestamps |
| 7 | **HEATMAP** | Syscall frequency visualization |

---

## Network Tracing

New in v0.4 — traces network-related syscalls:

| Syscall | Event Type | Captures |
|---|---|---|
| `connect` | `Connect` | Remote IPv4/IPv6/Unix address |
| `accept` | `Accept` | Remote address |
| `sendto` | `SendTo` | Destination + bytes sent |
| `recvfrom` | `RecvFrom` | Source + bytes received |

### Event format

```json
{
  "type": "event",
  "kind": "Connect",
  "pid": 1234,
  "comm": "curl",
  "file": "93.184.216.34:443"
}
```

---

## Web Dashboard

Optional build with `--features web`:

```bash
# Build with web support
cargo build --release --features web

# Start web server
sudo process-monitor --web 0.0.0.0:8080
```

### Endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/` | GET | Cyberpunk web dashboard |
| `/ws` | WebSocket | Live event stream |
| `/api/v1/stats` | GET | Global statistics |
| `/api/v1/processes` | GET | Tracked processes |
| `/api/v1/files` | GET | Top opened files |
| `/api/v1/extensions` | GET | File extension frequency |
| `/api/v1/threshold` | POST | Update threshold at runtime |
| `/metrics` | GET | Prometheus metrics |

---

## Go Agent

Lightweight CLI that connects to the Halcyon daemon:

```bash
cd go-agent
go build -o halcyon-agent .

# Usage
./halcyon-agent stats
./halcyon-agent processes
./halcyon-agent files
./halcyon-agent watch      # WebSocket live events
./halcyon-agent --json stats
```

---

## C FFI Library

C-compatible library for integrating Halcyon with other languages:

```c
#include "halcyon.h"

halcyon_monitor_t* monitor;
halcyon_monitor_create("/path/to/bpf.o", 50, &monitor);

halcyon_event_t events[100];
uint32_t count;
halcyon_monitor_poll(monitor, events, 100, &count);

for (uint32_t i = 0; i < count; i++) {
    printf("[%d] %s\n", events[i].pid, events[i].comm);
    halcyon_free_string(events[i].comm);
}
halcyon_free_events(events, count);
halcyon_monitor_destroy(monitor);
```

---

## Kubernetes

Deploy Halcyon as a DaemonSet on every node:

```bash
kubectl create namespace observability
kubectl apply -f k8s/
```

### Components

| Resource | Description |
|---|---|
| `DaemonSet` | Runs Halcyon on every node with eBPF access |
| `Service` | ClusterIP service for API access |
| `ConfigMap` | Configuration (threshold, log level, etc.) |
| `ServiceMonitor` | Prometheus operator integration |

---

## JSON Output

```jsonc
{"ts": "14:09:16.531", "type": "open", "pid": 29645, "uid": 1000, "comm": "process-monitor", "file": "/dev/tty"}
{"ts": "14:09:16.973", "type": "alert", "pid": 2126, "uid": 1000, "comm": "Cache2 I/O", "opens_in_1s": 50}
{"ts": "14:09:17.100", "type": "connect", "pid": 1234, "uid": 1000, "comm": "curl", "file": "93.184.216.34:443"}
```

---

## How It Works

1. Kernel tracepoints capture PID, UID, `comm`, target filename/address into `ProcessEvent`
2. Reader thread opens perf buffer per CPU, forwards events over MPSC channel
3. Monitor keeps 1-second sliding window per PID, alerts at threshold
4. Output renders: TUI (7 panels), JSON, plain, web dashboard, or diagnostic report

---

## Project Structure

```
halcyon-process-monitor/
├── process-monitor/          # Userspace: monitor core + TUI + web + FFI
│   └── src/
│       ├── main.rs           # CLI, mode selection, signal handling
│       ├── monitor.rs        # eBPF loading, perf reader, sliding window
│       ├── tui.rs            # Ultra-advanced ratatui cyberpunk interface
│       ├── web.rs            # axum web server (optional, --features web)
│       └── ffi.rs            # C FFI bindings (libhalcyon)
├── process-monitor-ebpf/     # Kernel side (#![no_std], aya-ebpf)
│   └── src/
│       ├── main.rs           # execve/openat tracepoints → PerfEventArray
│       └── network.rs        # connect/accept/sendto/recvfrom tracepoints
├── go-agent/                 # Go CLI agent (HTTP/WebSocket client)
├── c-api/                    # C header for libhalcyon
├── k8s/                      # Kubernetes manifests (DaemonSet, Service, etc.)
├── proto/                    # Protobuf schema (gRPC service definition)
├── build.sh                  # Build script (--web, --all variants)
├── install.sh                # Distro-aware installer
├── NEW_FEATURES.md           # Documentation for v0.4 features
└── Cargo.toml                # Workspace definition
```

---

## Security

- Installer makes **no authenticated network requests**
- Dependencies from distribution repos + official `rustup.rs`
- Kernel code follows strict eBPF safety: `bpf_probe_read_user` only
- eBPF programs require root

---

## Docker

```bash
# Build
docker build -t halcyon-process-monitor .

# Run (requires --privileged for eBPF)
docker run --privileged -v /sys/kernel/btf:/sys/kernel/btf \
    halcyon-process-monitor process-monitor
```

---

## Troubleshooting

```bash
sudo process-monitor --diagnose    # 5-second self-diagnostic
```

- **No events** → check tracepoints exist in `/sys/kernel/tracing/events/syscalls/`
- **Failed to load eBPF** → pass `--bpf` explicitly or re-run `./install.sh`
- **Not root** → `CAP_BPF` or `CAP_SYS_ADMIN` required

---

## Uninstall

```bash
./install.sh --uninstall            # user-local
./install.sh --uninstall --system   # system-wide
```

---

## Why?

Because ransomware detection should not require a $50K enterprise solution. Halcyon runs at the kernel level with eBPF, scores file-open rates in real-time, and alerts you in a live TUI — all from a single static binary. Open source, auditable, and free.

---

## License

MIT

---
---

## 📺 Demo

![halcyon Demo](assets/halcyon-demo.gif)