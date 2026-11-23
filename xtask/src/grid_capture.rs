// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Extract the four Q6 control-plane exchanges from a local test-grid capture.
//! Only generated identities and test endpoint addresses are substituted. The
//! compared form/JSON fields, methods, statuses and content types remain.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use ferroan_capture::{Kind, Record, read_complete_prefix};
use ferroan_http::{Field, FieldMap, build_query_string, parse_query_string};
use serde_json::{Map, Value, json};

const MAX_CAPTURE: u64 = 64 * 1024 * 1024;
const MAX_BODY: u64 = 1024 * 1024;
const AGENT: &str = "00000000-0000-4000-8000-000000000001";
const REGION: &str = "00000000-0000-4000-8000-000000000002";
const SECRET: &str = "00000000-0000-4000-8000-000000000003";
const SESSION: &str = "00000000-0000-4000-8000-000000000004";
const SECURE_SESSION: &str = "00000000-0000-4000-8000-000000000005";
const CAPS: &str = "00000000000040008000000000000006";

pub fn generate(directory: &Path) -> Result<String, String> {
    let udp = records(&directory.join("capture-udp.cap"))?;
    let login = crate::arrival_fixtures::prove(&udp)?;
    let exchanges = extract(
        directory,
        true,
        Some((login.circuit_code, login.session_id)),
    )?;
    let provenance = login.value;
    let target = crate::repo_root().join("fixtures/region-login");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    for name in ["register", "deregister", "query-access", "create-agent"] {
        let exchange = &exchanges[name];
        for direction in ["request", "response"] {
            let (extension, body) = if name == "register" || name == "deregister" {
                (
                    "txt",
                    exchange[direction]
                        .as_str()
                        .ok_or("form exchange is not text")?
                        .to_owned(),
                )
            } else {
                (
                    "json",
                    serde_json::to_string(&exchange[direction])
                        .map_err(|error| error.to_string())?,
                )
            };
            fs::write(target.join(format!("{name}.{direction}.{extension}")), body)
                .map_err(|error| error.to_string())?;
        }
    }
    fs::write(
        target.join("exchanges.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&exchanges).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    let manifest = json!({
        "revision":crate::arrival_fixtures::PIN,
        "source":"cargo xtask live run login grid",
        "exchanges":["register","deregister","query-access","create-agent"],
        "normalization":"Test identity and endpoint folding described in README.md; form ordering, status, content type and decoded bodies retained.",
        "provenance":provenance,
    });
    fs::write(
        target.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok("login fixtures: four C# request/reply exchanges scrubbed".to_string())
}

pub fn compare(directory: &Path) -> Result<String, String> {
    let actual = extract(directory, false, None)?;
    let path = crate::repo_root().join("fixtures/region-login/exchanges.json");
    let expected: Value = serde_json::from_slice(&bounded(&path, MAX_BODY)?)
        .map_err(|error| format!("invalid login fixture: {error}"))?;
    for name in ["register", "deregister", "query-access", "create-agent"] {
        if actual[name] != expected[name] {
            return Err(format!(
                "grid login HTTP differs for {name}; inspect the private captures under .scratch/live/grid"
            ));
        }
    }
    Ok("grid login http: register, QueryAccess, CreateAgent, deregister match C#".to_string())
}

/// Both harness accounts admitted. Responses match the committed C# fixtures.
/// Request bodies are not compared: the second account is not the fixture user.
pub fn compare_two_roots(directory: &Path) -> Result<String, String> {
    let home = records(&directory.join("proxy-home.cap"))?;
    let query_path = crate::repo_root().join("fixtures/region-login/query-access.response.json");
    let create_path = crate::repo_root().join("fixtures/region-login/create-agent.response.json");
    let query_fixture: Value = serde_json::from_slice(&bounded(&query_path, MAX_BODY)?)
        .map_err(|error| format!("invalid query-access fixture: {error}"))?;
    let create_fixture: Value = serde_json::from_slice(&bounded(&create_path, MAX_BODY)?)
        .map_err(|error| format!("invalid create-agent fixture: {error}"))?;
    let mut normalizer = Normalizer {
        home_port: String::new(),
        item_ids: BTreeMap::new(),
    };
    let mut queries = BTreeSet::new();
    let mut creates = BTreeSet::new();
    for (index, record) in home.iter().enumerate() {
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let request = Message::parse(&record.bytes)?;
        if request.probe {
            continue;
        }
        let kind = match request.method.as_str() {
            "QUERYACCESS" if request.path.starts_with("/agent/") => "query-access",
            "POST" if request.path.starts_with("/agent/") => "create-agent",
            _ => continue,
        };
        let agent = agent_on_path(&request.path)?;
        let reply = paired(&home, index)?;
        let mut response: Value = serde_json::from_str(&reply.text()?)
            .map_err(|_| format!("{kind} response is not JSON"))?;
        if response.get("success") != Some(&Value::Bool(true)) {
            continue;
        }
        normalizer
            .json(&mut response, "")
            .map_err(|_| format!("{kind} response could not be normalized"))?;
        let fixture = if kind == "query-access" {
            &query_fixture
        } else {
            &create_fixture
        };
        if &response != fixture {
            return Err(format!("{kind} response differs from the C# fixture"));
        }
        if kind == "query-access" {
            queries.insert(agent);
        } else {
            creates.insert(agent);
        }
    }
    expect_both_accounts(&queries, "QueryAccess")?;
    expect_both_accounts(&creates, "CreateAgent")?;
    Ok(
        "two roots: QueryAccess and CreateAgent for both harness accounts match the C# fixtures"
            .to_string(),
    )
}

fn agent_on_path(path: &str) -> Result<String, String> {
    let mut parts = path.trim_matches('/').split('/');
    if parts.next() != Some("agent") {
        return Err("agent path is missing".to_string());
    }
    let text = parts.next().unwrap_or("");
    let id = uuid::Uuid::parse_str(text).map_err(|_| "agent path is not a uuid".to_string())?;
    Ok(id.to_string())
}

fn expect_both_accounts(ids: &BTreeSet<String>, exchange: &str) -> Result<(), String> {
    let mut expected = BTreeSet::new();
    expected.insert(crate::live::ESTATE_OWNER.to_string());
    expected.insert(crate::live::CHAT_USER.to_string());
    if ids == &expected {
        return Ok(());
    }
    Err(format!(
        "{exchange} admitted {} account(s); both harness accounts are required",
        ids.len()
    ))
}

/// Q10f service exchanges selected from the real C# login/logout and probes.
pub(super) fn service_exchanges(directory: &Path) -> Result<Value, String> {
    service_exchanges_selected(directory, false)
}

pub(super) fn root_arrival_exchanges(directory: &Path) -> Result<Value, String> {
    service_exchanges_selected(directory, true)
}

pub(super) fn root_arrival_marker(directory: &Path) -> Result<Value, String> {
    use ferroan_protocol::{Msg, msgs::AgentMovementComplete, parse_packet};
    const CHANNEL: &[u8] =
        b"OpenSim-NGC Tranquillity Release 1.2.192-alpha+f67db54399 (X64/Unix/DotNet)\0";
    for record in records(&directory.join("capture-udp.cap"))? {
        if record.kind != Kind::UdpOut {
            continue;
        }
        let packet =
            parse_packet(&record.bytes).map_err(|_| "invalid root-arrival capture datagram")?;
        if packet.id != AgentMovementComplete::ID {
            continue;
        }
        let arrival = packet
            .decode::<AgentMovementComplete>()
            .map_err(|_| "invalid captured root arrival")?;
        if arrival.sim_data.channel_version != CHANNEL {
            return Err("root-arrival capture lacks the pinned C# channel/version marker".into());
        }
        return Ok(
            json!({"channel_version":String::from_utf8_lossy(CHANNEL).trim_end_matches('\0'), "arrival_position":arrival.data.position.to_array(), "arrival_look_at":arrival.data.look_at.to_array(), "arrival_handle":arrival.data.region_handle}),
        );
    }
    Err("root-arrival capture has no outgoing arrival".into())
}

pub(super) fn compare_root_arrival(directory: &Path) -> Result<String, String> {
    let mut actual = root_arrival_exchanges(directory)?;
    let baseline: Value = serde_json::from_slice(&bounded(
        &crate::repo_root().join("fixtures/root-arrival/exchanges.json"),
        MAX_BODY,
    )?)
    .map_err(|error| error.to_string())?;
    let mut expected = baseline["services"].clone();
    for name in ["presence-report", "griduser-setposition"] {
        actual[name]
            .as_object_mut()
            .ok_or("missing root-arrival actual exchange")?
            .remove("record_index");
        expected[name]
            .as_object_mut()
            .ok_or("missing root-arrival baseline exchange")?
            .remove("record_index");
        if name == "griduser-setposition" {
            let mut fields = parse_query_string(
                actual[name]["request"]
                    .as_str()
                    .ok_or("root-arrival form is not text")?,
            )
            .map_err(|error| error.to_string())?;
            let expected_fields = parse_query_string(
                expected[name]["request"]
                    .as_str()
                    .ok_or("root-arrival fixture form is not text")?,
            )
            .map_err(|error| error.to_string())?;
            // The online observer checked this run's exact position against its
            // UDP arrival. Terrain-adjusted C# Z is pending P2.0e; retain every
            // other field and the exact field ordering in this wire comparison.
            let actual_position = super::login_probe::vector(&text_field(&fields, "Position")?)
                .ok_or("invalid root-arrival position")?;
            let expected_position =
                super::login_probe::vector(&text_field(&expected_fields, "Position")?)
                    .ok_or("invalid root-arrival fixture position")?;
            if actual_position[..2] != expected_position[..2] {
                return Err("root-arrival form XY differs from C#".into());
            }
            fields.insert(
                "Position".into(),
                expected_fields
                    .get("Position")
                    .ok_or("missing fixture position")?
                    .clone(),
            );
            actual[name]["request"] =
                json!(build_query_string(&fields).map_err(|error| error.to_string())?);
        }
        if actual[name] != expected[name] {
            return Err(format!(
                "root-arrival HTTP differs for {name}; inspect private grid captures"
            ));
        }
    }
    Ok(
        "root arrival http: Presence report and GridUser setposition match C# (terrain Z pending)"
            .into(),
    )
}

const HARNESS_CHAT: &str = "ferroan harness chat";
struct HarnessSay {
    reliable: bool,
    zerocoded: bool,
    name: String,
    source_id: uuid::Uuid,
    owner_id: uuid::Uuid,
    source_type: u8,
    chat_type: u8,
    audible: u8,
    position: [f32; 3],
    message: Vec<u8>,
}

fn harness_says(records: &[Record]) -> Result<Vec<HarnessSay>, String> {
    use ferroan_protocol::{Msg, PacketFlags, field_str, msgs::ChatFromSimulator, parse_packet};
    let mut says = Vec::new();
    for record in records {
        if record.kind != Kind::UdpOut {
            continue;
        }
        let Ok(packet) = parse_packet(&record.bytes) else {
            continue;
        };
        if packet.id != ChatFromSimulator::ID {
            continue;
        }
        let decoded = packet
            .decode::<ChatFromSimulator>()
            .map_err(|_| "captured ChatFromSimulator did not decode".to_string())?;
        if field_str(&decoded.chat_data.message) != HARNESS_CHAT {
            continue;
        }
        says.push(HarnessSay {
            reliable: packet.reliable(),
            zerocoded: packet.flags & PacketFlags::ZEROCODED != 0,
            name: field_str(&decoded.chat_data.from_name),
            source_id: decoded.chat_data.source_id,
            owner_id: decoded.chat_data.owner_id,
            source_type: decoded.chat_data.source_type,
            chat_type: decoded.chat_data.chat_type,
            audible: decoded.chat_data.audible,
            position: decoded.chat_data.position.to_array(),
            message: decoded.chat_data.message,
        });
    }
    Ok(says)
}

fn arrival_position(records: &[Record]) -> Result<[f32; 3], String> {
    use ferroan_protocol::{Msg, msgs::AgentMovementComplete, parse_packet};
    for record in records {
        if record.kind != Kind::UdpOut {
            continue;
        }
        let Ok(packet) = parse_packet(&record.bytes) else {
            continue;
        };
        if packet.id != AgentMovementComplete::ID {
            continue;
        }
        let arrival = packet
            .decode::<AgentMovementComplete>()
            .map_err(|_| "captured arrival did not decode".to_string())?;
        return Ok(arrival.data.position.to_array());
    }
    Err("chat capture has no outgoing arrival".into())
}

/// Channel 0 say from the Ferroan region against the committed C# session.
/// `fixtures/captures/session.cap` is a redacted `live run all`, so the name and
/// owner id in that file are placeholders and the position is after the walk.
/// The message bytes, source type, chat type, audible level and header flags
/// are the comparison. The live name, both ids and this run's arrival position
/// are checked separately.
pub(super) fn compare_local_chat(directory: &Path) -> Result<String, String> {
    let live = records(&directory.join("capture-udp.cap"))?;
    let fixture = records(&crate::repo_root().join("fixtures/captures/session.cap"))?;
    let csharp = harness_says(&fixture)?;
    let ferroan = harness_says(&live)?;
    let reference = csharp
        .first()
        .ok_or("C# session capture has no harness say")?;
    if ferroan.len() < 2 {
        return Err(format!(
            "Ferroan capture has {} harness say(s); the speaker and the listener each need one",
            ferroan.len()
        ));
    }
    let owner = uuid::Uuid::parse_str(crate::live::ESTATE_OWNER)
        .map_err(|_| "estate owner is not a uuid".to_string())?;
    for say in &ferroan {
        if !say.reliable || say.zerocoded {
            return Err("harness say is not reliable and unencoded".into());
        }
        if say.message != reference.message
            || say.source_type != reference.source_type
            || say.chat_type != reference.chat_type
            || say.audible != reference.audible
            || say.reliable != reference.reliable
            || say.zerocoded != reference.zerocoded
        {
            return Err("harness say fields differ from the C# session capture".into());
        }
        if say.name != "Test User" || say.source_id != owner || say.owner_id != owner {
            return Err(
                "harness say name, source or owner is not the speaking harness account".into(),
            );
        }
        if say.source_id != reference.source_id {
            return Err("harness say source id differs from the C# session capture".into());
        }
    }
    let arrival = arrival_position(&live)?;
    if ferroan.iter().any(|say| say.position != arrival) {
        return Err(
            "harness say position differs from this run's arrival; terrain Z was not wildcarded"
                .into(),
        );
    }
    Ok(format!(
        "chat bytes: {} ChatFromSimulator says match the C# session capture (agent, fully audible, reliable, not zero-coded); name Test User; source and owner are the speaker; position matches this run's arrival",
        ferroan.len()
    ))
}

/// `TeleportStart` then `TeleportLocal` against the committed C# session.
/// The fixture agent id is redacted. The live agent must be the harness owner.
/// The two packets do not have to be adjacent.
pub(super) fn compare_local_teleport(directory: &Path) -> Result<String, String> {
    let live = records(&directory.join("capture-udp.cap"))?;
    let fixture = records(&crate::repo_root().join("fixtures/captures/session.cap"))?;
    let csharp = teleport_pair(&fixture)?;
    let ferroan = teleport_pair(&live)?;
    expect_teleport_pair("C# session capture", &csharp)?;
    expect_teleport_pair("Ferroan capture", &ferroan)?;
    let owner = uuid::Uuid::parse_str(crate::live::ESTATE_OWNER)
        .map_err(|_| "estate owner is not a uuid".to_string())?;
    if ferroan.1.agent_id != owner {
        return Err("TeleportLocal agent is not the harness owner".into());
    }
    Ok(
        "teleport bytes: TeleportStart flags 16, then TeleportLocal location 2 at (40.0,200.0,30.0) look (1.0,0.0,0.0) flags 16, reliable, not zero-coded; the live agent is the harness owner\nknown differences: a different region gets TeleportFailed and no map block (P2.0i); no post-teleport terse; IsFlying is not tracked; Godlike is not set on this path"
            .to_string(),
    )
}

const TELEPORT_FLAGS: u32 = 16;
const TELEPORT_LOCATION: u32 = 2;
const TELEPORT_POSITION: [f32; 3] = [40.0, 200.0, 30.0];
const TELEPORT_LOOK: [f32; 3] = [1.0, 0.0, 0.0];

struct TeleportStartHit {
    index: usize,
    reliable: bool,
    zerocoded: bool,
    flags: u32,
}

struct TeleportLocalHit {
    index: usize,
    reliable: bool,
    zerocoded: bool,
    agent_id: uuid::Uuid,
    location_id: u32,
    position: [f32; 3],
    look_at: [f32; 3],
    flags: u32,
}

fn teleport_pair(records: &[Record]) -> Result<(TeleportStartHit, TeleportLocalHit), String> {
    use ferroan_protocol::msgs::{TeleportLocal, TeleportStart};
    use ferroan_protocol::{Msg, PacketFlags, parse_packet};
    let mut start = None;
    for (index, record) in records.iter().enumerate() {
        if record.kind != Kind::UdpOut {
            continue;
        }
        let Ok(packet) = parse_packet(&record.bytes) else {
            continue;
        };
        if start.is_none() && packet.id == TeleportStart::ID {
            let decoded = packet
                .decode::<TeleportStart>()
                .map_err(|_| "captured TeleportStart did not decode".to_string())?;
            start = Some(TeleportStartHit {
                index,
                reliable: packet.reliable(),
                zerocoded: packet.flags & PacketFlags::ZEROCODED != 0,
                flags: decoded.info.teleport_flags,
            });
        } else if start.is_some() && packet.id == TeleportLocal::ID {
            let decoded = packet
                .decode::<TeleportLocal>()
                .map_err(|_| "captured TeleportLocal did not decode".to_string())?;
            let Some(found) = start else {
                return Err("capture has no TeleportStart".into());
            };
            return Ok((
                found,
                TeleportLocalHit {
                    index,
                    reliable: packet.reliable(),
                    zerocoded: packet.flags & PacketFlags::ZEROCODED != 0,
                    agent_id: decoded.info.agent_id,
                    location_id: decoded.info.location_id,
                    position: decoded.info.position.to_array(),
                    look_at: decoded.info.look_at.to_array(),
                    flags: decoded.info.teleport_flags,
                },
            ));
        }
    }
    Err("capture has no TeleportStart followed by TeleportLocal".into())
}

fn expect_teleport_pair(
    label: &str,
    pair: &(TeleportStartHit, TeleportLocalHit),
) -> Result<(), String> {
    let (start, local) = pair;
    if !start.reliable || start.zerocoded || start.flags != TELEPORT_FLAGS {
        return Err(format!(
            "{label} TeleportStart is not reliable, unencoded, flags {TELEPORT_FLAGS}"
        ));
    }
    if local.index <= start.index
        || !local.reliable
        || local.zerocoded
        || local.location_id != TELEPORT_LOCATION
        || local.flags != TELEPORT_FLAGS
        || local.position != TELEPORT_POSITION
        || local.look_at != TELEPORT_LOOK
    {
        return Err(format!(
            "{label} TeleportLocal does not match location {TELEPORT_LOCATION} at (40.0,200.0,30.0)"
        ));
    }
    Ok(())
}

const LANDING: [f32; 3] = [128.0, 128.0, 30.0];
const LANDING_LOOK: [f32; 3] = [1.0, 0.0, 0.0];
const LANDING_POS: &str = "<128, 128, 30>";
const SERVICE_08: f64 = 0.8;
const SERVICE_03: f64 = 0.3;

struct NeighbourSide {
    label: &'static str,
    udp_file: &'static str,
    http_file: &'static str,
    region_id: &'static str,
    region_name: &'static str,
    x: u32,
    y: u32,
    /// `AgentMovementComplete` handle for this region.
    here: u64,
    /// `TeleportFinish` names the region being entered.
    entering: u64,
    movement_complete: usize,
    negotiated: usize,
    enable_simulator: usize,
    establish: usize,
    close_agent: usize,
}

/// Both directions of `cargo xtask live run neighbour grid` at pin `1d7abb8337`.
///
/// The same checks run on the committed redacted files and on a fresh capture.
/// Ports, capability paths and session ids are not compared.
pub(super) fn compare_neighbour_teleport(directory: &Path) -> Result<String, String> {
    let home = side_home();
    let neighbour = side_neighbour();
    let home_udp = check_udp_side(directory, &home)?;
    let neighbour_udp = check_udp_side(directory, &neighbour)?;
    let home_http = check_http_side(directory, &home)?;
    let neighbour_http = check_http_side(directory, &neighbour)?;
    Ok(format!(
        "teleport neighbour bytes: simulation service 0.8 both ways (TransferAgent_V2); QueryAccess viaTeleport at (128, 128, 30) x{}; CreateAgent child flags 16 x{}; UpdateAgent AgentData wait_for_root x{}; ReleaseAgent {}; CloseAgent {} on the neighbour; TeleportStart flags 16 x{}; event-queue TeleportFinish location 4 sim access 13 flags 16 size 256 x{}; AgentMovementComplete (128, 128, 30) look (1, 0, 0) x{}; UseCircuitCode {}; CrossedRegion {}; EnableSimulator {}; no UDP TeleportFinish",
        home_http.queries + neighbour_http.queries,
        home_http.creates + neighbour_http.creates,
        home_http.updates + neighbour_http.updates,
        home_http.releases + neighbour_http.releases,
        neighbour_http.closes,
        home_udp.starts + neighbour_udp.starts,
        home_http.finishes + neighbour_http.finishes,
        home_udp.landings + neighbour_udp.landings,
        home_udp.circuits + neighbour_udp.circuits,
        home_http.crossed + neighbour_http.crossed,
        home_http.enables + neighbour_http.enables,
    ))
}

fn side_home() -> NeighbourSide {
    NeighbourSide {
        label: "home",
        udp_file: "capture-udp.cap",
        http_file: "proxy-home.cap",
        region_id: crate::live::REGION_UUID,
        region_name: super::HOME_NAME,
        x: super::HOME_X,
        y: super::HOME_Y,
        here: region_handle(super::HOME_X, super::HOME_Y),
        entering: region_handle(super::NEIGHBOUR_X, super::NEIGHBOUR_Y),
        movement_complete: 2,
        negotiated: 2,
        enable_simulator: 0,
        establish: 0,
        close_agent: 0,
    }
}

fn side_neighbour() -> NeighbourSide {
    NeighbourSide {
        label: "neighbour",
        udp_file: "capture-udp-neighbour.cap",
        http_file: "proxy-neighbour.cap",
        region_id: super::NEIGHBOUR_UUID,
        region_name: super::NEIGHBOUR_NAME,
        x: super::NEIGHBOUR_X,
        y: super::NEIGHBOUR_Y,
        here: region_handle(super::NEIGHBOUR_X, super::NEIGHBOUR_Y),
        entering: region_handle(super::HOME_X, super::HOME_Y),
        movement_complete: 1,
        negotiated: 1,
        enable_simulator: 1,
        establish: 1,
        close_agent: 1,
    }
}

fn region_handle(x: u32, y: u32) -> u64 {
    (u64::from(x) << 40) | (u64::from(y) << 8)
}

struct UdpCounts {
    circuits: usize,
    starts: usize,
    landings: usize,
}

fn check_udp_side(directory: &Path, side: &NeighbourSide) -> Result<UdpCounts, String> {
    use ferroan_protocol::msgs::{
        AgentMovementComplete, TeleportFinish, TeleportStart, UseCircuitCode,
    };
    use ferroan_protocol::{Msg, PacketFlags, parse_packet};
    let captured = records(&directory.join(side.udp_file))?;
    let mut circuits = 0usize;
    let mut starts = 0usize;
    let mut udp_finish = 0usize;
    let mut landings = 0usize;
    let mut movement = 0usize;
    for record in &captured {
        let Ok(packet) = parse_packet(&record.bytes) else {
            continue;
        };
        if packet.resent() {
            continue;
        }
        let reliable = packet.reliable();
        let zerocoded = packet.flags & PacketFlags::ZEROCODED != 0;
        if record.kind == Kind::UdpIn && packet.id == UseCircuitCode::ID {
            packet
                .decode::<UseCircuitCode>()
                .map_err(|_| format!("{} UseCircuitCode did not decode", side.label))?;
            if !reliable || zerocoded {
                return Err(format!(
                    "{} UseCircuitCode is not reliable and unencoded",
                    side.label
                ));
            }
            circuits += 1;
        }
        if record.kind == Kind::UdpOut && packet.id == TeleportStart::ID {
            let decoded = packet
                .decode::<TeleportStart>()
                .map_err(|_| format!("{} TeleportStart did not decode", side.label))?;
            if !reliable || zerocoded || decoded.info.teleport_flags != TELEPORT_FLAGS {
                return Err(format!(
                    "{} TeleportStart is not reliable, unencoded, flags {TELEPORT_FLAGS}",
                    side.label
                ));
            }
            starts += 1;
        }
        if packet.id == TeleportFinish::ID {
            udp_finish += 1;
        }
        if record.kind == Kind::UdpOut && packet.id == AgentMovementComplete::ID {
            let decoded = packet
                .decode::<AgentMovementComplete>()
                .map_err(|_| format!("{} AgentMovementComplete did not decode", side.label))?;
            if !reliable || zerocoded || decoded.data.region_handle != side.here {
                return Err(format!(
                    "{} AgentMovementComplete is not reliable, unencoded, on this region",
                    side.label
                ));
            }
            if !known_agent(decoded.agent_data.agent_id) {
                return Err(format!(
                    "{} AgentMovementComplete agent is not the harness owner",
                    side.label
                ));
            }
            movement += 1;
            if near_vec(decoded.data.position.to_array(), LANDING)
                && near_vec(decoded.data.look_at.to_array(), LANDING_LOOK)
            {
                landings += 1;
            }
        }
    }
    if circuits != 1
        || starts != 1
        || udp_finish != 0
        || landings != 1
        || movement != side.movement_complete
    {
        return Err(format!(
            "{} viewer sequence: UseCircuitCode {circuits}, TeleportStart {starts}, UDP TeleportFinish {udp_finish}, landing {landings}, AgentMovementComplete {movement}",
            side.label
        ));
    }
    Ok(UdpCounts {
        circuits,
        starts,
        landings,
    })
}

struct HttpCounts {
    queries: usize,
    creates: usize,
    updates: usize,
    releases: usize,
    closes: usize,
    finishes: usize,
    crossed: usize,
    enables: usize,
}

fn check_http_side(directory: &Path, side: &NeighbourSide) -> Result<HttpCounts, String> {
    let captured = records(&directory.join(side.http_file))?;
    let mut queries = 0usize;
    let mut creates = 0usize;
    let mut updates = 0usize;
    let mut releases = 0usize;
    let mut closes = 0usize;
    let mut negotiated = 0usize;
    let mut trues = 0usize;
    let mut close_replies = 0usize;
    let meters_x = (side.x * 256).to_string();
    let meters_y = (side.y * 256).to_string();
    for record in &captured {
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let request = Message::parse(&record.bytes)?;
        let path = if let Some((path, _)) = request.path.split_once('?') {
            path
        } else {
            request.path.as_str()
        };
        if request.method == "QUERYACCESS" && path.starts_with("/agent/") {
            let body = json_body(&request, side.label)?;
            if body.get("position").and_then(Value::as_str) != Some(LANDING_POS) {
                continue;
            }
            if body.get("viaTeleport").and_then(Value::as_bool) != Some(true) {
                return Err(format!(
                    "{} QueryAccess viaTeleport is not true",
                    side.label
                ));
            }
            if !path.contains(side.region_id) {
                return Err(format!(
                    "{} QueryAccess path is not this region",
                    side.label
                ));
            }
            if body.get("my_version").and_then(Value::as_str) != Some("SIMULATION/0.3") {
                return Err(format!(
                    "{} QueryAccess my_version is not SIMULATION/0.3",
                    side.label
                ));
            }
            require_version(
                &body,
                "simulation_service_supported_min",
                SERVICE_03,
                side.label,
            )?;
            require_version(
                &body,
                "simulation_service_supported_max",
                SERVICE_08,
                side.label,
            )?;
            require_version(
                &body,
                "simulation_service_accepted_min",
                SERVICE_03,
                side.label,
            )?;
            require_version(
                &body,
                "simulation_service_accepted_max",
                SERVICE_08,
                side.label,
            )?;
            context_versions(&body, side.label)?;
            queries += 1;
        } else if request.method == "POST" && path.starts_with("/agent/") {
            let body = json_body(&request, side.label)?;
            if body.get("teleport_flags").and_then(Value::as_str) != Some("16")
                || body.get("start_pos").and_then(Value::as_str) != Some(LANDING_POS)
            {
                continue;
            }
            if body.get("child").and_then(Value::as_bool) != Some(true) {
                return Err(format!("{} CreateAgent child is not true", side.label));
            }
            require_destination(&body, side, &meters_x, &meters_y)?;
            context_versions(&body, side.label)?;
            creates += 1;
        } else if request.method == "PUT" && path.starts_with("/agent/") {
            let body = json_body(&request, side.label)?;
            if body.get("message_type").and_then(Value::as_str) != Some("AgentData") {
                continue;
            }
            if body.get("wait_for_root").and_then(Value::as_bool) != Some(true) {
                return Err(format!(
                    "{} UpdateAgent wait_for_root is not true",
                    side.label
                ));
            }
            if body.get("position").and_then(Value::as_str) != Some(LANDING_POS) {
                return Err(format!(
                    "{} UpdateAgent position is not {LANDING_POS}",
                    side.label
                ));
            }
            require_destination(&body, side, &meters_x, &meters_y)?;
            context_versions(&body, side.label)?;
            updates += 1;
        } else if request.method == "DELETE" && path.starts_with("/agent/") {
            if path.contains("release") {
                releases += 1;
            } else if path.contains(side.region_id) {
                closes += 1;
            } else {
                return Err(format!("{} CloseAgent path is not this region", side.label));
            }
        }
    }
    for record in &captured {
        if record.kind != Kind::HttpResponse {
            continue;
        }
        let reply = Message::parse(&record.bytes)?;
        let Ok(text) = reply.text() else {
            continue;
        };
        let trimmed = text.trim();
        if trimmed == "True" {
            trues += 1;
        }
        if let Some(rest) = trimmed.strip_prefix("OpenSim agent ") {
            let id = uuid::Uuid::parse_str(rest.trim())
                .map_err(|_| format!("{} CloseAgent reply is not an agent id", side.label))?;
            if !known_agent(id) {
                return Err(format!(
                    "{} CloseAgent reply agent is not the harness owner",
                    side.label
                ));
            }
            close_replies += 1;
        }
        if let Some(value) = trimmed.strip_prefix('{').map(|_| trimmed) {
            let Ok(body) = serde_json::from_str::<Value>(value) else {
                continue;
            };
            if body.get("negotiated_outbound_version").is_some() {
                if body.get("success") != Some(&Value::Bool(true))
                    || body.get("version").and_then(Value::as_str) != Some("SIMULATION/0")
                {
                    return Err(format!(
                        "{} QueryAccess response is not success at legacy SIMULATION/0",
                        side.label
                    ));
                }
                require_version(&body, "negotiated_inbound_version", SERVICE_08, side.label)?;
                require_version(&body, "negotiated_outbound_version", SERVICE_08, side.label)?;
                negotiated += 1;
            }
        }
    }
    let events = check_events(&captured, side)?;
    if queries != 1
        || creates != 1
        || updates != 1
        || releases != 0
        || closes != side.close_agent
        || close_replies != side.close_agent
        || negotiated != side.negotiated
        || trues == 0
    {
        return Err(format!(
            "{} HTTP: QueryAccess {queries}, CreateAgent {creates}, UpdateAgent {updates}, ReleaseAgent {releases}, CloseAgent {closes} replies {close_replies}, negotiated {negotiated}, True {trues}",
            side.label
        ));
    }
    Ok(HttpCounts {
        queries,
        creates,
        updates,
        releases,
        closes,
        finishes: events.finishes,
        crossed: events.crossed,
        enables: events.enables,
    })
}

struct EventCounts {
    finishes: usize,
    crossed: usize,
    enables: usize,
}

fn check_events(records: &[Record], side: &NeighbourSide) -> Result<EventCounts, String> {
    let mut finishes = 0usize;
    let mut crossed = 0usize;
    let mut enables = 0usize;
    let mut establish = 0usize;
    for record in records {
        if record.kind != Kind::HttpResponse {
            continue;
        }
        let reply = Message::parse(&record.bytes)?;
        if !reply.content_type.contains("xml") && !reply.content_type.contains("llsd") {
            continue;
        }
        let Ok(text) = reply.text() else {
            continue;
        };
        if !text.contains("<llsd") && !text.contains("<LLSD") {
            continue;
        }
        let value = ferroan_llsd::from_xml(text.as_bytes())
            .map_err(|error| format!("{} event queue: {error}", side.label))?;
        if !value.has("events") {
            continue;
        }
        for event in value.get("events").as_array() {
            match event.get("message").as_str() {
                "CrossedRegion" => crossed += 1,
                "EnableSimulator" => enables += 1,
                "EstablishAgentCommunication" => establish += 1,
                "TeleportFinish" => {
                    check_teleport_finish(event, side)?;
                    finishes += 1;
                }
                _ => {}
            }
        }
    }
    if finishes != 1
        || crossed != 0
        || enables != side.enable_simulator
        || establish != side.establish
    {
        return Err(format!(
            "{} events: TeleportFinish {finishes}, CrossedRegion {crossed}, EnableSimulator {enables}, EstablishAgentCommunication {establish}",
            side.label
        ));
    }
    Ok(EventCounts {
        finishes,
        crossed,
        enables,
    })
}

fn check_teleport_finish(event: &ferroan_llsd::Llsd, side: &NeighbourSide) -> Result<(), String> {
    let info = event.get("body").get("Info").at(0);
    // EventQueueGetHandlers.TeleportFinishEvent stores LocationID 4.
    // The caller passes 0. The viewer sees 4.
    if info.get("LocationID").as_u32() != 4
        || info.get("SimAccess").as_i32() != 13
        || info.get("TeleportFlags").as_u32() != TELEPORT_FLAGS
        || info.get("RegionSizeX").as_u32() != 256
        || info.get("RegionSizeY").as_u32() != 256
        || info.get("RegionHandle").as_u64() != side.entering
    {
        return Err(format!(
            "{} TeleportFinish fields are not location 4, access 13, flags 16, size 256, destination handle",
            side.label
        ));
    }
    let port = info.get("SimPort").as_i32();
    let seed = info.get("SeedCapability").as_str();
    if !(1..=65535).contains(&port) || info.get("SimIP").as_binary() != [127, 0, 0, 1] {
        return Err(format!(
            "{} TeleportFinish endpoint is not loopback with a port",
            side.label
        ));
    }
    if !seed.starts_with("http://") || !seed.contains("/CAPS/") {
        return Err(format!("{} TeleportFinish has no caps seed", side.label));
    }
    if !known_agent(info.get("AgentID").as_uuid()) {
        return Err(format!(
            "{} TeleportFinish agent is not the harness owner",
            side.label
        ));
    }
    Ok(())
}

fn json_body(message: &Message, label: &str) -> Result<Value, String> {
    if !message.content_type.contains("json") {
        return Err(format!("{label} agent body is not JSON"));
    }
    serde_json::from_str(&message.text()?).map_err(|_| format!("{label} agent body is not JSON"))
}

fn require_version(body: &Value, key: &str, expected: f64, label: &str) -> Result<(), String> {
    let Some(got) = body.get(key).and_then(Value::as_f64) else {
        return Err(format!("{label} is missing {key}"));
    };
    if (got - expected).abs() < 1.0e-5 {
        Ok(())
    } else {
        Err(format!("{label} {key} is not {expected}"))
    }
}

fn context_versions(body: &Value, label: &str) -> Result<(), String> {
    let Some(context) = body.get("context") else {
        return Err(format!("{label} is missing context"));
    };
    require_version(context, "InboundVersion", SERVICE_08, label)?;
    require_version(context, "OutboundVersion", SERVICE_08, label)
}

fn require_destination(
    body: &Value,
    side: &NeighbourSide,
    meters_x: &str,
    meters_y: &str,
) -> Result<(), String> {
    if body.get("destination_uuid").and_then(Value::as_str) != Some(side.region_id)
        || body.get("destination_name").and_then(Value::as_str) != Some(side.region_name)
        || body.get("destination_x").and_then(Value::as_str) != Some(meters_x)
        || body.get("destination_y").and_then(Value::as_str) != Some(meters_y)
    {
        return Err(format!(
            "{} destination is not {} at {},{}",
            side.label, side.region_name, side.x, side.y
        ));
    }
    Ok(())
}

fn known_agent(id: uuid::Uuid) -> bool {
    id == uuid::Uuid::from_u128(1) || id.to_string() == crate::live::ESTATE_OWNER
}

fn near_vec(got: [f32; 3], expected: [f32; 3]) -> bool {
    got.iter()
        .zip(expected)
        .all(|(got, expected)| (got - expected).abs() < 0.001)
}

/// Presence `logoutregion` at Ferroan region start and stop.
pub(super) fn compare_logout_region(directory: &Path) -> Result<String, String> {
    let fixture: Value = serde_json::from_slice(&bounded(
        &crate::repo_root().join("fixtures/pin3/login-robustness.json"),
        MAX_BODY,
    )?)
    .map_err(|error| error.to_string())?;
    let fixture_region = fixture["logout_region"]["region"]
        .as_str()
        .ok_or("logoutregion fixture has no region")?;
    let expected = fixture["logout_region"]["calls"][0]["request"]
        .as_str()
        .ok_or("logoutregion fixture has no request")?
        .replace(fixture_region, crate::live::REGION_UUID);
    let records = records(&directory.join("proxy-robust-private.cap"))?;
    let mut found = 0usize;
    for record in &records {
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let request = Message::parse(&record.bytes)?;
        if request.method != "POST" || request.path != "/presence" {
            continue;
        }
        let fields = parse_query_string(&request.text()?).map_err(|error| error.to_string())?;
        if text_field(&fields, "METHOD").unwrap_or_default() != "logoutregion" {
            continue;
        }
        if text_field(&fields, "RegionID")? != crate::live::REGION_UUID {
            continue;
        }
        if fields.contains_key("SessionID") {
            return Err("logoutregion capture includes SessionID".into());
        }
        let body = request.text()?;
        if body != expected {
            return Err("logoutregion form differs from the C# fixture".into());
        }
        found += 1;
    }
    if found != 2 {
        return Err(format!(
            "expected two Presence logoutregion calls for the Ferroan region, found {found}"
        ));
    }
    Ok("presence logoutregion: start and stop match the C# form".into())
}

fn service_exchanges_selected(directory: &Path, root_arrival: bool) -> Result<Value, String> {
    let records = records(&directory.join("proxy-robust-private.cap"))?;
    let normalizer = Normalizer {
        home_port: String::new(),
        item_ids: BTreeMap::new(),
    };
    let mut output = Map::new();
    for (index, record) in records.iter().enumerate() {
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let request = Message::parse(&record.bytes)?;
        if request.probe || !matches!(request.path.as_str(), "/presence" | "/griduser") {
            continue;
        }
        let mut fields = parse_query_string(&request.text()?).map_err(|error| error.to_string())?;
        let method = text_field(&fields, "METHOD")?;
        if !matches!(
            method.as_str(),
            "getagent" | "logout" | "loggedout" | "report" | "setposition"
        ) {
            continue;
        }
        // Never infer pairing through concurrent requests. Later unambiguous
        // exchanges may provide the same case; all four cases are required.
        let Ok(reply) = paired(&records, index) else {
            continue;
        };
        let response = reply.text()?;
        let decoded =
            ferroan_http::parse_xml_response(&response).map_err(|error| error.to_string())?;
        let name = match (
            request.path.as_str(),
            method.as_str(),
            decoded.get("result"),
        ) {
            ("/presence", "getagent", Some(Field::Map(info))) if matches!(info.get("UserID"), Some(Field::Str(user)) if user == crate::live::ESTATE_OWNER) => {
                "presence-get"
            }
            ("/presence", "getagent", Some(Field::Str(value))) if value == "null" => {
                "presence-missing"
            }
            ("/presence", "logout", _) => "presence-logout",
            ("/griduser", "loggedout", _)
                if text_field(&fields, "UserID")? == crate::live::ESTATE_OWNER =>
            {
                "griduser-loggedout"
            }
            ("/presence", "report", _)
                if text_field(&fields, "RegionID")? == crate::live::REGION_UUID =>
            {
                "presence-report"
            }
            ("/griduser", "setposition", _)
                if text_field(&fields, "UserID")? == crate::live::ESTATE_OWNER
                    && text_field(&fields, "RegionID")? == crate::live::REGION_UUID =>
            {
                "griduser-setposition"
            }
            _ => continue,
        };
        if root_arrival != matches!(name, "presence-report" | "griduser-setposition") {
            continue;
        }
        if output.contains_key(name) {
            continue;
        }
        for (key, field) in &mut fields {
            let Field::Str(value) = field else {
                return Err("service capture has a non-string field".into());
            };
            *value = if key == "SessionID" {
                non_nil_uuid(value, SESSION)?
            } else {
                normalizer.text(value)
            };
        }
        let mut captured = exchange(
            &request,
            &reply,
            Value::String(build_query_string(&fields).map_err(|error| error.to_string())?),
            Value::String(normalizer.text(&response)),
            &normalizer,
        );
        if root_arrival {
            captured["record_index"] = json!(index);
        }
        output.insert(name.into(), captured);
    }
    if !root_arrival && directory.join("proxy-presence-missing.cap").exists() {
        let isolated = self::records(&directory.join("proxy-presence-missing.cap"))?;
        if isolated.len() != 2 || isolated[0].kind != Kind::HttpRequest {
            return Err("isolated Presence capture must contain exactly one exchange".into());
        }
        let request = Message::parse(&isolated[0].bytes)?;
        let reply = paired(&isolated, 0)?;
        let fields = parse_query_string(&request.text()?).map_err(|error| error.to_string())?;
        let response = reply.text()?;
        let decoded =
            ferroan_http::parse_xml_response(&response).map_err(|error| error.to_string())?;
        if request.method != "POST"
            || request.path != "/presence"
            || text_field(&fields, "METHOD")? != "getagent"
            || text_field(&fields, "SessionID")? != SESSION
            || !matches!(decoded.get("result"), Some(Field::Str(value)) if value == "null")
        {
            return Err("isolated capture is not the missing test-session Presence query".into());
        }
        let mut captured = exchange(
            &request,
            &reply,
            json!(build_query_string(&fields).map_err(|error| error.to_string())?),
            json!(normalizer.text(&response)),
            &normalizer,
        );
        captured["capture_source"] = json!(
            "Explicit missing-session query through an isolated one-exchange proxy to the same live C# Robust backend; not inferred from overlapping CreateAgent inventory traffic."
        );
        output.insert("presence-missing".into(), captured);
    }
    let required: &[&str] = if root_arrival {
        &["presence-report", "griduser-setposition"]
    } else {
        &[
            "presence-get",
            "presence-missing",
            "presence-logout",
            "griduser-loggedout",
        ]
    };
    for name in required {
        if !output.contains_key(*name) {
            return Err(format!("C# capture has no unambiguous {name} exchange"));
        }
    }
    Ok(Value::Object(output))
}

fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds its size limit", path.display()));
    }
    Ok(bytes)
}

