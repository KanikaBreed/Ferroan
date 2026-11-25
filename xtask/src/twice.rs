// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Compare two standalone `login` recordings at the current pin.
//!
//! Counts are exact per message type and direction, except the timer and
//! acknowledgement types named in P0.10n5, which are reported. Within a login,
//! every repeated reliable sequence in the same direction must carry the resent
//! flag, the original message ID and the same decoded body. It counts once
//! with the original (P0.10n6). Across logins the sequence and resent flag are
//! wildcards. The order check uses the first appearance of the handshake and
//! logout messages. See [`WILDCARDS`] for the other cross-login allowances.

// The live runner calls this on Linux. Windows and macOS still compile the
// binary, and the unit tests cover the comparator there.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use ferroan_capture::{Kind, Record, read_capture};
use ferroan_protocol::msgs::{
    AgentMovementComplete, AvatarAnimation, ObjectUpdate, UseCircuitCode,
};
use ferroan_protocol::{Msg, Writer, message_info, parse_packet, scrub_body};

/// Timer-driven and acknowledgement traffic. A healthy login does not keep a
/// stable count of these. Everything else is an exact count.
const REPORTED: &[&str] = &[
    "StartPingCheck",
    "CompletePingCheck",
    "CoarseLocationUpdate",
    "SimStats",
    "SimulatorViewerTimeMessage",
    "ImprovedTerseObjectUpdate",
    "PacketAck",
    "KillObject",
];

const ORDER: &[&str] = &[
    "UseCircuitCode",
    "RegionHandshake",
    "RegionHandshakeReply",
    "CompleteAgentMovement",
    "AgentMovementComplete",
    "LogoutRequest",
    "LogoutReply",
];

const WILDCARDS: &str = "\
wildcards across logins: UDP sequence (one counter per direction) and resent flag \
(retransmission timer). Within one login a repeated reliable sequence in the same \
direction must have the resent flag, original message ID and same decoded body. \
Other wildcards: appended acknowledgements (which packets have arrived); session, agent, \
circuit, name and IP (scrubbed; each login is a new session); AgentMovementComplete \
timestamp (simulator clock); AgentMovementComplete position Z and the ObjectUpdate \
packed position Z (terrain height at the spawn point; not bit-stable); ObjectUpdate \
local id and CRC (the region allocates a new avatar); AvatarAnimation sequence \
(animation counter)";

#[derive(Default)]
struct Messages {
    packets: Vec<Seen>,
    resends: usize,
}

type SeenMap = BTreeMap<(&'static str, &'static str), Messages>;

/// Split one region capture into the two `login` circuits and compare them.
pub(crate) fn compare_capture(bytes: &[u8]) -> Result<String, String> {
    let records = read_capture(bytes).map_err(|error| error.to_string())?;
    let (left, right) = split_logins(&records)?;
    compare_sessions(&left, &right)
}

fn split_logins(records: &[Record]) -> Result<(Vec<Record>, Vec<Record>), String> {
    let mut opens = Vec::new();
    for (index, record) in records.iter().enumerate() {
        if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
            continue;
        }
        let packet = parse_packet(&record.bytes)
            .map_err(|error| format!("UDP record {index} does not parse: {error}"))?;
        if record.kind == Kind::UdpIn && packet.id == UseCircuitCode::ID && !packet.resent() {
            opens.push(index);
        }
    }
    let found = opens.len();
    if found != 2 {
        return Err(format!(
            "the capture is not two logins: found {found} UseCircuitCode"
        ));
    }
    let first = opens[0];
    let second = opens[1];
    let preamble = records[..first]
        .iter()
        .any(|record| matches!(record.kind, Kind::UdpIn | Kind::UdpOut));
    if preamble {
        return Err("the capture has UDP before the first UseCircuitCode".to_string());
    }
    Ok((records[first..second].to_vec(), records[second..].to_vec()))
}

