# Halcyon Process Monitor — Test Report & QA

> Generated: 2026-08-13 · Rust `cargo 1.97.1` · Linux
> Re-run: `cargo test`

## Whole project

**✅ 3 tests · 0 failed** (userspace `process-monitor` crate).

## Per-crate

| Crate | Status |
|---|---|
| `process-monitor` (userspace) | ✅ builds + 3 tests pass |
| `process-monitor-ebpf` | ⚠️ targets `bpfel-unknown-none` — not buildable on the host toolchain; `build.sh` invokes it with `-Z build-std` explicitly |

## Static analysis

| Check | Result |
|---|---|
| Clippy | 0 warnings, 0 errors |
| `unsafe` blocks | 7 (syscall/interop layer — expected for a process monitor, reviewed) |