pub(super) fn records(path: &Path) -> Result<Vec<Record>, String> {
    read_complete_prefix(&bounded(path, MAX_CAPTURE)?).map_err(|error| error.to_string())
}

fn extract(
    directory: &Path,
    allow_neighbour_deregister: bool,
    circuit: Option<(u32, uuid::Uuid)>,
) -> Result<Value, String> {
    let private = records(&directory.join("proxy-robust-private.cap"))?;
    let home = records(&directory.join("proxy-home.cap"))?;
    let mut output = Map::new();
    let mut mismatched_circuit = false;
    let mut normalizer = Normalizer {
        home_port: String::new(),
        item_ids: BTreeMap::new(),
    };
    for (index, record) in private.iter().enumerate() {
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let request = Message::parse(&record.bytes)?;
        if request.probe || request.path != "/grid" {
            continue;
        }
        let mut fields = parse_query_string(&request.text()?).map_err(|error| error.to_string())?;
        let method = text_field(&fields, "METHOD")?;
        if method != "register" && method != "deregister" {
            continue;
        }
        let id_key = if method == "register" {
            "uuid"
        } else {
            "REGIONID"
        };
        let region_id = text_field(&fields, id_key)?;
        if region_id != crate::live::REGION_UUID
            && !(allow_neighbour_deregister
                && method == "deregister"
                && region_id == super::NEIGHBOUR_UUID)
        {
            continue;
        }
        if output.contains_key(&method) {
            continue;
        }
        if method == "register" {
            normalizer.home_port = text_field(&fields, "serverHttpPort")?;
            for key in ["regionMapTexture", "parcelMapTexture"] {
                if text_field(&fields, key)? != uuid::Uuid::nil().to_string() {
                    return Err("login fixtures require [Map] GenerateMaptiles=false with a nil static map UUID".to_string());
                }
            }
        }
        let reply = match paired(&private, index) {
            Ok(reply) => reply,
            // Shutdown may overlap another HTTP request. A neighbouring test
            // region has the same deregister schema and only a different UUID;
            // select its unambiguous pair instead of guessing correlations.
            Err(_) if allow_neighbour_deregister && method == "deregister" => continue,
            Err(error) => return Err(error),
        };
        normalizer.form(&mut fields)?;
        output.insert(
            method,
            exchange(
                &request,
                &reply,
                Value::String(build_query_string(&fields).map_err(|error| error.to_string())?),
                Value::String(reply.text()?),
                &normalizer,
            ),
        );
    }
    if normalizer.home_port.is_empty() {
        return Err("capture has no home-region register exchange".to_string());
    }
    for (index, record) in home.iter().enumerate() {
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let request = Message::parse(&record.bytes)?;
        if request.probe {
            continue;
        }
        let name = match request.method.as_str() {
            "QUERYACCESS" if request.path.starts_with("/agent/") => "query-access",
            "POST" if request.path.starts_with("/agent/") => "create-agent",
            _ => continue,
        };
        if output.contains_key(name) {
            continue;
        }
        let reply = paired(&home, index)?;
        let mut body: Value = serde_json::from_str(&request.text()?)
            .map_err(|error| format!("invalid {name} JSON: {error}"))?;
        let mut response: Value = serde_json::from_str(&reply.text()?)
            .map_err(|error| format!("invalid {name} response JSON: {error}"))?;
        validate_json(name, &body, &response)?;
        if name == "create-agent"
            && let Some((code, session)) = circuit
        {
            let wire = crate::caps_fixtures::json_circuit(body.get("circuit_code"))?;
            let wire_session = body.get("session_id").and_then(Value::as_str).unwrap_or("");
            if wire != code || wire_session != session.to_string() {
                mismatched_circuit = true;
                continue;
            }
        }
        normalizer.json(&mut body, "")?;
        normalizer.json(&mut response, "")?;
        output.insert(
            name.to_string(),
            exchange(&request, &reply, body, response, &normalizer),
        );
    }
    if circuit.is_some() && mismatched_circuit && !output.contains_key("create-agent") {
        return Err("create-agent does not match the C# login circuit".to_string());
    }
    for name in ["register", "deregister", "query-access", "create-agent"] {
        if !output.contains_key(name) {
            return Err(format!("capture has no {name} exchange"));
        }
    }
    Ok(Value::Object(output))
}

