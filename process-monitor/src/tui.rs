//! Halcyon Process Monitor — FrankenTUI-based TUI layer
//!
//! Uses the Elm/Bubbletea architecture: Model → update → view → BufferDiff → ANSI.
//! FrankenTUI provides diff-based rendering (zero flicker), RAII cleanup,
//! and 80+ widgets including sparklines, trees, tables, and log viewers.

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

// ── Modern dark palette (2026) ────────────────────────────────────────────

const ACCENT_BLUE: PackedRgba = PackedRgba::rgb(88, 166, 255);
const ACCENT_GREEN: PackedRgba = PackedRgba::rgb(63, 185, 80);
const ACCENT_RED: PackedRgba = PackedRgba::rgb(248, 81, 73);
const ACCENT_AMBER: PackedRgba = PackedRgba::rgb(210, 153, 34);
const ACCENT_PURPLE: PackedRgba = PackedRgba::rgb(188, 140, 255);
const ACCENT_CYAN: PackedRgba = PackedRgba::rgb(56, 189, 248);
const TEXT_DIM: PackedRgba = PackedRgba::rgb(76, 82, 99);
const TEXT_BRIGHT: PackedRgba = PackedRgba::rgb(139, 148, 168);
const BORDER_SUBTLE: PackedRgba = PackedRgba::rgb(48, 55, 73);
const BORDER_ACTIVE: PackedRgba = PackedRgba::rgb(88, 166, 255);

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

/// Batch of outputs from a single poll() call, plus periodic stats snapshot.
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

// ── Network entry (for network panel) ────────────────────────────────────

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
    // Log viewers
    log: LogViewer,
    log_state: LogViewerState,
    alerts: LogViewer,
    alerts_state: LogViewerState,

    // Network panel data
    network: VecDeque<NetworkEntry>,

    // Process tree (flattened: depth, comm, pid, alerts, opens)
    process_rows: Vec<(usize, String, u32, u64, u64)>,

    // Top files
    top_files: Vec<TopFileEntry>,

    // Extension counts
    ext_entries: Vec<(String, u64)>,

    // Sparkline data
    rate_history: VecDeque<RateSample>,

    // Navigation
    focused: usize,
    paused: bool,
    help_visible: bool,

    // Stats for status bar
    events_per_sec: f64,
    opens_per_sec: f64,
    total_events: u64,
    total_alerts: u64,
    uptime_secs: u64,

    // Channel to receive monitor data
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

