// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The seed capability and one event queue.
//!
//! The seed follows Tranquillity `BunchOfCaps.SeedCapRequest`: a POST whose
//! `Content-Type` is `application/llsd+xml`, with an LLSD array of names, gets
//! an LLSD map. Anything else is 404 or 400, as that method does. The reply
//! here includes only `EventQueueGet`. The other names the bot asks for are
//! later rows. The activation wait (503) and the client check (403) are not
//! done. CapsRegistry ties route lifetime to an accepted agent; it does not
//! yet perform those activation or client-address checks.
//!
//! `EventQueueGetModule.GetEvents` puts queued messages in an LLSD map of
//! `events` and `id`, with content type `application/xml`. An empty queue is
//! `NoEvents`: 502 when the agent has a queue, otherwise 404, and the poll is
//! held. This row asks for an empty event set and a new id, so a poll is
//! answered at once with an empty `events` array, the next id, and informal
//! LLSD XML (`application/llsd+xml`). Ids start at 1. Tranquillity draws the
//! first id from `Random.Shared.Next(30000000)`.
//!
//! Read from
//!   Source/OpenSim.Region.ClientStack.LindenCaps/BunchOfCaps/BunchOfCaps.cs
//!   SeedCapRequest
//!   Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/EventQueueGetModule.cs
//!   GetEvents, NoEvents
//!   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
//! `EventQueueGetModule.cs` is unchanged from `713857f`. `SeedCapRequest` is
//! unchanged; the file differs elsewhere.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ferroan_http::{llsd_from_http_body, llsd_response, read_body};
use ferroan_llsd::Llsd;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Seed and poll bodies larger than this are refused. A seed is a short list
/// of capability names.
const SEED_BODY_LIMIT: usize = 64 * 1024;
/// Names in one seed POST. A viewer asks for well under this.
const MAX_SEED_NAMES: usize = 256;
/// One capability name. The bot's names are shorter.
const MAX_SEED_NAME_LEN: usize = 128;

/// Failure to bind the capability listener.
#[derive(Debug, thiserror::Error)]
pub enum CapsError {
    #[error("caps listen: {0}")]
    Bind(#[from] std::io::Error),
    #[error("invalid capability base URL or seed path")]
    InvalidAddress,
    #[error("this region already has an active capability session")]
    Occupied,
}

#[derive(Clone)]
struct CapsState {
    event_queue_url: String,
    next_id: Arc<AtomicI32>,
}

#[derive(Clone)]
struct RegisteredCaps {
    seed_path: String,
    events_path: String,
    state: CapsState,
}

/// One agent's capabilities on the region HTTP listener.
///
/// Registration uses the seed path supplied by Robust during CreateAgent.
/// The owning circuit removes the routes when the agent leaves. The existing
/// seed and event-queue behavior is shared with [`CapsServer`].
#[derive(Clone)]
pub struct CapsRegistry {
    base_url: String,
    active: Arc<Mutex<Option<RegisteredCaps>>>,
}

impl CapsRegistry {
    pub fn new(base_url: &str) -> Result<Self, CapsError> {
        let uri: axum::http::Uri = base_url.parse().map_err(|_| CapsError::InvalidAddress)?;
        if base_url.len() > 2048
            || !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_none()
            || uri.path() != "/"
            || uri.query().is_some()
        {
            return Err(CapsError::InvalidAddress);
        }
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            active: Arc::new(Mutex::new(None)),
        })
    }

    pub fn register(&self, seed_path: &str) -> Result<(), CapsError> {
        if seed_path.len() > 512
            || !seed_path.starts_with("/CAPS/")
            || !seed_path.ends_with("0000/")
            || !seed_path
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-'))
        {
            return Err(CapsError::InvalidAddress);
        }
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if active.is_some() {
            return Err(CapsError::Occupied);
        }
        let events_path = format!("/CE/{}", Uuid::new_v4());
        *active = Some(RegisteredCaps {
            seed_path: seed_path.to_owned(),
            state: CapsState {
                event_queue_url: format!("{}{events_path}", self.base_url),
                next_id: Arc::new(AtomicI32::new(1)),
            },
            events_path,
        });
        Ok(())
    }

    pub fn clear(&self) {
        *self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    }

    /// Merge this fallback with the region's more specific simulation routes.
    pub fn router(&self) -> Router {
        Router::new()
            .fallback(registered_caps)
            .with_state(self.clone())
    }
}