fn validate_json(name: &str, body: &Value, response: &Value) -> Result<(), String> {
    let required = if name == "query-access" {
        &[
            "viaTeleport",
            "position",
            "my_version",
            "context",
            "features",
        ][..]
    } else {
        &[
            "agent_id",
            "caps_path",
            "circuit_code",
            "session_id",
            "secure_session_id",
            "packed_appearance",
            "destination_uuid",
        ][..]
    };
    if !body.is_object() || required.iter().any(|key| body.get(*key).is_none()) {
        return Err(format!(
            "captured {name} is missing required request fields"
        ));
    }
    if response.get("success") != Some(&Value::Bool(true)) || !response["reason"].is_string() {
        return Err(format!("captured {name} is not a successful response"));
    }
    Ok(())
}

fn paired(records: &[Record], index: usize) -> Result<Message, String> {
    let mut outstanding = 0usize;
    for record in &records[..index] {
        match record.kind {
            Kind::HttpRequest => outstanding += 1,
            Kind::HttpResponse => {
                outstanding = outstanding
                    .checked_sub(1)
                    .ok_or("HTTP response has no preceding request")?
            }
            _ => {}
        }
    }
    if outstanding != 0 {
        return Err(
            "selected control-plane request overlaps another request; recapture an isolated login"
                .to_string(),
        );
    }
    let response = records
        .get(index + 1)
        .filter(|record| record.kind == Kind::HttpResponse)
        .ok_or("selected control-plane request has no unambiguous adjacent response")?;
    Message::parse(&response.bytes)
}

