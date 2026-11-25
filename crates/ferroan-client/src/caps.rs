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
//! An uploader path granted to one agent is answered for that agent. An
//! unmatched `/uuid` path is still that agent's only when one agent is registered.

use std::future::Future;
use std::pin::Pin;
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

/// Inventory and asset bodies. A seed stays on the smaller limit below.
const CAP_BODY_LIMIT: usize = 8 * 1024 * 1024;

/// Where a granted capability is answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapTarget {
    /// A path on this region's HTTP listener, allocated when the agent is accepted.
    Local,
    /// Returned in the seed and not handled here.
    External(String),
}

/// Capabilities added beside `EventQueueGet`.
///
/// The seed includes a name only when the client asked for it. An empty set
/// leaves the existing seed and event queue unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapGrantSet {
    grants: Vec<(String, CapTarget)>,
}

impl CapGrantSet {
    pub fn grant(&mut self, name: impl Into<String>, target: CapTarget) {
        let name = name.into();
        if let Some(existing) = self.grants.iter_mut().find(|(found, _)| found == &name) {
            existing.1 = target;
        } else {
            self.grants.push((name, target));
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &CapTarget)> {
        self.grants
            .iter()
            .map(|(name, target)| (name.as_str(), target))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.grants.iter().any(|(found, _)| found == name)
    }
}

/// One request for a local capability, including an uploader path.
pub struct CapExchange {
    pub name: String,
    pub path: String,
    pub method: String,
    pub query: String,
    pub range: Option<String>,
    pub body: Vec<u8>,
    pub agent_id: Uuid,
}

/// The region's answer. `handled: false` becomes the ordinary 404.
pub struct CapAnswer {
    pub handled: bool,
    pub status: StatusCode,
    pub content_type: String,
    pub body: Vec<u8>,
    pub content_range: Option<String>,
}

impl CapAnswer {
    pub fn miss() -> Self {
        Self {
            handled: false,
            status: StatusCode::NOT_FOUND,
            content_type: SEED_MIME.to_owned(),
            body: Vec::new(),
            content_range: None,
        }
    }
}

/// Region handler for local capability paths.
pub type CapService =
    Arc<dyn Fn(CapExchange) -> Pin<Box<dyn Future<Output = CapAnswer> + Send>> + Send + Sync>;

/// Seed and poll bodies larger than this are refused. A seed is a short list
/// of capability names.
const SEED_BODY_LIMIT: usize = 64 * 1024;
/// Names in one seed POST. A viewer asks for well under this.
const MAX_SEED_NAMES: usize = 256;
/// One capability name. The bot's names are shorter.
const MAX_SEED_NAME_LEN: usize = 128;
/// Granted caps one agent may hold, counting the named seed caps and every
/// uploader path. Each uploader path stays until that upload is posted or its
/// 120 second timer fires. A further uploader path is refused.
const MAX_AGENT_CAPS: usize = 64;

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
struct GrantedCap {
    name: String,
    url: String,
    path: Option<String>,
}

#[derive(Clone)]
struct CapsState {
    event_queue_url: String,
    queue: EventQueue,
    agent_id: Uuid,
    granted: Vec<GrantedCap>,
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
    active: Arc<Mutex<Vec<RegisteredCaps>>>,
    grants: CapGrantSet,
    service: Option<CapService>,
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
            active: Arc::new(Mutex::new(Vec::new())),
            grants: CapGrantSet::default(),
            service: None,
        })
    }

    /// Seed grants beyond `EventQueueGet`. Local names are answered by `service`.
    pub fn with_grants(
        base_url: &str,
        grants: CapGrantSet,
        service: Option<CapService>,
    ) -> Result<Self, CapsError> {
        let mut registry = Self::new(base_url)?;
        registry.grants = grants;
        registry.service = service;
        Ok(registry)
    }

    pub fn register(&self, seed_path: &str) -> Result<(), CapsError> {
        self.register_agent(seed_path, Uuid::nil())
    }

    pub fn register_agent(&self, seed_path: &str, agent_id: Uuid) -> Result<(), CapsError> {
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
        if active
            .iter()
            .any(|registered| registered.seed_path == seed_path)
        {
            return Err(CapsError::Occupied);
        }
        let events_path = format!("/CE/{}", Uuid::new_v4());
        let mut granted = Vec::new();
        for (name, target) in self.grants.iter() {
            if name.len() > MAX_SEED_NAME_LEN {
                continue;
            }
            match target {
                CapTarget::Local => {
                    let path = format!("/{}", Uuid::new_v4());
                    granted.push(GrantedCap {
                        name: name.to_owned(),
                        url: format!("{}{path}", self.base_url),
                        path: Some(path),
                    });
                }
                CapTarget::External(url) => {
                    if url.len() > 2048 {
                        continue;
                    }
                    granted.push(GrantedCap {
                        name: name.to_owned(),
                        url: url.clone(),
                        path: None,
                    });
                }
            }
        }
        active.push(RegisteredCaps {
            seed_path: seed_path.to_owned(),
            state: CapsState {
                event_queue_url: format!("{}{events_path}", self.base_url),
                queue: EventQueue::new(),
                agent_id,
                granted,
            },
            events_path,
        });
        Ok(())
    }

    pub fn clear(&self) {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for registered in active.drain(..) {
            registered.state.queue.close();
        }
    }

    /// Close one agent's seed, event queue and granted paths.
    pub fn clear_agent(&self, agent_id: Uuid) {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        active.retain(|registered| {
            if registered.state.agent_id == agent_id {
                registered.state.queue.close();
                false
            } else {
                true
            }
        });
    }

    /// Queue an event when exactly one agent is registered.
    ///
    /// Zero agents, or more than one, is [`EventQueueError::Closed`]: the
    /// caller would otherwise have to guess which queue to use.
    pub fn enqueue_event(&self, event: Llsd) -> Result<(), EventQueueError> {
        let active = self
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if active.len() != 1 {
            return Err(EventQueueError::Closed);
        }
        active[0].state.queue.enqueue(event)
    }

    /// Queue an event on one agent's event queue.
    ///
    /// A missing agent is [`EventQueueError::Closed`]. The queue is cloned out
    /// of the registry lock before the event is stored.
    pub fn enqueue_for(&self, agent_id: Uuid, event: Llsd) -> Result<(), EventQueueError> {
        let queue = {
            let active = self
                .active
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(registered) = active
                .iter()
                .find(|registered| registered.state.agent_id == agent_id)
            else {
                return Err(EventQueueError::Closed);
            };
            registered.state.queue.clone()
        };
        queue.enqueue(event)
    }

    /// Merge this fallback with the region's more specific simulation routes.
    pub fn router(&self) -> Router {
        Router::new()
            .fallback(registered_caps)
            .with_state(self.clone())
    }

    /// Paths this region can add to one agent after the seed is registered.
    ///
    /// The closure that serves caps does not hold this registry, so granting
    /// an uploader does not keep the registry alive.
    pub fn uploader_routes(&self) -> UploaderRoutes {
        UploaderRoutes {
            active: Some(Arc::clone(&self.active)),
            base_url: self.base_url.clone(),
        }
    }
}

