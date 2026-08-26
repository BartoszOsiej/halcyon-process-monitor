//! Halcyon Process Monitor — FrankenTUI layer (clean professional build)
//!
//! Elm architecture. No emoji. No gradients. Dense data, clean alignment.

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

// ── Palette ───────────────────────────────────────────────────────────────

const BLUE: PackedRgba = PackedRgba::rgb(88, 166, 255);
const GREEN: PackedRgba = PackedRgba::rgb(63, 185, 80);
const RED: PackedRgba = PackedRgba::rgb(248, 81, 73);
const AMBER: PackedRgba = PackedRgba::rgb(210, 153, 34);
const PURPLE: PackedRgba = PackedRgba::rgb(188, 140, 255);
const CYAN: PackedRgba = PackedRgba::rgb(56, 189, 248);
const DIM: PackedRgba = PackedRgba::rgb(80, 90, 110);
const FG: PackedRgba = PackedRgba::rgb(180, 190, 210);
const BRIGHT: PackedRgba = PackedRgba::rgb(220, 230, 245);
const BORDER: PackedRgba = PackedRgba::rgb(50, 56, 72);
const BORDER_HI: PackedRgba = PackedRgba::rgb(88, 166, 255);

// ── Panels ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel { Events, Processes, Network, TopFiles, Extensions, Alerts }

impl Panel {
    const ALL: [Panel; 6] = [
        Panel::Events, Panel::Processes, Panel::Network,
        Panel::TopFiles, Panel::Extensions, Panel::Alerts,
    ];
    fn name(&self) -> &'static str {
        match self { Self::Events=>"EVENTS", Self::Processes=>"PROCESSES", Self::Network=>"NETWORK",
            Self::TopFiles=>"TOP FILES", Self::Extensions=>"FILE TYPES", Self::Alerts=>"ALERTS" }
    }
}

// ── Messages ──────────────────────────────────────────────────────────────

#[derive(Debug)]
enum Msg { Key(KeyEvent), Tick, Noop }

impl From<Event> for Msg {
    fn from(e: Event) -> Self { match e { Event::Key(k)=>Msg::Key(k), _=>Msg::Noop } }
}

// ── Snapshot from monitor thread ──────────────────────────────────────────

struct Snapshot {
    events: Vec<Evt>,
    stats: Vec<ProcStats>,
    top_files: Vec<FileRank>,
    exts: std::collections::HashMap<String, u64>,
    rates: VecDeque<RateSample>,
    total: u64,
    lost: u64,
    uptime: u64,
}

#[derive(Clone)]
struct Evt { ts: String, kind: String, pid: u32, comm: String, file: Option<String>, is_alert: bool, opens: u64 }

// ── Network entry ─────────────────────────────────────────────────────────

#[derive(Clone)]
struct NetEntry { ts: String, pid: u32, comm: String, kind: String, addr: String }

// ── State ─────────────────────────────────────────────────────────────────

struct App_ {
    log: LogViewer, log_st: LogViewerState,
    alerts: LogViewer, alert_st: LogViewerState,
    net: VecDeque<NetEntry>,
    procs: Vec<ProcRow>,
    files: Vec<FileRow>,
    exts: Vec<(String, u64)>,
    rates: VecDeque<RateSample>,
    focused: usize,
    paused: bool,
    help: bool,
    total: u64, lost: u64, uptime: u64, alert_count: u64,
    rx: mpsc::Receiver<Snapshot>,
}

struct ProcRow { pid: u32, comm: String, opens: u64, alerts: u64 }
struct FileRow { name: String, count: u64, ext: String, entropy: f64 }

impl App_ {
    fn new(rx: mpsc::Receiver<Snapshot>) -> Self {
        let mut log = LogViewer::new(5000);
        log.push("[halcyon] eBPF monitor started");
        log.push("[halcyon] press ? for help");
        Self {
            log, log_st: LogViewerState::default(),
            alerts: LogViewer::new(200), alert_st: LogViewerState::default(),
            net: VecDeque::new(), procs: Vec::new(), files: Vec::new(), exts: Vec::new(),
            rates: VecDeque::new(), focused: 0, paused: false, help: false,
            total: 0, lost: 0, uptime: 0, alert_count: 0, rx,
        }
    }

