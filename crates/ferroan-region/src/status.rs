// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Region status forms from Tranquillity Pin6 `1d7abb8337a477842fe1d7dd84beffe389ee52ac`.
//!
//! `/simstatus` follows `SimStatusHandler` in
//! `Source/OpenSim.Server.RegionServer/OpenSimBase.cs`.
//! `/monitorstats/<region id>` follows `MonitorModule.StatsPage`.
//! Managed stats follow `StatsManager.HandleStatsRequest`: `cat`, `cont` and
//! `stat` each default to `all` (TB15, #456). `/SStats` follows
//! `WebStatsModule` when `[WebStats] enabled` is true. Prometheus `/metrics`,
//! `/healthz` and `/readyz` are the `[Monitoring]` additions from D18 and stay
//! off unless `Metrics` is set. JSON logs, `LOGDIR` rotation, OTLP and the
//! remote console are P0.13.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{OriginalUri, Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use axum::{Router, middleware};
use ferroan_client::UdpCounters;
use uuid::Uuid;

use crate::config::MonitoringSettings;

const HTTP_404: &str = "<HTML><HEAD><TITLE>404 Page not found</TITLE><BODY><BR /><H1>Ooops!</H1><P>The page you requested has been obsconded with by knomes. Find hippos quick!</P></BODY></HTML>";

/// Content type OSWebServer applies when a simple stream handler sets none.
const DEFAULT_HTML: &str = "text/html;charset=UTF-8";

/// `Default_Report.RenderJson` for an idle database: OSD strings, insertion order.
const SSTATS_JSON: &str = "{\"totalUsers\":\"0\",\"totalSessions\":\"0\",\"averageClientFPS\":\"0\",\"averageClientMem\":\"0\",\"averageSimFPS\":\"0\",\"averagePingTime\":\"0\",\"totalKBOut\":\"0\",\"totalKBIn\":\"0\"}";

/// `MonitorModule.AddMonitors` order. `GetName()` is the element name.
const MONITORS: &[&str] = &[
    "AgentCountMonitor",
    "ChildAgentCountMonitor",
    "GCMemoryMonitor",
    "ObjectCountMonitor",
    "PhysicsFrameMonitor",
    "PhysicsUpdateFrameMonitor",
    "PWSMemoryMonitor",
    "ThreadCountMonitor",
    "TotalFrameMonitor",
    "EventFrameMonitor",
    "LandFrameMonitor",
    "LastFrameTimeMonitor",
    "TimeDilationMonitor",
    "SimFPSMonitor",
    "PhysicsFPSMonitor",
    "AgentUpdatesPerSecondMonitor",
    "ActiveObjectCountMonitor",
    "ActiveScriptsMonitor",
    "ScriptEventsPerSecondMonitor",
    "InPacketsPerSecondMonitor",
    "OutPacketsPerSecondMonitor",
    "UnackedBytesMonitor",
    "PendingDownloadsMonitor",
    "PendingUploadsMonitor",
    "TotalFrameTimeMonitor",
    "NetFrameTimeMonitor",
    "PhysicsFrameTimeMonitor",
    "SimulationFrameTimeMonitor",
    "AgentFrameTimeMonitor",
    "ImagesFrameTimeMonitor",
    "SpareFrameTimeMonitor",
];

/// `RegisterStatsManagerRegionStatistics` names. Sorted, as `SortedDictionary` emits them.
const MANAGED_STATS: &[&str] = &[
    "ActivePrims",
    "ActiveScripts",
    "AgentTime",
    "AgentUpdates",
    "ChildAgents",
    "FrameTime",
    "ImageTime",
    "NetTime",
    "NPCs",
    "OtherTime",
    "PhysicsFPS",
    "PhysicsTime",
    "RootAgents",
    "ScriptEPS",
    "SimFPS",
    "SimSpareMS",
    "SlowFrames",
    "TimeDilation",
    "TotalPrims",
];

struct HttpCount {
    route: &'static str,
    status: u16,
    count: u64,
}

/// Gauges and counters for one region process.
pub struct RegionMeters {
    root_agents: AtomicU64,
    child_agents: AtomicU64,
    circuits: AtomicU64,
    udp: Arc<UdpCounters>,
    eq_polls: AtomicU64,
    eq_events: AtomicU64,
    http: Mutex<Vec<HttpCount>>,
    registered: AtomicBool,
    managed_prefix: Mutex<String>,
}

