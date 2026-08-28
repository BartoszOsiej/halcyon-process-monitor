# Talus Process Monitor — Test Report & QA

> Generated: 2026-08-26 · Rust `cargo 1.97.1` · Linux
> Re-run: `cargo test`

## Whole project

**✅ 13 tests · 0 failed** (userspace `process-monitor` crate).

## Per-crate

| Crate | Status |
|---|---|
| `process-monitor` (userspace) | ✅ builds + 13 tests pass |
| `process-monitor-ebpf` | ✅ all 10 tracepoints load and attach successfully |

## Tracepoints (verified live)

| Tracepoint | Status | Notes |
|---|---|---|
| `sys_enter_execve` | ✅ | Process execution tracking |
| `sys_enter_openat` | ✅ | File open tracking with extension detection |
| `sys_enter_connect` | ✅ | IPv4/IPv6/Unix socket connections with IP:port |
| `sys_enter_accept` | ✅ | Incoming socket connections |
| `sys_enter_sendto` | ✅ | Outgoing data sends |
| `sys_enter_recvfrom` | ✅ | Incoming data receives |
| `sys_enter_mkdir` | ✅ | Directory creation |
| `sys_enter_unlinkat` | ✅ | File deletion |
| `sys_enter_kill` | ✅ | Signal delivery with signal name resolution |
| `sys_enter_fchmodat` | ✅ | File permission changes |

## eBPF fixes (v0.4.0 → v0.5.0)

### Network tracepoints (connect/accept/sendto/recvfrom)
- **Root cause**: LLVM generated `.text.unlikely` cold-path sections for `bpf_probe_read_user` with array types and `copy_from_slice` calls
- **Fix**: Rewrote `try_read_sockaddr` to read sockaddr byte-by-byte using individual `bpf_probe_read_user::<u8>` calls, avoiding array types and slice operations that trigger `slice_index_fail` panic handlers
- **Result**: All 4 network tracepoints now load and attach successfully

### Kill tracepoint
- **Root cause**: Same `.text.unlikely` issue from `format_pid` using iterators and array indexing
- **Fix**: Rewrote PID formatting to use direct pointer arithmetic, replaced `match` with `if-else` chains, wrote signal names byte-by-byte via `write_sig_ptr`
- **Result**: Kill tracepoint loads and resolves signal names (SIGHUP, SIGINT, SIGKILL, SIGTERM, etc.)

### BPF profile
- Changed `codegen-units = 4` → `codegen-units = 1` to reduce code duplication

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
- Process tree builds hierarchy
- Flatten tree depth ordering
- Multiple roots handled correctly

### TUI (tui.rs)
- Compiles without warnings (0 clippy warnings)
- Uses FrankenTUI widgets (MiniBar, BarChart, LineChart, Canvas, Badge, Sparkline, heatmap)
- Network panel: per-process aggregated connections + Canvas traffic flow
- Heatmap: process × extension matrix

## Static analysis

| Check | Result |
|---|---|
| `cargo check` | 0 warnings, 0 errors |
| `cargo clippy` | 0 warnings, 0 errors |
| `cargo test` | 13/13 pass |
| `unsafe` blocks | Expected for eBPF/FFI layer |