fn compare_sessions(left: &[Record], right: &[Record]) -> Result<String, String> {
    let left_order = ordered_names(left)?;
    let right_order = ordered_names(right)?;
    let left = view(left)?;
    let right = view(right)?;
    let mut errors = Vec::new();
    for name in ORDER {
        if !left_order.contains(name) {
            errors.push(format!("order: {name} is missing from the first login"));
        }
        if !right_order.contains(name) {
            errors.push(format!("order: {name} is missing from the second login"));
        }
    }
    let left_canonical = canonical_order(&left_order);
    let right_canonical = canonical_order(&right_order);
    if left_canonical != right_canonical {
        let left_text = left_order.join(", ");
        let right_text = right_order.join(", ");
        errors.push(format!(
            "order: first appearance differs: [{left_text}] then [{right_text}]"
        ));
    }

    let mut keys = BTreeSet::new();
    keys.extend(left.keys().copied());
    keys.extend(right.keys().copied());
    let mut lines = Vec::new();
    lines.push("record-twice: two standalone login recordings".to_string());
    lines.push(WILDCARDS.to_string());
    let order_text = left_order.join(", ");
    let right_text = right_order.join(", ");
    lines.push(format!("order: {order_text}"));
    if left_order != right_order {
        lines.push(format!("order other: {right_text}"));
    }
    lines.push(
        "order note: RegionHandshakeReply and AgentMovementComplete may swap; the viewer answers the handshake while the simulator sends arrival"
            .to_string(),
    );
    for (direction, name) in keys {
        let empty = Messages::default();
        let left = left.get(&(direction, name)).unwrap_or(&empty);
        let right = right.get(&(direction, name)).unwrap_or(&empty);
        let left_count = left.packets.len();
        let right_count = right.packets.len();
        if reported(name) {
            let reason = reported_reason(name);
            lines.push(format!(
                "reported {direction} {name}: {left_count} and {right_count} ({reason})"
            ));
            continue;
        }
        let left_resends = left.resends;
        let right_resends = right.resends;
        if left_count != right_count {
            errors.push(format!(
                "count {direction} {name}: {left_count} and {right_count} (resends {left_resends} and {right_resends})"
            ));
            continue;
        }
        lines.push(format!(
            "exact {direction} {name}: {left_count} (resends {left_resends} and {right_resends})"
        ));
        for (index, (left_packet, right_packet)) in
            left.packets.iter().zip(&right.packets).enumerate()
        {
            let shown = index + 1;
            if left_packet.reliable != right_packet.reliable {
                errors.push(format!(
                    "value {direction} {name} packet {shown}: reliable flag differs"
                ));
            }
            if left_packet.body != right_packet.body {
                let offset = left_packet
                    .body
                    .iter()
                    .zip(right_packet.body.iter())
                    .position(|(left_byte, right_byte)| left_byte != right_byte)
                    .unwrap_or(left_packet.body.len().min(right_packet.body.len()));
                let left_len = left_packet.body.len();
                let right_len = right_packet.body.len();
                errors.push(format!(
                    "value {direction} {name} packet {shown}: body differs at offset {offset} (lengths {left_len} and {right_len})"
                ));
            }
        }
    }
    if !errors.is_empty() {
        let kept = errors.len().min(24);
        let mut text = errors[..kept].join("\n");
        if errors.len() > kept {
            let more = errors.len() - kept;
            text.push_str(&format!("\nand {more} more"));
        }
        return Err(text);
    }
    lines.push("values: matched".to_string());
    Ok(lines.join("\n"))
}

fn reported(name: &str) -> bool {
    REPORTED.contains(&name)
}

fn reported_reason(name: &str) -> &'static str {
    match name {
        "StartPingCheck" | "CompletePingCheck" => "timer-driven ping; the count follows the clock",
        "CoarseLocationUpdate" => "timer-driven location broadcast; the count follows the clock",
        "SimStats" => "timer-driven stats; the count follows the clock",
        "SimulatorViewerTimeMessage" => "timer-driven sun time; the count follows the clock",
        "ImprovedTerseObjectUpdate" => "timer-driven terse updates; the count follows the clock",
        "PacketAck" => "acknowledgements; the count follows which packets arrived",
        "KillObject" => {
            "the scene deletes the avatar after logout; the datagram can fall in the gap before the next login"
        }
        _ => "reported by the P0.10n5 rule",
    }
}

struct Seen {
    reliable: bool,
    body: Vec<u8>,
}

