//! Halcyon Process Monitor — FrankenTUI-based TUI layer (v2 premium)
//!
//! Elm/Bubbletea architecture: Model → update → view → BufferDiff → ANSI.
//! Visual design: dark cyberpunk terminal with gradient bars, heat colors,
//! Unicode tree connectors, protocol icons, and micro-charts.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_layout::{Constraint, Flex};
use ftui_render::cell::PackedRgba;
use ftui_render::frame::Frame;
use ftui_runtime::program::{App, Cmd, Model};
use ftui_runtime::terminal_writer::ScreenMode;
use ftui_runtime::{Every, Subscription};
use ftui_style::{Style, StyleFlags};
use ftui_text::Line;
use ftui_text::Span;
use ftui_widgets::block::Block;
use ftui_widgets::borders::{BorderType, Borders};
use ftui_widgets::log_viewer::{LogViewer, LogViewerState};
use ftui_widgets::paragraph::Paragraph;
use ftui_widgets::sparkline::Sparkline;
use ftui_widgets::status_line::{StatusItem, StatusLine};
use ftui_widgets::{StatefulWidget, Widget};

use crate::monitor::{FileRank, Monitor, Output, ProcStats, RateSample};

// ── Premium dark palette ──────────────────────────────────────────────────

// Primary accents
const BLUE: PackedRgba = PackedRgba::rgb(88, 166, 255);
const BLUE_DIM: PackedRgba = PackedRgba::rgb(56, 100, 180);
const BLUE_GLOW: PackedRgba = PackedRgba::rgb(120, 190, 255);
const GREEN: PackedRgba = PackedRgba::rgb(63, 185, 80);
const GREEN_DIM: PackedRgba = PackedRgba::rgb(35, 120, 50);
const RED: PackedRgba = PackedRgba::rgb(248, 81, 73);
const RED_DIM: PackedRgba = PackedRgba::rgb(160, 50, 45);
const AMBER: PackedRgba = PackedRgba::rgb(210, 153, 34);
const AMBER_DIM: PackedRgba = PackedRgba::rgb(140, 100, 20);
const PURPLE: PackedRgba = PackedRgba::rgb(188, 140, 255);
const PURPLE_DIM: PackedRgba = PackedRgba::rgb(120, 80, 170);
const CYAN: PackedRgba = PackedRgba::rgb(56, 189, 248);
const CYAN_DIM: PackedRgba = PackedRgba::rgb(30, 120, 160);

// Neutrals
const FG: PackedRgba = PackedRgba::rgb(180, 190, 210);
const FG_DIM: PackedRgba = PackedRgba::rgb(80, 90, 110);
const FG_BRIGHT: PackedRgba = PackedRgba::rgb(220, 230, 245);
const FG_MUTED: PackedRgba = PackedRgba::rgb(55, 62, 78);
const BG_HEADER: PackedRgba = PackedRgba::rgb(15, 18, 28);
const BG_PANEL: PackedRgba = PackedRgba::rgb(12, 14, 22);

// ── Unicode building blocks ───────────────────────────────────────────────

const BAR_FULL: &str = "█";
const BAR_THREE_Q: &str = "▓";
const BAR_HALF: &str = "▒";
const BAR_QUARTER: &str = "░";
const TREE_BRANCH: &str = "├── ";
const TREE_LAST: &str = "└── ";
const TREE_PIPE: &str = "│   ";
const TREE_SPACE: &str = "    ";
const ARROW_OUT: &str = "→";
const ARROW_IN: &str = "←";
const ARROW_BI: &str = "⇄";
const DIAMOND: &str = "◆";
const DOT: &str = "●";
const CIRCLE: &str = "○";
const SEPARATOR: &str = "│";

// ── Panel IDs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Events = 0,
    Processes = 1,
    Network = 2,
    TopFiles = 3,
    Extensions = 4,
    Alerts = 5,
}

impl Panel {
    fn all() -> &'static [Panel] {
        &[
            Panel::Events,
            Panel::Processes,
            Panel::Network,
            Panel::TopFiles,
            Panel::Extensions,
            Panel::Alerts,
        ]
    }

    fn name(&self) -> &'static str {
        match self {
            Panel::Events => "EVENTS",
            Panel::Processes => "PROCESSES",
            Panel::Network => "NETWORK",
            Panel::TopFiles => "TOP FILES",
            Panel::Extensions => "FILE TYPES",
            Panel::Alerts => "ALERTS",
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            Panel::Events => "⚡",
            Panel::Processes => "⚙",
            Panel::Network => "⇄",
            Panel::TopFiles => "📁",
            Panel::Extensions => "🧩",
            Panel::Alerts => "⚠",
        }
    }

    fn count() -> usize {
        6
    }
}

// ── Elm-style Message type ────────────────────────────────────────────────

#[derive(Debug)]
enum Msg {
    Key(KeyEvent),
    Tick,
    MonitorTick(MonitorSnapshot),
    Noop,
}

