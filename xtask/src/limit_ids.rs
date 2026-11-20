// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `docs/LIMITS.md` must list every id in `catalog.rs`, and each const must be
//! named from a Rust file outside `ferroan-diag`.

use std::fs;
use std::path::Path;

const SKIP: &[&str] = &[".git", "target", ".scratch"];

pub(crate) fn check(root: &Path) -> bool {
    let catalog = root.join("crates/ferroan-diag/src/catalog.rs");
    let docs = root.join("docs/LIMITS.md");
    let consts = match fs::read_to_string(&catalog) {
        Ok(text) => parse_consts(&text),
        Err(error) => {
            eprintln!("limits: could not read {}: {error}", catalog.display());
            return false;
        }
    };
    let listed = match fs::read_to_string(&docs) {
        Ok(text) => parse_table(&text),
        Err(error) => {
            eprintln!("limits: could not read {}: {error}", docs.display());
            return false;
        }
    };
    let mut problems = Vec::new();
    for (name, id) in &consts {
        if !listed.iter().any(|row| row == id) {
            problems.push(format!("`{id}` ({name}) is missing from docs/LIMITS.md"));
        }
    }
    for id in &listed {
        if !consts.iter().any(|(_, const_id)| const_id == id) {
            problems.push(format!(
                "docs/LIMITS.md lists `{id}`, which is not in catalog.rs"
            ));
        }
    }
    let mut sources = Vec::new();
    collect_rust(root, &mut sources);
    for (name, id) in &consts {
        let used = sources.iter().any(|text| contains_ident(text, name));
        if !used {
            problems.push(format!(
                "{name} (`{id}`) has no call site outside ferroan-diag"
            ));
        }
    }
    if problems.is_empty() {
        return true;
    }
    for problem in &problems {
        eprintln!("limits: {problem}");
    }
    false
}

fn parse_consts(text: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("pub const ") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once(':') else {
            continue;
        };
        let Some(id) = rest.split('"').nth(1) else {
            continue;
        };
        if !id.is_empty() {
            rows.push((name.trim().to_owned(), id.to_owned()));
        }
    }
    rows
}

fn parse_table(text: &str) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let mut cells = line.split('|');
        let _ = cells.next();
        let Some(cell) = cells.next() else {
            continue;
        };
        let cell = cell.trim();
        if let Some(id) = cell
            .strip_prefix('`')
            .and_then(|rest| rest.strip_suffix('`'))
            && !id.is_empty()
        {
            rows.push(id.to_owned());
        }
    }
    rows
}

fn collect_rust(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if SKIP.contains(&name) || name == "ferroan-diag" {
            continue;
        }
        if path.is_dir() {
            collect_rust(&path, out);
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "rs")
            && let Ok(text) = fs::read_to_string(&path)
        {
            out.push(text);
        }
    }
}

fn contains_ident(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();
    let needle = name.as_bytes();
    let mut start = 0;
    while let Some(offset) = text[start..].find(name) {
        let at = start + offset;
        let before_ok = at == 0 || !is_ident(bytes[at - 1]);
        let after = at + needle.len();
        let after_ok = after >= bytes.len() || !is_ident(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        start = at + name.len();
    }
    false
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
