// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Refuses a `JOLTC_LIB_DIR` bypass and checks the SHAs recorded beside D8
//! against `oxijolt-sys`'s `prebuilt.txt`.
//!
//! `oxijolt-sys` treats a set `JOLTC_LIB_DIR` as an install prefix and skips
//! the SHA-checked archive. This script cannot set `JOLTC_PREBUILT` for that
//! crate. The repository default stays in `.cargo/config.toml`.

use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Build-script failure. `Debug` prints the sentence, which is what Cargo shows.
struct Fail(String);

impl fmt::Debug for Fail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Crate version the D8 rows describe. The `+jolt-…` suffix comes from the lockfile.
const OXIJOLT_VERSION: &str = "1.6.0";

/// `sources=` line of `prebuilt.txt` for this `oxijolt-sys` release.
const SOURCES_FINGERPRINT: &str =
    "e865c544add1d212ee5c8e473eead596b9fa379429d4cf3e23dbb617b210c7df";

struct RecordedArchive {
    name: &'static str,
    sha256: &'static str,
    host: &'static str,
}

/// Single precision, cross-platform deterministic, debug renderer off, asserts off.
const ARCHIVES: &[RecordedArchive] = &[
    RecordedArchive {
        name: "oxijolt-sys-1.6.0-x86_64-unknown-linux-gnu-cross-platform-deterministic",
        sha256: "f9ecd326c2b32c571736d0b5685be4b989247291bad677168f94ced55efaf8d4",
        host: "glibc=2.35",
    },
    RecordedArchive {
        name: "oxijolt-sys-1.6.0-x86_64-pc-windows-msvc-cross-platform-deterministic",
        sha256: "6d8cca7e890597f563ffdd4fbda8f7bdb88eb61baeb725bdda4ab6eba38da989",
        host: "msvc=14.44",
    },
    RecordedArchive {
        name: "oxijolt-sys-1.6.0-aarch64-apple-darwin-cross-platform-deterministic",
        sha256: "8eaa98d6de3c6f95c98a7d93064991987e16c7d1e4e6776f33b5bb25c944c83b",
        host: "macos=11.0",
    },
];

fn main() -> Result<(), Fail> {
    println!("cargo:rerun-if-env-changed=JOLTC_LIB_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("JOLTC_LIB_DIR").is_some() {
        return Err(Fail(
            "JOLTC_LIB_DIR is set. oxijolt-sys uses that prefix and skips the SHA-checked archive. \
             Ferroan refuses the setting. Unset it so the repository default JOLTC_PREBUILT=require \
             uses the prebuilt, or set JOLTC_PREBUILT=off to build from source."
                .to_string(),
        ));
    }
    check_recorded_shas().map_err(Fail)
}

fn check_recorded_shas() -> Result<(), String> {
    let manifest = env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
        "CARGO_MANIFEST_DIR is unset, so the oxijolt-sys SHAs recorded beside D8 were not checked"
            .to_string()
    })?;
    let lock_path = PathBuf::from(&manifest).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock_text = fs::read_to_string(&lock_path)
        .map_err(|error| format!("read Cargo.lock to check the oxijolt-sys version: {error}"))?;
    let version = locked_version(&lock_text)?;
    let lists = prebuilt_lists(&version, Path::new(&manifest))?;
    for list in &lists {
        println!("cargo:rerun-if-changed={}", list.display());
        let text = fs::read_to_string(list).map_err(|error| {
            format!("read oxijolt-sys prebuilt.txt to check the SHAs recorded beside D8: {error}")
        })?;
        check_text(&text)?;
    }
    Ok(())
}

