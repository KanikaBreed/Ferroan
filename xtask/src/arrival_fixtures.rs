// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Extract login arrival evidence from the pinned local C# test region.
//! This is a capture selector, not an oracle that generates expected packets.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use ferroan_capture::{Kind, Record, check_udp, read_capture, redact_records, write_capture};
use ferroan_protocol::msgs::{
    AgentMovementComplete, CompleteAgentMovement, RegionHandshake, UseCircuitCode,
};
use ferroan_protocol::{Msg, parse_packet};
use serde_json::{Value, json};

const MAX_CAPTURE: u64 = 64 * 1024 * 1024;
pub(crate) const PIN: &str = "40432e0f83d3802434f898a4e28687bd977d5cf5";
const HOME_HANDLE: u64 = (256_000u64 << 32) | 256_000;
// LLClientView.MoveAgentIntoRegion carries Scene.GetSimulatorVersion(), including
// the pinned C# build version. The Rust harness uses the same region name/UUID.
const CSHARP_CHANNEL: &[u8] =
    b"OpenSim-NGC Tranquillity Release 1.2.154-alpha+40432e0f83 (X64/Unix/DotNet)\0";

pub(crate) fn csharp_provenance(records: &[Record]) -> Result<Value, String> {
    let (_, manifest) = select(records)?;
    Ok(json!({
        "marker": "AgentMovementComplete.SimData.ChannelVersion",
        "channel_version": String::from_utf8_lossy(CSHARP_CHANNEL).trim_end_matches('\0'),
        "region_handle": manifest["arrival"]["region_handle"],
        "checked": "matching local test circuit, movement request, home handshake and reliable C# arrival",
    }))
}

pub fn generate(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_CAPTURE + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("could not read arrival capture: {error}"))?;
    if bytes.len() as u64 > MAX_CAPTURE {
        return Err("arrival capture exceeds 64 MiB".to_string());
    }
    let records = read_capture(&bytes).map_err(|error| error.to_string())?;
    let (trace, manifest) = select(&records)?;
    let target = crate::repo_root().join("fixtures/arrival");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    let arrival = trace.last().ok_or("no arrival evidence")?;
    fs::write(target.join("agent-movement-complete.bin"), &arrival.bytes)
        .map_err(|error| error.to_string())?;
    fs::write(
        target.join("sequence.cap"),
        write_capture(&trace).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        target.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok("arrival fixtures: captured C# request and reliable arrival; identities and timestamp folded".to_string())
}

