// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY EXPRESS OR
// IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
// DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE
// GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER
// IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
// OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF
// ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Configuration adapted for Ferroan from
//   Source/OpenSim.Server.GridServer/AppData/GridServer.ini.example
//   Source/OpenSim.Server.RegionServer/AppData/config-include/Grid.ini
//   Source/OpenSim.Server.RegionServer/AppData/config-include/GridCommon.ini.example
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// The configuration files carry the repository's OpenSimulator BSD notice above.
// Process management and capture orchestration are new Ferroan code.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grid mode: `cargo xtask live run <scenario>|all|neighbour grid`.
//!
//! Standalone stays `cargo xtask live run`. This starts Tranquillity's
//! GridServer and two neighbouring regions. Each region uses the shipped
//! `Grid.ini` and an unchanged `GridCommon.ini`. Service URLs point at
//! `ferroan-capture` proxies. The region HTTP listener is on another
//! loopback address from the address it advertises, so the proxy can bind
//! the advertised port. UDP stays on `127.0.0.1`. Each region has its own
//! working directory, so the relative SQLite files in `SQLiteStandalone.ini`
//! do not collide. The bot logs in on Robust's own public address.
//!
//! The ini text follows GridServer.ini.example, Grid.ini and
//! GridCommon.ini.example at Tranquillity `40432e0`. Nothing here edits
//! those files.

#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::live;

#[path = "grid_capture.rs"]
mod login_capture;
#[path = "grid_probe.rs"]
mod login_probe;

#[path = "ferrobot_capture.rs"]
pub(crate) mod ferrobot_capture;

const HOME_NAME: &str = "Ferroan Test";
const NEIGHBOUR_NAME: &str = "Ferroan Neighbour";
const NEIGHBOUR_UUID: &str = "c3e1a9b4-6d27-4f80-91ab-5e8c0d2f7a33";
const HOME_X: u32 = 1000;
const HOME_Y: u32 = 1000;
const NEIGHBOUR_X: u32 = 1001;
const NEIGHBOUR_Y: u32 = 1000;
const ADVERTISED: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
const HOME_BIND: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
const NEIGHBOUR_BIND: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 3);
const ROBUST_BIND: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 4);
const PROMPT: &str = "GridServer>";

pub fn run(scenario: &str) -> std::process::ExitCode {
    live::finish(run_inner(scenario, false))
}

pub fn run_ferroan(scenario: &str) -> std::process::ExitCode {
    live::finish(run_inner(scenario, true))
}

pub fn login_fixtures(directory: &Path) -> std::process::ExitCode {
    live::finish(login_capture::generate(directory))
}

pub fn parity_fixtures(directory: &Path) -> std::process::ExitCode {
    live::finish(login_probe::generate(directory))
}

pub fn root_arrival_fixtures(directory: &Path) -> std::process::ExitCode {
    live::finish(login_probe::generate_root_arrival(directory))
}

fn run_inner(scenario: &str, ferroan: bool) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        run_linux(scenario, ferroan)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (scenario, ferroan);
        Err("cargo xtask live runs on Linux".to_string())
    }
}

/// One region process. `http_bind` is where it listens. Viewers and other
/// regions use `advertised` and `http_port` (the proxy).
struct RegionIni<'a> {
    name: &'a str,
    uuid: &'a str,
    x: u32,
    y: u32,
    http_bind: Ipv4Addr,
    udp_bind: Ipv4Addr,
    http_port: u16,
    regions_dir: &'a str,
    public_proxy: u16,
    private_proxy: u16,
    password: &'a str,
    capture: Option<&'a str>,
}

fn region_server_ini(region: &RegionIni<'_>) -> Result<String, String> {
    plain(region.regions_dir, "regions directory")?;
    plain(region.password, "password")?;
    if let Some(capture) = region.capture {
        plain(capture, "capture path")?;
    }
    if region.http_port == 0 || region.public_proxy == 0 || region.private_proxy == 0 {
        return Err("grid mode needs a region port and two Robust proxy ports".to_string());
    }
    let mut ini = format!(
        r#"[Const]
    BaseHostname = "{advertised}"
    BaseURL = http://${{Const|BaseHostname}}
    PublicPort = "{public_proxy}"
    PrivURL = ${{Const|BaseURL}}
    PrivatePort = "{private_proxy}"

[Startup]
    regionload_regionsdir = "{dir}"
    ConsoleHistoryFileEnabled = false

[Network]
    http_listener_port = {http_port}
    bind_ip_address = "{http_bind}"

[Estates]
    DefaultEstateName = "Ferroan Test Estate"
    DefaultEstateOwnerName = "Test User"
    DefaultEstateOwnerEMail = "test@example.invalid"
    DefaultEstateOwnerUUID = "{owner}"
    DefaultEstateOwnerPassword = "{password}"

[Architecture]
    Include-Architecture = "config-include/Grid.ini"
"#,
        advertised = ADVERTISED,
        public_proxy = region.public_proxy,
        private_proxy = region.private_proxy,
        dir = region.regions_dir,
        http_port = region.http_port,
        http_bind = region.http_bind,
        owner = live::ESTATE_OWNER,
        password = region.password,
    );
    if let Some(capture) = region.capture {
        ini.push_str(&format!(
            "\n[ClientStack.LindenUDP]\n    CaptureFile = \"{capture}\"\n"
        ));
    }
    Ok(ini)
}

