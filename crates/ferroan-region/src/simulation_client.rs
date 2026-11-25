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
// Ported to Rust for Ferroan from
//   Source/OpenSim.Services.Connectors/Simulation/SimulationServiceConnector.cs
//   Source/OpenSim.Framework/AgentCircuitData.cs
//   Source/OpenSim.Framework/EntityTransferContext.cs
//   Source/OpenSim.Framework/AvatarAppearance.cs
//   Source/OpenSim.Framework/AvatarWearable.cs
//   Source/OpenSim.Framework/AvatarAttachment.cs
//   Source/OpenSim.Framework/ChildAgentDataUpdate.cs
//   Source/OpenSim.Framework/WearableCacheItem.cs
//   Source/OpenSim.Framework/Animation.cs
//   Source/OpenSim.Framework/VersionInfo.cs
//   Source/OpenSim.Framework/WebUtil.cs (ServiceOSDRequest)
//   OpenMetaverse.StructuredData JSON omit-defaults
//   OpenSim-NGC/OpenSim-Tranquillity @ 1052cbae82ae7dc3b8abfcffe33e316498eeaad9
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Outbound simulation connector.
//!
//! Asks a neighbour `QueryAccess`, posts `CreateAgent`, puts `AgentData`, and
//! deletes with `CloseAgent`. JSON omits default scalars the way the neighbour
//! parser expects, and a compressed create that does not complete is posted
//! again uncompressed. This module does not send viewer packets or change
//! presence.

use std::io::Write;
use std::time::Duration;

use ferroan_diag::{self, HTTP_POST_BODY, HTTP_RESPONSE_BODY, Kind};
use flate2::Compression;
use flate2::write::GzEncoder;
use reqwest::Method;
use serde_json::{Map, Value};
use uuid::Uuid;

const VERSION_MIN: f32 = 0.3;
const VERSION_MAX: f32 = 0.8;
const MAX_BODY: usize = 1024 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const CREATE_TIMEOUT: Duration = Duration::from_secs(30);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(200);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);
const LEGACY_WEARABLES: i32 = 15;
const PV7_WEARABLES: usize = 16;
const PV7_BAKES: usize = 6;

/// A neighbour region the connector can post to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionEnd {
    pub server_uri: String,
    pub region_id: Uuid,
    pub name: String,
    /// Region location in meters, the same integers the neighbour stores as
    /// `destination_x` and `destination_y`.
    pub loc_x: u32,
    pub loc_y: u32,
}

/// Versions carried with every simulation call. A new transfer starts at 0.8.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransferContext {
    pub inbound: f32,
    pub outbound: f32,
    pub wearables_count: i32,
}

impl TransferContext {
    pub fn negotiated() -> Self {
        Self {
            inbound: VERSION_MAX,
            outbound: VERSION_MAX,
            wearables_count: -1,
        }
    }
}

/// One child capability seed, sent only when the circuit has any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildSeed {
    pub handle: u64,
    pub seed: String,
}

/// One worn item. A wearable slot is a list of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WearableItem {
    pub item: Uuid,
    pub asset: Uuid,
}

/// One baked texture. A nil id is left off the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BakedCache {
    pub texture_index: u32,
    pub cache_id: Uuid,
    pub texture_id: Uuid,
}

/// One attachment point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub point: i32,
    pub item: Uuid,
    pub asset: Uuid,
}

/// Avatar appearance packed inside a circuit or an agent update.
///
/// `has_texture` is false when the avatar has no texture entry. Only serial,
/// height and hover are sent in that case.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub serial: i32,
    pub height: f32,
    pub aphz: f32,
    pub has_texture: bool,
    pub wearables: Vec<Vec<WearableItem>>,
    pub te8: Vec<u8>,
    pub textures: Vec<Uuid>,
    pub baked: Vec<BakedCache>,
    pub visual_params: Vec<u8>,
    pub attachments: Vec<Attachment>,
}

/// Circuit fields a neighbour accepts on `CreateAgent`.
#[derive(Clone, Debug, PartialEq)]
pub struct Circuit {
    pub agent_id: Uuid,
    pub base_folder: Uuid,
    pub caps_path: String,
    pub children_seeds: Vec<ChildSeed>,
    pub child: bool,
    pub circuit_code: u32,
    pub first_name: String,
    pub last_name: String,
    pub inventory_folder: Uuid,
    pub secure_session_id: Uuid,
    pub session_id: Uuid,
    pub service_session_id: String,
    pub start_pos: [f32; 3],
    pub client_ip: String,
    pub viewer: String,
    pub channel: String,
    pub mac: String,
    pub id0: String,
    pub far: f32,
    pub appearance: Option<Appearance>,
    pub service_urls: Vec<(String, String)>,
}

/// An animation block. A nil object id is left off the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anim {
    pub animation: Uuid,
    pub object_id: Uuid,
    pub sequence: i32,
}

/// `AgentData` put to a neighbour after a child create.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentData {
    pub region_id: Uuid,
    pub circuit_code: u32,
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub center: [f32; 3],
    pub size: [f32; 3],
    pub at_axis: [f32; 3],
    pub left_axis: [f32; 3],
    pub up_axis: [f32; 3],
    pub wait_for_root: bool,
    pub far: f32,
    pub throttles: Vec<u8>,
    pub locomotion_state: u32,
    pub head_rotation: [f32; 4],
    pub body_rotation: [f32; 4],
    pub control_flags: u32,
    /// An empty god block is still sent. The hop's block has no keys.
    pub god_data: bool,
    pub agent_access: u32,
    pub default_animation: Option<Anim>,
    pub animation_state: Option<Anim>,
    pub motion_state: i32,
    pub appearance: Option<Appearance>,
    pub sit_offset: [f32; 3],
    pub children_seeds: Vec<ChildSeed>,
    /// Release callback for a hop outside the view. Packed as `cb_uri`.
    /// An in-view hop leaves this empty, so the key is omitted.
    pub callback: Option<String>,
}

/// What the caller asks `QueryAccess`.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessQuery {
    pub agent_id: Uuid,
    pub agent_home_uri: Option<String>,
    pub via_teleport: bool,
    pub position: [f32; 3],
    pub features: Vec<Uuid>,
    pub context: TransferContext,
}

/// Versions and feature ids taken from the access answer.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryOutcome {
    pub success: bool,
    pub reason: String,
    pub inbound_version: f32,
    pub outbound_version: f32,
    pub wearables_count: i32,
    pub features: Vec<Uuid>,
    pub carried_riders: bool,
}

/// Whether the neighbour accepted the child, and the reason it gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateOutcome {
    pub success: bool,
    pub reason: String,
}

/// The neighbour call could not be finished inside Ferroan's own limits.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClientError {
    #[error("simulation client could not start")]
    Unavailable,
    #[error("simulation request exceeds the body limit")]
    RequestLimit,
    #[error("simulation reply exceeds the body limit")]
    ReplyLimit,
    #[error("neighbour close failed")]
    CloseFailed,
}

enum Osd {
    Bool(bool),
    Int(i32),
    Real(f32),
    Str(String),
    Uuid(Uuid),
    Bytes(Vec<u8>),
    Array(Vec<Osd>),
    Map(Vec<(String, Osd)>),
}

enum Attempt {
    Responded(String),
    Failed(String),
}

struct ParsedCreate {
    done: bool,
    success: bool,
    reason: String,
    service_session_id: Option<String>,
}

struct Outbound {
    method: Method,
    url: String,
    body: Option<Vec<u8>>,
    compress: bool,
    keepalive: bool,
    timeout: Duration,
}

/// Posts simulation JSON to one neighbour.
#[derive(Clone, Debug)]
pub struct SimulationClient {
    http: reqwest::Client,
}

