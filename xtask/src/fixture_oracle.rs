// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run a pinned C# fixture oracle against the Tranquillity checkout.
//! The caller names the fixture path and the action.
//! Build output goes under `.scratch/`, never into the checkout.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

pub(crate) fn run(suite: &str, project_file: &str, fixture: &Path, action: &str) -> ExitCode {
    let root = crate::repo_root();
    let reference =
        match crate::http_oracle::reference_root().and_then(crate::oracle_reference::prepare) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("{suite}: {error}");
                return ExitCode::FAILURE;
            }
        };
    let revision = match crate::http_oracle::pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("{suite}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(format!(".scratch/{suite}-oracle/artifacts"));
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("{suite}: could not create {}: {error}", artifacts.display());
        return ExitCode::FAILURE;
    }
    let project = root
        .join(format!("tools/{suite}-oracle"))
        .join(project_file);
    if !crate::http_oracle::dotnet(&root, &reference, &artifacts, &project, "build", false, &[]) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("{suite}: {error}");
        return ExitCode::FAILURE;
    }
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
        eprintln!("{suite}: {error}");
        return ExitCode::FAILURE;
    }
    println!("{suite}: {action} passed against {revision}");
    ExitCode::SUCCESS
}
