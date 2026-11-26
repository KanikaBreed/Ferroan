// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pull-request scope, the live-job gate, and the sqlite3 install step.
//!
//! `cargo xtask ci scope` writes `docs_only=true` or `docs_only=false` to
//! `GITHUB_OUTPUT` (or stdout when that variable is unset). A push, a
//! schedule, a manual run, a missing base, and a fetch or diff failure are
//! not documentation-only, so the full suite still runs.
//!
//! `cargo xtask ci gate` is the required check `live (ubuntu-latest)`. It
//! fails unless every parallel live job succeeded. A documentation-only run
//! succeeds only when those jobs were skipped.
//!
//! `cargo xtask ci ensure-sqlite` installs the `sqlite3` command when it is
//! missing and `apt-get` is on `PATH`. When `sqlite3` is already present it
//! does nothing, which is the Arch runner (no `apt-get`).

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

const LIVE_JOBS: [&str; 6] = [
    "SCENARIOS",
    "FERROBOT",
    "SERVICES_REGION",
    "SERVICES_GRID",
    "SERVICES_ASSETS",
    "IMAGES",
];

/// Check each direct dependency of the required live gate.
fn evaluate(scope: &str, docs_only: &str, jobs: &[(&str, String)]) -> Result<(), String> {
    if scope != "success" {
        return Err(format!("live scope result: {scope}"));
    }
    let expected = if docs_only == "true" {
        "skipped"
    } else {
        "success"
    };
    for (name, actual) in jobs {
        if actual != expected {
            return Err(format!("{name} result: {actual} (expected {expected})"));
        }
    }
    Ok(())
}