struct Message {
    method: String,
    path: String,
    status: Option<u16>,
    content_type: String,
    encoding: String,
    x_encoding: String,
    probe: bool,
    body: Vec<u8>,
}

impl Message {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let split = bytes
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .ok_or("HTTP capture has no header terminator")?;
        let header =
            std::str::from_utf8(&bytes[..split]).map_err(|_| "HTTP header is not UTF-8")?;
        let mut lines = header.lines();
        let start: Vec<_> = lines
            .next()
            .ok_or("HTTP capture has no start line")?
            .split_whitespace()
            .collect();
        if start.len() < 3 {
            return Err("HTTP start line is incomplete".to_string());
        }
        let response = start[0].starts_with("HTTP/");
        let mut message = Self {
            method: if response {
                String::new()
            } else {
                start[0].to_string()
            },
            path: if response {
                String::new()
            } else {
                start[1].to_string()
            },
            status: if response {
                Some(start[1].parse().map_err(|_| "HTTP status is invalid")?)
            } else {
                None
            },
            content_type: String::new(),
            encoding: String::new(),
            x_encoding: String::new(),
            probe: false,
            body: bytes[split + 4..].to_vec(),
        };
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-type") {
                    message.content_type = value.trim().to_ascii_lowercase();
                }
                if key.eq_ignore_ascii_case("content-encoding") {
                    message.encoding = value.trim().to_ascii_lowercase();
                }
                if key.eq_ignore_ascii_case("x-content-encoding") {
                    message.x_encoding = value.trim().to_ascii_lowercase();
                }
                if key.eq_ignore_ascii_case("x-ferroan-probe") && !value.trim().is_empty() {
                    message.probe = true;
                }
            }
        }
        Ok(message)
    }

    fn text(&self) -> Result<String, String> {
        if self.body.len() as u64 > MAX_BODY {
            return Err("encoded control-plane body exceeds size limit".to_string());
        }
        let bytes = if self.encoding == "gzip"
            || self.x_encoding == "gzip"
            || self.content_type == "application/x-gzip"
        {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(self.body.as_slice())
                .take(MAX_BODY + 1)
                .read_to_end(&mut out)
                .map_err(|_| "captured gzip body is invalid")?;
            out
        } else if self.encoding.is_empty() && self.x_encoding.is_empty() {
            self.body.clone()
        } else {
            return Err("unsupported captured content encoding".to_string());
        };
        if bytes.len() as u64 > MAX_BODY {
            return Err("decoded control-plane body exceeds size limit".to_string());
        }
        String::from_utf8(bytes).map_err(|_| "control-plane body is not UTF-8".to_string())
    }
}