fn view(records: &[Record]) -> Result<SeenMap, String> {
    let mut out: SeenMap = BTreeMap::new();
    let mut originals = BTreeMap::new();
    for (index, record) in records.iter().enumerate() {
        if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
            continue;
        }
        let packet = parse_packet(&record.bytes)
            .map_err(|error| format!("UDP record {index} does not parse: {error}"))?;
        let info = message_info(packet.id)
            .ok_or_else(|| format!("UDP record {index} is not a known message"))?;
        let direction = if record.kind == Kind::UdpIn {
            "in"
        } else {
            "out"
        };
        let messages = out.entry((direction, info.name)).or_default();
        messages.resends += usize::from(packet.resent());
        if packet.reliable()
            && let Some(previous) =
                originals.insert((direction, packet.sequence), record.bytes.as_slice())
        {
            // Compare within this circuit before identity folding or cross-login
            // wildcards. Appended ACKs can change on retransmission.
            if !packet.resent() {
                return Err(format!(
                    "UDP record {index}: {direction} {} sequence {}: duplicate lacks the resent flag",
                    info.name, packet.sequence
                ));
            }
            let original = parse_packet(previous).map_err(|error| error.to_string())?;
            if packet.id != original.id || packet.body != original.body {
                return Err(format!(
                    "UDP record {index}: {direction} {} sequence {}: resent message differs from the original",
                    info.name, packet.sequence
                ));
            }
            continue;
        }
        let scrubbed = scrub_body(packet.id, &packet.body).map_err(|error| {
            format!("UDP record {index} ({}) does not scrub: {error}", info.name)
        })?;
        let body = normalize(info.name, &scrubbed)?;
        messages.packets.push(Seen {
            reliable: packet.reliable(),
            body,
        });
    }
    Ok(out)
}

/// `RegionHandshakeReply` and `AgentMovementComplete` are produced on different
/// tasks. When they sit next to each other in the named-message order, either
/// one may be first. Any other difference still fails.
fn canonical_order<'a>(names: &[&'a str]) -> Vec<&'a str> {
    let mut out = names.to_vec();
    let Some(reply) = out.iter().position(|name| *name == "RegionHandshakeReply") else {
        return out;
    };
    let Some(arrival) = out.iter().position(|name| *name == "AgentMovementComplete") else {
        return out;
    };
    if reply.abs_diff(arrival) == 1 && arrival < reply {
        out.swap(arrival, reply);
    }
    out
}

fn ordered_names(records: &[Record]) -> Result<Vec<&'static str>, String> {
    let mut seen = Vec::new();
    for (index, record) in records.iter().enumerate() {
        if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
            continue;
        }
        let packet = parse_packet(&record.bytes)
            .map_err(|error| format!("UDP record {index} does not parse: {error}"))?;
        let info = message_info(packet.id)
            .ok_or_else(|| format!("UDP record {index} is not a known message"))?;
        if ORDER.contains(&info.name) && !seen.contains(&info.name) {
            seen.push(info.name);
        }
    }
    Ok(seen)
}

fn normalize(name: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    match name {
        "AgentMovementComplete" => {
            let mut message = AgentMovementComplete::decode(body)
                .map_err(|error| format!("could not normalize AgentMovementComplete: {error}"))?;
            message.data.timestamp = 0;
            message.data.position.z = 0.0;
            Ok(encoded(&message))
        }
        "ObjectUpdate" => {
            let mut message = ObjectUpdate::decode(body)
                .map_err(|error| format!("could not normalize ObjectUpdate: {error}"))?;
            for object in &mut message.object_data {
                object.id = 0;
                object.crc = 0;
                // 76-byte avatar blob: collision plane, then position at byte 16.
                // Only Z follows the terrain. OpenMetaverse ObjectManager case 76.
                if object.object_data.len() == 76 {
                    object.object_data[24..28].fill(0);
                }
            }
            Ok(encoded(&message))
        }
        "AvatarAnimation" => {
            let mut message = AvatarAnimation::decode(body)
                .map_err(|error| format!("could not normalize AvatarAnimation: {error}"))?;
            for animation in &mut message.animation_list {
                animation.anim_sequence_id = 0;
            }
            Ok(encoded(&message))
        }
        _ => Ok(body.to_vec()),
    }
}

fn encoded<M: Msg>(message: &M) -> Vec<u8> {
    let mut writer = Writer::new();
    message.encode_body(&mut writer);
    writer.buf
}