impl RegionMeters {
    pub fn new() -> Self {
        Self {
            root_agents: AtomicU64::new(0),
            child_agents: AtomicU64::new(0),
            circuits: AtomicU64::new(0),
            udp: Arc::new(UdpCounters::default()),
            eq_polls: AtomicU64::new(0),
            eq_events: AtomicU64::new(0),
            http: Mutex::new(Vec::new()),
            registered: AtomicBool::new(false),
            managed_prefix: Mutex::new(String::new()),
        }
    }

    pub fn udp(&self) -> Arc<UdpCounters> {
        self.udp.clone()
    }

    /// Circuits, agent counts and UDP totals for a diagnostic dump.
    pub fn diag_lines(&self) -> String {
        format!(
            "circuits: {}\nroot_agents: {}\nchild_agents: {}\npackets_in: {}\nunacked: {}\n",
            self.circuits.load(Ordering::Relaxed),
            self.root_agents.load(Ordering::Relaxed),
            self.child_agents.load(Ordering::Relaxed),
            self.udp.packets_in.load(Ordering::Relaxed),
            self.udp.unacked.load(Ordering::Relaxed),
        )
    }

    pub fn set_managed_prefix(&self, prefix: &str) {
        *self
            .managed_prefix
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = prefix.to_owned();
    }

    /// Root agents, child agents kept after a source hop, and circuits.
    pub fn set_agents(&self, root: u64, child: u64, circuits: u64) {
        self.root_agents.store(root, Ordering::Relaxed);
        self.child_agents.store(child, Ordering::Relaxed);
        self.circuits.store(circuits, Ordering::Relaxed);
    }

    pub fn set_registered(&self, registered: bool) {
        self.registered.store(registered, Ordering::Relaxed);
    }

    pub fn registered(&self) -> bool {
        self.registered.load(Ordering::Relaxed)
    }

    pub fn record_http(&self, route: &'static str, status: u16) {
        let mut rows = self
            .http
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(row) = rows
            .iter_mut()
            .find(|row| row.route == route && row.status == status)
        {
            row.count = row.count.saturating_add(1);
            return;
        }
        if rows.len() < 64 {
            rows.push(HttpCount {
                route,
                status,
                count: 1,
            });
        }
    }