impl SimulationClient {
    pub fn new() -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .http1_only()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(UPDATE_TIMEOUT)
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| ClientError::Unavailable)?;
        Ok(Self { http })
    }

    /// `QueryAccess`. A missing destination answer is unsuccessful.
    pub async fn query_access(
        &self,
        destination: &RegionEnd,
        query: &AccessQuery,
    ) -> Result<QueryOutcome, ClientError> {
        let url = format!(
            "{}/agent/{}/{}/",
            trim_uri(&destination.server_uri),
            query.agent_id,
            destination.region_id
        );
        let body = encode_json(&pack_query(query))?;
        let attempt = self
            .execute(Outbound {
                method: query_method(),
                url,
                body: Some(body),
                compress: false,
                keepalive: true,
                timeout: QUERY_TIMEOUT,
            })
            .await?;
        Ok(parse_query(attempt, &query.context))
    }

    /// `CreateAgent`. A missing destination does not open a connection.
    ///
    /// The compressed post is sent first. A connection failure, a non-2xx
    /// status, or the body `false` is posted again uncompressed. A JSON answer
    /// is final, including an answer whose success flag is false.
    pub async fn create_agent(
        &self,
        source: Option<&RegionEnd>,
        destination: Option<&RegionEnd>,
        circuit: &mut Circuit,
        flags: u32,
        context: &TransferContext,
    ) -> Result<CreateOutcome, ClientError> {
        let Some(destination) = destination else {
            return Ok(CreateOutcome {
                success: false,
                reason: "Destination not found".to_string(),
            });
        };
        let url = format!(
            "{}/agent/{}/",
            trim_uri(&destination.server_uri),
            circuit.agent_id
        );
        let body = encode_json(&pack_create(source, destination, circuit, flags, context))?;
        let first = self
            .execute(Outbound {
                method: Method::POST,
                url: url.clone(),
                body: Some(body.clone()),
                compress: true,
                keepalive: false,
                timeout: CREATE_TIMEOUT,
            })
            .await?;
        let parsed = parse_create(first);
        if parsed.done {
            apply_session(circuit, &parsed);
            return Ok(finish_create(parsed));
        }
        let second = self
            .execute(Outbound {
                method: Method::POST,
                url,
                body: Some(body),
                compress: false,
                keepalive: false,
                timeout: CREATE_TIMEOUT,
            })
            .await?;
        let parsed = parse_create(second);
        if parsed.done {
            apply_session(circuit, &parsed);
        }
        Ok(finish_create(parsed))
    }

    /// `UpdateAgent` for `AgentData`. At outbound 0.3 and above this is one
    /// compressed PUT. The boolean is the HTTP result, not a field inside a
    /// JSON object.
    pub async fn update_agent(
        &self,
        destination: &RegionEnd,
        data: &AgentData,
        context: &TransferContext,
    ) -> Result<bool, ClientError> {
        let url = format!(
            "{}/agent/{}/",
            trim_uri(&destination.server_uri),
            data.agent_id
        );
        let body = encode_json(&pack_update(destination, data, context))?;
        let compressed = self
            .execute(Outbound {
                method: Method::PUT,
                url: url.clone(),
                body: Some(body.clone()),
                compress: true,
                keepalive: false,
                timeout: UPDATE_TIMEOUT,
            })
            .await?;
        let compressed_ok = http_success(&compressed);
        if context.outbound >= 0.3 || compressed_ok || context.outbound >= 0.2 {
            return Ok(compressed_ok);
        }
        let plain = self
            .execute(Outbound {
                method: Method::PUT,
                url,
                body: Some(body),
                compress: false,
                keepalive: false,
                timeout: UPDATE_TIMEOUT,
            })
            .await?;
        Ok(http_success(&plain))
    }

    /// `CloseAgent`. A non-2xx status or a dropped connection is an error
    /// (TB56). The DELETE path and the auth query are unchanged.
    pub async fn close_agent(
        &self,
        destination: &RegionEnd,
        agent_id: Uuid,
        auth: &str,
    ) -> Result<(), ClientError> {
        let url = format!(
            "{}/agent/{}/{}/?auth={auth}",
            trim_uri(&destination.server_uri),
            agent_id,
            destination.region_id
        );
        let attempt = self
            .execute(Outbound {
                method: Method::DELETE,
                url,
                body: None,
                compress: false,
                keepalive: false,
                timeout: CLOSE_TIMEOUT,
            })
            .await?;
        match attempt {
            Attempt::Responded(_) => Ok(()),
            Attempt::Failed(_) => Err(ClientError::CloseFailed),
        }
    }

    async fn execute(&self, call: Outbound) -> Result<Attempt, ClientError> {
        let mut request = self
            .http
            .request(call.method, &call.url)
            .timeout(call.timeout);
        if let Some(plain) = call.body {
            let bytes = if call.compress {
                request = request.header("X-Content-Encoding", "gzip");
                gzip_bytes(&plain)?
            } else {
                plain
            };
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(bytes);
        }
        request = if call.keepalive {
            request
                .header("Keep-Alive", "timeout=30, max=10")
                .header(reqwest::header::CONNECTION, "Keep-Alive")
        } else {
            request.header(reqwest::header::CONNECTION, "close")
        };
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => {
                return Ok(Attempt::Failed(
                    "Service request failed: connection failed".to_string(),
                ));
            }
        };
        let status = response.status();
        if !status.is_success() {
            return Ok(Attempt::Failed(format!(
                "Service request failed: [{}]",
                status.as_u16()
            )));
        }
        let bytes = match read_reply(response).await {
            Ok(bytes) => bytes,
            Err(ReadFail::Transport) => {
                return Ok(Attempt::Failed(
                    "Service request failed: connection failed".to_string(),
                ));
            }
            Err(ReadFail::Limit) => return Err(ClientError::ReplyLimit),
        };
        match String::from_utf8(bytes) {
            Ok(text) => Ok(Attempt::Responded(text)),
            Err(_) => Ok(Attempt::Failed(
                "Service request failed: connection failed".to_string(),
            )),
        }
    }
}

enum ReadFail {
    Transport,
    Limit,
}

async fn read_reply(mut response: reqwest::Response) -> Result<Vec<u8>, ReadFail> {
    if let Some(length) = response
        .content_length()
        .filter(|length| *length > MAX_BODY as u64)
    {
        ferroan_diag::refuse(
            HTTP_RESPONSE_BODY,
            Kind::Hard,
            MAX_BODY as u64,
            length,
            "simulation",
        );
        return Err(ReadFail::Limit);
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) => return Err(ReadFail::Transport),
        };
        let next = bytes.len().saturating_add(chunk.len());
        if next > MAX_BODY {
            ferroan_diag::refuse(
                HTTP_RESPONSE_BODY,
                Kind::Hard,
                MAX_BODY as u64,
                next as u64,
                "simulation",
            );
            return Err(ReadFail::Limit);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn query_method() -> Method {
    match Method::from_bytes(b"QUERYACCESS") {
        Ok(method) => method,
        Err(_) => Method::GET,
    }
}

fn trim_uri(uri: &str) -> &str {
    uri.trim_end_matches('/')
}

fn encode_json(value: &Value) -> Result<Vec<u8>, ClientError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ClientError::Unavailable)?;
    if bytes.len() > MAX_BODY {
        ferroan_diag::refuse(
            HTTP_POST_BODY,
            Kind::Hard,
            MAX_BODY as u64,
            bytes.len() as u64,
            "simulation",
        );
        return Err(ClientError::RequestLimit);
    }
    Ok(bytes)
}

fn gzip_bytes(plain: &[u8]) -> Result<Vec<u8>, ClientError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(plain)
        .map_err(|_| ClientError::Unavailable)?;
    encoder.finish().map_err(|_| ClientError::Unavailable)
}

fn finish_create(parsed: ParsedCreate) -> CreateOutcome {
    CreateOutcome {
        success: parsed.success,
        reason: parsed.reason,
    }
}

fn apply_session(circuit: &mut Circuit, parsed: &ParsedCreate) {
    if parsed.success
        && let Some(token) = &parsed.service_session_id
    {
        circuit.service_session_id.clone_from(token);
    }
}

fn http_success(attempt: &Attempt) -> bool {
    match attempt {
        Attempt::Failed(_) => false,
        Attempt::Responded(body) if body.eq_ignore_ascii_case("false") => false,
        Attempt::Responded(_) => true,
    }
}

fn parse_create(attempt: Attempt) -> ParsedCreate {
    let body = match attempt {
        Attempt::Failed(message) => {
            return ParsedCreate {
                done: false,
                success: false,
                reason: message,
                service_session_id: None,
            };
        }
        Attempt::Responded(body) => body,
    };
    if body.eq_ignore_ascii_case("true") {
        return ParsedCreate {
            done: true,
            success: false,
            reason: String::new(),
            service_session_id: None,
        };
    }
    if body.eq_ignore_ascii_case("false") {
        return ParsedCreate {
            done: false,
            success: false,
            reason: String::new(),
            service_session_id: None,
        };
    }
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&body) else {
        return ParsedCreate {
            done: true,
            success: false,
            reason: String::new(),
            service_session_id: None,
        };
    };
    let success = osd_bool(map.get("success"));
    let service_session_id = map
        .get("service_session_id")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty() && success)
        .map(ToOwned::to_owned);
    ParsedCreate {
        done: true,
        success,
        reason: osd_string(map.get("reason")),
        service_session_id,
    }
}