fn regions_ini(region: &RegionIni<'_>) -> String {
    format!(
        r#"[{name}]
RegionUUID = {uuid}
Location = {x},{y}
InternalAddress = {udp_bind}
InternalPort = {port}
AllowAlternatePorts = False
ExternalHostName = {advertised}
"#,
        name = region.name,
        uuid = region.uuid,
        x = region.x,
        y = region.y,
        advertised = ADVERTISED,
        udp_bind = region.udp_bind,
        port = region.http_port,
    )
}

fn robust_ini(
    public_port: u16,
    private_port: u16,
    database: &str,
    library: &str,
    assets: &str,
) -> Result<String, String> {
    plain(database, "database path")?;
    plain(library, "library path")?;
    plain(assets, "asset set path")?;
    if public_port == 0 || private_port == 0 || public_port == private_port {
        return Err("Robust needs two different ports".to_string());
    }
    // Auth and assets each open a connection and keep it. Sharing one file
    // with the account store makes the next login fail with "database is locked".
    let asset_db = format!("{database}.assets");
    let auth_db = format!("{database}.auth");
    let sqlite =
        |path: &str| format!("Data Source={path};Version=3;Journal Mode=Wal;BusyTimeout=15000;");
    let accounts = sqlite(database);
    let asset_store = sqlite(&asset_db);
    let auth_store = sqlite(&auth_db);
    Ok(format!(
        r#"[Const]
    BaseHostname = "{bind}"
    BaseURL = "http://{bind}"
    PublicPort = "{public_port}"
    PrivatePort = "{private_port}"

[Startup]
    RegistryLocation = "."
    ConsoleHistoryFileEnabled = false
    EnableRobustSelfsignedCertSupport = false
    NoVerifyCertChain = true
    NoVerifyCertHostname = true

[Network]
    port = {private_port}
    bind_ip_address = "{bind}"

[ServiceList]
    AssetServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:AssetServiceConnector"
    InventoryInConnector = "{private_port}/OpenSim.Server.Handlers.dll:XInventoryInConnector"
    GridServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:GridServiceConnector"
    GridInfoServerInConnector = "{public_port}/OpenSim.Server.Handlers.dll:GridInfoServerInConnector"
    AuthenticationServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:AuthenticationServiceConnector"
    AvatarServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:AvatarServiceConnector"
    LLLoginServiceInConnector = "{public_port}/OpenSim.Server.Handlers.dll:LLLoginServiceInConnector"
    PresenceServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:PresenceServiceConnector"
    UserAccountServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:UserAccountServiceConnector"
    GridUserServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:GridUserServiceConnector"
    AgentPreferencesServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:AgentPreferencesServiceConnector"
    FriendsServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:FriendsServiceConnector"
    MapAddServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:MapAddServiceConnector"
    MapGetServiceConnector = "{public_port}/OpenSim.Server.Handlers.dll:MapGetServiceConnector"
    MuteListConnector = "{private_port}/OpenSim.Server.Handlers.dll:MuteListServiceConnector"

[DatabaseService]
    StorageProvider = "OpenSim.Data.SQLite.dll"
    ConnectionString = "{accounts}"

[AssetService]
    LocalServiceModule = "OpenSim.Services.AssetService.dll:AssetService"
    StorageProvider = "OpenSim.Data.SQLite.dll"
    ConnectionString = "{asset_store}"
    DefaultAssetLoader = "OpenSim.Framework.AssetLoader.Filesystem.dll"
    AssetLoaderArgs = "{assets}"

[InventoryService]
    LocalServiceModule = "OpenSim.Services.InventoryService.dll:XInventoryService"
    AllowDelete = true

[GridService]
    LocalServiceModule = "OpenSim.Services.GridService.dll:GridService"
    ;; SQLite has no IRegionData. Null keeps the region table in this process.
    StorageProvider = "OpenSim.Data.Null.dll"
    Region_Ferroan_Test = "DefaultRegion"

[AuthenticationService]
    LocalServiceModule = "OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService"
    StorageProvider = "OpenSim.Data.SQLite.dll"
    ConnectionString = "{auth_store}"

[UserAccountService]
    LocalServiceModule = "OpenSim.Services.UserAccountService.dll:UserAccountService"
    AuthenticationService = "OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService"
    PresenceService = "OpenSim.Services.PresenceService.dll:PresenceService"
    GridService = "OpenSim.Services.GridService.dll:GridService"
    InventoryService = "OpenSim.Services.InventoryService.dll:XInventoryService"
    AvatarService = "OpenSim.Services.AvatarService.dll:AvatarService"
    GridUserService = "OpenSim.Services.UserAccountService.dll:GridUserService"
    CreateDefaultAvatarEntries = true

[GridUserService]
    LocalServiceModule = "OpenSim.Services.UserAccountService.dll:GridUserService"

[AgentPreferencesService]
    LocalServiceModule = "OpenSim.Services.UserAccountService.dll:AgentPreferencesService"

[PresenceService]
    LocalServiceModule = "OpenSim.Services.PresenceService.dll:PresenceService"
    ;; SQLite has no IPresenceData. Null keeps sessions in this process.
    StorageProvider = "OpenSim.Data.Null.dll"

[AvatarService]
    LocalServiceModule = "OpenSim.Services.AvatarService.dll:AvatarService"

[FriendsService]
    LocalServiceModule = "OpenSim.Services.FriendsService.dll:FriendsService"

[MuteListService]
    LocalServiceModule = "OpenSim.Services.MuteListService.dll:MuteListService"

[LibraryService]
    LibraryName = "OpenSim Library"
    DefaultLibrary = "{library}"

[LoginService]
    LocalServiceModule = "OpenSim.Services.LLLoginService.dll:LLLoginService"
    UserAccountService = "OpenSim.Services.UserAccountService.dll:UserAccountService"
    GridUserService = "OpenSim.Services.UserAccountService.dll:GridUserService"
    AuthenticationService = "OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService"
    InventoryService = "OpenSim.Services.InventoryService.dll:XInventoryService"
    AvatarService = "OpenSim.Services.AvatarService.dll:AvatarService"
    PresenceService = "OpenSim.Services.PresenceService.dll:PresenceService"
    GridService = "OpenSim.Services.GridService.dll:GridService"
    SimulationService = "OpenSim.Services.Connectors.dll:SimulationServiceConnector"
    LibraryService = "OpenSim.Services.InventoryService.dll:LibraryService"
    FriendsService = "OpenSim.Services.FriendsService.dll:FriendsService"
    WelcomeMessage = "Welcome, Avatar!"
    AllowRemoteSetLoginLevel = "false"
    AllowLoginFallbackToAnyRegion = false

[GridInfoService]
    login = http://{bind}:{public_port}/
    gridname = "Ferroan Test Grid"
    gridnick = "ferroan"
"#,
        bind = ROBUST_BIND,
    ))
}

