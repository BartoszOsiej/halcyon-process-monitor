use std::collections::VecDeque;
use std::io;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table,
    },
    Frame,
};

use crate::monitor::{Kind, Monitor, Output};

// ── Constants ─────────────────────────────────────────────────────────────

const LOG_CAP: usize = 5000;
const ALERT_CAP: usize = 200;
const NETWORK_CAP: usize = 500;
const HEATMAP_BUCKETS: usize = 10;
const MAX_FILES: usize = 12;
const TICK_MS: u64 = 50; // 20 FPS

// ── Cyberpunk palette ─────────────────────────────────────────────────────

const CYAN: Color = Color::Rgb(0, 255, 255);
const MAGENTA: Color = Color::Rgb(255, 0, 255);
const NEON_GREEN: Color = Color::Rgb(0, 255, 100);
const NEON_RED: Color = Color::Rgb(255, 50, 50);
const NEON_YELLOW: Color = Color::Rgb(255, 255, 0);
const NEON_ORANGE: Color = Color::Rgb(255, 160, 0);
const DIM: Color = Color::Rgb(80, 80, 100);
const DIM_BRIGHT: Color = Color::Rgb(120, 120, 150);
const PANEL_BORDER: Color = Color::Rgb(60, 60, 90);
const PANEL_BORDER_ACTIVE: Color = Color::Rgb(0, 180, 255);
const BG_DARK: Color = Color::Rgb(5, 5, 15);
const BG_PANEL: Color = Color::Rgb(10, 10, 30);

// ── Panel IDs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Events = 0,
    ProcessTree = 1,
    Network = 2,
    TopFiles = 3,
    Extensions = 4,
    Alerts = 5,
    Heatmap = 6,
}

impl Panel {
    fn all() -> &'static [Panel] {
        &[
            Panel::Events,
            Panel::ProcessTree,
            Panel::Network,
            Panel::TopFiles,
            Panel::Extensions,
            Panel::Alerts,
            Panel::Heatmap,
        ]
    }

    fn name(&self) -> &str {
        match self {
            Panel::Events => "EVENTS",
            Panel::ProcessTree => "PROCESSES",
            Panel::Network => "NETWORK",
            Panel::TopFiles => "TOP FILES",
            Panel::Extensions => "FILE TYPES",
            Panel::Alerts => "ALERTS",
            Panel::Heatmap => "HEATMAP",
        }
    }
}

// ── App state ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct LogLine {
    style: Style,
    text: String,
    kind: LogKind,
}

#[derive(Clone, Copy, PartialEq)]
enum LogKind {
    Exec,
    Open,
    Network,
    Alert,
    Info,
}

#[derive(Clone)]
struct NetworkEntry {
    ts: String,
    pid: u32,
    comm: String,
    kind: String,
    addr: String,
    bytes: Option<String>,
}

#[derive(Clone)]
struct HeatmapBucket {
    exec_count: u64,
    open_count: u64,
    network_count: u64,
    alert_count: u64,
}

struct App {
    log: VecDeque<LogLine>,
    alerts: VecDeque<LogLine>,
    network: VecDeque<NetworkEntry>,
    heatmap: VecDeque<HeatmapBucket>,
    scroll: usize,
    paused: bool,
    focused: usize,
    help_visible: bool,
    search_mode: bool,
    search_query: String,
    cursor_pos: usize,
    detail_pid: Option<u32>,
    show_detail: bool,
    tick_count: u64,
    last_tick: Instant,
    // Stats for status bar
    events_per_sec: f64,
    opens_per_sec: f64,
    alerts_per_sec: f64,
    network_per_sec: f64,
    // Pane split ratios (0.0 - 1.0)
    left_ratio: f64,
    middle_ratio: f64,
}

