// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The one place the harness sends signals to the processes it starts.
//!
//! Every signal goes through the `kill` binary as `kill -s NAME -- -PGID` (or
//! `-- PID`). The `--` matters: procps-ng 4.0.4's `kill` (Ubuntu 24.04) reads
//! `kill -TERM -147893` as a signal to `-1`, the first digit, and the kernel
//! then signals every process the user owns. util-linux `kill` parses both
//! forms. A process group or PID of 0 or 1 is refused before anything runs, so
//! a bad value can never become `kill(0, ..)` or `kill(-1, ..)`.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::process::{Command, ExitStatus, Stdio};

/// The signals the harness sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signal {
    Term,
    Kill,
}

impl Signal {
    fn name(self) -> &'static str {
        match self {
            Self::Term => "TERM",
            Self::Kill => "KILL",
        }
    }
}

/// Sends `signal` to every process in group `pgid`.
pub(crate) fn signal_group(signal: Signal, pgid: u32) -> Result<ExitStatus, String> {
    run(group_args(signal, pgid)?, false)
}

/// Sends `signal` to the single process `pid`, with the `kill` binary's own
/// output discarded.
pub(crate) fn signal_pid(signal: Signal, pid: u32) -> Result<ExitStatus, String> {
    run(pid_args(signal, pid)?, true)
}

fn group_args(signal: Signal, pgid: u32) -> Result<[String; 4], String> {
    let pgid = checked(pgid, "process group")?;
    Ok([
        "-s".into(),
        signal.name().into(),
        "--".into(),
        format!("-{pgid}"),
    ])
}

fn pid_args(signal: Signal, pid: u32) -> Result<[String; 4], String> {
    let pid = checked(pid, "process")?;
    Ok([
        "-s".into(),
        signal.name().into(),
        "--".into(),
        pid.to_string(),
    ])
}

/// Accepts only ids the kernel treats as one ordinary process or group:
/// above 1 and within `pid_t`.
fn checked(id: u32, what: &str) -> Result<u32, String> {
    if id <= 1 || i32::try_from(id).is_err() {
        return Err(format!("refusing to signal {what} {id}"));
    }
    Ok(id)
}

fn run(args: [String; 4], quiet: bool) -> Result<ExitStatus, String> {
    let mut command = Command::new("kill");
    command.args(args);
    if quiet {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }
    command
        .status()
        .map_err(|error| format!("could not run kill: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn group_argv_puts_the_group_after_double_dash() {
        assert_eq!(
            group_args(Signal::Term, 147_893).unwrap(),
            ["-s", "TERM", "--", "-147893"]
        );
        assert_eq!(
            group_args(Signal::Kill, 147_893).unwrap(),
            ["-s", "KILL", "--", "-147893"]
        );
        assert_eq!(
            pid_args(Signal::Term, 4242).unwrap(),
            ["-s", "TERM", "--", "4242"]
        );
    }

    #[test]
    fn zero_one_and_out_of_range_ids_are_refused() {
        for id in [0, 1, u32::MAX, 1 << 31] {
            for signal in [Signal::Term, Signal::Kill] {
                assert!(group_args(signal, id).is_err());
                assert!(pid_args(signal, id).is_err());
                // These return before a Command is built, so nothing runs.
                assert!(signal_group(signal, id).is_err());
                assert!(signal_pid(signal, id).is_err());
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn signals_only_the_named_group() {
        use std::os::unix::process::CommandExt;
        use std::thread;
        use std::time::{Duration, Instant};

        let spawn = || {
            Command::new("sleep")
                .arg("60")
                .process_group(0)
                .spawn()
                .unwrap()
        };
        let mut target = spawn();
        let mut bystander = spawn();
        let pgid = target.id();
        // Even with a broken helper this test must never ask for group 0 or 1.
        assert!(pgid > 1);
        assert_ne!(pgid, bystander.id());

        let status = signal_group(Signal::Term, pgid).unwrap();
        assert!(status.success());
        let deadline = Instant::now() + Duration::from_secs(10);
        let exited = loop {
            if let Some(status) = target.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= deadline {
                break None;
            }
            thread::sleep(Duration::from_millis(20));
        };
        let bystander_alive = bystander.try_wait().unwrap().is_none();

        let _ = target.kill();
        let _ = target.wait();
        let _ = bystander.kill();
        let _ = bystander.wait();

        let exited = exited.expect("signalled group did not exit");
        assert!(!exited.success());
        assert!(bystander_alive, "a process outside the group was signalled");
    }

    #[test]
    fn no_other_module_runs_kill_directly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut pending = vec![root];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && path.file_name().is_some_and(|name| name != "signal.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let compact: String = text.split_whitespace().collect();
                    if ["\"kill\"", "\"/bin/kill\"", "\"/usr/bin/kill\""]
                        .iter()
                        .any(|binary| compact.contains(&format!("Command::new({binary})")))
                    {
                        offenders.push(path.display().to_string());
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "run kill through signal::signal_group or signal_pid: {offenders:?}"
        );
    }
}
