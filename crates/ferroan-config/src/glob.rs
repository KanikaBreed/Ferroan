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
    // `canonicalize` on Windows returns `\\?\C:\...`. The walk has to start
    // at that root; a normal `C:` split rejects the `\\?\` prefix.
    if let Some(rest) = strip_verbatim(pattern) {
        return rest;
    }
    if cfg!(windows)
        && let Some((volume, rest)) = pattern.split_once(':')
        && !volume.is_empty()
        && !volume.contains(['/', '\\'])
    {
        return (format!("{volume}:\\"), rest);
    }
    (std::path::MAIN_SEPARATOR.to_string(), pattern)
}

/// Splits a `\\?\C:\` or `\\?\UNC\server\share\` path into the root and the rest.
fn strip_verbatim(pattern: &str) -> Option<(String, &str)> {
    let after = pattern.strip_prefix(r"\\?\")?;
    if let Some(unc) = after.strip_prefix(r"UNC\") {
        let mut pieces = unc.split(['/', '\\']);
        let server = pieces.next().filter(|part| !part.is_empty())?;
        let share = pieces.next().filter(|part| !part.is_empty())?;
        let prefix_len = r"\\?\UNC\".len() + server.len() + 1 + share.len();
        let root = format!(r"\\?\UNC\{server}\{share}\");
        return Some((root, pattern.get(prefix_len..).unwrap_or("")));
    }
    let mut chars = after.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_alphabetic() || chars.next() != Some(':') {
        return None;
    }
    let root = format!(r"\\?\{letter}:\");
    Some((root, pattern.get(r"\\?\".len() + 2..).unwrap_or("")))
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
        // `is_dir` and `is_file` follow the link, as `GetDirectories` and
        // `GetFiles` do. `DirEntry::file_type` does not, so a symlink or a
        // Windows junction would otherwise hide the directory. A broken link
        // is neither, and is skipped.
        if want_dirs {
            if !path.is_dir() {
                continue;
            }
        } else if !path.is_file() {
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

#[cfg(test)]
mod tests {
    use super::strip_verbatim;

    #[test]
    fn verbatim_drive_and_unc_split() {
        assert_eq!(
            strip_verbatim(r"\\?\C:\Users\example\nested\*.ini"),
            Some((r"\\?\C:\".to_string(), r"\Users\example\nested\*.ini"))
        );
        assert_eq!(
            strip_verbatim(r"\\?\UNC\server\share\dir\*.ini"),
            Some((r"\\?\UNC\server\share\".to_string(), r"\dir\*.ini"))
        );
        assert_eq!(strip_verbatim(r"C:\Users"), None);
    }
}
