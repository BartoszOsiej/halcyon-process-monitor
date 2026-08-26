//! Halcyon Process Monitor — FrankenTUI dashboard
//!
//! Uses MiniBar, BarChart, LineChart, Canvas, Badge, Sparkline, heatmap_gradient,
//! and StyledText with color wave effects from ftui-extras.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ftui_core::event::{Event, KeyCode, KeyEvent, KeyEventKind, Modifiers};
use ftui_core::geometry::Rect;
use ftui_extras::canvas::{Canvas, Mode, Painter};
use ftui_extras::charts::{
    BarChart, BarDirection, BarGroup, BarMode, LineChart, Series, Sparkline, heatmap_gradient,
};
use ftui_extras::text_effects::{StyledText, TextEffect};
use ftui_layout::{Constraint, Flex};
use ftui_render::cell::{Cell as RenderCell, PackedRgba};
use ftui_render::frame::Frame;
use ftui_runtime::program::{App, Cmd, Model};
use ftui_runtime::terminal_writer::ScreenMode;
use ftui_runtime::{Every, Subscription};
use ftui_style::{Style, StyleFlags};
use ftui_text::Line;
use ftui_text::Span;
use ftui_widgets::badge::Badge;
use ftui_widgets::block::{Alignment, Block};
use ftui_widgets::borders::{BorderType, Borders};
use ftui_widgets::log_viewer::{LogViewer, LogViewerState};
use ftui_widgets::paragraph::Paragraph;
use ftui_widgets::progress::{MiniBar, MiniBarColors};
use ftui_widgets::status_line::{StatusItem, StatusLine};
use ftui_widgets::{StatefulWidget, Widget};

use crate::monitor::{FileRank, Monitor, Output, ProcStats, ProcessNode, RateSample};

// ── Palette — cohesive cyberpunk ─────────────────────────────────────────

const BLUE: PackedRgba = PackedRgba::rgb(88, 166, 255);
const TEAL: PackedRgba = PackedRgba::rgb(56, 189, 215);
const GREEN: PackedRgba = PackedRgba::rgb(80, 200, 120);
const RED: PackedRgba = PackedRgba::rgb(248, 81, 73);
const AMBER: PackedRgba = PackedRgba::rgb(250, 180, 50);
const PURPLE: PackedRgba = PackedRgba::rgb(160, 120, 240);
const PINK: PackedRgba = PackedRgba::rgb(240, 100, 170);
const CYAN: PackedRgba = PackedRgba::rgb(100, 210, 245);
const DIM: PackedRgba = PackedRgba::rgb(65, 72, 88);
const FG: PackedRgba = PackedRgba::rgb(170, 180, 200);
const BRIGHT: PackedRgba = PackedRgba::rgb(230, 235, 248);
const BORDER: PackedRgba = PackedRgba::rgb(40, 46, 62);
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
    tree: Vec<(usize, u32, String, u64, u64)>, // (depth, pid, comm, opens, alerts)
    top_files: Vec<FileRank>,
    exts: std::collections::HashMap<String, u64>,
    rates: VecDeque<RateSample>,
    total: u64,
    lost: u64,
    uptime: u64,
}

/// Row in the heatmap: process name + per-extension counts.
struct HeatmapRow {
    label: String,
    ext_counts: Vec<(String, u64)>,
}

#[derive(Clone)]
struct Evt { ts: String, kind: String, pid: u32, comm: String, file: Option<String>, is_alert: bool, opens: u64 }

#[derive(Clone)]
struct NetEntry { ts: String, pid: u32, comm: String, kind: String, addr: String }

// ── State ─────────────────────────────────────────────────────────────────

struct App_ {
    log: LogViewer, log_st: LogViewerState,
    alerts: LogViewer, alert_st: LogViewerState,
    net: VecDeque<NetEntry>,
    procs: Vec<ProcRow>,
    tree: Vec<(usize, u32, String, u64, u64)>,
    files: Vec<FileRow>,
    exts: Vec<(String, u64)>,
    heatmap: Vec<HeatmapRow>,
    heatmap_exts: Vec<String>,
    rates: VecDeque<RateSample>,
    rate_chart: Vec<(f64, f64)>, // (x, smoothed_exec) for LineChart
    open_chart: Vec<(f64, f64)>,
    alert_chart: Vec<(f64, f64)>,
    chart_x: f64,
    // EMA smoothing
    smooth_exec: f64,
    smooth_open: f64,
    smooth_alert: f64,
    focused: usize,
    paused: bool,
    help: bool,
    total: u64, lost: u64, uptime: u64, alert_count: u64,
    time: f64,
    rx: mpsc::Receiver<Snapshot>,
}

