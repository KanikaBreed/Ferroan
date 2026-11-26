// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Join the two observers' receipts to the region socket capture before redaction.
//! Pin8 c401bceecd1684832ae8ba86a31b5ee01a4ded7b,
//! Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs, ProcessEntityUpdates.

use ferroan_bot::scenario::PRIM_CAPTURE_COUNT;
use ferroan_capture::{Kind, Record, read_capture, write_capture};
use ferroan_protocol::msgs::{
    AgentThrottle, ImprovedTerseObjectUpdate, LogoutRequest, ObjectUpdate, UseCircuitCode,
};
use ferroan_protocol::{IncomingPacket, Msg, PacketFlags, Writer, packet::build_raw, parse_packet};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt::Write;
use std::fs;
use std::path::Path;
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const VIEWERS: [&str; 2] = ["control", "limited"];

fn agent_alias(id: Uuid) -> Result<Uuid> {
    if id == crate::live::ESTATE_OWNER.parse::<Uuid>()? {
        Ok(Uuid::from_u128(100))
    } else if id == crate::live::CHAT_USER.parse::<Uuid>()? {
        Ok(Uuid::from_u128(101))
    } else {
        Err("unexpected avatar or owner".into())
    }
}

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

fn redact_update(
    packet: &IncomingPacket,
    original: &[u8],
    aliases: &BTreeMap<Uuid, Uuid>,
) -> Result<Vec<u8>> {
    if packet.id == ObjectUpdate::ID {
        let mut msg: ObjectUpdate = checked_message(packet, original)?;
        for part in &mut msg.object_data {
            if part.p_code == 47 {
                // Exclude the avatar's name, appearance and other personal fields.
                *part = ferroan_protocol::msgs::object_update::ObjectData {
                    full_id: agent_alias(part.full_id)?,
                    id: part.id,
                    p_code: part.p_code,
                    parent_id: part.parent_id,
                    ..Default::default()
                };
            } else {
                part.full_id = *aliases.get(&part.full_id).ok_or("unknown stored part")?;
                if !part.owner_id.is_nil() {
                    part.owner_id = agent_alias(part.owner_id)?;
                }
            }
        }
        Ok(encoded(packet, &msg))
    } else {
        let mut msg: ImprovedTerseObjectUpdate = checked_message(packet, original)?;
        for block in &mut msg.object_data {
            block.texture_entry.clear();
        }
        Ok(encoded(packet, &msg))
    }
}

/// Use the region's control messages for the endpoints, independently of whether
/// the viewers received the first or last update. The region capture has no addresses.
fn arrival_window(server: &[Record]) -> Result<std::ops::Range<usize>> {
    let control: Uuid = crate::live::ESTATE_OWNER.parse()?;
    let limited: Uuid = crate::live::CHAT_USER.parse()?;
    let mut start = None;
    for (i, record) in server
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == Kind::UdpIn)
    {
        let p = parse_packet(&record.bytes)?;
        if start.is_none()
            && p.id == UseCircuitCode::ID
            && p.decode::<UseCircuitCode>()?.circuit_code.id == control
        {
            start = Some(i);
        }
        if let Some(start) = start
            && p.id == LogoutRequest::ID
            && p.decode::<LogoutRequest>()?.agent_data.agent_id == limited
        {
            return Ok(start..i);
        }
    }
    Err(
        "capture lacks the control viewer's circuit open or the limited viewer's logout request"
            .into(),
    )
}

fn arrival_record(kind: Kind, packet: &IncomingPacket) -> bool {
    (kind == Kind::UdpIn && packet.id == AgentThrottle::ID)
        || (kind == Kind::UdpOut
            && matches!(packet.id, ObjectUpdate::ID | ImprovedTerseObjectUpdate::ID))
}