impl App {
    fn new() -> Self {
        let mut heatmap = VecDeque::with_capacity(120);
        for _ in 0..120 {
            heatmap.push_back(HeatmapBucket {
                exec_count: 0,
                open_count: 0,
                network_count: 0,
                alert_count: 0,
            });
        }
        Self {
            log: VecDeque::new(),
            alerts: VecDeque::new(),
            network: VecDeque::new(),
            heatmap,
            scroll: 0,
            paused: false,
            focused: 0,
            help_visible: false,
            search_mode: false,
            search_query: String::new(),
            cursor_pos: 0,
            detail_pid: None,
            show_detail: false,
            tick_count: 0,
            last_tick: Instant::now(),
            events_per_sec: 0.0,
            opens_per_sec: 0.0,
            alerts_per_sec: 0.0,
            network_per_sec: 0.0,
            left_ratio: 0.35,
            middle_ratio: 0.35,
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        // Help overlay — any key closes
        if self.help_visible {
            self.help_visible = false;
            return false;
        }

        // Detail view — Esc or 'd' closes
        if self.show_detail {
            match key.code {
                KeyCode::Esc | KeyCode::Char('d') | KeyCode::Char('q') => {
                    self.show_detail = false;
                    self.detail_pid = None;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    // scroll detail up
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // scroll detail down
                }
                _ => {}
            }
            return false;
        }

        // Search mode
        if self.search_mode {
            match key.code {
                KeyCode::Esc => {
                    self.search_mode = false;
                    self.search_query.clear();
                    self.cursor_pos = 0;
                }
                KeyCode::Enter => {
                    self.search_mode = false;
                    self.scroll = 0; // jump to first match
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.cursor_pos = self.cursor_pos.saturating_sub(1);
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.cursor_pos += 1;
                }
                _ => {}
            }
            return false;
        }

        // Normal mode
        match key.code {
            // Quit
            KeyCode::Char('q') | KeyCode::Esc => {
                if self.focused == 0 && self.scroll == 0 {
                    return true; // quit only when at top of events
                }
                self.scroll = 0;
                false
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => true,

            // Pause
            KeyCode::Char('p') => {
                self.paused = !self.paused;
                false
            }

            // Clear
            KeyCode::Char('c') => {
                self.log.clear();
                self.alerts.clear();
                self.network.clear();
                self.scroll = 0;
                false
            }

            // Navigation
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_add(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_sub(1);
                false
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(20);
                false
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(20);
                false
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.scroll = usize::MAX;
                false
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.scroll = 0;
                false
            }

            // Panel navigation
            KeyCode::Tab => {
                self.focused = (self.focused + 1) % Panel::all().len();
                self.scroll = 0;
                false
            }
            KeyCode::BackTab => {
                self.focused = if self.focused == 0 {
                    Panel::all().len() - 1
                } else {
                    self.focused - 1
                };
                self.scroll = 0;
                false
            }

            // Direct panel focus with number keys
            KeyCode::Char('1') => { self.focused = 0; self.scroll = 0; false }
            KeyCode::Char('2') => { self.focused = 1; self.scroll = 0; false }
            KeyCode::Char('3') => { self.focused = 2; self.scroll = 0; false }
            KeyCode::Char('4') => { self.focused = 3; self.scroll = 0; false }
            KeyCode::Char('5') => { self.focused = 4; self.scroll = 0; false }
            KeyCode::Char('6') => { self.focused = 5; self.scroll = 0; false }
            KeyCode::Char('7') => { self.focused = 6; self.scroll = 0; false }

            // Search
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_query.clear();
                self.cursor_pos = 0;
                false
            }

            // Help
            KeyCode::Char('?') | KeyCode::Char('h') => {
                self.help_visible = true;
                false
            }

            // Process detail — press Enter on focused panel to open detail
            KeyCode::Enter => {
                if self.focused == 1 {
                    // Process tree — open detail for highlighted process
                    self.show_detail = true;
                    // Detail PID would be set from the currently highlighted row
                }
                false
            }

            // Pane resize with [ and ]
            KeyCode::Char('[') => {
                self.left_ratio = (self.left_ratio - 0.05).max(0.15);
                false
            }
            KeyCode::Char(']') => {
                self.left_ratio = (self.left_ratio + 0.05).min(0.55);
                false
            }
            KeyCode::Char('{') => {
                self.middle_ratio = (self.middle_ratio - 0.05).max(0.15);
                false
            }
            KeyCode::Char('}') => {
                self.middle_ratio = (self.middle_ratio + 0.05).min(0.55);
                false
            }

            _ => false,
        }
    }

    fn on_events(&mut self, outputs: Vec<Output>) {
        for output in outputs {
            match output {
                Output::Event(ev) => {
                    // Update heatmap
                    if let Some(bucket) = self.heatmap.back_mut() {
                        match ev.kind {
                            Kind::Exec => bucket.exec_count += 1,
                            Kind::Open => bucket.open_count += 1,
                            _ => {}
                        }
                    }

                    // Check search filter
                    let passes_search = if self.search_query.is_empty() {
                        true
                    } else {
                        let q = self.search_query.to_lowercase();
                        match ev.kind {
                            Kind::Exec => {
                                ev.comm.to_lowercase().contains(&q)
                                    || ev.argv.as_deref().map(|a| a.to_lowercase().contains(&q)).unwrap_or(false)
                            }
                            Kind::Open => {
                                ev.file.as_deref().map(|f| f.to_lowercase().contains(&q)).unwrap_or(false)
                                    || ev.comm.to_lowercase().contains(&q)
                            }
                            _ => ev.comm.to_lowercase().contains(&q),
                        }
                    };

                    if !passes_search {
                        continue;
                    }

                    let (style, tag, body, kind) = match ev.kind {
                        Kind::Exec => {
                            let argv_info = ev.argv.as_ref().map(|a| format!(" {a}")).unwrap_or_default();
                            (
                                Style::new().fg(NEON_GREEN).add_modifier(Modifier::BOLD),
                                String::from("EXEC"),
                                format!("[{}] {} (uid {}){}", ev.pid, ev.comm, ev.uid, argv_info),
                                LogKind::Exec,
                            )
                        }
                        Kind::Open => {
                            let ext_badge = ev
                                .extension
                                .as_ref()
                                .filter(|e| !e.is_empty())
                                .map(|e| format!(".{e}"))
                                .unwrap_or_default();
                            (
                                Style::new().fg(CYAN),
                                String::from("OPEN"),
                                format!(
                                    "[{}] {} -> {} {}",
                                    ev.pid,
                                    ev.comm,
                                    ev.file.as_deref().unwrap_or("?"),
                                    if ext_badge.is_empty() {
                                        String::new()
                                    } else {
                                        format!("[{ext_badge}]")
                                    }
                                ),
                                LogKind::Open,
                            )
                        }
                        Kind::Connect | Kind::Accept | Kind::SendTo | Kind::RecvFrom => {
                            let kind_str = format!("{:?}", ev.kind);
                            let addr = ev.file.as_deref().unwrap_or("?");
                            let bytes_str = ev.bytes.as_ref().map(|b| format!(" ({b} bytes)")).unwrap_or_default();
                            // Add to network panel
                            self.push_network(NetworkEntry {
                                ts: ev.ts.clone(),
                                pid: ev.pid,
                                comm: ev.comm.clone(),
                                kind: kind_str.clone(),
                                addr: addr.to_string(),
                                bytes: ev.bytes.clone(),
                            });
                            if let Some(bucket) = self.heatmap.back_mut() {
                                bucket.network_count += 1;
                            }
                            (
                                Style::new().fg(MAGENTA),
                                kind_str,
                                format!("[{}] {} -> {}{}", ev.pid, ev.comm, addr, bytes_str),
                                LogKind::Network,
                            )
                        }
                    };
                    self.push_log(LogLine {
                        style,
                        text: format!("{} {:>8} {}", ev.ts, tag, body),
                        kind,
                    });
                }
                Output::Alert(alert) => {
                    if let Some(bucket) = self.heatmap.back_mut() {
                        bucket.alert_count += 1;
                    }
                    let line = LogLine {
                        style: Style::new()
                            .fg(NEON_RED)
                            .add_modifier(Modifier::BOLD),
                        text: format!(
                            "{} ⚠ ALERT [{}] {} — {} opens/s",
                            alert.ts, alert.pid, alert.comm, alert.opens
                        ),
                        kind: LogKind::Alert,
                    };
                    self.push_log(line.clone());
                    if self.alerts.len() >= ALERT_CAP {
                        self.alerts.pop_front();
                    }
                    self.alerts.push_back(line);
                }
            }
        }
    }

    fn update_rates(&mut self) {
        self.tick_count += 1;
        if self.last_tick.elapsed() >= Duration::from_secs(1) {
            let ticks = self.tick_count as f64;
            self.events_per_sec = ticks / self.last_tick.elapsed().as_secs_f64();
            // Approximate from recent heatmap
            if let Some(last) = self.heatmap.back() {
                self.opens_per_sec = last.open_count as f64;
                self.alerts_per_sec = last.alert_count as f64;
                self.network_per_sec = last.network_count as f64;
            }
            self.last_tick = Instant::now();
            self.tick_count = 0;
            // Rotate heatmap (new bucket every second)
            self.heatmap.pop_front();
            self.heatmap.push_back(HeatmapBucket {
                exec_count: 0,
                open_count: 0,
                network_count: 0,
                alert_count: 0,
            });
        }
    }

    fn push_log(&mut self, line: LogLine) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    fn push_network(&mut self, entry: NetworkEntry) {
        if self.network.len() >= NETWORK_CAP {
            self.network.pop_front();
        }
        self.network.push_back(entry);
    }

    fn get_visible_log(&self, height: usize) -> Vec<&LogLine> {
        let total = self.log.len();
        let max_scroll = total.saturating_sub(height);
        let scroll = self.scroll.min(max_scroll);
        let start = total.saturating_sub(height.saturating_add(scroll));
        self.log.iter().skip(start).take(height).collect()
    }
}

// ── Public entry point ────────────────────────────────────────────────────

pub fn run(monitor: &mut Monitor) -> Result<(), anyhow::Error> {
    let mut terminal = ratatui::init();
    let mut app = App::new();
    let tick = Duration::from_millis(TICK_MS);

    let result = (|| -> io::Result<()> {
        loop {
            if crate::QUIT.load(Ordering::SeqCst) {
                break;
            }
            if event::poll(tick)? {
                if let TermEvent::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press && app.on_key(key) {
                        break;
                    }
                }
            }
            if !app.paused {
                app.on_events(monitor.poll());
                app.update_rates();
            }
            terminal.draw(|frame| draw(frame, &app, monitor))?;
        }
        Ok(())
    })();

