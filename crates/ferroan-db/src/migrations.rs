// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Read the version numbers in Tranquillity's migration resources.
//!
//! The rules follow `OpenSim.Data.Migration.GetMigrationsAfter` at the pinned
//! revision: a new-style `Store.migrations` file is a sequence of `:VERSION n`
//! sections, and an old-style `NNN_Store.sql` file is version `NNN`. The
//! version left in the `migrations` table after an update from empty is the
//! highest of those numbers. This module does not execute the SQL.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Highest migration version for each store file in `dir`.
///
/// Names are sorted. A file that is neither `*.migrations` nor `NNN_Store.sql`
/// is ignored. A store that appears in both styles keeps the higher version.
pub fn store_versions_in_dir(dir: &Path) -> Result<Vec<(String, i32)>, String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("could not read {}: {error}", dir.display()))?;
    let mut versions = BTreeMap::<String, i32>::new();
    let mut saw_file = false;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read {}: {error}", dir.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        saw_file = true;
        if let Some((store, version)) = old_style(name) {
            insert_max(&mut versions, store, version);
            continue;
        }
        if let Some(store) = new_style_store(name) {
            let text = fs::read_to_string(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            let version = max_version_marker(&text)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let Some(version) = version else {
                return Err(format!("{} has no :VERSION marker", path.display()));
            };
            insert_max(&mut versions, store, version);
        }
    }
    if !saw_file {
        return Err(format!("{} has no migration files", dir.display()));
    }
    if versions.is_empty() {
        return Err(format!("{} has no store migrations", dir.display()));
    }
    Ok(versions.into_iter().collect())
}

fn insert_max(versions: &mut BTreeMap<String, i32>, store: String, version: i32) {
    versions
        .entry(store)
        .and_modify(|current| {
            if version > *current {
                *current = version;
            }
        })
        .or_insert(version);
}

/// `001_GridUserStore.sql` -> (`GridUserStore`, 1).
fn old_style(file_name: &str) -> Option<(String, i32)> {
    let (prefix, store) = file_name.split_once('_')?;
    let store = store.strip_suffix(".sql")?;
    if prefix.len() != 3 || !prefix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if store.is_empty() || store.contains('.') {
        return None;
    }
    let version = prefix.parse().ok()?;
    Some((store.to_string(), version))
}

/// `AssetStore.migrations` or `AssetStore.migrations.006` -> `AssetStore`.
fn new_style_store(file_name: &str) -> Option<String> {
    let (store, rest) = file_name.split_once(".migrations")?;
    if store.is_empty() || store.contains('.') {
        return None;
    }
    if rest.is_empty() {
        return Some(store.to_string());
    }
    let rest = rest.strip_prefix('.')?;
    if rest.is_empty() || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(store.to_string())
}

/// Highest positive `:VERSION n` marker, using Tranquillity's line rules.
///
/// An invalid marker is an error. Tranquillity stops at the first invalid
/// marker, so a later valid marker must not raise the result.
pub fn max_version_marker(text: &str) -> Result<Option<i32>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut highest: Option<i32> = None;
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(rest) = version_prefix(line) else {
            continue;
        };
        let rest = match rest.find('#') {
            Some(hash) => &rest[..hash],
            None => rest,
        };
        let rest = rest.trim();
        let version: i32 = rest
            .parse()
            .map_err(|_| format!("invalid version marker on line {}: {line}", index + 1))?;
        if version <= 0 {
            return Err(format!(
                "version marker on line {} is not positive",
                index + 1
            ));
        }
        highest = Some(highest.map_or(version, |current| current.max(version)));
    }
    Ok(highest)
}

/// The text after a leading `:VERSION `, compared case-insensitively.
fn version_prefix(line: &str) -> Option<&str> {
    const PREFIX: &str = ":version ";
    let mut line_bytes = line.bytes();
    for expected in PREFIX.bytes() {
        let actual = line_bytes.next()?.to_ascii_lowercase();
        if actual != expected {
            return None;
        }
    }
    Some(&line[PREFIX.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_markers_skip_comments_and_keep_the_highest() {
        let text = "\u{feff}:VERSION 1\n# :VERSION 99\n\nCREATE TABLE t(a);\n:VERSION 4 # skipped comment\n:version 2\n";
        assert_eq!(max_version_marker(text).unwrap(), Some(4));
    }

    #[test]
    fn invalid_marker_is_an_error() {
        let text = ":VERSION 2\n:VERSION no\n:VERSION 9\n";
        assert!(max_version_marker(text).is_err());
    }

    #[test]
    fn old_style_name_parses_the_three_digit_prefix() {
        let parsed = old_style("001_GridUserStore.sql").unwrap();
        assert_eq!(parsed.0, "GridUserStore");
        assert_eq!(parsed.1, 1);
        assert!(old_style("AssetStore.migrations").is_none());
        assert_eq!(
            new_style_store("os_groups_Store.migrations").as_deref(),
            Some("os_groups_Store")
        );
    }
}
