// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pin8 child-view capture: take the viewer receipts on its separate child socket.

use std::fs;
use std::path::Path;

use ferroan_bot::scenario::child_view::{EventPoll, Observation};

use super::login_capture::{Message, bounded, records};
use ferroan_capture::{Kind, Record};
use ferroan_llsd::Llsd;
use ferroan_protocol::msgs::{
    AgentMovementComplete, AgentUpdate, CompleteAgentMovement, LogoutReply, LogoutRequest,
    ObjectUpdate, RegionHandshakeReply, UseCircuitCode,
    object_update::{ObjectData, RegionData},
};
use ferroan_protocol::{Msg, Reader, Writer, parse_packet};
use serde_json::{Value, json};
use uuid::Uuid;

const OBSERVER: &str = crate::live::ESTATE_OWNER;
const RESIDENT: &str = crate::live::CHAT_USER;
fn neighbour_handle() -> u64 {
    ferroan_bot::scenario::region_handle(super::NEIGHBOUR_X, super::NEIGHBOUR_Y)
}

struct Circuit {
    session: Uuid,
    code: u32,
}

fn circuit(udp: &[Record], agent: Uuid) -> Result<Circuit, String> {
    for record in udp.iter().filter(|r| r.kind == Kind::UdpIn) {
        let packet = parse_packet(&record.bytes).map_err(|e| e.to_string())?;
        if packet.id == UseCircuitCode::ID {
            let id = packet
                .decode::<UseCircuitCode>()
                .map_err(|e| e.to_string())?
                .circuit_code;
            if id.id == agent && !id.session_id.is_nil() && id.code != 0 {
                return Ok(Circuit {
                    session: id.session_id,
                    code: id.code,
                });
            }
        }
    }
    Err("missing UseCircuitCode".into())
}

/// Only take messages with this viewer's agent and session on this region.
/// A resident's own full updates on the same server are not child receipts.
fn udp_events(
    udp: &[Record],
    agent: Uuid,
    circuit: &Circuit,
    label: &str,
    start: u64,
    output: &mut Vec<Value>,
) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for r in udp {
        let p = parse_packet(&r.bytes).map_err(|e| e.to_string())?;
        let (name, id, session, fields) = match (r.kind, p.id) {
            (Kind::UdpIn, UseCircuitCode::ID) => {
                let d = p
                    .decode::<UseCircuitCode>()
                    .map_err(|e| e.to_string())?
                    .circuit_code;
                ("UseCircuitCode", d.id, d.session_id, json!({}))
            }
            (Kind::UdpIn, CompleteAgentMovement::ID) => {
                let d = p
                    .decode::<CompleteAgentMovement>()
                    .map_err(|e| e.to_string())?
                    .agent_data;
                ("CompleteAgentMovement", d.agent_id, d.session_id, json!({}))
            }
            (Kind::UdpOut, AgentMovementComplete::ID) => {
                let m = p
                    .decode::<AgentMovementComplete>()
                    .map_err(|e| e.to_string())?;
                (
                    "AgentMovementComplete",
                    m.agent_data.agent_id,
                    m.agent_data.session_id,
                    json!({"handle":m.data.region_handle,"position":m.data.position.to_array()}),
                )
            }
            (Kind::UdpIn, RegionHandshakeReply::ID) => {
                let m = p
                    .decode::<RegionHandshakeReply>()
                    .map_err(|e| e.to_string())?;
                (
                    "RegionHandshakeReply",
                    m.agent_data.agent_id,
                    m.agent_data.session_id,
                    json!({"flags":m.region_info.flags}),
                )
            }
            (Kind::UdpIn, AgentUpdate::ID) => {
                let d = p
                    .decode::<AgentUpdate>()
                    .map_err(|e| e.to_string())?
                    .agent_data;
                if d.control_flags & 1 == 0 {
                    continue;
                }
                (
                    "AgentUpdate",
                    d.agent_id,
                    d.session_id,
                    json!({"controls":d.control_flags,"camera":d.camera_center.to_array(),"far":d.far}),
                )
            }
            (Kind::UdpIn, LogoutRequest::ID) => {
                let d = p
                    .decode::<LogoutRequest>()
                    .map_err(|e| e.to_string())?
                    .agent_data;
                ("LogoutRequest", d.agent_id, d.session_id, json!({}))
            }
            (Kind::UdpOut, LogoutReply::ID) => {
                let d = p
                    .decode::<LogoutReply>()
                    .map_err(|e| e.to_string())?
                    .agent_data;
                ("LogoutReply", d.agent_id, d.session_id, json!({}))
            }
            _ => continue,
        };
        if id == agent && session == circuit.session && seen.insert(name) {
            output.push(json!({"ms":r.time_ms.checked_sub(start).ok_or("UDP clock before capture")?, "circuit":label,"message":name,"fields":fields}));
        }
    }
    Ok(())
}

