// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scrub selected local C# CAPS exchanges without reserializing their bodies.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use ferroan_capture::{Kind, Record, read_capture};
use serde_json::{Map, Value, json};

const LIMIT: u64 = 64 * 1024 * 1024;
const EVENT_URL: &str = "http://192.0.2.1:9000/CE/00000000-0000-4000-8000-000000000001";
const EVENT_PATH: &str = "/CE/00000000-0000-4000-8000-000000000001";
const SEED_PATH: &str = "/CAPS/000000000000400080000000000000020000/";
pub(crate) const CONNECTION_BYTES: &str = "The proxy rewrites hop headers to Connection: close before the region reads the request, and the capture stores those forwarded bytes. response_connection is the Connection value C# copied from that request (HttpResponse.cs copies request.Connection), except where a handler such as the event poll sets Keep-Alive.";

pub fn generate(directory: &Path) -> Result<String, String> {
    let udp = read_records(&directory.join("capture-udp.cap"))?;
    let login = crate::arrival_fixtures::prove(&udp)?;
    let records = read_records(&directory.join("proxy-home.cap"))?;
    let caps_path = login_caps_path(&records, login.circuit_code, login.session_id)?;
    let mut evidence = extract(&records)?;
    seed_belongs(&evidence.seed_path, &caps_path)?;
    evidence.manifest["provenance"] = login.value;
    let target = crate::repo_root().join("fixtures/caps");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    for (name, bytes) in evidence.files {
        fs::write(target.join(name), bytes).map_err(|error| error.to_string())?;
    }
    fs::write(
        target.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&evidence.manifest).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok(
        "caps fixtures: C# seed, URI seed, errors, OPTIONS and held empty poll captured"
            .to_string(),
    )
}

fn read_records(path: &Path) -> Result<Vec<Record>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("read CAPS capture: {error}"))?;
    if bytes.len() as u64 > LIMIT {
        return Err("CAPS capture exceeds 64 MiB".to_string());
    }
    read_capture(&bytes).map_err(|error| error.to_string())
}

struct Evidence {
    manifest: Value,
    files: BTreeMap<String, Vec<u8>>,
    seed_path: String,
}

