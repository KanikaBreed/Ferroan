// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Robust status forms and the Prometheus listener.
//!
//! Asset, account and alias connectors register no status route. `/get_grid_info`,
//! `/json_grid_info` and `/get_grid_stats` follow
//! `Source/OpenSim.Server.Handlers/Grid/GridInfoHandlers.cs` when
//! `[GridInfoService]` is present. `/helo` follows `HeloServerGetAndHeadHandler`
//! when `[HeloService]` is present. C# loads that connector from `ServiceList`;
//! this process has no service list, so the section is the switch.
//! `/metrics`, `/healthz` and `/readyz` are off unless `[Monitoring] Metrics`
//! is set. Logging stays on P0.13.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use axum::{Router, middleware};
use ferroan_config::Config;
use ferroan_services::{AccountHttp, AliasHttp, AssetHttp};

const BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];

/// Content type OSWebServer applies when a handler sets none.
const DEFAULT_HTML: &str = "text/html;charset=UTF-8";

struct HttpCount {
    service: &'static str,
    route: String,
    status: u16,
    count: u64,
}

struct Histogram {
    buckets: [AtomicU64; 10],
    sum_nanos: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            sum_nanos: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    fn observe(&self, seconds: f64) {
        let nanos = seconds.max(0.0) * 1_000_000_000.0;
        self.sum_nanos.fetch_add(nanos as u64, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        for (index, limit) in BUCKETS.iter().enumerate() {
            if seconds <= *limit {
                self.buckets[index].fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Pools {
    asset: Option<AssetHttp>,
    account: Option<AccountHttp>,
    alias: Option<AliasHttp>,
}

impl Pools {
    pub fn asset(mut self, http: AssetHttp) -> Self {
        self.asset = Some(http);
        self
    }

    pub fn account(mut self, http: AccountHttp) -> Self {
        self.account = Some(http);
        self
    }

    pub fn alias(mut self, http: AliasHttp) -> Self {
        self.alias = Some(http);
        self
    }
}

pub struct RobustMeters {
    http: Mutex<Vec<HttpCount>>,
    histogram: Histogram,
    pools: Pools,
}

impl RobustMeters {
    pub fn new(pools: Pools) -> Self {
        Self {
            http: Mutex::new(Vec::new()),
            histogram: Histogram::new(),
            pools,
        }
    }

    /// Pool occupancy for a diagnostic dump. Passwords and connection strings stay out.
    pub fn diag_lines(&self) -> String {
        let mut text = String::new();
        for (name, http) in [
            (
                "asset",
                self.pools.asset.as_ref().map(AssetHttp::pool_occupancy),
            ),
            (
                "account",
                self.pools.account.as_ref().map(AccountHttp::pool_occupancy),
            ),
            (
                "alias",
                self.pools.alias.as_ref().map(AliasHttp::pool_occupancy),
            ),
        ] {
            match http {
                Some((open, idle)) => {
                    text.push_str(&format!("{name}: open={open} idle={idle}\n"));
                }
                None => text.push_str(&format!("{name}: off\n")),
            }
        }
        text
    }
}

pub fn metrics_listen(config: &Config) -> Result<Option<SocketAddr>, String> {
    let enabled = match config.get("Monitoring", "Metrics") {
        None => false,
        Some(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => return Err("[Monitoring] Metrics: expected true or false".to_owned()),
        },
    };
    if !enabled {
        return Ok(None);
    }
    match config
        .get("Monitoring", "Listen")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        None => Ok(Some(SocketAddr::from((
            std::net::Ipv4Addr::LOCALHOST,
            9110,
        )))),
        Some(text) => text
            .parse()
            .map(Some)
            .map_err(|_| "[Monitoring] Listen: expected host:port".to_owned()),
    }
}

pub fn grid_info_enabled(config: &Config) -> bool {
    config.section_names().any(|name| name == "GridInfoService")
}

pub fn helo_enabled(config: &Config) -> bool {
    config.section_names().any(|name| name == "HeloService")
}

pub fn with_request_counts(router: Router, meters: Arc<RobustMeters>) -> Router {
    router.layer(middleware::from_fn(move |request: Request, next: Next| {
        let meters = meters.clone();
        async move {
            let started = Instant::now();
            let path = request.uri().path().to_owned();
            // Axum sets this before the per-route layer runs. A fallback has none,
            // and every unmatched path shares one row.
            let matched = request
                .extensions()
                .get::<MatchedPath>()
                .map(|matched| matched.as_str().to_owned());
            let (service, route) = match matched {
                Some(route) => (service_label(&path), route),
                None => ("other", "other".to_owned()),
            };
            let response = next.run(request).await;
            let status = response.status().as_u16();
            record(&meters, service, route, status);
            meters.histogram.observe(started.elapsed().as_secs_f64());
            response
        }
    }))
}

fn record(meters: &RobustMeters, service: &'static str, route: String, status: u16) {
    let mut rows = meters
        .http
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(row) = rows
        .iter_mut()
        .find(|row| row.service == service && row.route == route && row.status == status)
    {
        row.count = row.count.saturating_add(1);
        return;
    }
    if rows.len() < 128 {
        rows.push(HttpCount {
            service,
            route,
            status,
            count: 1,
        });
    }
}

fn service_label(path: &str) -> &'static str {
    if path.starts_with("/assets") {
        "asset"
    } else if path.starts_with("/accounts") {
        "account"
    } else if path.starts_with("/useralias") {
        "alias"
    } else if path.starts_with("/get_grid_") || path.starts_with("/json_grid_info") {
        "gridinfo"
    } else if path == "/helo" {
        "helo"
    } else {
        "other"
    }
}

#[derive(Clone)]
struct StatusApp {
    info: Vec<(String, String)>,
    helo: String,
}

pub fn status_router(config: &Config) -> Router {
    let app = StatusApp {
        info: grid_info(config),
        helo: config
            .get("HeloService", "Handlers")
            .unwrap_or("opensim-robust")
            .to_owned(),
    };
    let mut router = Router::new();
    if grid_info_enabled(config) {
        router = router
            .route("/get_grid_info", get(grid_xml))
            .route("/json_grid_info", get(grid_json));
        let disabled = matches!(
            config
                .get("GridInfoService", "DisableStatsEndpoint")
                .map(|text| text.trim().eq_ignore_ascii_case("true")),
            Some(true)
        );
        if !disabled {
            router = router.route("/get_grid_stats", get(grid_stats_unavailable));
        }
    }
    if helo_enabled(config) {
        router = router.route("/helo", axum::routing::any(helo));
    }
    router.with_state(app)
}

pub fn metrics_router(meters: Arc<RobustMeters>) -> Router {
    Router::new()
        .route("/metrics", get(metrics))
        .route("/diag", get(diag))
        .route("/healthz", get(|| async { plain(StatusCode::OK, "ok\n") }))
        .route("/readyz", get(|| async { plain(StatusCode::OK, "ok\n") }))
        .with_state(meters)
}

async fn diag() -> Response {
    typed(
        StatusCode::OK,
        "text/plain; charset=utf-8",
        ferroan_diag::render(),
    )
}

fn grid_info(config: &Config) -> Vec<(String, String)> {
    let mut pairs = vec![("platform".to_owned(), "OpenSim".to_owned())];
    for (key, value) in config.keys("GridInfoService") {
        if let Some(existing) = pairs.iter_mut().find(|(name, _)| name == key) {
            existing.1 = value.to_owned();
        } else {
            pairs.push((key.to_owned(), value.to_owned()));
        }
    }
    // Later sections override, matching Util.GetConfigVarFromSections.
    for (key, ini_key) in [
        ("home", "HomeURI"),
        ("homealias", "HomeURIAlias"),
        ("gatekeeper", "GatekeeperURI"),
        ("gatekeeperalias", "GatekeeperURIAlias"),
    ] {
        for section in ["Startup", "Hypergrid"] {
            if let Some(value) = config
                .get(section, ini_key)
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                upsert(&mut pairs, key, value);
            }
        }
    }
    pairs
}

fn upsert(pairs: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(existing) = pairs.iter_mut().find(|(name, _)| name == key) {
        existing.1 = value.to_owned();
    } else {
        pairs.push((key.to_owned(), value.to_owned()));
    }
}

fn grid_document_xml(info: &[(String, String)]) -> String {
    let mut body = String::from("<gridinfo>");
    for (key, value) in info {
        body.push('<');
        body.push_str(key);
        body.push('>');
        body.push_str(&xml_escape(value));
        body.push_str("</");
        body.push_str(key);
        body.push('>');
    }
    body.push_str("</gridinfo>");
    body
}

fn grid_document_json(info: &[(String, String)]) -> String {
    let mut body = String::from("{");
    for (index, (key, value)) in info.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        body.push_str(&json_string(key));
        body.push(':');
        body.push_str(&json_string(value));
    }
    body.push('}');
    body
}

async fn grid_xml(State(app): State<StatusApp>) -> Response {
    typed(
        StatusCode::OK,
        "application/xml",
        grid_document_xml(&app.info),
    )
}

async fn grid_json(State(app): State<StatusApp>) -> Response {
    typed(
        StatusCode::OK,
        "application/json",
        grid_document_json(&app.info),
    )
}

async fn grid_stats_unavailable() -> Response {
    // stats_available is false without GridService plus IGridUserData.
    // The handler sets no content type; OSWebServer fills the default.
    typed(StatusCode::METHOD_NOT_ALLOWED, DEFAULT_HTML, "")
}

async fn helo(State(app): State<StatusApp>, request: Request) -> Response {
    let method = request.method();
    if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
        return typed(StatusCode::METHOD_NOT_ALLOWED, DEFAULT_HTML, "");
    }
    // GET and HEAD stay keep-alive. The handler only adds X-Handlers-Provided.
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(DEFAULT_HTML));
    if let Ok(value) = HeaderValue::from_str(&app.helo) {
        response.headers_mut().insert("X-Handlers-Provided", value);
    }
    response
}

