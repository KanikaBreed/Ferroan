// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rebuild `tools/yengine-tokens-oracle` against a task-owned clone of the pin
//! and diff a fresh run with `fixtures/yengine-tokens`. The shared reference
//! checkout is only read. This command is not part of `cargo xtask check`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::asset_oracle::{
    checkout_is_clean, fail_if_dirty, git, pinned_and_checked, reference_root,
};
use crate::oracle_reference::prepare;

const LABEL: &str = "yengine-tokens";
const ARTIFACTS: &str = ".scratch/yengine-tokens/artifacts";
const EXTENSIONS: &str = ".scratch/yengine-tokens/msbuild";
const PROJECT: &str = "tools/yengine-tokens-oracle/YEngineTokensOracle.csproj";
const LOGGING: &str = "tools/yengine-tokens-oracle/Logging.props";
const FIXTURE: &str = "fixtures/yengine-tokens";
const FRESH: &str = ".scratch/yengine-tokens/fresh";
const CASES: &str = "fixtures/yengine-tokens/cases.json";
const DLL_NAME: &str = "YEngineTokensOracle.dll";

pub fn verify() -> ExitCode {
    let root = crate::repo_root();
    let (dll, reference) = match build_oracle() {
        Ok(built) => built,
        Err(code) => return code,
    };
    let fresh = root.join(FRESH);
    if fresh.exists()
        && let Err(error) = fs::remove_dir_all(&fresh)
    {
        eprintln!("{LABEL}: could not clear {}: {error}", fresh.display());
        return ExitCode::FAILURE;
    }
    if let Err(error) = fs::create_dir_all(&fresh) {
        eprintln!("{LABEL}: could not create {}: {error}", fresh.display());
        return ExitCode::FAILURE;
    }
    let cases = root.join(CASES);
    let mut command = crate::dotnet_ci::command("dotnet");
    command
        .current_dir(&root)
        .env("FERROAN_REF", &reference)
        .arg(&dll)
        .arg(&cases)
        .arg(&fresh);
    println!(
        "$ FERROAN_REF=.scratch/oracle-sources dotnet {} {} {}",
        display(&root, &dll),
        display(&root, &cases),
        display(&root, &fresh)
    );
    let output = match command.output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("{LABEL}: could not run dotnet: {error}");
            return ExitCode::FAILURE;
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }
    if !output.status.success() {
        eprintln!(
            "{LABEL}: oracle exited with {}\n{}",
            output.status,
            stdout.trim()
        );
        return ExitCode::FAILURE;
    }
    if !stdout.trim().is_empty() {
        println!("{}", stdout.trim());
    }
    match diff(&root.join(FIXTURE), &fresh) {
        Ok(()) => {
            println!("{LABEL}: {FIXTURE} matches the pin");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn build_oracle() -> Result<(PathBuf, PathBuf), ExitCode> {
    let root = crate::repo_root();
    let source = match reference_root() {
        Ok(path) => path.join("OpenSim-Tranquillity"),
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let source_head = match git(&source, &["rev-parse", "HEAD"]) {
        Ok(head) => head,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let Some(parent) = source.parent().map(Path::to_path_buf) else {
        eprintln!("{LABEL}: the reference checkout has no parent");
        return Err(ExitCode::FAILURE);
    };
    let reference = match prepare(parent) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let pinned = match fs::read_to_string(root.join("docs/REFERENCES.md"))
        .map_err(|error| error.to_string())
        .and_then(|text| crate::asset_oracle::pinned_revision(&text))
    {
        Ok(pin) => pin,
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    let clone = reference.join("OpenSim-Tranquillity");
    if let Err(error) = detach_at_pin(&clone, &source, &pinned) {
        eprintln!("{LABEL}: {error}");
        return Err(ExitCode::FAILURE);
    }
    if let Err(error) = pinned_and_checked(&root, &reference) {
        eprintln!("{LABEL}: {error}");
        return Err(ExitCode::FAILURE);
    }
    let artifacts = root.join(ARTIFACTS);
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("{LABEL}: could not create {}: {error}", artifacts.display());
        return Err(ExitCode::FAILURE);
    }
    let extensions = root.join(EXTENSIONS);
    if let Err(error) = install_logging(&root, &reference, &extensions) {
        eprintln!("{LABEL}: {error}");
        return Err(ExitCode::FAILURE);
    }
    let project = root.join(PROJECT);
    if !dotnet_build(&root, &reference, &artifacts, &extensions, &project) {
        return Err(fail_if_dirty(LABEL, &reference));
    }
    if !checkout_is_clean(LABEL, &reference) {
        return Err(ExitCode::FAILURE);
    }
    if !source_unchanged(&source, &source_head) {
        return Err(ExitCode::FAILURE);
    }
    match locate_dll(&artifacts) {
        Ok(dll) => Ok((dll, reference)),
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            Err(ExitCode::FAILURE)
        }
    }
}

/// YEngine calls `Microsoft.Extensions.Logging` and does not reference it.
/// The props file is imported from a scratch user-extensions directory so the
/// pin is not edited. `Current` is what this SDK reports for `MSBuildToolsVersion`.
fn install_logging(root: &Path, reference: &Path, extensions: &Path) -> Result<(), String> {
    let version = msbuild_tools_version(root, &root.join(PROJECT), reference)?;
    let dest = extensions
        .join(version)
        .join("Imports")
        .join("Microsoft.Common.props")
        .join("ImportBefore");
    fs::create_dir_all(&dest).map_err(|error| error.to_string())?;
    fs::copy(root.join(LOGGING), dest.join("Logging.props"))
        .map_err(|error| format!("could not copy Logging.props: {error}"))?;
    Ok(())
}

fn msbuild_tools_version(root: &Path, project: &Path, reference: &Path) -> Result<String, String> {
    let mut command = crate::dotnet_ci::command("dotnet");
    command
        .current_dir(root)
        .arg("msbuild")
        .arg(project)
        .arg("-getProperty:MSBuildToolsVersion")
        .arg("-nologo")
        .arg(format!("-p:FerroanRef={}", reference.display()));
    let output = command
        .output()
        .map_err(|error| format!("could not run dotnet msbuild: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "could not read MSBuildToolsVersion ({}): {}",
            output.status,
            stderr.trim()
        ));
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty()
        || version.contains('/')
        || version.contains('\\')
        || version.contains("..")
        || version.chars().any(char::is_control)
    {
        return Err(format!("unexpected MSBuildToolsVersion {version:?}"));
    }
    Ok(version)
}

fn dotnet_build(
    root: &Path,
    reference: &Path,
    artifacts: &Path,
    extensions: &Path,
    project: &Path,
) -> bool {
    let mut artifacts_arg = artifacts.display().to_string();
    if !artifacts_arg.ends_with('/') {
        artifacts_arg.push('/');
    }
    let mut command = crate::dotnet_ci::command("dotnet");
    command
        .current_dir(root)
        .arg("build")
        .arg(project)
        .arg("-c")
        .arg("Release")
        .arg("--verbosity")
        .arg("minimal")
        .arg("-m:1")
        .arg(format!("-p:FerroanRef={}", reference.display()))
        .arg("-p:UseArtifactsOutput=true")
        .arg(format!("-p:ArtifactsPath={artifacts_arg}"))
        .arg("-p:GeneratePackageOnBuild=false")
        .arg("-p:ContinuousIntegrationBuild=true")
        .arg("-p:UseSharedCompilation=false")
        .arg("-p:TreatWarningsAsErrors=false")
        .arg("-p:ImportUserLocationsByWildcardBeforeMicrosoftCommonProps=true")
        .arg(format!(
            "-p:MSBuildUserExtensionsPath={}",
            extensions.display()
        ))
        .arg(crate::dotnet_ci::CLOUD_BUILD_VERSION_VARS_OFF);
    println!("$ dotnet build -c Release -m:1 {}", display(root, project));
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("{LABEL}: dotnet exited with {status}");
            false
        }
        Err(error) => {
            eprintln!(
                "{LABEL}: could not run dotnet (the .NET SDK is required for this oracle): {error}"
            );
            false
        }
    }
}

fn locate_dll(artifacts: &Path) -> Result<PathBuf, String> {
    let mut found = Vec::new();
    find_app_dll(artifacts, &mut found)?;
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!("missing {DLL_NAME} under {}", artifacts.display())),
        _ => Err(format!(
            "several {DLL_NAME} files under {}",
            artifacts.display()
        )),
    }
}

