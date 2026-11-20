// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent byte suffixes from Second Life viewer's message template at
//! ff75ab037607d8fe305f43f8b1f43126693ba2db, appended to pinned C# oracle bodies.
//! These constructed suffixes test layout; the separate captured IM proves
//! Tranquillity's sender shape.

// Integration tests are a test crate; the production lints stay on the library.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ferroan_protocol::msgs::{
    AlertMessage, AvatarAppearance, ChildAgentUpdate, ImprovedInstantMessage, InviteGroupResponse,
    RegionHandshake, RegionInfo, SimStatus, TransferInventory, improved_instant_message,
};
use ferroan_protocol::{Msg, Writer, parse_packet, reencode_body};

fn oracle_body(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/oracle/packet")
        .join(format!("{name}.bin"));
    parse_packet(&std::fs::read(path).unwrap()).unwrap().body
}

fn encoded<M: Msg>(message: &M) -> Vec<u8> {
    let mut writer = Writer::new();
    message.encode_body(&mut writer);
    writer.buf
}

macro_rules! suffix_cases {
    ($test:ident, $message:ident, $absent:literal, [$($suffix:expr),+ $(,)?]) => {
        #[test]
        fn $test() {
            let base = oracle_body(stringify!($message));
            let message = $message::decode(&base).unwrap();
            assert_eq!(message.omitted_trailing_blocks, $absent);
            assert_eq!(encoded(&message), base);
            $(
                let suffix: Vec<u8> = $suffix;
                let mut bytes = base.clone();
                bytes.extend_from_slice(&suffix);
                let message = $message::decode(&bytes).unwrap();
                assert_eq!(message.omitted_trailing_blocks, 0);
                assert_eq!(encoded(&message), bytes);
                assert_eq!(reencode_body($message::ID, &bytes).unwrap(), bytes);
            )+
        }
    };
}

suffix_cases!(
    alert_agent_info_absent_empty_and_present,
    AlertMessage,
    1,
    [vec![0], [&[1][..], &[0x17; 16]].concat()]
);
suffix_cases!(
    appearance_attachments_absent_empty_and_present,
    AvatarAppearance,
    1,
    [vec![0], [&[1][..], &[0x18; 16], &[7]].concat()]
);
suffix_cases!(
    child_inventory_host_absent_empty_and_present,
    ChildAgentUpdate,
    1,
    [vec![0], vec![1, 3, b'h', b'o', b's']]
);
suffix_cases!(
    im_single_and_metadata_absent_empty_and_present,
    ImprovedInstantMessage,
    2,
    [
        vec![0x12, 0x34, 0x56, 0x78, 0],
        vec![0x12, 0x34, 0x56, 0x78, 1, 3, 0, 9, 8, 7]
    ]
);
suffix_cases!(
    invite_group_limit_single_absent_and_present,
    InviteGroupResponse,
    1,
    [vec![0x12, 0x34, 0x56, 0x78]]
);
suffix_cases!(
    sim_flags_single_absent_and_present,
    SimStatus,
    1,
    [vec![0x12, 0x34, 0x56, 0x78, 0x11, 0x22, 0x33, 0x44]]
);
suffix_cases!(
    inventory_validation_single_absent_and_present,
    TransferInventory,
    1,
    [vec![1, 0x12, 0x34, 0x56, 0x78]]
);
suffix_cases!(
    region_chat_and_combat_absent_empty_and_present,
    RegionInfo,
    2,
    [vec![0, 0], [&[1][..], &[0; 28], &[1], &[0; 21]].concat()]
);

#[test]
fn im_estate_without_metadata_matches_pinned_sender_layout() {
    let base = oracle_body("ImprovedInstantMessage");
    let mut bytes = base.clone();
    bytes.extend_from_slice(&[0; 4]);
    let message = ImprovedInstantMessage::decode(&bytes).unwrap();
    assert_eq!(message.omitted_trailing_blocks, 1);
    assert_eq!(message.estate_block.estate_id, 0);
    assert_eq!(encoded(&message), bytes);
    assert_eq!(ImprovedInstantMessage::default().omitted_trailing_blocks, 1);
}

