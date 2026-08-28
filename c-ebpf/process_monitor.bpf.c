// SPDX-License-Identifier: MIT
// process_monitor.bpf.c — eBPF kernel programs for Talus Process Monitor
//
// Ported from the Rust/Aya implementation to C/libbpf.
// Traces execve, openat, connect, accept, sendto, recvfrom, mkdir,
// unlinkat, kill, fchmodat syscalls via tracepoints.
// Events are pushed to userspace through a per-CPU perf buffer.

#include <linux/bpf.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>

// ── Event type constants (must match userspace) ──────────────────────────

#define EVENT_EXECVE    0
#define EVENT_OPENAT    1
#define EVENT_CONNECT   2
#define EVENT_ACCEPT    3
#define EVENT_SENDTO    4
#define EVENT_RECVFROM  5
#define EVENT_MKDIR     6
#define EVENT_UNLINK    7
#define EVENT_KILL      8
#define EVENT_CHMOD     9

// ── Buffer sizes ─────────────────────────────────────────────────────────

#define COMM_LEN       16
#define FILENAME_LEN   64
#define ARGV_LEN       128

// ── Event record shared with userspace (packed, no padding) ──────────────

struct process_event {
    __u8  event_type;
    __u32 pid;
    __u32 uid;
    __u8  comm[COMM_LEN];
    __u8  filename[FILENAME_LEN];
    __u8  argv[ARGV_LEN];
} __attribute__((packed));

// ── Perf event array map ─────────────────────────────────────────────────

struct {
    __uint(type, BPF_MAP_TYPE_PERF_EVENT_ARRAY);
    __uint(key_size, sizeof(__u32));
    __uint(value_size, sizeof(__u32));
} events SEC(".maps");

// ── Helpers ──────────────────────────────────────────────────────────────

static __always_inline void raw_copy(__u8 *dst, const __u8 *src, __u32 len)
{
    for (__u32 i = 0; i < len; i++)
        dst[i] = src[i];
}

static __always_inline void read_comm(struct process_event *e)
{
    __builtin_memset(e->comm, 0, COMM_LEN);
    bpf_get_current_comm(e->comm, COMM_LEN);
    e->comm[COMM_LEN - 1] = 0;
}

// Format a u8 as decimal digits into buf at pos. Returns new pos.
static __always_inline __u32 write_octet(__u8 *buf, __u32 pos, __u8 val)
{
    if (val >= 100) { buf[pos++] = '0' + val / 100; }
    if (val >= 10)  { buf[pos++] = '0' + (val / 10) % 10; }
    buf[pos++] = '0' + val % 10;
    return pos;
}

// Format a u16 as decimal digits into buf at pos. Returns new pos.
static __always_inline __u32 write_u16(__u8 *buf, __u32 pos, __u16 val)
{
    if (val >= 10000) { buf[pos++] = '0' + val / 10000; val %= 10000; }
    if (val >= 1000)  { buf[pos++] = '0' + val / 1000;  val %= 1000; }
    if (val >= 100)   { buf[pos++] = '0' + val / 100;   val %= 100; }
    if (val >= 10)    { buf[pos++] = '0' + val / 10; }
    buf[pos++] = '0' + val % 10;
    return pos;
}

// Try to read an IPv4/IPv6/Unix sockaddr into event.filename.
static __always_inline void try_read_sockaddr(void *ctx, __u32 arg_idx,
                                               struct process_event *e)
{
    void *sockaddr_ptr = NULL;
    bpf_probe_read_kernel(&sockaddr_ptr, sizeof(sockaddr_ptr),
                          (void *)(ctx + 16 + arg_idx * sizeof(void *)));
    if (!sockaddr_ptr)
        return;

    __u16 family = 0;
    bpf_probe_read_user(&family, sizeof(family), sockaddr_ptr);

    __builtin_memset(e->filename, 0, FILENAME_LEN);