/// Require a receipt for every occurrence, including identical sends on two
/// circuits and reliable resends. Ignore teardown traffic outside the window.
fn match_window(server: &[Record], viewers: [Vec<Record>; 2]) -> Result<[Vec<Record>; 2]> {
    let window = arrival_window(server)?;
    let mut counts = HashMap::new();
    for (i, record) in server.iter().enumerate() {
        if arrival_record(record.kind, &parse_packet(&record.bytes)?) {
            let counts = counts.entry(record.bytes.as_slice()).or_insert([0usize; 2]);
            counts[usize::from(!window.contains(&i))] += 1;
        }
    }
    let mut matched = [Vec::new(), Vec::new()];
    for (stream, records) in matched.iter_mut().zip(viewers) {
        for record in records {
            if !arrival_record(record.kind, &parse_packet(&record.bytes)?) {
                return Err("unexpected viewer datagram".into());
            }
            match counts.get_mut(record.bytes.as_slice()) {
                Some([inside, 0]) if *inside > 0 => {
                    *inside -= 1;
                    stream.push(record);
                }
                Some([0, outside]) if *outside > 0 => *outside -= 1,
                Some([inside, outside]) if *inside > 0 && *outside > 0 => {
                    return Err("identical datagram on both sides of the capture boundary".into());
                }
                _ => return Err("viewer datagram has no matching region occurrence".into()),
            }
        }
    }
    if counts.values().any(|c| c[0] != 0) {
        return Err(
            "missing viewer receipt for a region datagram inside the arrival window".into(),
        );
    }
    Ok(matched)
}

pub(super) fn extract(run: &Path, home_log: &Path, ferroan: bool) -> Result<String> {
    let map: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/prim-updates/map.json"))?;
    let mut aliases = BTreeMap::new();
    for part in map["parts"].as_array().ok_or("missing prim map")? {
        let uuid = |field: &str| -> Result<Uuid> {
            Ok(part[field].as_str().ok_or("missing mapped uuid")?.parse()?)
        };
        aliases.insert(uuid("store_uuid")?, uuid("capture_uuid")?);
    }
    let bytes = super::login_capture::bounded(&run.join("capture-udp.cap"), 64 * 1024 * 1024)?;
    let server = read_capture(&bytes)?;
    let mut viewers = [Vec::new(), Vec::new()];
    for (stream, alias) in viewers.iter_mut().zip(VIEWERS) {
        let path = home_log.with_file_name(format!("prim-arrival-{alias}.cap"));
        *stream = read_capture(&super::login_capture::bounded(&path, 2 * 1024 * 1024)?)?;
    }
    let mut circuits = match_window(&server, viewers)?;
    for record in circuits.iter_mut().flatten() {
        let packet = parse_packet(&record.bytes)?;
        record.bytes = if record.kind == Kind::UdpIn && packet.id == AgentThrottle::ID {
            let mut msg: AgentThrottle = checked_message(&packet, &record.bytes)?;
            msg.agent_data.agent_id = agent_alias(msg.agent_data.agent_id)?;
            msg.agent_data.session_id = Uuid::nil();
            msg.agent_data.circuit_code = 0;
            encoded(&packet, &msg)
        } else if record.kind == Kind::UdpOut {
            redact_update(&packet, &record.bytes, &aliases)?
        } else {
            return Err("unexpected viewer datagram".into());
        };
    }
    if ferroan {
        let reference = [
            read_capture(include_bytes!("../../fixtures/prim-arrival/control.cap"))?,
            read_capture(include_bytes!("../../fixtures/prim-arrival/limited.cap"))?,
        ];
        return compare_arrival(&reference, &circuits);
    }
    let report = replay(&circuits)?;
    let dir = run.join("prim-arrival");
    fs::create_dir_all(&dir)?;
    for (name, stream) in VIEWERS.iter().zip(circuits) {
        fs::write(dir.join(format!("{name}.cap")), write_capture(&stream)?)?;
    }
    fs::write(dir.join("arrival.txt"), report)?;
    Ok(format!(
        "prim-arrival: two Pin8 circuits, matching part and parent IDs, Task delayed and resumed; fixture in {}",
        dir.display()
    ))
}

