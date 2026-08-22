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

use crate::ProcessEvent;

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

/// Trace `connect` syscall — captures remote socket address.
///
/// Tracepoint args: fd (int), uservaddr (struct sockaddr *), addrlen (int)
#[tracepoint(name = "sys_enter_connect", category = "syscalls")]
pub fn sys_enter_connect(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;

    let mut event = NetworkEvent {
        event_type: EVENT_CONNECT,
        pid,
        uid,
        comm: [0u8; 16],
        filename: [0u8; 64],
        argv: [0u8; 128],
    };

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        event.comm[..n].copy_from_slice(&comm[..n]);
    }

    // Read sockaddr pointer from tracepoint args (arg1 = uservaddr)
    let ptr_size = core::mem::size_of::<*const c_char>();
    let sockaddr_offset = 16 + 1 * ptr_size;

    if let Ok(sockaddr_ptr) = unsafe { ctx.read_at::<*const u8>(sockaddr_offset) } {
        if !sockaddr_ptr.is_null() {
            // Read sa_family (first 2 bytes of sockaddr)
            let mut sa_family: u16 = 0;
            let rc = unsafe {
                bpf_probe_read_user(
                    (&mut sa_family as *mut u16).cast::<()>(),
                    2,
                    sockaddr_ptr.cast::<()>(),
                )
            };
            if rc == 0 {
                // AF_INET (2): read sin_addr (4 bytes at offset 4) + sin_port (2 bytes at offset 2)
                if sa_family == 2 {
                    let mut port: u16 = 0;
                    let mut addr: [u8; 4] = [0; 4];
                    unsafe {
                        bpf_probe_read_user(
                            (&mut port as *mut u16).cast::<()>(),
                            2,
                            sockaddr_ptr.add(2).cast::<()>(),
                        );
                        bpf_probe_read_user(
                            addr.as_mut_ptr().cast::<()>(),
                            4,
                            sockaddr_ptr.add(4).cast::<()>(),
                        );
                    }
                    // Format: "IP:PORT" (e.g., "192.168.1.1:443")
                    let ip_fmt = format_ipv4_port(addr, port);
                    let bytes = ip_fmt.as_bytes();
                    let len = bytes.len().min(63);
                    event.filename[..len].copy_from_slice(&bytes[..len]);
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
                        event.filename[..bytes.len()].copy_from_slice(bytes);
                    }
                }
            }
        }
    }

    NETWORK_EVENTS.output(ctx, &event, 0);
    0
}

