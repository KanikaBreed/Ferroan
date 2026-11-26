// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build the pinned `OpenSim.ConsoleClient` and point it at a Ferroan console.
//!
//! The step is not part of `cargo xtask check`. It skips, with exit 0, where
//! `dotnet` is absent. A missing pin is an error that names `FERROAN_REF`.
//! Credentials are synthetic and are not printed.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const USER: &str = "console-test";
const PASS: &str = "console-test-pass";

pub fn run() -> ExitCode {
    match drive() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("console-client: {}", scrub(&error));
            ExitCode::from(1)
        }
    }
}

fn drive() -> Result<String, String> {
    let pin = locate_pin()?;
    let Some(dotnet) = which_dotnet() else {
        return Ok("console-client: skipped (dotnet is absent)".to_string());
    };
    let dll = build(&dotnet, &pin)?;
    // Worker threads keep the console serving after `listen` returns.
    // A current-thread runtime stops polling when `block_on` returns.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("could not start the console runtime: {error}"))?;
    let settings = ferroan_log::Settings::remote_console("ferroan-region", USER, PASS);
    let guard = ferroan_log::install(&settings).map_err(|error| error.to_string())?;
    let router = guard
        .console_router()
        .ok_or_else(|| "the remote console was not registered".to_string())?;
    let listener = runtime
        .block_on(ferroan_log::listen(router))
        .map_err(|error| error.to_string())?;
    let port = listener.addr.port();
    tracing::info!("remote console ready");
    let output = invoke(&dotnet, &dll, port)?;
    drop(listener);
    drop(guard);
    judge(&output)
}

fn judge(output: &str) -> Result<String, String> {
    if output.contains("Ferroan 0.1.0") {
        return Ok("console-client: OpenSim.ConsoleClient printed Ferroan 0.1.0".to_string());
    }
    // Pin5 carries the TB17 fix (#465). The old key error is a failure.
    if output.contains("was not present in the dictionary") {
        return Err(
            "console-client: OpenSim.ConsoleClient stopped in CommandConsole.FromXml before show version"
                .to_string(),
        );
    }
    Err(format!(
        "OpenSim.ConsoleClient did not print Ferroan 0.1.0: {}",
        scrub(output)
    ))
}

fn invoke(dotnet: &Path, dll: &Path, port: u16) -> Result<String, String> {
    let mut child = crate::dotnet_ci::command(dotnet)
        .arg(dll)
        .arg("--console")
        .arg("basic")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--user")
        .arg(USER)
        .arg("--pass")
        .arg(PASS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start OpenSim.ConsoleClient: {error}"))?;
    // Leave stdin open. Closing it makes Prompt() parse a null line and throw.
    // Login is asynchronous. Since Pin8 (TB32, #528) `ReadResponses` shows
    // every line as sent and takes its prompt from StartSession. The client
    // still prints its own `Client> ` prompt first (line 134). `remote
    // console ready` is a scroll line from after `FromXml`, so the help tree
    // is loaded before the send.
    let mut stdin = child.stdin.take();
    let collected = Arc::new(Mutex::new(String::new()));
    if let Some(pipe) = child.stdout.take() {
        let slot = Arc::clone(&collected);
        thread::spawn(move || pump(pipe, slot));
    }
    if let Some(pipe) = child.stderr.take() {
        let slot = Arc::clone(&collected);
        thread::spawn(move || pump(pipe, slot));
    }
    let started = Instant::now();
    let mut sent = false;
    loop {
        let text = collected
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let logged_in = text.contains("remote console ready")
            || text.contains("+++")
            || text.contains("Ferroan#");
        if !sent && logged_in && text.contains("Client>") {
            if let Some(stdin) = stdin.as_mut() {
                let _ = stdin.write_all(b"show version\n");
                let _ = stdin.flush();
            }
            sent = true;
        }
        if text.contains("Ferroan 0.1.0")
            // The pin's message is "The given key 'quit' was not present in the dictionary."
            || text.contains("was not present in the dictionary")
            || text.contains("Connection data info was not valid")
        {
            break;
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(15) => break,
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(format!("could not wait for OpenSim.ConsoleClient: {error}")),
        }
    }
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    thread::sleep(Duration::from_millis(200));
    let text = collected
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    Ok(text)
}

fn pump(mut pipe: impl Read + Send, slot: Arc<Mutex<String>>) {
    let mut buf = [0_u8; 256];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut text = slot.lock().unwrap_or_else(|poison| poison.into_inner());
                text.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }
    }
}

