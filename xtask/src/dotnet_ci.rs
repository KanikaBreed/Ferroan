// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One environment for every harness `dotnet` process.
//!
//! Nerdbank.GitVersioning appends version variables to `GITHUB_ENV` from
//! several MSBuild nodes at once. A torn line fails the Actions step after
//! the build has succeeded. Removing `GITHUB_ENV` leaves the runner's file
//! untouched. `NBGV_SetCloudBuildVersionVars=false` is also required:
//! version 3.10.94 still runs that target when `GITHUB_ACTIONS` is set, and
//! it throws if the path is empty. `MSBUILDDISABLENODEREUSE` stops a worker
//! node from staying alive, and holding locks, after this process exits.

use std::ffi::OsStr;
use std::process::Command;

/// Passed to every harness build, publish and `run` that invokes MSBuild.
pub(crate) const CLOUD_BUILD_VERSION_VARS_OFF: &str = "-p:NBGV_SetCloudBuildVersionVars=false";

/// `dotnet` with the CI shield applied. Callers add their own arguments.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    shield(&mut command);
    command
}

/// Hide the runner's `GITHUB_ENV` and refuse MSBuild node reuse.
pub(crate) fn shield(command: &mut Command) {
    command
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("NUGET_XMLDOC_MODE", "skip")
        .env("MSBUILDDISABLENODEREUSE", "1")
        .env_remove("GITHUB_ENV");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_hides_github_env_and_disables_node_reuse() {
        let mut command = Command::new("true");
        command.env("GITHUB_ENV", "/runner/set_env");
        shield(&mut command);
        let mut saw_github_env = false;
        let mut saw_node_reuse = false;
        for (key, value) in command.get_envs() {
            if key == "GITHUB_ENV" {
                saw_github_env = true;
                assert!(value.is_none());
            } else if key == "MSBUILDDISABLENODEREUSE" {
                saw_node_reuse = value == Some(OsStr::new("1"));
            }
        }
        assert!(saw_github_env);
        assert!(saw_node_reuse);
        assert_eq!(
            CLOUD_BUILD_VERSION_VARS_OFF,
            "-p:NBGV_SetCloudBuildVersionVars=false"
        );
    }

    #[test]
    fn no_harness_dotnet_process_is_built_outside_the_helper() {
        let src = crate::repo_root().join("xtask/src");
        let mut files = std::fs::read_dir(&src)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .collect::<Vec<_>>();
        files.sort();
        for path in files {
            let name = path.file_name().unwrap().to_string_lossy();
            let text = std::fs::read_to_string(&path).unwrap();
            if name != "dotnet_ci.rs" {
                assert!(
                    !text.contains("Command::new(\"dotnet\")")
                        && !text.contains("Command::new(dotnet")
                        && !text.contains("Command::new(&dotnet"),
                    "{name} starts dotnet outside dotnet_ci::command"
                );
            }
        }
        // These files invoke MSBuild. Each one must turn the cloud-build
        // target off, or a partial line can still be written when the
        // variable is present and the property is absent.
        for name in [
            "access_oracle.rs",
            "asset_oracle.rs",
            "config_oracle.rs",
            "console_client.rs",
            "http_oracle.rs",
            "live.rs",
            "oracle.rs",
            "schema.rs",
        ] {
            let text = std::fs::read_to_string(src.join(name)).unwrap();
            assert!(
                text.contains("dotnet_ci::command"),
                "{name} does not use the helper"
            );
            assert!(
                text.contains("CLOUD_BUILD_VERSION_VARS_OFF")
                    || text.contains("-p:NBGV_SetCloudBuildVersionVars=false"),
                "{name} builds without turning cloud build version vars off"
            );
        }
        let workflows = crate::repo_root().join(".github");
        for relative in [
            "workflows/live.yml",
            "workflows/ci.yml",
            "actions/live-setup/action.yml",
        ] {
            let text = std::fs::read_to_string(workflows.join(relative)).unwrap();
            for line in text.lines() {
                let trimmed = line.trim();
                assert!(
                    !trimmed.contains("dotnet build")
                        && !trimmed.contains("dotnet publish")
                        && !trimmed.contains("dotnet run"),
                    "{relative} runs dotnet directly: {trimmed}"
                );
            }
        }
        let setup =
            std::fs::read_to_string(workflows.join("actions/live-setup/action.yml")).unwrap();
        assert!(setup.contains("git clone --filter=blob:none"));
        assert!(!setup.contains("git clone \"$url\""));
    }
}
