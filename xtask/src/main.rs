// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Repository tasks, run as `cargo xtask <task>`.
//!
//! `check` verifies that every Markdown link to a local file resolves, then
//! runs formatting, clippy (warnings denied) and the workspace tests. With
//! `--docs-only` it stops after the Markdown check.
//!
//! `oracle` builds and runs the C# reference oracle: `build`, `generate` and
//! `verify`. The Linux live workflow runs `verify` against pinned references.
//!
//! `schema` builds the C# schema oracle and generates or verifies the SQLite
//! fixtures. The Linux live workflow runs `verify` with the .NET SDK and
//! reference checkouts.
//!
//! `config` builds the C# config oracle and generates or verifies the resolved
//! INI fixtures. The Linux live workflow runs `verify` with the .NET SDK and
//! reference checkouts.
//!
//! `http` builds the C# ServerUtils oracle and generates or verifies the
//! query-string and Robust XML fixtures. `http status-verify` runs the pinned
//! Grid, Presence and GridUser connectors against the status fixtures. Both
//! need the .NET SDK and the reference checkouts. The Linux live workflow
//! runs `verify` and `status-verify`.
//!
//! `asset` builds the C# asset oracle and generates or verifies `AssetBase`
//! XML fixtures. It needs the .NET SDK and the reference checkouts. The Linux
//! live workflow runs `verify`.
//!
//! `live` builds the pinned Tranquillity region into `.scratch/live/`,
//! starts or stops it, and runs bot scenarios against it. It runs on Linux.
//! The live CI job runs standalone and grid mode.
//!
//! `docker build` builds the Ferroan and pinned Tranquillity images without
//! pushing them. `docker smoke` logs in through that grid. Both run on Linux.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod accounts;
mod arrival_fixtures;
mod asset_oracle;
mod assets;
mod caps_fixtures;
mod circuit_fixtures;
mod config_oracle;
mod docker;
mod ferrobot;
mod ferrobot_build;
mod grid;
mod grid_smoke;
mod grid_state;
mod http_oracle;
mod inventory_probe;
mod live;
mod live_args;
mod live_proxy;
mod nodelay;
mod oracle;
mod oracle_reference;
mod pin3_oracle;
mod schema;
mod trailing_fixtures;
mod twice;

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
       cargo xtask oracle verify [fixture-dir]
       cargo xtask schema generate [directory]
       cargo xtask schema verify [directory]
       cargo xtask config generate [directory]
       cargo xtask config verify [directory]
       cargo xtask http generate [directory]
       cargo xtask http verify [directory]
       cargo xtask http status-generate
       cargo xtask http status-verify
       cargo xtask asset generate [directory]
       cargo xtask asset verify [directory]
       cargo xtask pin3 generate|verify
       cargo xtask live build
       cargo xtask live record-twice
       cargo xtask live up [--assets ferroan]
       cargo xtask live up grid --region ferroan [--bind <IPv4>]
       cargo xtask live persistent-grid-check
       cargo xtask live down
       cargo xtask live run <scenario>|all [grid] [--region ferroan] [--rust <service>,…] [--db sqlite|mysql|pgsql]
       cargo xtask live run login grid --region ferroan
       cargo xtask live run caps grid [--region ferroan]
       cargo xtask live run ferrobot [grid [--region ferroan]]
       cargo xtask live run assets grid --region ferroan
       cargo xtask live run <scenario>|all grid --rust assets [--db sqlite]
       cargo xtask live run login grid --rust accounts
       cargo xtask live run login grid --rust assets,accounts
       cargo xtask live caps-fixtures <capture-dir>
       cargo xtask live login-fixtures <capture-dir>
       cargo xtask live region-parity-fixtures <capture-dir>
       cargo xtask live root-arrival-fixtures <capture-dir>
       cargo xtask live circuit-fixtures <udp-capture>
       cargo xtask live arrival-fixtures <udp-capture>
       cargo xtask live trailing-fixtures <udp-capture>
       cargo xtask live run assets --running
       cargo xtask live compare <service> [--db sqlite|mysql|pgsql]
       cargo xtask live assets [--db sqlite|mysql|pgsql]
       cargo xtask live accounts [--db sqlite|mysql|pgsql]
       cargo xtask docker build
       cargo xtask docker smoke";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if refs.first() == Some(&"live") {
        return match live_args::dispatch(&refs[1..]) {
            live_args::Outcome::Exit(code) => code,
            live_args::Outcome::Usage => {
                eprintln!("{USAGE}");
                ExitCode::from(2)
            }
        };
    }
    match refs.as_slice() {
        ["check"] => check(false),
        ["pin3", "generate"] => pin3_oracle::run(true),
        ["pin3", "verify"] => pin3_oracle::run(false),
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
        ["schema", "generate"] => schema::generate(None),
        ["schema", "generate", directory] => schema::generate(Some(PathBuf::from(directory))),
        ["schema", "verify"] => schema::verify(None),
        ["schema", "verify", directory] => schema::verify(Some(PathBuf::from(directory))),
        ["config", "generate"] => config_oracle::generate(None),
        ["config", "generate", directory] => {
            config_oracle::generate(Some(PathBuf::from(directory)))
        }
        ["config", "verify"] => config_oracle::verify(None),
        ["config", "verify", directory] => config_oracle::verify(Some(PathBuf::from(directory))),
        ["http", "generate"] => http_oracle::generate(None),
        ["http", "generate", directory] => http_oracle::generate(Some(PathBuf::from(directory))),
        ["http", "verify"] => http_oracle::verify(None),
        ["http", "verify", directory] => http_oracle::verify(Some(PathBuf::from(directory))),
        ["http", "status-generate"] => http_oracle::status_generate(),
        ["http", "status-verify"] => http_oracle::status_verify(),
        ["asset", "generate"] => asset_oracle::generate(None),
        ["asset", "generate", directory] => asset_oracle::generate(Some(PathBuf::from(directory))),
        ["asset", "verify"] => asset_oracle::verify(None),
        ["asset", "verify", directory] => asset_oracle::verify(Some(PathBuf::from(directory))),
        ["docker", "build"] => docker::build(),
        ["docker", "smoke"] => docker::smoke(),
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
