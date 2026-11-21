// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pull-request scope and the live-job gate.
//!
//! `cargo xtask ci scope` writes `docs_only=true` or `docs_only=false` to
//! `GITHUB_OUTPUT` (or stdout when that variable is unset). A push, a
//! schedule, a manual run, a missing base, and a fetch or diff failure are
//! not documentation-only, so the full suite still runs.
//!
//! `cargo xtask ci gate` is the required check `live (ubuntu-latest)`. It
//! fails unless every parallel live job succeeded. A documentation-only run
//! succeeds only when those jobs were skipped.

use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Command, ExitCode};

/// Markdown a program reads. A search of `xtask/src` on 20 November 2026
/// found these and no others: the pin and the Tranquillity cache key, the
/// limit-id check (and `crates/ferroan-diag`), and the root readme.
const PROGRAM_READ: &[&str] = &["README.md", "docs/REFERENCES.md", "docs/LIMITS.md"];

/// True when every path is Markdown and none is a file a program reads.
/// An empty list is false.
pub(crate) fn paths_are_docs_only(paths: &str) -> bool {
    let mut found = false;
    for path in paths.lines() {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        found = true;
        if !path.ends_with(".md") || PROGRAM_READ.contains(&path) {
            return false;
        }
    }
    found
}

/// `pull_request` plus a base revision can be documentation-only. Anything
/// else, including a diff that could not be read, is not.
pub(crate) fn documentation_only(event: &str, base_sha: &str, diff: Result<&str, ()>) -> bool {
    if event != "pull_request" || !is_full_sha(base_sha) {
        return false;
    }
    match diff {
        Ok(paths) => paths_are_docs_only(paths),
        Err(()) => false,
    }
}

fn is_full_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn scope() -> ExitCode {
    let event = env::var("EVENT_NAME").unwrap_or_default();
    let base = env::var("BASE_SHA").unwrap_or_default();
    let docs_only = if event == "pull_request" && is_full_sha(&base) {
        match changed_paths(&base) {
            Ok(paths) => documentation_only(&event, &base, Ok(&paths)),
            Err(error) => {
                eprintln!("ci scope: {error}");
                documentation_only(&event, &base, Err(()))
            }
        }
    } else {
        documentation_only(&event, &base, Err(()))
    };
    if let Err(error) = write_docs_only(docs_only) {
        eprintln!("ci scope: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// `--no-renames` lists a renamed source file under its old name, so a rename
/// from code to Markdown does not look like a documentation-only change.
fn changed_paths(base: &str) -> Result<String, String> {
    let fetch = Command::new("git")
        .args(["fetch", "--no-tags", "--depth=1", "origin", base])
        .output()
        .map_err(|error| format!("could not run git fetch: {error}"))?;
    if !fetch.status.success() {
        return Err("git fetch of the base revision failed".to_string());
    }
    let diff = Command::new("git")
        .args(["diff", "--name-only", "--no-renames", base, "HEAD"])
        .output()
        .map_err(|error| format!("could not run git diff: {error}"))?;
    if !diff.status.success() {
        return Err("git diff against the base revision failed".to_string());
    }
    String::from_utf8(diff.stdout).map_err(|_| "git diff output is not UTF-8".to_string())
}

fn write_docs_only(docs_only: bool) -> Result<(), String> {
    let line = format!("docs_only={}\n", if docs_only { "true" } else { "false" });
    match env::var_os("GITHUB_OUTPUT") {
        Some(path) => {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|error| format!("could not append GITHUB_OUTPUT: {error}"))?;
            file.write_all(line.as_bytes())
                .map_err(|error| format!("could not write GITHUB_OUTPUT: {error}"))?;
        }
        None => print!("{line}"),
    }
    Ok(())
}

/// The required live check. `docs_only` is the string `true` or anything else.
pub(crate) fn evaluate(
    scope: &str,
    docs_only: &str,
    scenarios: &str,
    ferrobot: &str,
    services: &str,
    images: &str,
) -> Result<(), String> {
    if scope != "success" {
        return Err(format!("live scope result: {scope}"));
    }
    let expected = if docs_only == "true" {
        "skipped"
    } else {
        "success"
    };
    for (name, actual) in [
        ("scenarios", scenarios),
        ("ferrobot", ferrobot),
        ("services", services),
        ("images", images),
    ] {
        if actual != expected {
            return Err(format!("{name} result: {actual} (expected {expected})"));
        }
    }
    Ok(())
}

pub(crate) fn gate() -> ExitCode {
    let read = |name: &str| env::var(name).map_err(|_| format!("{name} is not set"));
    let result = (|| {
        evaluate(
            &read("SCOPE")?,
            &read("DOCS_ONLY")?,
            &read("SCENARIOS")?,
            &read("FERROBOT")?,
            &read("SERVICES")?,
            &read("IMAGES")?,
        )
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{documentation_only, evaluate, paths_are_docs_only};

    #[test]
    fn documentation_only_paths_skip_the_live_run() {
        assert!(paths_are_docs_only("docs/TESTING.md\ndocs/LIVE.md\n"));
        assert!(paths_are_docs_only("crates/ferroan-region/README.md\n"));
        assert!(!paths_are_docs_only(""));
        assert!(!paths_are_docs_only("README.md\n"));
        assert!(!paths_are_docs_only("docs/REFERENCES.md\n"));
        assert!(!paths_are_docs_only("docs/LIMITS.md\n"));
        assert!(!paths_are_docs_only("docs/TESTING.md\nxtask/src/main.rs\n"));
        assert!(!paths_are_docs_only(
            "docs/TESTING.md\n.github/workflows/live.yml\n"
        ));
    }

    #[test]
    fn a_push_or_a_schedule_is_not_documentation_only() {
        let paths = "docs/TESTING.md\n";
        let sha = "a".repeat(40);
        assert!(!documentation_only("push", &sha, Ok(paths)));
        assert!(!documentation_only("schedule", &sha, Ok(paths)));
        assert!(!documentation_only("workflow_dispatch", &sha, Ok(paths)));
        assert!(!documentation_only("pull_request", "", Ok(paths)));
        assert!(!documentation_only("pull_request", &sha, Err(())));
        assert!(documentation_only("pull_request", &sha, Ok(paths)));
    }

    #[test]
    fn the_live_gate_fails_when_any_parallel_job_fails() {
        assert!(
            evaluate(
                "success", "false", "success", "success", "success", "success"
            )
            .is_ok()
        );
        assert_eq!(
            evaluate(
                "success", "false", "failure", "success", "success", "success"
            )
            .unwrap_err(),
            "scenarios result: failure (expected success)"
        );
        assert!(
            evaluate(
                "success",
                "false",
                "cancelled",
                "success",
                "success",
                "success"
            )
            .is_err()
        );
        assert_eq!(
            evaluate(
                "failure", "false", "skipped", "skipped", "skipped", "skipped"
            )
            .unwrap_err(),
            "live scope result: failure"
        );
        assert!(
            evaluate(
                "success", "true", "skipped", "skipped", "skipped", "skipped"
            )
            .is_ok()
        );
        assert!(
            evaluate(
                "success", "true", "success", "skipped", "skipped", "skipped"
            )
            .is_err()
        );
    }
}
