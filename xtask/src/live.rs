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
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};
use std::thread;
use std::time::{Duration, Instant};

use crate::repo_root;

const READY: &str = "INITIALIZATION COMPLETE FOR Ferroan Test - LOGINS ENABLED";
const ESTATE_OWNER: &str = "4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55";
const REGION_UUID: &str = "6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01";
const CHAT_USER: &str = "8c4e1a70-2b59-4d6f-9e13-5a7c0d8b2f44";
const START_TIMEOUT: Duration = Duration::from_secs(180);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const ALTERS: &[&str] = &[
    "ALTER TABLE UserAccounts ADD COLUMN DisplayName VARCHAR(31) NULL;",
    "ALTER TABLE UserAccounts ADD COLUMN NameChanged INTEGER NULL DEFAULT 0;",
];

pub fn build() -> ExitCode {
    finish(build_inner())
}

pub fn up() -> ExitCode {
    finish(up_inner())
}

pub fn down() -> ExitCode {
    finish(down_inner())
}

pub fn run(scenario: &str) -> ExitCode {
    finish(run_inner(scenario))
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

fn up_inner() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        up_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
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

fn run_inner(scenario: &str) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        run_linux(scenario)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = scenario;
        Err("cargo xtask live runs on Linux".to_string())
    }
}

fn live_root() -> PathBuf {
    repo_root().join(".scratch").join("live")
}

fn source_dir() -> PathBuf {
    live_root().join("src")
}

fn run_dir() -> PathBuf {
    live_root().join("run")
}

fn publish_dir() -> PathBuf {
    source_dir().join("Source/OpenSim.Server.RegionServer/bin/Release/net10.0/linux-x64/publish")
}

fn region_binary() -> PathBuf {
    publish_dir().join("OpenSim.Server.RegionServer")
}

fn state_path() -> PathBuf {
    run_dir().join("state")
}

struct RunState {
    pid: u32,
    public_port: u16,
    private_port: u16,
}

fn write_state(path: &Path, state: &RunState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let text = format!(
        "pid={}\npublic_port={}\nprivate_port={}\n",
        state.pid, state.public_port, state.private_port
    );
    fs::write(path, text).map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn read_state(path: &Path) -> Result<Option<RunState>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let mut pid = None;
    let mut public_port = None;
    let mut private_port = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "pid" => pid = value.parse().ok(),
            "public_port" => public_port = value.parse().ok(),
            "private_port" => private_port = value.parse().ok(),
            _ => {}
        }
    }
    match (pid, public_port, private_port) {
        (Some(pid), Some(public_port), Some(private_port)) => Ok(Some(RunState {
            pid,
            public_port,
            private_port,
        })),
        _ => Err(format!("{} is missing pid or ports", path.display())),
    }
}

