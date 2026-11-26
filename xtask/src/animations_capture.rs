// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The walker's `AvatarAnimation` packets from a Ferroan `animations` run
//! against the C# capture in `fixtures/animations/animations.cap`.
//!
//! Both captures must re-encode as they are before anything is read from
//! them. The capture does not record which viewer a datagram went to, and
//! here both viewers get the same bytes: every animation names the agent or
//! nothing, so the walker's own viewer gets no objects either
//! (`LLClientView.SendAnimations`). The packs are taken in capture order,
//! each run of equal packs folded with its length (one per viewer), and the
//! two lists must match: the body byte for byte, and of the header the
//! first byte (flags) and the extra-header length. The packet sequence
//! number and appended acks are each circuit's own and are not compared.
//! C# starts the animation sequence numbers from a clock
//! (`NextAnimationSequenceNumber`), so every number but a new set's 1 is
//! moved by one offset for the whole capture, its smallest number other
//! than 1, before the bytes are compared. That keeps every equality and
//! step a viewer reads. `Sender.ID` is not redacted.

use std::path::Path;

use ferroan_capture::{Kind, Record, check_udp};
use ferroan_protocol::msgs::AvatarAnimation;
use ferroan_protocol::{Msg, PacketFlags, Writer, parse_packet};
use uuid::Uuid;

/// The first header byte of `AvatarAnimationHeader`: reliable only.
const RELIABLE_ONLY: u8 = PacketFlags::RELIABLE;

/// One run of equal packs: the first header byte, the extra-header length,
/// the body with relative sequence numbers (and its animation list), and how
/// many came in a row.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    flags: u8,
    extra: usize,
    body: Vec<u8>,
    list: Vec<(Uuid, i32)>,
    count: usize,
}

/// The walker's packs, unresent, in capture order, folded into runs.
fn runs(records: &[Record], walker: Uuid) -> Result<Vec<Run>, String> {
    let mut packs = Vec::new();
    for record in records.iter().filter(|record| record.kind == Kind::UdpOut) {
        let Ok(packet) = parse_packet(&record.bytes) else {
            continue;
        };
        if packet.id != AvatarAnimation::ID || packet.flags & PacketFlags::RESENT != 0 {
            continue;
        }
        let message = packet
            .decode::<AvatarAnimation>()
            .map_err(|_| "an AvatarAnimation did not decode".to_string())?;
        if message.sender.id == walker {
            packs.push((packet.flags, packet.extra.len(), message));
        }
    }
    let base = packs
        .iter()
        .flat_map(|(_, _, message)| &message.animation_list)
        .map(|block| block.anim_sequence_id)
        .filter(|sequence| *sequence != 1)
        .min()
        .unwrap_or(0);
    let mut out: Vec<Run> = Vec::new();
    for (flags, extra, mut message) in packs {
        for block in &mut message.animation_list {
            if block.anim_sequence_id != 1 {
                block.anim_sequence_id = block.anim_sequence_id.wrapping_sub(base).wrapping_add(2);
            }
        }
        let mut writer = Writer::new();
        message.encode_body(&mut writer);
        let run = Run {
            flags,
            extra,
            body: writer.buf,
            list: message
                .animation_list
                .iter()
                .map(|block| (block.anim_id, block.anim_sequence_id))
                .collect(),
            count: 1,
        };
        match out.last_mut() {
            Some(last)
                if last.flags == run.flags && last.extra == run.extra && last.body == run.body =>
            {
                last.count += 1;
            }
            _ => out.push(run),
        }
    }
    Ok(out)
}

