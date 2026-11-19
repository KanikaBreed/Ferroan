// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build and run `tools/reference-oracle` against the pinned libopenmetaverse
//! checkout. Build output goes under `.scratch/`, never into the checkout.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

pub enum CommandKind {
    Build,
    Generate,
    Verify,
}

pub fn run(kind: CommandKind, fixture_dir: Option<PathBuf>) -> ExitCode {
    let root = crate::repo_root();
    let reference = match reference_root().and_then(crate::oracle_reference::prepare) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("oracle: {error}");
            return ExitCode::FAILURE;
        }
    };
    let revision = match pinned_and_checked(&root, &reference) {
        Ok(revision) => revision,
        Err(error) => {
            eprintln!("oracle: {error}");
            return ExitCode::FAILURE;
        }
    };

    let artifacts = root.join(".scratch/reference-oracle/artifacts");
    if let Err(error) = fs::create_dir_all(&artifacts) {
        eprintln!("oracle: could not create {}: {error}", artifacts.display());
        return ExitCode::FAILURE;
    }

    let project = root.join("tools/reference-oracle/ReferenceOracle.csproj");
    if !dotnet(&root, &reference, &artifacts, &project, "build", false, &[]) {
        return fail_if_dirty(&reference);
    }
    if !reference_is_clean(&reference) {
        return ExitCode::FAILURE;
    }
    if matches!(kind, CommandKind::Build) {
        println!("oracle: built ReferenceOracle against {revision}");
        return ExitCode::SUCCESS;
    }

    let fixtures = fixture_dir.map_or_else(
        || root.join("fixtures/oracle"),
        |path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        },
    );

    let mut program = vec![
        kind_arg(&kind).to_string(),
        fixtures.display().to_string(),
        "--revision".to_string(),
        revision,
    ];
    if !dotnet(
        &root, &reference, &artifacts, &project, "run", true, &program,
    ) {
        return ExitCode::FAILURE;
    }
    if !matches!(kind, CommandKind::Generate) {
        return ExitCode::SUCCESS;
    }

    let scratch = root.join(".scratch/reference-oracle");
    let left = scratch.join("determinism-a");
    let right = scratch.join("determinism-b");
    if let Err(error) = reset_dir(&left).and_then(|()| reset_dir(&right)) {
        eprintln!("oracle: {error}");
        return ExitCode::FAILURE;
    }
    program[0] = "generate".to_string();
    program[1] = left.display().to_string();
    if !dotnet(
        &root, &reference, &artifacts, &project, "run", true, &program,
    ) {
        return ExitCode::FAILURE;
    }
    program[1] = right.display().to_string();
    if !dotnet(
        &root, &reference, &artifacts, &project, "run", true, &program,
    ) {
        return ExitCode::FAILURE;
    }
    if let Err(error) = same_tree(&left, &right).and_then(|()| same_tree(&left, &fixtures)) {
        eprintln!("oracle: fixtures are not deterministic: {error}");
        let _ = fs::remove_dir_all(&left);
        let _ = fs::remove_dir_all(&right);
        return ExitCode::FAILURE;
    }
    let _ = fs::remove_dir_all(&left);
    let _ = fs::remove_dir_all(&right);

    program[0] = "verify".to_string();
    program[1] = fixtures.display().to_string();
    if !dotnet(
        &root, &reference, &artifacts, &project, "run", true, &program,
    ) {
        return ExitCode::FAILURE;
    }
    println!("oracle: two generates matched and verify passed.");
    ExitCode::SUCCESS
}

fn kind_arg(kind: &CommandKind) -> &'static str {
    match kind {
        CommandKind::Build | CommandKind::Generate => "generate",
        CommandKind::Verify => "verify",
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
    let checkout = reference.join("libopenmetaverse");
    let project = checkout.join("OpenMetaverse.StructuredData/OpenMetaverse.StructuredData.csproj");
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
            "libopenmetaverse is at {actual}, docs/REFERENCES.md pins {pinned}"
        ));
    }
    let status = git(&checkout, &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Err(format!(
            "libopenmetaverse checkout is not clean; the oracle only reads it:\n{status}"
        ));
    }
    let tranq_pin = pinned_named(&text, "OpenSim-Tranquillity")?;
    let tranq = reference.join("OpenSim-Tranquillity");
    let framework = tranq.join("Source/OpenSim.Framework/OpenSim.Framework.csproj");
    if !framework.is_file() {
        return Err(format!(
            "missing {} (FERROAN_REF={})",
            framework.display(),
            reference.display()
        ));
    }
    let tranq_actual = git(&tranq, &["rev-parse", "HEAD"])?;
    if tranq_actual != tranq_pin {
        return Err(format!(
            "OpenSim-Tranquillity is at {tranq_actual}, docs/REFERENCES.md pins {tranq_pin}"
        ));
    }
    let tranq_status = git(&tranq, &["status", "--porcelain"])?;
    if !tranq_status.is_empty() {
        return Err(format!(
            "OpenSim-Tranquillity checkout is not clean; the oracle only reads it:\n{tranq_status}"
        ));
    }
    Ok(pinned)
}

