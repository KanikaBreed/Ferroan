// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Container images and the example grid.
//!
//! `cargo xtask docker build` builds `ferroan-region` and `ferroan-robust`,
//! plus the pinned Tranquillity region and grid images. It does not push.
//! `cargo xtask docker smoke` checks the default user, restarts the asset
//! service against a mounted database, then logs in with `ferroan-bot` and
//! Ferrobot. The grid uses host networking so the harness loopback binds work.
//! Smoke creates no Docker network and no named volume. Cleanup removes the
//! known containers and prunes build cache older than seven days.
//! On a rootful daemon the process runs as the invoking user, which is not
//! root, so mode-0600 configuration stays private. On a rootless daemon that
//! host uid is a subordinate id and cannot read those files; container uid 0
//! is the host user, so the process uses that. Before the first grid launch,
//! a rootless smoke checks that container uid 0 creates a scratch file owned
//! by the invoking user. The image default user is checked separately and is
//! also not root. Detached container logs are streamed under
//! `.scratch/live/docker-logs` before removal, at most 64 MiB. Attached
//! `--rm` containers are already gone by then; their output is in the harness
//! log under `.scratch/live/grid`.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(any(target_os = "linux", test))]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::io::{Seek, SeekFrom};
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
#[cfg(any(target_os = "linux", test))]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(any(target_os = "linux", test))]
use std::sync::Mutex;
#[cfg(any(target_os = "linux", test))]
use std::sync::atomic::{AtomicU64, Ordering};
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
pub(crate) const NAME_LOKI: &str = "ferroan-p011-loki";
#[cfg(target_os = "linux")]
pub(crate) const NAME_OTELCOL: &str = "ferroan-p011-otelcol";
#[cfg(target_os = "linux")]
const LOKI_IMAGE: &str = "grafana/loki:3.2.1";
#[cfg(target_os = "linux")]
const OTELCOL_IMAGE: &str = "otel/opentelemetry-collector-contrib:0.128.0";

#[cfg(target_os = "linux")]
struct SmokeFlags {
    otlp: Option<String>,
    labels: bool,
}

#[cfg(target_os = "linux")]
static SMOKE: std::sync::Mutex<SmokeFlags> = std::sync::Mutex::new(SmokeFlags {
    otlp: None,
    labels: false,
});

#[cfg(target_os = "linux")]
pub(crate) fn set_smoke(otlp: Option<String>, labels: bool) {
    let mut flags = SMOKE.lock().unwrap_or_else(|poison| poison.into_inner());
    flags.otlp = otlp;
    flags.labels = labels;
}

#[cfg(target_os = "linux")]
pub(crate) fn smoke_otlp() -> Option<String> {
    SMOKE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .otlp
        .clone()
}

#[cfg(target_os = "linux")]
fn smoke_labels() -> bool {
    SMOKE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .labels
}

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

// Called from the Linux branches of `build` and `smoke`. Windows and macOS
// take the other branch, so this stays out of those builds.
#[cfg(target_os = "linux")]
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
    discard_known();
    let built = build_linux()?;
    let users = format!("{}\n{}", prove_non_root()?, prove_rootless_owner()?);
    let restart = prove_volume_restart()?;
    let env_note = write_example_env()?;
    let collectors = start_collectors()?;
    let otel_http = crate::ports::assigned(crate::ports::Slot::OtelHttp)
        .ok_or_else(|| "the collector HTTP port was not allocated".to_string())?;
    set_smoke(Some(format!("http://127.0.0.1:{otel_http}")), true);
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
        "{built}\n{users}\n{restart}\n{env_note}\n{collectors}\nlogin:\n{login}\nferrobot:\n{ferrobot}\ndocker smoke login+ferrobot: {elapsed}s"
    ))
}

#[cfg(target_os = "linux")]
struct ContainerGuard;

#[cfg(target_os = "linux")]
impl Drop for ContainerGuard {
    fn drop(&mut self) {
        set_smoke(None, false);
        remove_known();
        // Ignore a missing builder. A successful smoke must still finish.
        let _ = Command::new(DOCKER)
            .args(builder_cache_prune_args())
            .status();
    }
}

/// Rootful: the host uid, which owns the mode-0600 mounts. Rootless: container
/// uid 0, which is that same host user. Passing the host uid into a rootless
/// container selects a subordinate uid that cannot read the mounts.
#[cfg(any(target_os = "linux", test))]
fn container_user(host_uid: &str, host_gid: &str, rootless: bool) -> Result<String, String> {
    if host_uid == "0" {
        return Err("refusing to run the grid as root".to_string());
    }
    if host_uid.is_empty() || host_gid.is_empty() {
        return Err("the host user id is empty".to_string());
    }
    if rootless {
        Ok("0:0".to_string())
    } else {
        Ok(format!("{host_uid}:{host_gid}"))
    }
}

#[cfg(any(target_os = "linux", test))]
fn security_options_are_rootless(text: &str) -> bool {
    text.split(|ch: char| {
        ch.is_ascii_whitespace() || matches!(ch, ',' | '[' | ']' | '"' | '\'' | '{' | '}')
    })
    .any(|part| part == "name=rootless")
}

