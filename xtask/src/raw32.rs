// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rebuild `tools/raw32-oracle` against a task-owned clone of the pin and
//! diff a fresh run with `fixtures/oar-raw32`. The shared reference checkout
//! is only read. The run starts the in-process scene the fixture README uses.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::asset_oracle::{
    checkout_is_clean, fail_if_dirty, git, pinned_and_checked, reference_root,
};
use crate::oracle_reference::prepare;

const LABEL: &str = "raw32";
const ARTIFACTS: &str = ".scratch/raw32-oracle/artifacts";
const PROJECT: &str = "tools/raw32-oracle/Raw32Oracle.csproj";
const FIXTURE: &str = "fixtures/oar-raw32";
const FRESH: &str = ".scratch/raw32-oracle/fresh";

const FILES: &[&str] = &[
    "bounding.r32",
    "clip.r32",
    "combination.r32",
    "displacement-3e9.r32",
    "displacement-negx.r32",
    "displacement.r32",
    "f32.oar",
    "f32.r32",
    "fractional.r32",
    "initial-512.r32",
    "initial.r32",
    "loaded.r32",
    "mask-member.r32",
    "mask-z.r32",
    "mask.oar",
    "mask.r32",
    "member.r32",
    "origin-3e9.r32",
    "region.oar",
    "rotation-neg45.r32",
    "rotation-neg90.r32",
    "rotation180.r32",
    "rotation270.r32",
    "rotation45-displacement.r32",
    "rotation45.r32",
    "rotation90.r32",
    "section-in.r32",
    "section-out.r32",
    "short.r32",
    "short.txt",
    "taint-in.r32",
    "taint.txt",
    "tiny-in.r32",
    "tiny-out.r32",
    "trim-in.r32",
    "trim-out.r32",
    "varregion.r32",
    "wide.oar",
    "wide.r32",
    "word-in.r32",
    "word-out.r32",
];

pub fn verify() -> ExitCode {
    let root = crate::repo_root();
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    let fresh = root.join(FRESH);
    if fresh.exists()
        && let Err(error) = fs::remove_dir_all(&fresh)
    {
        eprintln!("{LABEL}: could not clear {}: {error}", fresh.display());
        return ExitCode::FAILURE;
    }
    if let Err(error) = fs::create_dir_all(&fresh) {
        eprintln!("{LABEL}: could not create {}: {error}", fresh.display());
        return ExitCode::FAILURE;
    }
    let Some(dll_dir) = dll.parent() else {
        eprintln!("{LABEL}: {} has no directory", dll.display());
        return ExitCode::FAILURE;
    };
    let mut command = crate::dotnet_ci::command("dotnet");
    command.current_dir(dll_dir).arg(&dll).arg(&fresh);
    println!("$ dotnet {} {}", dll.display(), fresh.display());
    let output = match command.output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("{LABEL}: could not run dotnet: {error}");
            return ExitCode::FAILURE;
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }
    if !output.status.success() || !stdout.contains("load oar matches LoadStream") {
        eprintln!(
            "{LABEL}: oracle exited with {}\n{}",
            output.status,
            stdout.trim()
        );
        return ExitCode::FAILURE;
    }
    println!("load oar matches LoadStream");
    match diff(&root.join(FIXTURE), &fresh) {
        Ok(()) => {
            println!("{LABEL}: {FIXTURE} matches the pin");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn build_oracle() -> Result<PathBuf, ExitCode> {
    let root = crate::repo_root();
    let source = match reference_root() {
        Ok(path) => path.join("OpenSim-Tranquillity"),
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let source_head = match git(&source, &["rev-parse", "HEAD"]) {
        Ok(head) => head,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let reference = match prepare(source.parent().unwrap_or(&source).to_path_buf()) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| crate::asset_oracle::pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let clone = reference.join("OpenSim-Tranquillity");
    if let Err(error) = detach_at_pin(&clone, &source, &pinned) {
        eprintln!("{LABEL}: {error}");
        return Err(ExitCode::FAILURE);
    }
    if let Err(error) = pinned_and_checked(&root, &reference) {
        eprintln!("{LABEL}: {error}");
        return Err(ExitCode::FAILURE);
    }
    let artifacts = root.join(ARTIFACTS);
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("{LABEL}: could not create {}: {error}", artifacts.display());
        return Err(ExitCode::FAILURE);
    }
    let project = root.join(PROJECT);
    if !dotnet_build(&root, &reference, &artifacts, &project) {
        return Err(fail_if_dirty(LABEL, &reference));
    }
    if !checkout_is_clean(LABEL, &reference) {
        return Err(ExitCode::FAILURE);
    }
    if !source_unchanged(&source, &source_head) {
        return Err(ExitCode::FAILURE);
    }
    match locate_dll(&artifacts) {
        Ok(dll) => Ok(dll),
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            Err(ExitCode::FAILURE)
        }
    }
}

/// Same flags as the other pin oracles. `-m:1` keeps two nodes from writing one
/// `deps.json` under the artifacts layout. That race fails this graph.
fn dotnet_build(root: &Path, reference: &Path, artifacts: &Path, project: &Path) -> bool {
    let mut command = crate::dotnet_ci::command("dotnet");
    command
        .current_dir(root)
        .arg("build")
        .arg(project)
        .arg("-c")
        .arg("Release")
        .arg("--verbosity")
        .arg("minimal")
        .arg(format!("-p:FerroanRef={}", reference.display()))
        .arg("-p:UseArtifactsOutput=true")
        .arg(format!("-p:ArtifactsPath={}", artifacts.display()))
        .arg("-p:GeneratePackageOnBuild=false")
        .arg("-p:ContinuousIntegrationBuild=true")
        .arg("-p:UseSharedCompilation=false")
        .arg("-m:1")
        .arg(crate::dotnet_ci::CLOUD_BUILD_VERSION_VARS_OFF);
    let shown = project.strip_prefix(root).unwrap_or(project).display();
    println!("$ dotnet build -c Release -m:1 {shown}");
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("{LABEL}: dotnet exited with {status}");
            false
        }
        Err(error) => {
            eprintln!(
                "{LABEL}: could not run dotnet (the .NET SDK is required for this oracle): {error}"
            );
            false
        }
    }
}