fn exchange(
    request: &Message,
    reply: &Message,
    body: Value,
    response: Value,
    normalizer: &Normalizer,
) -> Value {
    json!({
        "method": request.method, "path": normalizer.text(&request.path),
        "request_content_type": request.content_type, "request_content_encoding": request.encoding,
        "request_x_content_encoding": request.x_encoding,
        "request": body, "status": reply.status, "response_content_type": reply.content_type,
        "response_content_encoding": reply.encoding,
        "response_x_content_encoding": reply.x_encoding,
        "response": response,
    })
}

fn text_field(fields: &FieldMap, key: &str) -> Result<String, String> {
    match fields.get(key) {
        Some(Field::Str(value)) => Ok(value.clone()),
        _ => Err(format!("captured form has no string {key}")),
    }
}

struct Normalizer {
    home_port: String,
    item_ids: BTreeMap<String, String>,
}

impl Normalizer {
    fn form(&self, fields: &mut FieldMap) -> Result<(), String> {
        for (key, value) in fields {
            let Field::Str(text) = value else {
                return Err("grid form contains a non-string value".to_string());
            };
            *text = match key.as_str() {
                "uuid" | "REGIONID" => REGION.to_string(),
                "owner_uuid" if text == crate::live::ESTATE_OWNER => AGENT.to_string(),
                "regionSecret" => non_nil_uuid(text, SECRET)?,
                "regionName" if text == super::HOME_NAME => "Fixture Region".to_string(),
                "serverHttpPort" | "serverPort" if text == &self.home_port => "9000".to_string(),
                _ => self.text(text),
            };
        }
        Ok(())
    }

