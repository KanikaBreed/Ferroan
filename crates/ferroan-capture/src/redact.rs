// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replace identity fields before a capture is committed.
//!
//! UDP records are refused when `reencode_datagram` does not reproduce the
//! stored bytes. That check has to pass on the raw file, before redaction.
//! The redacted datagram is then `scrub_datagram` of those bytes, so the
//! committed file still round-trips and no longer carries the session.

use ferroan_protocol::{reencode_datagram, scrub_datagram};

use crate::format::{Kind, Record};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RedactError {
    #[error("record {index} ({kind}) does not re-encode: {detail}")]
    Udp {
        index: usize,
        kind: &'static str,
        detail: String,
    },
}

pub fn check_udp(records: &[Record]) -> Result<(), RedactError> {
    for (index, record) in records.iter().enumerate() {
        if !is_udp(record.kind) {
            continue;
        }
        match reencode_datagram(&record.bytes) {
            Ok(again) if again == record.bytes => {}
            Ok(_) => {
                return Err(RedactError::Udp {
                    index,
                    kind: kind_name(record.kind),
                    detail: "the encoded bytes differ".to_string(),
                });
            }
            Err(error) => {
                return Err(RedactError::Udp {
                    index,
                    kind: kind_name(record.kind),
                    detail: error.to_string(),
                });
            }
        }
    }
    Ok(())
}

pub fn redact_records(records: &[Record]) -> Result<Vec<Record>, RedactError> {
    check_udp(records)?;
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let bytes = if is_udp(record.kind) {
            scrub_datagram(&record.bytes).map_err(|error| RedactError::Udp {
                index: out.len(),
                kind: kind_name(record.kind),
                detail: error.to_string(),
            })?
        } else {
            redact_http(&record.bytes)
        };
        out.push(Record {
            kind: record.kind,
            time_ms: record.time_ms,
            bytes,
        });
    }
    Ok(out)
}

fn is_udp(kind: Kind) -> bool {
    matches!(kind, Kind::UdpIn | Kind::UdpOut)
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::UdpIn => "udp in",
        Kind::UdpOut => "udp out",
        Kind::HttpRequest => "http request",
        Kind::HttpResponse => "http response",
    }
}

fn redact_http(raw: &[u8]) -> Vec<u8> {
    let Some(split) = find_subsequence(raw, b"\r\n\r\n") else {
        return redact_text(raw);
    };
    let head_end = split + 4;
    let mut out = redact_headers(&raw[..head_end]);
    if text_body(&out) {
        out.extend(redact_text(&raw[head_end..]));
    } else {
        out.extend_from_slice(&raw[head_end..]);
    }
    out
}

fn text_body(head: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(head).to_ascii_lowercase();
    let Some(line) = lower.lines().find(|line| line.starts_with("content-type:")) else {
        return true;
    };
    line.contains("text")
        || line.contains("xml")
        || line.contains("json")
        || line.contains("form")
        || line.contains("llsd")
}

fn redact_headers(head: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let (name, _rest) = match line.split_once(':') {
            Some(pair) => pair,
            None => {
                out.push_str(&redact_ipv4(line));
                continue;
            }
        };
        let key = name.trim().to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
        ) {
            let ending = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str(name);
            out.push_str(": Redacted");
            out.push_str(ending);
        } else {
            out.push_str(&redact_ipv4(line));
        }
    }
    out.into_bytes()
}

fn redact_text(body: &[u8]) -> Vec<u8> {
    let mut text = redact_ipv4(&String::from_utf8_lossy(body));
    for name in [
        "password",
        "passwd",
        "token",
        "secret",
        "first",
        "last",
        "firstname",
        "lastname",
        "session",
        "session_id",
        "secure_session_id",
        "agent_id",
        "circuit_code",
        "seed_capability",
    ] {
        text = redact_named_value(&text, name);
    }
    for key in [
        "owner_id",
        "agent_id",
        "creator_id",
        "last_owner_id",
        "session_id",
        "secure_session_id",
    ] {
        text = redact_llsd_key(&text, key);
    }
    text.into_bytes()
}

