# 🔬 Halcyon Process Monitor

![License](https://img.shields.io/badge/License-MIT-green?style=flat-square)
![Rust](https://img.shields.io/badge/Rust-2021-DEA584?style=flat-square&logo=rust)
![crates.io](https://img.shields.io/crates/v/process-monitor?style=flat-square&label=process-monitor&logo=rust)
![eBPF](https://img.shields.io/badge/eBPF-Linux%205.8+-FCD900?style=flat-square&logo=linux)
![Docker](https://img.shields.io/badge/Docker-GHCR-2496ED?style=flat-square&logo=docker)

**Real-time, eBPF-based process and file-operation telemetry for Linux.**

Halcyon Process Monitor traces `execve` and `openat` syscalls at the kernel
level using eBPF tracepoints, streams the events into userspace through per-CPU
perf buffers, and surfaces them in a live terminal TUI — while continuously
scoring per-process file-open rates against a sliding window to flag
ransomware-style mass file access.

> 🇵🇱 [Wersja polska](README.pl.md) · [Documentation](https://bartoszosiej.github.io/Docs/projects/halcyon-process-monitor/) · [Architecture](ARCHITECTURE.md)

---

## Table of Contents

- [Features](#features)
- [Architecture](#architecture)
- [Requirements](#requirements)
- [Quick Start](#quick-start)
- [Usage](#usage)
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
| **Kernel-level tracing** | `execve` and `openat` tracepoints on every online CPU |
| **Verifier-safe** | Userspace pointers read via `bpf_probe_read_user` — never dereferenced |
| **Zero-copy pipeline** | Fixed-size `ProcessEvent` through per-CPU `PerfEventArray` |
| **Live TUI** | Cyberpunk dashboard: event log, process table, file ranking, sparklines, alerts |
| **Sparkline charts** | Real-time exec/s, open/s, alert/s visualisations (120s rolling window) |
| **File-type tracking** | Per-extension open frequency with colour-coded categories |
| **Top-files leaderboard** | Most-accessed files with Shannon entropy scores |
| **Sliding-window heuristic** | 1-second rolling window per PID; alerts at configurable threshold |
| **Multiple output modes** | Human TUI, JSON, plain text, self-diagnostic |
| **Lost-event accounting** | Perf-buffer overruns counted and reported |
| **Single static binary** | Full LTO, `panic = "abort"`, symbol-stripped |

---

## Architecture

```
                 ┌──────────────────────────────────────────────┐
                 │            Kernel space (eBPF)               │
   execve  ──► sys_enter_execve ─┐                               │
   openat  ──► sys_enter_openat ─┼──► ProcessEvent ──► EVENTS   │
                                 │         (map)     (perf array)│
                 └───────────────────────────┬──────────────────┘
                                             │  per-CPU perf buffers
                 ┌───────────────────────────▼──────────────────┐
                 │           Userspace (Rust)                   │
                 │  reader thread ──► channel ──► Monitor       │
                 │        │                        │            │
                 │        │                  sliding window     │
                 │        │                  + alerting         │
                 │        └──► TUI / JSON / plain / diagnose    │
                 └──────────────────────────────────────────────┘
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
./build.sh
sudo target/release/process-monitor
```

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

### TUI Keys

| Key | Action |
|---|---|
| `q`, `Esc`, `Ctrl+C` | Quit |
| `p` | Pause / resume |
| `c` | Clear event log |
| `↑`/`↓`, `j`/`k` | Scroll |
| `Tab` | Cycle panel focus |

---

## JSON Output

```jsonc
{"ts": "14:09:16.531", "type": "open", "pid": 29645, "uid": 1000, "comm": "process-monitor", "file": "/dev/tty"}
{"ts": "14:09:16.973", "type": "alert", "pid": 2126, "uid": 1000, "comm": "Cache2 I/O", "opens_in_1s": 50}
```

---

## How It Works

1. Kernel tracepoints capture PID, UID, `comm`, target filename into `ProcessEvent`
2. Reader thread opens perf buffer per CPU, forwards events over MPSC channel
3. Monitor keeps 1-second sliding window per PID, alerts at threshold
4. Output renders: TUI, JSON, plain, or diagnostic report

---

## Project Structure

```
halcyon-process-monitor/
├── process-monitor/          # Userspace: monitor core + TUI
│   └── src/
│       ├── main.rs           # CLI, mode selection, signal handling
│       ├── monitor.rs        # eBPF loading, perf reader, sliding window
│       └── tui.rs            # ratatui cyberpunk interface
├── process-monitor-ebpf/     # Kernel side (#![no_std], aya-ebpf)
│   └── src/main.rs           # tracepoint hooks → PerfEventArray
├── build.sh                  # Build script
├── install.sh                # Distro-aware installer
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

## License

MIT

---

> 🤖 Generated with [Codebuff](https://codebuff.com) · [Portfolio](https://bartoszosiej.github.io/Portfolio/)