impl HalcyonApp {
    fn new(rx: mpsc::Receiver<MonitorSnapshot>) -> Self {
        let mut log = LogViewer::new(5000);
        log.push("⚡ Halcyon eBPF Monitor started");
        log.push("  FrankenTUI diff-based renderer · zero flicker");
        log.push("  Press ? for help");

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
            uptime_secs: 0,
            rx,
        }
    }

    /// Process incoming monitor outputs into panel data.
    fn process_monitor_outputs(&mut self, outputs: Vec<MonitorOutput>) {
        for out in outputs {
            match out {
                MonitorOutput::Event {
                    ts,
                    kind_name,
                    pid,
                    comm,
                    file,
                } => {
                    // Build log line
                    let file_str = file.as_deref().unwrap_or("");
                    let line = format!("{} {:>8} [{}] {} {}", ts, kind_name, pid, comm, file_str);
                    self.log.push(line.as_str());

                    // Network panel
                    let is_net = matches!(
                        kind_name.as_str(),
                        "Connect" | "Accept" | "SendTo" | "RecvFrom"
                    );
                    if is_net {
                        self.network.push_front(NetworkEntry {
                            ts,
                            pid,
                            comm,
                            kind: kind_name,
                            addr: file_str.to_string(),
                        });
                        // Keep last 200
                        while self.network.len() > 200 {
                            self.network.pop_back();
                        }
                    }

                    self.total_events += 1;
                }
                MonitorOutput::Alert {
                    ts,
                    pid,
                    comm,
                    opens,
                } => {
                    let line = format!(
                        "⚠ {} [{}] {} opened {} files in 1s!",
                        ts, pid, comm, opens
                    );
                    self.alerts.push(line.as_str());
                    self.total_alerts += 1;
                }
            }
        }
    }

    /// Update process tree from monitor data.
    fn update_process_tree(&mut self, stats: &[ProcStats]) {
        self.process_rows.clear();
        // Sort by window_opens descending, take top 50
        let mut sorted: Vec<&ProcStats> = stats.iter().collect();
        sorted.sort_by_key(|s| std::cmp::Reverse(s.window_opens));
        for s in sorted.iter().take(50) {
            self.process_rows.push((
                0, // flat display, no depth for now
                s.comm.clone(),
                s.pid,
                s.alerts,
                s.window_opens,
            ));
        }
    }

    /// Update top files panel.
    fn update_top_files(&mut self, files: &[FileRank]) {
        self.top_files = files
            .iter()
            .map(|f| TopFileEntry {
                path: f.path.clone(),
                count: f.count,
                extension: f.extension.clone(),
                entropy: f.entropy,
            })
            .collect();
    }

    /// Update extensions panel.
    fn update_extensions(&mut self, exts: &std::collections::HashMap<String, u64>) {
        let mut entries: Vec<(String, u64)> = exts.iter().map(|(k, v)| (k.clone(), *v)).collect();
        entries.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        self.ext_entries = entries;
    }

    /// Update sparkline rate data.
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
                        if self.focused == 0 {
                            return Cmd::Quit;
                        }
                    }
                    KeyCode::Char('?') => self.help_visible = true,
                    KeyCode::Char('p') => self.paused = !self.paused,
                    KeyCode::Tab => {
                        self.focused = (self.focused + 1) % Panel::count();
                    }
                    KeyCode::BackTab => {
                        self.focused = if self.focused == 0 {
                            Panel::count() - 1
                        } else {
                            self.focused - 1
                        };
                    }
                    KeyCode::Char('1') => self.focused = 0,
                    KeyCode::Char('2') => self.focused = 1,
                    KeyCode::Char('3') => self.focused = 2,
                    KeyCode::Char('4') => self.focused = 3,
                    KeyCode::Char('5') => self.focused = 4,
                    KeyCode::Char('6') => self.focused = 5,
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.log.scroll_up(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.log.scroll_down(1);
                    }
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
                self.uptime_secs = snapshot.uptime_secs;
            }
            Msg::Tick if !self.paused => {
                // Drain channel for new monitor snapshots
                while let Ok(snapshot) = self.rx.try_recv() {
                    self.process_monitor_outputs(snapshot.outputs);
                    self.update_process_tree(&snapshot.stats);
                    self.update_top_files(&snapshot.top_files);
                    self.update_extensions(&snapshot.ext_counts);
                    self.update_rates(&snapshot.rate_history);
                    self.total_events = snapshot.total_events;
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
                Constraint::Fixed(2),  // Header
                Constraint::Fixed(1),  // Tab bar
                Constraint::Min(0),   // Body
                Constraint::Fixed(2), // Status + sparklines
            ])
            .split(area);

        self.render_header(frame, chunks[0]);
        self.render_tab_bar(frame, chunks[1]);
        self.render_body(frame, chunks[2]);
        self.render_status(frame, chunks[3]);
    }

    fn subscriptions(&self) -> Vec<Box<dyn Subscription<Self::Message>>> {
        vec![Box::new(Every::new(Duration::from_millis(50), || {
            Msg::Tick
        }))]
    }
}

// ── Rendering methods ─────────────────────────────────────────────────────