fn parse_query(attempt: Attempt, context: &TransferContext) -> QueryOutcome {
    let mut outcome = QueryOutcome {
        success: false,
        reason: "Failed to contact destination".to_string(),
        inbound_version: context.inbound,
        outbound_version: context.outbound,
        wearables_count: context.wearables_count,
        features: Vec::new(),
        carried_riders: false,
    };
    let body = match attempt {
        Attempt::Failed(message) => {
            outcome.reason = message;
            return outcome;
        }
        Attempt::Responded(body) => body,
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&body) else {
        outcome.reason.clear();
        return outcome;
    };
    outcome.carried_riders = osd_bool(map.get("carried_riders"));
    let success = osd_bool(map.get("success"));
    outcome.reason = osd_string(map.get("reason"));
    if map.contains_key("negotiated_inbound_version") {
        outcome.inbound_version = json_f32(map.get("negotiated_inbound_version"));
        outcome.outbound_version = json_f32(map.get("negotiated_outbound_version"));
    } else if let Some(version) = map.get("version").and_then(Value::as_str)
        && !version.is_empty()
        && let Some((_, rest)) = version.split_once('/')
        && let Ok(parsed) = rest.parse::<f32>()
    {
        outcome.inbound_version = parsed;
        outcome.outbound_version = parsed;
    }
    if !success || outcome.inbound_version == 0.0 || outcome.outbound_version == 0.0 {
        return outcome;
    }
    outcome.success = true;
    outcome.features = features_of(map.get("features"));
    outcome.wearables_count = wearables_for(outcome.outbound_version);
    outcome
}

fn wearables_for(outbound: f32) -> i32 {
    if outbound < 0.5 {
        LEGACY_WEARABLES
    } else if outbound < 0.6 {
        LEGACY_WEARABLES + 1
    } else {
        -1
    }
}

fn features_of(value: Option<&Value>) -> Vec<Uuid> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.as_str().and_then(|text| Uuid::parse_str(text).ok()))
        .collect()
}

fn osd_bool(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => {
            !text.is_empty() && text != "0" && !text.eq_ignore_ascii_case("false")
        }
        _ => false,
    }
}

fn osd_string(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or("").to_string()
}

fn pack_query(query: &AccessQuery) -> Value {
    let mut entries = vec![
        ("viaTeleport", Osd::Bool(query.via_teleport)),
        ("position", Osd::Str(format_vec3(query.position))),
        ("my_version", Osd::Str(format!("SIMULATION/{VERSION_MIN}"))),
        ("simulation_service_supported_min", Osd::Real(VERSION_MIN)),
        ("simulation_service_supported_max", Osd::Real(VERSION_MAX)),
        ("simulation_service_accepted_min", Osd::Real(VERSION_MIN)),
        ("simulation_service_accepted_max", Osd::Real(VERSION_MAX)),
        ("context", pack_context(&query.context)),
        (
            "features",
            Osd::Array(
                query
                    .features
                    .iter()
                    .map(|id| Osd::Str(id.to_string()))
                    .collect(),
            ),
        ),
    ];
    if let Some(home) = &query.agent_home_uri {
        entries.push(("agent_home_uri", Osd::Str(home.clone())));
    }
    object(entries)
}

fn pack_create(
    source: Option<&RegionEnd>,
    destination: &RegionEnd,
    circuit: &Circuit,
    flags: u32,
    context: &TransferContext,
) -> Value {
    let mut entries = vec![
        ("agent_id", Osd::Uuid(circuit.agent_id)),
        ("base_folder", Osd::Uuid(circuit.base_folder)),
        ("caps_path", Osd::Str(circuit.caps_path.clone())),
    ];
    if !circuit.children_seeds.is_empty() {
        entries.push(("children_seeds", pack_seeds(&circuit.children_seeds)));
    }
    entries.extend([
        ("child", Osd::Bool(circuit.child)),
        ("circuit_code", Osd::Str(circuit.circuit_code.to_string())),
        ("first_name", Osd::Str(circuit.first_name.clone())),
        ("last_name", Osd::Str(circuit.last_name.clone())),
        ("inventory_folder", Osd::Uuid(circuit.inventory_folder)),
        ("secure_session_id", Osd::Uuid(circuit.secure_session_id)),
        ("session_id", Osd::Uuid(circuit.session_id)),
        (
            "service_session_id",
            Osd::Str(circuit.service_session_id.clone()),
        ),
        ("start_pos", Osd::Str(format_vec3(circuit.start_pos))),
        ("client_ip", Osd::Str(circuit.client_ip.clone())),
        ("viewer", Osd::Str(circuit.viewer.clone())),
        ("channel", Osd::Str(circuit.channel.clone())),
        ("mac", Osd::Str(circuit.mac.clone())),
        ("id0", Osd::Str(circuit.id0.clone())),
    ]);
    if circuit.far > 0.0 {
        entries.push(("far", Osd::Real(circuit.far)));
    }
    if let Some(appearance) = &circuit.appearance {
        entries.push(("appearance_serial", Osd::Int(appearance.serial)));
        entries.push((
            "packed_appearance",
            pack_appearance(appearance, context.outbound),
        ));
    }
    entries.extend(pack_service_urls(&circuit.service_urls));
    entries.push(("context", pack_context(context)));
    entries.extend(pack_ends(source, destination, Some(flags)));
    object(entries)
}

fn pack_update(destination: &RegionEnd, data: &AgentData, context: &TransferContext) -> Value {
    let mut entries = vec![
        ("message_type", Osd::Str("AgentData".to_string())),
        ("region_id", Osd::Str(data.region_id.to_string())),
        ("circuit_code", Osd::Str(data.circuit_code.to_string())),
        ("agent_uuid", Osd::Uuid(data.agent_id)),
        ("session_uuid", Osd::Uuid(data.session_id)),
        ("position", Osd::Str(format_vec3(data.position))),
        ("velocity", Osd::Str(format_vec3(data.velocity))),
        ("center", Osd::Str(format_vec3(data.center))),
        ("size", Osd::Str(format_vec3(data.size))),
        ("at_axis", Osd::Str(format_vec3(data.at_axis))),
        ("left_axis", Osd::Str(format_vec3(data.left_axis))),
        ("up_axis", Osd::Str(format_vec3(data.up_axis))),
        ("changed_grid", Osd::Bool(data.wait_for_root)),
        ("wait_for_root", Osd::Bool(data.wait_for_root)),
        ("far", Osd::Real(data.far)),
        ("aspect", Osd::Real(0.0)),
    ];
    if !data.throttles.is_empty() {
        entries.push(("throttles", Osd::Bytes(data.throttles.clone())));
    }
    entries.extend([
        (
            "locomotion_state",
            Osd::Str(data.locomotion_state.to_string()),
        ),
        ("head_rotation", Osd::Str(format_vec4(data.head_rotation))),
        ("body_rotation", Osd::Str(format_vec4(data.body_rotation))),
        ("control_flags", Osd::Str(data.control_flags.to_string())),
        ("energy_level", Osd::Real(0.0)),
    ]);
    if data.god_data {
        entries.push(("god_data", Osd::Map(Vec::new())));
    }
    entries.extend([
        ("always_run", Osd::Bool(false)),
        ("prey_agent", Osd::Uuid(Uuid::nil())),
        ("agent_access", Osd::Str(data.agent_access.to_string())),
        ("agent_cof", Osd::Uuid(Uuid::nil())),
        ("crossingflags", Osd::Int(0)),
        ("active_group_id", Osd::Uuid(Uuid::nil())),
        ("active_group_name", Osd::Str(String::new())),
    ]);
    if !data.children_seeds.is_empty() {
        entries.push(("children_seeds", pack_seeds(&data.children_seeds)));
    }
    if let Some(callback) = &data.callback {
        entries.push(("cb_uri", Osd::Str(callback.clone())));
    }
    if let Some(anim) = data.default_animation {
        entries.push(("default_animation", pack_anim(anim)));
    }
    if let Some(anim) = data.animation_state {
        entries.push(("animation_state", pack_anim(anim)));
    }
    if data.motion_state != 0 {
        entries.push(("motion_state", Osd::Int(data.motion_state)));
    }
    if let Some(appearance) = &data.appearance {
        entries.push((
            "packed_appearance",
            pack_appearance(appearance, context.outbound),
        ));
    }
    entries.push(("parent_part", Osd::Uuid(Uuid::nil())));
    entries.push(("sit_offset", Osd::Str(format_vec3(data.sit_offset))));
    entries.extend(pack_ends(None, destination, None));
    entries.push(("context", pack_context(context)));
    object(entries)
}