    fn apply_snapshot(&mut self, s: Snapshot) {
        for e in &s.events {
            let kind_tag = match e.kind.as_str() {
                "Exec" => "EXEC  ",
                "Open" => "OPEN  ",
                "Connect"|"Accept"|"SendTo"|"RecvFrom" => "NET   ",
                "Mkdir" => "MKDIR ",
                "Unlink" => "UNLINK",
                "Kill" => "KILL  ",
                "Chmod" => "CHMOD ",
                _ => "EVENT ",
            };
            let file_part = e.file.as_deref().unwrap_or("");
            let line = if e.is_alert {
                format!("{} *** ALERT  [{}] {} opened {} files/s", e.ts, e.pid, e.comm, e.opens)
            } else {
                format!("{} {} [{:>6}] {:<16} {}", e.ts, kind_tag, e.pid, e.comm, file_part)
            };
            self.log.push(line.as_str());

            if matches!(e.kind.as_str(), "Connect"|"Accept"|"SendTo"|"RecvFrom") {
                self.net.push_front(NetEntry {
                    ts: e.ts.clone(), pid: e.pid, comm: e.comm.clone(),
                    kind: e.kind.clone(), addr: file_part.to_string(),
                });
                while self.net.len() > 200 { self.net.pop_back(); }
            }
            self.total += 1;
            if e.is_alert {
                self.alert_count += 1;
                self.alerts.push(line.as_str());
            }
        }

        self.procs = s.stats.iter().map(|p| ProcRow {
            pid: p.pid, comm: p.comm.clone(), opens: p.window_opens, alerts: p.alerts,
        }).collect();
        self.procs.sort_by(|a,b| b.opens.cmp(&a.opens));

        self.files = s.top_files.iter().map(|f| FileRow {
            name: f.path.rsplit('/').next().unwrap_or(&f.path).to_string(),
            count: f.count, ext: f.extension.clone(), entropy: f.entropy,
        }).collect();

        let mut ext_vec: Vec<(String,u64)> = s.exts.iter().map(|(k,v)| (k.clone(),*v)).collect();
        ext_vec.sort_by(|a,b| b.1.cmp(&a.1));
        self.exts = ext_vec;

        self.rates = s.rates;
        self.total = s.total;
        self.lost = s.lost;
        self.uptime = s.uptime;
    }
}

impl Model for App_ {
    type Message = Msg;

    fn init(&mut self) -> Cmd<Msg> { Cmd::None }

    fn update(&mut self, msg: Msg) -> Cmd<Msg> {
        match msg {
            Msg::Key(k) if k.kind == KeyEventKind::Press => {
                if self.help { self.help = false; return Cmd::None; }
                if k.modifiers.contains(Modifiers::CTRL) && k.code == KeyCode::Char('c') { return Cmd::Quit; }
                match k.code {
                    KeyCode::Char('q')|KeyCode::Escape => { if self.focused==0 { return Cmd::Quit; } }
                    KeyCode::Char('?') => self.help = true,
                    KeyCode::Char('p') => self.paused = !self.paused,
                    KeyCode::Tab => self.focused = (self.focused+1) % 6,
                    KeyCode::BackTab => self.focused = if self.focused==0 {5} else {self.focused-1},
                    KeyCode::Char(c @ '1'..='6') => {
                        self.focused = (c as usize) - ('1' as usize);
                    }
                    KeyCode::Up|KeyCode::Char('k') => self.log.scroll_up(1),
                    KeyCode::Down|KeyCode::Char('j') => self.log.scroll_down(1),
                    KeyCode::PageUp => self.log.page_up(&self.log_st),
                    KeyCode::PageDown => self.log.page_down(&self.log_st),
                    KeyCode::Home => self.log.scroll_to_top(),
                    KeyCode::End => self.log.scroll_to_bottom(),
                    _ => {}
                }
            }
            Msg::Tick if !self.paused => {
                while let Ok(s) = self.rx.try_recv() { self.apply_snapshot(s); }
            }
            _ => {}
        }
        Cmd::None
    }

    fn view(&self, frame: &mut Frame) {
        let area = Rect::from_size(frame.buffer.width(), frame.buffer.height());
        if self.help { return self.draw_help(frame, area); }

        let outer = Flex::vertical().constraints([
            Constraint::Fixed(1), // header
            Constraint::Min(0),  // body
            Constraint::Fixed(1), // status
        ]).split(area);

        self.draw_header(frame, outer[0]);
        self.draw_body(frame, outer[1]);
        self.draw_status(frame, outer[2]);
    }

    fn subscriptions(&self) -> Vec<Box<dyn Subscription<Msg>>> {
        vec![Box::new(Every::new(Duration::from_millis(50), || Msg::Tick))]
    }
}

// ── Drawing ───────────────────────────────────────────────────────────────