/// `<name>password</name>...<string>secret</string>` becomes a fixed string.
fn redact_named_value(text: &str, name: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let needle = format!("<name>{name}</name>");
    let mut out = String::new();
    let mut rest = text;
    let mut lower_rest = lower.as_str();
    while let Some(at) = lower_rest.find(&needle) {
        out.push_str(&rest[..at + needle.len()]);
        rest = &rest[at + needle.len()..];
        lower_rest = &lower_rest[at + needle.len()..];
        if let Some(value_at) = find_scalar_element(lower_rest) {
            let (start, end) = value_at;
            out.push_str(&rest[..start]);
            out.push_str("Redacted");
            rest = &rest[end..];
            lower_rest = &lower_rest[end..];
        }
    }
    out.push_str(rest);
    out
}

/// The next `<string>`, `<int>` or `<i4>` before another `<name>`.
fn find_scalar_element(lower: &str) -> Option<(usize, usize)> {
    let limit = lower.find("<name>").unwrap_or(lower.len());
    let window = &lower[..limit];
    let mut best: Option<(usize, &str, &str)> = None;
    for (start_tag, end_tag) in [
        ("<string>", "</string>"),
        ("<int>", "</int>"),
        ("<i4>", "</i4>"),
    ] {
        if let Some(start) = window.find(start_tag)
            && best.is_none_or(|(at, _, _)| start < at)
        {
            best = Some((start, start_tag, end_tag));
        }
    }
    let (start, start_tag, end_tag) = best?;
    let from = start + start_tag.len();
    let end = window[from..].find(end_tag)? + from;
    Some((from, end))
}

/// `<key>owner_id</key>` followed by `<uuid>` or `<string>` before the next key.
fn redact_llsd_key(text: &str, key: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let needle = format!("<key>{key}</key>");
    let mut out = String::new();
    let mut rest = text;
    let mut lower_rest = lower.as_str();
    while let Some(at) = lower_rest.find(&needle) {
        out.push_str(&rest[..at + needle.len()]);
        rest = &rest[at + needle.len()..];
        lower_rest = &lower_rest[at + needle.len()..];
        if let Some((from, end)) = llsd_value(lower_rest) {
            out.push_str(&rest[..from]);
            out.push_str("Redacted");
            rest = &rest[end..];
            lower_rest = &lower_rest[end..];
        }
    }
    out.push_str(rest);
    out
}

fn llsd_value(lower: &str) -> Option<(usize, usize)> {
    let limit = lower.find("<key>").unwrap_or(lower.len());
    let window = &lower[..limit];
    let mut best: Option<(usize, usize)> = None;
    for start_tag in ["<uuid>", "<string>"] {
        let Some(start) = window.find(start_tag) else {
            continue;
        };
        let end_tag = start_tag.replace('<', "</");
        let from = start + start_tag.len();
        let Some(end) = window[from..].find(end_tag.as_str()) else {
            continue;
        };
        let end = end + from;
        if best.is_none_or(|(at, _)| start < at) {
            best = Some((from, end));
        }
    }
    best
}