fn redact_events(proxy: &[Record], poll: &EventPoll) -> Result<(Vec<u8>, String), String> {
    if !poll.path.starts_with("/CE/") {
        return Err("invalid home event queue path".into());
    }
    let mut request = Llsd::new_map();
    if let Some(ack) = poll.ack {
        request.insert("ack", ack);
    }
    request.insert("done", false);
    let mut pending = false;
    let mut selected = None;
    for record in proxy {
        let message = Message::parse(&record.bytes)?;
        if record.kind == Kind::HttpRequest && message.path == poll.path {
            pending = message.method == "POST" && {
                let body = ferroan_llsd::from_xml(message.text()?.as_bytes())
                    .map_err(|e| e.to_string())?;
                body == request
            };
        } else if pending && record.kind == Kind::HttpResponse && message.status == Some(200) {
            // Agent admission can finish during the long poll. The viewer retained
            // its exact response body, so a concurrent proxy response cannot replace it.
            let Ok(text) = message.text() else { continue };
            if text.as_bytes() != poll.body {
                continue;
            }
            if selected.is_some() {
                return Err("ambiguous home event poll response".into());
            }
            let scrubbed = crate::caps_fixtures::scrub_events(text.as_bytes())?;
            let batch = ferroan_bot::caps::parse_event_queue(text.as_bytes())?;
            let seed = batch
                .events
                .get(1)
                .ok_or("missing neighbour communication event")?
                .body
                .get("seed-capability")
                .as_str()
                .to_owned();
            selected = Some((scrubbed, seed));
            pending = false;
        }
    }
    selected.ok_or_else(|| "home proxy lacks the observed event poll and response".into())
}

const RESIDENT_NAME: &[u8] =
    b"FirstName STRING RW SV Chat\nLastName STRING RW SV Bot\nTitle STRING RW SV \0";
const ALIAS_NAME: &[u8] =
    b"FirstName STRING RW SV Test\nLastName STRING RW SV Doe\nTitle STRING RW SV \0";

fn child_block(
    receipt: Option<&ferroan_protocol::IncomingPacket>,
    udp: &[Record],
    name: &str,
    id: Uuid,
    code: u8,
    alias: Uuid,
    start: u64,
) -> Result<Option<(Vec<u8>, Value)>, String> {
    let Some(receipt) = receipt else {
        return Ok(None);
    };
    for r in udp.iter().filter(|r| r.kind == Kind::UdpOut) {
        let p = parse_packet(&r.bytes).map_err(|e| e.to_string())?;
        if p.id != ObjectUpdate::ID || p.sequence != receipt.sequence || p.body != receipt.body {
            continue;
        }
        let mut reader = Reader::new(&p.body);
        let region = RegionData::decode(&mut reader).map_err(|e| e.to_string())?;
        if !p.reliable() || region.region_handle != neighbour_handle() {
            return Err("full update from wrong region or on an unreliable packet".into());
        }
        let count = reader.u8().map_err(|e| e.to_string())?;
        let mut selected = None;
        for _ in 0..count {
            let begin = p.body.len() - reader.remaining();
            let block = ObjectData::decode(&mut reader).map_err(|e| e.to_string())?;
            if block.full_id == id && block.p_code == code {
                let mut original = Writer::new();
                block.encode(&mut original);
                if original.buf != p.body[begin..p.body.len() - reader.remaining()] {
                    return Err(format!(
                        "unredacted {name} does not reproduce the C# block bytes"
                    ));
                }
                if selected.replace(block).is_some() {
                    return Err(format!("duplicate {name} block"));
                }
            }
        }
        if reader.remaining() != 0 {
            return Err("trailing ObjectUpdate bytes".into());
        }
        let mut block =
            selected.ok_or("server update lacks the expected resident or stored box")?;
        block.full_id = alias;
        if !block.owner_id.is_nil() {
            if block.owner_id.to_string() != OBSERVER && block.owner_id.to_string() != RESIDENT {
                return Err("unexpected owner on captured block".into());
            }
            block.owner_id = if block.owner_id.to_string() == OBSERVER {
                Uuid::from_u128(0x00000000_0000_4000_8000_000000000004)
            } else {
                Uuid::from_u128(5)
            };
        }
        match code {
            47 if block.name_value == RESIDENT_NAME => block.name_value = ALIAS_NAME.to_vec(),
            9 if block.name_value.is_empty() => {}
            _ => return Err(format!("unexpected {name} NameValue fields")),
        }
        let mut w = Writer::new();
        block.encode(&mut w);
        return Ok(Some((
            w.buf,
            json!({"ms":r.time_ms.checked_sub(start).ok_or("receipt clock before capture")?, "circuit":"observer-child","message":"ObjectUpdate","entity":name,"handle":region.region_handle,"file":format!("{name}.bin")}),
        )));
    }
    Err("viewer receipt has no matching C# datagram with the same sequence and body".into())
}

