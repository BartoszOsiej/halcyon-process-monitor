//! C FFI bindings for libhalcyon.
//!
//! This module exposes the Halcyon eBPF process monitor as a C-compatible library,
//! allowing integration with C, C++, Python (via cffi/ctypes), and other languages.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::sync::Mutex;
use std::time::Instant;

use crate::monitor::{Kind, Monitor};

// ── C types ───────────────────────────────────────────────────────────────

#[repr(C)]
pub struct HalcyonMonitor {
    inner: Mutex<Monitor>,
    threshold: u64,
    started: Instant,
}

#[repr(C)]
pub struct HalcyonEvent {
    pub kind: c_int,          // 0 = EXEC, 1 = OPEN
    pub pid: u32,
    pub uid: u32,
    pub comm: *mut c_char,
    pub file: *mut c_char,    // NULL for EXEC events
    pub argv: *mut c_char,    // NULL for OPEN events
    pub timestamp: *mut c_char,
}

#[repr(C)]
pub struct HalcyonProcessStats {
    pub pid: u32,
    pub ppid: u32,
    pub comm: *mut c_char,
    pub total_opens: u64,
    pub total_execs: u64,
    pub alerts: u64,
    pub window_opens: u64,
}

#[repr(C)]
pub struct HalcyonFileRank {
    pub path: *mut c_char,
    pub count: u64,
    pub extension: *mut c_char,
    pub entropy: f64,
}

#[repr(C)]
pub struct HalcyonStats {
    pub total_events: u64,
    pub total_lost: u64,
    pub uptime_secs: u64,
    pub active_pids: u64,
    pub threshold: u64,
}

// ── Error codes ───────────────────────────────────────────────────────────

pub const HALCYON_OK: c_int = 0;
pub const HALCYON_ERR_NOMEM: c_int = -1;
pub const HALCYON_ERR_INVAL: c_int = -2;
pub const HALCYON_ERR_PERM: c_int = -3;
pub const HALCYON_ERR_IO: c_int = -4;
pub const HALCYON_ERR_NOT_FOUND: c_int = -5;

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

// ── FFI functions ─────────────────────────────────────────────────────────

/// Returns the library version string.
#[unsafe(no_mangle)]
pub extern "C" fn halcyon_version() -> *const c_char {
    c"0.3.0".as_ptr()
}

/// Returns a human-readable error message.
#[unsafe(no_mangle)]
pub extern "C" fn halcyon_strerror(err: c_int) -> *const c_char {
    match err {
        HALCYON_OK => c"success".as_ptr(),
        HALCYON_ERR_NOMEM => c"out of memory".as_ptr(),
        HALCYON_ERR_INVAL => c"invalid argument".as_ptr(),
        HALCYON_ERR_PERM => c"permission denied".as_ptr(),
        HALCYON_ERR_IO => c"I/O error".as_ptr(),
        HALCYON_ERR_NOT_FOUND => c"not found".as_ptr(),
        _ => c"unknown error".as_ptr(),
    }
}

/// Returns the last error message (thread-local).
#[unsafe(no_mangle)]
pub extern "C" fn halcyon_last_error() -> *const c_char {
    LAST_ERROR.with(|e| match e.borrow().as_ref() {
        Some(s) => s.as_ptr(),
        None => ptr::null(),
    })
}

/// Creates a new monitor instance.
///
/// # Safety
/// `bpf_path` must be a valid null-terminated C string.
/// `out` must be a valid pointer to a `*mut HalcyonMonitor`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_monitor_create(
    bpf_path: *const c_char,
    threshold: u64,
    out: *mut *mut HalcyonMonitor,
) -> c_int {
    if bpf_path.is_null() || out.is_null() {
        set_last_error("null pointer argument");
        return HALCYON_ERR_INVAL;
    }

    let path_str = match unsafe { CStr::from_ptr(bpf_path) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("invalid UTF-8 in bpf_path");
            return HALCYON_ERR_INVAL;
        }
    };

    let path = std::path::Path::new(path_str);
    let monitor = match Monitor::start(path, threshold) {
        Ok(m) => m,
        Err(e) => {
            set_last_error(&format!("failed to create monitor: {e}"));
            return HALCYON_ERR_IO;
        }
    };

    let halcyon = Box::new(HalcyonMonitor {
        inner: Mutex::new(monitor),
        threshold,
        started: Instant::now(),
    });

    unsafe {
        *out = Box::into_raw(halcyon);
    }
    HALCYON_OK
}

