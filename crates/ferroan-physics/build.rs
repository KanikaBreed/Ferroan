// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Refuses a `JOLTC_LIB_DIR` bypass and checks the SHA constants in this file
//! against `prebuilt.txt` beside the `oxijolt-sys` package Cargo resolves.
//!
//! `oxijolt-sys` treats a set `JOLTC_LIB_DIR` as an install prefix and skips
//! the SHA-checked archive. This script cannot set `JOLTC_PREBUILT` for that
//! crate. The repository default stays in `.cargo/config.toml`.
//!
//! The package is located with `cargo metadata --format-version 1 --offline
//! --locked`, using the package id on `ferroan-physics`'s resolve edge. A
//! `[patch]` or a vendored source replacement is the tree that is checked.
//! D8 quotes these constants.

use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Build-script failure. `Debug` prints the sentence, which is what Cargo shows.
struct Fail(String);

impl fmt::Debug for Fail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Crate version the constants describe. The `+jolt-…` suffix comes from the lockfile.
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
/// D8 quotes these values. They live here.
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
    println!("cargo:rerun-if-env-changed=CARGO_HOME");
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
        "CARGO_MANIFEST_DIR is unset, so the oxijolt-sys SHA constants in build.rs were not checked"
            .to_string()
    })?;
    let manifest_dir = PathBuf::from(&manifest);
    let lock_path = manifest_dir.join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock_text = fs::read_to_string(&lock_path)
        .map_err(|error| format!("read Cargo.lock to check the oxijolt-sys version: {error}"))?;
    let version = locked_version(&lock_text)?;
    let list = resolved_prebuilt(&version, &manifest_dir)?;
    println!("cargo:rerun-if-changed={}", list.display());
    let text = fs::read_to_string(&list).map_err(|error| {
        format!("read oxijolt-sys prebuilt.txt to check the SHA constants in build.rs: {error}")
    })?;
    check_text(&text)
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
                "Cargo.lock resolves oxijolt-sys {version}, and build.rs records {OXIJOLT_VERSION}. \
                 Update the SHA constants in build.rs with the oxijolt-sys version, and quote them beside D8."
            ));
        }
        return Ok(version.to_string());
    }
    Err("Cargo.lock has no oxijolt-sys package".to_string())
}

/// `prebuilt.txt` beside the `oxijolt-sys` Cargo resolves for this package.
fn resolved_prebuilt(locked: &str, manifest_dir: &Path) -> Result<PathBuf, String> {
    let metadata = cargo_metadata(manifest_dir)?;
    let json = parse_json(&metadata)?;
    let root = object(&json)?;
    let packages = array(field(root, "packages")?)?;
    let physics_id = package_id_named(packages, "ferroan-physics")?;
    let nodes = array(field(object(field(root, "resolve")?)?, "nodes")?)?;
    let physics = node_by_id(nodes, &physics_id)?;
    let pkg_id = oxijolt_dep_id(physics)?;
    let package = package_by_id(packages, &pkg_id)?;
    let name = string(field(package, "name")?)?;
    if name != "oxijolt-sys" {
        return Err(format!(
            "ferroan-physics resolves package id {pkg_id}, and that package is {name}"
        ));
    }
    let version = string(field(package, "version")?)?;
    if version != locked {
        return Err(format!(
            "cargo metadata resolved oxijolt-sys {version} ({pkg_id}), and Cargo.lock records {locked}"
        ));
    }
    let manifest = string(field(package, "manifest_path")?)?;
    let Some(dir) = Path::new(manifest).parent() else {
        return Err(format!(
            "oxijolt-sys {version} ({pkg_id}) manifest path {manifest} has no directory"
        ));
    };
    let prebuilt = dir.join("prebuilt.txt");
    if !prebuilt.is_file() {
        return Err(format!(
            "oxijolt-sys {version} ({pkg_id}) has no prebuilt.txt beside {manifest}, \
             so the SHA constants in build.rs were not checked"
        ));
    }
    Ok(prebuilt)
}