fn extract(records: &[Record]) -> Result<Evidence, String> {
    let mut outstanding = 0usize;
    let mut selected = Map::new();
    let mut files = BTreeMap::new();
    let mut event_url = String::new();
    let mut seed_path = String::new();
    for (index, record) in records.iter().enumerate() {
        if record.kind == Kind::HttpResponse {
            outstanding = outstanding.checked_sub(1).ok_or("unpaired HTTP response")?;
            continue;
        }
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let was_outstanding = outstanding;
        outstanding += 1;
        let request = Http::parse(&record.bytes)?;
        let mut name = if request.target.starts_with("/CAPS/") {
            match (request.method.as_str(), request.body.as_slice()) {
                ("POST", b"<llsd><array><string>EventQueueGet</string></array></llsd>") => "seed",
                ("POST", b"<llsd><array><uri>EventQueueGet</uri></array></llsd>") => "seed-uri",
                ("POST", b"not LLSD XML") => "seed-malformed",
                ("POST", b"<llsd><array><binary>!</binary></array></llsd>") => {
                    "seed-binary-invalid"
                }
                ("POST", b"<llsd><array><binary>YQ</binary></array></llsd>") => {
                    "seed-binary-unpadded"
                }
                (
                    "POST",
                    b"<llsd><array><binary encoding=\"base16\">61</binary></array></llsd>",
                ) => "seed-binary-base16",
                ("POST", b"<llsd><array><binary encoding=\"base16\"></binary></array></llsd>") => {
                    "seed-binary-empty-base16"
                }
                ("POST", b"<llsd><array><binary encoding=\"base16\"/></array></llsd>") => {
                    "seed-binary-selfclosing-base16"
                }
                ("GET", _) => "seed-get",
                ("OPTIONS", _) => "seed-options",
                _ => continue,
            }
        } else if request.target.starts_with("/CE/") {
            "empty-poll"
        } else {
            continue;
        };
        if was_outstanding != 0 {
            return Err(
                "selected CAPS exchange overlaps another HTTP request; cannot pair safely"
                    .to_string(),
            );
        }
        let next = records
            .get(index + 1)
            .filter(|record| record.kind == Kind::HttpResponse)
            .ok_or("selected CAPS request has no adjacent response")?;
        let reply = Http::parse(&next.bytes)?;
        if request.header("content-type") != "application/llsd+xml" {
            return Err("selected CAPS request has an unexpected MIME type".to_string());
        }
        if matches!(name, "seed-get" | "seed-options") && !request.body.is_empty() {
            return Err("GET/OPTIONS seed capture must have an empty request body".to_string());
        }
        for message in [&request, &reply] {
            if !message.header("content-encoding").is_empty()
                || !message.header("x-content-encoding").is_empty()
            {
                return Err("compressed CAPS evidence is unsupported".to_string());
            }
        }
        if name == "seed" && seed_path.is_empty() {
            seed_path.clone_from(&request.target);
        }
        if name != "empty-poll" && request.target != seed_path {
            return Err("selected seed exchanges use different paths".to_string());
        }
        if name == "empty-poll" {
            let (_, suffix) = event_url
                .split_once("/CE/")
                .ok_or("poll precedes selected seed reply")?;
            if request.target != format!("/CE/{suffix}") || request.method != "POST" {
                return Err("selected poll does not belong to the seed capability".to_string());
            }
            let body = ferroan_llsd::from_xml(&request.body)
                .map_err(|_| "selected poll request is not LLSD")?;
            let Some(map) = body.as_map() else {
                return Err("selected poll is not a map".to_string());
            };
            if map
                .keys()
                .any(|key| !matches!(key.as_str(), "ack" | "done"))
                || body.get("done") != &ferroan_llsd::Llsd::Boolean(false)
                || map
                    .get("ack")
                    .is_some_and(|value| !matches!(value, ferroan_llsd::Llsd::Integer(_)))
            {
                return Err("selected poll contains unexpected fields".to_string());
            }
        }
        let elapsed = next
            .time_ms
            .checked_sub(record.time_ms)
            .ok_or("capture timestamp moved backwards")?;
        if name == "empty-poll" {
            if reply.status == 200 {
                if !neighbour_events(&reply.body)? {
                    continue;
                }
                name = "events";
            } else if reply.status != 502 || elapsed < 55_000 {
                continue;
            }
        }
        if selected.contains_key(name) {
            continue;
        }
        let expected_status = match name {
            "seed-get" => 404,
            "seed-malformed" => 400,
            name if name.starts_with("seed-binary-")
                && name != "seed-binary-selfclosing-base16" =>
            {
                400
            }
            "empty-poll" => 502,
            _ => 200,
        };
        if reply.status != expected_status {
            return Err(format!(
                "{name} capture status {}, expected {expected_status}",
                reply.status
            ));
        }
        let mime = match name {
            "empty-poll" => "text/html",
            "events" => "application/xml",
            _ => "text/html;charset=UTF-8",
        };
        if reply.header("content-type") != mime {
            return Err(format!("{name} captured MIME differs from C# baseline"));
        }
        let mut response = reply.body.clone();
        if matches!(name, "seed" | "seed-uri") {
            let value = ferroan_llsd::from_xml(&reply.body).map_err(|error| error.to_string())?;
            if value.as_map().is_none_or(|map| map.len() != 1) {
                return Err("seed capture must contain only EventQueueGet".to_string());
            }
            let url = value.get("EventQueueGet").as_str();
            if !url.starts_with("http://127.0.0.1:") || !url.contains("/CE/") {
                return Err("seed URL is not the local test event queue".to_string());
            }
            if !event_url.is_empty() && event_url != url {
                return Err(
                    "seed URI and string requests returned different capabilities".to_string(),
                );
            }
            event_url = url.to_string();
            let text = std::str::from_utf8(&reply.body).map_err(|_| "seed body is not UTF-8")?;
            response = text.replace(url, EVENT_URL).into_bytes();
        }
        if (expected_status == 400
            || matches!(
                name,
                "empty-poll" | "seed-get" | "seed-malformed" | "seed-options"
            ))
            && !response.is_empty()
        {
            return Err(format!("captured {name} has a nonempty response"));
        }
        if name == "events" {
            response = scrub_events(&response)?;
        }
        let request_ext = if matches!(name, "seed" | "seed-uri" | "empty-poll" | "events") {
            "xml"
        } else {
            "body"
        };
        let response_ext = if matches!(name, "seed" | "seed-uri" | "events") {
            "xml"
        } else {
            "body"
        };
        let request_file = format!("{name}.request.{request_ext}");
        let response_file = format!("{name}.response.{response_ext}");
        files.insert(request_file.clone(), request.body.clone());
        files.insert(response_file.clone(), response);
        let cors: BTreeMap<_, _> = reply
            .headers
            .iter()
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "access-control-allow-origin"
                        | "access-control-allow-headers"
                        | "access-control-allow-methods"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        selected.insert(name.to_string(), json!({
            "method": request.method, "path": if matches!(name, "empty-poll" | "events") { EVENT_PATH } else { SEED_PATH },
            "status": reply.status, "request_content_type": request.header("content-type"),
            "response_content_type": reply.header("content-type"), "elapsed_ms": elapsed,
            "response_connection": reply.headers.get("connection"),
            "request_file": request_file, "response_file": response_file,
            "cors": cors,
        }));
    }
    if outstanding != 0 {
        return Err("capture ends with outstanding HTTP requests".to_string());
    }
    for required in [
        "seed",
        "seed-uri",
        "seed-get",
        "seed-malformed",
        "seed-binary-invalid",
        "seed-binary-unpadded",
        "seed-binary-base16",
        "seed-binary-empty-base16",
        "seed-binary-selfclosing-base16",
        "seed-options",
        "events",
        "empty-poll",
    ] {
        if !selected.contains_key(required) {
            return Err(format!("no captured {required} exchange"));
        }
    }
    let manifest = json!({
        "tranquillity": "f67db54399c027fd74e5aefcb2193905b4dc1646",
        "source": "cargo xtask live run caps grid",
        "redaction": "seed/event URLs and neighbour event agent/IP/port replaced; event IDs and all other body bytes preserved",
        "connection_bytes": CONNECTION_BYTES,
        "event_queue_url": EVENT_URL,
        "event_queue_url_shape": event_url_shape(&event_url)?,
        "exchanges": selected,
    });
    Ok(Evidence {
        manifest,
        files,
        seed_path,
    })
}

