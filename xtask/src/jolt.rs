// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run the Jolt oracles against the pinned checkout.
//! Build output goes under `.scratch/`, never into the checkout.
//!
//! `jolt generate` and `jolt verify` stay with the matched-native harness (P3.0).
//! `jolt file` calls one pinned method and does not start native code.
//! `jolt record` runs the pinned harness in-process and does not start a region.

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
    run(Job {
        writing,
        fixture_rel: "fixtures/jolt-config",
        write_action: "generate",
        check_action: "verify",
        label: "jolt-config",
        project_rel: "tools/jolt-config-oracle/JoltConfigOracle.csproj",
        artifacts_rel: ".scratch/jolt-config-oracle/artifacts",
        clear_physics_rate: false,
    })
}

pub(crate) fn file(writing: bool) -> ExitCode {
    run(Job {
        writing,
        fixture_rel: "fixtures/jolt-file",
        write_action: "file-generate",
        check_action: "file-verify",
        label: "jolt-file",
        project_rel: "tools/jolt-config-oracle/JoltConfigOracle.csproj",
        artifacts_rel: ".scratch/jolt-config-oracle/artifacts",
        clear_physics_rate: false,
    })
}

pub(crate) fn record(writing: bool) -> ExitCode {
    run(Job {
        writing,
        fixture_rel: "fixtures/jolt-record",
        write_action: "generate",
        check_action: "verify",
        label: "jolt-record",
        project_rel: "tools/jolt-config-oracle/JoltRecordOracle.csproj",
        artifacts_rel: ".scratch/jolt-record-oracle/artifacts",
        clear_physics_rate: true,
    })
}

struct Job {
    writing: bool,
    fixture_rel: &'static str,
    write_action: &'static str,
    check_action: &'static str,
    label: &'static str,
    project_rel: &'static str,
    artifacts_rel: &'static str,
    clear_physics_rate: bool,
}

fn run(job: Job) -> ExitCode {
    let root = crate::repo_root();
    let reference =
        match crate::http_oracle::reference_root().and_then(crate::oracle_reference::prepare) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("{}: {error}", job.label);
                return ExitCode::FAILURE;
            }
        };
    let revision = match crate::http_oracle::pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("{}: {error}", job.label);
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(job.artifacts_rel);
    let tmp = root.join(".scratch/tmp");
    if let Err(error) = fs::create_dir_all(&artifacts).and_then(|_| fs::create_dir_all(&tmp)) {
        eprintln!(
            "{}: could not create {}: {error}",
            job.label,
            artifacts.display()
        );
        return ExitCode::FAILURE;
    }
    let project = root.join(job.project_rel);
    let fixture = root.join(job.fixture_rel);
    let action = if job.writing {
        job.write_action
    } else {
        job.check_action
    };
    let dotnet = Dotnet {
        root: &root,
        reference: &reference,
        artifacts: &artifacts,
        project: &project,
        label: job.label,
        clear_physics_rate: job.clear_physics_rate,
    };
    if !dotnet.run("build", false, &[]) {
        return fail_if_dirty(&root, &reference, job.label);
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("{}: {error}", job.label);
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
        return fail_if_dirty(&root, &reference, job.label);
    }
    if let Err(error) = crate::http_oracle::pinned_and_checked(&root, &reference) {
        eprintln!("{}: {error}", job.label);
        return ExitCode::FAILURE;
    }
    println!("{}: {action} passed against {revision}", job.label);
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
    clear_physics_rate: bool,
}

impl Dotnet<'_> {
    fn run(&self, action: &str, no_build: bool, program_args: &[String]) -> bool {
        let tmp = self.root.join(".scratch/tmp");
        let mut command = crate::dotnet_ci::command("dotnet");
        command
            .current_dir(self.root)
            .env("TMPDIR", &tmp)
            .env("TMP", &tmp)
            .env("TEMP", &tmp);
        if self.clear_physics_rate {
            command.env_remove("JOLT_HARNESS_PHYSICS_RATE");
        }
        command.arg(action);
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