    ratatui::restore();
    result?;
    Ok(())
}

// ── Rendering ─────────────────────────────────────────────────────────────

fn draw(frame: &mut Frame, app: &App, monitor: &Monitor) {
    frame.render_widget(Clear, frame.area());
    let area = frame.area();

    // Main layout: header (2) | tab bar (1) | sparklines (2) | body | status (1) | footer (1)
    let [header_area, tab_area, spark_area, body_area, status_area, footer_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, app, monitor, header_area);
    draw_tab_bar(frame, app, tab_area);
    draw_sparklines(frame, monitor, spark_area);

    // Overlay: help or detail
    if app.help_visible {
        draw_help_overlay(frame, body_area);
    } else if app.show_detail {
        draw_detail_overlay(frame, app, monitor, body_area);
    } else {
        draw_body(frame, app, monitor, body_area);
    }

    draw_status_bar(frame, app, monitor, status_area);
    draw_footer(frame, app, footer_area);
}

// ── Header ────────────────────────────────────────────────────────────────

fn draw_header(frame: &mut Frame, app: &App, monitor: &Monitor, area: Rect) {
    let [logo_area, stats_area] =
        Layout::horizontal([Constraint::Length(38), Constraint::Min(0)]).areas(area);

    let logo_lines = vec![
        Line::from(vec![Span::styled(
            "  ⚡ HALCYON eBPF MONITOR",
            Style::new()
                .fg(CYAN)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "  kernel tracepoint · real-time · v0.4",
            Style::new().fg(DIM),
        )]),
    ];
    frame.render_widget(Paragraph::new(logo_lines), logo_area);

    let status_color = if app.paused { NEON_YELLOW } else { NEON_GREEN };
    let status_text = if app.paused { "▐ PAUSED " } else { "▐ LIVE " };

    let elapsed = monitor.uptime().as_secs();
    let (days, hours, mins, secs) = (
        elapsed / 86400,
        (elapsed % 86400) / 3600,
        (elapsed % 3600) / 60,
        elapsed % 60,
    );
    let uptime_str = if days > 0 {
        format!("{days}d {hours:02}:{mins:02}:{secs:02}")
    } else {
        format!("{hours:02}:{mins:02}:{secs:02}")
    };

    let stats_line = Line::from(vec![
        Span::styled(
            status_text,
            Style::new()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  evt {:>9}  │  lost {:>6}  │  uptime {}  │  threshold {} /s  ",
                format_number(monitor.total_events),
                format_number(monitor.total_lost),
                uptime_str,
                monitor.threshold,
            ),
            Style::new().fg(DIM_BRIGHT),
        ),
    ]);
    frame.render_widget(Paragraph::new(stats_line), stats_area);
}

