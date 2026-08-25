//! Filesystem and signal eBPF tracepoint programs for Halcyon Process Monitor.
//!
//! Traces mkdir, unlink, rmdir, kill, and chmod syscalls — high-signal
//! events for security monitoring (file tampering, process killing).

use core::slice;

use aya_ebpf::{
    cty::c_char,
    helpers::{
        bpf_get_current_comm, bpf_get_current_pid_tgid, bpf_get_current_uid_gid,
        bpf_probe_read_user_str_bytes,
    },
    macros::{map, tracepoint},
    maps::PerfEventArray,
    programs::TracePointContext,
};

pub const EVENT_MKDIR: u8 = 6;
pub const EVENT_UNLINK: u8 = 7;
pub const EVENT_KILL: u8 = 8;
pub const EVENT_CHMOD: u8 = 9;

#[repr(C)]
pub struct FsEvent {
    pub event_type: u8,
    pub pid: u32,
    pub uid: u32,
    pub comm: [u8; 16],
    pub filename: [u8; 64],
    pub argv: [u8; 128],
}

#[map]
pub static FS_EVENTS: PerfEventArray<FsEvent> = PerfEventArray::new(0);

/// Shared EVENTS map from main.rs — fs events go through the same channel.
use super::EVENTS;

#[inline(always)]
unsafe fn zero_fs_event() -> FsEvent {
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

fn format_pid(pid: u32) -> [u8; 128] {
    let mut buf = unsafe { core::mem::MaybeUninit::<[u8; 128]>::uninit().assume_init() };
    let mut pos = 127;
    let mut v = pid;
    if v == 0 {
        buf[127] = b'0';
        return buf;
    }
    while v > 0 && pos > 0 {
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
        pos -= 1;
    }
    let start = pos + 1;
    let len = 128 - start;
    let mut result = unsafe { core::mem::MaybeUninit::<[u8; 128]>::uninit().assume_init() };
    unsafe { raw_copy(result.as_mut_ptr(), buf[start..].as_ptr(), len) };
    result
}

/// Trace `mkdir` — directory creation.
///
/// Tracepoint args: pathname (const char *), mode (umode_t)
#[tracepoint(name = "sys_enter_mkdir", category = "syscalls")]
pub fn sys_enter_mkdir(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;

    let mut event = unsafe { zero_fs_event() };
    event.event_type = EVENT_MKDIR;
    event.pid = pid;
    event.uid = uid;

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        unsafe { raw_copy(event.comm.as_mut_ptr(), comm.as_ptr(), n) };
    }

    // Read pathname (arg0)

    let pathname_offset = 16;
    if let Ok(pathname_ptr) = unsafe { ctx.read_at::<*const c_char>(pathname_offset) } {
        if !pathname_ptr.is_null() {
            let dst = unsafe { slice::from_raw_parts_mut(event.filename.as_mut_ptr(), 64) };
            if let Ok(bytes) =
                unsafe { bpf_probe_read_user_str_bytes(pathname_ptr.cast::<u8>(), dst) }
            {
                let n = bytes.len().min(63);
                unsafe { raw_copy(event.filename.as_mut_ptr(), bytes.as_ptr(), n) };
            }
        }
    }

    // SAFETY: FsEvent and ProcessEvent have identical #[repr(C)] layout.
    EVENTS.output(
        &ctx,
        unsafe { &*(&event as *const _ as *const super::ProcessEvent) },
        0,
    );
    0
}

/// Trace `unlink` — file deletion.
///
/// Tracepoint args: pathname (const char *)
#[tracepoint(name = "sys_enter_unlinkat", category = "syscalls")]
pub fn sys_enter_unlinkat(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;

    let mut event = unsafe { zero_fs_event() };
    event.event_type = EVENT_UNLINK;
    event.pid = pid;
    event.uid = uid;

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        unsafe { raw_copy(event.comm.as_mut_ptr(), comm.as_ptr(), n) };
    }

    // Read pathname (arg1 — arg0 is dirfd)
    let ptr_size = core::mem::size_of::<*const c_char>();

    let pathname_offset = 16 + 1 * ptr_size;
    if let Ok(pathname_ptr) = unsafe { ctx.read_at::<*const c_char>(pathname_offset) } {
        if !pathname_ptr.is_null() {
            let dst = unsafe { slice::from_raw_parts_mut(event.filename.as_mut_ptr(), 64) };
            if let Ok(bytes) =
                unsafe { bpf_probe_read_user_str_bytes(pathname_ptr.cast::<u8>(), dst) }
            {
                let n = bytes.len().min(63);
                unsafe { raw_copy(event.filename.as_mut_ptr(), bytes.as_ptr(), n) };
            }
        }
    }

    // SAFETY: FsEvent and ProcessEvent have identical #[repr(C)] layout.
    EVENTS.output(
        &ctx,
        unsafe { &*(&event as *const _ as *const super::ProcessEvent) },
        0,
    );
    0
}

