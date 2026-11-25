// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Join the two observers' receipts to the Pin8 server capture before redaction.
//! Pin8 c401bceecd1684832ae8ba86a31b5ee01a4ded7b,
//! Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs, ProcessEntityUpdates.

use ferroan_bot::scenario::PRIM_CAPTURE_COUNT;
use ferroan_capture::{Kind, Record, read_capture, write_capture};
use ferroan_protocol::msgs::{AgentThrottle, ImprovedTerseObjectUpdate, ObjectUpdate};
use ferroan_protocol::{IncomingPacket, Msg, PacketFlags, Writer, packet::build_raw, parse_packet};
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

fn encoded<M: Msg>(packet: &IncomingPacket, msg: M) -> Vec<u8> {
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

fn redact_update(packet: &IncomingPacket, aliases: &BTreeMap<Uuid, Uuid>) -> Result<Vec<u8>> {
    if packet.id == ObjectUpdate::ID {
        let mut msg = packet.decode::<ObjectUpdate>()?;
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
        Ok(encoded(packet, msg))
    } else {
        let mut msg = packet.decode::<ImprovedTerseObjectUpdate>()?;
        for block in &mut msg.object_data {
            block.texture_entry.clear();
        }
        Ok(encoded(packet, msg))
    }
}

pub(super) fn extract(run: &Path, home_log: &Path) -> Result<String> {
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
    // Two circuits can receive exactly the same sequence, flags and body.
    // Check multiplicity against the server. Each socket supplies its own order and clock.
    let server = read_capture(&bytes)?;
    let mut sends = [HashMap::new(), HashMap::new()];
    for record in &server {
        *sends[usize::from(record.kind == Kind::UdpOut)]
            .entry(record.bytes.as_slice())
            .or_insert(0usize) += 1;
    }
    let mut circuits = [Vec::new(), Vec::new()];
    for (i, alias) in VIEWERS.iter().enumerate() {
        let path = home_log.with_file_name(format!("prim-arrival-{alias}.cap"));
        let captured = super::login_capture::bounded(&path, 2 * 1024 * 1024)?;
        for mut record in read_capture(&captured)? {
            let count = sends[usize::from(record.kind == Kind::UdpOut)]
                .get_mut(record.bytes.as_slice())
                .ok_or("viewer datagram missing from Pin8 capture")?;
            *count = count
                .checked_sub(1)
                .ok_or("too few matching datagrams in Pin8 capture")?;
            let packet = parse_packet(&record.bytes)?;
            record.bytes = if record.kind == Kind::UdpIn && packet.id == AgentThrottle::ID {
                let mut msg = packet.decode::<AgentThrottle>()?;
                msg.agent_data.agent_id = agent_alias(msg.agent_data.agent_id)?;
                msg.agent_data.session_id = Uuid::nil();
                msg.agent_data.circuit_code = 0;
                encoded(&packet, msg)
            } else if record.kind == Kind::UdpOut {
                redact_update(&packet, &aliases)?
            } else {
                return Err("unexpected viewer datagram".into());
            };
            circuits[i].push(record);
        }
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
        let mut peer_terse = false;
        let mut referenced = BTreeSet::new();
        for record in records {
            let packet = parse_packet(&record.bytes)?;
            write!(
                text,
                "{}ms seq={} flags={:02x}",
                record.time_ms, packet.sequence, packet.flags
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
                for (id, avatar, _) in ferroan_bot::terse_in(&packet) {
                    if avatar {
                        referenced.insert(id);
                        peer_terse |= avatars.get(&id) == Some(&(101 - i as u128));
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
            || (i == 1 && !peer_terse)
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
    fn moving_one_prim_over_the_datagram_boundary_changes_the_replay() {
        let mut circuits = circuits();
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
        left.bytes = encoded(&l_packet, l_update);
        right.bytes = encoded(&r_packet, r_update);
        assert_ne!(replay(&circuits).unwrap(), EXPECTED);
    }
}