#[derive(Debug)]
#[allow(dead_code)]
struct MonitorSnapshot {
    outputs: Vec<MonitorOutput>,
    stats: Vec<ProcStats>,
    top_files: Vec<FileRank>,
    ext_counts: std::collections::HashMap<String, u64>,
    rate_history: VecDeque<RateSample>,
    total_events: u64,
    total_lost: u64,
    uptime_secs: u64,
}

#[derive(Debug)]
enum MonitorOutput {
    Event {
        ts: String,
        kind_name: String,
        pid: u32,
        comm: String,
        file: Option<String>,
    },
    Alert {
        ts: String,
        pid: u32,
        comm: String,
        opens: u64,
    },
}

impl From<Event> for Msg {
    fn from(e: Event) -> Self {
        match e {
            Event::Key(k) => Msg::Key(k),
            _ => Msg::Noop,
        }
    }
}

// ── Network entry ─────────────────────────────────────────────────────────

#[derive(Clone)]
struct NetworkEntry {
    ts: String,
    pid: u32,
    comm: String,
    kind: String,
    addr: String,
}

// ── Application state ─────────────────────────────────────────────────────

struct HalcyonApp {
    log: LogViewer,
    log_state: LogViewerState,
    alerts: LogViewer,
    alerts_state: LogViewerState,
    network: VecDeque<NetworkEntry>,
    process_rows: Vec<(usize, String, u32, u64, u64)>,
    top_files: Vec<TopFileEntry>,
    ext_entries: Vec<(String, u64)>,
    rate_history: VecDeque<RateSample>,
    focused: usize,
    paused: bool,
    help_visible: bool,
    events_per_sec: f64,
    opens_per_sec: f64,
    total_events: u64,
    total_alerts: u64,
    total_lost: u64,
    uptime_secs: u64,
    rx: mpsc::Receiver<MonitorSnapshot>,
}

#[derive(Clone)]
#[allow(dead_code)]
struct TopFileEntry {
    path: String,
    count: u64,
    extension: String,
    entropy: f64,
}

// ── Visual helpers ────────────────────────────────────────────────────────

/// Build a proportional bar using Unicode block elements.
/// Returns a string like "██████░░░░" based on ratio 0.0–1.0.
fn proportional_bar(ratio: f64, width: usize) -> String {
    let filled_f = ratio * width as f64;
    let full_blocks = filled_f.floor() as usize;
    let remainder = filled_f - filled_f.floor();

    let mut s = String::with_capacity(width);
    for _ in 0..full_blocks.min(width) {
        s.push_str(BAR_FULL);
    }
    if full_blocks < width {
        if remainder > 0.75 {
            s.push_str(BAR_THREE_Q);
        } else if remainder > 0.5 {
            s.push_str(BAR_HALF);
        } else if remainder > 0.25 {
            s.push_str(BAR_QUARTER);
        }
    }
    // Pad to exact width
    let visual_len = s.chars().count();
    if visual_len < width {
        for _ in 0..(width - visual_len) {
            s.push(' ');
        }
    }
    s
}

/// Heat color for entropy value (0.0–1.0).
fn entropy_color(e: f64) -> PackedRgba {
    if e > 0.75 {
        RED
    } else if e > 0.55 {
        AMBER
    } else if e > 0.35 {
        CYAN
    } else {
        GREEN
    }
}

/// Category color for file extension.
fn ext_color(ext: &str) -> PackedRgba {
    match ext {
        // Encrypted / suspicious
        "enc" | "locked" | "crypto" | "encrypted" => RED,
        // Source code
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "go" | "java" | "rb" => GREEN,
        // Documents
        "pdf" | "doc" | "docx" | "txt" | "md" | "odt" => AMBER,
        // Images / media
        "jpg" | "jpeg" | "png" | "gif" | "mp4" | "mp3" | "wav" | "webm" => PURPLE,
        // Config / data
        "json" | "toml" | "yaml" | "yml" | "xml" | "csv" => CYAN,
        // Binaries
        "so" | "dll" | "exe" | "bin" | "elf" => AMBER_DIM,
        _ => BLUE,
    }
}

/// Format seconds into human-readable uptime.
fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Network protocol icon.
fn net_icon(kind: &str) -> (&str, PackedRgba) {
    match kind {
        "Connect" => (ARROW_OUT, BLUE),
        "Accept" => (ARROW_IN, GREEN),
        "SendTo" => (ARROW_OUT, AMBER),
        "RecvFrom" => (ARROW_IN, PURPLE),
        _ => ("·", FG_DIM),
    }
}

/// Truncate a filename to max width, keeping extension visible.
fn truncate_filename(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        format!("{:>width$}", name, width = max_len)
    } else {
        let keep = max_len.saturating_sub(2); // space for "…"
        format!("{}…{}", &name[..keep.min(name.len())], &name[name.len().saturating_sub(2)..])
    }
}

// ── HalcyonApp impl ───────────────────────────────────────────────────────

