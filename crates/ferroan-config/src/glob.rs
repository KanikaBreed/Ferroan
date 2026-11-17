// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `OpenSim.Framework.Util.Glob` at Tranquillity `713857f`.
//!
//! The walk starts at the filesystem root (or the Windows drive root). Each
//! path component is a `Directory.GetDirectories` / `GetFiles` pattern: `*`
//! and `?` match inside that one component. The returned paths are sorted by
//! ordinal byte order so two includes that set the same key do not depend on
//! directory enumeration. The shipped examples never make that matter: every
//! include matches one file or none.

use std::path::{Path, PathBuf};

use crate::error::ConfigError;

const MAX_MATCHES: usize = 10_000;

pub(crate) fn glob(pattern: &str) -> Result<Vec<PathBuf>, ConfigError> {
    let (root, rest) = split_root(pattern);
    let components: Vec<&str> = rest
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    let mut directories = vec![PathBuf::from(&root)];
    let mut found = Vec::new();
    for (index, component) in components.iter().enumerate() {
        let last = index + 1 == components.len();
        let mut next = Vec::new();
        for directory in &directories {
            if last {
                found.extend(list_matching(directory, component, false)?);
            }
            next.extend(list_matching(directory, component, true)?);
        }
        directories = next;
        if found.len() > MAX_MATCHES {
            return Err(ConfigError::Limit {
                what: "glob matches",
            });
        }
    }
    found.sort();
    Ok(found)
}

fn split_root(pattern: &str) -> (String, &str) {
    if cfg!(windows)
        && let Some((volume, rest)) = pattern.split_once(':')
        && !volume.is_empty()
        && !volume.contains(['/', '\\'])
    {
        return (format!("{volume}:\\"), rest);
    }
    (std::path::MAIN_SEPARATOR.to_string(), pattern)
}

fn list_matching(
    directory: &Path,
    pattern: &str,
    want_dirs: bool,
) -> Result<Vec<PathBuf>, ConfigError> {
    let entries = std::fs::read_dir(directory).map_err(|error| ConfigError::Io {
        path: directory.display().to_string(),
        message: error.to_string(),
    })?;
    let mut matched = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ConfigError::Io {
            path: directory.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if want_dirs != is_dir {
            continue;
        }
        let Some(name) = path.file_name().and_then(|item| item.to_str()) else {
            continue;
        };
        if name_matches(pattern, name) {
            matched.push(path);
        }
    }
    Ok(matched)
}

fn name_matches(pattern: &str, name: &str) -> bool {
    match_at(pattern.as_bytes(), name.as_bytes())
}

fn match_at(pattern: &[u8], name: &[u8]) -> bool {
    let mut pattern_index = 0;
    let mut name_index = 0;
    while pattern_index < pattern.len() {
        match pattern[pattern_index] {
            b'*' => {
                if pattern_index + 1 == pattern.len() {
                    return true;
                }
                for skip in name_index..=name.len() {
                    if match_at(&pattern[pattern_index + 1..], &name[skip..]) {
                        return true;
                    }
                }
                return false;
            }
            b'?' => {
                if name_index >= name.len() {
                    return false;
                }
                pattern_index += 1;
                name_index += 1;
            }
            byte => {
                if name_index >= name.len() || !byte_eq(byte, name[name_index]) {
                    return false;
                }
                pattern_index += 1;
                name_index += 1;
            }
        }
    }
    name_index == name.len()
}

fn byte_eq(pattern_byte: u8, name_byte: u8) -> bool {
    if cfg!(windows) {
        pattern_byte.eq_ignore_ascii_case(&name_byte)
    } else {
        pattern_byte == name_byte
    }
}