pub(crate) fn gate() -> ExitCode {
    let read = |name: &str| env::var(name).map_err(|_| format!("{name} is not set"));
    let result = (|| {
        let mut jobs = Vec::with_capacity(LIVE_JOBS.len());
        for name in LIVE_JOBS {
            jobs.push((name, read(name)?));
        }
        evaluate(&read("SCOPE")?, &read("DOCS_ONLY")?, &jobs)
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// What `ensure-sqlite` does. Tests call this; they do not run `apt-get`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SqliteInstall {
    /// `sqlite3` is already on `PATH`.
    Present,
    /// Install it with `apt-get`.
    Apt,
}

/// `Present` when `sqlite3` is on `PATH`. `Apt` when it is missing and
/// `apt-get` exists. An error when neither exists, which is an Arch machine
/// that has not installed the `sqlite3` command.
pub(crate) fn sqlite_install(
    has_sqlite: bool,
    has_apt: bool,
) -> Result<SqliteInstall, &'static str> {
    if has_sqlite {
        Ok(SqliteInstall::Present)
    } else if has_apt {
        Ok(SqliteInstall::Apt)
    } else {
        Err("sqlite3 is not installed and apt-get is not available")
    }
}

pub(crate) fn ensure_sqlite() -> ExitCode {
    let choice = match sqlite_install(command_on_path("sqlite3"), command_on_path("apt-get")) {
        Ok(choice) => choice,
        Err(error) => {
            eprintln!("ci ensure-sqlite: {error}");
            return ExitCode::FAILURE;
        }
    };
    match choice {
        SqliteInstall::Present => ExitCode::SUCCESS,
        SqliteInstall::Apt => match apt_install_sqlite() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("ci ensure-sqlite: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

fn command_on_path(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

fn apt_install_sqlite() -> Result<(), String> {
    run_apt(&["update"], "apt-get failed to install sqlite3")?;
    run_apt(
        &["install", "-y", "sqlite3"],
        "apt-get failed to install sqlite3",
    )
}

/// `Present` when `mysql` and `psql` are both on `PATH`. `Apt` when either is
/// missing and `apt-get` exists.
pub(crate) fn db_client_install(
    has_mysql: bool,
    has_psql: bool,
    has_apt: bool,
) -> Result<SqliteInstall, &'static str> {
    if has_mysql && has_psql {
        Ok(SqliteInstall::Present)
    } else if has_apt {
        Ok(SqliteInstall::Apt)
    } else {
        Err("mysql and psql are not both installed and apt-get is not available")
    }
}

pub(crate) fn ensure_db_clients() -> ExitCode {
    let choice = match db_client_install(
        command_on_path("mysql"),
        command_on_path("psql"),
        command_on_path("apt-get"),
    ) {
        Ok(choice) => choice,
        Err(error) => {
            eprintln!("ci ensure-db-clients: {error}");
            return ExitCode::FAILURE;
        }
    };
    match choice {
        SqliteInstall::Present => ExitCode::SUCCESS,
        SqliteInstall::Apt => match apt_install_db_clients() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("ci ensure-db-clients: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

fn apt_install_db_clients() -> Result<(), String> {
    run_apt(&["update"], "apt-get failed to install mysql and psql")?;
    run_apt(
        &["install", "-y", "mysql-client", "postgresql-client"],
        "apt-get failed to install mysql and psql",
    )
}

fn run_apt(args: &[&str], failure: &str) -> Result<(), String> {
    let status = Command::new("sudo")
        .args(["-n", "apt-get"])
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .status()
        .map_err(|error| format!("could not run apt-get: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(failure.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LIVE_JOBS, SqliteInstall, db_client_install, documentation_only, evaluate,
        paths_are_docs_only, sqlite_install,
    };

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
        let mut jobs = LIVE_JOBS.map(|name| (name, String::from("success")));
        assert!(evaluate("success", "false", &jobs).is_ok());
        for i in 0..jobs.len() {
            for bad in ["failure", "cancelled", "skipped"] {
                jobs[i].1 = bad.into();
                assert!(
                    evaluate("success", "false", &jobs).is_err(),
                    "{} {bad}",
                    jobs[i].0
                );
            }
            jobs[i].1 = "success".into();
        }
        let mut skipped = LIVE_JOBS.map(|name| (name, String::from("skipped")));
        assert!(evaluate("failure", "true", &skipped).is_err());
        assert!(evaluate("success", "true", &skipped).is_ok());
        skipped[0].1 = "success".into();
        assert!(evaluate("success", "true", &skipped).is_err());
    }

    #[test]
    fn the_required_gate_reads_each_result_and_the_services_blocks_are_disjoint() {
        let workflow = include_str!("../../.github/workflows/live.yml");
        let (_, gate) = workflow.split_once("\n  live:\n").unwrap();
        let needs = gate
            .lines()
            .find_map(|line| line.trim().strip_prefix("needs: [")?.strip_suffix(']'))
            .unwrap();
        let mut bases = std::collections::BTreeSet::new();
        for var in LIVE_JOBS {
            let job = var.to_ascii_lowercase();
            assert!(needs.split(", ").any(|needed| needed == job));
            assert!(gate.contains(&format!("{var}: ${{{{ needs.{job}.result }}}}")));
            if job.starts_with("services_") {
                let (_, rest) = workflow.split_once(&format!("\n  {job}:\n")).unwrap();
                let body = rest.split_once("\n\n").unwrap().0;
                assert!(body.contains("needs: scope"));
                let base: u16 = body
                    .lines()
                    .find_map(|line| {
                        line.trim()
                            .strip_prefix("FERROAN_LIVE_PORT_BASE: ")?
                            .trim_matches('"')
                            .parse()
                            .ok()
                    })
                    .unwrap();
                assert!((1024..=32736).contains(&base) && base.is_multiple_of(crate::ports::SPAN));
                assert!(bases.insert(base), "reused port base {base}");
            }
        }
    }

    #[test]
    fn sqlite_is_installed_only_when_it_is_missing() {
        assert_eq!(sqlite_install(true, true), Ok(SqliteInstall::Present));
        assert_eq!(sqlite_install(true, false), Ok(SqliteInstall::Present));
        assert_eq!(sqlite_install(false, true), Ok(SqliteInstall::Apt));
        assert_eq!(
            sqlite_install(false, false),
            Err("sqlite3 is not installed and apt-get is not available")
        );
    }

    #[test]
    fn db_clients_are_installed_only_when_one_is_missing() {
        assert_eq!(
            db_client_install(true, true, true),
            Ok(SqliteInstall::Present)
        );
        assert_eq!(
            db_client_install(true, true, false),
            Ok(SqliteInstall::Present)
        );
        assert_eq!(db_client_install(false, true, true), Ok(SqliteInstall::Apt));
        assert_eq!(db_client_install(true, false, true), Ok(SqliteInstall::Apt));
        assert_eq!(
            db_client_install(false, false, false),
            Err("mysql and psql are not both installed and apt-get is not available")
        );
    }
}
