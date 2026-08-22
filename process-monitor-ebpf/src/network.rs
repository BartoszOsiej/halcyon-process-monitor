//! Network eBPF tracepoint programs for Halcyon Process Monitor.
//!
//! These programs trace network-related syscalls and emit events through
//! the same PerfEventArray used by process events.

use core::slice;

use aya_ebpf::{
    cty::c_char,
    helpers::{
        bpf_get_current_comm, bpf_get_current_pid_tgid, bpf_get_current_uid_gid,
        bpf_probe_read_user, bpf_probe_read_user_str_bytes,
    },
    macros::{map, tracepoint},
    maps::PerfEventArray,
    programs::TracePointContext,
};

pub const EVENT_CONNECT: u8 = 2;
pub const EVENT_ACCEPT: u8 = 3;
pub const EVENT_SENDTO: u8 = 4;
pub const EVENT_RECVFROM: u8 = 5;

/// Network event record with additional fields for socket operations.
#[repr(C)]
pub struct NetworkEvent {
    pub event_type: u8,
    pub pid: u32,
    pub uid: u32,
    pub comm: [u8; 16],
    pub filename: [u8; 64], // Remote address (IP:port or path)
    pub argv: [u8; 128],    // Extra context (e.g., bytes count)
}

#[map]
pub static NETWORK_EVENTS: PerfEventArray<NetworkEvent> = PerfEventArray::new(0);

// ── eBPF-safe byte helpers (avoids LLVM memset/memcpy builtins) ──────────

/// Zero-initialize a NetworkEvent on the stack without triggering memset.
/// SAFETY: BPF stack memory is zeroed by the kernel before the program runs,
/// so this is equivalent to `core::mem::zeroed()` but avoids the LLVM builtin.
#[inline(always)]
unsafe fn zero_event() -> NetworkEvent {
    core::mem::MaybeUninit::uninit().assume_init()
}

/// Copy `len` bytes from `src` to `dst` without triggering memcpy.
#[inline(always)]
unsafe fn raw_copy(dst: *mut u8, src: *const u8, len: usize) {
    let mut i = 0;
    while i < len {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
}

/// Trace `connect` syscall — captures remote socket address.
///
/// Tracepoint args: fd (int), uservaddr (struct sockaddr *), addrlen (int)
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

    // Read sockaddr pointer from tracepoint args (arg1 = uservaddr)
    let ptr_size = core::mem::size_of::<*const c_char>();
    let sockaddr_offset = 16 + 1 * ptr_size;

    if let Ok(sockaddr_ptr) = unsafe { ctx.read_at::<*const u8>(sockaddr_offset) } {
        if !sockaddr_ptr.is_null() {
            // Read sa_family (first 2 bytes of sockaddr)
            if let Ok(sa_family) = unsafe { bpf_probe_read_user::<u16>(sockaddr_ptr.cast()) } {
                // AF_INET (2): read sin_port (2 bytes at offset 2) + sin_addr (4 bytes at offset 4)
                if sa_family == 2 {
                    if let Ok(port) = unsafe {
                        bpf_probe_read_user::<u16>(sockaddr_ptr.add(2).cast())
                    } {
                        if let Ok(addr) = unsafe {
                            bpf_probe_read_user::<[u8; 4]>(sockaddr_ptr.add(4).cast())
                        } {
                            // Format: "IP:PORT" (e.g., "192.168.1.1:443")
                            let ip_fmt = format_ipv4_port(addr, port);
                            let len = ip_fmt.iter().position(|&b| b == 0).unwrap_or(21);
                            let copy_len = len.min(63);
                            unsafe {
                                raw_copy(
                                    event.filename.as_mut_ptr(),
                                    ip_fmt.as_ptr(),
                                    copy_len,
                                )
                            };
                        }
                    }
                }
                // AF_INET6 (10): simplified — just note it's IPv6
                else if sa_family == 10 {
                    event.filename[..6].copy_from_slice(b"[IPv6]");
                }
                // AF_UNIX (1): read path
                else if sa_family == 1 {
                    // sun_path starts at offset 2 in sockaddr_un
                    let sun_path_ptr = unsafe { sockaddr_ptr.add(2) };
                    let dst = unsafe { slice::from_raw_parts_mut(event.filename.as_mut_ptr(), 64) };
                    if let Ok(bytes) =
                        unsafe { bpf_probe_read_user_str_bytes(sun_path_ptr.cast::<u8>(), dst) }
                    {
                        let n = bytes.len().min(63);
                        unsafe { raw_copy(event.filename.as_mut_ptr(), bytes.as_ptr(), n) };
                    }
                }
            }
        }
    }

    NETWORK_EVENTS.output(&ctx, &event, 0);
    0
}

