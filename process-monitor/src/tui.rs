//! Halcyon Process Monitor — FrankenTUI-based TUI layer
//!
//! Uses the Elm/Bubbletea architecture: Model → update → view → BufferDiff → ANSI.
//! FrankenTUI provides diff-based rendering (zero flicker), RAII cleanup,
//! and 80+ widgets including sparklines, trees, tables, and log viewers.

use std::collections::VecDeque;
use std::time::Duration;

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_layout::{Constraint, Flex};
use ftui_render::cell::PackedRgba;
use ftui_render::frame::Frame;
use ftui_runtime::{App, Cmd, Every, Model, ScreenMode, Subscription};
use ftui_style::{Style, StyleFlags};
use ftui_text::Line;
use ftui_text::Span;
use ftui_widgets::block::Block;
use ftui_widgets::borders::{BorderType, Borders};
use ftui_widgets::log_viewer::{LogViewer, LogViewerState};
use ftui_widgets::paragraph::Paragraph;
use ftui_widgets::status_line::{StatusItem, StatusLine};
use ftui_widgets::{StatefulWidget, Widget};

use crate::monitor::Monitor;

// ── Modern dark palette (2026) ────────────────────────────────────────────

const ACCENT_BLUE: PackedRgba = ftui_render::cell::PackedRgba::rgb(88, 166, 255);
const ACCENT_GREEN: PackedRgba = ftui_render::cell::PackedRgba::rgb(63, 185, 80);
const ACCENT_RED: PackedRgba = ftui_render::cell::PackedRgba::rgb(248, 81, 73);
const ACCENT_AMBER: PackedRgba = ftui_render::cell::PackedRgba::rgb(210, 153, 34);
const ACCENT_PURPLE: PackedRgba = ftui_render::cell::PackedRgba::rgb(188, 140, 255);
const TEXT_DIM: PackedRgba = ftui_render::cell::PackedRgba::rgb(76, 82, 99);
const TEXT_BRIGHT: PackedRgba = ftui_render::cell::PackedRgba::rgb(139, 148, 168);
const BORDER_SUBTLE: PackedRgba = ftui_render::cell::PackedRgba::rgb(48, 55, 73);
const BORDER_ACTIVE: PackedRgba = ftui_render::cell::PackedRgba::rgb(88, 166, 255);

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
}

// ── Elm-style Message type ────────────────────────────────────────────────

#[derive(Debug)]
enum Msg {
    Key(KeyEvent),
    Tick,
    Noop,
}

impl From<Event> for Msg {
    fn from(e: Event) -> Self {
        match e {
            Event::Key(k) => Msg::Key(k),
            _ => Msg::Noop,
        }
    }
}

// ── Application state ─────────────────────────────────────────────────────

struct HalcyonApp {
    log: LogViewer,
    log_state: LogViewerState,
    alerts: LogViewer,
    alerts_state: LogViewerState,
    network: VecDeque<NetworkEntry>,
    focused: usize,
    paused: bool,
    help_visible: bool,
    events_per_sec: f64,
    opens_per_sec: f64,
    tick_count: u64,
}

#[derive(Clone)]
struct NetworkEntry {
    ts: String,
    pid: u32,
    comm: String,
    kind: String,
    addr: String,
}

impl HalcyonApp {
    fn new() -> Self {
        let mut log = LogViewer::new(5000);
        log.push("⚡ Halcyon eBPF Monitor started");
        log.push("  FrankenTUI diff-based renderer · zero flicker");
        log.push("  Press ? for help");

        let alerts = LogViewer::new(200);

        Self {
            log,
            log_state: LogViewerState::default(),
            alerts,
            alerts_state: LogViewerState::default(),
            network: VecDeque::new(),
            focused: 0,
            paused: false,
            help_visible: false,
            events_per_sec: 0.0,
            opens_per_sec: 0.0,
            tick_count: 0,
        }
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
                        self.focused = (self.focused + 1) % Panel::all().len();
                    }
                    KeyCode::BackTab => {
                        self.focused = if self.focused == 0 {
                            Panel::all().len() - 1
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
            Msg::Tick if !self.paused => {
                self.tick_count += 1;
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
                Constraint::Fixed(2),
                Constraint::Fixed(1),
                Constraint::Min(0),
                Constraint::Fixed(1),
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
            Span::styled("eBPF · PROCESS MONITOR", Style::new().fg(TEXT_DIM)),
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

        let text = Paragraph::new(Line::from_spans(vec![Span::styled(
            "  Process tree — populated from monitor",
            Style::new().fg(TEXT_DIM),
        )]));

        let inner = block.inner(area);
        block.render(area, frame);
        text.render(inner, frame);
    }

    fn render_extensions_panel(&self, frame: &mut Frame, area: Rect) {
        let block = Block::new()
            .title(" FILE TYPES ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(BORDER_SUBTLE));

        let text = Paragraph::new(Line::from_spans(vec![Span::styled(
            "  Extension frequency — populated from monitor",
            Style::new().fg(TEXT_DIM),
        )]));

        let inner = block.inner(area);
        block.render(area, frame);
        text.render(inner, frame);
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

        let text = Paragraph::new(Line::from_spans(vec![Span::styled(
            "  Top accessed files — populated from monitor",
            Style::new().fg(TEXT_DIM),
        )]));

        let inner = block.inner(area);
        block.render(area, frame);
        text.render(inner, frame);
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
        let panel_name = Panel::all()[self.focused].name();
        let rate_str = format!(
            "evt/s: {:.0}  open/s: {:.0}",
            self.events_per_sec, self.opens_per_sec
        );
        let status = StatusLine::new()
            .left(StatusItem::text(panel_name))
            .center(StatusItem::text(&rate_str))
            .right(StatusItem::key_hint("?", "Help"));
        status.render(area, frame);
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
            Line::raw(""),
            Line::from_spans(vec![Span::styled(
                "  Actions",
                Style::new().fg(ACCENT_AMBER).attrs(StyleFlags::BOLD),
            )]),
            Line::raw("  p                 Pause/Resume"),
            Line::raw("  q / Esc           Quit"),
            Line::raw("  Ctrl+C            Force quit"),
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
pub fn run(_monitor: &mut Monitor) -> anyhow::Result<()> {
    let app = HalcyonApp::new();

    App::new(app).screen_mode(ScreenMode::AltScreen).run()?;

    Ok(())
}