/// The create-agent on the arrival's circuit, not a later or probe request.
pub(crate) fn login_caps_path(
    records: &[Record],
    circuit_code: u32,
    session: uuid::Uuid,
) -> Result<String, String> {
    let mut saw_other = false;
    for (index, record) in records.iter().enumerate() {
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let request = Http::parse(&record.bytes)?;
        if is_probe(&request) || request.method != "POST" || !request.target.starts_with("/agent/")
        {
            continue;
        }
        let reply = adjacent_response(records, index)?;
        // CreateAgent is gzip on the wire (X-Content-Encoding). The seed and
        // event bodies extracted below stay verbatim and still reject compression.
        let body: Value = serde_json::from_slice(&decoded_body(&request)?)
            .map_err(|error| format!("invalid create-agent JSON: {error}"))?;
        let response: Value = serde_json::from_slice(&decoded_body(&reply)?)
            .map_err(|error| format!("invalid create-agent response JSON: {error}"))?;
        if body.get("agent_id").and_then(Value::as_str) != Some(crate::live::ESTATE_OWNER) {
            continue;
        }
        let wire = json_circuit(body.get("circuit_code"))?;
        let wire_session = body.get("session_id").and_then(Value::as_str).unwrap_or("");
        if wire != circuit_code || wire_session != session.to_string() {
            saw_other = true;
            continue;
        }
        if response.get("success") != Some(&Value::Bool(true)) {
            return Err("create-agent on the login circuit is not a successful response".into());
        }
        return body
            .get("caps_path")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "create-agent has no caps_path".to_string());
    }
    if saw_other {
        Err("create-agent does not match the C# login circuit".to_string())
    } else {
        Err("capture has no create-agent on the C# login circuit".to_string())
    }
}

pub(crate) fn seed_belongs(target: &str, caps_path: &str) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(caps_path)
        .map_err(|_| "create-agent caps_path is not a UUID".to_string())?;
    let target = target.trim_end_matches('/');
    let hyphenated = format!("/CAPS/{id}0000");
    let simple = format!("/CAPS/{}0000", id.simple());
    if target == hyphenated || target == simple {
        Ok(())
    } else {
        Err("proxy-home seed path is not the login circuit's capability".to_string())
    }
}

pub(crate) fn json_circuit(value: Option<&Value>) -> Result<u32, String> {
    let code = match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|number| u32::try_from(number).ok()),
        Some(Value::String(text)) => text.parse().ok(),
        _ => None,
    };
    code.ok_or_else(|| "circuit_code is not a u32".to_string())
}

fn is_probe(message: &Http) -> bool {
    !message.header("x-ferroan-probe").is_empty()
}

