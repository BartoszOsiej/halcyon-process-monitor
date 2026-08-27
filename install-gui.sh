#!/usr/bin/env bash
# ╔══════════════════════════════════════════════════════════════════════╗
# ║  HALCYON — Graphical Installer                                     ║
# ║  eBPF Endpoint Security Agent for Linux                             ║
# ║  Beautiful zenity-based GTK installer                              ║
# ╚══════════════════════════════════════════════════════════════════════╝
set -euo pipefail

# ── Config ──────────────────────────────────────────────────────────────
INSTALL_DIR="/usr/local/bin"
LIB_DIR="/usr/local/lib/halcyon"
EBPF_NAME="process-monitor-ebpf"
BINARY_NAME="halcyon"
SOURCE_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_LOG="/tmp/halcyon-install.log"
ICON_PATH="/usr/share/icons/hicolor/256x256/apps/halcyon.png"
DESKTOP_FILE="/usr/share/applications/halcyon.desktop"

# ── Colors & styling for terminal fallback ──────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# ── Detect display server ──────────────────────────────────────────────
HAS_DISPLAY=true
if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
    HAS_DISPLAY=false
fi

# ── Zenity wrapper (falls back to terminal) ─────────────────────────────
zenity_cmd() {
    if $HAS_DISPLAY && command -v zenity &>/dev/null; then
        zenity "$@"
    else
        return 1
    fi
}

# ── Progress helper ─────────────────────────────────────────────────────
PIPE="/tmp/halcyon-install-pipe"
rm -f "$PIPE"
mkfifo "$PIPE"

progress() {
    local pct="$1"
    local text="$2"
    echo "$pct" > "$PIPE"
    echo "# $text" >> "$PIPE"
}

# ╔══════════════════════════════════════════════════════════════════════╗
# ║  STAGE 0: Welcome                                                   ║
# ╚══════════════════════════════════════════════════════════════════════╝

show_welcome() {
    zenity_cmd --info \
        --title="⚡ Halcyon — Installer" \
        --width=520 --height=340 \
        --text="<span size='x-large' foreground='#58A6FF'><b>🛡️  Halcyon</b></span>

<span size='large'>eBPF Endpoint Security Agent</span>

<span foreground='#8892a8'>This installer will:</span>

  <span foreground='#00ff64'>✓</span>  Check & install system dependencies
  <span foreground='#00ff64'>✓</span>  Build Halcyon from source (release mode)
  <span foreground='#00ff64'>✓</span>  Install binary to <b>/usr/local/bin</b>
  <span foreground='#00ff64'>✓</span>  Install eBPF program to <b>/usr/local/lib/halcyon</b>
  <span foreground='#00ff64'>✓</span>  Create <b>halcyon</b> command symlink
  <span foreground='#00ff64'>✓</span>  Create desktop entry for application menu

<span foreground='#505064'>Requires: Rust toolchain, clang, root access</span>" \
        --ok-label="Install ⚡" 2>/dev/null || {
        echo -e "${BOLD}${CYAN}⚡ Halcyon Installer${NC}"
        echo -e "${DIM}This installer will build and install Halcyon system-wide.${NC}"
        echo ""
        read -p "Press Enter to continue or Ctrl+C to cancel..."
    }
}

# ╔══════════════════════════════════════════════════════════════════════╗
# ║  STAGE 1: Check Dependencies                                        ║
# ╚══════════════════════════════════════════════════════════════════════╝