impl HalcyonApp {
    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let header = Paragraph::new(Line::from_spans(vec![
            Span::styled(
                "  ⬡ HALCYON  ",
                Style::new().fg(ACCENT_BLUE).attrs(StyleFlags::BOLD),
            ),
            Span::styled(
                format!("eBPF · PROCESS MONITOR · {} events", self.total_events),
                Style::new().fg(TEXT_DIM),
            ),
        ]));
        header.render(area, frame);
    }

    fn render_tab_bar(&self, frame: &mut Frame, area: Rect) {
        let mut spans = Vec::new();
        for (i, p) in Panel::all().iter().enumerate() {
            let color = if i == self.focused {
                ACCENT_BLUE
            } else {
                TEXT_DIM
            };
            let dot = if i == self.focused { "●" } else { "○" };
            spans.push(Span::styled(
                format!(" {} {} ", dot, p.name()),
                Style::new().fg(color),
            ));
        }
        let tab_line = Line::from_spans(spans);
        let tab_widget = Paragraph::new(tab_line);
        tab_widget.render(area, frame);
    }

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

    fn render_events_panel(&self, frame: &mut Frame, area: Rect) {
        let border = if self.focused == 0 {
            BORDER_ACTIVE
        } else {
            BORDER_SUBTLE
        };

        let block = Block::new()
            .title(" EVENTS ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border));

        let inner = block.inner(area);
        block.render(area, frame);

        let mut state = self.log_state.clone();
        self.log.render(inner, frame, &mut state);
    }

    fn render_process_panel(&self, frame: &mut Frame, area: Rect) {
        let border = if self.focused == 1 {
            BORDER_ACTIVE
        } else {
            BORDER_SUBTLE
        };

        let block = Block::new()
            .title(" PROCESSES ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border));

        let inner = block.inner(area);
        block.render(area, frame);

        if self.process_rows.is_empty() {
            let text = Paragraph::new(Line::from_spans(vec![Span::styled(
                "  Waiting for events...",
                Style::new().fg(TEXT_DIM),
            )]));
            text.render(inner, frame);
            return;
        }

        // Render process list with mini-bars
        let max_rows = inner.height as usize;
        let max_opens = self
            .process_rows
            .iter()
            .map(|(_, _, _, _, opens)| *opens)
            .max()
            .unwrap_or(1)
            .max(1);

        let lines: Vec<Line> = self
            .process_rows
            .iter()
            .take(max_rows)
            .map(|(_depth, comm, pid, alerts, opens)| {
                let bar_width = ((inner.width as f64 - 30.0) * (*opens as f64 / max_opens as f64))
                    as usize;
                let bar: String = "█".repeat(bar_width.min(40));

                let alert_color = if *alerts > 0 {
                    ACCENT_RED
                } else {
                    ACCENT_GREEN
                };

                Line::from_spans(vec![
                    Span::styled(
                        format!(" {:>5} ", pid),
                        Style::new().fg(TEXT_DIM),
                    ),
                    Span::styled(
                        format!("{:>16}", comm),
                        Style::new().fg(TEXT_BRIGHT),
                    ),
                    Span::styled(
                        format!(" {:>4}", opens),
                        Style::new().fg(ACCENT_CYAN),
                    ),
                    Span::styled(
                        format!(" {} ", bar),
                        Style::new().fg(alert_color),
                    ),
                    Span::styled(
                        format!("⚠{}", alerts),
                        Style::new().fg(alert_color),
                    ),
                ])
            })
            .collect();

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(lines));
        paragraph.render(inner, frame);
    }

    fn render_extensions_panel(&self, frame: &mut Frame, area: Rect) {
        let block = Block::new()
            .title(" FILE TYPES ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(BORDER_SUBTLE));

        let inner = block.inner(area);
        block.render(area, frame);

        if self.ext_entries.is_empty() {
            let text = Paragraph::new(Line::from_spans(vec![Span::styled(
                "  Waiting for events...",
                Style::new().fg(TEXT_DIM),
            )]));
            text.render(inner, frame);
            return;
        }

        let max_count = self
            .ext_entries
            .iter()
            .map(|(_, c)| *c)
            .max()
            .unwrap_or(1);
        let max_rows = inner.height as usize;

        let lines: Vec<Line> = self
            .ext_entries
            .iter()
            .take(max_rows)
            .map(|(ext, count)| {
                let bar_width =
                    ((inner.width as f64 - 20.0) * (*count as f64 / max_count as f64)) as usize;
                let bar: String = "█".repeat(bar_width.min(30));

                let color = match ext.as_str() {
                    "pdf" | "doc" | "docx" => ACCENT_AMBER,
                    "rs" | "py" | "js" | "ts" => ACCENT_GREEN,
                    "jpg" | "png" | "mp4" => ACCENT_PURPLE,
                    "enc" | "locked" => ACCENT_RED,
                    _ => ACCENT_BLUE,
                };

                Line::from_spans(vec![
                    Span::styled(
                        format!("  .{:<12}", ext),
                        Style::new().fg(color),
                    ),
                    Span::styled(
                        format!(" {:>6} ", count),
                        Style::new().fg(TEXT_BRIGHT),
                    ),
                    Span::styled(bar, Style::new().fg(color)),
                ])
            })
            .collect();

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(lines));
        paragraph.render(inner, frame);
    }

    fn render_network_panel(&self, frame: &mut Frame, area: Rect) {
        let border = if self.focused == 2 {
            BORDER_ACTIVE
        } else {
            BORDER_SUBTLE
        };

        let title = format!(" NETWORK ({}) ", self.network.len());
        let block = Block::new()
            .title(title.as_str())
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border));

        let inner = block.inner(area);
        block.render(area, frame);

        if self.network.is_empty() {
            let text = Paragraph::new(Line::from_spans(vec![Span::styled(
                "  Waiting for network events...",
                Style::new().fg(TEXT_DIM),
            )]));
            text.render(inner, frame);
            return;
        }

        let inner_height = inner.height as usize;
        let lines: Vec<Line> = self
            .network
            .iter()
            .rev()
            .take(inner_height)
            .map(|entry| {
                let kind_color = match entry.kind.as_str() {
                    "Connect" => ACCENT_BLUE,
                    "Accept" => ACCENT_GREEN,
                    "SendTo" => ACCENT_AMBER,
                    "RecvFrom" => ACCENT_PURPLE,
                    _ => TEXT_DIM,
                };
                Line::from_spans(vec![
                    Span::styled(format!("{} ", entry.ts), Style::new().fg(TEXT_DIM)),
                    Span::styled(
                        format!("{:>8} ", entry.kind),
                        Style::new().fg(kind_color).attrs(StyleFlags::BOLD),
                    ),
                    Span::styled(format!("[{}] ", entry.pid), Style::new().fg(TEXT_BRIGHT)),
                    Span::raw(&entry.comm),
                    Span::styled(
                        format!(" -> {}", entry.addr),
                        Style::new().fg(ACCENT_PURPLE),
                    ),
                ])
            })
            .collect();

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(lines));
        paragraph.render(inner, frame);
    }

    fn render_top_files_panel(&self, frame: &mut Frame, area: Rect) {
        let block = Block::new()
            .title(" TOP FILES ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(BORDER_SUBTLE));

        let inner = block.inner(area);
        block.render(area, frame);

        if self.top_files.is_empty() {
            let text = Paragraph::new(Line::from_spans(vec![Span::styled(
                "  Waiting for events...",
                Style::new().fg(TEXT_DIM),
            )]));
            text.render(inner, frame);
            return;
        }

        let max_count = self
            .top_files
            .iter()
            .map(|f| f.count)
            .max()
            .unwrap_or(1);
        let max_rows = inner.height as usize;

        let lines: Vec<Line> = self
            .top_files
            .iter()
            .take(max_rows)
            .map(|entry| {
                let bar_width = ((inner.width as f64 - 40.0) * (entry.count as f64 / max_count as f64))
                    as usize;
                let bar: String = "█".repeat(bar_width.min(20));

                // Shorten path
                let short_path = entry
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&entry.path);
                let truncated = if short_path.len() > 18 {
                    format!("{}…", &short_path[..17])
                } else {
                    short_path.to_string()
                };

                let entropy_color = if entry.entropy > 0.7 {
                    ACCENT_RED
                } else if entry.entropy > 0.4 {
                    ACCENT_AMBER
                } else {
                    ACCENT_GREEN
                };

                Line::from_spans(vec![
                    Span::styled(
                        format!(" {:>18}", truncated),
                        Style::new().fg(TEXT_BRIGHT),
                    ),
                    Span::styled(
                        format!(" {:>5} ", entry.count),
                        Style::new().fg(ACCENT_CYAN),
                    ),
                    Span::styled(bar, Style::new().fg(ACCENT_BLUE)),
                    Span::styled(
                        format!(" H:{:.1}", entry.entropy),
                        Style::new().fg(entropy_color),
                    ),
                ])
            })
            .collect();

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(lines));
        paragraph.render(inner, frame);
    }

    fn render_alerts_panel(&self, frame: &mut Frame, area: Rect) {
        let alert_count = self.alerts.len();
        let border = if alert_count > 0 {
            ACCENT_RED
        } else if self.focused == 5 {
            BORDER_ACTIVE
        } else {
            BORDER_SUBTLE
        };

        let title = format!(" ⚠ ALERTS ({}) ", alert_count);
        let block = Block::new()
            .title(title.as_str())
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border));

        let inner = block.inner(area);
        block.render(area, frame);

        if alert_count == 0 {
            let text = Paragraph::new(Line::from_spans(vec![
                Span::styled(
                    "  ✓ ",
                    Style::new().fg(ACCENT_GREEN).attrs(StyleFlags::BOLD),
                ),
                Span::styled("all clear — no alerts", Style::new().fg(TEXT_BRIGHT)),
            ]));
            text.render(inner, frame);
        } else {
            let mut state = self.alerts_state.clone();
            self.alerts.render(inner, frame, &mut state);
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        // Split status area into two rows: status line + sparklines
        let chunks = Flex::vertical()
            .constraints([Constraint::Fixed(1), Constraint::Fixed(1)])
            .split(area);

        // Status line
        let panel_name = Panel::all()[self.focused].name();
        let rate_str = format!(
            "evt/s: {:.0}  open/s: {:.0}  alerts: {}",
            self.events_per_sec, self.opens_per_sec, self.total_alerts
        );
        let status = StatusLine::new()
            .left(StatusItem::text(panel_name))
            .center(StatusItem::text(&rate_str))
            .right(StatusItem::key_hint("?", "Help"));
        status.render(chunks[0], frame);

        // Sparkline row
        if self.rate_history.len() > 1 {
            let spark_width = chunks[1].width as usize;
            let recent: Vec<f64> = self
                .rate_history
                .iter()
                .rev()
                .take(spark_width)
                .rev()
                .map(|r| r.exec_count as f64)
                .collect();

            let sparkline = Sparkline::new(&recent)
                .style(Style::new().fg(ACCENT_BLUE));

            let spark_area = Rect::new(chunks[1].x, chunks[1].y, chunks[1].width, 1);
            sparkline.render(spark_area, frame);
        }
    }

    fn render_help(&self, frame: &mut Frame, area: Rect) {
        let help_lines = vec![
            Line::from_spans(vec![Span::styled(
                "  ⬡ HALCYON — Keyboard Shortcuts",
                Style::new().fg(ACCENT_BLUE).attrs(StyleFlags::BOLD),
            )]),
            Line::raw(""),
            Line::from_spans(vec![Span::styled(
                "  Navigation",
                Style::new().fg(ACCENT_AMBER).attrs(StyleFlags::BOLD),
            )]),
            Line::raw("  Tab / Shift+Tab    Cycle panels"),
            Line::raw("  1-6               Jump to panel"),
            Line::raw("  ↑/↓ or j/k        Scroll"),
            Line::raw("  PgUp/PgDn         Page scroll"),
            Line::raw("  Home / End        Jump to top/bottom"),
            Line::raw(""),
            Line::from_spans(vec![Span::styled(
                "  Actions",
                Style::new().fg(ACCENT_AMBER).attrs(StyleFlags::BOLD),
            )]),
            Line::raw("  p                 Pause/Resume"),
            Line::raw("  q / Esc           Quit (or clear scroll)"),
            Line::raw("  Ctrl+C            Force quit"),
            Line::raw(""),
            Line::from_spans(vec![Span::styled(
                "  Panels",
                Style::new().fg(ACCENT_AMBER).attrs(StyleFlags::BOLD),
            )]),
            Line::raw("  EVENTS       Live event log"),
            Line::raw("  PROCESSES    Process tree with open counts"),
            Line::raw("  NETWORK      Real-time connections"),
            Line::raw("  TOP FILES    Most-accessed files + entropy"),
            Line::raw("  FILE TYPES   Extension frequency"),
            Line::raw("  ALERTS       Ransomware-style alerts"),
            Line::raw(""),
            Line::from_spans(vec![Span::styled(
                "  FrankenTUI — diff-based renderer, zero flicker",
                Style::new().fg(TEXT_DIM),
            )]),
        ];

        let block = Block::new()
            .title(" Help ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(ACCENT_BLUE));

        let inner = block.inner(area);
        block.render(area, frame);

        let paragraph = Paragraph::new(ftui_text::Text::from_lines(help_lines));
        paragraph.render(inner, frame);
    }
}