fn build(dotnet: &Path, pin: &Path) -> Result<PathBuf, String> {
    let scratch = crate::repo_root().join(".scratch/console-client");
    let bin = scratch.join("bin");
    let obj = scratch.join("obj");
    fs::create_dir_all(&obj)
        .map_err(|error| format!("could not create the build directory: {error}"))?;
    let project = pin.join("Source/OpenSim.ConsoleClient/OpenSim.ConsoleClient.csproj");
    if !project.is_file() {
        return Err("OpenSim.ConsoleClient.csproj is not in the pin".to_string());
    }
    // Keep the reference tree clean. One intermediate directory is enough:
    // per-project paths make ResolveProjectReferences circular.
    let intermediate = format!("IntermediateOutputPath={}/", obj.display());
    let output = crate::dotnet_ci::command(dotnet)
        .arg("build")
        .arg(&project)
        .arg("-c")
        .arg("Release")
        .arg("-o")
        .arg(&bin)
        .arg("-p:UseSharedCompilation=false")
        .arg(crate::dotnet_ci::CLOUD_BUILD_VERSION_VARS_OFF)
        .arg(format!("-p:{intermediate}"))
        .output()
        .map_err(|error| format!("could not run dotnet build: {error}"))?;
    if !output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        return Err(format!("dotnet build failed: {}", scrub(&text)));
    }
    // The client's deps.json does not list Microsoft.Extensions.Logging.
    // WebUtil's type initializer needs it. A runtime that has no ASP.NET
    // shared framework (this machine) then fails before login. The framework
    // library's own deps.json has the closure; fold it in and probe the
    // NuGet cache. CI's shared framework already has the assembly, and the
    // same closure still resolves there.
    fold_logging_closure(&bin)?;
    let dll = bin.join("OpenSim.ConsoleClient.dll");
    if dll.is_file() {
        Ok(dll)
    } else {
        Err("dotnet build did not write OpenSim.ConsoleClient.dll".to_string())
    }
}

/// Packages `WebUtil`'s logger needs, as named in `OpenSim.Framework.deps.json`.
fn logging_package(name: &str) -> bool {
    name.starts_with("Microsoft.Extensions.Logging")
        || name.starts_with("Microsoft.Extensions.Options/")
        || name.starts_with("Microsoft.Extensions.DependencyInjection")
        || name.starts_with("Microsoft.Extensions.Primitives/")
}

fn fold_logging_closure(bin: &Path) -> Result<(), String> {
    let client_path = bin.join("OpenSim.ConsoleClient.deps.json");
    let framework_path = bin.join("OpenSim.Framework.deps.json");
    let mut client: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&client_path)
            .map_err(|error| format!("could not read ConsoleClient deps.json: {error}"))?,
    )
    .map_err(|error| format!("could not parse ConsoleClient deps.json: {error}"))?;
    let framework: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&framework_path)
            .map_err(|error| format!("could not read Framework deps.json: {error}"))?,
    )
    .map_err(|error| format!("could not parse Framework deps.json: {error}"))?;
    let framework_targets = framework
        .get("targets")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "Framework deps.json has no targets".to_string())?;
    let client_targets = client
        .get_mut("targets")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "ConsoleClient deps.json has no targets".to_string())?;
    for (tfm, entries) in client_targets {
        let Some(client_entries) = entries.as_object_mut() else {
            continue;
        };
        let Some(framework_entries) = framework_targets
            .get(tfm)
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        for (name, info) in framework_entries {
            if logging_package(name) && !client_entries.contains_key(name) {
                client_entries.insert(name.clone(), info.clone());
            }
        }
    }
    let framework_libraries = framework
        .get("libraries")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "Framework deps.json has no libraries".to_string())?;
    let client_libraries = client
        .get_mut("libraries")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "ConsoleClient deps.json has no libraries".to_string())?;
    for (name, info) in framework_libraries {
        if logging_package(name) && !client_libraries.contains_key(name) {
            client_libraries.insert(name.clone(), info.clone());
        }
    }
    fs::write(
        &client_path,
        serde_json::to_vec_pretty(&client)
            .map_err(|error| format!("could not encode ConsoleClient deps.json: {error}"))?,
    )
    .map_err(|error| format!("could not write ConsoleClient deps.json: {error}"))?;

    let nuget = nuget_packages_dir()?;
    let runtime_path = bin.join("OpenSim.ConsoleClient.runtimeconfig.json");
    let mut runtime: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&runtime_path)
            .map_err(|error| format!("could not read ConsoleClient runtimeconfig: {error}"))?,
    )
    .map_err(|error| format!("could not parse ConsoleClient runtimeconfig: {error}"))?;
    let options = runtime
        .get_mut("runtimeOptions")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "ConsoleClient runtimeconfig has no runtimeOptions".to_string())?;
    let paths = options
        .entry("additionalProbingPaths")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let paths = paths
        .as_array_mut()
        .ok_or_else(|| "additionalProbingPaths is not an array".to_string())?;
    let nuget_text = nuget.to_string_lossy().into_owned();
    if !paths
        .iter()
        .any(|path| path.as_str() == Some(nuget_text.as_str()))
    {
        paths.push(serde_json::Value::String(nuget_text));
    }
    fs::write(
        &runtime_path,
        serde_json::to_vec_pretty(&runtime)
            .map_err(|error| format!("could not encode ConsoleClient runtimeconfig: {error}"))?,
    )
    .map_err(|error| format!("could not write ConsoleClient runtimeconfig: {error}"))?;
    Ok(())
}

