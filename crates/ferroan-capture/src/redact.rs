// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replace identity fields before a capture is committed.
//!
//! A packet the server sent is refused when `reencode_datagram_keeping_quats`
//! does not reproduce the stored bytes. That re-encode keeps each packed
//! quaternion's x, y and z: C# `ToBytes` output read back and packed again is
//! not always the same bytes, so a valid C# packet cannot fail on its
//! quaternions. A packet the client sent may differ in packed quaternion
//! components, within the packing tolerance, and nowhere else, and its
//! zero-coding must be canonical. That check has to pass on the raw file,
//! before redaction. The redacted datagram is then `scrub_datagram` of those
//! bytes, which also keeps quaternion bytes, so the committed file still
//! round-trips and no longer carries the session.
//!
//! One name stays: a `UUIDNameReply` block that names its own id `Unknown`
//! and the id, which is what C# sends for an id no grid service knows (TB89).
//! It names nobody, and the fixture's bytes then show that reply.

use ferroan_protocol::msgs::UUIDNameReply;
use ferroan_protocol::packet::build_raw;
use ferroan_protocol::{
    Msg, PacketFlags, Writer, client_datagram_within_quat_packing, message_info, parse_packet,
    reencode_datagram, reencode_datagram_keeping_quats, scrub_datagram, str_field,
};
use std::path::{Path, PathBuf};

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

/// One UDP record whose bytes do not survive `reencode_datagram`.
///
/// `index` is its position among every record in the file, including HTTP.
/// The harness writes this to `.scratch/live/diag/reencode-mismatch.log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpMismatch {
    pub index: usize,
    pub direction: &'static str,
    pub message: String,
    pub time_ms: u64,
    pub original: Vec<u8>,
    pub encoded: Option<Vec<u8>>,
    pub detail: String,
}

impl UdpMismatch {
    /// One line for the job log. The full datagrams stay in the mismatch file.
    pub fn summary(&self) -> String {
        let encoded = self
            .encoded
            .as_ref()
            .map(|bytes| bytes.len().to_string())
            .unwrap_or_else(|| "none".to_string());
        let at = first_difference(&self.original, self.encoded.as_deref());
        let place = at
            .map(|offset| format!("first difference at {offset}"))
            .unwrap_or_else(|| "no shared prefix".to_string());
        format!(
            "{} at {} ms, {}, original {} bytes, encoded {encoded} bytes, {place}",
            self.message,
            self.time_ms,
            self.detail,
            self.original.len(),
        )
    }
}

/// The first UDP record that does not re-encode, if any.
pub fn udp_mismatch(records: &[Record]) -> Option<UdpMismatch> {
    for (index, record) in records.iter().enumerate() {
        if !is_udp(record.kind) {
            continue;
        }
        let direction = kind_name(record.kind);
        let message = message_name(&record.bytes);
        let again = if record.kind == Kind::UdpOut {
            reencode_datagram_keeping_quats(&record.bytes)
        } else {
            reencode_datagram(&record.bytes)
        };
        match again {
            Ok(again) if again == record.bytes => {}
            Ok(_)
                if record.kind == Kind::UdpIn
                    && matches!(client_datagram_within_quat_packing(&record.bytes), Ok(true)) => {}
            Ok(again) => {
                return Some(UdpMismatch {
                    index,
                    direction,
                    message,
                    time_ms: record.time_ms,
                    original: record.bytes.clone(),
                    encoded: Some(again),
                    detail: "the encoded bytes differ".to_string(),
                });
            }
            Err(error) => {
                return Some(UdpMismatch {
                    index,
                    direction,
                    message,
                    time_ms: record.time_ms,
                    original: record.bytes.clone(),
                    encoded: None,
                    detail: error.to_string(),
                });
            }
        }
    }
    None
}

pub fn check_udp(records: &[Record]) -> Result<(), RedactError> {
    match udp_mismatch(records) {
        None => Ok(()),
        Some(mismatch) => Err(RedactError::Udp {
            index: mismatch.index,
            kind: mismatch.direction,
            detail: mismatch.summary(),
        }),
    }
}