fn cargo_metadata(manifest_dir: &Path) -> Result<String, String> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let workspace = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "ferroan-physics has no workspace parent".to_string())?
        .join("Cargo.toml");
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--offline",
            "--locked",
            "--manifest-path",
        ])
        .arg(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("run cargo metadata to locate oxijolt-sys: {error}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let err = err.trim();
        let err = if err.len() > 800 { &err[..800] } else { err };
        return Err(format!(
            "cargo metadata --format-version 1 --offline --locked failed: {err}"
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("cargo metadata wrote a non-UTF-8 manifest graph: {error}"))
}

fn package_id_named(packages: &[Json], name: &str) -> Result<String, String> {
    let mut found = None;
    for package in packages {
        let package = object(package)?;
        if string(field(package, "name")?)? != name {
            continue;
        }
        let id = string(field(package, "id")?)?.to_string();
        if found.is_some() {
            return Err(format!("cargo metadata lists more than one {name} package"));
        }
        found = Some(id);
    }
    found.ok_or_else(|| format!("cargo metadata has no {name} package"))
}

fn package_by_id<'a>(packages: &'a [Json], id: &str) -> Result<&'a [(String, Json)], String> {
    for package in packages {
        let package = object(package)?;
        if string(field(package, "id")?)? == id {
            return Ok(package);
        }
    }
    Err(format!("cargo metadata has no package id {id}"))
}

fn node_by_id<'a>(nodes: &'a [Json], id: &str) -> Result<&'a [(String, Json)], String> {
    for node in nodes {
        let node = object(node)?;
        if string(field(node, "id")?)? == id {
            return Ok(node);
        }
    }
    Err(format!("cargo metadata resolve has no node {id}"))
}

