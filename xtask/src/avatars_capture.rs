// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The other avatar's packets from a Ferroan `avatars` run against the C#
//! capture in `fixtures/avatars/avatars.cap`: full `ObjectUpdate`,
//! `AvatarAppearance` and `KillObject`. The capture does not record which
//! viewer a datagram went to, so each kind is compared per avatar. The live
//! run is redacted the way the fixture was before the bytes are compared;
//! the names are checked on the raw run.

use std::path::Path;

use ferroan_capture::{Kind, Record, redact_records};
use ferroan_protocol::msgs::{AvatarAppearance, KillObject, ObjectUpdate};
use ferroan_protocol::{IncomingPacket, Msg, PacketFlags, Writer, parse_packet, str_field};
use uuid::Uuid;

/// `PCode.Avatar`.
const PCODE_AVATAR: u8 = 47;
/// Collision plane and Z inside the 76-byte avatar block.
const PLANE: std::ops::Range<usize> = 0..16;
const Z: std::ops::Range<usize> = 24..28;
/// `Vector4.UnitW`, as the stand-in leaves it.
const UNIT_W: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3f];

/// One avatar packet and its header flags.
struct Seen<T> {
    flags: u8,
    message: T,
}

#[derive(Default)]
struct AvatarPackets {
    fulls: Vec<Seen<ObjectUpdate>>,
    appearances: Vec<Seen<AvatarAppearance>>,
    kills: Vec<Seen<KillObject>>,
}

fn outgoing(records: &[Record]) -> Vec<IncomingPacket> {
    records
        .iter()
        .filter(|record| record.kind == Kind::UdpOut)
        .filter_map(|record| parse_packet(&record.bytes).ok())
        // A resend repeats a datagram already counted.
        .filter(|packet| packet.flags & PacketFlags::RESENT == 0)
        .collect()
}

fn avatar_packets(records: &[Record], agents: &[Uuid]) -> Result<AvatarPackets, String> {
    let mut found = AvatarPackets::default();
    for packet in outgoing(records) {
        if packet.id == ObjectUpdate::ID {
            let message = packet
                .decode::<ObjectUpdate>()
                .map_err(|_| "an ObjectUpdate did not decode".to_string())?;
            if message.object_data.len() == 1
                && message.object_data[0].p_code == PCODE_AVATAR
                && agents.contains(&message.object_data[0].full_id)
            {
                found.fulls.push(Seen {
                    flags: packet.flags,
                    message,
                });
            }
        } else if packet.id == AvatarAppearance::ID {
            let message = packet
                .decode::<AvatarAppearance>()
                .map_err(|_| "an AvatarAppearance did not decode".to_string())?;
            if agents.contains(&message.sender.id) {
                found.appearances.push(Seen {
                    flags: packet.flags,
                    message,
                });
            }
        } else if packet.id == KillObject::ID {
            let message = packet
                .decode::<KillObject>()
                .map_err(|_| "a KillObject did not decode".to_string())?;
            found.kills.push(Seen {
                flags: packet.flags,
                message,
            });
        }
    }
    Ok(found)
}

fn body<M: Msg>(message: &M) -> Vec<u8> {
    let mut writer = Writer::new();
    message.encode_body(&mut writer);
    writer.buf
}

/// A redacted update with the local id and Z masked. Ferroan's block must
/// carry the unit-W plane; C#'s plane is masked with it, because physics
/// sets one once the avatar lands.
fn masked_full(update: &ObjectUpdate) -> ObjectUpdate {
    let mut update = update.clone();
    let block = &mut update.object_data[0];
    block.id = 0;
    if block.object_data.len() == 76 {
        block.object_data[PLANE].copy_from_slice(&UNIT_W);
        block.object_data[Z].fill(0);
    }
    update
}

fn name_value(first: &str, last: &str) -> Vec<u8> {
    str_field(&format!(
        "FirstName STRING RW SV {first}\nLastName STRING RW SV {last}\nTitle STRING RW SV "
    ))
}

