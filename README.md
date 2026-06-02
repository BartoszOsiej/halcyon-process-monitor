# Halcyon Process Monitor

**eBPF-based real-time process and file operation monitor** built with Rust and Aya.

Originally developed as a technical demonstration for a **Linux Agent Engineer** role at Halcyon — an anti-ransomware security company. The project showcases production-quality Rust eBPF development, kernel-level tracing, and real-time event processing.

## Architecture

```
┌──────────────────────────────────────────────────┐
│                Userspace (Rust)                   │
│  ┌──────────┐  ┌──────────────────────────────┐  │
│  │   CLI    │  │   Perf Buffer Reader          │  │
│  │  (clap)  │  │   ┌────────────────────────┐ │  │
│  │          │  │   │ Event Parser           │ │  │
│  │          │  │   │ Ransomware Heuristic   │ │  │
│  │          │  │   │ JSON/Formatted Output  │ │  │
│  │          │  │   └────────────────────────┘ │  │
│  └──────────┘  └──────────────┬───────────────┘  │
│                               │                   │
│                     ┌─────────▼──────────┐        │
│                     │  PerfEventArray    │        │
│                     │  (perf buffer)     │        │
│                     └─────────▲──────────┘        │
└────────────────────────────────┼──────────────────┘
                                 │
┌────────────────────────────────┼──────────────────┐
│           Kernel (eBPF)        │                   │
│                     ┌──────────┴──────────┐        │
│                     │   eBPF Programs     │        │
│  ┌──────────────────┴─┐  ┌────────────────┴──┐    │
│  │ sys_enter_execve   │  │ sys_enter_openat  │    │
│  │ (tracepoint)       │  │ (tracepoint)      │    │
│  └──────────┬─────────┘  └────────┬──────────┘    │
│             │                     │                │
│     ┌───────▼─────────────────────▼───────┐       │
│     │        Linux Kernel                 │       │
│     │  syscalls: execve, openat          │       │
│     └─────────────────────────────────────┘       │
└──────────────────────────────────────────────────┘
```

## Features

### Core eBPF Tracing
- **Process execution monitoring** — traces every `execve` syscall via tracepoints
- **File operation monitoring** — traces every `openat` syscall in real-time
- **Per-event metadata** — PID, UID, process name (`comm`), filename

### Ransomware Detection Heuristic
- Tracks file open rate **per process**
- Alerts when a process exceeds configurable open-per-second threshold
- Designed to detect mass file encryption behavior characteristic of ransomware

### Output Formats
- **Human-readable** — color-coded terminal output with timestamps
- **JSON mode** (`--json`) — structured output for pipeline integration

## Requirements

- Linux kernel 5.8+ (for BPF CO-RE and tracepoint support)
- Rust nightly toolchain (for BPF target compilation)
- `sudo` (eBPF programs require root)
- `gcc` / `kernel-devel` (C compiler for build scripts)
- `bpftool` (optional, for debugging)

## Quick Start

### Install C compiler (if missing)

```bash
# Debian/Ubuntu (apt-based)
sudo apt install build-essential

# openSUSE (zypper-based)
sudo zypper install gcc gcc-c++ kernel-devel

# Fedora/RHEL (dnf-based)
sudo dnf install gcc gcc-c++ kernel-devel
```

### Build & Run

```bash
# Build
./build.sh

# Run
sudo ./target/release/process-monitor

# With custom alert threshold (file opens/sec before alarm)
sudo ./target/release/process-monitor --alert-threshold 100

# JSON output for log aggregation
sudo ./target/release/process-monitor --json | jq .
```

## Project Structure

```
ebpf-process-monitor/
├── Cargo.toml                          # Workspace definition
├── process-monitor/                    # User-space binary
│   ├── Cargo.toml
│   └── src/main.rs                     # CLI, perf buffer, event processing
├── process-monitor-ebpf/               # eBPF programs
│   ├── Cargo.toml
│   └── src/main.rs                     # Tracepoint hooks, perf event output
├── build.sh                            # Build script
└── README.md
```

## Technical Details

### eBPF Side (`process-monitor-ebpf`)
- Written in `#![no_std]` Rust using `aya-ebpf`
- Attaches to `tracepoint/syscalls/sys_enter_execve` and `tracepoint/syscalls/sys_enter_openat`
- Emits structured `ProcessEvent` structs to a `PerfEventArray`
- Minimal overhead — only captures on syscall entry, no heavy processing in kernel context

### Userspace Side (`process-monitor`)
- Built with `aya` for eBPF program loading and management
- `PerfEventArray` reader with async Tokio runtime for non-blocking event processing
- Per-process file open rate tracking using `HashMap<pid, sliding window counter>`
- Clap-based CLI with configurable alert thresholds
- Proper error handling with `anyhow`

## Relevance to Halcyon

This project directly demonstrates skills required for the **Linux Agent Engineer** role:

| Requirement | Demonstrated |
|---|---|
| Rust production code | ✅ Full Rust project with proper error handling, async, crate structure |
| eBPF program development | ✅ Tracepoint-based syscall monitoring with perf buffer |
| User-space eBPF loaders | ✅ Aya-based program loading and attachment |
| Event processing pipelines | ✅ Real-time event parsing, formatting, alerting |
| Performance awareness | ✅ Minimal kernel overhead, sliding window counters |
| Linux internals | ✅ Syscall tracing, kernel-userspace communication |

## License

MIT