fn locate_dll(artifacts: &Path) -> Result<PathBuf, String> {
    let mut found = Vec::new();
    find_app_dll(artifacts, &mut found)?;
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!(
            "missing Raw32Oracle.dll under {}",
            artifacts.display()
        )),
        _ => Err(format!(
            "several Raw32Oracle.dll files under {}",
            artifacts.display()
        )),
    }
}

fn find_app_dll(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            find_app_dll(&path, found)?;
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("Raw32Oracle.dll")
            && path.with_file_name("Raw32Oracle.deps.json").is_file()
        {
            found.push(path);
        }
    }
    Ok(())
}

fn detach_at_pin(clone: &Path, source: &Path, pin: &str) -> Result<(), String> {
    if git(clone, &["cat-file", "-e", &format!("{pin}^{{commit}}")]).is_err() {
        let remote = source.to_str().ok_or("reference path is not UTF-8")?;
        git(clone, &["fetch", remote, pin])?;
    }
    git(clone, &["checkout", "--detach", pin])?;
    Ok(())
}

fn source_unchanged(source: &Path, head: &str) -> bool {
    match git(source, &["rev-parse", "HEAD"]) {
        Ok(now) if now == head => {}
        Ok(now) => {
            eprintln!("{LABEL}: the reference checkout moved from {head} to {now}");
            return false;
        }
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return false;
        }
    }
    match git(source, &["status", "--porcelain"]) {
        Ok(status) if status.is_empty() => true,
        Ok(status) => {
            eprintln!("{LABEL}: the reference checkout is no longer clean:\n{status}");
            false
        }
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            false
        }
    }
}

fn diff(committed: &Path, fresh: &Path) -> Result<(), String> {
    let mut problems = Vec::new();
    for name in FILES {
        let left = match fs::read(committed.join(name)) {
            Ok(bytes) => bytes,
            Err(error) => {
                problems.push(format!("could not read {FIXTURE}/{name}: {error}"));
                continue;
            }
        };
        let right = match fs::read(fresh.join(name)) {
            Ok(bytes) => bytes,
            Err(error) => {
                problems.push(format!("oracle did not write {name}: {error}"));
                continue;
            }
        };
        if left != right {
            problems.push(format!(
                "{FIXTURE}/{name} differs from the pin (committed {}, fresh {})",
                left.len(),
                right.len()
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}