// ── Tab bar ───────────────────────────────────────────────────────────────

fn draw_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    let mut all_spans: Vec<Span> = Vec::new();
    for (i, p) in Panel::all().iter().enumerate() {
        let color = if i == app.focused { CYAN } else { DIM };
        let modifier = if i == app.focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        all_spans.push(Span::styled(
            format!(" [{}] {} ", i + 1, p.name()),
            Style::new().fg(color).add_modifier(modifier),
        ));
    }
    let tab_widget = Paragraph::new(Line::from(all_spans));
    frame.render_widget(tab_widget, area);
}

// ── Sparklines (event rate) ───────────────────────────────────────────────

fn draw_sparklines(frame: &mut Frame, monitor: &Monitor, area: Rect) {
    let [exec_area, open_area, net_area, alert_area] = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(30),
        Constraint::Percentage(25),
        Constraint::Percentage(15),
    ])
    .areas(area);

    let history = monitor.rate_history();

    // Exec sparkline
    let exec_data: Vec<u64> = history.iter().map(|s| s.exec_count).collect();
    let max_exec = exec_data.iter().copied().max().unwrap_or(1).max(1);
    let spark = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    format!(" exec/s (peak {max_exec}) "),
                    Style::new().fg(NEON_GREEN).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(PANEL_BORDER)),
        )
        .data(&exec_data)
        .max(max_exec);
    frame.render_widget(spark, exec_area);

    // Open sparkline
    let open_data: Vec<u64> = history.iter().map(|s| s.open_count).collect();
    let max_open = open_data.iter().copied().max().unwrap_or(1).max(1);
    let spark = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    format!(" open/s (peak {max_open}) "),
                    Style::new().fg(CYAN).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(PANEL_BORDER)),
        )
        .data(&open_data)
        .max(max_open);
    frame.render_widget(spark, open_area);

    // Network sparkline
    let net_data: Vec<u64> = history.iter().map(|s| s.open_count / 2).collect(); // approx
    let max_net = net_data.iter().copied().max().unwrap_or(1).max(1);
    let spark = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " net/s ",
                    Style::new().fg(MAGENTA).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(PANEL_BORDER)),
        )
        .data(&net_data)
        .max(max_net);
    frame.render_widget(spark, net_area);

    // Alert sparkline
    let alert_data: Vec<u64> = history.iter().map(|s| s.alert_count).collect();
    let max_alert = alert_data.iter().copied().max().unwrap_or(1).max(1);
    let spark = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " alerts/s ",
                    Style::new().fg(NEON_RED).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(PANEL_BORDER)),
        )
        .data(&alert_data)
        .max(max_alert);
    frame.render_widget(spark, alert_area);
}