/// Destroys a monitor instance.
///
/// # Safety
/// `monitor` must have been created with `halcyon_monitor_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_monitor_destroy(monitor: *mut HalcyonMonitor) {
    if !monitor.is_null() {
        unsafe { drop(Box::from_raw(monitor)); }
    }
}

/// Polls for new events.
///
/// # Safety
/// `events` must point to an array of `max_events` `HalcyonEvent` structs.
/// `count` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_monitor_poll(
    monitor: *mut HalcyonMonitor,
    events: *mut HalcyonEvent,
    max_events: u32,
    count: *mut u32,
) -> c_int {
    if monitor.is_null() || events.is_null() || count.is_null() {
        set_last_error("null pointer argument");
        return HALCYON_ERR_INVAL;
    }

    let monitor = unsafe { &mut *monitor };
    let mut mon = match monitor.inner.lock() {
        Ok(m) => m,
        Err(_) => {
            set_last_error("monitor lock poisoned");
            return HALCYON_ERR_IO;
        }
    };

    let outputs = mon.poll();
    let n = outputs.len().min(max_events as usize);

    for (i, output) in outputs.into_iter().take(n).enumerate() {
        let event = unsafe { &mut *events.add(i) };
        match output {
            crate::monitor::Output::Event(ev) => {
                event.kind = match ev.kind {
                    Kind::Exec => 0,
                    Kind::Open => 1,
                    Kind::Connect => 2,
                    Kind::Accept => 3,
                    Kind::SendTo => 4,
                    Kind::RecvFrom => 5,
                };
                event.pid = ev.pid;
                event.uid = ev.uid;
                event.comm = CString::new(ev.comm).unwrap_or_default().into_raw();
                event.file = ev
                    .file
                    .map(|f| CString::new(f).unwrap_or_default().into_raw())
                    .unwrap_or(ptr::null_mut());
                event.argv = ev
                    .argv
                    .map(|a| CString::new(a).unwrap_or_default().into_raw())
                    .unwrap_or(ptr::null_mut());
                event.timestamp = CString::new(ev.ts).unwrap_or_default().into_raw();
            }
            crate::monitor::Output::Alert(al) => {
                event.kind = -1; // Alert marker
                event.pid = al.pid;
                event.uid = al.uid;
                event.comm = CString::new(al.comm).unwrap_or_default().into_raw();
                event.file = ptr::null_mut();
                event.argv = ptr::null_mut();
                event.timestamp = CString::new(al.ts).unwrap_or_default().into_raw();
            }
        }
    }

    unsafe { *count = n as u32; }
    HALCYON_OK
}

/// Returns monitor statistics.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_monitor_stats(
    monitor: *mut HalcyonMonitor,
    stats: *mut HalcyonStats,
) -> c_int {
    if monitor.is_null() || stats.is_null() {
        set_last_error("null pointer argument");
        return HALCYON_ERR_INVAL;
    }

    let monitor = unsafe { &*monitor };
    let mon = match monitor.inner.lock() {
        Ok(m) => m,
        Err(_) => {
            set_last_error("monitor lock poisoned");
            return HALCYON_ERR_IO;
        }
    };

    let out = unsafe { &mut *stats };
    out.total_events = mon.total_events;
    out.total_lost = mon.total_lost;
    out.uptime_secs = mon.uptime().as_secs();
    out.active_pids = mon.stats_sorted().len() as u64;
    out.threshold = mon.threshold;

    HALCYON_OK
}

/// Updates the alert threshold at runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_monitor_set_threshold(
    monitor: *mut HalcyonMonitor,
    threshold: u64,
) -> c_int {
    if monitor.is_null() {
        set_last_error("null pointer argument");
        return HALCYON_ERR_INVAL;
    }

    let monitor = unsafe { &mut *monitor };
    match monitor.inner.lock() {
        Ok(mut m) => {
            m.threshold = threshold;
            monitor.threshold = threshold;
        }
        Err(_) => {
            set_last_error("monitor lock poisoned");
            return HALCYON_ERR_IO;
        }
    }

    HALCYON_OK
}

