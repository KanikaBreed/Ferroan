// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Compares ferroan-config with the key/value sets `tools/config-oracle` wrote
//! from Nini. Cases under `region-ref` and `grid-ref` read the pinned
//! Tranquillity tree when it is available and are skipped otherwise.

// Integration tests are a test crate; the production lints stay on the library.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ferroan_config::{Config, ConfigError, GridLoad, RegionLoad};

#[test]
fn resolved_sets_match_the_nini_oracle() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/config");
    let manifest =
        std::fs::read_to_string(root.join("manifest.txt")).expect("fixtures/config/manifest.txt");
    let mut cases = Vec::new();
    for line in manifest.lines() {
        if line.is_empty()
            || line.starts_with("format ")
            || line.starts_with("nini ")
            || line.starts_with("revision ")
        {
            continue;
        }
        cases.push(line.to_string());
    }
    assert!(cases.len() >= 10, "expected the oracle cases");
    let reference = reference_root();
    let mut skipped = Vec::new();
    for id in cases {
        let case_dir = root.join(&id);
        let kind = std::fs::read_to_string(case_dir.join("kind.txt"))
            .unwrap()
            .trim()
            .to_string();
        if matches!(kind.as_str(), "region-ref" | "grid-ref") && reference.is_none() {
            skipped.push(id);
            continue;
        }
        match run_case(&case_dir, &kind, reference.as_deref()) {
            Ok(outcome) => assert_case(&case_dir, &outcome),
            Err(error) => {
                let expected = std::fs::read_to_string(case_dir.join("error.txt")).ok();
                let actual = format_error(&error);
                let expected = expected
                    .as_deref()
                    .map(|text| text.strip_suffix('\n').unwrap_or(text));
                assert_eq!(expected, Some(actual.as_str()), "{id}");
            }
        }
    }
    if !skipped.is_empty() {
        eprintln!(
            "skipped Tranquillity INI cases (no reference checkout): {}",
            skipped.join(", ")
        );
    }
}

#[test]
fn unknown_keys_are_reported_and_the_value_remains() {
    let mut config = Config::parse("[Startup]\nphysics = ubODE\nextra = 1\n").unwrap();
    config.expand().unwrap();
    assert_eq!(config.get("Startup", "extra"), Some("1"));
    assert_eq!(config.unknown_keys("Startup", &["physics"]), vec!["extra"]);
    assert!(
        config
            .unknown_keys("Startup", &["physics", "extra"])
            .is_empty()
    );
}

