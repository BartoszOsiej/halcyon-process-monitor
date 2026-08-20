use std::collections::VecDeque;
use std::io;
use std::sync::atomic::Ordering;
use std::time::Duration;

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

const LOG_CAP: usize = 2000;
const ALERT_CAP: usize = 50;
const MAX_FILES: usize = 8;
const TICK_MS: u64 = 100;

// ── Cyberpunk palette ─────────────────────────────────────────────────────

const CYAN: Color = Color::Rgb(0, 255, 255);
const MAGENTA: Color = Color::Rgb(255, 0, 255);
const NEON_GREEN: Color = Color::Rgb(0, 255, 100);
const NEON_RED: Color = Color::Rgb(255, 50, 50);
const NEON_YELLOW: Color = Color::Rgb(255, 255, 0);
const DIM: Color = Color::Rgb(80, 80, 100);
const DIM_BRIGHT: Color = Color::Rgb(120, 120, 150);
const PANEL_BORDER: Color = Color::Rgb(60, 60, 90);

fn cyber_title(text: &str) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::new()
            .fg(CYAN)
            .add_modifier(Modifier::BOLD),
    )
}

// ── App state ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct LogLine {
    style: Style,
    text: String,
}

struct App {
    log: VecDeque<LogLine>,
    alerts: VecDeque<LogLine>,
    scroll: usize,
    paused: bool,
    /// 0 = left panel (events), 1 = middle (processes), 2 = right (files)
    focus: usize,
}

impl App {
    fn new() -> Self {
        Self {
            log: VecDeque::new(),
            alerts: VecDeque::new(),
            scroll: 0,
            paused: false,
            focus: 0,
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
            KeyCode::Char('p') => {
                self.paused = !self.paused;
                false
            }
            KeyCode::Char('c') => {
                self.log.clear();
                self.alerts.clear();
                self.scroll = 0;
                false
            }
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
            KeyCode::Home => {
                self.scroll = usize::MAX;
                false
            }
            KeyCode::End => {
                self.scroll = 0;
                false
            }
            KeyCode::Tab => {
                self.focus = (self.focus + 1) % 3;
                false
            }
            _ => false,
        }
    }

