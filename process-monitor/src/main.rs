use anyhow::{Context, Result};
use aya::{
    programs::TracePoint,
    Bpf,
};
use aya::maps::perf::PerfBufferReader;
use aya::maps::PerfEventArray;
use chrono::Local;
use clap::Parser;
use colored::*;
use log::{info, warn, error};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const EVENT_EXECVE: u8 = 0;
const EVENT_OPENAT: u8 = 1;
const EVENT_COMM_LEN: usize = 16;
const EVENT_FILENAME_LEN: usize = 64;

#[repr(C)]
struct ProcessEvent {
    event_type: u8,
    pid: u32,
    uid: u32,
    comm: [i8; EVENT_COMM_LEN],
    filename: [i8; EVENT_FILENAME_LEN],
}

unsafe impl aya::Pod for ProcessEvent {}

#[derive(Parser)]
#[command(author, version, about = "Real-time process monitor using eBPF")]
struct Args {
    #[arg(short, long, default_value = "target/release/process-monitor-ebpf")]
    bpf: String,

    #[arg(long, default_value = "50")]
    alert_threshold: u64,

    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone)]
struct FileOpenTracker {
    count: u64,
    window_start: Instant,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    println!("{}", "Halcyon Process Monitor v0.1.0".bold().cyan());
    println!("{}", "eBPF-based process and file operation monitoring".dimmed());
    println!("Alert threshold: {} opens/sec\n", args.alert_threshold);

    let bpf = Bpf::load_file(&args.bpf)
        .context("Failed to load eBPF program")?;

    let program: &mut TracePoint = bpf
        .program_mut("sys_enter_execve")
        .context("Failed to get execve program")?
        .try_into()?;
    program.load()?;
    program.attach("syscalls", "sys_enter_execve")
        .context("Failed to attach execve")?;
    println!("{} execve tracepoint", "✓".green());

    let program: &mut TracePoint = bpf
        .program_mut("sys_enter_openat")
        .context("Failed to get openat program")?
        .try_into()?;
    program.load()?;
    program.attach("syscalls", "sys_enter_openat")
        .context("Failed to attach openat")?;
    println!("{} openat tracepoint", "✓".green());

    let perf_map: PerfEventArray<ProcessEvent> = bpf
        .map_mut("EVENTS")
        .context("Failed to get EVENTS map")?
        .try_into()?;

    let tracker = Arc::new(Mutex::new(HashMap::<u32, FileOpenTracker>::new()));

    let mut reader = perf_map
        .reader(512)
        .context("Failed to create perf buffer reader")?;
    println!("{} Monitoring started. Press Ctrl+C to stop.\n", "▶".green());

    let tracker_clone = tracker.clone();
    let alert_threshold = args.alert_threshold;
    let json_output = args.json;

    tokio::spawn(async move {
        loop {
            match reader.read_events(&mut |events| {
                for event in events.iter() {
                    let ts = Local::now().format("%H:%M:%S%.3f");
                    match event.event_type {
                        EVENT_EXECVE => {
                            let comm = c_char_array_to_string(&event.comm);
                            if json_output {
                                println!(
                                    r#"{{"ts":"{}","type":"exec","pid":{},"uid":{},"comm":"{}"}}"#,
                                    ts, event.pid, event.uid, comm
                                );
                            } else {
                                println!(
                                    "{} {} [{}] {} {} {}",
                                    ts,
                                    "EXEC".bold().green(),
                                    event.pid,
                                    comm.bold(),
                                    "by uid".dimmed(),
                                    event.uid
                                );
                            }
                        }
                        EVENT_OPENAT => {
                            let comm = c_char_array_to_string(&event.comm);
                            let filename = c_char_array_to_string(&event.filename);
                            let truncated = if filename.len() > 60 {
                                format!("{}...", &filename[..57])
                            } else {
                                filename.clone()
                            };

                            let mut track = tracker_clone.blocking_lock();
                            let entry = track.entry(event.pid).or_insert(FileOpenTracker {
                                count: 0,
                                window_start: Instant::now(),
                            });

                            if entry.window_start.elapsed() > Duration::from_secs(1) {
                                entry.count = 0;
                                entry.window_start = Instant::now();
                            }
                            entry.count += 1;

                            if entry.count >= alert_threshold {
                                warn!(
                                    "{} SUSPICIOUS: Process {} ({}) opened {} files in 1s!",
                                    "⚠".yellow(),
                                    event.pid,
                                    comm,
                                    entry.count
                                );
                                entry.count = 0;
                                entry.window_start = Instant::now();
                            }

                            if json_output {
                                let escaped: String = filename
                                    .chars()
                                    .flat_map(|c| c.escape_default())
                                    .collect();
                                println!(
                                    r#"{{"ts":"{}","type":"open","pid":{},"uid":{},"comm":"{}","file":"{}"}}"#,
                                    ts, event.pid, event.uid, comm, escaped
                                );
                            } else if !filename.is_empty() {
                                println!(
                                    "{} {} [{}] {} \u{2192} {}",
                                    ts,
                                    "OPEN".bold().blue(),
                                    event.pid,
                                    comm.dimmed(),
                                    truncated.dimmed()
                                );
                            }
                        }
                        _ => {
                            if json_output {
                                println!(r#"{{"ts":"{}","type":"unknown","pid":{}}}"#, ts, event.pid);
                            }
                        }
                    }
                }
            }) {
                Ok(_) => {}
                Err(e) => {
                    error!("Perf buffer error: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    println!();
    info!("Shutting down...");
    Ok(())
}

fn c_char_array_to_string(arr: &[i8]) -> String {
    let bytes: Vec<u8> = arr
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).to_string()
}
