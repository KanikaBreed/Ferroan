// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Extract circuit evidence from the local, pinned C# grid's UDP capture.
//! Identity folding is exactly the P0.10d protocol scrubber. Packet headers,
//! sequence numbers, appended ACKs and configuration fields are retained.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use ferroan_capture::{Kind, Record, check_udp, read_capture, redact_records, write_capture};
use ferroan_protocol::msgs::{
    LogoutReply, LogoutRequest, PacketAck, RegionHandshake, UseCircuitCode,
};
use ferroan_protocol::{Msg, message_info, parse_packet};
use serde_json::{Value, json};

const MAX_CAPTURE: u64 = 64 * 1024 * 1024;
const PIN: &str = "858321883f5487a5d192d36ea9d7b90fa468310c";

pub fn generate(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_CAPTURE + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("could not read UDP capture: {error}"))?;
    if bytes.len() as u64 > MAX_CAPTURE {
        return Err("UDP capture exceeds 64 MiB".to_string());
    }
    let records = read_capture(&bytes).map_err(|error| error.to_string())?;
    let evidence = select(&records)?;
    let target = crate::repo_root().join("fixtures/circuit");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    for (name, record) in &evidence.packets {
        fs::write(target.join(format!("{name}.bin")), &record.bytes)
            .map_err(|error| error.to_string())?;
    }
    fs::write(
        target.join("sequence.cap"),
        write_capture(&evidence.trace).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        target.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&evidence.manifest).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "circuit fixtures: {} C# datagrams, original headers and configuration retained",
        evidence.trace.len()
    ))
}