fn plain(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.contains(['"', '\n', '\r', ';']) {
        Err(format!(
            "{label} is empty or contains a quote, break or semicolon"
        ))
    } else {
        Ok(())
    }
}

fn count_http_requests(bytes: &[u8]) -> Result<usize, String> {
    let records =
        ferroan_capture::read_complete_prefix(bytes).map_err(|error| error.to_string())?;
    Ok(records
        .iter()
        .filter(|record| record.kind == ferroan_capture::Kind::HttpRequest)
        .count())
}

fn contains_secret(bytes: &[u8], secret: &str) -> bool {
    !secret.is_empty()
        && bytes
            .windows(secret.len())
            .any(|window| window == secret.as_bytes())
}

#[cfg(target_os = "linux")]
fn grid_root() -> PathBuf {
    live::live_root().join("grid")
}

#[cfg(target_os = "linux")]
struct Proc {
    child: Child,
    stdin: Option<ChildStdin>,
    log: PathBuf,
    tracked: crate::grid_state::Tracked,
}

#[cfg(target_os = "linux")]
impl Drop for Proc {
    fn drop(&mut self) {
        drop(self.stdin.take());
        stop_child(&mut self.child);
        if let Err(error) = self.tracked.forget() {
            eprintln!("live: grid process cleanup: {error}");
        }
    }
}

#[cfg(target_os = "linux")]
struct Grid {
    robust: Option<Proc>,
    home: Option<Proc>,
    neighbour: Option<Proc>,
    proxies: Option<crate::live_proxy::Proxies>,
    passwords: Vec<String>,
    _state: crate::grid_state::Run,
}

#[cfg(target_os = "linux")]
impl Grid {
    fn stop(&mut self) -> Result<(), String> {
        for slot in [&mut self.home, &mut self.neighbour, &mut self.robust] {
            drop(slot.take());
        }
        self.proxies
            .take()
            .map_or(Ok(()), |proxies| proxies.finish())
    }
}

fn stop_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let pid = child.id();
    let _ = Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .status();
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) if started.elapsed() >= Duration::from_secs(30) => break,
            Ok(None) => thread::sleep(Duration::from_millis(200)),
        }
    }
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .status();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
impl Drop for Grid {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("live: {error}");
        }
    }
}

