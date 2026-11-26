// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run the Jolt oracles against the pinned checkout.
//! Build output goes under `.scratch/`, never into the checkout.
//!
//! `jolt generate` and `jolt verify` stay with the harness wrapper (P3.0).

use std::fs;
use std::path::Path;
use std::process::ExitCode;

pub(crate) fn substeps(writing: bool) -> ExitCode {
    let root = crate::repo_root();
    let fixture = root.join("fixtures/jolt/substeps.json");
    let action = if writing {
        "substeps-generate"
    } else {
        "substeps-verify"
    };
    crate::fixture_oracle::run("jolt", "JoltOracle.csproj", &fixture, action)
}

pub(crate) fn config(writing: bool) -> ExitCode {
    let root = crate::repo_root();
    let reference =
        match crate::http_oracle::reference_root().and_then(crate::oracle_reference::prepare) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("jolt-config: {error}");
                return ExitCode::FAILURE;
            }
        };
    let revision = match crate::http_oracle::pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("jolt-config: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(".scratch/jolt-config-oracle/artifacts");
    let tmp = root.join(".scratch/tmp");
    if let Err(error) = fs::create_dir_all(&artifacts).and_then(|_| fs::create_dir_all(&tmp)) {
        eprintln!(
            "jolt-config: could not create {}: {error}",
            artifacts.display()
        );
        return ExitCode::FAILURE;
    }
    let project = root.join("tools/jolt-config-oracle/JoltConfigOracle.csproj");
    let fixture = root.join("fixtures/jolt-config");
    let action = if writing { "generate" } else { "verify" };
    if !dotnet(&root, &reference, &artifacts, &project, "build", false, &[]) {
        return fail_if_dirty(&root, &reference);
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("jolt-config: {error}");
        return ExitCode::FAILURE;
    }
    let args = vec![
        action.to_string(),
        fixture.display().to_string(),
        "--revision".to_string(),
        revision.clone(),
        "--reference".to_string(),
        reference.display().to_string(),
    ];
    if !dotnet(&root, &reference, &artifacts, &project, "run", true, &args) {
        return fail_if_dirty(&root, &reference);
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("jolt-config: {error}");
        return ExitCode::FAILURE;
    }
    println!("jolt-config: {action} passed against {revision}");
    ExitCode::SUCCESS
}

fn fail_if_dirty(root: &Path, reference: &Path) -> ExitCode {
    if let Err(error) = crate::http_oracle::pinned_and_checked(root, reference) {
        eprintln!("jolt-config: {error}");
    }
    ExitCode::FAILURE
}

fn dotnet(
    root: &Path,
    reference: &Path,
    artifacts: &Path,
    project: &Path,
    action: &str,
    no_build: bool,
    program_args: &[String],
) -> bool {
    let tmp = root.join(".scratch/tmp");
    let mut command = crate::dotnet_ci::command("dotnet");
    command
        .current_dir(root)
        .env("TMPDIR", &tmp)
        .env("TMP", &tmp)
        .env("TEMP", &tmp)
        .arg(action);
    if action == "run" {
        command.arg("--project").arg(project);
    } else {
        command.arg(project);
    }
    command
        .arg("-c")
        .arg("Release")
        .arg("--verbosity")
        .arg("minimal")
        .arg(format!("-p:FerroanRef={}", reference.display()))
        .arg("-p:UseArtifactsOutput=true")
        .arg(format!("-p:ArtifactsPath={}", artifacts.display()))
        .arg("-p:GeneratePackageOnBuild=false")
        .arg("-p:TreatWarningsAsErrors=false")
        .arg("-p:UseSharedCompilation=false")
        .arg("-p:ContinuousIntegrationBuild=true")
        .arg(crate::dotnet_ci::CLOUD_BUILD_VERSION_VARS_OFF);
    if no_build {
        command.arg("--no-build");
    }
    if !program_args.is_empty() {
        command.arg("--");
        command.args(program_args);
    }
    let shown = project.strip_prefix(root).unwrap_or(project).display();
    if program_args.is_empty() {
        println!("$ dotnet {action} -c Release {shown}");
    } else {
        println!(
            "$ dotnet {action} -c Release {shown} -- {}",
            program_args.join(" ")
        );
    }
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("jolt-config: dotnet exited with {status}");
            false
        }
        Err(error) => {
            eprintln!(
                "jolt-config: could not run dotnet (the .NET SDK is required for Jolt config fixtures): {error}"
            );
            false
        }
    }
}
