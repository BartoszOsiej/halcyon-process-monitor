mod ffi;
mod monitor;
mod tui;
#[cfg(feature = "web")]
mod web;

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use monitor::{Kind, Monitor, Output};
use serde_json::json;

static QUIT: AtomicBool = AtomicBool::new(false);

const BPF_CANDIDATES: &[&str] = &[
    "target/bpfel-unknown-none/release/process-monitor-ebpf",
    "target/release/process-monitor-ebpf",
    "process-monitor-ebpf/target/bpfel-unknown-none/release/process-monitor-ebpf",
    "/usr/local/lib/halcyon/process-monitor-ebpf",
];
#[derive(Parser)]
#[command(
    author,
    version,
    about = "eBPF-based real-time process and file monitor",
    long_about = "Traces execve and openat syscalls via eBPF and watches for \
                  ransomware-style mass file opening. Runs as a TUI by default."
)]
struct Args {
    /// Path to the compiled eBPF program
    #[arg(short, long, value_name = "PATH")]
    bpf: Option<PathBuf>,

    /// Alert when a process opens N+ files within 1 second (0 disables alerts)
    #[arg(long, default_value_t = 50, value_name = "N")]
    alert_threshold: u64,

    /// Newline-delimited JSON output (no TUI)
    #[arg(long)]
    json: bool,

    /// Plain text log output (no TUI)
    #[arg(long, conflicts_with = "json")]
    plain: bool,

    /// Force the TUI even when stdout is not a terminal
    #[arg(long)]
    tui: bool,

    /// Run a 5-second end-to-end self-diagnostic and exit
    #[arg(long, conflicts_with_all = ["json", "plain", "tui"])]
    diagnose: bool,

    /// Only show events matching this extension filter (e.g. "pdf", "enc")
    #[arg(long, value_name = "EXT")]
    filter_ext: Option<String>,

    /// Show top-N files in TUI (default: 8)
    #[arg(long, default_value_t = 8, value_name = "N")]
    top_files: usize,

    /// Start web server with REST API, WebSocket, and dashboard (requires feature "web")
    #[arg(long, value_name = "ADDR", default_value = None)]
    web: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Give the diagnose mode a clear, helpful non-root message before
    // Monitor::start would bail with a generic error.
    if args.diagnose && unsafe { libc::geteuid() } != 0 {
        eprintln!("run with: sudo process-monitor --diagnose");
        return Ok(());
    }

    let bpf_path = resolve_bpf_path(args.bpf.as_ref())?;
    let mut monitor = Monitor::start(&bpf_path, args.alert_threshold).with_context(|| {
        format!(
            "failed to initialize the eBPF monitor using '{}'",
            bpf_path.display()
        )
    })?;

    install_signal_handler();

    let use_tui = args.tui || (!args.json && !args.plain && io::stdout().is_terminal());

    eprintln!("[halcyon] eBPF program: {}", bpf_path.display());
    eprintln!(
        "[halcyon] alert threshold: {} file opens/s",
        args.alert_threshold
    );
    if let Some(ref ext) = args.filter_ext {
        eprintln!("[halcyon] extension filter: .{ext}");
    }

    if args.diagnose {
        run_diagnose(&mut monitor)?;
    } else if let Some(addr_str) = &args.web {
        #[cfg(feature = "web")]
        {
            use std::net::SocketAddr;
            let addr: SocketAddr = addr_str.parse().context("invalid web server address")?;
            let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
            rt.block_on(web::start_web_server(monitor, addr, args.alert_threshold))?;
        }
        #[cfg(not(feature = "web"))]
        {
            let _ = addr_str;
            bail!("--web requires the 'web' feature. Rebuild with: cargo build --features web");
        }
    } else if use_tui {
        eprintln!("[halcyon] TUI mode (q quit, p pause, c clear, arrows scroll, Tab switch panel)");
        tui::run(&mut monitor)?;
    } else if args.json {
        run_json(&mut monitor)?;
    } else {
        run_plain(&mut monitor)?;
    }

    eprintln!("[halcyon] shutdown complete");
    Ok(())
}