    fn text(&self, text: &str) -> String {
        let out = text
            .replace(crate::live::ESTATE_OWNER, AGENT)
            .replace(crate::live::REGION_UUID, REGION)
            .replace("127.0.0.1", "192.0.2.1")
            .replace("127.0.0.2", "192.0.2.2")
            .replace("127.0.0.4", "192.0.2.4");
        if self.home_port.is_empty() {
            out
        } else {
            out.replace(&format!(":{}", self.home_port), ":9000")
        }
    }

    fn json(&mut self, value: &mut Value, key: &str) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    self.json(value, key)?;
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.json(value, key)?;
                }
            }
            Value::String(text) => {
                *text = match key {
                    "session_id" => non_nil_uuid(text, SESSION)?,
                    "secure_session_id" => non_nil_uuid(text, SECURE_SESSION)?,
                    "caps_path" => non_nil_uuid(text, CAPS)?,
                    "item" => {
                        non_nil_uuid(text, text)?;
                        let next =
                            format!("00000000-0000-4000-8000-{:012x}", 16 + self.item_ids.len());
                        self.item_ids.entry(text.clone()).or_insert(next).clone()
                    }
                    "circuit_code" => {
                        text.parse::<u32>()
                            .map_err(|_| "circuit_code is not a u32")?;
                        "1".to_string()
                    }
                    "first_name" | "firstname" if text == "Test" => "Fixture".to_string(),
                    "last_name" | "lastname" if text == "User" => "Resident".to_string(),
                    "destination_name" if text == super::HOME_NAME => "Fixture Region".to_string(),
                    _ => self.text(text),
                };
            }
            _ => {}
        }
        Ok(())
    }
}

