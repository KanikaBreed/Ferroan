// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build D17's separately executed test client without changing a reference
//! checkout or adding it to the Ferroan dependency graph.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const REPOSITORY: &str = "https://github.com/BlackBox-Roleplay-Systems/Ferrobot.git";
const MAX_METADATA: usize = 64 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
const BUILD_TIMEOUT: Duration = Duration::from_secs(900);

pub(crate) fn build() -> Result<PathBuf, String> {
    let root = crate::repo_root();
    let scratch = root.join(".scratch");
    fs::create_dir_all(&scratch).map_err(|_| "could not create Ferrobot scratch directory")?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(scratch.join("ferrobot-build.lock"))
        .map_err(|_| "could not open Ferrobot build lock")?;
    lock.try_lock()
        .map_err(|_| "another Ferrobot build is active")?;
    let pin = parse_pin(&read_bounded(&root.join("docs/REFERENCES.md"))?)?;
    let source = source(&scratch, reference().as_deref(), &pin)?;
    let target = scratch.join("ferrobot-target");
    let log_path = scratch.join("ferrobot-build.log");
    let log = fs::File::create(&log_path).map_err(|_| "could not create Ferrobot build log")?;
    let errors = log
        .try_clone()
        .map_err(|_| "could not open Ferrobot build log")?;
    let mut command = Command::new("cargo");
    command
        .current_dir(&source)
        .args([
            "build",
            "--locked",
            "-p",
            "ferrobot",
            "--bin",
            "ferrobot",
            "--target-dir",
        ])
        .arg(&target)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(errors);
    println!("ferrobot: building pinned client {pin}");
    run(&mut command, BUILD_TIMEOUT)
        .map_err(|error| format!("Ferrobot build {error}; see {}", log_path.display()))?;
    // --locked forbids lockfile changes; catch any unexpected source mutation
    // by build tooling rather than silently accepting changed scratch source.
    if !clean_at(&source, &pin)? {
        return Err(
            "Ferrobot source changed during its build; checkout preserved for inspection"
                .to_owned(),
        );
    }
    let binary = target
        .join("debug")
        .join(format!("ferrobot{}", env::consts::EXE_SUFFIX));
    if !binary.is_file() {
        return Err("Ferrobot build produced no executable".to_owned());
    }
    println!("ferrobot: build passed ({pin})");
    Ok(binary)
}