fn resolve_bpf_path(explicit: Option<&PathBuf>) -> Result<PathBuf> {
    let mut tried: Vec<String> = Vec::new();

    if let Some(path) = explicit {
        if path.exists() {
            return Ok(path.clone());
        }
        bail!(
            "eBPF program not found at '{}' (build it with ./build.sh or install.sh)",
            path.display()
        );
    }

    // 1. Build tree (CARGO_TARGET_DIR is set by build.sh / install.sh).
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        for sub in ["bpfel-unknown-none/bpf/process-monitor-ebpf", "bpfel-unknown-none/release/process-monitor-ebpf"] {
            let candidate = PathBuf::from(&*dir).join(sub);
            if candidate.exists() {
                return Ok(candidate);
            }
            tried.push(candidate.display().to_string());
        }
    }

    // 2. Relative to the running binary (checked before user-local installs so a
    //    freshly built tree is preferred over a stale ~/.local copy):
    //    - <root>/target/bpfel-unknown-none/... for <root>/target/release/process-monitor
    //    - <bin>/../lib/halcyon/...             for ~/.local/bin/process-monitor
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [
                dir.join("../bpfel-unknown-none/bpf/process-monitor-ebpf"),
                dir.join("../bpfel-unknown-none/release/process-monitor-ebpf"),
                dir.join("../lib/halcyon/process-monitor-ebpf"),
            ] {
                if candidate.exists() {
                    return Ok(candidate);
                }
                tried.push(candidate.display().to_string());
            }
        }
    }

    // 3. User-local install, found through the invoking user's real home.
    //    `sudo` resets $HOME to /root, so derive the home from SUDO_UID (set
    //    by sudo) or the real uid via the passwd database, and also try $HOME.
    let mut homes: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        homes.push(PathBuf::from(home));
    }
    let uid = std::env::var("SUDO_UID")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or_else(|| unsafe { libc::getuid() });
    if let Some(dir) = passwd_dir(uid) {
        homes.push(dir);
    }
    for home in homes {
        let candidate = home.join(".local/lib/halcyon/process-monitor-ebpf");
        if candidate.exists() {
            return Ok(candidate);
        }
        tried.push(candidate.display().to_string());
    }

    // 4. Working-directory-relative candidates and the system install.
    for candidate in BPF_CANDIDATES {
        if Path::new(candidate).exists() {
            return Ok(PathBuf::from(candidate));
        }
        tried.push(candidate.to_string());
    }

    bail!(
        "could not locate the eBPF program; tried: {}. Build it with ./build.sh, \
         install it with install.sh, or pass --bpf PATH",
        tried.join(", ")
    )
}

/// Resolves the home directory for `uid` from the passwd database.
///
/// Used to find user-local installs when running under `sudo` (where `$HOME`
/// points at the target user's home, not the invoking user's).
fn passwd_dir(uid: u32) -> Option<PathBuf> {
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0u8; 4096];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(
            uid,
            &mut pwd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        );
        if rc == 0 && !result.is_null() && !pwd.pw_dir.is_null() {
            let dir = std::ffi::CStr::from_ptr(pwd.pw_dir)
                .to_string_lossy()
                .into_owned();
            Some(PathBuf::from(dir))
        } else {
            None
        }
    }
}