// ── Body: multi-panel layout ──────────────────────────────────────────────

fn draw_body(frame: &mut Frame, app: &App, monitor: &Monitor, area: Rect) {
    // 3 columns: left (events) | middle (processes + extensions) | right (network + files + alerts)
    let left_pct = (app.left_ratio * 100.0) as u16;
    let middle_pct = (app.middle_ratio * 100.0) as u16;
    let right_pct = 100 - left_pct - middle_pct;

    let [left, middle, right] = Layout::horizontal([
        Constraint::Percentage(left_pct),
        Constraint::Percentage(middle_pct),
        Constraint::Percentage(right_pct),
    ])
    .areas(area);

    let [middle_top, middle_bottom] =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(middle);

    let [right_top, right_middle, right_bottom] = Layout::vertical([
        Constraint::Percentage(35),
        Constraint::Percentage(35),
        Constraint::Percentage(30),
    ])
    .areas(right);

    draw_log(frame, app, left);
    draw_process_tree(frame, app, monitor, middle_top);
    draw_extensions(frame, monitor, middle_bottom);
    draw_network_panel(frame, app, right_top);
    draw_top_files(frame, monitor, right_middle);
    draw_alerts(frame, app, right_bottom);
}

// ── Event log ─────────────────────────────────────────────────────────────

fn draw_log(frame: &mut Frame, app: &App, area: Rect) {
    let inner_height = area.height.saturating_sub(2) as usize;

    let lines: Vec<Line> = app
        .get_visible_log(inner_height)
        .into_iter()
        .map(|l| {
            let mut spans = vec![Span::styled(l.text.clone(), l.style)];
            // Highlight search matches
            if !app.search_query.is_empty() {
                let text = l.text.to_lowercase();
                if text.contains(&app.search_query.to_lowercase()) {
                    spans.push(Span::styled(
                        " ◀ MATCH",
                        Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD),
                    ));
                }
            }
            Line::from(spans)
        })
        .collect();

    let focus_border = if app.focused == 0 {
        PANEL_BORDER_ACTIVE
    } else {
        PANEL_BORDER
    };
    let search_indicator = if app.search_mode {
        format!(" [/{}}}]", app.search_query)
    } else if !app.search_query.is_empty() {
        format!(" [/{}]", app.search_query)
    } else {
        String::new()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" EVENTS ({}) {} ", app.log.len(), search_indicator),
            Style::new().fg(CYAN).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(focus_border));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Process tree ─────────────────────────────────────────────────────────

fn draw_process_tree(frame: &mut Frame, app: &App, monitor: &Monitor, area: Rect) {
    let tree = monitor.build_process_tree();
    let flat = Monitor::flatten_tree(&tree);
    let inner_height = area.height.saturating_sub(2) as usize;

    let lines: Vec<Line> = flat
        .iter()
        .take(inner_height)
        .map(|(depth, node)| {
            let indent = "  ".repeat(*depth);
            let prefix = if node.children.is_empty() { "└─ " } else { "├─ " };
            let node_style = if node.alerts > 0 {
                Style::new().fg(NEON_RED).add_modifier(Modifier::BOLD)
            } else if node.total_opens > 100 {
                Style::new().fg(NEON_ORANGE)
            } else if node.total_opens > 0 {
                Style::new().fg(NEON_GREEN)
            } else {
                Style::new().fg(Color::White)
            };
            let pid_style = Style::new().fg(DIM_BRIGHT);
            let alert_badge = if node.alerts > 0 {
                format!(" ⚠{}", node.alerts)
            } else {
                String::new()
            };
            let opens_badge = if node.total_opens > 0 {
                // Mini bar visualization
                let bar_len = (node.total_opens.min(50) / 5) as usize;
                let bar = "█".repeat(bar_len);
                format!(" {} ({})", bar, node.total_opens)
            } else {
                String::new()
            };
            Line::from(vec![
                Span::styled(
                    format!("{indent}{prefix}"),
                    Style::new().fg(PANEL_BORDER),
                ),
                Span::styled(&node.comm, node_style),
                Span::styled(
                    format!(" [{}]", node.pid),
                    pid_style,
                ),
                Span::styled(opens_badge, Style::new().fg(DIM_BRIGHT)),
                Span::styled(
                    alert_badge,
                    Style::new().fg(NEON_RED).add_modifier(Modifier::BOLD),
                ),
            ])
        })
        .collect();

    let focus_border = if app.focused == 1 {
        PANEL_BORDER_ACTIVE
    } else {
        PANEL_BORDER
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" PROCESS TREE ({}) ", flat.len()),
            Style::new().fg(CYAN).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(focus_border));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Network panel ─────────────────────────────────────────────────────────

