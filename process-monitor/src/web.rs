use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::registry::Registry;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};

use axum::http::{header, HeaderValue, Request, Response};
use tower::{Layer, Service};

use crate::monitor::{Kind, Monitor, Output};

// ── Shared state ──────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    monitor: Arc<Mutex<Monitor>>,
    tx: broadcast::Sender<WsEvent>,
    metrics: Arc<Mutex<Registry>>,
    metrics_events: Counter,
    metrics_exec: Counter,
    metrics_open: Counter,
    metrics_alerts: Counter,
    #[allow(dead_code)]
    metrics_lost: Counter,
    metrics_ws: Counter,
}

impl AppState {
    fn new(monitor: Monitor) -> Self {
        let mut registry = Registry::default();
        let events_total = Counter::default();
        registry.register(
            "talus_events_total",
            "Total number of eBPF events received",
            events_total.clone(),
        );
        let exec_events_total = Counter::default();
        registry.register(
            "talus_exec_events_total",
            "Total number of execve events",
            exec_events_total.clone(),
        );
        let open_events_total = Counter::default();
        registry.register(
            "talus_open_events_total",
            "Total number of openat events",
            open_events_total.clone(),
        );
        let alerts_total = Counter::default();
        registry.register(
            "talus_alerts_total",
            "Total number of alerts fired",
            alerts_total.clone(),
        );
        let lost_events_total = Counter::default();
        registry.register(
            "talus_lost_events_total",
            "Total number of lost events (perf buffer overruns)",
            lost_events_total.clone(),
        );
        let ws_connections = Counter::default();
        registry.register(
            "talus_ws_connections_total",
            "Total number of WebSocket connections",
            ws_connections.clone(),
        );
        let (tx, _) = broadcast::channel(1024);
        Self {
            monitor: Arc::new(Mutex::new(monitor)),
            tx,
            metrics: Arc::new(Mutex::new(registry)),
            metrics_events: events_total,
            metrics_exec: exec_events_total,
            metrics_open: open_events_total,
            metrics_alerts: alerts_total,
            metrics_lost: lost_events_total,
            metrics_ws: ws_connections,
        }
    }
}

// ── WebSocket event types ─────────────────────────────────────────────────

#[derive(Clone, Serialize)]
#[serde(tag = "type")]
enum WsEvent {
    #[serde(rename = "event")]
    Event {
        ts: String,
        kind: String,
        pid: u32,
        uid: u32,
        comm: String,
        file: Option<String>,
        extension: Option<String>,
        argv: Option<String>,
    },
    #[serde(rename = "alert")]
    Alert {
        ts: String,
        pid: u32,
        uid: u32,
        comm: String,
        opens: u64,
    },
}

// ── REST API response types ───────────────────────────────────────────────

#[derive(Serialize)]
struct ApiResponse<T: Serialize> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}

#[derive(Serialize)]
struct StatsResponse {
    total_events: u64,
    total_lost: u64,
    uptime_secs: u64,
    active_pids: usize,
    threshold: u64,
}

#[derive(Serialize)]
struct ProcessInfo {
    pid: u32,
    ppid: u32,
    comm: String,
    total_opens: u64,
    total_execs: u64,
    alerts: u64,
}

#[derive(Serialize)]
struct FileRankResponse {
    path: String,
    count: u64,
    extension: String,
    entropy: f64,
}

#[derive(Serialize)]
struct ExtensionResponse {
    extension: String,
    count: u64,
}

#[derive(Deserialize)]
struct ThresholdQuery {
    threshold: Option<u64>,
}

// ── API handlers ──────────────────────────────────────────────────────────

async fn get_stats(State(state): State<AppState>) -> Json<ApiResponse<StatsResponse>> {
    let mon = state.monitor.lock().await;
    let pids = mon.stats_sorted().len();
    Json(ApiResponse {
        ok: true,
        data: Some(StatsResponse {
            total_events: mon.total_events,
            total_lost: mon.total_lost,
            uptime_secs: mon.uptime().as_secs(),
            active_pids: pids,
            threshold: mon.threshold,
        }),
        error: None,
    })
}

async fn get_processes(State(state): State<AppState>) -> Json<ApiResponse<Vec<ProcessInfo>>> {
    let mon = state.monitor.lock().await;
    let processes: Vec<ProcessInfo> = mon
        .stats_sorted()
        .into_iter()
        .map(|s| ProcessInfo {
            pid: s.pid,
            ppid: s.ppid,
            comm: s.comm.trim_end_matches('\0').to_string(),
            total_opens: s.total_opens,
            total_execs: s.total_execs,
            alerts: s.alerts,
        })
        .collect();
    Json(ApiResponse {
        ok: true,
        data: Some(processes),
        error: None,
    })
}