fn pack_ends(
    source: Option<&RegionEnd>,
    destination: &RegionEnd,
    flags: Option<u32>,
) -> Vec<(&'static str, Osd)> {
    let mut entries = Vec::new();
    if let Some(source) = source {
        entries.extend([
            ("source_x", Osd::Str(source.loc_x.to_string())),
            ("source_y", Osd::Str(source.loc_y.to_string())),
            ("source_name", Osd::Str(source.name.clone())),
            ("source_uuid", Osd::Str(source.region_id.to_string())),
        ]);
        if !source.server_uri.is_empty() {
            entries.push(("source_server_uri", Osd::Str(source.server_uri.clone())));
        }
    }
    entries.extend([
        ("destination_x", Osd::Str(destination.loc_x.to_string())),
        ("destination_y", Osd::Str(destination.loc_y.to_string())),
        ("destination_name", Osd::Str(destination.name.clone())),
        (
            "destination_uuid",
            Osd::Str(destination.region_id.to_string()),
        ),
    ]);
    if let Some(flags) = flags {
        entries.push(("teleport_flags", Osd::Str(flags.to_string())));
    }
    entries
}

fn pack_context(context: &TransferContext) -> Osd {
    Osd::Map(vec![
        ("InboundVersion".to_string(), Osd::Real(context.inbound)),
        ("OutboundVersion".to_string(), Osd::Real(context.outbound)),
        (
            "WearablesCount".to_string(),
            Osd::Int(context.wearables_count),
        ),
    ])
}

fn pack_seeds(seeds: &[ChildSeed]) -> Osd {
    Osd::Array(
        seeds
            .iter()
            .map(|seed| {
                Osd::Map(vec![
                    ("handle".to_string(), Osd::Str(seed.handle.to_string())),
                    ("seed".to_string(), Osd::Str(seed.seed.clone())),
                ])
            })
            .collect(),
    )
}

fn pack_service_urls(urls: &[(String, String)]) -> Vec<(&'static str, Osd)> {
    if urls.is_empty() {
        return Vec::new();
    }
    let flat = Osd::Array(
        urls.iter()
            .flat_map(|(key, value)| [Osd::Str(key.clone()), Osd::Str(value.clone())])
            .collect(),
    );
    let map = Osd::Map(
        urls.iter()
            .map(|(key, value)| (key.clone(), Osd::Str(value.clone())))
            .collect(),
    );
    vec![("service_urls", flat), ("serviceurls", map)]
}

fn pack_anim(anim: Anim) -> Osd {
    Osd::Map(vec![
        ("animation".to_string(), Osd::Uuid(anim.animation)),
        ("object_id".to_string(), Osd::Uuid(anim.object_id)),
        ("seq_num".to_string(), Osd::Int(anim.sequence)),
    ])
}

fn pack_appearance(appearance: &Appearance, outbound: f32) -> Osd {
    let mut entries = vec![
        ("serial".to_string(), Osd::Int(appearance.serial)),
        ("height".to_string(), Osd::Real(appearance.height)),
        ("aphz".to_string(), Osd::Real(appearance.aphz)),
    ];
    if !appearance.has_texture {
        return Osd::Map(entries);
    }
    let pv8 = outbound >= 0.8;
    if pv8 && appearance.wearables.len() > PV7_WEARABLES {
        let tail = appearance.wearables[PV7_WEARABLES..]
            .iter()
            .map(|items| pack_wearable(items))
            .collect();
        entries.push(("wrbls8".to_string(), Osd::Array(tail)));
        let head = appearance.wearables[..PV7_WEARABLES]
            .iter()
            .map(|items| pack_wearable(items))
            .collect();
        entries.push(("wearables".to_string(), Osd::Array(head)));
    } else {
        let count = if pv8 {
            appearance.wearables.len()
        } else if outbound >= 0.6 {
            PV7_WEARABLES.min(appearance.wearables.len())
        } else {
            15
        };
        let count = count.min(appearance.wearables.len());
        let wears = appearance.wearables[..count]
            .iter()
            .map(|items| pack_wearable(items))
            .collect();
        entries.push(("wearables".to_string(), Osd::Array(wears)));
    }
    if pv8 {
        entries.push(("te8".to_string(), Osd::Bytes(appearance.te8.clone())));
    } else {
        let textures = appearance.textures.iter().copied().map(Osd::Uuid).collect();
        entries.push(("textures".to_string(), Osd::Array(textures)));
    }
    if !appearance.baked.is_empty() {
        let split = PV7_BAKES.min(appearance.baked.len());
        entries.push((
            "bakedcache".to_string(),
            Osd::Array(appearance.baked[..split].iter().map(pack_baked).collect()),
        ));
        if appearance.baked.len() > PV7_BAKES {
            entries.push((
                "bc8".to_string(),
                Osd::Array(
                    appearance.baked[PV7_BAKES..]
                        .iter()
                        .map(pack_baked)
                        .collect(),
                ),
            ));
        }
    }
    entries.push((
        "visualparams".to_string(),
        Osd::Bytes(appearance.visual_params.clone()),
    ));
    let attachments = appearance.attachments.iter().map(pack_attachment).collect();
    entries.push(("attachments".to_string(), Osd::Array(attachments)));
    Osd::Map(entries)
}

fn pack_wearable(items: &[WearableItem]) -> Osd {
    Osd::Array(
        items
            .iter()
            .map(|item| {
                Osd::Map(vec![
                    ("item".to_string(), Osd::Uuid(item.item)),
                    ("asset".to_string(), Osd::Uuid(item.asset)),
                ])
            })
            .collect(),
    )
}

fn pack_baked(item: &BakedCache) -> Osd {
    Osd::Map(vec![
        (
            "textureindex".to_string(),
            Osd::Bytes(item.texture_index.to_be_bytes().to_vec()),
        ),
        ("cacheid".to_string(), Osd::Uuid(item.cache_id)),
        ("textureid".to_string(), Osd::Uuid(item.texture_id)),
    ])
}

fn pack_attachment(item: &Attachment) -> Osd {
    Osd::Map(vec![
        ("point".to_string(), Osd::Int(item.point)),
        ("item".to_string(), Osd::Uuid(item.item)),
        ("asset".to_string(), Osd::Uuid(item.asset)),
    ])
}

fn object(entries: Vec<(&'static str, Osd)>) -> Value {
    let map = Osd::Map(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    );
    match omit_value(map) {
        Some(kept) => to_json(&kept),
        None => Value::Object(Map::new()),
    }
}

fn omit_value(value: Osd) -> Option<Osd> {
    match value {
        Osd::Bool(false) | Osd::Int(0) => None,
        Osd::Real(0.0) => None,
        Osd::Str(text) if text.is_empty() => None,
        Osd::Uuid(id) if id.is_nil() => None,
        Osd::Bytes(bytes) if bytes.is_empty() => None,
        Osd::Array(items) => Some(Osd::Array(items.into_iter().map(omit_element).collect())),
        Osd::Map(entries) => Some(Osd::Map(omit_entries(entries))),
        other => Some(other),
    }
}

fn omit_entries(entries: Vec<(String, Osd)>) -> Vec<(String, Osd)> {
    entries
        .into_iter()
        .filter_map(|(key, value)| omit_value(value).map(|value| (key, value)))
        .collect()
}

/// Array elements stay, including zeros and nil ids. Maps inside them still
/// drop their own default fields.
fn omit_element(value: Osd) -> Osd {
    match value {
        Osd::Map(entries) => Osd::Map(omit_entries(entries)),
        Osd::Array(items) => Osd::Array(items.into_iter().map(omit_element).collect()),
        other => other,
    }
}

fn to_json(value: &Osd) -> Value {
    match value {
        Osd::Bool(flag) => Value::Bool(*flag),
        Osd::Int(number) => Value::Number((*number).into()),
        Osd::Real(number) => real_json(*number),
        Osd::Str(text) => Value::String(text.clone()),
        Osd::Uuid(id) => Value::String(id.to_string()),
        Osd::Bytes(bytes) => Value::Array(bytes.iter().copied().map(byte_json).collect()),
        Osd::Array(items) => Value::Array(items.iter().map(to_json).collect()),
        Osd::Map(entries) => {
            let mut map = Map::new();
            for (key, child) in entries {
                map.insert(key.clone(), to_json(child));
            }
            Value::Object(map)
        }
    }
}

fn real_json(number: f32) -> Value {
    match serde_json::Number::from_f64(f64::from(number)) {
        Some(number) => Value::Number(number),
        None => Value::from(0),
    }
}

fn byte_json(byte: u8) -> Value {
    Value::Number(u64::from(byte).into())
}

fn format_vec3(value: [f32; 3]) -> String {
    format!(
        "<{}, {}, {}>",
        format_component(value[0]),
        format_component(value[1]),
        format_component(value[2])
    )
}

fn format_vec4(value: [f32; 4]) -> String {
    format!(
        "<{}, {}, {}, {}>",
        format_component(value[0]),
        format_component(value[1]),
        format_component(value[2]),
        format_component(value[3])
    )
}

fn format_component(value: f32) -> String {
    if value == 0.0 {
        if value.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        }
    } else {
        value.to_string()
    }
}

