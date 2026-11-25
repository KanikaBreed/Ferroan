// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The other avatar's packets from a Ferroan `avatars` run against the C#
//! capture in `fixtures/avatars/avatars.cap`: full `ObjectUpdate`,
//! `AvatarAppearance`, `KillObject` and the minimap's `CoarseLocationUpdate`;
//! and the `UUIDNameReply` packets against `fixtures/avatars/names.cap`.
//! The capture does not record which viewer a datagram went to, so each kind
//! is compared per avatar, and each minimap list by its `You` index. The live
//! run is redacted the way the fixture was before the bytes are compared;
//! the names are checked on the raw run.

use std::path::Path;

use ferroan_bot::scenario::UNKNOWN_NAME_ID;
use ferroan_capture::{Kind, Record, redact_records};
use ferroan_protocol::msgs::{
    AvatarAppearance, CoarseLocationUpdate, KillObject, ObjectUpdate, UUIDNameReply,
};
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
    names: Vec<Seen<UUIDNameReply>>,
    /// `flags` here is the whole first header byte, and the extra-header
    /// length must be 0: C# writes this header by hand.
    coarse: Vec<Seen<CoarseLocationUpdate>>,
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
        } else if packet.id == CoarseLocationUpdate::ID {
            let message = packet
                .decode::<CoarseLocationUpdate>()
                .map_err(|_| "a CoarseLocationUpdate did not decode".to_string())?;
            found.coarse.push(Seen {
                flags: packet.flags,
                message,
            });
        } else if packet.id == KillObject::ID {
            let message = packet
                .decode::<KillObject>()
                .map_err(|_| "a KillObject did not decode".to_string())?;
            found.kills.push(Seen {
                flags: packet.flags,
                message,
            });
        } else if packet.id == UUIDNameReply::ID {
            let message = packet
                .decode::<UUIDNameReply>()
                .map_err(|_| "a UUIDNameReply did not decode".to_string())?;
            found.names.push(Seen {
                flags: packet.flags,
                message,
            });
        }
    }
    Ok(found)
}

/// The first header byte and the extra-header length of every outgoing
/// `CoarseLocationUpdate`, resends included (there should be none).
fn coarse_headers(records: &[Record]) -> Vec<(u8, u8)> {
    records
        .iter()
        .filter(|record| record.kind == Kind::UdpOut)
        .filter(|record| {
            parse_packet(&record.bytes).is_ok_and(|packet| packet.id == CoarseLocationUpdate::ID)
        })
        .filter_map(|record| Some((*record.bytes.first()?, *record.bytes.get(5)?)))
        .collect()
}

/// The C# and Ferroan minimap headers, from [`coarse_headers`].
type Headers<'a> = (&'a [(u8, u8)], &'a [(u8, u8)]);

/// A minimap list with the location bytes zeroed: the agents, `You` and
/// `Prey` are what must match byte for byte.
fn masked_coarse(update: &CoarseLocationUpdate) -> Vec<u8> {
    let mut update = update.clone();
    for location in &mut update.location {
        *location = Default::default();
    }
    body(&update)
}