async fn get_files(State(state): State<AppState>) -> Json<ApiResponse<Vec<FileRankResponse>>> {
    let mon = state.monitor.lock().await;
    let files: Vec<FileRankResponse> = mon
        .top_files(50)
        .into_iter()
        .map(|f| FileRankResponse {
            path: f.path,
            count: f.count,
            extension: f.extension,
            entropy: f.entropy,
        })
        .collect();
    Json(ApiResponse {
        ok: true,
        data: Some(files),
        error: None,
    })
}

async fn get_extensions(
    State(state): State<AppState>,
) -> Json<ApiResponse<Vec<ExtensionResponse>>> {
    let mon = state.monitor.lock().await;
    let mut exts: Vec<ExtensionResponse> = mon
        .extension_counts()
        .iter()
        .map(|(k, &v)| ExtensionResponse {
            extension: k.clone(),
            count: v,
        })
        .collect();
    exts.sort_by_key(|e| std::cmp::Reverse(e.count));
    Json(ApiResponse {
        ok: true,
        data: Some(exts),
        error: None,
    })
}

async fn set_threshold(
    State(state): State<AppState>,
    Json(body): Json<ThresholdQuery>,
) -> Json<ApiResponse<StatsResponse>> {
    let mut mon = state.monitor.lock().await;
    if let Some(t) = body.threshold {
        mon.threshold = t;
    }
    let pids = mon.stats_sorted().len();
    Json(ApiResponse {
        ok: true,
        data: Some(StatsResponse {
            total_events: mon.total_events,
            total_lost: mon.total_lost,
            uptime_secs: mon.uptime().as_secs(),
            active_pids: pids,
            threshold: mon.threshold,
        }),
        error: None,
    })
}