/// Returns tracked processes sorted by window opens.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_monitor_processes(
    monitor: *mut HalcyonMonitor,
    stats: *mut HalcyonProcessStats,
    max_stats: u32,
    count: *mut u32,
) -> c_int {
    if monitor.is_null() || stats.is_null() || count.is_null() {
        set_last_error("null pointer argument");
        return HALCYON_ERR_INVAL;
    }

    let monitor = unsafe { &*monitor };
    let mon = match monitor.inner.lock() {
        Ok(m) => m,
        Err(_) => {
            set_last_error("monitor lock poisoned");
            return HALCYON_ERR_IO;
        }
    };

    let sorted = mon.stats_sorted();
    let n = sorted.len().min(max_stats as usize);

    for (i, s) in sorted.into_iter().take(n).enumerate() {
        let out = unsafe { &mut *stats.add(i) };
        out.pid = s.pid;
        out.ppid = s.ppid;
        out.comm = CString::new(s.comm).unwrap_or_default().into_raw();
        out.total_opens = s.total_opens;
        out.total_execs = s.total_execs;
        out.alerts = s.alerts;
        out.window_opens = s.window_opens;
    }

    unsafe { *count = n as u32; }
    HALCYON_OK
}

/// Returns top-N most-opened files.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_monitor_top_files(
    monitor: *mut HalcyonMonitor,
    files: *mut HalcyonFileRank,
    n: u32,
    count: *mut u32,
) -> c_int {
    if monitor.is_null() || files.is_null() || count.is_null() {
        set_last_error("null pointer argument");
        return HALCYON_ERR_INVAL;
    }

    let monitor = unsafe { &*monitor };
    let mon = match monitor.inner.lock() {
        Ok(m) => m,
        Err(_) => {
            set_last_error("monitor lock poisoned");
            return HALCYON_ERR_IO;
        }
    };

    let top = mon.top_files(n as usize);
    let len = top.len().min(n as usize);

    for (i, f) in top.into_iter().take(len).enumerate() {
        let out = unsafe { &mut *files.add(i) };
        out.path = CString::new(f.path).unwrap_or_default().into_raw();
        out.count = f.count;
        out.extension = CString::new(f.extension).unwrap_or_default().into_raw();
        out.entropy = f.entropy;
    }

    unsafe { *count = len as u32; }
    HALCYON_OK
}

/// Frees a C string returned by the API.
///
/// # Safety
/// `s` must have been returned by a halcyon function, or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)); }
    }
}

/// Frees an array of events.
///
/// # Safety
/// `events` must have been returned by `halcyon_monitor_poll`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_free_events(events: *mut HalcyonEvent, count: u32) {
    if events.is_null() {
        return;
    }
    for i in 0..count as usize {
        let event = unsafe { &*events.add(i) };
        if !event.comm.is_null() {
            unsafe { halcyon_free_string(event.comm); }
        }
        if !event.file.is_null() {
            unsafe { halcyon_free_string(event.file); }
        }
        if !event.argv.is_null() {
            unsafe { halcyon_free_string(event.argv); }
        }
        if !event.timestamp.is_null() {
            unsafe { halcyon_free_string(event.timestamp); }
        }
    }
    unsafe { drop(Vec::from_raw_parts(events, count as usize, count as usize)); }
}

/// Frees an array of process stats.
///
/// # Safety
/// `stats` must have been returned by `halcyon_monitor_processes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_free_processes(stats: *mut HalcyonProcessStats, count: u32) {
    if stats.is_null() {
        return;
    }
    for i in 0..count as usize {
        let s = unsafe { &*stats.add(i) };
        if !s.comm.is_null() {
            unsafe { halcyon_free_string(s.comm); }
        }
    }
    unsafe { drop(Vec::from_raw_parts(stats, count as usize, count as usize)); }
}

/// Frees an array of file ranks.
///
/// # Safety
/// `files` must have been returned by `halcyon_monitor_top_files`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn halcyon_free_files(files: *mut HalcyonFileRank, count: u32) {
    if files.is_null() {
        return;
    }
    for i in 0..count as usize {
        let f = unsafe { &*files.add(i) };
        if !f.path.is_null() {
            unsafe { halcyon_free_string(f.path); }
        }
        if !f.extension.is_null() {
            unsafe { halcyon_free_string(f.extension); }
        }
    }
    unsafe { drop(Vec::from_raw_parts(files, count as usize, count as usize)); }
}
