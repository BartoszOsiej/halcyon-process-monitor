#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ftui_pty::{PtyConfig, spawn_command};
use portable_pty::CommandBuilder;

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn unique_dir(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ftui-pty-canonicalize-{}-{}-{}",
        label,
        std::process::id(),
        id
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn capture_pty_bytes(script: &str, cols: u16, rows: u16) -> Vec<u8> {
    let config = PtyConfig::default()
        .with_size(cols, rows)
        .with_test_name("pty_canonicalize_fixture")
        .logging(false);

    let mut cmd = CommandBuilder::new("sh");
    cmd.args(["-c", script]);

    let mut session = spawn_command(config, cmd).expect("spawn PTY command");
    session
        .wait_and_drain(Duration::from_secs(2))
        .expect("wait and drain PTY output");
    session.output().to_vec()
}

fn run_canonicalize(
    input: &std::path::Path,
    output: &std::path::Path,
    cols: u16,
    rows: u16,
    extra_args: &[&str],
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pty_canonicalize"));
    cmd.args([
        "--input",
        input.to_string_lossy().as_ref(),
        "--output",
        output.to_string_lossy().as_ref(),
        "--cols",
        &cols.to_string(),
        "--rows",
        &rows.to_string(),
    ]);
    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }
    cmd.output().expect("run pty_canonicalize")
}

#[test]
fn canonicalize_basic_cursor_moves() {
    let bytes = capture_pty_bytes("printf 'HELLO'; printf '\\033[2;1HROW2'", 10, 2);
    assert!(!bytes.is_empty(), "expected PTY fixture bytes");

    let dir = unique_dir("basic");
    let input = dir.join("basic_input.pty");
    let output = dir.join("basic_output.txt");
    fs::write(&input, &bytes).expect("write PTY input fixture");

    let run = run_canonicalize(&input, &output, 10, 2, &[]);
    assert!(
        run.status.success(),
        "pty_canonicalize failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let text = fs::read_to_string(&output).expect("read canonical output");
    assert_eq!(text, "HELLO\nROW2");
}

#[test]
fn canonicalize_screen_profile_immediate_wrap() {
    let bytes = capture_pty_bytes("printf 'ABCDE\\rZ'", 5, 2);
    assert!(!bytes.is_empty(), "expected PTY fixture bytes");

    let dir = unique_dir("screen_profile");
    let input = dir.join("wrap_input.pty");
    let output_default = dir.join("wrap_default.txt");
    let output_screen = dir.join("wrap_screen.txt");
    fs::write(&input, &bytes).expect("write PTY input fixture");

    let run_default = run_canonicalize(&input, &output_default, 5, 2, &[]);
    assert!(
        run_default.status.success(),
        "pty_canonicalize default failed: {}",
        String::from_utf8_lossy(&run_default.stderr)
    );
    let text_default = fs::read_to_string(&output_default).expect("read default output");
    assert_eq!(text_default, "ZBCDE\n");

    let run_screen = run_canonicalize(&input, &output_screen, 5, 2, &["--profile", "screen"]);
    assert!(
        run_screen.status.success(),
        "pty_canonicalize screen profile failed: {}",
        String::from_utf8_lossy(&run_screen.stderr)
    );
    let text_screen = fs::read_to_string(&output_screen).expect("read screen profile output");
    assert_eq!(text_screen, "ABCDE\nZ");
}