fn reference_is_clean(reference: &Path) -> bool {
    clean_checkout(reference, "libopenmetaverse")
        && clean_checkout(reference, "OpenSim-Tranquillity")
}

fn clean_checkout(reference: &Path, name: &str) -> bool {
    match git(&reference.join(name), &["status", "--porcelain"]) {
        Ok(status) if status.is_empty() => true,
        Ok(status) => {
            eprintln!(
                "oracle: the build wrote into the {name} checkout. That checkout is read-only:\n{status}"
            );
            false
        }
        Err(error) => {
            eprintln!("oracle: {error}");
            false
        }
    }
}

fn fail_if_dirty(reference: &Path) -> ExitCode {
    let _ = reference_is_clean(reference);
    ExitCode::FAILURE
}

fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
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

/// The 40-character revision in the `libopenmetaverse` row of `docs/REFERENCES.md`.
fn pinned_revision(markdown: &str) -> Result<String, String> {
    pinned_named(markdown, "libopenmetaverse")
}

/// The 40-character revision in one checkout row of `docs/REFERENCES.md`.
fn pinned_named(markdown: &str, checkout: &str) -> Result<String, String> {
    let marker = format!("| `{checkout}`");
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(&marker) {
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
        return Err(format!("{checkout} row has no 40-character revision"));
    }
    Err(format!("docs/REFERENCES.md has no {checkout} row"))
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
        .env("FERROAN_REF", reference)
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
            eprintln!("oracle: dotnet exited with {status}");
            false
        }
        Err(error) => {
            eprintln!(
                "oracle: could not run dotnet (the .NET SDK is required for the oracle): {error}"
            );
            false
        }
    }
}

fn reset_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("could not remove {}: {error}", path.display()))?;
    }
    fs::create_dir_all(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))
}

fn same_tree(left: &Path, right: &Path) -> Result<(), String> {
    let left_files = snapshot(left)?;
    let right_files = snapshot(right)?;
    for (name, bytes) in &left_files {
        match right_files.get(name) {
            None => return Err(format!("{name} exists only in {}", left.display())),
            Some(other) if other != bytes => {
                return Err(format!(
                    "{name} differs ({} vs {} bytes, first mismatch at {})",
                    bytes.len(),
                    other.len(),
                    first_mismatch(bytes, other)
                ));
            }
            Some(_) => {}
        }
    }
    for name in right_files.keys() {
        if !left_files.contains_key(name) {
            return Err(format!("{name} exists only in {}", right.display()));
        }
    }
    Ok(())
}

fn snapshot(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    collect(root, root, &mut files)?;
    Ok(files)
}

fn collect(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("could not read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read {}: {error}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, files)?;
            continue;
        }
        let name = entry.file_name();
        if name == "README.md" || name == ".gitattributes" {
            continue;
        }
        let relative = relative_name(root, &path)?;
        let bytes = fs::read(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        files.insert(relative, bytes);
    }
    Ok(())
}

fn relative_name(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("{} is outside {}", path.display(), root.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        parts.push(
            component
                .as_os_str()
                .to_str()
                .ok_or_else(|| format!("{} is not valid Unicode", path.display()))?
                .to_string(),
        );
    }
    Ok(parts.join("/"))
}

fn first_mismatch(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| left.len().min(right.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_libopenmetaverse_row() {
        let text = "| `libopenmetaverse` | [name](https://example.com) | `a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b` | BSD |\n";
        assert_eq!(
            pinned_revision(text).as_deref(),
            Ok("a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b")
        );
    }

    #[test]
    fn rejects_a_table_without_the_row() {
        assert!(
            pinned_revision("| `other` | `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa` |").is_err()
        );
    }

    #[test]
    fn reads_the_checked_in_references() {
        let text = fs::read_to_string(crate::repo_root().join("docs/REFERENCES.md")).unwrap();
        let pin = pinned_revision(&text).unwrap();
        assert_eq!(pin.len(), 40);
        assert!(pin.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