fn source(scratch: &Path, reference: Option<&Path>, pin: &str) -> Result<PathBuf, String> {
    let reference = reference.filter(|path| path.is_dir() && clean_at(path, pin).unwrap_or(false));
    let checkout = scratch.join("ferrobot-src");
    if fs::symlink_metadata(&checkout).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(
            "scratch Ferrobot source must not be a symlink; references are read-only".to_owned(),
        );
    }
    if checkout.exists() {
        if clean_at(&checkout, pin)? {
            return Ok(checkout);
        }
        return Err("scratch Ferrobot checkout is dirty or at another revision; preserve it and move it aside before rebuilding".to_owned());
    }
    // Clone into a private staging directory. Failure or interruption never
    // makes a partially fetched checkout look like the cached source.
    let temporary = scratch.join(format!("ferrobot-clone-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&temporary).map_err(|_| "could not create Ferrobot clone directory")?;
    let result = (|| {
        if let Some(reference) = reference {
            let reference = reference
                .to_str()
                .ok_or("Ferrobot reference path is not UTF-8")?;
            let destination = temporary
                .to_str()
                .ok_or("Ferrobot scratch path is not UTF-8")?;
            // References are only read. Even with an external target directory,
            // Cargo/build scripts must never run with a reference as their cwd.
            git(
                scratch,
                &[
                    "clone",
                    "--quiet",
                    "--no-hardlinks",
                    "--no-checkout",
                    "--",
                    reference,
                    destination,
                ],
            )?;
        } else {
            git(&temporary, &["init", "--quiet"])?;
            git(
                &temporary,
                &["fetch", "--quiet", "--depth=1", REPOSITORY, pin],
            )?;
        }
        git(&temporary, &["checkout", "--quiet", "--detach", pin])?;
        if !clean_at(&temporary, pin)? {
            return Err("downloaded Ferrobot checkout does not match the clean pin".to_owned());
        }
        fs::rename(&temporary, &checkout)
            .map_err(|_| "could not save pinned Ferrobot source".to_owned())?;
        Ok(checkout)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn reference() -> Option<PathBuf> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let root = match env::var_os("FERROAN_REF").filter(|value| !value.is_empty()) {
        Some(value) => {
            if let Some(tail) = value.to_str().and_then(|value| value.strip_prefix("~/")) {
                home?.join(tail)
            } else {
                PathBuf::from(value)
            }
        }
        None => home?.join("Git/reference"),
    };
    Some(root.join("Ferrobot"))
}

fn parse_pin(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_METADATA {
        return Err("reference document exceeds size limit".to_owned());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "reference document is not UTF-8")?;
    let mut rows = text.lines().filter(|line| {
        line.split('|')
            .nth(1)
            .is_some_and(|cell| cell.trim() == "`Ferrobot`")
    });
    let row = rows
        .next()
        .ok_or("docs/REFERENCES.md has no Ferrobot pin")?;
    if rows.next().is_some() {
        return Err("docs/REFERENCES.md has multiple Ferrobot pins".to_owned());
    }
    let pin = row
        .split('|')
        .nth(3)
        .ok_or("Ferrobot reference row has no revision")?
        .trim()
        .trim_matches('`');
    if pin.len() != 40 || !pin.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Ferrobot pin must be a full 40-character Git revision".to_owned());
    }
    Ok(pin.to_ascii_lowercase())
}

fn clean_at(path: &Path, pin: &str) -> Result<bool, String> {
    // Do not mistake a directory inside the Ferroan repository for a checkout.
    if !path.join(".git").exists()
        || !path.join("Cargo.toml").is_file()
        || !path.join("Cargo.lock").is_file()
    {
        return Ok(false);
    }
    let revision = git(path, &["rev-parse", "--verify", "HEAD"])?;
    if revision.trim() != pin {
        return Ok(false);
    }
    Ok(git(path, &["status", "--porcelain", "--untracked-files=all"])?.is_empty())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| "could not open reference document")?;
    let mut bytes = Vec::new();
    file.take(MAX_METADATA as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read reference document")?;
    if bytes.len() > MAX_METADATA {
        return Err("reference document exceeds size limit".to_owned());
    }
    Ok(bytes)
}

fn git(path: &Path, args: &[&str]) -> Result<String, String> {
    let deadline = Instant::now() + GIT_TIMEOUT;
    let mut command = Command::new("git");
    command
        .current_dir(path)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| "could not start git for Ferrobot")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("git output pipe was not created")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(MAX_METADATA as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    // A failed/timed-out git helper can leave a descendant holding stdout.
    // Detach the bounded reader in that case instead of hanging on join.
    wait(&mut child, GIT_TIMEOUT).map_err(|error| format!("Ferrobot git {error}"))?;
    let bytes = receive_metadata(receiver, deadline.saturating_duration_since(Instant::now()))?;
    if bytes.len() > MAX_METADATA {
        return Err("Ferrobot git metadata exceeds size limit".to_owned());
    }
    String::from_utf8(bytes).map_err(|_| "Ferrobot git metadata is not UTF-8".to_owned())
}

fn receive_metadata(
    receiver: mpsc::Receiver<std::io::Result<Vec<u8>>>,
    remaining: Duration,
) -> Result<Vec<u8>, String> {
    receiver
        .recv_timeout(remaining)
        .map_err(|_| "Ferrobot git output did not finish within its timeout")?
        .map_err(|_| "could not read Ferrobot git metadata".to_owned())
}

fn run(command: &mut Command, timeout: Duration) -> Result<(), String> {
    let mut child = command
        .spawn()
        .map_err(|_| "could not start command".to_owned())?;
    wait(&mut child, timeout)
}

fn wait(child: &mut std::process::Child, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("failed ({status})"))
                };
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                stop(child);
                return Err("timed out".to_owned());
            }
            Err(_) => {
                stop(child);
                return Err("could not wait for command".to_owned());
            }
        }
    }
}

