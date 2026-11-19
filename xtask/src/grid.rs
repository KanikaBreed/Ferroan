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
//! `Grid.ini` and `GridCommon.ini`. Service URLs point at `ferroan-capture`
//! proxies. The region HTTP listener is on another loopback address from
//! the address it advertises, so the proxy can bind the advertised port.
//! UDP stays on `127.0.0.1`. Each region has its own working directory, so
//! the relative SQLite files in `SQLiteStandalone.ini` do not collide. The
//! bot logs in on Robust's own public address.
//!
//! `--rust assets` also starts `ferroan-robust` on `127.0.0.1` with the
//! seeded SQLite asset store. C# Robust keeps the other services and drops
//! its asset connector. Each region then gets its own `GridCommon.ini`,
//! copied from the example, whose `AssetServerURI` is that Rust service.
//! `--rust accounts` starts the same process with its own UserAccount
//! SQLite file. C# Robust drops `UserAccountServiceConnector` and points
//! `[LoginService] UserAccountService` at
//! `OpenSim.Services.Connectors.dll:UserAccountServicesConnector`, which
//! reads `[UserAccountService] UserAccountServerURI`. Each region's
//! `UserAccountServerURI` is that same Rust URI. The published example is
//! not edited.
//!
//! The ini text follows GridServer.ini.example, Grid.ini and
//! GridCommon.ini.example at Tranquillity `40432e0`. Nothing here edits
//! those files.

#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::fs;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::live;

#[path = "grid_capture.rs"]
mod login_capture;
#[path = "grid_probe.rs"]
pub(crate) mod login_probe;

#[path = "grid_up.rs"]
mod persistent;

pub fn up(bind: Ipv4Addr) -> std::process::ExitCode {
    live::finish(persistent::up(bind))
}

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

pub fn run_with(spec: &crate::live_args::RunSpec) -> std::process::ExitCode {
    live::finish(run_inner(spec))
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

fn run_inner(spec: &crate::live_args::RunSpec) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        run_linux(spec)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = spec;
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
    advertised: Ipv4Addr,
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
    ControlPlaneTrustedHosts = "{advertised}"

[Estates]
    DefaultEstateName = "Ferroan Test Estate"
    DefaultEstateOwnerName = "Test User"
    DefaultEstateOwnerEMail = "test@example.invalid"
    DefaultEstateOwnerUUID = "{owner}"
    DefaultEstateOwnerPassword = "{password}"

[Architecture]
    Include-Architecture = "config-include/Grid.ini"
"#,
        advertised = region.advertised,
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
        advertised = region.advertised,
        udp_bind = region.udp_bind,
        port = region.http_port,
    )
}

struct ServiceUris<'a> {
    asset: Option<&'a str>,
    account: Option<&'a str>,
}

