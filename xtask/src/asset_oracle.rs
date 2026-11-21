// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run `tools/asset-oracle` against the pinned Tranquillity checkout.
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

struct PluginCross {
    label: &'static str,
    artifacts: &'static str,
    project: &'static str,
    dll: &'static str,
    oracle_env: &'static str,
    ado_prefix: &'static str,
}

const ASSET_CROSS: PluginCross = PluginCross {
    label: "asset",
    artifacts: ".scratch/asset-oracle/artifacts",
    project: "tools/asset-oracle/AssetOracle.csproj",
    dll: "bin/AssetOracle/release/AssetOracle.dll",
    oracle_env: "FERROAN_ASSET_ORACLE",
    ado_prefix: "FERROAN_ASSET_ADO",
};

const GRIDUSER_CROSS: PluginCross = PluginCross {
    label: "griduser",
    artifacts: ".scratch/griduser-oracle/artifacts",
    project: "tools/griduser-oracle/GridUserOracle.csproj",
    dll: "bin/GridUserOracle/release/GridUserOracle.dll",
    oracle_env: "FERROAN_GRIDUSER_ORACLE",
    ado_prefix: "FERROAN_GRIDUSER_ADO",
};

/// Rust writes rows and the pinned C# plugins read them, then the reverse,
/// on SQLite, MySQL and PostgreSQL.
///
/// The local containers are the ones `cargo xtask schema servers` uses.
/// Connection strings stay in the child environment and are not printed.
pub fn cross() -> ExitCode {
    run_plugin_cross(
        &ASSET_CROSS,
        "csharp_and_rust_read_each_others_rows",
        "FERROAN_ASSET_CROSS",
        true,
    )
}

/// Rust writes `XAssetsMeta` / `XAssetsData` rows and the pinned XAsset plugins
/// read them, then the reverse, on MySQL and PostgreSQL.
pub fn xasset_cross() -> ExitCode {
    run_plugin_cross(
        &ASSET_CROSS,
        "xasset_plugins_cross_read",
        "FERROAN_XASSET_CROSS",
        true,
    )
}

/// Rust writes `fsassets` rows and the pinned FSAssets plugins read them, then
/// the reverse, on MySQL and PostgreSQL. No filesystem and no live harness.
pub fn fsassets_cross() -> ExitCode {
    run_plugin_cross(
        &ASSET_CROSS,
        "fsassets_plugins_cross_read",
        "FERROAN_FSASSET_CROSS",
        true,
    )
}

/// Rust and the pinned `GetFsData` read each other's FSAssets files.
/// No database and no live harness.
pub fn fsassets_files() -> ExitCode {
    run_plugin_cross(
        &ASSET_CROSS,
        "fsassets_files_cross_read",
        "FERROAN_FSASSET_FILES",
        false,
    )
}

/// Rust writes `GridUser` rows and the pinned grid-user plugins read them,
/// then the reverse, on SQLite, MySQL and PostgreSQL. No HTTP and no live harness.
pub fn griduser_cross() -> ExitCode {
    run_plugin_cross(
        &GRIDUSER_CROSS,
        "griduser_plugins_cross_read",
        "FERROAN_GRIDUSER_CROSS",
        true,
    )
}