async fn metrics(State(meters): State<Arc<RobustMeters>>) -> Response {
    typed(
        StatusCode::OK,
        "text/plain; version=0.0.4",
        prometheus(&meters),
    )
}

fn prometheus(meters: &RobustMeters) -> String {
    let mut text = String::from(
        "# HELP ferroan_robust_http_requests_total HTTP requests by service, route and status.\n# TYPE ferroan_robust_http_requests_total counter\n",
    );
    let rows = meters
        .http
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    for row in rows.iter() {
        text.push_str(&format!(
            "ferroan_robust_http_requests_total{{service=\"{}\",route=\"{}\",status=\"{}\"}} {}\n",
            row.service,
            escape_label(&row.route),
            row.status,
            row.count
        ));
    }
    drop(rows);
    text.push_str("# HELP ferroan_robust_request_seconds HTTP request latency.\n# TYPE ferroan_robust_request_seconds histogram\n");
    for (index, limit) in BUCKETS.iter().enumerate() {
        let count = meters.histogram.buckets[index].load(Ordering::Relaxed);
        text.push_str(&format!(
            "ferroan_robust_request_seconds_bucket{{le=\"{limit}\"}} {count}\n"
        ));
    }
    let total = meters.histogram.count.load(Ordering::Relaxed);
    let sum_nanos = meters.histogram.sum_nanos.load(Ordering::Relaxed);
    text.push_str(&format!(
        "ferroan_robust_request_seconds_bucket{{le=\"+Inf\"}} {total}\n"
    ));
    let sum = sum_nanos as f64 / 1_000_000_000.0;
    text.push_str(&format!("ferroan_robust_request_seconds_sum {sum}\n"));
    text.push_str(&format!("ferroan_robust_request_seconds_count {total}\n"));
    text.push_str("# HELP ferroan_robust_db_pool_connections Database pool occupancy.\n# TYPE ferroan_robust_db_pool_connections gauge\n");
    for (name, http) in [
        (
            "asset",
            meters.pools.asset.as_ref().map(AssetHttp::pool_occupancy),
        ),
        (
            "account",
            meters
                .pools
                .account
                .as_ref()
                .map(AccountHttp::pool_occupancy),
        ),
        (
            "alias",
            meters.pools.alias.as_ref().map(AliasHttp::pool_occupancy),
        ),
    ] {
        if let Some((open, idle)) = http {
            text.push_str(&format!(
                "ferroan_robust_db_pool_connections{{service=\"{name}\",state=\"open\"}} {open}\n"
            ));
            text.push_str(&format!(
                "ferroan_robust_db_pool_connections{{service=\"{name}\",state=\"idle\"}} {idle}\n"
            ));
        }
    }
    text.push_str(&ferroan_diag::metrics());
    text
}

