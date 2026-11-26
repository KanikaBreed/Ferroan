// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pin8 `ParcelProperties` elements. The fixture is the original `<map>` element.
//! The poll id stays in the captured response and is not written. The extractor
//! slices each event out of the captured body and copies a `ParcelProperties`
//! element.

use std::fs;
use std::path::Path;

use ferroan_capture::{Kind, Record, read_capture};
use ferroan_protocol::msgs::{
    AgentMovementComplete, AgentUpdate, ParcelPropertiesRequest, TeleportLocationRequest,
    UseCircuitCode,
};
use ferroan_protocol::{Msg, parse_packet};

use super::login_capture::{Message, bounded, object_cross::decode_original, records};

const LIMIT: u64 = 64 * 1024 * 1024;
const HEADER: &[u8] = b"<llsd><map><key>events</key><array>";
const OWNER: &str = crate::live::ESTATE_OWNER;

struct Moment {
    name: &'static str,
    sequence: i32,
    result: i32,
    file: Option<&'static str>,
}

const MOMENTS: [Moment; 6] = [
    Moment {
        name: "Your Parcel",
        sequence: 0,
        result: 0,
        file: Some("arrival.bin"),
    },
    Moment {
        name: "West",
        sequence: 7,
        result: 0,
        file: Some("request-cell.bin"),
    },
    Moment {
        name: "West",
        sequence: 8,
        result: 1,
        file: Some("request-span-west.bin"),
    },
    Moment {
        name: "East",
        sequence: 8,
        result: 1,
        file: Some("request-span-east.bin"),
    },
    Moment {
        name: "West",
        sequence: 0,
        result: 0,
        file: None,
    },
    Moment {
        name: "East",
        sequence: 0,
        result: 0,
        file: Some("walk.bin"),
    },
];

const PUBLISHED: [&str; 5] = [
    "arrival.bin",
    "request-cell.bin",
    "request-span-west.bin",
    "request-span-east.bin",
    "walk.bin",
];

struct Split<'a> {
    elements: Vec<&'a [u8]>,
    #[cfg(test)]
    trailer: &'a [u8],
}

pub(super) fn extract(root: &Path) -> Result<String, String> {
    let udp = read_capture(&bounded(&root.join("capture-udp.cap"), LIMIT)?)
        .map_err(|error| error.to_string())?;
    let http = records(&root.join("proxy-home.cap"))?;
    let elements = select(&udp, &http)?;
    let out = root.join("parcel-properties");
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    for (moment, element) in MOMENTS.iter().zip(&elements) {
        let Some(file) = moment.file else {
            continue;
        };
        fs::write(out.join(file), element).map_err(|error| error.to_string())?;
    }
    let report = replay(&out)?;
    Ok(format!("{report}; fixtures in {}", out.display()))
}

