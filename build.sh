#!/usr/bin/env bash
set -euo pipefail

echo "=== Halcyon Process Monitor - Build Script ==="
echo ""

# Prerequisites check
command -v cargo >/dev/null 2>&1 || { echo "Rust not installed. Install: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"; exit 1; }

if ! command -v cc >/dev/null 2>&1; then
  echo "C compiler not found. Installing build dependencies..."
  if command -v apt >/dev/null 2>&1; then
    sudo apt install -y build-essential
  elif command -v zypper >/dev/null 2>&1; then
    sudo zypper install -y gcc gcc-c++ kernel-devel
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y gcc gcc-c++ kernel-devel
  else
    echo "Package manager not recognized. Install C compiler manually."
    exit 1
  fi
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/target}"

echo "[1/2] Building eBPF program (bpfel-unknown-none)..."
cd "$SCRIPT_DIR/process-monitor-ebpf"
RUSTFLAGS="-C link-arg=-z -C link-arg=note-got" \
cargo +nightly build \
  --release \
  --target bpfel-unknown-none \
  -Z build-std=core 2>&1

echo "[2/2] Building userspace loader..."
cd "$SCRIPT_DIR/process-monitor"
cargo build --release 2>&1

echo ""
echo "=== Build complete ==="
echo ""
echo "Run:"
echo "  sudo $CARGO_TARGET_DIR/release/process-monitor \\"
echo "    --bpf $CARGO_TARGET_DIR/bpfel-unknown-none/release/process-monitor-ebpf \\"
echo "    --alert-threshold 50"
echo ""
echo "Options:"
echo "  --json                  JSON-formatted output"
echo "  --alert-threshold N     File opens/sec before alert (default: 50)"
echo ""
