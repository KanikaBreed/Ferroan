// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//

// Ported from BunchOfCaps/BunchOfCaps.cs (SeedCapRequest) and
// EventQueue/EventQueueGetModule.cs in Source/OpenSim.Region.ClientStack.LindenCaps,
// and Source/OpenSim.Framework.Servers.HttpServer/{BaseHttpServer,PollServiceHttpRequest}.cs,
// plus OSHttpServer/HttpResponse.cs there (default MIME and error connection close),
// OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Seed discovery and bounded long-poll queues for one accepted agent.
//! Seed method/type checks and OSD.AsString capability lookup follow C#.
//! Activation waiting and client-address checks remain unimplemented; registry
//! lifetime follows the accepted circuit. Empty polls hold 58 seconds then
//! return 502, while removal wakes them with 404. Event delivery advances one
//! id per batch. See event_queue for crossing-marker limitations and bounds.

use std::sync::{Arc, Mutex};

use crate::event_queue::{EventQueue, EventQueueError, PollReply};
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use ferroan_http::read_body;
use ferroan_llsd::{Llsd, from_xml, to_xml};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use uuid::Uuid;

const SEED_MIME: &str = "text/html;charset=UTF-8";

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
    queue: EventQueue,
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
                queue: EventQueue::new(),
            },
            events_path,
        });
        Ok(())
    }

    pub fn clear(&self) {
        if let Some(active) = self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            active.state.queue.close();
        }
    }

    /// Queue an event for the active agent, subject to event and queue bounds.
    pub fn enqueue_event(&self, event: Llsd) -> Result<(), EventQueueError> {
        let active = self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        active
            .as_ref()
            .ok_or(EventQueueError::Closed)?
            .state
            .queue
            .enqueue(event)
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
    // C# checks poll routes before the global OPTIONS handler.
    if let Some(active) = active {
        if request.uri().path() == active.events_path {
            return poll(State(active.state), request).await;
        }
        if request.uri().path() == active.seed_path {
            return seed(State(active.state), request).await;
        }
    }
    if request.method() == Method::OPTIONS {
        options()
    } else {
        response(StatusCode::NOT_FOUND, SEED_MIME, Vec::new())
    }
}

/// HTTP seed and event queue on `127.0.0.1`.
///
/// Dropping the server stops the listener.
pub struct CapsServer {
    seed_url: String,
    event_queue_url: String,
    queue: EventQueue,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for CapsServer {
    fn drop(&mut self) {
        self.queue.close();
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
            queue: EventQueue::new(),
        };
        let queue = state.queue.clone();
        let app = Router::new()
            .route("/seed", any(seed))
            .route(&events_path, any(poll))
            .with_state(state);
        let (shutdown, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                eprintln!("ferroan-client: CAPS server stopped: {error}");
            }
        });
        tokio::task::yield_now().await;
        Ok(Self {
            seed_url,
            event_queue_url,
            queue,
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

    /// Queue an LLSD event for the next poll.
    pub fn enqueue_event(&self, event: Llsd) -> Result<(), EventQueueError> {
        self.queue.enqueue(event)
    }
}

fn is_seed_content_type(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        == Some("application/llsd+xml")
}

async fn seed(State(state): State<CapsState>, request: Request) -> Response {
    if request.method() == Method::OPTIONS {
        return options();
    }
    if request.method() != Method::POST || !is_seed_content_type(request.headers()) {
        return response(StatusCode::NOT_FOUND, SEED_MIME, Vec::new());
    }
    let bytes = match read_body(request.into_body(), SEED_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let names = match from_xml(&bytes) {
        Ok(Llsd::Array(names)) => names,
        _ => return response(StatusCode::BAD_REQUEST, SEED_MIME, Vec::new()),
    };
    let want = match wants_event_queue(&names) {
        Ok(want) => want,
        Err(_) => return response(StatusCode::BAD_REQUEST, SEED_MIME, Vec::new()),
    };
    let mut map = Llsd::new_map();
    if want {
        map.insert("EventQueueGet", state.event_queue_url);
    }
    response(StatusCode::OK, SEED_MIME, to_xml(&map))
}

async fn poll(State(state): State<CapsState>, request: Request) -> Response {
    // C# ignores method, MIME, ack/done and even malformed poll bodies.
    // Still bound the incoming bytes rather than accumulating an unlimited body.
    if let Err(error) = read_body(request.into_body(), SEED_BODY_LIMIT).await {
        return error.into_response();
    }
    match state.queue.poll().await {
        PollReply::Events(body) => response(StatusCode::OK, "application/xml", body),
        PollReply::Empty => response(StatusCode::BAD_GATEWAY, "text/html", Vec::new()),
        PollReply::Closed => response(StatusCode::NOT_FOUND, "text/html", Vec::new()),
        // Explicit overload policy: at most four simultaneous polls per queue.
        PollReply::Busy => response(StatusCode::SERVICE_UNAVAILABLE, "text/html", Vec::new()),
    }
}

fn response(status: StatusCode, mime: &'static str, body: Vec<u8>) -> Response {
    let mut response = (status, [(header::CONTENT_TYPE, mime)], body).into_response();
    if !status.is_success() {
        response.headers_mut().insert(
            header::CONNECTION,
            axum::http::HeaderValue::from_static("close"),
        );
    }
    response
}

fn options() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, SEED_MIME),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (
                header::ACCESS_CONTROL_ALLOW_METHODS,
                "GET, POST, DELETE, PUT, OPTIONS",
            ),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type"),
        ],
    )
        .into_response()
}

