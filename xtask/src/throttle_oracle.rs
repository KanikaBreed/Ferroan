// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run `tools/throttle-oracle` against the pinned Tranquillity checkout.
//! Build output goes under `.scratch/`, never into the checkout.

use std::fs;
use std::process::ExitCode;

pub fn generate() -> ExitCode {
    run(true)
}

pub fn verify() -> ExitCode {
    run(false)
}

fn run(writing: bool) -> ExitCode {
    let root = crate::repo_root();
    let reference =
        match crate::http_oracle::reference_root().and_then(crate::oracle_reference::prepare) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("throttle: {error}");
                return ExitCode::FAILURE;
            }
        };
    let revision = match crate::http_oracle::pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("throttle: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(".scratch/throttle-oracle/artifacts");
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!(
            "throttle: could not create {}: {error}",
            artifacts.display()
        );
        return ExitCode::FAILURE;
    }
    let project = root.join("tools/throttle-oracle/ThrottleOracle.csproj");
    if !crate::http_oracle::dotnet(&root, &reference, &artifacts, &project, "build", false, &[]) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("throttle: {error}");
        return ExitCode::FAILURE;
    }
    let action = if writing { "generate" } else { "verify" };
    let args = vec![
        action.to_string(),
        root.join("fixtures/throttle").display().to_string(),
        "--revision".to_string(),
        revision.clone(),
    ];
    if !crate::http_oracle::dotnet(&root, &reference, &artifacts, &project, "run", true, &args) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("throttle: {error}");
        return ExitCode::FAILURE;
    }
    println!("throttle: {action} passed against {revision}");
    ExitCode::SUCCESS
}
