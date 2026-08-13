# Halcyon Process Monitor — Test Report & QA

> Generated: 2026-08-13 · Rust `cargo 1.97.1` · Linux
> Re-run: `cargo test`

## Whole project

**✅ 9 tests · 0 failed** (userspace `process-monitor` crate).

## Per-crate

| Crate | Status |
|---|---|
| `process-monitor` (userspace) | ✅ builds + 9 tests pass |
| `process-monitor-ebpf` | ⚠️ targets `bpfel-unknown-none` — not buildable on the host toolchain; `build.sh` invokes it with `-Z build-std` explicitly |

## Coverage

- TUI: rendering with events/alerts, key handling, alert tracking
- Monitor: C-string decoding, event stats (exec/open), ransomware heuristic
  (alert fires exactly once at the 1-second-window threshold), window expiry,
  per-process stats sorting

## Static analysis

| Check | Result |
|---|---|
| Clippy | 0 warnings, 0 errors |
| `unsafe` blocks | 7 (syscall/interop layer — expected for a process monitor, reviewed) |