#[cfg(test)]
mod tests {
    use ferroan_capture::{Kind, Record, write_capture};
    use ferroan_protocol::msgs::avatar_animation::AnimationList;
    use ferroan_protocol::msgs::object_update::ObjectData;
    use ferroan_protocol::msgs::{
        AgentMovementComplete, AvatarAnimation, CompleteAgentMovement, KillObject, LayerData,
        LogoutReply, LogoutRequest, ObjectUpdate, PacketAck, RegionHandshake, RegionHandshakeReply,
        StartPingCheck, UseCircuitCode,
    };
    use ferroan_protocol::{Msg, build_packet, parse_packet};
    use uuid::Uuid;

    use super::compare_capture;

    fn packet<M: Msg>(
        kind: Kind,
        sequence: u32,
        reliable: bool,
        resent: bool,
        acks: &[u32],
        message: &M,
    ) -> Record {
        let mut bytes = Vec::new();
        build_packet(message, sequence, reliable, resent, acks, &mut bytes);
        Record {
            kind,
            time_ms: u64::from(sequence),
            bytes,
        }
    }

    fn session(seed: u32) -> Vec<Record> {
        let mut circuit = UseCircuitCode::default();
        circuit.circuit_code.code = 1_000 + seed;
        circuit.circuit_code.session_id = Uuid::from_u128(10 + u128::from(seed));
        circuit.circuit_code.id = Uuid::from_u128(20 + u128::from(seed));

        let mut movement = AgentMovementComplete::default();
        movement.data.timestamp = 5_000 + seed;
        movement.data.position.x = 128.0;
        movement.data.position.y = 128.0;
        movement.data.position.z = 25.0 + seed as f32;

        let mut packed = vec![0u8; 76];
        packed[16..20].copy_from_slice(&128.0f32.to_le_bytes());
        packed[20..24].copy_from_slice(&128.0f32.to_le_bytes());
        packed[24..28].copy_from_slice(&(25.0 + seed as f32).to_le_bytes());
        let mut object = ObjectUpdate::default();
        object.object_data.push(ObjectData {
            id: 10 + seed,
            crc: 80 + seed,
            full_id: Uuid::from_u128(0xabc),
            object_data: packed,
            ..ObjectData::default()
        });

        let mut animation = AvatarAnimation::default();
        animation.animation_list.push(AnimationList {
            anim_id: Uuid::from_u128(0xdef),
            anim_sequence_id: i32::try_from(seed).unwrap_or(0),
        });

        let mut layer = LayerData::default();
        layer.layer_data.data = b"terrain-bytes".to_vec();

        let mut ping = StartPingCheck::default();
        ping.ping_id.ping_id = u8::try_from(seed).unwrap_or(0);

        let base = seed.saturating_mul(10);
        vec![
            packet(Kind::UdpIn, base + 1, true, false, &[], &circuit),
            packet(
                Kind::UdpIn,
                base + 2,
                true,
                false,
                &[],
                &CompleteAgentMovement::default(),
            ),
            packet(
                Kind::UdpOut,
                base + 1,
                true,
                seed == 2,
                &[seed],
                &RegionHandshake::default(),
            ),
            packet(
                Kind::UdpIn,
                base + 3,
                true,
                false,
                &[],
                &RegionHandshakeReply::default(),
            ),
            packet(
                Kind::UdpOut,
                base + 2,
                true,
                false,
                &[seed, seed + 1],
                &movement,
            ),
            packet(Kind::UdpOut, base + 3, true, false, &[], &object),
            packet(Kind::UdpOut, base + 4, true, false, &[], &animation),
            packet(Kind::UdpOut, base + 5, true, false, &[], &layer),
            packet(Kind::UdpIn, base + 4, false, false, &[], &ping),
            packet(
                Kind::UdpOut,
                base + 6,
                false,
                false,
                &[],
                &PacketAck::default(),
            ),
            packet(
                Kind::UdpIn,
                base + 5,
                true,
                false,
                &[],
                &LogoutRequest::default(),
            ),
            packet(
                Kind::UdpOut,
                base + 7,
                true,
                false,
                &[seed],
                &LogoutReply::default(),
            ),
        ]
    }

    fn capture(records: &[Record]) -> Vec<u8> {
        write_capture(records).unwrap()
    }