/// Compare first deliveries and circuit-relative times. Absolute local IDs,
/// the order of unrelated parts and terse grouping can vary at every start.
#[derive(Default)]
struct Arrival {
    ids: BTreeMap<Uuid, u32>,
    batches: Vec<usize>,
    constrained: Vec<usize>,
    first: Option<u64>,
    restored: Option<u64>,
    last: u64,
}

fn arrival(records: &[Record]) -> Result<Arrival> {
    let mut result = Arrival::default();
    let mut constrained = false;
    for record in records {
        let packet = parse_packet(&record.bytes)?;
        if packet.id == AgentThrottle::ID {
            if constrained {
                result.restored = Some(record.time_ms);
            }
            constrained = packet
                .decode::<AgentThrottle>()?
                .throttle
                .throttles
                .get(16..20)
                == Some(8000f32.to_le_bytes().as_slice());
        } else if packet.id == ObjectUpdate::ID {
            let mut firsts = 0;
            for block in packet.decode::<ObjectUpdate>()?.object_data {
                if block.p_code != 47 && result.ids.insert(block.full_id, block.id).is_none() {
                    result.first.get_or_insert(record.time_ms);
                    result.last = record.time_ms;
                    firsts += 1;
                }
            }
            if firsts > 0 {
                result.batches.push(firsts);
                if constrained {
                    result.constrained.push(firsts);
                }
            }
        }
    }
    let base = *result.ids.values().min().ok_or("no first prims")?;
    for id in result.ids.values_mut() {
        *id -= base;
    }
    Ok(result)
}

fn compare_arrival(reference: &[Vec<Record>; 2], live: &[Vec<Record>; 2]) -> Result<String> {
    // Both streams must independently satisfy the link, reliability, throttle
    // and moving peer checks before comparing their stable properties.
    replay(reference)?;
    replay(live)?;
    let mut report = Vec::new();
    for (i, name) in VIEWERS.iter().enumerate() {
        let pin = arrival(&reference[i])?;
        let got = arrival(&live[i])?;
        if got.ids != pin.ids {
            return Err(format!("{name}: relative part IDs differ from Pin8").into());
        }
        if i == 0 {
            if got.batches != pin.batches {
                return Err(format!(
                    "control: first batches {:?}, Pin8 {:?}",
                    got.batches, pin.batches
                )
                .into());
            }
            report.push(format!("control batches {:?}", got.batches));
        } else {
            if got.constrained.is_empty()
                || !got
                    .constrained
                    .iter()
                    .chain(&pin.constrained)
                    .all(|n| *n == 1)
            {
                return Err(format!(
                    "limited: unexpected constrained batches {:?}",
                    got.constrained
                )
                .into());
            }
            let timing = |a: &Arrival| -> Result<(u64, u64)> {
                let restore = a.restored.ok_or("missing Task restore")?;
                Ok((
                    restore
                        .checked_sub(a.first.ok_or("no initial prim")?)
                        .ok_or("restore before first prim")?,
                    a.last
                        .checked_sub(restore)
                        .ok_or("no parts outstanding at the restore")?,
                ))
            };
            let (pin_delay, pin_resume) = timing(&pin)?;
            let (delay, resume) = timing(&got)?;
            // Allow two 20 ms queue-empty intervals of scheduler variance.
            if resume > pin_resume + 40 {
                return Err(format!(
                    "limited: completion {resume} ms after restore, Pin8 {pin_resume} ms"
                )
                .into());
            }
            report.push(format!("limited batches of one, constrained {delay} ms (Pin8 {pin_delay}), complete {resume} ms after restore (Pin8 {pin_resume})"));
        }
    }
    Ok(format!(
        "prim-arrival: matching local-ID offsets, parents and peer movement; {}",
        report.join("; ")
    ))
}