impl HalcyonApp {
    fn new(rx: mpsc::Receiver<MonitorSnapshot>) -> Self {
        let mut log = LogViewer::new(5000);
        log.push("  ⬡ Halcyon eBPF Monitor initialized");
        log.push("  ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄");
        log.push("  FrankenTUI · diff-based renderer · zero flicker");
        log.push("  Kernel tracepoints attached · perf buffers open");
        log.push("  Press ? for keyboard shortcuts");

        Self {
            log,
            log_state: LogViewerState::default(),
            alerts: LogViewer::new(200),
            alerts_state: LogViewerState::default(),
            network: VecDeque::new(),
            process_rows: Vec::new(),
            top_files: Vec::new(),
            ext_entries: Vec::new(),
            rate_history: VecDeque::new(),
            focused: 0,
            paused: false,
            help_visible: false,
            events_per_sec: 0.0,
            opens_per_sec: 0.0,
            total_events: 0,
            total_alerts: 0,
            total_lost: 0,
            uptime_secs: 0,
            rx,
        }
    }

    fn process_monitor_outputs(&mut self, outputs: Vec<MonitorOutput>) {
        for out in outputs {
            match out {
                MonitorOutput::Event { ts, kind_name, pid, comm, file } => {
                    let file_str = file.as_deref().unwrap_or("");
                    let kind_icon = match kind_name.as_str() {
                        "Exec" => "▶",
                        "Open" => "◉",
                        "Connect" | "Accept" | "SendTo" | "RecvFrom" => "⇄",
                        "Mkdir" => "⊕",
                        "Unlink" => "⊖",
                        "Kill" => "☠",
                        "Chmod" => "⚿",
                        _ => "·",
                    };
                    let line = format!("{} {} {:>8} [{}] {}", ts, kind_icon, kind_name, pid, comm);
                    let line = if !file_str.is_empty() {
                        format!("{} {}", line, file_str)
                    } else {
                        line
                    };
                    self.log.push(line.as_str());

                    let is_net = matches!(kind_name.as_str(), "Connect" | "Accept" | "SendTo" | "RecvFrom");
                    if is_net {
                        self.network.push_front(NetworkEntry {
                            ts, pid, comm, kind: kind_name, addr: file_str.to_string(),
                        });
                        while self.network.len() > 200 {
                            self.network.pop_back();
                        }
                    }
                    self.total_events += 1;
                }
                MonitorOutput::Alert { ts, pid, comm, opens } => {
                    let line = format!("🚨 {} [{}] {} → {} opens/s", ts, pid, comm, opens);
                    self.alerts.push(line.as_str());
                    self.total_alerts += 1;
                }
            }
        }
    }

    fn update_process_tree(&mut self, stats: &[ProcStats]) {
        self.process_rows.clear();
        let mut sorted: Vec<&ProcStats> = stats.iter().collect();
        sorted.sort_by_key(|s| std::cmp::Reverse(s.window_opens));
        for s in sorted.iter().take(50) {
            self.process_rows.push((0, s.comm.clone(), s.pid, s.alerts, s.window_opens));
        }
    }

    fn update_top_files(&mut self, files: &[FileRank]) {
        self.top_files = files.iter().map(|f| TopFileEntry {
            path: f.path.clone(),
            count: f.count,
            extension: f.extension.clone(),
            entropy: f.entropy,
        }).collect();
    }

    fn update_extensions(&mut self, exts: &std::collections::HashMap<String, u64>) {
        let mut entries: Vec<(String, u64)> = exts.iter().map(|(k, v)| (k.clone(), *v)).collect();
        entries.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        self.ext_entries = entries;
    }

    fn update_rates(&mut self, history: &VecDeque<RateSample>) {
        self.rate_history = history.clone();
    }
}

impl Model for HalcyonApp {
    type Message = Msg;

    fn init(&mut self) -> Cmd<Self::Message> {
        Cmd::None
    }