fn redact_appearance(value: &mut Value) -> Result<(), String> {
    let map = value.as_object().ok_or("invalid packed appearance")?;
    if map.keys().any(|key| {
        !matches!(
            key.as_str(),
            "height" | "wearables" | "textures" | "visualparams" | "attachments"
        )
    }) {
        return Err("unrecognized packed appearance field".into());
    }
    if !value["attachments"].as_array().is_some_and(Vec::is_empty) {
        return Err("expected an empty attachment list".into());
    }
    if !value["height"].is_number()
        || !value["textures"]
            .as_array()
            .is_some_and(|v| v.iter().all(is_uuid))
        || !value["visualparams"]
            .as_array()
            .is_some_and(|v| v.iter().all(|p| p.as_u64().is_some_and(|n| n <= 255)))
    {
        return Err("unexpected packed appearance values".into());
    }
    let wearables = value["wearables"]
        .as_array_mut()
        .ok_or("missing child wearables")?;
    let mut aliases = std::collections::BTreeMap::new();
    for entry in wearables.iter_mut() {
        for wearable in entry.as_array_mut().ok_or("invalid child wearable list")? {
            if !wearable
                .as_object()
                .is_some_and(|m| m.len() == 2 && m.contains_key("item") && m.contains_key("asset"))
                || !is_uuid(&wearable["asset"])
            {
                return Err("unexpected child wearable fields".into());
            }
            let item = wearable["item"]
                .as_str()
                .ok_or("missing wearable item")?
                .parse::<Uuid>()
                .map_err(|_| "invalid wearable item")?;
            if !item.is_nil() {
                let next = Uuid::from_u128(100 + aliases.len() as u128);
                wearable["item"] = json!(aliases.entry(item).or_insert(next).to_string());
            }
        }
    }
    Ok(())
}

fn is_uuid(value: &Value) -> bool {
    value.as_str().is_some_and(|s| Uuid::parse_str(s).is_ok())
}

// Only publish the known Pin8 field set. Keep wire types, service URL forms,
// appearance payloads and pose values; replace identities consistently.
fn redact_transfer(mut body: Value) -> Result<Value, String> {
    let map = body
        .as_object_mut()
        .ok_or("child request is not an object")?;
    for (key, value) in map {
        let alias = match key.as_str() {
            "agent_id" | "agent_uuid" => "00000000-0000-4000-8000-000000000004",
            "session_id" | "session_uuid" => "00000000-0000-4000-8000-000000000007",
            "secure_session_id" => "00000000-0000-4000-8000-000000000008",
            "caps_path" => "00000000000040008000000000000003",
            "circuit_code" if value.as_str() != Some("0") => "1",
            "client_ip" => "192.0.2.2",
            "first_name" => "Fixture",
            "last_name" => "Observer",
            "mac" | "id0" => "00000000000000000000000000000000",
            "source_uuid" | "destination_uuid" => "00000000-0000-4000-8000-000000000009",
            "source_name" | "destination_name" => "Fixture Neighbour",
            "source_server_uri" => "http://192.0.2.1:9001/",
            "serviceurls" | "service_urls" => {
                let expected = if key == "serviceurls" {
                    json!({"HomeURI":"/","InventoryServerURI":"/","AssetServerURI":"/"})
                } else {
                    json!([
                        "HomeURI",
                        "/",
                        "InventoryServerURI",
                        "/",
                        "AssetServerURI",
                        "/"
                    ])
                };
                if *value != expected {
                    return Err("unexpected test service URLs".into());
                }
                continue;
            }
            "packed_appearance" => {
                redact_appearance(value)?;
                continue;
            }
            "child" | "start_pos" | "far" | "context" | "source_x" | "source_y"
            | "destination_x" | "destination_y" | "teleport_flags" | "message_type"
            | "region_handle" | "position" | "velocity" | "center" | "size" | "at_axis"
            | "left_axis" | "up_axis" | "god_data" | "throttles" | "circuit_code" | "viewer"
            | "channel" => continue,
            _ => {
                return Err("unrecognized field in child request; review before publishing".into());
            }
        };
        *value = json!(alias);
    }
    Ok(body)
}

