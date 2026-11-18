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
// Ported to Rust for Ferroan from Source/OpenSim.Server.Handlers/Simulation/
//   AgentHandlers.cs and Utils.cs, Source/OpenSim.Framework/AgentCircuitData.cs,
//   EntityTransferContext.cs, VersionInfo.cs and CapsUtil.cs, and
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Simulation/LocalSimulationConnector.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
// Ferroan changes: bounded local login admission, one active circuit, gzip limits.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The simulation service calls made by C# Robust during a local grid login.
//! Later transfer operations (PUT/DELETE) are deliberately not implemented here.

use std::{
    io::Read,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use axum::{
    Router,
    body::to_bytes,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use ferroan_client::{CapsRegistry, CircuitExpect};
use flate2::read::GzDecoder;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

const MAX_BODY: usize = 1024 * 1024;
const MAX_FEATURES: usize = 256;
const MIN_VERSION: f32 = 0.3;
const MAX_VERSION: f32 = 0.8;
const VIA_LOGIN: u32 = 1 << 7;

#[derive(Debug, thiserror::Error)]
enum SimulationError {
    #[error("invalid simulation JSON")]
    Json,
    #[error("invalid compressed simulation body")]
    Compression,
    #[error("simulation body exceeds its limit")]
    BodyLimit,
    #[error("invalid simulation field: {0}")]
    Field(&'static str),
}

#[derive(Clone)]
pub struct SimulationState {
    region_id: Uuid,
    caps: CapsRegistry,
    sender: mpsc::Sender<AcceptedAgent>,
    occupied: Arc<AtomicBool>,
}

/// Keeps the accepted seed and circuit reserved until logout, timeout or failure.
/// Do not log this value: it includes session credentials received from Robust.
pub struct AcceptedAgent {
    pub expect: CircuitExpect,
    pub secure_session_id: Uuid,
    pub caps_path: String,
    pub start_pos: [f32; 3],
    pub first_name: String,
    pub last_name: String,
    _lease: SessionLease,
}

struct SessionLease {
    occupied: Arc<AtomicBool>,
    caps: CapsRegistry,
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        self.caps.clear();
        self.occupied.store(false, Ordering::Release);
    }
}

impl SimulationState {
    pub fn new(region_id: Uuid, caps: CapsRegistry, sender: mpsc::Sender<AcceptedAgent>) -> Self {
        Self {
            region_id,
            caps,
            sender,
            occupied: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Serve using `into_make_service_with_connect_info::<SocketAddr>()`.
    pub fn router(self) -> Router {
        Router::new()
            .route("/agent/{*path}", any(handle))
            .with_state(self)
    }

    fn accept(&self, fields: AgentFields) -> Result<(), &'static str> {
        if self
            .occupied
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("Region already has an active login");
        }
        let path = format!("/CAPS/{}0000/", fields.caps_path);
        if self.caps.register(&path).is_err() {
            self.occupied.store(false, Ordering::Release);
            return Err("Could not create seed capability");
        }
        let lease = SessionLease {
            occupied: self.occupied.clone(),
            caps: self.caps.clone(),
        };
        let accepted = AcceptedAgent {
            expect: CircuitExpect {
                agent_id: fields.agent_id,
                session_id: fields.session_id,
                circuit_code: fields.circuit_code,
                region_id: self.region_id,
            },
            secure_session_id: fields.secure_session_id,
            caps_path: fields.caps_path,
            start_pos: fields.start_pos,
            first_name: fields.first_name,
            last_name: fields.last_name,
            _lease: lease,
        };
        self.sender
            .try_send(accepted)
            .map_err(|_| "Region is not accepting logins")
    }
}

async fn handle(
    State(state): State<SimulationState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_owned();
    let path: Vec<_> = request.uri().path().trim_matches('/').split('/').collect();
    let agent = path.get(1).and_then(|s| Uuid::parse_str(s).ok());
    let Some(agent) = agent else {
        return bare(StatusCode::NOT_FOUND);
    };
    let region = path.get(2).and_then(|s| Uuid::parse_str(s).ok());
    if method != "POST" && method != "QUERYACCESS" {
        return bare(StatusCode::METHOD_NOT_ALLOWED);
    }
    // This first region runner admits local test-grid control-plane requests only.
    // Never trust viewer-supplied requests to install a circuit.
    if method == "POST"
        && (!peer.ip().is_loopback() || request.headers().contains_key("x-secondlife-shard"))
    {
        return (StatusCode::FORBIDDEN, [(header::CONNECTION, "close")], "").into_response();
    }
    if agent.is_nil() || (method == "QUERYACCESS" && region.is_none_or(|id| id.is_nil())) {
        return bare(StatusCode::BAD_REQUEST);
    }
    let gzip = is_gzip(request.headers());
    let Ok(body) = to_bytes(request.into_body(), MAX_BODY).await else {
        return bare(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let args = match decode_body(&body, gzip) {
        Ok(args) => args,
        Err(SimulationError::BodyLimit) => return bare(StatusCode::PAYLOAD_TOO_LARGE),
        Err(_) => return bare(StatusCode::BAD_REQUEST),
    };
    if method == "QUERYACCESS" {
        return match query_access(&args, region == Some(state.region_id)) {
            Ok(answer) => json_reply(answer, false),
            Err(_) => bare(StatusCode::BAD_REQUEST),
        };
    }
    let fields = match AgentFields::parse(&args, agent, state.region_id) {
        Ok(fields) => fields,
        Err(_) => return bare(StatusCode::BAD_REQUEST),
    };
    let answer = match state.accept(fields) {
        Ok(()) => json!({"reason":"Authorized", "success":true, "your_ip":peer.ip().to_string()}),
        // The C# serializer omits false values in CreateAgent replies.
        Err(reason) => json!({"reason":reason, "your_ip":peer.ip().to_string()}),
    };
    json_reply(answer, true)
}

fn bare(status: StatusCode) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CONNECTION, "close"),
        ],
        "false",
    )
        .into_response()
}