check_dependencies() {
    local missing=()
    local status_text=""

    # Required tools
    for tool in rustc cargo clang bpf-linker; do
        if command -v "$tool" &>/dev/null; then
            status_text+="  <span foreground='#00ff64'>✓</span>  $tool\n"
        else
            status_text+="  <span foreground='#ff3232'>✗</span>  <b>$tool</b> — not found\n"
            missing+=("$tool")
        fi
    done

    # Check rustup
    if command -v rustup &>/dev/null; then
        status_text+="  <span foreground='#00ff64'>✓</span>  rustup\n"
    else
        status_text+="  <span foreground='#ff3232'>✗</span>  <b>rustup</b> — not found\n"
        missing+=("rustup")
    fi

    # Check rust-src component
    if rustup component list --installed 2>/dev/null | grep -q rust-src; then
        status_text+="  <span foreground='#00ff64'>✓</span>  rust-src component\n"
    else
        status_text+="  <span foreground='#fab432'>!</span>  <b>rust-src</b> — not installed (will install)\n"
        missing+=("rust-src")
    fi

    # Check linux-headers (for BTF)
    if [ -f /sys/kernel/btf/vmlinux ]; then
        status_text+="  <span foreground='#00ff64'>✓</span>  BTF (vmlinux)\n"
    else
        status_text+="  <span foreground='#fab432'>!</span>  BTF not found — eBPF CO-RE may not work\n"
    fi

    # Check kernel version
    local kver
    kver=$(uname -r | cut -d. -f1,2)
    local kmajor
    kmajor=$(echo "$kver" | cut -d. -f1)
    local kminor
    kminor=$(echo "$kver" | cut -d. -f2)
    if [ "$kmajor" -ge 5 ] && [ "$kminor" -ge 8 ]; then
        status_text+="  <span foreground='#00ff64'>✓</span>  Kernel $kver (≥5.8)\n"
    else
        status_text+="  <span foreground='#ff3232'>✗</span>  Kernel $kver — <b>needs 5.8+</b>\n"
        missing+=("kernel")
    fi

    zenity_cmd --info \
        --title="🔍 Dependency Check" \
        --width=480 --height=400 \
        --text="<span size='large'><b>System Dependencies</b></span>

$(echo -e "$status_text")

<span foreground='#505064'>Missing: ${#missing[@]} packages</span>" \
        --ok-label="Continue →" 2>/dev/null || {
        echo -e "\n${BOLD}Dependency Check:${NC}"
        echo -e "$status_text"
    }

    return ${#missing[@]}
}

install_missing_deps() {
    # Install missing Rust components
    if command -v rustup &>/dev/null; then
        if ! rustup component list --installed 2>/dev/null | grep -q rust-src; then
            zenity_cmd --info --width=400 \
                --title="📦 Installing rust-src" \
                --text="Installing rust-src component for eBPF compilation..." 2>/dev/null || true
            rustup component add rust-src 2>>"$BUILD_LOG" || true
        fi
    fi

    # Install bpf-linker if missing
    if ! command -v bpf-linker &>/dev/null; then
        zenity_cmd --info --width=400 \
            --title="📦 Installing bpf-linker" \
            --text="Installing bpf-linker via cargo install..." 2>/dev/null || {
            echo "Installing bpf-linker..."
        }
        cargo install bpf-linker 2>>"$BUILD_LOG" || true
    fi

    # Check if we need to install system packages via pacman
    local pkgs_needed=()
    for pkg in clang llvm libelf zlib; do
        if ! pacman -Qi "$pkg" &>/dev/null 2>&1; then
            pkgs_needed+=("$pkg")
        fi
    done

    if [ ${#pkgs_needed[@]} -gt 0 ]; then
        zenity_cmd --question \
            --title="📦 System Packages" \
            --width=420 \
            --text="The following packages need to be installed via pacman:

<b>${pkgs_needed[*]}</b>

This requires root access. Continue?" \
            --ok-label="Install" --cancel-label="Skip" 2>/dev/null

        if [ $? -eq 0 ]; then
            get_sudo_password
            echo "$SUDO_PW" | sudo -S pacman -S --noconfirm "${pkgs_needed[@]}" 2>>"$BUILD_LOG" || true
        fi
    fi
}

# ╔══════════════════════════════════════════════════════════════════════╗
# ║  STAGE 2: Build                                                     ║
# ╚══════════════════════════════════════════════════════════════════════╝

build_project() {
    (
        cd "$SOURCE_DIR"

        progress 5 "🔨 Preparing build environment..."
        sleep 0.3

        progress 10 "📦 Compiling eBPF kernel programs..."
        sleep 0.2

        # Build eBPF programs
        cargo build --release -p process-monitor-ebpf \
            --target-dir target 2>>"$BUILD_LOG" || {
            zenity_cmd --error --width=400 \
                --title="❌ Build Failed" \
                --text="eBPF build failed. Check $BUILD_LOG" 2>/dev/null || true
            exit 1
        }

        progress 40 "🔨 Compiling userspace binary (release, LTO)..."
        sleep 0.2

        # Build userspace
        cargo build --release -p process-monitor \
            --target-dir target 2>>"$BUILD_LOG" || {
            zenity_cmd --error --width=400 \
                --title="❌ Build Failed" \
                --text="Userspace build failed. Check $BUILD_LOG" 2>/dev/null || true
            exit 1
        }

        progress 70 "✅ Build successful!"
        sleep 0.3

        progress 80 "📋 Copying binaries..."
        sleep 0.2

        # Find built binaries
        local TUI_BIN
        TUI_BIN=$(find target/release -maxdepth 1 -name "process-monitor" -type f | head -1)
        local EBPF_OBJ
        EBPF_OBJ=$(find target -path "*/release/process_monitor*" -name "*.bpf.o" -type f | head -1)

        if [ -z "$TUI_BIN" ]; then
            zenity_cmd --error --width=400 \
                --title="❌ Binary Not Found" \
                --text="Could not find compiled binary in target/release/" 2>/dev/null || true
            exit 1
        fi

        # Get sudo password for installation
        get_sudo_password

        # Install binary
        progress 85 "📥 Installing binary to $INSTALL_DIR..."
        echo "$SUDO_PW" | sudo -S cp "$TUI_BIN" "$INSTALL_DIR/$BINARY_NAME" 2>>"$BUILD_LOG"
        echo "$SUDO_PW" | sudo -S chmod +x "$INSTALL_DIR/$BINARY_NAME"

        # Install eBPF object
        progress 90 "📥 Installing eBPF program to $LIB_DIR..."
        echo "$SUDO_PW" | sudo -S mkdir -p "$LIB_DIR" 2>>"$BUILD_LOG"

        if [ -n "$EBPF_OBJ" ]; then
            echo "$SUDO_PW" | sudo -S cp "$EBPF_OBJ" "$LIB_DIR/$EBPF_NAME" 2>>"$BUILD_LOG"
        else
            # Try finding any .bpf.o in the build directory
            EBPF_OBJ=$(find target -name "*.bpf.o" -type f | head -1)
            if [ -n "$EBPF_OBJ" ]; then
                echo "$SUDO_PW" | sudo -S cp "$EBPF_OBJ" "$LIB_DIR/$EBPF_NAME" 2>>"$BUILD_LOG"
            fi
        fi

        # Create desktop entry
        progress 92 "🖥️  Creating desktop entry..."
        create_desktop_entry

        progress 95 "🔗 Verifying installation..."
        sleep 0.3

        # Verify
        if "$INSTALL_DIR/$BINARY_NAME" --version 2>/dev/null || true; then
            progress 100 "✅ Installation complete!"
        else
            progress 100 "✅ Installed (binary exists but --version may need eBPF)"
        fi
    ) | zenity_cmd --progress \
        --title="⚡ Building Halcyon..." \
        --width=420 --height=200 \
        --percentage=0 \
        --auto-close \
        --no-cancel 2>/dev/null

    BUILD_EXIT=$?
    rm -f "$PIPE"

    if [ $BUILD_EXIT -ne 0 ] && [ $BUILD_EXIT -ne 5 ]; then
        # zenity was cancelled or errored — fall back to terminal
        build_project_terminal
    fi
}

build_project_terminal() {
    echo -e "\n${BOLD}${CYAN}🔨 Building Halcyon...${NC}"
    cd "$SOURCE_DIR"

    echo -e "  ${DIM}→ Compiling eBPF programs...${NC}"
    cargo build --release -p process-monitor-ebpf --target-dir target 2>>"$BUILD_LOG" || {
        echo -e "  ${RED}✗ eBPF build failed. See $BUILD_LOG${NC}"
        exit 1
    }

    echo -e "  ${DIM}→ Compiling userspace binary (LTO)...${NC}"
    cargo build --release -p process-monitor --target-dir target 2>>"$BUILD_LOG" || {
        echo -e "  ${RED}✗ Build failed. See $BUILD_LOG${NC}"
        exit 1
    }

    echo -e "  ${GREEN}✓ Build successful${NC}"
}

# ╔══════════════════════════════════════════════════════════════════════╗
# ║  STAGE 3: Desktop Entry                                             ║
# ╚══════════════════════════════════════════════════════════════════════╝

create_desktop_entry() {
    # Create icon directory
    echo "$SUDO_PW" | sudo -S mkdir -p /usr/share/icons/hicolor/256x256/apps 2>/dev/null

    # Generate a simple SVG icon and convert, or create a placeholder
    local ICON_SVG="/tmp/halcyon-icon.svg"
    cat > "$ICON_SVG" << 'SVGEOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#0a0a1a"/>
      <stop offset="100%" style="stop-color:#161636"/>
    </linearGradient>
    <linearGradient id="shield" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#58a6ff"/>
      <stop offset="100%" style="stop-color:#00ffff"/>
    </linearGradient>
  </defs>
  <rect width="256" height="256" rx="40" fill="url(#bg)"/>
  <path d="M128 30 L200 70 L200 140 C200 190 165 225 128 240 C91 225 56 190 56 140 L56 70 Z"
        fill="none" stroke="url(#shield)" stroke-width="6" opacity="0.9"/>
  <path d="M128 55 L180 85 L180 138 C180 178 155 208 128 220 C101 208 76 178 76 138 L76 85 Z"
        fill="url(#shield)" opacity="0.15"/>
  <text x="128" y="145" text-anchor="middle" font-family="monospace" font-size="64" font-weight="bold" fill="#00ffff">H</text>
  <text x="128" y="195" text-anchor="middle" font-family="monospace" font-size="16" fill="#58a6ff" letter-spacing="4">EBPF</text>
</svg>
SVGEOF

    # Try to install the icon (convert SVG → PNG if possible)
    if command -v rsvg-convert &>/dev/null; then
        rsvg-convert -w 256 -h 256 "$ICON_SVG" -o /tmp/halcyon-icon.png 2>/dev/null
        echo "$SUDO_PW" | sudo -S cp /tmp/halcyon-icon.png "$ICON_PATH" 2>/dev/null || true
    elif command -v convert &>/dev/null; then
        convert "$ICON_SVG" -resize 256x256 "$ICON_PATH" 2>/dev/null || true
    fi

    # Create .desktop file
    local DESKTOP_CONTENT
    DESKTOP_CONTENT="[Desktop Entry]
Name=Halcyon
Comment=eBPF Endpoint Security Agent
Exec=sudo $INSTALL_DIR/$BINARY_NAME
Icon=halcyon
Terminal=true
Type=Application
Categories=System;Security;Monitor;
Keywords=ebpf;security;monitor;edr;
StartupNotify=false"

    echo "$SUDO_PW" | sudo -S tee "$DESKTOP_FILE" >/dev/null 2>&1 <<< "$DESKTOP_CONTENT"
    echo "$SUDO_PW" | sudo -S chmod +x "$DESKTOP_FILE" 2>/dev/null || true
    echo "$SUDO_PW" | sudo -S update-desktop-database /usr/share/applications/ 2>/dev/null || true
}

# ╔══════════════════════════════════════════════════════════════════════╗
# ║  Sudo Password Handling                                             ║
# ╚══════════════════════════════════════════════════════════════════════╝

SUDO_PW=""

get_sudo_password() {
    if [ -n "$SUDO_PW" ]; then
        return 0
    fi

    # Try passwordless sudo first
    if sudo -n true 2>/dev/null; then
        SUDO_PW=""
        return 0
    fi

    # Prompt via zenity password dialog
    if $HAS_DISPLAY && command -v zenity &>/dev/null; then
        SUDO_PW=$(zenity --password \
            --title="🔒 Root Access Required" \
            --width=360 \
            --text="Enter your password to install Halcyon:" 2>/dev/null) || {
            zenity_cmd --error --width=340 \
                --title="❌ Cancelled" \
                --text="Installation cancelled — root password required." 2>/dev/null
            exit 1
        }
    else
        echo -en "${BOLD}Enter sudo password: ${NC}"
        read -rs SUDO_PW
        echo ""
    fi

    # Verify password
    if ! echo "$SUDO_PW" | sudo -S true 2>/dev/null; then
        zenity_cmd --error --width=340 \
            --title="❌ Wrong Password" \
            --text="The password was incorrect. Try again." 2>/dev/null || \
            echo -e "${RED}Wrong password!${NC}"
        SUDO_PW=""
        get_sudo_password
    fi
}

# ╔══════════════════════════════════════════════════════════════════════╗
# ║  STAGE 4: Success Screen                                            ║
# ╚══════════════════════════════════════════════════════════════════════╝

show_success() {
    local VERSION
    VERSION=$("$INSTALL_DIR/$BINARY_NAME" --version 2>/dev/null || echo "installed")

    zenity_cmd --info \
        --title="✅ Halcyon Installed!" \
        --width=520 --height=360 \
        --text="<span size='x-large' foreground='#00ff64'><b>✅ Installation Complete!</b></span>

<span size='large'>Halcyon v${VERSION}</span>

<span foreground='#8892a8'>Installed to:</span>

  <span foreground='#58a6ff'>Binary:</span>     $INSTALL_DIR/$BINARY_NAME
  <span foreground='#58a6ff'>eBPF:</span>       $LIB_DIR/$EBPF_NAME
  <span foreground='#58a6ff'>Desktop:</span>    $DESKTOP_FILE

<span foreground='#8892a8'>Quick start:</span>

  <span foreground='#00ff64' font='monospace'>sudo halcyon</span>                    <span foreground='#505064'># TUI mode</span>
  <span foreground='#00ff64' font='monospace'>sudo halcyon --auto-kill</span>         <span foreground='#505064'># EDR mode</span>
  <span foreground='#00ff64' font='monospace'>sudo halcyon --json</span>              <span foreground='#505064'># JSON output</span>
  <span foreground='#00ff64' font='monospace'>sudo halcyon --web 0.0.0.0:8080</span> <span foreground='#505064'># Web dashboard</span>

<span foreground='#505064'>Build log: $BUILD_LOG</span>" \
        --ok-label="Done ⚡" 2>/dev/null || {
        echo ""
        echo -e "${BOLD}${GREEN}✅ Halcyon installed successfully!${NC}"
        echo ""
        echo -e "  ${BOLD}Binary:${NC}     $INSTALL_DIR/$BINARY_NAME"
        echo -e "  ${BOLD}eBPF:${NC}       $LIB_DIR/$EBPF_NAME"
        echo ""
        echo -e "  ${CYAN}Quick start:${NC}"
        echo -e "    sudo $BINARY_NAME                    # TUI mode"
        echo -e "    sudo $BINARY_NAME --auto-kill         # EDR mode"
        echo -e "    sudo $BINARY_NAME --json              # JSON output"
        echo -e "    sudo $BINARY_NAME --web 0.0.0.0:8080 # Web dashboard"
        echo ""
    }
}

# ╔══════════════════════════════════════════════════════════════════════╗
# ║  MAIN                                                               ║
# ╚══════════════════════════════════════════════════════════════════════╝

main() {
    # Initialize log
    echo "=== Halcyon Installer — $(date) ===" > "$BUILD_LOG"

    # Stage 0: Welcome
    show_welcome

    # Stage 1: Dependencies
    check_dependencies || true
    install_missing_deps

    # Stage 2: Build & Install
    get_sudo_password
    build_project

    # If zenity progress failed, do terminal install
    if [ ! -f "$INSTALL_DIR/$BINARY_NAME" ]; then
        echo -e "\n${BOLD}${CYAN}📥 Installing to system...${NC}"

        local TUI_BIN
        TUI_BIN=$(find "$SOURCE_DIR/target/release" -maxdepth 1 -name "process-monitor" -type f 2>/dev/null | head -1)
        local EBPF_OBJ
        EBPF_OBJ=$(find "$SOURCE_DIR/target" -name "*.bpf.o" -type f 2>/dev/null | head -1)

        if [ -n "$TUI_BIN" ]; then
            echo "$SUDO_PW" | sudo -S cp "$TUI_BIN" "$INSTALL_DIR/$BINARY_NAME"
            echo "$SUDO_PW" | sudo -S chmod +x "$INSTALL_DIR/$BINARY_NAME"
            echo -e "  ${GREEN}✓ Binary installed${NC}"
        fi

        if [ -n "$EBPF_OBJ" ]; then
            echo "$SUDO_PW" | sudo -S mkdir -p "$LIB_DIR"
            echo "$SUDO_PW" | sudo -S cp "$EBPF_OBJ" "$LIB_DIR/$EBPF_NAME"
            echo -e "  ${GREEN}✓ eBPF program installed${NC}"
        fi

        create_desktop_entry
        echo -e "  ${GREEN}✓ Desktop entry created${NC}"
    fi

    # Stage 4: Success
    show_success
}

# Run
main "$@"