/// Ferroan's minimap lists against C#'s. Each run sends one list to each
/// viewer every 50 frames, so the count follows the clock and is not
/// compared; the kinds of list are: both avatars to each viewer, and the
/// owner alone after the second bot leaves. A list to a viewer whose avatar
/// has not arrived yet (`You` -1: the owner alone to the second bot, when a
/// 50-frame boundary falls between its UseCircuitCode and its arrival, or
/// an empty list when nobody is root) is allowed on either side. Redaction puts the
/// agent placeholder in every `AgentID`, so the redacted lists are compared
/// with the fixture, and the order (the owner first, as it arrived first) is
/// checked on the raw run.
fn compare_coarse(
    csharp: &[Seen<CoarseLocationUpdate>],
    ferroan: &[Seen<CoarseLocationUpdate>],
    raw: &[Seen<CoarseLocationUpdate>],
    headers: Headers<'_>,
    owner: Uuid,
    chat: Uuid,
) -> Result<String, String> {
    for seen in raw {
        let agents: Vec<Uuid> = seen
            .message
            .agent_data
            .iter()
            .map(|data| data.agent_id)
            .collect();
        if !(agents.is_empty() || agents == [owner] || agents == [owner, chat]) {
            return Err(
                "a Ferroan minimap list does not name the owner, then the second bot".into(),
            );
        }
    }
    if headers
        .0
        .iter()
        .chain(headers.1)
        .any(|header| *header != (0, 0))
    {
        return Err("a CoarseLocationUpdate is not unreliable, unencoded and plain".into());
    }
    // A list to a viewer that is not root yet (`You` -1) depends on whether
    // a 50-frame boundary falls between its UseCircuitCode and its arrival.
    let kinds = |seen: &[Seen<CoarseLocationUpdate>]| {
        let mut kinds: Vec<Vec<u8>> = seen
            .iter()
            .filter(|seen| !seen.message.agent_data.is_empty() && seen.message.index.you != -1)
            .map(|seen| masked_coarse(&seen.message))
            .collect();
        kinds.sort();
        kinds.dedup();
        kinds
    };
    let expected = kinds(csharp);
    if expected.len() != 3 {
        return Err(format!(
            "the C# capture has {} kinds of minimap list; both avatars to each viewer and the owner alone make 3",
            expected.len()
        ));
    }
    if kinds(ferroan) != expected {
        return Err(
            "Ferroan's minimap lists differ from the C# capture in their agents, You or Prey"
                .into(),
        );
    }
    let both = |seen: &[Seen<CoarseLocationUpdate>]| -> Vec<CoarseLocationUpdate> {
        seen.iter()
            .map(|seen| seen.message.clone())
            .filter(|message| message.agent_data.len() == 2 && message.location.len() == 2)
            .collect()
    };
    for (side, lists) in [("C#", both(csharp)), ("Ferroan", both(ferroan))] {
        let last = lists
            .last()
            .ok_or(format!("{side} sent no minimap list with both avatars"))?;
        for list in &lists {
            if list.location.iter().any(|location| location.y != 128) {
                return Err(format!("a {side} minimap dot left row 128"));
            }
            if !(127..=128).contains(&list.location[0].x) {
                return Err(format!("{side} moved the owner's minimap dot"));
            }
        }
        if last.location[1].x < 130 {
            return Err(format!(
                "{side}'s last minimap list has the walker at cell {}, not two cells east of 128",
                last.location[1].x
            ));
        }
    }
    Ok(format!(
        "minimap bytes: {} CoarseLocationUpdate, unreliable and unencoded, carry the agents, You and Prey of the C# capture; every dot is on row 128 and the walker ends two or more cells east; known differences: Z (0 here, 6 in C#: the home store has no terrain row), the owner's X (127 in C#: the newcomer's capsule, stacked by the landing ray cast, pushes it 2 cm west), the walker's last cell (where the walk stops) and the number of lists (one every 50 frames)",
        ferroan.len()
    ))
}

fn body<M: Msg>(message: &M) -> Vec<u8> {
    let mut writer = Writer::new();
    message.encode_body(&mut writer);
    writer.buf
}

/// A redacted update with the local id and Z masked. Z differs for two
/// reasons: the Ferroan home store has no terrain row, and C#'s landing ray
/// cast stacks the second avatar on the first, which the stand-in does not. Ferroan's block must
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

/// The name replies for one id. Each must carry that one block.
fn replies_for(names: &[Seen<UUIDNameReply>], id: Uuid) -> Vec<&Seen<UUIDNameReply>> {
    names
        .iter()
        .filter(|seen| {
            seen.message
                .uuid_name_block
                .iter()
                .any(|block| block.id == id)
        })
        .collect()
}

