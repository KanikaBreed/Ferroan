// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Container images and the example grid.
//!
//! `cargo xtask docker build` builds `ferroan-region` and `ferroan-robust`,
//! plus the pinned Tranquillity region and grid images. It does not push.
//! `cargo xtask docker smoke` checks the default user, restarts the asset
//! service against a mounted database, then logs in with `ferroan-bot` and
//! Ferrobot. The grid uses host networking so the harness loopback binds work.
//! Processes run as the invoking user, which is not root on the CI runner or
//! a normal workstation, so mode-0600 configuration stays private. The image
//! default user is checked separately and is also not root.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use crate::live;

#[cfg(target_os = "linux")]
pub(crate) const REGION_IMAGE: &str = "ferroan-region:p0.11";
#[cfg(target_os = "linux")]
pub(crate) const ROBUST_IMAGE: &str = "ferroan-robust:p0.11";
#[cfg(target_os = "linux")]
pub(crate) const CSHARP_REGION_IMAGE: &str = "tranquillity-region:p0.11";
#[cfg(target_os = "linux")]
const CSHARP_REGION_PIN_IMAGE: &str = "tranquillity-region:p0.11-pin";
#[cfg(target_os = "linux")]
pub(crate) const CSHARP_GRID_IMAGE: &str = "tranquillity-grid:p0.11";

#[cfg(target_os = "linux")]
pub(crate) const NAME_HOME: &str = "ferroan-p011-home";
#[cfg(target_os = "linux")]
pub(crate) const NAME_NEIGHBOUR: &str = "ferroan-p011-neighbour";
#[cfg(target_os = "linux")]
pub(crate) const NAME_ROBUST: &str = "ferroan-p011-robust";
#[cfg(target_os = "linux")]
pub(crate) const NAME_ASSETS: &str = "ferroan-p011-assets";
#[cfg(target_os = "linux")]
const NAME_PERSIST: &str = "ferroan-p011-persist";

#[cfg(target_os = "linux")]
const MIN_FREE: u64 = 12 * 1024 * 1024 * 1024;
#[cfg(target_os = "linux")]
const DOCKER: &str = "/usr/bin/docker";

pub fn build() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        finish(build_linux())
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("cargo xtask docker runs on Linux");
        ExitCode::from(1)
    }
}

pub fn smoke() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        finish(smoke_linux())
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("cargo xtask docker runs on Linux");
        ExitCode::from(1)
    }
}