    fn update(&mut self, msg: Msg) -> Cmd<Self::Message> {
        match msg {
            Msg::Key(k) if k.kind == KeyEventKind::Press => {
                if self.help_visible {
                    self.help_visible = false;
                    return Cmd::None;
                }
                if k.modifiers.contains(Modifiers::CTRL) && k.code == KeyCode::Char('c') {
                    return Cmd::Quit;
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Escape => {
                        if self.focused == 0 { return Cmd::Quit; }
                    }
                    KeyCode::Char('?') => self.help_visible = true,
                    KeyCode::Char('p') => self.paused = !self.paused,
                    KeyCode::Tab => self.focused = (self.focused + 1) % Panel::count(),
                    KeyCode::BackTab => {
                        self.focused = if self.focused == 0 { Panel::count() - 1 } else { self.focused - 1 };
                    }
                    KeyCode::Char('1') => self.focused = 0,
                    KeyCode::Char('2') => self.focused = 1,
                    KeyCode::Char('3') => self.focused = 2,
                    KeyCode::Char('4') => self.focused = 3,
                    KeyCode::Char('5') => self.focused = 4,
                    KeyCode::Char('6') => self.focused = 5,
                    KeyCode::Up | KeyCode::Char('k') => self.log.scroll_up(1),
                    KeyCode::Down | KeyCode::Char('j') => self.log.scroll_down(1),
                    KeyCode::PageUp => self.log.page_up(&self.log_state),
                    KeyCode::PageDown => self.log.page_down(&self.log_state),
                    KeyCode::Home => self.log.scroll_to_top(),
                    KeyCode::End => self.log.scroll_to_bottom(),
                    _ => {}
                }
            }
            Msg::MonitorTick(snapshot) if !self.paused => {
                self.process_monitor_outputs(snapshot.outputs);
                self.update_process_tree(&snapshot.stats);
                self.update_top_files(&snapshot.top_files);
                self.update_extensions(&snapshot.ext_counts);
                self.update_rates(&snapshot.rate_history);
                self.total_events = snapshot.total_events;
                self.total_lost = snapshot.total_lost;
                self.uptime_secs = snapshot.uptime_secs;
            }
            Msg::Tick if !self.paused => {
                while let Ok(snapshot) = self.rx.try_recv() {
                    self.process_monitor_outputs(snapshot.outputs);
                    self.update_process_tree(&snapshot.stats);
                    self.update_top_files(&snapshot.top_files);
                    self.update_extensions(&snapshot.ext_counts);
                    self.update_rates(&snapshot.rate_history);
                    self.total_events = snapshot.total_events;
                    self.total_lost = snapshot.total_lost;
                    self.uptime_secs = snapshot.uptime_secs;
                }
            }
            _ => {}
        }
        Cmd::None
    }

    fn view(&self, frame: &mut Frame) {
        let area = Rect::from_size(frame.buffer.width(), frame.buffer.height());

        if self.help_visible {
            self.render_help(frame, area);
            return;
        }

        let chunks = Flex::vertical()
            .constraints([
                Constraint::Fixed(3),  // Header (with gradient bar)
                Constraint::Fixed(1),  // Tab bar
                Constraint::Min(0),   // Body
                Constraint::Fixed(2), // Status + sparkline
            ])
            .split(area);

        self.render_header(frame, chunks[0]);
        self.render_tab_bar(frame, chunks[1]);
        self.render_body(frame, chunks[2]);
        self.render_status(frame, chunks[3]);
    }