fn run_plugin_cross(
    target: &PluginCross,
    test_name: &str,
    gate: &str,
    databases: bool,
) -> ExitCode {
    let root = crate::repo_root();
    let reference = match reference_root().and_then(crate::oracle_reference::prepare) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{}: {error}", target.label);
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = pinned_and_checked(&root, &reference) {
        eprintln!("{}: {error}", target.label);
        return ExitCode::FAILURE;
    }
    let artifacts = root.join(target.artifacts);
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!(
            "{}: could not create {}: {error}",
            target.label,
            artifacts.display()
        );
        return ExitCode::FAILURE;
    }
    let project = root.join(target.project);
    if !dotnet(
        target.label,
        &root,
        &reference,
        &artifacts,
        &project,
        "build",
        false,
        &[],
    ) {
        return fail_if_dirty(target.label, &reference);
    }
    if !checkout_is_clean(target.label, &reference) {
        return ExitCode::FAILURE;
    }
    let dll = artifacts.join(target.dll);
    if !dll.is_file() {
        eprintln!("{}: missing {}", target.label, dll.display());
        return ExitCode::FAILURE;
    }
    let mut command = Command::new("cargo");
    command
        .current_dir(&root)
        .env(gate, "1")
        .env(target.oracle_env, &dll);
    if databases {
        let mysql_ado = format!("{}_MYSQL", target.ado_prefix);
        let postgres_ado = format!("{}_POSTGRES", target.ado_prefix);
        command
            .env(
                "FERROAN_MYSQL_URL",
                "mysql://ferroan:ferroan@127.0.0.1:13306/ferroan?ssl-mode=disabled",
            )
            .env(
                "FERROAN_POSTGRES_URL",
                "postgres://ferroan:ferroan@127.0.0.1:15432/ferroan?sslmode=disable",
            )
            .env(
                mysql_ado,
                "Server=127.0.0.1;Port=13306;Database=ferroan;User ID=ferroan;Password=ferroan;SslMode=None;Pooling=false",
            )
            .env(
                postgres_ado,
                "Server=127.0.0.1;Port=15432;Database=ferroan;User Id=ferroan;Password=ferroan;SSL Mode=Disable;Pooling=false",
            );
    }
    command.args([
        "test",
        "-p",
        "ferroan-services",
        "--lib",
        test_name,
        "--",
        "--nocapture",
    ]);
    println!("$ cargo test -p ferroan-services --lib {test_name}");
    match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("{}: cross-read exited with {status}", target.label);
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("{}: could not run cargo: {error}", target.label);
            ExitCode::FAILURE
        }
    }
}

fn run(writing: bool, fixture_dir: Option<PathBuf>) -> ExitCode {
    let root = crate::repo_root();
    let reference = match reference_root().and_then(crate::oracle_reference::prepare) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("asset: {error}");
            return ExitCode::FAILURE;
        }
    };
    let revision = match pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("asset: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifacts = root.join(".scratch/asset-oracle/artifacts");
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("asset: could not create {}: {error}", artifacts.display());
        return ExitCode::FAILURE;
    }
    let project = root.join("tools/asset-oracle/AssetOracle.csproj");
    let fixtures = fixture_dir.map_or_else(
        || root.join("fixtures/asset"),
        |path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        },
    );
    if !dotnet(
        "asset",
        &root,
        &reference,
        &artifacts,
        &project,
        "build",
        false,
        &[],
    ) {
        return fail_if_dirty("asset", &reference);
    }
    if !checkout_is_clean("asset", &reference) {
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
        "asset", &root, &reference, &artifacts, &project, "run", true, &program,
    ) {
        return fail_if_dirty("asset", &reference);
    }
    if !checkout_is_clean("asset", &reference) {
        return ExitCode::FAILURE;
    }
    if !pilot_tests(&root, &reference) {
        return fail_if_dirty("asset", &reference);
    }
    println!("asset: {action} passed against {revision}");
    ExitCode::SUCCESS
}

fn pilot_tests(root: &Path, reference: &Path) -> bool {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .env("FERROAN_ASSET_LIBRARY", "1")
        .env("FERROAN_REF", reference)
        .args([
            "test",
            "-p",
            "ferroan-services",
            "--lib",
            "pilot_",
            "--locked",
            "--",
            "--test-threads=8",
        ]);
    println!("$ cargo test -p ferroan-services --lib pilot_");
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("asset: pilot tests exited with {status}");
            false
        }
        Err(error) => {
            eprintln!("asset: could not run cargo: {error}");
            false
        }
    }
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
    let project = checkout.join("Source/OpenSim.Server.Base/OpenSim.Server.Base.csproj");
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
            "OpenSim-Tranquillity checkout is not clean; asset generation only reads it:\n{status}"
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

fn checkout_is_clean(label: &str, reference: &Path) -> bool {
    match git(
        &reference.join("OpenSim-Tranquillity"),
        &["status", "--porcelain"],
    ) {
        Ok(status) if status.is_empty() => true,
        Ok(status) => {
            eprintln!(
                "{label}: the build wrote into the OpenSim-Tranquillity checkout. That checkout is read-only:\n{status}"
            );
            false
        }
        Err(error) => {
            eprintln!("{label}: {error}");
            false
        }
    }
}

fn fail_if_dirty(label: &str, reference: &Path) -> ExitCode {
    let _ = checkout_is_clean(label, reference);
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

#[allow(clippy::too_many_arguments)]
fn dotnet(
    label: &str,
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
            eprintln!("{label}: dotnet exited with {status}");
            false
        }
        Err(error) => {
            eprintln!(
                "{label}: could not run dotnet (the .NET SDK is required for this oracle): {error}"
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
}