fn draw_network_panel(frame: &mut Frame, app: &App, area: Rect) {
    let inner_height = area.height.saturating_sub(2) as usize;

    let lines: Vec<Line> = app
        .network
        .iter()
        .rev()
        .take(inner_height)
        .map(|entry| {
            let kind_color = match entry.kind.as_str() {
                "Connect" => MAGENTA,
                "Accept" => CYAN,
                "SendTo" => NEON_GREEN,
                "RecvFrom" => NEON_YELLOW,
                _ => DIM,
            };
            let kind_icon = match entry.kind.as_str() {
                "Connect" => "↗",
                "Accept" => "↙",
                "SendTo" => "📤",
                "RecvFrom" => "📥",
                _ => "•",
            };
            let bytes_info = entry
                .bytes
                .as_ref()
                .map(|b| format!(" [{b}]"))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(
                    format!("{} ", entry.ts),
                    Style::new().fg(DIM),
                ),
                Span::styled(
                    format!("{kind_icon} {:>8}", entry.kind),
                    Style::new().fg(kind_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" [{}] ", entry.pid),
                    Style::new().fg(DIM_BRIGHT),
                ),
                Span::styled(&entry.comm, Style::new().fg(Color::White)),
                Span::styled(
                    format!(" -> {}", entry.addr),
                    Style::new().fg(MAGENTA),
                ),
                Span::styled(bytes_info, Style::new().fg(DIM)),
            ])
        })
        .collect();

    let focus_border = if app.focused == 2 {
        PANEL_BORDER_ACTIVE
    } else {
        PANEL_BORDER
    };
    let net_count = app.network.len();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" NETWORK ({}) ", net_count),
            Style::new().fg(MAGENTA).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(focus_border));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Extension frequency ───────────────────────────────────────────────────