fn json_reply(value: Value, close: bool) -> Response {
    let mut response = (
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response();
    if close || value.get("success") != Some(&Value::Bool(true)) {
        response.headers_mut().insert(
            header::CONNECTION,
            header::HeaderValue::from_static("close"),
        );
    }
    response
}

fn is_gzip(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v == "application/x-gzip")
        || headers
            .get(header::CONTENT_ENCODING)
            .is_some_and(|v| v == "gzip")
        || headers
            .get("x-content-encoding")
            .is_some_and(|v| v == "gzip")
}

fn decode_body(body: &[u8], gzip: bool) -> Result<Map<String, Value>, SimulationError> {
    if body.len() > MAX_BODY {
        return Err(SimulationError::BodyLimit);
    }
    let mut decoded = Vec::new();
    let bytes = if gzip {
        GzDecoder::new(body)
            .take((MAX_BODY + 1) as u64)
            .read_to_end(&mut decoded)
            .map_err(|_| SimulationError::Compression)?;
        if decoded.len() > MAX_BODY {
            return Err(SimulationError::BodyLimit);
        }
        decoded.as_slice()
    } else {
        body
    };
    match serde_json::from_slice(bytes).map_err(|_| SimulationError::Json)? {
        Value::Object(map) => Ok(map),
        _ => Err(SimulationError::Json),
    }
}