/// Trace `accept` syscall — captures remote socket address.
///
/// Tracepoint args: fd (int), upeer_sockaddr (struct sockaddr *), upeer_addrlen (int)
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

    // Similar to connect — read remote address
    let ptr_size = core::mem::size_of::<*const c_char>();
    let sockaddr_offset = 16 + 1 * ptr_size;

    if let Ok(sockaddr_ptr) = unsafe { ctx.read_at::<*const u8>(sockaddr_offset) } {
        if !sockaddr_ptr.is_null() {
            if let Ok(sa_family) = unsafe { bpf_probe_read_user::<u16>(sockaddr_ptr.cast()) } {
                if sa_family == 2 {
                    if let Ok(port) = unsafe {
                        bpf_probe_read_user::<u16>(sockaddr_ptr.add(2).cast())
                    } {
                        if let Ok(addr) = unsafe {
                            bpf_probe_read_user::<[u8; 4]>(sockaddr_ptr.add(4).cast())
                        } {
                            let ip_fmt = format_ipv4_port(addr, port);
                            let len = ip_fmt.iter().position(|&b| b == 0).unwrap_or(21);
                            let copy_len = len.min(63);
                            unsafe {
                                raw_copy(
                                    event.filename.as_mut_ptr(),
                                    ip_fmt.as_ptr(),
                                    copy_len,
                                )
                            };
                        }
                    }
                } else if sa_family == 10 {
                    event.filename[..6].copy_from_slice(b"[IPv6]");
                }
            }
        }
    }

    NETWORK_EVENTS.output(&ctx, &event, 0);
    0
}

/// Trace `sendto` syscall — captures destination address and size.
///
/// Tracepoint args: fd (int), buff (void *), size (size_t),
///                  flags (int), addr (struct sockaddr *), addr_len (int)
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

    let ptr_size = core::mem::size_of::<*const c_char>();

    // Read size (arg2 = size_t)
    let size_offset = 16 + 2 * ptr_size;
    if let Ok(size) = unsafe { ctx.read_at::<usize>(size_offset) } {
        let size_str = format_usize(size);
        let len = size_str.iter().position(|&b| b == 0).unwrap_or(20);
        let copy_len = len.min(127);
        unsafe { raw_copy(event.argv.as_mut_ptr(), size_str.as_ptr(), copy_len) };
    }

    // Read destination address if present (arg4 = struct sockaddr *)
    let addr_offset = 16 + 4 * ptr_size;
    if let Ok(addr_ptr) = unsafe { ctx.read_at::<*const u8>(addr_offset) } {
        if !addr_ptr.is_null() {
            if let Ok(sa_family) = unsafe { bpf_probe_read_user::<u16>(addr_ptr.cast()) } {
                if sa_family == 2 {
                    if let Ok(port) = unsafe {
                        bpf_probe_read_user::<u16>(addr_ptr.add(2).cast())
                    } {
                        if let Ok(addr) = unsafe {
                            bpf_probe_read_user::<[u8; 4]>(addr_ptr.add(4).cast())
                        } {
                            let ip_fmt = format_ipv4_port(addr, port);
                            let len = ip_fmt.iter().position(|&b| b == 0).unwrap_or(21);
                            let copy_len = len.min(63);
                            unsafe {
                                raw_copy(
                                    event.filename.as_mut_ptr(),
                                    ip_fmt.as_ptr(),
                                    copy_len,
                                )
                            };
                        }
                    }
                }
            }
        }
    }

    NETWORK_EVENTS.output(&ctx, &event, 0);
    0
}