    fn on_events(&mut self, outputs: Vec<Output>) {
        for output in outputs {
            match output {
                Output::Event(ev) => {
                    let (style, tag, body) = match ev.kind {
                        Kind::Exec => {
                            let argv_info = ev.argv.as_ref().map(|a| format!(" {a}")).unwrap_or_default();
                            (
                                Style::new().fg(NEON_GREEN),
                                "EXEC",
                                format!("[{}] {} (uid {}){}", ev.pid, ev.comm, ev.uid, argv_info),
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
                                "OPEN",
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
                            )
                        }
                    };
                    self.push_log(LogLine {
                        style,
                        text: format!("{} {:>5} {}", ev.ts, tag, body),
                    });
                }
                Output::Alert(alert) => {
                    let line = LogLine {
                        style: Style::new()
                            .fg(NEON_RED)
                            .add_modifier(Modifier::BOLD),
                        text: format!(
                            "{} ⚠ ALERT [{}] {} — {} opens/s",
                            alert.ts, alert.pid, alert.comm, alert.opens
                        ),
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

    fn push_log(&mut self, line: LogLine) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(line);
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
    // Clear with dark background.
    frame.render_widget(Clear, frame.area());

    let area = frame.area();

    // Main layout: header (3 lines) | sparklines (3 lines) | body | footer
    let [header_area, spark_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, app, monitor, header_area);
    draw_sparklines(frame, monitor, spark_area);
    draw_body(frame, app, monitor, body_area);
    draw_footer(frame, footer_area);
}

// ── Header ────────────────────────────────────────────────────────────────

fn draw_header(frame: &mut Frame, app: &App, monitor: &Monitor, area: Rect) {
    let [logo_area, stats_area] =
        Layout::horizontal([Constraint::Length(34), Constraint::Min(0)]).areas(area);

    // Logo
    let logo_lines = vec![
        Line::from(vec![Span::styled(
            "  ⚡ HALCYON eBPF MONITOR",
            Style::new()
                .fg(CYAN)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "  kernel tracepoint · real-time",
            Style::new().fg(DIM),
        )]),
    ];
    frame.render_widget(Paragraph::new(logo_lines), logo_area);

    // Stats bar
    let status_color = if app.paused { NEON_YELLOW } else { NEON_GREEN };
    let status_text = if app.paused { "▐ PAUSED " } else { "▐ RUN " };

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

// ── Sparklines (event rate) ───────────────────────────────────────────────

fn draw_sparklines(frame: &mut Frame, monitor: &Monitor, area: Rect) {
    let [exec_area, open_area, alert_area] = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(40),
        Constraint::Percentage(20),
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

// ── Body: 3-column layout ─────────────────────────────────────────────────

fn draw_body(frame: &mut Frame, app: &App, monitor: &Monitor, area: Rect) {
    let [left, middle, right] = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(30),
        Constraint::Percentage(30),
    ])
    .areas(area);

    let [middle_top, middle_bottom] =
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)])
            .areas(middle);

    let [right_top, right_bottom] =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(right);

    draw_log(frame, app, left);
    draw_process_tree(frame, app, monitor, middle_top);
    draw_extensions(frame, monitor, middle_bottom);
    draw_top_files(frame, monitor, right_top);
    draw_alerts(frame, app, right_bottom);
}

// ── Event log ─────────────────────────────────────────────────────────────

fn draw_log(frame: &mut Frame, app: &App, area: Rect) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let total = app.log.len();
    let max_scroll = total.saturating_sub(inner_height);
    let scroll = app.scroll.min(max_scroll);
    let start = total.saturating_sub(inner_height.saturating_add(scroll));

    let lines: Vec<Line> = app
        .log
        .iter()
        .skip(start)
        .take(inner_height)
        .map(|l| Line::from(vec![Span::styled(l.text.clone(), l.style)]))
        .collect();

    let focus_border = if app.focus == 0 { CYAN } else { PANEL_BORDER };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(cyber_title(&format!(
            " EVENTS ({}) ",
            total
        )))
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
            let prefix = "├─ ";
            let node_style = if node.alerts > 0 {
                Style::new().fg(NEON_RED).add_modifier(Modifier::BOLD)
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
                format!(" ({})", node.total_opens)
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
                Span::styled(alert_badge, Style::new().fg(NEON_RED).add_modifier(Modifier::BOLD)),
            ])
        })
        .collect();

    let focus_border = if app.focus == 1 { CYAN } else { PANEL_BORDER };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(cyber_title(&format!(
            " PROCESS TREE ({}) ",
            flat.len()
        )))
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
    exts.truncate(6);

    let max_ext = exts.iter().map(|e| e.1).max().unwrap_or(1).max(1);

    let lines: Vec<Line> = exts
        .iter()
        .map(|(ext, count)| {
            let bar_width = (count * 20 / max_ext) as usize;
            let bar = "░".repeat(20 - bar_width) + &"█".repeat(bar_width);
            let ext_color = match ext.as_str() {
                "pdf" | "doc" | "docx" | "xls" | "xlsx" => NEON_YELLOW,
                "zip" | "tar" | "gz" | "7z" | "rar" => MAGENTA,
                "enc" | "locked" | "crypt" | "cipher" => NEON_RED,
                "rs" | "py" | "js" | "ts" | "go" | "c" | "cpp" => NEON_GREEN,
                _ => CYAN,
            };
            Line::from(vec![
                Span::styled(
                    format!(" .{:<6}", ext),
                    Style::new().fg(ext_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {bar}"),
                    Style::new().fg(DIM_BRIGHT),
                ),
                Span::styled(
                    format!(" {:>5}", count),
                    Style::new().fg(Color::White),
                ),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(cyber_title(" FILE TYPES "))
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
            } else {
                Style::new().fg(DIM_BRIGHT)
            };
            let ext_color = match f.extension.as_str() {
                "pdf" | "doc" | "docx" => NEON_YELLOW,
                "enc" | "locked" | "crypt" => NEON_RED,
                _ => CYAN,
            };
            let entropy_color = if f.entropy > 0.7 {
                NEON_RED
            } else if f.entropy > 0.5 {
                NEON_YELLOW
            } else {
                DIM_BRIGHT
            };

            // Truncate path for display.
            let path_display = if f.path.len() > 18 {
                format!("…{}", &f.path[f.path.len() - 15..])
            } else {
                f.path.clone()
            };

            Row::new(vec![
                Cell::from(format!("#{}", i + 1)).style(rank_style),
                Cell::from(path_display).style(Style::new().fg(Color::White)),
                Cell::from(format!(".{}", f.extension)).style(
                    Style::new().fg(ext_color).add_modifier(Modifier::BOLD),
                ),
                Cell::from(f.count.to_string()).style(Style::new().fg(NEON_GREEN)),
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
        Constraint::Length(6),
    ];

    let header = Row::new(vec![" #", "FILE", "EXT", "OPENS", "ENTR"])
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
                .title(cyber_title(" TOP FILES "))
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
            Span::styled("system clean", Style::new().fg(DIM)),
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
            format!(" ⚠ ALERTS ({alert_count}) "),
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

// ── Footer ────────────────────────────────────────────────────────────────

fn draw_footer(frame: &mut Frame, area: Rect) {
    let footer_line = Line::from(vec![
        Span::styled(" q ", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        Span::raw("quit"),
        Span::styled(" │ ", Style::new().fg(DIM)),
        Span::styled("p ", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        Span::raw("pause"),
        Span::styled(" │ ", Style::new().fg(DIM)),
        Span::styled("c ", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        Span::raw("clear"),
        Span::styled(" │ ", Style::new().fg(DIM)),
        Span::styled("↑↓ ", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        Span::raw("scroll"),
        Span::styled(" │ ", Style::new().fg(DIM)),
        Span::styled("Tab ", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        Span::raw("panel"),
        Span::styled(" │ ", Style::new().fg(DIM)),
        Span::styled("Ctrl+C ", Style::new().fg(NEON_YELLOW).add_modifier(Modifier::BOLD)),
        Span::raw("quit"),
    ]);
    frame.render_widget(
        Paragraph::new(footer_line).style(Style::new().fg(DIM_BRIGHT)),
        area,
    );
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
            }),
            Output::Alert(Alert {
                ts: "00:00:00.002".into(),
                pid: 2,
                uid: 1000,
                comm: "sh".into(),
                opens: 3,
            }),
        ]);

        let backend = ratatui::backend::TestBackend::new(120, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app, &monitor)).unwrap();
        terminal.draw(|f| draw(f, &app, &monitor)).unwrap();
    }

    #[test]
    fn key_handling() {
        let mut app = App::new();
        app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        assert!(app.paused);
        assert!(app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn tab_cycles_focus() {
        let mut app = App::new();
        assert_eq!(app.focus, 0);
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus, 1);
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus, 2);
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus, 0);
    }

    #[test]
    fn monitor_tracks_alerts() {
        let mut monitor = Monitor::dummy();
        let mut outputs = Vec::new();
        for _ in 0..3 {
            monitor.handle_event(
                &RecordedEvent {
                    ts: "00:00:00.000".into(),
                    kind: Kind::Open,
                    pid: 42,
                    uid: 1000,
                    comm: "probe".into(),
                    file: Some("/tmp/x".into()),
                    extension: None,
                    argv: None,
                },
                &mut outputs,
            );
        }
        let alerts = outputs
            .iter()
            .filter(|o| matches!(o, Output::Alert(_)))
            .count();
        assert_eq!(alerts, 1, "exactly one alert at the threshold");
        assert_eq!(monitor.stats_sorted()[0].alerts, 1);
    }

    #[test]
    fn format_number_scales() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1500), "1.5K");
        assert_eq!(format_number(1_500_000), "1.5M");
    }
}