fn robust_ini(
    bind: Ipv4Addr,
    public_port: u16,
    private_port: u16,
    database: &str,
    library: &str,
    assets: &str,
    services: ServiceUris<'_>,
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
    let (asset_connector, asset_section) = if let Some(uri) = services.asset {
        service_uri(uri)?;
        (
            String::new(),
            format!("[AssetService]\n    AssetServerURI = \"{uri}\"\n\n"),
        )
    } else {
        (
            format!(
                "    AssetServiceConnector = \"{private_port}/OpenSim.Server.Handlers.dll:AssetServiceConnector\"\n"
            ),
            format!(
                "[AssetService]\n    \
                 LocalServiceModule = \"OpenSim.Services.AssetService.dll:AssetService\"\n    \
                 StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
                 ConnectionString = \"{asset_store}\"\n    \
                 DefaultAssetLoader = \"OpenSim.Framework.AssetLoader.Filesystem.dll\"\n    \
                 AssetLoaderArgs = \"{assets}\"\n\n"
            ),
        )
    };
    // LoginService.UserAccountService and the ServiceList connector are the
    // harness Robust loads of UserAccountService. The connector reads
    // UserAccountServerURI from [UserAccountService]. GridUserService stays
    // in process: it is a different class in the same DLL.
    let (account_connector, account_section, login_account, allow_set_password) = if let Some(uri) =
        services.account
    {
        service_uri(uri)?;
        (
            String::new(),
            format!("[UserAccountService]\n    UserAccountServerURI = \"{uri}\"\n\n"),
            "OpenSim.Services.Connectors.dll:UserAccountServicesConnector",
            "\n    AllowSetPassword = true",
        )
    } else {
        (
            format!(
                "    UserAccountServiceConnector = \"{private_port}/OpenSim.Server.Handlers.dll:UserAccountServiceConnector\"\n"
            ),
            "[UserAccountService]\n    \
                 LocalServiceModule = \"OpenSim.Services.UserAccountService.dll:UserAccountService\"\n    \
                 AuthenticationService = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\"\n    \
                 PresenceService = \"OpenSim.Services.PresenceService.dll:PresenceService\"\n    \
                 GridService = \"OpenSim.Services.GridService.dll:GridService\"\n    \
                 InventoryService = \"OpenSim.Services.InventoryService.dll:XInventoryService\"\n    \
                 AvatarService = \"OpenSim.Services.AvatarService.dll:AvatarService\"\n    \
                 GridUserService = \"OpenSim.Services.UserAccountService.dll:GridUserService\"\n    \
                 CreateDefaultAvatarEntries = true\n\n"
                .to_string(),
            "OpenSim.Services.UserAccountService.dll:UserAccountService",
            "",
        )
    };
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
{asset_connector}    InventoryInConnector = "{private_port}/OpenSim.Server.Handlers.dll:XInventoryInConnector"
    GridServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:GridServiceConnector"
    GridInfoServerInConnector = "{public_port}/OpenSim.Server.Handlers.dll:GridInfoServerInConnector"
    AuthenticationServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:AuthenticationServiceConnector"
    AvatarServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:AvatarServiceConnector"
    LLLoginServiceInConnector = "{public_port}/OpenSim.Server.Handlers.dll:LLLoginServiceInConnector"
    PresenceServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:PresenceServiceConnector"
{account_connector}    GridUserServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:GridUserServiceConnector"
    AgentPreferencesServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:AgentPreferencesServiceConnector"
    FriendsServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:FriendsServiceConnector"
    MapAddServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:MapAddServiceConnector"
    MapGetServiceConnector = "{public_port}/OpenSim.Server.Handlers.dll:MapGetServiceConnector"
    MuteListConnector = "{private_port}/OpenSim.Server.Handlers.dll:MuteListServiceConnector"

[DatabaseService]
    StorageProvider = "OpenSim.Data.SQLite.dll"
    ConnectionString = "{accounts}"

{asset_section}[InventoryService]
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
    ConnectionString = "{auth_store}"{allow_set_password}

{account_section}[GridUserService]
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
    UserAccountService = "{login_account}"
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
    ))
}

fn service_uri(uri: &str) -> Result<(), String> {
    if uri.is_empty() || uri.contains(['"', '\n', '\r', ' ', ';']) {
        return Err(
            "service URL is empty or contains a space, quote, break or semicolon".to_string(),
        );
    }
    Ok(())
}

/// Point one `key =` assignment at `uri`. Commented lines stay.
pub(crate) fn rewrite_named_server_uri(text: &str, key: &str, uri: &str) -> Result<String, String> {
    service_uri(uri)?;
    let mut found = false;
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        let trimmed = bare.trim();
        let assignment = !trimmed.starts_with(';') && assignment_name(trimmed) == Some(key);
        if assignment {
            if found {
                return Err(format!("GridCommon.ini has more than one {key}"));
            }
            found = true;
            out.push_str("    ");
            out.push_str(key);
            out.push_str(" = \"");
            out.push_str(uri);
            out.push('"');
            out.push_str(&line[bare.len()..]);
        } else {
            out.push_str(line);
        }
    }
    if !found {
        return Err(format!("GridCommon.ini has no {key}"));
    }
    Ok(out)
}

fn assignment_name(trimmed: &str) -> Option<&str> {
    let (name, _) = trimmed.split_once('=')?;
    let name = name.trim();
    if name.is_empty() { None } else { Some(name) }
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
    stdin: Option<Box<dyn std::io::Write>>,
    log: PathBuf,
    tracked: crate::grid_state::Tracked,
    detached: bool,
}

#[cfg(target_os = "linux")]
impl Drop for Proc {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if self.detached {
            return;
        }
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
    rust_assets: Option<crate::assets::Owned>,
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
        drop(self.rust_assets.take());
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
fn require_started<'a, T>(slot: &'a Option<T>, what: &str) -> Result<&'a T, String> {
    slot.as_ref()
        .ok_or_else(|| format!("{what} is not started"))
}

#[cfg(target_os = "linux")]
fn require_started_mut<'a, T>(slot: &'a mut Option<T>, what: &str) -> Result<&'a mut T, String> {
    slot.as_mut()
        .ok_or_else(|| format!("{what} is not started"))
}

