// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build the pinned `OpenSim.ConsoleClient` and point it at a Ferroan console.
//!
//! The step is not part of `cargo xtask check`. It skips, with exit 0, where
//! `dotnet` or the pin is absent. Credentials are synthetic and are not printed.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
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
    let Some(pin) = locate_pin() else {
        return Ok("console-client: skipped (the Tranquillity pin is absent)".to_string());
    };
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
    let mut child = Command::new(dotnet)
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
    // The login callback is asynchronous, so the command is sent only after a
    // session prompt. The old FromXml error still ends the wait, and judge
    // then fails.
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
        if !sent && (text.contains("+++") || text.contains("Ferroan#") || text.contains("Region ("))
        {
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
    let intermediate = format!("IntermediateOutputPath={}/", obj.display());
    let output = Command::new(dotnet)
        .arg("build")
        .arg(&project)
        .arg("-c")
        .arg("Release")
        .arg("-o")
        .arg(&bin)
        .arg("-p:UseSharedCompilation=false")
        .arg(format!("-p:{intermediate}"))
        .output()
        .map_err(|error| format!("could not run dotnet build: {error}"))?;
    if !output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        return Err(format!("dotnet build failed: {}", scrub(&text)));
    }
    let dll = bin.join("OpenSim.ConsoleClient.dll");
    if dll.is_file() {
        Ok(dll)
    } else {
        Err("dotnet build did not write OpenSim.ConsoleClient.dll".to_string())
    }
}

fn locate_pin() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("FERROAN_REF") {
        candidates.push(PathBuf::from(root).join("OpenSim-Tranquillity"));
    }
    candidates.push(PathBuf::from(
        "/home/cinder/Git/reference/OpenSim-Tranquillity",
    ));
    candidates.into_iter().find(|path| {
        path.join("Source/OpenSim.ConsoleClient/OpenSim.ConsoleClient.csproj")
            .is_file()
    })
}

fn which_dotnet() -> Option<PathBuf> {
    let output = Command::new("dotnet").arg("--version").output().ok()?;
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
