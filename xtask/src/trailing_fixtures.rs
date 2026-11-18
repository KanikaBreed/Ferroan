// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Select a real C# IM, retaining its exact framing and trailing block bytes.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use ferroan_capture::{Kind, Record, check_udp, read_capture};
use ferroan_protocol::msgs::{AgentMovementComplete, ImprovedInstantMessage};
use ferroan_protocol::{Msg, parse_packet};
use serde_json::{Value, json};

const PIN: &str = "858321883f5487a5d192d36ea9d7b90fa468310c";
const CHANNEL: &[u8] =
    b"OpenSim-NGC Tranquillity Release 1.2.132-alpha+858321883f (X64/Unix/DotNet)\0";
const MAX_CAPTURE: u64 = 64 * 1024 * 1024;

pub fn generate(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_CAPTURE + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("could not read trailing-block capture: {error}"))?;
    if bytes.len() as u64 > MAX_CAPTURE {
        return Err("trailing-block capture exceeds 64 MiB".into());
    }
    let records = read_capture(&bytes).map_err(|error| error.to_string())?;
    let (packet, manifest) = select(&records)?;
    let target = crate::repo_root().join("fixtures/trailing");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    fs::write(target.join("improved-instant-message.bin"), packet)
        .map_err(|error| error.to_string())?;
    fs::write(
        target.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok("trailing fixtures: captured C# ImprovedInstantMessage; EstateBlock present, MetaData count absent".into())
}

fn select(records: &[Record]) -> Result<(Vec<u8>, Value), String> {
    // Check before redaction: never turn a failed raw round trip into evidence.
    check_udp(records).map_err(|error| error.to_string())?;
    let mut arrivals = Vec::new();
    for (index, record) in records.iter().enumerate() {
        if record.kind != Kind::UdpOut {
            continue;
        }
        let packet = parse_packet(&record.bytes).map_err(|error| error.to_string())?;
        if packet.id == AgentMovementComplete::ID {
            let movement = packet
                .decode::<AgentMovementComplete>()
                .map_err(|error| error.to_string())?;
            if movement.sim_data.channel_version == CHANNEL && arrivals.len() < 16 {
                arrivals.push(movement.agent_data.agent_id);
            }
            continue;
        }
        if packet.id != ImprovedInstantMessage::ID {
            continue;
        }
        let message = packet
            .decode::<ImprovedInstantMessage>()
            .map_err(|error| error.to_string())?;
        if !arrivals.contains(&message.message_block.to_agent_id)
            || message.message_block.dialog != 0
            || !message
                .message_block
                .message
                .starts_with(b"ferroan-im-to-ferrobot-")
        {
            continue;
        }
        // This fixture intentionally covers the unencoded, no-ACK C# writer.
        // Slice replacement preserves packet length and every unredacted byte.
        if packet.flags != 0x40 || record.bytes.get(5) != Some(&0) || !packet.acks.is_empty() {
            return Err("unsupported C# IM envelope".into());
        }
        let mut bytes = record.bytes.clone();
        let name_len = usize::from(*bytes.get(113).ok_or("short IM name length")?);
        let message_offset = 114 + name_len;
        let message_size = bytes
            .get(message_offset..message_offset + 2)
            .ok_or("short IM message length")?;
        let message_len = usize::from(u16::from_le_bytes([message_size[0], message_size[1]]));
        let bucket_offset = message_offset + 2 + message_len;
        let bucket_size = bytes
            .get(bucket_offset..bucket_offset + 2)
            .ok_or("short IM bucket length")?;
        let bucket_len = usize::from(u16::from_le_bytes([bucket_size[0], bucket_size[1]]));
        let prefix_len = bucket_offset + 2 + bucket_len;
        if bucket_len != 0 || bytes.get(prefix_len..) != Some(&[0, 0, 0, 0][..]) {
            return Err(
                "C# IM must have empty bucket and exactly EstateID=0 after the legacy prefix"
                    .into(),
            );
        }
        for offset in [10, 43, 63, 93] {
            bytes[offset..offset + 16].copy_from_slice(uuid::Uuid::from_u128(1).as_bytes());
        }
        bytes[26..42].copy_from_slice(uuid::Uuid::from_u128(2).as_bytes());
        bytes[109..113].fill(0);
        for range in [114..message_offset, message_offset + 2..bucket_offset] {
            for byte in &mut bytes[range] {
                if *byte != 0 {
                    *byte = b'x';
                }
            }
        }
        let manifest = json!({
            "tranquillity": PIN,
            "source": "cargo xtask live run ferrobot (preserved Q10 candidate, C# standalone)",
            "source_file": "Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs::SendInstantMessage",
            "channel_version": String::from_utf8_lossy(CHANNEL).trim_end_matches('\0'),
            "selected_record": index,
            "original_length": record.bytes.len(),
            "libopenmetaverse_prefix_length": prefix_len,
            "estate_id": 0,
            "metadata_count_present": false,
            "redaction": "Direct in-place replacement, no serializer: UUIDs at offsets 10,43,63,93 to 1; session at 26 to 2; timestamp at 109 to 0; non-NUL name/message bytes to x. Length, header, other fields and suffix unchanged. Raw capture passed lossless check first."
        });
        return Ok((bytes, manifest));
    }
    Err("no harness IM to an agent with a preceding pinned C# channel/version arrival".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_csharp_channel_evidence_is_rejected() {
        let packet =
            fs::read(crate::repo_root().join("fixtures/trailing/improved-instant-message.bin"))
                .unwrap();
        let records = [Record {
            kind: Kind::UdpOut,
            time_ms: 0,
            bytes: packet,
        }];
        assert!(
            select(&records)
                .unwrap_err()
                .contains("preceding pinned C#")
        );
    }

    #[test]
    fn truncated_capture_packet_is_not_redacted_into_evidence() {
        let records = [Record {
            kind: Kind::UdpOut,
            time_ms: 0,
            bytes: vec![0x40],
        }];
        assert!(select(&records).is_err());
    }
}
