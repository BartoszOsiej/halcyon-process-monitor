# Halcyon Process Monitor

> **Real-time, eBPF-based process and file-operation telemetry for Linux.**

Halcyon Process Monitor traces `execve` and `openat` syscalls at the kernel level
using eBPF tracepoints, streams the events into userspace through per-CPU perf
buffers, and surfaces them in a live terminal TUI — while continuously scoring
per-process file-open rates against a sliding window to flag
ransomware-style mass file access.

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

## Screenshot

![Halcyon Process Monitor — live TUI](screenshots/tui.png)

*Verified live: eBPF loaded, events flowing through per-CPU perf buffers, TCP
alert fired when a process exceeded the 1-second window threshold.*

## Features

| Capability | Description |
|---|---|
| **Kernel-level tracing** | `execve` and `openat` tracepoints attached on every online CPU |
| **Verifier-safe kernel code** | Userspace pointers read exclusively via `bpf_probe_read_user` — never dereferenced |
| **Zero-copy event pipeline** | Fixed-size `ProcessEvent` records streamed through per-CPU `PerfEventArray` buffers |
| **Live TUI** | Event log, per-process stats table, and alert panel rendered with `ratatui` |
| **Sliding-window heuristic** | 1-second rolling window per PID; alerts when a process exceeds the configured open rate |
| **Multiple output modes** | Human TUI, newline-delimited JSON, plain text log, and a built-in self-diagnostic |
| **Lost-event accounting** | Perf-buffer overruns are counted and reported, never silently dropped |
| **Single static binary** | Full LTO, `panic = "abort"`, symbol-stripped release profile |

## Requirements

| Requirement | Notes |
|---|---|
| Linux kernel **5.8+** | eBPF + tracepoint support |
| **root** (`CAP_BPF` / `CAP_SYS_ADMIN`) | Required to load and attach eBPF programs |
| Rust **nightly** + `rust-src` | Builds the eBPF program with `-Z build-std` |
| `bpf-linker`, `clang`, C compiler | eBPF linking toolchain |
| BTF (`/sys/kernel/btf/vmlinux`) | Recommended for CO-RE compatibility |

The userspace binary builds on stable Rust; only the eBPF crate needs nightly.

## Quick Start

The fastest path is the distro-aware installer — it detects your package
manager (apt, dnf, pacman, zypper, apk, xbps), installs build dependencies from
your distribution's official repositories, provisions the Rust toolchain,
builds, and installs:

```bash
# System-wide install to /usr/local (prompts before every change):
./install.sh --system

# User-local install to ~/.local (no sudo needed):
./install.sh
```

Or build manually:

```bash
./build.sh
sudo target/release/process-monitor
```

## Usage

```bash
# TUI (default when stdout is a terminal)
sudo process-monitor

# Raise the alert threshold (file opens per second before an alert fires)
sudo process-monitor --alert-threshold 100

# Machine-readable output for pipelines / log aggregation
sudo process-monitor --json | jq .

# Plain text log (no TUI, no JSON)
sudo process-monitor --plain

# Explicit path to the compiled eBPF object (auto-detected otherwise)
sudo process-monitor --bpf /path/to/process-monitor-ebpf

# 5-second end-to-end self-diagnostic, then exit
sudo process-monitor --diagnose
```

### Command-line reference

| Flag | Default | Description |
|---|---|---|
| `-b, --bpf <PATH>` | auto-discovered | Path to the compiled eBPF object |
| `--alert-threshold <N>` | `50` | Alert when a process opens N+ files within 1 s (`0` disables alerts) |
| `--json` | off | Newline-delimited JSON output (no TUI) |
| `--plain` | off | Plain text log output (no TUI); conflicts with `--json` |
| `--tui` | off | Force the TUI even when stdout is not a terminal |
| `--diagnose` | off | Run a 5-second end-to-end self-diagnostic and exit |

### TUI keys

| Key | Action |
|---|---|
| `q`, `Esc`, `Ctrl+C` | Quit |
| `p` | Pause / resume the event stream |
| `c` | Clear the event log |
| `↑` / `↓`, `j` / `k` | Scroll the event log |
| `PgUp` / `PgDn` | Scroll faster |
| `Home` / `End` | Jump to top / bottom |