fn replay(circuits: &[Vec<Record>; 2]) -> Result<String> {
    let mut text = String::new();
    let mut scene = BTreeMap::new();
    let mut delivery_spans = [None; 2];
    let mut regular = ferroan_bot::throttle::initial_throttle_bytes();
    regular[16..20].copy_from_slice(&512_000f32.to_le_bytes());
    for (i, records) in circuits.iter().enumerate() {
        writeln!(text, "[{}]", VIEWERS[i])?;
        let mut ids = BTreeMap::new();
        let mut avatars = BTreeMap::new();
        let mut throttles = 0;
        let mut first_prim = None;
        let mut first_peer_xy = None;
        let mut peer_moved = false;
        let mut referenced = BTreeSet::new();
        for record in records {
            let packet = parse_packet(&record.bytes)?;
            write!(
                text,
                "{}ms seq={} flags={:02x} len={} sha256={:x}",
                record.time_ms,
                packet.sequence,
                packet.flags,
                record.bytes.len(),
                Sha256::digest(&record.bytes)
            )?;
            if packet.id == AgentThrottle::ID && record.kind == Kind::UdpIn {
                let msg = packet.decode::<AgentThrottle>()?;
                let mut expected = regular.clone();
                let slow = i == 1 && throttles == 0;
                if slow {
                    expected[16..20].copy_from_slice(&8000f32.to_le_bytes());
                }
                if !msg.agent_data.session_id.is_nil() || msg.agent_data.circuit_code != 0 {
                    return Err("unredacted viewer circuit in fixture".into());
                }
                if msg.throttle.throttles != expected
                    || msg.agent_data.agent_id != Uuid::from_u128(100 + i as u128)
                {
                    return Err("incorrect per-viewer throttle group".into());
                }
                if i == 1
                    && throttles == 1
                    && !(first_prim.is_some_and(|at| record.time_ms.saturating_sub(at) >= 250)
                        && ids.len() < PRIM_CAPTURE_COUNT)
                {
                    return Err(
                        "expected outstanding prims during the constrained Task interval".into(),
                    );
                }
                writeln!(text, " Task={}bps", if slow { 8000 } else { 512000 })?;
                throttles += 1;
            } else if packet.id == ObjectUpdate::ID && record.kind == Kind::UdpOut {
                if !packet.reliable() || packet.flags & PacketFlags::ZEROCODED == 0 {
                    return Err("unreliable or unencoded full update".into());
                }
                write!(text, " full")?;
                for part in packet.decode::<ObjectUpdate>()?.object_data {
                    let n = part.full_id.as_u128();
                    if part.p_code == 47 {
                        avatars.insert(part.id, n);
                        write!(text, " avatar{n}@{}", part.id)?;
                    } else {
                        if part.id == 0 || !(1..=PRIM_CAPTURE_COUNT as u128).contains(&n) {
                            return Err("unexpected prim identity".into());
                        }
                        if throttles == 0 || (n == 2 && ids.get(&1) != Some(&part.parent_id)) {
                            return Err("prim delivered before its throttle or linked root".into());
                        }
                        let identity = (part.id, part.parent_id);
                        if scene
                            .insert(n, identity)
                            .is_some_and(|previous| previous != identity)
                        {
                            return Err("part or parent local ID changed between deliveries".into());
                        }
                        let start = *first_prim.get_or_insert(record.time_ms);
                        let repeat = ids.insert(n, part.id).is_some();
                        if !repeat && ids.len() == PRIM_CAPTURE_COUNT {
                            delivery_spans[i] = record.time_ms.checked_sub(start);
                        }
                        write!(
                            text,
                            " p{n}@{}->{}:{}",
                            part.id,
                            part.parent_id,
                            if repeat { "repeat" } else { "first" }
                        )?;
                    }
                }
                writeln!(text)?;
            } else if packet.id == ImprovedTerseObjectUpdate::ID && record.kind == Kind::UdpOut {
                write!(text, " terse")?;
                for (id, avatar, position) in ferroan_bot::terse_in(&packet) {
                    if avatar {
                        referenced.insert(id);
                        if avatars.get(&id) == Some(&(101 - i as u128)) && position.is_finite() {
                            let xy = position.truncate();
                            let first = first_peer_xy.get_or_insert(xy);
                            peer_moved |= xy.distance_squared(*first) > 0.01;
                        }
                        write!(text, " avatar@{id}")?;
                    }
                }
                writeln!(text)?;
            } else {
                return Err("unexpected capture record".into());
            }
        }
        if ids.len() != PRIM_CAPTURE_COUNT
            || throttles != i + 1
            || !referenced.iter().all(|id| avatars.contains_key(id))
            || (i == 1 && !peer_moved)
        {
            return Err("incomplete prim arrival, throttle or avatar history".into());
        }
    }
    match delivery_spans {
        [Some(normal), Some(limited)] if normal < limited => {}
        _ => {
            return Err(
                "limited viewer did not take longer to receive its first eight parts".into(),
            );
        }
    }
    let root = scene.get(&1).ok_or("missing root")?.0;
    let mut locals = BTreeSet::new();
    for (part, (local, parent)) in &scene {
        if !locals.insert(local) || *parent != if *part == 2 { root } else { 0 } {
            return Err("incorrect linkset or reused local ID".into());
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: &str = include_str!("../../fixtures/prim-arrival/arrival.txt");

    fn circuits() -> [Vec<Record>; 2] {
        [
            read_capture(include_bytes!("../../fixtures/prim-arrival/control.cap")).unwrap(),
            read_capture(include_bytes!("../../fixtures/prim-arrival/limited.cap")).unwrap(),
        ]
    }

    #[test]
    fn pin8_prim_arrival_replays_each_viewers_order_and_packet_boundaries() {
        assert_eq!(replay(&circuits()).unwrap(), EXPECTED);
    }

    #[test]
    fn live_comparison_accepts_unrelated_part_order_but_detects_late_resumption() {
        let reference = circuits();
        let mut changed = reference.clone();
        for record in &mut changed[0] {
            let packet = parse_packet(&record.bytes).unwrap();
            if packet.id == ObjectUpdate::ID {
                let mut msg = packet.decode::<ObjectUpdate>().unwrap();
                if msg.object_data.len() >= 6 {
                    msg.object_data.swap(2, 3);
                    record.bytes = encoded(&packet, &msg);
                }
            }
        }
        compare_arrival(&reference, &changed).unwrap();
        let restore = arrival(&reference[1]).unwrap().restored.unwrap();
        for record in &mut changed[1] {
            if record.time_ms > restore {
                record.time_ms += 200;
            }
        }
        assert!(
            compare_arrival(&reference, &changed)
                .unwrap_err()
                .to_string()
                .contains("after restore")
        );
    }

    #[test]
    fn stationary_peer_terse_records_do_not_prove_movement() {
        let mut streams = circuits();
        for record in &mut streams[1] {
            let packet = parse_packet(&record.bytes).unwrap();
            if packet.id != ImprovedTerseObjectUpdate::ID {
                continue;
            }
            let mut msg = packet.decode::<ImprovedTerseObjectUpdate>().unwrap();
            for block in &mut msg.object_data {
                block.data[22..30].fill(0);
            }
            record.bytes = encoded(&packet, &msg);
        }
        assert!(replay(&streams).is_err());
    }

    #[test]
    fn changing_one_crc_byte_changes_the_replay() {
        let mut circuits = circuits();
        let record = circuits[0]
            .iter_mut()
            .find(|record| {
                let p = parse_packet(&record.bytes).unwrap();
                p.id == ObjectUpdate::ID && p.sequence == 32
            })
            .unwrap();
        let packet = parse_packet(&record.bytes).unwrap();
        let mut update = packet.decode::<ObjectUpdate>().unwrap();
        update.object_data[0].crc ^= 1;
        let changed = encoded(&packet, &update);
        assert_eq!(record.bytes.len(), changed.len());
        assert_eq!(
            record
                .bytes
                .iter()
                .zip(&changed)
                .filter(|(a, b)| a != b)
                .count(),
            1
        );
        record.bytes = changed;
        assert_ne!(replay(&circuits).unwrap(), EXPECTED);
    }

    #[test]
    fn a_decodable_packet_must_reproduce_its_original_bytes() {
        let mut raw = circuits()[0][0].bytes.clone();
        // The decoder accepts an extra header byte. The fixture encoder omits it.
        raw[5] = 1;
        raw.insert(6, 0x42);
        let packet = parse_packet(&raw).unwrap();
        assert!(packet.decode::<AgentThrottle>().is_ok());
        assert!(checked_message::<AgentThrottle>(&packet, &raw).is_err());
    }

    #[test]
    fn every_server_update_up_to_logout_needs_its_own_receipt() {
        fn input<M: Msg>(msg: &M) -> Record {
            let mut bytes = Vec::new();
            ferroan_protocol::build_packet(msg, 1, true, false, &[], &mut bytes);
            Record {
                kind: Kind::UdpIn,
                time_ms: 0,
                bytes,
            }
        }
        let mut open = UseCircuitCode::default();
        open.circuit_code.id = crate::live::ESTATE_OWNER.parse().unwrap();
        let mut close = LogoutRequest::default();
        close.agent_data.agent_id = crate::live::CHAT_USER.parse().unwrap();
        let packets = circuits();
        let update = packets[0]
            .iter()
            .find(|r| r.kind == Kind::UdpOut)
            .unwrap()
            .clone();
        let mut resend = update.clone();
        resend.bytes[0] |= PacketFlags::RESENT;
        let mut teardown = update.clone();
        teardown.bytes[1..5].copy_from_slice(&900u32.to_be_bytes());
        let server = [
            input(&open),
            update.clone(),
            resend.clone(),
            resend.clone(),
            input(&close),
            teardown.clone(),
        ];
        assert!(match_window(&server, [vec![update.clone(), resend.clone()], vec![]]).is_err());
        assert!(match_window(&server, [vec![resend.clone(), resend.clone()], vec![]]).is_err());
        let received = vec![update.clone(), resend.clone(), resend];
        let mut including_teardown = received.clone();
        including_teardown.push(teardown);
        assert_eq!(
            match_window(&server, [including_teardown, vec![]]).unwrap()[0],
            received
        );
    }

    #[test]
    fn moving_one_prim_over_the_datagram_boundary_changes_the_replay() {
        let reference = circuits();
        let mut circuits = reference.clone();
        let mut batches = circuits[0].iter_mut().filter_map(|record| {
            let packet = parse_packet(&record.bytes).ok()?;
            if packet.id != ObjectUpdate::ID {
                return None;
            }
            let update = packet.decode::<ObjectUpdate>().ok()?;
            update
                .object_data
                .iter()
                .any(|p| p.p_code != 47)
                .then_some((record, packet, update))
        });
        let (left, l_packet, mut l_update) = batches.next().unwrap();
        let (right, r_packet, mut r_update) = batches.next().unwrap();
        // Keep the flattened list of blocks, their order and all identities intact.
        r_update
            .object_data
            .insert(0, l_update.object_data.pop().unwrap());
        left.bytes = encoded(&l_packet, &l_update);
        right.bytes = encoded(&r_packet, &r_update);
        assert!(
            compare_arrival(&reference, &circuits)
                .unwrap_err()
                .to_string()
                .contains("first batches")
        );
        assert_ne!(replay(&circuits).unwrap(), EXPECTED);
    }
}
