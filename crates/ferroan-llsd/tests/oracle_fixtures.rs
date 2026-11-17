// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every LLSD fixture from the OpenMetaverse oracle: the decoded value matches
//! `manifest.json`, and a round trip (or the known source value, when C# itself
//! cannot read what it wrote) matches the file bytes.

use std::fs;
use std::path::PathBuf;

use ferroan_llsd::{
    Llsd, from_binary, from_notation, from_xml, to_binary, to_binary_framed, to_notation, to_xml,
    to_xml_formal,
};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oracle")
}

fn decode(format: &str, bytes: &[u8]) -> Result<Llsd, ferroan_llsd::LlsdError> {
    match format {
        "xml" | "xml-formal" => from_xml(bytes),
        "binary" | "binary-raw" => from_binary(bytes).map(|(v, _)| v),
        "notation" | "notation-header" => from_notation(bytes),
        other => panic!("unknown llsd format {other}"),
    }
}

fn encode(format: &str, value: &Llsd) -> Vec<u8> {
    match format {
        "xml" => to_xml(value),
        "xml-formal" => to_xml_formal(value),
        "binary" => to_binary_framed(value),
        "binary-raw" => to_binary(value),
        "notation" => to_notation(value).into_bytes(),
        "notation-header" => {
            let mut bytes = b"<? llsd/notation ?>".to_vec();
            bytes.extend(to_notation(value).into_bytes());
            bytes
        }
        other => panic!("unknown llsd format {other}"),
    }
}

/// Values whose files are the encoder output even when C# cannot decode them.
fn source_value(id: &str) -> Option<Llsd> {
    match id.rsplit('/').next() {
        Some("real-nan") => Some(Llsd::Real(f64::from_bits(0xfff8_0000_0000_0000))),
        Some("real-pos-inf") => Some(Llsd::Real(f64::INFINITY)),
        Some("real-neg-inf") => Some(Llsd::Real(f64::NEG_INFINITY)),
        Some("string-nul") => Some(Llsd::String("a\0b".into())),
        _ => None,
    }
}

fn assert_decoded(got: &Llsd, expect: &Value, id: &str) {
    let kind = expect["type"].as_str().unwrap_or("");
    match (kind, got) {
        ("undef", Llsd::Undef) => {}
        ("boolean", Llsd::Boolean(b)) => {
            assert_eq!(*b, expect["value"].as_bool().unwrap(), "{id}");
        }
        ("integer", Llsd::Integer(n)) => {
            assert_eq!(i64::from(*n), expect["value"].as_i64().unwrap(), "{id}");
        }
        ("real", Llsd::Real(n)) => {
            assert_eq!(
                format!("{:016x}", n.to_bits()),
                expect["bits"].as_str().unwrap(),
                "{id}"
            );
        }
        ("string", Llsd::String(s)) => {
            assert_eq!(s, expect["value"].as_str().unwrap(), "{id}");
        }
        ("uuid", Llsd::Uuid(u)) => {
            assert_eq!(u.to_string(), expect["value"].as_str().unwrap(), "{id}");
        }
        ("date", Llsd::Date(_)) => {
            assert_eq!(
                got.to_string_value(),
                expect["value"].as_str().unwrap(),
                "{id}"
            );
        }
        ("uri", Llsd::Uri(s)) => {
            assert_eq!(s, expect["value"].as_str().unwrap(), "{id}");
        }
        ("binary", Llsd::Binary(b)) => {
            let text = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b);
            assert_eq!(text, expect["base64"].as_str().unwrap(), "{id}");
        }
        ("array", Llsd::Array(items)) => {
            let expect_items = expect["value"].as_array().unwrap();
            assert_eq!(items.len(), expect_items.len(), "{id}");
            for (item, expect_item) in items.iter().zip(expect_items) {
                assert_decoded(item, expect_item, id);
            }
        }
        ("map", Llsd::Map(pairs)) => {
            let expect_pairs = expect["pairs"].as_array().unwrap();
            assert_eq!(pairs.len(), expect_pairs.len(), "{id}");
            for ((key, value), expect_pair) in pairs.iter().zip(expect_pairs) {
                let expect_pair = expect_pair.as_array().unwrap();
                assert_eq!(key, expect_pair[0].as_str().unwrap(), "{id}");
                assert_decoded(value, &expect_pair[1], id);
            }
        }
        _ => panic!("{id}: decoded {got:?}, manifest {expect}"),
    }
}

#[test]
fn every_llsd_fixture_matches_openmetaverse() {
    let root = fixtures();
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut seen = 0;
    for case in manifest["cases"].as_array().unwrap() {
        if case["family"].as_str() != Some("llsd") {
            continue;
        }
        seen += 1;
        let id = case["id"].as_str().unwrap();
        let format = case["format"].as_str().unwrap();
        let bytes = fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        let decoded = &case["decoded"];
        match decode(format, &bytes) {
            Ok(value) => {
                assert_ne!(
                    decoded["type"].as_str(),
                    Some("decode-error"),
                    "{id} parsed a rejected value"
                );
                assert_decoded(&value, decoded, id);
                if case["round_trip"] == true {
                    assert_eq!(encode(format, &value), bytes, "{id} re-encode");
                }
            }
            Err(err) => {
                assert_eq!(
                    decoded["type"].as_str(),
                    Some("decode-error"),
                    "{id} returned {err}, manifest decoded {decoded}"
                );
            }
        }
        if let Some(source) = source_value(id) {
            assert_eq!(encode(format, &source), bytes, "{id} encode");
        }
    }
    assert!(seen > 100, "expected the llsd cases, saw {seen}");
}

#[test]
fn xml_nesting_past_the_limit_is_an_error() {
    let mut doc = String::from("<llsd>");
    for _ in 0..300 {
        doc.push_str("<array>");
    }
    doc.push_str("<undef/>");
    for _ in 0..300 {
        doc.push_str("</array>");
    }
    doc.push_str("</llsd>");
    assert!(from_xml(doc.as_bytes()).is_err());
}
