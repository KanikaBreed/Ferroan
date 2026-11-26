// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Capture the negotiated material-override datagram and compare it with Pin8.
//! Pin8 c401bceecd1684832ae8ba86a31b5ee01a4ded7b,
//! Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs, needMaterials.

use ferroan_bot::scenario::PRIM_CAPTURE_COUNT;
use ferroan_capture::{Kind, Record, read_capture, write_capture};
use ferroan_protocol::msgs::generic_streaming_message::{DataBlock, MethodData};
use ferroan_protocol::msgs::{GenericStreamingMessage, ObjectUpdate};
use ferroan_protocol::{IncomingPacket, Msg, PacketFlags, Writer, packet::build_raw, parse_packet};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::process::Command;
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const VIEWERS: [&str; 2] = ["control", "limited"];
const BOX: &str = "65000000-0000-4000-8000-000000000001";
const TREE: &str = "65000000-0000-4000-8000-000000000003";
const PROBE: &str = "65000000-0000-4000-8000-000000000006";

fn encoded<M: Msg>(packet: &IncomingPacket, msg: &M) -> Vec<u8> {
    let mut payload = Writer::new();
    M::ID.encode(&mut payload.buf);
    msg.encode_body(&mut payload);
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

fn checked_message<M: Msg>(packet: &IncomingPacket, original: &[u8]) -> Result<M> {
    let msg = packet.decode::<M>()?;
    if encoded(packet, &msg) != original {
        return Err(format!(
            "unredacted {} seq {} differs from the captured datagram",
            M::NAME,
            packet.sequence
        )
        .into());
    }
    Ok(msg)
}

fn face(te: u8, data: &str) -> Vec<u8> {
    let text = data.as_bytes();
    let len = u16::try_from(text.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(3 + text.len());
    out.push(te);
    out.extend(len.to_le_bytes());
    out.extend(text);
    out
}

fn materials_blob(faces: &[(u8, &str)]) -> Vec<u8> {
    let mut out = vec![u8::try_from(faces.len()).unwrap_or(u8::MAX)];
    for (te, data) in faces {
        out.extend(face(*te, data));
    }
    out
}

fn blob_hex(faces: &[(u8, &str)]) -> String {
    materials_blob(faces)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

/// Patch the disposable scene copy. The committed store is left unchanged.
pub(super) fn install_store(db: &Path) -> std::result::Result<(), String> {
    let sql = format!(
        "UPDATE primshapes SET MatOvrd = X'{}' WHERE UUID = '{BOX}';\n\
         UPDATE primshapes SET MatOvrd = X'{}' WHERE UUID = '{TREE}';\n\
         UPDATE primshapes SET MatOvrd = X'{}' WHERE UUID = '{PROBE}';",
        blob_hex(&[(0, "{'bc':[r1,r0,r0,r1]}"), (2, "{'ds':1}"),]),
        blob_hex(&[(0, "{'tree':1}")]),
        blob_hex(&[(0, "{'probe':1}")]),
    );
    let output = Command::new("sqlite3")
        .arg(db)
        .arg(format!(
            "{sql}\nSELECT COUNT(*) FROM primshapes WHERE UUID IN ('{BOX}','{TREE}','{PROBE}') AND length(MatOvrd) > 1;"
        ))
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() || output.stdout != b"3\n" {
        return Err("could not store the material override entries".into());
    }
    Ok(())
}

fn part_numbers() -> Result<HashMap<Uuid, u32>> {
    let map: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/prim-updates/map.json"))?;
    let mut numbers = HashMap::new();
    for part in map["parts"].as_array().ok_or("missing prim map")? {
        let store: Uuid = part["store_uuid"]
            .as_str()
            .ok_or("missing mapped uuid")?
            .parse()?;
        let capture: Uuid = part["capture_uuid"]
            .as_str()
            .ok_or("missing mapped uuid")?
            .parse()?;
        let number = u32::try_from(capture.as_u128() & 0xff)?;
        numbers.insert(store, number);
    }
    Ok(numbers)
}

fn local_parts(records: &[Record], parts: &HashMap<Uuid, u32>) -> Result<HashMap<u32, u32>> {
    let mut locals = HashMap::new();
    let mut seen = BTreeSet::new();
    for record in records {
        let packet = parse_packet(&record.bytes)?;
        if packet.id != ObjectUpdate::ID {
            continue;
        }
        for block in packet.decode::<ObjectUpdate>()?.object_data {
            if block.p_code == 47 {
                continue;
            }
            let part = *parts.get(&block.full_id).ok_or("unknown stored part")?;
            if locals
                .insert(block.id, part)
                .is_some_and(|previous| previous != part)
            {
                return Err("one local id belongs to two stored parts".into());
            }
            seen.insert(part);
        }
    }
    if seen.len() != PRIM_CAPTURE_COUNT {
        return Err(format!("viewer saw {} of the eight stored parts", seen.len()).into());
    }
    Ok(locals)
}

fn local_from_token(token: &str) -> Result<u32> {
    if let Ok(signed) = token.parse::<i32>() {
        Ok(signed as u32)
    } else {
        Ok(token.parse::<u32>()?)
    }
}

fn alias_notation(notation: &str, locals: &HashMap<u32, u32>) -> Result<String> {
    const MARKER: &str = "'id':i";
    if notation.matches(MARKER).count() != 1 {
        return Err("material notation does not carry one id".into());
    }
    let start = notation.find(MARKER).ok_or("material notation has no id")?;
    let digits = start + MARKER.len();
    let rest = &notation[digits..];
    let end = rest
        .find(|c: char| c != '-' && !c.is_ascii_digit())
        .ok_or("material id has no terminator")?;
    let local = local_from_token(&rest[..end])?;
    let part = locals
        .get(&local)
        .ok_or("material local id is not a stored part")?;
    Ok(format!("{}{part}{}", &notation[..digits], &rest[end..]))
}

fn alias_material(
    packet: &IncomingPacket,
    original: &[u8],
    locals: &HashMap<u32, u32>,
) -> Result<Vec<u8>> {
    if !packet.reliable() || packet.flags & PacketFlags::ZEROCODED != 0 {
        return Err("material override was unreliable or zero-coded".into());
    }
    let message: GenericStreamingMessage = checked_message(packet, original)?;
    if message.method_data.method != 16757 {
        return Err(format!("material method {}", message.method_data.method).into());
    }
    let notation = std::str::from_utf8(&message.data_block.data)?;
    let aliased = alias_notation(notation, locals)?;
    let message = GenericStreamingMessage {
        method_data: MethodData {
            method: message.method_data.method,
        },
        data_block: DataBlock {
            data: aliased.into_bytes(),
        },
    };
    Ok(encoded(packet, &message))
}

/// Non-resend method 16757 payloads, in arrival order. A resend is not another send.
fn material_datagrams(records: &[Record]) -> Result<Vec<Vec<u8>>> {
    let mut payloads = Vec::new();
    for record in records {
        let packet = parse_packet(&record.bytes)?;
        if packet.id != GenericStreamingMessage::ID || packet.resent() {
            continue;
        }
        if !packet.reliable() || packet.flags & PacketFlags::ZEROCODED != 0 {
            return Err("published material override was unreliable or zero-coded".into());
        }
        let message = packet.decode::<GenericStreamingMessage>()?;
        if message.method_data.method != 16757 {
            return Err("published material method was not 16757".into());
        }
        payloads.push(message.data_block.data);
    }
    Ok(payloads)
}

fn require_region_copy(server: &[Record], viewers: &[Vec<Record>; 2]) -> Result<()> {
    let mut counts: HashMap<&[u8], usize> = HashMap::new();
    for record in server {
        if record.kind != Kind::UdpOut {
            continue;
        }
        let packet = parse_packet(&record.bytes)?;
        if packet.id == GenericStreamingMessage::ID {
            *counts.entry(record.bytes.as_slice()).or_default() += 1;
        }
    }
    for records in viewers {
        for record in records {
            let packet = parse_packet(&record.bytes)?;
            if packet.id != GenericStreamingMessage::ID {
                continue;
            }
            let count = counts
                .get_mut(record.bytes.as_slice())
                .ok_or("viewer material datagram has no matching region occurrence")?;
            if *count == 0 {
                return Err("viewer material datagram has no matching region occurrence".into());
            }
            *count -= 1;
        }
    }
    Ok(())
}

fn read_fixture(name: &str) -> Result<Vec<Record>> {
    let bytes: &[u8] = match name {
        "control.cap" => include_bytes!("../../fixtures/material-overrides/control.cap"),
        "limited.cap" => include_bytes!("../../fixtures/material-overrides/limited.cap"),
        other => return Err(format!("unknown material fixture {other}").into()),
    };
    Ok(read_capture(bytes)?)
}

pub(super) fn extract(run: &Path, home_log: &Path, ferroan: bool) -> Result<String> {
    let parts = part_numbers()?;
    let bytes = super::login_capture::bounded(&run.join("capture-udp.cap"), 64 * 1024 * 1024)?;
    let server = read_capture(&bytes)?;
    let mut viewers = [Vec::new(), Vec::new()];
    for (stream, alias) in viewers.iter_mut().zip(VIEWERS) {
        let path = home_log.with_file_name(format!("material-override-{alias}.cap"));
        *stream = read_capture(&super::login_capture::bounded(&path, 8 * 1024 * 1024)?)?;
    }
    require_region_copy(&server, &viewers)?;
    let mut published = [Vec::new(), Vec::new()];
    for (output, records) in published.iter_mut().zip(viewers.iter()) {
        let locals = local_parts(records, &parts)?;
        for record in records {
            let packet = parse_packet(&record.bytes)?;
            if packet.id != GenericStreamingMessage::ID {
                continue;
            }
            output.push(Record {
                kind: record.kind,
                time_ms: record.time_ms,
                bytes: alias_material(&packet, &record.bytes, &locals)?,
            });
        }
    }
    if ferroan {
        let reference = [read_fixture("control.cap")?, read_fixture("limited.cap")?];
        return compare(&reference, &published);
    }
    if published[1].is_empty() && !published[0].is_empty() {
        let dir = run.join("material-overrides");
        fs::create_dir_all(&dir)?;
        for (name, stream) in VIEWERS.iter().zip(published) {
            fs::write(dir.join(format!("{name}.cap")), write_capture(&stream)?)?;
        }
        return Ok(format!(
            "material-override: Pin8 sent method 16757 to the PBR viewer only; fixture in {}",
            dir.display()
        ));
    }
    Err("Pin8 material recipients were not one PBR viewer and one viewer without it".into())
}

fn compare(reference: &[Vec<Record>; 2], live: &[Vec<Record>; 2]) -> Result<String> {
    let pin = material_datagrams(&reference[0])?;
    let got = material_datagrams(&live[0])?;
    let pin_set: BTreeSet<&[u8]> = pin.iter().map(Vec::as_slice).collect();
    let got_set: BTreeSet<&[u8]> = got.iter().map(Vec::as_slice).collect();
    if pin.is_empty() || got.len() != pin.len() || got_set != pin_set {
        return Err(format!(
            "control material notation differs from the Pin8 capture (Ferroan {}, Pin8 {})",
            got.len(),
            pin.len()
        )
        .into());
    }
    let limited_pin = material_datagrams(&reference[1])?;
    let limited_got = material_datagrams(&live[1])?;
    if !limited_pin.is_empty() || !limited_got.is_empty() {
        return Err(format!(
            "the viewer that did not request PBR received a material override (Ferroan {}, Pin8 {})",
            limited_got.len(),
            limited_pin.len()
        )
        .into());
    }
    let count = got.len();
    Ok(format!(
        "material-override: Ferroan sent {count} reliable unencoded method 16757 datagram{} to the PBR viewer",
        if count == 1 { "" } else { "s" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patched_blobs_prefix_each_face_with_its_index_and_length() {
        let blob = materials_blob(&[(0, "{'bc':[r1,r0,r0,r1]}"), (2, "{'ds':1}")]);
        assert_eq!(blob[0], 2);
        assert_eq!(&blob[1..4], &[0, 20, 0]);
        assert_eq!(&blob[4..24], b"{'bc':[r1,r0,r0,r1]}");
        assert_eq!(&blob[24..27], &[2, 8, 0]);
        assert_eq!(&blob[27..], b"{'ds':1}");
        assert_eq!(materials_blob(&[(0, "{'tree':1}")])[1..4], [0, 10, 0]);
        assert_eq!(materials_blob(&[(0, "{'probe':1}")])[1..4], [0, 11, 0]);
    }

    #[test]
    fn aliasing_changes_only_the_id_token_and_its_length_prefix() {
        let notation = b"{'id':i702,'te':[i0,i2],'od':[{'bc':[r1,r0,r0,r1]},{'ds':1}]}";
        let message = GenericStreamingMessage {
            method_data: MethodData { method: 16757 },
            data_block: DataBlock {
                data: notation.to_vec(),
            },
        };
        let mut datagram = Vec::new();
        ferroan_protocol::build_packet(&message, 4, true, false, &[], &mut datagram);
        let packet = parse_packet(&datagram).unwrap();
        assert_eq!(encoded(&packet, &message), datagram);
        let locals = HashMap::from([(702, 1)]);
        let aliased = alias_material(&packet, &datagram, &locals).unwrap();
        assert_ne!(aliased, datagram);
        let again = parse_packet(&aliased).unwrap();
        assert!(again.reliable());
        assert_eq!(again.flags & PacketFlags::ZEROCODED, 0);
        assert_eq!(again.sequence, 4);
        let body = again.decode::<GenericStreamingMessage>().unwrap();
        assert_eq!(
            body.data_block.data,
            b"{'id':i1,'te':[i0,i2],'od':[{'bc':[r1,r0,r0,r1]},{'ds':1}]}"
        );
        let mut changed = datagram.clone();
        changed[11] ^= 0x01;
        assert!(checked_message::<GenericStreamingMessage>(&packet, &changed).is_err());
    }

    #[test]
    fn the_pin8_fixture_is_one_reliable_override_for_the_pbr_viewer() {
        let control = read_fixture("control.cap").unwrap();
        let notes = material_datagrams(&control).unwrap();
        assert_eq!(
            notes,
            vec![b"{'id':i1,'te':[i0,i2],'od':[{'bc':[r1,r0,r0,r1]},{'ds':1}]}".to_vec()]
        );
        let packet = parse_packet(&control[0].bytes).unwrap();
        assert!(packet.reliable());
        assert!(!packet.resent());
        assert_eq!(packet.flags & PacketFlags::ZEROCODED, 0);
        assert_eq!(
            packet
                .decode::<GenericStreamingMessage>()
                .unwrap()
                .method_data
                .method,
            16757
        );
        let limited = read_fixture("limited.cap").unwrap();
        assert!(material_datagrams(&limited).unwrap().is_empty());
        let message = compare(&[control.clone(), limited.clone()], &[control, limited]).unwrap();
        assert!(message.contains("Ferroan sent 1 "));
    }

    #[test]
    fn a_second_fresh_override_fails_and_a_resend_does_not_count() {
        let control = read_fixture("control.cap").unwrap();
        let limited = read_fixture("limited.cap").unwrap();
        let reference = [control.clone(), limited.clone()];
        let mut doubled = control.clone();
        doubled.push(control[0].clone());
        let err = compare(&reference, &[doubled, limited.clone()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("Ferroan 2"));
        assert!(err.contains("Pin8 1"));

        let mut resent = control[0].clone();
        resent.bytes[0] |= PacketFlags::RESENT;
        let mut with_resend = control.clone();
        with_resend.push(resent);
        let message = compare(&reference, &[with_resend, limited]).unwrap();
        assert!(message.contains("Ferroan sent 1 "));
    }
}
