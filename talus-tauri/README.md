# Talus — Tauri Desktop Dashboard

Native desktop GUI for the Talus Endpoint Security Agent.  
Built with **Tauri 2** (Rust backend) + **React 19** (TypeScript frontend) + **Recharts**.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  Tauri Desktop App                                  │
│  ┌─────────────────┐   ┌────────────────────────┐  │
│  │  Rust Backend    │   │  React Frontend        │  │
│  │  (Tauri 2)       │   │  (Vite + TypeScript)   │  │
│  │                  │   │                        │  │
│  │  - Spawns        │   │  - Events panel        │  │
│  │    talus       │◄──│  - Processes panel     │  │
│  │    --json        │   │  - Network panel       │  │
│  │  - Emits events  │   │  - Files panel         │  │
│  │    to frontend   │   │  - Extensions panel    │  │
│  │  - Tauri IPC     │   │  - Alerts panel        │  │
│  │    commands      │   │  - Rate chart (Recharts)│  │
│  └─────────────────┘   └────────────────────────┘  │
└──────────────────────┬──────────────────────────────┘
                       │
                       ▼
            ┌─────────────────────┐
            │  talus binary     │
            │  (eBPF kernel prog) │
            │  --json mode        │
            └─────────────────────┘
```

The Tauri backend spawns `sudo talus --json` as a child process, reads its stdout line by line, and emits each JSON event to the React frontend via Tauri's event system. The frontend also polls the REST API (`/api/v1/stats`, `/api/v1/processes`, etc.) for aggregate data.

## Prerequisites

### System dependencies (Linux)

```bash
# Arch Linux
sudo pacman -S webkit2gtk-4.1 javascriptcoregtk-4.1 libsoup3 gtk3

# Ubuntu/Debian
sudo apt install libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev

# Fedora
sudo dnf install webkit2gtk4.1-devel javascriptcoregtk4.1-devel \
  gtk3-devel libsoup3-devel libappindicator-gtk3-devel librsvg2-devel
```

### Toolchain

- Rust (stable, 1.77+)
- Node.js 22+
- talus binary installed at `/usr/local/bin/talus`

## Quick Start

```bash
cd talus-tauri

# Install JS deps
npm install

# Development (hot reload)
npm run tauri dev

# Production build
npm run tauri build
```

## Frontend Dev (without Tauri)

If talus web server is running on port 3080:

```bash
npm run dev
# Open http://localhost:5173
```

The React app connects to `ws://localhost:3080/ws` for real-time events and polls `http://localhost:3080/api/v1/*` for aggregate data.

## Rust Backend

The Tauri backend (`src-tauri/src/lib.rs`) provides 3 IPC commands:

| Command | Description |
|---------|-------------|
| `start_monitor` | Spawns `sudo talus --json` with optional flags (`--auto-kill`, `--kafka-brokers`, etc.) |
| `stop_monitor` | Sends SIGTERM to the talus process |
| `get_status` | Returns `{ running, pid }` |

Events flow: `talus stdout → Rust thread → Tauri emit → React state update`

## Dashboard Panels

| Panel | Content |
|-------|---------|
| **EVENTS** | Real-time event log (exec, open, connect, etc.) |
| **PROCESSES** | Top processes by file open rate with horizontal bars |
| **NETWORK** | Per-process connection aggregation (connect/accept/send/recv) |
| **TOP FILES** | Most-accessed files with entropy indicator |
| **FILE TYPES** | Extension distribution as horizontal bars |
| **ALERTS** | Triggered heuristic alerts |
| **EVENT RATE** | Live line chart of exec/open/alert rates (Recharts) |