/// Empty OSD.AsString results and unknown capability names are ignored.
fn wants_event_queue(names: &[Llsd]) -> Result<bool, &'static str> {
    if names.len() > MAX_SEED_NAMES {
        return Err("seed lists too many names");
    }
    let mut want = false;
    for name in names {
        let text = name.to_osd_string();
        if text.len() > MAX_SEED_NAME_LEN {
            return Err("seed name is too long");
        }
        if text == "EventQueueGet" {
            want = true;
        }
    }
    Ok(want)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ferroan_llsd::{Llsd, from_xml, to_xml};

    use super::*;
    use crate::circuit::{CircuitExpect, CircuitServer};

    #[tokio::test(start_paused = true)]
    async fn held_empty_poll_matches_csharp_capture_and_ignores_poll_body() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/caps/manifest.json")).unwrap();
        let fixture = &manifest["exchanges"]["empty-poll"];
        assert!(fixture["elapsed_ms"].as_u64().unwrap() >= 58_000);
        let state = CapsState {
            event_queue_url: "http://example.invalid/CE/test".into(),
            queue: EventQueue::new(),
        };
        // C# treats even OPTIONS and malformed bodies as polls on an EQ route.
        let request = Request::builder()
            .method(Method::OPTIONS)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(axum::body::Body::from("not LLSD"))
            .unwrap();
        let task = tokio::spawn(poll(State(state), request));
        tokio::task::yield_now().await;
        tokio::time::advance(crate::event_queue::HOLD - Duration::from_millis(1)).await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_millis(1)).await;
        let response = task.await.unwrap();
        assert_eq!(
            u64::from(response.status().as_u16()),
            fixture["status"].as_u64().unwrap()
        );
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            fixture["response_content_type"].as_str().unwrap()
        );
        assert_eq!(response.headers()[header::CONNECTION], "close");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .as_ref(),
            include_bytes!("../../../fixtures/caps/empty-poll.response.body")
        );
    }

    #[test]
    fn seed_capability_lookup_uses_osd_as_string_and_ignores_unknown_types() {
        assert!(wants_event_queue(&[Llsd::Uri("EventQueueGet".into())]).unwrap());
        assert!(
            !wants_event_queue(&[
                Llsd::Binary(b"EventQueueGet".to_vec()),
                Llsd::Boolean(true),
                Llsd::Boolean(false),
                Llsd::Integer(1),
                Llsd::Real(1.0),
                Llsd::Date(0),
                Llsd::Uuid(Uuid::nil()),
                Llsd::Undef,
                Llsd::new_array(),
                Llsd::new_map(),
            ])
            .unwrap()
        );
    }

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
        registry.enqueue_event(Llsd::new_map()).unwrap();
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
    async fn a_bad_seed_is_refused() {
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
        assert_eq!(response.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn binary_seed_requests_match_captured_csharp_replies() {
        let server = CapsServer::bind().await.unwrap();
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/caps");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap())
                .unwrap();
        for name in [
            "seed-binary-invalid",
            "seed-binary-unpadded",
            "seed-binary-base16",
            "seed-binary-empty-base16",
            "seed-binary-selfclosing-base16",
        ] {
            let fixture = &manifest["exchanges"][name];
            let request =
                std::fs::read(directory.join(fixture["request_file"].as_str().unwrap())).unwrap();
            let expected =
                std::fs::read(directory.join(fixture["response_file"].as_str().unwrap())).unwrap();
            let (status, body, mime) =
                post(server.seed_url(), "application/llsd+xml", request).await;
            assert_eq!(
                u64::from(status),
                fixture["status"].as_u64().unwrap(),
                "{name}"
            );
            assert_eq!(body, expected, "{name}");
            assert_eq!(
                mime.as_deref(),
                fixture["response_content_type"].as_str(),
                "{name}"
            );
        }
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

        session.logout(Duration::from_secs(8)).await.unwrap();
        running.await.unwrap().unwrap();
    }
}