/// Rootful smoke does not run the probe. Rootless smoke refuses unless
/// container uid 0 created the scratch file as the invoking user. A daemon
/// that reports `name=rootless` but writes as another account fails here.
#[cfg(any(target_os = "linux", test))]
fn probe_owner_result(
    rootless: bool,
    host_uid: &str,
    file_uid: Option<u32>,
) -> Result<String, String> {
    if !rootless {
        return Ok(format!("rootful daemon: grid user is host uid {host_uid}"));
    }
    let Some(file_uid) = file_uid else {
        return Err("rootless owner check could not read the probe file".to_string());
    };
    if host_uid == file_uid.to_string() {
        Ok(format!(
            "rootless owner check: container uid 0 wrote the probe file as host uid {host_uid}"
        ))
    } else {
        Err(format!(
            "refusing rootless smoke: container uid 0 created the probe file as uid {file_uid}, not host uid {host_uid}"
        ))
    }
}

#[cfg(any(target_os = "linux", test))]
struct ContainerSnapshot {
    status: String,
    exit_code: String,
    pid: u32,
    oom: bool,
    error: String,
}

#[cfg(any(target_os = "linux", test))]
fn parse_container_snapshot(text: &str) -> Option<ContainerSnapshot> {
    let mut lines = text.lines();
    let status = lines.next()?.trim().to_string();
    let exit_code = lines.next()?.trim().to_string();
    let pid = lines.next()?.trim().parse::<u32>().ok()?;
    let oom = lines.next()?.trim() == "true";
    let error = lines.collect::<Vec<_>>().join("\n");
    let error = error.trim().to_string();
    if status.is_empty() {
        return None;
    }
    Some(ContainerSnapshot {
        status,
        exit_code,
        pid,
        oom,
        error,
    })
}

/// A running container has a host pid. An exited one will never gain one.
#[cfg(any(target_os = "linux", test))]
fn pid_is_ready(snapshot: &ContainerSnapshot) -> bool {
    snapshot.pid > 0 && !container_has_exited(snapshot)
}

#[cfg(any(target_os = "linux", test))]
fn container_has_exited(snapshot: &ContainerSnapshot) -> bool {
    matches!(snapshot.status.as_str(), "exited" | "dead" | "removing")
}

#[cfg(any(target_os = "linux", test))]
fn last_log_lines(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() > 40 {
        lines = lines.split_off(lines.len() - 40);
    }
    let joined = lines.join("\n");
    let mut start = joined.len().saturating_sub(4000);
    while !joined.is_char_boundary(start) {
        start += 1;
    }
    joined[start..].to_string()
}

#[cfg(any(target_os = "linux", test))]
fn missing_pid_report(name: &str, snapshot: Option<&ContainerSnapshot>, logs: &str) -> String {
    let head = match snapshot {
        Some(snapshot) => {
            let mut line = format!(
                "{name} has no host pid (status {}, exit {})",
                snapshot.status, snapshot.exit_code
            );
            if snapshot.oom {
                line.push_str(", killed by the out-of-memory killer");
            }
            if !snapshot.error.is_empty() {
                line.push_str(": ");
                line.push_str(&snapshot.error);
            }
            line
        }
        None => format!("{name} has no host pid"),
    };
    let tail = last_log_lines(logs);
    if tail.is_empty() {
        head
    } else {
        format!("{head}\n{tail}")
    }
}

#[cfg(any(target_os = "linux", test))]
fn docker_log_file(live_root: &Path, name: &str) -> PathBuf {
    live_root.join("docker-logs").join(format!("{name}.log"))
}

/// Same directory, with `.partial` appended, so a failed `docker logs` can be
/// deleted without replacing a log that was already saved.
#[cfg(any(target_os = "linux", test))]
fn partial_log_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    let mut partial = name.to_os_string();
    partial.push(".partial");
    Some(path.with_file_name(partial))
}

/// A failed `docker logs` means the container is already gone. Keep the file
/// from the remove that found it. Do not replace that file with the error.
#[cfg(any(target_os = "linux", test))]
fn retained_container_log(status_ok: bool, text: &str) -> Option<&str> {
    if status_ok { Some(text) } else { None }
}

/// `docker logs` can be large. Stop writing at this bound and keep reading so
/// the pipe does not fill. Same ceiling as `log_tail_secrets`.
#[cfg(any(target_os = "linux", test))]
const DOCKER_LOG_LIMIT: u64 = 64 * 1024 * 1024;

#[cfg(any(target_os = "linux", test))]
const DOCKER_ERROR_CHARS: usize = 400;

#[cfg(any(target_os = "linux", test))]
fn first_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(any(target_os = "linux", test))]
fn docker_failure_text(stderr: &str) -> String {
    let head = first_chars(stderr, DOCKER_ERROR_CHARS);
    if head.is_empty() {
        "docker command failed".to_string()
    } else {
        format!("docker command failed: {head}")
    }
}