    fn healthy_pair() -> Vec<Record> {
        let mut records = session(1);
        records.push(Record {
            kind: Kind::HttpRequest,
            time_ms: 50,
            bytes: b"GET /login HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
        });
        let mut right = session(2);
        let extra = packet(
            Kind::UdpIn,
            90,
            false,
            false,
            &[],
            &StartPingCheck::default(),
        );
        right.insert(9, extra);
        right.push(packet(
            Kind::UdpOut,
            91,
            false,
            false,
            &[],
            &PacketAck::default(),
        ));
        right.push(packet(
            Kind::UdpOut,
            92,
            true,
            false,
            &[],
            &KillObject::default(),
        ));
        records.extend(right);
        records
    }

    #[test]
    fn stable_fields_match_and_timer_counts_are_reported() {
        let report = compare_capture(&capture(&healthy_pair())).unwrap();
        assert!(report.contains("values: matched"), "{report}");
        assert!(report.contains("exact in UseCircuitCode: 1"), "{report}");
        assert!(report.contains("exact out LayerData: 1"), "{report}");
        assert!(report.contains("exact out ObjectUpdate: 1"), "{report}");
        assert!(
            report.contains("reported in StartPingCheck: 1 and 2"),
            "{report}"
        );
        assert!(
            report.contains("reported out PacketAck: 1 and 2"),
            "{report}"
        );
        assert!(
            report.contains(
                "order: UseCircuitCode, CompleteAgentMovement, RegionHandshake, RegionHandshakeReply, AgentMovementComplete, LogoutRequest, LogoutReply"
            ),
            "{report}"
        );
        assert!(report.contains("may swap"), "{report}");
        assert!(
            report.contains("reported out KillObject: 0 and 1"),
            "{report}"
        );
        assert!(report.contains("position Z"), "{report}");
    }

    #[test]
    fn an_extra_content_message_fails_the_count() {
        let mut records = healthy_pair();
        records.push(packet(
            Kind::UdpOut,
            200,
            true,
            false,
            &[],
            &LayerData::default(),
        ));
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(error.contains("count out LayerData: 1 and 2"), "{error}");
    }

