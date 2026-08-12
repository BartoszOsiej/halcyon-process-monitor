use std::collections::VecDeque;
use std::io;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::monitor::{Kind, Monitor, Output};

const LOG_CAP: usize = 2000;
const ALERT_CAP: usize = 30;
const MAX_ROWS: usize = 15;
const TICK_MS: u64 = 100;

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
}

impl App {
    fn new() -> Self {
        Self {
            log: VecDeque::new(),
            alerts: VecDeque::new(),
            scroll: 0,
            paused: false,
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
            _ => false,
        }
    }

    fn on_events(&mut self, outputs: Vec<Output>) {
        for output in outputs {
            match output {
                Output::Event(ev) => {
                    let (style, tag, body) = match ev.kind {
                        Kind::Exec => (
                            Style::new().fg(Color::Green),
                            "EXEC",
                            format!("[{}] {} (uid {})", ev.pid, ev.comm, ev.uid),
                        ),
                        Kind::Open => (
                            Style::new().fg(Color::Blue),
                            "OPEN",
                            match ev.file {
                                Some(file) => format!("[{}] {} -> {}", ev.pid, ev.comm, file),
                                None => format!("[{}] {}", ev.pid, ev.comm),
                            },
                        ),
                    };
                    self.push_log(LogLine {
                        style,
                        text: format!("{} {:>5} {}", ev.ts, tag, body),
                    });
                }
                Output::Alert(alert) => {
                    let line = LogLine {
                        style: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
                        text: format!(
                            "{}  ALERT [{}] {} opened {} files in 1s!",
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

fn draw(frame: &mut Frame, app: &App, monitor: &Monitor) {
    let area = frame.area();
    let [title, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);

    let status = if app.paused {
        Span::styled(
            " PAUSED ",
            Style::new()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " RUNNING ",
            Style::new()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    };

    let title_line = Line::from(vec![
        Span::styled(
            "Halcyon Process Monitor",
            Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        status,
        Span::styled(
            format!(
                " events {} | lost {} | uptime {:.0}s | threshold {} opens/s ",
                monitor.total_events,
                monitor.total_lost,
                monitor.uptime().as_secs_f32(),
                monitor.threshold,
            ),
            Style::new().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(title_line), title);

    let [left, right] = Layout::horizontal([
        Constraint::Percentage(62),
        Constraint::Percentage(38),
    ])
    .areas(body);
    let [stats_area, alerts_area] = Layout::vertical([
        Constraint::Percentage(55),
        Constraint::Percentage(45),
    ])
    .areas(right);

    draw_log(frame, app, left);
    draw_stats(frame, monitor, stats_area);
    draw_alerts(frame, app, alerts_area);

    let footer_line = Line::from(vec![
        Span::styled(" q ", Style::new().fg(Color::Yellow)),
        Span::raw("quit"),
        Span::styled(" p ", Style::new().fg(Color::Yellow)),
        Span::raw("pause"),
        Span::styled(" c ", Style::new().fg(Color::Yellow)),
        Span::raw("clear"),
        Span::styled(" \u{2191}/\u{2193} ", Style::new().fg(Color::Yellow)),
        Span::raw("scroll"),
        Span::raw("  |  Ctrl+C quits"),
    ]);
    frame.render_widget(
        Paragraph::new(footer_line).style(Style::new().fg(Color::DarkGray)),
        footer,
    );
}

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

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Events ",
            Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(Color::DarkGray));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_stats(frame: &mut Frame, monitor: &Monitor, area: Rect) {
    let rows: Vec<Row> = monitor
        .stats_sorted()
        .into_iter()
        .take(MAX_ROWS)
        .map(|s| {
            let hot = s.alerts > 0;
            let style = if hot {
                Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Color::Gray)
            };
            Row::new(vec![
                Cell::from(s.pid.to_string()),
                Cell::from(s.comm),
                Cell::from(s.window_opens.to_string()),
                Cell::from(s.total_opens.to_string()),
                Cell::from(s.alerts.to_string()).style(style),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Min(5),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(7),
    ];

    let header = Row::new(vec![
        Cell::from("PID"),
        Cell::from("COMM"),
        Cell::from("OPENS/s"),
        Cell::from("TOTAL"),
        Cell::from("ALERTS"),
    ])
    .style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " Top processes ",
                    Style::new()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::new().fg(Color::DarkGray)),
        );

    frame.render_widget(table, area);
}

fn draw_alerts(frame: &mut Frame, app: &App, area: Rect) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = if app.alerts.is_empty() {
        vec![Line::from(Span::styled(
            "No alerts yet",
            Style::new().fg(Color::DarkGray),
        ))]
    } else {
        app.alerts
            .iter()
            .rev()
            .take(inner_height)
            .map(|l| Line::from(vec![Span::styled(l.text.clone(), l.style)]))
            .collect()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Alerts ",
            Style::new()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(Color::DarkGray));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

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
            }),
            Output::Event(RecordedEvent {
                ts: "00:00:00.001".into(),
                kind: Kind::Open,
                pid: 2,
                uid: 1000,
                comm: "sh".into(),
                file: Some("/etc/passwd".into()),
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
}