/// Trace `recvfrom` syscall — captures source address and size.
///
/// Tracepoint args: fd (int), ubuf (void *), size (size_t),
///                  flags (int), addr (struct sockaddr *), addr_len (int)
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

    let ptr_size = core::mem::size_of::<*const c_char>();

    // Read size (arg2 = size_t)
    let size_offset = 16 + 2 * ptr_size;
    if let Ok(size) = unsafe { ctx.read_at::<usize>(size_offset) } {
        let size_str = format_usize(size);
        let len = size_str.iter().position(|&b| b == 0).unwrap_or(20);
        let copy_len = len.min(127);
        unsafe { raw_copy(event.argv.as_mut_ptr(), size_str.as_ptr(), copy_len) };
    }

    // Read source address if present (arg4 = struct sockaddr *)
    let addr_offset = 16 + 4 * ptr_size;
    if let Ok(addr_ptr) = unsafe { ctx.read_at::<*const u8>(addr_offset) } {
        if !addr_ptr.is_null() {
            if let Ok(sa_family) = unsafe { bpf_probe_read_user::<u16>(addr_ptr.cast()) } {
                if sa_family == 2 {
                    if let Ok(port) = unsafe {
                        bpf_probe_read_user::<u16>(addr_ptr.add(2).cast())
                    } {
                        if let Ok(addr) = unsafe {
                            bpf_probe_read_user::<[u8; 4]>(addr_ptr.add(4).cast())
                        } {
                            let ip_fmt = format_ipv4_port(addr, port);
                            let len = ip_fmt.iter().position(|&b| b == 0).unwrap_or(21);
                            let copy_len = len.min(63);
                            unsafe {
                                raw_copy(
                                    event.filename.as_mut_ptr(),
                                    ip_fmt.as_ptr(),
                                    copy_len,
                                )
                            };
                        }
                    }
                }
            }
        }
    }

    NETWORK_EVENTS.output(&ctx, &event, 0);
    0
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Format IPv4 address and port as "A.B.C.D:PORT".
fn format_ipv4_port(addr: [u8; 4], port: u16) -> [u8; 21] {
    // Max length: "255.255.255.255:65535" = 21 bytes
    let mut buf = unsafe { core::mem::MaybeUninit::<[u8; 21]>::uninit().assume_init() };
    let mut pos = 0;

    // Write IP
    let mut i = 0;
    while i < 4 {
        let octet = addr[i];
        if i > 0 {
            buf[pos] = b'.';
            pos += 1;
        }
        if octet >= 100 {
            buf[pos] = b'0' + octet / 100;
            pos += 1;
            buf[pos] = b'0' + (octet / 10) % 10;
            pos += 1;
            buf[pos] = b'0' + octet % 10;
            pos += 1;
        } else if octet >= 10 {
            buf[pos] = b'0' + octet / 10;
            pos += 1;
            buf[pos] = b'0' + octet % 10;
            pos += 1;
        } else {
            buf[pos] = b'0' + octet;
            pos += 1;
        }
        i += 1;
    }

    // Write ":PORT"
    buf[pos] = b':';
    pos += 1;

    if port >= 10000 {
        buf[pos] = b'0' + (port / 10000) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 1000) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 100) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 10) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + (port % 10) as u8;
    } else if port >= 1000 {
        buf[pos] = b'0' + (port / 1000) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 100) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 10) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + (port % 10) as u8;
    } else if port >= 100 {
        buf[pos] = b'0' + (port / 100) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 10) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + (port % 10) as u8;
    } else if port >= 10 {
        buf[pos] = b'0' + (port / 10) as u8;
        pos += 1;
        buf[pos] = b'0' + (port % 10) as u8;
    } else {
        buf[pos] = b'0' + port as u8;
    }

    buf
}

/// Format usize as decimal string (no alloc, eBPF safe).
fn format_usize(val: usize) -> [u8; 20] {
    let mut buf = unsafe { core::mem::MaybeUninit::<[u8; 20]>::uninit().assume_init() };
    let mut pos = 19;
    let mut v = val;

    if v == 0 {
        buf[19] = b'0';
        return buf;
    }

    while v > 0 && pos > 0 {
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
        pos -= 1;
    }

    // Shift to start
    let start = pos + 1;
    let len = 20 - start;
    let mut result = unsafe { core::mem::MaybeUninit::<[u8; 20]>::uninit().assume_init() };
    unsafe { raw_copy(result.as_mut_ptr(), buf[start..].as_ptr(), len) };
    result
}