    fn subscriptions(&self) -> Vec<Box<dyn Subscription<Self::Message>>> {
        vec![Box::new(Every::new(Duration::from_millis(50), || Msg::Tick))]
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────

impl HalcyonApp {
    // ── Header ────────────────────────────────────────────────────────────
    fn render_header(&self, frame: &mut Frame, area: Rect) {
        // Row 1: gradient accent bar
        let bar_area = Rect::new(area.x, area.y, area.width, 1);
        let mut bar_spans = Vec::new();
        let bar_width = area.width as usize;
        // Gradient: blue → purple → red (left to right)
        for i in 0..bar_width {
            let ratio = i as f64 / bar_width as f64;
            let r = (88.0 + ratio * 160.0) as u8;
            let g = (166.0 - ratio * 85.0) as u8;
            let b = (255.0 - ratio * 182.0) as u8;
            bar_spans.push(Span::styled(BAR_FULL, Style::new().fg(PackedRgba::rgb(r, g, b))));
        }
        let bar_line = Line::from_spans(bar_spans);
        Paragraph::new(bar_line).render(bar_area, frame);

        // Row 2: title + stats
        let title_area = Rect::new(area.x, area.y + 1, area.width, 1);
        let uptime = format_uptime(self.uptime_secs);
        let lost_str = if self.total_lost > 0 {
            format!(" ⚠{} lost", self.total_lost)
        } else {
            String::new()
        };
        let header_spans = vec![
            Span::styled("  ⬡ ", Style::new().fg(BLUE_GLOW).attrs(StyleFlags::BOLD)),
            Span::styled("HALCYON", Style::new().fg(BLUE_GLOW).attrs(StyleFlags::BOLD)),
            Span::styled("  ", Style::new().fg(FG_DIM)),
            Span::styled(SEPARATOR, Style::new().fg(FG_MUTED)),
            Span::styled(" eBPF PROCESS MONITOR ", Style::new().fg(FG_DIM)),
            Span::styled(SEPARATOR, Style::new().fg(FG_MUTED)),
            Span::styled(format!(" ⚡ {} events", self.total_events), Style::new().fg(FG)),
            Span::styled(SEPARATOR, Style::new().fg(FG_MUTED)),
            Span::styled(format!(" 🕐 {}", uptime), Style::new().fg(FG)),
        ];
        let header_line = Line::from_spans(header_spans);
        Paragraph::new(header_line).render(title_area, frame);

        // Row 3: subtitle
        let sub_area = Rect::new(area.x, area.y + 2, area.width, 1);
        let sub_spans = vec![
            Span::styled("    ", Style::new().fg(FG_DIM)),
            Span::styled(" kernel tracepoints active ", Style::new().fg(GREEN_DIM)),
            Span::styled("  ", Style::new().fg(FG_DIM)),
            Span::styled(" perf buffers streaming ", Style::new().fg(CYAN_DIM)),
            Span::styled("  ", Style::new().fg(FG_DIM)),
            Span::styled(&lost_str, Style::new().fg(RED)),
        ];
        let sub_line = Line::from_spans(sub_spans);
        Paragraph::new(sub_line).render(sub_area, frame);
    }

    // ── Tab bar ───────────────────────────────────────────────────────────
    fn render_tab_bar(&self, frame: &mut Frame, area: Rect) {
        let mut spans = Vec::new();
        for (i, p) in Panel::all().iter().enumerate() {
            let active = i == self.focused;
            let num = format!("{}", i + 1);

            if active {
                // Active tab: bright with underline effect
                spans.push(Span::styled(
                    format!(" {}{}{} ", num, DIAMOND, p.name()),
                    Style::new().fg(BLUE_GLOW).attrs(StyleFlags::BOLD),
                ));
            } else {
                // Inactive tab: dim with circle
                spans.push(Span::styled(
                    format!(" {}{}{} ", num, CIRCLE, p.name()),
                    Style::new().fg(FG_DIM),
                ));
            }
            spans.push(Span::styled("  ", Style::new().fg(FG_MUTED)));
        }

        // Pause indicator
        if self.paused {
            spans.push(Span::styled(
                "  ⏸ PAUSED ",
                Style::new().fg(AMBER).attrs(StyleFlags::BOLD),
            ));
        }

        let tab_line = Line::from_spans(spans);
        Paragraph::new(tab_line).render(area, frame);

        // Underline active tab
        // (We can't easily do per-tab underlines without knowing exact positions,
        //  so we skip this for now)
    }

    // ── Body layout ───────────────────────────────────────────────────────
    fn render_body(&self, frame: &mut Frame, area: Rect) {
        let body_chunks = Flex::horizontal()
            .constraints([
                Constraint::Percentage(35.0),
                Constraint::Percentage(35.0),
                Constraint::Percentage(30.0),
            ])
            .split(area);

        let middle_chunks = Flex::vertical()
            .constraints([Constraint::Percentage(55.0), Constraint::Percentage(45.0)])
            .split(body_chunks[1]);

        let right_chunks = Flex::vertical()
            .constraints([
                Constraint::Percentage(35.0),
                Constraint::Percentage(35.0),
                Constraint::Percentage(30.0),
            ])
            .split(body_chunks[2]);

        self.render_events_panel(frame, body_chunks[0]);
        self.render_process_panel(frame, middle_chunks[0]);
        self.render_extensions_panel(frame, middle_chunks[1]);
        self.render_network_panel(frame, right_chunks[0]);
        self.render_top_files_panel(frame, right_chunks[1]);
        self.render_alerts_panel(frame, right_chunks[2]);
    }

    // ── Events panel ──────────────────────────────────────────────────────
    fn render_events_panel(&self, frame: &mut Frame, area: Rect) {
        let focused = self.focused == 0;
        let border_color = if focused { BLUE } else { FG_MUTED };
        let title_color = if focused { BLUE_GLOW } else { FG_DIM };

        let block = Block::new()
            .title(" ⚡ EVENTS ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border_color));

        let inner = block.inner(area);
        block.render(area, frame);

        let mut state = self.log_state.clone();
        self.log.render(inner, frame, &mut state);
    }

    // ── Process panel ─────────────────────────────────────────────────────
    fn render_process_panel(&self, frame: &mut Frame, area: Rect) {
        let focused = self.focused == 1;
        let border_color = if focused { BLUE } else { FG_MUTED };
        let title_color = if focused { BLUE_GLOW } else { FG_DIM };

        let title_str = format!(" ⚙ PROCESSES ({}) ", self.process_rows.len());
        let block = Block::new()
            .title(&title_str)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border_color));

        let inner = block.inner(area);
        block.render(area, frame);

        if self.process_rows.is_empty() {
            let waiting = Paragraph::new(Line::from_spans(vec![
                Span::styled("    ", Style::new().fg(FG_DIM)),
                Span::styled(CIRCLE, Style::new().fg(FG_DIM)),
                Span::styled("  Waiting for events...", Style::new().fg(FG_DIM)),
            ]));
            waiting.render(inner, frame);
            return;
        }

        let max_rows = inner.height as usize;
        let max_opens = self.process_rows.iter()
            .map(|(_, _, _, _, opens)| *opens)
            .max().unwrap_or(1).max(1);
        let bar_area_width = (inner.width as f64 * 0.35) as usize;

        let lines: Vec<Line> = self.process_rows.iter().take(max_rows).map(|(_, comm, pid, alerts, opens)| {
            let ratio = *opens as f64 / max_opens as f64;
            let bar = proportional_bar(ratio, bar_area_width);

            let (status_icon, status_color) = if *alerts > 0 {
                (format!(" {}⚠", DIAMOND), RED)
            } else if *opens > 0 {
                (" ▸".to_string(), GREEN)
            } else {
                (" ·".to_string(), FG_DIM)
            };

            let comm_color = if *alerts > 0 { RED } else if *opens > 10 { AMBER } else { FG_BRIGHT };

            Line::from_spans(vec![
                Span::styled(status_icon, Style::new().fg(status_color)),
                Span::styled(format!(" {:>5} ", pid), Style::new().fg(FG_DIM)),
                Span::styled(format!("{:<16}", truncate_filename(comm, 16)), Style::new().fg(comm_color)),
                Span::styled(format!(" {:>4} ", opens), Style::new().fg(CYAN)),
                Span::styled(bar, Style::new().fg(if *alerts > 0 { RED } else { BLUE_DIM })),
                Span::styled(format!(" {:>3}", alerts), Style::new().fg(status_color)),
            ])
        }).collect();

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(lines));
        paragraph.render(inner, frame);
    }

