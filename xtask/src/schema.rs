// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run `tools/schema-oracle` against the pinned Tranquillity checkout.
//! Build output goes under `.scratch/`, never into the checkout.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

pub fn generate(fixture_dir: Option<PathBuf>) -> ExitCode {
    run(true, fixture_dir)
}

pub fn verify(fixture_dir: Option<PathBuf>) -> ExitCode {
    run(false, fixture_dir)
}

fn run(writing: bool, fixture_dir: Option<PathBuf>) -> ExitCode {
    let root = crate::repo_root();
    let reference = match reference_root().and_then(crate::oracle_reference::prepare) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("schema: {error}");
            return ExitCode::FAILURE;
        }
    };
    let revision = match pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("schema: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(".scratch/schema-oracle/artifacts");
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("schema: could not create {}: {error}", artifacts.display());
        return ExitCode::FAILURE;
    }
    let project = root.join("tools/schema-oracle/SchemaOracle.csproj");
    let fixtures = fixture_dir.map_or_else(
        || root.join("fixtures/db/sqlite"),
        |path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        },
    );
    if !dotnet(&root, &reference, &artifacts, &project, "build", false, &[]) {
        return fail_if_dirty(&reference);
    }
    if !checkout_is_clean(&reference) {
        return ExitCode::FAILURE;
    }
    let action = if writing { "generate" } else { "verify" };
    let program = vec![
        action.to_string(),
        fixtures.display().to_string(),
        "--revision".to_string(),
        revision.clone(),
    ];
    if !dotnet(
        &root, &reference, &artifacts, &project, "run", true, &program,
    ) {
        return ExitCode::FAILURE;
    }
    if !checkout_is_clean(&reference) {
        return ExitCode::FAILURE;
    }
    println!("schema: {action} passed against {revision}");
    ExitCode::SUCCESS
}

fn reference_root() -> Result<PathBuf, String> {
    match env::var("FERROAN_REF") {
        Ok(value) if !value.is_empty() => Ok(expand_user(&value)),
        Ok(_) | Err(_) => home_dir().map(|home| home.join("Git/reference")),
    }
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            "set FERROAN_REF, or HOME / USERPROFILE, to find ~/Git/reference".to_string()
        })
}

fn expand_user(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

fn pinned_and_checked(root: &Path, reference: &Path) -> Result<String, String> {
    let text = fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| format!("could not read docs/REFERENCES.md: {error}"))?;
    let pinned = pinned_revision(&text)?;
    let checkout = reference.join("OpenSim-Tranquillity");
    let project = checkout.join("Source/OpenSim.Data.SQLite/OpenSim.Data.SQLite.csproj");
    if !project.is_file() {
        return Err(format!(
            "missing {} (FERROAN_REF={})",
            project.display(),
            reference.display()
        ));
    }
    let actual = git(&checkout, &["rev-parse", "HEAD"])?;
    if actual != pinned {
        return Err(format!(
            "OpenSim-Tranquillity is at {actual}, docs/REFERENCES.md pins {pinned}"
        ));
    }
    let status = git(&checkout, &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Err(format!(
            "OpenSim-Tranquillity checkout is not clean; schema generation only reads it:\n{status}"
        ));
    }
    Ok(pinned)
}

fn pinned_revision(markdown: &str) -> Result<String, String> {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| `OpenSim-Tranquillity`") {
            continue;
        }
        let bytes = trimmed.as_bytes();
        let mut index = 0;
        while index + 42 <= bytes.len() {
            if bytes[index] == b'`' && bytes[index + 41] == b'`' {
                let hash = &trimmed[index + 1..index + 41];
                if hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Ok(hash.to_ascii_lowercase());
                }
            }
            index += 1;
        }
        return Err("OpenSim-Tranquillity row has no 40-character revision".to_string());
    }
    Err("docs/REFERENCES.md has no OpenSim-Tranquillity row".to_string())
}

fn checkout_is_clean(reference: &Path) -> bool {
    match git(
        &reference.join("OpenSim-Tranquillity"),
        &["status", "--porcelain"],
    ) {
        Ok(status) if status.is_empty() => true,
        Ok(status) => {
            eprintln!(
                "schema: the build wrote into the OpenSim-Tranquillity checkout. That checkout is read-only:\n{status}"
            );
            false
        }
        Err(error) => {
            eprintln!("schema: {error}");
            false
        }
    }
}

fn fail_if_dirty(reference: &Path) -> ExitCode {
    let _ = checkout_is_clean(reference);
    ExitCode::FAILURE
}

fn git(checkout: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
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
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
    command
        .current_dir(root)
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("NUGET_XMLDOC_MODE", "skip")
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
        .arg("-p:ContinuousIntegrationBuild=true");
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
            eprintln!("schema: dotnet exited with {status}");
            false
        }
        Err(error) => {
            eprintln!(
                "schema: could not run dotnet (the .NET SDK is required for schema fixtures): {error}"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tranquillity_revision() {
        let markdown = "| `OpenSim-Tranquillity` | somewhere | `713857f10b8e91a639a967d4354a4682b080793c` | note |\n";
        assert_eq!(
            pinned_revision(markdown).unwrap(),
            "713857f10b8e91a639a967d4354a4682b080793c"
        );
    }

    #[test]
    fn rejects_a_table_without_the_row() {
        assert!(
            pinned_revision("| `other` | `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa` |\n").is_err()
        );
    }
}