fn redact_ipv4(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(len) = ipv4_at(bytes, i) {
            out.extend_from_slice(b"127.0.0.1");
            i += len;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

fn ipv4_at(bytes: &[u8], start: usize) -> Option<usize> {
    if start > 0 && bytes[start - 1].is_ascii_digit() {
        return None;
    }
    let mut i = start;
    for part in 0..4 {
        if i >= bytes.len() || !bytes[i].is_ascii_digit() {
            return None;
        }
        let mut value = 0u32;
        let mark = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            if i - mark >= 3 {
                return None;
            }
            value = value.saturating_mul(10) + u32::from(bytes[i] - b'0');
            if value > 255 {
                return None;
            }
            i += 1;
        }
        if part < 3 {
            if i >= bytes.len() || bytes[i] != b'.' {
                return None;
            }
            i += 1;
        }
    }
    if i < bytes.len() && bytes[i].is_ascii_digit() {
        return None;
    }
    Some(i - start)
}

fn find_subsequence(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_protocol::msgs::UseCircuitCode;
    use ferroan_protocol::{Msg, packet, reencode_datagram};
    use uuid::Uuid;

    use crate::format::{Kind, write_capture};

    #[test]
    fn a_circuit_packet_loses_its_identity_and_still_matches() {
        let mut message = UseCircuitCode::default();
        message.circuit_code.code = 99;
        message.circuit_code.session_id = Uuid::from_u128(0x1111);
        message.circuit_code.id = Uuid::from_u128(0x2222);
        let mut raw = Vec::new();
        packet::build_packet(&message, 7, true, false, &[], &mut raw);
        let records = vec![Record {
            kind: Kind::UdpIn,
            time_ms: 1,
            bytes: raw,
        }];
        let redacted = redact_records(&records).unwrap();
        assert!(check_udp(&redacted).is_ok());
        assert_eq!(
            reencode_datagram(&redacted[0].bytes).unwrap(),
            redacted[0].bytes
        );
        let back = packet::parse_packet(&redacted[0].bytes)
            .unwrap()
            .decode::<UseCircuitCode>()
            .unwrap();
        assert_eq!(back.circuit_code.code, 1);
        assert_eq!(back.circuit_code.id, Uuid::from_u128(1));
        assert_eq!(back.circuit_code.session_id, Uuid::from_u128(2));
        let stored = write_capture(&redacted).unwrap();
        assert!(
            !stored
                .windows(16)
                .any(|window| window == 0x1111u128.to_be_bytes())
        );
    }

    #[test]
    fn http_text_loses_passwords_addresses_and_auth_headers() {
        let raw = b"POST /login HTTP/1.1\r\n\
Host: 192.0.2.10\r\n\
Authorization: Bearer secret-token\r\n\
Content-Type: text/xml\r\n\
\r\n\
<member><name>password</name><value><string>hunter2</string></value></member>\
<member><name>first</name><value><string>Ada</string></value></member>";
        let records = vec![Record {
            kind: Kind::HttpRequest,
            time_ms: 2,
            bytes: raw.to_vec(),
        }];
        let redacted = redact_records(&records).unwrap();
        let text = String::from_utf8(redacted[0].bytes.clone()).unwrap();
        assert!(!text.contains("192.0.2.10"));
        assert!(!text.contains("secret-token"));
        assert!(!text.contains("hunter2"));
        assert!(!text.contains(">Ada<"));
        assert!(text.contains("127.0.0.1"));
        assert!(text.contains("Authorization: Redacted"));
    }

    #[test]
    fn login_identity_and_an_inventory_owner_are_removed() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\n\r\n\
<member><name>agent_id</name><value><string>11111111-1111-1111-1111-111111111111</string></value></member>\
<member><name>circuit_code</name><value><int>42</int></value></member>\
<key>owner_id</key><uuid>22222222-2222-2222-2222-222222222222</uuid>\
<key>asset_id</key><uuid>33333333-3333-3333-3333-333333333333</uuid>";
        let records = vec![Record {
            kind: Kind::HttpResponse,
            time_ms: 3,
            bytes: raw.to_vec(),
        }];
        let text = String::from_utf8(redact_records(&records).unwrap()[0].bytes.clone()).unwrap();
        assert!(!text.contains("11111111-1111-1111-1111-111111111111"));
        assert!(!text.contains(">42<"));
        assert!(!text.contains("22222222-2222-2222-2222-222222222222"));
        assert!(text.contains("33333333-3333-3333-3333-333333333333"));
    }

    #[test]
    fn the_harness_session_replays() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/captures/session.cap");
        let bytes = std::fs::read(&path).unwrap();
        let records = crate::read_capture(&bytes).unwrap();
        check_udp(&records).unwrap();
        let chat = b"ferroan harness chat";
        assert!(bytes.windows(chat.len()).any(|window| window == chat));
        let mut circuits = 0;
        for record in &records {
            if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
                continue;
            }
            let packet = ferroan_protocol::parse_packet(&record.bytes).unwrap();
            if packet.id != ferroan_protocol::msgs::UseCircuitCode::ID {
                continue;
            }
            let message = packet
                .decode::<ferroan_protocol::msgs::UseCircuitCode>()
                .unwrap();
            assert_eq!(message.circuit_code.code, 1);
            assert_eq!(message.circuit_code.session_id, uuid::Uuid::from_u128(2));
            assert_eq!(message.circuit_code.id, uuid::Uuid::from_u128(1));
            circuits += 1;
        }
        assert!(circuits >= 2, "both bots send UseCircuitCode");
    }
}