    // ── Extensions panel ──────────────────────────────────────────────────
    fn render_extensions_panel(&self, frame: &mut Frame, area: Rect) {
        let focused = self.focused == 4;
        let border_color = if focused { BLUE } else { FG_MUTED };
        let title_color = if focused { BLUE_GLOW } else { FG_DIM };

        let block = Block::new()
            .title(" 🧩 FILE TYPES ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border_color));

        let inner = block.inner(area);
        block.render(area, frame);

        if self.ext_entries.is_empty() {
            let waiting = Paragraph::new(Line::from_spans(vec![
                Span::styled("    ", Style::new().fg(FG_DIM)),
                Span::styled(CIRCLE, Style::new().fg(FG_DIM)),
                Span::styled("  Waiting for events...", Style::new().fg(FG_DIM)),
            ]));
            waiting.render(inner, frame);
            return;
        }

        let max_count = self.ext_entries.iter().map(|(_, c)| *c).max().unwrap_or(1);
        let max_rows = inner.height as usize;
        let bar_width = (inner.width as f64 * 0.40) as usize;

        let lines: Vec<Line> = self.ext_entries.iter().take(max_rows).map(|(ext, count)| {
            let ratio = *count as f64 / max_count as f64;
            let bar = proportional_bar(ratio, bar_width);
            let color = ext_color(ext);

            Line::from_spans(vec![
                Span::styled(format!("  .{:<10}", ext), Style::new().fg(color)),
                Span::styled(format!(" {:>6} ", count), Style::new().fg(FG_BRIGHT)),
                Span::styled(bar, Style::new().fg(color)),
            ])
        }).collect();

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(lines));
        paragraph.render(inner, frame);
    }