fn find_app_dll(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            find_app_dll(&path, found)?;
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some(DLL_NAME)
            && path
                .with_file_name("YEngineTokensOracle.deps.json")
                .is_file()
        {
            found.push(path);
        }
    }
    Ok(())
}

fn detach_at_pin(clone: &Path, source: &Path, pin: &str) -> Result<(), String> {
    if git(clone, &["cat-file", "-e", &format!("{pin}^{{commit}}")]).is_err() {
        let remote = source.to_str().ok_or("reference path is not UTF-8")?;
        git(clone, &["fetch", remote, pin])?;
    }
    git(clone, &["checkout", "--detach", pin])?;
    Ok(())
}

fn source_unchanged(source: &Path, head: &str) -> bool {
    match git(source, &["rev-parse", "HEAD"]) {
        Ok(now) if now == head => {}
        Ok(now) => {
            eprintln!("{LABEL}: the reference checkout moved from {head} to {now}");
            return false;
        }
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            return false;
        }
    }
    match git(source, &["status", "--porcelain"]) {
        Ok(status) if status.is_empty() => true,
        Ok(status) => {
            eprintln!("{LABEL}: the reference checkout is no longer clean:\n{status}");
            false
        }
        Err(error) => {
            eprintln!("{LABEL}: {error}");
            false
        }
    }
}

