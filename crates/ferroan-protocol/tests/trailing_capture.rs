// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! C# SendInstantMessage capture, not a Rust-generated expected datagram.

// Integration tests are a test crate; the production lints stay on the library.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ferroan_protocol::msgs::ImprovedInstantMessage;
use ferroan_protocol::{parse_packet, reencode_datagram};

#[test]
fn captured_csharp_im_keeps_estate_block_and_omits_metadata_count() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/trailing");
    let bytes = std::fs::read(root.join("improved-instant-message.bin")).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(bytes.len(), 191);
    assert_eq!(manifest["original_length"], 191);
    assert_eq!(manifest["libopenmetaverse_prefix_length"], 187);
    assert_eq!(manifest["metadata_count_present"], false);
    assert_eq!(&bytes[187..], &[0; 4]);
    let packet = parse_packet(&bytes).unwrap();
    assert!(packet.reliable());
    let message = packet.decode::<ImprovedInstantMessage>().unwrap();
    assert_eq!(message.omitted_trailing_blocks, 1);
    assert_eq!(reencode_datagram(&bytes).unwrap(), bytes);
}
