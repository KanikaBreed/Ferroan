#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The digest and the literal messages come from `tools/format-oracle` on
//! .NET 10.0.14. This test rebuilds the same canonical text and checks it.

use std::fs;
use std::path::PathBuf;

use ferroan_format::{FormatError, Parsed, Piece, accept, parse};
use sha2::{Digest, Sha256};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/format-items/{name}"))
}

#[derive(Clone)]
struct Case {
    format: Option<String>,
    args: Vec<String>,
}

fn load_cases(text: &str) -> Vec<Case> {
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|row| Case {
            format: match &row["format"] {
                serde_json::Value::Null => None,
                other => Some(other.as_str().unwrap().to_owned()),
            },
            args: row["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap().to_owned())
                .collect(),
        })
        .collect()
}

fn outcome(case: &Case) -> String {
    let format = case.format.as_deref();
    match accept(format, case.args.len()) {
        Ok(parsed) => ok_text(&parsed, &case.args),
        Err(error @ FormatError::Null) => {
            err_text("ArgumentNullException", &error.to_string(), None)
        }
        Err(error @ FormatError::Invalid { .. }) => {
            err_text("FormatException", &error.to_string(), None)
        }
        Err(FormatError::ArgIndex { parsed }) => {
            let message = FormatError::ArgIndex { parsed: None }.to_string();
            err_text("FormatException", &message, parsed.as_ref())
        }
        Err(error) => panic!("format {:?} produced {error}", case.format),
    }
}

fn ok_text(parsed: &Parsed, args: &[String]) -> String {
    let rendered = render(parsed, args);
    let mut out = format!("OK\nR {}\n{rendered}\n", utf16(&rendered));
    push_segments(&mut out, parsed);
    out
}

fn err_text(kind: &str, message: &str, parsed: Option<&Parsed>) -> String {
    let mut out = format!("ERR\n{kind}\n{message}\n");
    if let Some(parsed) = parsed {
        push_segments(&mut out, parsed);
    }
    out
}

fn push_segments(out: &mut String, parsed: &Parsed) {
    for piece in parsed.pieces() {
        match piece {
            Piece::Literal(text) => {
                let count = utf16(text);
                out.push_str(&format!("L {count}\n{text}\n"));
            }
            Piece::Hole {
                index,
                alignment,
                format: None,
            } => out.push_str(&format!("H {index} {alignment} -\n")),
            Piece::Hole {
                index,
                alignment,
                format: Some(item),
            } => {
                let count = utf16(item);
                out.push_str(&format!("H {index} {alignment} {count}\n{item}\n"));
            }
        }
    }
}

fn render(parsed: &Parsed, args: &[String]) -> String {
    let mut out = String::new();
    for piece in parsed.pieces() {
        match piece {
            Piece::Literal(text) => out.push_str(text),
            Piece::Hole {
                index, alignment, ..
            } => {
                let arg = usize::try_from(*index)
                    .ok()
                    .and_then(|slot| args.get(slot))
                    .unwrap_or_else(|| panic!("hole {index}"));
                pad(&mut out, arg, *alignment);
            }
        }
    }
    out
}

fn pad(out: &mut String, arg: &str, alignment: i32) {
    let width = usize::try_from(alignment.unsigned_abs()).unwrap_or(usize::MAX);
    let units = utf16(arg);
    if width <= units {
        out.push_str(arg);
        return;
    }
    let spaces = " ".repeat(width - units);
    if alignment < 0 {
        out.push_str(arg);
        out.push_str(&spaces);
    } else {
        out.push_str(&spaces);
        out.push_str(arg);
    }
}

fn utf16(text: &str) -> usize {
    text.encode_utf16().count()
}

