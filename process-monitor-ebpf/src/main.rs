#![no_std]
#![no_main]
#![allow(linker_messages)]

mod network;

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
pub const EVENT_ARGV_LEN: usize = 128;

/// Event record shared between kernel and userspace (`#[repr(C)]`).
#[repr(C)]
pub struct ProcessEvent {
    pub event_type: u8,
    pub pid: u32,
    pub uid: u32,
    pub comm: [u8; EVENT_COMM_LEN],
    pub filename: [u8; EVENT_FILENAME_LEN],
    pub argv: [u8; EVENT_ARGV_LEN],
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
        argv: [0u8; EVENT_ARGV_LEN],
    };

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(EVENT_COMM_LEN - 1);
        event.comm[..n].copy_from_slice(&comm[..n]);
    }

    let ptr_size = core::mem::size_of::<*const c_char>();
    let filename_offset = 16 + filename_arg as usize * ptr_size;

    // Read the filename from the tracepoint args.
    if let Ok(filename) = unsafe { ctx.read_at::<*const c_char>(filename_offset) } {
        if !filename.is_null() {
            let dst = unsafe {
                slice::from_raw_parts_mut(event.filename.as_mut_ptr(), EVENT_FILENAME_LEN)
            };
            if let Ok(bytes) = unsafe { bpf_probe_read_user_str_bytes(filename.cast::<u8>(), dst) }
            {
                event.filename[..bytes.len()].copy_from_slice(bytes);
            }
        }
    }

    // For execve: read argv[0] (full command path as typed by user).
    // Tracepoint layout: arg0=filename, arg1=argv (userspace char** pointer).
    if event_type == EVENT_EXECVE {
        let argv_offset = 16 + 1 * ptr_size;
        // Read the argv pointer value from the tracepoint context buffer.
        if let Ok(argv_ptr) = unsafe { ctx.read_at::<*const *const c_char>(argv_offset) } {
            if !argv_ptr.is_null() {
                // Dereference argv to get argv[0] (a userspace pointer).
                // Use bpf_probe_read_user since argv_ptr points to userspace memory.
                let mut arg0: *const c_char = core::ptr::null();
                let rc = unsafe {
                    aya_ebpf::helpers::bpf_probe_read_user(
                        (&mut arg0 as *mut *const c_char).cast::<()>(),
                        ptr_size as u32,
                        argv_ptr.cast::<()>(),
                    )
                };
                if rc == 0 && !arg0.is_null() {
                    let dst = unsafe {
                        slice::from_raw_parts_mut(event.argv.as_mut_ptr(), EVENT_ARGV_LEN)
                    };
                    if let Ok(bytes) =
                        unsafe { bpf_probe_read_user_str_bytes(arg0.cast::<u8>(), dst) }
                    {
                        event.argv[..bytes.len()].copy_from_slice(bytes);
                    }
                }
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
