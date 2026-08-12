#!/usr/bin/env bash
# Halcyon Process Monitor - build script.
#
# Builds the eBPF program (requires Rust nightly + rust-src) and the userspace
# TUI binary (works on stable Rust). Does NOT require root. Use install.sh for
# a full setup (toolchain, dependencies, install).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/target}"

die() {
    echo "ERROR: $*" >&2
    exit 1
}
info() { echo "==> $*"; }

command -v cargo >/dev/null 2>&1 || die "Rust/Cargo not found. Install it from https://rustup.rs"

# The eBPF crate needs a nightly toolchain with rust-src to build core for
# the bpfel-unknown-none target via -Z build-std.
if ! cargo +nightly --version >/dev/null 2>&1; then
    die "Nightly Rust toolchain not found.
    Install it with:
      rustup toolchain install nightly --profile minimal --component rust-src"
fi

if ! command -v bpf-linker >/dev/null 2>&1; then
    die "bpf-linker not found (used to link eBPF programs).
    Install it with:
      cargo install bpf-linker"
fi

echo "=== Halcyon Process Monitor - build ==="
echo "Target dir: $CARGO_TARGET_DIR"
echo ""

info "[1/2] Building eBPF program (bpfel-unknown-none)..."
cargo +nightly build --release \
    --target bpfel-unknown-none \
    -Z build-std=core \
    --manifest-path "$SCRIPT_DIR/process-monitor-ebpf/Cargo.toml"

info "[2/2] Building userspace binary..."
cargo build --release -p process-monitor --manifest-path "$SCRIPT_DIR/Cargo.toml"

echo ""
echo "=== Build complete ==="
echo ""
echo "  Binary: $CARGO_TARGET_DIR/release/process-monitor"
echo "  eBPF:   $CARGO_TARGET_DIR/bpfel-unknown-none/release/process-monitor-ebpf"
echo ""
echo "Run (requires root for eBPF):"
echo "  sudo $CARGO_TARGET_DIR/release/process-monitor"
echo ""
echo "Other modes:"
echo "  --json         newline-delimited JSON output"
echo "  --plain        plain text log output"
echo "  --threshold N  alert after N file opens in 1s (default: 50)"