#[test]
fn format_matches_dotnet_digest_and_literals() {
    let cases = load_cases(&fs::read_to_string(fixture("cases.json")).unwrap());
    let digest_file: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture("digest.json")).unwrap()).unwrap();
    assert_eq!(digest_file["runtime"], "10.0.14");
    assert_eq!(
        digest_file["pin"],
        "41f18f983bb2bfc82e033fed410d361ba57d5aa4"
    );
    let mut canonical = String::new();
    for case in &cases {
        canonical.push_str(&outcome(case));
    }
    let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    let expected = digest_file["digest"].as_str().unwrap();
    if digest != expected {
        if let Ok(path) = std::env::var("FERROAN_FORMAT_EXPECT") {
            report_against_full(&path, &cases);
        }
        panic!("digest {digest} != {expected}");
    }

    let literals: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture("literals.json")).unwrap()).unwrap();
    let rows = literals["cases"].as_array().unwrap();
    assert!(rows.len() > 12, "literal sample is too small");
    for row in rows {
        let case = Case {
            format: match &row["format"] {
                serde_json::Value::Null => None,
                other => Some(other.as_str().unwrap().to_owned()),
            },
            args: row["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap().to_owned())
                .collect(),
        };
        if row["ok"] == true {
            let rendered = row["rendered"].as_str().unwrap();
            match accept(case.format.as_deref(), case.args.len()) {
                Ok(parsed) => assert_eq!(render(&parsed, &case.args), rendered),
                Err(error) => panic!("expected ok {:?}: {error}", case.format),
            }
        } else {
            let kind = row["type"].as_str().unwrap();
            let message = row["message"].as_str().unwrap();
            match accept(case.format.as_deref(), case.args.len()) {
                Err(FormatError::Null) => {
                    assert_eq!(kind, "ArgumentNullException");
                    assert_eq!(message, FormatError::Null.to_string());
                }
                Err(FormatError::Invalid { message: actual }) => {
                    assert_eq!(kind, "FormatException");
                    assert_eq!(actual, message);
                }
                Err(FormatError::ArgIndex { .. }) => {
                    assert_eq!(kind, "FormatException");
                    assert_eq!(message, FormatError::ArgIndex { parsed: None }.to_string());
                }
                other => panic!("format {:?}: {other:?}", case.format),
            }
        }
    }
}

fn report_against_full(path: &str, cases: &[Case]) {
    let full: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let outcomes = full["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), cases.len());
    let mut shown = 0;
    for (index, row) in outcomes.iter().enumerate() {
        let expected = row["canonical"].as_str().unwrap();
        let actual = outcome(&cases[index]);
        if actual != expected {
            eprintln!(
                "mismatch {index}: {:?}\n  rust {actual:?}\n  net  {expected:?}",
                cases[index].format
            );
            shown += 1;
            if shown == 8 {
                break;
            }
        }
    }
}

#[test]
fn format_past_the_utf16_cap_is_a_ferroan_limit() {
    let format = "a".repeat(65_537);
    match parse(Some(&format)) {
        Err(FormatError::Limit { id }) => assert_eq!(id, ferroan_diag::FORMAT_CHARS),
        other => panic!("{other:?}"),
    }
    assert!(parse(Some(&"a".repeat(65_536))).is_ok());
}

#[test]
fn width_at_the_digit_stop_is_stored_and_one_more_digit_is_unclosed() {
    match parse(Some("{0,1000000}")) {
        Ok(parsed) => match parsed.pieces().get(1) {
            Some(Piece::Hole { alignment, .. }) => assert_eq!(*alignment, 1_000_000),
            other => panic!("{other:?}"),
        },
        Err(error) => panic!("{error}"),
    }
    match parse(Some("{0,-0}")) {
        Ok(parsed) => match parsed.pieces().get(1) {
            Some(Piece::Hole { alignment, .. }) => assert_eq!(*alignment, 0),
            other => panic!("{other:?}"),
        },
        Err(error) => panic!("{error}"),
    }
    match parse(Some("{0,10000000}")) {
        Err(FormatError::Invalid { message }) => {
            assert!(
                message.contains("Format item ends prematurely."),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
}
