// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Refuse absolute home and mount paths in tracked files.
//!
//! `cargo xtask check --docs-only` runs this. A test may use the fake
//! directory name `example` (`/home/example/`, `/Users/example/`,
//! `/mnt/example/`, or a `C:` Users path ending in `example`).

use std::path::Path;
use std::process::Command;

pub(crate) fn check(root: &Path) -> bool {
    let pattern = git_pattern();
    let output = match Command::new("git")
        .args(["grep", "-a", "-n", "-E", &pattern, "--", "."])
        .current_dir(root)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            eprintln!("personal paths: could not run git grep: {error}");
            return false;
        }
    };
    if output.status.code() == Some(1) && output.stdout.is_empty() {
        println!("Personal paths: no home or mount path in tracked files.");
        return true;
    }
    if !output.status.success() && output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("personal paths: git grep failed: {stderr}");
        return false;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut problems = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        match forbidden_in_grep_line(line) {
            Some(problem) => problems.push(problem),
            None if line.starts_with("Binary file ") => {
                problems.push(line.to_string());
            }
            None => {}
        }
    }
    if problems.is_empty() {
        println!("Personal paths: no home or mount path in tracked files.");
        return true;
    }
    for problem in &problems {
        eprintln!("personal paths: {problem}");
    }
    false
}

fn git_pattern() -> String {
    let windows = windows_users_prefix().replace('\\', "\\\\");
    format!(
        "/home/[A-Za-z0-9][A-Za-z0-9._-]*/|/Users/[A-Za-z0-9][A-Za-z0-9._-]*/|/mnt/[A-Za-z0-9][A-Za-z0-9._-]*/|{windows}"
    )
}

fn windows_users_prefix() -> String {
    let mut prefix = String::from("C:");
    prefix.push('\\');
    prefix.push_str("Users");
    prefix.push('\\');
    prefix
}

fn forbidden_in_grep_line(line: &str) -> Option<String> {
    let (path, rest) = line.split_once(':')?;
    let (number, content) = rest.split_once(':')?;
    let hits = forbidden_paths(content);
    if hits.is_empty() {
        return None;
    }
    Some(format!("{path}:{number}: {}", hits.join(", ")))
}

fn forbidden_paths(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    scan_slash_path(text, "/home/", &mut hits);
    scan_slash_path(text, "/Users/", &mut hits);
    scan_slash_path(text, "/mnt/", &mut hits);
    scan_windows_users(text, &mut hits);
    hits
}

fn scan_slash_path(text: &str, prefix: &str, hits: &mut Vec<String>) {
    let mut from = 0;
    while let Some(relative) = text[from..].find(prefix) {
        let start = from + relative;
        let name_at = start + prefix.len();
        let rest = &text[name_at..];
        let name_len = name_len(rest);
        if name_len > 0 && rest[name_len..].starts_with('/') {
            let name = &rest[..name_len];
            if name != "example" {
                hits.push(text[start..name_at + name_len + 1].to_string());
            }
        }
        from = name_at;
    }
}

fn scan_windows_users(text: &str, hits: &mut Vec<String>) {
    let prefix = windows_users_prefix();
    let mut from = 0;
    while let Some(relative) = text[from..].find(&prefix) {
        let start = from + relative;
        let name_at = start + prefix.len();
        let rest = &text[name_at..];
        let name_len = name_len(rest);
        let name = &rest[..name_len];
        if name != "example" {
            hits.push(text[start..name_at + name_len].to_string());
        }
        from = name_at;
    }
}

fn name_len(text: &str) -> usize {
    text.chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        .map(char::len_utf8)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_paths_are_allowed_and_other_names_are_not() {
        let allowed = format!("/home/{}/repo", "example");
        assert!(forbidden_paths(&allowed).is_empty(), "{allowed}");
        let bad_name = "ada";
        let home = format!("/home/{bad_name}/repo");
        assert_eq!(forbidden_paths(&home), vec![format!("/home/{bad_name}/")]);
        assert!(forbidden_paths(&format!("/Users/{}/repo", "example")).is_empty());
        let users = format!("/Users/{bad_name}/repo");
        assert_eq!(forbidden_paths(&users), vec![format!("/Users/{bad_name}/")]);
        assert!(forbidden_paths(&format!("/mnt/{}/repo", "example")).is_empty());
        let mount_name = "data";
        let mount = format!("/mnt/{mount_name}/repo");
        assert_eq!(forbidden_paths(&mount), vec![format!("/mnt/{mount_name}/")]);
        let windows_ok = format!("C:{}Users{}example{}", '\\', '\\', '\\');
        assert!(forbidden_paths(&windows_ok).is_empty(), "{windows_ok}");
        let windows_bad = format!("C:{}Users{}runner{}", '\\', '\\', '\\');
        assert_eq!(
            forbidden_paths(&windows_bad),
            vec![windows_users_prefix() + "runner"]
        );
        assert!(forbidden_paths("refuses `/home/` paths").is_empty());
        assert!(forbidden_paths("set FERROAN_REF or ~/Git/reference").is_empty());
    }
}