    // ── Network panel ─────────────────────────────────────────────────────
    fn render_network_panel(&self, frame: &mut Frame, area: Rect) {
        let focused = self.focused == 2;
        let border_color = if focused { BLUE } else { FG_MUTED };
        let title_color = if focused { BLUE_GLOW } else { FG_DIM };

        let title_str = format!(" ⇄ NETWORK ({}) ", self.network.len());
        let block = Block::new()
            .title(&title_str)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border_color));

        let inner = block.inner(area);
        block.render(area, frame);

        if self.network.is_empty() {
            let waiting = Paragraph::new(Line::from_spans(vec![
                Span::styled("    ", Style::new().fg(FG_DIM)),
                Span::styled(CIRCLE, Style::new().fg(FG_DIM)),
                Span::styled("  Waiting for network events...", Style::new().fg(FG_DIM)),
            ]));
            waiting.render(inner, frame);
            return;
        }

        let max_rows = inner.height as usize;
        let lines: Vec<Line> = self.network.iter().rev().take(max_rows).map(|entry| {
            let (icon, kind_color) = net_icon(&entry.kind);

            Line::from_spans(vec![
                Span::styled(format!("{} ", icon), Style::new().fg(kind_color).attrs(StyleFlags::BOLD)),
                Span::styled(format!("{:>8} ", entry.kind), Style::new().fg(kind_color)),
                Span::styled(format!("{:>6} ", entry.pid), Style::new().fg(FG_DIM)),
                Span::styled(format!("{:<12}", truncate_filename(&entry.comm, 12)), Style::new().fg(FG)),
                Span::styled(format!(" → {}", entry.addr), Style::new().fg(PURPLE)),
            ])
        }).collect();

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(lines));
        paragraph.render(inner, frame);
    }

    // ── Top files panel ───────────────────────────────────────────────────
    fn render_top_files_panel(&self, frame: &mut Frame, area: Rect) {
        let focused = self.focused == 3;
        let border_color = if focused { BLUE } else { FG_MUTED };
        let title_color = if focused { BLUE_GLOW } else { FG_DIM };

        let block = Block::new()
            .title(" 📁 TOP FILES ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border_color));

        let inner = block.inner(area);
        block.render(area, frame);

        if self.top_files.is_empty() {
            let waiting = Paragraph::new(Line::from_spans(vec![
                Span::styled("    ", Style::new().fg(FG_DIM)),
                Span::styled(CIRCLE, Style::new().fg(FG_DIM)),
                Span::styled("  Waiting for events...", Style::new().fg(FG_DIM)),
            ]));
            waiting.render(inner, frame);
            return;
        }

        let max_count = self.top_files.iter().map(|f| f.count).max().unwrap_or(1);
        let max_rows = inner.height as usize;
        let bar_width = (inner.width as f64 * 0.30) as usize;

        let lines: Vec<Line> = self.top_files.iter().take(max_rows).enumerate().map(|(rank, entry)| {
            let ratio = entry.count as f64 / max_count as f64;
            let bar = proportional_bar(ratio, bar_width);

            let short_name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
            let truncated = truncate_filename(short_name, 18);

            let entropy_c = entropy_color(entry.entropy);
            let ext_c = ext_color(&entry.extension);

            // Rank indicator
            let rank_str = format!("{:>2}.", rank + 1);
            let rank_color = if rank < 3 { AMBER } else { FG_DIM };

            Line::from_spans(vec![
                Span::styled(rank_str, Style::new().fg(rank_color)),
                Span::styled(format!(" {}", truncated), Style::new().fg(FG_BRIGHT)),
                Span::styled(format!(" {:>5}", entry.count), Style::new().fg(CYAN)),
                Span::styled(bar, Style::new().fg(ext_c)),
                Span::styled(format!(" .{}", entry.extension), Style::new().fg(ext_c)),
                Span::styled(format!(" H:{:.1}", entry.entropy), Style::new().fg(entropy_c)),
            ])
        }).collect();

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(lines));
        paragraph.render(inner, frame);
    }

    // ── Alerts panel ──────────────────────────────────────────────────────
    fn render_alerts_panel(&self, frame: &mut Frame, area: Rect) {
        let alert_count = self.alerts.len();
        let focused = self.focused == 5;
        let (border_color, title_color) = if alert_count > 0 {
            (RED, RED)
        } else if focused {
            (BLUE, BLUE_GLOW)
        } else {
            (FG_MUTED, FG_DIM)
        };

        let title_str = format!(" ⚠ ALERTS ({}) ", alert_count);
        let block = Block::new()
            .title(&title_str)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border_color));

        let inner = block.inner(area);
        block.render(area, frame);

        if alert_count == 0 {
            let clear = Paragraph::new(Line::from_spans(vec![
                Span::styled("    ", Style::new().fg(FG_DIM)),
                Span::styled("✓", Style::new().fg(GREEN).attrs(StyleFlags::BOLD)),
                Span::styled("  all clear — no ransomware activity", Style::new().fg(FG)),
            ]));
            clear.render(inner, frame);
        } else {
            let mut state = self.alerts_state.clone();
            self.alerts.render(inner, frame, &mut state);
        }
    }

    // ── Status bar ────────────────────────────────────────────────────────
    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let chunks = Flex::vertical()
            .constraints([Constraint::Fixed(1), Constraint::Fixed(1)])
            .split(area);

        // Row 1: Status line
        let panel = Panel::all()[self.focused];
        let panel_label = format!(" {} {} ", panel.icon(), panel.name());
        let stats = format!(
            " ⚡{:.0}/s  ◉{:.0}/s  ⚠{}  ◆{} lost  🕐{}",
            self.events_per_sec, self.opens_per_sec,
            self.total_alerts, self.total_lost,
            format_uptime(self.uptime_secs),
        );

        let status = StatusLine::new()
            .left(StatusItem::text(&panel_label))
            .center(StatusItem::text(&stats))
            .right(StatusItem::key_hint("?", "Help"));
        status.render(chunks[0], frame);

        // Row 2: Sparkline
        if self.rate_history.len() > 1 {
            let spark_width = chunks[1].width as usize;
            let recent: Vec<f64> = self.rate_history.iter()
                .rev().take(spark_width).rev()
                .map(|r| r.exec_count as f64)
                .collect();

            let sparkline = Sparkline::new(&recent)
                .style(Style::new().fg(BLUE));

            let spark_area = Rect::new(chunks[1].x, chunks[1].y, chunks[1].width, 1);
            sparkline.render(spark_area, frame);
        }
    }

    // ── Help overlay ──────────────────────────────────────────────────────
    fn render_help(&self, frame: &mut Frame, area: Rect) {
        let help_lines = vec![
            Line::from_spans(vec![
                Span::styled("    ⬡ HALCYON", Style::new().fg(BLUE_GLOW).attrs(StyleFlags::BOLD)),
                Span::styled("  ──────────────────────────────────────────────", Style::new().fg(FG_MUTED)),
            ]),
            Line::raw(""),
            Line::from_spans(vec![
                Span::styled("    ", Style::new().fg(FG_DIM)),
                Span::styled("NAVIGATION", Style::new().fg(AMBER).attrs(StyleFlags::BOLD)),
            ]),
            Line::from_spans(vec![
                Span::styled("      Tab / Shift+Tab    ", Style::new().fg(FG)),
                Span::styled("cycle panels", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      1-6               ", Style::new().fg(FG)),
                Span::styled("jump to panel", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      ↑/↓ or j/k        ", Style::new().fg(FG)),
                Span::styled("scroll", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      PgUp/PgDn         ", Style::new().fg(FG)),
                Span::styled("page scroll", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      Home / End        ", Style::new().fg(FG)),
                Span::styled("jump to top/bottom", Style::new().fg(FG_DIM)),
            ]),
            Line::raw(""),
            Line::from_spans(vec![
                Span::styled("    ", Style::new().fg(FG_DIM)),
                Span::styled("ACTIONS", Style::new().fg(AMBER).attrs(StyleFlags::BOLD)),
            ]),
            Line::from_spans(vec![
                Span::styled("      p                 ", Style::new().fg(FG)),
                Span::styled("pause / resume", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      q / Esc           ", Style::new().fg(FG)),
                Span::styled("quit", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      Ctrl+C            ", Style::new().fg(FG)),
                Span::styled("force quit", Style::new().fg(FG_DIM)),
            ]),
            Line::raw(""),
            Line::from_spans(vec![
                Span::styled("    ", Style::new().fg(FG_DIM)),
                Span::styled("PANELS", Style::new().fg(AMBER).attrs(StyleFlags::BOLD)),
            ]),
            Line::from_spans(vec![
                Span::styled("      ⚡ EVENTS       ", Style::new().fg(BLUE)),
                Span::styled("live event log", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      ⚙ PROCESSES    ", Style::new().fg(BLUE)),
                Span::styled("process tree + open counts", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      ⇄ NETWORK      ", Style::new().fg(BLUE)),
                Span::styled("real-time connections", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      📁 TOP FILES    ", Style::new().fg(BLUE)),
                Span::styled("most-accessed + entropy", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      🧩 FILE TYPES   ", Style::new().fg(BLUE)),
                Span::styled("extension frequency", Style::new().fg(FG_DIM)),
            ]),
            Line::from_spans(vec![
                Span::styled("      ⚠ ALERTS       ", Style::new().fg(BLUE)),
                Span::styled("ransomware detection", Style::new().fg(FG_DIM)),
            ]),
            Line::raw(""),
            Line::from_spans(vec![
                Span::styled("    ──────────────────────────────────────────────", Style::new().fg(FG_MUTED)),
            ]),
            Line::from_spans(vec![
                Span::styled("    FrankenTUI", Style::new().fg(FG_DIM)),
                Span::styled(" · diff-based renderer · zero flicker", Style::new().fg(FG_MUTED)),
            ]),
        ];

        let block = Block::new()
            .title(" ⬡ HALCYON — Keyboard Shortcuts ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(BLUE));

        let inner = block.inner(area);
        block.render(area, frame);

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(help_lines));
        paragraph.render(inner, frame);
    }
}

// ── Public API ────────────────────────────────────────────────────────────

/// Run the FrankenTUI-based TUI.
pub fn run(mut monitor: Monitor) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<MonitorSnapshot>();

    let mut tick_count: u64 = 0;
    let monitor_thread = thread::Builder::new()
        .name("halcyon-tui-poll".into())
        .spawn(move || {
            loop {
                let poll_outputs: Vec<MonitorOutput> = monitor.poll()
                    .into_iter()
                    .filter_map(|o| match o {
                        Output::Event(ev) => Some(MonitorOutput::Event {
                            ts: ev.ts,
                            kind_name: format!("{:?}", ev.kind),
                            pid: ev.pid,
                            comm: ev.comm,
                            file: ev.file,
                        }),
                        Output::Alert(al) => Some(MonitorOutput::Alert {
                            ts: al.ts, pid: al.pid, comm: al.comm, opens: al.opens,
                        }),
                    })
                    .collect();

                tick_count += 1;
                let send_snapshot = tick_count % 6 == 0 || !poll_outputs.is_empty();

                if send_snapshot {
                    let snapshot = MonitorSnapshot {
                        outputs: poll_outputs,
                        stats: monitor.stats_sorted(),
                        top_files: monitor.top_files(20),
                        ext_counts: monitor.extension_counts().clone(),
                        rate_history: monitor.rate_history().clone(),
                        total_events: monitor.total_events,
                        total_lost: monitor.total_lost,
                        uptime_secs: monitor.uptime().as_secs(),
                    };
                    if tx.send(snapshot).is_err() {
                        break;
                    }
                }

                thread::sleep(Duration::from_millis(16));
            }
        })?;

    let app = HalcyonApp::new(rx);
    let result = App::new(app).screen_mode(ScreenMode::AltScreen).run();
    let _ = monitor_thread.join();

    result.map_err(|e| anyhow::anyhow!("TUI error: {}", e))
}