async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let registry = state.metrics.lock().await;
    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        buffer,
    )
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    state.metrics_ws.inc();
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.tx.subscribe();

    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event) {
                if sender.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg {
                break;
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

// ── Event forwarder ───────────────────────────────────────────────────────

fn spawn_event_forwarder(state: AppState) {
    tokio::spawn(async move {
        loop {
            {
                let mut mon = state.monitor.lock().await;
                let outputs = mon.poll();
                for output in outputs {
                    match output {
                        Output::Event(ev) => {
                            state.metrics_events.inc();
                            match ev.kind {
                                Kind::Exec => {
                                    state.metrics_exec.inc();
                                }
                                Kind::Open => {
                                    state.metrics_open.inc();
                                }
                                _ => {}
                            }
                            let _ = state.tx.send(WsEvent::Event {
                                ts: ev.ts,
                                kind: format!("{:?}", ev.kind),
                                pid: ev.pid,
                                uid: ev.uid,
                                comm: ev.comm.trim_end_matches('\0').to_string(),
                                file: ev.file,
                                extension: ev.extension,
                                argv: ev.argv,
                            });
                        }
                        Output::Alert(al) => {
                            state.metrics_alerts.inc();
                            let _ = state.tx.send(WsEvent::Alert {
                                ts: al.ts,
                                pid: al.pid,
                                uid: al.uid,
                                comm: al.comm.trim_end_matches('\0').to_string(),
                                opens: al.opens,
                            });
                        }
                        Output::Action(_) => {}
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
}

// ── Dashboard HTML ────────────────────────────────────────────────────────

async fn dashboard() -> impl IntoResponse {
    axum::response::Html(DASHBOARD_HTML)
}

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Talus eBPF Monitor</title>
<style>
  :root { --bg: #0a0a1a; --panel: #111127; --border: #1e1e3a; --cyan: #00ffff; --green: #00ff64; --red: #ff3232; --yellow: #ffff00; --magenta: #ff00ff; --dim: #505064; }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { background: var(--bg); color: #ccc; font-family: 'JetBrains Mono', 'Fira Code', monospace; font-size: 13px; }
  .header { background: var(--panel); border-bottom: 1px solid var(--border); padding: 12px 20px; display: flex; justify-content: space-between; align-items: center; }
  .header h1 { color: var(--cyan); font-size: 16px; }
  .header .stats { color: var(--dim); }
  .grid { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; padding: 8px; height: calc(100vh - 50px); }
  .panel { background: var(--panel); border: 1px solid var(--border); border-radius: 4px; overflow: hidden; display: flex; flex-direction: column; }
  .panel-header { padding: 8px 12px; border-bottom: 1px solid var(--border); color: var(--cyan); font-weight: bold; font-size: 12px; }
  .panel-body { flex: 1; overflow-y: auto; padding: 4px 8px; }
  .event { padding: 2px 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .event .ts { color: var(--dim); }
  .event .tag { font-weight: bold; padding: 0 4px; }
  .tag-exec { color: var(--green); }
  .tag-open { color: var(--cyan); }
  .tag-alert { color: var(--red); font-weight: bold; }
  .process-row { display: flex; justify-content: space-between; padding: 2px 0; }
  .process-row .alerts { color: var(--red); }
  .file-row { display: grid; grid-template-columns: 2fr 1fr 1fr; padding: 2px 0; gap: 8px; }
  .ext-bar { display: flex; align-items: center; gap: 8px; padding: 2px 0; }
  .ext-bar .bar { height: 12px; background: var(--cyan); border-radius: 2px; }
  .status { position: fixed; bottom: 0; left: 0; right: 0; background: var(--panel); border-top: 1px solid var(--border); padding: 4px 20px; display: flex; gap: 20px; color: var(--dim); font-size: 11px; }
  .connected { color: var(--green); }
  .disconnected { color: var(--red); }
</style>
</head>
<body>
<div class="header">
  <h1>⚡ TALUS eBPF MONITOR</h1>
  <div class="stats" id="stats">loading...</div>
</div>
<div class="grid">
  <div class="panel" style="grid-row: span 2">
    <div class="panel-header">EVENTS (<span id="event-count">0</span>)</div>
    <div class="panel-body" id="events"></div>
  </div>
  <div class="panel"><div class="panel-header">PROCESSES</div><div class="panel-body" id="processes"></div></div>
  <div class="panel"><div class="panel-header">TOP FILES</div><div class="panel-body" id="files"></div></div>
  <div class="panel"><div class="panel-header">FILE TYPES</div><div class="panel-body" id="extensions"></div></div>
  <div class="panel"><div class="panel-header">ALERTS</div><div class="panel-body" id="alerts"></div></div>
</div>
<div class="status">
  <span>WebSocket: <span id="ws-status" class="disconnected">disconnected</span></span>
  <span>Events: <span id="total-events">0</span></span>
  <span>Lost: <span id="total-lost">0</span></span>
  <span>Uptime: <span id="uptime">0s</span></span>
</div>
<script>
const MAX_EVENTS=500;let eventCount=0;const eventsEl=document.getElementById('events');const alertsEl=document.getElementById('alerts');
function connect(){const ws=new WebSocket(`ws://${location.host}/ws`);ws.onopen=()=>{document.getElementById('ws-status').textContent='connected';document.getElementById('ws-status').className='connected'};ws.onclose=()=>{document.getElementById('ws-status').textContent='disconnected';document.getElementById('ws-status').className='disconnected';setTimeout(connect,2000)};ws.onmessage=e=>{const d=JSON.parse(e.data);if(d.type==='event'){eventCount++;document.getElementById('event-count').textContent=eventCount;const div=document.createElement('div');div.className='event';const tc=d.kind==='Exec'?'tag-exec':'tag-open';const tt=d.kind==='Exec'?'EXEC':'OPEN';const f=d.file?' → '+d.file:'';div.innerHTML=`<span class="ts">${d.ts}</span> <span class="tag ${tc}">${tt}</span> [${d.pid}] ${d.comm}${f}`;eventsEl.appendChild(div);if(eventsEl.children.length>MAX_EVENTS)eventsEl.removeChild(eventsEl.firstChild);eventsEl.scrollTop=eventsEl.scrollHeight}else if(d.type==='alert'){const div=document.createElement('div');div.className='event';div.innerHTML=`<span class="ts">${d.ts}</span> <span class="tag tag-alert">⚠ ALERT</span> [${d.pid}] ${d.comm} — ${d.opens} opens/s`;alertsEl.appendChild(div)}}}
connect();
setInterval(async()=>{try{const r=await fetch('/api/v1/stats');const j=await r.json();if(j.ok){const d=j.data;document.getElementById('stats').textContent=`evt ${d.total_events} | lost ${d.total_lost} | uptime ${d.uptime_secs}s | threshold ${d.threshold}/s`;document.getElementById('total-events').textContent=d.total_events;document.getElementById('total-lost').textContent=d.total_lost;document.getElementById('uptime').textContent=d.uptime_secs+'s'}}catch(e){}},2000);
setInterval(async()=>{try{const r=await fetch('/api/v1/files');const j=await r.json();if(j.ok){document.getElementById('files').innerHTML=j.data.map(f=>`<div class="file-row"><span>${f.path.length>30?'…'+f.path.slice(-27):f.path}</span><span>.${f.extension}</span><span>${f.count} E:${f.entropy.toFixed(2)}</span></div>`).join('')}}catch(e){}try{const r=await fetch('/api/v1/extensions');const j=await r.json();if(j.ok&&j.data.length>0){const max=Math.max(...j.data.map(e=>e.count));document.getElementById('extensions').innerHTML=j.data.slice(0,8).map(e=>`<div class="ext-bar"><span style="width:50px">.${e.extension}</span><div class="bar" style="width:${(e.count/max*100).toFixed(0)}%"></div><span>${e.count}</span></div>`).join('')}}catch(e){}try{const r=await fetch('/api/v1/processes');const j=await r.json();if(j.ok){document.getElementById('processes').innerHTML=j.data.slice(0,30).map(p=>{const ab=p.alerts>0?` <span class="alerts">⚠${p.alerts}</span>`:'';return`<div class="process-row"><span>${p.comm} [${p.pid}]</span><span>${p.total_opens} opens${ab}</span></div>`}).join('')}}catch(e){}},3000);
</script>
</body>
</html>"#;

// ── Security Headers Middleware ─────────────────────────────────────────

/// Layer that adds security headers to all HTTP responses.
///
/// Headers added:
/// - `X-Content-Type-Options: nosniff` — prevents MIME type sniffing
/// - `X-Frame-Options: DENY` — prevents clickjacking
/// - `X-XSS-Protection: 1; mode=block` — legacy XSS filter
/// - `Referrer-Policy: strict-origin-when-cross-origin` — limits referrer leakage
/// - `Permissions-Policy: camera=(), microphone=(), geolocation=()` — disables dangerous APIs
/// - `Content-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline'; ...`
#[derive(Clone)]
struct SecurityHeadersLayer;

impl<S> Layer<S> for SecurityHeadersLayer {
    type Service = SecurityHeadersService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        SecurityHeadersService { inner }
    }
}

#[derive(Clone)]
struct SecurityHeadersService<S> {
    inner: S,
}

impl<S, ReqBody> Service<Request<ReqBody>> for SecurityHeadersService<S>
where
    S: Service<Request<ReqBody>, Response = Response<axum::body::Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response<axum::body::Body>;
    type Error = S::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let fut = self.inner.call(req);
        Box::pin(async move {
            let mut response = fut.await?;
            let headers = response.headers_mut();

            headers.insert(
                header::HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            );
            headers.insert(
                header::HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("DENY"),
            );
            headers.insert(
                header::HeaderName::from_static("x-xss-protection"),
                HeaderValue::from_static("1; mode=block"),
            );
            headers.insert(
                header::HeaderName::from_static("referrer-policy"),
                HeaderValue::from_static("strict-origin-when-cross-origin"),
            );
            headers.insert(
                header::HeaderName::from_static("permissions-policy"),
                HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
            );
            headers.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:"),
            );

            Ok(response)
        })
    }
}

// ── Public entry point ────────────────────────────────────────────────────

pub async fn start_web_server(
    monitor: Monitor,
    addr: SocketAddr,
    _threshold: u64,
) -> anyhow::Result<()> {
    let state = AppState::new(monitor);

    // Start event forwarder
    spawn_event_forwarder(state.clone());

    let app = Router::new()
        .route("/", get(dashboard))
        .route("/ws", get(ws_handler))
        .route("/api/v1/stats", get(get_stats))
        .route("/api/v1/processes", get(get_processes))
        .route("/api/v1/files", get(get_files))
        .route("/api/v1/extensions", get(get_extensions))
        .route("/api/v1/threshold", post(set_threshold))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
        .layer(tower_http::cors::CorsLayer::permissive())
        .layer(SecurityHeadersLayer);

    eprintln!("[talus] web server listening on http://{addr}");
    eprintln!("[talus] dashboard: http://{addr}/");
    eprintln!("[talus] websocket: ws://{addr}/ws");
    eprintln!("[talus] prometheus: http://{addr}/metrics");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