#[cfg(target_os = "linux")]
fn run_linux(scenario: &str, ferroan: bool) -> Result<String, String> {
    if ferroan && !matches!(scenario, "login" | "caps" | "ferrobot") {
        return Err(
            "the Ferroan grid region currently supports login, caps and ferrobot scenarios"
                .to_string(),
        );
    }
    if !matches!(scenario, "neighbour" | "caps" | "ferrobot")
        && ferroan_bot::scenario::selection(scenario).is_none()
    {
        return Err(format!(
            "unknown scenario {scenario}; expected all, neighbour or {}",
            ferroan_bot::scenario::NAMES.join(", ")
        ));
    }
    if let Some(pid) = live::standalone_is_up()? {
        return Err(format!(
            "the harness is already up (pid {pid}); cargo xtask live down stops an interrupted run"
        ));
    }
    let state = crate::grid_state::claim()?;
    live::ensure_region_built()?;
    let ferrobot_binary = if scenario == "ferrobot" {
        Some(crate::ferrobot_build::build()?)
    } else {
        None
    };
    let mut ports = Vec::new();
    for _ in 0..32 {
        if ports.len() == 6 {
            break;
        }
        let port = live::free_port()?;
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    if ports.len() != 6 {
        return Err("could not allocate six local ports".to_string());
    }
    let robust_public = ports[0];
    let robust_private = ports[1];
    let public_proxy = ports[2];
    let private_proxy = ports[3];
    let home_port = ports[4];
    let neighbour_port = ports[5];
    let root = grid_root();
    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|error| format!("could not clear {}: {error}", root.display()))?;
    }
    fs::create_dir_all(&root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;
    let estate = live::generate_password()?;
    let chat = live::generate_password()?;
    let mut grid = Grid {
        robust: None,
        home: None,
        neighbour: None,
        proxies: None,
        passwords: vec![estate.clone(), chat.clone()],
        _state: state,
    };
    let captures = ProxyPaths {
        robust_private: root.join("proxy-robust-private.cap"),
        robust_public: root.join("proxy-robust-public.cap"),
        home: root.join("proxy-home.cap"),
        neighbour: root.join("proxy-neighbour.cap"),
    };
    grid.proxies = Some(start_proxies(&[
        (
            SocketAddr::from((ADVERTISED, private_proxy)),
            SocketAddr::from((ROBUST_BIND, robust_private)),
            captures.robust_private.clone(),
        ),
        (
            SocketAddr::from((ADVERTISED, public_proxy)),
            SocketAddr::from((ROBUST_BIND, robust_public)),
            captures.robust_public.clone(),
        ),
        (
            SocketAddr::from((ADVERTISED, home_port)),
            SocketAddr::from((HOME_BIND, home_port)),
            captures.home.clone(),
        ),
        (
            SocketAddr::from((ADVERTISED, neighbour_port)),
            SocketAddr::from((NEIGHBOUR_BIND, neighbour_port)),
            captures.neighbour.clone(),
        ),
    ])?);
    let udp_relay = if ferroan && scenario == "ferrobot" {
        Some(grid.proxies.as_ref().expect("proxies started").block_on(
            ferrobot_capture::start_udp(
                SocketAddr::from((ADVERTISED, home_port)),
                SocketAddr::from((HOME_BIND, home_port)),
                &root.join("capture-udp.cap"),
            ),
        )?)
    } else {
        None
    };
    println!(
        "live: grid Robust on {ROBUST_BIND}:{robust_public} (private {robust_private}), proxies {public_proxy}/{private_proxy}"
    );
    grid.robust = Some(start_robust(
        &root,
        robust_public,
        robust_private,
        &grid.passwords,
    )?);
    create_account(
        grid.robust.as_mut().expect("robust is started"),
        "Test",
        "User",
        &estate,
        "test@example.invalid",
        live::ESTATE_OWNER,
        &grid.passwords,
    )?;
    create_account(
        grid.robust.as_mut().expect("robust is started"),
        "Chat",
        "Bot",
        &chat,
        "chat@example.invalid",
        live::CHAT_USER,
        &grid.passwords,
    )?;
    let publish = live::publish_dir();
    live::copy_example(&publish, "GridCommon.ini")?;
    let source = live::source_dir();
    println!("live: grid region {HOME_NAME} on {HOME_BIND}:{home_port}");
    grid.home = Some(start_region(
        &root.join("home"),
        &publish,
        &source,
        &estate,
        RegionStart {
            ferroan,
            login_only: matches!(scenario, "login" | "caps" | "ferrobot"),
            name: HOME_NAME,
            uuid: live::REGION_UUID,
            x: HOME_X,
            y: HOME_Y,
            http_bind: HOME_BIND,
            udp_bind: if ferroan && scenario == "ferrobot" {
                HOME_BIND
            } else {
                ADVERTISED
            },
            http_port: home_port,
            public_proxy,
            private_proxy,
            capture: Some(root.join("capture-udp.cap")),
        },
        &grid.passwords,
    )?);
    println!("live: grid region {NEIGHBOUR_NAME} on {NEIGHBOUR_BIND}:{neighbour_port}");
    grid.neighbour = Some(start_region(
        &root.join("neighbour"),
        &publish,
        &source,
        &estate,
        RegionStart {
            ferroan: false,
            login_only: matches!(scenario, "login" | "caps" | "ferrobot"),
            name: NEIGHBOUR_NAME,
            uuid: NEIGHBOUR_UUID,
            x: NEIGHBOUR_X,
            y: NEIGHBOUR_Y,
            http_bind: NEIGHBOUR_BIND,
            udp_bind: ADVERTISED,
            http_port: neighbour_port,
            public_proxy,
            private_proxy,
            capture: None,
        },
        &grid.passwords,
    )?);
    let home_log = grid
        .home
        .as_ref()
        .expect("home region is started")
        .log
        .clone();
    let report = if let Some(executable) = ferrobot_binary {
        let harness = live::harness(
            &format!("http://{ADVERTISED}:{public_proxy}/"),
            &estate,
            &chat,
            &home_log,
        )?;
        let config = crate::ferrobot::Config {
            executable,
            root: root.join("ferrobot"),
            region_name: HOME_NAME.to_string(),
            ferroan,
            udp_capture: root.join("capture-udp.cap"),
            http_captures: vec![captures.robust_public.clone(), captures.home.clone()],
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime
            .block_on(crate::ferrobot::run(&harness, &config))
            .map(|lines| BotReport {
                lines,
                // Read the complete proxy evidence after grid.stop drains it.
                http: Vec::new(),
            })
    } else {
        run_bot(
        scenario,
        &format!("http://{ROBUST_BIND}:{robust_public}/"),
        &estate,
        &chat,
        &home_log,
        ferroan,
        (&format!("http://{ADVERTISED}:{private_proxy}"), &root, ferroan),
    ).and_then(|mut report| {
        if scenario == "login" {
            let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()
                .map_err(|error| format!("could not start login probe runtime: {error}"))?;
            let private_uri = format!("http://{ADVERTISED}:{private_proxy}");
            // run_bot has already awaited service cleanup after each logout.
            if ferroan {
                let second = run_bot(scenario, &format!("http://{ROBUST_BIND}:{robust_public}/"), &estate, &chat, &home_log, true, (&private_uri, &root, true))?;
                report.http.extend(second.http);
                report.lines.push_str("\nlogout state: GridUser offline, Presence empty; second login and logout pass");
            }
            let line = runtime.block_on(login_probe::probe(&format!("http://{ADVERTISED}:{home_port}"), &root, ferroan))?;
            capture_missing_presence(SocketAddr::from((ROBUST_BIND, robust_private)), &root)?;
            report.lines.push('\n'); report.lines.push_str(&line);
        }
        Ok(report)
    })
    };
    let relay_result = if let Some(task) = udp_relay {
        if task.is_finished() {
            grid.proxies
                .as_ref()
                .expect("proxies started")
                .block_on(task)
                .map_err(|error| format!("UDP capture task stopped: {error}"))
                .and_then(|result| result)
        } else {
            task.abort();
            // Finish cancellation before reading the capture or stopping the
            // runtime, so the relay has released its socket and output file.
            let _ = grid
                .proxies
                .as_ref()
                .expect("proxies started")
                .block_on(task);
            Ok(())
        }
    } else {
        Ok(())
    };
    let cleanup = grid.stop();
    let report = match (report, relay_result, cleanup) {
        (Ok(report), Ok(()), Ok(())) => report,
        (report, relay, cleanup) => {
            return Err([report.err(), relay.err(), cleanup.err()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n"));
        }
    };
    let proxy_report = proxy_report(&captures, &grid.passwords)?;
    if scenario == "ferrobot" {
        let http = ferrobot_capture::http_records(&[
            captures.robust_public.clone(),
            captures.home.clone(),
        ])?;
        let capture =
            live::finish_capture(&root.join("capture-udp.cap"), &http, &[&estate, &chat])?;
        Ok(format!("{}\n{capture}\n{proxy_report}", report.lines))
    } else if ferroan {
        let comparison = login_capture::compare(&root)?;
        let arrival = if scenario == "login" {
            login_capture::compare_root_arrival(&root)?
        } else {
            String::new()
        };
        Ok(format!(
            "{}\nlogout: LogoutReply\n{comparison}\n{arrival}\n{proxy_report}",
            report.lines
        ))
    } else {
        let capture = live::finish_capture(
            &root.join("capture-udp.cap"),
            &report.http,
            &[&estate, &chat],
        )?;
        Ok(format!("{}\n{capture}\n{proxy_report}", report.lines))
    }
}

#[cfg(target_os = "linux")]
struct ProxyPaths {
    robust_private: PathBuf,
    robust_public: PathBuf,
    home: PathBuf,
    neighbour: PathBuf,
}

#[cfg(target_os = "linux")]
struct BotReport {
    lines: String,
    http: Vec<ferroan_capture::Record>,
}

#[cfg(target_os = "linux")]
fn capture_missing_presence(upstream: SocketAddr, root: &Path) -> Result<(), String> {
    let (addresses, proxy) = crate::live_proxy::Proxies::start(&[(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        upstream,
        root.join("proxy-presence-missing.cap"),
    )])?;
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?
        .block_on(login_probe::missing_presence(&format!(
            "http://{}",
            addresses[0]
        )));
    let finished = proxy.finish();
    match (result, finished) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}; {cleanup}")),
    }
}

#[cfg(target_os = "linux")]
fn run_bot(
    scenario: &str,
    login_uri: &str,
    estate: &str,
    chat: &str,
    region_log: &Path,
    strict_logout: bool,
    online: (&str, &Path, bool),
) -> Result<BotReport, String> {
    let neighbour_id = uuid::Uuid::parse_str(NEIGHBOUR_UUID)
        .map_err(|_| "the neighbour uuid is not a uuid".to_string())?;
    let mut harness = live::harness(login_uri, estate, chat, region_log)?;
    harness.require_logout_reply = strict_logout;
    harness.neighbour = Some(ferroan_bot::scenario::Neighbour {
        region_id: neighbour_id,
        x: NEIGHBOUR_X,
        y: NEIGHBOUR_Y,
        home_x: HOME_X,
        home_y: HOME_Y,
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start tokio: {error}"))?;
    let (private_uri, directory, ferroan) = online;
    let harness_ref = &harness;
    let lines = runtime.block_on(login_probe::run_scenarios(
        scenario,
        private_uri,
        |selected| async move {
            if scenario == "login" {
                ferroan_bot::scenario::run_login_observed(harness_ref, |arrival| {
                    login_probe::online(private_uri, directory, ferroan, arrival)
                })
                .await
            } else {
                ferroan_bot::scenario::run(selected, harness_ref).await
            }
        },
    ))?;
    Ok(BotReport {
        lines,
        http: harness.http_trace.records(),
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn start_proxies(
    binds: &[(SocketAddr, SocketAddr, PathBuf)],
) -> Result<crate::live_proxy::Proxies, String> {
    crate::live_proxy::Proxies::start(binds).map(|(_, proxies)| proxies)
}

#[cfg(target_os = "linux")]
fn start_robust(
    root: &Path,
    public_port: u16,
    private_port: u16,
    passwords: &[String],
) -> Result<Proc, String> {
    let binary = live::grid_server_binary();
    if !binary.is_file() {
        return Err("run cargo xtask live build first".to_string());
    }
    let database = root.join("robust.db");
    let library = live::source_dir()
        .join("Source/OpenSim.Services.InventoryService/AppData/inventory/Libraries.xml");
    let assets = live::asset_set_path();
    if !library.is_file() {
        return Err(format!("missing {}", library.display()));
    }
    if !assets.is_file() {
        return Err(format!("missing {}", assets.display()));
    }
    let ini = robust_ini(
        public_port,
        private_port,
        database.to_str().ok_or("database path is not utf-8")?,
        library.to_str().ok_or("library path is not utf-8")?,
        assets.to_str().ok_or("asset set path is not utf-8")?,
    )?;
    let ini_path = root.join("robust.ini");
    live::write_secret(&ini_path, &ini)?;
    let empty = root.join("robust-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let log = root.join("robust.log");
    let ini_arg = ini_path.to_str().ok_or("robust ini path is not utf-8")?;
    let empty_arg = empty.to_str().ok_or("robust config path is not utf-8")?;
    let mut proc = spawn_proc(
        &binary,
        &[
            "--inimaster",
            ini_arg,
            "--inidirectory",
            empty_arg,
            "--console",
            "basic",
        ],
        binary.parent().unwrap_or(Path::new(".")),
        &log,
        true,
    )?;
    wait_log(
        &mut proc.child,
        &log,
        PROMPT,
        live::START_TIMEOUT,
        passwords,
    )?;
    live::require_listen(proc.child.id(), ROBUST_BIND, public_port)?;
    live::require_listen(proc.child.id(), ROBUST_BIND, private_port)?;
    Ok(proc)
}

#[cfg(target_os = "linux")]
fn create_account(
    proc: &mut Proc,
    first: &str,
    last: &str,
    password: &str,
    email: &str,
    uuid: &str,
    passwords: &[String],
) -> Result<(), String> {
    let stdin = proc
        .stdin
        .as_mut()
        .ok_or("Robust was started without a console")?;
    // An empty model answer keeps the default appearance. The handler asks
    // when the argument is absent.
    let command = format!("create user {first} {last} {password} {email} {uuid}\n\n");
    use std::io::Write;
    stdin
        .write_all(command.as_bytes())
        .map_err(|error| format!("could not write the create user command: {error}"))?;
    stdin
        .flush()
        .map_err(|error| format!("could not write the create user command: {error}"))?;
    let needle = format!("Account {first} {last} {uuid} created successfully");
    wait_log(
        &mut proc.child,
        &proc.log,
        &needle,
        Duration::from_secs(60),
        passwords,
    )?;
    if live::log_contains(&proc.log, password)? {
        return Err("the Robust log contains a password".to_string());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct RegionStart {
    ferroan: bool,
    login_only: bool,
    name: &'static str,
    uuid: &'static str,
    x: u32,
    y: u32,
    http_bind: Ipv4Addr,
    udp_bind: Ipv4Addr,
    http_port: u16,
    public_proxy: u16,
    private_proxy: u16,
    capture: Option<PathBuf>,
}

#[cfg(target_os = "linux")]
fn start_region(
    cwd: &Path,
    publish: &Path,
    source: &Path,
    password: &str,
    spec: RegionStart,
    passwords: &[String],
) -> Result<Proc, String> {
    let binary = if spec.ferroan {
        live::build_binary("ferroan-region")?
    } else {
        publish.join("OpenSim.Server.RegionServer")
    };
    if !binary.is_file() {
        return Err("run cargo xtask live build first".to_string());
    }
    prepare_region_cwd(cwd, publish, source)?;
    let config = cwd.join("config");
    let regions = cwd.join("Regions");
    fs::create_dir_all(&config)
        .map_err(|error| format!("could not create {}: {error}", config.display()))?;
    fs::create_dir_all(&regions)
        .map_err(|error| format!("could not create {}: {error}", regions.display()))?;
    let regions_text = regions
        .to_str()
        .ok_or("regions directory is not utf-8")?
        .to_string();
    let capture_text = match &spec.capture {
        Some(path) => Some(
            path.to_str()
                .ok_or("capture path is not utf-8")?
                .to_string(),
        ),
        None => None,
    };
    let region = RegionIni {
        name: spec.name,
        uuid: spec.uuid,
        x: spec.x,
        y: spec.y,
        http_bind: spec.http_bind,
        udp_bind: spec.udp_bind,
        http_port: spec.http_port,
        regions_dir: &regions_text,
        public_proxy: spec.public_proxy,
        private_proxy: spec.private_proxy,
        password,
        capture: capture_text.as_deref(),
    };
    let mut ini = region_server_ini(&region)?;
    if spec.login_only {
        // Scene.RegisterRegionWithGrid skips map generation in this supported
        // configuration. Q6 checks login, before Ferroan has terrain/maps.
        ini.push_str("\n[Map]\n    GenerateMaptiles = false\n    MaptileStaticUUID = 00000000-0000-0000-0000-000000000000\n");
    }
    live::write_secret(&config.join("RegionServer.ini"), &ini)?;
    fs::write(regions.join("Regions.ini"), regions_ini(&region))
        .map_err(|error| format!("could not write Regions.ini: {error}"))?;
    let log = cwd.join("region.log");
    let config_arg = config.to_str().ok_or("region config path is not utf-8")?;
    let mut proc = spawn_proc(
        &binary,
        &["--inidirectory", config_arg, "--background"],
        cwd,
        &log,
        false,
    )?;
    let ready = format!(
        "INITIALIZATION COMPLETE FOR {} - LOGINS ENABLED",
        region.name
    );
    wait_log(
        &mut proc.child,
        &log,
        &ready,
        live::START_TIMEOUT,
        passwords,
    )?;
    live::require_listen(proc.child.id(), region.http_bind, region.http_port)?;
    live::require_udp(proc.child.id(), region.udp_bind, region.http_port)?;
    Ok(proc)
}

#[cfg(target_os = "linux")]
fn prepare_region_cwd(cwd: &Path, publish: &Path, source: &Path) -> Result<(), String> {
    fs::create_dir_all(cwd)
        .map_err(|error| format!("could not create {}: {error}", cwd.display()))?;
    // Plugin loading uses Assembly.LoadFrom with a path relative to the
    // working directory, and configDir() is that same directory. Link the
    // published assemblies/configuration, excluding mutable standalone data.
    // SQLite opens relative connection strings in each region's own cwd.
    let entries = fs::read_dir(publish)
        .map_err(|error| format!("could not read {}: {error}", publish.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not read {}: {error}", publish.display()))?;
        let name = entry.file_name();
        if matches!(
            name.to_str(),
            Some("Regions" | "Estates" | "assetcache" | "ScriptEngines")
        ) || live::is_sqlite_file(&name.to_string_lossy())
        {
            continue;
        }
        link_path(&cwd.join(name), &entry.path())?;
    }
    if !cwd.join("inventory/Libraries.xml").is_file() {
        let inventory =
            first_file(&[source.join(
                "Source/OpenSim.Services.InventoryService/AppData/inventory/Libraries.xml",
            )])?;
        link_dir(
            &cwd.join("inventory"),
            inventory.parent().ok_or("inventory path has no parent")?,
        )?;
    }
    if !cwd.join("assets/AssetSets.xml").is_file() {
        let assets =
            first_file(&[source
                .join("Source/OpenSim.Services.InventoryService/AppData/assets/AssetSets.xml")])?;
        link_dir(
            &cwd.join("assets"),
            assets.parent().ok_or("asset path has no parent")?,
        )?;
    }
    if !cwd.join("RegionServerDefaults.ini").is_file() {
        return Err(format!(
            "missing {}",
            publish.join("RegionServerDefaults.ini").display()
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn first_file(paths: &[PathBuf]) -> Result<PathBuf, String> {
    paths
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| {
            format!(
                "missing {}",
                paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" or ")
            )
        })
}

#[cfg(target_os = "linux")]
fn link_dir(link: &Path, target: &Path) -> Result<(), String> {
    if !target.is_dir() {
        return Err(format!("missing {}", target.display()));
    }
    link_path(link, target)
}

#[cfg(target_os = "linux")]
fn link_path(link: &Path, target: &Path) -> Result<(), String> {
    if link.symlink_metadata().is_ok() {
        fs::remove_file(link)
            .map_err(|error| format!("could not replace {}: {error}", link.display()))?;
    }
    std::os::unix::fs::symlink(target, link).map_err(|error| {
        format!(
            "could not link {} to {}: {error}",
            link.display(),
            target.display()
        )
    })
}

#[cfg(target_os = "linux")]
fn spawn_proc(
    binary: &Path,
    args: &[&str],
    cwd: &Path,
    log: &Path,
    pipe_stdin: bool,
) -> Result<Proc, String> {
    use std::os::unix::process::CommandExt;
    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let log_file = fs::File::create(log)
        .map_err(|error| format!("could not create {}: {error}", log.display()))?;
    let err_file = log_file
        .try_clone()
        .map_err(|error| format!("could not open {}: {error}", log.display()))?;
    let mut command = Command::new(binary);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(err_file));
    if pipe_stdin {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let launch = crate::grid_state::prepare_spawn(binary, args, cwd)?;
    launch.configure(&mut command);
    let mut child = match command.process_group(0).spawn() {
        Ok(child) => child,
        Err(error) => {
            launch.cancel()?;
            return Err(format!("could not start {}: {error}", binary.display()));
        }
    };
    let stdin = child.stdin.take();
    let tracked = match launch.record(&child) {
        Ok(tracked) => tracked,
        Err(error) => {
            stop_child(&mut child);
            if child.try_wait().is_ok_and(|status| status.is_some()) {
                launch.cancel()?;
            }
            return Err(error);
        }
    };
    Ok(Proc {
        child,
        stdin,
        log: log.to_path_buf(),
        tracked,
    })
}

#[cfg(target_os = "linux")]
fn wait_log(
    child: &mut Child,
    log: &Path,
    needle: &str,
    timeout: Duration,
    passwords: &[String],
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if live::log_contains(log, needle)? {
            return Ok(());
        }
        match child
            .try_wait()
            .map_err(|error| format!("could not wait for the process: {error}"))?
        {
            Some(status) => {
                return Err(format!(
                    "exited ({status}) before {needle}\n{}",
                    tail(log, passwords)
                ));
            }
            None if started.elapsed() >= timeout => {
                return Err(format!(
                    "timed out waiting for {needle}\n{}",
                    tail(log, passwords)
                ));
            }
            None => thread::sleep(Duration::from_millis(200)),
        }
    }
}

#[cfg(target_os = "linux")]
fn tail(log: &Path, passwords: &[String]) -> String {
    let passwords: Vec<_> = passwords.iter().map(String::as_str).collect();
    live::log_tail_secrets(log, &passwords)
}

#[cfg(target_os = "linux")]
fn proxy_report(paths: &ProxyPaths, passwords: &[String]) -> Result<String, String> {
    let private_n = request_count(&paths.robust_private, passwords)?;
    let public_n = request_count(&paths.robust_public, passwords)?;
    let home_n = request_count(&paths.home, passwords)?;
    let neighbour_n = request_count(&paths.neighbour, passwords)?;
    if private_n == 0 {
        return Err(
            "the Robust private proxy recorded no HTTP requests from the regions".to_string(),
        );
    }
    if home_n == 0 && neighbour_n == 0 {
        return Err("neither region proxy recorded an HTTP request".to_string());
    }
    Ok(format!(
        "grid http: robust-private {private_n}, robust-public {public_n}, home {home_n}, neighbour {neighbour_n}"
    ))
}

#[cfg(target_os = "linux")]
fn request_count(path: &Path, passwords: &[String]) -> Result<usize, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    for password in passwords {
        if contains_secret(&bytes, password) {
            return Err(format!(
                "a proxy capture contains a password ({})",
                path.display()
            ));
        }
    }
    count_http_requests(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn regions_do_not_link_standalone_databases_or_sqlite_sidecars() {
        let root = crate::repo_root()
            .join(".scratch")
            .join(format!("region-cwd-{}", uuid::Uuid::new_v4()));
        let publish = root.join("publish");
        for (directory, file) in [("inventory", "Libraries.xml"), ("assets", "AssetSets.xml")] {
            fs::create_dir_all(publish.join(directory)).unwrap();
            fs::write(publish.join(directory).join(file), b"test").unwrap();
        }
        fs::write(publish.join("RegionServerDefaults.ini"), b"test").unwrap();
        let names = [
            "OpenSim.db",
            "OpenSim.db-wal",
            "OpenSim.db-shm",
            "OpenSim.db-journal",
            "auth.db",
            "inventory.db",
        ];
        for name in names {
            fs::write(publish.join(name), b"standalone state").unwrap();
        }
        for region in ["home", "neighbour"] {
            let cwd = root.join(region);
            prepare_region_cwd(&cwd, &publish, &root).unwrap();
            for name in names {
                assert!(
                    cwd.join(name).symlink_metadata().is_err(),
                    "{region}/{name}"
                );
            }
            fs::write(cwd.join("OpenSim.db"), region).unwrap();
        }
        assert_eq!(fs::read(root.join("home/OpenSim.db")).unwrap(), b"home");
        assert_eq!(
            fs::read(root.join("neighbour/OpenSim.db")).unwrap(),
            b"neighbour"
        );
        assert_eq!(
            fs::read(publish.join("OpenSim.db")).unwrap(),
            b"standalone state"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn sample_region<'a>(capture: Option<&'a str>) -> RegionIni<'a> {
        RegionIni {
            name: HOME_NAME,
            uuid: live::REGION_UUID,
            x: HOME_X,
            y: HOME_Y,
            http_bind: HOME_BIND,
            udp_bind: ADVERTISED,
            http_port: 9000,
            regions_dir: "/tmp/ferroan-regions",
            public_proxy: 8002,
            private_proxy: 8003,
            password: "generated-password",
            capture,
        }
    }

    #[test]
    fn the_region_ini_points_services_at_the_proxy_and_listens_elsewhere() {
        let ini = region_server_ini(&sample_region(Some("/tmp/ferroan-capture.cap"))).unwrap();
        assert!(ini.contains("Include-Architecture = \"config-include/Grid.ini\""));
        assert!(ini.contains("bind_ip_address = \"127.0.0.2\""));
        assert!(ini.contains("http_listener_port = 9000"));
        assert!(ini.contains("PublicPort = \"8002\""));
        assert!(ini.contains("PrivatePort = \"8003\""));
        assert!(ini.contains("CaptureFile = \"/tmp/ferroan-capture.cap\""));
        assert!(ini.contains("DefaultEstateOwnerPassword = \"generated-password\""));
        assert!(!ini.contains("127.0.0.4"));
        let regions = regions_ini(&sample_region(None));
        assert!(regions.contains("Location = 1000,1000"));
        assert!(regions.contains("InternalAddress = 127.0.0.1"));
        assert!(regions.contains("ExternalHostName = 127.0.0.1"));
        assert!(regions.contains("InternalPort = 9000"));
    }

    #[test]
    fn the_robust_ini_binds_its_own_address_and_has_no_password() {
        let ini = robust_ini(
            8002,
            8003,
            "/tmp/ferroan-robust.db",
            "/tmp/Libraries.xml",
            "/tmp/AssetSets.xml",
        )
        .unwrap();
        assert!(ini.contains("bind_ip_address = \"127.0.0.4\""));
        assert!(ini.contains("LLLoginServiceInConnector = \"8002/"));
        assert!(ini.contains("GridServiceConnector = \"8003/"));
        assert!(ini.contains("Region_Ferroan_Test = \"DefaultRegion\""));
        assert!(ini.contains(
            "Data Source=/tmp/ferroan-robust.db;Version=3;Journal Mode=Wal;BusyTimeout=15000;"
        ));
        assert!(ini.contains(
            "Data Source=/tmp/ferroan-robust.db.assets;Version=3;Journal Mode=Wal;BusyTimeout=15000;"
        ));
        assert!(ini.contains(
            "Data Source=/tmp/ferroan-robust.db.auth;Version=3;Journal Mode=Wal;BusyTimeout=15000;"
        ));
        assert_eq!(ini.matches("OpenSim.Data.Null.dll").count(), 2);
        assert!(ini.contains("StorageProvider = \"OpenSim.Data.SQLite.dll\""));
        assert!(ini.contains("DefaultLibrary = \"/tmp/Libraries.xml\""));
        assert!(!ini.contains("password"));
        assert!(plain("bad;path", "database").is_err());
    }

    #[test]
    fn a_proxy_capture_counts_requests_and_a_secret_is_visible() {
        use ferroan_capture::{Kind, Record, write_capture};
        let bytes = write_capture(&[
            Record {
                kind: Kind::HttpRequest,
                time_ms: 1,
                bytes: b"POST /grid HTTP/1.1\r\n\r\n".to_vec(),
            },
            Record {
                kind: Kind::HttpResponse,
                time_ms: 2,
                bytes: b"HTTP/1.1 200 OK\r\n\r\n".to_vec(),
            },
        ])
        .unwrap();
        assert_eq!(count_http_requests(&bytes).unwrap(), 1);
        assert!(contains_secret(b"create user secret", "secret"));
        assert!(!contains_secret(b"create user", ""));
        assert!(!contains_secret(b"create user", "secret"));
    }
}