fn region_server_ini(
    regions_dir: &Path,
    public_port: u16,
    private_port: u16,
    password: &str,
) -> Result<String, String> {
    let dir = regions_dir
        .to_str()
        .ok_or("regions directory is not utf-8")?;
    if dir.contains(['"', '\n', '\r']) || password.contains(['"', '\n', '\r']) {
        return Err("a configuration value contains a quote or a line break".to_string());
    }
    if password.is_empty() || public_port == 0 || private_port == 0 || public_port == private_port {
        return Err("the region needs two different ports and a password".to_string());
    }
    Ok(format!(
        r#"[Const]
    BaseHostname = "127.0.0.1"
    BaseURL = http://${{Const|BaseHostname}}
    PublicPort = "{public_port}"
    PrivURL = ${{Const|BaseURL}}
    PrivatePort = "{private_port}"

[Startup]
    regionload_regionsdir = "{dir}"

[Network]
    http_listener_port = {public_port}

[Estates]
    DefaultEstateName = "Ferroan Test Estate"
    DefaultEstateOwnerName = "Test User"
    DefaultEstateOwnerEMail = "test@example.invalid"
    DefaultEstateOwnerUUID = "{ESTATE_OWNER}"
    DefaultEstateOwnerPassword = "{password}"

[Architecture]
    Include-Architecture = "config-include/Standalone.ini"
"#
    ))
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

fn up_report(port: u16, log: &Path, altered: bool) -> String {
    let sqlite = if altered {
        "\nsqlite: added DisplayName and NameChanged"
    } else {
        ""
    };
    format!(
        "live: up\nlogin: http://127.0.0.1:{port}/\nhttp_listen: 127.0.0.1:{port}\nlog: {}{sqlite}",
        log.display()
    )
}

fn redact(text: &str, password: &str) -> String {
    if password.is_empty() {
        text.to_string()
    } else {
        text.replace(password, "<redacted>")
    }
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
        if socket.listening
            && socket.local == IpAddr::V4(Ipv4Addr::LOCALHOST)
            && socket.port == port
        {
            http = true;
        }
        if socket.udp && socket.local == IpAddr::V4(Ipv4Addr::LOCALHOST) && socket.port == port {
            udp = true;
        }
    }
    if !http || !udp {
        return Err(format!(
            "expected TCP and UDP on 127.0.0.1:{port}, saw {}",
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
    let pin = read_pin()?;
    let reference = tranquillity_reference()?;
    require_clean_pin(&reference, &pin)?;
    let src = prepare_source(&reference, &pin)?;
    let capture = repo_root().join("tools/capture/tranquillity-capture.patch");
    let listen = repo_root().join("tools/live/tranquillity-listen.patch");
    let stamp_text = format!(
        "pin={pin}\ncapture={}\nlisten={}\n",
        file_hash(&capture)?,
        file_hash(&listen)?
    );
    let binary = region_binary();
    let stamp = live_root().join("stamp");
    if binary.is_file() && fs::read_to_string(&stamp).ok().as_deref() == Some(stamp_text.as_str()) {
        return Ok(format!(
            "live: build is current\nbinary: {}",
            binary.display()
        ));
    }
    println!("live: building Tranquillity {pin}");
    reset_source(&src, &pin)?;
    git_apply(&src, &capture)?;
    git_apply(&src, &listen)?;
    publish(&src)?;
    copy_examples(&publish_dir())?;
    fs::write(&stamp, &stamp_text)
        .map_err(|error| format!("could not write {}: {error}", stamp.display()))?;
    Ok(format!("live: built {pin}\nbinary: {}", binary.display()))
}

#[cfg(target_os = "linux")]
struct Started {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    public_port: u16,
    password: String,
    log: PathBuf,
    altered: bool,
}

#[cfg(target_os = "linux")]
fn up_linux() -> Result<String, String> {
    let started = start_region(false)?;
    let report = up_report(started.public_port, &started.log, started.altered);
    // Dropping Child does not kill the process. `live up` exits and init inherits it.
    drop(started.child);
    drop(started.stdin);
    Ok(report)
}

#[cfg(target_os = "linux")]
fn run_linux(scenario: &str) -> Result<String, String> {
    if ferroan_bot::scenario::selection(scenario).is_none() {
        return Err(format!(
            "unknown scenario {scenario}; expected all or {}",
            ferroan_bot::scenario::NAMES.join(", ")
        ));
    }
    if let Some(state) = read_state(&state_path())?
        && region_pid_live(state.pid)
    {
        return Err(format!(
            "the region is already up (pid {}); cargo xtask live down stops it",
            state.pid
        ));
    }
    let mut started = start_region(true)?;
    let result = run_scenarios(&mut started, scenario);
    let _ = stop_started(&mut started);
    result
}

#[cfg(target_os = "linux")]
fn run_scenarios(started: &mut Started, scenario: &str) -> Result<String, String> {
    let chat_password = generate_password()?;
    create_chat_user(started, &chat_password)?;
    let texture = texture_bytes()?;
    let region_id = uuid::Uuid::parse_str(REGION_UUID)
        .map_err(|_| "the region uuid is not a uuid".to_string())?;
    let library = uuid::Uuid::parse_str("2a4880b6-b7a3-690a-2049-bfbe38eafb9f")
        .map_err(|_| "the library texture uuid is not a uuid".to_string())?;
    let harness = ferroan_bot::scenario::Harness {
        login_uri: format!("http://127.0.0.1:{}/", started.public_port),
        region_id,
        owner: ferroan_bot::scenario::Account {
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            password: started.password.clone(),
        },
        chat: ferroan_bot::scenario::Account {
            first_name: "Chat".to_string(),
            last_name: "Bot".to_string(),
            password: chat_password,
        },
        texture,
        library_texture: library,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start tokio: {error}"))?;
    runtime.block_on(ferroan_bot::scenario::run(scenario, &harness))
}

#[cfg(target_os = "linux")]
fn texture_bytes() -> Result<Vec<u8>, String> {
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
                redact(&log_tail(&started.log, password), &started.password)
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
    Ok(())
}

#[cfg(target_os = "linux")]
fn start_region(console: bool) -> Result<Started, String> {
    let binary = region_binary();
    if !binary.is_file() {
        return Err("run cargo xtask live build first".to_string());
    }
    if let Some(state) = read_state(&state_path())?
        && region_pid_live(state.pid)
    {
        return Err(format!(
            "the region is already up (pid {}); cargo xtask live down stops it",
            state.pid
        ));
    }
    command_ok("sqlite3", &["-version"])?;
    let publish = publish_dir();
    require_under_scratch(&publish)?;
    copy_examples(&publish)?;
    delete_databases(&publish)?;
    let public_port = free_port()?;
    let private_port = free_port_except(public_port)?;
    let password = generate_password()?;
    let run = run_dir();
    let regions = run.join("Regions");
    let config = run.join("config");
    fs::create_dir_all(&regions)
        .map_err(|error| format!("could not create {}: {error}", regions.display()))?;
    fs::create_dir_all(&config)
        .map_err(|error| format!("could not create {}: {error}", config.display()))?;
    let server_ini = region_server_ini(&regions, public_port, private_port, &password)?;
    write_secret(&config.join("RegionServer.ini"), &server_ini)?;
    fs::write(regions.join("Regions.ini"), regions_ini(public_port))
        .map_err(|error| format!("could not write Regions.ini: {error}"))?;
    let first_log = run.join("region-first.log");
    println!("live: starting the region on 127.0.0.1:{public_port}");
    let mut first = spawn_region(&binary, &publish, &config, &first_log, console)?;
    match wait_region(&mut first, &first_log, START_TIMEOUT)? {
        Waited::Ready => {
            return finish_start(
                first,
                &first_log,
                public_port,
                private_port,
                false,
                &password,
            );
        }
        Waited::Timeout => {
            stop_child(&mut first);
            return Err(format!(
                "the first start did not finish\n{}",
                log_tail(&first_log, &password)
            ));
        }
        Waited::Exited(_) => {}
    }
    let database = publish.join("userprofiles.db");
    if !database.is_file() {
        return Err(format!(
            "the first start exited without userprofiles.db\n{}",
            log_tail(&first_log, &password)
        ));
    }
    let altered = add_missing_columns(&database)?;
    if !altered {
        return Err(format!(
            "the first start exited and the user columns were already present\n{}",
            log_tail(&first_log, &password)
        ));
    }
    println!("live: added the SQLite user columns; starting again");
    let log = run.join("region.log");
    let mut child = spawn_region(&binary, &publish, &config, &log, console)?;
    match wait_region(&mut child, &log, START_TIMEOUT)? {
        Waited::Ready => finish_start(child, &log, public_port, private_port, true, &password),
        Waited::Timeout => {
            stop_child(&mut child);
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
    log: &Path,
    public_port: u16,
    private_port: u16,
    altered: bool,
    password: &str,
) -> Result<Started, String> {
    let pid = child.id();
    let checked = process_sockets(pid).and_then(|sockets| loopback_report(&sockets, public_port));
    if let Err(error) = checked {
        stop_child(&mut child);
        return Err(format!("{error}\n{}", log_tail(log, password)));
    }
    write_state(
        &state_path(),
        &RunState {
            pid,
            public_port,
            private_port,
        },
    )?;
    let stdin = child.stdin.take();
    Ok(Started {
        child,
        stdin,
        public_port,
        password: password.to_owned(),
        log: log.to_owned(),
        altered,
    })
}

#[cfg(target_os = "linux")]
fn down_linux() -> Result<String, String> {
    let Some(state) = read_state(&state_path())? else {
        return Ok("live: down".to_string());
    };
    if region_pid_live(state.pid) {
        stop_pid(state.pid)?;
    }
    let _ = fs::remove_file(state_path());
    Ok("live: down".to_string())
}

#[cfg(target_os = "linux")]
fn read_pin() -> Result<String, String> {
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

#[cfg(target_os = "linux")]
fn publish(src: &Path) -> Result<(), String> {
    require_under_scratch(src)?;
    let output = Command::new("dotnet")
        .args([
            "publish",
            "Source/OpenSim.Server.RegionServer/OpenSim.Server.RegionServer.csproj",
            "-c",
            "Release",
            "-r",
            "linux-x64",
            "--nologo",
        ])
        .current_dir(src)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .output()
        .map_err(|error| format!("could not run dotnet: {error}"))?;
    if output.status.success() && region_binary().is_file() {
        Ok(())
    } else {
        Err(format!("dotnet publish failed\n{}", output_tail(&output)))
    }
}

#[cfg(target_os = "linux")]
fn copy_examples(publish: &Path) -> Result<(), String> {
    let include = publish.join("config-include");
    for name in ["StandaloneCommon.ini", "FlotsamCache.ini"] {
        let dest = include.join(name);
        if dest.is_file() {
            continue;
        }
        let source = include.join(format!("{name}.example"));
        fs::copy(&source, &dest).map_err(|error| {
            format!(
                "could not copy {} to {}: {error}",
                source.display(),
                dest.display()
            )
        })?;
    }
    Ok(())
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
        if name.ends_with(".db")
            || name.ends_with(".db-shm")
            || name.ends_with(".db-wal")
            || name.ends_with(".db-journal")
        {
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
) -> Result<std::process::Child, String> {
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
    let mut command = Command::new(binary);
    command
        .current_dir(publish)
        .arg("--inidirectory")
        .arg(config);
    if console {
        // Foreground keeps the console prompt, so `live run` can create the
        // second account. `--console` defaults to local, which reads keys and
        // throws when stdin is a pipe, and that default is applied after the
        // ini. `basic` uses ReadLine. `--background` is what `live up` uses.
        command.args(["--console", "basic"]).stdin(Stdio::piped());
    } else {
        command.arg("--background").stdin(Stdio::null());
    }
    command
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(err_file));
    let child = command
        .process_group(0)
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", binary.display()))?;
    Ok(child)
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

#[cfg(target_os = "linux")]
fn log_contains(path: &Path, needle: &str) -> Result<bool, String> {
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
fn log_tail(path: &Path, password: &str) -> String {
    let Ok(bytes) = fs::read(path) else {
        return format!("(no log at {})", path.display());
    };
    let start = bytes.len().saturating_sub(4000);
    let text = String::from_utf8_lossy(&bytes[start..]);
    redact(&text, password)
}

#[cfg(target_os = "linux")]
fn add_missing_columns(database: &Path) -> Result<bool, String> {
    let names = sqlite(
        database,
        "SELECT name FROM pragma_table_info('UserAccounts');",
    )?;
    let mut added = false;
    for sql in ALTERS {
        let column = if sql.contains("DisplayName") {
            "DisplayName"
        } else {
            "NameChanged"
        };
        if names.lines().any(|name| name.trim() == column) {
            continue;
        }
        sqlite(database, sql)?;
        added = true;
    }
    Ok(added)
}

#[cfg(target_os = "linux")]
fn sqlite(database: &Path, sql: &str) -> Result<String, String> {
    let output = Command::new("sqlite3")
        .arg(database)
        .arg(sql)
        .output()
        .map_err(|error| format!("could not run sqlite3: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "sqlite3 failed on {}\n{}",
            database.display(),
            output_tail(&output)
        ))
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
    let Ok(bytes) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    String::from_utf8_lossy(&bytes).contains("OpenSim.Server.RegionServer")
}

#[cfg(target_os = "linux")]
fn stop_child(child: &mut std::process::Child) {
    let pid = child.id();
    let _ = stop_pid(pid);
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
fn stop_pid(pid: u32) -> Result<(), String> {
    if !region_pid_live(pid) {
        return Ok(());
    }
    let _ = Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .status();
    let started = Instant::now();
    while region_pid_live(pid) && started.elapsed() < STOP_TIMEOUT {
        thread::sleep(Duration::from_millis(200));
    }
    if region_pid_live(pid) {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .status();
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
fn generate_password() -> Result<String, String> {
    let mut bytes = [0u8; 18];
    let mut file = File::open("/dev/urandom")
        .map_err(|error| format!("could not read /dev/urandom: {error}"))?;
    std::io::Read::read_exact(&mut file, &mut bytes)
        .map_err(|error| format!("could not read /dev/urandom: {error}"))?;
    Ok(hex_encode(&bytes))
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
fn write_secret(path: &Path, text: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("could not bind a local port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("could not read a local port: {error}"))
}

#[cfg(target_os = "linux")]
fn free_port_except(taken: u16) -> Result<u16, String> {
    for _ in 0..8 {
        let port = free_port()?;
        if port != taken {
            return Ok(port);
        }
    }
    Err("could not allocate two local ports".to_string())
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
fn command_ok(program: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("could not run {program}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed\n{}", output_tail(&output)))
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

    #[test]
    fn the_sqlite_workaround_does_not_touch_migrations() {
        assert_eq!(ALTERS.len(), 2);
        for sql in ALTERS {
            assert!(sql.starts_with("ALTER TABLE UserAccounts ADD COLUMN "));
            assert!(!sql.to_ascii_lowercase().contains("migration"));
        }
    }

    #[test]
    fn the_region_ini_has_the_const_keys_and_the_report_omits_the_password() {
        let ini =
            region_server_ini(Path::new("/tmp/regions"), 9100, 9103, "0123456789abcdef").unwrap();
        assert!(ini.contains("BaseURL = http://${Const|BaseHostname}"));
        assert!(ini.contains("PrivURL = ${Const|BaseURL}"));
        assert!(ini.contains("PrivatePort = \"9103\""));
        assert!(ini.contains(ESTATE_OWNER));
        assert!(ini.contains("DefaultEstateOwnerPassword = \"0123456789abcdef\""));
        assert!(ini.contains("regionload_regionsdir = \"/tmp/regions\""));
        let report = up_report(9100, Path::new(".scratch/live/run/region.log"), true);
        assert!(report.contains("login: http://127.0.0.1:9100/"));
        assert!(report.contains("sqlite: added DisplayName and NameChanged"));
        assert!(!report.contains("0123456789abcdef"));
        assert_eq!(
            redact("password 0123456789abcdef done", "0123456789abcdef"),
            "password <redacted> done"
        );
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
            },
        )
        .unwrap();
        let state = read_state(&path).unwrap().unwrap();
        assert_eq!(state.pid, 42);
        assert_eq!(state.public_port, 9100);
        assert_eq!(state.private_port, 9103);
        let _ = fs::remove_dir_all(&dir);
    }
}
