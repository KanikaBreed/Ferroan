// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Asset XML compared with `ServerUtils.SerializeResult` output.

// Integration tests are a test crate; the production lints stay on the library.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use ferroan_services::{
    Asset, content_type, decode_asset, decode_metadata_type, encode_asset, encode_bool,
    encode_bools, encode_metadata, encode_string, encode_strings,
};
use serde_json::Value;
use uuid::Uuid;

#[test]
fn asset_documents_match_the_csharp_serializer() {
    let root = fixture_root();
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("manifest.json")).expect("manifest"),
    )
    .expect("manifest json");
    assert_eq!(manifest["format"], 1);
    assert_eq!(
        manifest["tranquillity"].as_str(),
        Some("1052cbae82ae7dc3b8abfcffe33e316498eeaad9")
    );
    let cases = manifest["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let path = root.join(case["file"].as_str().unwrap());
        let expected = std::fs::read(&path).unwrap_or_else(|_| panic!("missing {id}"));
        let encoded = match case["kind"].as_str() {
            Some("asset") => encode_asset(&asset_from_case(case)),
            Some("metadata") => encode_metadata(&asset_from_case(case)),
            Some("scalar") => encode_scalar(id),
            other => panic!("{id}: unexpected kind {other:?}"),
        };
        assert_eq!(
            encoded,
            expected,
            "{id}\n{}",
            String::from_utf8_lossy(&encoded)
        );
        if case["kind"] == "asset" {
            let decoded = decode_asset(&expected).unwrap_or_else(|error| panic!("{id}: {error}"));
            let asset = asset_from_case(case);
            assert_eq!(decoded.asset.id, asset.id);
            assert_eq!(decoded.id_text, asset.id.to_string());
            assert_eq!(decoded.asset.name, asset.name);
            assert_eq!(decoded.asset.description, asset.description);
            assert_eq!(decoded.asset.type_code, asset.type_code);
            assert_eq!(decoded.asset.local, asset.local);
            assert_eq!(decoded.asset.temporary, asset.temporary);
            assert_eq!(decoded.asset.creator_id, asset.creator_id);
            assert_eq!(decoded.asset.flags, asset.flags);
            assert_eq!(decoded.asset.data, asset.data);
            assert!(!decoded.data_nil);
        }
        if case["kind"] == "metadata" {
            let (id_value, type_code, id_text) =
                decode_metadata_type(&expected).unwrap_or_else(|error| panic!("{id}: {error}"));
            let asset = asset_from_case(case);
            assert_eq!(id_value, asset.id);
            assert_eq!(type_code, asset.type_code);
            assert_eq!(id_text, asset.id.to_string());
        }
    }
}

#[test]
fn content_types_match_slutil() {
    let text = std::fs::read_to_string(fixture_root().join("content-types.json")).unwrap();
    let types: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
    for (key, value) in types {
        let code: i8 = key.parse().unwrap();
        assert_eq!(content_type(code), value.as_str().unwrap(), "{key}");
    }
}

