// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run the Jolt oracles against the pinned checkout.
//! Build output goes under `.scratch/`, never into the checkout.
//!
//! `jolt generate` and `jolt verify` stay with the harness wrapper (P3.0).
//! `jolt file` calls one pinned method and does not start native code.

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
    run(
        writing,
        "fixtures/jolt-config",
        "generate",
        "verify",
        "jolt-config",
    )
}

pub(crate) fn file(writing: bool) -> ExitCode {
    run(
        writing,
        "fixtures/jolt-file",
        "file-generate",
        "file-verify",
        "jolt-file",
    )
}

fn run(
    writing: bool,
    fixture_rel: &str,
    write_action: &str,
    check_action: &str,
    label: &str,
) -> ExitCode {
    let root = crate::repo_root();
    let reference =
        match crate::http_oracle::reference_root().and_then(crate::oracle_reference::prepare) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("{label}: {error}");
                return ExitCode::FAILURE;
            }
        };
    let revision = match crate::http_oracle::pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("{label}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(".scratch/jolt-config-oracle/artifacts");
    let tmp = root.join(".scratch/tmp");
    if let Err(error) = fs::create_dir_all(&artifacts).and_then(|_| fs::create_dir_all(&tmp)) {
        eprintln!("{label}: could not create {}: {error}", artifacts.display());
        return ExitCode::FAILURE;
    }
    let project = root.join("tools/jolt-config-oracle/JoltConfigOracle.csproj");
    let fixture = root.join(fixture_rel);
    let action = if writing { write_action } else { check_action };
    let dotnet = Dotnet {
        root: &root,
        reference: &reference,
        artifacts: &artifacts,
        project: &project,
        label,
    };
    if !dotnet.run("build", false, &[]) {
        return fail_if_dirty(&root, &reference, label);
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("{label}: {error}");
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
    if !dotnet.run("run", true, &args) {
        return fail_if_dirty(&root, &reference, label);
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("{label}: {error}");
        return ExitCode::FAILURE;
    }
    println!("{label}: {action} passed against {revision}");
    ExitCode::SUCCESS
}

fn fail_if_dirty(root: &Path, reference: &Path, label: &str) -> ExitCode {
    if let Err(error) = crate::http_oracle::pinned_and_checked(root, reference) {
        eprintln!("{label}: {error}");
    }
    ExitCode::FAILURE
}

struct Dotnet<'a> {
    root: &'a Path,
    reference: &'a Path,
    artifacts: &'a Path,
    project: &'a Path,
    label: &'a str,
}

impl Dotnet<'_> {
    fn run(&self, action: &str, no_build: bool, program_args: &[String]) -> bool {
        let tmp = self.root.join(".scratch/tmp");
        let mut command = crate::dotnet_ci::command("dotnet");
        command
            .current_dir(self.root)
            .env("TMPDIR", &tmp)
            .env("TMP", &tmp)
            .env("TEMP", &tmp)
            .arg(action);
        if action == "run" {
            command.arg("--project").arg(self.project);
        } else {
            command.arg(self.project);
        }
        command
            .arg("-c")
            .arg("Release")
            .arg("--verbosity")
            .arg("minimal")
            .arg(format!("-p:FerroanRef={}", self.reference.display()))
            .arg("-p:UseArtifactsOutput=true")
            .arg(format!("-p:ArtifactsPath={}", self.artifacts.display()))
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
        let shown = self
            .project
            .strip_prefix(self.root)
            .unwrap_or(self.project)
            .display();
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
                eprintln!("{}: dotnet exited with {status}", self.label);
                false
            }
            Err(error) => {
                eprintln!(
                    "{}: could not run dotnet (the .NET SDK is required for Jolt fixtures): {error}",
                    self.label
                );
                false
            }
        }
    }
}