fn draw_extensions(frame: &mut Frame, monitor: &Monitor, area: Rect) {
    let mut exts: Vec<(String, u64)> = monitor
        .extension_counts()
        .iter()
        .map(|(k, &v)| (k.clone(), v))
        .collect();
    exts.sort_by_key(|b| std::cmp::Reverse(b.1));
    exts.truncate(8);

    let max_ext = exts.iter().map(|e| e.1).max().unwrap_or(1).max(1);

    let lines: Vec<Line> = exts
        .iter()
        .map(|(ext, count)| {
            let bar_width = (count * 20 / max_ext) as usize;
            let filled = "█".repeat(bar_width);
            let empty = "░".repeat(20 - bar_width);
            let ext_color = match ext.as_str() {
                "pdf" | "doc" | "docx" | "xls" | "xlsx" => NEON_YELLOW,
                "zip" | "tar" | "gz" | "7z" | "rar" => MAGENTA,
                "enc" | "locked" | "crypt" | "cipher" => NEON_RED,
                "rs" | "py" | "js" | "ts" | "go" | "c" | "cpp" => NEON_GREEN,
                "jpg" | "png" | "mp4" | "avi" | "mkv" => NEON_ORANGE,
                _ => CYAN,
            };
            let count_color = if *count > max_ext / 2 {
                NEON_YELLOW
            } else {
                Color::White
            };
            Line::from(vec![
                Span::styled(
                    format!(" .{:<6}", ext),
                    Style::new().fg(ext_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(filled, Style::new().fg(ext_color)),
                Span::styled(empty, Style::new().fg(DIM)),
                Span::styled(
                    format!(" {:>5}", count),
                    Style::new().fg(count_color),
                ),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " FILE TYPES ",
            Style::new().fg(CYAN).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(PANEL_BORDER));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Top files ─────────────────────────────────────────────────────────────

fn draw_top_files(frame: &mut Frame, monitor: &Monitor, area: Rect) {
    let top = monitor.top_files(MAX_FILES);

    let rows: Vec<Row> = top
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let rank_style = if i == 0 {
                Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)
            } else if i < 3 {
                Style::new().fg(NEON_ORANGE)
            } else {
                Style::new().fg(DIM_BRIGHT)
            };
            let ext_color = match f.extension.as_str() {
                "pdf" | "doc" | "docx" => NEON_YELLOW,
                "enc" | "locked" | "crypt" => NEON_RED,
                "rs" | "py" | "js" | "ts" | "go" => NEON_GREEN,
                _ => CYAN,
            };
            let entropy_color = if f.entropy > 0.7 {
                NEON_RED
            } else if f.entropy > 0.5 {
                NEON_YELLOW
            } else if f.entropy > 0.3 {
                NEON_GREEN
            } else {
                DIM_BRIGHT
            };

            // Truncate path for display.
            let path_display = if f.path.len() > 16 {
                format!("…{}", &f.path[f.path.len() - 13..])
            } else {
                f.path.clone()
            };

            // Mini bar for count
            let bar_len = (f.count.min(20)) as usize;
            let bar = "▪".repeat(bar_len);

            Row::new(vec![
                Cell::from(format!("#{}", i + 1)).style(rank_style),
                Cell::from(path_display).style(Style::new().fg(Color::White)),
                Cell::from(format!(".{}", f.extension)).style(
                    Style::new().fg(ext_color).add_modifier(Modifier::BOLD),
                ),
                Cell::from(f.count.to_string()).style(Style::new().fg(NEON_GREEN)),
                Cell::from(bar).style(Style::new().fg(DIM_BRIGHT)),
                Cell::from(format!("{:.2}", f.entropy)).style(
                    Style::new().fg(entropy_color),
                ),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(8),
        Constraint::Length(6),
    ];

    let header = Row::new(vec![" #", "FILE", "EXT", "OPS", "BAR", "ENTR"])
        .style(
            Style::new()
                .fg(CYAN)
                .add_modifier(Modifier::BOLD),
        );

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    format!(" TOP FILES ({}) ", top.len()),
                    Style::new().fg(CYAN).add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(PANEL_BORDER)),
        );

    frame.render_widget(table, area);
}

// ── Alerts ────────────────────────────────────────────────────────────────

fn draw_alerts(frame: &mut Frame, app: &App, area: Rect) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let alert_count = app.alerts.len();

    let lines: Vec<Line> = if alert_count == 0 {
        vec![Line::from(vec![
            Span::styled(
                "  ◆ ",
                Style::new().fg(NEON_GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled("system clean — no alerts", Style::new().fg(DIM)),
        ])]
    } else {
        let mut lines: Vec<Line> = app
            .alerts
            .iter()
            .rev()
            .take(inner_height)
            .map(|l| Line::from(vec![Span::styled(l.text.clone(), l.style)]))
            .collect();
        lines.reverse();
        lines
    };

    let title_color = if alert_count > 0 { NEON_RED } else { NEON_GREEN };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" ⚠ ALERTS ({}) ", alert_count),
            Style::new()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(if alert_count > 0 {
            NEON_RED
        } else {
            PANEL_BORDER
        }));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Status bar ────────────────────────────────────────────────────────────

fn draw_status_bar(frame: &mut Frame, app: &App, _monitor: &Monitor, area: Rect) {
    let focused_panel = Panel::all()[app.focused];
    let search_status = if app.search_mode {
        format!(" SEARCH: /{}▏", app.search_query)
    } else if !app.search_query.is_empty() {
        format!(" filter: /{}", app.search_query)
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" 📊 panel: {} ", focused_panel.name()),
            Style::new().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " │ events/s: {:.0}  opens/s: {:.0}  net/s: {:.0}  alerts/s: {:.0} ",
                app.events_per_sec, app.opens_per_sec, app.network_per_sec, app.alerts_per_sec,
            ),
            Style::new().fg(DIM_BRIGHT),
        ),
        Span::styled(
            search_status,
            Style::new().fg(NEON_YELLOW),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::new().bg(BG_DARK)),
        area,
    );
}

// ── Footer ────────────────────────────────────────────────────────────────

fn draw_footer(frame: &mut Frame, _app: &App, area: Rect) {
    let mut spans = vec![];
    let key = |label: &str, desc: &str| -> Vec<Span<'static>> {
        vec![
            Span::styled(
                format!(" {label} "),
                Style::new()
                    .fg(NEON_YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(desc.to_string()),
            Span::styled(" │ ".to_string(), Style::new().fg(DIM)),
        ]
    };

    spans.extend(key("q", "quit"));
    spans.extend(key("p", "pause"));
    spans.extend(key("c", "clear"));
    spans.extend(key("↑↓", "scroll"));
    spans.extend(key("Tab", "panel"));
    spans.extend(key("1-7", "jump"));
    spans.extend(key("/", "search"));
    spans.extend(key("?", "help"));
    spans.extend(key("[]", "resize"));

    // Remove trailing separator
    if let Some(last) = spans.last() {
        if last.content == " │ " {
            spans.pop();
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().fg(DIM_BRIGHT)),
        area,
    );
}

// ── Help overlay ──────────────────────────────────────────────────────────

fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    let [_, center, _] = Layout::vertical([
        Constraint::Percentage(10),
        Constraint::Percentage(80),
        Constraint::Percentage(10),
    ])
    .areas(area);

    let help_text = vec![
        Line::from(vec![Span::styled(
            " ⚡ HALCYON KEYBINDINGS ",
            Style::new()
                .fg(CYAN)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  NAVIGATION", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        ]),
        Line::raw("    q / Esc      Quit (or clear scroll)"),
        Line::raw("    p            Pause / resume"),
        Line::raw("    c            Clear all panels"),
        Line::raw("    ↑ / k        Scroll up"),
        Line::raw("    ↓ / j        Scroll down"),
        Line::raw("    PgUp / PgDn  Page up/down"),
        Line::raw("    g / Home     Jump to top"),
        Line::raw("    G / End      Jump to bottom"),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  PANELS", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        ]),
        Line::raw("    Tab          Next panel"),
        Line::raw("    Shift+Tab    Previous panel"),
        Line::raw("    1-7          Jump to panel"),
        Line::raw("    Enter        Open process detail"),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  SEARCH & FILTER", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        ]),
        Line::raw("    /            Start search"),
        Line::raw("    Esc          Cancel search"),
        Line::raw("    Enter        Apply filter"),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  LAYOUT", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        ]),
        Line::raw("    [ / ]        Resize left panel"),
        Line::raw("    { / }        Resize middle panel"),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  OTHER", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        ]),
        Line::raw("    ? / h        Show this help"),
        Line::raw("    Ctrl+C       Force quit"),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " HELP — press any key to close ",
            Style::new()
                .fg(CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(CYAN))
        .style(Style::new().bg(BG_PANEL));

    frame.render_widget(Clear, center);
    frame.render_widget(Paragraph::new(help_text).block(block), center);
}

// ── Detail overlay ────────────────────────────────────────────────────────

fn draw_detail_overlay(frame: &mut Frame, _app: &App, monitor: &Monitor, area: Rect) {
    let [_, center, _] = Layout::vertical([
        Constraint::Percentage(5),
        Constraint::Percentage(90),
        Constraint::Percentage(5),
    ])
    .areas(area);

    let tree = monitor.build_process_tree();
    let flat = Monitor::flatten_tree(&tree);
    let inner_height = center.height.saturating_sub(4) as usize;

    let lines: Vec<Line> = flat
        .iter()
        .take(inner_height)
        .map(|(depth, node)| {
            let indent = "  ".repeat(*depth);
            let prefix = if node.children.is_empty() { "└─ " } else { "├─ " };
            let style = if node.alerts > 0 {
                Style::new().fg(NEON_RED).add_modifier(Modifier::BOLD)
            } else if node.total_opens > 0 {
                Style::new().fg(NEON_GREEN)
            } else {
                Style::new().fg(Color::White)
            };
            Line::from(vec![
                Span::styled(
                    format!("{indent}{prefix}{} [{}] opens={} alerts={}", node.comm, node.pid, node.total_opens, node.alerts),
                    style,
                ),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " PROCESS DETAIL — press d/Esc to close ",
            Style::new()
                .fg(CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(CYAN))
        .style(Style::new().bg(BG_PANEL));

    frame.render_widget(Clear, center);
    frame.render_widget(Paragraph::new(lines).block(block), center);
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{Alert, Kind, Monitor, Output, RecordedEvent};

    #[test]
    fn renders_with_events_and_alerts() {
        let monitor = Monitor::dummy();
        let mut app = App::new();
        app.on_events(vec![
            Output::Event(RecordedEvent {
                ts: "00:00:00.000".into(),
                kind: Kind::Exec,
                pid: 1,
                uid: 0,
                comm: "init".into(),
                file: None,
                extension: None,
                argv: Some("/sbin/init".into()),
                bytes: None,
            }),
            Output::Event(RecordedEvent {
                ts: "00:00:00.001".into(),
                kind: Kind::Open,
                pid: 2,
                uid: 1000,
                comm: "sh".into(),
                file: Some("/etc/passwd".into()),
                extension: None,
                argv: None,
                bytes: None,
            }),
            Output::Alert(Alert {
                ts: "00:00:00.002".into(),
                pid: 2,
                uid: 1000,
                comm: "sh".into(),
                opens: 3,
            }),
        ]);

        let backend = ratatui::backend::TestBackend::new(160, 50);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app, &monitor)).unwrap();
    }    #[test]
    fn key_handling() {
        let mut app = App::new();
        app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        assert!(app.paused);
        // q at top of events panel returns true (quit)
        assert!(app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn tab_cycles_focus() {
        let mut app = App::new();
        let panel_count = Panel::all().len();
        assert_eq!(app.focused, 0);
        for i in 1..panel_count {
            app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
            assert_eq!(app.focused, i);
        }
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focused, 0);
    }

    #[test]
    fn number_keys_jump_to_panel() {
        let mut app = App::new();
        app.on_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.focused, 2);
        app.on_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        assert_eq!(app.focused, 6);
    }

    #[test]
    fn search_mode() {
        let mut app = App::new();
        app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(app.search_mode);
        app.on_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!(app.search_query, "tes");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.search_mode);
        assert_eq!(app.search_query, "tes");
    }

    #[test]
    fn help_overlay() {
        let mut app = App::new();
        app.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.help_visible);
        app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(!app.help_visible);
    }

    #[test]
    fn pane_resize() {
        let mut app = App::new();
        let orig = app.left_ratio;
        app.on_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert!(app.left_ratio > orig);
        app.on_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert!((app.left_ratio - orig).abs() < 0.01);
    }

    #[test]
    fn format_number_scales() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1500), "1.5K");
        assert_eq!(format_number(1_500_000), "1.5M");
    }
}
