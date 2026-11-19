// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every packet fixture from the OpenMetaverse oracle round-trips through the
//! typed message structs and the Tranquillity framing rules.

// Integration tests are a test crate; the production lints stay on the library.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

use ferroan_protocol::{parse_packet, reencode_datagram};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oracle")
}

#[test]
fn viewer_extensions_preserve_the_openmetaverse_prefix() {
    let root = fixtures();
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
    let mut seen = 0;
    for case in manifest["cases"].as_array().unwrap() {
        if case["family"].as_str() != Some("packet-extension") {
            continue;
        }
        seen += 1;
        let id = case["id"].as_str().unwrap();
        let bytes = fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        assert_eq!(reencode_datagram(&bytes).unwrap(), bytes, "{id}");
        let name = id.strip_prefix("packet-extension/").unwrap();
        let old_bytes = fs::read(root.join(format!("packet/{name}.bin"))).unwrap();
        let old = parse_packet(&old_bytes).unwrap();
        let extended = parse_packet(&bytes).unwrap();
        assert_eq!(extended.id, old.id, "{id}");
        assert!(extended.body.len() > old.body.len(), "{id}");
        assert_eq!(&extended.body[..old.body.len()], old.body, "{id}");
    }
    assert_eq!(seen, 8);
}

#[test]
fn every_packet_matches_openmetaverse() {
    let root = fixtures();
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
    let mut seen = 0u32;
    for case in manifest["cases"].as_array().unwrap() {
        if case["family"].as_str() != Some("packet") {
            continue;
        }
        seen += 1;
        let id = case["id"].as_str().unwrap();
        let file = root.join(case["file"].as_str().unwrap());
        let bytes = fs::read(&file).unwrap();
        let again = reencode_datagram(&bytes).unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(again, bytes, "{id}");
    }
    assert_eq!(seen, 478, "saw {seen} packet fixtures");
}