fn finish(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(message) => {
            if !message.is_empty() {
                println!("{message}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("docker: {error}");
            ExitCode::from(1)
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) struct Mount {
    pub host: PathBuf,
    pub container: PathBuf,
    pub read_only: bool,
}

#[cfg(target_os = "linux")]
pub(crate) struct LaunchRequest<'a> {
    pub name: &'a str,
    pub image: &'a str,
    pub entrypoint: Option<&'a str>,
    pub args: &'a [String],
    pub workdir: &'a Path,
    pub log: &'a Path,
    pub pipe_stdin: bool,
    pub detach: bool,
    pub mounts: &'a [Mount],
    pub track: bool,
    pub redact: &'a [&'a str],
}

#[cfg(target_os = "linux")]
pub(crate) struct Attached {
    pub child: Child,
    pub stdin: Option<std::process::ChildStdin>,
    pub name: String,
    pub listen_pid: u32,
    pub tracked: Option<crate::grid_state::Tracked>,
}

#[cfg(target_os = "linux")]
fn build_linux() -> Result<String, String> {
    require_disk()?;
    let root = crate::repo_root();
    build_image(
        &root.join("deploy/docker/ferroan-region.Dockerfile"),
        &root,
        REGION_IMAGE,
    )?;
    // The robust file repeats the same build stage, so this copy is a cache hit.
    build_image(
        &root.join("deploy/docker/ferroan-robust.Dockerfile"),
        &root,
        ROBUST_IMAGE,
    )?;
    let pin = pin_context()?;
    build_image(
        &pin.join("Source/OpenSim.Server.RegionServer/Dockerfile"),
        &pin,
        CSHARP_REGION_PIN_IMAGE,
    )?;
    // libSkiaSharp.so needs libfontconfig1. The pin Dockerfile stays as it is.
    build_image(
        &root.join("deploy/docker/tranquillity-region.runtime.Dockerfile"),
        &root.join("deploy/docker"),
        CSHARP_REGION_IMAGE,
    )?;
    build_image(
        &pin.join("Source/OpenSim.Server.GridServer/Dockerfile"),
        &pin,
        CSHARP_GRID_IMAGE,
    )?;
    Ok(format!(
        "built {REGION_IMAGE}, {ROBUST_IMAGE}, {CSHARP_REGION_IMAGE}, {CSHARP_GRID_IMAGE}"
    ))
}

#[cfg(target_os = "linux")]
fn smoke_linux() -> Result<String, String> {
    let _containers = ContainerGuard;
    if let Some(pid) = live::standalone_is_up()? {
        return Err(format!(
            "the harness is already up (pid {pid}); cargo xtask live down stops an interrupted run"
        ));
    }
    if let Some(pid) = crate::grid_state::survivor()? {
        return Err(format!(
            "grid process {pid} is still running; cargo xtask live down stops an interrupted run"
        ));
    }
    remove_known();
    let built = build_linux()?;
    let users = prove_non_root()?;
    let restart = prove_volume_restart()?;
    let env_note = write_example_env()?;
    let started = Instant::now();
    let login = crate::grid::run_docker(&crate::live_args::RunSpec {
        scenario: "login".to_string(),
        grid: true,
        ferroan_region: true,
        rust_services: vec!["assets".to_string()],
        db: crate::live_args::Database::Sqlite,
    })?;
    let ferrobot = crate::grid::run_docker(&crate::live_args::RunSpec {
        scenario: "ferrobot".to_string(),
        grid: true,
        ferroan_region: true,
        rust_services: vec!["assets".to_string()],
        db: crate::live_args::Database::Sqlite,
    })?;
    let elapsed = started.elapsed().as_secs();
    remove_built_images();
    let _ = Command::new(DOCKER).args(["image", "prune", "-f"]).status();
    Ok(format!(
        "{built}\n{users}\n{restart}\n{env_note}\nlogin:\n{login}\nferrobot:\n{ferrobot}\ndocker smoke login+ferrobot: {elapsed}s"
    ))
}

#[cfg(target_os = "linux")]
struct ContainerGuard;

#[cfg(target_os = "linux")]
impl Drop for ContainerGuard {
    fn drop(&mut self) {
        remove_known();
    }
}

#[cfg(target_os = "linux")]
fn require_disk() -> Result<(), String> {
    let free = free_bytes()?;
    if free < MIN_FREE {
        return Err(format!(
            "less than 12 GiB free ({free} bytes); refusing to build images"
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn free_bytes() -> Result<u64, String> {
    let output = Command::new("df")
        .args(["-B1", "--output=avail", "/"])
        .output()
        .map_err(|error| format!("could not read free disk: {error}"))?;
    if !output.status.success() {
        return Err("could not read free disk".to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .next_back()
        .ok_or_else(|| "df did not report free bytes".to_string())
}

#[cfg(target_os = "linux")]
fn build_image(file: &Path, context: &Path, tag: &str) -> Result<(), String> {
    require_disk()?;
    if !file.is_file() {
        return Err(format!("missing {}", file.display()));
    }
    println!("docker: build {tag}");
    let file_arg = file.display().to_string();
    let context_arg = context.display().to_string();
    let status = Command::new(DOCKER)
        .args(["build", "-f", &file_arg, "-t", tag, &context_arg])
        .status()
        .map_err(|error| format!("could not run docker build: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("docker build failed for {tag}"))
    }
}

#[cfg(target_os = "linux")]
fn prove_non_root() -> Result<String, String> {
    let mut lines = Vec::new();
    for image in [
        REGION_IMAGE,
        ROBUST_IMAGE,
        CSHARP_REGION_IMAGE,
        CSHARP_GRID_IMAGE,
    ] {
        let configured = docker_output(&["image", "inspect", "-f", "{{.Config.User}}", image])?;
        let configured = configured.trim();
        if configured.is_empty() || configured == "root" || configured == "0" || configured == "0:0"
        {
            return Err(format!("{image} default user is {configured:?}"));
        }
        let id = Command::new(DOCKER)
            .args([
                "run",
                "--rm",
                "--network",
                "none",
                "--entrypoint",
                "id",
                image,
                "-u",
            ])
            .output()
            .map_err(|error| format!("could not run id in {image}: {error}"))?;
        if !id.status.success() {
            return Err(format!("{image} did not run id"));
        }
        let uid = String::from_utf8_lossy(&id.stdout);
        let uid = uid.trim();
        if uid.is_empty() || uid == "0" {
            return Err(format!("{image} ran id as {uid:?}"));
        }
        lines.push(format!("{image} user {configured} uid {uid}"));
    }
    Ok(lines.join("\n"))
}

#[cfg(target_os = "linux")]
fn prove_volume_restart() -> Result<String, String> {
    live::ensure_region_built()?;
    // The library loader is a tracked host process. Claim only for that seed,
    // then drop the claim before the restart container and the grid runs.
    let seed = {
        let _claim = crate::grid_state::claim()?;
        crate::assets::seed_library_database()?
    };
    let root = crate::repo_root().join(".scratch/docker/persist");
    fs::create_dir_all(&root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;
    let database = root.join("assets.db");
    fs::copy(&seed, &database)
        .map_err(|error| format!("could not copy the asset database: {error}"))?;
    let empty = root.join("empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_path = root.join("assets.ini");
    let port = live::allocate_port()?;
    let db = database.display();
    let ini = format!(
        "[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n[AssetService]\n    \
         LocalServiceModule = \"OpenSim.Services.AssetService.dll:AssetService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n"
    );
    live::write_secret(&ini_path, &ini)?;
    let logdata = root.join("logdata");
    fs::create_dir_all(&logdata)
        .map_err(|error| format!("could not create {}: {error}", logdata.display()))?;
    let ini_arg = ini_path.display().to_string();
    let empty_arg = empty.display().to_string();
    let args = vec![
        "--inimaster".to_string(),
        ini_arg,
        "--inidirectory".to_string(),
        empty_arg,
    ];
    let mounts = vec![
        Mount {
            host: root.clone(),
            container: root.clone(),
            read_only: false,
        },
        Mount {
            host: logdata,
            container: PathBuf::from("/app/logdata"),
            read_only: false,
        },
    ];
    let log = root.join("persist.log");
    let _started = launch(LaunchRequest {
        name: NAME_PERSIST,
        image: ROBUST_IMAGE,
        entrypoint: None,
        args: &args,
        workdir: &root,
        log: &log,
        pipe_stdin: false,
        detach: true,
        mounts: &mounts,
        track: false,
        redact: &[],
    })?;
    wait_log_count(NAME_PERSIST, "asset service", 1, Duration::from_secs(60))?;
    if texture_visible(&database).is_err() {
        remove_container(NAME_PERSIST);
        return Err(
            "the mounted asset database lost the library texture before restart".to_string(),
        );
    }
    let status = Command::new(DOCKER)
        .args(["restart", NAME_PERSIST])
        .status()
        .map_err(|error| format!("could not restart the asset container: {error}"))?;
    if !status.success() {
        remove_container(NAME_PERSIST);
        return Err("docker restart failed for the asset container".to_string());
    }
    wait_log_count(NAME_PERSIST, "asset service", 2, Duration::from_secs(60))?;
    if texture_visible(&database).is_err() {
        remove_container(NAME_PERSIST);
        return Err(
            "the mounted asset database lost the library texture after restart".to_string(),
        );
    }
    remove_container(NAME_PERSIST);
    Ok("restart kept the library texture in the mounted asset database".to_string())
}

#[cfg(target_os = "linux")]
fn texture_visible(database: &Path) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if crate::assets::library_texture_stored(database)? {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(10) {
            return Err("library texture is not in the asset database".to_string());
        }
        thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(target_os = "linux")]
fn wait_log_count(name: &str, needle: &str, want: usize, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let text = docker_output(&["logs", name]).unwrap_or_default();
        if text.matches(needle).count() >= want {
            return Ok(());
        }
        if started.elapsed() > timeout {
            let mut start = text.len().saturating_sub(2000);
            while !text.is_char_boundary(start) {
                start += 1;
            }
            return Err(format!(
                "{name} did not log {needle} {want} times\n{}",
                &text[start..]
            ));
        }
        thread::sleep(Duration::from_millis(300));
    }
}

#[cfg(target_os = "linux")]
fn write_example_env() -> Result<String, String> {
    let estate = live::generate_password()?;
    let chat = live::generate_password()?;
    let uid = command_trim(&["id", "-u"])?;
    let gid = command_trim(&["id", "-g"])?;
    let body = format!(
        "RUN_UID={uid}\nRUN_GID={gid}\nCONFIG_DIR=\nDATA_DIR=\nLOG_DIR=\nESTATE_PASSWORD={estate}\nCHAT_PASSWORD={chat}\n"
    );
    let path = crate::repo_root().join("deploy/compose/.env");
    live::write_secret(&path, &body)?;
    Ok("wrote deploy/compose/.env (gitignored)".to_string())
}

#[cfg(target_os = "linux")]
fn command_trim(args: &[&str]) -> Result<String, String> {
    let output = Command::new(args[0])
        .args(&args[1..])
        .output()
        .map_err(|error| format!("could not run {}: {error}", args[0]))?;
    if !output.status.success() {
        return Err(format!("{} failed", args[0]));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.trim().to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn launch(request: LaunchRequest<'_>) -> Result<Attached, String> {
    require_known_name(request.name)?;
    remove_container(request.name);
    if let Some(parent) = request.log.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let workdir = request
        .workdir
        .canonicalize()
        .map_err(|error| format!("could not resolve {}: {error}", request.workdir.display()))?;
    let user = run_as()?;
    let mut args = vec![
        "run".to_string(),
        "--name".to_string(),
        request.name.to_string(),
        "--network".to_string(),
        "host".to_string(),
        "--user".to_string(),
        user,
        "-e".to_string(),
        format!("HOME={}", workdir.display()),
        "-e".to_string(),
        "LOGDIR=/app/logdata".to_string(),
        "-e".to_string(),
        format!("DOTNET_CLI_HOME={}", workdir.display()),
        "-e".to_string(),
        "COMPlus_EnableDiagnostics=0".to_string(),
        "-w".to_string(),
        workdir.display().to_string(),
    ];
    if request.detach {
        args.push("-d".to_string());
    } else {
        args.push("--rm".to_string());
    }
    if request.pipe_stdin {
        args.push("-i".to_string());
    }
    for mount in request.mounts {
        fs::create_dir_all(&mount.host)
            .map_err(|error| format!("could not create {}: {error}", mount.host.display()))?;
        let host = mount
            .host
            .canonicalize()
            .map_err(|error| format!("could not resolve {}: {error}", mount.host.display()))?;
        args.push("-v".to_string());
        let mut spec = format!("{}:{}", host.display(), mount.container.display());
        if mount.read_only {
            spec.push_str(":ro");
        }
        args.push(spec);
    }
    if let Some(entrypoint) = request.entrypoint {
        args.push("--entrypoint".to_string());
        args.push(entrypoint.to_string());
    }
    args.push(request.image.to_string());
    args.extend(request.args.iter().cloned());
    let log_file = fs::File::create(request.log)
        .map_err(|error| format!("could not create {}: {error}", request.log.display()))?;
    let err_file = log_file
        .try_clone()
        .map_err(|error| format!("could not open {}: {error}", request.log.display()))?;
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = Command::new(DOCKER);
    command
        .args(&arg_refs)
        .current_dir(&workdir)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(err_file));
    if request.pipe_stdin {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    command.process_group(0);
    let (child, tracked) = if request.track {
        let launch = crate::grid_state::prepare_spawn(Path::new(DOCKER), &arg_refs, &workdir)?;
        launch.configure(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                launch.cancel()?;
                return Err(format!("could not start docker: {error}"));
            }
        };
        match launch.record(&child) {
            Ok(tracked) => (child, Some(tracked)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                remove_container(request.name);
                return Err(error);
            }
        }
    } else {
        let child = command
            .spawn()
            .map_err(|error| format!("could not start docker: {error}"))?;
        (child, None)
    };
    let mut child = child;
    let stdin = child.stdin.take();
    if request.detach {
        let status = child
            .wait()
            .map_err(|error| format!("could not wait for docker: {error}"))?;
        if !status.success() {
            remove_container(request.name);
            return Err(format!(
                "docker run failed for {}\n{}",
                request.name,
                live::log_tail_secrets(request.log, request.redact)
            ));
        }
    }
    let listen_pid = match container_pid(request.name) {
        Ok(pid) => pid,
        Err(error) => {
            remove_container(request.name);
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{error}\n{}",
                live::log_tail_secrets(request.log, request.redact)
            ));
        }
    };
    Ok(Attached {
        child,
        stdin,
        name: request.name.to_string(),
        listen_pid,
        tracked,
    })
}

#[cfg(target_os = "linux")]
fn run_as() -> Result<String, String> {
    let uid = command_trim(&["id", "-u"])?;
    let gid = command_trim(&["id", "-g"])?;
    if uid == "0" {
        return Err("refusing to run the grid as root".to_string());
    }
    Ok(format!("{uid}:{gid}"))
}

#[cfg(target_os = "linux")]
fn container_pid(name: &str) -> Result<u32, String> {
    let started = Instant::now();
    loop {
        if let Ok(text) = docker_output(&["inspect", "-f", "{{.State.Pid}}", name])
            && let Ok(pid) = text.trim().parse::<u32>()
            && pid > 0
        {
            return Ok(pid);
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err(format!("{name} has no host pid"));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(target_os = "linux")]
fn docker_output(args: &[&str]) -> Result<String, String> {
    let output = Command::new(DOCKER)
        .args(args)
        .output()
        .map_err(|error| format!("could not run docker: {error}"))?;
    if !output.status.success() {
        return Err("docker command failed".to_string());
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

#[cfg(target_os = "linux")]
pub(crate) fn remove_container(name: &str) {
    if require_known_name(name).is_err() {
        return;
    }
    let _ = Command::new(DOCKER).args(["rm", "-f", name]).output();
}

/// SIGTERM and wait, as the host harness does, so `ferroan-region` can
/// deregister before the proxy capture is read. `docker rm -f` would kill it first.
#[cfg(target_os = "linux")]
pub(crate) fn stop_container(name: &str) {
    if require_known_name(name).is_err() {
        return;
    }
    let _ = Command::new(DOCKER)
        .args(["stop", "--time", "30", name])
        .output();
    remove_container(name);
}

#[cfg(target_os = "linux")]
fn remove_known() {
    for name in [
        NAME_HOME,
        NAME_NEIGHBOUR,
        NAME_ROBUST,
        NAME_ASSETS,
        NAME_PERSIST,
    ] {
        remove_container(name);
    }
}

#[cfg(target_os = "linux")]
fn remove_built_images() {
    for tag in [
        REGION_IMAGE,
        ROBUST_IMAGE,
        CSHARP_REGION_PIN_IMAGE,
        CSHARP_REGION_IMAGE,
        CSHARP_GRID_IMAGE,
    ] {
        let _ = Command::new(DOCKER).args(["rmi", tag]).output();
    }
}

#[cfg(target_os = "linux")]
fn require_known_name(name: &str) -> Result<(), String> {
    if matches!(
        name,
        NAME_HOME | NAME_NEIGHBOUR | NAME_ROBUST | NAME_ASSETS | NAME_PERSIST
    ) {
        Ok(())
    } else {
        Err(format!("refusing to touch unexpected container {name}"))
    }
}

#[cfg(target_os = "linux")]
fn pin_context() -> Result<PathBuf, String> {
    let pin = read_pin()?;
    if let Some(checkout) = reference_checkout()
        && clean_at(&checkout, &pin).is_ok()
    {
        return Ok(checkout);
    }
    let dest = crate::repo_root().join(".scratch/docker/OpenSim-Tranquillity");
    require_scratch(&dest)?;
    if dest.join(".git").exists() && clean_at(&dest, &pin).is_ok() {
        return Ok(dest);
    }
    if dest.exists() {
        fs::remove_dir_all(&dest)
            .map_err(|error| format!("could not remove {}: {error}", dest.display()))?;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let url = repository_url()?;
    let dest_arg = dest.display().to_string();
    let status = Command::new("git")
        .args(["clone", &url, &dest_arg])
        .status()
        .map_err(|error| format!("could not clone Tranquillity: {error}"))?;
    if !status.success() {
        return Err("could not clone the pinned Tranquillity checkout".to_string());
    }
    let status = Command::new("git")
        .args(["-C", &dest_arg, "checkout", "--detach", &pin])
        .status()
        .map_err(|error| format!("could not check out the pin: {error}"))?;
    if !status.success() {
        return Err("could not check out the pinned Tranquillity revision".to_string());
    }
    Ok(dest)
}

#[cfg(target_os = "linux")]
fn require_scratch(path: &Path) -> Result<(), String> {
    let scratch = crate::repo_root().join(".scratch");
    if path.starts_with(&scratch) {
        return Ok(());
    }
    let root = crate::repo_root()
        .canonicalize()
        .map_err(|error| format!("could not resolve the worktree: {error}"))?;
    if path.starts_with(root.join(".scratch")) {
        Ok(())
    } else {
        Err("refusing to replace a checkout outside .scratch".to_string())
    }
}

#[cfg(target_os = "linux")]
fn reference_checkout() -> Option<PathBuf> {
    let root = match std::env::var("FERROAN_REF") {
        Ok(value) if !value.is_empty() => expand_user(&value),
        _ => home_dir()?.join("Git/reference"),
    };
    let checkout = root.join("OpenSim-Tranquillity");
    if checkout.join(".git").exists() || checkout.join("Source").is_dir() {
        Some(checkout)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

#[cfg(target_os = "linux")]
fn expand_user(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

#[cfg(target_os = "linux")]
fn clean_at(path: &Path, pin: &str) -> Result<(), String> {
    let head = git(path, &["rev-parse", "HEAD"])?;
    if head != pin {
        return Err(format!("{head} is not the pinned revision"));
    }
    let status = git(path, &["status", "--porcelain"])?;
    if status.is_empty() {
        Ok(())
    } else {
        Err("checkout is not clean".to_string())
    }
}

#[cfg(target_os = "linux")]
fn git(path: &Path, args: &[&str]) -> Result<String, String> {
    let cwd = path.display().to_string();
    let mut command = vec!["-C", cwd.as_str()];
    command.extend_from_slice(args);
    let output = Command::new("git")
        .args(&command)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        return Err("git failed".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "linux")]
fn read_pin() -> Result<String, String> {
    let path = crate::repo_root().join("docs/REFERENCES.md");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    pinned_tranquillity(&text)
}

#[cfg(any(target_os = "linux", test))]
fn pinned_tranquillity(markdown: &str) -> Result<String, String> {
    let marker = "| `OpenSim-Tranquillity`";
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(marker) {
            continue;
        }
        let bytes = trimmed.as_bytes();
        let mut index = 0;
        while index + 42 <= bytes.len() {
            if bytes[index] == b'`' && bytes[index + 41] == b'`' {
                let hash = &trimmed[index + 1..index + 41];
                if hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Ok(hash.to_ascii_lowercase());
                }
            }
            index += 1;
        }
        return Err("OpenSim-Tranquillity row has no 40-character revision".to_string());
    }
    Err("docs/REFERENCES.md has no OpenSim-Tranquillity row".to_string())
}

#[cfg(target_os = "linux")]
fn repository_url() -> Result<String, String> {
    let path = crate::repo_root().join("docs/REFERENCES.md");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    tranquillity_url(&text)
}

#[cfg(any(target_os = "linux", test))]
fn tranquillity_url(markdown: &str) -> Result<String, String> {
    let marker = "| `OpenSim-Tranquillity`";
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(marker) {
            continue;
        }
        let Some(start) = trimmed.find("https://github.com/") else {
            return Err("OpenSim-Tranquillity row has no GitHub URL".to_string());
        };
        let rest = &trimmed[start..];
        let end = rest.find(')').unwrap_or(rest.len());
        let mut url = rest[..end].trim().to_string();
        if !url.ends_with(".git") {
            url.push_str(".git");
        }
        return Ok(url);
    }
    Err("docs/REFERENCES.md has no OpenSim-Tranquillity row".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGION_DOCKERFILE: &str = include_str!("../../deploy/docker/ferroan-region.Dockerfile");
    const ROBUST_DOCKERFILE: &str = include_str!("../../deploy/docker/ferroan-robust.Dockerfile");
    const COMPOSE: &str = include_str!("../../deploy/compose/compose.yaml");
    const ENV_EXAMPLE: &str = include_str!("../../deploy/compose/.env.example");
    const REGION_RUNTIME: &str =
        include_str!("../../deploy/docker/tranquillity-region.runtime.Dockerfile");

    #[test]
    fn images_share_one_release_build() {
        let region_build = REGION_DOCKERFILE
            .split("FROM debian:bookworm-slim")
            .next()
            .unwrap_or("");
        let robust_build = ROBUST_DOCKERFILE
            .split("FROM debian:bookworm-slim")
            .next()
            .unwrap_or("");
        assert_eq!(region_build, robust_build);
        assert!(region_build.contains("cargo build --locked --release"));
        assert!(region_build.contains("FROM rust:1.100.0-bookworm AS build"));
        for text in [REGION_DOCKERFILE, ROBUST_DOCKERFILE] {
            assert!(text.contains("USER app"));
            assert!(text.contains("/app/config"));
            assert!(text.contains("/app/data"));
            assert!(text.contains("/app/logdata"));
            assert!(text.contains("--uid 10001"));
        }
        assert!(REGION_DOCKERFILE.contains("/app/ferroan-region"));
        assert!(ROBUST_DOCKERFILE.contains("/app/ferroan-robust"));
    }

    #[test]
    fn compose_names_the_grid_without_passwords() {
        for name in [
            "robust:",
            "neighbour:",
            "ferroan-robust:",
            "ferroan-region:",
        ] {
            assert!(COMPOSE.contains(name), "{name}");
        }
        assert!(COMPOSE.contains("network_mode: host"));
        assert!(COMPOSE.contains("tranquillity-grid:p0.11"));
        assert!(COMPOSE.contains("tranquillity-region:p0.11"));
        assert!(COMPOSE.contains("ferroan-robust:p0.11"));
        assert!(COMPOSE.contains("ferroan-region:p0.11"));
        assert!(!COMPOSE.to_ascii_lowercase().contains("password"));
        for line in ENV_EXAMPLE.lines() {
            if let Some((_, value)) = line.split_once("PASSWORD") {
                assert!(value.trim_start_matches('=').trim().is_empty(), "{line}");
            }
        }
    }

    #[test]
    fn region_runtime_adds_fontconfig_and_keeps_the_app_user() {
        assert!(REGION_RUNTIME.contains("FROM tranquillity-region:p0.11-pin"));
        assert!(REGION_RUNTIME.contains("libfontconfig1"));
        assert!(REGION_RUNTIME.trim_end().ends_with("USER app"));
        assert!(!REGION_RUNTIME.to_ascii_lowercase().contains("password"));
    }

    #[test]
    fn references_name_the_tranquillity_pin() {
        let text = include_str!("../../docs/REFERENCES.md");
        let pin = pinned_tranquillity(text).unwrap_or_default();
        assert_eq!(pin.len(), 40);
        assert!(pin.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let url = tranquillity_url(text).unwrap_or_default();
        assert!(url.starts_with("https://github.com/"));
        assert!(url.ends_with(".git"));
    }
}
