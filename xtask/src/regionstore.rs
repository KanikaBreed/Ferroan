// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build `tools/regionstore-oracle` against a task-owned clone of the pin and
//! compare its SQLite object dump with the committed fixture. The shared
//! reference checkout is only read.

use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::asset_oracle::{
    checkout_is_clean, dotnet, fail_if_dirty, git, pinned_and_checked, pinned_revision,
    reference_root,
};
use crate::oracle_reference::prepare;

const LABEL: &str = "regionstore";
const ARTIFACTS: &str = ".scratch/regionstore-oracle/artifacts";
const PROJECT: &str = "tools/regionstore-oracle/RegionStoreOracle.csproj";
const DLL: &str = "bin/RegionStoreOracle/release/RegionStoreOracle.dll";
const FIXTURE: &str = "fixtures/regionstore/objects.db";
const DUMP: &str = "fixtures/regionstore/objects.json";
const REGION: &str = "11111111-1111-1111-1111-111111111111";

pub fn generate() -> ExitCode {
    dispatch(true)
}

pub fn verify() -> ExitCode {
    dispatch(false)
}

pub fn bulk(db: &str, region: &str, count: &str) -> ExitCode {
    let dll = match build_oracle() {
        Ok(path) => path,
        Err(code) => return code,
    };
    if oracle(&dll, &["bulk", db, region, count]) {
        println!("{LABEL}: wrote {count} prims for {region} to {db}");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
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
        return write_fixture(&root, &dll, &pinned);
    }
    verify_fixture(&root, &dll)
}

fn build_oracle() -> Result<std::path::PathBuf, ExitCode> {
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
    let db = root.join(FIXTURE);
    let dump = root.join(DUMP);
    if let Some(parent) = db.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        eprintln!("{LABEL}: could not create {}: {error}", parent.display());
        return ExitCode::FAILURE;
    }
    if db.exists()
        && let Err(error) = fs::remove_file(&db)
    {
        eprintln!("{LABEL}: could not replace {}: {error}", db.display());
        return ExitCode::FAILURE;
    }
    let db_arg = db.display().to_string();
    if !oracle(dll, &["seed", &db_arg]) {
        return ExitCode::FAILURE;
    }
    let json = match oracle_stdout(dll, &["load", &db_arg, REGION, pin]) {
        Some(text) => text,
        None => return ExitCode::FAILURE,
    };
    if let Err(error) = fs::write(&dump, json) {
        eprintln!("{LABEL}: could not write {}: {error}", dump.display());
        return ExitCode::FAILURE;
    }
    println!("{LABEL}: wrote {} and {}", FIXTURE, DUMP);
    ExitCode::SUCCESS
}

fn verify_fixture(root: &Path, dll: &Path) -> ExitCode {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .env("FERROAN_REGIONSTORE", "1")
        .env("FERROAN_REGIONSTORE_ORACLE", dll)
        .args([
            "test",
            "-p",
            "ferroan-scene",
            "--lib",
            "regionstore_",
            "--",
            "--nocapture",
        ]);
    println!("$ cargo test -p ferroan-scene --lib regionstore_");
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

fn oracle(dll: &Path, args: &[&str]) -> bool {
    oracle_stdout(dll, args).is_some()
}

fn oracle_stdout(dll: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("dotnet");
    command
        .arg(dll)
        .args(args)
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("MSBUILDDISABLENODEREUSE", "1");
    println!("$ dotnet {} {}", dll.display(), args.join(" "));
    match command.output() {
        Ok(output) if output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                eprint!("{stderr}");
            }
            Some(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Ok(output) => {
            eprintln!(
                "{LABEL}: oracle exited with {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(error) => {
            eprintln!("{LABEL}: could not run dotnet: {error}");
            None
        }
    }
}
