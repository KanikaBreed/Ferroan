// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use ferroan_scripting::{
    Failure, HEADER_STRING, MAX_NAME_LEN, OBJECT_BYTES, SOURCE_CHARS, read_header, source_hash,
    tokenize,
};
use sha2::{Digest, Sha256};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/yengine-tokens")
        .join(name)
}

fn canonical(case: &serde_json::Value, hash: &str, header: &ferroan_scripting::Header) -> String {
    let mut out = String::new();
    out.push_str("C ");
    out.push_str(case["name"].as_str().unwrap());
    out.push('\n');
    out.push_str("H ");
    out.push_str(hash);
    out.push('\n');
    out.push_str("V ");
    out.push_str(&header.version.to_string());
    out.push('\n');
    out.push('Z');
    for size in header.sizes {
        out.push(' ');
        out.push_str(&size.to_string());
    }
    out.push('\n');
    for state in &header.states {
        out.push_str("S ");
        out.push_str(state);
        out.push('\n');
    }
    for global in &header.globals {
        out.push_str("G ");
        out.push_str(&global.name);
        out.push('\n');
        out.push_str(&global.type_name);
        out.push('\n');
        out.push_str(&global.index.to_string());
        out.push('\n');
    }
    out.push_str(".\n");
    out
}

#[test]
fn pin_object_files_match_the_tokenizer() {
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["runtime"].as_str(), Some("10.0.14"));
    assert_eq!(
        manifest["pin"].as_str(),
        Some("41f18f983bb2bfc82e033fed410d361ba57d5aa4")
    );
    let mut canon = String::new();
    for case in manifest["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let parsed = tokenize(source).unwrap_or_else(|error| panic!("{error}"));
        let texts: Vec<String> = parsed.tokens.iter().map(|token| token.text()).collect();
        let expected: Vec<String> = case["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|token| token.as_str().unwrap().to_string())
            .collect();
        assert_eq!(texts, expected, "{}", case["name"]);
        let warnings: Vec<String> = case["tokenize_messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message.as_str().unwrap().to_string())
            .collect();
        assert_eq!(parsed.warnings, warnings, "{}", case["name"]);
        let hash = source_hash(&parsed);
        assert_eq!(hash, case["hash"].as_str().unwrap(), "{}", case["name"]);
        let name = case["name"].as_str().unwrap();
        let bytes = std::fs::read(fixture(&format!("objects/{name}.yobj"))).unwrap();
        let header = read_header(&bytes).unwrap();
        assert_eq!(header.version, -11);
        assert_eq!(header.source_hash, hash);
        assert_eq!(header.sizes.len(), 11);
        let sizes: Vec<i64> = case["sizes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_i64().unwrap())
            .collect();
        assert_eq!(
            header
                .sizes
                .iter()
                .map(|value| i64::from(*value))
                .collect::<Vec<_>>(),
            sizes
        );
        let states: Vec<&str> = case["states"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert_eq!(header.states, states);
        let globals = case["globals"].as_array().unwrap();
        assert_eq!(header.globals.len(), globals.len(), "{name}");
        for (got, want) in header.globals.iter().zip(globals) {
            assert_eq!(got.name, want["name"].as_str().unwrap());
            assert_eq!(got.type_name, want["type"].as_str().unwrap());
            assert_eq!(i64::from(got.index), want["index"].as_i64().unwrap());
        }
        canon.push_str(&canonical(case, &hash, &header));
    }
    let digest = format!("{:x}", Sha256::digest(canon.as_bytes()));
    let recorded: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("digest.json")).unwrap()).unwrap();
    assert_eq!(digest, recorded["digest"].as_str().unwrap());
    assert_eq!(digest, manifest["digest"].as_str().unwrap());
}

#[test]
fn pin_tokenize_errors() {
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("manifest.json")).unwrap()).unwrap();
    for case in manifest["errors"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let Err(Failure::Invalid { messages }) = tokenize(source) else {
            panic!("expected tokenize failure {}", case["name"]);
        };
        let expected: Vec<String> = case["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message.as_str().unwrap().to_string())
            .collect();
        assert_eq!(messages, expected, "{}", case["name"]);
    }
}

#[test]
fn bounds_refuse_before_a_huge_input() {
    let source = "a".repeat(usize::try_from(SOURCE_CHARS).unwrap() + 1);
    match tokenize(&source) {
        Err(Failure::Limit { id }) => assert_eq!(id, ferroan_diag::YENGINE_SOURCE_CHARS),
        other => panic!("{other:?}"),
    }
    let mut name = "b".repeat(MAX_NAME_LEN + 1);
    name.push(';');
    match tokenize(&name) {
        Err(Failure::Invalid { messages }) => {
            assert_eq!(messages, vec![format!("name too long, max {MAX_NAME_LEN}")]);
        }
        other => panic!("{other:?}"),
    }
    let mut over = vec![0xFF; usize::try_from(OBJECT_BYTES).unwrap() + 1];
    match read_header(&over) {
        Err(Failure::Limit { id }) => assert_eq!(id, ferroan_diag::YENGINE_OBJECT_BYTES),
        other => panic!("{other:?}"),
    }
    over.clear();
    over.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x01]);
    match read_header(&over) {
        Err(Failure::Object(_)) => {}
        other => panic!("truncated magic, got {other:?}"),
    }
    let mut header = Vec::new();
    header.extend_from_slice(b"YObjectCode");
    header.extend_from_slice(&(-11i32).to_le_bytes());
    header.extend_from_slice(&[0x81, 0x20]);
    match read_header(&header) {
        Err(Failure::Limit { id }) => assert_eq!(id, ferroan_diag::YENGINE_HEADER_STRING),
        other => panic!("header string bound, got {other:?}"),
    }
    let _ = HEADER_STRING;
}

#[test]
fn a_foreign_object_version_is_refused_before_the_hash() {
    let mut bytes = std::fs::read(fixture("objects/bare.yobj")).unwrap();
    assert!(
        bytes.len() > 15,
        "bare.yobj is shorter than a version field"
    );
    assert_eq!(&bytes[..11], b"YObjectCode");
    bytes[11..15].copy_from_slice(&(-10i32).to_le_bytes());
    match read_header(&bytes) {
        Err(Failure::Version { found, accepted }) => {
            assert_eq!(found, -10);
            assert_eq!(accepted, -11);
            assert_eq!(
                Failure::Version { found, accepted }.to_string(),
                "Object version is -10 but accept only -11"
            );
        }
        other => panic!("mutated bare.yobj, got {other:?}"),
    }
    let mut only = b"YObjectCode".to_vec();
    only.extend_from_slice(&(-9i32).to_le_bytes());
    match read_header(&only) {
        Err(Failure::Version { found, accepted }) => {
            assert_eq!((found, accepted), (-9, -11));
        }
        other => panic!("version with no hash, got {other:?}"),
    }
}

#[test]
fn a_max_line_directive_wraps_through_later_newlines() {
    let source = "# 2147483647 \"other.lsl\"\n// c\n/* a\nb */\n\"a\nb\"";
    let parsed = tokenize(source).unwrap_or_else(|error| panic!("{error}"));
    assert!(parsed.warnings.is_empty());
    let texts: Vec<String> = parsed.tokens.iter().map(|token| token.text()).collect();
    assert_eq!(texts, vec!["\"a\\nb".to_string()]);
}
