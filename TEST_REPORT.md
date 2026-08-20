# Halcyon Process Monitor — Test Report & QA

> Generated: 2026-08-20 · Rust `cargo 1.97.1` · Linux
> Re-run: `cargo test`

## Whole project

**✅ 15 tests · 0 failed** (userspace `process-monitor` crate).

## Per-crate

| Crate | Status |
|---|---|
| `process-monitor` (userspace) | ✅ builds + 15 tests pass |
| `process-monitor-ebpf` | ⚠️ targets `bpfel-unknown-none` — not buildable on the host toolchain; `build.sh` invokes it with `-Z build-std` explicitly |

## Coverage

### Monitor (monitor.rs)
- C-string decoding (nul-terminated, padded)
- ProcessEvent string helpers
- Exec events update stats without alerts
- Opens trigger alert exactly once at the threshold
- No second alert above threshold
- Stats sorted by window_opens descending
- File extension extraction (basic, dotfiles, multi-dot)
- Shannon entropy range (empty, uniform, varied)
- Top files sorted by open count
- Extension counts aggregate correctly

### TUI (tui.rs)
- Renders with events and alerts (TestBackend 120×30)
- Key handling (pause, quit, Ctrl+C)
- Tab cycles focus between panels (0→1→2→0)
- Monitor tracks alerts (threshold alerting)
- format_number scales (0, 999, 1.5K, 1.5M)

## New in v0.3.0

| Feature | Tests |
|---|---|
| File extension tracking | `extract_extension_basic`, `extension_counts_aggregate` |
| Shannon entropy | `shannon_entropy_range` |
| Top-files ranking | `top_files_sorted_by_count` |
| Sparkline rate history | Built into `poll()` (120s window) |
| Tab panel focus | `tab_cycles_focus` |
| Cyberpunk TUI theme | Visual rendering test |

## Static analysis

| Check | Result |
|---|---|
| Clippy | 0 warnings, 0 errors |
| `unsafe` blocks | 7 (syscall/interop layer — expected for a process monitor, reviewed) |
