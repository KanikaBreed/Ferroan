// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `cargo xtask live` builds and starts the pinned Tranquillity region.
//!
//! The clone, the build and the run files stay under `.scratch/live/`. The
//! reference checkout is only read. The password is generated per run, written
//! into the ignored ini, and never printed.

#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};
use std::thread;
use std::time::{Duration, Instant};

use crate::repo_root;

const READY: &str = "INITIALIZATION COMPLETE FOR Ferroan Test - LOGINS ENABLED";
pub(crate) const ESTATE_OWNER: &str = "4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55";
pub(crate) const REGION_UUID: &str = "6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01";
pub(crate) const CHAT_USER: &str = "8c4e1a70-2b59-4d6f-9e13-5a7c0d8b2f44";
pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(180);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const PASSWORD_BYTES: usize = 18;

pub fn build() -> ExitCode {
    finish(build_inner())
}

pub fn up(ferroan_assets: bool) -> ExitCode {
    finish(up_inner(ferroan_assets))
}

pub fn run_running_assets() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        finish(run_running_assets_linux())
    }
    #[cfg(not(target_os = "linux"))]
    {
        finish(Err("cargo xtask live runs on Linux".to_string()))
    }
}

pub fn down() -> ExitCode {
    finish(down_inner())
}

pub fn run(scenario: &str, basic_physics: bool) -> ExitCode {
    finish(run_inner(scenario, basic_physics))
}

pub fn record_twice() -> ExitCode {
    finish(record_twice_inner())
}

pub(crate) fn finish(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(message) => {
            if !message.is_empty() {
                println!("{message}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("live: {error}");
            ExitCode::from(1)
        }
    }
}

fn build_inner() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        build_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("cargo xtask live runs on Linux".to_string())
    }
}

fn up_inner(ferroan_assets: bool) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        up_linux(ferroan_assets)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ferroan_assets;
        Err("cargo xtask live runs on Linux".to_string())
    }
}

fn down_inner() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        down_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("cargo xtask live runs on Linux".to_string())
    }
}

fn run_inner(scenario: &str, basic_physics: bool) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        run_linux(scenario, basic_physics)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (scenario, basic_physics);
        Err("cargo xtask live runs on Linux".to_string())
    }
}

fn record_twice_inner() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        record_twice_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("cargo xtask live runs on Linux".to_string())
    }
}

pub(crate) fn live_root() -> PathBuf {
    repo_root().join(".scratch").join("live")
}

pub(crate) fn source_dir() -> PathBuf {
    live_root().join("src")
}

fn run_dir() -> PathBuf {
    if crate::ports::isolated() {
        crate::ports::run_root().join("standalone")
    } else {
        live_root().join("run")
    }
}

pub(crate) fn diag_dir() -> PathBuf {
    if crate::ports::isolated() {
        crate::ports::run_root().join("diag")
    } else {
        live_root().join("diag")
    }
}

pub(crate) fn publish_dir() -> PathBuf {
    source_dir().join("Source/OpenSim.Server.RegionServer/bin/Release/net10.0/linux-x64/publish")
}

fn region_binary() -> PathBuf {
    publish_dir().join("OpenSim.Server.RegionServer")
}

fn grid_publish_dir() -> PathBuf {
    source_dir().join("Source/OpenSim.Server.GridServer/bin/Release/net10.0/linux-x64/publish")
}

fn grid_binary() -> PathBuf {
    grid_publish_dir().join("OpenSim.Server.GridServer")
}

fn state_path() -> PathBuf {
    run_dir().join("state")
}

struct RunState {
    pid: u32,
    public_port: u16,
    private_port: u16,
    assets: Option<AssetState>,
}

struct AssetState {
    pid: u32,
    start_time: u64,
    port: u16,
}

fn write_state(path: &Path, state: &RunState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut text = format!(
        "pid={}\npublic_port={}\nprivate_port={}\n",
        state.pid, state.public_port, state.private_port
    );
    if let Some(asset) = &state.assets {
        text.push_str(&format!(
            "asset_pid={}\nasset_start_time={}\nasset_port={}\n",
            asset.pid, asset.start_time, asset.port
        ));
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, text)
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn read_state(path: &Path) -> Result<Option<RunState>, String> {
    let text = match read_bounded_text(path, 4096) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let mut pid = None;
    let mut public_port = None;
    let mut private_port = None;
    let mut asset_pid = None;
    let mut asset_start_time = None;
    let mut asset_port = None;
    let mut has_assets = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "pid" => pid = value.parse().ok(),
            "public_port" => public_port = value.parse().ok(),
            "private_port" => private_port = value.parse().ok(),
            "asset_pid" => {
                has_assets = true;
                asset_pid = value.parse().ok();
            }
            "asset_start_time" => {
                has_assets = true;
                asset_start_time = value.parse().ok();
            }
            "asset_port" => {
                has_assets = true;
                asset_port = value.parse().ok();
            }
            _ => {}
        }
    }
    let assets = match (has_assets, asset_pid, asset_start_time, asset_port) {
        (false, _, _, _) => None,
        (true, Some(pid), Some(start_time), Some(port))
            if pid > 1 && start_time > 0 && port > 0 =>
        {
            Some(AssetState {
                pid,
                start_time,
                port,
            })
        }
        _ => {
            return Err(format!(
                "{} has invalid asset process state",
                path.display()
            ));
        }
    };
    match (pid, public_port, private_port) {
        (Some(pid), Some(public_port), Some(private_port))
            if pid > 1 && public_port > 0 && private_port > 0 =>
        {
            Ok(Some(RunState {
                pid,
                public_port,
                private_port,
                assets,
            }))
        }
        _ => Err(format!("{} is missing pid or ports", path.display())),
    }
}

fn read_bounded_text(path: &Path, limit: usize) -> std::io::Result<String> {
    use std::io::Read;
    let mut text = String::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_string(&mut text)?;
    if text.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds its size limit",
        ));
    }
    Ok(text)
}

struct StandaloneIni<'a> {
    regions_dir: &'a Path,
    capture_file: Option<&'a Path>,
    public_port: u16,
    private_port: u16,
    password: &'a str,
    asset_include: Option<&'a Path>,
    http_bind: Ipv4Addr,
    basic_physics: bool,
}

fn region_server_ini(region: &StandaloneIni<'_>) -> Result<String, String> {
    let regions_dir = region.regions_dir;
    let capture_file = region.capture_file;
    let public_port = region.public_port;
    let private_port = region.private_port;
    let password = region.password;
    let asset_include = region.asset_include;
    let http_bind = region.http_bind;
    let basic_physics = region.basic_physics;
    let dir = regions_dir
        .to_str()
        .ok_or("regions directory is not utf-8")?;
    let capture = match capture_file {
        Some(path) => Some(path.to_str().ok_or("capture path is not utf-8")?),
        None => None,
    };
    if dir.contains(['"', '\n', '\r'])
        || capture.is_some_and(|value| value.contains(['"', '\n', '\r']))
        || password.contains(['"', '\n', '\r'])
    {
        return Err("a configuration value contains a quote or a line break".to_string());
    }
    if password.is_empty() || public_port == 0 || private_port == 0 || public_port == private_port {
        return Err("the region needs two different ports and a password".to_string());
    }
    // Standalone.ini is included after this file, and it sets
    // LocalGridAssetService. A later include is the setting that wins.
    let asset_include = match asset_include {
        Some(path) => {
            let path = path.to_str().ok_or("asset override path is not utf-8")?;
            if path.contains(['"', '\n', '\r']) {
                return Err("the asset override path contains a quote or a line break".to_string());
            }
            format!("\n    Include-Asset = \"{path}\"")
        }
        None => String::new(),
    };
    let physics_line = if basic_physics {
        "\n    physics = basicphysics"
    } else {
        ""
    };
    let mut ini = format!(
        r#"[Const]
    BaseHostname = "127.0.0.1"
    BaseURL = http://${{Const|BaseHostname}}
    PublicPort = "{public_port}"
    PrivURL = ${{Const|BaseURL}}
    PrivatePort = "{private_port}"

[Startup]
    regionload_regionsdir = "{dir}"{physics_line}

[Network]
    http_listener_port = {public_port}
    bind_ip_address = "{http_bind}"

[Estates]
    DefaultEstateName = "Ferroan Test Estate"
    DefaultEstateOwnerName = "Test User"
    DefaultEstateOwnerEMail = "test@example.invalid"
    DefaultEstateOwnerUUID = "{ESTATE_OWNER}"
    DefaultEstateOwnerPassword = "{password}"

[Architecture]
    Include-Architecture = "config-include/Standalone.ini"{asset_include}
"#
    );
    // Standalone logins probe this process. Grid inis leave the key unset.
    ini.push_str("\n[ClientStack.LindenUDP]\n    ServiceProbe = true\n");
    if let Some(capture) = capture {
        ini.push_str(&format!("    CaptureFile = \"{capture}\"\n"));
    }
    Ok(ini)
}

fn regions_ini(public_port: u16) -> String {
    format!(
        r#"[Ferroan Test]
RegionUUID = {REGION_UUID}
Location = 1000,1000
InternalAddress = 127.0.0.1
InternalPort = {public_port}
AllowAlternatePorts = False
ExternalHostName = 127.0.0.1
"#
    )
}

fn up_report(port: u16, log: &Path) -> String {
    format!(
        "live: up\nlogin: http://127.0.0.1:{port}/\nhttp_listen: 127.0.0.1:{port}\nlog: {}",
        log.display()
    )
}

pub(crate) fn redact_secrets(text: &str, passwords: &[&str]) -> String {
    let mut passwords = passwords.to_vec();
    passwords.sort_by_key(|password| std::cmp::Reverse(password.len()));
    let mut out = text.to_owned();
    for password in passwords {
        if !password.is_empty() {
            out = out.replace(password, "<redacted>");
        }
    }
    out
}

