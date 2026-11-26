// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build `tools/oar-oracle` against a task-owned clone of the pin and compare
//! its control-file dump with the committed fixture. The shared reference
//! checkout is only read. The command does not start a region.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::asset_oracle::{
    checkout_is_clean, dotnet, fail_if_dirty, git, pinned_and_checked, pinned_revision,
    reference_root,
};
use crate::oracle_reference::prepare;

const LABEL: &str = "oar";
const ARTIFACTS: &str = ".scratch/oar-oracle/artifacts";
const PROJECT: &str = "tools/oar-oracle/OarOracle.csproj";
const DLL: &str = "bin/OarOracle/release/OarOracle.dll";
const FIXTURE: &str = "fixtures/oar-control";

pub fn generate() -> ExitCode {
    dispatch(true)
}

pub fn verify() -> ExitCode {
    dispatch(false)
}

fn dispatch(writing: bool) -> ExitCode {
    let root = crate::repo_root();
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return ExitCode::FAILURE;
        }
    };
    if writing {
        write_fixture(&root, &dll, &pinned)
    } else {
        verify_fixture(&root, &dll, &pinned)
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
        .and_then(|text| pinned_revision(&text))
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
    if !dotnet(
        LABEL,
        &root,
        &reference,
        &artifacts,
        &project,
        "build",
        false,
        &[],
    ) {
        return Err(fail_if_dirty(LABEL, &reference));
    }
    if !checkout_is_clean(LABEL, &reference) {
        return Err(ExitCode::FAILURE);
    }
    if !source_unchanged(&source, &source_head) {
        return Err(ExitCode::FAILURE);
    }
    let dll = artifacts.join(DLL);
    if !dll.is_file() {
        eprintln!("{LABEL}: missing {}", dll.display());
        return Err(ExitCode::FAILURE);
    }
    Ok(dll)
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

fn write_fixture(root: &Path, dll: &Path, pin: &str) -> ExitCode {
    let dir = root.join(FIXTURE);
    if let Err(error) = fs::create_dir_all(&dir) {
        eprintln!("{LABEL}: could not create {}: {error}", dir.display());
        return ExitCode::FAILURE;
    }
    if !oracle(dll, &dir, pin) {
        return ExitCode::FAILURE;
    }
    println!("{LABEL}: wrote {FIXTURE}");
    ExitCode::SUCCESS
}

fn verify_fixture(root: &Path, dll: &Path, pin: &str) -> ExitCode {
    let dir = root.join(".scratch/oar-oracle/verify");
    if dir.exists()
        && let Err(error) = fs::remove_dir_all(&dir)
    {
        eprintln!("{LABEL}: could not clear {}: {error}", dir.display());
        return ExitCode::FAILURE;
    }
    if let Err(error) = fs::create_dir_all(&dir) {
        eprintln!("{LABEL}: could not create {}: {error}", dir.display());
        return ExitCode::FAILURE;
    }
    if !oracle(dll, &dir, pin) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = same_output(&root.join(FIXTURE), &dir) {
        eprintln!("{LABEL}: {error}");
        return ExitCode::FAILURE;
    }
    let mut command = Command::new("cargo");
    command.current_dir(root).args([
        "test",
        "-p",
        "ferroan-scene",
        "--lib",
        "pin8_load_oar_control",
        "--",
        "--nocapture",
    ]);
    println!("$ cargo test -p ferroan-scene --lib pin8_load_oar_control");
    match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("{LABEL}: fixture tests exited with {status}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("{LABEL}: could not run cargo: {error}");
            ExitCode::FAILURE
        }
    }
}

fn oracle(dll: &Path, dir: &Path, pin: &str) -> bool {
    let mut command = crate::dotnet_ci::command("dotnet");
    command.arg(dll).arg("generate").arg(dir).arg(pin);
    println!(
        "$ dotnet {} generate {} {pin}",
        dll.display(),
        dir.display()
    );
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("{LABEL}: oracle exited with {status}");
            false
        }
        Err(error) => {
            eprintln!("{LABEL}: could not run dotnet: {error}");
            false
        }
    }
}

fn same_output(committed: &Path, fresh: &Path) -> Result<(), String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(fresh).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err("oracle wrote a file whose name is not UTF-8".into());
        };
        names.push(name.to_string());
    }
    names.sort();
    if names.is_empty() {
        return Err("oracle wrote no files".into());
    }
    for name in &names {
        let left = fs::read(committed.join(name))
            .map_err(|error| format!("could not read committed {name}: {error}"))?;
        let right = fs::read(fresh.join(name))
            .map_err(|error| format!("could not read fresh {name}: {error}"))?;
        if left != right {
            return Err(format!("{name} differs from the committed fixture"));
        }
    }
    for entry in fs::read_dir(committed).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name == "README.md" || names.iter().any(|fresh| fresh == &name) {
            continue;
        }
        return Err(format!("committed {name} was not regenerated"));
    }
    Ok(())
}