fn transfers(run: &Path, circuit: &Circuit, seed: &str, start: u64) -> Result<Vec<Value>, String> {
    let prefix = format!("/agent/{OBSERVER}/");
    let mut result = Vec::new();
    let proxy = records(&run.join("proxy-neighbour.cap"))?;
    for (index, r) in proxy
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == Kind::HttpRequest)
    {
        let m = Message::parse(&r.bytes)?;
        let Some(suffix) = m.path.strip_prefix(&prefix) else {
            continue;
        };
        let mut fields = json!({});
        if m.method == "POST" || m.method == "PUT" {
            let body: Value = serde_json::from_str(&m.text()?).map_err(|_| "invalid child JSON")?;
            let (agent, session) = if m.method == "POST" {
                ("agent_id", "session_id")
            } else {
                ("agent_uuid", "session_uuid")
            };
            if body[agent] != OBSERVER
                || body[session] != circuit.session.to_string()
                || body["destination_uuid"] != super::NEIGHBOUR_UUID
            {
                return Err("child transfer has wrong identity or destination".into());
            }
            if m.method == "POST" {
                if body["child"] != true
                    || crate::caps_fixtures::json_circuit(body.get("circuit_code"))? != circuit.code
                {
                    return Err("admission differs from the viewer's child circuit".into());
                }
                let path = reqwest::Url::parse(seed).map_err(|_| "invalid observer child seed")?;
                crate::caps_fixtures::seed_belongs(
                    path.path(),
                    body["caps_path"]
                        .as_str()
                        .ok_or("child admission lacks caps_path")?,
                )?;
                if body["source_uuid"] != super::NEIGHBOUR_UUID {
                    return Err("unexpected source region on child creation".into());
                }
            }
            fields = redact_transfer(body)?;
        } else if m.method == "DELETE" {
            if suffix.split('/').next() != Some(super::NEIGHBOUR_UUID) {
                return Err("child delete for wrong region".into());
            }
        } else {
            return Err("unexpected neighbour transfer for observer".into());
        }
        let response = super::login_capture::paired(&proxy, index)?;
        let reply = response.text()?;
        let accepted = match m.method.as_str() {
            "POST" => serde_json::from_str::<Value>(&reply)
                .ok()
                .is_some_and(|v| v["success"] == true),
            "PUT" => reply.eq_ignore_ascii_case("true"),
            "DELETE" => reply.ends_with(OBSERVER),
            _ => false,
        };
        if response.status != Some(200) || !accepted {
            return Err(format!("unsuccessful child {} reply", m.method));
        }
        result.push(json!({"ms":r.time_ms.checked_sub(start).ok_or("proxy clock before capture")?,"method":m.method,"agent":"observer","destination":"neighbour","fields":fields,"status":response.status}));
    }
    Ok(result)
}

