// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every `fixtures/http` case against the bytes `ServerUtils` produced.

// Integration tests are a test crate; the production lints stay on the library.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

use ferroan_http::{
    Field, FieldMap, REFERENCE_TRANQUILLITY_REVISION, build_query_string, build_xml_response,
    parse_query_string, parse_xml_response,
};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/http")
}

fn field_map(value: &Value) -> FieldMap {
    let mut map = FieldMap::new();
    let object = value.as_object().unwrap();
    for (key, item) in object {
        map.insert(key.clone(), field(item));
    }
    map
}

fn field(value: &Value) -> Field {
    match value["t"].as_str().unwrap() {
        "null" => Field::Null,
        "str" => Field::Str(value["v"].as_str().unwrap().to_string()),
        "bool" => Field::Bool(value["v"].as_bool().unwrap()),
        "int" => Field::Int(value["v"].as_i64().unwrap().try_into().unwrap()),
        "uuid" => Field::Uuid(value["v"].as_str().unwrap().to_string()),
        "list" => Field::List(
            value["v"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item.as_str().unwrap().to_string())
                .collect(),
        ),
        "map" => Field::Map(field_map(&value["v"])),
        other => panic!("unknown field {other}"),
    }
}

#[test]
fn every_http_fixture_matches_serverutils() {
    let root = fixtures();
    let manifest = fs::read_to_string(root.join("manifest.txt")).unwrap();
    let mut seen = 0;
    let mut revision = None;
    for line in manifest.lines() {
        if let Some(hash) = line.strip_prefix("revision ") {
            revision = Some(hash.to_string());
            continue;
        }
        if line.is_empty() || line.starts_with("format ") || line.starts_with("source ") {
            continue;
        }
        let case: Value =
            serde_json::from_slice(&fs::read(root.join(format!("{line}.json"))).unwrap()).unwrap();
        let kind = case["kind"].as_str().unwrap();
        if let Some(error) = case.get("error") {
            let failed = match kind {
                "query-build" => build_query_string(&field_map(&case["input"])).is_err(),
                "xml-build" => build_xml_response(&field_map(&case["input"])).is_err(),
                other => panic!("{line} error on {other}"),
            };
            assert!(failed, "{line} should fail ({})", error.as_str().unwrap());
            seen += 1;
            continue;
        }
        match kind {
            "query-parse" => {
                let parsed = parse_query_string(case["input"].as_str().unwrap()).unwrap();
                assert_eq!(parsed, field_map(&case["parsed"]), "{line} parse");
                let built = build_query_string(&parsed).unwrap();
                assert_eq!(built, case["built"].as_str().unwrap(), "{line} build");
            }
            "query-build" => {
                let built = build_query_string(&field_map(&case["input"])).unwrap();
                assert_eq!(built, case["built"].as_str().unwrap(), "{line}");
            }
            "xml-build" => {
                let fields = field_map(&case["input"]);
                let built = build_xml_response(&fields).unwrap();
                assert_eq!(built, case["built"].as_str().unwrap(), "{line} build");
                let parsed = parse_xml_response(&built).unwrap();
                assert_eq!(parsed, field_map(&case["parsed"]), "{line} parse");
            }
            "xml-parse" => {
                let parsed = parse_xml_response(case["input"].as_str().unwrap()).unwrap();
                assert_eq!(parsed, field_map(&case["parsed"]), "{line}");
            }
            other => panic!("{line} kind {other}"),
        }
        seen += 1;
    }
    assert!(seen > 40, "seen {seen}");
    assert_eq!(revision.as_deref(), Some(REFERENCE_TRANQUILLITY_REVISION));
}