## JSON output schema

Each event is one JSON object per line (newline-delimited JSON):

```jsonc
// Process event
{"ts": "14:09:16.531", "type": "open", "pid": 29645, "uid": 1000, "comm": "process-monitor", "file": "/dev/tty"}
{"ts": "14:09:16.532", "type": "exec", "pid": 29645, "uid": 1000, "comm": "bash", "file": "/usr/bin/ls"}

// Alert
{"ts": "14:09:16.973", "type": "alert", "pid": 2126, "uid": 1000, "comm": "Cache2 I/O", "opens_in_1s": 50}
```

| Field | Present in | Meaning |
|---|---|---|
| `ts` | all | Local timestamp, `HH:MM:SS.mmm` |
| `type` | all | `exec`, `open`, or `alert` |
| `pid` | all | Process ID that triggered the syscall |
| `uid` | all | User ID of the process |
| `comm` | all | Process name (16-byte `comm`, truncated) |
| `file` | exec/open | Target path (64-byte buffer, truncated) |
| `opens_in_1s` | alert | File opens counted in the current sliding window |

## How it works

1. Two kernel tracepoints (`sys_enter_execve`, `sys_enter_openat`) capture the
   PID, UID, `comm`, and target filename into a compact, fixed-layout
   `ProcessEvent` and push it into the `EVENTS` `PerfEventArray`.
2. A dedicated reader thread opens one perf buffer per online CPU and forwards
   decoded events over an MPSC channel to the monitor core.
3. The monitor keeps a 1-second sliding window of file opens per PID and raises
   an alert when a process crosses the configured threshold.
4. Output is rendered according to the selected mode: TUI, JSON, plain, or the
   diagnostic report.

See **[ARCHITECTURE.md](ARCHITECTURE.md)** for the full design.

## Project structure

```
halcyon-process-monitor/
├── process-monitor/          # Userspace: monitor core + TUI + output modes
│   └── src/
│       ├── main.rs           # CLI, mode selection, signal handling
│       ├── monitor.rs        # eBPF loading, perf reader, sliding-window tracker
│       └── tui.rs            # ratatui interface (events / stats / alerts)
├── process-monitor-ebpf/     # Kernel side (#![no_std], aya-ebpf)
│   └── src/main.rs           # tracepoint hooks → PerfEventArray
├── build.sh                  # Build script (nightly for eBPF, stable for TUI)
├── install.sh                # Distro-aware installer / uninstaller
└── Cargo.toml                # Workspace definition
```

## Security notes

- The installer never reads or logs tokens, credentials, or environment
  secrets, and makes **no authenticated network requests**.
- Dependencies are installed only from your distribution's official
  repositories; the only downloads are the official `rustup.rs` installer
  (with a prompt) and crates from crates.io.
- All shell commands are quoted and safe by default; user-local installs need
  no `sudo`.
- The kernel code follows strict eBPF safety rules: all userspace memory is
  read with `bpf_probe_read_user`, never dereferenced.
- eBPF programs must run as root; run the monitor under `sudo`.

## Troubleshooting

Run the built-in self-diagnostic to verify the toolchain, tracepoint
availability, eBPF loading, and event flow end to end:

```bash
sudo process-monitor --diagnose
```

Common checks:

- **No events at all** → confirm tracepoints exist
  (`/sys/kernel/tracing/events/syscalls/sys_enter_execve/id`) and that the
  perf buffers opened (`[halcyon] opening perf buffers on N CPUs`).
- **`failed to load eBPF program`** → the eBPF object path is wrong; pass
  `--bpf` explicitly or re-run `./install.sh`.
- **Not running as root** → `Monitor::start` bails with a clear
  `CAP_BPF / CAP_SYS_ADMIN` message.

## Uninstall

```bash
./install.sh --uninstall            # remove user-local install
./install.sh --uninstall --system   # remove system install
```

## License

MIT — the crates declare `license = "MIT"` in their `Cargo.toml` manifests.