/// Trace `accept` syscall — captures remote socket address.
///
/// Tracepoint args: fd (int), upeer_sockaddr (struct sockaddr *), upeer_addrlen (int)
#[tracepoint(name = "sys_enter_accept", category = "syscalls")]
pub fn sys_enter_accept(ctx: TracePointContext) -> u32 {
    let pid = (bpf_get_current_pid_tgid() >> 32) as u32;
    let uid = bpf_get_current_uid_gid() as u32;

    let mut event = NetworkEvent {
        event_type: EVENT_ACCEPT,
        pid,
        uid,
        comm: [0u8; 16],
        filename: [0u8; 64],
        argv: [0u8; 128],
    };

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        event.comm[..n].copy_from_slice(&comm[..n]);
    }

    // Similar to connect — read remote address
    let ptr_size = core::mem::size_of::<*const c_char>();
    let sockaddr_offset = 16 + 1 * ptr_size;

    if let Ok(sockaddr_ptr) = unsafe { ctx.read_at::<*const u8>(sockaddr_offset) } {
        if !sockaddr_ptr.is_null() {
            let mut sa_family: u16 = 0;
            let rc = unsafe {
                bpf_probe_read_user(
                    (&mut sa_family as *mut u16).cast::<()>(),
                    2,
                    sockaddr_ptr.cast::<()>(),
                )
            };
            if rc == 0 && sa_family == 2 {
                let mut port: u16 = 0;
                let mut addr: [u8; 4] = [0; 4];
                unsafe {
                    bpf_probe_read_user(
                        (&mut port as *mut u16).cast::<()>(),
                        2,
                        sockaddr_ptr.add(2).cast::<()>(),
                    );
                    bpf_probe_read_user(
                        addr.as_mut_ptr().cast::<()>(),
                        4,
                        sockaddr_ptr.add(4).cast::<()>(),
                    );
                }
                let ip_fmt = format_ipv4_port(addr, port);
                let bytes = ip_fmt.as_bytes();
                let len = bytes.len().min(63);
                event.filename[..len].copy_from_slice(&bytes[..len]);
            } else if sa_family == 10 {
                event.filename[..6].copy_from_slice(b"[IPv6]");
            }
        }
    }

    NETWORK_EVENTS.output(ctx, &event, 0);
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

    let mut event = NetworkEvent {
        event_type: EVENT_SENDTO,
        pid,
        uid,
        comm: [0u8; 16],
        filename: [0u8; 64],
        argv: [0u8; 128],
    };

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        event.comm[..n].copy_from_slice(&comm[..n]);
    }

    let ptr_size = core::mem::size_of::<*const c_char>();

    // Read size (arg2 = size_t)
    let size_offset = 16 + 2 * ptr_size;
    if let Ok(size) = unsafe { ctx.read_at::<usize>(size_offset) } {
        let size_str = format_usize(size);
        let bytes = size_str.as_bytes();
        let len = bytes.len().min(127);
        event.argv[..len].copy_from_slice(&bytes[..len]);
    }

    // Read destination address if present (arg4 = struct sockaddr *)
    let addr_offset = 16 + 4 * ptr_size;
    if let Ok(addr_ptr) = unsafe { ctx.read_at::<*const u8>(addr_offset) } {
        if !addr_ptr.is_null() {
            let mut sa_family: u16 = 0;
            let rc = unsafe {
                bpf_probe_read_user(
                    (&mut sa_family as *mut u16).cast::<()>(),
                    2,
                    addr_ptr.cast::<()>(),
                )
            };
            if rc == 0 && sa_family == 2 {
                let mut port: u16 = 0;
                let mut addr: [u8; 4] = [0; 4];
                unsafe {
                    bpf_probe_read_user(
                        (&mut port as *mut u16).cast::<()>(),
                        2,
                        addr_ptr.add(2).cast::<()>(),
                    );
                    bpf_probe_read_user(
                        addr.as_mut_ptr().cast::<()>(),
                        4,
                        addr_ptr.add(4).cast::<()>(),
                    );
                }
                let ip_fmt = format_ipv4_port(addr, port);
                let bytes = ip_fmt.as_bytes();
                let len = bytes.len().min(63);
                event.filename[..len].copy_from_slice(&bytes[..len]);
            }
        }
    }

    NETWORK_EVENTS.output(ctx, &event, 0);
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

    let mut event = NetworkEvent {
        event_type: EVENT_RECVFROM,
        pid,
        uid,
        comm: [0u8; 16],
        filename: [0u8; 64],
        argv: [0u8; 128],
    };

    if let Ok(comm) = bpf_get_current_comm() {
        let n = comm.len().min(15);
        event.comm[..n].copy_from_slice(&comm[..n]);
    }

    let ptr_size = core::mem::size_of::<*const c_char>();

    // Read size (arg2 = size_t)
    let size_offset = 16 + 2 * ptr_size;
    if let Ok(size) = unsafe { ctx.read_at::<usize>(size_offset) } {
        let size_str = format_usize(size);
        let bytes = size_str.as_bytes();
        let len = bytes.len().min(127);
        event.argv[..len].copy_from_slice(&bytes[..len]);
    }

    // Read source address if present (arg4 = struct sockaddr *)
    let addr_offset = 16 + 4 * ptr_size;
    if let Ok(addr_ptr) = unsafe { ctx.read_at::<*const u8>(addr_offset) } {
        if !addr_ptr.is_null() {
            let mut sa_family: u16 = 0;
            let rc = unsafe {
                bpf_probe_read_user(
                    (&mut sa_family as *mut u16).cast::<()>(),
                    2,
                    addr_ptr.cast::<()>(),
                )
            };
            if rc == 0 && sa_family == 2 {
                let mut port: u16 = 0;
                let mut addr: [u8; 4] = [0; 4];
                unsafe {
                    bpf_probe_read_user(
                        (&mut port as *mut u16).cast::<()>(),
                        2,
                        addr_ptr.add(2).cast::<()>(),
                    );
                    bpf_probe_read_user(
                        addr.as_mut_ptr().cast::<()>(),
                        4,
                        addr_ptr.add(4).cast::<()>(),
                    );
                }
                let ip_fmt = format_ipv4_port(addr, port);
                let bytes = ip_fmt.as_bytes();
                let len = bytes.len().min(63);
                event.filename[..len].copy_from_slice(&bytes[..len]);
            }
        }
    }

    NETWORK_EVENTS.output(ctx, &event, 0);
    0
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Format IPv4 address and port as "A.B.C.D:PORT".
fn format_ipv4_port(addr: [u8; 4], port: u16) -> [u8; 21] {
    // Max length: "255.255.255.255:65535" = 21 bytes
    let mut buf = [0u8; 21];
    let mut pos = 0;

    // Write IP
    for i in 0..4 {
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
        pos += 1;
    } else if port >= 1000 {
        buf[pos] = b'0' + (port / 1000) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 100) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 10) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + (port % 10) as u8;
        pos += 1;
    } else if port >= 100 {
        buf[pos] = b'0' + (port / 100) as u8;
        pos += 1;
        buf[pos] = b'0' + ((port / 10) % 10) as u8;
        pos += 1;
        buf[pos] = b'0' + (port % 10) as u8;
        pos += 1;
    } else if port >= 10 {
        buf[pos] = b'0' + (port / 10) as u8;
        pos += 1;
        buf[pos] = b'0' + (port % 10) as u8;
        pos += 1;
    } else {
        buf[pos] = b'0' + port as u8;
        pos += 1;
    }

    buf
}

/// Format usize as decimal string (no alloc, eBPF safe).
fn format_usize(val: usize) -> [u8; 20] {
    let mut buf = [0u8; 20];
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
    let mut result = [0u8; 20];
    result[..len].copy_from_slice(&buf[start..]);
    result
}