fn nuget_packages_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("NUGET_PACKAGES") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        "NUGET_PACKAGES and HOME are unset, so the logging assemblies cannot be probed".to_string()
    })?;
    Ok(PathBuf::from(home).join(".nuget/packages"))
}

fn locate_pin() -> Result<PathBuf, String> {
    let ferroan_ref = std::env::var_os("FERROAN_REF").map(PathBuf::from);
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    choose_pin(ferroan_ref.as_deref(), home.as_deref())
}

/// `FERROAN_REF/OpenSim-Tranquillity`, or `~/Git/reference/OpenSim-Tranquillity`
/// when that variable is unset. docs/REFERENCES.md names the second location.
fn choose_pin(ferroan_ref: Option<&Path>, home: Option<&Path>) -> Result<PathBuf, String> {
    let mut tried = Vec::new();
    if let Some(root) = ferroan_ref.filter(|path| !path.as_os_str().is_empty()) {
        let path = root.join("OpenSim-Tranquillity");
        if pin_present(&path) {
            return Ok(path);
        }
        tried.push(path);
    }
    if let Some(home) = home.filter(|path| !path.as_os_str().is_empty()) {
        let path = home
            .join("Git")
            .join("reference")
            .join("OpenSim-Tranquillity");
        if pin_present(&path) {
            return Ok(path);
        }
        tried.push(path);
    }
    let looked = if tried.is_empty() {
        "nowhere, because FERROAN_REF and HOME / USERPROFILE are unset".to_string()
    } else {
        tried
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(format!(
        "set FERROAN_REF to the parent of the OpenSim-Tranquillity checkout (docs/REFERENCES.md uses ~/Git/reference). Looked at {looked}"
    ))
}

fn pin_present(path: &Path) -> bool {
    path.join("Source/OpenSim.ConsoleClient/OpenSim.ConsoleClient.csproj")
        .is_file()
}

fn which_dotnet() -> Option<PathBuf> {
    let output = crate::dotnet_ci::command("dotnet")
        .arg("--version")
        .output()
        .ok()?;
    if output.status.success() {
        Some(PathBuf::from("dotnet"))
    } else {
        None
    }
}

fn scrub(text: &str) -> String {
    let cleaned = text.replace(PASS, "[redacted]");
    let head: String = cleaned.chars().take(800).collect();
    head.replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempTree(PathBuf);

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pin_tree() -> TempTree {
        let root =
            std::env::temp_dir().join(format!("ferroan-console-pin-{}", uuid::Uuid::new_v4()));
        let project = root
            .join("OpenSim-Tranquillity")
            .join("Source")
            .join("OpenSim.ConsoleClient");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("OpenSim.ConsoleClient.csproj"), b"<Project />").unwrap();
        TempTree(root)
    }

    #[test]
    fn the_pin_comes_from_ferroan_ref_or_the_documented_reference() {
        let tree = pin_tree();
        let found = choose_pin(Some(&tree.0), None).unwrap();
        assert_eq!(found, tree.0.join("OpenSim-Tranquillity"));

        let home =
            std::env::temp_dir().join(format!("ferroan-console-home-{}", uuid::Uuid::new_v4()));
        let documented = home.join("Git").join("reference");
        fs::create_dir_all(&documented).unwrap();
        let home_guard = TempTree(home.clone());
        // Move the pin under the documented ~/Git/reference location.
        let placed = documented.join("OpenSim-Tranquillity");
        fs::rename(tree.0.join("OpenSim-Tranquillity"), &placed).unwrap();
        let found = choose_pin(None, Some(&home)).unwrap();
        assert_eq!(found, placed);
        drop(home_guard);

        let missing = choose_pin(None, None).unwrap_err();
        assert!(missing.contains("FERROAN_REF"), "{missing}");
        assert!(missing.contains("~/Git/reference"), "{missing}");
    }
}
