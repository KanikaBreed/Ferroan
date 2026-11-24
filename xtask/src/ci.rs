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
//!
//! `cargo xtask ci live-port-base --offset <0|32|64|96>` prints the harness
//! port base for one live job. `cargo xtask ci live-db-ports` prints the
//! MySQL and PostgreSQL host ports for that same workflow run. Both read
//! `GITHUB_RUN_ID`. A local run leaves `FERROAN_LIVE_PORT_BASE` unset; these
//! commands are what CI uses so its listeners stay off that default.

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

/// First harness port a CI job may use. Local runs leave the port base unset.
pub(crate) const LIVE_PORT_ORIGIN: u16 = 8192;
/// Ports reserved for one workflow run: four jobs, each with a span of 32.
pub(crate) const LIVE_PORT_SLOT_STRIDE: u16 = 128;
/// Distinct run slots. The workflow run id modulo this value picks the slot.
pub(crate) const LIVE_PORT_SLOTS: u64 = 400;
/// Scenarios, Ferrobot, services, then images.
pub(crate) const LIVE_JOB_OFFSETS: [u16; 4] = [0, 32, 64, 96];
/// Distinct database host-port slots. The run id modulo this value picks one.
pub(crate) const LIVE_DB_SLOTS: u64 = 2000;
/// MySQL host ports sit above every harness slot.
pub(crate) const MYSQL_PORT_ORIGIN: u16 = 60000;
/// PostgreSQL host ports sit above the MySQL range.
pub(crate) const POSTGRES_PORT_ORIGIN: u16 = 62000;

/// Harness base for one CI job. `offset` is one of [`LIVE_JOB_OFFSETS`].
/// Two jobs in one run do not share a port. Two runs share a base only when
/// their run ids are equal modulo [`LIVE_PORT_SLOTS`].
pub(crate) fn live_port_base(run_id: u64, offset: u16) -> Result<u16, String> {
    if !LIVE_JOB_OFFSETS.contains(&offset) {
        return Err(format!("port offset {offset} is not one of 0, 32, 64, 96"));
    }
    let slot = u16::try_from(run_id % LIVE_PORT_SLOTS)
        .map_err(|_| "the port slot does not fit in a u16".to_string())?;
    let base_u32 = u32::from(LIVE_PORT_ORIGIN)
        + u32::from(slot) * u32::from(LIVE_PORT_SLOT_STRIDE)
        + u32::from(offset);
    let base = u16::try_from(base_u32)
        .map_err(|_| format!("port base {base_u32} does not fit in a u16"))?;
    if u32::from(base) + u32::from(crate::ports::SPAN) - 1 > u32::from(u16::MAX) {
        return Err(format!(
            "port base {base} does not leave room for {} ports",
            crate::ports::SPAN
        ));
    }
    Ok(base)
}

/// Host ports for the services job's MySQL and PostgreSQL containers.
/// They sit above the harness range and away from the local schema ports.
pub(crate) fn live_db_ports(run_id: u64) -> Result<(u16, u16), String> {
    let slot = u16::try_from(run_id % LIVE_DB_SLOTS)
        .map_err(|_| "the database port slot does not fit in a u16".to_string())?;
    let mysql = u16::try_from(u32::from(MYSQL_PORT_ORIGIN) + u32::from(slot))
        .map_err(|_| "the MySQL port does not fit in a u16".to_string())?;
    let postgres = u16::try_from(u32::from(POSTGRES_PORT_ORIGIN) + u32::from(slot))
        .map_err(|_| "the PostgreSQL port does not fit in a u16".to_string())?;
    Ok((mysql, postgres))
}

fn github_run_id() -> Result<u64, String> {
    let text = env::var("GITHUB_RUN_ID").map_err(|_| "GITHUB_RUN_ID is not set".to_string())?;
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("GITHUB_RUN_ID is not a number".to_string());
    }
    text.parse::<u64>()
        .map_err(|_| "GITHUB_RUN_ID is not a number".to_string())
}

pub(crate) fn print_live_port_base(offset: &str) -> ExitCode {
    let offset: u16 = match offset.parse() {
        Ok(value) => value,
        Err(_) => {
            eprintln!("ci live-port-base: offset is not a number");
            return ExitCode::from(2);
        }
    };
    let run_id = match github_run_id() {
        Ok(id) => id,
        Err(error) => {
            eprintln!("ci live-port-base: {error}");
            return ExitCode::from(2);
        }
    };
    match live_port_base(run_id, offset) {
        Ok(base) => {
            println!("{base}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ci live-port-base: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn print_live_db_ports() -> ExitCode {
    let run_id = match github_run_id() {
        Ok(id) => id,
        Err(error) => {
            eprintln!("ci live-db-ports: {error}");
            return ExitCode::from(2);
        }
    };
    match live_db_ports(run_id) {
        Ok((mysql, postgres)) => {
            println!("{mysql} {postgres}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ci live-db-ports: {error}");
            ExitCode::from(2)
        }
    }
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
        LIVE_JOB_OFFSETS, LIVE_PORT_ORIGIN, LIVE_PORT_SLOTS, MYSQL_PORT_ORIGIN,
        POSTGRES_PORT_ORIGIN, SqliteInstall, db_client_install, documentation_only, evaluate,
        live_db_ports, live_port_base, paths_are_docs_only, sqlite_install,
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
    fn ci_port_bases_stay_apart_and_off_the_local_default() {
        let runs = [0_u64, 1, 399, 400, 401, 45352025217, u64::MAX];
        for run_id in runs {
            let mut bases = Vec::new();
            for offset in LIVE_JOB_OFFSETS {
                let base = live_port_base(run_id, offset).unwrap();
                assert!(base >= LIVE_PORT_ORIGIN);
                assert!(u32::from(base) + 31 < u32::from(MYSQL_PORT_ORIGIN));
                bases.push(base);
            }
            for left in 0..bases.len() {
                for right in (left + 1)..bases.len() {
                    let (lo, hi) = if bases[left] < bases[right] {
                        (bases[left], bases[right])
                    } else {
                        (bases[right], bases[left])
                    };
                    assert!(u32::from(hi) >= u32::from(lo) + 32);
                }
            }
            let (mysql, postgres) = live_db_ports(run_id).unwrap();
            assert!((60000..=61999).contains(&mysql));
            assert!((62000..=63999).contains(&postgres));
            assert_ne!(mysql, postgres);
            assert!(mysql < POSTGRES_PORT_ORIGIN);
        }
        assert!(live_port_base(1, 1).is_err());
        assert!(live_port_base(1, 128).is_err());
        // The next slot starts where this slot's last job ends.
        let last = live_port_base(0, 96).unwrap();
        let next = live_port_base(1, 0).unwrap();
        assert_eq!(u32::from(next), u32::from(last) + 32);
        // 400 run ids later is the same slot.
        assert_eq!(
            live_port_base(5, 64).unwrap(),
            live_port_base(5 + LIVE_PORT_SLOTS, 64).unwrap()
        );
        let (mysql, postgres) = live_db_ports(0).unwrap();
        assert_eq!(mysql, MYSQL_PORT_ORIGIN);
        assert_eq!(postgres, POSTGRES_PORT_ORIGIN);
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