async fn registered_caps(State(registry): State<CapsRegistry>, request: Request) -> Response {
    let active = registry
        .active
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    let Some(active) = active else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let path = request.uri().path();
    let is_seed = path == active.seed_path;
    if !is_seed && path != active.events_path {
        return StatusCode::NOT_FOUND.into_response();
    }
    if request.method() != axum::http::Method::POST {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(axum::http::header::ALLOW, "POST")],
        )
            .into_response();
    }
    if is_seed {
        seed(State(active.state), request).await
    } else {
        poll(State(active.state), request).await
    }
}

/// HTTP seed and event queue on `127.0.0.1`.
///
/// Dropping the server stops the listener.
pub struct CapsServer {
    seed_url: String,
    event_queue_url: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for CapsServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl CapsServer {
    /// Bind `127.0.0.1` on a free port and serve `/seed` and one event queue.
    pub async fn bind() -> Result<Self, CapsError> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let local_addr = listener.local_addr()?;
        let events_path = format!("/CE/{}", Uuid::new_v4());
        let seed_url = format!("http://{local_addr}/seed");
        let event_queue_url = format!("http://{local_addr}{events_path}");
        let state = CapsState {
            event_queue_url: event_queue_url.clone(),
            next_id: Arc::new(AtomicI32::new(1)),
        };
        let app = Router::new()
            .route("/seed", post(seed))
            .route(&events_path, post(poll))
            .layer(DefaultBodyLimit::max(SEED_BODY_LIMIT))
            .with_state(state);
        let (shutdown, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        tokio::task::yield_now().await;
        Ok(Self {
            seed_url,
            event_queue_url,
            shutdown: Some(shutdown),
        })
    }

    /// URL the bot POSTs in `run_until_ready`.
    pub fn seed_url(&self) -> &str {
        &self.seed_url
    }

    /// `EventQueueGet` URL returned by the seed.
    pub fn event_queue_url(&self) -> &str {
        &self.event_queue_url
    }
}

fn is_seed_content_type(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        == Some("application/llsd+xml")
}

async fn seed(State(state): State<CapsState>, request: Request) -> Response {
    if !is_seed_content_type(request.headers()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let bytes = match read_body(request.into_body(), SEED_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let value = match llsd_from_http_body(&bytes) {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };
    let Llsd::Array(names) = value else {
        return (StatusCode::BAD_REQUEST, "seed body is not an LLSD array").into_response();
    };
    let want = match wants_event_queue(&names) {
        Ok(want) => want,
        Err(reason) => return (StatusCode::BAD_REQUEST, reason).into_response(),
    };
    let mut map = Llsd::new_map();
    if want {
        map.insert("EventQueueGet", state.event_queue_url);
    }
    match llsd_response(
        &map,
        Some("application/llsd+xml"),
        Some("application/llsd+xml"),
    ) {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn poll(State(state): State<CapsState>, request: Request) -> Response {
    let bytes = match read_body(request.into_body(), SEED_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    if !bytes.is_empty() && !matches!(llsd_from_http_body(&bytes), Ok(Llsd::Map(_))) {
        return (StatusCode::BAD_REQUEST, "event poll is not an LLSD map").into_response();
    }
    let Some(id) = next_event_id(&state.next_id) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut map = Llsd::new_map();
    map.insert("events", Llsd::new_array());
    map.insert("id", id);
    match llsd_response(&map, None, None) {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

/// `true` when the viewer asked for `EventQueueGet`.
///
/// Empty names are skipped, as `SeedCapRequest` skips an empty `AsString`.
/// Other names are ignored: this server has no URL for them.
fn wants_event_queue(names: &[Llsd]) -> Result<bool, &'static str> {
    if names.len() > MAX_SEED_NAMES {
        return Err("seed lists too many names");
    }
    let mut want = false;
    for name in names {
        let Llsd::String(text) = name else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        if text.len() > MAX_SEED_NAME_LEN {
            return Err("seed name is too long");
        }
        if text == "EventQueueGet" {
            want = true;
        }
    }
    Ok(want)
}

fn next_event_id(next: &AtomicI32) -> Option<i32> {
    let id = next.fetch_add(1, Ordering::Relaxed);
    if id <= 0 { None } else { Some(id) }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ferroan_llsd::{Llsd, from_xml, to_xml};

    use super::*;
    use crate::circuit::{CircuitExpect, CircuitServer};

    fn name_list<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
        let mut array = Llsd::new_array();
        for name in names {
            array.push(name);
        }
        to_xml(&array)
    }

    async fn post(url: &str, content_type: &str, body: Vec<u8>) -> (u16, Vec<u8>, Option<String>) {
        let http = ferroan_bot::login::http_client().unwrap();
        let response = http
            .post(url)
            .header("content-type", content_type)
            .body(body)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await.unwrap().to_vec();
        (status, bytes, content_type)
    }

    fn event_id(bytes: &[u8]) -> i32 {
        let value = from_xml(bytes).unwrap();
        assert!(value.get("events").is_array());
        assert_eq!(value.get("events").len(), 0);
        assert!(value.has("id"));
        value.get("id").as_i32()
    }

    #[tokio::test]
    async fn registered_seed_uses_the_region_listener_and_expires_with_the_agent() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let registry = CapsRegistry::new(&base).unwrap();
        let app = registry.router();
        let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let path = format!("/CAPS/{}0000/", Uuid::from_u128(10));
        let seed_url = format!("{}{}", base.trim_end_matches('/'), path);
        assert_eq!(
            post(
                &seed_url,
                "application/llsd+xml",
                name_list(["EventQueueGet"])
            )
            .await
            .0,
            404
        );
        registry.register(&path).unwrap();
        assert!(matches!(registry.register(&path), Err(CapsError::Occupied)));
        let (status, bytes, _) = post(
            &seed_url,
            "application/llsd+xml",
            name_list(["EventQueueGet"]),
        )
        .await;
        assert_eq!(status, 200);
        let map = from_xml(&bytes).unwrap();
        let events = map.get("EventQueueGet").as_str().to_owned();
        assert!(events.starts_with(&format!("{base}CE/")));
        assert_eq!(
            post(&events, "application/llsd+xml", Vec::new()).await.0,
            200
        );
        registry.clear();
        assert_eq!(
            post(&seed_url, "application/llsd+xml", Vec::new()).await.0,
            404
        );
        assert_eq!(
            post(&events, "application/llsd+xml", Vec::new()).await.0,
            404
        );
        registry.register(&path).unwrap();
        assert_eq!(
            post(
                &seed_url,
                "application/llsd+xml",
                name_list(["EventQueueGet"])
            )
            .await
            .0,
            200
        );
        serving.abort();
    }

    #[tokio::test]
    async fn the_seed_returns_event_queue_get_and_an_empty_poll_has_a_new_id() {
        let server = CapsServer::bind().await.unwrap();
        let (status, body, content_type) = post(
            server.seed_url(),
            "application/llsd+xml",
            name_list([
                "EventQueueGet",
                "GetTexture",
                "FetchInventoryDescendents2",
                "FetchInventory2",
                "NewFileAgentInventory",
            ]),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(content_type.as_deref(), Some("application/llsd+xml"));
        let map = from_xml(&body).unwrap();
        assert_eq!(map.get("EventQueueGet").as_str(), server.event_queue_url());
        assert!(!map.has("GetTexture"));
        assert_eq!(map.len(), 1);

        let (status, body, _) =
            post(server.event_queue_url(), "application/llsd+xml", Vec::new()).await;
        assert_eq!(status, 200);
        assert_eq!(event_id(&body), 1);

        let mut ack = Llsd::new_map();
        ack.insert("ack", 0);
        ack.insert("done", false);
        let (status, body, content_type) = post(
            server.event_queue_url(),
            "application/llsd+xml",
            to_xml(&ack),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(content_type.as_deref(), Some("application/llsd+xml"));
        assert_eq!(event_id(&body), 2);
    }

    #[tokio::test]
    async fn a_bad_seed_or_poll_is_refused() {
        let server = CapsServer::bind().await.unwrap();
        let (status, _, _) = post(
            server.seed_url(),
            "text/plain",
            name_list(["EventQueueGet"]),
        )
        .await;
        assert_eq!(status, 404);

        let (status, _, _) = post(
            server.seed_url(),
            "application/llsd+xml",
            to_xml(&Llsd::new_map()),
        )
        .await;
        assert_eq!(status, 400);

        let long = "n".repeat(MAX_SEED_NAME_LEN + 1);
        let (status, _, _) = post(
            server.seed_url(),
            "application/llsd+xml",
            name_list([long.as_str()]),
        )
        .await;
        assert_eq!(status, 400);

        let too_many = vec!["EventQueueGet"; MAX_SEED_NAMES + 1];
        let (status, _, _) = post(
            server.seed_url(),
            "application/llsd+xml",
            name_list(too_many),
        )
        .await;
        assert_eq!(status, 400);

        let (status, _, _) = post(
            server.seed_url(),
            "application/llsd+xml",
            vec![b'x'; SEED_BODY_LIMIT + 1],
        )
        .await;
        assert_eq!(status, 413);

        let http = ferroan_bot::login::http_client().unwrap();
        let response = http.get(server.seed_url()).send().await.unwrap();
        assert_eq!(response.status().as_u16(), 405);

        let (status, body, _) =
            post(server.event_queue_url(), "application/llsd+xml", Vec::new()).await;
        assert_eq!(status, 200);
        assert_eq!(event_id(&body), 1);
        let (status, _, _) = post(
            server.event_queue_url(),
            "application/llsd+xml",
            to_xml(&Llsd::new_array()),
        )
        .await;
        assert_eq!(status, 400);
        let (status, body, _) =
            post(server.event_queue_url(), "application/llsd+xml", Vec::new()).await;
        assert_eq!(status, 200);
        assert_eq!(event_id(&body), 2);
    }

    #[tokio::test]
    async fn the_bot_fetches_the_seed_during_the_handshake() {
        let caps = CapsServer::bind().await.unwrap();
        let circuit = CircuitServer::bind().await.unwrap();
        let addr = circuit.local_addr().unwrap();
        let expect = CircuitExpect {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
            region_id: Uuid::from_u128(9),
        };
        let running =
            tokio::spawn(async move { circuit.serve_one(expect, Duration::from_secs(20)).await });
        let http = ferroan_bot::login::http_client().unwrap();
        let mut session = ferroan_bot::Session::connect(
            addr,
            ferroan_bot::Agent {
                agent_id: expect.agent_id,
                session_id: expect.session_id,
                circuit_code: expect.circuit_code,
            },
        )
        .await
        .unwrap();
        session
            .run_until_ready(&http, caps.seed_url(), Duration::from_secs(8))
            .await
            .unwrap();
        assert_eq!(session.region_id(), Some(expect.region_id));
        assert_eq!(
            session.caps().get("EventQueueGet").map(String::as_str),
            Some(caps.event_queue_url())
        );
        assert_eq!(session.caps().len(), 1);

        let (status, body, _) =
            post(caps.event_queue_url(), "application/llsd+xml", Vec::new()).await;
        assert_eq!(status, 200);
        assert_eq!(event_id(&body), 1);

        session.logout(Duration::from_secs(8)).await.unwrap();
        running.await.unwrap().unwrap();
    }
}
