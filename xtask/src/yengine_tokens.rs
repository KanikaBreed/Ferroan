// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rebuild `tools/yengine-tokens-oracle` against a task-owned clone of the pin
//! and diff a fresh run with `fixtures/yengine-tokens` and `fixtures/yengine-slots`.
//! The slot run asks for the name, type and index digest. The shared reference
//! checkout is only read. One build serves both fixtures. This command is not
//! part of `cargo xtask check`.

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
const SLOT_FIXTURE: &str = "fixtures/yengine-slots";
const SLOT_FRESH: &str = ".scratch/yengine-tokens/slots-fresh";
const SLOT_CASES: &str = "fixtures/yengine-slots/cases.json";
const DLL_NAME: &str = "YEngineTokensOracle.dll";

struct FixtureCheck {
    fixture: &'static str,
    fresh: &'static str,
    cases: &'static str,
    slots: bool,
    published: &'static [&'static str],
    scratch: &'static [&'static str],
}

pub fn verify() -> ExitCode {
    let (dll, reference) = match build_oracle() {
        Ok(built) => built,
        Err(code) => return code,
    };
    let tokens = check_fixture(
        &dll,
        &reference,
        &FixtureCheck {
            fixture: FIXTURE,
            fresh: FRESH,
            cases: CASES,
            slots: false,
            published: &["digest.json", "manifest.json"],
            scratch: &["canonical.txt"],
        },
    );
    let slots = check_fixture(
        &dll,
        &reference,
        &FixtureCheck {
            fixture: SLOT_FIXTURE,
            fresh: SLOT_FRESH,
            cases: SLOT_CASES,
            slots: true,
            published: &["digest.json"],
            scratch: &[],
        },
    );
    if tokens == ExitCode::SUCCESS && slots == ExitCode::SUCCESS {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn check_fixture(dll: &Path, reference: &Path, check: &FixtureCheck) -> ExitCode {
    let root = crate::repo_root();
    let fresh = root.join(check.fresh);
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
    let cases = root.join(check.cases);
    let mut command = crate::dotnet_ci::command("dotnet");
    command
        .current_dir(&root)
        .env("FERROAN_REF", reference)
        .arg(dll);
    let mut shown = vec![display(&root, dll)];
    if check.slots {
        command.arg("--slots");
        shown.push("--slots".to_string());
    }
    command.arg(&cases).arg(&fresh);
    shown.push(display(&root, &cases));
    shown.push(display(&root, &fresh));
    println!(
        "$ FERROAN_REF=.scratch/oracle-sources dotnet {}",
        shown.join(" ")
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
    match diff(
        &root.join(check.fixture),
        &fresh,
        check.fixture,
        check.published,
        check.scratch,
    ) {
        Ok(()) => {
            println!("{LABEL}: {} matches the pin", check.fixture);
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

fn diff(
    committed: &Path,
    fresh: &Path,
    fixture: &str,
    published: &[&str],
    scratch: &[&str],
) -> Result<(), String> {
    let mut problems = Vec::new();
    for name in published {
        compare_file(committed, fresh, fixture, name, &mut problems);
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
        compare_file(committed, fresh, fixture, &relative, &mut problems);
    }
    match fs::read_dir(fresh) {
        Ok(entries) => {
            for entry in entries {
                let Ok(entry) = entry else {
                    problems.push("could not read a fresh oracle file".to_string());
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                if published.contains(&name.as_str())
                    || scratch.contains(&name.as_str())
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

fn compare_file(
    committed: &Path,
    fresh: &Path,
    fixture: &str,
    name: &str,
    problems: &mut Vec<String>,
) {
    let left = match fs::read(committed.join(name)) {
        Ok(bytes) => bytes,
        Err(error) => {
            problems.push(format!("could not read {fixture}/{name}: {error}"));
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
            "{fixture}/{name} differs from the pin (committed {}, fresh {})",
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