fn query_access(args: &Map<String, Value>, known_region: bool) -> Result<Value, SimulationError> {
    let number = |key| args.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let supported_min = number("simulation_service_supported_min");
    let supported_max = number("simulation_service_supported_max");
    let accepted_min = number("simulation_service_accepted_min");
    let accepted_max = number("simulation_service_accepted_max");
    let mut version = 0.0;
    let mut inbound = 0.0;
    let mut outbound = 0.0;
    let deny = |reason: String| Ok(json!({"success":false,"reason":reason}));
    if supported_min == 0.0 {
        version = args
            .get("my_version")
            .and_then(Value::as_str)
            .and_then(|v| v.split('/').nth(1))
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0);
        if version == 0.0 {
            return deny("Your region is running a old version of opensim no longer supported. Consider updating it".into());
        }
        if !(MIN_VERSION..=MAX_VERSION).contains(&version) {
            return deny(format!(
                "Your region protocol version is {version} and we accept only {MIN_VERSION} - {MAX_VERSION}. No version overlap."
            ));
        }
    } else {
        if supported_min > MAX_VERSION || supported_max < MIN_VERSION {
            return deny(format!(
                "Your region provide protocol versions {supported_min} - {supported_max} and we accept only {MIN_VERSION} - {MAX_VERSION}. No version overlap."
            ));
        }
        if accepted_min > MAX_VERSION || accepted_max < MIN_VERSION {
            return deny(format!(
                "You require region protocol versions {accepted_min} - {accepted_max} and we provide only {MIN_VERSION} - {MAX_VERSION}. No version overlap."
            ));
        }
        outbound = supported_max.min(MAX_VERSION);
        inbound = accepted_max.min(MAX_VERSION);
    }
    let features = match args.get("features") {
        None => Vec::new(),
        Some(Value::Array(values)) if values.len() <= MAX_FEATURES => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .map(|id| Value::String(id.to_string()))
                    .ok_or(SimulationError::Field("features"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(SimulationError::Field("features")),
    };
    Ok(json!({
        "success":known_region,
        "reason":if known_region { "" } else { "Communications failure" },
        "version":format!("SIMULATION/{version}"),
        "negotiated_inbound_version":wire_version(inbound),
        "negotiated_outbound_version":wire_version(outbound),
        "features":features,
        "carried_riders":false,
    }))
}

fn wire_version(version: f32) -> f64 {
    // OpenMetaverse's JSON conversion emits the captured C# 15-digit double.
    format!("{:.15}", f64::from(version)).parse().unwrap_or(0.0)
}

struct AgentFields {
    agent_id: Uuid,
    session_id: Uuid,
    secure_session_id: Uuid,
    circuit_code: u32,
    caps_path: String,
    start_pos: [f32; 3],
    first_name: String,
    last_name: String,
}

impl AgentFields {
    fn parse(
        args: &Map<String, Value>,
        path_agent: Uuid,
        region_id: Uuid,
    ) -> Result<Self, SimulationError> {
        let uuid = |key| {
            required_str(args, key).and_then(|s| {
                Uuid::parse_str(s)
                    .ok()
                    .filter(|v| !v.is_nil())
                    .ok_or(SimulationError::Field(key))
            })
        };
        let agent_id = uuid("agent_id")?;
        if agent_id != path_agent || uuid("destination_uuid")? != region_id {
            return Err(SimulationError::Field("destination_uuid/agent_id"));
        }
        let session_id = uuid("session_id")?;
        let secure_session_id = uuid("secure_session_id")?;
        let circuit_code = required_str(args, "circuit_code")?
            .parse()
            .map_err(|_| SimulationError::Field("circuit_code"))?;
        let flags: u32 = required_str(args, "teleport_flags")?
            .parse()
            .map_err(|_| SimulationError::Field("teleport_flags"))?;
        if flags & VIA_LOGIN == 0 || args.get("child").and_then(Value::as_bool) == Some(true) {
            return Err(SimulationError::Field("teleport_flags/child"));
        }
        let caps_path = required_str(args, "caps_path")?;
        if caps_path.is_empty()
            || caps_path.len() > 128
            || !caps_path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(SimulationError::Field("caps_path"));
        }
        let position = required_str(args, "start_pos")?
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');
        let mut coordinates = position.split(',');
        let mut start_pos = [0.0; 3];
        for coordinate in &mut start_pos {
            *coordinate = coordinates
                .next()
                .and_then(|c| c.trim().parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .ok_or(SimulationError::Field("start_pos"))?;
        }
        if coordinates.next().is_some() {
            return Err(SimulationError::Field("start_pos"));
        }
        Ok(Self {
            agent_id,
            session_id,
            secure_session_id,
            circuit_code,
            caps_path: caps_path.to_owned(),
            start_pos,
            first_name: required_str(args, "first_name")?.to_owned(),
            last_name: required_str(args, "last_name")?.to_owned(),
        })
    }
}

fn required_str<'a>(
    args: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, SimulationError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or(SimulationError::Field(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const QUERY_REQUEST: &str =
        include_str!("../../../fixtures/region-login/query-access.request.json");
    const QUERY_RESPONSE: &str =
        include_str!("../../../fixtures/region-login/query-access.response.json");
    const CREATE_REQUEST: &str =
        include_str!("../../../fixtures/region-login/create-agent.request.json");
    const CREATE_RESPONSE: &str =
        include_str!("../../../fixtures/region-login/create-agent.response.json");

    #[tokio::test]
    async fn captured_csharp_login_requests_install_circuit_and_seed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let caps = CapsRegistry::new(&base).unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let expected = AgentFields::parse(&args, agent, region).unwrap();
        let state = SimulationState::new(region, caps.clone(), sender);
        let app = state.router().merge(caps.router());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let query = client
            .request(
                reqwest::Method::from_bytes(b"QUERYACCESS").unwrap(),
                format!("{base}/agent/{agent}/{region}/"),
            )
            .header("content-type", "application/json")
            .body(QUERY_REQUEST)
            .send()
            .await
            .unwrap();
        assert_eq!(query.status(), StatusCode::OK);
        // Defaults, ordering and C# floating point formatting come from the capture.
        assert_eq!(query.text().await.unwrap(), QUERY_RESPONSE.trim());
        let mut compressed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        compressed.write_all(CREATE_REQUEST.as_bytes()).unwrap();
        let response = client
            .post(format!("{base}/agent/{agent}/"))
            .header("content-type", "application/x-gzip")
            .body(compressed.finish().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("connection").unwrap(), "close");
        let mut answer: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(answer["your_ip"], "127.0.0.1");
        answer["your_ip"] = Value::String("192.0.2.1".into());
        assert_eq!(answer.to_string(), CREATE_RESPONSE.trim());
        let accepted = receiver.recv().await.unwrap();
        assert_eq!(
            accepted.expect,
            CircuitExpect {
                agent_id: agent,
                session_id: expected.session_id,
                circuit_code: expected.circuit_code,
                region_id: region,
            }
        );
        assert_eq!(accepted.secure_session_id, expected.secure_session_id);
        assert_eq!(accepted.start_pos, [128.0, 128.0, 0.0]);
        let seed = format!("{base}/CAPS/{}0000/", expected.caps_path);
        let response = client
            .post(&seed)
            .header("content-type", "application/llsd+xml")
            .body("<llsd><array><string>EventQueueGet</string></array></llsd>")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.text().await.unwrap().contains("EventQueueGet"));
        drop(accepted);
        assert_eq!(
            client.post(&seed).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        task.abort();
        let _ = task.await;
    }

    #[test]
    fn captured_agent_rejects_mismatched_identity_and_invalid_capability_path() {
        let mut args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        assert!(AgentFields::parse(&args, Uuid::nil(), region).is_err());
        assert!(AgentFields::parse(&args, agent, Uuid::nil()).is_err());
        args.insert("caps_path".into(), Value::String("../escape".into()));
        assert!(AgentFields::parse(&args, agent, region).is_err());
    }

    fn synthetic_agent() -> AgentFields {
        AgentFields {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            secure_session_id: Uuid::from_u128(3),
            circuit_code: 42,
            caps_path: Uuid::from_u128(4).to_string(),
            start_pos: [128.0, 128.0, 25.0],
            first_name: "Synthetic".into(),
            last_name: "Test".into(),
        }
    }

    #[tokio::test]
    async fn admission_lease_preserves_active_login_and_cleans_closed_queue() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let state = SimulationState::new(Uuid::from_u128(5), caps.clone(), sender);
        state.accept(synthetic_agent()).unwrap();
        let accepted = receiver.recv().await.unwrap();
        assert_eq!(accepted.expect.circuit_code, 42);
        assert!(state.accept(synthetic_agent()).is_err());
        assert!(caps.register("/CAPS/occupied0000/").is_err());
        drop(accepted);
        state.accept(synthetic_agent()).unwrap();
        drop(receiver.recv().await.unwrap());
        drop(receiver);
        assert!(state.accept(synthetic_agent()).is_err());
        assert!(!state.occupied.load(Ordering::Acquire));
        caps.register("/CAPS/after-closed-queue0000/").unwrap();
    }

    #[test]
    fn malformed_and_oversize_bodies_are_rejected() {
        assert!(matches!(
            decode_body(b"[]", false),
            Err(SimulationError::Json)
        ));
        assert!(matches!(
            decode_body(b"not gzip", true),
            Err(SimulationError::Compression)
        ));
        assert!(matches!(
            decode_body(&vec![b' '; MAX_BODY + 1], false),
            Err(SimulationError::BodyLimit)
        ));
        let mut compressed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        compressed.write_all(&vec![b' '; MAX_BODY + 1]).unwrap();
        assert!(matches!(
            decode_body(&compressed.finish().unwrap(), true),
            Err(SimulationError::BodyLimit)
        ));
    }

    #[test]
    fn protocol_negotiation_refuses_non_overlapping_versions() {
        let args = json!({
            "simulation_service_supported_min":0.9,
            "simulation_service_supported_max":1.0,
            "simulation_service_accepted_min":0.3,
            "simulation_service_accepted_max":0.8,
        });
        let answer = query_access(args.as_object().unwrap(), true).unwrap();
        assert_eq!(answer["success"], false);
        assert_eq!(
            answer["reason"],
            "Your region provide protocol versions 0.9 - 1 and we accept only 0.3 - 0.8. No version overlap."
        );
        let answer = query_access(&Map::new(), true).unwrap();
        assert_eq!(answer["success"], false);
        assert_eq!(
            answer["reason"],
            "Your region is running a old version of opensim no longer supported. Consider updating it"
        );
    }
}