struct ProcRow { pid: u32, comm: String, opens: u64, alerts: u64 }
struct FileRow { name: String, count: u64, ext: String, entropy: f64 }

impl App_ {
    fn new(rx: mpsc::Receiver<Snapshot>) -> Self {
        let mut log = LogViewer::new(5000);
        log.push("[halcyon] eBPF monitor started — press ? for help");
        Self {
            log, log_st: LogViewerState::default(),
            alerts: LogViewer::new(200), alert_st: LogViewerState::default(),
            net: VecDeque::new(), procs: Vec::new(), tree: Vec::new(), files: Vec::new(), exts: Vec::new(),
            heatmap: Vec::new(), heatmap_exts: Vec::new(),
            rate_chart: Vec::new(), open_chart: Vec::new(), alert_chart: Vec::new(), chart_x: 0.0,
            smooth_exec: 0.0, smooth_open: 0.0, smooth_alert: 0.0,
            rates: VecDeque::new(), focused: 0, paused: false, help: false,
            total: 0, lost: 0, uptime: 0, alert_count: 0, time: 0.0, rx,
        }
    }

    fn apply_snapshot(&mut self, s: Snapshot) {
        for e in &s.events {
            let kind_tag = match e.kind.as_str() {
                "Exec" => "EXEC  ", "Open" => "OPEN  ",
                "Connect"|"Accept"|"SendTo"|"RecvFrom" => "NET   ",
                "Mkdir" => "MKDIR ", "Unlink" => "UNLINK",
                "Kill" => "KILL  ", "Chmod" => "CHMOD ", _ => "EVENT ",
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
            if e.is_alert { self.alert_count += 1; self.alerts.push(line.as_str()); }
        }
        let system_comms = ["systemd", "kthreadd", "rcu_sched", "ksoftirqd", "migration",
            "watchdog", "khungtaskd", "kswapd0", "kcompactd0", "jbd2", "init"];
        self.procs = s.stats.iter()
            .filter(|p| !system_comms.contains(&p.comm.as_str()))
            .map(|p| ProcRow {
                pid: p.pid, comm: p.comm.clone(), opens: p.window_opens, alerts: p.alerts,
            }).collect();
        self.procs.sort_by(|a,b| b.opens.cmp(&a.opens));
        self.tree = s.tree.into_iter()
            .filter(|(_, _, comm, _, _)| !system_comms.contains(&comm.as_str()))
            .collect();
        self.files = s.top_files.iter().map(|f| FileRow {
            name: f.path.rsplit('/').next().unwrap_or(&f.path).to_string(),
            count: f.count, ext: f.extension.clone(), entropy: f.entropy,
        }).collect();
        let mut ext_vec: Vec<(String,u64)> = s.exts.iter().map(|(k,v)| (k.clone(),*v)).collect();
        ext_vec.sort_by(|a,b| b.1.cmp(&a.1));
        self.exts = ext_vec;
        self.rates = s.rates;

        // EMA smoothing for line chart (alpha = 0.3)
        let alpha = 0.3;
        for r in &self.rates {
            self.chart_x += 1.0;
            self.smooth_exec = self.smooth_exec * (1.0 - alpha) + r.exec_count as f64 * alpha;
            self.smooth_open = self.smooth_open * (1.0 - alpha) + r.open_count as f64 * alpha;
            self.smooth_alert = self.smooth_alert * (1.0 - alpha) + r.alert_count as f64 * alpha;
            self.rate_chart.push((self.chart_x, self.smooth_exec));
            self.open_chart.push((self.chart_x, self.smooth_open));
            self.alert_chart.push((self.chart_x, self.smooth_alert));
        }
        let max_pts = 120;
        if self.rate_chart.len() > max_pts {
            self.rate_chart.drain(..self.rate_chart.len() - max_pts);
            self.open_chart.drain(..self.open_chart.len() - max_pts);
            self.alert_chart.drain(..self.alert_chart.len() - max_pts);
        }
        self.total = s.total; self.lost = s.lost; self.uptime = s.uptime;

        // Build heatmap: top procs × top exts (using global ext counts)
        // Filter system processes
        let system_comms = ["systemd", "kthreadd", "rcu_sched", "ksoftirqd", "migration",
            "watchdog", "khungtaskd", "kswapd0", "kcompactd0", "jbd2"];
        let mut procs_for_heat: Vec<&ProcStats> = s.stats.iter()
            .filter(|p| !system_comms.contains(&p.comm.as_str()))
            .collect();
        procs_for_heat.sort_by(|a,b| b.total_opens.cmp(&a.total_opens));
        procs_for_heat.truncate(8);

        let mut all_exts: Vec<(String, u64)> = s.exts.iter().map(|(k,v)| (k.clone(),*v)).collect();
        all_exts.sort_by(|a,b| b.1.cmp(&a.1));
        let top_exts: Vec<String> = all_exts.iter().take(6).map(|(e,_)| e.clone()).collect();

        if !procs_for_heat.is_empty() && !top_exts.is_empty() {
            self.heatmap = procs_for_heat.iter().map(|p| {
                let ext_counts: Vec<(String, u64)> = top_exts.iter().map(|ext| {
                    (ext.clone(), p.extensions.get(ext).copied().unwrap_or(0))
                }).collect();
                HeatmapRow { label: p.comm.clone(), ext_counts }
            }).collect();
            self.heatmap_exts = top_exts;
        } else {
            // Fallback: use global extension counts as single-row heatmap
            self.heatmap = vec![];
            self.heatmap_exts = vec![];
        }
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
                    KeyCode::Char(c @ '1'..='6') => self.focused = (c as usize) - ('1' as usize),
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
                self.time += 0.05;
                while let Ok(s) = self.rx.try_recv() { self.apply_snapshot(s); }
            }
            _ => {}
        }
        Cmd::None
    }

    fn view(&self, frame: &mut Frame) {
        let area = Rect::from_size(frame.buffer.width(), frame.buffer.height());

        // Fill entire screen with dark background
        let bg = PackedRgba::rgb(10, 14, 22);
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = frame.buffer.get_mut(x, y) {
                    cell.bg = bg;
                }
            }
        }

        if self.help { return self.draw_help(frame, area); }

        let outer = Flex::vertical().constraints([
            Constraint::Fixed(2),  // header with badges + color wave
            Constraint::Min(0),   // body
            Constraint::Fixed(6), // linechart
            Constraint::Fixed(1), // status
        ]).split(area);

        self.draw_header(frame, outer[0]);
        self.draw_body(frame, outer[1]);
        self.draw_linechart(frame, outer[2]);
        self.draw_status(frame, outer[3]);
    }

    fn subscriptions(&self) -> Vec<Box<dyn Subscription<Msg>>> {
        vec![Box::new(Every::new(Duration::from_millis(50), || Msg::Tick))]
    }
}