impl App_ {
    fn border(&self, idx: usize) -> PackedRgba { if self.focused==idx { BORDER_HI } else { BORDER } }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        let up = fmt_dur(self.uptime);
        let line = Line::from_spans(vec![
            Span::styled(" halcyon ", Style::new().fg(BLUE).attrs(StyleFlags::BOLD)),
            Span::styled(format!("{} ", self.border_char()), Style::new().fg(DIM)),
            Span::styled("eBPF process monitor", Style::new().fg(DIM)),
            Span::styled(format!("  {} ", self.border_char()), Style::new().fg(DIM)),
            Span::styled(format!("{} events", self.total), Style::new().fg(FG)),
            Span::styled(format!("  {} ", self.border_char()), Style::new().fg(DIM)),
            Span::styled(format!("{} lost", self.lost), Style::new().fg(if self.lost>0 {RED} else {DIM})),
            Span::styled(format!("  {} ", self.border_char()), Style::new().fg(DIM)),
            Span::styled(format!("up {}", up), Style::new().fg(FG)),
            Span::styled(format!("  {} ", self.border_char()), Style::new().fg(DIM)),
            Span::styled(ts, Style::new().fg(DIM)),
        ]);
        Paragraph::new(line).render(area, f);
    }

    fn draw_body(&self, f: &mut Frame, area: Rect) {
        let cols = Flex::horizontal().constraints([
            Constraint::Percentage(35.0), Constraint::Percentage(35.0), Constraint::Percentage(30.0),
        ]).split(area);

        let mid = Flex::vertical().constraints([
            Constraint::Percentage(55.0), Constraint::Percentage(45.0),
        ]).split(cols[1]);

        let right = Flex::vertical().constraints([
            Constraint::Percentage(35.0), Constraint::Percentage(35.0), Constraint::Percentage(30.0),
        ]).split(cols[2]);

        self.draw_events(f, cols[0]);
        self.draw_procs(f, mid[0]);
        self.draw_exts(f, mid[1]);
        self.draw_net(f, right[0]);
        self.draw_files(f, right[1]);
        self.draw_alerts(f, right[2]);
    }

    fn panel_block<'a>(&self, idx: usize, title: &'a str) -> Block<'a> {
        Block::new().title(title)
            .borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::new().fg(self.border(idx)))
    }

    fn waiting(&self) -> Paragraph {
        Paragraph::new(Line::from_spans(vec![Span::styled("  --", Style::new().fg(DIM))]))
    }

    fn draw_events(&self, f: &mut Frame, area: Rect) {
        let b = self.panel_block(0, " EVENTS ");
        let inner = b.inner(area); b.render(area, f);
        let mut st = self.log_st.clone();
        self.log.render(inner, f, &mut st);
    }

    fn draw_procs(&self, f: &mut Frame, area: Rect) {
        let title = format!(" PROCESSES ({}) ", self.procs.len());
        let b = self.panel_block(1, &title);
        let inner = b.inner(area); b.render(area, f);
        if self.procs.is_empty() { self.waiting().render(inner, f); return; }

        let max = self.procs.iter().map(|p| p.opens).max().unwrap_or(1).max(1);
        let bar_w = ((inner.width as f64 - 34.0) * 0.5) as usize;
        let rows = inner.height as usize;

        let lines: Vec<Line> = self.procs.iter().take(rows).map(|p| {
            let ratio = p.opens as f64 / max as f64;
            let filled = (ratio * bar_w as f64) as usize;
            let bar: String = "#".repeat(filled.min(bar_w));
            let pad: String = " ".repeat(bar_w.saturating_sub(filled));

            let (pid_c, bar_c) = if p.alerts > 0 { (RED, RED) } else { (DIM, BLUE) };
            let comm_c = if p.alerts > 0 { RED } else { BRIGHT };

            Line::from_spans(vec![
                Span::styled(format!(" {:>5} ", p.pid), Style::new().fg(pid_c)),
                Span::styled(format!("{:<16}", trunc(&p.comm, 16)), Style::new().fg(comm_c)),
                Span::styled(format!("{:>5} ", p.opens), Style::new().fg(CYAN)),
                Span::styled(bar.to_string(), Style::new().fg(bar_c)),
                Span::styled(pad.to_string(), Style::new().fg(DIM)),
                Span::styled(format!(" {:>3}", p.alerts), Style::new().fg(if p.alerts>0 {RED} else {DIM})),
            ])
        }).collect();
        Paragraph::new(ftui_text::Text::from_lines(lines)).render(inner, f);
    }

    fn draw_exts(&self, f: &mut Frame, area: Rect) {
        let b = self.panel_block(4, " FILE TYPES ");
        let inner = b.inner(area); b.render(area, f);
        if self.exts.is_empty() { self.waiting().render(inner, f); return; }

        let max = self.exts.iter().map(|(_,c)| *c).max().unwrap_or(1);
        let bar_w = ((inner.width as f64 - 22.0) * 0.5) as usize;
        let rows = inner.height as usize;

        let lines: Vec<Line> = self.exts.iter().take(rows).map(|(ext, cnt)| {
            let ratio = *cnt as f64 / max as f64;
            let filled = (ratio * bar_w as f64) as usize;
            let bar: String = "#".repeat(filled.min(bar_w));
            let pad: String = " ".repeat(bar_w.saturating_sub(filled));
            let c = ext_color(ext);

            Line::from_spans(vec![
                Span::styled(format!(" .{:<10}", ext), Style::new().fg(c)),
                Span::styled(format!("{:>6} ", cnt), Style::new().fg(FG)),
                Span::styled(bar.to_string(), Style::new().fg(c)),
                Span::styled(pad.to_string(), Style::new().fg(DIM)),
            ])
        }).collect();
        Paragraph::new(ftui_text::Text::from_lines(lines)).render(inner, f);
    }

    fn draw_net(&self, f: &mut Frame, area: Rect) {
        let title = format!(" NETWORK ({}) ", self.net.len());
        let b = self.panel_block(2, &title);
        let inner = b.inner(area); b.render(area, f);
        if self.net.is_empty() { self.waiting().render(inner, f); return; }

        let rows = inner.height as usize;
        let lines: Vec<Line> = self.net.iter().rev().take(rows).map(|e| {
            let (arrow, c) = match e.kind.as_str() {
                "Connect" => (">", BLUE), "Accept" => ("<", GREEN),
                "SendTo" => (">", AMBER), "RecvFrom" => ("<", PURPLE),
                _ => ("?", DIM),
            };
            Line::from_spans(vec![
                Span::styled(format!("{} ", arrow), Style::new().fg(c).attrs(StyleFlags::BOLD)),
                Span::styled(format!("{:<8}", e.kind), Style::new().fg(c)),
                Span::styled(format!("{:>6} ", e.pid), Style::new().fg(DIM)),
                Span::styled(format!("{:<12}", trunc(&e.comm, 12)), Style::new().fg(FG)),
                Span::styled(format!(" {}", e.addr), Style::new().fg(PURPLE)),
            ])
        }).collect();
        Paragraph::new(ftui_text::Text::from_lines(lines)).render(inner, f);
    }

    fn draw_files(&self, f: &mut Frame, area: Rect) {
        let b = self.panel_block(3, " TOP FILES ");
        let inner = b.inner(area); b.render(area, f);
        if self.files.is_empty() { self.waiting().render(inner, f); return; }

        let max = self.files.iter().map(|r| r.count).max().unwrap_or(1);
        let bar_w = ((inner.width as f64 - 32.0) * 0.5) as usize;
        let rows = inner.height as usize;

        let lines: Vec<Line> = self.files.iter().take(rows).enumerate().map(|(i, r)| {
            let ratio = r.count as f64 / max as f64;
            let filled = (ratio * bar_w as f64) as usize;
            let bar: String = "#".repeat(filled.min(bar_w));
            let pad: String = " ".repeat(bar_w.saturating_sub(filled));

            let ecol = if r.entropy > 0.7 { RED } else if r.entropy > 0.4 { AMBER } else { GREEN };
            let rank_c = if i < 3 { AMBER } else { DIM };

            Line::from_spans(vec![
                Span::styled(format!(" {:>2}.", i+1), Style::new().fg(rank_c)),
                Span::styled(format!(" {:<16}", trunc(&r.name, 16)), Style::new().fg(BRIGHT)),
                Span::styled(format!(" {:>5}", r.count), Style::new().fg(CYAN)),
                Span::styled(bar.to_string(), Style::new().fg(BLUE)),
                Span::styled(pad.to_string(), Style::new().fg(DIM)),
                Span::styled(format!(" .{:<6}", r.ext), Style::new().fg(ext_color(&r.ext))),
                Span::styled(format!(" {:.1}", r.entropy), Style::new().fg(ecol)),
            ])
        }).collect();
        Paragraph::new(ftui_text::Text::from_lines(lines)).render(inner, f);
    }

    fn draw_alerts(&self, f: &mut Frame, area: Rect) {
        let title = format!(" ALERTS ({}) ", self.alert_count);
        let b = self.panel_block(5, &title);
        let inner = b.inner(area); b.render(area, f);
        if self.alert_count == 0 {
            Paragraph::new(Line::from_spans(vec![
                Span::styled("  ok", Style::new().fg(GREEN)),
            ])).render(inner, f);
        } else {
            let mut st = self.alert_st.clone();
            self.alerts.render(inner, f, &mut st);
        }
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let p = Panel::ALL[self.focused];
        let left = format!(" {} ", p.name());
        let center = format!("evt/s:{}  lost:{}  up:{}", self.total, self.lost, fmt_dur(self.uptime));
        let status = StatusLine::new()
            .left(StatusItem::text(&left))
            .center(StatusItem::text(&center))
            .right(StatusItem::key_hint("?", "help"));
        status.render(area, f);
    }

    fn draw_help(&self, f: &mut Frame, area: Rect) {
        let lines = vec![
            Line::from_spans(vec![Span::styled(" halcyon — keyboard shortcuts", Style::new().fg(BLUE).attrs(StyleFlags::BOLD))]),
            Line::raw(""),
            Line::from_spans(vec![Span::styled(" NAVIGATION", Style::new().fg(AMBER).attrs(StyleFlags::BOLD))]),
            Line::raw("   Tab/Shift+Tab  cycle panels"),
            Line::raw("   1-6            jump to panel"),
            Line::raw("   j/k or arrows  scroll"),
            Line::raw("   PgUp/PgDn      page scroll"),
            Line::raw("   Home/End       top/bottom"),
            Line::raw(""),
            Line::from_spans(vec![Span::styled(" ACTIONS", Style::new().fg(AMBER).attrs(StyleFlags::BOLD))]),
            Line::raw("   p              pause/resume"),
            Line::raw("   q/Esc          quit"),
            Line::raw("   Ctrl+C         force quit"),
        ];
        let b = Block::new().title(" help ").borders(Borders::ALL)
            .border_type(BorderType::Rounded).border_style(Style::new().fg(BLUE));
        let inner = b.inner(area); b.render(area, f);
        Paragraph::new(ftui_text::Text::from_lines(lines)).render(inner, f);
    }

    fn border_char(&self) -> &'static str { "|" }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn trunc(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}~", &s[..max-1]) }
}