/// Copy until `limit` bytes have been written, then drain the rest.
#[cfg(any(target_os = "linux", test))]
fn copy_bounded(
    mut reader: impl Read,
    file: &Mutex<impl Write>,
    written: &AtomicU64,
    limit: u64,
) -> bool {
    let mut ok = true;
    let mut buf = [0u8; 8192];
    loop {
        let read = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                ok = false;
                break;
            }
        };
        let so_far = written.load(Ordering::Relaxed);
        if so_far < limit {
            let room = (limit - so_far) as usize;
            let take = read.min(room);
            if take > 0 {
                let mut guard = file.lock().unwrap_or_else(|poison| poison.into_inner());
                if guard.write_all(&buf[..take]).is_err() {
                    ok = false;
                } else {
                    written.fetch_add(take as u64, Ordering::Relaxed);
                }
            }
        }
    }
    ok
}

/// Unused BuildKit cache older than seven days. Recent cache stays, which is
/// the warm build on the self-hosted runner. Smoke does not create a Docker
/// network or a named volume, so this does not prune those.
#[cfg(any(target_os = "linux", test))]
fn builder_cache_prune_args() -> &'static [&'static str] {
    &[
        "builder",
        "prune",
        "--all",
        "--force",
        "--filter",
        "until=168h",
    ]
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

/// `name=rootless` alone is not enough: another account's daemon, reached
/// through `DOCKER_HOST`, also reports that token. Container uid 0 must create
/// a scratch file owned by this process. The probe does not run on a rootful
/// daemon, and it does not print the scratch path.
#[cfg(target_os = "linux")]
fn prove_rootless_owner() -> Result<String, String> {
    let host_uid = command_trim(&["id", "-u"])?;
    let user = run_as()?;
    if user != "0:0" {
        return probe_owner_result(false, &host_uid, None);
    }
    let dir = container_log_root().join("owner-probe");
    fs::create_dir_all(&dir)
        .map_err(|_| "rootless owner check could not create the probe directory".to_string())?;
    let probe = dir.join("owner");
    let _ = fs::remove_file(&probe);
    let mount = format!("{}:/probe", dir.display());
    let output = Command::new(DOCKER)
        .args([
            "run",
            "--rm",
            "--network",
            "none",
            "--user",
            "0:0",
            "-v",
            &mount,
            "--entrypoint",
            "/bin/sh",
            REGION_IMAGE,
            "-c",
            "touch /probe/owner",
        ])
        .output()
        .map_err(|_| "rootless owner check could not run the probe container".to_string())?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let head =
            first_chars(&err, DOCKER_ERROR_CHARS).replace(&dir.display().to_string(), "probe-dir");
        let _ = fs::remove_file(&probe);
        return Err(format!("rootless owner check failed: {head}"));
    }
    let file_uid = fs::metadata(&probe).ok().map(|meta| meta.uid());
    let _ = fs::remove_file(&probe);
    probe_owner_result(true, &host_uid, file_uid)
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
    let root = if crate::ports::isolated() {
        crate::ports::run_root().join("docker-persist")
    } else {
        crate::repo_root().join(".scratch/docker/persist")
    };
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
    let persist = runtime_name(NAME_PERSIST);
    let status = Command::new(DOCKER)
        .args(["restart", &persist])
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
    let name = runtime_name(name);
    let started = Instant::now();
    loop {
        let text = docker_output(&["logs", &name]).unwrap_or_default();
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
    // The same user `docker run --user` uses, so a later `docker compose up`
    // can read the mode-0600 files on a rootless daemon too.
    let user = run_as()?;
    let (uid, gid) = user
        .split_once(':')
        .ok_or_else(|| "the container user is not uid:gid".to_string())?;
    let body = format!(
        "RUN_UID={uid}\nRUN_GID={gid}\nCONFIG_DIR=\nDATA_DIR=\nLOG_DIR=\nESTATE_PASSWORD={estate}\nCHAT_PASSWORD={chat}\n"
    );
    let path = if crate::ports::isolated() {
        crate::ports::run_root().join("compose.env")
    } else {
        crate::repo_root().join("deploy/compose/.env")
    };
    live::write_secret(&path, &body)?;
    if crate::ports::isolated() {
        Ok(format!("wrote {} (gitignored)", path.display()))
    } else {
        Ok("wrote deploy/compose/.env (gitignored)".to_string())
    }
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
    let name = runtime_name(request.name);
    require_known_name(&name)?;
    remove_container(&name);
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
        name.clone(),
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
    if smoke_labels() && request.name == NAME_HOME {
        let compose_dir = crate::repo_root().join("deploy/compose");
        let compose_file = compose_dir.join("compose.yaml");
        // `docker compose logs` lists containers that have the project label
        // and the config-hash label. The hash value is not compared.
        let labels = [
            (
                "com.docker.compose.project",
                crate::ports::compose_project(crate::ports::current()),
            ),
            ("com.docker.compose.service", "ferroan-region".to_string()),
            ("com.docker.compose.container-number", "1".to_string()),
            ("com.docker.compose.oneoff", "False".to_string()),
            ("com.docker.compose.config-hash", "smoke".to_string()),
            (
                "com.docker.compose.project.working_dir",
                compose_dir.display().to_string(),
            ),
            (
                "com.docker.compose.project.config_files",
                compose_file.display().to_string(),
            ),
        ];
        for (key, value) in labels {
            args.push("--label".to_string());
            args.push(format!("{key}={value}"));
        }
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
            remove_container(&name);
            return Err(format!(
                "docker run failed for {name}\n{}",
                live::log_tail_secrets(request.log, request.redact)
            ));
        }
    }
    let listen_pid = match container_pid(&name) {
        Ok(pid) => pid,
        Err(error) => {
            let error = live::redact_secrets(&error, request.redact);
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
        name,
        listen_pid,
        tracked,
    })
}

