// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Observe the pinned C# login/close and trust code through its real public APIs.

use std::{fs, process::ExitCode};

pub fn run(writing: bool) -> ExitCode {
    match run_inner(writing) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pin3 oracle: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(writing: bool) -> Result<(), String> {
    let root = crate::repo_root();
    let reference =
        crate::http_oracle::reference_root().and_then(crate::oracle_reference::prepare)?;
    let revision = crate::http_oracle::pinned_and_checked(&root, &reference)?;
    let artifacts = root.join(".scratch/pin3-oracle/artifacts");
    fs::create_dir_all(&artifacts).map_err(|error| error.to_string())?;
    let project = root.join("tools/pin3-oracle/Pin3Oracle.csproj");
    if !crate::http_oracle::dotnet(&root, &reference, &artifacts, &project, "build", false, &[]) {
        return Err("C# build failed".into());
    }
    let action = if writing { "generate" } else { "verify" };
    let args = vec![
        action.to_owned(),
        root.join("fixtures/pin3").display().to_string(),
        "--revision".into(),
        revision,
    ];
    if !crate::http_oracle::dotnet(&root, &reference, &artifacts, &project, "run", true, &args) {
        return Err("C# observations failed".into());
    }
    Ok(())
}