/// `UUIDNameReply` for the other bot on each circuit and for the unknown id
/// once its miss is cached, against `fixtures/avatars/names.cap`: one reply
/// each, the C# body once redacted, the C# header flags, and the account
/// names (or `Unknown` and the id) raw.
fn compare_names(
    csharp: &AvatarPackets,
    ferroan: &AvatarPackets,
    redacted: &AvatarPackets,
    agents: [(Uuid, (&str, &str)); 2],
) -> Result<usize, String> {
    let unknown = UNKNOWN_NAME_ID.to_string();
    let mut compared = 0;
    for (id, names) in [
        agents[0],
        agents[1],
        (UNKNOWN_NAME_ID, ("Unknown", unknown.as_str())),
    ] {
        // The second bot's first request for the unknown id can come after
        // the owner's miss is cached, and is then answered too (TB89).
        let most = if id == UNKNOWN_NAME_ID { 2 } else { 1 };
        let reference = replies_for(&csharp.names, id);
        let Some(reference) = reference.first().filter(|_| reference.len() <= most) else {
            return Err(format!(
                "the C# capture has {} UUIDNameReply for {id}, not 1 to {most}",
                reference.len()
            ));
        };
        let sent = replies_for(&ferroan.names, id);
        let redacted_sent = replies_for(&redacted.names, id);
        if sent.is_empty() || sent.len() > most {
            return Err(format!(
                "Ferroan sent {} UUIDNameReply for {id}, not 1 to {most}",
                sent.len()
            ));
        }
        if redacted_sent.len() != sent.len() {
            return Err("redaction changed which name replies the run holds".into());
        }
        for (sent, redacted_sent) in sent.iter().zip(&redacted_sent) {
            let block = &sent.message.uuid_name_block;
            let want = (str_field(names.0), str_field(names.1));
            if block.len() != 1 || (block[0].first_name.clone(), block[0].last_name.clone()) != want
            {
                return Err(format!(
                    "Ferroan's UUIDNameReply for {id} does not carry {names:?}"
                ));
            }
            if sent.flags & 0xc0 != reference.flags & 0xc0 {
                return Err(format!(
                    "the UUIDNameReply header for {id} differs from the C# capture"
                ));
            }
            if body(&redacted_sent.message) != body(&reference.message) {
                return Err(format!(
                    "the UUIDNameReply for {id} differs from the C# capture"
                ));
            }
            compared += 1;
        }
    }
    Ok(compared)
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
    let named =
        super::login_capture::records(&crate::repo_root().join("fixtures/avatars/names.cap"))?;
    let named = avatar_packets(&named, &agents)?;
    let names = compare_names(
        &named,
        &ferroan,
        &ferroan_redacted,
        [(owner, ("Test", "User")), (chat, ("Chat", "Bot"))],
    )?;
    if ferroan.kills.iter().any(|seen| {
        seen.message
            .object_data
            .iter()
            .any(|block| block.id == local_ids[0])
    }) {
        return Err("Ferroan killed the owner's avatar while it stayed".into());
    }
    let minimap = compare_coarse(
        &csharp.coarse,
        &ferroan_redacted.coarse,
        &ferroan.coarse,
        (&coarse_headers(&fixture), &coarse_headers(&live)),
        owner,
        chat,
    )?;
    Ok(format!(
        "{minimap}\navatar bytes: {} full ObjectUpdate, {} AvatarAppearance and {} KillObject match the C# capture with local ids masked, and {names} UUIDNameReply (each bot's name, and the unknown id's cached miss) match fixtures/avatars/names.cap byte for byte; Z and the collision plane are folded (absolute Z: the home store has no terrain row; relative Z: C#'s landing ray cast stacks the newcomer on the other avatar and the stand-in has none; plane: the stand-in sets none); known differences: C# can also send the kill to the leaving viewer after its LogoutReply (TB87, racing the circuit close; {} kill(s) in the C# capture), and AvatarAnimation is P2.3b1",
        ferroan.fulls.len(),
        ferroan.appearances.len(),
        kills.len(),
        csharp.kills.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed C# run: both bots' full updates and appearances, the
    /// kill of the second bot, and the minimap lists around its walk.
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
        // The owner's kill of the second bot. C#'s kill to the leaving viewer
        // after its LogoutReply (TB87) races the bot closing its circuit and
        // is not in this recording.
        assert_eq!(found.kills.len(), 1);
        for kill in &found.kills {
            assert_eq!(kill.flags & 0xc0, PacketFlags::RELIABLE);
            assert_eq!(kill.message.object_data.len(), 1);
            assert_eq!(kill.message.object_data[0].id, chat_id);
        }
        for full in &found.fulls {
            assert_eq!(full.flags & 0xc0, 0xc0);
        }
        // Two lists of both avatars to each viewer, then the owner alone.
        assert!(
            coarse_headers(&records)
                .iter()
                .all(|header| *header == (0, 0))
        );
        let lists: Vec<_> = found
            .coarse
            .iter()
            .map(|seen| {
                let message = &seen.message;
                (
                    message.agent_data.len(),
                    message.index.you,
                    message.index.prey,
                )
            })
            .collect();
        assert_eq!(
            lists,
            [(2, 0, -1), (2, 1, -1), (2, 0, -1), (2, 1, -1), (1, 0, -1)]
        );
        let cells: Vec<_> = found.coarse[2]
            .message
            .location
            .iter()
            .map(|location| (location.x, location.y, location.z))
            .collect();
        assert_eq!(cells, [(127, 128, 6), (132, 128, 6)]);
        assert!(compare_coarse(&found.coarse, &found.coarse, &[], (&[], &[]), owner, chat).is_ok());
    }

    /// The compare fails when a list carries the wrong `You`, a reliable
    /// header, or a Z-scaled dot off its row.
    #[test]
    fn the_minimap_compare_catches_a_wrong_index_header_or_row() {
        let records = super::super::login_capture::records(
            &crate::repo_root().join("fixtures/avatars/avatars.cap"),
        )
        .unwrap();
        let owner = Uuid::parse_str(crate::live::ESTATE_OWNER).unwrap();
        let chat = Uuid::parse_str(crate::live::CHAT_USER).unwrap();
        let found = avatar_packets(&records, &[owner, chat]).unwrap();
        let mut wrong = found
            .coarse
            .iter()
            .map(|seen| Seen {
                flags: seen.flags,
                message: seen.message.clone(),
            })
            .collect::<Vec<_>>();
        wrong[1].message.index.you = 0;
        wrong[3].message.index.you = 0;
        assert!(compare_coarse(&found.coarse, &wrong, &[], (&[], &[]), owner, chat).is_err());
        assert!(
            compare_coarse(
                &found.coarse,
                &found.coarse,
                &[],
                (&[], &[(0x40, 0)]),
                owner,
                chat
            )
            .is_err()
        );
        let mut swapped = found.coarse[0].message.clone();
        swapped.agent_data = vec![
            ferroan_protocol::msgs::coarse_location_update::AgentData { agent_id: chat },
            ferroan_protocol::msgs::coarse_location_update::AgentData { agent_id: owner },
        ];
        let swapped = [Seen {
            flags: 0,
            message: swapped,
        }];
        assert!(
            compare_coarse(
                &found.coarse,
                &found.coarse,
                &swapped,
                (&[], &[]),
                owner,
                chat
            )
            .is_err()
        );
        let mut off_row = found
            .coarse
            .iter()
            .map(|seen| Seen {
                flags: seen.flags,
                message: seen.message.clone(),
            })
            .collect::<Vec<_>>();
        off_row[2].message.location[1].y = 129;
        assert!(compare_coarse(&found.coarse, &off_row, &[], (&[], &[]), owner, chat).is_err());
    }

    /// A frame boundary between the second bot's UseCircuitCode and its
    /// arrival sends its viewer the owner alone with `You` -1. That list is
    /// allowed on either side and is not a fourth kind.
    #[test]
    fn a_list_before_the_arrival_is_not_a_kind() {
        let records = super::super::login_capture::records(
            &crate::repo_root().join("fixtures/avatars/avatars.cap"),
        )
        .unwrap();
        let owner = Uuid::parse_str(crate::live::ESTATE_OWNER).unwrap();
        let chat = Uuid::parse_str(crate::live::CHAT_USER).unwrap();
        let found = avatar_packets(&records, &[owner, chat]).unwrap();
        let alone = found
            .coarse
            .iter()
            .find(|seen| seen.message.agent_data.len() == 1)
            .unwrap();
        let mut early = alone.message.clone();
        early.index.you = -1;
        let mut ferroan = vec![Seen {
            flags: 0,
            message: early.clone(),
        }];
        ferroan.extend(found.coarse.iter().map(|seen| Seen {
            flags: seen.flags,
            message: seen.message.clone(),
        }));
        let mut raw_early = early;
        raw_early.agent_data[0].agent_id = owner;
        let raw = [Seen {
            flags: 0,
            message: raw_early,
        }];
        assert!(compare_coarse(&ferroan, &ferroan, &raw, (&[], &[]), owner, chat).is_ok());
        assert!(compare_coarse(&found.coarse, &ferroan, &raw, (&[], &[]), owner, chat).is_ok());
        assert!(compare_coarse(&ferroan, &found.coarse, &[], (&[], &[]), owner, chat).is_ok());
    }

    /// One `UUIDNameReply` for each bot, on the other's circuit, and one for
    /// the unknown id's second request; none for its first. Each is one
    /// redacted block, reliable and not zero-coded.
    #[test]
    fn the_csharp_capture_names_each_bot_and_the_cached_miss() {
        let records = super::super::login_capture::records(
            &crate::repo_root().join("fixtures/avatars/names.cap"),
        )
        .unwrap();
        let owner = Uuid::parse_str(crate::live::ESTATE_OWNER).unwrap();
        let chat = Uuid::parse_str(crate::live::CHAT_USER).unwrap();
        let found = avatar_packets(&records, &[owner, chat]).unwrap();
        assert_eq!(found.names.len(), 3);
        for id in [owner, chat, UNKNOWN_NAME_ID] {
            let [seen] = replies_for(&found.names, id)[..] else {
                panic!("not one reply for {id}");
            };
            assert_eq!(seen.flags & 0xc0, PacketFlags::RELIABLE);
            let block = &seen.message.uuid_name_block;
            assert_eq!(block.len(), 1);
            // The redactor keeps C#'s placeholder for an unknown id (TB89).
            let (first, last) = if id == UNKNOWN_NAME_ID {
                (str_field("Unknown"), str_field(&id.to_string()))
            } else {
                (b"Redacted".to_vec(), b"Redacted".to_vec())
            };
            assert_eq!(block[0].first_name, first);
            assert_eq!(block[0].last_name, last);
        }
    }

    /// The compare refuses a reply with other names, a second reply, and a
    /// missing one.
    #[test]
    fn the_name_compare_catches_a_wrong_or_missing_reply() {
        let records = super::super::login_capture::records(
            &crate::repo_root().join("fixtures/avatars/names.cap"),
        )
        .unwrap();
        let owner = Uuid::parse_str(crate::live::ESTATE_OWNER).unwrap();
        let chat = Uuid::parse_str(crate::live::CHAT_USER).unwrap();
        let csharp = avatar_packets(&records, &[owner, chat]).unwrap();
        let agents = [(owner, ("Test", "User")), (chat, ("Chat", "Bot"))];
        let unknown = UNKNOWN_NAME_ID.to_string();
        let live = |names: &[(Uuid, &str, &str)]| AvatarPackets {
            names: names
                .iter()
                .map(|(id, first, last)| Seen {
                    flags: PacketFlags::RELIABLE,
                    message: UUIDNameReply {
                        uuid_name_block: vec![
                            ferroan_protocol::msgs::uuid_name_reply::UUIDNameBlock {
                                id: *id,
                                first_name: str_field(first),
                                last_name: str_field(last),
                            },
                        ],
                        ..UUIDNameReply::default()
                    },
                })
                .collect(),
            ..AvatarPackets::default()
        };
        let redact = |packets: &AvatarPackets| AvatarPackets {
            names: packets
                .names
                .iter()
                .map(|seen| {
                    let mut message = seen.message.clone();
                    for block in &mut message.uuid_name_block {
                        if block.id != UNKNOWN_NAME_ID {
                            block.first_name = b"Redacted".to_vec();
                            block.last_name = b"Redacted".to_vec();
                        }
                    }
                    Seen {
                        flags: seen.flags,
                        message,
                    }
                })
                .collect(),
            ..AvatarPackets::default()
        };
        let good = [
            (owner, "Test", "User"),
            (chat, "Chat", "Bot"),
            (UNKNOWN_NAME_ID, "Unknown", unknown.as_str()),
        ];
        let run = live(&good);
        assert_eq!(compare_names(&csharp, &run, &redact(&run), agents), Ok(3));
        let wrong = live(&[good[0], (chat, "Other", "Name"), good[2]]);
        assert!(compare_names(&csharp, &wrong, &redact(&wrong), agents).is_err());
        let twice = live(&[good[0], good[1], good[1], good[2]]);
        // Two for the unknown id is the cached-miss race, and passes.
        let unknown_twice = live(&[good[0], good[1], good[2], good[2]]);
        assert_eq!(
            compare_names(&csharp, &unknown_twice, &redact(&unknown_twice), agents),
            Ok(4)
        );
        assert!(compare_names(&csharp, &twice, &redact(&twice), agents).is_err());
        let other_unknown = live(&[good[0], good[1], (UNKNOWN_NAME_ID, "Unknown", "User")]);
        assert!(compare_names(&csharp, &other_unknown, &redact(&other_unknown), agents).is_err());
        let missing = live(&good[..2]);
        assert!(compare_names(&csharp, &missing, &redact(&missing), agents).is_err());
    }
}