#[allow(clippy::cast_possible_truncation)]
fn json_f32(value: Option<&Value>) -> f32 {
    value
        .and_then(Value::as_f64)
        .map_or(0.0, |number| number as f32)
}

fn json_i32_value(value: Option<&Value>) -> i32 {
    value
        .and_then(Value::as_i64)
        .and_then(|number| i32::try_from(number).ok())
        .unwrap_or(0)
}

fn uuid_value(value: Option<&Value>) -> Uuid {
    value
        .and_then(Value::as_str)
        .and_then(|text| Uuid::parse_str(text).ok())
        .unwrap_or_else(Uuid::nil)
}

fn byte_list(value: Option<&Value>) -> Vec<u8> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.as_u64().and_then(|number| u8::try_from(number).ok()))
        .collect()
}

/// `packed_appearance` from a CreateAgent or AgentData body.
pub(crate) fn appearance_from_json(value: &Value) -> Appearance {
    Appearance {
        serial: json_i32_value(value.get("serial")),
        height: json_f32(value.get("height")),
        aphz: json_f32(value.get("aphz")),
        has_texture: value.get("te8").is_some()
            || value.get("textures").is_some()
            || value.get("wearables").is_some()
            || value.get("wrbls8").is_some(),
        wearables: {
            let mut slots = wearables_from(value.get("wearables"));
            slots.extend(wearables_from(value.get("wrbls8")));
            slots
        },
        te8: byte_list(value.get("te8")),
        textures: uuid_values(value.get("textures")),
        baked: baked_from(value.get("bakedcache"))
            .into_iter()
            .chain(baked_from(value.get("bc8")))
            .collect(),
        visual_params: byte_list(value.get("visualparams")),
        attachments: attachments_from(value.get("attachments")),
    }
}

fn wearables_from(value: Option<&Value>) -> Vec<Vec<WearableItem>> {
    let Some(Value::Array(slots)) = value else {
        return Vec::new();
    };
    slots
        .iter()
        .map(|slot| {
            slot.as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| WearableItem {
                            item: uuid_value(item.get("item")),
                            asset: uuid_value(item.get("asset")),
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect()
}

fn baked_from(value: Option<&Value>) -> Vec<BakedCache> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| BakedCache {
            texture_index: texture_index_from(item.get("textureindex")),
            cache_id: uuid_value(item.get("cacheid")),
            texture_id: uuid_value(item.get("textureid")),
        })
        .collect()
}

fn attachments_from(value: Option<&Value>) -> Vec<Attachment> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| Attachment {
            point: json_i32_value(item.get("point")),
            item: uuid_value(item.get("item")),
            asset: uuid_value(item.get("asset")),
        })
        .collect()
}

fn uuid_values(value: Option<&Value>) -> Vec<Uuid> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items.iter().map(|item| uuid_value(Some(item))).collect()
}

