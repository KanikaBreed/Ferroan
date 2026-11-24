// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One grid-mode command that requires the same report lines from a C# home
//! region and from the Ferroan home region.
//!
//! The lines are login, chat, the in-region teleport, the teleport to the
//! neighbour, and `live: run passed`. The queue names no TB or D id for these
//! lines, so any difference fails the command. An id is listed only when a
//! queue row already allows that difference.

// The harness runs on Linux. macOS and Windows still compile these helpers.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::process::ExitCode;

/// Report lines, in order. A trailing space means the line must start with
/// that prefix; the landing text after it still has to match.
const SHAPE: &[&str] = &[
    "login: pass",
    "chat: pass",
    "teleport: pass ",
    "teleport neighbour: pass ",
    "live: run passed",
];

/// TB and D ids the queue already allows for this compare. None.
const ALLOWED: &[&str] = &[];

pub(crate) fn compare() -> ExitCode {
    match compare_inner() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("live: {error}");
            ExitCode::from(1)
        }
    }
}

fn compare_inner() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        compare_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("cargo xtask live compare region runs on Linux".to_string())
    }
}

/// The report lines in `text`, in the order they appear.
pub(crate) fn report_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|line| is_report_line(line)).collect()
}

fn is_report_line(line: &str) -> bool {
    SHAPE.iter().any(|pattern| matches_shape(line, pattern))
}

fn matches_shape(line: &str, pattern: &str) -> bool {
    if pattern.ends_with(' ') {
        line.starts_with(pattern)
    } else {
        line == pattern
    }
}

/// Both sides must print the same lines, in order. `allowed` is empty unless
/// the queue already names those ids for this compare.
pub(crate) fn compare_reports(
    csharp: &str,
    ferroan: &str,
    allowed: &[&str],
) -> Result<String, String> {
    let left = report_lines(csharp);
    let right = report_lines(ferroan);
    if !shape_ok(&left) || !shape_ok(&right) || left != right {
        return Err(format!(
            "region report differs\ncsharp:\n{}\nferroan:\n{}",
            left.join("\n"),
            right.join("\n")
        ));
    }
    let mut out = String::from("region: match");
    for line in &left {
        out.push('\n');
        out.push_str(line);
    }
    if !allowed.is_empty() {
        out.push_str("\nknown differences: ");
        out.push_str(&allowed.join(", "));
    }
    Ok(out)
}

fn shape_ok(lines: &[&str]) -> bool {
    if lines.len() != SHAPE.len() {
        return false;
    }
    lines
        .iter()
        .zip(SHAPE.iter())
        .all(|(line, pattern)| matches_shape(line, pattern))
}

#[cfg(target_os = "linux")]
fn compare_linux() -> Result<String, String> {
    use std::time::Duration;

    if crate::live::standalone_is_up()?.is_some() || crate::grid_state::survivor()?.is_some() {
        return Err("a live harness is already running; stop it before compare region".to_string());
    }
    crate::live::ensure_region_built()?;
    // One port block apart, so the two grids never share a listener or a
    // directory. They run one after the other.
    const CSHARP_BASE: u16 = 22016;
    const FERROAN_BASE: u16 = 22048;
    const SIDE_LIMIT: Duration = Duration::from_secs(15 * 60);
    let csharp = run_side(CSHARP_BASE, false, SIDE_LIMIT)?;
    let ferroan = run_side(FERROAN_BASE, true, SIDE_LIMIT)?;
    compare_reports(&csharp, &ferroan, ALLOWED)
}

