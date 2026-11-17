// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Repository tasks, run as `cargo xtask <task>`.
//!
//! `check` verifies that every Markdown link to a local file resolves, then
//! runs formatting, clippy (warnings denied) and the workspace tests. With
//! `--docs-only` it stops after the Markdown check.
//!
//! `oracle` builds and runs the C# reference oracle: `build`, `generate` and
//! `verify`. CI does not run it.

mod oracle;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};

/// Directories never searched for Markdown files.
const SKIP_DIRS: &[&str] = &[".git", "target", ".scratch"];

const USAGE: &str = "\
usage: cargo xtask check [--docs-only]
       cargo xtask oracle build
       cargo xtask oracle generate [fixture-dir]
       cargo xtask oracle verify [fixture-dir]";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match refs.as_slice() {
        ["check"] => check(false),
        ["check", "--docs-only"] => check(true),
        ["oracle", "build"] => oracle::run(oracle::CommandKind::Build, None),
        ["oracle", "generate"] => oracle::run(oracle::CommandKind::Generate, None),
        ["oracle", "verify"] => oracle::run(oracle::CommandKind::Verify, None),
        ["oracle", "generate", dir] => {
            oracle::run(oracle::CommandKind::Generate, Some(PathBuf::from(*dir)))
        }
        ["oracle", "verify", dir] => {
            oracle::run(oracle::CommandKind::Verify, Some(PathBuf::from(*dir)))
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn check(docs_only: bool) -> ExitCode {
    let root = repo_root();
    if !check_markdown(&root) {
        return ExitCode::FAILURE;
    }
    if docs_only {
        return ExitCode::SUCCESS;
    }
    let steps: [&[&str]; 3] = [
        &["fmt", "--all", "--", "--check"],
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        &["test", "--workspace", "--locked"],
    ];
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    for step in steps {
        println!("$ cargo {}", step.join(" "));
        match Command::new(&cargo).args(step).current_dir(&root).status() {
            Ok(status) if status.success() => {}
            Ok(_) => return ExitCode::FAILURE,
            Err(error) => {
                eprintln!("could not run cargo: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// The workspace root: the parent of this crate's manifest directory.
pub(crate) fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(Path::to_path_buf).unwrap_or(manifest)
}

fn check_markdown(root: &Path) -> bool {
    let mut files = Vec::new();
    collect_markdown(root, &mut files);
    files.sort();
    let mut problems = Vec::new();
    for file in &files {
        match fs::read_to_string(file) {
            Ok(text) => problems.extend(broken_links(root, file, &text)),
            Err(error) => problems.push(format!("{}: unreadable: {error}", display(root, file))),
        }
    }
    for problem in &problems {
        eprintln!("{problem}");
    }
    if problems.is_empty() {
        println!("Markdown: local links resolve in {} files.", files.len());
    }
    problems.is_empty()
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if !SKIP_DIRS.iter().any(|skip| name == *skip) {
                collect_markdown(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

/// Reports links in `text` whose local target does not exist inside `root`.
fn broken_links(root: &Path, file: &Path, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut fenced = false;
    let base = file.parent().unwrap_or(root);
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        for target in link_targets(line) {
            let Some(path) = local_path(target) else {
                continue;
            };
            let resolved = normalize(&base.join(path));
            if !resolved.starts_with(root) || !resolved.exists() {
                problems.push(format!(
                    "{}:{}: missing local target {target}",
                    display(root, file),
                    index + 1
                ));
            }
        }
    }
    problems
}

/// The targets of inline links `[text](target)` on one line.
fn link_targets(line: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        let end = after
            .find(|c: char| c == ')' || c.is_whitespace())
            .unwrap_or(after.len());
        let target = after[..end].trim_start_matches('<').trim_end_matches('>');
        if !target.is_empty() {
            targets.push(target);
        }
        rest = &after[end..];
    }
    targets
}

/// The decoded file path of a link, or `None` for remote URLs and in-page anchors.
fn local_path(target: &str) -> Option<String> {
    if target.contains("://") || target.starts_with("mailto:") || target.starts_with('#') {
        return None;
    }
    let path = target.split(['#', '?']).next().unwrap_or("");
    if path.is_empty() {
        return None;
    }
    Some(percent_decode(path))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Resolves `.` and `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn display(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_inline_link_targets() {
        let line = "See [a](docs/A.md), [b](<B.md>) and [c](https://example.com).";
        assert_eq!(
            link_targets(line),
            vec!["docs/A.md", "B.md", "https://example.com"]
        );
    }

    #[test]
    fn skips_remote_links_and_anchors() {
        assert_eq!(local_path("https://example.com/x.md"), None);
        assert_eq!(local_path("#section"), None);
        assert_eq!(local_path("docs/A.md#part").as_deref(), Some("docs/A.md"));
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("My%20File.md"), "My File.md");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("bad%zzescape"), "bad%zzescape");
    }

    #[test]
    fn normalizes_parent_components() {
        assert_eq!(
            normalize(Path::new("/r/docs/../README.md")),
            PathBuf::from("/r/README.md")
        );
    }

    #[test]
    fn reports_missing_targets_outside_fences() {
        let root = repo_root();
        let file = root.join("README.md");
        let text = "[ok](LICENSE)\n[gone](no-such-file.md)\n```\n[ignored](missing.md)\n```\n";
        let problems = broken_links(&root, &file, text);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("no-such-file.md"));
    }
}