struct Evidence {
    packets: Vec<(&'static str, Record)>,
    trace: Vec<Record>,
    manifest: Value,
}

fn select(records: &[Record]) -> Result<Evidence, String> {
    let provenance = crate::arrival_fixtures::csharp_provenance(records)?;
    let mut trace = Vec::new();
    let mut order = Vec::new();
    let mut outgoing = Vec::new();
    let mut ucc_sequence = None;
    let mut handshake = None;
    for (index, record) in records.iter().enumerate() {
        if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
            continue;
        }
        let packet =
            parse_packet(&record.bytes).map_err(|error| format!("UDP record {index}: {error}"))?;
        let name = message_info(packet.id)
            .ok_or("unknown captured packet type")?
            .name;
        if record.kind == Kind::UdpOut {
            outgoing.push(json!({"record": index, "message": name, "sequence": packet.sequence, "flags": packet.flags, "appended_acks": packet.acks}));
        }
        let keep = if record.kind == Kind::UdpIn && packet.id == UseCircuitCode::ID {
            let ucc = packet
                .decode::<UseCircuitCode>()
                .map_err(|error| error.to_string())?;
            if ucc.circuit_code.id.to_string() != crate::live::ESTATE_OWNER {
                return Err("capture is not the local harness test account".to_string());
            }
            ucc_sequence.get_or_insert(packet.sequence);
            true
        } else if record.kind == Kind::UdpOut && packet.id == RegionHandshake::ID {
            let decoded = packet
                .decode::<RegionHandshake>()
                .map_err(|error| error.to_string())?;
            if decoded.region_info2.region_id.to_string() != crate::live::REGION_UUID {
                return Err("capture is not the local harness home region".to_string());
            }
            if decoded.region_info.sim_owner.to_string() != crate::live::ESTATE_OWNER
                || decoded.region_info.sim_name != b"Ferroan Test\0"
            {
                return Err("handshake is not the local harness test configuration".to_string());
            }
            if handshake.is_none() {
                handshake = Some(decoded);
            }
            true
        } else {
            (record.kind == Kind::UdpOut
                && (packet.id == PacketAck::ID || packet.id == LogoutReply::ID))
                || (record.kind == Kind::UdpIn && packet.id == LogoutRequest::ID)
        };
        if keep {
            trace.push(record.clone());
            order.push(json!({"record": index, "message": name, "direction": if record.kind == Kind::UdpOut { "out" } else { "in" }}));
        }
    }
    let ucc_sequence = ucc_sequence.ok_or("capture has no UseCircuitCode")?;
    let handshake = handshake.ok_or("capture has no RegionHandshake")?;
    check_udp(&trace).map_err(|error| error.to_string())?;
    let trace = redact_records(&trace).map_err(|error| error.to_string())?;
    let mut packets = Vec::new();
    for record in &trace {
        let packet = parse_packet(&record.bytes).map_err(|error| error.to_string())?;
        let name = if record.kind == Kind::UdpOut && packet.id == PacketAck::ID {
            let ack = packet
                .decode::<PacketAck>()
                .map_err(|error| error.to_string())?;
            if packet.sequence == 0 && ack.packets.iter().any(|ack| ack.id == ucc_sequence) {
                Some("packet-ack")
            } else if packet.sequence != 0 {
                Some("packet-ack-established")
            } else {
                None
            }
        } else if packet.id == RegionHandshake::ID {
            Some("region-handshake")
        } else if packet.id == LogoutReply::ID {
            Some("logout-reply")
        } else {
            None
        };
        if let Some(name) = name
            && !packets.iter().any(|(existing, _)| *existing == name)
        {
            packets.push((name, record.clone()));
        }
    }
    for required in [
        "packet-ack",
        "packet-ack-established",
        "region-handshake",
        "logout-reply",
    ] {
        if !packets.iter().any(|(name, _)| *name == required) {
            return Err(format!(
                "capture has no {required}; do not substitute Rust-generated bytes"
            ));
        }
    }
    let manifest = json!({
        "tranquillity": PIN,
        "provenance": provenance,
        "source": "cargo xtask live run login grid",
        "redaction": "ferroan_protocol::scrub_datagram (P0.10d); no further field normalization",
        "outgoing_sequence": outgoing,
        "selected_records": order,
        "handshake_configuration": {
            "sim_name": handshake.region_info.sim_name,
            "sim_access": handshake.region_info.sim_access,
            "sim_owner": handshake.region_info.sim_owner.to_string(),
            "estate_manager": handshake.region_info.is_estate_manager,
            "region_flags": handshake.region_info.region_flags,
            "water_height": handshake.region_info.water_height,
            "billable_factor": handshake.region_info.billable_factor,
            "cache_id": handshake.region_info.cache_id.to_string(),
            "region_id": handshake.region_info2.region_id.to_string(),
            "product_name": handshake.region_info3.product_name,
            "cpu_class_id": handshake.region_info3.cpu_class_id,
            "cpu_ratio": handshake.region_info3.cpu_ratio,
            "colo_name": handshake.region_info3.colo_name,
            "product_sku": handshake.region_info3.product_sku,
            "terrain_base": [handshake.region_info.terrain_base0.to_string(), handshake.region_info.terrain_base1.to_string(), handshake.region_info.terrain_base2.to_string(), handshake.region_info.terrain_base3.to_string()],
            "terrain_detail": [handshake.region_info.terrain_detail0.to_string(), handshake.region_info.terrain_detail1.to_string(), handshake.region_info.terrain_detail2.to_string(), handshake.region_info.terrain_detail3.to_string()],
            "terrain_start_heights": [handshake.region_info.terrain_start_height00, handshake.region_info.terrain_start_height01, handshake.region_info.terrain_start_height10, handshake.region_info.terrain_start_height11],
            "terrain_height_ranges": [handshake.region_info.terrain_height_range00, handshake.region_info.terrain_height_range01, handshake.region_info.terrain_height_range10, handshake.region_info.terrain_height_range11],
            "region_flags_extended": handshake.region_info4.iter().map(|block| block.region_flags_extended).collect::<Vec<_>>(),
            "region_protocols": handshake.region_info4.iter().map(|block| block.region_protocols).collect::<Vec<_>>()
        }
    });
    Ok(Evidence {
        packets,
        trace,
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_packets_keep_ack_sequence_and_reliable_logout() {
        let immediate =
            parse_packet(include_bytes!("../../fixtures/circuit/packet-ack.bin")).unwrap();
        let handshake = parse_packet(include_bytes!(
            "../../fixtures/circuit/region-handshake.bin"
        ))
        .unwrap();
        let established = parse_packet(include_bytes!(
            "../../fixtures/circuit/packet-ack-established.bin"
        ))
        .unwrap();
        let logout =
            parse_packet(include_bytes!("../../fixtures/circuit/logout-reply.bin")).unwrap();
        assert_eq!(immediate.sequence, 0);
        assert!(!immediate.reliable());
        assert_eq!(immediate.decode::<PacketAck>().unwrap().packets[0].id, 1);
        assert_eq!(handshake.sequence, 1);
        assert!(established.sequence > handshake.sequence);
        assert!(!established.reliable());
        assert!(logout.sequence > established.sequence);
        assert!(logout.reliable());
        let logout = logout.decode::<LogoutReply>().unwrap();
        assert_eq!(logout.agent_data.agent_id, uuid::Uuid::from_u128(1));
        assert_eq!(logout.agent_data.session_id, uuid::Uuid::from_u128(2));
        assert_eq!(logout.inventory_data.len(), 1);
        assert!(logout.inventory_data[0].item_id.is_nil());
    }

    #[test]
    fn extractor_refuses_a_trace_without_csharp_channel_evidence() {
        let records = read_capture(include_bytes!("../../fixtures/circuit/sequence.cap")).unwrap();
        let without_logout: Vec<_> = records
            .into_iter()
            .filter(|record| parse_packet(&record.bytes).unwrap().id != LogoutReply::ID)
            .collect();
        // The test capture's identities have already been folded. Restore only
        // the known local account for the extractor's source-account check.
        let mut without_logout = without_logout;
        let ucc = without_logout
            .iter_mut()
            .find(|record| parse_packet(&record.bytes).unwrap().id == UseCircuitCode::ID)
            .unwrap();
        let packet = parse_packet(&ucc.bytes).unwrap();
        let mut message = packet.decode::<UseCircuitCode>().unwrap();
        message.circuit_code.id = crate::live::ESTATE_OWNER.parse().unwrap();
        ferroan_protocol::build_packet(
            &message,
            packet.sequence,
            packet.reliable(),
            packet.resent(),
            &packet.acks,
            &mut ucc.bytes,
        );
        assert!(
            select(&without_logout)
                .err()
                .unwrap()
                .contains("no C# AgentMovementComplete")
        );
        let mut with_csharp_arrival = crate::arrival_fixtures::tests::source_records();
        with_csharp_arrival.extend(without_logout);
        assert!(
            select(&with_csharp_arrival)
                .err()
                .unwrap()
                .contains("no logout-reply")
        );
    }
}