/// Package id of the `oxijolt-sys` edge. Cargo spells that dep name with an underscore.
fn oxijolt_dep_id(node: &[(String, Json)]) -> Result<String, String> {
    let deps = array(field(node, "deps")?)?;
    for dep in deps {
        let dep = object(dep)?;
        let name = string(field(dep, "name")?)?;
        if name != "oxijolt_sys" && name != "oxijolt-sys" {
            continue;
        }
        return Ok(string(field(dep, "pkg")?)?.to_string());
    }
    Err("ferroan-physics does not resolve an oxijolt-sys dependency".to_string())
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
                     constants in build.rs. Update those constants with the oxijolt-sys version, \
                     and quote them beside D8.",
                    archive.name
                ));
            }
        }
    }
    if !saw_version {
        return Err(format!(
            "oxijolt-sys prebuilt.txt has no version={OXIJOLT_VERSION} line. \
             Update the SHA constants in build.rs with the oxijolt-sys version, and quote them beside D8."
        ));
    }
    if !saw_sources {
        return Err(
            "oxijolt-sys prebuilt.txt sources fingerprint differs from the constant in build.rs. \
             Update that constant with the oxijolt-sys version, and quote it beside D8."
                .to_string(),
        );
    }
    for (index, archive) in ARCHIVES.iter().enumerate() {
        if !saw_archive[index] {
            return Err(format!(
                "oxijolt-sys prebuilt.txt has no {} archive for the sha in build.rs. \
                 Update the SHA constants with the oxijolt-sys version, and quote them beside D8.",
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

enum Json {
    Null,
    Bool,
    Number,
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

fn object(value: &Json) -> Result<&[(String, Json)], String> {
    match value {
        Json::Object(fields) => Ok(fields),
        _ => Err("cargo metadata JSON value is not an object".to_string()),
    }
}

fn array(value: &Json) -> Result<&[Json], String> {
    match value {
        Json::Array(items) => Ok(items),
        _ => Err("cargo metadata JSON value is not an array".to_string()),
    }
}

fn string(value: &Json) -> Result<&str, String> {
    match value {
        Json::String(text) => Ok(text),
        _ => Err("cargo metadata JSON value is not a string".to_string()),
    }
}

fn field<'a>(object: &'a [(String, Json)], key: &str) -> Result<&'a Json, String> {
    object
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .ok_or_else(|| format!("cargo metadata JSON object has no {key}"))
}

struct Scan<'a> {
    bytes: &'a [u8],
    index: usize,
}

fn parse_json(text: &str) -> Result<Json, String> {
    let mut scan = Scan {
        bytes: text.as_bytes(),
        index: 0,
    };
    let value = scan.value()?;
    scan.skip();
    if scan.index != scan.bytes.len() {
        return Err("cargo metadata JSON has trailing bytes".to_string());
    }
    Ok(value)
}

impl Scan<'_> {
    fn skip(&mut self) {
        while self
            .bytes
            .get(self.index)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.index += 1;
        }
    }

    fn peek(&mut self) -> Result<u8, String> {
        self.skip();
        self.bytes
            .get(self.index)
            .copied()
            .ok_or_else(|| "unexpected end of cargo metadata JSON".to_string())
    }

    fn raw(&mut self) -> Result<u8, String> {
        let byte = self
            .bytes
            .get(self.index)
            .copied()
            .ok_or_else(|| "unexpected end of cargo metadata JSON".to_string())?;
        self.index += 1;
        Ok(byte)
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek()? {
            b'n' => self.literal(b"null", Json::Null),
            b't' => self.literal(b"true", Json::Bool),
            b'f' => self.literal(b"false", Json::Bool),
            b'"' => Ok(Json::String(self.string()?)),
            b'[' => self.array(),
            b'{' => self.object(),
            b'-' | b'0'..=b'9' => self.number(),
            other => Err(format!("unexpected cargo metadata JSON byte {other}")),
        }
    }

    fn literal(&mut self, text: &[u8], value: Json) -> Result<Json, String> {
        for expected in text {
            let byte = self.raw()?;
            if byte != *expected {
                return Err("cargo metadata JSON literal is truncated".to_string());
            }
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, String> {
        self.index += 1;
        while self.bytes.get(self.index).is_some_and(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-')
        }) {
            self.index += 1;
        }
        Ok(Json::Number)
    }

    fn array(&mut self) -> Result<Json, String> {
        if self.raw()? != b'[' {
            return Err("cargo metadata JSON array is missing '['".to_string());
        }
        let mut items = Vec::new();
        if self.peek()? == b']' {
            self.index += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            match self.peek()? {
                b',' => self.index += 1,
                b']' => {
                    self.index += 1;
                    break;
                }
                _ => return Err("cargo metadata JSON array is missing ',' or ']'".to_string()),
            }
        }
        Ok(Json::Array(items))
    }

    fn object(&mut self) -> Result<Json, String> {
        if self.raw()? != b'{' {
            return Err("cargo metadata JSON object is missing '{'".to_string());
        }
        let mut fields = Vec::new();
        if self.peek()? == b'}' {
            self.index += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            if self.peek()? != b'"' {
                return Err("cargo metadata JSON object key is not a string".to_string());
            }
            let key = self.string()?;
            if self.peek()? != b':' {
                return Err("cargo metadata JSON object is missing ':'".to_string());
            }
            self.index += 1;
            fields.push((key, self.value()?));
            match self.peek()? {
                b',' => self.index += 1,
                b'}' => {
                    self.index += 1;
                    break;
                }
                _ => return Err("cargo metadata JSON object is missing ',' or '}'".to_string()),
            }
        }
        Ok(Json::Object(fields))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.raw()? != b'"' {
            return Err("cargo metadata JSON string is missing '\"'".to_string());
        }
        let mut out = String::new();
        loop {
            match self.raw()? {
                b'"' => return Ok(out),
                b'\\' => out.push(self.escape()?),
                byte => {
                    let width = utf8_width(byte)?;
                    let start = self.index - 1;
                    let end = start + width;
                    if end > self.bytes.len() {
                        return Err("cargo metadata JSON string is truncated".to_string());
                    }
                    let text = std::str::from_utf8(&self.bytes[start..end])
                        .map_err(|_| "cargo metadata JSON string is not UTF-8".to_string())?;
                    out.push_str(text);
                    self.index = end;
                }
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        match self.raw()? {
            b'"' => Ok('"'),
            b'\\' => Ok('\\'),
            b'/' => Ok('/'),
            b'b' => Ok('\u{0008}'),
            b'f' => Ok('\u{000c}'),
            b'n' => Ok('\n'),
            b'r' => Ok('\r'),
            b't' => Ok('\t'),
            b'u' => self.unicode_escape(),
            _ => Err("cargo metadata JSON string has an unknown escape".to_string()),
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let unit = self.hex4()?;
        if !(0xD800..=0xDBFF).contains(&unit) {
            return char::from_u32(unit)
                .ok_or_else(|| "cargo metadata JSON string has a bad unicode escape".to_string());
        }
        if self.raw()? != b'\\' || self.raw()? != b'u' {
            return Err("cargo metadata JSON string has a lone surrogate".to_string());
        }
        let second = self.hex4()?;
        if !(0xDC00..=0xDFFF).contains(&second) {
            return Err("cargo metadata JSON string has a lone surrogate".to_string());
        }
        let scalar = 0x10000 + (((unit - 0xD800) << 10) | (second - 0xDC00));
        char::from_u32(scalar)
            .ok_or_else(|| "cargo metadata JSON string has a bad unicode escape".to_string())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut value = 0;
        for _ in 0..4 {
            let byte = self.raw()?;
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err("cargo metadata JSON unicode escape is not hex".to_string()),
            };
            value = (value << 4) | u32::from(digit);
        }
        Ok(value)
    }
}

fn utf8_width(first: u8) -> Result<usize, String> {
    let width = if first < 0x80 {
        1
    } else if first & 0xE0 == 0xC0 {
        2
    } else if first & 0xF0 == 0xE0 {
        3
    } else if first & 0xF8 == 0xF0 {
        4
    } else {
        return Err("cargo metadata JSON string is not UTF-8".to_string());
    };
    Ok(width)
}