/// Ferroan's packs for the walker against C#'s.
pub(super) fn compare_animations(directory: &Path) -> Result<String, String> {
    let live = super::login_capture::records(&directory.join("capture-udp.cap"))?;
    check_udp(&live).map_err(|error| format!("the Ferroan capture: {error}"))?;
    let fixture = super::login_capture::records(
        &crate::repo_root().join("fixtures/animations/animations.cap"),
    )?;
    check_udp(&fixture).map_err(|error| format!("the C# capture: {error}"))?;
    let walker = Uuid::parse_str(crate::live::ESTATE_OWNER)
        .map_err(|_| "estate owner is not a uuid".to_string())?;
    let csharp = runs(&fixture, walker)?;
    let ferroan = runs(&live, walker)?;
    if csharp.len() != 5 {
        return Err(format!(
            "the C# capture has {} runs of walker animation packs, not 5",
            csharp.len()
        ));
    }
    if let Some(run) = ferroan
        .iter()
        .find(|run| run.flags & 0xf0 != RELIABLE_ONLY || run.extra != 0)
    {
        return Err(format!(
            "a Ferroan AvatarAnimation header is {:#04x} with {} extra bytes, not reliable, unencoded and plain",
            run.flags, run.extra
        ));
    }
    if ferroan != csharp {
        let shape = |runs: &[Run]| -> Vec<(usize, usize)> {
            runs.iter().map(|run| (run.body.len(), run.count)).collect()
        };
        return Err(format!(
            "the walker's AvatarAnimation packs differ from the C# capture (body length and count per run: Ferroan {:?}, C# {:?})",
            shape(&ferroan),
            shape(&csharp)
        ));
    }
    let total: usize = ferroan.iter().map(|run| run.count).sum();
    Ok(format!(
        "animation bytes: {total} AvatarAnimation for the walker (STAND 1, WALK, STAND, the started animation and its stop, each run {:?} long) match the C# capture: bodies byte for byte, and the first header byte and extra-header length, with each animation sequence number but 1 moved by one offset for the whole capture (C# starts them from a clock)",
        ferroan.iter().map(|run| run.count).collect::<Vec<_>>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed C# run re-encodes, and holds the five steps the bot
    /// checks, each to both viewers.
    #[test]
    fn the_csharp_capture_has_the_five_steps_to_both_viewers() {
        let records = super::super::login_capture::records(
            &crate::repo_root().join("fixtures/animations/animations.cap"),
        )
        .unwrap();
        check_udp(&records).unwrap();
        let walker = Uuid::parse_str(crate::live::ESTATE_OWNER).unwrap();
        let found = runs(&records, walker).unwrap();
        assert_eq!(
            found.iter().map(|run| run.count).collect::<Vec<_>>(),
            [2, 2, 2, 2, 2]
        );
        for run in &found {
            assert_eq!((run.flags & 0xf0, run.extra), (RELIABLE_ONLY, 0));
        }
        let lists: Vec<Vec<(Uuid, i32)>> = found.iter().map(|run| run.list.clone()).collect();
        assert_eq!(
            ferroan_bot::scenario::animation_steps(&lists),
            Ok(2),
            "{lists:?}"
        );
    }

    /// Runs fold repeats only when they are next to each other, and the
    /// sequence numbers are written relative to the smallest one but 1.
    #[test]
    fn repeats_fold_and_numbers_are_relative() {
        let walker = Uuid::from_u128(5);
        let pack = |list: &[(u128, i32)], sequence: u32| -> Record {
            let message = AvatarAnimation {
                sender: ferroan_protocol::msgs::avatar_animation::Sender { id: walker },
                animation_list: list
                    .iter()
                    .map(
                        |(id, sequence)| ferroan_protocol::msgs::avatar_animation::AnimationList {
                            anim_id: Uuid::from_u128(*id),
                            anim_sequence_id: *sequence,
                        },
                    )
                    .collect(),
                omitted_trailing_blocks: 0,
                ..Default::default()
            };
            let mut bytes = Vec::new();
            ferroan_protocol::build_packet(&message, sequence, true, false, &[], &mut bytes);
            Record {
                kind: Kind::UdpOut,
                time_ms: 0,
                bytes,
            }
        };
        let records = [
            pack(&[(1, 1)], 1),
            pack(&[(1, 1)], 2),
            pack(&[(2, 900)], 3),
            pack(&[(1, 901)], 4),
            pack(&[(2, 900)], 5),
        ];
        let found = runs(&records, walker).unwrap();
        assert_eq!(
            found.iter().map(|run| run.count).collect::<Vec<_>>(),
            [2, 1, 1, 1]
        );
        assert_eq!(found[1].body, found[3].body);
        let shifted = [
            pack(&[(1, 1)], 1),
            pack(&[(1, 1)], 2),
            pack(&[(2, 7)], 3),
            pack(&[(1, 8)], 4),
            pack(&[(2, 7)], 5),
        ];
        assert_eq!(runs(&shifted, walker).unwrap(), found);
        assert!(runs(&records, Uuid::from_u128(6)).unwrap().is_empty());
    }
}