/// TB1. `StoredAssetFlags.FromStored` masks flags 8, and flags 9 (Maptile
/// mixed with that unnamed bit). `XmlSerializer` then accepts both.
/// `flags_text` emits `Normal` and `Maptile`.
#[test]
fn csharp_unnamed_asset_flags_are_masked() {
    let root = fixture_root();
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("manifest.json")).expect("manifest"),
    )
    .expect("manifest json");
    let record: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("unnamed-flags.json")).expect("unnamed-flags"),
    )
    .expect("unnamed-flags json");
    assert_eq!(record["id"], "unnamed-flags");
    assert_eq!(record["flags"], 8);
    assert_eq!(record["known_flags"], 0);
    assert_eq!(record["threw"], false);
    assert_eq!(record["exception_type"], "");
    assert_eq!(record["tranquillity"], manifest["tranquillity"]);
    assert!(
        manifest["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .all(|case| case["id"] != "unnamed-flags"),
        "unnamed-flags is a mask record, not a manifest case"
    );
    let encoded = encode_asset(&Asset {
        id: Uuid::parse_str("33333333-4444-5555-6666-777777777777").unwrap(),
        name: "unnamed-flags".to_string(),
        description: String::new(),
        type_code: 0,
        local: false,
        temporary: false,
        creator_id: "creator".to_string(),
        flags: 8,
        data: Vec::new(),
        upload_attempts: 0,
    });
    let text = String::from_utf8(encoded).unwrap();
    assert!(
        text.contains("<Flags>Normal</Flags>"),
        "TB1: flags_text still emits unnamed bits. Change crates/ferroan-services/src/asset/xml.rs flags_text. {text}"
    );

    let mixed: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("mixed-flags.json")).expect("mixed-flags"),
    )
    .expect("mixed-flags json");
    assert_eq!(mixed["id"], "mixed-flags");
    assert_eq!(mixed["flags"], 9);
    assert_eq!(mixed["known_flags"], 1);
    assert_eq!(mixed["threw"], false);
    assert_eq!(mixed["exception_type"], "");
    assert_eq!(mixed["tranquillity"], manifest["tranquillity"]);
    assert!(
        manifest["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .all(|case| case["id"] != "mixed-flags" && case["id"] != "unnamed-flags"),
        "flag probes are mask records, not manifest cases"
    );
    let mixed_text = String::from_utf8(encode_asset(&Asset {
        id: Uuid::parse_str("33333333-4444-5555-6666-777777777778").unwrap(),
        name: "mixed-flags".to_string(),
        description: String::new(),
        type_code: 0,
        local: false,
        temporary: false,
        creator_id: "creator".to_string(),
        flags: 9,
        data: Vec::new(),
        upload_attempts: 0,
    }))
    .unwrap();
    assert!(
        mixed_text.contains("<Flags>Maptile</Flags>"),
        "TB1: flags_text still emits unnamed bits mixed with named ones. Change crates/ferroan-services/src/asset/xml.rs flags_text. {mixed_text}"
    );
    assert_eq!(
        std::fs::read(root.join("asset/unnamed-flags.xml")).expect("unnamed xml"),
        encode_asset(&Asset {
            id: Uuid::parse_str("33333333-4444-5555-6666-777777777777").unwrap(),
            name: "unnamed-flags".to_string(),
            description: String::new(),
            type_code: 0,
            local: false,
            temporary: false,
            creator_id: "creator".to_string(),
            flags: 8,
            data: Vec::new(),
            upload_attempts: 0,
        })
    );
    assert_eq!(
        std::fs::read(root.join("asset/mixed-flags.xml")).expect("mixed xml"),
        mixed_text.into_bytes()
    );
}

fn encode_scalar(id: &str) -> Vec<u8> {
    let texture = "00112233-4455-6677-8899-aabbccddeeff";
    let zero = "00000000-0000-0000-0000-000000000000";
    match id {
        "bool-true" => encode_bool(true),
        "bool-false" => encode_bool(false),
        "string-id" => encode_string(texture),
        "bools" => encode_bools(&[true, false, true]),
        "ids" => encode_strings(&[texture, zero]),
        "bools-empty" => encode_bools(&[]),
        "ids-empty" => encode_strings(&[]),
        other => panic!("unknown scalar {other}"),
    }
}

fn asset_from_case(case: &Value) -> Asset {
    Asset {
        id: Uuid::parse_str(case["uuid"].as_str().unwrap()).unwrap(),
        name: case["name"].as_str().unwrap().to_string(),
        description: case["description"].as_str().unwrap().to_string(),
        type_code: i8::try_from(case["type"].as_i64().unwrap()).unwrap(),
        local: case["local"].as_bool().unwrap(),
        temporary: case["temporary"].as_bool().unwrap(),
        creator_id: case["creator_id"].as_str().unwrap().to_string(),
        flags: i32::try_from(case["flags"].as_i64().unwrap()).unwrap(),
        data: hex_decode(case["data_hex"].as_str().unwrap()),
        upload_attempts: 0,
    }
}

fn hex_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    assert!(bytes.len().is_multiple_of(2));
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let (pairs, _) = bytes.as_chunks::<2>();
    for pair in pairs {
        let hi = hex_nibble(pair[0]);
        let lo = hex_nibble(pair[1]);
        out.push((hi << 4) | lo);
    }
    out
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        other => panic!("hex {other}"),
    }
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/asset")
}