    pub fn note_event_poll(&self) {
        self.eq_polls.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for RegionMeters {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct StatusState {
    region_id: Uuid,
    region_name: String,
    monitor_enabled: bool,
    managed_uri: String,
    managed_password: String,
    web_stats: bool,
    meters: Arc<RegionMeters>,
}

impl StatusState {
    pub fn new(
        region_id: Uuid,
        region_name: String,
        monitoring: &MonitoringSettings,
        meters: Arc<RegionMeters>,
    ) -> Self {
        if !monitoring.managed_stats_uri.is_empty() {
            meters.set_managed_prefix(&format!("/{}", monitoring.managed_stats_uri));
        }
        Self {
            region_id,
            region_name,
            monitor_enabled: monitoring.monitor_enabled,
            managed_uri: monitoring.managed_stats_uri.clone(),
            managed_password: monitoring.managed_stats_password.clone(),
            web_stats: monitoring.web_stats,
            meters,
        }
    }

    pub fn meters(&self) -> Arc<RegionMeters> {
        self.meters.clone()
    }
}

pub fn region_router(state: StatusState) -> Router {
    let mut router = Router::new().route("/simstatus", axum::routing::any(simstatus));
    if state.monitor_enabled {
        router = router.route("/monitorstats/{id}", axum::routing::any(monitor));
    }
    if !state.managed_uri.is_empty() {
        let root = format!("/{}", state.managed_uri);
        let slash = format!("/{}/", state.managed_uri);
        let rest = format!("/{}/{{*rest}}", state.managed_uri);
        router = router
            .route(&root, axum::routing::any(managed))
            .route(&slash, axum::routing::any(managed))
            .route(&rest, axum::routing::any(managed));
    }
    if state.web_stats {
        router = router
            .route("/SStats", axum::routing::any(sstats))
            .route("/SStats/{*rest}", axum::routing::any(sstats));
    }
    router.with_state(state)
}

pub fn metrics_router(meters: Arc<RegionMeters>) -> Router {
    Router::new()
        .route("/metrics", get(metrics))
        .route("/diag", get(diag))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(meters)
}

pub fn with_request_counts(router: Router, meters: Arc<RegionMeters>) -> Router {
    router.layer(middleware::from_fn(move |request: Request, next: Next| {
        let meters = meters.clone();
        async move { count_request(meters, request, next).await }
    }))
}

async fn count_request(meters: Arc<RegionMeters>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let response = next.run(request).await;
    let status = response.status().as_u16();
    meters.record_http(route_label(&path, &meters), status);
    if path.starts_with("/CE/") {
        meters.note_event_poll();
    }
    response
}

fn route_label(path: &str, meters: &RegionMeters) -> &'static str {
    let managed = meters
        .managed_prefix
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    if path == "/simstatus" {
        "/simstatus"
    } else if path.starts_with("/monitorstats/") {
        "/monitorstats"
    } else if path == "/SStats" || path.starts_with("/SStats/") {
        "/SStats"
    } else if path.starts_with("/agent/") {
        "/agent"
    } else if path.starts_with("/CE/") {
        "/CE"
    } else if path.starts_with("/CAPS/") {
        "/CAPS"
    } else if !managed.is_empty() && (path == managed || path.starts_with(&format!("{managed}/"))) {
        "/managed-stats"
    } else {
        "other"
    }
}

async fn simstatus() -> Response {
    // SimStatusHandler leaves the content type unset. OSWebServer then sends
    // text/html;charset=UTF-8.
    raw(StatusCode::OK, Some(DEFAULT_HTML), "OK")
}

async fn monitor(
    State(state): State<StatusState>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Response {
    if !monitor_target(&state, &id) {
        return raw(StatusCode::NOT_FOUND, Some(DEFAULT_HTML), Vec::new());
    }
    if request.method() != axum::http::Method::GET {
        return raw(StatusCode::NOT_FOUND, Some(DEFAULT_HTML), Vec::new());
    }
    if let Some(name) = query_first(uri.query(), "monitor") {
        return match monitor_value(&state, &name) {
            Some(value) => raw(StatusCode::OK, Some(DEFAULT_HTML), value),
            None => raw(StatusCode::NOT_FOUND, Some(DEFAULT_HTML), Vec::new()),
        };
    }
    raw(StatusCode::OK, Some("text/xml"), monitor_document(&state))
}

fn monitor_target(state: &StatusState, id: &str) -> bool {
    id == state.region_id.to_string() || percent_decode(id) == state.region_name
}

fn monitor_value(state: &StatusState, name: &str) -> Option<String> {
    let value = monitor_number(state, name)?;
    Some(csharp_real(value))
}

fn monitor_number(state: &StatusState, name: &str) -> Option<f64> {
    if !MONITORS.contains(&name) {
        return None;
    }
    Some(match name {
        "AgentCountMonitor" => state.meters.root_agents.load(Ordering::Relaxed) as f64,
        "ChildAgentCountMonitor" => state.meters.child_agents.load(Ordering::Relaxed) as f64,
        _ => 0.0,
    })
}

fn monitor_document(state: &StatusState) -> String {
    let mut xml = String::from("<data>");
    for name in MONITORS {
        let value = monitor_number(state, name).unwrap_or(0.0);
        let text = csharp_real(value);
        xml.push('<');
        xml.push_str(name);
        xml.push('>');
        xml.push_str(&text);
        xml.push_str("</");
        xml.push_str(name);
        xml.push('>');
    }
    xml.push_str("</data>");
    xml
}

async fn managed(State(state): State<StatusState>, OriginalUri(uri): OriginalUri) -> Response {
    let pass = query_first(uri.query(), "pass").unwrap_or_default();
    if !state.managed_password.is_empty() && pass != state.managed_password {
        let mut response = raw(StatusCode::OK, Some("text/plain"), "Access denied");
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            header::HeaderValue::from_static("*"),
        );
        return response;
    }
    // HandleStatsRequest reads cat, cont and stat independently. A missing
    // name is AllSubCommand ("all"). cont is the container (TB15, #456).
    let category = query_first(uri.query(), "cat").unwrap_or_else(|| "all".to_owned());
    let container = query_first(uri.query(), "cont").unwrap_or_else(|| "all".to_owned());
    let stat = query_first(uri.query(), "stat").unwrap_or_else(|| "all".to_owned());
    let mut body = managed_json(&state, &category, &container, &stat);
    if let Some(callback) = query_first(uri.query(), "callback")
        && callback.len() <= 128
        && !callback.bytes().any(|byte| byte == b'\r' || byte == b'\n')
    {
        body = format!("{callback}({body});");
    }
    let mut response = raw(StatusCode::OK, Some("text/json"), body);
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::HeaderValue::from_static("*"),
    );
    response
}

fn managed_json(state: &StatusState, category: &str, container: &str, stat: &str) -> String {
    if !selector_matches(category, "scene") {
        return "{}".to_owned();
    }
    let mut containers = String::new();
    if selector_matches(container, &state.region_name) {
        let mut stats = String::new();
        let mut first = true;
        for name in MANAGED_STATS {
            if !selector_matches(stat, name) {
                continue;
            }
            if !first {
                stats.push(',');
            }
            first = false;
            let value = managed_value(state, name);
            stats.push_str(&json_string(name));
            stats.push_str(":{\"Value\":");
            stats.push_str(&osd_real(value));
            stats.push('}');
        }
        containers.push_str(&json_string(&state.region_name));
        containers.push_str(":{");
        containers.push_str(&stats);
        containers.push('}');
    }
    format!("{{\"scene\":{{{containers}}}}}")
}

fn managed_value(state: &StatusState, name: &str) -> f64 {
    match name {
        "RootAgents" => state.meters.root_agents.load(Ordering::Relaxed) as f64,
        "ChildAgents" => state.meters.child_agents.load(Ordering::Relaxed) as f64,
        _ => 0.0,
    }
}

fn selector_matches(selector: &str, name: &str) -> bool {
    selector.is_empty() || selector == "all" || selector == name
}

async fn sstats(State(_state): State<StatusState>, OriginalUri(uri): OriginalUri) -> Response {
    let path = uri.path();
    // WebStatsModule keeps paths of length 9 or less (`/SStats` and `/SStats/`)
    // on default.report, and drops the first 8 bytes of a longer path.
    let report = if path.len() > 9 {
        path.get(8..).unwrap_or("")
    } else {
        "default.report"
    };
    if report != "default.report" {
        return raw(StatusCode::NOT_FOUND, Some("text/html"), HTTP_404);
    }
    let json = query_first(uri.query(), "json").is_some();
    if json {
        // Pin5's query uses COALESCE, so an empty table succeeds (TB16, #456).
        // Ferroan has no viewer-stats database, so the idle summary is that
        // zero document. The n_in/n_out alias swap in the C# query is unchanged.
        return raw(StatusCode::OK, Some("text/json"), SSTATS_JSON);
    }
    raw(StatusCode::OK, Some("text/html"), default_report_html())
}

fn default_report_html() -> String {
    // Markers from Default_Report.rep_Default_report_view. The SQLite
    // viewer-stats database is not ported; idle counters stay 0.
    "<HTML><HEAD><STYLE>\nTABLE.defaultr { }\nTR.defaultr { padding: 5px; }\n</STYLE></HEAD><BODY>\
         <TABLE class=\"defaultr\"><TR class=\"defaultr\"><TD class=\"header\"># Users Total</TD>\
         <TD class=\"header\"># Sessions Total</TD><TD class=\"header\">Avg Client FPS</TD>\
         <TD class=\"header\">Avg Client Mem Use</TD><TD class=\"header\">Avg Sim FPS</TD>\
         <TD class=\"header\">Avg Ping</TD><TD class=\"header\">KB Out Total</TD>\
         <TD class=\"header\">KB In Total</TD></TR>\
         <TR class=\"defaultr\"><TD class=\"content\">0</TD><TD class=\"content\">0</TD>\
         <TD class=\"content\">0</TD><TD class=\"content\">0</TD><TD class=\"content\">0</TD>\
         <TD class=\"content\">0</TD><TD class=\"content\">0</TD><TD class=\"content\">0</TD></TR></TABLE></BODY></HTML>"
        .to_string()
}

async fn metrics(State(meters): State<Arc<RegionMeters>>) -> Response {
    raw(
        StatusCode::OK,
        Some("text/plain; version=0.0.4"),
        prometheus(&meters),
    )
}

async fn diag() -> Response {
    raw(
        StatusCode::OK,
        Some("text/plain; charset=utf-8"),
        ferroan_diag::render(),
    )
}

async fn healthz() -> Response {
    raw(StatusCode::OK, Some("text/plain"), "ok\n")
}

async fn readyz(State(meters): State<Arc<RegionMeters>>) -> Response {
    if meters.registered() {
        raw(StatusCode::OK, Some("text/plain"), "ok\n")
    } else {
        raw(
            StatusCode::SERVICE_UNAVAILABLE,
            Some("text/plain"),
            "unregistered\n",
        )
    }
}

fn prometheus(meters: &RegionMeters) -> String {
    let root = meters.root_agents.load(Ordering::Relaxed);
    let child = meters.child_agents.load(Ordering::Relaxed);
    let circuits = meters.circuits.load(Ordering::Relaxed);
    let polls = meters.eq_polls.load(Ordering::Relaxed);
    let events = meters.eq_events.load(Ordering::Relaxed);
    let registered = u64::from(meters.registered());
    let packets_in = meters.udp.packets_in.load(Ordering::Relaxed);
    let bytes_in = meters.udp.bytes_in.load(Ordering::Relaxed);
    let packets_out = meters.udp.packets_out.load(Ordering::Relaxed);
    let bytes_out = meters.udp.bytes_out.load(Ordering::Relaxed);
    let resends = meters.udp.resends.load(Ordering::Relaxed);
    let unacked = meters.udp.unacked.load(Ordering::Relaxed);
    let mut text = String::new();
    gauge(
        &mut text,
        "ferroan_region_root_agents",
        "Root agents in this region.",
        root,
    );
    gauge(
        &mut text,
        "ferroan_region_child_agents",
        "Child agents. A source hop demotes its root into one and keeps the circuit.",
        child,
    );
    gauge(
        &mut text,
        "ferroan_region_circuits",
        "Circuits reserved by an active session.",
        circuits,
    );
    counter(
        &mut text,
        "ferroan_region_udp_packets_in_total",
        "UDP datagrams received.",
        packets_in,
    );
    counter(
        &mut text,
        "ferroan_region_udp_bytes_in_total",
        "UDP payload bytes received.",
        bytes_in,
    );
    counter(
        &mut text,
        "ferroan_region_udp_packets_out_total",
        "UDP datagrams sent.",
        packets_out,
    );
    counter(
        &mut text,
        "ferroan_region_udp_bytes_out_total",
        "UDP payload bytes sent.",
        bytes_out,
    );
    counter(
        &mut text,
        "ferroan_region_udp_resends_total",
        "Reliable UDP payloads resent.",
        resends,
    );
    gauge(
        &mut text,
        "ferroan_region_udp_unacked",
        "Reliable UDP payloads waiting for acknowledgement.",
        unacked,
    );
    counter(
        &mut text,
        "ferroan_region_event_queue_polls_total",
        "Event-queue polls on /CE/.",
        polls,
    );
    counter(
        &mut text,
        "ferroan_region_event_queue_events_total",
        "Event-queue events enqueued. The source hop enqueues TeleportFinish on the agent queue. This counter stays 0.",
        events,
    );
    gauge(
        &mut text,
        "ferroan_region_frame_seconds",
        "Last scene frame time in seconds. Ferroan has no scene frame loop yet, so this stays 0.",
        0,
    );
    gauge(
        &mut text,
        "ferroan_region_frame_rate",
        "Scene frames per second. Ferroan has no scene frame loop yet, so this stays 0.",
        0,
    );
    text.push_str("# HELP ferroan_region_http_requests_total HTTP requests by route and status.\n");
    text.push_str("# TYPE ferroan_region_http_requests_total counter\n");
    let rows = meters
        .http
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    for row in rows.iter() {
        let route = row.route;
        let status = row.status;
        let count = row.count;
        text.push_str(&format!(
            "ferroan_region_http_requests_total{{route=\"{route}\",status=\"{status}\"}} {count}\n"
        ));
    }
    drop(rows);
    gauge(
        &mut text,
        "ferroan_region_registered",
        "1 after the region is registered with the grid.",
        registered,
    );
    text.push_str(&ferroan_diag::metrics());
    text
}

fn gauge(text: &mut String, name: &str, help: &str, value: u64) {
    text.push_str("# HELP ");
    text.push_str(name);
    text.push(' ');
    text.push_str(help);
    text.push('\n');
    text.push_str("# TYPE ");
    text.push_str(name);
    text.push_str(" gauge\n");
    text.push_str(name);
    text.push(' ');
    text.push_str(&value.to_string());
    text.push('\n');
}

fn counter(text: &mut String, name: &str, help: &str, value: u64) {
    text.push_str("# HELP ");
    text.push_str(name);
    text.push(' ');
    text.push_str(help);
    text.push('\n');
    text.push_str("# TYPE ");
    text.push_str(name);
    text.push_str(" counter\n");
    text.push_str(name);
    text.push(' ');
    text.push_str(&value.to_string());
    text.push('\n');
}

fn raw(status: StatusCode, content_type: Option<&str>, body: impl Into<Vec<u8>>) -> Response {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONNECTION, "close");
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder
        .body(Body::from(body.into()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn query_first(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(name) == key {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'+' {
            out.push(b' ');
            index += 1;
            continue;
        }
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            out.push((high << 4) | low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// OSD JSON real: invariant general format, with `.0` when there is no point or exponent.
fn osd_real(value: f64) -> String {
    let text = csharp_real(value);
    if text.contains(['.', 'E', 'e']) {
        text
    } else {
        format!("{text}.0")
    }
}

/// .NET `double.ToString()` general format for the finite values these stats use.
fn csharp_real(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity".to_owned()
        } else {
            "Infinity".to_owned()
        };
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let text = format!("{value}");
    if text.contains(['e', 'E']) {
        return format!("{value:.15}");
    }
    text
}

/// Bind address for the metrics listener. `None` when `[Monitoring] Metrics` is not set.
pub fn metrics_listen(monitoring: &MonitoringSettings) -> Option<SocketAddr> {
    monitoring.metrics_listen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MonitoringSettings;
    use std::net::Ipv4Addr;

    fn state() -> StatusState {
        let meters = Arc::new(RegionMeters::new());
        meters.set_agents(1, 0, 1);
        StatusState::new(
            Uuid::parse_str("6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01").unwrap(),
            "Ferroan Test".to_owned(),
            &MonitoringSettings {
                monitor_enabled: true,
                metrics_listen: Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 9109))),
                managed_stats_uri: "ManagedStats".to_owned(),
                managed_stats_password: "status-test".to_owned(),
                web_stats: true,
            },
            meters,
        )
    }

    #[test]
    fn monitor_xml_lists_every_static_name_and_the_root_agent() {
        let xml = monitor_document(&state());
        let mut cursor = 0;
        for name in MONITORS {
            let start = xml.find(&format!("<{name}>")).unwrap();
            assert!(start >= cursor, "{name}");
            cursor = start;
        }
        assert!(xml.contains("<AgentCountMonitor>1</AgentCountMonitor>"));
        assert!(xml.contains("<ChildAgentCountMonitor>0</ChildAgentCountMonitor>"));
        assert!(xml.contains("<GCMemoryMonitor>0</GCMemoryMonitor>"));
        assert!(xml.starts_with("<data>"));
        assert!(xml.ends_with("</data>"));
    }

    #[test]
    fn managed_stats_omit_a_missing_name_and_read_cat_for_cont() {
        let body = managed_json(&state(), "all", "all", "all");
        assert!(body.contains("\"RootAgents\":{\"Value\":1.0}"));
        assert!(body.contains("\"ChildAgents\":{\"Value\":0.0}"));
        assert!(!body.contains("GCMemory"));
        let filtered = managed_json(&state(), "scene", "Ferroan Test", "RootAgents");
        assert!(filtered.contains("\"RootAgents\":{\"Value\":1.0}"));
        assert!(!filtered.contains("\"ChildAgents\""));
        // A container that is not the region name stays empty. The captured
        // body used cont=not-the-region.
        let wrong_container = managed_json(&state(), "scene", "not-the-region", "RootAgents");
        assert_eq!(wrong_container, "{\"scene\":{}}");
        let named = managed_json(&state(), "all", "Ferroan Test", "RootAgents");
        assert!(named.contains("\"RootAgents\":{\"Value\":1.0}"));
        let absent = managed_json(&state(), "all", "all", "DoesNotExist");
        assert!(absent.contains("\"Ferroan Test\":{}"));
        let wrong_category = managed_json(&state(), "nope", "all", "all");
        assert_eq!(wrong_category, "{}");
    }

    #[test]
    fn prometheus_text_names_the_region_series() {
        let meters = Arc::new(RegionMeters::new());
        meters.set_agents(1, 0, 1);
        meters.set_registered(true);
        let text = prometheus(&meters);
        assert!(text.contains("ferroan_region_root_agents 1\n"));
        assert!(text.contains("ferroan_region_child_agents 0\n"));
        assert!(text.contains("ferroan_region_frame_seconds 0\n"));
        assert!(text.contains("ferroan_region_frame_rate 0\n"));
        assert!(text.contains("ferroan_region_registered 1\n"));
        assert!(text.contains("# TYPE ferroan_region_udp_resends_total counter\n"));
    }

    #[tokio::test]
    async fn simstatus_and_readyz_follow_registration() {
        let state = state();
        let meters = state.meters();
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let app = region_router(state.clone()).merge(metrics_router(meters.clone()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let client = reqwest::Client::new();
        let base = format!("http://{address}");
        let sim = client
            .get(format!("{base}/simstatus"))
            .send()
            .await
            .unwrap();
        assert_eq!(sim.status(), reqwest::StatusCode::OK);
        assert_eq!(sim.text().await.unwrap(), "OK");
        let ready = client.get(format!("{base}/readyz")).send().await.unwrap();
        assert_eq!(ready.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        meters.set_registered(true);
        let ready = client.get(format!("{base}/readyz")).send().await.unwrap();
        assert_eq!(ready.status(), reqwest::StatusCode::OK);
        let denied = client
            .get(format!("{base}/ManagedStats/"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), reqwest::StatusCode::OK);
        assert_eq!(denied.text().await.unwrap(), "Access denied");
        let missing = client
            .get(format!(
                "{base}/monitorstats/6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01?monitor=DoesNotExist"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
        let unknown = client
            .get(format!("{base}/SStats/no-such-report"))
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);
        assert_eq!(unknown.text().await.unwrap(), HTTP_404);
        let missing_cat = client
            .get(format!(
                "{base}/ManagedStats/fetch?pass=status-test&cont=scene"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(missing_cat.status(), reqwest::StatusCode::OK);
        assert_eq!(missing_cat.text().await.unwrap(), "{\"scene\":{}}");
        let named = client
            .get(format!(
                "{base}/ManagedStats/fetch?pass=status-test&cont=Ferroan%20Test"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(named.status(), reqwest::StatusCode::OK);
        assert!(named.text().await.unwrap().contains("RootAgents"));
    }

    #[test]
    fn captured_status_forms_match() {
        assert_eq!(
            include_bytes!("../../../fixtures/status/simstatus.body"),
            b"OK"
        );
        assert_eq!(
            include_str!("../../../fixtures/status/managed-denied.body"),
            "Access denied"
        );
        assert_eq!(
            include_str!("../../../fixtures/status/managed-cont.body"),
            managed_json(&state(), "scene", "not-the-region", "RootAgents")
        );
        assert_eq!(
            include_str!("../../../fixtures/status/http404.body"),
            HTTP_404
        );
        let monitor = include_str!("../../../fixtures/status/monitor.xml");
        let mut cursor = 0;
        for name in MONITORS {
            let start = monitor.find(&format!("<{name}>")).expect(name);
            assert!(start >= cursor, "{name}");
            cursor = start;
        }
        let captured = include_str!("../../../fixtures/status/sstats-json.json").trim_end();
        assert_eq!(captured.replace("\"1\"", "\"0\""), SSTATS_JSON);
        let page = include_str!("../../../fixtures/status/sstats.html");
        let idle = default_report_html();
        for marker in [
            "# Users Total",
            "# Sessions Total",
            "Avg Client FPS",
            "Avg Client Mem Use",
            "Avg Sim FPS",
            "Avg Ping",
            "KB Out Total",
            "KB In Total",
            "TABLE.defaultr",
            "class=\"defaultr\"",
        ] {
            assert!(page.contains(marker), "{marker}");
            assert!(idle.contains(marker), "{marker}");
        }
        let names: Vec<&str> = include_str!("../../../fixtures/status/scene-stats.txt")
            .lines()
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(names.as_slice(), MANAGED_STATS);
    }
}
