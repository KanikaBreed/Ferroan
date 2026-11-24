// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Exercise the public persistent-grid commands, using their printed login
//! address and password file. A wire LogoutReply and settled Robust services
//! are both required before reusing the account.

#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

const REPORT_LIMIT: usize = 64 * 1024;
const LOG_LIMIT: u64 = 16 * 1024 * 1024;
const REPORT_FIELDS: &[&str] = &[
    "login_uri",
    "robust_private_uri",
    "account",
    "password_file",
    "robust_pid",
    "home_pid",
    "neighbour_pid",
    "robust_public_http",
    "robust_private_http",
    "home_http",
    "home_udp",
    "neighbour_http",
    "neighbour_udp",
    "ferroan_commit",
];

pub(crate) fn run() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        crate::live::finish(run_linux())
    }
    #[cfg(not(target_os = "linux"))]
    {
        crate::live::finish(Err("cargo xtask live runs on Linux".into()))
    }
}

struct Report {
    login_uri: String,
    private_uri: String,
    password_file: PathBuf,
    pids: [u32; 3],
    tcp: [SocketAddr; 4],
    udp: [SocketAddr; 2],
}

impl Report {
    fn parse(text: &str) -> Result<Self, String> {
        if text.len() > REPORT_LIMIT {
            return Err("persistent grid report exceeds 64 KiB".into());
        }
        let mut fields = BTreeMap::new();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once(": ")
                // Build progress precedes the report and may repeat labels.
                && REPORT_FIELDS.contains(&key)
                && fields.insert(key, value).is_some()
            {
                return Err("persistent grid report contains duplicate fields".into());
            }
        }
        let field = |key| {
            fields
                .get(key)
                .copied()
                .ok_or_else(|| format!("persistent grid report lacks {key}"))
        };
        let address = |key| -> Result<SocketAddr, String> {
            let value = field(key)?
                .parse::<SocketAddr>()
                .map_err(|_| format!("persistent grid report has invalid {key}"))?;
            if !value.ip().is_loopback() || value.port() == 0 {
                return Err(format!("persistent grid check requires loopback {key}"));
            }
            Ok(value)
        };
        let pid = |key| -> Result<u32, String> {
            field(key)?
                .parse::<u32>()
                .ok()
                .filter(|pid| *pid > 1)
                .ok_or_else(|| format!("persistent grid report has invalid {key}"))
        };
        let tcp = [
            address("robust_public_http")?,
            address("robust_private_http")?,
            address("home_http")?,
            address("neighbour_http")?,
        ];
        let udp = [address("home_udp")?, address("neighbour_udp")?];
        let login_uri = field("login_uri")?;
        let private_uri = field("robust_private_uri")?;
        if login_uri != format!("http://{}/", tcp[0])
            || private_uri.trim_end_matches('/') != format!("http://{}", tcp[1])
        {
            return Err("persistent grid printed URLs do not match its listeners".into());
        }
        if field("account")? != "Test User" {
            return Err("persistent grid did not print its test account".into());
        }
        let commit = field("ferroan_commit")?;
        if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("persistent grid did not print a full Ferroan commit".into());
        }
        let pids = [pid("robust_pid")?, pid("home_pid")?, pid("neighbour_pid")?];
        if pids[0] == pids[1] || pids[0] == pids[2] || pids[1] == pids[2] {
            return Err("persistent grid did not print three distinct processes".into());
        }
        Ok(Self {
            login_uri: login_uri.to_owned(),
            private_uri: private_uri.trim_end_matches('/').to_owned(),
            password_file: PathBuf::from(field("password_file")?),
            pids,
            tcp,
            udp,
        })
    }
}