// ── Drawing ───────────────────────────────────────────────────────────────

impl App_ {
    fn border(&self, idx: usize) -> PackedRgba { if self.focused==idx { BORDER_HI } else { BORDER } }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let row0 = Rect::new(area.x, area.y, area.width, 1);
        let row1 = Rect::new(area.x, area.y + 1, area.width, 1);

        // Row 0: Animated color wave title + badges
        let title = format!("halcyon  eBPF process monitor  {} events  {} lost  up {}",
            self.total, self.lost, fmt_dur(self.uptime));
        let styled_title = StyledText::new(title)
            .bold()
            .effect(TextEffect::ColorWave {
                color1: BLUE.into(),
                color2: CYAN.into(),
                speed: 1.2,
                wavelength: 8.0,
            })
            .base_color(BLUE)
            .time(self.time);
        styled_title.render(row0, f);

        // Row 1: Status badges
        let lost_label = format!("{} LOST", self.lost);
        let badges_data: [(&str, PackedRgba); 3] = [
            ("eBPF", GREEN),
            ("LIVE", if self.alert_count > 0 { RED } else { GREEN }),
            (&lost_label, if self.lost > 0 { AMBER } else { DIM }),
        ];
        let mut x = area.x;
        for &(label, color) in &badges_data {
            let style = Style::new().fg(PackedRgba::rgb(10, 12, 20)).bg(color).attrs(StyleFlags::BOLD);
            let badge = Badge::new(label).with_style(style).with_padding(1, 1);
            let w = badge.width().min(area.width.saturating_sub(x - area.x));
            if w == 0 { break; }
            badge.render(Rect::new(x, area.y + 1, w, 1), f);
            x += w + 1;
        }

