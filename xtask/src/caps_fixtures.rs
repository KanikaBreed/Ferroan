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

pub fn generate(directory: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(directory.join("proxy-home.cap"))
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("read CAPS capture: {error}"))?;
    if bytes.len() as u64 > LIMIT {
        return Err("CAPS capture exceeds 64 MiB".to_string());
    }
    let records = read_capture(&bytes).map_err(|error| error.to_string())?;
    let evidence = extract(&records)?;
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

struct Evidence {
    manifest: Value,
    files: BTreeMap<String, Vec<u8>>,
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
        "tranquillity": "858321883f5487a5d192d36ea9d7b90fa468310c",
        "source": "cargo xtask live run caps grid",
        "redaction": "seed/event URLs and neighbour event agent/IP/port replaced; event IDs and all other body bytes preserved",
        "event_queue_url": EVENT_URL,
        "exchanges": selected,
    });
    Ok(Evidence { manifest, files })
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

fn scrub_events(body: &[u8]) -> Result<Vec<u8>, String> {
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
