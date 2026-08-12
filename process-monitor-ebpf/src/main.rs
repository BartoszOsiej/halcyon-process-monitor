#![no_std]
#![no_main]
#![allow(linker_messages)]

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

pub const EVENT_EXECVE: u8 = 0;
pub const EVENT_OPENAT: u8 = 1;
pub const EVENT_FILENAME_LEN: usize = 64;
pub const EVENT_COMM_LEN: usize = 16;

#[repr(C)]
pub struct ProcessEvent {
    pub event_type: u8,
    pub pid: u32,
    pub uid: u32,
    pub comm: [u8; EVENT_COMM_LEN],
    pub filename: [u8; EVENT_FILENAME_LEN],
}

#[map]
pub static EVENTS: PerfEventArray<ProcessEvent> = PerfEventArray::new(0);

#[tracepoint(name = "sys_enter_execve", category = "syscalls")]
pub fn sys_enter_execve(ctx: TracePointContext) -> u32 {
    emit_event(&ctx, EVENT_EXECVE, 0)
}

#[tracepoint(name = "sys_enter_openat", category = "syscalls")]
pub fn sys_enter_openat(ctx: TracePointContext) -> u32 {
    emit_event(&ctx, EVENT_OPENAT, 1)
}

fn emit_event(ctx: &TracePointContext, event_type: u8, filename_arg: u32) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;

    let mut event = ProcessEvent {
        event_type,
        pid,
        uid,
        comm: [0u8; EVENT_COMM_LEN],
        filename: [0u8; EVENT_FILENAME_LEN],
    };

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(EVENT_COMM_LEN - 1);
        event.comm[..n].copy_from_slice(&comm[..n]);
    }

    // Tracepoint buffer layout (64-bit): struct trace_entry (8 bytes) followed
    // by a long syscall id (8 bytes), then up to 6 syscall arguments. The
    // filename is argument 0 for execve and argument 1 for openat.
    let ptr_size = core::mem::size_of::<*const c_char>();
    let filename_offset = 16 + filename_arg as usize * ptr_size;

    // The filename is a userspace pointer: it must be read with the
    // bpf_probe_read_user helper, never dereferenced directly, or the
    // verifier will reject the program.
    if let Ok(filename) = unsafe { ctx.read_at::<*const c_char>(filename_offset) } {
        if !filename.is_null() {
            let dst = unsafe {
                slice::from_raw_parts_mut(event.filename.as_mut_ptr(), EVENT_FILENAME_LEN)
            };
            if let Ok(bytes) = unsafe { bpf_probe_read_user_str_bytes(filename.cast::<u8>(), dst) } {
                event.filename[..bytes.len()].copy_from_slice(bytes);
            }
        }
    }

    EVENTS.output(ctx, &event, 0);
    0
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