        // Sparkline of event rate next to badges
        if self.rates.len() > 2 && x < area.right() {
            let spark_w = (area.right() - x).saturating_sub(2) as usize;
            let data: Vec<f64> = self.rates.iter().rev().take(spark_w).rev()
                .map(|r| r.exec_count as f64).collect();
            if !data.is_empty() {
                let spark_area = Rect::new(x, area.y + 1, spark_w as u16, 1);
                Sparkline::new(&data)
                    .style(Style::new().fg(BLUE))
                    .render(spark_area, f);
            }
        }
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
        self.draw_exts_chart(f, mid[1]);
        self.draw_net(f, right[0]);
        self.draw_files(f, right[1]);
        self.draw_heatmap(f, right[2]);
    }

    fn block<'a>(&self, idx: usize, title: &'a str) -> Block<'a> {
        Block::new().title(title).borders(Borders::ALL)
            .border_type(BorderType::Rounded).border_style(Style::new().fg(self.border(idx)))
    }

    fn draw_events(&self, f: &mut Frame, area: Rect) {
        let b = self.block(0, " EVENTS ");
        let inner = b.inner(area); b.render(area, f);
        let mut st = self.log_st.clone();
        self.log.render(inner, f, &mut st);
    }

    fn draw_procs(&self, f: &mut Frame, area: Rect) {
        let title = format!(" PROCESSES ({}) ", self.tree.len());
        let b = self.block(1, &title);
        let inner = b.inner(area); b.render(area, f);
        if self.tree.is_empty() { return; }

        let max = self.tree.iter().map(|&(_, _, _, opens, _)| opens).max().unwrap_or(1).max(1);
        let bar_w = inner.width.saturating_sub(38) as usize;
        let colors = MiniBarColors::new(BLUE, GREEN, AMBER, RED);
        let rows = inner.height as usize;

        for (i, &(depth, pid, ref comm, opens, alerts)) in self.tree.iter().take(rows).enumerate() {
            let y = inner.y + i as u16;
            let value = (opens as f64 / max as f64).clamp(0.0, 1.0);

            // Tree connector prefix
            let indent = "│   ".repeat(depth);
            let connector = if i + 1 < self.tree.len() && self.tree.get(i + 1).map_or(false, |&(d, _, _, _, _)| d > depth) {
                "├── "
            } else if depth > 0 {
                "└── "
            } else {
                "    "
            };

            let prefix = format!("{}{}", indent, connector);
            let label = format!("{} {:>5} {:<16}", prefix, pid, trunc(comm, 16));
            let label_c = if alerts > 0 { RED } else if opens > 10 { AMBER } else { BRIGHT };

            let text_w = inner.width.saturating_sub(bar_w as u16 + 1);
            Paragraph::new(Line::from_spans(vec![
                Span::styled(label, Style::new().fg(label_c)),
            ])).render(Rect::new(inner.x, y, text_w, 1), f);

            // MiniBar
            let bar_x = inner.x + text_w;
            let bar_area = Rect::new(bar_x, y, bar_w as u16, 1);
            MiniBar::new(value, bar_w as u16)
                .colors(colors)
                .show_percent(true)
                .render(bar_area, f);
        }
    }

    fn draw_exts_chart(&self, f: &mut Frame, area: Rect) {
        let b = self.block(4, " FILE TYPES ");
        let inner = b.inner(area); b.render(area, f);
        if self.exts.is_empty() { return; }

        // BarChart from ftui-extras
        let max_count = self.exts.iter().map(|(_,c)| *c as f64).fold(0.0f64, f64::max).max(1.0);
        let groups: Vec<BarGroup> = self.exts.iter().take(10).map(|(ext, cnt)| {
            BarGroup::new(ext.as_str(), vec![*cnt as f64])
        }).collect();

        let palette: Vec<PackedRgba> = self.exts.iter().take(10)
            .map(|(ext, _)| ext_color(ext)).collect();

        BarChart::new(groups)
            .direction(BarDirection::Horizontal)
            .mode(BarMode::Grouped)
            .bar_width(1)
            .colors(palette)
            .style(Style::new().fg(FG))
            .max(max_count)
            .render(inner, f);
    }

    fn draw_net(&self, f: &mut Frame, area: Rect) {
        let title = format!(" NETWORK ({}) ", self.net.len());
        let b = self.block(2, &title);
        let inner = b.inner(area); b.render(area, f);
        if self.net.is_empty() {
            Paragraph::new(Line::from_spans(vec![
                Span::styled("  no network events captured", Style::new().fg(DIM)),
                Span::styled("\n  (kernel may lack sockaddr capture)", Style::new().fg(DIM)),
            ])).render(inner, f);
            return;
        }

        // Split: top = aggregated per-process, bottom = Canvas traffic flow
        let split = Flex::vertical().constraints([
            Constraint::Percentage(60.0),
            Constraint::Percentage(40.0),
        ]).split(inner);

        // --- Top: per-process aggregated connections ---
        let mut proc_conns: std::collections::HashMap<String, (u64, u64, u64, u64)> =
            std::collections::HashMap::new(); // (connect, accept, send, recv)
        for e in self.net.iter() {
            let entry = proc_conns.entry(e.comm.clone()).or_insert((0,0,0,0));
            match e.kind.as_str() {
                "Connect" => entry.0 += 1,
                "Accept" => entry.1 += 1,
                "SendTo" => entry.2 += 1,
                "RecvFrom" => entry.3 += 1,
                _ => {}
            }
        }
        let mut procs: Vec<_> = proc_conns.into_iter().collect();
        procs.sort_by(|a,b| {
            let a_total = a.1.0 + a.1.1 + a.1.2 + a.1.3;
            let b_total = b.1.0 + b.1.1 + b.1.2 + b.1.3;
            b_total.cmp(&a_total)
        });
        let max_conns = procs.iter().map(|(_,c)| c.0+c.1+c.2+c.3).max().unwrap_or(1).max(1);

        let rows = split[0].height as usize;
        let bar_w = split[0].width.saturating_sub(38) as usize;
        for (i, (comm, (conn, acc, send, recv))) in procs.iter().take(rows).enumerate() {
            let y = split[0].y + i as u16;
            let total = conn + acc + send + recv;
            let value = (total as f64 / max_conns as f64).clamp(0.0, 1.0);

            // Label: comm + counts
            let label = format!(" {:<14}", trunc(comm, 14));
            Paragraph::new(Line::from_spans(vec![
                Span::styled(label, Style::new().fg(BRIGHT)),
                Span::styled(format!(" >{:<4} <{:<4} ", conn, acc), Style::new().fg(CYAN)),
                Span::styled(format!(" {:>4}  {:>4} ", send, recv), Style::new().fg(PURPLE)),
            ])).render(Rect::new(split[0].x, y, 38.min(split[0].width), 1), f);

            // Bar: stacked direction
            let bar_area = Rect::new(split[0].x + 38, y, bar_w as u16, 1);
            let colors = MiniBarColors::new(BLUE, GREEN, AMBER, PURPLE);
            MiniBar::new(value, bar_w as u16).colors(colors).show_percent(false).render(bar_area, f);
        }

        // --- Bottom: Canvas traffic flow visualization ---
        if split[1].height >= 3 {
            let canvas_area = Rect::new(split[1].x + 1, split[1].y + 1,
                split[1].width.saturating_sub(2), split[1].height.saturating_sub(2));
            if canvas_area.width >= 4 && canvas_area.height >= 2 {
                let mut painter = Painter::for_area(canvas_area, Mode::Block);
                let (pw, ph) = painter.size();
                let t = self.time;

                // Draw traffic flow: each recent event becomes a colored dot
                // moving right (out) or left (in)
                for (i, e) in self.net.iter().rev().take((pw * ph) as usize).enumerate() {
                    let frac = i as f64 / (pw * ph).max(1) as f64;
                    let px = if matches!(e.kind.as_str(), "Connect"|"SendTo") {
                        // Outgoing: left to right
                        (frac * pw as f64) as i32
                    } else {
                        // Incoming: right to left
                        ((1.0 - frac) * pw as f64) as i32
                    };
                    let py = ((i as f64 / pw as f64).fract() * ph as f64) as i32;
                    let c = match e.kind.as_str() {
                        "Connect" => BLUE, "Accept" => GREEN,
                        "SendTo" => AMBER, "RecvFrom" => PURPLE, _ => CYAN,
                    };
                    painter.point_colored(px, py, c);
                }

                Canvas::from_painter(&painter)
                    .style(Style::new().fg(FG))
                    .render(canvas_area, f);
            }
        }
    }

    fn draw_files(&self, f: &mut Frame, area: Rect) {
        let b = self.block(3, " TOP FILES ");
        let inner = b.inner(area); b.render(area, f);
        if self.files.is_empty() { return; }

        let max = self.files.iter().map(|r| r.count).max().unwrap_or(1).max(1);
        let rows = inner.height as usize;
        let bar_w = inner.width.saturating_sub(36) as usize;
        let colors = MiniBarColors::new(BLUE, GREEN, AMBER, RED);

        for (i, r) in self.files.iter().take(rows).enumerate() {
            let y = inner.y + i as u16;
            let value = (r.count as f64 / max as f64).clamp(0.0, 1.0);

            let rank_c = if i < 3 { AMBER } else { DIM };
            let label = format!("{:>2}. {:<16}", i+1, trunc(&r.name, 16));
            let ext_c = ext_color(&r.ext);
            let entropy_c = if r.entropy > 0.7 { RED } else if r.entropy > 0.4 { AMBER } else { GREEN };

            let label_w = 28.min(inner.width);
            Paragraph::new(Line::from_spans(vec![
                Span::styled(label, Style::new().fg(rank_c)),
                Span::styled(format!(" .{:<4}", r.ext), Style::new().fg(ext_c)),
            ])).render(Rect::new(inner.x, y, label_w, 1), f);

            let bar_area = Rect::new(inner.x + label_w + 1, y, bar_w as u16, 1);
            MiniBar::new(value, bar_w as u16)
                .colors(colors)
                .show_percent(false)
                .render(bar_area, f);

            // Entropy indicator after bar
            let entropy_x = inner.x + label_w + 1 + bar_w as u16 + 1;
            if entropy_x < inner.right() {
                Paragraph::new(Line::from_spans(vec![
                    Span::styled(format!("H:{:.1}", r.entropy), Style::new().fg(entropy_c)),
                ])).render(Rect::new(entropy_x, y, 6, 1), f);
            }
        }
    }

    fn draw_heatmap(&self, f: &mut Frame, area: Rect) {
        let title = format!(" HEATMAP ({}x{}) ", self.heatmap.len(), self.heatmap_exts.len());
        let b = self.block(5, &title);
        let inner = b.inner(area); b.render(area, f);
        if inner.width < 8 || inner.height < 4 || self.heatmap.is_empty() {
            if self.heatmap.is_empty() && inner.height >= 2 {
                Paragraph::new(Line::from_spans(vec![
                    Span::styled("  waiting for file open events...", Style::new().fg(DIM)),
                ])).render(Rect::new(inner.x + 1, inner.y + 1, inner.width - 2, 1), f);
            }
            return;
        }

        // Layout: top row = ext labels, left col = proc names, rest = canvas heatmap
        let label_w: u16 = 12;
        let header_h: u16 = 1;
        let canvas_area = Rect::new(
            inner.x + label_w,
            inner.y + header_h,
            inner.width.saturating_sub(label_w),
            inner.height.saturating_sub(header_h),
        );

        if canvas_area.width < 2 || canvas_area.height < 2 { return; }

        // Find max count for normalization
        let mut max_count: u64 = 1;
        for row in &self.heatmap {
            for &(_, cnt) in &row.ext_counts {
                if cnt > max_count { max_count = cnt; }
            }
        }

        // Extension labels on top
        let ext_cols = self.heatmap_exts.len().max(1) as u16;
        let col_w = canvas_area.width / ext_cols;
        for (i, ext) in self.heatmap_exts.iter().enumerate() {
            let x = canvas_area.x + i as u16 * col_w;
            if x + col_w > canvas_area.right() { break; }
            let label = format!(".{:<w$}", ext, w = (col_w as usize).saturating_sub(1));
            Paragraph::new(Line::from_spans(vec![Span::styled(label, Style::new().fg(DIM))]))
                .render(Rect::new(x, inner.y, col_w, 1), f);
        }

        // Process names on left
        let proc_rows = self.heatmap.len().max(1) as u16;
        let row_h = canvas_area.height / proc_rows;
        for (i, row) in self.heatmap.iter().enumerate() {
            let y = canvas_area.y + i as u16 * row_h;
            if y + row_h > canvas_area.bottom() { break; }
            let label = trunc(&row.label, label_w as usize);
            Paragraph::new(Line::from_spans(vec![Span::styled(label, Style::new().fg(FG))]))
                .render(Rect::new(inner.x, y, label_w, 1), f);
        }

        // Canvas heatmap with Mode::Block (2x2 subpixels per cell)
        let mut painter = Painter::for_area(canvas_area, Mode::Block);
        let (pw, ph) = painter.size();

        for (row_i, row) in self.heatmap.iter().enumerate() {
            for (col_i, ext) in self.heatmap_exts.iter().enumerate() {
                let count = row.ext_counts.iter()
                    .find(|(e, _)| e == ext)
                    .map(|(_, c)| *c)
                    .unwrap_or(0);
                let value = (count as f64 / max_count as f64).clamp(0.0, 1.0);
                let color = heatmap_gradient(value);

                // Map to canvas pixel coordinates
                let px_start = (col_i as f64 / self.heatmap_exts.len().max(1) as f64 * pw as f64) as i32;
                let px_end = ((col_i + 1) as f64 / self.heatmap_exts.len().max(1) as f64 * pw as f64) as i32;
                let py_start = (row_i as f64 / self.heatmap.len().max(1) as f64 * ph as f64) as i32;
                let py_end = ((row_i + 1) as f64 / self.heatmap.len().max(1) as f64 * ph as f64) as i32;

                for py in py_start..py_end {
                    for px in px_start..px_end {
                        painter.point_colored(px, py, color);
                    }
                }
            }
        }

        Canvas::from_painter(&painter)
            .style(Style::new().fg(FG))
            .render(canvas_area, f);
    }

    fn draw_linechart(&self, f: &mut Frame, area: Rect) {
        let b = self.block(6, " EVENT RATE ");
        let inner = b.inner(area); b.render(area, f);
        if inner.width < 10 || inner.height < 3 { return; }
        if self.rate_chart.len() < 2 { return; }

        let series = vec![
            Series::new("exec", &self.rate_chart, BLUE).markers(true),
            Series::new("open", &self.open_chart, GREEN),
            Series::new("alert", &self.alert_chart, RED),
        ];

        let max_y = self.rate_chart.iter().chain(&self.open_chart).chain(&self.alert_chart)
            .map(|&(_, y)| y).fold(0.0f64, f64::max).max(1.0);

        LineChart::new(series)
            .style(Style::new().fg(FG))
            .x_labels(vec!["-60", "-30", "now"])
            .y_labels(vec!["0", &format!("{:.0}", max_y / 2.0), &format!("{:.0}", max_y)])
            .legend(true)
            .y_bounds(0.0, max_y)
            .render(inner, f);
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
            Line::from_spans(vec![Span::styled(" NAV", Style::new().fg(AMBER).attrs(StyleFlags::BOLD))]),
            Line::raw("   Tab/Shift+Tab   cycle panels"),
            Line::raw("   1-6             jump to panel"),
            Line::raw("   j/k or arrows   scroll"),
            Line::raw(""),
            Line::from_spans(vec![Span::styled(" ACT", Style::new().fg(AMBER).attrs(StyleFlags::BOLD))]),
            Line::raw("   p               pause/resume"),
            Line::raw("   q/Esc           quit"),
            Line::raw("   Ctrl+C          force quit"),
        ];
        let b = Block::new().title(" help ").borders(Borders::ALL)
            .border_type(BorderType::Rounded).border_style(Style::new().fg(BLUE));
        let inner = b.inner(area); b.render(area, f);
        Paragraph::new(ftui_text::Text::from_lines(lines)).render(inner, f);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn trunc(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}~", &s[..max.saturating_sub(1)]) }
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
        "json"|"toml"|"yaml"|"yml" => CYAN, _ => BLUE,
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
            let tree_raw = {
                let tree = monitor.build_process_tree();
                Monitor::flatten_tree(&tree).into_iter().map(|(depth, node)| {
                    (depth, node.pid, node.comm.clone(), node.total_opens, node.alerts)
                }).collect::<Vec<_>>()
            };
            let snap = Snapshot {
                events: evts, stats: monitor.stats_sorted(),
                tree: tree_raw,
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
