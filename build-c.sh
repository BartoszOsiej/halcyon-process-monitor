#!/usr/bin/env bash
# build-c.sh — Build script for Talus Process Monitor (C + Go version)
#
# Components:
#   1. eBPF kernel programs → process_monitor.bpf.o
#   2. Core monitor library → libtalus_monitor (static)
#   3. TUI frontend → talus (ncurses)
#   4. Web dashboard → talus-web (Go)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ── Colors ────────────────────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[build]${NC} $*"; }
ok()    { echo -e "${GREEN}[build]${NC} $*"; }
warn()  { echo -e "${YELLOW}[build]${NC} $*"; }
fail()  { echo -e "${RED}[build]${NC} $*"; exit 1; }

# ── Check dependencies ───────────────────────────────────────────────────

check_dep() {
    if ! command -v "$1" &>/dev/null; then
        fail "Required tool not found: $1 ($2)"
    fi
}

info "Checking dependencies..."
check_dep gcc       "C compiler (apt install gcc)"
check_dep clang     "eBPF compiler (apt install clang)"

# bpftool optional — needed only for auto-generating vmlinux.h
HAS_BPFTOOL=false
if command -v bpftool &>/dev/null; then
    HAS_BPFTOOL=true
else
    warn "bpftool not found — skipping auto vmlinux.h generation"
    warn "If eBPF build fails, install: apt install linux-tools-common"
fi

check_dep pkg-config "pkg-config (apt install pkg-config)"

# Check for libbpf
if ! pkg-config --exists libbpf 2>/dev/null; then
    warn "libbpf not found via pkg-config, trying direct linking..."
fi

# Check for ncurses
if ! pkg-config --exists ncurses 2>/dev/null; then
    warn "ncurses not found via pkg-config, trying -lncurses"
fi

# Check for Go (for web dashboard)
HAS_GO=false
if command -v go &>/dev/null; then
    HAS_GO=true
fi

# ── Build eBPF programs ──────────────────────────────────────────────────

info "Building eBPF kernel programs..."

mkdir -p c-ebpf

clang -O2 -g -Wall \
    -target bpf \
    -D__TARGET_ARCH_x86 \
    -c c-ebpf/process_monitor.bpf.c \
    -o c-ebpf/process_monitor.bpf.o

ok "eBPF object: c-ebpf/process_monitor.bpf.o"

# ── Build monitor library ────────────────────────────────────────────────

info "Building monitor library..."

mkdir -p build/lib

gcc -Wall -Wextra -O2 -std=c11 -D_GNU_SOURCE -fPIC \
    -Ic-monitor/include \
    -c c-monitor/src/monitor.c \
    -o build/lib/monitor.o

# Create static library
ar rcs build/lib/libtalus_monitor.a build/lib/monitor.o

# Create shared library for Go CGO linking
gcc -shared -o build/lib/libtalus_monitor.so build/lib/monitor.o \
    -lpthread -lelf -lz -lbpf -lm

ok "Monitor library: build/lib/libtalus_monitor.a"

# ── Build TUI ────────────────────────────────────────────────────────────

info "Building TUI (ncurses)..."

gcc -Wall -Wextra -O2 -std=c11 -D_GNU_SOURCE \
    -Ic-monitor/include \
    -c c-tui/src/tui.c \
    -o build/lib/tui.o

gcc -o talus build/lib/tui.o build/lib/monitor.o \
    -lpthread -lncurses -lelf -lz -lbpf -lm

ok "TUI binary: talus"

# ── Build web dashboard (Go) ─────────────────────────────────────────────

if [ "$HAS_GO" = true ] && [ -d "go-web" ]; then
    info "Building Go web dashboard..."
    (cd go-web && CGO_CFLAGS="-I$(pwd)/../c-monitor/include" CGO_LDFLAGS="-L$(pwd)/../build/lib -ltalus_monitor -lpthread -lelf -lz -lbpf -lm" go build -o ../talus-web .)
    ok "Web dashboard: talus-web"
else
    if [ "$HAS_GO" = false ]; then
        warn "Go not installed, skipping web dashboard"
    fi
fi

# ── Done ─────────────────────────────────────────────────────────────────

echo ""
ok "Build complete!"
echo ""
echo "  talus          — TUI monitor (ncurses)"
if [ -f talus-web ]; then
    echo "  talus-web      — Web dashboard (Go)"
fi
echo "  c-ebpf/*.bpf.o   — eBPF kernel program"
echo ""
echo "Run with:"
echo "  sudo ./talus [--bpf c-ebpf/process_monitor.bpf.o]"
if [ -f talus-web ]; then
    echo "  sudo ./talus-web [--bpf c-ebpf/process_monitor.bpf.o]"
fi