pub(super) fn finish(run: &Path, observed: &Observation) -> Result<String, String> {
    let output = run.join("child-view-fixture");
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let home = records(&run.join("capture-udp.cap"))?;
    let neighbour = records(&run.join("capture-udp-neighbour.cap"))?;
    let start = home
        .iter()
        .chain(&neighbour)
        .map(|r| r.time_ms)
        .min()
        .ok_or("empty UDP capture")?;
    let observer = OBSERVER.parse::<Uuid>().map_err(|e| e.to_string())?;
    let resident = RESIDENT.parse::<Uuid>().map_err(|e| e.to_string())?;
    let root = circuit(&home, observer)?;
    let child = circuit(&neighbour, observer)?;
    if root.code != child.code || root.session != child.session {
        return Err("child and home identities differ".into());
    }
    let mut udp = Vec::new();
    udp_events(&home, observer, &root, "observer-home", start, &mut udp)?;
    udp_events(
        &neighbour,
        observer,
        &child,
        "observer-child",
        start,
        &mut udp,
    )?;
    udp_events(
        &neighbour,
        resident,
        &circuit(&neighbour, resident)?,
        "resident-neighbour",
        start,
        &mut udp,
    )?;
    for (name, receipt, id, code, alias) in [
        ("avatar", observed.avatar.as_ref(), resident, 47, 5),
        (
            "prim",
            observed.prim.as_ref(),
            ferroan_bot::scenario::child_view::PRIM,
            9,
            6,
        ),
    ] {
        if let Some((bytes, event)) = child_block(
            receipt,
            &neighbour,
            name,
            id,
            code,
            Uuid::from_u128(alias),
            start,
        )? {
            fs::write(output.join(format!("{name}.bin")), bytes).map_err(|e| e.to_string())?;
            udp.push(event);
        }
    }
    udp.sort_by_key(|v| v["ms"].as_u64());
    let (events, seed) = redact_events(&records(&run.join("proxy-home.cap"))?, &observed.events)?;
    fs::write(output.join("events.xml"), events).map_err(|e| e.to_string())?;
    let requests = transfers(run, &child, &seed, start)?;
    let trace = json!({"udp":udp,"requests":requests});
    fs::write(
        output.join("trace.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&trace).map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok(format!(
        "{}\nchild-view fixture: {}",
        replay(&output)?,
        output.display()
    ))
}

/// Replay the capture from each circuit separately. A neighbour-wide observer
/// would also see the resident viewer receiving its own avatar.
fn replay(dir: &Path) -> Result<String, String> {
    let bytes = bounded(&dir.join("trace.json"), 1024 * 1024)?;
    let trace: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    replay_trace(dir, &trace)
}