#[cfg(target_os = "linux")]
fn run_linux() -> Result<String, String> {
    if crate::live::standalone_is_up()?.is_some() || crate::grid_state::survivor()?.is_some() {
        return Err(
            "a live harness is already running; stop it before the persistent grid check".into(),
        );
    }
    let directory = if crate::ports::isolated() {
        crate::ports::run_root().join("persistent-grid-check")
    } else {
        crate::live::live_root().join("persistent-grid-check")
    };
    fs::create_dir_all(&directory).map_err(|_| "could not create persistent grid check logs")?;
    let mut observed = None;
    let outcome = (|| {
        let stdout = command(
            &["live", "up", "grid", "--region", "ferroan"],
            &directory,
            "up",
            Duration::from_secs(600),
        )?;
        let report = Report::parse(&read_text(&stdout, REPORT_LIMIT)?)?;
        let identities = report
            .pids
            .iter()
            .map(|pid| {
                crate::live::process_start_time(*pid)
                    .ok_or_else(|| format!("persistent grid process {pid} exited before login"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        observed = Some((report, identities));
        let report = &observed
            .as_ref()
            .ok_or("missing persistent grid observation")?
            .0;
        let secret = fs::canonicalize(&report.password_file)
            .map_err(|_| "could not resolve the printed password file")?;
        let live_root = fs::canonicalize(crate::live::live_root())
            .map_err(|_| "could not resolve the live harness directory")?;
        if !secret.starts_with(live_root) {
            return Err("the printed password file is outside the live harness directory".into());
        }
        let password = read_text(&secret, 1024)?;
        let region_log = secret
            .parent()
            .unwrap_or(secret.as_path())
            .join("home/region.log");
        let mut harness = crate::live::harness(
            &report.login_uri,
            &report.private_uri,
            password.trim(),
            "",
            &region_log,
        )?;
        harness.require_logout_reply = true;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| "could not start the persistent grid check runtime")?;
        runtime.block_on(async {
            for attempt in 1..=2 {
                tokio::time::timeout(
                    Duration::from_secs(120),
                    ferroan_bot::scenario::run("login", &harness),
                )
                .await
                .map_err(|_| format!("persistent grid login {attempt} timed out"))?
                .map_err(|error| {
                    let path = directory.join(format!("login-{attempt}.error"));
                    match crate::live::write_secret(&path, &error) {
                        Ok(()) => format!("persistent grid login {attempt} failed; private diagnostics: {}", path.display()),
                        Err(_) => format!("persistent grid login {attempt} failed; could not save private diagnostics"),
                    }
                })?;
                crate::grid::login_probe::offline(&report.private_uri).await?;
                println!("persistent grid: login {attempt}, wire LogoutReply and offline service state passed");
            }
            Ok::<(), String>(())
        })
    })();
    // Even a timed-out or interrupted startup can leave recorded children.
    let cleanup = command(
        &["live", "down"],
        &directory,
        "down",
        Duration::from_secs(150),
    );
    let stopped = if cleanup.is_ok() {
        verify_stopped(observed.as_ref())
    } else {
        Ok(())
    };
    let errors: Vec<_> = [outcome.err(), cleanup.err(), stopped.err()]
        .into_iter()
        .flatten()
        .collect();
    if errors.is_empty() {
        Ok("persistent grid: up, two logins, down passed; no owned process or TCP/UDP listener remains".into())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(target_os = "linux")]
fn leftover_grid_state() -> Option<PathBuf> {
    let legacy = crate::live::live_root().join("grid-state");
    if legacy.is_file() {
        return Some(legacy);
    }
    let runs = crate::live::live_root().join("runs");
    let entries = fs::read_dir(&runs).ok()?;
    for entry in entries.flatten() {
        let path = entry.path().join("grid-state");
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn verify_stopped(observed: Option<&(Report, Vec<u64>)>) -> Result<(), String> {
    if let Some(path) = leftover_grid_state() {
        return Err(format!(
            "live down left grid process state behind ({})",
            path.display()
        ));
    }
    if let Some((report, identities)) = observed {
        for (pid, start) in report.pids.iter().zip(identities) {
            if crate::live::process_start_time(*pid) == Some(*start) {
                return Err(format!(
                    "live down left persistent grid process {pid} alive"
                ));
            }
        }
        for addr in report.tcp {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
                _ => return Err(format!("live down did not close TCP listener {addr}")),
            }
        }
        for addr in report.udp {
            UdpSocket::bind(addr)
                .map_err(|_| format!("live down left UDP port {addr} occupied"))?;
        }
    }
    Ok(())
}

fn read_text(path: &Path, limit: usize) -> Result<String, String> {
    let mut text = String::new();
    File::open(path)
        .and_then(|file| file.take(limit as u64 + 1).read_to_string(&mut text))
        .map_err(|_| "could not read bounded persistent grid check file")?;
    if text.len() > limit {
        return Err("persistent grid check file exceeds its size limit".into());
    }
    Ok(text)
}

/// Keep raw command diagnostics private and bound both output and wall time.
fn command(
    args: &[&str],
    directory: &Path,
    name: &str,
    timeout: Duration,
) -> Result<PathBuf, String> {
    let stdout = directory.join(format!("{name}.stdout"));
    let stderr = directory.join(format!("{name}.stderr"));
    let out = File::create(&stdout).map_err(|_| "could not create command stdout log")?;
    let err = File::create(&stderr).map_err(|_| "could not create command stderr log")?;
    // The child installs the same base and reserves that block. Listeners
    // still held here make every slot look busy to that child.
    crate::ports::release_block();
    let mut child = Command::new(std::env::current_exe().map_err(|_| "could not find xtask")?)
        .args(args)
        .current_dir(crate::repo_root())
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|_| "could not start persistent grid command")?;
    let deadline = Instant::now() + timeout;
    loop {
        let poll = child.try_wait();
        let oversized = [&stdout, &stderr]
            .iter()
            .any(|path| fs::metadata(path).map_or(true, |meta| meta.len() > LOG_LIMIT));
        if let Ok(Some(status)) = poll {
            return if status.success() && !oversized {
                Ok(stdout)
            } else {
                Err(format!(
                    "persistent grid {name} failed; private diagnostics: {}",
                    stderr.display()
                ))
            };
        }
        if poll.is_err() || oversized || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "persistent grid {name} exceeded command bounds or could not be observed"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> String {
        "login_uri: http://127.0.0.1:8002/\nrobust_private_uri: http://127.0.0.1:8003\naccount: Test User\npassword_file: /scratch/live/password\nrobust_pid: 42\nhome_pid: 43\nneighbour_pid: 44\nrobust_public_http: 127.0.0.1:8002\nrobust_private_http: 127.0.0.1:8003\nhome_http: 127.0.0.1:9000\nhome_udp: 127.0.0.1:9000\nneighbour_http: 127.0.0.1:9001\nneighbour_udp: 127.0.0.1:9001\nferroan_commit: 5591af8b46ffe5f66b16ba0dd8eb8dcc87ff6acc\n".into()
    }

    #[test]
    fn printed_login_uri_must_name_the_reported_public_listener() {
        let valid = Report::parse(&report()).unwrap();
        assert_eq!(valid.login_uri, "http://127.0.0.1:8002/");
        assert!(Report::parse(&report().replacen("8002/", "8003/", 1)).is_err());
        assert!(Report::parse(&report().replace("127.0.0.1", "192.0.2.1")).is_err());
    }

    #[test]
    fn persistent_report_requires_all_distinct_processes_and_unique_fields() {
        assert!(Report::parse(&report().replace("neighbour_pid: 44\n", "")).is_err());
        assert!(
            Report::parse(&report().replace("neighbour_pid: 44", "neighbour_pid: 43")).is_err()
        );
        assert!(Report::parse(&format!("{}account: Other User\n", report())).is_err());
        assert!(Report::parse(&"x".repeat(REPORT_LIMIT + 1)).is_err());
    }

    #[test]
    fn persistent_report_ignores_repeated_build_progress_labels() {
        let output = format!(
            "ferrobot: building pinned client 9cc7ab284c78b7f17ae209039e8a3405a12620b6\nferrobot: build passed (.scratch/live/ferrobot)\nmode: grid\n{}",
            report()
        );
        let parsed = Report::parse(&output).unwrap();
        assert_eq!(parsed.pids, [42, 43, 44]);
        assert_eq!(parsed.login_uri, "http://127.0.0.1:8002/");
        assert!(Report::parse(&format!("{output}home_pid: 45\n")).is_err());
    }
}