fn plain(status: StatusCode, body: &'static str) -> Response {
    typed(status, "text/plain", body)
}

fn typed(status: StatusCode, content_type: &str, body: impl Into<axum::body::Body>) -> Response {
    let mut response = Response::new(body.into());
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("text/plain")),
    );
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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

fn escape_label(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_info_matches_the_captured_grid() {
        let config = Config::parse(
            "[GridInfoService]\n\
             login = http://127.0.0.1:8002/\n\
             gridname = \"Ferroan Test Grid\"\n\
             gridnick = ferroan\n",
        )
        .expect("ini");
        let info = grid_info(&config);
        assert_eq!(
            grid_document_xml(&info),
            include_str!("../../../fixtures/status/grid-info.xml")
        );
        assert_eq!(
            grid_document_json(&info),
            include_str!("../../../fixtures/status/grid-json.json")
        );
    }

    #[tokio::test]
    async fn junk_paths_share_one_row_and_a_known_route_is_still_counted() {
        let config =
            Config::parse("[GridInfoService]\nlogin = http://127.0.0.1:8002/\n").expect("ini");
        let router = status_router(&config).route("/assets/{id}/data", get(|| async { "ok" }));
        let meters = Arc::new(RobustMeters::new(Pools::default()));
        let app = with_request_counts(router, Arc::clone(&meters));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind");
        let address = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        for index in 0..160 {
            http_get(address, &format!("/junk-{index}")).await;
        }
        http_get(
            address,
            "/assets/6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01/missing",
        )
        .await;
        http_get(address, "/assets/6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01/data").await;
        http_get(address, "/get_grid_info").await;
        server.abort();

        let rows = meters.http.lock().expect("rows");
        let count = |route: &str| -> u64 {
            rows.iter()
                .filter(|row| row.route == route)
                .map(|row| row.count)
                .sum()
        };
        assert_eq!(count("other"), 161);
        assert_eq!(count("/assets/{id}/data"), 1);
        assert_eq!(count("/get_grid_info"), 1);
        assert_eq!(rows.len(), 3);
        assert!(rows.len() < 128);
    }

    async fn http_get(address: std::net::SocketAddr, path: &str) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect");
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut body = Vec::new();
        stream.read_to_end(&mut body).await.expect("read");
        assert!(
            body.windows(5).any(|window| window == b"HTTP/"),
            "{path} got no status line"
        );
    }
}