    if (family == 2) {
        // AF_INET: port (u16 big-endian at offset 2), addr (4 bytes at offset 4)
        __u16 port_be = 0;
        bpf_probe_read_user(&port_be, sizeof(port_be),
                            (__u8 *)sockaddr_ptr + 2);
        __u16 port = bpf_ntohs(port_be);

        __u8 a0, a1, a2, a3;
        bpf_probe_read_user(&a0, 1, (__u8 *)sockaddr_ptr + 4);
        bpf_probe_read_user(&a1, 1, (__u8 *)sockaddr_ptr + 5);
        bpf_probe_read_user(&a2, 1, (__u8 *)sockaddr_ptr + 6);
        bpf_probe_read_user(&a3, 1, (__u8 *)sockaddr_ptr + 7);

        __u32 pos = 0;
        pos = write_octet(e->filename, pos, a0);
        e->filename[pos++] = '.';
        pos = write_octet(e->filename, pos, a1);
        e->filename[pos++] = '.';
        pos = write_octet(e->filename, pos, a2);
        e->filename[pos++] = '.';
        pos = write_octet(e->filename, pos, a3);
        e->filename[pos++] = ':';
        pos = write_u16(e->filename, pos, port);
    } else if (family == 10) {
        raw_copy(e->filename, (__u8 *)"[IPv6]", 6);
    } else if (family == 1) {
        raw_copy(e->filename, (__u8 *)"[ Unix ]", 7);
    }
}

// ── Core event emitter ───────────────────────────────────────────────────