fn replay_trace(dir: &Path, trace: &Value) -> Result<String, String> {
    let udp = trace["udp"].as_array().ok_or("missing UDP records")?;
    let find = |circuit: &str, msg: &str| {
        udp.iter()
            .find(|v| v["circuit"] == circuit && v["message"] == msg)
    };
    let event = |circuit: &str, msg: &str| {
        find(circuit, msg).ok_or_else(|| format!("missing {msg} on {circuit}"))
    };
    let stamp = |v: &Value| {
        v["ms"]
            .as_u64()
            .ok_or("missing capture timestamp".to_string())
    };
    let home = "observer-home";
    let child = "observer-child";
    let resident = "resident-neighbour";
    let home_handle = ferroan_bot::scenario::region_handle(super::HOME_X, super::HOME_Y);
    for (c, h) in [(home, home_handle), (resident, neighbour_handle())] {
        event(c, "UseCircuitCode")?;
        event(c, "CompleteAgentMovement")?;
        if event(c, "AgentMovementComplete")?["fields"]["handle"] != h {
            return Err("root arrived on wrong region".into());
        }
        event(c, "LogoutReply")?;
    }
    let arrival = stamp(event(home, "AgentMovementComplete")?)?;
    let child_open = stamp(event(child, "UseCircuitCode")?)?;
    let ready = stamp(event(child, "RegionHandshakeReply")?)?;
    let movement = stamp(event(home, "AgentUpdate")?)?;
    let logout = stamp(event(home, "LogoutRequest")?)?;
    if !(arrival <= child_open
        && child_open <= ready
        && ready <= movement
        && movement < logout
        && logout <= stamp(event(home, "LogoutReply")?)?)
    {
        return Err("invalid home/child circuit chronology".into());
    }
    for name in [
        "CompleteAgentMovement",
        "AgentMovementComplete",
        "LogoutRequest",
    ] {
        if find(child, name).is_some() {
            return Err(format!("unexpected {name} on child"));
        }
    }

    let requests = trace["requests"]
        .as_array()
        .ok_or("missing child requests")?;
    let (create, rest) = requests.split_first().ok_or("missing child admission")?;
    let (close, updates) = rest.split_last().ok_or("missing child close")?;
    if requests
        .iter()
        .any(|r| r["agent"] != "observer" || r["destination"] != "neighbour" || r["status"] != 200)
        || create["method"] != "POST"
        || create["fields"]["child"] != true
        || create["fields"]["caps_path"] != "00000000000040008000000000000003"
        || stamp(create)? > child_open
        || close["method"] != "DELETE"
        || stamp(close)? < logout
    {
        return Err("invalid child admission or close".into());
    }
    let start_x = event(home, "AgentMovementComplete")?["fields"]["position"][0]
        .as_f64()
        .ok_or("missing root position")?;
    let translated = create["fields"]["start_pos"]
        .as_str()
        .and_then(super::login_probe::vector)
        .ok_or("invalid child start position")?;
    if (start_x - 256.0 - f64::from(translated[0])).abs() > 0.01
        || create["fields"]["teleport_flags"] != "0"
    {
        return Err("unexpected child start position or flags".into());
    }
    let mut moved = false;
    for r in updates {
        let f = &r["fields"];
        if r["method"] != "PUT"
            || f["message_type"] != "AgentPosition"
            || f["agent_uuid"] != create["fields"]["agent_id"]
            || f["session_uuid"] != create["fields"]["session_id"]
            || f["region_handle"].as_str() != Some(&home_handle.to_string())
        {
            return Err("unexpected child update".into());
        }
        let pos = f["position"]
            .as_str()
            .and_then(super::login_probe::vector)
            .ok_or("invalid child position")?;
        let t = stamp(r)?;
        moved |= movement <= t
            && t < logout
            && f64::from(pos[0]) - start_x > 10.0
            && f["far"] == create["fields"]["far"]
            && f["far"] == event(home, "AgentUpdate")?["fields"]["far"];
    }
    if !moved {
        return Err("missing movement-triggered AgentPosition".into());
    }

    let xml = bounded(&dir.join("events.xml"), 65536)?;
    let eq = ferroan_bot::caps::parse_event_queue(&xml)?;
    let [enable, establish] = eq.events.as_slice() else {
        return Err("expected two child events".into());
    };
    let first = ferroan_bot::caps::crossing_hint(&enable.message, &enable.body)
        .ok_or("invalid EnableSimulator")?;
    let second = ferroan_bot::caps::crossing_hint(&establish.message, &establish.body)
        .ok_or("invalid EstablishAgentCommunication")?;
    if enable.message != "EnableSimulator"
        || establish.message != "EstablishAgentCommunication"
        || first.handle != Some(neighbour_handle())
        || first.ip != second.ip
        || first.port != second.port
        || second.seed.is_none()
    {
        return Err("events do not identify the same neighbour".into());
    }

    let seed = reqwest::Url::parse(second.seed.as_deref().ok_or("missing child seed")?)
        .map_err(|_| "invalid child seed")?;
    crate::caps_fixtures::seed_belongs(
        seed.path(),
        create["fields"]["caps_path"]
            .as_str()
            .ok_or("missing create seed")?,
    )?;
    let mut ids = Vec::new();
    for (name, code, alias) in [("avatar", 47, 5), ("prim", 9, 6)] {
        let receipt = udp
            .iter()
            .find(|r| {
                r["circuit"] == child && r["entity"] == name && r["message"] == "ObjectUpdate"
            })
            .ok_or_else(|| format!("missing child receipt for {name}"))?;
        let t = stamp(receipt)?;
        if receipt["handle"] != neighbour_handle()
            || t < ready
            || t > logout
            || (name == "avatar" && t < stamp(event(resident, "AgentMovementComplete")?)?)
        {
            return Err("object delivered outside its region or session".into());
        }
        let bytes = bounded(&dir.join(format!("{name}.bin")), 65536)?;
        let mut reader = ferroan_protocol::Reader::new(&bytes);
        let block = ferroan_protocol::msgs::object_update::ObjectData::decode(&mut reader)
            .map_err(|e| e.to_string())?;
        if reader.remaining() != 0
            || block.full_id != Uuid::from_u128(alias)
            || block.p_code != code
            || (code == 47 && block.name_value != ALIAS_NAME)
            || block.id == 0
            || block.parent_id != 0
            || ids.contains(&block.id)
        {
            return Err(format!("incorrect {name} identity or full block"));
        }
        ids.push(block.id);
    }
    Ok("child view: resident avatar and stored prim on observer child, movement AgentPosition, home root through logout, child closed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn http_record(kind: Kind, start: &str, body: &[u8]) -> Record {
        let mut bytes = format!("{start}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body);
        Record {
            kind,
            time_ms: 0,
            bytes,
        }
    }

    #[test]
    fn pin8_child_view_scrubs_only_the_receipted_home_poll() {
        let public =
            include_str!("../../fixtures/child-view/events.xml").replace("<llsd>", "<llsd>\n");
        let original = public
            .replace("192.0.2.1", "127.0.0.1")
            .replace("wAACAQ==", "fwAAAQ==")
            .replace("00000000-0000-4000-8000-000000000004", OBSERVER);
        let mut poll = EventPoll {
            path: "/CE/test".into(),
            ack: Some(17),
            body: original.into_bytes(),
        };
        let response = |body: &[u8]| http_record(Kind::HttpResponse, "HTTP/1.1 200 OK", body);
        let mut proxy = vec![
            http_record(Kind::HttpRequest, "POST /CE/test HTTP/1.1", b"<llsd><map><key>ack</key><integer>17</integer><key>done</key><boolean>false</boolean></map></llsd>"),
            http_record(Kind::HttpRequest, "POST /agent/test HTTP/1.1", b"{}"),
            response(b"{}"), response(&poll.body)
        ];
        assert_eq!(redact_events(&proxy, &poll).unwrap().0, public.as_bytes());
        poll.ack = Some(16);
        assert!(redact_events(&proxy, &poll).is_err());
        poll.ack = Some(17);
        poll.path = "/CE/another".into();
        assert!(redact_events(&proxy, &poll).is_err());
        poll.path = "/CE/test".into();
        proxy.pop();
        assert!(redact_events(&proxy, &poll).is_err());
        for name in ["EnableSimulator", "EstablishAgentCommunication"] {
            poll.body = format!("<llsd><map><key>events</key><array><map><key>message</key><string>{name}</string></map></array><key>id</key><integer>18</integer></map></llsd>").into_bytes();
            proxy.push(response(&poll.body));
        }
        assert!(redact_events(&proxy, &poll).is_err());
    }

    #[test]
    fn pin8_child_view_refuses_unreviewed_appearance_data() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/child-view/trace.json")).unwrap();
        let appearance = &fixture["requests"][0]["fields"]["packed_appearance"];
        let mut unknown = appearance.clone();
        unknown["extra"] = json!("private");
        assert!(redact_appearance(&mut unknown).is_err());
        let mut attached = appearance.clone();
        attached["attachments"] = json!([{"item": Uuid::nil().to_string()}]);
        assert!(redact_appearance(&mut attached).is_err());
        let mut wearable = appearance.clone();
        wearable["wearables"][0][0]["other"] = json!("private");
        assert!(redact_appearance(&mut wearable).is_err());
    }

    #[test]
    fn pin8_child_view_replays_the_two_roots_and_the_observer_child() {
        let dir = crate::repo_root().join("fixtures/child-view");
        replay(&dir).unwrap();
        let mut digest = Sha256::new();
        for name in ["trace.json", "events.xml", "avatar.bin", "prim.bin"] {
            digest.update(bounded(&dir.join(name), 1024 * 1024).unwrap());
        }
        assert_eq!(
            format!("{:x}", digest.finalize()),
            "3194f67e7a44aed4629133eafcfd3fe444b1e26572b0e494d110e419906804be"
        );
    }

    #[test]
    fn pin8_child_view_rejects_the_residents_own_traffic_and_a_stationary_observer() {
        let dir = crate::repo_root().join("fixtures/child-view");
        let original: Value =
            serde_json::from_slice(&bounded(&dir.join("trace.json"), 1024 * 1024).unwrap())
                .unwrap();
        let mut wrong = original.clone();
        for p in wrong["udp"].as_array_mut().unwrap() {
            if p["entity"] == "avatar" {
                p["circuit"] = json!("resident-neighbour");
            }
        }
        assert_eq!(
            replay_trace(&dir, &wrong).unwrap_err(),
            "missing child receipt for avatar"
        );
        let mut still = original;
        for r in still["requests"].as_array_mut().unwrap() {
            if r["method"] == "PUT" {
                r["fields"]["position"] = json!("<200, 128, 30>");
            }
        }
        assert_eq!(
            replay_trace(&dir, &still).unwrap_err(),
            "missing movement-triggered AgentPosition"
        );
    }
}