#[cfg(target_os = "linux")]
fn run_as() -> Result<String, String> {
    let uid = command_trim(&["id", "-u"])?;
    let gid = command_trim(&["id", "-g"])?;
    let options = docker_stdout(&["info", "--format", "{{.SecurityOptions}}"])?;
    container_user(&uid, &gid, security_options_are_rootless(&options))
}

#[cfg(target_os = "linux")]
fn container_pid(name: &str) -> Result<u32, String> {
    let name = runtime_name(name);
    let started = Instant::now();
    let mut latest = None;
    loop {
        if let Ok(text) = docker_stdout(&[
            "inspect",
            "-f",
            "{{.State.Status}}\n{{.State.ExitCode}}\n{{.State.Pid}}\n{{.State.OOMKilled}}\n{{.State.Error}}",
            &name,
        ]) && let Some(snapshot) = parse_container_snapshot(&text)
        {
            if container_has_exited(&snapshot) {
                return Err(pid_failure(&name, Some(&snapshot)));
            }
            if pid_is_ready(&snapshot) {
                return Ok(snapshot.pid);
            }
            latest = Some(snapshot);
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err(pid_failure(&name, latest.as_ref()));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(target_os = "linux")]
fn pid_failure(name: &str, snapshot: Option<&ContainerSnapshot>) -> String {
    let logs = if save_container_log(name) {
        container_log_text(name)
    } else {
        // `docker logs` failed, usually because an attached `--rm` container
        // is already gone. Do not append the daemon error. That container's
        // output is in its harness log under `.scratch/live/grid`.
        String::new()
    };
    missing_pid_report(name, snapshot, &logs)
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
fn docker_stdout(args: &[&str]) -> Result<String, String> {
    let output = Command::new(DOCKER)
        .args(args)
        .output()
        .map_err(|error| format!("could not run docker: {error}"))?;
    if !output.status.success() {
        return Err(docker_failure_text(&String::from_utf8_lossy(
            &output.stderr,
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Tail of the log `save_container_log` wrote. Empty when that save failed,
/// so a missing container does not contribute `docker logs` error text.
#[cfg(target_os = "linux")]
fn container_log_text(name: &str) -> String {
    let path = docker_log_file(&container_log_root(), name);
    let Ok(mut file) = fs::File::open(&path) else {
        return String::new();
    };
    let Ok(len) = file.metadata().map(|meta| meta.len()) else {
        return String::new();
    };
    let start = len.saturating_sub(8192);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    if file.take(8192).read_to_end(&mut buf).is_err() {
        return String::new();
    }
    last_log_lines(&String::from_utf8_lossy(&buf))
}

#[cfg(target_os = "linux")]
fn container_log_root() -> PathBuf {
    if crate::ports::isolated() {
        crate::ports::run_root()
    } else {
        crate::live::live_root()
    }
}

/// Stream `docker logs` into the artifact. A non-zero exit leaves any earlier
/// file in place: the partial copy, which would be the daemon's "No such
/// container" text, is deleted. Returns whether a log was saved.
#[cfg(target_os = "linux")]
fn save_container_log(name: &str) -> bool {
    if require_known_name(name).is_err() {
        return false;
    }
    let path = docker_log_file(&container_log_root(), name);
    let Some(parent) = path.parent() else {
        return false;
    };
    if fs::create_dir_all(parent).is_err() {
        return false;
    }
    let Some(partial) = partial_log_path(&path) else {
        return false;
    };
    let mut child = match Command::new(DOCKER)
        .args(["logs", name])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };
    let Ok(file) = fs::File::create(&partial) else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };
    let file = Arc::new(Mutex::new(file));
    let written = Arc::new(AtomicU64::new(0));
    let stdout_thread = {
        let file = Arc::clone(&file);
        let written = Arc::clone(&written);
        thread::spawn(move || copy_bounded(stdout, &file, &written, DOCKER_LOG_LIMIT))
    };
    let stderr_thread = {
        let file = Arc::clone(&file);
        let written = Arc::clone(&written);
        thread::spawn(move || copy_bounded(stderr, &file, &written, DOCKER_LOG_LIMIT))
    };
    let status = child.wait();
    let stdout_ok = stdout_thread.join().unwrap_or(false);
    let stderr_ok = stderr_thread.join().unwrap_or(false);
    let flush_ok = file
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .flush()
        .is_ok();
    drop(file);
    let docker_ok = status.is_ok_and(|status| status.success());
    // Keep a previous good log when `docker logs` fails or the copy stops short.
    if retained_container_log(docker_ok && stdout_ok && stderr_ok && flush_ok, "").is_none() {
        let _ = fs::remove_file(&partial);
        return false;
    }
    if fs::rename(&partial, &path).is_err() {
        let _ = fs::remove_file(&partial);
        return false;
    }
    true
}

#[cfg(target_os = "linux")]
pub(crate) fn remove_container(name: &str) {
    remove_container_saving(name, true);
}

#[cfg(target_os = "linux")]
fn remove_container_saving(name: &str, save: bool) {
    let name = runtime_name(name);
    if require_known_name(&name).is_err() {
        return;
    }
    if save {
        save_container_log(&name);
    }
    let _ = Command::new(DOCKER).args(["rm", "-f", &name]).output();
}

/// SIGTERM and wait, as the host harness does, so `ferroan-region` can
/// deregister before the proxy capture is read. `docker rm -f` would kill it first.
#[cfg(target_os = "linux")]
pub(crate) fn stop_container(name: &str) {
    let name = runtime_name(name);
    if require_known_name(&name).is_err() {
        return;
    }
    let _ = Command::new(DOCKER)
        .args(["stop", "--time", "30", &name])
        .output();
    remove_container(&name);
}

#[cfg(target_os = "linux")]
fn remove_known() {
    for name in known_container_names() {
        remove_container(name);
    }
}

/// Drop a previous run's containers without copying their logs into this run.
#[cfg(target_os = "linux")]
fn discard_known() {
    for name in known_container_names() {
        remove_container_saving(name, false);
    }
}

#[cfg(target_os = "linux")]
fn known_container_names() -> [&'static str; 7] {
    [
        NAME_HOME,
        NAME_NEIGHBOUR,
        NAME_ROBUST,
        NAME_ASSETS,
        NAME_PERSIST,
        NAME_LOKI,
        NAME_OTELCOL,
    ]
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
pub(crate) fn host_log_dir(name: &str) -> PathBuf {
    let root = if crate::ports::isolated() {
        crate::ports::run_root()
    } else {
        crate::live::live_root()
    };
    root.join("docker-log").join(name)
}

#[cfg(target_os = "linux")]
fn runtime_name(name: &str) -> String {
    const NAMES: &[&str] = &[
        NAME_HOME,
        NAME_NEIGHBOUR,
        NAME_ROBUST,
        NAME_ASSETS,
        NAME_PERSIST,
        NAME_LOKI,
        NAME_OTELCOL,
    ];
    if NAMES.contains(&name) {
        crate::ports::scoped_name(name, crate::ports::current())
    } else {
        name.to_string()
    }
}

#[cfg(target_os = "linux")]
fn require_known_name(name: &str) -> Result<(), String> {
    const NAMES: &[&str] = &[
        NAME_HOME,
        NAME_NEIGHBOUR,
        NAME_ROBUST,
        NAME_ASSETS,
        NAME_PERSIST,
        NAME_LOKI,
        NAME_OTELCOL,
    ];
    let known = NAMES.iter().any(|canonical| {
        name == *canonical
            || name.strip_prefix(*canonical).is_some_and(|rest| {
                rest.strip_prefix('-').is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit())
                })
            })
    });
    if known {
        Ok(())
    } else {
        Err(format!("refusing to touch unexpected container {name}"))
    }
}

#[cfg(target_os = "linux")]
fn start_collectors() -> Result<String, String> {
    let loki = crate::ports::alloc(crate::ports::Slot::Loki)?;
    let _grpc = crate::ports::alloc(crate::ports::Slot::LokiGrpc)?;
    let _otel_grpc = crate::ports::alloc(crate::ports::Slot::OtelGrpc)?;
    let otel_http = crate::ports::alloc(crate::ports::Slot::OtelHttp)?;
    let mut loki_args = vec![LOKI_IMAGE.to_string()];
    loki_args.extend(crate::ports::loki_server_args(crate::ports::current()));
    let loki_refs: Vec<&str> = loki_args.iter().map(String::as_str).collect();
    start_detached(NAME_LOKI, &loki_refs)?;
    wait_http(&format!("http://127.0.0.1:{loki}/ready"), "loki")?;
    let config = collector_config()?;
    let stdout_dir = otel_stdout_dir();
    fs::create_dir_all(&stdout_dir)
        .map_err(|error| format!("could not create {}: {error}", stdout_dir.display()))?;
    let config_mount = format!("{}:/etc/otelcol-contrib/config.yaml:ro", config.display());
    let stdout_mount = format!("{}:/var/log/ferroan:ro", stdout_dir.display());
    start_detached(
        NAME_OTELCOL,
        &[
            "-v",
            &config_mount,
            "-v",
            &stdout_mount,
            OTELCOL_IMAGE,
            "--config=/etc/otelcol-contrib/config.yaml",
        ],
    )?;
    let validated = validate_exporters()?;
    if crate::ports::isolated() {
        Ok(format!(
            "loki and otel-collector started on {loki} and {otel_http}; {validated}"
        ))
    } else {
        Ok(format!("loki and otel-collector started; {validated}"))
    }
}

#[cfg(target_os = "linux")]
fn collector_config() -> Result<PathBuf, String> {
    let committed = crate::repo_root().join("deploy/monitoring/collector/loki.yaml");
    if !crate::ports::isolated() {
        return Ok(committed);
    }
    let text = fs::read_to_string(&committed)
        .map_err(|error| format!("could not read {}: {error}", committed.display()))?;
    let rewritten = crate::ports::rewrite_collector(&text, crate::ports::current());
    let path = crate::ports::run_root().join("loki.yaml");
    fs::write(&path, rewritten)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(target_os = "linux")]
fn otel_stdout_dir() -> PathBuf {
    if crate::ports::isolated() {
        crate::ports::run_root().join("otel-stdout")
    } else {
        crate::repo_root().join(".scratch/otel-stdout")
    }
}

#[cfg(target_os = "linux")]
fn start_detached(name: &str, extra: &[&str]) -> Result<(), String> {
    let name = runtime_name(name);
    require_known_name(&name)?;
    remove_container(&name);
    let mut args = vec!["run", "-d", "--name", name.as_str(), "--network", "host"];
    args.extend(extra);
    let output = Command::new(DOCKER)
        .args(&args)
        .output()
        .map_err(|error| format!("could not start {name}: {error}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "could not start {name}: {}",
            err.chars().take(400).collect::<String>()
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn exporter_validate_args<'a>(mount: &'a str, image: &'a str) -> [&'a str; 9] {
    [
        "run",
        "--rm",
        "--network",
        "none",
        "-v",
        mount,
        image,
        "validate",
        "--config=/etc/otelcol-contrib/config.yaml",
    ]
}

#[cfg(target_os = "linux")]
fn validate_exporters() -> Result<String, String> {
    let root = crate::repo_root().join("deploy/monitoring/collector");
    let mut checked = Vec::new();
    for name in [
        "opensearch.yaml",
        "signoz.yaml",
        "graylog.yaml",
        "splunk.yaml",
    ] {
        let path = root.join(name);
        let mount = format!("{}:/etc/otelcol-contrib/config.yaml:ro", path.display());
        let args = exporter_validate_args(&mount, OTELCOL_IMAGE);
        let output = Command::new(DOCKER)
            .args(args)
            .output()
            .map_err(|error| format!("could not validate {name}: {error}"))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let out = String::from_utf8_lossy(&output.stdout);
            return Err(format!(
                "otelcol validate {name} failed: {}{}",
                out.chars().take(400).collect::<String>(),
                err.chars().take(400).collect::<String>()
            ));
        }
        checked.push(name);
    }
    Ok(format!("otelcol validate {}", checked.join(", ")))
}

#[cfg(target_os = "linux")]
fn wait_http(url: &str, name: &str) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(40) {
        if http_text(url).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(format!("{name} did not become ready"))
}

#[cfg(target_os = "linux")]
fn http_text(url: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let rest = url
        .strip_prefix("http://")
        .ok_or("only http URLs are read")?;
    let (hostport, path) = rest.split_once('/').ok_or("the URL has no path")?;
    let path = format!("/{path}");
    let (host, port) = hostport.split_once(':').ok_or("the URL has no port")?;
    let port: u16 = port.parse().map_err(|_| "the URL port is not a number")?;
    let mut stream = TcpStream::connect((host, port)).map_err(|error| error.to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let request = format!("GET {path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    if !text.contains(" 200 ") && !text.contains(" 204 ") {
        let head: String = text.chars().filter(|c| c.is_ascii()).take(160).collect();
        return Err(format!("not success: {head}"));
    }
    Ok(text)
}

#[cfg(target_os = "linux")]
pub(crate) fn container_logs(name: &str) -> Result<String, String> {
    let name = runtime_name(name);
    require_known_name(&name)?;
    let output = Command::new(DOCKER)
        .args(["logs", &name])
        .output()
        .map_err(|error| format!("could not read docker logs: {error}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() && text.is_empty() {
        return Err(format!("docker logs {name} failed"));
    }
    Ok(text)
}

#[cfg(target_os = "linux")]
pub(crate) fn compose_region_logs() -> Result<String, String> {
    let file = crate::repo_root().join("deploy/compose/compose.yaml");
    let project = crate::ports::compose_project(crate::ports::current());
    let output = Command::new(DOCKER)
        .args(["compose", "-p"])
        .arg(&project)
        .arg("-f")
        .arg(&file)
        .args(["logs", "--no-color", "ferroan-region"])
        .output()
        .map_err(|error| format!("could not run docker compose logs: {error}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "docker compose logs failed: {}",
            err.chars().take(400).collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
pub(crate) fn wait_loki_login() -> Result<String, String> {
    // Loki rejects a log selector on the instant `/query` endpoint. A range
    // covers the smoke run.
    let query = "{service_name=\"ferroan-region\"}";
    let encoded = query
        .replace('{', "%7B")
        .replace('}', "%7D")
        .replace('"', "%22")
        .replace('=', "%3D");
    let end_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let start_ns = end_ns.saturating_sub(5 * 60 * 1_000_000_000);
    let loki = crate::ports::assigned(crate::ports::Slot::Loki)
        .or_else(|| crate::ports::current().today_or_base(crate::ports::Slot::Loki))
        .unwrap_or(3100);
    let url = format!(
        "http://127.0.0.1:{loki}/loki/api/v1/query_range?query={encoded}&start={start_ns}&end={end_ns}&limit=50"
    );
    let started = Instant::now();
    let mut last = String::from("no response");
    let mut labels = String::new();
    while started.elapsed() < Duration::from_secs(45) {
        match http_text(&url) {
            Ok(text) => {
                let login = text.contains("login accepted");
                let trace = text.contains("trace span=login");
                if login && trace {
                    return Ok("service_name=ferroan-region".to_string());
                }
                last = format!("login={login}, trace={trace}, {} bytes", text.len());
            }
            Err(error) => last = error.chars().take(180).collect(),
        }
        if let Ok(text) = http_text(&format!("http://127.0.0.1:{loki}/loki/api/v1/labels")) {
            labels = text.chars().filter(|c| c.is_ascii()).take(180).collect();
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(format!(
        "loki did not store the login log and trace ({last}; labels {labels})"
    ))
}

#[cfg(target_os = "linux")]
fn pin_context() -> Result<PathBuf, String> {
    let pin = read_pin()?;
    if let Some(checkout) = reference_checkout()
        && clean_at(&checkout, &pin).is_ok()
    {
        return Ok(checkout);
    }
    let dest = if crate::ports::isolated() {
        crate::ports::run_root().join("docker/OpenSim-Tranquillity")
    } else {
        crate::repo_root().join(".scratch/docker/OpenSim-Tranquillity")
    };
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
    use std::path::Path;

    use super::*;

    const REGION_DOCKERFILE: &str = include_str!("../../deploy/docker/ferroan-region.Dockerfile");
    const ROBUST_DOCKERFILE: &str = include_str!("../../deploy/docker/ferroan-robust.Dockerfile");
    const COMPOSE: &str = include_str!("../../deploy/compose/compose.yaml");
    const ENV_EXAMPLE: &str = include_str!("../../deploy/compose/.env.example");
    const REGION_RUNTIME: &str =
        include_str!("../../deploy/docker/tranquillity-region.runtime.Dockerfile");

    #[test]
    fn smoke_prunes_build_cache_older_than_seven_days() {
        let args = builder_cache_prune_args().join(" ");
        assert_eq!(args, "builder prune --all --force --filter until=168h");
        assert!(!args.contains("network"));
        assert!(!args.contains("volume"));
    }

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
            "prometheus:",
            "grafana:",
            "loki:",
            "otel-collector:",
        ] {
            assert!(COMPOSE.contains(name), "{name}");
        }
        assert!(COMPOSE.contains("network_mode: host"));
        assert!(COMPOSE.contains("tranquillity-grid:p0.11"));
        assert!(COMPOSE.contains("tranquillity-region:p0.11"));
        assert!(COMPOSE.contains("ferroan-robust:p0.11"));
        assert!(COMPOSE.contains("ferroan-region:p0.11"));
        assert!(COMPOSE.contains("grafana/loki:3.2.1"));
        assert!(COMPOSE.contains("otel/opentelemetry-collector-contrib:0.128.0"));
        assert!(COMPOSE.contains("collector/loki.yaml"));
        assert!(!COMPOSE.to_ascii_lowercase().contains("password"));
        let loki = include_str!("../../deploy/monitoring/collector/loki.yaml");
        assert!(loki.contains("127.0.0.1:4318"));
        assert!(loki.contains("3100/otlp"));
        assert!(loki.contains("/var/log/ferroan"));
        assert!(
            include_str!("../../deploy/monitoring/collector/opensearch.yaml")
                .contains("opensearch")
        );
        assert!(
            include_str!("../../deploy/monitoring/collector/signoz.yaml")
                .contains("otlphttp/signoz")
        );
        assert!(
            include_str!("../../deploy/monitoring/collector/graylog.yaml").contains("otlp/graylog")
        );
        let splunk = include_str!("../../deploy/monitoring/collector/splunk.yaml");
        assert!(splunk.contains("splunk_hec"));
        assert!(splunk.contains("local-validate-token"));
        assert!(!splunk.to_ascii_lowercase().contains("password"));
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
    fn rootless_owner_check_requires_the_host_uid() {
        assert_eq!(
            probe_owner_result(false, "1000", None).unwrap(),
            "rootful daemon: grid user is host uid 1000"
        );
        assert_eq!(
            probe_owner_result(true, "1001", Some(1001)).unwrap(),
            "rootless owner check: container uid 0 wrote the probe file as host uid 1001"
        );
        let refused = probe_owner_result(true, "1001", Some(0)).unwrap_err();
        assert!(refused.contains("refusing rootless smoke"), "{refused}");
        assert!(refused.contains("uid 0"), "{refused}");
        assert!(refused.contains("host uid 1001"), "{refused}");
        let unread = probe_owner_result(true, "1001", None).unwrap_err();
        assert!(unread.contains("could not read the probe file"), "{unread}");
    }

    #[test]
    fn rootless_smoke_uses_container_root_and_rootful_keeps_the_host_user() {
        assert_eq!(container_user("1001", "1001", true).unwrap(), "0:0");
        assert_eq!(container_user("1000", "1000", false).unwrap(), "1000:1000");
        for rootless in [false, true] {
            let error = container_user("0", "0", rootless).unwrap_err();
            assert!(
                error.contains("refusing to run the grid as root"),
                "{error}"
            );
        }
        assert!(container_user("", "1000", false).is_err());
        assert!(container_user("1000", "", true).is_err());
    }

    #[test]
    fn security_options_mark_rootless_only_as_its_own_token() {
        assert!(!security_options_are_rootless(
            "[name=seccomp,profile=builtin name=cgroupns]"
        ));
        assert!(security_options_are_rootless(
            "[name=seccomp,profile=builtin name=rootless name=cgroupns]"
        ));
        assert!(security_options_are_rootless(
            r#"["name=seccomp,profile=builtin","name=rootless"]"#
        ));
        assert!(!security_options_are_rootless("name=rootlesskit"));
        assert!(!security_options_are_rootless(""));
    }

    #[test]
    fn missing_pid_reports_the_exit_and_the_last_log_lines() {
        let running = parse_container_snapshot("running\n0\n4242\nfalse\n").unwrap();
        assert!(pid_is_ready(&running));
        assert!(!container_has_exited(&running));

        let exited = parse_container_snapshot("exited\n1\n4242\nfalse\npermission denied").unwrap();
        assert!(!pid_is_ready(&exited));
        assert!(container_has_exited(&exited));
        let report = missing_pid_report(
            "ferroan-p011-persist",
            Some(&exited),
            "line-a\nsecret-value\nline-c",
        );
        assert!(report.contains("has no host pid"));
        assert!(report.contains("status exited"));
        assert!(report.contains("exit 1"));
        assert!(report.contains("permission denied"));
        assert!(report.contains("line-c"));
        let redacted = crate::live::redact_secrets(&report, &["secret-value"]);
        assert!(!redacted.contains("secret-value"));
        assert!(redacted.contains("<redacted>"));

        let oom = parse_container_snapshot("exited\n137\n0\ntrue\n").unwrap();
        let oom_report = missing_pid_report("ferroan-p011-persist", Some(&oom), "");
        assert!(oom_report.contains("out-of-memory"));
        assert!(oom_report.contains("exit 137"));

        let removing = parse_container_snapshot("removing\n0\n9\nfalse\n").unwrap();
        assert!(container_has_exited(&removing));
        assert!(!pid_is_ready(&removing));

        let bare = missing_pid_report("ferroan-p011-persist", None, "");
        assert_eq!(bare, "ferroan-p011-persist has no host pid");
        assert!(parse_container_snapshot("").is_none());
        assert!(parse_container_snapshot("running\n").is_none());
    }

    #[test]
    fn container_logs_match_the_images_artifact() {
        let path = docker_log_file(Path::new(".scratch/live"), "ferroan-p011-persist");
        let text = path.to_string_lossy().replace('\\', "/");
        assert!(text.ends_with(".scratch/live/docker-logs/ferroan-p011-persist.log"));
        let workflow = include_str!("../../.github/workflows/live.yml");
        let images = workflow
            .split("name: live-logs-images")
            .nth(1)
            .unwrap()
            .split("name: live (ubuntu-latest)")
            .next()
            .unwrap();
        assert!(images.contains(".scratch/live/**/*.log"));
        assert_eq!(
            retained_container_log(true, "asset service"),
            Some("asset service")
        );
        assert_eq!(retained_container_log(true, ""), Some(""));
        assert_eq!(retained_container_log(false, "No such container"), None);
        let partial = partial_log_path(Path::new(
            ".scratch/live/docker-logs/ferroan-p011-persist.log",
        ))
        .unwrap();
        let partial = partial.to_string_lossy().replace('\\', "/");
        assert!(partial.ends_with("ferroan-p011-persist.log.partial"));
        assert_eq!(DOCKER_LOG_LIMIT, 64 * 1024 * 1024);
    }

    #[test]
    fn docker_failure_includes_the_start_of_stderr() {
        assert_eq!(docker_failure_text(""), "docker command failed");
        assert_eq!(
            docker_failure_text("No such container"),
            "docker command failed: No such container"
        );
        let long = "e".repeat(500);
        let text = docker_failure_text(&long);
        let prefix = "docker command failed: ";
        assert!(text.starts_with(prefix), "{text}");
        assert_eq!(
            text.chars().count(),
            prefix.chars().count() + DOCKER_ERROR_CHARS
        );
    }

    #[test]
    fn copy_bounded_stops_writing_at_the_limit_and_drains() {
        let file = Mutex::new(Vec::new());
        let written = AtomicU64::new(0);
        assert!(copy_bounded(&b"0123456789"[..], &file, &written, 10));
        assert!(copy_bounded(&b"abcdefghij"[..], &file, &written, 10));
        assert_eq!(written.load(Ordering::Relaxed), 10);
        assert_eq!(file.into_inner().unwrap(), b"0123456789");
    }

    #[test]
    fn exporter_validate_uses_network_none() {
        let args = exporter_validate_args(
            "config.yaml",
            "otel/opentelemetry-collector-contrib:0.128.0",
        );
        assert_eq!(args[2], "--network");
        assert_eq!(args[3], "none");
        assert!(!args.contains(&"host"));
    }

    #[test]
    fn last_log_lines_keeps_forty_lines_and_four_thousand_chars() {
        let text = (0..50)
            .map(|n| format!("line-{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = last_log_lines(&text);
        assert!(tail.starts_with("line-10\n"));
        assert!(tail.contains("line-49"));
        assert!(!tail.contains("line-9\n"));
        let long = "é".repeat(5000);
        let cut = last_log_lines(&long);
        assert!(cut.len() <= 4000);
        assert!(cut.ends_with('é'));
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
