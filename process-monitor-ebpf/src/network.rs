//! Network eBPF tracepoint programs for Halcyon Process Monitor.
//!
//! Simplified: always emit event with PID+comm, attempt sockaddr read
//! but don't fail the whole program if it doesn't work.

use aya_ebpf::{
    helpers::{bpf_get_current_comm, bpf_get_current_pid_tgid, bpf_get_current_uid_gid, bpf_probe_read_user},
    macros::tracepoint,
    programs::TracePointContext,
};

pub const EVENT_CONNECT: u8 = 2;
pub const EVENT_ACCEPT: u8 = 3;
pub const EVENT_SENDTO: u8 = 4;
pub const EVENT_RECVFROM: u8 = 5;

use super::EVENTS;
use super::ProcessEvent;

// eBPF-safe: zero-init without memset
#[inline(always)]
unsafe fn zero_event() -> ProcessEvent {
    core::mem::MaybeUninit::uninit().assume_init()
}

#[inline(always)]
unsafe fn raw_copy(dst: *mut u8, src: *const u8, len: usize) {
    let mut i = 0;
    while i < len {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
}

/// Try to read IPv4 sockaddr into event.filename.
/// Returns true if successful.
#[inline(always)]
unsafe fn try_read_sockaddr(ctx: &TracePointContext, arg_idx: usize, event: &mut ProcessEvent) -> bool {
    // Tracepoint args are at offset 16 + arg_idx * sizeof(pointer)
    let offset = 16 + arg_idx * core::mem::size_of::<usize>();
    let ptr: *const u8 = match ctx.read_at(offset) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if ptr.is_null() { return false; }

    // Read sa_family (u16 at offset 0)
    let family: u16 = match bpf_probe_read_user(ptr.cast()) {
        Ok(v) => v,
        Err(_) => return false,
    };

    match family {
        2 => { // AF_INET
            let port: u16 = match bpf_probe_read_user(ptr.add(2).cast()) {
                Ok(v) => v,
                Err(_) => return false,
            };
            let addr: [u8; 4] = match bpf_probe_read_user(ptr.add(4).cast()) {
                Ok(v) => v,
                Err(_) => return false,
            };
            // Format "A.B.C.D:PORT"
            let mut buf = [0u8; 21];
            let mut pos = 0;
            let mut i = 0;
            while i < 4 {
                if i > 0 { buf[pos] = b'.'; pos += 1; }
                let octet = addr[i];
                if octet >= 100 { buf[pos] = b'0' + octet / 100; pos += 1; }
                if octet >= 10 { buf[pos] = b'0' + (octet / 10) % 10; pos += 1; }
                buf[pos] = b'0' + octet % 10; pos += 1;
                i += 1;
            }
            buf[pos] = b':'; pos += 1;
            let mut p = port;
            if p >= 10000 { buf[pos] = b'0' + (p / 10000) as u8; pos += 1; p %= 10000; }
            if p >= 1000 { buf[pos] = b'0' + (p / 1000) as u8; pos += 1; p %= 1000; }
            if p >= 100 { buf[pos] = b'0' + (p / 100) as u8; pos += 1; p %= 100; }
            if p >= 10 { buf[pos] = b'0' + (p / 10) as u8; pos += 1; }
            buf[pos] = b'0' + (p % 10) as u8; pos += 1;

            raw_copy(event.filename.as_mut_ptr(), buf.as_ptr(), pos);
            true
        }
        10 => { // AF_INET6
            event.filename[..6].copy_from_slice(b"[IPv6]");
            true
        }
        1 => { // AF_UNIX
            event.filename[..7].copy_from_slice(b"[ Unix ]");
            true
        }
        _ => false,
    }
}

// ── Tracepoint programs ──────────────────────────────────────────────────

#[tracepoint(name = "sys_enter_connect", category = "syscalls")]
pub fn sys_enter_connect(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;
    let mut event = unsafe { zero_event() };
    event.event_type = EVENT_CONNECT;
    event.pid = pid;
    event.uid = uid;
    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        unsafe { raw_copy(event.comm.as_mut_ptr(), comm.as_ptr(), n) };
    }
    // arg1 = uservaddr (struct sockaddr *)
    unsafe { try_read_sockaddr(&ctx, 1, &mut event); }
    EVENTS.output(&ctx, &event, 0);
    0
}

#[tracepoint(name = "sys_enter_accept", category = "syscalls")]
pub fn sys_enter_accept(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;
    let mut event = unsafe { zero_event() };
    event.event_type = EVENT_ACCEPT;
    event.pid = pid;
    event.uid = uid;
    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        unsafe { raw_copy(event.comm.as_mut_ptr(), comm.as_ptr(), n) };
    }
    // arg1 = upeer_sockaddr
    unsafe { try_read_sockaddr(&ctx, 1, &mut event); }
    EVENTS.output(&ctx, &event, 0);
    0
}

#[tracepoint(name = "sys_enter_sendto", category = "syscalls")]
pub fn sys_enter_sendto(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;
    let mut event = unsafe { zero_event() };
    event.event_type = EVENT_SENDTO;
    event.pid = pid;
    event.uid = uid;
    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        unsafe { raw_copy(event.comm.as_mut_ptr(), comm.as_ptr(), n) };
    }
    // arg4 = addr (struct sockaddr *)
    unsafe { try_read_sockaddr(&ctx, 4, &mut event); }
    EVENTS.output(&ctx, &event, 0);
    0
}

#[tracepoint(name = "sys_enter_recvfrom", category = "syscalls")]
pub fn sys_enter_recvfrom(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;
    let mut event = unsafe { zero_event() };
    event.event_type = EVENT_RECVFROM;
    event.pid = pid;
    event.uid = uid;
    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        unsafe { raw_copy(event.comm.as_mut_ptr(), comm.as_ptr(), n) };
    }
    // arg4 = addr (struct sockaddr *)
    unsafe { try_read_sockaddr(&ctx, 4, &mut event); }
    EVENTS.output(&ctx, &event, 0);
    0
}