/// Trace `kill` — signal delivery to process.
///
/// Tracepoint args: pid (pid_t), sig (int)
#[tracepoint(name = "sys_enter_kill", category = "syscalls")]
pub fn sys_enter_kill(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;

    let mut event = unsafe { zero_fs_event() };
    event.event_type = EVENT_KILL;
    event.pid = pid;
    event.uid = uid;

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        unsafe { raw_copy(event.comm.as_mut_ptr(), comm.as_ptr(), n) };
    }

    // Read target PID and signal number
    let ptr_size = core::mem::size_of::<*const c_char>();

    // arg0 = target pid (int, so 4 bytes but stored as usize in tracepoint buffer)
    let pid_offset = 16;
    if let Ok(target_pid) = unsafe { ctx.read_at::<i32>(pid_offset) } {
        // arg1 = signal (int)
        let sig_offset = 16 + ptr_size;
        if let Ok(sig) = unsafe { ctx.read_at::<i32>(sig_offset) } {
            // Format as "pid=sig" in argv
            let pid_bytes = format_pid(target_pid as u32);
            let sig_name = match sig {
                1 => "SIGHUP",
                2 => "SIGINT",
                3 => "SIGQUIT",
                6 => "SIGABRT",
                9 => "SIGKILL",
                15 => "SIGTERM",
                _ => "?",
            };

            // Write "target_pid:sig_name" into argv
            let mut pos = 0;
            // Copy target pid digits
            let start = pid_bytes.iter().position(|&b| b != 0).unwrap_or(0);
            for &b in &pid_bytes[start..] {
                if pos < 127 {
                    event.argv[pos] = b;
                    pos += 1;
                }
            }
            if pos < 127 {
                event.argv[pos] = b':';
                pos += 1;
            }
            for &b in sig_name.as_bytes() {
                if pos < 127 {
                    event.argv[pos] = b;
                    pos += 1;
                }
            }
        }
    }

    // SAFETY: FsEvent and ProcessEvent have identical #[repr(C)] layout.
    EVENTS.output(
        &ctx,
        unsafe { &*(&event as *const _ as *const super::ProcessEvent) },
        0,
    );
    0
}

/// Trace `chmod` — file permission change.
///
/// Tracepoint args: filename (const char *), mode (umode_t)
#[tracepoint(name = "sys_enter_fchmodat", category = "syscalls")]
pub fn sys_enter_fchmodat(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;

    let mut event = unsafe { zero_fs_event() };
    event.event_type = EVENT_CHMOD;
    event.pid = pid;
    event.uid = uid;

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        unsafe { raw_copy(event.comm.as_mut_ptr(), comm.as_ptr(), n) };
    }

    // arg0 = dirfd, arg1 = filename
    let ptr_size = core::mem::size_of::<*const c_char>();

    let pathname_offset = 16 + 1 * ptr_size;
    if let Ok(pathname_ptr) = unsafe { ctx.read_at::<*const c_char>(pathname_offset) } {
        if !pathname_ptr.is_null() {
            let dst = unsafe { slice::from_raw_parts_mut(event.filename.as_mut_ptr(), 64) };
            if let Ok(bytes) =
                unsafe { bpf_probe_read_user_str_bytes(pathname_ptr.cast::<u8>(), dst) }
            {
                let n = bytes.len().min(63);
                unsafe { raw_copy(event.filename.as_mut_ptr(), bytes.as_ptr(), n) };
            }
        }
    }

    // SAFETY: FsEvent and ProcessEvent have identical #[repr(C)] layout.
    EVENTS.output(
        &ctx,
        unsafe { &*(&event as *const _ as *const super::ProcessEvent) },
        0,
    );
    0
}