fn stop(child: &mut std::process::Child) {
    // Build/fetch inherit the terminal process group, so Ctrl-C reaches their
    // helpers too. A timeout stops the direct child; compiler descendants can
    // finish naturally, and the build log stays available for inspection.
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_pin_is_exact_and_ambiguous_or_unbounded_input_is_rejected() {
        let pin = parse_pin(include_bytes!("../../docs/REFERENCES.md")).unwrap();
        assert_eq!(pin, "9cc7ab284c78b7f17ae209039e8a3405a12620b6");
        let row = format!("| `Ferrobot` | repo | `{pin}` | GPL | test |\n");
        assert_eq!(parse_pin(row.as_bytes()).unwrap(), pin);
        assert!(parse_pin(format!("{row}{row}").as_bytes()).is_err());
        assert!(parse_pin(row.replace(&pin, "main").as_bytes()).is_err());
        assert!(parse_pin(&vec![b' '; MAX_METADATA + 1]).is_err());
        assert!(parse_pin(b"| `Other` | repo | main |").is_err());
    }

    #[test]
    fn source_selection_refuses_dirty_or_wrong_pin_scratch_and_never_changes_reference() {
        let scratch = crate::repo_root()
            .join(".scratch")
            .join(format!("ferrobot-build-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&scratch).unwrap();
        let reference = scratch.join("reference");
        fs::create_dir(&reference).unwrap();
        git(&reference, &["init", "--quiet"]).unwrap();
        fs::write(reference.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(reference.join("Cargo.lock"), "version = 4\n").unwrap();
        git(&reference, &["add", "Cargo.toml", "Cargo.lock"]).unwrap();
        git(
            &reference,
            &[
                "-c",
                "user.name=Ferroan test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "test fixture",
            ],
        )
        .unwrap();
        let pin = git(&reference, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_owned();
        let copied = source(&scratch, Some(&reference), &pin).unwrap();
        assert_eq!(copied, scratch.join("ferrobot-src"));
        assert!(clean_at(&copied, &pin).unwrap());
        assert!(clean_at(&reference, &pin).unwrap());
        assert!(!clean_at(&reference, &"0".repeat(40)).unwrap());
        assert!(source(&scratch, None, &"0".repeat(40)).is_err());
        let fallback = scratch.join("ferrobot-src");
        fs::write(fallback.join("untracked"), "preserve copied source").unwrap();
        fs::write(reference.join("untracked"), "preserve me").unwrap();
        assert!(!clean_at(&reference, &pin).unwrap());
        assert!(source(&scratch, Some(&reference), &pin).is_err());
        assert_eq!(
            fs::read_to_string(reference.join("untracked")).unwrap(),
            "preserve me"
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn git_output_deadline_does_not_wait_for_a_descendant_to_close_stdout() {
        let (_held_open, receiver) = mpsc::sync_channel(1);
        assert!(receive_metadata(receiver, Duration::from_millis(10)).is_err());
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.send(Ok(b"bounded metadata".to_vec())).unwrap();
        assert_eq!(
            receive_metadata(receiver, Duration::ZERO).unwrap(),
            b"bounded metadata"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cached_source_cannot_redirect_a_build_into_a_reference_checkout() {
        let scratch = crate::repo_root()
            .join(".scratch")
            .join(format!("ferrobot-symlink-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&scratch).unwrap();
        let reference = scratch.join("reference");
        fs::create_dir(&reference).unwrap();
        fs::write(reference.join("preserve"), b"read only").unwrap();
        std::os::unix::fs::symlink(&reference, scratch.join("ferrobot-src")).unwrap();
        assert!(
            source(&scratch, None, &"0".repeat(40))
                .unwrap_err()
                .contains("symlink")
        );
        assert_eq!(fs::read(reference.join("preserve")).unwrap(), b"read only");
        fs::remove_dir_all(scratch).unwrap();
    }
}