fn select(udp: &[Record], http: &[Record]) -> Result<Vec<Vec<u8>>, String> {
    let window = owner_window(udp)?;
    let mut requests = Vec::new();
    let mut teleport = None;
    let mut arrival = None;
    let mut handle = None;
    let mut walked = false;
    let mut teleport_at = None;
    for record in window {
        let packet = parse_packet(&record.bytes).map_err(|error| error.to_string())?;
        if packet.resent() {
            continue;
        }
        match (record.kind, packet.id) {
            (Kind::UdpOut, AgentMovementComplete::ID) if arrival.is_none() => {
                let message = decode_original::<AgentMovementComplete>(&packet, &record.bytes)?;
                arrival = Some(message.data.position);
                handle = Some(message.data.region_handle);
            }
            (Kind::UdpIn, ParcelPropertiesRequest::ID) => {
                let message = decode_original::<ParcelPropertiesRequest>(&packet, &record.bytes)?;
                if message.agent_data.agent_id.to_string() != OWNER {
                    return Err("ParcelPropertiesRequest is not the estate owner".into());
                }
                requests.push(message);
            }
            (Kind::UdpIn, TeleportLocationRequest::ID) if teleport.is_none() => {
                let message = decode_original::<TeleportLocationRequest>(&packet, &record.bytes)?;
                if message.agent_data.agent_id.to_string() != OWNER {
                    return Err("TeleportLocationRequest is not the estate owner".into());
                }
                teleport = Some(message);
                teleport_at = Some(record.time_ms);
            }
            (Kind::UdpIn, AgentUpdate::ID) => {
                let message = decode_original::<AgentUpdate>(&packet, &record.bytes)?;
                if teleport_at.is_some_and(|start| record.time_ms > start)
                    && message.agent_data.control_flags & 1 != 0
                {
                    walked = true;
                }
            }
            _ => {}
        }
    }
    let arrival = arrival.ok_or("missing AgentMovementComplete")?;
    if (arrival.x - 128.0).abs() > 2.0 || (arrival.y - 128.0).abs() > 2.0 {
        return Err(format!(
            "arrival at {:.1},{:.1} is outside the default parcel",
            arrival.x, arrival.y
        ));
    }
    let [cell, span] = requests.as_slice() else {
        return Err(format!(
            "expected one cell request and one span request, found {}",
            requests.len()
        ));
    };
    expect_request(cell, 7, [5.0, 1.0, 6.0, 2.0])?;
    expect_request(span, 8, [4.0, 0.0, 12.0, 4.0])?;
    let teleport = teleport.ok_or("missing TeleportLocationRequest")?;
    let position = teleport.info.position;
    let look = teleport.info.look_at;
    if teleport.info.region_handle != handle.ok_or("missing region handle")?
        || position.x != 6.0
        || position.y != 2.0
        || position.z != 30.0
        || look.x != 1.0
        || look.y != 0.0
        || look.z != 0.0
    {
        return Err("teleport did not request West at (6, 2, 30)".into());
    }
    if !walked {
        return Err("no forward AgentUpdate after the teleport".into());
    }
    let elements = event_elements(http)?;
    if elements.len() != MOMENTS.len() {
        return Err(format!(
            "expected {} ParcelProperties elements, found {} ({})",
            MOMENTS.len(),
            elements.len(),
            elements
                .iter()
                .map(|element| element_label(element))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    for (moment, element) in MOMENTS.iter().zip(&elements) {
        expect_element(element, moment)?;
    }
    Ok(elements)
}

fn expect_request(
    message: &ParcelPropertiesRequest,
    sequence: i32,
    bounds: [f32; 4],
) -> Result<(), String> {
    let data = &message.parcel_data;
    if data.sequence_id != sequence
        || data.snap_selection
        || data.west != bounds[0]
        || data.south != bounds[1]
        || data.east != bounds[2]
        || data.north != bounds[3]
    {
        return Err(format!(
            "ParcelPropertiesRequest {sequence} does not match the capture script"
        ));
    }
    Ok(())
}

fn owner_window(udp: &[Record]) -> Result<&[Record], String> {
    let mut start = None;
    for (index, record) in udp.iter().enumerate() {
        if record.kind != Kind::UdpIn {
            continue;
        }
        let packet = parse_packet(&record.bytes).map_err(|error| error.to_string())?;
        if packet.id != UseCircuitCode::ID {
            continue;
        }
        let id = packet
            .decode::<UseCircuitCode>()
            .map_err(|error| error.to_string())?
            .circuit_code
            .id;
        if id.to_string() == OWNER {
            start = Some(index);
            break;
        }
    }
    let start = start.ok_or("missing owner circuit")?;
    for (index, record) in udp.iter().enumerate().skip(start) {
        if record.kind != Kind::UdpOut {
            continue;
        }
        let packet = parse_packet(&record.bytes).map_err(|error| error.to_string())?;
        if packet.id == ferroan_protocol::msgs::LogoutReply::ID {
            return Ok(&udp[start..=index]);
        }
    }
    Err("missing owner LogoutReply".into())
}

fn event_elements(http: &[Record]) -> Result<Vec<Vec<u8>>, String> {
    let mut stack = Vec::new();
    let mut elements = Vec::new();
    for record in http {
        match record.kind {
            Kind::HttpRequest => {
                let message = Message::parse(&record.bytes)?;
                let poll = message.method == "POST"
                    && message.path.starts_with("/CE/")
                    && !has_probe(&record.bytes);
                stack.push(poll);
            }
            Kind::HttpResponse => {
                let poll = stack.pop().ok_or("event-queue response has no request")?;
                if !poll {
                    continue;
                }
                let message = Message::parse(&record.bytes)?;
                match message.status {
                    Some(200) => {
                        let body = message.text()?;
                        let split = split_events(body.as_bytes())?;
                        for element in split.elements {
                            if element.starts_with(
                                b"<map><key>message</key><string>ParcelProperties</string>",
                            ) {
                                if count(element, b"<key>id</key>") != 0 {
                                    return Err(
                                        "poll id is inside a ParcelProperties element".into()
                                    );
                                }
                                elements.push(element.to_vec());
                            }
                        }
                    }
                    Some(502 | 404 | 408 | 503) => {}
                    Some(status) => return Err(format!("event poll status {status}")),
                    None => return Err("event poll response has no status".into()),
                }
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("event poll request has no response".into());
    }
    Ok(elements)
}

fn has_probe(bytes: &[u8]) -> bool {
    let needle = b"x-ferroan-probe";
    bytes
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn split_events(body: &[u8]) -> Result<Split<'_>, String> {
    if !body.starts_with(HEADER) {
        return Err("event queue body does not start with the C# events header".into());
    }
    let mut index = HEADER.len();
    let mut elements = Vec::new();
    while index < body.len() {
        if body[index..].starts_with(b"</array>") {
            let trailer = &body[index..];
            if !trailer.starts_with(b"</array><key>id</key>")
                || !trailer.ends_with(b"</map></llsd>")
                || trailer.windows(5).any(|window| window == b"<map>")
            {
                return Err("event queue trailer is not the C# id suffix".into());
            }
            return Ok(Split {
                elements,
                #[cfg(test)]
                trailer,
            });
        }
        if !body[index..].starts_with(b"<map>") {
            return Err("event element does not open with <map>".into());
        }
        let end = map_end(body, index)?;
        elements.push(&body[index..end]);
        index = end;
    }
    Err("event queue body has no id trailer".into())
}

fn map_end(body: &[u8], start: usize) -> Result<usize, String> {
    let mut index = start;
    let mut depth = 0i32;
    while index < body.len() {
        let (name, next, kind) = tag_at(body, index)?;
        match kind {
            Tag::Close if name == b"map" => {
                depth -= 1;
                if depth == 0 {
                    return Ok(next);
                }
                if depth < 0 {
                    return Err("event element closed an extra map".into());
                }
            }
            Tag::Open if name == b"map" => depth += 1,
            Tag::Open if text_closer(name).is_some() => {
                index = text_end(body, next, name)?;
                continue;
            }
            Tag::Open if name == b"array" || name == b"llsd" => {}
            Tag::Empty => {}
            Tag::Close => {}
            Tag::Open => return Err("event element has an unexpected tag".into()),
        }
        index = next;
    }
    Err("event element map is unclosed".into())
}

enum Tag {
    Open,
    Close,
    Empty,
}

fn tag_at(body: &[u8], index: usize) -> Result<(&[u8], usize, Tag), String> {
    if body.get(index) != Some(&b'<') {
        return Err("event element has a gap between tags".into());
    }
    let mut cursor = index + 1;
    let close = body.get(cursor) == Some(&b'/');
    if close {
        cursor += 1;
    }
    let name_at = cursor;
    while body.get(cursor).is_some_and(u8::is_ascii_alphanumeric) {
        cursor += 1;
    }
    if cursor == name_at {
        return Err("event element has an empty tag".into());
    }
    let name = &body[name_at..cursor];
    let mut empty = false;
    while body.get(cursor).is_some_and(|byte| *byte != b'>') {
        if body[cursor] == b'/' {
            empty = true;
        }
        cursor += 1;
    }
    if body.get(cursor) != Some(&b'>') {
        return Err("event element has an unclosed tag".into());
    }
    let kind = if close {
        Tag::Close
    } else if empty {
        Tag::Empty
    } else {
        Tag::Open
    };
    Ok((name, cursor + 1, kind))
}

fn text_end(body: &[u8], content: usize, name: &[u8]) -> Result<usize, String> {
    let closer = text_closer(name).ok_or("event element has an unexpected tag")?;
    let rest = body
        .get(content..)
        .ok_or("event element ended inside a value")?;
    let at = rest
        .windows(closer.len())
        .position(|window| window == closer)
        .ok_or("event element left a value open")?;
    Ok(content + at + closer.len())
}

fn text_closer(name: &[u8]) -> Option<&'static [u8]> {
    Some(match name {
        b"string" => b"</string>",
        b"key" => b"</key>",
        b"binary" => b"</binary>",
        b"integer" => b"</integer>",
        b"real" => b"</real>",
        b"uuid" => b"</uuid>",
        b"date" => b"</date>",
        b"uri" => b"</uri>",
        b"boolean" => b"</boolean>",
        _ => return None,
    })
}

fn element_label(element: &[u8]) -> String {
    let name = between(element, b"<key>Name</key><string>", b"</string>").unwrap_or("?");
    let sequence = between(element, b"<key>SequenceID</key>", b"<key>").unwrap_or("?");
    let result = between(element, b"<key>RequestResult</key>", b"<key>").unwrap_or("?");
    format!("{name} sequence {sequence} result {result}")
}

fn between<'a>(haystack: &'a [u8], start: &[u8], end: &[u8]) -> Option<&'a str> {
    let at = haystack
        .windows(start.len())
        .position(|window| window == start)?;
    let rest = &haystack[at + start.len()..];
    let stop = rest.windows(end.len()).position(|window| window == end)?;
    std::str::from_utf8(&rest[..stop]).ok()
}

fn expect_element(element: &[u8], moment: &Moment) -> Result<(), String> {
    if !element.starts_with(b"<map><key>message</key><string>ParcelProperties</string>")
        || !element.ends_with(b"</map>")
    {
        return Err(format!("{} is not a ParcelProperties element", moment.name));
    }
    let sequence = if moment.sequence == 0 {
        b"<key>SequenceID</key><integer />".to_vec()
    } else {
        format!(
            "<key>SequenceID</key><integer>{}</integer>",
            moment.sequence
        )
        .into_bytes()
    };
    let result = if moment.result == 0 {
        b"<key>RequestResult</key><integer />".to_vec()
    } else {
        format!(
            "<key>RequestResult</key><integer>{}</integer>",
            moment.result
        )
        .into_bytes()
    };
    let name = format!("<key>Name</key><string>{}</string>", moment.name).into_bytes();
    for needle in [
        sequence.as_slice(),
        result.as_slice(),
        name.as_slice(),
        b"<key>SnapSelection</key><boolean>0</boolean>",
        b"<key>Bitmap</key><binary>",
        b"<integer />",
    ] {
        if count(element, needle) != 1 && needle != b"<integer />" {
            return Err(format!(
                "{} element does not contain one {}",
                moment.file.unwrap_or("landing"),
                String::from_utf8_lossy(needle)
            ));
        }
        if needle == b"<integer />" && count(element, needle) == 0 {
            return Err(format!(
                "{} lost Encode2's empty integer",
                moment.file.unwrap_or("landing")
            ));
        }
    }
    let value = ferroan_llsd::from_xml(element).map_err(|error| error.to_string())?;
    let data = value.get("body").get("ParcelData").at(0);
    if data.get("Name").as_str() != moment.name
        || data.get("SequenceID").as_i32() != moment.sequence
        || data.get("RequestResult").as_i32() != moment.result
        || data.get("SnapSelection").as_bool()
    {
        return Err(format!(
            "parsed {} does not match the element bytes",
            moment.file.unwrap_or("landing")
        ));
    }
    Ok(())
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn replay(dir: &Path) -> Result<String, String> {
    let mut lengths = Vec::new();
    for (file, moment) in PUBLISHED
        .iter()
        .zip(MOMENTS.iter().filter(|moment| moment.file.is_some()))
    {
        let bytes = bounded(&dir.join(file), LIMIT)?;
        expect_element(&bytes, moment)?;
        if count(&bytes, b"<key>id</key>") != 0 || count(&bytes, b"<llsd>") != 0 {
            return Err(format!("{file} includes the poll envelope"));
        }
        lengths.push(format!("{file} {}", bytes.len()));
    }
    Ok(format!("parcel properties: {}", lengths.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(HEADER);
        body.extend_from_slice(b"<map><key>message</key><string>ParcelProperties</string><key>body</key><map><key>Name</key><string>A&lt;map&gt;B</string><key>SequenceID</key><integer /><key>ParcelData</key><array><map><key>LocalID</key><integer>1</integer></map></array></map></map>");
        body.extend_from_slice(
            b"<map><key>message</key><string>Other</string><key>body</key><map /></map>",
        );
        body.extend_from_slice(b"</array><key>id</key><integer>31415926</integer></map></llsd>");
        body
    }

    #[test]
    fn event_queue_split_keeps_the_poll_id_in_the_trailer() {
        let body = sample();
        let split = split_events(&body).unwrap();
        assert_eq!(split.elements.len(), 2);
        assert_eq!(count(split.elements[0], b"A&lt;map&gt;B"), 1);
        assert_eq!(count(split.elements[0], b"<integer />"), 1);
        assert_eq!(count(split.trailer, b"<key>id</key>"), 1);
        for element in &split.elements {
            assert_eq!(count(element, b"<key>id</key>"), 0);
        }
        let mut gapped = body.clone();
        gapped.insert(HEADER.len(), b'X');
        assert!(split_events(&gapped).is_err());
    }

    fn poll_exchange(request: &[u8], body: &[u8]) -> Vec<Record> {
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\n\r\n".to_vec();
        response.extend_from_slice(body);
        vec![
            Record {
                kind: Kind::HttpRequest,
                time_ms: 0,
                bytes: request.to_vec(),
            },
            Record {
                kind: Kind::HttpResponse,
                time_ms: 1,
                bytes: response,
            },
        ]
    }

    #[test]
    fn a_probe_poll_is_not_copied() {
        let probe = b"POST /CE/seed HTTP/1.1\r\nX-Ferroan-Probe: harness\r\n\r\n";
        assert!(has_probe(probe));
        assert!(!has_probe(b"POST /CE/seed HTTP/1.1\r\nHost: grid\r\n\r\n"));
        let copied = event_elements(&poll_exchange(probe, &sample())).unwrap();
        assert!(copied.is_empty());
    }

    #[test]
    fn a_parcel_element_that_contains_the_poll_id_is_refused() {
        let mut body = sample();
        let marker = b"<string>ParcelProperties</string>";
        let at = body
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap();
        body.splice(
            at + marker.len()..at + marker.len(),
            b"<key>id</key><integer>1</integer>".iter().copied(),
        );
        let request = b"POST /CE/seed HTTP/1.1\r\nHost: grid\r\n\r\n";
        let error = event_elements(&poll_exchange(request, &body)).unwrap_err();
        assert!(
            error.contains("poll id is inside a ParcelProperties element"),
            "{error}"
        );
    }

    #[test]
    fn replay_rejects_an_element_that_includes_the_poll_id() {
        let fixture = crate::repo_root().join("fixtures/parcel-properties");
        let dir =
            std::env::temp_dir().join(format!("parcel-properties-poll-id-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for file in PUBLISHED {
            let mut bytes = fs::read(fixture.join(file)).unwrap();
            if file == "walk.bin" {
                let marker = b"<string>ParcelProperties</string>";
                let at = bytes
                    .windows(marker.len())
                    .position(|window| window == marker)
                    .unwrap();
                bytes.splice(
                    at + marker.len()..at + marker.len(),
                    b"<key>id</key><integer>1</integer>".iter().copied(),
                );
            }
            fs::write(dir.join(file), &bytes).unwrap();
        }
        let error = replay(&dir).unwrap_err();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            error.contains("walk.bin includes the poll envelope"),
            "{error}"
        );
    }

    #[test]
    fn pin8_parcel_properties_replay_the_captured_elements() {
        use sha2::{Digest, Sha256};
        let dir = crate::repo_root().join("fixtures/parcel-properties");
        replay(&dir).unwrap();
        let mut digest = Sha256::new();
        for file in PUBLISHED {
            digest.update(bounded(&dir.join(file), LIMIT).unwrap());
        }
        assert_eq!(
            format!("{:x}", digest.finalize()),
            "3a024d0ab3e9a4e2eefb3208054bc3f60126853eca77df652b2a15e81cca4755"
        );
    }
}
