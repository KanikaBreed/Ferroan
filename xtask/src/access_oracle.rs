// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Observe full-region admission through the pinned scene and permissions module.

use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

pub fn run(writing: bool) -> ExitCode {
    match run_inner(writing) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("access oracle: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(writing: bool) -> Result<(), String> {
    let root = crate::repo_root();
    scrub_scratch_clone(&root);
    let reference =
        crate::http_oracle::reference_root().and_then(crate::oracle_reference::prepare)?;
    let revision = crate::http_oracle::pinned_and_checked(&root, &reference)?;
    let artifacts = root.join(".scratch/access-oracle/artifacts");
    fs::create_dir_all(&artifacts).map_err(|error| error.to_string())?;
    let project = root.join("tools/access-oracle/AccessOracle.csproj");
    if !dotnet(&root, &reference, &artifacts, &project, "build", false, &[]) {
        return Err(after_failure(&reference, "C# build failed"));
    }
    restore_clone(&reference)?;
    let action = if writing { "generate" } else { "verify" };
    let args = vec![
        action.to_owned(),
        root.join("fixtures/access").display().to_string(),
        "--revision".into(),
        revision,
    ];
    if !dotnet(&root, &reference, &artifacts, &project, "run", true, &args) {
        return Err(after_failure(&reference, "C# observations failed"));
    }
    restore_clone(&reference)?;
    Ok(())
}

fn after_failure(reference: &Path, message: &str) -> String {
    match restore_clone(reference) {
        Ok(()) => message.to_owned(),
        Err(error) => format!("{message}; {error}"),
    }
}

/// Drop build dirt in the task-owned clone. The reference checkout is not touched.
fn scrub_scratch_clone(root: &Path) {
    let checkout = root.join(".scratch/oracle-sources/OpenSim-Tranquillity");
    if !checkout.join(".git").is_dir() {
        return;
    }
    let _ = git(&checkout, &["checkout", "--", "."]);
    let _ = git(&checkout, &["clean", "-fd"]);
}

fn restore_clone(reference: &Path) -> Result<(), String> {
    let checkout = reference.join("OpenSim-Tranquillity");
    if !checkout.join(".git").is_dir() {
        return Ok(());
    }
    let status = git(&checkout, &["status", "--porcelain"])?;
    if status.is_empty() {
        return Ok(());
    }
    eprintln!("access oracle: restoring scratch clone edits:\n{status}");
    git(&checkout, &["checkout", "--", "."])?;
    git(&checkout, &["clean", "-fd"])?;
    let status = git(&checkout, &["status", "--porcelain"])?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(format!("oracle clone stayed dirty:\n{status}"))
    }
}

fn git(directory: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `NBGV_SetCloudBuildVersionVars=false` skips the target that appends to
/// `GITHUB_ENV`. Removing the variable is not enough while `GITHUB_ACTIONS` is set.
fn build_properties() -> &'static [&'static str] {
    &[
        "-p:UseArtifactsOutput=true",
        "-p:GeneratePackageOnBuild=false",
        "-p:ContinuousIntegrationBuild=true",
        "-p:NBGV_SetCloudBuildVersionVars=false",
    ]
}

fn configure_dotnet(command: &mut Command) {
    command
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("NUGET_XMLDOC_MODE", "skip")
        .env_remove("GITHUB_ENV");
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
    let mut command = Command::new("dotnet");
    command.current_dir(root).arg(action);
    configure_dotnet(&mut command);
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
        .args(build_properties())
        .arg(format!("-p:ArtifactsPath={}", artifacts.display()));
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
            eprintln!("access oracle: dotnet exited with {status}");
            false
        }
        Err(error) => {
            eprintln!(
                "access oracle: could not run dotnet (the .NET SDK is required for access fixtures): {error}"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_build_version_vars_are_off_and_github_env_is_removed() {
        assert!(build_properties().contains(&"-p:NBGV_SetCloudBuildVersionVars=false"));
        let mut command = Command::new("dotnet");
        command.env("GITHUB_ENV", "/runner/set_env");
        configure_dotnet(&mut command);
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "GITHUB_ENV" && value.is_none())
        );
    }
}
