// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build oracle dependencies from task-owned clones, never reference sources.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub fn prepare(reference: PathBuf) -> Result<PathBuf, String> {
    let destination = crate::repo_root().join(".scratch/oracle-sources");
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    for name in ["OpenSim-Tranquillity", "libopenmetaverse"] {
        let source = reference.join(name);
        if !source.exists() {
            continue;
        }
        if !git(&source, &["status", "--porcelain"])?.is_empty() {
            return Err(format!("reference {name} is not clean"));
        }
        let revision = git(&source, &["rev-parse", "HEAD"])?;
        let target = destination.join(name);
        if !target.exists() {
            let result = Command::new("git")
                .args(["clone", "--no-hardlinks", "--no-checkout"])
                .arg(&source)
                .arg(&target)
                .output()
                .map_err(|error| error.to_string())?;
            if !result.status.success() {
                return Err(format!("could not clone oracle source {name}"));
            }
        } else if !git(&target, &["status", "--porcelain"])?.is_empty() {
            return Err(format!(
                "oracle source {name} contains edits; preserve them before rebuilding"
            ));
        }
        if git(&target, &["cat-file", "-e", &revision]).is_err() {
            let source = source
                .to_str()
                .ok_or("oracle reference path is not UTF-8")?;
            git(&target, &["fetch", source, &revision])?;
        }
        git(&target, &["checkout", "--detach", &revision])?;
    }
    Ok(destination)
}

fn git(directory: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("oracle source git {} failed", args[0]));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