fn select(records: &[Record]) -> Result<(Vec<Record>, Value), String> {
    check_udp(records).map_err(|error| error.to_string())?;
    let mut circuit = None;
    let mut requested = false;
    let mut home = false;
    let mut selected = Vec::new();
    let mut trace = Vec::new();
    let mut arrival = None;
    for (index, record) in records.iter().enumerate() {
        if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
            continue;
        }
        let packet = parse_packet(&record.bytes).map_err(|error| error.to_string())?;
        let name = if record.kind == Kind::UdpIn && packet.id == UseCircuitCode::ID {
            let code = packet
                .decode::<UseCircuitCode>()
                .map_err(|error| error.to_string())?;
            if code.circuit_code.id.to_string() != crate::live::ESTATE_OWNER {
                return Err("arrival capture is not the local harness test account".to_string());
            }
            if let Some(existing) = &circuit {
                if *existing != code.circuit_code {
                    return Err("arrival capture mixes circuits".to_string());
                }
                continue;
            }
            circuit = Some(code.circuit_code);
            UseCircuitCode::NAME
        } else if record.kind == Kind::UdpIn && packet.id == CompleteAgentMovement::ID {
            let movement = packet
                .decode::<CompleteAgentMovement>()
                .map_err(|error| error.to_string())?;
            let code = circuit
                .as_ref()
                .ok_or("movement request precedes circuit open")?;
            if movement.agent_data.agent_id != code.id
                || movement.agent_data.session_id != code.session_id
                || movement.agent_data.circuit_code != code.code
            {
                return Err("movement request does not match the captured circuit".to_string());
            }
            if requested {
                continue;
            }
            requested = true;
            CompleteAgentMovement::NAME
        } else if record.kind == Kind::UdpOut && packet.id == RegionHandshake::ID {
            let handshake = packet
                .decode::<RegionHandshake>()
                .map_err(|error| error.to_string())?;
            if handshake.region_info2.region_id.to_string() != crate::live::REGION_UUID
                || handshake.region_info.sim_name != b"Ferroan Test\0"
            {
                return Err("arrival capture is not the local harness home region".to_string());
            }
            if home {
                continue;
            }
            home = true;
            RegionHandshake::NAME
        } else if record.kind == Kind::UdpOut && packet.id == AgentMovementComplete::ID {
            if !requested || !home {
                return Err("arrival lacks its movement request or home handshake".to_string());
            }
            let movement = packet
                .decode::<AgentMovementComplete>()
                .map_err(|error| error.to_string())?;
            let code = circuit.as_ref().ok_or("arrival lacks circuit open")?;
            if movement.agent_data.agent_id != code.id
                || movement.agent_data.session_id != code.session_id
            {
                return Err("arrival identity does not match the captured circuit".to_string());
            }
            if movement.sim_data.channel_version != CSHARP_CHANNEL {
                return Err("arrival lacks the pinned C# channel/version marker".to_string());
            }
            if movement.data.region_handle != HOME_HANDLE
                || !packet.reliable()
                || packet.resent()
                || packet.flags & 0x80 != 0
                || !movement.data.position.is_finite()
                || !movement.data.look_at.is_finite()
                || movement.sim_data.channel_version.len() > 1024
            {
                return Err("arrival differs from the expected C# home-region envelope".to_string());
            }
            arrival = Some(movement);
            AgentMovementComplete::NAME
        } else {
            continue;
        };
        trace.push(record.clone());
        selected.push(json!({"record":index,"message":name,"direction":if record.kind == Kind::UdpOut {"out"} else {"in"},"sequence":packet.sequence,"flags":packet.flags,"appended_acks":packet.acks}));
        if arrival.is_some() {
            break;
        }
    }
    let arrival = arrival
        .ok_or("capture has no C# AgentMovementComplete; do not substitute Rust-generated bytes")?;
    let mut trace = redact_records(&trace).map_err(|error| error.to_string())?;
    let origin = trace.first().ok_or("no circuit evidence")?.time_ms;
    for record in &mut trace {
        record.time_ms = record
            .time_ms
            .checked_sub(origin)
            .ok_or("capture timestamps are not monotonic")?;
    }
    let last = trace.last_mut().ok_or("no arrival evidence")?;
    // The checked source datagram is unencoded and has the standard ten-byte
    // low-frequency header. Change only its U32 timestamp, never reconstruct
    // the remaining body to manufacture agreement with our serializer.
    if last.bytes.get(5) != Some(&0) || last.bytes.len() < 80 {
        return Err("unsupported C# arrival header".to_string());
    }
    last.bytes[74..78].fill(0);
    check_udp(&trace).map_err(|error| error.to_string())?;
    let manifest = json!({
        "tranquillity":PIN,
        "source":"cargo xtask live run login grid",
        "redaction":"P0.10d scrub_datagram identities; arrival timestamp=0; capture timestamps relative to circuit open; all other fields retained",
        "selected_records":selected,
        "arrival":{
            "agent_id":uuid::Uuid::from_u128(1).to_string(),
            "session_id":uuid::Uuid::from_u128(2).to_string(),
            "position":arrival.data.position.to_array(),
            "look_at":arrival.data.look_at.to_array(),
            "region_handle":arrival.data.region_handle,
            "timestamp":0,
            "channel_version":arrival.sim_data.channel_version,
        }
    });
    Ok((trace, manifest))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ferroan_protocol::build_packet;

    pub(crate) fn source_records() -> Vec<Record> {
        let mut records =
            read_capture(include_bytes!("../../fixtures/arrival/sequence.cap")).unwrap();
        let owner = crate::live::ESTATE_OWNER.parse().unwrap();
        for record in &mut records {
            let packet = parse_packet(&record.bytes).unwrap();
            if packet.id == UseCircuitCode::ID {
                let mut message = packet.decode::<UseCircuitCode>().unwrap();
                message.circuit_code.id = owner;
                build_packet(
                    &message,
                    packet.sequence,
                    packet.reliable(),
                    packet.resent(),
                    &packet.acks,
                    &mut record.bytes,
                );
            } else if packet.id == CompleteAgentMovement::ID {
                let mut message = packet.decode::<CompleteAgentMovement>().unwrap();
                message.agent_data.agent_id = owner;
                build_packet(
                    &message,
                    packet.sequence,
                    packet.reliable(),
                    packet.resent(),
                    &packet.acks,
                    &mut record.bytes,
                );
            } else if packet.id == AgentMovementComplete::ID {
                let mut message = packet.decode::<AgentMovementComplete>().unwrap();
                message.agent_data.agent_id = owner;
                message.data.timestamp = 1_800_000_000;
                build_packet(
                    &message,
                    packet.sequence,
                    packet.reliable(),
                    packet.resent(),
                    &packet.acks,
                    &mut record.bytes,
                );
            }
            record.time_ms += 1_800_000_000_000;
        }
        records
    }

    #[test]
    fn arrival_extraction_folds_only_identities_and_time() {
        let records = source_records();
        let (trace, manifest) = select(&records).unwrap();
        assert_eq!(
            trace.last().unwrap().bytes,
            include_bytes!("../../fixtures/arrival/agent-movement-complete.bin")
        );
        let original = &records.last().unwrap().bytes;
        let folded = &trace.last().unwrap().bytes;
        assert_eq!(&original[..10], &folded[..10]);
        assert_eq!(&original[42..74], &folded[42..74]);
        assert_eq!(&original[78..], &folded[78..]);
        assert_eq!(&folded[74..78], &[0; 4]);
        assert_eq!(trace[0].time_ms, 0);
        assert_eq!(manifest["arrival"]["region_handle"], HOME_HANDLE);
        let message = parse_packet(folded)
            .unwrap()
            .decode::<AgentMovementComplete>()
            .unwrap();
        assert_eq!(message.agent_data.agent_id, uuid::Uuid::from_u128(1));
        assert_eq!(message.agent_data.session_id, uuid::Uuid::from_u128(2));
    }

    #[test]
    fn arrival_extraction_refuses_missing_request_and_reply() {
        let mut records = source_records();
        records.pop();
        assert!(
            select(&records)
                .unwrap_err()
                .contains("no C# AgentMovementComplete")
        );
        let records: Vec<_> = source_records()
            .into_iter()
            .filter(|record| parse_packet(&record.bytes).unwrap().id != CompleteAgentMovement::ID)
            .collect();
        assert!(
            select(&records)
                .unwrap_err()
                .contains("lacks its movement request")
        );
    }

    #[test]
    fn arrival_extraction_refuses_another_session_or_region() {
        for wrong_session in [true, false] {
            let mut records = source_records();
            let record = records.last_mut().unwrap();
            let packet = parse_packet(&record.bytes).unwrap();
            let mut message = packet.decode::<AgentMovementComplete>().unwrap();
            if wrong_session {
                message.agent_data.session_id = uuid::Uuid::from_u128(9);
            } else {
                message.data.region_handle += 256;
            }
            build_packet(
                &message,
                packet.sequence,
                packet.reliable(),
                packet.resent(),
                &packet.acks,
                &mut record.bytes,
            );
            let error = select(&records).unwrap_err();
            assert!(error.contains(if wrong_session {
                "identity"
            } else {
                "home-region"
            }));
        }
    }

    #[test]
    fn matching_harness_identity_cannot_substitute_a_rust_or_missing_channel() {
        for channel in [
            b"Ferroan 0.1.0\0".as_slice(),
            b"",
            b"OpenSim-NGC Tranquillity other pin\0",
        ] {
            let mut records = source_records();
            let record = records.last_mut().unwrap();
            let packet = parse_packet(&record.bytes).unwrap();
            let mut arrival = packet.decode::<AgentMovementComplete>().unwrap();
            arrival.sim_data.channel_version = channel.to_vec();
            build_packet(
                &arrival,
                packet.sequence,
                packet.reliable(),
                false,
                &[],
                &mut record.bytes,
            );
            assert!(
                csharp_provenance(&records)
                    .unwrap_err()
                    .contains("pinned C# channel/version")
            );
        }
        assert_eq!(
            csharp_provenance(&source_records()).unwrap()["region_handle"],
            HOME_HANDLE
        );
    }
}