/// Adds and removes one agent's uploader path.
///
/// The path is a granted cap with an empty name. The next POST on it is that
/// agent's upload completion. A path that is not a bare UUID, or that another
/// cap already uses, is refused.
#[derive(Clone, Default)]
pub struct UploaderRoutes {
    active: Option<Arc<Mutex<Vec<RegisteredCaps>>>>,
    base_url: String,
}

impl UploaderRoutes {
    pub fn grant(&self, agent_id: Uuid, path: &str) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        if !is_uuid_path(path) {
            return false;
        }
        let mut registrations = active.lock().unwrap_or_else(|poison| poison.into_inner());
        if registrations
            .iter()
            .any(|registered| path_owned(registered, path))
        {
            return false;
        }
        let Some(registered) = registrations
            .iter_mut()
            .find(|registered| registered.state.agent_id == agent_id)
        else {
            return false;
        };
        if registered.state.granted.len() >= MAX_AGENT_CAPS {
            let agent = agent_id.to_string();
            let observed = registered.state.granted.len() as u64 + 1;
            ferroan_diag::hit(&ferroan_diag::Hit {
                id: ferroan_diag::REGION_AGENT_CAPS,
                kind: ferroan_diag::Kind::Hard,
                configured: MAX_AGENT_CAPS as u64,
                observed,
                action: ferroan_diag::Action::Refuse,
                route: "uploader",
                agent_id: Some(&agent),
            });
            return false;
        }
        registered.state.granted.push(GrantedCap {
            name: String::new(),
            url: format!("{}{path}", self.base_url),
            path: Some(path.to_owned()),
        });
        true
    }

    pub fn revoke(&self, agent_id: Uuid, path: &str) {
        let Some(active) = &self.active else {
            return;
        };
        let mut registrations = active.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(registered) = registrations
            .iter_mut()
            .find(|registered| registered.state.agent_id == agent_id)
        else {
            return;
        };
        registered
            .state
            .granted
            .retain(|cap| !(cap.name.is_empty() && cap.path.as_deref() == Some(path)));
    }
}