#[test]
fn glob_include_finds_one_file() {
    let root = std::env::temp_dir().join(format!("ferroan-config-glob-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::write(root.join("nested/one.ini"), "[S]\nA=1\n").unwrap();
    std::fs::write(root.join("skip.txt"), "no\n").unwrap();
    let mut argv = Config::default();
    argv.set("Startup", "inimaster", "none");
    argv.set("Startup", "inifile", "user.ini");
    argv.set("Startup", "inidirectory", "no-such-dir");
    std::fs::write(
        root.join("user.ini"),
        "[S]\nInclude-Nested = nested/*.ini\nB=2\n",
    )
    .unwrap();
    let resolved = RegionLoad {
        config_dir: root.clone(),
        argv,
        environment: BTreeMap::new(),
    }
    .load()
    .unwrap();
    assert_eq!(
        resolved.config.get("S", "A"),
        Some("1"),
        "config_dir {}",
        root.display()
    );
    assert_eq!(resolved.config.get("S", "B"), Some("2"));
    assert!(resolved.warnings.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn glob_include_follows_a_directory_symlink() {
    let root = std::env::temp_dir().join(format!("ferroan-config-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("real")).unwrap();
    std::fs::write(root.join("real/one.ini"), "[S]\nA=1\n").unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("linked")).unwrap();
    let mut argv = Config::default();
    argv.set("Startup", "inimaster", "none");
    argv.set("Startup", "inifile", "user.ini");
    argv.set("Startup", "inidirectory", "no-such-dir");
    std::fs::write(
        root.join("user.ini"),
        "[S]\nInclude-Nested = linked/*.ini\n",
    )
    .unwrap();
    let resolved = RegionLoad {
        config_dir: root.clone(),
        argv,
        environment: BTreeMap::new(),
    }
    .load()
    .unwrap();
    assert_eq!(
        resolved.config.get("S", "A"),
        Some("1"),
        "config_dir {}",
        root.display()
    );
    let _ = std::fs::remove_dir_all(&root);
}

struct Ran {
    rows: Vec<String>,
    warnings: Vec<String>,
}

fn run_case(case_dir: &Path, kind: &str, reference: Option<&Path>) -> Result<Ran, ConfigError> {
    if kind == "parse" || kind == "expand" {
        let text = std::fs::read_to_string(case_dir.join("source.ini")).unwrap();
        let mut config = Config::parse(&text)?;
        if kind == "expand" {
            config.expand()?;
        }
        return Ok(rows_of(&config, &[]));
    }
    let spec = std::fs::read_to_string(case_dir.join("load.txt")).unwrap();
    let loaded = load_spec(case_dir, kind, &spec, reference)?;
    Ok(rows_of(&loaded.config, &loaded.warnings))
}

fn rows_of(config: &Config, warnings: &[ferroan_config::LoadWarning]) -> Ran {
    let mut rows = Vec::new();
    for (section, key, value) in config.entries() {
        rows.push(format!(
            "{}\t{}\t{}",
            escape(section),
            escape(key),
            escape(value)
        ));
    }
    rows.sort();
    let warnings = warnings
        .iter()
        .map(|warning| format!("missing-include\t{}", escape(&warning.value)))
        .collect();
    Ran { rows, warnings }
}

fn assert_case(case_dir: &Path, outcome: &Ran) {
    let resolved = std::fs::read_to_string(case_dir.join("resolved.txt")).unwrap_or_else(|error| {
        panic!(
            "{}: expected success, read resolved.txt: {error}",
            case_dir.display()
        )
    });
    let actual = if outcome.rows.is_empty() {
        String::new()
    } else {
        format!("{}\n", outcome.rows.join("\n"))
    };
    assert_eq!(actual, resolved, "{}", case_dir.display());
    let warnings = std::fs::read_to_string(case_dir.join("warnings.txt")).unwrap_or_default();
    let actual_warnings = if outcome.warnings.is_empty() {
        String::new()
    } else {
        format!("{}\n", outcome.warnings.join("\n"))
    };
    assert_eq!(actual_warnings, warnings, "{}", case_dir.display());
}

fn load_spec(
    case_dir: &Path,
    kind: &str,
    text: &str,
    reference: Option<&Path>,
) -> Result<ferroan_config::Resolved, ConfigError> {
    let mut appdata = None;
    let mut master = None;
    let mut files = Vec::new();
    let mut ini_directory = None;
    let mut saw_directory = false;
    let mut environment = BTreeMap::new();
    let mut argv_keys = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (field, rest) = line.split_once(' ').unwrap_or((line, ""));
        match field {
            "reference_appdata" => appdata = Some(rest.to_string()),
            "ini_master" => master = Some(rest.to_string()),
            "ini_file" => files.push(rest.to_string()),
            "ini_directory" => {
                saw_directory = true;
                ini_directory = Some(rest.to_string());
            }
            "env" => {
                let (key, value) = rest.split_once('=').expect("env KEY=VALUE");
                environment.insert(key.to_string(), value.to_string());
            }
            "argv" => {
                let mut parts = rest.splitn(3, ' ');
                let section = parts.next().unwrap().to_string();
                let key = parts.next().unwrap().to_string();
                let value = parts.next().unwrap().to_string();
                argv_keys.push((section, key, value));
            }
            other => panic!("unknown load field {other}"),
        }
    }
    let config_dir = if let Some(relative) = appdata {
        reference
            .expect("reference checkout")
            .join("OpenSim-Tranquillity/Source")
            .join(relative)
    } else {
        case_dir.join("tree")
    };
    if kind == "grid" || kind == "grid-ref" {
        return GridLoad {
            config_dir,
            ini_master: master,
            ini_files: files,
            ini_directory: if saw_directory { ini_directory } else { None },
            environment,
        }
        .load();
    }
    let mut argv = Config::default();
    if let Some(master) = master {
        argv.set("Startup", "inimaster", &master);
    }
    for (index, file) in files.iter().enumerate() {
        let key = if index == 0 {
            "inifile".to_string()
        } else {
            format!("inifile{}", index + 1)
        };
        argv.set("Startup", &key, file);
    }
    if saw_directory {
        argv.set(
            "Startup",
            "inidirectory",
            ini_directory.as_deref().unwrap_or(""),
        );
    }
    for (section, key, value) in argv_keys {
        argv.set(&section, &key, &value);
    }
    RegionLoad {
        config_dir,
        argv,
        environment,
    }
    .load()
}

fn format_error(error: &ConfigError) -> String {
    match error {
        ConfigError::ExpandKey { key }
        | ConfigError::ExpandSelf { key }
        | ConfigError::MissingMaster { name: key } => {
            format!("{}\t{}", error.code(), escape(key))
        }
        ConfigError::ExpandSection { section } => format!("expand-section\t{}", escape(section)),
        ConfigError::RemoteInclude { value } => format!("remote-include\t{}", escape(value)),
        other => other.code().to_string(),
    }
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn reference_root() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("FERROAN_REF")
        && !value.is_empty()
    {
        return Some(PathBuf::from(value));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let path = PathBuf::from(home).join("Git/reference/OpenSim-Tranquillity/Source");
    path.is_dir()
        .then_some(path.parent().unwrap().parent().unwrap().to_path_buf())
}
