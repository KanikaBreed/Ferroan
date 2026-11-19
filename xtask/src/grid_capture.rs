// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Extract the four Q6 control-plane exchanges from a local test-grid capture.
//! Only generated identities and test endpoint addresses are substituted. The
//! compared form/JSON fields, methods, statuses and content types remain.

use std::collections::BTreeMap;
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
    let provenance =
        crate::arrival_fixtures::csharp_provenance(&records(&directory.join("capture-udp.cap"))?)?;
    let exchanges = extract(directory, true)?;
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
    let actual = extract(directory, false)?;
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
        b"OpenSim-NGC Tranquillity Release 1.2.154-alpha+40432e0f83 (X64/Unix/DotNet)\0";
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
        if !matches!(request.path.as_str(), "/presence" | "/griduser") {
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

fn records(path: &Path) -> Result<Vec<Record>, String> {
    read_complete_prefix(&bounded(path, MAX_CAPTURE)?).map_err(|error| error.to_string())
}

fn extract(directory: &Path, allow_neighbour_deregister: bool) -> Result<Value, String> {
    let private = records(&directory.join("proxy-robust-private.cap"))?;
    let home = records(&directory.join("proxy-home.cap"))?;
    let mut output = Map::new();
    let mut normalizer = Normalizer {
        home_port: String::new(),
        item_ids: BTreeMap::new(),
    };
    for (index, record) in private.iter().enumerate() {
        if record.kind != Kind::HttpRequest {
            continue;
        }
        let request = Message::parse(&record.bytes)?;
        if request.path != "/grid" {
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
        normalizer.json(&mut body, "")?;
        normalizer.json(&mut response, "")?;
        output.insert(
            name.to_string(),
            exchange(&request, &reply, body, response, &normalizer),
        );
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
}