fn non_nil_uuid(text: &str, placeholder: &str) -> Result<String, String> {
    let id = uuid::Uuid::parse_str(text).map_err(|_| "captured identity is not a UUID")?;
    if id.is_nil() {
        return Err("captured generated identity is nil".to_string());
    }
    Ok(placeholder.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn normalizer() -> Normalizer {
        Normalizer {
            home_port: "12345".to_string(),
            item_ids: BTreeMap::new(),
        }
    }

    #[test]
    fn generated_ids_normalize_without_hiding_semantic_fields() {
        let mut input = json!({
            "session_id": "11223344-5566-7788-99aa-bbccddeeff00",
            "item": "11223344-5566-7788-99aa-bbccddeeff01",
            "asset": "11223344-5566-7788-99aa-bbccddeeff02",
            "destination_x": "256256", "your_ip": "127.0.0.2",
            "teleport_flags": "4224", "reason": "different reason"
        });
        normalizer().json(&mut input, "").unwrap();
        assert_eq!(input["session_id"], SESSION);
        assert_eq!(input["item"], "00000000-0000-4000-8000-000000000010");
        assert_eq!(input["asset"], "11223344-5566-7788-99aa-bbccddeeff02");
        assert_eq!(input["destination_x"], "256256");
        assert_eq!(input["your_ip"], "192.0.2.2");
        assert_eq!(input["reason"], "different reason");
        let mut invalid = json!({"session_id": uuid::Uuid::nil().to_string()});
        assert!(normalizer().json(&mut invalid, "").is_err());
        let mut invalid = json!({"caps_path": "not-a-uuid"});
        assert!(normalizer().json(&mut invalid, "").is_err());
    }

    #[test]
    fn the_committed_session_say_matches_channel_zero_chat() {
        let records = records(&crate::repo_root().join("fixtures/captures/session.cap")).unwrap();
        let says = harness_says(&records).unwrap();
        assert!(says.len() >= 2);
        let owner = uuid::Uuid::parse_str(crate::live::ESTATE_OWNER).unwrap();
        for say in &says {
            assert!(say.reliable);
            assert!(!say.zerocoded);
            assert_eq!(say.source_type, 1);
            assert_eq!(say.chat_type, 1);
            assert_eq!(say.audible, 1);
            assert_eq!(say.message, b"ferroan harness chat\0");
            assert_eq!(say.source_id, owner);
        }
    }

    #[test]
    fn the_committed_session_has_the_local_teleport() {
        let records = records(&crate::repo_root().join("fixtures/captures/session.cap")).unwrap();
        let pair = teleport_pair(&records).unwrap();
        expect_teleport_pair("C# session capture", &pair).unwrap();
        // The capture redacts the agent to the protocol placeholder, not the harness owner.
        assert_eq!(pair.1.agent_id, uuid::Uuid::from_u128(1));
    }

    #[test]
    fn overlapping_http_pairs_are_rejected() {
        let record = |kind, bytes: &[u8]| Record {
            kind,
            time_ms: 0,
            bytes: bytes.to_vec(),
        };
        let request = record(Kind::HttpRequest, b"POST /grid HTTP/1.1\r\n\r\n");
        let response = record(Kind::HttpResponse, b"HTTP/1.1 200 OK\r\n\r\n");
        let records = [request.clone(), request, response.clone(), response];
        assert!(paired(&records, 0).is_err());
        assert!(paired(&records, 1).is_err());
        assert!(paired(&records[1..3], 0).is_ok());
    }

    #[test]
    fn csharp_gzip_header_decodes_with_a_bound() {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(b"{\"success\":true}").unwrap();
        let mut bytes = b"POST /agent/ HTTP/1.1\r\nContent-Type: application/json\r\nX-Content-Encoding: gzip\r\n\r\n".to_vec();
        bytes.extend(gzip.finish().unwrap());
        assert_eq!(
            Message::parse(&bytes).unwrap().text().unwrap(),
            "{\"success\":true}"
        );
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(&vec![b' '; MAX_BODY as usize + 1]).unwrap();
        let mut message = Message::parse(&bytes).unwrap();
        message.body = gzip.finish().unwrap();
        assert!(message.text().is_err());
    }

    fn http_pair(request: &str, response: &str) -> [Record; 2] {
        [
            Record {
                kind: Kind::HttpRequest,
                time_ms: 0,
                bytes: request.as_bytes().to_vec(),
            },
            Record {
                kind: Kind::HttpResponse,
                time_ms: 1,
                bytes: response.as_bytes().to_vec(),
            },
        ]
    }

    #[test]
    fn login_fixtures_refuse_a_ferroan_channel_and_a_different_circuit() {
        let directory = crate::repo_root()
            .join(".scratch")
            .join(format!("login-fixtures-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let mut udp = crate::arrival_fixtures::tests::source_records();
        let login = crate::arrival_fixtures::prove(&udp).unwrap();
        let last = udp.last_mut().unwrap();
        let packet = ferroan_protocol::parse_packet(&last.bytes).unwrap();
        let mut arrival = packet
            .decode::<ferroan_protocol::msgs::AgentMovementComplete>()
            .unwrap();
        arrival.sim_data.channel_version = b"Ferroan 0.1.0\0".to_vec();
        ferroan_protocol::build_packet(
            &arrival,
            packet.sequence,
            packet.reliable(),
            false,
            &[],
            &mut last.bytes,
        );
        fs::write(
            directory.join("capture-udp.cap"),
            ferroan_capture::write_capture(&udp).unwrap(),
        )
        .unwrap();
        let refused = generate(&directory).unwrap_err();
        assert!(refused.contains("pinned C# channel/version"), "{refused}");

        fs::write(
            directory.join("capture-udp.cap"),
            ferroan_capture::write_capture(&crate::arrival_fixtures::tests::source_records())
                .unwrap(),
        )
        .unwrap();
        let owner = crate::live::ESTATE_OWNER;
        let region = crate::live::REGION_UUID;
        let nil = "00000000-0000-0000-0000-000000000000";
        let register = format!(
            "POST /grid HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\n\r\nMETHOD=register&uuid={region}&serverHttpPort=9000&regionMapTexture={nil}&parcelMapTexture={nil}&regionName=Ferroan%20Test"
        );
        fs::write(
            directory.join("proxy-robust-private.cap"),
            ferroan_capture::write_capture(&http_pair(
                &register,
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nok",
            ))
            .unwrap(),
        )
        .unwrap();
        let body = format!(
            "{{\"agent_id\":\"{owner}\",\"caps_path\":\"11223344-5566-7788-99aa-bbccddeeff00\",\"circuit_code\":\"{}\",\"session_id\":\"{}\",\"secure_session_id\":\"00000000-0000-4000-8000-000000000005\",\"destination_uuid\":\"{region}\",\"packed_appearance\":{{}}}}",
            login.circuit_code.wrapping_add(1),
            login.session_id
        );
        let request = format!(
            "POST /agent/{owner}/ HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{body}"
        );
        fs::write(
            directory.join("proxy-home.cap"),
            ferroan_capture::write_capture(&http_pair(
                &request,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"success\":true,\"reason\":\"ok\"}",
            ))
            .unwrap(),
        )
        .unwrap();
        let mismatched = generate(&directory).unwrap_err();
        fs::remove_dir_all(&directory).unwrap();
        assert!(
            mismatched.contains("does not match the C# login circuit"),
            "{mismatched}"
        );
    }

    #[test]
    fn probe_getagent_never_becomes_the_presence_fixture() {
        let directory = crate::repo_root()
            .join(".scratch")
            .join(format!("probe-presence-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let owner = crate::live::ESTATE_OWNER;
        let region = crate::live::REGION_UUID;
        let present = format!(
            "<?xml version=\"1.0\"?><ServerResponse><result type=\"List\"><UserID>{owner}</UserID><RegionID>{region}</RegionID></result></ServerResponse>"
        );
        let form =
            |method: &str, extra: &str| format!("VERSIONMIN=1&VERSIONMAX=1&METHOD={method}{extra}");
        let request = |path: &str, body: &str, probe: bool| {
            format!(
                "POST {path} HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\n{}Content-Length: {}\r\n\r\n{body}",
                if probe {
                    "X-Ferroan-Probe: harness\r\n"
                } else {
                    ""
                },
                body.len()
            )
        };
        let response = |body: &str| {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        let probe_body = form(
            "getagent",
            "&SessionID=00000000-0000-4000-8000-000000000099&Note=from-probe",
        );
        let real_body = form(
            "getagent",
            "&SessionID=00000000-0000-4000-8000-000000000004",
        );
        let missing_body = form(
            "getagent",
            "&SessionID=00000000-0000-4000-8000-000000000004",
        );
        let logout_body = form("logout", "&SessionID=00000000-0000-4000-8000-000000000004");
        let logged_body = form("loggedout", &format!("&UserID={owner}&RegionID={region}"));
        let success =
            "<?xml version=\"1.0\"?><ServerResponse><result>Success</result></ServerResponse>";
        let missing =
            "<?xml version=\"1.0\"?><ServerResponse><result>null</result></ServerResponse>";
        let mut records = Vec::new();
        records.extend(http_pair(
            &request("/presence", &probe_body, true),
            &response(&present),
        ));
        records.extend(http_pair(
            &request("/presence", &real_body, false),
            &response(&present),
        ));
        records.extend(http_pair(
            &request("/presence", &missing_body, false),
            &response(missing),
        ));
        records.extend(http_pair(
            &request("/presence", &logout_body, false),
            &response(success),
        ));
        records.extend(http_pair(
            &request("/griduser", &logged_body, false),
            &response(success),
        ));
        fs::write(
            directory.join("proxy-robust-private.cap"),
            ferroan_capture::write_capture(&records).unwrap(),
        )
        .unwrap();
        let selected = service_exchanges_selected(&directory, false).unwrap();
        assert!(
            !selected["presence-get"]["request"]
                .as_str()
                .unwrap()
                .contains("from-probe"),
            "{}",
            selected["presence-get"]["request"]
        );
        let probe_only = [
            http_pair(
                &request("/presence", &probe_body, true),
                &response(&present),
            ),
            http_pair(
                &request("/presence", &missing_body, false),
                &response(missing),
            ),
            http_pair(
                &request("/presence", &logout_body, false),
                &response(success),
            ),
            http_pair(
                &request("/griduser", &logged_body, false),
                &response(success),
            ),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        fs::write(
            directory.join("proxy-robust-private.cap"),
            ferroan_capture::write_capture(&probe_only).unwrap(),
        )
        .unwrap();
        let error = service_exchanges_selected(&directory, false).unwrap_err();
        fs::remove_dir_all(&directory).unwrap();
        assert!(error.contains("presence-get"), "{error}");
    }

    #[test]
    fn neighbour_teleport_fixture_replays_and_compares() {
        let directory = crate::repo_root().join("fixtures/neighbour-teleport");
        let auth = format!("auth={SECRET}");
        for name in [
            "capture-udp.cap",
            "capture-udp-neighbour.cap",
            "proxy-home.cap",
            "proxy-neighbour.cap",
        ] {
            let bytes = fs::read(directory.join(name)).unwrap();
            let captured = read_complete_prefix(&bytes).unwrap();
            ferroan_capture::check_udp(&captured).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            let auth_values = text.matches("auth=").count();
            let auth_placeholders = text.matches(auth.as_str()).count();
            assert_eq!(auth_values, auth_placeholders, "{name} CloseAgent auth");
        }
        let line = compare_neighbour_teleport(&directory).unwrap();
        assert!(line.contains("ReleaseAgent 0"), "{line}");
        assert!(line.contains("CloseAgent 1"), "{line}");
        assert!(line.contains("CrossedRegion 0"), "{line}");
        assert!(line.contains("UseCircuitCode 2"), "{line}");
        assert!(line.contains("no UDP TeleportFinish"), "{line}");
    }
}