fn decoded_body(message: &Http) -> Result<Vec<u8>, String> {
    const LIMIT: u64 = 1024 * 1024;
    let encoding = message.header("content-encoding");
    let extra = message.header("x-content-encoding");
    if encoding == "gzip" || extra == "gzip" {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(message.body.as_slice())
            .take(LIMIT + 1)
            .read_to_end(&mut out)
            .map_err(|_| "captured gzip body is invalid".to_string())?;
        if out.len() as u64 > LIMIT {
            return Err("decoded create-agent body exceeds size limit".into());
        }
        Ok(out)
    } else if encoding.is_empty() && extra.is_empty() {
        Ok(message.body.clone())
    } else {
        Err("unsupported captured content encoding".into())
    }
}

fn adjacent_response(records: &[Record], index: usize) -> Result<Http, String> {
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
            "selected create-agent overlaps another request; recapture an isolated login"
                .to_string(),
        );
    }
    let response = records
        .get(index + 1)
        .filter(|record| record.kind == Kind::HttpResponse)
        .ok_or("selected create-agent has no unambiguous adjacent response")?;
    Http::parse(&response.bytes)
}

fn event_url_shape(url: &str) -> Result<Value, String> {
    let local = url
        .strip_prefix("http://127.0.0.1:")
        .ok_or("seed URL is not a local HTTP endpoint")?;
    let (port, path) = local.split_once('/').ok_or("seed URL has no path")?;
    if port.parse::<u16>().ok().is_none_or(|port| port == 0) {
        return Err("seed URL has an invalid port".into());
    }
    let id = path.strip_prefix("CE/").ok_or("seed URL has no CE path")?;
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| "event path is not a UUID")?;
    if uuid.to_string() != id {
        return Err("event UUID is not lowercase hyphenated text".into());
    }
    Ok(json!({
        "path_prefix": "/CE/",
        "uuid_format": "hyphenated-lowercase",
        "uuid_version": uuid.get_version_num(),
        "uuid_variant": format!("{:?}", uuid.get_variant()),
        "trailing_slash": path.ends_with('/'),
    }))
}