/// End-to-end self-diagnostic: verifies the environment, loads + attaches the
/// eBPF programs, then listens for events for 5 seconds and reports counts.
fn run_diagnose(monitor: &mut Monitor) -> Result<()> {
    println!("=== Halcyon Process Monitor diagnostic ===");
    println!();
    println!("OK: running as root");

    for (cat, name) in [
        ("syscalls", "sys_enter_execve"),
        ("syscalls", "sys_enter_openat"),
    ] {
        let id_path = Path::new("/sys/kernel/tracing/events")
            .join(cat)
            .join(name)
            .join("id");
        match std::fs::read_to_string(&id_path) {
            Ok(id) => println!("OK: tracepoint {cat}/{name} id={}", id.trim()),
            Err(e) => println!("FAIL: cannot read {}: {e}", id_path.display()),
        }
    }

    println!();
    println!("Loading and attaching eBPF programs...");
    println!("Listening for 5 seconds; generate events by running, in another terminal:");
    println!("    ls -la /");
    println!();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut last_report = std::time::Instant::now();
    let mut execs = 0u64;
    let mut opens = 0u64;
    let mut alerts = 0u64;
    while std::time::Instant::now() < deadline {
        for output in monitor.poll() {
            match output {
                Output::Event(ev) => match ev.kind {
                    Kind::Exec => execs += 1,
                    Kind::Open => opens += 1,
                    Kind::Connect | Kind::Accept | Kind::SendTo | Kind::RecvFrom => {}
                },
                Output::Alert(_) => alerts += 1,
            }
        }
        if last_report.elapsed() >= Duration::from_secs(1) {
            last_report = std::time::Instant::now();
            println!(
                "  t-{:.0}s  exec={execs}  open={opens}  alerts={alerts}  lost={}",
                deadline
                    .saturating_duration_since(std::time::Instant::now())
                    .as_secs_f32(),
                monitor.total_lost
            );
        }
        thread::sleep(Duration::from_millis(50));
    }

    println!();
    println!("=== RESULT ===");
    println!("exec events: {execs}");
    println!("open events: {opens}");
    println!("alerts:      {alerts}");
    println!("lost:        {}", monitor.total_lost);
    if execs + opens > 0 {
        println!("SUCCESS: events are flowing through the eBPF pipeline.");
    } else {
        println!("NO EVENTS RECEIVED within the diagnostic window.");
        println!("Check the [halcyon] stderr lines above for attach/load errors.");
    }
    Ok(())
}

fn install_signal_handler() {
    unsafe {
        let handler = handle_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

extern "C" fn handle_signal(_: libc::c_int) {
    QUIT.store(true, Ordering::SeqCst);
}

fn run_json(monitor: &mut Monitor) -> Result<()> {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    loop {
        for output in monitor.poll() {
            match output {
                Output::Event(ev) => {
                    let kind = match ev.kind {
                        Kind::Exec => "exec",
                        Kind::Open => "open",
                        Kind::Connect => "connect",
                        Kind::Accept => "accept",
                        Kind::SendTo => "sendto",
                        Kind::RecvFrom => "recvfrom",
                    };
                    let value = json!({
                        "ts": ev.ts,
                        "type": kind,
                        "pid": ev.pid,
                        "uid": ev.uid,
                        "comm": ev.comm,
                        "file": ev.file,
                    });
                    writeln!(out, "{value}")?;
                }
                Output::Alert(al) => {
                    let value = json!({
                        "ts": al.ts,
                        "type": "alert",
                        "pid": al.pid,
                        "uid": al.uid,
                        "comm": al.comm,
                        "opens_in_1s": al.opens,
                    });
                    writeln!(out, "{value}")?;
                }
            }
        }
        out.flush()?;
        if QUIT.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn run_plain(monitor: &mut Monitor) -> Result<()> {
    use colored::Colorize;
    loop {
        for output in monitor.poll() {
            match output {
                Output::Event(ev) => match ev.kind {
                    Kind::Exec => {
                        println!(
                            "{} {} [{}] {} by uid {}",
                            ev.ts,
                            "EXEC".green().bold(),
                            ev.pid,
                            ev.comm.bold(),
                            ev.uid
                        );
                    }
                    Kind::Open => {
                        if let Some(file) = ev.file {
                            println!(
                                "{} {} [{}] {} -> {}",
                                ev.ts,
                                "OPEN".blue().bold(),
                                ev.pid,
                                ev.comm.dimmed(),
                                file.dimmed()
                            );
                        }
                    }
                    Kind::Connect | Kind::Accept | Kind::SendTo | Kind::RecvFrom => {
                        let kind_str = format!("{:?}", ev.kind).to_uppercase();
                        let addr = ev.file.as_deref().unwrap_or("?");
                        println!(
                            "{} {} [{}] {} -> {}",
                            ev.ts,
                            kind_str.magenta().bold(),
                            ev.pid,
                            ev.comm.dimmed(),
                            addr.dimmed()
                        );
                    }
                },
                Output::Alert(al) => {
                    println!(
                        "{} {} [{}] {} opened {} files in 1s!",
                        al.ts,
                        "SUSPICIOUS".yellow().bold(),
                        al.pid,
                        al.comm.bold(),
                        al.opens
                    );
                }
            }
        }
        if QUIT.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
