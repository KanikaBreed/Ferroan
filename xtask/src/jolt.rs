// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run the substep accumulator oracle against the pinned checkout.
//! Build output goes under `.scratch/`, never into the checkout.
//!
//! `jolt generate` and `jolt verify` stay with the harness wrapper (P3.0).

use std::fs;
use std::process::ExitCode;

pub(crate) fn substeps(writing: bool) -> ExitCode {
    let root = crate::repo_root();
    let reference =
        match crate::http_oracle::reference_root().and_then(crate::oracle_reference::prepare) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("jolt: {error}");
                return ExitCode::FAILURE;
            }
        };
    let revision = match crate::http_oracle::pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("jolt: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(".scratch/jolt-oracle/artifacts");
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("jolt: could not create {}: {error}", artifacts.display());
        return ExitCode::FAILURE;
    }
    let project = root.join("tools/jolt-oracle/JoltOracle.csproj");
    if !crate::http_oracle::dotnet(&root, &reference, &artifacts, &project, "build", false, &[]) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("jolt: {error}");
        return ExitCode::FAILURE;
    }
    let action = if writing { "generate" } else { "verify" };
    let fixture = root.join("fixtures/jolt/substeps.json");
    let args = vec![
        action.to_string(),
        fixture.display().to_string(),
        "--revision".to_string(),
        revision.clone(),
    ];
    if !crate::http_oracle::dotnet(&root, &reference, &artifacts, &project, "run", true, &args) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("jolt: {error}");
        return ExitCode::FAILURE;
    }
    let label = if writing {
        "substeps"
    } else {
        "substeps-verify"
    };
    println!("jolt {label}: passed against {revision}");
    ExitCode::SUCCESS
}