/// Resolved `oxijolt-sys` version from `Cargo.lock`, including the `+jolt-…` suffix.
fn locked_version(lock_text: &str) -> Result<String, String> {
    let mut in_package = false;
    for raw in lock_text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.starts_with("[[") {
            in_package = false;
        }
        if line == "name = \"oxijolt-sys\"" {
            in_package = true;
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = line.strip_prefix("version = \"") else {
            continue;
        };
        let version = rest
            .strip_suffix('"')
            .ok_or_else(|| "Cargo.lock oxijolt-sys version is not a quoted string".to_string())?;
        let numeric = version.split('+').next().unwrap_or(version);
        if numeric != OXIJOLT_VERSION {
            return Err(format!(
                "Cargo.lock resolves oxijolt-sys {version}, and D8 records {OXIJOLT_VERSION}. \
                 Update the recorded SHAs with the oxijolt-sys version."
            ));
        }
        return Ok(version.to_string());
    }
    Err("Cargo.lock has no oxijolt-sys package".to_string())
}

fn cargo_home() -> Option<PathBuf> {
    if let Some(home) = env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".cargo"));
    }
    env::var_os("USERPROFILE").map(|profile| PathBuf::from(profile).join(".cargo"))
}

fn prebuilt_lists(version: &str, manifest_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let dir_name = format!("oxijolt-sys-{version}");
    let mut found = Vec::new();
    if let Some(home) = cargo_home() {
        let src = home.join("registry").join("src");
        if let Ok(indexes) = fs::read_dir(&src) {
            for index in indexes {
                let Ok(index) = index else {
                    continue;
                };
                let candidate = index.path().join(&dir_name).join("prebuilt.txt");
                if candidate.is_file() {
                    found.push(candidate);
                }
            }
        }
    }
    let vendor = manifest_dir.join("../../vendor");
    for name in [dir_name.as_str(), "oxijolt-sys"] {
        let candidate = vendor.join(name).join("prebuilt.txt");
        if candidate.is_file() {
            found.push(candidate);
        }
    }
    if found.is_empty() {
        return Err(format!(
            "oxijolt-sys {version} prebuilt.txt was not in the Cargo registry source or the \
             workspace vendor directory, so the SHAs recorded beside D8 were not checked"
        ));
    }
    Ok(found)
}

fn check_text(text: &str) -> Result<(), String> {
    let mut saw_version = false;
    let mut saw_sources = false;
    let mut saw_archive = [false; ARCHIVES.len()];
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r').trim();
        if line == format!("version={OXIJOLT_VERSION}") {
            saw_version = true;
        }
        if line == format!("sources={SOURCES_FINGERPRINT}") {
            saw_sources = true;
        }
        for (index, archive) in ARCHIVES.iter().enumerate() {
            let Some(first) = line.split_whitespace().next() else {
                continue;
            };
            if first != format!("archive={}", archive.name) {
                continue;
            }
            if archive_line_ok(line, archive) {
                saw_archive[index] = true;
            } else {
                return Err(format!(
                    "oxijolt-sys prebuilt.txt lists {} with a sha or feature set other than the \
                     one recorded beside D8. Update the recorded SHAs with the oxijolt-sys version.",
                    archive.name
                ));
            }
        }
    }
    if !saw_version {
        return Err(format!(
            "oxijolt-sys prebuilt.txt has no version={OXIJOLT_VERSION} line. \
             Update the recorded SHAs with the oxijolt-sys version."
        ));
    }
    if !saw_sources {
        return Err(
            "oxijolt-sys prebuilt.txt sources fingerprint differs from the one recorded beside D8. \
             Update the recorded SHAs with the oxijolt-sys version."
                .to_string(),
        );
    }
    for (index, archive) in ARCHIVES.iter().enumerate() {
        if !saw_archive[index] {
            return Err(format!(
                "oxijolt-sys prebuilt.txt has no {} archive for the recorded sha. \
                 Update the recorded SHAs with the oxijolt-sys version.",
                archive.name
            ));
        }
    }
    Ok(())
}

fn archive_line_ok(line: &str, archive: &RecordedArchive) -> bool {
    has_token(line, &format!("sha256={}", archive.sha256))
        && has_token(line, archive.host)
        && has_token(line, "double_precision=OFF")
        && has_token(line, "cross_platform_deterministic=ON")
        && has_token(line, "debug_renderer=OFF")
        && has_token(line, "asserts=OFF")
}

fn has_token(line: &str, token: &str) -> bool {
    line.split_whitespace().any(|part| part == token)
}