fn diff(committed: &Path, fresh: &Path) -> Result<(), String> {
    let mut problems = Vec::new();
    for name in ["digest.json", "manifest.json"] {
        compare_file(committed, fresh, name, &mut problems);
    }
    let left = yobj_names(&committed.join("objects"), &mut problems, "committed");
    let right = yobj_names(&fresh.join("objects"), &mut problems, "fresh");
    let mut names: Vec<String> = left.iter().chain(right.iter()).cloned().collect();
    names.sort();
    names.dedup();
    for name in &names {
        let relative = format!("objects/{name}");
        if !left.iter().any(|item| item == name) {
            problems.push(format!("oracle wrote an extra {relative}"));
            continue;
        }
        if !right.iter().any(|item| item == name) {
            problems.push(format!("oracle did not write {relative}"));
            continue;
        }
        compare_file(committed, fresh, &relative, &mut problems);
    }
    match fs::read_dir(fresh) {
        Ok(entries) => {
            for entry in entries {
                let Ok(entry) = entry else {
                    problems.push("could not read a fresh oracle file".to_string());
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "digest.json"
                    || name == "manifest.json"
                    || name == "canonical.txt"
                    || name == "objects"
                {
                    continue;
                }
                problems.push(format!("oracle wrote an unexpected {name}"));
            }
        }
        Err(error) => problems.push(format!("could not read the fresh directory: {error}")),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

fn yobj_names(dir: &Path, problems: &mut Vec<String>, which: &str) -> Vec<String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            problems.push(format!("could not read {which} objects: {error}"));
            return Vec::new();
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            problems.push(format!("could not read a {which} object file"));
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".yobj") || !entry.path().is_file() {
            problems.push(format!("{which} objects has unexpected {name}"));
            continue;
        }
        names.push(name);
    }
    names.sort();
    names
}

fn compare_file(committed: &Path, fresh: &Path, name: &str, problems: &mut Vec<String>) {
    let left = match fs::read(committed.join(name)) {
        Ok(bytes) => bytes,
        Err(error) => {
            problems.push(format!("could not read {FIXTURE}/{name}: {error}"));
            return;
        }
    };
    let right = match fs::read(fresh.join(name)) {
        Ok(bytes) => bytes,
        Err(error) => {
            problems.push(format!("oracle did not write {name}: {error}"));
            return;
        }
    };
    if left != right {
        problems.push(format!(
            "{FIXTURE}/{name} differs from the pin (committed {}, fresh {})",
            left.len(),
            right.len()
        ));
    }
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