#[cfg(target_os = "linux")]
fn run_linux(spec: &crate::live_args::RunSpec) -> Result<String, String> {
    let scenario = spec.scenario.as_str();
    let ferroan = spec.ferroan_region;
    let rust_assets = spec.rust_assets();
    let rust_accounts = spec.rust_accounts();
    if ferroan
        && !matches!(
            scenario,
            "login" | "caps" | "ferrobot" | "texture" | "inventory" | "assets"
        )
    {
        return Err(
            "the Ferroan grid region currently supports login, caps, ferrobot, texture, inventory and assets scenarios"
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
    let seeded = if rust_assets {
        Some(crate::assets::seed_library_database()?)
    } else {
        None
    };
    let ferrobot_binary = if scenario == "ferrobot" {
        Some(crate::ferrobot_build::build()?)
    } else {
        None
    };
    let port_count = if rust_assets || rust_accounts { 7 } else { 6 };
    let mut ports = Vec::new();
    for _ in 0..32 {
        if ports.len() == port_count {
            break;
        }
        let port = live::free_port()?;
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    if ports.len() != port_count {
        return Err(format!("could not allocate {port_count} local ports"));
    }
    let robust_public = ports[0];
    let robust_private = ports[1];
    let public_proxy = ports[2];
    let private_proxy = ports[3];
    let home_port = ports[4];
    let neighbour_port = ports[5];
    let rust_port = if rust_assets || rust_accounts {
        Some(ports[6])
    } else {
        None
    };
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
        rust_assets: None,
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
        Some(
            require_started(&grid.proxies, "proxies")?
                .block_on(ferrobot_capture::start_udp(
                    SocketAddr::from((ADVERTISED, home_port)),
                    SocketAddr::from((HOME_BIND, home_port)),
                    &root.join("capture-udp.cap"),
                ))?
                .0,
        )
    } else {
        None
    };
    let (asset_uri, account_uri) = if let Some(port) = rust_port {
        let uri = format!("http://127.0.0.1:{port}");
        let asset_database = if rust_assets {
            let seed = seeded.as_ref().ok_or("the asset seed was not prepared")?;
            let database = root.join("assets.db");
            fs::copy(seed, &database)
                .map_err(|error| format!("could not copy the asset database: {error}"))?;
            Some(database)
        } else {
            None
        };
        let account_database = if rust_accounts {
            let fixture = crate::repo_root().join("fixtures/db/sqlite/UserAccount.db");
            if !fixture.is_file() {
                return Err(format!("missing {}", fixture.display()));
            }
            let database = root.join("accounts.db");
            fs::copy(&fixture, &database)
                .map_err(|error| format!("could not copy the account database: {error}"))?;
            Some(database)
        } else {
            None
        };
        let (log_name, ini_name) = if rust_accounts {
            ("rust-services.log", "rust-services.ini")
        } else {
            ("assets-ferroan.log", "assets-ferroan.ini")
        };
        grid.rust_assets = Some(crate::assets::start_grid_services(
            asset_database.as_deref(),
            account_database.as_deref(),
            port,
            &root.join(log_name),
            &root.join(ini_name),
        )?);
        if rust_assets {
            println!("live: rust assets on {uri}");
        }
        if rust_accounts {
            println!("live: rust accounts on {uri}");
        }
        (
            rust_assets.then(|| uri.clone()),
            rust_accounts.then_some(uri),
        )
    } else {
        (None, None)
    };
    println!(
        "live: grid Robust on {ROBUST_BIND}:{robust_public} (private {robust_private}), proxies {public_proxy}/{private_proxy}"
    );
    grid.robust = Some(start_robust(
        &root,
        robust_public,
        robust_private,
        &grid.passwords,
        ServiceUris {
            asset: asset_uri.as_deref(),
            account: account_uri.as_deref(),
        },
    )?);
    if rust_accounts {
        let log = require_started(&grid.robust, "Robust")?.log.clone();
        let text = fs::read_to_string(&log)
            .map_err(|error| format!("could not read {}: {error}", log.display()))?;
        if in_process_user_account(&text) {
            return Err(
                "C# Robust loaded UserAccountService in process; --rust accounts requires the remote connector"
                    .to_string(),
            );
        }
    }
    provision_account(
        require_started_mut(&mut grid.robust, "Robust")?,
        account_uri.as_deref(),
        robust_private,
        AccountSeed {
            first: "Test",
            last: "User",
            password: &estate,
            email: "test@example.invalid",
            uuid: live::ESTATE_OWNER,
        },
        &grid.passwords,
    )?;
    provision_account(
        require_started_mut(&mut grid.robust, "Robust")?,
        account_uri.as_deref(),
        robust_private,
        AccountSeed {
            first: "Chat",
            last: "Bot",
            password: &chat,
            email: "chat@example.invalid",
            uuid: live::CHAT_USER,
        },
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
            login_only: ferroan || matches!(scenario, "login" | "caps" | "ferrobot"),
            name: HOME_NAME,
            uuid: live::REGION_UUID,
            x: HOME_X,
            y: HOME_Y,
            http_bind: HOME_BIND,
            advertised: ADVERTISED,
            udp_bind: if ferroan && scenario == "ferrobot" {
                HOME_BIND
            } else {
                ADVERTISED
            },
            http_port: home_port,
            public_proxy,
            private_proxy,
            capture: Some(root.join("capture-udp.cap")),
            asset_uri: asset_uri.clone(),
            account_uri: account_uri.clone(),
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
            advertised: ADVERTISED,
            udp_bind: ADVERTISED,
            http_port: neighbour_port,
            public_proxy,
            private_proxy,
            capture: None,
            asset_uri: asset_uri.clone(),
            account_uri: account_uri.clone(),
        },
        &grid.passwords,
    )?);
    let home_log = require_started(&grid.home, "home region")?.log.clone();
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
        if ferroan && matches!(scenario, "texture" | "inventory" | "assets") {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("could not start inventory probe runtime: {error}"))?;
            let line = runtime.block_on(crate::inventory_probe::compare(
                &format!("http://{ROBUST_BIND}:{robust_public}/"),
                &estate,
                &format!("http://{ADVERTISED}:{private_proxy}"),
                &root,
            ))?;
            report.lines.push('\n');
            report.lines.push_str(&line);
        }
        Ok(report)
    })
    };
    let relay_result = if let Some(task) = udp_relay {
        if task.is_finished() {
            require_started(&grid.proxies, "proxies").and_then(|proxies| {
                proxies
                    .block_on(task)
                    .map_err(|error| format!("UDP capture task stopped: {error}"))
                    .and_then(|result| result)
            })
        } else {
            task.abort();
            // Finish cancellation before reading the capture or stopping the
            // runtime, so the relay has released its socket and output file.
            if let Ok(proxies) = require_started(&grid.proxies, "proxies") {
                let _ = proxies.block_on(task);
            }
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
        let logout_region = login_capture::compare_logout_region(&root)?;
        Ok(format!(
            "{}\nlogout: LogoutReply\n{comparison}\n{arrival}\n{logout_region}\n{proxy_report}",
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
    services: ServiceUris<'_>,
) -> Result<Proc, String> {
    start_robust_at(
        root,
        public_port,
        private_port,
        passwords,
        ROBUST_BIND,
        None,
        services,
    )
}

#[cfg(target_os = "linux")]
fn start_robust_at(
    root: &Path,
    public_port: u16,
    private_port: u16,
    passwords: &[String],
    bind: Ipv4Addr,
    console: Option<&fs::File>,
    services: ServiceUris<'_>,
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
        bind,
        public_port,
        private_port,
        database.to_str().ok_or("database path is not utf-8")?,
        library.to_str().ok_or("library path is not utf-8")?,
        assets.to_str().ok_or("asset set path is not utf-8")?,
        services,
    )?;
    let ini_path = root.join("robust.ini");
    live::write_secret(&ini_path, &ini)?;
    let empty = root.join("robust-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let log = root.join("robust.log");
    let ini_arg = ini_path.to_str().ok_or("robust ini path is not utf-8")?;
    let empty_arg = empty.to_str().ok_or("robust config path is not utf-8")?;
    let mut proc = spawn_proc_with_input(
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
        console,
    )?;
    wait_log(
        &mut proc.child,
        &log,
        PROMPT,
        live::START_TIMEOUT,
        passwords,
    )?;
    live::require_listen(proc.child.id(), bind, public_port)?;
    live::require_listen(proc.child.id(), bind, private_port)?;
    Ok(proc)
}

#[cfg(target_os = "linux")]
struct AccountSeed<'a> {
    first: &'a str,
    last: &'a str,
    password: &'a str,
    email: &'a str,
    uuid: &'a str,
}

#[cfg(target_os = "linux")]
fn provision_account(
    proc: &mut Proc,
    account_uri: Option<&str>,
    private_port: u16,
    user: AccountSeed<'_>,
    passwords: &[String],
) -> Result<(), String> {
    if let Some(uri) = account_uri {
        provision_remote_account(
            uri,
            private_port,
            user.first,
            user.last,
            user.password,
            user.email,
            user.uuid,
        )?;
        if live::log_contains(&proc.log, user.password)? {
            return Err("the Robust log contains a password".to_string());
        }
        return Ok(());
    }
    create_account(
        proc,
        user.first,
        user.last,
        user.password,
        user.email,
        user.uuid,
        passwords,
    )
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

/// The in-process `create user` command is registered by `UserAccountService`.
/// With the remote connector, the account row is `POST /accounts`, the
/// password is `POST /auth/plain` on C# Robust, and the inventory skeleton is
/// `POST /xinventory`. `AuthenticationServicesConnector` posts to `/auth/plain`;
/// `/auth` itself returns an empty body. None of these requests is written to a log.
#[cfg(target_os = "linux")]
fn provision_remote_account(
    account_uri: &str,
    private_port: u16,
    first: &str,
    last: &str,
    password: &str,
    email: &str,
    uuid: &str,
) -> Result<(), String> {
    let account_port = rust_service_port(account_uri)?;
    let created = post_form(
        SocketAddr::from((Ipv4Addr::LOCALHOST, account_port)),
        "/accounts",
        &format!(
            "METHOD=createuser&ScopeID=00000000-0000-0000-0000-000000000000&FirstName={first}&LastName={last}&Password={password}&Email={email}&PrincipalID={uuid}"
        ),
    )?;
    if created.contains(password) {
        return Err("the account response contains a password".to_string());
    }
    if !created.contains(uuid) {
        return Err(format!("could not create account {first} {last}"));
    }
    let auth = post_form(
        SocketAddr::from((ROBUST_BIND, private_port)),
        "/auth/plain",
        &format!("METHOD=setpassword&PRINCIPAL={uuid}&PASSWORD={password}"),
    )?;
    if auth.contains(password) {
        return Err("the authentication response contains a password".to_string());
    }
    if !auth.contains("Success") {
        return Err(format!("could not store the password for {first} {last}"));
    }
    let inventory = post_form(
        SocketAddr::from((ROBUST_BIND, private_port)),
        "/xinventory",
        &format!("METHOD=CREATEUSERINVENTORY&PRINCIPAL={uuid}"),
    )?;
    if !inventory.contains("True") {
        return Err(format!("could not create inventory for {first} {last}"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn rust_service_port(uri: &str) -> Result<u16, String> {
    let rest = uri
        .strip_prefix("http://127.0.0.1:")
        .ok_or("the account service URL is not on 127.0.0.1")?;
    rest.parse::<u16>()
        .map_err(|_| "the account service URL has no port".to_string())
}

#[cfg(target_os = "linux")]
fn post_form(address: SocketAddr, path: &str, body: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|error| format!("could not connect to {address}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|error| format!("could not set the read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| format!("could not set the write timeout: {error}"))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("could not write {path}: {error}"))?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read {path}: {error}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let Some((headers, response)) = text.split_once("\r\n\r\n") else {
        return Err(format!("{path} returned no HTTP body"));
    };
    let status = headers.lines().next().unwrap_or("");
    if !status.starts_with("HTTP/1.1 200") && !status.starts_with("HTTP/1.0 200") {
        return Err(format!("{path} returned {status}"));
    }
    Ok(response.to_string())
}

fn in_process_user_account(log: &str) -> bool {
    log.contains("UserAccountServiceConnector loaded successfully")
        || log.contains("[USER ACCOUNT SERVICE]")
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
    advertised: Ipv4Addr,
    http_port: u16,
    public_proxy: u16,
    private_proxy: u16,
    capture: Option<PathBuf>,
    asset_uri: Option<String>,
    account_uri: Option<String>,
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
    let mut rewrites = Vec::new();
    if let Some(uri) = &spec.asset_uri {
        rewrites.push(("AssetServerURI", uri.as_str()));
    }
    if let Some(uri) = &spec.account_uri {
        rewrites.push(("UserAccountServerURI", uri.as_str()));
    }
    if !rewrites.is_empty() {
        install_grid_common(cwd, &rewrites)?;
    }
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
        advertised: spec.advertised,
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
    // working directory, and configDir() is that same directory. Link only
    // published assemblies and static content. maptiles/, MeshCache/, console
    // history, the daily log and other runtime files stay per region.
    let entries = fs::read_dir(publish)
        .map_err(|error| format!("could not read {}: {error}", publish.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not read {}: {error}", publish.display()))?;
        let name = entry.file_name();
        let is_dir = entry
            .file_type()
            .map_err(|error| format!("could not read {}: {error}", entry.path().display()))?
            .is_dir();
        if !published_region_entry(&name, is_dir) {
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

/// Static publish output the region opens from its working directory.
/// Runtime names from the pin (`maptiles`, `MeshCache`, `OpenSimConsoleHistory.txt`,
/// `OpenSim.log` and the dated `OpenSim.Server.RegionServerYYYYMMDD.log`) are absent
/// on purpose. A new static publish kind has to be added here.
#[cfg(target_os = "linux")]
fn published_region_entry(name: &std::ffi::OsStr, is_dir: bool) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if is_dir {
        return matches!(
            name,
            "config-include"
                | "inventory"
                | "assets"
                | "data"
                | "Library"
                | "openmetaverse_data"
                | "runtimes"
                | "Scripts"
        ) || is_dotnet_satellite(name);
    }
    if live::is_sqlite_file(name) || is_region_runtime_file(name) {
        return false;
    }
    name == "OpenSim.Server.RegionServer"
        || matches!(
            Path::new(name).extension().and_then(|ext| ext.to_str()),
            Some(
                "dll"
                    | "pdb"
                    | "so"
                    | "dylib"
                    | "json"
                    | "config"
                    | "xml"
                    | "ini"
                    | "example"
                    | "txt"
                    | "html"
            )
        )
}

#[cfg(target_os = "linux")]
fn is_region_runtime_file(name: &str) -> bool {
    name == "OpenSimConsoleHistory.txt" || name.ends_with(".log") || name.ends_with(".dat")
}

#[cfg(target_os = "linux")]
fn is_dotnet_satellite(name: &str) -> bool {
    let mut parts = name.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    let region = parts.next();
    language.len() == 2
        && language.bytes().all(|byte| byte.is_ascii_lowercase())
        && parts.next().is_none()
        && region.is_none_or(|region| {
            (2..=8).contains(&region.len())
                && region.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
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

/// Replace the published `config-include` link with a region-local copy so
/// one service URI can name `ferroan-robust` without editing the shared tree.
#[cfg(target_os = "linux")]
fn install_grid_common(cwd: &Path, rewrites: &[(&str, &str)]) -> Result<(), String> {
    let linked = cwd.join("config-include");
    let meta = fs::symlink_metadata(&linked)
        .map_err(|error| format!("could not inspect {}: {error}", linked.display()))?;
    if !meta.file_type().is_symlink() {
        return Err(format!(
            "{} is not the published config-include link",
            linked.display()
        ));
    }
    let source = fs::read_link(&linked)
        .map_err(|error| format!("could not read {}: {error}", linked.display()))?;
    let source = if source.is_absolute() {
        source
    } else {
        cwd.join(source)
    };
    let example = fs::read_to_string(source.join("GridCommon.ini")).map_err(|error| {
        format!(
            "could not read {}: {error}",
            source.join("GridCommon.ini").display()
        )
    })?;
    let mut rewritten = example;
    for (key, uri) in rewrites {
        rewritten = rewrite_named_server_uri(&rewritten, key, uri)?;
    }
    fs::remove_file(&linked)
        .map_err(|error| format!("could not replace {}: {error}", linked.display()))?;
    fs::create_dir(&linked)
        .map_err(|error| format!("could not create {}: {error}", linked.display()))?;
    let entries = fs::read_dir(&source)
        .map_err(|error| format!("could not read {}: {error}", source.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not read {}: {error}", source.display()))?;
        let name = entry.file_name();
        let dest = linked.join(&name);
        if name == "GridCommon.ini" {
            fs::write(&dest, &rewritten)
                .map_err(|error| format!("could not write {}: {error}", dest.display()))?;
        } else {
            link_path(&dest, &entry.path())?;
        }
    }
    Ok(())
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
    spawn_proc_with_input(binary, args, cwd, log, pipe_stdin, None)
}

#[cfg(target_os = "linux")]
fn spawn_proc_with_input(
    binary: &Path,
    args: &[&str],
    cwd: &Path,
    log: &Path,
    pipe_stdin: bool,
    input: Option<&fs::File>,
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
    let console = input
        .map(|file| {
            file.try_clone()
                .map_err(|error| format!("could not clone grid console: {error}"))
        })
        .transpose()?;
    if let Some(file) = input {
        command
            .stdin(Stdio::from(file.try_clone().map_err(|error| {
                format!("could not clone grid console: {error}")
            })?));
    } else if pipe_stdin {
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
    let stdin = console
        .map(|file| Box::new(file) as Box<dyn std::io::Write>)
        .or_else(|| {
            child
                .stdin
                .take()
                .map(|pipe| Box::new(pipe) as Box<dyn std::io::Write>)
        });
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
        detached: false,
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
        fs::write(publish.join("OpenSim.Region.Framework.dll"), b"assembly").unwrap();
        fs::create_dir_all(publish.join("config-include")).unwrap();
        fs::write(publish.join("config-include/Grid.ini"), b"grid").unwrap();
        fs::create_dir_all(publish.join("de")).unwrap();
        fs::write(publish.join("de/Satellite.resources.dll"), b"sat").unwrap();
        for (directory, bytes) in [
            ("maptiles", "tile.jpg"),
            ("MeshCache", "cntr"),
            ("ScriptEngines", "state"),
            ("Regions", "Regions.ini"),
            ("assetcache", "blob"),
        ] {
            fs::create_dir_all(publish.join(directory)).unwrap();
            fs::write(publish.join(directory).join(bytes), b"shared").unwrap();
        }
        fs::write(publish.join("OpenSimConsoleHistory.txt"), b"shared").unwrap();
        fs::write(
            publish.join("OpenSim.Server.RegionServer20261118.log"),
            b"shared",
        )
        .unwrap();
        fs::write(publish.join("avatar-texture.dat"), b"shared").unwrap();
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
            for name in [
                "maptiles",
                "MeshCache",
                "ScriptEngines",
                "Regions",
                "assetcache",
                "OpenSimConsoleHistory.txt",
                "OpenSim.Server.RegionServer20261118.log",
                "avatar-texture.dat",
            ] {
                assert!(
                    cwd.join(name).symlink_metadata().is_err(),
                    "{region} linked runtime {name}"
                );
            }
            assert_eq!(
                fs::read(cwd.join("OpenSim.Region.Framework.dll")).unwrap(),
                b"assembly"
            );
            assert_eq!(
                fs::read(cwd.join("config-include/Grid.ini")).unwrap(),
                b"grid"
            );
            assert_eq!(
                fs::read(cwd.join("de/Satellite.resources.dll")).unwrap(),
                b"sat"
            );
            fs::create_dir_all(cwd.join("maptiles")).unwrap();
            fs::write(cwd.join("maptiles/own.jpg"), region).unwrap();
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
        assert_eq!(
            fs::read(publish.join("maptiles/tile.jpg")).unwrap(),
            b"shared"
        );
        assert_eq!(
            fs::read(root.join("home/maptiles/own.jpg")).unwrap(),
            b"home"
        );
        assert_eq!(
            fs::read(root.join("neighbour/maptiles/own.jpg")).unwrap(),
            b"neighbour"
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
            advertised: ADVERTISED,
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
            ROBUST_BIND,
            8002,
            8003,
            "/tmp/ferroan-robust.db",
            "/tmp/Libraries.xml",
            "/tmp/AssetSets.xml",
            ServiceUris {
                asset: None,
                account: None,
            },
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
        assert!(ini.contains("AssetServiceConnector"));
        assert!(ini.contains("AssetLoaderArgs = \"/tmp/AssetSets.xml\""));
    }

    #[test]
    fn a_rust_asset_uri_replaces_robust_local_assets_and_one_gridcommon_line() {
        let ini = robust_ini(
            ROBUST_BIND,
            8002,
            8003,
            "/tmp/ferroan-robust.db",
            "/tmp/Libraries.xml",
            "/tmp/AssetSets.xml",
            ServiceUris {
                asset: Some("http://127.0.0.1:9"),
                account: None,
            },
        )
        .unwrap();
        assert!(ini.contains("AssetServerURI = \"http://127.0.0.1:9\""));
        assert!(!ini.contains("AssetServiceConnector"));
        assert!(!ini.contains("OpenSim.Services.AssetService.dll:AssetService"));
        assert!(!ini.contains("ferroan-robust.db.assets"));
        assert!(ini.contains("InventoryInConnector"));
        assert!(ini.contains("DefaultLibrary = \"/tmp/Libraries.xml\""));
        assert!(
            robust_ini(
                ROBUST_BIND,
                8002,
                8003,
                "/tmp/ferroan-robust.db",
                "/tmp/Libraries.xml",
                "/tmp/AssetSets.xml",
                ServiceUris {
                    asset: Some("http://bad uri"),
                    account: None,
                },
            )
            .is_err()
        );

        let common = "\
[AssetService]\n    \
    AssetServerURI = \"${Const|PrivURL}:${Const|PrivatePort}\"\n\n\
[InventoryService]\n    \
    InventoryServerURI = \"${Const|PrivURL}:${Const|PrivatePort}\"\n\
    ; RegionHGAssetServerURI = ${Const|BaseURL}:${Const|PublicPort}\n";
        let pointed =
            rewrite_named_server_uri(common, "AssetServerURI", "http://127.0.0.1:9").unwrap();
        assert!(pointed.contains("AssetServerURI = \"http://127.0.0.1:9\""));
        assert!(!pointed.contains("${Const|PrivURL}:${Const|PrivatePort}\"\n\n[Inventory"));
        assert!(pointed.contains("InventoryServerURI = \"${Const|PrivURL}:${Const|PrivatePort}\""));
        assert!(pointed.contains("RegionHGAssetServerURI"));
        assert!(
            rewrite_named_server_uri("[AssetService]\n", "AssetServerURI", "http://127.0.0.1:9")
                .is_err()
        );
        let twice = "AssetServerURI = \"a\"\nAssetServerURI = \"b\"\n";
        assert!(rewrite_named_server_uri(twice, "AssetServerURI", "http://127.0.0.1:9").is_err());
    }

    #[test]
    fn a_rust_account_uri_points_login_at_the_remote_connector() {
        let ini = robust_ini(
            ROBUST_BIND,
            8002,
            8003,
            "/tmp/ferroan-robust.db",
            "/tmp/Libraries.xml",
            "/tmp/AssetSets.xml",
            ServiceUris {
                asset: None,
                account: Some("http://127.0.0.1:9"),
            },
        )
        .unwrap();
        assert!(ini.contains("UserAccountServerURI = \"http://127.0.0.1:9\""));
        assert!(ini.contains(
            "UserAccountService = \"OpenSim.Services.Connectors.dll:UserAccountServicesConnector\""
        ));
        assert!(!ini.contains("UserAccountServiceConnector"));
        assert!(!ini.contains("OpenSim.Services.UserAccountService.dll:UserAccountService"));
        assert!(ini.contains("OpenSim.Services.UserAccountService.dll:GridUserService"));
        assert!(ini.contains("AllowSetPassword = true"));
        assert!(ini.contains("AssetServiceConnector"));
        assert!(!ini.contains("CreateDefaultAvatarEntries"));
        assert!(in_process_user_account(
            "[SERVER]: UserAccountServiceConnector loaded successfully"
        ));
        assert!(in_process_user_account(
            "[USER ACCOUNT SERVICE]: Account created successfully"
        ));
        assert!(!in_process_user_account(
            "[SERVER]: GridUserServiceConnector loaded successfully\n[GRID USER SERVICE]: Starting user grid service"
        ));

        let common = "[UserAccountService]\n    UserAccountServerURI = \"${Const|PrivURL}:${Const|PrivatePort}\"\n";
        let pointed =
            rewrite_named_server_uri(common, "UserAccountServerURI", "http://127.0.0.1:9").unwrap();
        assert!(pointed.contains("UserAccountServerURI = \"http://127.0.0.1:9\""));
        assert!(!pointed.contains("${Const|PrivURL}"));

        let both = robust_ini(
            ROBUST_BIND,
            8002,
            8003,
            "/tmp/ferroan-robust.db",
            "/tmp/Libraries.xml",
            "/tmp/AssetSets.xml",
            ServiceUris {
                asset: Some("http://127.0.0.1:9"),
                account: Some("http://127.0.0.1:9"),
            },
        )
        .unwrap();
        assert!(both.contains("AssetServerURI = \"http://127.0.0.1:9\""));
        assert!(both.contains("UserAccountServerURI = \"http://127.0.0.1:9\""));
        assert!(!both.contains("AssetServiceConnector"));
        assert!(!both.contains("UserAccountServiceConnector"));
    }

    #[test]
    fn persistent_grid_config_advertises_and_trusts_its_direct_bind() {
        for bind in [Ipv4Addr::LOCALHOST, Ipv4Addr::new(192, 0, 2, 7)] {
            let mut region = sample_region(None);
            region.http_bind = bind;
            region.udp_bind = bind;
            region.advertised = bind;
            let server = region_server_ini(&region).unwrap();
            assert!(server.contains(&format!("bind_ip_address = \"{bind}\"")));
            assert!(server.contains(&format!("ControlPlaneTrustedHosts = \"{bind}\"")));
            assert!(server.contains(&format!("BaseHostname = \"{bind}\"")));
            let regions = regions_ini(&region);
            assert!(regions.contains(&format!("InternalAddress = {bind}")));
            assert!(regions.contains(&format!("ExternalHostName = {bind}")));
            let robust = robust_ini(
                bind,
                8002,
                8003,
                "/tmp/grid.db",
                "/tmp/library.xml",
                "/tmp/assets.xml",
                ServiceUris {
                    asset: None,
                    account: None,
                },
            )
            .unwrap();
            assert!(robust.contains(&format!("bind_ip_address = \"{bind}\"")));
            assert!(robust.contains(&format!("login = http://{bind}:8002/")));
        }
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