fn fmt_dur(s: u64) -> String {
    if s < 60 { format!("{}s", s) }
    else if s < 3600 { format!("{}m{}s", s/60, s%60) }
    else { format!("{}h{}m", s/3600, (s%3600)/60) }
}

fn ext_color(ext: &str) -> PackedRgba {
    match ext {
        "enc"|"locked"|"crypto" => RED,
        "rs"|"py"|"js"|"ts"|"c"|"cpp"|"go"|"java" => GREEN,
        "pdf"|"doc"|"docx"|"txt"|"md" => AMBER,
        "jpg"|"png"|"mp4"|"mp3" => PURPLE,
        "json"|"toml"|"yaml"|"yml" => CYAN,
        _ => BLUE,
    }
}

// ── Public API ────────────────────────────────────────────────────────────

pub fn run(mut monitor: Monitor) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<Snapshot>();

    let mut tick: u64 = 0;
    let h = thread::Builder::new().name("poll".into()).spawn(move || loop {
        let evts: Vec<Evt> = monitor.poll().into_iter().filter_map(|o| match o {
            Output::Event(ev) => Some(Evt {
                ts: ev.ts, kind: format!("{:?}", ev.kind), pid: ev.pid,
                comm: ev.comm, file: ev.file, is_alert: false, opens: 0,
            }),
            Output::Alert(a) => Some(Evt {
                ts: a.ts, kind: "Alert".into(), pid: a.pid,
                comm: a.comm, file: None, is_alert: true, opens: a.opens,
            }),
        }).collect();

        tick += 1;
        if tick % 6 == 0 || !evts.is_empty() {
            let snap = Snapshot {
                events: evts, stats: monitor.stats_sorted(),
                top_files: monitor.top_files(20),
                exts: monitor.extension_counts().clone(),
                rates: monitor.rate_history().clone(),
                total: monitor.total_events, lost: monitor.total_lost,
                uptime: monitor.uptime().as_secs(),
            };
            if tx.send(snap).is_err() { break; }
        }
        thread::sleep(Duration::from_millis(16));
    })?;

    let app = App_::new(rx);
    let res = App::new(app).screen_mode(ScreenMode::AltScreen).run();
    let _ = h.join();
    res.map_err(|e| anyhow::anyhow!("tui: {}", e))
}