/// Ferroan's packets about the owner and the second bot against C#'s.
pub(super) fn compare_avatars(directory: &Path) -> Result<String, String> {
    let live = super::login_capture::records(&directory.join("capture-udp.cap"))?;
    let redacted = redact_records(&live).map_err(|error| error.to_string())?;
    let fixture =
        super::login_capture::records(&crate::repo_root().join("fixtures/avatars/avatars.cap"))?;
    let owner = Uuid::parse_str(crate::live::ESTATE_OWNER)
        .map_err(|_| "estate owner is not a uuid".to_string())?;
    let chat = Uuid::parse_str(crate::live::CHAT_USER)
        .map_err(|_| "chat user is not a uuid".to_string())?;
    let agents = [owner, chat];
    let csharp = avatar_packets(&fixture, &agents)?;
    let ferroan = avatar_packets(&live, &agents)?;
    let ferroan_redacted = avatar_packets(&redacted, &agents)?;
    if ferroan_redacted.fulls.len() != ferroan.fulls.len() {
        return Err("redaction changed which avatar updates the run holds".into());
    }
    let mut local_ids = Vec::new();
    for (agent, names) in [(owner, ("Test", "User")), (chat, ("Chat", "Bot"))] {
        let reference = csharp
            .fulls
            .iter()
            .find(|seen| seen.message.object_data[0].full_id == agent)
            .ok_or("the C# capture has no full update for a harness avatar")?;
        let reference_body = body(&masked_full(&reference.message));
        let own: Vec<_> = ferroan
            .fulls
            .iter()
            .zip(&ferroan_redacted.fulls)
            .filter(|(seen, _)| seen.message.object_data[0].full_id == agent)
            .collect();
        // Its own viewer, then the other one.
        if own.len() < 2 {
            return Err(format!(
                "Ferroan sent {} full update(s) of a harness avatar; its own viewer and the other bot each need one",
                own.len()
            ));
        }
        let local_id = own[0].0.message.object_data[0].id;
        for (seen, redacted) in &own {
            let block = &seen.message.object_data[0];
            if seen.flags & 0xc0 != reference.flags & 0xc0 || seen.flags & 0xc0 != 0xc0 {
                return Err("an avatar ObjectUpdate is not reliable and zero-coded".into());
            }
            if block.id != local_id {
                return Err("one avatar's full updates carry different local ids".into());
            }
            if block.name_value != name_value(names.0, names.1) {
                return Err("an avatar ObjectUpdate does not name its harness account".into());
            }
            if block.object_data.get(PLANE) != Some(&UNIT_W[..]) {
                return Err("an avatar ObjectUpdate carries a collision plane".into());
            }
            if body(&masked_full(&redacted.message)) != reference_body {
                return Err(
                    "an avatar ObjectUpdate differs from the C# capture past the local id and Z"
                        .into(),
                );
            }
        }
        local_ids.push(local_id);
        let reference = csharp
            .appearances
            .iter()
            .find(|seen| seen.message.sender.id == agent)
            .ok_or("the C# capture has no appearance for a harness avatar")?;
        let reference_body = body(&reference.message);
        let sent: Vec<_> = ferroan_redacted
            .appearances
            .iter()
            .filter(|seen| seen.message.sender.id == agent)
            .collect();
        if sent.len() < 2 {
            return Err(format!(
                "Ferroan sent {} appearance(s) of a harness avatar; its own viewer and the other bot each need one",
                sent.len()
            ));
        }
        for seen in sent {
            if seen.flags & 0xc0 != reference.flags & 0xc0 {
                return Err("an AvatarAppearance header differs from the C# capture".into());
            }
            if body(&seen.message) != reference_body {
                return Err("an AvatarAppearance differs from the C# capture".into());
            }
        }
    }
    if local_ids[0] == local_ids[1] {
        return Err("the two avatars share a local id".into());
    }
    // The second bot leaves first; the owner sees it go.
    let reference = csharp
        .kills
        .first()
        .ok_or("the C# capture has no KillObject")?;
    let mut reference_kill = reference.message.clone();
    reference_kill
        .object_data
        .iter_mut()
        .for_each(|block| block.id = 0);
    let kills: Vec<_> = ferroan
        .kills
        .iter()
        .filter(|seen| {
            seen.message
                .object_data
                .iter()
                .any(|block| block.id == local_ids[1])
        })
        .collect();
    if kills.len() != 1 {
        return Err(format!(
            "Ferroan sent {} KillObject(s) for the second bot's local id; the owner needs one",
            kills.len()
        ));
    }
    let mut kill = kills[0].message.clone();
    kill.object_data.iter_mut().for_each(|block| block.id = 0);
    if kills[0].flags & 0xc0 != reference.flags & 0xc0 || body(&kill) != body(&reference_kill) {
        return Err("the KillObject differs from the C# capture past the local id".into());
    }
    if ferroan.kills.iter().any(|seen| {
        seen.message
            .object_data
            .iter()
            .any(|block| block.id == local_ids[0])
    }) {
        return Err("Ferroan killed the owner's avatar while it stayed".into());
    }
    Ok(format!(
        "avatar bytes: {} full ObjectUpdate, {} AvatarAppearance and {} KillObject match the C# capture with local ids masked; Z and the collision plane are folded (the home store has no terrain row, and the stand-in sets no plane); known differences: C# also sends the kill to the leaving viewer after its LogoutReply ({} kill(s) in the C# capture), and AvatarAnimation is P2.3b1",
        ferroan.fulls.len(),
        ferroan.appearances.len(),
        kills.len(),
        csharp.kills.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed C# run: both bots' full updates and appearances, and one
    /// kill of the second bot on each of the two circuits.
    #[test]
    fn the_csharp_capture_has_both_avatars_and_the_kill() {
        let records = super::super::login_capture::records(
            &crate::repo_root().join("fixtures/avatars/avatars.cap"),
        )
        .unwrap();
        let owner = Uuid::parse_str(crate::live::ESTATE_OWNER).unwrap();
        let chat = Uuid::parse_str(crate::live::CHAT_USER).unwrap();
        let found = avatar_packets(&records, &[owner, chat]).unwrap();
        for agent in [owner, chat] {
            assert!(
                found
                    .fulls
                    .iter()
                    .filter(|seen| seen.message.object_data[0].full_id == agent)
                    .count()
                    >= 2
            );
            assert!(
                found
                    .appearances
                    .iter()
                    .filter(|seen| seen.message.sender.id == agent)
                    .count()
                    >= 2
            );
        }
        let chat_id = found
            .fulls
            .iter()
            .find(|seen| seen.message.object_data[0].full_id == chat)
            .unwrap()
            .message
            .object_data[0]
            .id;
        assert_eq!(found.kills.len(), 2);
        for kill in &found.kills {
            assert_eq!(kill.flags & 0xc0, PacketFlags::RELIABLE);
            assert_eq!(kill.message.object_data.len(), 1);
            assert_eq!(kill.message.object_data[0].id, chat_id);
        }
        for full in &found.fulls {
            assert_eq!(full.flags & 0xc0, 0xc0);
        }
    }
}