/// Write the failing datagram next to the other live diagnostics.
/// The name matches the `live-logs-*` artifact glob.
pub fn write_udp_mismatch(directory: &Path, mismatch: &UdpMismatch) -> Result<PathBuf, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let path = directory.join("reencode-mismatch.log");
    let mut text = format!(
        "record {}\ndirection {}\nmessage {}\ntime_ms {}\noriginal_len {}\n",
        mismatch.index,
        mismatch.direction,
        mismatch.message,
        mismatch.time_ms,
        mismatch.original.len()
    );
    match &mismatch.encoded {
        Some(encoded) => {
            text.push_str(&format!("encoded_len {}\n", encoded.len()));
            match first_difference(&mismatch.original, Some(encoded)) {
                Some(offset) => text.push_str(&format!("first_difference {offset}\n")),
                None => text.push_str("first_difference none\n"),
            }
            text.push_str("encoded_hex\n");
            text.push_str(&hex(encoded));
            text.push('\n');
        }
        None => text.push_str("encoded_len none\nfirst_difference none\n"),
    }
    text.push_str(&format!("detail {}\noriginal_hex\n", mismatch.detail));
    text.push_str(&hex(&mismatch.original));
    text.push('\n');
    std::fs::write(&path, text)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(path)
}

fn message_name(bytes: &[u8]) -> String {
    let Ok(packet) = parse_packet(bytes) else {
        return "unparsed".to_string();
    };
    message_info(packet.id)
        .map(|info| info.name.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn first_difference(original: &[u8], encoded: Option<&[u8]>) -> Option<usize> {
    let encoded = encoded?;
    original
        .iter()
        .zip(encoded.iter())
        .position(|(left, right)| left != right)
        .or_else(|| (original.len() != encoded.len()).then_some(original.len().min(encoded.len())))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn redact_records(records: &[Record]) -> Result<Vec<Record>, RedactError> {
    check_udp(records)?;
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let bytes = if is_udp(record.kind) {
            let scrubbed = scrub_datagram(&record.bytes).map_err(|error| RedactError::Udp {
                index: out.len(),
                kind: kind_name(record.kind),
                detail: error.to_string(),
            })?;
            keep_unknown_names(&record.bytes, scrubbed)
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

/// Put back the names of each `UUIDNameReply` block that reads `Unknown`
/// and its own id. Anything that does not parse is left as scrubbed.
fn keep_unknown_names(original: &[u8], scrubbed: Vec<u8>) -> Vec<u8> {
    let (Ok(raw), Ok(packet)) = (parse_packet(original), parse_packet(&scrubbed)) else {
        return scrubbed;
    };
    if raw.id != UUIDNameReply::ID {
        return scrubbed;
    }
    let (Ok(raw_reply), Ok(mut reply)) = (
        raw.decode::<UUIDNameReply>(),
        packet.decode::<UUIDNameReply>(),
    ) else {
        return scrubbed;
    };
    let mut kept = false;
    for (raw_block, block) in raw_reply
        .uuid_name_block
        .iter()
        .zip(reply.uuid_name_block.iter_mut())
    {
        if raw_block.first_name == b"Unknown\0"
            && raw_block.last_name == str_field(&raw_block.id.to_string())
        {
            block.first_name = raw_block.first_name.clone();
            block.last_name = raw_block.last_name.clone();
            kept = true;
        }
    }
    if !kept {
        return scrubbed;
    }
    let mut payload = Writer::new();
    UUIDNameReply::ID.encode(&mut payload.buf);
    reply.encode_body(&mut payload);
    let mut out = Vec::new();
    build_raw(
        &payload.buf,
        packet.flags & PacketFlags::ZEROCODED != 0,
        packet.sequence,
        packet.reliable(),
        packet.resent(),
        &packet.acks,
        &mut out,
    );
    out
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
    fn an_unknown_name_reply_keeps_its_placeholder_and_a_real_name_does_not() {
        use ferroan_protocol::msgs::uuid_name_reply::UUIDNameBlock;
        let unknown = uuid::Uuid::from_u128(0x84);
        let named = uuid::Uuid::from_u128(0x85);
        let reply = UUIDNameReply {
            uuid_name_block: vec![
                UUIDNameBlock {
                    id: unknown,
                    first_name: str_field("Unknown"),
                    last_name: str_field(&unknown.to_string()),
                },
                UUIDNameBlock {
                    id: named,
                    first_name: str_field("Unknown"),
                    last_name: str_field("Person"),
                },
            ],
            ..UUIDNameReply::default()
        };
        let mut bytes = Vec::new();
        ferroan_protocol::build_packet(&reply, 7, true, false, &[], &mut bytes);
        let record = Record {
            kind: Kind::UdpOut,
            time_ms: 0,
            bytes,
        };
        let out = redact_records(&[record]).unwrap();
        let packet = parse_packet(&out[0].bytes).unwrap();
        assert!(packet.reliable());
        assert_eq!(packet.sequence, 7);
        let blocks = packet.decode::<UUIDNameReply>().unwrap().uuid_name_block;
        assert_eq!(blocks[0].first_name, str_field("Unknown"));
        assert_eq!(blocks[0].last_name, str_field(&unknown.to_string()));
        assert_eq!(blocks[1].first_name, b"Redacted");
        assert_eq!(blocks[1].last_name, b"Redacted");
    }

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
    fn a_datagram_that_does_not_reencode_keeps_its_name_and_bytes() {
        let mut message = UseCircuitCode::default();
        message.circuit_code.code = 99;
        message.circuit_code.session_id = Uuid::from_u128(0x1111);
        message.circuit_code.id = Uuid::from_u128(0x2222);
        let mut raw = Vec::new();
        packet::build_packet(&message, 7, true, false, &[], &mut raw);
        raw.push(0xab);
        let records = vec![
            Record {
                kind: Kind::HttpRequest,
                time_ms: 4,
                bytes: b"GET / HTTP/1.1\r\n\r\n".to_vec(),
            },
            Record {
                kind: Kind::UdpIn,
                time_ms: 90,
                bytes: raw.clone(),
            },
        ];
        let mismatch = udp_mismatch(&records).unwrap();
        assert_eq!(mismatch.index, 1);
        assert_eq!(mismatch.direction, "udp in");
        assert_eq!(mismatch.message, "UseCircuitCode");
        assert_eq!(mismatch.time_ms, 90);
        assert_eq!(mismatch.original, raw);
        let encoded = mismatch.encoded.as_ref().unwrap();
        assert_ne!(encoded, &raw);
        assert_eq!(encoded.len() + 1, raw.len());
        let error = check_udp(&records).unwrap_err().to_string();
        assert!(error.contains("record 1 (udp in)"), "{error}");
        assert!(error.contains("UseCircuitCode"), "{error}");
        assert!(error.contains("at 90 ms"), "{error}");
        assert!(error.contains("the encoded bytes differ"), "{error}");
        let directory = std::env::temp_dir().join(format!("ferroan-reencode-{}", Uuid::new_v4()));
        let path = write_udp_mismatch(&directory, &mismatch).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("direction udp in"), "{text}");
        assert!(text.contains("message UseCircuitCode"), "{text}");
        assert!(text.contains("time_ms 90"), "{text}");
        assert!(text.contains("record 1\n"), "{text}");
        assert!(text.contains(&hex(&raw)), "{text}");
        assert!(text.contains(&hex(encoded)), "{text}");
        std::fs::remove_dir_all(&directory).unwrap();
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

    #[test]
    fn a_ferrobot_agent_update_is_field_checked_and_a_server_copy_keeps_its_quaternions() {
        let datagram = ferrobot_agent_update();
        assert_ne!(reencode_datagram(&datagram).unwrap(), datagram);
        let client = vec![Record {
            kind: Kind::UdpIn,
            time_ms: 40,
            bytes: datagram.clone(),
        }];
        check_udp(&client).unwrap();
        let redacted = redact_records(&client).unwrap();
        check_udp(&redacted).unwrap();
        // Redaction changes identities only: the quaternion bytes stay.
        let before = parse_packet(&datagram).unwrap().body;
        let after = parse_packet(&redacted[0].bytes).unwrap().body;
        assert_eq!(after[32..56], before[32..56]);
        // The same bytes as a server packet: the rescale that differs is
        // packing, not layout, so the check keeps x, y, z and passes.
        let server = vec![Record {
            kind: Kind::UdpOut,
            time_ms: 41,
            bytes: datagram.clone(),
        }];
        assert!(udp_mismatch(&server).is_none());
        check_udp(&server).unwrap();
        // Everything outside the quaternions still has to match: a trailing
        // byte fails in both directions.
        let mut torn = datagram.clone();
        torn.push(0xab);
        for kind in [Kind::UdpIn, Kind::UdpOut] {
            let torn_records = vec![Record {
                kind,
                time_ms: 42,
                bytes: torn.clone(),
            }];
            assert!(check_udp(&torn_records).is_err(), "{kind:?}");
        }
        let server = vec![Record {
            kind: Kind::UdpOut,
            time_ms: 42,
            bytes: torn,
        }];
        let mismatch = udp_mismatch(&server).unwrap();
        assert_eq!(mismatch.direction, "udp out");
        assert_eq!(mismatch.message, "AgentUpdate");
    }

    #[test]
    fn every_openmetaverse_tobytes_quaternion_survives_the_server_check() {
        // ToBytes output from the oracle batch, each in an AgentUpdate's
        // BodyRotation and HeadRotation, as a server packet.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/oracle/bytes/quaternion-batch.bin");
        let file = std::fs::read(path).unwrap();
        let base = ferrobot_agent_update_body();
        let mut repacked = 0;
        for record in file.as_chunks::<40>().0 {
            let mut body = base.clone();
            body[1 + 32..1 + 44].copy_from_slice(&record[16..28]);
            body[1 + 44..1 + 56].copy_from_slice(&record[16..28]);
            let datagram = frame_agent_update(&body);
            if reencode_datagram(&datagram).unwrap() != datagram {
                repacked += 1;
            }
            let server = vec![Record {
                kind: Kind::UdpOut,
                time_ms: 44,
                bytes: datagram,
            }];
            assert!(udp_mismatch(&server).is_none());
        }
        assert!(repacked > 0, "packing again changes some C# quaternions");
    }

    #[test]
    fn a_client_datagram_with_non_canonical_zero_coding_is_refused() {
        let datagram = ferrobot_agent_update();
        assert_ne!(datagram[0] & packet::PacketFlags::ZEROCODED, 0);
        assert!(ferroan_protocol::client_datagram_within_quat_packing(&datagram).unwrap());
        // Split the first zero run of two or more into two runs. It decodes
        // to the same body; C# never writes it.
        let mut split = datagram[..6].to_vec();
        let mut index = 6;
        let mut done = false;
        while index < datagram.len() {
            let byte = datagram[index];
            if byte == 0 && !done && index + 1 < datagram.len() && datagram[index + 1] >= 2 {
                split.extend_from_slice(&[0, 1, 0, datagram[index + 1] - 1]);
                index += 2;
                done = true;
                continue;
            }
            if byte == 0 {
                split.extend_from_slice(&datagram[index..index + 2]);
                index += 2;
                continue;
            }
            split.push(byte);
            index += 1;
        }
        assert!(done);
        assert_eq!(
            parse_packet(&split).unwrap().body,
            parse_packet(&datagram).unwrap().body
        );
        assert!(!ferroan_protocol::client_datagram_within_quat_packing(&split).unwrap());
        let records = vec![Record {
            kind: Kind::UdpIn,
            time_ms: 43,
            bytes: split,
        }];
        assert!(check_udp(&records).is_err());
    }

    /// Ferrobot writes glam's normalize, then the sign-flipped x, y, z.
    fn ferrobot_agent_update() -> Vec<u8> {
        frame_agent_update(&ferrobot_agent_update_body())
    }

    fn frame_agent_update(payload: &[u8]) -> Vec<u8> {
        use ferroan_protocol::Msg;
        use ferroan_protocol::msgs::AgentUpdate;

        let mut datagram = Vec::new();
        packet::build_raw(
            payload,
            AgentUpdate::ZEROCODED,
            40,
            false,
            false,
            &[],
            &mut datagram,
        );
        datagram
    }

    /// Message number and body, before framing.
    fn ferrobot_agent_update_body() -> Vec<u8> {
        use ferroan_protocol::Msg;
        use ferroan_protocol::msgs::AgentUpdate;
        use glam::Quat;

        let mut heading = None;
        for step in 1..20_000 {
            let turned = ferrobot_quat(Quat::from_rotation_z(step as f32 * 0.001));
            let decoded =
                ferroan_protocol::unpack_quat(glam::Vec3::new(turned[0], turned[1], turned[2]));
            if ferroan_protocol::pack_quat(decoded) != turned {
                heading = Some(turned);
                break;
            }
        }
        let turned = heading.expect("a heading whose rescaling changes the bytes");
        let mut message = AgentUpdate::default();
        message.agent_data.agent_id = Uuid::from_u128(0x1111);
        message.agent_data.session_id = Uuid::from_u128(0x2222);
        message.agent_data.body_rotation = Quat::from_xyzw(turned[0], turned[1], turned[2], 1.0);
        message.agent_data.head_rotation = message.agent_data.body_rotation;
        message.agent_data.far = 64.0;
        let mut payload = Vec::new();
        AgentUpdate::ID.encode(&mut payload);
        let mut body = ferroan_protocol::Writer::new();
        message.encode_body(&mut body);
        payload.extend(body.buf);
        let packed = turned
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        payload[1 + 32..1 + 44].copy_from_slice(&packed);
        payload[1 + 44..1 + 56].copy_from_slice(&packed);
        payload
    }

    fn ferrobot_quat(q: glam::Quat) -> [f32; 3] {
        let mut q = q.normalize();
        if !q.is_finite() {
            q = glam::Quat::IDENTITY;
        }
        if q.w < 0.0 {
            q = -q;
        }
        [q.x, q.y, q.z]
    }
}
