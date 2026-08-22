#!/usr/bin/env bash
# Halcyon Process Monitor - build script.
#
# Builds the eBPF program (requires Rust nightly + rust-src) and the userspace
# binary. By default builds TUI-only variant. Use --web for the web-featured build.
#
# Usage:
#   ./build.sh          # Build TUI-only (default, 1.7MB)
#   ./build.sh --web    # Build with web server (2.5MB)
#   ./build.sh --all    # Build both variants

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/target}"

BUILD_TUI=true
BUILD_WEB=false

for arg in "$@"; do
    case "$arg" in
        --web) BUILD_WEB=true; BUILD_TUI=false ;;
        --all) BUILD_WEB=true; BUILD_TUI=true ;;
        --help|-h)
            echo "Usage: ./build.sh [--web|--all]"
            echo ""
            echo "  (default)  Build TUI-only variant"
            echo "  --web      Build with web server (requires axum, tokio)"
            echo "  --all      Build both TUI and web variants"
            exit 0
            ;;
        *) echo "Unknown option: $arg" >&2; exit 1 ;;
    esac
done

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

info "[1/3] Building eBPF program (bpfel-unknown-none)..."
cargo +nightly build --release \
    --target bpfel-unknown-none \
    -Z build-std=core \
    --manifest-path "$SCRIPT_DIR/process-monitor-ebpf/Cargo.toml"

if [ "$BUILD_TUI" = true ]; then
    info "[2/3] Building userspace binary (TUI-only)..."
    cargo build --release -p process-monitor --manifest-path "$SCRIPT_DIR/Cargo.toml"
    cp "$CARGO_TARGET_DIR/release/process-monitor" "$CARGO_TARGET_DIR/release/process-monitor-tui"
    echo "  TUI binary: $CARGO_TARGET_DIR/release/process-monitor-tui (1.7MB)"
fi

if [ "$BUILD_WEB" = true ]; then
    info "[3/3] Building userspace binary (with web server)..."
    cargo build --release -p process-monitor --features web --manifest-path "$SCRIPT_DIR/Cargo.toml"
    cp "$CARGO_TARGET_DIR/release/process-monitor" "$CARGO_TARGET_DIR/release/process-monitor-web"
    echo "  Web binary: $CARGO_TARGET_DIR/release/process-monitor-web (2.5MB)"
fi

# Default binary is always the TUI variant
if [ "$BUILD_TUI" = true ]; then
    cp "$CARGO_TARGET_DIR/release/process-monitor-tui" "$CARGO_TARGET_DIR/release/process-monitor"
fi

echo ""
echo "=== Build complete ==="
echo ""
echo "  eBPF:   $CARGO_TARGET_DIR/bpfel-unknown-none/release/process-monitor-ebpf"
if [ "$BUILD_TUI" = true ]; then
    echo "  TUI:    $CARGO_TARGET_DIR/release/process-monitor-tui"
fi
if [ "$BUILD_WEB" = true ]; then
    echo "  Web:    $CARGO_TARGET_DIR/release/process-monitor-web"
fi
echo ""
echo "Run TUI (requires root for eBPF):"
echo "  sudo $CARGO_TARGET_DIR/release/process-monitor-tui"
echo ""
echo "Run web dashboard (requires --features web):"
echo "  sudo $CARGO_TARGET_DIR/release/process-monitor-web --web 0.0.0.0:8080"
echo ""
echo "Other modes:"
echo "  --json         newline-delimited JSON output"
echo "  --plain        plain text log output"
echo "  --threshold N  alert after N file opens in 1s (default: 50)"
echo "  --web ADDR     start web server (requires web build)"
