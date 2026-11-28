#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The digest and the literal messages come from `tools/regex-oracle` on
//! .NET 10.0.14. This test rebuilds the same canonical text and checks it.

use std::fs;
use std::path::PathBuf;

use ferroan_regex::{ParseError, parse};
use sha2::{Digest, Sha256};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/regex-parse/{name}"))
}

fn outcome(pattern: &str) -> String {
    match parse(pattern) {
        Ok(_) => "OK\n".to_owned(),
        Err(ParseError::Invalid { message }) => format!("ERR\n{message}\n"),
        Err(error) => panic!("pattern {pattern:?} produced {error}"),
    }
}

#[test]
fn parse_matches_dotnet_digest_and_literals() {
    let patterns: Vec<String> =
        serde_json::from_str(&fs::read_to_string(fixture("patterns.json")).unwrap()).unwrap();
    let digest_file: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture("digest.json")).unwrap()).unwrap();
    assert_eq!(digest_file["runtime"], "10.0.14");
    assert_eq!(
        digest_file["pin"],
        "41f18f983bb2bfc82e033fed410d361ba57d5aa4"
    );
    let mut canonical = String::new();
    for pattern in &patterns {
        canonical.push_str(&outcome(pattern));
    }
    let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    let expected = digest_file["digest"].as_str().unwrap();
    if digest != expected {
        if let Ok(path) = std::env::var("FERROAN_REGEX_EXPECT") {
            report_against_full(&path, &patterns);
        }
        panic!("digest {digest} != {expected}");
    }

    let literals: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture("literals.json")).unwrap()).unwrap();
    let cases = literals["cases"].as_array().unwrap();
    assert!(cases.len() > 20, "literal sample is too small");
    for case in cases {
        let pattern = case["pattern"].as_str().unwrap();
        if case["ok"] == true {
            parse(pattern).unwrap_or_else(|error| panic!("expected ok {pattern:?}: {error}"));
        } else {
            let message = case["message"].as_str().unwrap();
            match parse(pattern) {
                Err(ParseError::Invalid { message: actual }) => {
                    assert_eq!(actual, message, "pattern {pattern:?}");
                }
                other => panic!("pattern {pattern:?}: {other:?}"),
            }
        }
    }
}

fn report_against_full(path: &str, patterns: &[String]) {
    let full: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let outcomes = full["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), patterns.len());
    let mut shown = 0;
    for (index, outcome_row) in outcomes.iter().enumerate() {
        let pattern = outcome_row["pattern"].as_str().unwrap();
        assert_eq!(pattern, patterns[index]);
        let actual = outcome(pattern);
        let expected = if outcome_row["ok"] == true {
            "OK\n".to_owned()
        } else {
            format!("ERR\n{}\n", outcome_row["message"].as_str().unwrap())
        };
        if actual != expected {
            eprintln!("mismatch {index}: {pattern:?}\n  rust {actual:?}\n  net  {expected:?}");
            shown += 1;
            if shown == 12 {
                break;
            }
        }
    }
}

#[test]
fn pattern_past_the_utf16_cap_is_a_ferroan_limit() {
    let pattern = "a".repeat(65_537);
    match parse(&pattern) {
        Err(ParseError::Limit { id }) => assert_eq!(id, ferroan_diag::REGEX_PATTERN_CHARS),
        other => panic!("{other:?}"),
    }
}

#[test]
fn sixty_four_class_subtractions_parse_and_the_next_is_a_limit() {
    assert!(parse(&nest(64)).is_ok());
    match parse(&nest(65)) {
        Err(ParseError::Limit { id }) => assert_eq!(id, ferroan_diag::REGEX_CLASS_DEPTH),
        other => panic!("{other:?}"),
    }
}

fn nest(depth: usize) -> String {
    let mut inner = String::from("b");
    for _ in 0..depth {
        inner = format!("a-[{inner}]");
    }
    format!("[{inner}]")
}