// ── Public API — called from main.rs ──────────────────────────────────────

/// Run the FrankenTUI-based TUI.
///
/// Spawns a monitor-polling thread that feeds events into the TUI via MPSC channel.
pub fn run(mut monitor: Monitor) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<MonitorSnapshot>();

    // Spawn monitor polling thread
    let mut tick_count: u64 = 0;
    let monitor_thread = thread::Builder::new()
        .name("halcyon-tui-poll".into())
        .spawn(move || {
            loop {
                let poll_outputs: Vec<MonitorOutput> = monitor
                    .poll()
                    .into_iter()
                    .filter_map(|o| match o {
                        Output::Event(ev) => {
                            let kind_name = format!("{:?}", ev.kind);
                            Some(MonitorOutput::Event {
                                ts: ev.ts,
                                kind_name,
                                pid: ev.pid,
                                comm: ev.comm,
                                file: ev.file,
                            })
                        }
                        Output::Alert(al) => Some(MonitorOutput::Alert {
                            ts: al.ts,
                            pid: al.pid,
                            comm: al.comm,
                            opens: al.opens,
                        }),
                    })
                    .collect();

                // Send snapshot every ~100ms (every 6 ticks at 16ms)
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
                        break; // TUI closed
                    }
                }

                thread::sleep(Duration::from_millis(16)); // ~60fps
            }
        })?;

    let app = HalcyonApp::new(rx);

    let result = App::new(app).screen_mode(ScreenMode::AltScreen).run();

    // Wait for the monitor thread to finish
    let _ = monitor_thread.join();

    result.map_err(|e| anyhow::anyhow!("TUI error: {}", e))
}