fn texture_index_from(value: Option<&Value>) -> u32 {
    let bytes = byte_list(value);
    let Ok(chunk) = <[u8; 4]>::try_from(bytes.as_slice()) else {
        return 0;
    };
    u32::from_be_bytes(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_capture::{Kind, read_complete_prefix};
    use flate2::read::GzDecoder;
    use serde_json::json;
    use std::io::Read;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const HOME_CAP: &[u8] = include_bytes!("../../../fixtures/neighbour-teleport/proxy-home.cap");
    const NEIGHBOUR_CAP: &[u8] =
        include_bytes!("../../../fixtures/neighbour-teleport/proxy-neighbour.cap");
    const LANDING: &str = "<128, 128, 30>";

    #[test]
    fn version_reals_use_the_widened_single_and_defaults_drop() {
        let packed = pack_query(&AccessQuery {
            agent_id: Uuid::nil(),
            agent_home_uri: Some("/".to_string()),
            via_teleport: true,
            position: [128.0, 128.0, 30.0],
            features: Vec::new(),
            context: TransferContext::negotiated(),
        });
        let text = serde_json::to_string(&packed).unwrap();
        assert!(text.contains("0.800000011920929"), "{text}");
        assert!(text.contains("0.30000001192092896"), "{text}");
        assert_eq!(packed["position"], LANDING);
        assert_eq!(packed["my_version"], "SIMULATION/0.3");
        assert_eq!(packed["viaTeleport"], true);
        assert!(packed["features"].as_array().unwrap().is_empty());
        assert_eq!(packed["context"]["WearablesCount"], -1);

        let mut circuit = circuit_blank();
        circuit.circuit_code = 0;
        circuit.child = false;
        circuit.service_session_id.clear();
        let region = sample_region();
        let created = pack_create(None, &region, &circuit, 16, &TransferContext::negotiated());
        assert!(created.get("child").is_none());
        assert!(created.get("base_folder").is_none());
        assert!(created.get("service_session_id").is_none());
        assert!(created.get("far").is_none());
        assert_eq!(created["circuit_code"], "0");
        assert_eq!(created["teleport_flags"], "16");
        assert_eq!(format_vec3([-0.0, 1.0, 0.0]), "<-0, 1, 0>");
        assert_eq!(real_json(255.0).to_string(), "255.0");
    }

    #[test]
    fn appearance_appends_the_seventeenth_wearable_slot() {
        let mut wearables = Vec::new();
        for slot in 0..16 {
            wearables.push(json!([{
                "item": Uuid::from_u128(slot + 1).to_string(),
                "asset": Uuid::from_u128(slot + 100).to_string(),
            }]));
        }
        let value = json!({
            "serial": 3,
            "height": 1.8,
            "wearables": wearables,
            "wrbls8": [[{
                "item": Uuid::from_u128(17).to_string(),
                "asset": Uuid::from_u128(117).to_string(),
            }]],
            "te8": [1, 2, 3, 4],
        });
        let appearance = appearance_from_json(&value);
        assert_eq!(appearance.wearables.len(), 17);
        assert_eq!(appearance.wearables[16][0].item, Uuid::from_u128(17));
        assert_eq!(appearance.wearables[16][0].asset, Uuid::from_u128(117));
        assert_eq!(appearance.wearables[0][0].item, Uuid::from_u128(1));
        assert!(appearance.has_texture);
    }

    #[test]
    fn packed_hop_fields_match_the_q18a_predicates() {
        let region = RegionEnd {
            server_uri: "http://127.0.0.1:9/".to_string(),
            region_id: Uuid::from_u128(7),
            name: "Ferroan Test".to_string(),
            loc_x: 256_000,
            loc_y: 256_000,
        };
        let source = RegionEnd {
            name: "Ferroan Neighbour".to_string(),
            loc_x: 256_256,
            loc_y: 256_000,
            ..region.clone()
        };
        let query = pack_query(&AccessQuery {
            agent_id: Uuid::from_u128(4),
            agent_home_uri: Some("/".to_string()),
            via_teleport: true,
            position: [128.0, 128.0, 30.0],
            features: Vec::new(),
            context: TransferContext::negotiated(),
        });
        assert_eq!(query["position"], LANDING);
        assert_eq!(query["viaTeleport"], true);
        assert_eq!(query["my_version"], "SIMULATION/0.3");
        assert!((query["simulation_service_supported_min"].as_f64().unwrap() - 0.3).abs() < 1e-5);
        assert!((query["simulation_service_accepted_max"].as_f64().unwrap() - 0.8).abs() < 1e-5);
        assert!((query["context"]["InboundVersion"].as_f64().unwrap() - 0.8).abs() < 1e-5);
        assert!((query["context"]["OutboundVersion"].as_f64().unwrap() - 0.8).abs() < 1e-5);

        let mut circuit = circuit_blank();
        circuit.agent_id = Uuid::from_u128(4);
        circuit.child = true;
        circuit.start_pos = [128.0, 128.0, 30.0];
        circuit.circuit_code = 1;
        let created = pack_create(
            Some(&source),
            &region,
            &circuit,
            16,
            &TransferContext::negotiated(),
        );
        assert_eq!(created["teleport_flags"], "16");
        assert_eq!(created["start_pos"], LANDING);
        assert_eq!(created["child"], true);
        assert_eq!(created["destination_x"], "256000");
        assert_eq!(created["destination_y"], "256000");
        assert_eq!(created["destination_name"], "Ferroan Test");
        assert_eq!(created["destination_uuid"], region.region_id.to_string());
        assert_eq!(created["source_x"], "256256");
        assert!((created["context"]["OutboundVersion"].as_f64().unwrap() - 0.8).abs() < 1e-5);

        let mut data = agent_blank();
        data.agent_id = circuit.agent_id;
        data.position = [128.0, 128.0, 30.0];
        data.wait_for_root = true;
        data.circuit_code = 1;
        let updated = pack_update(&region, &data, &TransferContext::negotiated());
        assert_eq!(updated["message_type"], "AgentData");
        assert_eq!(updated["wait_for_root"], true);
        assert_eq!(updated["changed_grid"], true);
        assert_eq!(updated["position"], LANDING);
        assert_eq!(updated["destination_uuid"], region.region_id.to_string());
        assert!((updated["context"]["InboundVersion"].as_f64().unwrap() - 0.8).abs() < 1e-5);
    }

    #[test]
    fn captured_hop_bodies_repack_to_the_same_keys() {
        for bytes in [HOME_CAP, NEIGHBOUR_CAP] {
            let bodies = hop_bodies(bytes);
            let context = TransferContext::negotiated();
            let created = pack_create(
                bodies.source.as_ref(),
                &bodies.destination,
                &bodies.circuit,
                16,
                &context,
            );
            assert_same(&bodies.create, &created, "create");
            let source = bodies.source.as_ref().unwrap();
            assert_eq!(bodies.agent.region_id, source.region_id);
            assert_ne!(bodies.agent.region_id, bodies.destination.region_id);
            let updated = pack_update(&bodies.destination, &bodies.agent, &context);
            assert_same(&bodies.update, &updated, "update");
            let query = pack_query(&bodies.query);
            assert_same(&bodies.access, &query, "query");
        }
    }

    #[tokio::test]
    async fn a_missing_destination_does_not_call_the_neighbour() {
        let client = SimulationClient::new().unwrap();
        let mut circuit = circuit_blank();
        let outcome = client
            .create_agent(None, None, &mut circuit, 16, &TransferContext::negotiated())
            .await
            .unwrap();
        assert!(!outcome.success);
        assert_eq!(outcome.reason, "Destination not found");
    }

    #[tokio::test]
    async fn compressed_create_falls_back_and_a_json_refusal_does_not() {
        let (addr, handle) = scripted(vec![Scripted::Drop, Scripted::Text(200, "{\"success\":true,\"reason\":\"\",\"service_session_id\":\"00000000-0000-4000-8000-000000000099\"}")]).await;
        let client = SimulationClient::new().unwrap();
        let region = bound_region(addr);
        let mut circuit = circuit_blank();
        circuit.agent_id = Uuid::from_u128(4);
        circuit.child = true;
        let outcome = client
            .create_agent(
                None,
                Some(&region),
                &mut circuit,
                16,
                &TransferContext::negotiated(),
            )
            .await
            .unwrap();
        let hits = finish_script(handle).await;
        assert!(outcome.success);
        assert_eq!(hits.len(), 2);
        assert!(header_has(&hits[0].header, "x-content-encoding", "gzip"));
        assert!(!header_has(&hits[1].header, "x-content-encoding", "gzip"));
        assert!(header_has(&hits[0].header, "connection", "close"));
        let decoded = gunzip(&hits[0].body);
        let body: Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(body["teleport_flags"], "16");
        assert_eq!(body["child"], true);
        assert_eq!(
            circuit.service_session_id,
            "00000000-0000-4000-8000-000000000099"
        );

        let (addr, handle) = scripted(vec![Scripted::Text(
            200,
            "{\"success\":false,\"reason\":\"full\"}",
        )])
        .await;
        let region = bound_region(addr);
        let outcome = client
            .create_agent(
                None,
                Some(&region),
                &mut circuit,
                16,
                &TransferContext::negotiated(),
            )
            .await
            .unwrap();
        let hits = finish_script(handle).await;
        assert!(!outcome.success);
        assert_eq!(outcome.reason, "full");
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn false_create_body_is_posted_again_uncompressed() {
        let (addr, handle) = scripted(vec![
            Scripted::Text(200, "false"),
            Scripted::Text(200, "{\"success\":true,\"reason\":\"Authorized\"}"),
        ])
        .await;
        let client = SimulationClient::new().unwrap();
        let mut circuit = circuit_blank();
        let outcome = client
            .create_agent(
                None,
                Some(&bound_region(addr)),
                &mut circuit,
                16,
                &TransferContext::negotiated(),
            )
            .await
            .unwrap();
        let hits = finish_script(handle).await;
        assert!(outcome.success);
        assert_eq!(outcome.reason, "Authorized");
        assert_eq!(hits.len(), 2);
        assert!(header_has(&hits[0].header, "x-content-encoding", "gzip"));
        assert!(!header_has(&hits[1].header, "x-content-encoding", "gzip"));
    }

    #[tokio::test]
    async fn agent_data_at_0_8_does_not_fall_back() {
        let (addr, handle) = scripted(vec![Scripted::Text(500, "no")]).await;
        let client = SimulationClient::new().unwrap();
        let mut data = agent_blank();
        data.agent_id = Uuid::from_u128(4);
        data.wait_for_root = true;
        let updated = client
            .update_agent(&bound_region(addr), &data, &TransferContext::negotiated())
            .await
            .unwrap();
        let hits = finish_script(handle).await;
        assert!(!updated);
        assert_eq!(hits.len(), 1);
        assert_eq!(request_method(&hits[0].header), "PUT");
        assert!(header_has(&hits[0].header, "x-content-encoding", "gzip"));

        let (addr, handle) = scripted(vec![Scripted::Text(200, "True")]).await;
        let updated = client
            .update_agent(&bound_region(addr), &data, &TransferContext::negotiated())
            .await
            .unwrap();
        let hits = finish_script(handle).await;
        assert!(updated);
        let plain = String::from_utf8(gunzip(&hits[0].body)).unwrap();
        let body: Value = serde_json::from_str(&plain).unwrap();
        assert_eq!(body["message_type"], "AgentData");
        assert_eq!(body["wait_for_root"], true);
    }

    #[tokio::test]
    async fn query_access_keeps_negotiated_versions_and_inner_features() {
        let feature = "11111111-1111-4111-8111-111111111111";
        let body = format!(
            "{{\"success\":true,\"reason\":\"\",\"version\":\"SIMULATION/0\",\"negotiated_inbound_version\":0.800000011920929,\"negotiated_outbound_version\":0.800000011920929,\"features\":[\"{feature}\"],\"carried_riders\":true}}"
        );
        let (addr, handle) = scripted(vec![Scripted::Text(200, leak(body))]).await;
        let client = SimulationClient::new().unwrap();
        let region = bound_region(addr);
        let outcome = client
            .query_access(
                &region,
                &AccessQuery {
                    agent_id: Uuid::from_u128(4),
                    agent_home_uri: Some("/".to_string()),
                    via_teleport: true,
                    position: [128.0, 128.0, 30.0],
                    features: Vec::new(),
                    context: TransferContext::negotiated(),
                },
            )
            .await
            .unwrap();
        let hits = finish_script(handle).await;
        assert!(outcome.success);
        assert!((outcome.inbound_version - 0.8).abs() < 1e-6);
        assert!((outcome.outbound_version - 0.8).abs() < 1e-6);
        assert_eq!(outcome.wearables_count, -1);
        assert!(outcome.carried_riders);
        assert_eq!(outcome.features, vec![Uuid::parse_str(feature).unwrap()]);
        assert_eq!(hits.len(), 1);
        assert_eq!(request_method(&hits[0].header), "QUERYACCESS");
        assert!(request_target(&hits[0].header).contains(&region.region_id.to_string()));
        assert!(
            header_has(&hits[0].header, "connection", "keep-alive")
                || header_has(&hits[0].header, "connection", "Keep-Alive")
        );
        let sent: Value = serde_json::from_slice(&hits[0].body).unwrap();
        assert_eq!(sent["position"], LANDING);
        assert_eq!(sent["viaTeleport"], true);
    }

    #[tokio::test]
    async fn close_returns_the_delivery_error_and_accepts_a_2xx_body() {
        let client = SimulationClient::new().unwrap();
        let agent = Uuid::from_u128(4);
        let (addr, handle) = scripted(vec![Scripted::Text(500, "no")]).await;
        let error = client
            .close_agent(
                &bound_region(addr),
                agent,
                "00000000-0000-4000-8000-000000000003",
            )
            .await
            .unwrap_err();
        let hits = finish_script(handle).await;
        assert_eq!(error, ClientError::CloseFailed);
        assert_eq!(hits.len(), 1);
        let target = request_target(&hits[0].header);
        assert!(target.contains("/agent/"));
        assert!(target.contains("?auth="));
        assert!(hits[0].body.is_empty());

        let (addr, handle) = scripted(vec![Scripted::Text(
            200,
            "OpenSim agent 00000000-0000-4000-8000-000000000004",
        )])
        .await;
        client
            .close_agent(
                &bound_region(addr),
                agent,
                "00000000-0000-4000-8000-000000000003",
            )
            .await
            .unwrap();
        finish_script(handle).await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let error = client
            .close_agent(
                &bound_region(addr),
                agent,
                "00000000-0000-4000-8000-000000000003",
            )
            .await
            .unwrap_err();
        assert_eq!(error, ClientError::CloseFailed);
    }

    #[test]
    fn the_neighbour_capture_closes_with_an_auth_query_and_no_body() {
        let records = read_complete_prefix(NEIGHBOUR_CAP).unwrap();
        let delete = records
            .iter()
            .filter(|record| record.kind == Kind::HttpRequest)
            .map(|record| http_text(&record.bytes))
            .find(|text| text.starts_with("DELETE /agent/"))
            .unwrap();
        let (header, body) = header_and_body(delete.as_bytes());
        assert!(header.starts_with("DELETE /agent/"));
        assert!(header.contains("?auth="));
        assert!(body.is_empty());
    }

    struct HopBodies {
        access: Value,
        create: Value,
        update: Value,
        query: AccessQuery,
        source: Option<RegionEnd>,
        destination: RegionEnd,
        circuit: Circuit,
        agent: AgentData,
    }

    fn hop_bodies(bytes: &[u8]) -> HopBodies {
        let records = read_complete_prefix(bytes).unwrap();
        let mut access = None;
        let mut create = None;
        let mut update = None;
        for record in records {
            if record.kind != Kind::HttpRequest {
                continue;
            }
            let (header, body) = header_and_body(&record.bytes);
            let start = header.lines().next().unwrap_or("");
            let mut parts = start.split_whitespace();
            let method = parts.next().unwrap_or("");
            if body.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<Value>(body) else {
                continue;
            };
            if method == "QUERYACCESS"
                && value.get("position").and_then(Value::as_str) == Some(LANDING)
            {
                access = Some(value);
            } else if method == "POST"
                && value.get("teleport_flags").and_then(Value::as_str) == Some("16")
                && value.get("start_pos").and_then(Value::as_str) == Some(LANDING)
            {
                create = Some(value);
            } else if method == "PUT"
                && value.get("message_type").and_then(Value::as_str) == Some("AgentData")
            {
                update = Some(value);
            }
        }
        let access = access.unwrap();
        let create = create.unwrap();
        let update = update.unwrap();
        let destination = region_from(&create, "destination");
        let source = create
            .get("source_uuid")
            .map(|_| region_from(&create, "source"));
        let circuit = circuit_from(&create);
        let agent = agent_from(&update);
        let query = AccessQuery {
            agent_id: circuit.agent_id,
            agent_home_uri: access
                .get("agent_home_uri")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            via_teleport: access
                .get("viaTeleport")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            position: parse_vec3(access.get("position").and_then(Value::as_str).unwrap_or("")),
            features: features_of(access.get("features")),
            context: context_from(access.get("context")),
        };
        HopBodies {
            access,
            create,
            update,
            query,
            source,
            destination,
            circuit,
            agent,
        }
    }

    fn region_from(body: &Value, prefix: &str) -> RegionEnd {
        let server_uri = if prefix == "source" {
            body.get("source_server_uri")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };
        RegionEnd {
            server_uri,
            region_id: uuid_of(body.get(format!("{prefix}_uuid"))),
            name: body
                .get(format!("{prefix}_name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            loc_x: body
                .get(format!("{prefix}_x"))
                .and_then(Value::as_str)
                .and_then(|text| text.parse().ok())
                .unwrap_or(0),
            loc_y: body
                .get(format!("{prefix}_y"))
                .and_then(Value::as_str)
                .and_then(|text| text.parse().ok())
                .unwrap_or(0),
        }
    }

    fn context_from(value: Option<&Value>) -> TransferContext {
        let Some(map) = value else {
            return TransferContext::negotiated();
        };
        TransferContext {
            inbound: json_f32(map.get("InboundVersion")),
            outbound: json_f32(map.get("OutboundVersion")),
            wearables_count: json_i32(map.get("WearablesCount")),
        }
    }

    fn circuit_from(body: &Value) -> Circuit {
        Circuit {
            agent_id: uuid_of(body.get("agent_id")),
            base_folder: uuid_of(body.get("base_folder")),
            caps_path: text_of(body.get("caps_path")),
            children_seeds: seeds_of(body.get("children_seeds")),
            child: body.get("child").and_then(Value::as_bool).unwrap_or(false),
            circuit_code: text_of(body.get("circuit_code")).parse().unwrap_or(0),
            first_name: text_of(body.get("first_name")),
            last_name: text_of(body.get("last_name")),
            inventory_folder: uuid_of(body.get("inventory_folder")),
            secure_session_id: uuid_of(body.get("secure_session_id")),
            session_id: uuid_of(body.get("session_id")),
            service_session_id: text_of(body.get("service_session_id")),
            start_pos: parse_vec3(body.get("start_pos").and_then(Value::as_str).unwrap_or("")),
            client_ip: text_of(body.get("client_ip")),
            viewer: text_of(body.get("viewer")),
            channel: text_of(body.get("channel")),
            mac: text_of(body.get("mac")),
            id0: text_of(body.get("id0")),
            far: json_f32(body.get("far")),
            appearance: body.get("packed_appearance").map(appearance_from_json),
            service_urls: service_urls_of(body.get("service_urls")),
        }
    }

    fn agent_from(body: &Value) -> AgentData {
        AgentData {
            region_id: uuid_of(body.get("region_id")),
            circuit_code: text_of(body.get("circuit_code")).parse().unwrap_or(0),
            agent_id: uuid_of(body.get("agent_uuid")),
            session_id: uuid_of(body.get("session_uuid")),
            position: parse_vec3(text_of(body.get("position")).as_str()),
            velocity: parse_vec3(text_of(body.get("velocity")).as_str()),
            center: parse_vec3(text_of(body.get("center")).as_str()),
            size: parse_vec3(text_of(body.get("size")).as_str()),
            at_axis: parse_vec3(text_of(body.get("at_axis")).as_str()),
            left_axis: parse_vec3(text_of(body.get("left_axis")).as_str()),
            up_axis: parse_vec3(text_of(body.get("up_axis")).as_str()),
            wait_for_root: body
                .get("wait_for_root")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            far: json_f32(body.get("far")),
            throttles: bytes_of(body.get("throttles")),
            locomotion_state: text_of(body.get("locomotion_state")).parse().unwrap_or(0),
            head_rotation: parse_vec4(text_of(body.get("head_rotation")).as_str()),
            body_rotation: parse_vec4(text_of(body.get("body_rotation")).as_str()),
            control_flags: text_of(body.get("control_flags")).parse().unwrap_or(0),
            god_data: body.get("god_data").is_some(),
            agent_access: text_of(body.get("agent_access")).parse().unwrap_or(0),
            default_animation: body.get("default_animation").map(anim_from),
            animation_state: body.get("animation_state").map(anim_from),
            motion_state: json_i32(body.get("motion_state")),
            appearance: body.get("packed_appearance").map(appearance_from_json),
            sit_offset: parse_vec3(text_of(body.get("sit_offset")).as_str()),
            children_seeds: seeds_of(body.get("children_seeds")),
            callback: body
                .get("cb_uri")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }

    fn anim_from(value: &Value) -> Anim {
        Anim {
            animation: uuid_of(value.get("animation")),
            object_id: uuid_of(value.get("object_id")),
            sequence: json_i32(value.get("seq_num")),
        }
    }

    fn seeds_of(value: Option<&Value>) -> Vec<ChildSeed> {
        let Some(Value::Array(items)) = value else {
            return Vec::new();
        };
        items
            .iter()
            .map(|item| ChildSeed {
                handle: text_of(item.get("handle")).parse().unwrap_or(0),
                seed: text_of(item.get("seed")),
            })
            .collect()
    }

    fn service_urls_of(value: Option<&Value>) -> Vec<(String, String)> {
        let Some(Value::Array(items)) = value else {
            return Vec::new();
        };
        let mut pairs = Vec::new();
        let mut index = 0;
        while index + 1 < items.len() {
            pairs.push((
                text_of(Some(&items[index])),
                text_of(Some(&items[index + 1])),
            ));
            index += 2;
        }
        pairs
    }

    fn bytes_of(value: Option<&Value>) -> Vec<u8> {
        let Some(Value::Array(items)) = value else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| item.as_u64().and_then(|number| u8::try_from(number).ok()))
            .collect()
    }

    fn uuid_of(value: Option<&Value>) -> Uuid {
        value
            .and_then(Value::as_str)
            .and_then(|text| Uuid::parse_str(text).ok())
            .unwrap_or_else(Uuid::nil)
    }

    fn text_of(value: Option<&Value>) -> String {
        value.and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn json_i32(value: Option<&Value>) -> i32 {
        value
            .and_then(Value::as_i64)
            .and_then(|number| i32::try_from(number).ok())
            .unwrap_or(0)
    }

    fn parse_vec3(text: &str) -> [f32; 3] {
        let mut out = [0.0; 3];
        fill_components(text, &mut out);
        out
    }

    fn parse_vec4(text: &str) -> [f32; 4] {
        let mut out = [0.0; 4];
        fill_components(text, &mut out);
        out
    }

    fn fill_components(text: &str, out: &mut [f32]) {
        let Some(inner) = text
            .strip_prefix('<')
            .and_then(|text| text.strip_suffix('>'))
        else {
            return;
        };
        for (slot, part) in out.iter_mut().zip(inner.split(',')) {
            if let Ok(value) = part.trim().parse() {
                *slot = value;
            }
        }
    }

    fn assert_same(left: &Value, right: &Value, path: &str) {
        match (left, right) {
            (Value::Object(left), Value::Object(right)) => {
                let mut left_keys: Vec<_> = left.keys().collect();
                let mut right_keys: Vec<_> = right.keys().collect();
                left_keys.sort();
                right_keys.sort();
                assert_eq!(left_keys, right_keys, "{path} keys");
                for key in left_keys {
                    assert_same(&left[key], &right[key], &format!("{path}.{key}"));
                }
            }
            (Value::Array(left), Value::Array(right)) => {
                assert_eq!(left.len(), right.len(), "{path} len");
                for (index, (left, right)) in left.iter().zip(right).enumerate() {
                    assert_same(left, right, &format!("{path}[{index}]"));
                }
            }
            (Value::Number(left), Value::Number(right)) => {
                let (Some(left), Some(right)) = (left.as_f64(), right.as_f64()) else {
                    assert_eq!(left, right, "{path}");
                    return;
                };
                assert!((left - right).abs() <= 1e-5, "{path}: {left} vs {right}");
            }
            _ => assert_eq!(left, right, "{path}"),
        }
    }

    fn circuit_blank() -> Circuit {
        Circuit {
            agent_id: Uuid::nil(),
            base_folder: Uuid::nil(),
            caps_path: String::new(),
            children_seeds: Vec::new(),
            child: false,
            circuit_code: 0,
            first_name: String::new(),
            last_name: String::new(),
            inventory_folder: Uuid::nil(),
            secure_session_id: Uuid::nil(),
            session_id: Uuid::nil(),
            service_session_id: String::new(),
            start_pos: [0.0; 3],
            client_ip: String::new(),
            viewer: String::new(),
            channel: String::new(),
            mac: String::new(),
            id0: String::new(),
            far: 0.0,
            appearance: None,
            service_urls: Vec::new(),
        }
    }

    fn agent_blank() -> AgentData {
        AgentData {
            region_id: Uuid::nil(),
            circuit_code: 0,
            agent_id: Uuid::nil(),
            session_id: Uuid::nil(),
            position: [0.0; 3],
            velocity: [0.0; 3],
            center: [0.0; 3],
            size: [0.0; 3],
            at_axis: [0.0; 3],
            left_axis: [0.0; 3],
            up_axis: [0.0; 3],
            wait_for_root: false,
            far: 0.0,
            throttles: Vec::new(),
            locomotion_state: 0,
            head_rotation: [0.0; 4],
            body_rotation: [0.0; 4],
            control_flags: 0,
            god_data: false,
            agent_access: 0,
            default_animation: None,
            animation_state: None,
            motion_state: 0,
            appearance: None,
            sit_offset: [0.0; 3],
            children_seeds: Vec::new(),
            callback: None,
        }
    }

    fn sample_region() -> RegionEnd {
        RegionEnd {
            server_uri: "http://127.0.0.1:9/".to_string(),
            region_id: Uuid::from_u128(1),
            name: "Test".to_string(),
            loc_x: 256,
            loc_y: 256,
        }
    }

    fn bound_region(addr: std::net::SocketAddr) -> RegionEnd {
        RegionEnd {
            server_uri: format!("http://{addr}/"),
            region_id: Uuid::from_u128(7),
            name: "Neighbour".to_string(),
            loc_x: 256_000,
            loc_y: 256_000,
        }
    }

    enum Scripted {
        Drop,
        Text(u16, &'static str),
    }

    struct Hit {
        header: String,
        body: Vec<u8>,
    }

    async fn scripted(
        steps: Vec<Scripted>,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<Vec<Hit>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut hits = Vec::new();
            for step in steps {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let (header, body) = read_http(&mut stream).await;
                let response = match step {
                    Scripted::Drop => None,
                    Scripted::Text(status, text) => Some((status, text)),
                };
                hits.push(Hit { header, body });
                if let Some((status, text)) = response {
                    let raw = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    );
                    let _ = stream.write_all(raw.as_bytes()).await;
                }
            }
            hits
        });
        (addr, handle)
    }

    async fn finish_script(mut handle: tokio::task::JoinHandle<Vec<Hit>>) -> Vec<Hit> {
        match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
            Ok(Ok(hits)) => hits,
            Ok(Err(error)) => panic!("script server joined: {error}"),
            Err(_) => {
                handle.abort();
                panic!("script server timed out");
            }
        }
    }

    async fn read_http(stream: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let count = stream.read(&mut tmp).await.unwrap_or(0);
            if count == 0 {
                break 0;
            }
            buf.extend_from_slice(&tmp[..count]);
            if let Some(position) = buf.windows(4).position(|part| part == b"\r\n\r\n") {
                break position;
            }
            if buf.len() > MAX_BODY {
                break buf.len();
            }
        };
        let header = String::from_utf8_lossy(&buf[..header_end.min(buf.len())]).into_owned();
        let mut body = if header_end + 4 <= buf.len() {
            buf[header_end + 4..].to_vec()
        } else {
            Vec::new()
        };
        let length = content_length(&header);
        while body.len() < length {
            let count = stream.read(&mut tmp).await.unwrap_or(0);
            if count == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..count]);
        }
        body.truncate(length);
        (header, body)
    }

    fn content_length(header: &str) -> usize {
        header
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
            .unwrap_or(0)
    }

    fn header_has(header: &str, name: &str, value: &str) -> bool {
        header.lines().any(|line| {
            line.split_once(':').is_some_and(|(key, got)| {
                key.eq_ignore_ascii_case(name) && got.trim().eq_ignore_ascii_case(value)
            })
        })
    }

    fn request_method(header: &str) -> &str {
        header
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().next())
            .unwrap_or("")
    }

    fn request_target(header: &str) -> &str {
        header
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("")
    }

    fn gunzip(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        GzDecoder::new(body).read_to_end(&mut out).unwrap();
        out
    }

    fn http_text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn header_and_body(bytes: &[u8]) -> (&str, &[u8]) {
        let split = bytes
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap_or(bytes.len());
        let header = std::str::from_utf8(&bytes[..split.min(bytes.len())]).unwrap_or("");
        let body = if split + 4 <= bytes.len() {
            &bytes[split + 4..]
        } else {
            &[]
        };
        (header, body)
    }

    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }
}