fn path_owned(registered: &RegisteredCaps, path: &str) -> bool {
    registered.seed_path == path
        || registered.events_path == path
        || registered
            .state
            .granted
            .iter()
            .any(|cap| cap.path.as_deref() == Some(path))
}

async fn registered_caps(State(registry): State<CapsRegistry>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let (active, service, alone) = {
        let registrations = registry
            .active
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let alone = registrations.len() == 1;
        let matched = registrations.iter().find(|registered| {
            registered.events_path == path
                || registered.seed_path == path
                || registered
                    .state
                    .granted
                    .iter()
                    .any(|cap| cap.path.as_deref() == Some(path.as_str()))
        });
        // An upload completes on a UUID path. A path granted to one agent is
        // theirs. A path nobody registered is the remaining agent's only when
        // exactly one agent is registered. With two, guessing would hand one
        // agent's upload to the other.
        let matched = matched
            .cloned()
            .or_else(|| (alone && is_uuid_path(&path)).then(|| registrations[0].clone()));
        (matched, registry.service.clone(), alone)
    };
    // C# checks poll routes before the global OPTIONS handler.
    if let Some(active) = active {
        if path == active.events_path {
            return poll(State(active.state), request).await;
        }
        if path == active.seed_path {
            return seed(State(active.state.clone()), request).await;
        }
        let named = active
            .state
            .granted
            .iter()
            .find(|cap| cap.path.as_deref() == Some(path.as_str()))
            .map(|cap| cap.name.clone());
        if (named.is_some() || (alone && is_uuid_path(&path)))
            && let Some(service) = service
        {
            return call_cap(
                service,
                active.state.agent_id,
                named.unwrap_or_default(),
                request,
            )
            .await;
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
            agent_id: Uuid::nil(),
            granted: Vec::new(),
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
    for name in names {
        let text = name.to_osd_string();
        if text == "EventQueueGet" {
            continue;
        }
        if let Some(cap) = state.granted.iter().find(|cap| cap.name == text) {
            map.insert(text, cap.url.clone());
        }
    }
    response(StatusCode::OK, SEED_MIME, to_xml(&map))
}

fn is_uuid_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    let mut parts = rest.split('-');
    let widths = [8, 4, 4, 4, 12];
    widths.into_iter().all(|width| {
        parts.next().is_some_and(|part| {
            part.len() == width && part.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    }) && parts.next().is_none()
}

async fn call_cap(service: CapService, agent_id: Uuid, name: String, request: Request) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = match read_body(request.into_body(), CAP_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    let answer = service(CapExchange {
        name,
        path,
        method,
        query,
        range,
        body: bytes.to_vec(),
        agent_id,
    })
    .await;
    if !answer.handled {
        return response(StatusCode::NOT_FOUND, SEED_MIME, Vec::new());
    }
    let mut response = (
        answer.status,
        [(header::CONTENT_TYPE, answer.content_type)],
        answer.body,
    )
        .into_response();
    if let Some(range) = answer.content_range
        && let Ok(value) = axum::http::HeaderValue::from_str(&range)
    {
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    if !answer.status.is_success() && answer.status != StatusCode::NOT_FOUND {
        response.headers_mut().insert(
            header::CONNECTION,
            axum::http::HeaderValue::from_static("close"),
        );
    }
    response
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
        let elapsed = Duration::from_millis(fixture["elapsed_ms"].as_u64().unwrap());
        // The capture includes scheduling/transport overhead. Its whole seconds
        // establish the expected deadline independently of the production HOLD.
        let captured_hold = Duration::from_secs(elapsed.as_secs());
        assert!(elapsed - captured_hold <= Duration::from_millis(250));
        assert_eq!(crate::event_queue::HOLD, captured_hold);
        let state = CapsState {
            event_queue_url: "http://example.invalid/CE/test".into(),
            queue: EventQueue::new(),
            agent_id: Uuid::nil(),
            granted: Vec::new(),
        };
        // C# treats even OPTIONS and malformed bodies as polls on an EQ route.
        let request = Request::builder()
            .method(Method::OPTIONS)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(axum::body::Body::from("not LLSD"))
            .unwrap();
        let task = tokio::spawn(poll(State(state), request));
        tokio::task::yield_now().await;
        tokio::time::advance(captured_hold - Duration::from_millis(1)).await;
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
        assert_eq!(
            response.headers()[header::CONNECTION],
            fixture["response_connection"].as_str().unwrap()
        );
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
    async fn two_agents_keep_separate_seeds_and_an_unmatched_uuid_is_not_guessed() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let service: CapService = Arc::new(move |exchange| {
            let record = Arc::clone(&record);
            Box::pin(async move {
                record
                    .lock()
                    .unwrap()
                    .push((exchange.agent_id, exchange.name, exchange.path));
                CapAnswer {
                    handled: true,
                    status: StatusCode::OK,
                    content_type: SEED_MIME.to_owned(),
                    body: Vec::new(),
                    content_range: None,
                }
            })
        });
        let registry =
            CapsRegistry::with_grants(&base, CapGrantSet::default(), Some(service)).unwrap();
        let app = registry.router();
        let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let first_agent = Uuid::from_u128(1);
        let second_agent = Uuid::from_u128(2);
        let first = format!("/CAPS/{}0000/", Uuid::from_u128(10));
        let second = format!("/CAPS/{}0000/", Uuid::from_u128(11));
        registry.register_agent(&first, first_agent).unwrap();
        registry.register_agent(&second, second_agent).unwrap();
        assert!(matches!(
            registry.register_agent(&first, first_agent),
            Err(CapsError::Occupied)
        ));
        assert!(matches!(
            registry.enqueue_event(Llsd::new_map()),
            Err(EventQueueError::Closed)
        ));
        assert!(registry.enqueue_for(first_agent, Llsd::new_map()).is_ok());
        assert!(matches!(
            registry.enqueue_for(Uuid::from_u128(99), Llsd::new_map()),
            Err(EventQueueError::Closed)
        ));
        let upload = format!("/{}", Uuid::new_v4());
        assert_eq!(
            post(
                &format!("{}{}", base.trim_end_matches('/'), upload),
                "application/octet-stream",
                Vec::new()
            )
            .await
            .0,
            404
        );
        registry.clear_agent(first_agent);
        assert_eq!(
            post(
                &format!("{}{}", base.trim_end_matches('/'), first),
                "application/llsd+xml",
                name_list(["EventQueueGet"])
            )
            .await
            .0,
            404
        );
        assert_eq!(
            post(
                &format!("{}{}", base.trim_end_matches('/'), second),
                "application/llsd+xml",
                name_list(["EventQueueGet"])
            )
            .await
            .0,
            200
        );
        registry.enqueue_event(Llsd::new_map()).unwrap();
        assert_eq!(
            post(
                &format!("{}{}", base.trim_end_matches('/'), upload),
                "application/octet-stream",
                Vec::new()
            )
            .await
            .0,
            200
        );
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen, vec![(second_agent, String::new(), upload)]);
        serving.abort();
    }

    #[tokio::test]
    async fn two_agents_receive_posts_on_the_uploader_path_granted_to_them() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        let service: CapService = Arc::new(move |exchange| {
            let record = Arc::clone(&record);
            Box::pin(async move {
                record
                    .lock()
                    .unwrap()
                    .push((exchange.agent_id, exchange.path));
                CapAnswer {
                    handled: true,
                    status: StatusCode::OK,
                    content_type: SEED_MIME.to_owned(),
                    body: Vec::new(),
                    content_range: None,
                }
            })
        });
        let registry =
            CapsRegistry::with_grants(&base, CapGrantSet::default(), Some(service)).unwrap();
        let routes = registry.uploader_routes();
        let app = registry.router();
        let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let first_agent = Uuid::from_u128(1);
        let second_agent = Uuid::from_u128(2);
        registry
            .register_agent(&format!("/CAPS/{}0000/", Uuid::from_u128(10)), first_agent)
            .unwrap();
        registry
            .register_agent(&format!("/CAPS/{}0000/", Uuid::from_u128(11)), second_agent)
            .unwrap();
        let first_path = format!("/{}", Uuid::new_v4());
        let second_path = format!("/{}", Uuid::new_v4());
        assert!(routes.grant(first_agent, &first_path));
        assert!(routes.grant(second_agent, &second_path));
        assert!(!routes.grant(first_agent, &second_path));
        assert!(!routes.grant(Uuid::from_u128(99), &format!("/{}", Uuid::new_v4())));
        let root = base.trim_end_matches('/');
        assert_eq!(
            post(
                &format!("{root}{first_path}"),
                "application/octet-stream",
                b"a".to_vec()
            )
            .await
            .0,
            200
        );
        assert_eq!(
            post(
                &format!("{root}{second_path}"),
                "application/octet-stream",
                b"b".to_vec()
            )
            .await
            .0,
            200
        );
        let stranger = format!("/{}", Uuid::new_v4());
        assert_eq!(
            post(
                &format!("{root}{stranger}"),
                "application/octet-stream",
                Vec::new()
            )
            .await
            .0,
            404
        );
        routes.revoke(first_agent, &first_path);
        assert_eq!(
            post(
                &format!("{root}{first_path}"),
                "application/octet-stream",
                Vec::new()
            )
            .await
            .0,
            404
        );
        assert_eq!(
            post(
                &format!("{root}{second_path}"),
                "application/octet-stream",
                Vec::new()
            )
            .await
            .0,
            200
        );
        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![
                (first_agent, first_path),
                (second_agent, second_path.clone()),
                (second_agent, second_path),
            ]
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

    fn agent_cap_refuses() -> u64 {
        ferroan_diag::metrics()
            .lines()
            .find_map(|line| {
                let rest = line.strip_prefix(
                    "ferroan_limit_hits_total{limit=\"region.agent-caps\",action=\"refuse\"} ",
                )?;
                rest.trim().parse().ok()
            })
            .unwrap_or(0)
    }

    fn uploader_path(n: u32) -> String {
        format!("/{n:08x}-0000-4000-8000-{n:012x}")
    }

    #[test]
    fn an_agent_past_the_cap_bound_is_refused() {
        let registry = CapsRegistry::new("http://127.0.0.1:9/").unwrap();
        let routes = registry.uploader_routes();
        let agent = Uuid::from_u128(0x21);
        let other = Uuid::from_u128(0x22);
        registry
            .register_agent("/CAPS/000000210000/", agent)
            .unwrap();
        registry
            .register_agent("/CAPS/000000220000/", other)
            .unwrap();
        for n in 0..MAX_AGENT_CAPS as u32 {
            assert!(
                routes.grant(agent, &uploader_path(n)),
                "grant {n} should fit"
            );
        }
        let before = agent_cap_refuses();
        assert!(!routes.grant(agent, &uploader_path(MAX_AGENT_CAPS as u32)));
        assert_eq!(agent_cap_refuses(), before + 1);
        let before = agent_cap_refuses();
        assert!(!routes.grant(agent, "not-a-uuid"));
        assert!(!routes.grant(Uuid::from_u128(0x99), &uploader_path(90)));
        assert!(!routes.grant(agent, &uploader_path(0)));
        assert_eq!(agent_cap_refuses(), before);
        assert!(routes.grant(other, &uploader_path(90)));
    }
}