static __always_inline int emit_event(void *ctx, __u8 event_type,
                                       __u32 filename_arg_idx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = event_type;
    e.pid = pid;
    e.uid = uid;

    read_comm(&e);

    // Read filename from tracepoint args
    void *filename_ptr = NULL;
    bpf_probe_read_kernel(&filename_ptr, sizeof(filename_ptr),
                          (void *)(ctx + 16 + filename_arg_idx * sizeof(void *)));
    if (filename_ptr) {
        bpf_probe_read_user_str(&e.filename, FILENAME_LEN, filename_ptr);
    }

    // For execve: read argv[0]
    if (event_type == EVENT_EXECVE) {
        void *argv_ptr_storage = NULL;
        bpf_probe_read_kernel(&argv_ptr_storage, sizeof(argv_ptr_storage),
                              (void *)(ctx + 16 + 1 * sizeof(void *)));
        if (argv_ptr_storage) {
            void *arg0 = NULL;
            bpf_probe_read_user(&arg0, sizeof(arg0), argv_ptr_storage);
            if (arg0) {
                bpf_probe_read_user_str(&e.argv, ARGV_LEN, arg0);
            }
        }
    }

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

// ── Tracepoint programs ──────────────────────────────────────────────────

SEC("tracepoint/syscalls/sys_enter_execve")
int sys_enter_execve(void *ctx)
{
    return emit_event(ctx, EVENT_EXECVE, 0);
}

SEC("tracepoint/syscalls/sys_enter_openat")
int sys_enter_openat(void *ctx)
{
    return emit_event(ctx, EVENT_OPENAT, 1);
}

SEC("tracepoint/syscalls/sys_enter_connect")
int sys_enter_connect(void *ctx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = EVENT_CONNECT;
    e.pid = pid;
    e.uid = uid;
    read_comm(&e);
    try_read_sockaddr(ctx, 1, &e);

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_accept")
int sys_enter_accept(void *ctx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = EVENT_ACCEPT;
    e.pid = pid;
    e.uid = uid;
    read_comm(&e);
    try_read_sockaddr(ctx, 1, &e);

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_sendto")
int sys_enter_sendto(void *ctx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = EVENT_SENDTO;
    e.pid = pid;
    e.uid = uid;
    read_comm(&e);
    try_read_sockaddr(ctx, 4, &e);

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_recvfrom")
int sys_enter_recvfrom(void *ctx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = EVENT_RECVFROM;
    e.pid = pid;
    e.uid = uid;
    read_comm(&e);
    try_read_sockaddr(ctx, 4, &e);

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

// ── Filesystem / signal tracepoints ──────────────────────────────────────

SEC("tracepoint/syscalls/sys_enter_mkdir")
int sys_enter_mkdir(void *ctx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = EVENT_MKDIR;
    e.pid = pid;
    e.uid = uid;
    read_comm(&e);

    void *pathname_ptr = NULL;
    bpf_probe_read_kernel(&pathname_ptr, sizeof(pathname_ptr),
                          (void *)(ctx + 16));
    if (pathname_ptr)
        bpf_probe_read_user_str(&e.filename, FILENAME_LEN, pathname_ptr);

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_unlinkat")
int sys_enter_unlinkat(void *ctx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = EVENT_UNLINK;
    e.pid = pid;
    e.uid = uid;
    read_comm(&e);

    void *pathname_ptr = NULL;
    bpf_probe_read_kernel(&pathname_ptr, sizeof(pathname_ptr),
                          (void *)(ctx + 16 + 1 * sizeof(void *)));
    if (pathname_ptr)
        bpf_probe_read_user_str(&e.filename, FILENAME_LEN, pathname_ptr);

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_kill")
int sys_enter_kill(void *ctx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = EVENT_KILL;
    e.pid = pid;
    e.uid = uid;
    read_comm(&e);

    __s32 target_pid = 0;
    bpf_probe_read_kernel(&target_pid, sizeof(target_pid), (void *)(ctx + 16));

    __s32 sig = 0;
    bpf_probe_read_kernel(&sig, sizeof(sig),
                          (void *)(ctx + 16 + sizeof(void *)));

    // Format "target_pid:sig_name" into argv using pointer arithmetic
    __u32 pos = 0;
    __u32 v = (__u32)(target_pid < 0 ? -target_pid : target_pid);
    if (v == 0) {
        e.argv[0] = '0';
        pos = 1;
    } else {
        __u32 divisor = 1;
        while (divisor <= v / 10) divisor *= 10;
        while (divisor > 0 && pos < ARGV_LEN - 1) {
            e.argv[pos] = '0' + (v / divisor) % 10;
            pos++;
            divisor /= 10;
        }
    }

    if (pos < ARGV_LEN - 1)
        e.argv[pos++] = ':';

    // Write signal name using pointer arithmetic
    if (sig == 1) {
        raw_copy(e.argv + pos, (const __u8 *)"SIGHUP", 6); pos += 6;
    } else if (sig == 2) {
        raw_copy(e.argv + pos, (const __u8 *)"SIGINT", 5); pos += 5;
    } else if (sig == 3) {
        raw_copy(e.argv + pos, (const __u8 *)"SIGQUIT", 6); pos += 6;
    } else if (sig == 6) {
        raw_copy(e.argv + pos, (const __u8 *)"SIGABRT", 6); pos += 6;
    } else if (sig == 9) {
        raw_copy(e.argv + pos, (const __u8 *)"SIGKILL", 6); pos += 6;
    } else if (sig == 15) {
        raw_copy(e.argv + pos, (const __u8 *)"SIGTERM", 6); pos += 6;
    } else {
        e.argv[pos++] = '?';
    }

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

SEC("tracepoint/syscalls/sys_enter_fchmodat")
int sys_enter_fchmodat(void *ctx)
{
    __u64 pid_tgid = bpf_get_current_pid_tgid();
    __u32 pid = pid_tgid >> 32;
    __u32 uid = (__u32)bpf_get_current_uid_gid();

    struct process_event e = {};
    e.event_type = EVENT_CHMOD;
    e.pid = pid;
    e.uid = uid;
    read_comm(&e);

    void *pathname_ptr = NULL;
    bpf_probe_read_kernel(&pathname_ptr, sizeof(pathname_ptr),
                          (void *)(ctx + 16 + 1 * sizeof(void *)));
    if (pathname_ptr)
        bpf_probe_read_user_str(&e.filename, FILENAME_LEN, pathname_ptr);

    bpf_perf_event_output(ctx, &events, BPF_F_CURRENT_CPU, &e, sizeof(e));
    return 0;
}

char LICENSE[] SEC("license") = "GPL";