    fn message_positions(records: &[Record], id: ferroan_protocol::MsgId) -> Vec<usize> {
        records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
                    return None;
                }
                let packet = parse_packet(&record.bytes).unwrap();
                (packet.id == id).then_some(index)
            })
            .collect()
    }

    #[test]
    fn handshake_order_is_the_first_appearance_only() {
        let mut records = healthy_pair();
        let movement = message_positions(&records, CompleteAgentMovement::ID)[1];
        let reply = message_positions(&records, RegionHandshakeReply::ID)[1];
        records.swap(movement, reply);
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(error.contains("first appearance differs"), "{error}");
    }

    #[test]
    fn arrival_and_handshake_reply_may_swap() {
        let mut records = healthy_pair();
        let reply = message_positions(&records, RegionHandshakeReply::ID)[1];
        let arrival = message_positions(&records, AgentMovementComplete::ID)[1];
        records.swap(reply, arrival);
        let report = compare_capture(&capture(&records)).unwrap();
        assert!(report.contains("values: matched"), "{report}");
        assert!(report.contains("order other:"), "{report}");
    }

    #[test]
    fn spawn_height_is_wildcarded_and_horizontal_position_is_not() {
        let mut records = healthy_pair();
        let index = message_positions(&records, AgentMovementComplete::ID)[1];
        let mut message = AgentMovementComplete::default();
        message.data.position.x = 10.0;
        message.data.position.y = 128.0;
        message.data.position.z = 99.0;
        records[index] = packet(Kind::UdpOut, 5, true, false, &[], &message);
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(error.contains("value out AgentMovementComplete"), "{error}");
    }

    #[test]
    fn a_content_byte_is_not_wildcarded() {
        let mut records = healthy_pair();
        let index = records
            .iter()
            .rposition(|record| parse_packet(&record.bytes).unwrap().id == LayerData::ID)
            .unwrap();
        let mut message = LayerData::default();
        message.layer_data.data = b"changed-terrain".to_vec();
        records[index] = packet(Kind::UdpOut, 5, true, false, &[], &message);
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(error.contains("value out LayerData"), "{error}");
    }

    #[test]
    fn a_resent_use_circuit_code_does_not_open_a_third_login() {
        let mut left = session(1);
        let mut resent = left[0].clone();
        resent.bytes[0] |= ferroan_protocol::PacketFlags::RESENT;
        left.insert(1, resent);
        let mut records = left;
        records.extend(session(2));
        let report = compare_capture(&capture(&records)).unwrap();
        assert!(report.contains("values: matched"), "{report}");
        assert!(
            report.contains("exact in UseCircuitCode: 1 (resends 1 and 0)"),
            "{report}"
        );
    }

    #[test]
    fn a_resent_logout_request_counts_once() {
        let mut records = healthy_pair();
        let second = message_positions(&records, LogoutRequest::ID)[1];
        let mut resent = records[second].clone();
        resent.bytes[0] |= ferroan_protocol::PacketFlags::RESENT;
        records.insert(second + 1, resent.clone());
        records.insert(second + 2, resent);
        let report = compare_capture(&capture(&records)).unwrap();
        assert!(report.contains("values: matched"), "{report}");
        assert!(
            report.contains("exact in LogoutRequest: 1 (resends 0 and 2)"),
            "{report}"
        );
    }

    #[test]
    fn duplicate_without_resent_is_rejected_even_in_both_logins() {
        let mut records = healthy_pair();
        for index in message_positions(&records, LogoutRequest::ID)
            .into_iter()
            .rev()
        {
            records.insert(index + 1, records[index].clone());
        }
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(
            error.contains("in LogoutRequest") && error.contains("duplicate lacks the resent flag"),
            "{error}"
        );
    }

    #[test]
    fn reported_reliable_kill_object_still_requires_a_matching_flagged_resend() {
        let mut records = healthy_pair();
        let index = message_positions(&records, KillObject::ID)[0];
        let duplicate = records[index].clone();
        records.insert(index + 1, duplicate);
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(
            error.contains("out KillObject") && error.contains("duplicate lacks the resent flag"),
            "{error}"
        );

        records[index + 1].bytes[0] |= ferroan_protocol::PacketFlags::RESENT;
        let report = compare_capture(&capture(&records)).unwrap();
        assert!(
            report.contains("reported out KillObject: 0 and 1"),
            "{report}"
        );

        let sequence = parse_packet(&records[index].bytes).unwrap().sequence;
        let mut changed = KillObject::default();
        changed.object_data.push(Default::default());
        records[index + 1] = packet(Kind::UdpOut, sequence, true, true, &[], &changed);
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(
            error.contains("out KillObject") && error.contains("resent message differs"),
            "{error}"
        );
    }

    #[test]
    fn resend_must_match_before_cross_login_wildcards() {
        let mut records = healthy_pair();
        let index = message_positions(&records, AgentMovementComplete::ID)[1];
        let first = parse_packet(&records[index].bytes).unwrap();
        let mut movement = first.decode::<AgentMovementComplete>().unwrap();
        // A new login may have another timestamp, but a resend of these bytes may not.
        movement.data.timestamp += 1;
        records.insert(
            index + 1,
            packet(Kind::UdpOut, first.sequence, true, true, &[], &movement),
        );
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(
            error.contains("out AgentMovementComplete") && error.contains("resent message differs"),
            "{error}"
        );
    }

    #[test]
    fn a_resend_can_carry_different_appended_acks() {
        let mut records = healthy_pair();
        let index = message_positions(&records, LogoutReply::ID)[1];
        let first = parse_packet(&records[index].bytes).unwrap();
        let reply = first.decode::<LogoutReply>().unwrap();
        records.insert(
            index + 1,
            packet(Kind::UdpOut, first.sequence, true, true, &[333], &reply),
        );
        let report = compare_capture(&capture(&records)).unwrap();
        assert!(
            report.contains("exact out LogoutReply: 1 (resends 0 and 1)"),
            "{report}"
        );
    }

    #[test]
    fn two_logout_request_sequences_still_fail_the_count() {
        let mut records = healthy_pair();
        let second = message_positions(&records, LogoutRequest::ID)[1];
        records.insert(
            second + 1,
            packet(
                Kind::UdpIn,
                400,
                true,
                false,
                &[],
                &LogoutRequest::default(),
            ),
        );
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(error.contains("count in LogoutRequest: 1 and 2"), "{error}");
    }

    #[test]
    fn udp_before_the_first_login_is_refused() {
        let mut records = vec![packet(
            Kind::UdpOut,
            1,
            false,
            false,
            &[],
            &PacketAck::default(),
        )];
        records.extend(session(1));
        records.extend(session(2));
        let error = compare_capture(&capture(&records)).unwrap_err();
        assert!(
            error.contains("UDP before the first UseCircuitCode"),
            "{error}"
        );
    }
}