struct Http {
    method: String,
    target: String,
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

fn neighbour_events(body: &[u8]) -> Result<bool, String> {
    let value = ferroan_llsd::from_xml(body).map_err(|error| error.to_string())?;
    let events = value.get("events").as_array();
    Ok(events.len() == 2
        && events[0].get("message").as_str() == "EnableSimulator"
        && events[1].get("message").as_str() == "EstablishAgentCommunication")
}

pub(crate) fn scrub_events(body: &[u8]) -> Result<Vec<u8>, String> {
    let value = ferroan_llsd::from_xml(body).map_err(|error| error.to_string())?;
    let events = value.get("events").as_array();
    if !neighbour_events(body)? {
        return Err("unexpected event batch for redaction".to_string());
    }
    let simulator = events[0].get("body").get("SimulatorInfo").at(0);
    let communication = events[1].get("body");
    let port = simulator.get("Port").as_i32();
    let seed = communication.get("seed-capability").as_str();
    if simulator.get("IP").as_binary() != [127, 0, 0, 1]
        || !(1..=65535).contains(&port)
        || communication.get("sim-ip-and-port").as_str() != format!("127.0.0.1:{port}")
        || communication.get("agent-id").as_uuid().to_string() != crate::live::ESTATE_OWNER
        || !seed.starts_with(&format!("http://127.0.0.1:{port}/CAPS/"))
    {
        return Err("event batch is not from the local test neighbour".to_string());
    }
    let mut text = std::str::from_utf8(body)
        .map_err(|_| "events body is not UTF-8")?
        .to_string();
    for (key, replacement) in [
        ("IP", "wAACAQ=="),
        ("Port", "9001"),
        ("sim-ip-and-port", "192.0.2.1:9001"),
        ("agent-id", "00000000-0000-4000-8000-000000000004"),
        (
            "seed-capability",
            "http://192.0.2.1:9001/CAPS/000000000000400080000000000000030000/",
        ),
    ] {
        replace_scalar(&mut text, key, replacement)?;
    }
    Ok(text.into_bytes())
}

/// Replace one known scalar's contents while preserving C#'s XML formatting.
fn replace_scalar(text: &mut String, key: &str, replacement: &str) -> Result<(), String> {
    let marker = format!("<key>{key}</key>");
    if text.matches(&marker).count() != 1 {
        return Err(format!("event has ambiguous {key} field"));
    }
    let from = text.find(&marker).ok_or("missing event scalar")? + marker.len();
    let start = from
        + text[from..]
            .find('>')
            .ok_or("event scalar has no opening tag")?
        + 1;
    let end = start
        + text[start..]
            .find("</")
            .ok_or("event scalar has no closing tag")?;
    if text[start..end].contains('<') {
        return Err("event scalar contains nested tags".to_string());
    }
    text.replace_range(start..end, replacement);
    Ok(())
}

impl Http {
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
            .ok_or("empty HTTP header")?
            .split_whitespace()
            .collect();
        if start.len() < 2 {
            return Err("incomplete HTTP start line".to_string());
        }
        let response = start[0].starts_with("HTTP/");
        let mut headers = BTreeMap::new();
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.to_ascii_lowercase(), value.trim().to_string());
            }
        }
        Ok(Self {
            method: if response {
                String::new()
            } else {
                start[0].to_string()
            },
            target: if response {
                String::new()
            } else {
                start[1].to_string()
            },
            status: if response {
                start[1].parse().map_err(|_| "invalid HTTP status")?
            } else {
                0
            },
            headers,
            body: bytes[split + 4..].to_vec(),
        })
    }
    fn header(&self, name: &str) -> &str {
        self.headers.get(name).map(String::as_str).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_rejects_same_harness_rust_arrival_before_reading_http() {
        use ferroan_protocol::msgs::AgentMovementComplete;
        use ferroan_protocol::{Msg, build_packet, parse_packet};
        let directory = crate::repo_root()
            .join(".scratch")
            .join(format!("caps-provenance-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let mut records = crate::arrival_fixtures::tests::source_records();
        let last = records.last_mut().unwrap();
        let packet = parse_packet(&last.bytes).unwrap();
        assert_eq!(packet.id, AgentMovementComplete::ID);
        let mut movement = packet.decode::<AgentMovementComplete>().unwrap();
        movement.sim_data.channel_version = b"Ferroan 0.1.0\0".to_vec();
        build_packet(
            &movement,
            packet.sequence,
            packet.reliable(),
            false,
            &[],
            &mut last.bytes,
        );
        fs::write(
            directory.join("capture-udp.cap"),
            ferroan_capture::write_capture(&records).unwrap(),
        )
        .unwrap();
        // No HTTP file exists. Reject provenance before parsing or emitting any
        // fixture, even though agent, session, region and handle all match.
        let result = generate(&directory);
        fs::remove_dir_all(directory).unwrap();
        assert!(result.unwrap_err().contains("pinned C# channel/version"));
    }

    #[test]
    fn seed_path_must_be_the_login_circuits_capability() {
        let id = "11223344-5566-7788-99aa-bbccddeeff00";
        seed_belongs(&format!("/CAPS/{id}0000/"), id).unwrap();
        seed_belongs("/CAPS/112233445566778899aabbccddeeff000000", id).unwrap();
        assert!(seed_belongs("/CAPS/ffffffffffffffffffffffffffffffff0000/", id).is_err());
        assert!(seed_belongs("/CAPS/11223344-5566-7788-99aa-bbccddeeff00", id).is_err());
    }

    #[test]
    fn caps_generation_rejects_a_create_agent_from_another_circuit() {
        let directory = crate::repo_root()
            .join(".scratch")
            .join(format!("caps-circuit-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let udp = crate::arrival_fixtures::tests::source_records();
        let login = crate::arrival_fixtures::prove(&udp).unwrap();
        fs::write(
            directory.join("capture-udp.cap"),
            ferroan_capture::write_capture(&udp).unwrap(),
        )
        .unwrap();
        let body = format!(
            "{{\"agent_id\":\"{}\",\"caps_path\":\"11223344-5566-7788-99aa-bbccddeeff00\",\"circuit_code\":\"{}\",\"session_id\":\"{}\",\"secure_session_id\":\"00000000-0000-4000-8000-000000000005\",\"destination_uuid\":\"{}\"}}",
            crate::live::ESTATE_OWNER,
            login.circuit_code.wrapping_add(1),
            login.session_id,
            crate::live::REGION_UUID
        );
        let records = vec![
            Record {
                kind: Kind::HttpRequest,
                time_ms: 0,
                bytes: format!(
                    "POST /agent/{}/ HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    crate::live::ESTATE_OWNER,
                    body.len()
                )
                .into_bytes(),
            },
            Record {
                kind: Kind::HttpResponse,
                time_ms: 1,
                bytes: b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 28\r\n\r\n{\"success\":true,\"reason\":\"ok\"}".to_vec(),
            },
        ];
        fs::write(
            directory.join("proxy-home.cap"),
            ferroan_capture::write_capture(&records).unwrap(),
        )
        .unwrap();
        let error = generate(&directory).unwrap_err();
        fs::remove_dir_all(directory).unwrap();
        assert!(
            error.contains("does not match the C# login circuit"),
            "{error}"
        );
    }

    #[test]
    fn captured_event_url_shape_rejects_non_uuid_and_preserves_version() {
        let shape =
            event_url_shape("http://127.0.0.1:9000/CE/00000000-0000-4000-8000-000000000001")
                .unwrap();
        assert_eq!(shape["uuid_version"], 4);
        assert_eq!(shape["uuid_variant"], "RFC4122");
        let different =
            event_url_shape("http://127.0.0.1:9000/CE/00000000-0000-1000-8000-000000000001")
                .unwrap();
        assert_eq!(different["uuid_version"], 1);
        for url in [
            "http://127.0.0.1:9000/CE/not-a-uuid",
            "http://127.0.0.1:9000/CE/00000000-0000-4000-8000-000000000001/",
            "http://127.0.0.1:0/CE/00000000-0000-4000-8000-000000000001",
        ] {
            assert!(event_url_shape(url).is_err());
        }
    }

    fn exchange(method: &str, path: &str, request_body: &str, response_body: &str) -> Vec<Record> {
        vec![
            Record { kind: Kind::HttpRequest, time_ms: 0, bytes: format!("{method} {path} HTTP/1.1\r\nContent-Type: application/llsd+xml\r\n\r\n{request_body}").into_bytes() },
            Record { kind: Kind::HttpResponse, time_ms: 1, bytes: format!("HTTP/1.1 200 OK\r\nContent-Type: text/html;charset=UTF-8\r\n\r\n{response_body}").into_bytes() },
        ]
    }

    fn seed() -> Vec<Record> {
        let response =
            include_str!("../../fixtures/caps/seed.response.xml").replace("192.0.2.1", "127.0.0.1");
        exchange(
            "POST",
            SEED_PATH,
            include_str!("../../fixtures/caps/seed.request.xml"),
            &response,
        )
    }

    #[test]
    fn extractor_rejects_mixed_capabilities_and_nonempty_get_bodies() {
        let mut records = seed();
        records.extend(exchange("GET", "/CAPS/different/", "", ""));
        assert!(extract(&records).err().unwrap().contains("different paths"));
        let mut records = seed();
        records.extend(exchange("GET", SEED_PATH, "private value", ""));
        assert!(
            extract(&records)
                .err()
                .unwrap()
                .contains("empty request body")
        );
        let mut records = seed();
        records.extend(exchange(
            "POST",
            "/CE/another-agent",
            "<llsd><map /></llsd>",
            "",
        ));
        assert!(extract(&records).err().unwrap().contains("does not belong"));
    }

    #[test]
    fn extractor_rejects_compression_and_ambiguous_pairing() {
        let mut records = seed();
        let response = String::from_utf8(records[1].bytes.clone())
            .unwrap()
            .replace("Content-Type:", "Content-Encoding: gzip\r\nContent-Type:");
        records[1].bytes = response.into_bytes();
        assert!(extract(&records).err().unwrap().contains("compressed CAPS"));
        let mut records = seed();
        records.insert(0, records[0].clone());
        assert!(
            extract(&records)
                .err()
                .unwrap()
                .contains("adjacent response")
        );
    }

    #[test]
    fn event_redaction_preserves_ids_and_xml_layout() {
        let captured = include_str!("../../fixtures/caps/events.response.xml");
        let mut raw = captured.to_string();
        for (key, replacement) in [
            ("IP", "fwAAAQ=="),
            ("Port", "12345"),
            ("agent-id", crate::live::ESTATE_OWNER),
            ("sim-ip-and-port", "127.0.0.1:12345"),
            ("seed-capability", "http://127.0.0.1:12345/CAPS/test-seed/"),
        ] {
            replace_scalar(&mut raw, key, replacement).unwrap();
        }
        assert_eq!(scrub_events(raw.as_bytes()).unwrap(), captured.as_bytes());
        assert!(
            replace_scalar(
                &mut "<key>IP</key><string>x</string><key>IP</key><string>y</string>".to_string(),
                "IP",
                "z"
            )
            .is_err()
        );
    }
}