#[cfg(target_os = "linux")]
fn run_side(base: u16, ferroan: bool, limit: std::time::Duration) -> Result<String, String> {
    use std::fs::{self, File};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let name = if ferroan { "ferroan" } else { "csharp" };
    println!("region: {name}");
    let directory = crate::live::live_root().join("region-report");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let stdout_path = directory.join(format!("{name}.log"));
    let stderr_path = directory.join(format!("{name}.err.log"));
    let stdout = File::create(&stdout_path)
        .map_err(|error| format!("could not create {}: {error}", stdout_path.display()))?;
    let stderr = File::create(&stderr_path)
        .map_err(|error| format!("could not create {}: {error}", stderr_path.display()))?;
    let exe = std::env::current_exe().map_err(|error| format!("could not find xtask: {error}"))?;
    let mut command = Command::new(&exe);
    command
        .arg("live")
        .arg("--port-base")
        .arg(base.to_string())
        .arg("run")
        .arg("report")
        .arg("grid");
    if ferroan {
        command.args(["--region", "ferroan"]);
    }
    let mut child = command
        .current_dir(crate::repo_root())
        .env("MSBUILDDISABLENODEREUSE", "1")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|error| format!("could not start the {name} region report: {error}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < limit => std::thread::sleep(Duration::from_millis(200)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stop_side(base);
                return Err(format!(
                    "region {name} exceeded its time limit; private diagnostics: {}",
                    stderr_path.display()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stop_side(base);
                return Err(format!(
                    "could not observe the {name} region report: {error}"
                ));
            }
        }
    };
    if !status.success() {
        let _ = stop_side(base);
        let detail = safe_live_lines(&read_log(&stderr_path)?);
        return Err(if detail.is_empty() {
            format!(
                "region {name} failed; private diagnostics: {}",
                stderr_path.display()
            )
        } else {
            format!("region {name} failed\n{detail}")
        });
    }
    let text = read_log(&stdout_path)?;
    if !shape_ok(&report_lines(&text)) {
        return Err(format!(
            "region {name} report is missing a line\n{}",
            report_lines(&text).join("\n")
        ));
    }
    Ok(text)
}

#[cfg(target_os = "linux")]
fn stop_side(base: u16) -> Result<(), String> {
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe().map_err(|error| format!("could not find xtask: {error}"))?;
    let status = Command::new(exe)
        .args(["live", "--port-base", &base.to_string(), "down"])
        .current_dir(crate::repo_root())
        .env("MSBUILDDISABLENODEREUSE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not stop port base {base}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("could not stop port base {base}"))
    }
}

#[cfg(target_os = "linux")]
fn read_log(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if bytes.len() > 1024 * 1024 {
        return Err(format!("{} exceeds 1 MiB", path.display()));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not utf-8", path.display()))
}

#[cfg(target_os = "linux")]
fn safe_live_lines(text: &str) -> String {
    text.lines()
        .filter(|line| line.starts_with("live:"))
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.contains("password")
                && !lower.contains("secret")
                && !lower.contains("/caps/")
                && line.len() <= 400
        })
        .take(12)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATCHED: &str = "\
live: grid region Ferroan Test on 127.0.0.2:1
login: pass
chat: pass
teleport: pass (40.0,200.0,30.0)
teleport neighbour: pass (1001,1000 and back)
live: run passed
";

    #[test]
    fn matching_report_lines_pass_and_list_no_difference() {
        let text = compare_reports(MATCHED, MATCHED, ALLOWED).unwrap();
        assert!(text.starts_with("region: match\n"));
        assert!(text.contains("login: pass\nchat: pass\n"));
        assert!(text.contains("teleport: pass (40.0,200.0,30.0)\n"));
        assert!(text.contains("teleport neighbour: pass (1001,1000 and back)\n"));
        assert!(text.ends_with("live: run passed"));
        assert!(!text.contains("known differences"));
        assert!(ALLOWED.is_empty());
    }

    #[test]
    fn a_neighbour_line_is_not_the_in_region_teleport() {
        let lines = report_lines("teleport neighbour: pass (1001,1000 and back)\n");
        assert_eq!(lines, vec!["teleport neighbour: pass (1001,1000 and back)"]);
    }

    #[test]
    fn a_different_landing_fails_without_inventing_an_id() {
        let other = MATCHED.replace("(40.0,200.0,30.0)", "(40.0,200.0,26.0)");
        let error = compare_reports(MATCHED, &other, ALLOWED).unwrap_err();
        assert!(error.contains("region report differs"));
        assert!(error.contains("teleport: pass (40.0,200.0,30.0)"));
        assert!(error.contains("teleport: pass (40.0,200.0,26.0)"));
        assert!(!error.contains("known differences"));
    }

    #[test]
    fn a_missing_line_fails() {
        let short = "login: pass\nchat: pass\nlive: run passed\n";
        let error = compare_reports(MATCHED, short, ALLOWED).unwrap_err();
        assert!(error.contains("region report differs"));
        assert!(!error.contains("known differences"));
    }

    #[test]
    fn an_allowed_id_is_listed_only_when_the_lines_match() {
        let text = compare_reports(MATCHED, MATCHED, &["TB0"]).unwrap();
        assert!(text.contains("\nknown differences: TB0"));
        let error = compare_reports(MATCHED, "login: pass\n", &["TB0"]).unwrap_err();
        assert!(!error.contains("known differences"));
    }
}