fn redacted_tail(text: &str, passwords: &[&str]) -> String {
    let text = redact_secrets(text, passwords);
    let mut start = text.len().saturating_sub(4000);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

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
                if hash.bytes().all(|byte| byte.is_ascii_hexdigit()) && hash.len() == 40 {
                    return Ok(hash.to_ascii_lowercase());
                }
            }
            index += 1;
        }
        return Err("OpenSim-Tranquillity row has no 40-character revision".to_string());
    }
    Err("docs/REFERENCES.md has no OpenSim-Tranquillity row".to_string())
}

struct ProcSock {
    local: IpAddr,
    port: u16,
    listening: bool,
    udp: bool,
    inode: u64,
}

fn parse_proc_addr(field: &str) -> Option<(IpAddr, u16)> {
    let (ip_hex, port_hex) = field.split_once(':')?;
    let port = u16::from_str_radix(port_hex, 16).ok()?;
    let ip = if ip_hex.len() == 8 {
        let value = u32::from_str_radix(ip_hex, 16).ok()?;
        IpAddr::V4(Ipv4Addr::from(value.to_le_bytes()))
    } else if ip_hex.len() == 32 {
        let mut octets = [0u8; 16];
        for (index, chunk) in ip_hex.as_bytes().chunks(8).enumerate() {
            let text = std::str::from_utf8(chunk).ok()?;
            let word = u32::from_str_radix(text, 16).ok()?;
            octets[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        IpAddr::V6(Ipv6Addr::from(octets))
    } else {
        return None;
    };
    Some((ip, port))
}

fn parse_proc_net(text: &str, udp: bool) -> Vec<ProcSock> {
    let mut sockets = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 10 {
            continue;
        }
        let Some((local, port)) = parse_proc_addr(fields[1]) else {
            continue;
        };
        let Ok(state) = u8::from_str_radix(fields[3], 16) else {
            continue;
        };
        let Ok(inode) = fields[9].parse::<u64>() else {
            continue;
        };
        sockets.push(ProcSock {
            local,
            port,
            listening: !udp && state == 0x0a,
            udp,
            inode,
        });
    }
    sockets
}

fn loopback_report(sockets: &[ProcSock], port: u16) -> Result<(), String> {
    bound_report(sockets, port, true, Ipv4Addr::LOCALHOST)
}

fn http_loopback_report(sockets: &[ProcSock], port: u16) -> Result<(), String> {
    bound_report(sockets, port, false, Ipv4Addr::LOCALHOST)
}

fn bound_report(
    sockets: &[ProcSock],
    port: u16,
    require_udp: bool,
    http_bind: Ipv4Addr,
) -> Result<(), String> {
    let mut lines = Vec::new();
    let mut http = false;
    let mut udp = false;
    for socket in sockets {
        let kind = if socket.listening {
            "listen"
        } else if socket.udp {
            "udp"
        } else {
            "socket"
        };
        lines.push(format!("{kind} {}:{}", socket.local, socket.port));
        if socket.local.is_unspecified() {
            return Err(format!(
                "a harness socket is not on 127.0.0.1: {}",
                lines.join(", ")
            ));
        }
        if socket.listening && socket.local == IpAddr::V4(http_bind) && socket.port == port {
            http = true;
        }
        if socket.udp && socket.local == IpAddr::V4(Ipv4Addr::LOCALHOST) && socket.port == port {
            udp = true;
        }
    }
    if !http || (require_udp && !udp) {
        let expected = if require_udp { "TCP and UDP" } else { "TCP" };
        return Err(format!(
            "expected {expected} on 127.0.0.1:{port}, saw {}",
            if lines.is_empty() {
                "no sockets".to_string()
            } else {
                lines.join(", ")
            }
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn build_linux() -> Result<String, String> {
    let dir = live_root();
    fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("build.lock"))
        .map_err(|error| format!("could not open the live build lock: {error}"))?;
    lock.lock()
        .map_err(|error| format!("could not lock the live build: {error}"))?;
    build_linux_locked()
}

#[cfg(target_os = "linux")]
fn build_linux_locked() -> Result<String, String> {
    let ferrobot = crate::ferrobot_build::build()?;
    let pin = read_pin()?;
    let reference = tranquillity_reference()?;
    require_clean_pin(&reference, &pin)?;
    let src = prepare_source(&reference, &pin)?;
    let capture = repo_root().join("tools/capture/tranquillity-capture.patch");
    let stamp_text = format!(
        "pin={pin}\ncapture={}\ngrid=self-contained\n",
        file_hash(&capture)?
    );
    let binary = region_binary();
    let grid = grid_binary();
    let stamp = live_root().join("stamp");
    if binary.is_file()
        && grid.is_file()
        && fs::read_to_string(&stamp).ok().as_deref() == Some(stamp_text.as_str())
    {
        return Ok(format!(
            "live: build is current\nbinary: {}\ngrid: {}\nferrobot: {}",
            binary.display(),
            grid.display(),
            ferrobot.display()
        ));
    }
    println!("live: building Tranquillity {pin}");
    reset_source(&src, &pin)?;
    git_apply(&src, &capture)?;
    publish(
        &src,
        "Source/OpenSim.Server.RegionServer/OpenSim.Server.RegionServer.csproj",
        &binary,
        false,
    )?;
    // GridServer references Microsoft.AspNetCore.App. The installed runtime is
    // Microsoft.NETCore.App only, so the publish bundles the ASP.NET runtime.
    publish(
        &src,
        "Source/OpenSim.Server.GridServer/OpenSim.Server.GridServer.csproj",
        &grid,
        true,
    )?;
    copy_examples(&publish_dir())?;
    fs::write(&stamp, &stamp_text)
        .map_err(|error| format!("could not write {}: {error}", stamp.display()))?;
    Ok(format!(
        "live: built {pin}\nbinary: {}\ngrid: {}\nferrobot: {}",
        binary.display(),
        grid.display(),
        ferrobot.display()
    ))
}

#[cfg(target_os = "linux")]
struct Started {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    public_port: u16,
    password: String,
    log: PathBuf,
    tracked: crate::grid_state::Tracked,
    proxy: Option<crate::live_proxy::Proxies>,
}

#[cfg(target_os = "linux")]
fn up_linux(ferroan_assets: bool) -> Result<String, String> {
    if let Some(pid) = standalone_is_up()? {
        return Err(format!(
            "the harness is already up (pid {pid}); cargo xtask live down stops it"
        ));
    }
    let _state = crate::grid_state::claim()?;
    let assets = if ferroan_assets {
        ensure_region_built()?;
        Some(crate::assets::start_persistent()?)
    } else {
        None
    };
    let uri = assets
        .as_ref()
        .map(|(_, port)| format!("http://127.0.0.1:{port}"));
    let mut started = start_region(false, uri.as_deref(), false, false)?;
    let saved = (|| {
        let mut report = up_report(started.public_port, &started.log);
        if let Some((server, port)) = &assets {
            let start_time = process_start_time(server.pid())
                .ok_or("the asset service exited during region startup")?;
            if !crate::assets::persistent_pid_live(server.pid(), start_time) {
                return Err("the asset service does not belong to this run".to_string());
            }
            let mut state =
                read_state(&state_path())?.ok_or("the region did not save run state")?;
            state.assets = Some(AssetState {
                pid: server.pid(),
                start_time,
                port: *port,
            });
            write_secret(&run_dir().join("password"), &started.password)?;
            write_state(&state_path(), &state)?;
            report.push_str(&format!(
                "\nassets: ferroan\nasset_http_listen: 127.0.0.1:{port}"
            ));
        }
        Ok(report)
    })();
    let report = match saved {
        Ok(report) => report,
        Err(error) => {
            let _ = stop_started(&mut started);
            let _ = fs::remove_file(run_dir().join("password"));
            return Err(error);
        }
    };
    if let Some((server, _)) = assets {
        server.detach();
    }
    // Dropping Child does not kill the process. `live up` exits and init inherits it.
    drop(started.child);
    drop(started.stdin);
    Ok(report)
}

#[cfg(target_os = "linux")]
fn run_linux(scenario: &str, basic_physics: bool) -> Result<String, String> {
    if scenario == "ferrobot" {
        return run_ferrobot(basic_physics);
    }
    if !ferroan_bot::register::runs_standalone(scenario) {
        return Err(ferroan_bot::register::standalone_unknown(scenario));
    }
    if let Some(pid) = standalone_is_up()? {
        return Err(format!(
            "the region is already up (pid {}); cargo xtask live down stops it",
            pid
        ));
    }
    let _state = crate::grid_state::claim()?;
    let mut started = start_region(true, None, true, basic_physics)?;
    let outcome = run_scenarios(&mut started, scenario);
    let _ = stop_started(&mut started);
    let outcome = outcome?;
    let capture = finish_capture(
        &capture_path(),
        &outcome.http,
        &[&started.password, &outcome.chat_password],
    )?;
    Ok(format!("{}\n{capture}", outcome.report))
}

#[cfg(target_os = "linux")]
fn record_twice_linux() -> Result<String, String> {
    if ferroan_bot::scenario::selection("login").is_none() {
        return Err("unknown scenario login".to_string());
    }
    if let Some(pid) = standalone_is_up()? {
        return Err(format!(
            "the region is already up (pid {pid}); cargo xtask live down stops it"
        ));
    }
    ensure_region_built()?;
    let started_at = Instant::now();
    let _state = crate::grid_state::claim()?;
    let mut started = start_region(true, None, true, false)?;
    let mut secrets = vec![started.password.clone()];
    let outcome: Result<(String, String, Vec<u8>), String> = (|| {
        let chat_password = generate_password()?;
        secrets.push(chat_password.clone());
        create_chat_user(&mut started, &chat_password)?;
        let first = run_scenarios_with_accounts(
            started.public_port,
            &started.password,
            &chat_password,
            &started.log,
            "login",
        )?;
        let second = run_scenarios_with_accounts(
            started.public_port,
            &started.password,
            &chat_password,
            &started.log,
            "login",
        )?;
        let bytes = fs::read(capture_path())
            .map_err(|error| format!("could not read the region UDP capture: {error}"))?;
        Ok((first.report, second.report, bytes))
    })();
    let cleanup = stop_started(&mut started);
    let secret_refs: Vec<&str> = secrets.iter().map(|secret| secret.as_str()).collect();
    let (first, second, bytes) = match outcome {
        Ok(value) => {
            cleanup.map_err(|error| redact_secrets(&error, &secret_refs))?;
            value
        }
        Err(error) => {
            let mut text = redact_secrets(&error, &secret_refs);
            if let Err(cleanup) = cleanup {
                text.push('\n');
                text.push_str(&redact_secrets(&cleanup, &secret_refs));
            }
            return Err(text);
        }
    };
    let seconds = started_at.elapsed().as_secs();
    match crate::twice::compare_capture(&bytes) {
        Ok(report) => Ok(format!(
            "{first}\n{second}\n{report}\nrecord-twice: {seconds}s"
        )),
        Err(error) => Err(format!(
            "{first}\n{second}\n{error}\nrecord-twice: {seconds}s"
        )),
    }
}

#[cfg(target_os = "linux")]
fn run_ferrobot(basic_physics: bool) -> Result<String, String> {
    let executable = crate::ferrobot_build::build()?;
    if let Some(pid) = standalone_is_up()? {
        return Err(format!(
            "the region is already up (pid {pid}); cargo xtask live down stops it"
        ));
    }
    let _state = crate::grid_state::claim()?;
    let mut started = start_region_with_proxy(true, None, true, true, basic_physics)?;
    let outcome = (|| {
        // Region and Ferrobot share the claim taken before either child starts.
        let chat_password = generate_password()?;
        create_chat_user(&mut started, &chat_password)?;
        let probe = format!("http://127.0.0.1:{}", started.public_port);
        let harness = harness(
            &format!("http://127.0.0.1:{}/", started.public_port),
            &probe,
            &started.password,
            &chat_password,
            &started.log,
        )?;
        let config = crate::ferrobot::Config {
            executable,
            root: run_dir().join("ferrobot"),
            region_name: "Ferroan Test".to_string(),
            ferroan: false,
            udp_capture: capture_path(),
            http_captures: vec![run_dir().join("ferrobot-http.cap")],
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let report = runtime.block_on(crate::ferrobot::run(&harness, &config))?;
        Ok::<_, String>((report, config.http_captures, chat_password))
    })();
    let cleanup = stop_started(&mut started);
    if let Err(error) = &outcome
        && let Err(cleanup) = &cleanup
    {
        return Err(format!("{error}\n{cleanup}"));
    }
    cleanup?;
    let (report, http_captures, chat_password) = outcome?;
    let http = crate::grid::ferrobot_capture::http_records(&http_captures)?;
    let capture = finish_capture(&capture_path(), &http, &[&started.password, &chat_password])?;
    Ok(format!("{report}\n{capture}"))
}

/// Start the region with its asset service pointed at `asset_uri`.
#[cfg(target_os = "linux")]
pub(crate) fn run_against_assets(asset_uri: &str, scenario: &str) -> Result<String, String> {
    if ferroan_bot::scenario::selection(scenario).is_none() {
        return Err(format!("unknown scenario {scenario}"));
    }
    // The asset comparison owns the shared process-state claim already.
    let mut started = start_region(true, Some(asset_uri), true, false)?;
    let outcome = run_scenarios(&mut started, scenario);
    let _ = stop_started(&mut started);
    let outcome = outcome?;
    let capture = finish_capture(
        &capture_path(),
        &outcome.http,
        &[&started.password, &outcome.chat_password],
    )?;
    Ok(format!("{}\n{capture}", outcome.report))
}

#[cfg(target_os = "linux")]
pub(crate) fn ensure_region_built() -> Result<(), String> {
    build_linux().map(|_| ())
}

/// Use Cargo's artifact path, including configured target directories/profiles.
#[cfg(target_os = "linux")]
pub(crate) fn build_binary(name: &str) -> Result<PathBuf, String> {
    build_binary_with_profile(name, "dev")
}

#[cfg(target_os = "linux")]
pub(crate) fn build_binary_with_profile(name: &str, profile: &str) -> Result<PathBuf, String> {
    use std::io::Read;
    use std::process::Stdio;
    let log = repo_root()
        .join(".scratch")
        .join(format!("cargo-build-{name}.log"));
    fs::create_dir_all(repo_root().join(".scratch"))
        .map_err(|error| format!("could not create Cargo log directory: {error}"))?;
    let stderr =
        File::create(&log).map_err(|error| format!("could not create Cargo log: {error}"))?;
    let mut child = Command::new("cargo")
        .args([
            "build",
            "--profile",
            profile,
            "-p",
            name,
            "--bin",
            name,
            "--locked",
            "--message-format=json",
        ])
        .current_dir(repo_root())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .map_err(|error| format!("could not build {name}: {error}"))?;
    let mut messages = Vec::new();
    let read = child
        .stdout
        .take()
        .ok_or("Cargo stdout was not piped")?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut messages);
    if read.is_err() || messages.len() > 16 * 1024 * 1024 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("could not read Cargo artifacts within the 16 MiB bound".to_owned());
    }
    let status = child
        .wait()
        .map_err(|error| format!("could not wait for Cargo: {error}"))?;
    if !status.success() {
        return Err(format!("cargo build {name} failed; see {}", log.display()));
    }
    artifact_executable(&messages, name)
}

#[cfg(any(target_os = "linux", test))]
fn artifact_executable(messages: &[u8], name: &str) -> Result<PathBuf, String> {
    if messages.len() > 16 * 1024 * 1024 {
        return Err("Cargo artifact output exceeds 16 MiB".to_owned());
    }
    let mut executable = None;
    for line in messages
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value = serde_json::from_slice(line)
            .map_err(|error| format!("invalid Cargo artifact message: {error}"))?;
        if value["reason"] == "compiler-artifact"
            && value["target"]["name"] == name
            && value["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
            && let Some(path) = value["executable"].as_str()
        {
            executable = Some(PathBuf::from(path));
        }
    }
    executable.ok_or_else(|| format!("Cargo did not report an executable for {name}"))
}

#[cfg(target_os = "linux")]
pub(crate) fn spawn_tracked(
    command: &mut Command,
    binary: &Path,
    args: &[&str],
    cwd: &Path,
) -> Result<(std::process::Child, crate::grid_state::Tracked), String> {
    let launch = crate::grid_state::prepare_spawn(binary, args, cwd)?;
    launch.configure(command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            launch.cancel()?;
            return Err(format!("could not start {}: {error}", binary.display()));
        }
    };
    match launch.record(&child) {
        Ok(tracked) => Ok((child, tracked)),
        Err(error) => {
            stop_child(&mut child);
            if child.try_wait().is_ok_and(|status| status.is_some()) {
                launch.cancel()?;
            }
            Err(error)
        }
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn is_sqlite_file(name: &str) -> bool {
    [
        ".db",
        ".db-shm",
        ".db-wal",
        ".db-journal",
        ".sqlite",
        ".sqlite3",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
}

#[cfg(target_os = "linux")]
pub(crate) fn grid_server_binary() -> PathBuf {
    grid_binary()
}

#[cfg(target_os = "linux")]
pub(crate) fn asset_set_path() -> PathBuf {
    source_dir().join("Source/OpenSim.Services.InventoryService/AppData/assets/AssetSets.xml")
}

#[cfg(target_os = "linux")]
pub(crate) fn scratch_run() -> PathBuf {
    run_dir()
}

#[cfg(target_os = "linux")]
pub(crate) fn allocate_port() -> Result<u16, String> {
    crate::ports::alloc_dynamic()
}

#[cfg(target_os = "linux")]
pub(crate) fn require_http_loopback(pid: u32, port: u16) -> Result<(), String> {
    let sockets = process_sockets(pid)?;
    http_loopback_report(&sockets, port)
}

#[cfg(target_os = "linux")]
struct ScenarioOutcome {
    report: String,
    http: Vec<ferroan_capture::Record>,
    chat_password: String,
}

#[cfg(target_os = "linux")]
fn capture_path() -> PathBuf {
    run_dir().join("capture-udp.cap")
}

#[cfg(target_os = "linux")]
pub(crate) fn finish_capture(
    udp_path: &Path,
    http: &[ferroan_capture::Record],
    passwords: &[&str],
) -> Result<String, String> {
    use ferroan_capture::{Kind, check_udp, read_complete_prefix, write_capture};
    let bytes = fs::read(udp_path)
        .map_err(|error| format!("could not read {}: {error}", udp_path.display()))?;
    let mut records = read_complete_prefix(&bytes).map_err(|error| error.to_string())?;
    let udp = records
        .iter()
        .filter(|record| matches!(record.kind, Kind::UdpIn | Kind::UdpOut))
        .count();
    if udp == 0 {
        return Err("the region wrote no UDP records".to_string());
    }
    let http_count = http.len();
    if http_count == 0 {
        return Err("the bot recorded no HTTP exchanges".to_string());
    }
    records.extend(http.iter().cloned());
    records.sort_by_key(|record| record.time_ms);
    if let Err(error) = check_udp(&records) {
        let mut failure = format!("capture check failed: {error}");
        if let Some(mismatch) = ferroan_capture::udp_mismatch(&records) {
            match ferroan_capture::write_udp_mismatch(&diag_dir(), &mismatch) {
                Ok(path) => failure.push_str(&format!("; wrote {}", path.display())),
                Err(write_error) => {
                    failure.push_str(&format!("; could not save the record: {write_error}"))
                }
            }
        }
        return Err(failure);
    }
    let redacted = ferroan_capture::redact_records(&records)
        .map_err(|error| format!("redact failed: {error}"))?;
    let bytes = write_capture(&redacted).map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    for password in passwords {
        if !password.is_empty() && text.contains(password) {
            return Err("the redacted capture still contains a password".to_string());
        }
    }
    let redacted_path = udp_path
        .parent()
        .unwrap_or(udp_path)
        .join("capture.redacted.cap");
    fs::write(&redacted_path, &bytes)
        .map_err(|error| format!("could not write {}: {error}", redacted_path.display()))?;
    check_udp(&redacted).map_err(|error| format!("capture replay failed: {error}"))?;
    Ok(format!(
        "capture: {udp} udp, {http_count} http, check passed, redact passed, replay passed\nredacted: {}",
        redacted_path.display()
    ))
}

#[cfg(target_os = "linux")]
fn run_scenarios(started: &mut Started, scenario: &str) -> Result<ScenarioOutcome, String> {
    let chat_password = generate_password()?;
    create_chat_user(started, &chat_password)?;
    run_scenarios_with_accounts(
        started.public_port,
        &started.password,
        &chat_password,
        &started.log,
        scenario,
    )
}

#[cfg(target_os = "linux")]
fn run_running_assets_linux() -> Result<String, String> {
    let state =
        read_state(&state_path())?.ok_or("run cargo xtask live up --assets ferroan first")?;
    let assets = state
        .assets
        .as_ref()
        .ok_or("this region was not started with --assets ferroan")?;
    if !region_pid_live(state.pid)
        || !crate::assets::persistent_pid_live(assets.pid, assets.start_time)
    {
        return Err("the saved region or asset service is no longer running; run cargo xtask live down first".to_string());
    }
    require_http_loopback(state.pid, state.public_port)?;
    require_http_loopback(assets.pid, assets.port)?;
    let path = run_dir().join("password");
    let text = read_bounded_text(&path, PASSWORD_BYTES * 2)
        .map_err(|error| format!("could not read the saved harness password: {error}"))?;
    let password = saved_password(text.into_bytes())?;
    // The assets selection only uses the owner account. It neither needs nor
    // creates the second account used by the chat scenario.
    let outcome = run_scenarios_with_accounts(
        state.public_port,
        &password,
        "",
        &run_dir().join("region.log"),
        "assets",
    )?;
    Ok(format!(
        "{}\nassets: running services kept up",
        outcome.report
    ))
}

#[cfg(target_os = "linux")]
fn run_scenarios_with_accounts(
    public_port: u16,
    password: &str,
    chat_password: &str,
    log: &Path,
    scenario: &str,
) -> Result<ScenarioOutcome, String> {
    let harness = harness(
        &format!("http://127.0.0.1:{public_port}/"),
        &format!("http://127.0.0.1:{public_port}"),
        password,
        chat_password,
        log,
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start tokio: {error}"))?;
    let report = runtime.block_on(ferroan_bot::scenario::run(scenario, &harness))?;
    Ok(ScenarioOutcome {
        report,
        http: harness.http_trace.records(),
        chat_password: chat_password.to_owned(),
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn harness(
    login_uri: &str,
    service_probe: &str,
    password: &str,
    chat_password: &str,
    log: &Path,
) -> Result<ferroan_bot::scenario::Harness, String> {
    let texture = texture_bytes()?;
    let region_id = uuid::Uuid::parse_str(REGION_UUID)
        .map_err(|_| "the region uuid is not a uuid".to_string())?;
    let library = uuid::Uuid::parse_str("2a4880b6-b7a3-690a-2049-bfbe38eafb9f")
        .map_err(|_| "the library texture uuid is not a uuid".to_string())?;
    let owner_id = uuid::Uuid::parse_str(ESTATE_OWNER)
        .map_err(|_| "the estate owner uuid is not a uuid".to_string())?;
    let chat_id = uuid::Uuid::parse_str(CHAT_USER)
        .map_err(|_| "the chat user uuid is not a uuid".to_string())?;
    let probe = service_probe.to_owned();
    Ok(ferroan_bot::scenario::Harness {
        login_uri: login_uri.to_owned(),
        region_id,
        owner: ferroan_bot::scenario::Account {
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            password: password.to_owned(),
            principal_id: owner_id,
        },
        chat: ferroan_bot::scenario::Account {
            first_name: "Chat".to_string(),
            last_name: "Bot".to_string(),
            password: chat_password.to_owned(),
            principal_id: chat_id,
        },
        texture,
        library_texture: library,
        http_trace: ferroan_bot::trace::HttpTrace::new(),
        region_log: log.to_owned(),
        require_logout_reply: false,
        neighbour: None,
        service_probe: probe.clone(),
        metrics: None,
        before_login: std::sync::Arc::new(move |principal| {
            let probe = probe.clone();
            Box::pin(async move {
                crate::grid::login_probe::await_settled(&probe, &principal.to_string()).await
            })
        }),
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn texture_bytes() -> Result<Vec<u8>, String> {
    let path = source_dir().join(
        "Source/OpenSim.Services.InventoryService/AppData/assets/TexturesAssetSet/rainbow.j2c",
    );
    let bytes =
        fs::read(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(format!("{} is larger than 2 MiB", path.display()));
    }
    if bytes.len() < 2 || bytes[0] != 0xff || bytes[1] != 0x4f {
        return Err(format!("{} is not a JPEG 2000 codestream", path.display()));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn create_chat_user(started: &mut Started, password: &str) -> Result<(), String> {
    let stdin = started
        .stdin
        .as_mut()
        .ok_or("the region was started without a console")?;
    let prompt_deadline = Instant::now() + Duration::from_secs(60);
    while !log_contains(&started.log, "# ")? {
        if log_contains(&started.log, "Cannot see if a key has been pressed")? {
            return Err(
                "the region console cannot read a pipe; --console basic was not applied"
                    .to_string(),
            );
        }
        if Instant::now() >= prompt_deadline {
            return Err(format!(
                "the region console did not prompt\n{}",
                log_tail(&started.log, &started.password)
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
    // The handler prompts for a model name when that argument is absent.
    // An empty answer keeps the default appearance.
    let command = format!("create user Chat Bot {password} chat@example.invalid {CHAT_USER}\n\n");
    use std::io::Write;
    stdin
        .write_all(command.as_bytes())
        .map_err(|error| format!("could not write the create user command: {error}"))?;
    stdin
        .flush()
        .map_err(|error| format!("could not write the create user command: {error}"))?;
    let needle = format!("Account Chat Bot {CHAT_USER} created successfully");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !log_contains(&started.log, &needle)? {
        if Instant::now() >= deadline {
            return Err(format!(
                "create user did not finish\n{}",
                log_tail_secrets(&started.log, &[password, &started.password])
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
    if log_contains(&started.log, password)? {
        return Err("the region log contains the chat password".to_string());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn stop_started(started: &mut Started) -> Result<(), String> {
    let pid = started.child.id();
    drop(started.stdin.take());
    if region_pid_live(pid) {
        stop_child(&mut started.child);
    } else {
        let _ = started.child.wait();
    }
    let _ = fs::remove_file(state_path());
    let tracked = started.tracked.forget();
    let proxy = started.proxy.take().map_or(Ok(()), |proxy| proxy.finish());
    match (tracked, proxy) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(proxy)) => Err(format!("{error}\n{proxy}")),
    }
}

#[cfg(target_os = "linux")]
fn start_region(
    console: bool,
    asset_uri: Option<&str>,
    record: bool,
    basic_physics: bool,
) -> Result<Started, String> {
    start_region_with_proxy(console, asset_uri, record, false, basic_physics)
}

#[cfg(target_os = "linux")]
fn start_region_with_proxy(
    console: bool,
    asset_uri: Option<&str>,
    record: bool,
    proxy_http: bool,
    basic_physics: bool,
) -> Result<Started, String> {
    let binary = region_binary();
    if !binary.is_file() {
        return Err("run cargo xtask live build first".to_string());
    }
    // Top-level callers claim process state before starting any child.
    let publish = publish_dir();
    require_under_scratch(&publish)?;
    copy_examples(&publish)?;
    delete_databases(&publish)?;
    let public_port = crate::ports::alloc(crate::ports::Slot::StandalonePublic)?;
    let private_port = crate::ports::alloc(crate::ports::Slot::StandalonePrivate)?;
    let password = generate_password()?;
    let run = run_dir();
    let regions = run.join("Regions");
    let config = run.join("config");
    fs::create_dir_all(&regions)
        .map_err(|error| format!("could not create {}: {error}", regions.display()))?;
    fs::create_dir_all(&config)
        .map_err(|error| format!("could not create {}: {error}", config.display()))?;
    crate::ports::write_fixture_ports(&run)?;
    let capture = capture_path();
    // The override must sit outside the ini directory. Files in that directory
    // are loaded before Standalone.ini is pulled in by Include-Architecture,
    // and Standalone.ini sets LocalGridAssetService back to the local service.
    let _ = fs::remove_file(config.join("AssetOverride.ini"));
    let asset_override = run.join("AssetOverride.ini");
    let asset_include = if let Some(uri) = asset_uri {
        if uri.contains(['"', '\n', '\r']) {
            return Err("the asset service URL contains a quote or a line break".to_string());
        }
        // Included after Standalone.ini, which would otherwise keep the
        // in-process asset service. Nini replaces keys from a later file.
        let body = format!(
            "[AssetService]\n    \
             LocalGridAssetService = \"OpenSim.Services.Connectors.dll:AssetServicesConnector\"\n    \
             AssetServerURI = \"{uri}\"\n"
        );
        write_secret(&asset_override, &body)?;
        Some(asset_override.as_path())
    } else {
        let _ = fs::remove_file(&asset_override);
        None
    };
    let server_ini = region_server_ini(&StandaloneIni {
        regions_dir: &regions,
        capture_file: record.then_some(capture.as_path()),
        public_port,
        private_port,
        password: &password,
        asset_include,
        http_bind: if proxy_http {
            Ipv4Addr::new(127, 0, 0, 2)
        } else {
            Ipv4Addr::LOCALHOST
        },
        basic_physics,
    })?;
    write_secret(&config.join("RegionServer.ini"), &server_ini)?;
    fs::write(regions.join("Regions.ini"), regions_ini(public_port))
        .map_err(|error| format!("could not write Regions.ini: {error}"))?;
    let log = run.join("region.log");
    let proxy = if proxy_http {
        Some(crate::grid::start_proxies(&[(
            std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, public_port)),
            std::net::SocketAddr::from((Ipv4Addr::new(127, 0, 0, 2), public_port)),
            run.join("ferrobot-http.cap"),
        )])?)
    } else {
        None
    };
    println!("live: starting the region on 127.0.0.1:{public_port}");
    let (mut child, tracked) = spawn_region(&binary, &publish, &config, &log, console)?;
    let waited = match wait_region(&mut child, &log, START_TIMEOUT) {
        Ok(waited) => waited,
        Err(error) => {
            stop_child(&mut child);
            tracked.forget()?;
            return Err(error);
        }
    };
    match waited {
        Waited::Ready => {
            if basic_physics
                && let Err(error) = require_basic_physics(&log, &publish, "Ferroan Test")
            {
                stop_child(&mut child);
                tracked.forget()?;
                return Err(error);
            }
            let mut started = finish_start(
                child,
                tracked,
                &log,
                public_port,
                private_port,
                &password,
                proxy_http,
            )?;
            started.proxy = proxy;
            Ok(started)
        }
        Waited::Timeout => {
            stop_child(&mut child);
            tracked.forget()?;
            Err(format!(
                "the region did not enable logins\n{}",
                log_tail(&log, &password)
            ))
        }
        Waited::Exited(code) => Err(format!(
            "the region exited ({code}) before logins were enabled\n{}",
            log_tail(&log, &password)
        )),
    }
}

#[cfg(target_os = "linux")]
fn finish_start(
    mut child: std::process::Child,
    tracked: crate::grid_state::Tracked,
    log: &Path,
    public_port: u16,
    private_port: u16,
    password: &str,
    proxy_http: bool,
) -> Result<Started, String> {
    let pid = child.id();
    let checked = process_sockets(pid).and_then(|sockets| {
        if proxy_http {
            bound_report(&sockets, public_port, true, Ipv4Addr::new(127, 0, 0, 2))
        } else {
            loopback_report(&sockets, public_port)
        }
    });
    if let Err(error) = checked {
        stop_child(&mut child);
        tracked.forget()?;
        return Err(format!("{error}\n{}", log_tail(log, password)));
    }
    if let Err(error) = write_state(
        &state_path(),
        &RunState {
            pid,
            public_port,
            private_port,
            assets: None,
        },
    ) {
        stop_child(&mut child);
        tracked.forget()?;
        return Err(error);
    }
    let stdin = child.stdin.take();
    Ok(Started {
        child,
        stdin,
        public_port,
        password: password.to_owned(),
        log: log.to_owned(),
        tracked,
        proxy: None,
    })
}

#[cfg(target_os = "linux")]
fn down_linux() -> Result<String, String> {
    // An active controller owns standalone and asset children too. Do not
    // bypass its refusal through the older standalone state below. A grid-state
    // error must still run standalone cleanup, and both errors are reported.
    // A port-base run keeps its state under `.scratch/live/runs/<base>-<pid>/`.
    report_shutdown(down_every_grid(), down_every_standalone())
}

#[cfg(target_os = "linux")]
fn down_every_grid() -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = crate::grid_state::down() {
        errors.push(error);
    }
    let runs = live_root().join("runs");
    if let Ok(entries) = fs::read_dir(&runs) {
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path().join("grid-state");
            if path.is_file()
                && let Err(error) = crate::grid_state::down_file(&path)
            {
                errors.push(error);
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(target_os = "linux")]
fn down_every_standalone() -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = down_standalone() {
        errors.push(error);
    }
    let runs = live_root().join("runs");
    if let Ok(entries) = fs::read_dir(&runs) {
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let run = entry.path().join("standalone");
            let state = run.join("state");
            if state.is_file()
                && let Err(error) = down_standalone_at(&state, &run)
            {
                errors.push(error);
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(target_os = "linux")]
fn report_shutdown(
    grid: Result<(), String>,
    standalone: Result<(), String>,
) -> Result<String, String> {
    match (grid, standalone) {
        (Ok(()), Ok(())) => Ok("live: down".to_string()),
        (Err(grid), Err(standalone)) => Err(format!("{grid}\n{standalone}")),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn down_standalone() -> Result<(), String> {
    down_standalone_at(&state_path(), &run_dir())
}

#[cfg(target_os = "linux")]
fn down_standalone_at(state_file: &Path, run: &Path) -> Result<(), String> {
    let Some(state) = read_state(state_file)? else {
        return Ok(());
    };
    let region_result = stop_pid(state.pid);
    let asset_result = state.assets.as_ref().map_or(Ok(()), |asset| {
        crate::assets::stop_persistent(asset.pid, asset.start_time)
    });
    let cleanup = match (region_result, asset_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(region), Err(asset)) => Err(format!("{region}\n{asset}")),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
    };
    if cleanup.is_ok() {
        let _ = fs::remove_file(state_file);
        let _ = fs::remove_file(run.join("password"));
    }
    cleanup
}

#[cfg(all(test, target_os = "linux"))]
fn shutdown_at(grid: &Path, standalone: &Path, run: &Path) -> Result<String, String> {
    report_shutdown(
        crate::grid_state::down_file(grid),
        down_standalone_at(standalone, run),
    )
}

pub(crate) fn read_pin() -> Result<String, String> {
    let path = repo_root().join("docs/REFERENCES.md");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    pinned_tranquillity(&text)
}

#[cfg(target_os = "linux")]
fn tranquillity_reference() -> Result<PathBuf, String> {
    let root = match env::var("FERROAN_REF") {
        Ok(value) if !value.is_empty() => expand_user(&value),
        _ => home_dir()?.join("Git/reference"),
    };
    let checkout = root.join("OpenSim-Tranquillity");
    if !checkout.join(".git").exists() && !checkout.join("Source").is_dir() {
        return Err(format!(
            "missing {} (FERROAN_REF={})",
            checkout.display(),
            root.display()
        ));
    }
    Ok(checkout)
}

#[cfg(target_os = "linux")]
fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "set FERROAN_REF, or HOME, to find ~/Git/reference".to_string())
}

#[cfg(target_os = "linux")]
fn expand_user(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

#[cfg(target_os = "linux")]
fn require_clean_pin(reference: &Path, pin: &str) -> Result<(), String> {
    let head = git(reference, &["rev-parse", "HEAD"])?;
    if head != pin {
        return Err(format!(
            "OpenSim-Tranquillity is at {head}, docs/REFERENCES.md pins {pin}"
        ));
    }
    let status = git(reference, &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Err(format!(
            "OpenSim-Tranquillity checkout is not clean; the harness only reads it:\n{status}"
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn prepare_source(reference: &Path, pin: &str) -> Result<PathBuf, String> {
    let src = source_dir();
    require_under_scratch(&src)?;
    if src.join(".git").is_dir() {
        let canonical = src.canonicalize().map_err(|error| error.to_string())?;
        let reference = reference
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if canonical == reference {
            return Err("refusing to modify the reference checkout".to_string());
        }
        if git(&src, &["cat-file", "-e", &format!("{pin}^{{commit}}")]).is_err() {
            fs::remove_dir_all(&src)
                .map_err(|error| format!("could not replace {}: {error}", src.display()))?;
        } else {
            return Ok(src);
        }
    } else if src.exists() {
        fs::remove_dir_all(&src)
            .map_err(|error| format!("could not replace {}: {error}", src.display()))?;
    }
    println!("live: cloning Tranquillity {pin}");
    let parent = src.parent().ok_or("scratch source has no parent")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let status = Command::new("git")
        .args(["clone", "--no-checkout"])
        .arg(reference)
        .arg(&src)
        .status()
        .map_err(|error| format!("could not run git clone: {error}"))?;
    if !status.success() {
        return Err("git clone of Tranquillity failed".to_string());
    }
    Ok(src)
}

#[cfg(target_os = "linux")]
fn reset_source(src: &Path, pin: &str) -> Result<(), String> {
    require_under_scratch(src)?;
    let canonical = src.canonicalize().map_err(|error| error.to_string())?;
    if canonical
        == tranquillity_reference()?
            .canonicalize()
            .map_err(|error| error.to_string())?
    {
        return Err("refusing to modify the reference checkout".to_string());
    }
    git(src, &["checkout", "--detach", pin])?;
    git(src, &["reset", "--hard", pin])?;
    git(src, &["clean", "-fdx"])?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn git_apply(src: &Path, patch: &Path) -> Result<(), String> {
    require_under_scratch(src)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(src)
        .args(["apply", "--whitespace=nowarn"])
        .arg(patch)
        .output()
        .map_err(|error| format!("could not run git apply: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "could not apply {}\n{}",
            patch.display(),
            output_tail(&output)
        ))
    }
}

/// MSBuild properties for the host Tranquillity publish.
///
/// `NBGV_SetCloudBuildVersionVars=false` skips the target that appends to
/// `GITHUB_ENV`. Nerdbank.GitVersioning 3.10.94 still runs that target when
/// `GITHUB_ACTIONS` is true, and `File.AppendAllLines` throws if the path is
/// empty, so removing the variable is not enough on its own.
#[cfg(any(target_os = "linux", test))]
fn tranquillity_publish_properties() -> &'static [&'static str] {
    &[
        "-p:AllowMissingPrunePackageData=true",
        "-p:UseSharedCompilation=false",
        crate::dotnet_ci::CLOUD_BUILD_VERSION_VARS_OFF,
    ]
}

/// Environment of the host Tranquillity `dotnet publish`.
///
/// Nerdbank.GitVersioning appends version variables to `GITHUB_ENV` from
/// several MSBuild nodes at once. A torn line fails the Actions step after
/// the publish has already succeeded (dotnet/Nerdbank.GitVersioning#683).
/// Removing the variable leaves the runner's file untouched. The cloud-build
/// version target is also off, so the publish does not require the file.
/// `MSBUILDDISABLENODEREUSE` keeps a worker node from holding file locks
/// after this process exits.
#[cfg(any(target_os = "linux", test))]
fn configure_dotnet_publish(command: &mut Command) {
    crate::dotnet_ci::shield(command);
}

#[cfg(target_os = "linux")]
fn publish(src: &Path, project: &str, binary: &Path, self_contained: bool) -> Result<(), String> {
    require_under_scratch(src)?;
    // SDK 10.0.114 ships no PrunePackageData directory. GridServer's
    // FrameworkReference to Microsoft.AspNetCore.App then fails restore with
    // NETSDK1226. This is the SDK's own bypass. The project files stay untouched.
    let mut args = vec![
        "publish",
        project,
        "-c",
        "Release",
        "-r",
        "linux-x64",
        "--nologo",
    ];
    args.extend(tranquillity_publish_properties().iter().copied());
    if self_contained {
        args.push("--self-contained");
        args.push("true");
    }
    let mut command = crate::dotnet_ci::command("dotnet");
    configure_dotnet_publish(&mut command);
    let output = command
        .args(&args)
        .current_dir(src)
        .output()
        .map_err(|error| format!("could not run dotnet: {error}"))?;
    if output.status.success() && binary.is_file() {
        Ok(())
    } else {
        Err(format!(
            "dotnet publish {project} failed\n{}",
            output_tail(&output)
        ))
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn standalone_is_up() -> Result<Option<u32>, String> {
    if let Some(pid) = crate::grid_state::survivor()? {
        return Ok(Some(pid));
    }
    if let Some(state) = read_state(&state_path())? {
        if region_pid_live(state.pid) {
            return Ok(Some(state.pid));
        }
        if let Some(asset) = state.assets
            && crate::assets::persistent_pid_live(asset.pid, asset.start_time)
        {
            return Ok(Some(asset.pid));
        }
    }
    Ok(None)
}

fn copy_examples(publish: &Path) -> Result<(), String> {
    for name in ["StandaloneCommon.ini", "FlotsamCache.ini", "GridCommon.ini"] {
        let source = publish
            .join("config-include")
            .join(format!("{name}.example"));
        if name == "GridCommon.ini" && !source.is_file() {
            continue;
        }
        copy_example(publish, name)?;
    }
    Ok(())
}

pub(crate) fn copy_example(publish: &Path, name: &str) -> Result<(), String> {
    let dest = publish.join("config-include").join(name);
    if dest.is_file() {
        return Ok(());
    }
    let source = dest.with_file_name(format!("{name}.example"));
    fs::copy(&source, &dest).map(|_| ()).map_err(|error| {
        format!(
            "could not copy {} to {}: {error}",
            source.display(),
            dest.display()
        )
    })
}

#[cfg(target_os = "linux")]
fn delete_databases(publish: &Path) -> Result<(), String> {
    let entries = fs::read_dir(publish)
        .map_err(|error| format!("could not read {}: {error}", publish.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not read {}: {error}", publish.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_sqlite_file(&name) {
            fs::remove_file(entry.path())
                .map_err(|error| format!("could not remove {}: {error}", entry.path().display()))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn spawn_region(
    binary: &Path,
    publish: &Path,
    config: &Path,
    log: &Path,
    console: bool,
) -> Result<(std::process::Child, crate::grid_state::Tracked), String> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let log_file = File::create(log)
        .map_err(|error| format!("could not create {}: {error}", log.display()))?;
    let err_file = log_file
        .try_clone()
        .map_err(|error| format!("could not open {}: {error}", log.display()))?;
    let config_arg = config.to_str().ok_or("region config path is not UTF-8")?;
    let mut args = vec!["--inidirectory", config_arg];
    let mut command = Command::new(binary);
    command.current_dir(publish);
    if console {
        // Foreground keeps the console prompt, so `live run` can create the
        // second account. `--console` defaults to local, which reads keys and
        // throws when stdin is a pipe, and that default is applied after the
        // ini. `basic` uses ReadLine. `--background` is what `live up` uses.
        args.extend(["--console", "basic"]);
        command.stdin(Stdio::piped());
    } else {
        args.push("--background");
        command.stdin(Stdio::null());
    }
    command
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(err_file));
    command.args(&args).process_group(0);
    spawn_tracked(&mut command, binary, &args, publish)
}

enum Waited {
    Ready,
    Exited(String),
    Timeout,
}

#[cfg(target_os = "linux")]
fn wait_region(
    child: &mut std::process::Child,
    log: &Path,
    timeout: Duration,
) -> Result<Waited, String> {
    let started = Instant::now();
    loop {
        if log_contains(log, READY)? {
            return Ok(Waited::Ready);
        }
        match child
            .try_wait()
            .map_err(|error| format!("could not wait for the region: {error}"))?
        {
            Some(status) => {
                let code = status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                return Ok(Waited::Exited(code));
            }
            None if started.elapsed() >= timeout => return Ok(Waited::Timeout),
            None => thread::sleep(Duration::from_millis(500)),
        }
    }
}

/// A `--physics basicphysics` region has started that engine. The scene name
/// `basicphysics/<region>` is enough. Otherwise the log must name basicphysics
/// and must not show the ubODE library line. The default harness does not call this.
pub(crate) fn basic_physics_log_ok(text: &str, region: &str) -> bool {
    if text.contains(&format!("basicphysics/{region}")) {
        return true;
    }
    text.contains("basicphysics") && !text.contains("ode library configuration")
}

pub(crate) fn require_basic_physics(log: &Path, cwd: &Path, region: &str) -> Result<(), String> {
    let text = physics_log_text(log, cwd)?;
    if basic_physics_log_ok(&text, region) {
        Ok(())
    } else {
        Err(format!(
            "--physics basicphysics did not select basicphysics for {region}"
        ))
    }
}

fn physics_log_text(log: &Path, cwd: &Path) -> Result<String, String> {
    let mut text = read_capped(log)?;
    let mut dirs = Vec::new();
    if let Some(parent) = log.parent() {
        dirs.push(parent.to_path_buf());
    }
    if cwd != log.parent().unwrap_or(Path::new("")) {
        dirs.push(cwd.to_path_buf());
    }
    for dir in dirs {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("could not read {}: {error}", dir.display()));
            }
        };
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("could not read {}: {error}", dir.display()))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("OpenSim.Server.RegionServer") && name.ends_with(".log") {
                text.push('\n');
                text.push_str(&read_capped(&entry.path())?);
            }
        }
    }
    Ok(text)
}

fn read_capped(path: &Path) -> Result<String, String> {
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > 32 * 1024 * 1024 {
                return Err(format!("{} is larger than 32 MiB", path.display()));
            }
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn log_contains(path: &Path, needle: &str) -> Result<bool, String> {
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > 32 * 1024 * 1024 {
                return Err(format!("{} is larger than 32 MiB", path.display()));
            }
            Ok(String::from_utf8_lossy(&bytes).contains(needle))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn log_tail(path: &Path, password: &str) -> String {
    log_tail_secrets(path, &[password])
}

#[cfg(target_os = "linux")]
pub(crate) fn log_tail_secrets(path: &Path, passwords: &[&str]) -> String {
    let Ok(text) = read_bounded_text(path, 64 * 1024 * 1024) else {
        return format!("(log unavailable or over 64 MiB at {})", path.display());
    };
    redacted_tail(&text, passwords)
}

#[cfg(target_os = "linux")]
pub(crate) fn require_listen(pid: u32, ip: Ipv4Addr, port: u16) -> Result<(), String> {
    let sockets = process_sockets(pid)?;
    refuse_unspecified(&sockets)?;
    if sockets
        .iter()
        .any(|socket| socket.listening && socket.local == IpAddr::V4(ip) && socket.port == port)
    {
        Ok(())
    } else {
        Err(format!("expected a TCP listen on {ip}:{port}"))
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn require_udp(pid: u32, ip: Ipv4Addr, port: u16) -> Result<(), String> {
    let sockets = process_sockets(pid)?;
    refuse_unspecified(&sockets)?;
    if sockets
        .iter()
        .any(|socket| socket.udp && socket.local == IpAddr::V4(ip) && socket.port == port)
    {
        Ok(())
    } else {
        Err(format!("expected UDP on {ip}:{port}"))
    }
}

#[cfg(target_os = "linux")]
fn refuse_unspecified(sockets: &[ProcSock]) -> Result<(), String> {
    if sockets.iter().any(|socket| socket.local.is_unspecified()) {
        Err("a harness socket is bound to every interface".to_string())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn process_sockets(pid: u32) -> Result<Vec<ProcSock>, String> {
    let inodes = socket_inodes(pid)?;
    let mut sockets = Vec::new();
    for name in ["tcp", "tcp6", "udp", "udp6"] {
        let path = PathBuf::from("/proc/net").join(name);
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let udp = name.starts_with("udp");
        for socket in parse_proc_net(&text, udp) {
            if inodes.contains(&socket.inode) {
                sockets.push(socket);
            }
        }
    }
    Ok(sockets)
}

#[cfg(target_os = "linux")]
fn socket_inodes(pid: u32) -> Result<std::collections::HashSet<u64>, String> {
    let dir = PathBuf::from(format!("/proc/{pid}/fd"));
    let mut inodes = std::collections::HashSet::new();
    let entries =
        fs::read_dir(&dir).map_err(|error| format!("could not read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read {}: {error}", dir.display()))?;
        let Ok(target) = fs::read_link(entry.path()) else {
            continue;
        };
        let text = target.to_string_lossy();
        if let Some(rest) = text.strip_prefix("socket:[")
            && let Some(number) = rest.strip_suffix(']')
            && let Ok(inode) = number.parse::<u64>()
        {
            inodes.insert(inode);
        }
    }
    Ok(inodes)
}

#[cfg(target_os = "linux")]
fn region_pid_live(pid: u32) -> bool {
    if process_start_time(pid).is_none() {
        return false;
    }
    let Ok(bytes) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let args: Vec<_> = bytes.split(|byte| *byte == 0).collect();
    let config = run_dir().join("config");
    args.first().is_some_and(|arg| {
        arg.rsplit(|byte| *byte == b'/').next() == Some(b"OpenSim.Server.RegionServer".as_slice())
    }) && args.windows(2).any(|pair| {
        pair[0] == b"--inidirectory" && pair[1] == config.as_os_str().as_encoded_bytes()
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn process_start_time(pid: u32) -> Option<u64> {
    if pid <= 1 {
        return None;
    }
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_process_start_time(&stat)
}

#[cfg(any(target_os = "linux", test))]
fn parse_process_start_time(stat: &str) -> Option<u64> {
    // The comm field can itself contain spaces and parentheses. Fields after
    // its final ')' begin with state (3); starttime is field 22.
    let (_, fields) = stat.rsplit_once(')')?;
    let mut fields = fields.split_whitespace();
    if matches!(fields.next()?, "Z" | "X") {
        return None;
    }
    fields.nth(18)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn stop_child(child: &mut std::process::Child) {
    // This handle owns a child launched as its process-group leader. Unlike
    // stop_pid (which validates a saved standalone region PID), this cleanup
    // also serves GridServer and ferroan-robust startup failures. An unreaped
    // child PID cannot be reused for an unrelated process.
    if !matches!(child.try_wait(), Ok(None)) {
        return;
    }
    let pid = child.id();
    let _ = crate::signal::signal_group(crate::signal::Signal::Term, pid);
    let started = Instant::now();
    while started.elapsed() < STOP_TIMEOUT {
        match child.try_wait() {
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Ok(Some(_)) | Err(_) => return,
        }
    }
    let _ = crate::signal::signal_group(crate::signal::Signal::Kill, pid);
    // Also target the owned child if signaling its group failed. Preserve its
    // recorded identity if even KILL cannot finish it within this bound.
    let _ = child.kill();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        match child.try_wait() {
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Ok(Some(_)) | Err(_) => return,
        }
    }
}

#[cfg(target_os = "linux")]
fn stop_pid(pid: u32) -> Result<(), String> {
    if !region_pid_live(pid) {
        return Ok(());
    }
    let _ = crate::signal::signal_group(crate::signal::Signal::Term, pid);
    let started = Instant::now();
    while region_pid_live(pid) && started.elapsed() < STOP_TIMEOUT {
        thread::sleep(Duration::from_millis(200));
    }
    if region_pid_live(pid) {
        let _ = crate::signal::signal_group(crate::signal::Signal::Kill, pid);
    }
    let started = Instant::now();
    while region_pid_live(pid) && started.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(200));
    }
    if region_pid_live(pid) {
        Err(format!("pid {pid} did not exit"))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn generate_password() -> Result<String, String> {
    let mut bytes = [0u8; PASSWORD_BYTES];
    let mut file = File::open("/dev/urandom")
        .map_err(|error| format!("could not read /dev/urandom: {error}"))?;
    std::io::Read::read_exact(&mut file, &mut bytes)
        .map_err(|error| format!("could not read /dev/urandom: {error}"))?;
    Ok(hex_encode(&bytes))
}

fn saved_password(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() != PASSWORD_BYTES * 2 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err("the saved harness password is invalid".to_string());
    }
    String::from_utf8(bytes).map_err(|_| "the saved harness password is invalid".to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(target_os = "linux")]
pub(crate) fn write_secret(path: &Path, text: &str) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not restrict {}: {error}", path.display()))?;
    file.set_len(0)
        .map_err(|error| format!("could not truncate {}: {error}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn require_under_scratch(path: &Path) -> Result<(), String> {
    let scratch = live_root();
    fs::create_dir_all(&scratch)
        .map_err(|error| format!("could not create {}: {error}", scratch.display()))?;
    let scratch = scratch
        .canonicalize()
        .map_err(|error| format!("could not resolve {}: {error}", scratch.display()))?;
    let resolved = if path.exists() {
        path.canonicalize()
            .map_err(|error| format!("could not resolve {}: {error}", path.display()))?
    } else if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        let parent = parent
            .canonicalize()
            .map_err(|error| format!("could not resolve {}: {error}", parent.display()))?;
        let name = path
            .file_name()
            .ok_or_else(|| format!("{} has no file name", path.display()))?;
        parent.join(name)
    } else {
        path.to_path_buf()
    };
    if resolved.starts_with(&scratch) {
        Ok(())
    } else {
        Err(format!(
            "refusing to change {}; the live harness only writes under .scratch/live",
            resolved.display()
        ))
    }
}

#[cfg(target_os = "linux")]
fn file_hash(path: &Path) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .map_err(|error| format!("could not run sha256sum: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "sha256sum failed for {}\n{}",
            path.display(),
            output_tail(&output)
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .next()
        .map(str::to_string)
        .filter(|hash| hash.len() == 64)
        .ok_or_else(|| format!("sha256sum returned nothing for {}", path.display()))
}

#[cfg(target_os = "linux")]
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "git {} failed in {}\n{}",
            args.join(" "),
            dir.display(),
            output_tail(&output)
        ))
    }
}

#[cfg(target_os = "linux")]
fn output_tail(output: &Output) -> String {
    let mut text = String::new();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stderr.is_empty() {
        text.push_str(stderr.trim());
    }
    if !stdout.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(stdout.trim());
    }
    if text.len() > 4000 {
        let mut start = text.len() - 4000;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        text = text[start..].to_string();
    }
    if text.is_empty() {
        format!("exit {}", output.status)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn owned_child_cleanup_reaps_a_non_region_process_group() {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new("/bin/sleep")
            .arg("60")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id();
        stop_child(&mut child);
        assert!(child.try_wait().unwrap().is_some());
        assert!(process_start_time(pid).is_none());
        // Repeated cleanup of a reaped handle never signals a reused PID.
        stop_child(&mut child);
    }

    #[test]
    fn tranquillity_publish_environment_has_no_github_env() {
        let mut command = Command::new("true");
        command.env("GITHUB_ENV", "/runner/set_env");
        configure_dotnet_publish(&mut command);
        let mut saw_github_env = false;
        let mut saw_telemetry = false;
        let mut saw_nologo = false;
        let mut saw_node_reuse = false;
        for (key, value) in command.get_envs() {
            if key == "GITHUB_ENV" {
                saw_github_env = true;
                assert!(value.is_none());
            } else if key == "DOTNET_CLI_TELEMETRY_OPTOUT" {
                saw_telemetry = value == Some(std::ffi::OsStr::new("1"));
            } else if key == "DOTNET_NOLOGO" {
                saw_nologo = value == Some(std::ffi::OsStr::new("1"));
            } else if key == "MSBUILDDISABLENODEREUSE" {
                saw_node_reuse = value == Some(std::ffi::OsStr::new("1"));
            }
        }
        assert!(saw_github_env);
        assert!(saw_telemetry);
        assert!(saw_nologo);
        assert!(saw_node_reuse);
        assert!(
            tranquillity_publish_properties().contains(&"-p:NBGV_SetCloudBuildVersionVars=false")
        );
    }

    #[test]
    fn cargo_artifact_selects_the_built_binary_in_a_custom_target_directory() {
        let messages = br#"{"reason":"compiler-artifact","target":{"name":"ferroan-robust","kind":["lib"]},"executable":null}
{"reason":"compiler-artifact","target":{"name":"ferroan-robust","kind":["bin"]},"executable":"/custom/build/debug/ferroan-robust"}
{"reason":"build-finished","success":true}
"#;
        assert_eq!(
            artifact_executable(messages, "ferroan-robust").unwrap(),
            PathBuf::from("/custom/build/debug/ferroan-robust")
        );
        assert!(artifact_executable(messages, "ferroan-region").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rewriting_a_secret_restricts_existing_permissions_before_writing() {
        use std::os::unix::fs::PermissionsExt;
        let path = repo_root()
            .join(".scratch")
            .join(format!("secret-mode-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "old longer value").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_secret(&path, "new").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn the_region_ini_has_the_const_keys_and_the_report_omits_the_password() {
        let standalone = |capture, public_port, private_port, asset_include, basic_physics| {
            region_server_ini(&StandaloneIni {
                regions_dir: Path::new("/tmp/regions"),
                capture_file: capture,
                public_port,
                private_port,
                password: "0123456789abcdef",
                asset_include,
                http_bind: Ipv4Addr::LOCALHOST,
                basic_physics,
            })
            .unwrap()
        };
        let ini = standalone(
            Some(Path::new("/tmp/regions/capture-udp.cap")),
            9100,
            9103,
            None,
            false,
        );
        assert!(ini.contains("BaseURL = http://${Const|BaseHostname}"));
        assert!(ini.contains("CaptureFile = \"/tmp/regions/capture-udp.cap\""));
        assert!(ini.contains("ServiceProbe = true"));
        let quiet = standalone(None, 9100, 9103, None, false);
        assert!(!quiet.contains("CaptureFile"));
        assert!(quiet.contains("ServiceProbe = true"));
        let pointed = standalone(
            None,
            9100,
            9103,
            Some(Path::new("/tmp/regions/AssetOverride.ini")),
            false,
        );
        assert!(pointed.contains("Include-Asset = \"/tmp/regions/AssetOverride.ini\""));
        assert!(
            pointed.find("Include-Architecture").unwrap() < pointed.find("Include-Asset").unwrap()
        );
        assert!(ini.contains("PrivURL = ${Const|BaseURL}"));
        assert!(ini.contains("PrivatePort = \"9103\""));
        assert!(ini.contains("bind_ip_address = \"127.0.0.1\""));
        assert!(ini.contains(ESTATE_OWNER));
        assert!(ini.contains("DefaultEstateOwnerPassword = \"0123456789abcdef\""));
        assert!(ini.contains("regionload_regionsdir = \"/tmp/regions\""));
        assert!(!ini.contains("physics ="));
        assert!(basic_physics_log_ok(
            "Adding scene \"Ferroan Test\" to non-shared module \"basicphysics\"\n",
            "Ferroan Test"
        ));
        assert!(basic_physics_log_ok(
            "physics scene basicphysics/Ferroan Test\n[ubODE] ode library configuration\n",
            "Ferroan Test"
        ));
        assert!(!basic_physics_log_ok(
            "[ubODE] ode library configuration\nAdding scene to module \"basicphysics\"\n",
            "Ferroan Test"
        ));
        assert!(!basic_physics_log_ok("region ready\n", "Ferroan Test"));
        let basic = standalone(None, 9100, 9103, None, true);
        assert!(basic.contains("physics = basicphysics"));
        assert!(basic.find("physics = basicphysics").unwrap() < basic.find("[Network]").unwrap());
        assert!(basic.find("[Startup]").unwrap() < basic.find("physics = basicphysics").unwrap());
        let home = 25_000 + crate::ports::Slot::Home as u16;
        let private = 25_000 + crate::ports::Slot::StandalonePrivate as u16;
        let moved = standalone(None, home, private, None, false);
        let regions = regions_ini(home);
        let rendered = format!("{moved}{regions}");
        assert!(
            crate::ports::ports_outside(&rendered, &[home, private]).is_empty(),
            "{rendered}"
        );
        let hardcoded = format!("{rendered}\nhttp_listener_port = 9000\n");
        assert_eq!(
            crate::ports::ports_outside(&hardcoded, &[home, private]),
            vec![9000]
        );
        assert!(!crate::ports::isolated());
        assert_eq!(run_dir(), live_root().join("run"));
        let report = up_report(9100, Path::new(".scratch/live/run/region.log"));
        assert!(report.contains("login: http://127.0.0.1:9100/"));
        assert!(!report.contains("0123456789abcdef"));
        assert_eq!(
            redact_secrets("password 0123456789abcdef done", &["0123456789abcdef"]),
            "password <redacted> done"
        );
    }

    #[test]
    fn passwords_across_the_log_tail_cut_are_redacted_before_slicing() {
        let password = "synthetic-password-crossing-the-cut";
        let other = "second-synthetic-secret";
        // The old last-4000-bytes slice starts halfway through the password.
        let text = format!(
            "{}{}{}{}",
            "prefix".repeat(1000),
            password,
            "x".repeat(3950),
            other
        );
        let raw_tail = &text[text.len() - 4000..];
        assert!(!raw_tail.contains(password));
        assert!(raw_tail.contains("crossing-the-cut"));
        let tail = redacted_tail(&text, &[password, other]);
        assert!(!tail.contains("crossing-the-cut"));
        assert!(!tail.contains(other));
        assert!(tail.contains("<redacted>"));
        assert!(tail.len() <= 4000);
        let unicode = redacted_tail(&"€".repeat(2000), &[]);
        assert!(unicode.len() <= 4000);
        assert!(!unicode.contains('\u{fffd}'));
    }

    #[test]
    fn proc_addresses_match_the_kernel_byte_order() {
        assert_eq!(
            parse_proc_addr("0100007F:0050"),
            Some((IpAddr::V4(Ipv4Addr::LOCALHOST), 80))
        );
        assert_eq!(
            parse_proc_addr("00000000:1F90"),
            Some((IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8080))
        );
        assert_eq!(
            parse_proc_addr("00000000000000000000000001000000:0050"),
            Some((IpAddr::V6(Ipv6Addr::LOCALHOST), 80))
        );
    }

    #[test]
    fn an_unspecified_listen_is_rejected() {
        let sockets = parse_proc_net(
            "  sl local rem st tx:rx tr tm retr uid timeout inode\n   0: 00000000:238C 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 10 1 0 0 0\n",
            false,
        );
        let error = loopback_report(&sockets, 9100).unwrap_err();
        assert!(error.contains("not on 127.0.0.1"));
    }

    #[test]
    fn loopback_tcp_and_udp_are_accepted() {
        let header = "  sl local rem st tx:rx tr tm retr uid timeout inode\n";
        let tcp = format!(
            "{header}   0: 0100007F:238C 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 10 1 0 0 0\n"
        );
        let udp = format!(
            "{header}   1: 0100007F:238C 00000000:0000 07 00000000:00000000 00:00000000 00000000 0 0 11 1 0 0 0\n"
        );
        let mut sockets = parse_proc_net(&tcp, false);
        sockets.extend(parse_proc_net(&udp, true));
        assert!(loopback_report(&sockets, 0x238c).is_ok());
        let tcp_only = parse_proc_net(&tcp, false);
        assert!(http_loopback_report(&tcp_only, 0x238c).is_ok());
        assert!(loopback_report(&tcp_only, 0x238c).is_err());
    }

    #[test]
    fn the_pin_row_is_read_from_the_references_table() {
        let markdown = "| `OpenSim-Tranquillity` | develop | `713857f10b8e91a639a967d4354a4682b080793c` | notes |\n";
        assert_eq!(
            pinned_tranquillity(markdown).unwrap(),
            "713857f10b8e91a639a967d4354a4682b080793c"
        );
    }

    #[test]
    fn state_round_trips() {
        let dir = std::env::temp_dir().join(format!("ferroan-live-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("state");
        write_state(
            &path,
            &RunState {
                pid: 42,
                public_port: 9100,
                private_port: 9103,
                assets: None,
            },
        )
        .unwrap();
        let state = read_state(&path).unwrap().unwrap();
        assert_eq!(state.pid, 42);
        assert_eq!(state.public_port, 9100);
        assert_eq!(state.private_port, 9103);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn persistent_state_requires_a_complete_safe_asset_identity() {
        let dir = repo_root()
            .join(".scratch")
            .join(format!("state-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("state");
        let state = RunState {
            pid: 42,
            public_port: 9100,
            private_port: 9103,
            assets: Some(AssetState {
                pid: 43,
                port: 9200,
                start_time: 987654,
            }),
        };
        write_state(&path, &state).unwrap();
        let parsed = read_state(&path).unwrap().unwrap();
        let asset = parsed.assets.unwrap();
        assert_eq!(
            (asset.pid, asset.port, asset.start_time),
            (43, 9200, 987654)
        );
        for fields in [
            "asset_pid=43\n",
            "asset_pid=0\nasset_start_time=987654\nasset_port=9200\n",
            "asset_pid=43\nasset_start_time=987654\nasset_port=65536\n",
        ] {
            fs::write(
                &path,
                format!("pid=42\npublic_port=9100\nprivate_port=9103\n{fields}"),
            )
            .unwrap();
            assert!(read_state(&path).is_err());
        }
        fs::write(&path, "pid=0\npublic_port=9100\nprivate_port=9103\n").unwrap();
        assert!(read_state(&path).is_err());
        fs::write(&path, "x".repeat(4097)).unwrap();
        assert!(read_state(&path).err().unwrap().contains("size limit"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn process_identity_handles_comm_parentheses_and_excludes_zombies() {
        let tail = format!("{}987654 0", "0 ".repeat(18));
        assert_eq!(
            parse_process_start_time(&format!("42 (name ) with spaces) S {tail}")),
            Some(987654)
        );
        assert_eq!(
            parse_process_start_time(&format!("42 (name) Z {tail}")),
            None
        );
        assert_eq!(parse_process_start_time("42 (broken) S 0"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_down_cleans_standalone_after_a_grid_state_stop_error() {
        let root = repo_root()
            .join(".scratch")
            .join(format!("down-both-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        // Not a process-group leader: stop must refuse before any signal.
        // /proc cmdline can be empty for a moment after exec.
        let child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        struct Reap(std::process::Child);
        impl Drop for Reap {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut child = Reap(child);
        let grid = root.join("grid-state");
        let pid = child.0.id();
        let visible = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "sleep fixture exited before its identity was recorded"
            );
            match crate::grid_state::write_ungrouped_stop_failure(&grid, pid) {
                Ok(()) => break,
                Err(error) if error == "the process has exited" && Instant::now() < visible => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("{error}"),
            }
        }
        let standalone = root.join("state");
        let run = root.join("run");
        fs::create_dir_all(&run).unwrap();
        write_state(
            &standalone,
            &RunState {
                pid: u32::MAX - 1,
                public_port: 1,
                private_port: 2,
                assets: None,
            },
        )
        .unwrap();
        fs::write(run.join("password"), b"not-a-grid-secret").unwrap();
        let error = shutdown_at(&grid, &standalone, &run).unwrap_err();
        assert!(error.contains("group leader"), "{error}");
        assert!(!standalone.exists(), "standalone state was not removed");
        assert!(!run.join("password").exists());
        assert!(
            grid.exists(),
            "a failed grid stop keeps its state for a retry"
        );
        drop(child);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_generated_password_can_be_saved_and_invalid_secrets_are_not_reported() {
        let password = hex_encode(&[0xab; PASSWORD_BYTES]);
        assert_eq!(
            saved_password(password.clone().into_bytes()).unwrap(),
            password
        );
        for bytes in [
            b"private-invalid-password".to_vec(),
            vec![b'g'; PASSWORD_BYTES * 2],
            vec![b'a'; PASSWORD_BYTES * 2 + 1],
        ] {
            assert_eq!(
                saved_password(bytes).unwrap_err(),
                "the saved harness password is invalid"
            );
        }
    }
}
