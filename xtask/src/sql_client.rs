// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! MySQL and PostgreSQL clients for the live asset compares.
//!
//! A machine with `mysql` or `psql` on `PATH` uses that program. Home1 has
//! neither, and the live workflow does not install them with `apt-get`. The
//! compares then use the client in the `mysql:8.4` or `postgres:16` image.
//! The password stays in a defaults file or in `PGPASSWORD`.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::Path;
use std::process::{Command, Output, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientKind {
    Host,
    Docker,
}

/// `Host` when the client program is installed. `Docker` when it is missing
/// and `docker` can run the image client. An error when neither is available.
pub(crate) fn client_kind(has_program: bool, has_docker: bool) -> Result<ClientKind, &'static str> {
    if has_program {
        Ok(ClientKind::Host)
    } else if has_docker {
        Ok(ClientKind::Docker)
    } else {
        Err("the database client is not installed and docker is not available")
    }
}

#[cfg(target_os = "linux")]
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

#[cfg(target_os = "linux")]
fn absolute(path: &Path) -> Result<std::path::PathBuf, String> {
    std::fs::canonicalize(path)
        .map_err(|error| format!("could not resolve {}: {error}", path.display()))
}

/// Runs `mysql` against `database`. `defaults` is a client defaults file that
/// holds the password. `extra` is inserted before the database name.
#[cfg(target_os = "linux")]
pub(crate) fn run_mysql(
    defaults: &Path,
    host: &str,
    port: &str,
    user: &str,
    database: &str,
    extra: &[&str],
    stdin: Stdio,
) -> Result<Output, String> {
    let kind = client_kind(on_path("mysql"), on_path("docker")).map_err(str::to_string)?;
    let mut command = match kind {
        ClientKind::Host => {
            let defaults_arg = defaults
                .to_str()
                .ok_or("mysql defaults path is not utf-8")?;
            let mut command = Command::new("mysql");
            command.arg(format!("--defaults-extra-file={defaults_arg}"));
            command
        }
        ClientKind::Docker => {
            let defaults_abs = absolute(defaults)?;
            let defaults_arg = defaults_abs
                .to_str()
                .ok_or("mysql defaults path is not utf-8")?;
            let mut command = Command::new("docker");
            command.args(["run", "--rm", "-i", "--network", "host", "-v"]);
            command.arg(format!("{defaults_arg}:/client.cnf:ro"));
            command.args([
                "mysql:8.4",
                "mysql",
                "--defaults-extra-file=/client.cnf",
                "--get-server-public-key",
            ]);
            command
        }
    };
    command.args([
        "--protocol=TCP",
        "-h",
        host,
        "-P",
        port,
        "-u",
        user,
        "--ssl-mode=DISABLED",
        "--batch",
    ]);
    command.args(extra);
    command.arg(database);
    command
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
        .output()
        .map_err(|error| format!("could not start mysql: {error}"))
}

/// Runs `psql` on `sql_path`. `extra` is inserted before `-f`.
#[cfg(target_os = "linux")]
pub(crate) fn run_psql(
    password: &str,
    host: &str,
    port: &str,
    user: &str,
    database: &str,
    sql_path: &Path,
    extra: &[&str],
) -> Result<Output, String> {
    let kind = client_kind(on_path("psql"), on_path("docker")).map_err(str::to_string)?;
    let mut command = match kind {
        ClientKind::Host => {
            let mut command = Command::new("psql");
            command
                .env("PGPASSWORD", password)
                .args(["-h", host, "-p", port, "-U", user, "-d", database]);
            command
        }
        ClientKind::Docker => {
            let sql_abs = absolute(sql_path)?;
            let sql_arg = sql_abs.to_str().ok_or("postgres seed path is not utf-8")?;
            let mut command = Command::new("docker");
            command.env("PGPASSWORD", password).args([
                "run",
                "--rm",
                "-i",
                "--network",
                "host",
                "-e",
                "PGPASSWORD",
                "-v",
            ]);
            command.arg(format!("{sql_arg}:/seed.sql:ro"));
            command.args([
                "postgres:16",
                "psql",
                "-h",
                host,
                "-p",
                port,
                "-U",
                user,
                "-d",
                database,
            ]);
            command
        }
    };
    command.args(["-v", "ON_ERROR_STOP=1"]);
    command.args(extra);
    match kind {
        ClientKind::Host => {
            command.arg("-f");
            command.arg(sql_path);
        }
        ClientKind::Docker => {
            command.args(["-f", "/seed.sql"]);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
        .output()
        .map_err(|error| format!("could not start psql: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{ClientKind, client_kind};

    #[test]
    fn a_host_client_wins_and_docker_covers_a_missing_client() {
        assert_eq!(client_kind(true, true), Ok(ClientKind::Host));
        assert_eq!(client_kind(true, false), Ok(ClientKind::Host));
        assert_eq!(client_kind(false, true), Ok(ClientKind::Docker));
        assert!(client_kind(false, false).is_err());
    }
}