#[test]
fn optional_singles_ignore_partially_present_bytes() {
    for (name, id, width) in [
        ("ImprovedInstantMessage", ImprovedInstantMessage::ID, 4),
        ("InviteGroupResponse", InviteGroupResponse::ID, 4),
        ("SimStatus", SimStatus::ID, 8),
        ("TransferInventory", TransferInventory::ID, 5),
    ] {
        let base = oracle_body(name);
        for present in 1..width {
            let mut bytes = base.clone();
            bytes.resize(bytes.len() + present, 0x5a);
            assert_eq!(
                reencode_body(id, &bytes).unwrap(),
                base,
                "{name}: {present}/{width}"
            );
        }
    }
    // A partial Variable block is still an error. RegionHandshake's trailing
    // RegionInfo4 is a count plus two U64s. A count with only part of that
    // payload fails. A count and nothing else is a zero-filled field at EOF,
    // which Reader::take still accepts.
    let full = oracle_body("RegionHandshake");
    let mut partial = full[..full.len() - 17].to_vec();
    partial.push(1);
    partial.extend_from_slice(&[0, 0, 0, 0]);
    assert!(reencode_body(RegionHandshake::ID, &partial).is_err());
}

#[test]
fn populated_trailing_blocks_are_kept_and_only_an_empty_suffix_is_omitted() {
    let kept = ImprovedInstantMessage {
        omitted_trailing_blocks: 2,
        estate_block: improved_instant_message::EstateBlock { estate_id: 7 },
        meta_data: vec![improved_instant_message::MetaData {
            data: b"x".to_vec(),
        }],
        ..ImprovedInstantMessage::default()
    };
    let bytes = encoded(&kept);
    let decoded = ImprovedInstantMessage::decode(&bytes).unwrap();
    assert_eq!(decoded.estate_block.estate_id, 7);
    assert_eq!(decoded.meta_data[0].data, b"x");
    assert_eq!(encoded(&decoded), bytes);

    // default() keeps the four zero EstateBlock bytes and omits empty
    // MetaData. Asking to omit two trailing blocks drops that empty estate
    // too. The oracle prefix is a populated sender body, so it is not the
    // empty encoding.
    let both_empty = ImprovedInstantMessage {
        omitted_trailing_blocks: 2,
        ..ImprovedInstantMessage::default()
    };
    let shorter = encoded(&both_empty);
    let decoded_empty = ImprovedInstantMessage::decode(&shorter).unwrap();
    assert_eq!(decoded_empty.omitted_trailing_blocks, 2);
    assert_eq!(decoded_empty.estate_block.estate_id, 0);
    assert!(decoded_empty.meta_data.is_empty());
    let mut with_estate = shorter.clone();
    with_estate.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(encoded(&ImprovedInstantMessage::default()), with_estate);
}

#[test]
fn existing_handshake_count_absent_empty_and_populated_remains_lossless() {
    let full = oracle_body("RegionHandshake");
    let base = &full[..full.len() - 17]; // oracle: count1, two U64s
    let message = RegionHandshake::decode(base).unwrap();
    assert_eq!(message.omitted_trailing_blocks, 1);
    assert!(message.region_info4.is_empty());
    assert_eq!(encoded(&message), base);
    let mut empty = base.to_vec();
    empty.push(0);
    let message = RegionHandshake::decode(&empty).unwrap();
    assert_eq!(message.omitted_trailing_blocks, 0);
    assert!(message.region_info4.is_empty());
    assert_eq!(encoded(&message), empty);
    assert_eq!(reencode_body(RegionHandshake::ID, &full).unwrap(), full);
}

#[test]
fn omission_count_cannot_remove_required_blocks() {
    let base = oracle_body("ImprovedInstantMessage");
    let mut message = ImprovedInstantMessage::decode(&base).unwrap();
    message.omitted_trailing_blocks = usize::MAX;
    assert_eq!(encoded(&message), base);
}

#[test]
fn consecutive_variable_tail_counts_are_preserved_individually() {
    let base = oracle_body("RegionInfo");
    let mut first_empty = base.clone();
    first_empty.push(0);
    let message = RegionInfo::decode(&first_empty).unwrap();
    assert_eq!(message.omitted_trailing_blocks, 1);
    assert_eq!(encoded(&message), first_empty);
    let mut both_empty = first_empty;
    both_empty.push(0);
    assert_eq!(
        reencode_body(RegionInfo::ID, &both_empty).unwrap(),
        both_empty
    );
}
