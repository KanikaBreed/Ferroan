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

//! Grid mode: `cargo xtask live run <scenario>|all|neighbour|report grid`.
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

#[path = "avatars_capture.rs"]
mod avatars_capture;
#[path = "child_capture.rs"]
mod child_capture;
#[path = "grid_capture.rs"]
mod login_capture;
#[path = "grid_probe.rs"]
pub(crate) mod login_probe;
#[path = "prim_arrival.rs"]
mod prim_arrival;
#[path = "prim_capture.rs"]
mod prim_capture;

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

pub fn object_cross_fixtures(directory: &Path) -> std::process::ExitCode {
    live::finish(login_capture::object_cross::extract(directory))
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
        run_linux(spec, false)
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
    prim_capture: bool,
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
    basic_physics: bool,
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
    let physics_line = if region.basic_physics {
        "\n    physics = basicphysics"
    } else {
        ""
    };
    let mut ini = format!(
        r#"[Const]
    BaseHostname = "{advertised}"
    BaseURL = http://${{Const|BaseHostname}}
    PublicPort = "{public_proxy}"
    PrivURL = ${{Const|BaseURL}}
    PrivatePort = "{private_proxy}"

[Startup]
    regionload_regionsdir = "{dir}"{physics_line}
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
        if region.prim_capture {
            ini.push_str("    SupportViewerObjectsCache = false\n");
        }
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
    auth: Option<&'a str>,
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
    // LLLoginService loads IAuthenticationService with (config, userAccountService).
    // AuthenticationServicesConnector has no such constructor, so login stays on
    // PasswordAuthenticationService and opens the same auth.db ferroan-robust
    // serves. The in-process account service loads the one-argument connector.
    // Its SetPassword is a no-op; the harness posts setpassword. The HTTP
    // handler stays omitted so C# does not also listen on /auth.
    let password_module =
        "OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService";
    let remote_auth = "OpenSim.Services.Connectors.dll:AuthenticationServicesConnector";
    let (auth_connector, login_auth, account_auth, remote_auth_uri) = if let Some(uri) =
        services.auth
    {
        service_uri(uri)?;
        (String::new(), password_module, remote_auth, Some(uri))
    } else {
        (
            format!(
                "    AuthenticationServiceConnector = \"{private_port}/OpenSim.Server.Handlers.dll:AuthenticationServiceConnector\"\n"
            ),
            password_module,
            password_module,
            None,
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
            format!(
                "[UserAccountService]\n    \
                 LocalServiceModule = \"OpenSim.Services.UserAccountService.dll:UserAccountService\"\n    \
                 AuthenticationService = \"{account_auth}\"\n    \
                 PresenceService = \"OpenSim.Services.PresenceService.dll:PresenceService\"\n    \
                 GridService = \"OpenSim.Services.GridService.dll:GridService\"\n    \
                 InventoryService = \"OpenSim.Services.InventoryService.dll:XInventoryService\"\n    \
                 AvatarService = \"OpenSim.Services.AvatarService.dll:AvatarService\"\n    \
                 GridUserService = \"OpenSim.Services.UserAccountService.dll:GridUserService\"\n    \
                 CreateDefaultAvatarEntries = true\n\n"
            ),
            "OpenSim.Services.UserAccountService.dll:UserAccountService",
            "",
        )
    };
    let auth_section = if let Some(uri) = remote_auth_uri {
        // start_grid copies the fixture to auth.db beside robust.db. Keep the
        // separator in the string: Path::with_file_name rewrites `/tmp/...` on
        // Windows, and this ini is built in tests there. BusyTimeout only.
        // Journal Mode=Wal would try to change the mode while Ferroan already
        // holds the file.
        let auth_path = shared_auth_path(database);
        plain(&auth_path, "auth database path")?;
        let shared = format!("Data Source={auth_path};Version=3;BusyTimeout=15000;");
        format!(
            "[AuthenticationService]\n    \
             AuthenticationServerURI = \"{uri}\"\n    \
             StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
             ConnectionString = \"{shared}\"\n\n"
        )
    } else {
        format!(
            "[AuthenticationService]\n    \
             LocalServiceModule = \"{password_module}\"\n    \
             StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
             ConnectionString = \"{auth_store}\"{allow_set_password}\n\n"
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
{auth_connector}    AvatarServiceConnector = "{private_port}/OpenSim.Server.Handlers.dll:AvatarServiceConnector"
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

{auth_section}{account_section}[GridUserService]
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
    AuthenticationService = "{login_auth}"
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

/// `auth.db` in the same directory as Robust's database path.
///
/// The separator stays as written. Live runs are Linux. The unit test passes
/// a `/tmp/...` string on Windows, where `Path` would emit a backslash.
fn shared_auth_path(database: &str) -> String {
    match database.rfind(['/', '\\']) {
        Some(index) => format!("{}auth.db", &database[..=index]),
        None => "auth.db".to_string(),
    }
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
    if crate::ports::isolated() {
        crate::ports::run_root().join("grid")
    } else {
        live::live_root().join("grid")
    }
}

#[cfg(target_os = "linux")]
struct Proc {
    child: Child,
    stdin: Option<Box<dyn std::io::Write>>,
    log: PathBuf,
    tracked: crate::grid_state::Tracked,
    detached: bool,
    container: Option<String>,
    listen_pid: u32,
}

#[cfg(target_os = "linux")]
impl Drop for Proc {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if self.detached {
            return;
        }
        if let Some(name) = self.container.take() {
            crate::docker::stop_container(&name);
        }
        stop_child(&mut self.child);
        if let Err(error) = self.tracked.forget() {
            eprintln!("live: grid process cleanup: {error}");
        }
    }
}

/// Indexes of the grid's capture proxies, in the order `run_linux` starts them.
const PROXY_ROBUST_PRIVATE: usize = 0;
const PROXY_ROBUST_PUBLIC: usize = 1;
const PROXY_HOME: usize = 2;
const PROXY_NEIGHBOUR: usize = 3;

/// The proxies in front of home, the neighbour and Robust, in the order
/// `Grid::stop` stops those servers.
const STOP_ORDER: [&[usize]; 3] = [
    &[PROXY_HOME],
    &[PROXY_NEIGHBOUR],
    &[PROXY_ROBUST_PRIVATE, PROXY_ROBUST_PUBLIC],
];

/// Stop each started server in `STOP_ORDER`, marking its proxies first.
///
/// A server still running may call one already gone (the C# neighbour's
/// CloseAgent to home after the bot logs out of it); that proxy then keeps the
/// request without failing. The mark comes just before the stop, so a refusal
/// while the server is meant to be up still fails the run.
#[cfg(target_os = "linux")]
fn stop_in_order<T>(slots: [&mut Option<T>; 3], proxies: Option<&crate::live_proxy::Proxies>) {
    for (slot, indexes) in slots.into_iter().zip(STOP_ORDER) {
        if slot.is_none() {
            continue;
        }
        if let Some(proxies) = proxies {
            for index in indexes {
                proxies.upstream_stopping(*index);
            }
        }
        drop(slot.take());
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
        stop_in_order(
            [&mut self.home, &mut self.neighbour, &mut self.robust],
            self.proxies.as_ref(),
        );
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
    let _ = crate::signal::signal_group(crate::signal::Signal::Term, pid);
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) if started.elapsed() >= Duration::from_secs(30) => break,
            Ok(None) => thread::sleep(Duration::from_millis(200)),
        }
    }
    let _ = crate::signal::signal_group(crate::signal::Signal::Kill, pid);
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
pub(crate) fn run_docker(spec: &crate::live_args::RunSpec) -> Result<String, String> {
    run_linux(spec, true)
}

#[cfg(target_os = "linux")]
fn run_linux(spec: &crate::live_args::RunSpec, docker: bool) -> Result<String, String> {
    let scenario = spec.scenario.as_str();
    let ferroan = spec.ferroan_region;
    let rust_assets = spec.rust_assets();
    let rust_accounts = spec.rust_accounts();
    let rust_auth = spec.rust_auth();
    if ferroan
        && !matches!(
            scenario,
            "login"
                | "caps"
                | "ferrobot"
                | "texture"
                | "inventory"
                | "assets"
                | "limits"
                | "roots"
                | "chat"
                | "teleport"
                | "hop"
                | "uploads"
                | "avatars"
                | "neighbour"
                | "walk"
                | "report"
        )
    {
        return Err(
            "the Ferroan grid region currently supports login, caps, ferrobot, texture, inventory, assets, limits, roots, chat, teleport, hop, uploads, avatars, neighbour, walk and report scenarios"
                .to_string(),
        );
    }
    if scenario == "limits" && !ferroan {
        return Err("the limits scenario runs on the Ferroan region".to_string());
    }
    if !matches!(
        scenario,
        "neighbour"
            | "caps"
            | "ferrobot"
            | "limits"
            | "roots"
            | "hop"
            | "uploads"
            | "avatars"
            | "object-cross"
            | "child-view"
            | "prim-capture"
            | "prim-arrival"
            | "report"
    ) && ferroan_bot::scenario::selection(scenario).is_none()
    {
        return Err(format!(
            "unknown scenario {scenario}; expected all, neighbour, object-cross, child-view, prim-arrival, prim-capture (live run prim-capture grid --physics basicphysics) or {}",
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
    let port_count = 6
        + usize::from(rust_assets || rust_accounts || rust_auth)
        + usize::from(ferroan)
        + usize::from(rust_assets || rust_accounts || rust_auth);
    let ports = crate::ports::alloc_grid(port_count)?;
    let robust_public = ports[0];
    let robust_private = ports[1];
    let public_proxy = ports[2];
    let private_proxy = ports[3];
    let home_port = ports[4];
    let neighbour_port = ports[5];
    let rust_services = rust_assets || rust_accounts || rust_auth;
    let rust_port = rust_services.then(|| ports[6]);
    let region_metrics = ferroan.then(|| ports[6 + usize::from(rust_services)]);
    let robust_metrics =
        rust_services.then(|| ports[6 + usize::from(rust_services) + usize::from(ferroan)]);
    let root = grid_root();
    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|error| format!("could not clear {}: {error}", root.display()))?;
    }
    fs::create_dir_all(&root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;
    crate::ports::write_fixture_ports(&root)?;
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
    // In the order of PROXY_ROBUST_PRIVATE, PROXY_ROBUST_PUBLIC, PROXY_HOME
    // and PROXY_NEIGHBOUR, which STOP_ORDER uses.
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
    let (asset_uri, account_uri, auth_uri) = if let Some(port) = rust_port {
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
        let auth_database = if rust_auth {
            let fixture = crate::repo_root().join("fixtures/db/sqlite/AuthStore.db");
            if !fixture.is_file() {
                return Err(format!("missing {}", fixture.display()));
            }
            let database = root.join("auth.db");
            fs::copy(&fixture, &database)
                .map_err(|error| format!("could not copy the auth database: {error}"))?;
            Some(database)
        } else {
            None
        };
        let (log_name, ini_name) = if rust_accounts || rust_auth {
            ("rust-services.log", "rust-services.ini")
        } else {
            ("assets-ferroan.log", "assets-ferroan.ini")
        };
        grid.rust_assets = Some(crate::assets::start_grid_services(
            crate::assets::GridServiceDatabases {
                asset: asset_database.as_deref(),
                account: account_database.as_deref(),
                auth: auth_database.as_deref(),
            },
            port,
            robust_metrics,
            &root.join(log_name),
            &root.join(ini_name),
            docker,
        )?);
        if rust_assets {
            println!("live: rust assets on {uri}");
        }
        if rust_accounts {
            println!("live: rust accounts on {uri}");
        }
        if rust_auth {
            println!("live: rust auth on {uri}");
        }
        (
            rust_assets.then(|| uri.clone()),
            rust_accounts.then(|| uri.clone()),
            rust_auth.then_some(uri),
        )
    } else {
        (None, None, None)
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
            auth: auth_uri.as_deref(),
        },
        docker,
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
        auth_uri.as_deref(),
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
        auth_uri.as_deref(),
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
            // With the Ferrobot relay the relay writes `capture-udp.cap`. Two
            // writers on one path, each at its own offset, overwrote each
            // other's records (the `live images` re-encode failures on
            // AgentUpdate), so the region's own capture goes beside it.
            capture: Some(root.join(if ferroan && scenario == "ferrobot" {
                "capture-udp-region.cap"
            } else {
                "capture-udp.cap"
            })),
            asset_uri: asset_uri.clone(),
            account_uri: account_uri.clone(),
            auth_uri: auth_uri.clone(),
            metrics_port: region_metrics,
            prim_capture: matches!(scenario, "prim-capture" | "prim-arrival" | "object-cross"),
            seed_prims: matches!(scenario, "prim-capture" | "prim-arrival" | "object-cross"),
            basic_physics: spec.basic_physics && !ferroan,
        },
        &grid.passwords,
        docker,
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
            // The return hop's TeleportStart and AgentMovementComplete are
            // UDP from this region. Home's capture file does not see them.
            capture: Some(root.join("capture-udp-neighbour.cap")),
            asset_uri: asset_uri.clone(),
            account_uri: account_uri.clone(),
            auth_uri: auth_uri.clone(),
            metrics_port: None,
            prim_capture: matches!(scenario, "object-cross" | "child-view"),
            seed_prims: scenario == "child-view",
            basic_physics: spec.basic_physics,
        },
        &grid.passwords,
        docker,
    )?);
    let home_log = require_started(&grid.home, "home region")?.log.clone();
    if ferroan {
        let extra = docker.then(|| crate::docker::host_log_dir(crate::docker::NAME_HOME));
        ferroan_bot::set_log_redaction_check(true, extra);
    }
    let report = if let Some(executable) = ferrobot_binary {
        let harness = live::harness(
            &format!("http://{ADVERTISED}:{public_proxy}/"),
            &format!("http://{ADVERTISED}:{private_proxy}"),
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
        // The relay carries Ferrobot's circuit only.
        let companion_udp =
            (ferroan && scenario == "ferrobot").then(|| SocketAddr::from((HOME_BIND, home_port)));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime
            .block_on(crate::ferrobot::run_with(&harness, &config, companion_udp))
            .map(|lines| BotReport {
                child: None,
                lines,
                // Read the complete proxy evidence after grid.stop drains it.
                http: Vec::new(),
            })
    } else if scenario == "limits" {
        if !ferroan {
            return Err("the limits scenario runs on the Ferroan region".to_string());
        }
        let Some(metrics_port) = region_metrics else {
            return Err("the limits scenario needs the region metrics listener".to_string());
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| format!("could not start limits runtime: {error}"))?;
        let line = runtime.block_on(crate::limits_probe::run(
            &format!("http://{ROBUST_BIND}:{robust_public}/"),
            &estate,
            home_port,
            &format!("http://127.0.0.1:{metrics_port}"),
            &home_log,
            &format!("http://{ADVERTISED}:{private_proxy}"),
        ))?;
        Ok(BotReport {
            child: None,
            lines: line,
            http: Vec::new(),
        })
    } else {
        let status = region_metrics.map(|port| {
            (
                format!("http://{HOME_BIND}:{home_port}"),
                format!("http://127.0.0.1:{port}"),
            )
        });
        run_bot(
        scenario,
        &format!("http://{ROBUST_BIND}:{robust_public}/"),
        &estate,
        &chat,
        &home_log,
        ferroan,
        (&format!("http://{ADVERTISED}:{private_proxy}"), &root, ferroan),
        status.as_ref().map(|(region, metrics)| (region.as_str(), metrics.as_str())),
    ).and_then(|mut report| {
        if scenario == "login" {
            let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()
                .map_err(|error| format!("could not start login probe runtime: {error}"))?;
            let private_uri = format!("http://{ADVERTISED}:{private_proxy}");
            // run_bot has already awaited service cleanup after each logout.
            if ferroan {
                if let Some((_, metrics)) = status.as_ref() {
                    let (code, body) = http_get_url(metrics, "/metrics")?;
                    if code != 200 || !body.contains("ferroan_region_root_agents 0\n") {
                        return Err(format!(
                            "metrics after logout: status {code}, root agents were not 0"
                        ));
                    }
                    report.lines.push_str("\nmetrics: root agents 0 after logout");
                }
                let second = run_bot(scenario, &format!("http://{ROBUST_BIND}:{robust_public}/"), &estate, &chat, &home_log, true, (&private_uri, &root, true), None)?;
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
    let scrape = scrape_running(ferroan, region_metrics, robust_metrics);
    let smoke_logs = observe_smoke_logs(docker, ferroan, scenario);
    let redaction = if ferroan {
        ferroan_bot::scan_passwords(&[&estate, &chat], &home_log)
    } else {
        Ok(())
    };
    let failed = report.is_err()
        || relay_result.is_err()
        || scrape.is_err()
        || smoke_logs.is_err()
        || redaction.is_err();
    let diag_note = if failed && ferroan {
        collect_diag(region_metrics, robust_metrics)
    } else {
        String::new()
    };
    let cleanup = grid.stop();
    let report = match (report, relay_result, cleanup, scrape, smoke_logs, redaction) {
        (Ok(mut report), Ok(()), Ok(()), Ok(lines), Ok(smoke), Ok(())) => {
            if !lines.is_empty() {
                report.lines.push('\n');
                report.lines.push_str(&lines);
            }
            if !smoke.is_empty() {
                report.lines.push('\n');
                report.lines.push_str(&smoke);
            }
            report
        }
        (report, relay, cleanup, scrape, smoke, redaction) => {
            let mut parts = [
                report.err(),
                relay.err(),
                cleanup.err(),
                scrape.err(),
                smoke.err(),
                redaction.err(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            if !diag_note.is_empty() {
                parts.push(diag_note);
            }
            return Err(parts.join("\n"));
        }
    };
    let proxy_report = proxy_report(&captures, &grid.passwords)?;
    if scenario == "object-cross" {
        let capture = login_capture::object_cross::extract(&root)?;
        Ok(format!("{}\n{capture}", report.lines))
    } else if let Some(observation) = report.child {
        let line = child_capture::finish(&root, &observation)?;
        Ok(format!("{}\n{line}", report.lines))
    } else if scenario == "prim-capture" {
        let fixture = prim_capture::extract(&root)?;
        Ok(format!("{}\n{fixture}", report.lines))
    } else if scenario == "prim-arrival" {
        let fixture = prim_arrival::extract(&root, &home_log).map_err(|e| e.to_string())?;
        Ok(format!("{}\n{fixture}", report.lines))
    } else if scenario == "report" {
        // The capture still has to pass check, redact and replay. Its packet
        // counts are not report lines: the two regions do not send the same
        // number of datagrams, and the compare is the lines below.
        live::finish_capture(
            &root.join("capture-udp.cap"),
            &report.http,
            &[&estate, &chat],
        )?;
        Ok(report.lines)
    } else if scenario == "ferrobot" {
        let http = ferrobot_capture::http_records(&[
            captures.robust_public.clone(),
            captures.home.clone(),
        ])?;
        let capture =
            live::finish_capture(&root.join("capture-udp.cap"), &http, &[&estate, &chat])?;
        Ok(format!("{}\n{capture}\n{proxy_report}", report.lines))
    } else if ferroan && scenario == "limits" {
        Ok(format!("{}\n{proxy_report}", report.lines))
    } else if ferroan && (scenario == "roots" || scenario == "uploads") {
        let comparison = login_capture::compare_two_roots(&root)?;
        Ok(format!("{}\n{comparison}\n{proxy_report}", report.lines))
    } else if ferroan && scenario == "avatars" {
        let roots = login_capture::compare_two_roots(&root)?;
        let comparison = avatars_capture::compare_avatars(&root)?;
        let capture = live::finish_capture(
            &root.join("capture-udp.cap"),
            &report.http,
            &[&estate, &chat],
        )?;
        Ok(format!(
            "{}\n{roots}\n{comparison}\n{capture}\n{proxy_report}",
            report.lines
        ))
    } else if ferroan && scenario == "chat" {
        let comparison = login_capture::compare_local_chat(&root)?;
        let capture = live::finish_capture(
            &root.join("capture-udp.cap"),
            &report.http,
            &[&estate, &chat],
        )?;
        Ok(format!(
            "{}\n{comparison}\n{capture}\n{proxy_report}",
            report.lines
        ))
    } else if ferroan && scenario == "hop" {
        Ok(format!("{}\n{proxy_report}", report.lines))
    } else if ferroan && scenario == "teleport" {
        let comparison = login_capture::compare_local_teleport(&root)?;
        let capture = live::finish_capture(
            &root.join("capture-udp.cap"),
            &report.http,
            &[&estate, &chat],
        )?;
        Ok(format!(
            "{}\n{comparison}\n{capture}\n{proxy_report}",
            report.lines
        ))
    } else if ferroan && scenario == "neighbour" {
        let comparison =
            login_capture::compare_neighbour_teleport(&root, login_capture::RegionServer::Ferroan)?;
        let capture = live::finish_capture(
            &root.join("capture-udp.cap"),
            &report.http,
            &[&estate, &chat],
        )?;
        Ok(format!(
            "{}\n{comparison}\n{capture}\n{proxy_report}",
            report.lines
        ))
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
    } else if scenario == "neighbour" {
        let comparison =
            login_capture::compare_neighbour_teleport(&root, login_capture::RegionServer::CSharp)?;
        let capture = live::finish_capture(
            &root.join("capture-udp.cap"),
            &report.http,
            &[&estate, &chat],
        )?;
        Ok(format!(
            "{}\n{comparison}\n{capture}\n{proxy_report}",
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
fn collect_diag(region: Option<u16>, robust: Option<u16>) -> String {
    let dir = live::diag_dir();
    if let Err(error) = fs::create_dir_all(&dir) {
        return format!("diag dir: {error}");
    }
    let mut notes = Vec::new();
    for (name, port) in [("region", region), ("robust", robust)] {
        let Some(port) = port else {
            continue;
        };
        let base = format!("http://127.0.0.1:{port}");
        match http_get_url(&base, "/diag") {
            Ok((200, body)) => {
                let path = dir.join(format!("{name}.txt"));
                match fs::write(&path, body) {
                    Ok(()) => notes.push(format!("{name} diag {}", path.display())),
                    Err(error) => notes.push(format!("{name} diag write: {error}")),
                }
            }
            Ok((code, _)) => notes.push(format!("{name} diag status {code}")),
            Err(error) => notes.push(format!("{name} diag failed: {error}")),
        }
    }
    if notes.is_empty() {
        "diag: no metrics listener".to_owned()
    } else {
        notes.join("; ")
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
    child: Option<ferroan_bot::scenario::child_view::Observation>,
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
fn http_get_url(base: &str, path: &str) -> Result<(u16, String), String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let host = base
        .strip_prefix("http://")
        .ok_or_else(|| format!("status URL {base} is not http"))?
        .trim_end_matches('/');
    let address: SocketAddr = host
        .parse()
        .map_err(|_| format!("status URL {base} is not host:port"))?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))
        .map_err(|error| format!("could not connect to {address}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("could not time out {address}: {error}"))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("could not write to {address}: {error}"))?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read from {address}: {error}"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(format!("response from {address} exceeds 1 MiB"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let (head, rest) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("short response from {address}"))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| format!("no status from {address}"))?;
    let status = status
        .parse::<u16>()
        .map_err(|_| format!("bad status from {address}"))?;
    let body = if let Some(length) = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())?
    }) {
        rest.get(..length)
            .ok_or_else(|| format!("short body from {address}"))?
            .to_owned()
    } else {
        rest.to_owned()
    };
    Ok((status, body))
}

#[cfg(target_os = "linux")]
async fn status_while_online(region: &str, metrics: &str) -> Result<String, String> {
    let (code, body) = http_get_url(region, "/simstatus")?;
    if code != 200 || body != "OK" {
        return Err(format!("simstatus during login: status {code}"));
    }
    let (code, body) = http_get_url(metrics, "/healthz")?;
    if code != 200 || body != "ok\n" {
        return Err(format!("healthz during login: status {code}"));
    }
    let (code, body) = http_get_url(metrics, "/metrics")?;
    if code != 200 || !body.contains("ferroan_region_root_agents 1\n") {
        return Err("metrics during login: root agents were not 1".to_owned());
    }
    Ok("status: /simstatus OK, /healthz ok, root agents 1".to_owned())
}

#[cfg(target_os = "linux")]
fn scrape_running(
    ferroan: bool,
    region: Option<u16>,
    robust: Option<u16>,
) -> Result<String, String> {
    let mut lines = Vec::new();
    if ferroan && let Some(port) = region {
        lines.push(scrape_one(
            "ferroan-region",
            port,
            "ferroan_region_registered",
        )?);
    }
    if let Some(port) = robust {
        lines.push(scrape_one(
            "ferroan-robust",
            port,
            "ferroan_robust_db_pool_connections",
        )?);
    }
    Ok(lines.join("\n"))
}

#[cfg(target_os = "linux")]
fn observe_smoke_logs(docker: bool, ferroan: bool, scenario: &str) -> Result<String, String> {
    if !docker || !ferroan || scenario != "login" || crate::docker::smoke_otlp().is_none() {
        return Ok(String::new());
    }
    let logs = crate::docker::container_logs(crate::docker::NAME_HOME)?;
    require_login_line(&logs, "docker logs")?;
    let compose = crate::docker::compose_region_logs()?;
    require_login_line(&compose, "docker compose logs")?;
    let loki = crate::docker::wait_loki_login()?;
    Ok(format!(
        "smoke logs: docker logs, compose logs, and loki ({loki})"
    ))
}

#[cfg(target_os = "linux")]
fn require_login_line(text: &str, source: &str) -> Result<(), String> {
    let login = text.contains("login accepted");
    let first = text.contains("first_name=Test");
    let last = text.contains("last_name=User");
    if login && first && last {
        return Ok(());
    }
    Err(format!(
        "{source} missing login evidence (login accepted={login}, first_name=Test={first}, last_name=User={last}, {} bytes)",
        text.len()
    ))
}

#[cfg(target_os = "linux")]
fn scrape_one(name: &str, port: u16, marker: &str) -> Result<String, String> {
    let (code, body) = http_get_url(&format!("http://127.0.0.1:{port}"), "/metrics")?;
    if code != 200 || !body.contains(marker) {
        return Err(format!("metrics scrape: {name} failed"));
    }
    println!("metrics scrape: {name} pass");
    Ok(format!("metrics scrape: {name} pass"))
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn run_bot(
    scenario: &str,
    login_uri: &str,
    estate: &str,
    chat: &str,
    region_log: &Path,
    strict_logout: bool,
    online: (&str, &Path, bool),
    status: Option<(&str, &str)>,
) -> Result<BotReport, String> {
    let neighbour_id = uuid::Uuid::parse_str(NEIGHBOUR_UUID)
        .map_err(|_| "the neighbour uuid is not a uuid".to_string())?;
    let mut harness = live::harness(login_uri, online.0, estate, chat, region_log)?;
    harness.require_logout_reply = strict_logout;
    if scenario == "roots" {
        harness.metrics = status.map(|(_, metrics)| metrics.to_string());
    }
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
    if scenario == "child-view" {
        let evidence = runtime.block_on(async {
            let observed =
                ferroan_bot::scenario::child_view::run(&harness, HOME_NAME, NEIGHBOUR_NAME).await?;
            login_probe::offline(private_uri).await?;
            login_probe::offline_user(private_uri, live::CHAT_USER).await?;
            Ok::<_, String>(observed)
        })?;
        return Ok(BotReport {
            lines: format!(
                "child view: home root walked and logged out; neighbour avatar {}, stored prim {}",
                evidence.avatar.is_some(),
                evidence.prim.is_some()
            ),
            http: Vec::new(),
            child: Some(evidence),
        });
    }
    let harness_ref = &harness;
    let lines = runtime.block_on(login_probe::run_scenarios(
        scenario,
        private_uri,
        |selected| async move {
            if scenario == "login" {
                ferroan_bot::scenario::run_login_observed(harness_ref, |arrival| async move {
                    let mut line =
                        login_probe::online(private_uri, directory, ferroan, arrival).await?;
                    if let Some((region, metrics)) = status {
                        line.push('\n');
                        line.push_str(&status_while_online(region, metrics).await?);
                    }
                    Ok(line)
                })
                .await
            } else {
                ferroan_bot::scenario::run(selected, harness_ref).await
            }
        },
    ))?;
    Ok(BotReport {
        child: None,
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
    docker: bool,
) -> Result<Proc, String> {
    start_robust_at(
        root,
        public_port,
        private_port,
        passwords,
        ROBUST_BIND,
        None,
        services,
        docker,
    )
}

#[cfg(target_os = "linux")]
// Host and container launches share this list; splitting it would duplicate the ini.
#[allow(clippy::too_many_arguments)]
fn start_robust_at(
    root: &Path,
    public_port: u16,
    private_port: u16,
    passwords: &[String],
    bind: Ipv4Addr,
    console: Option<&fs::File>,
    services: ServiceUris<'_>,
    docker: bool,
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
    let mut proc = if docker {
        // grid-start.sh cds to /app and passes --inifile. The harness ini is
        // the master file, so smoke runs the image's dotnet binary with the
        // same --inimaster arguments as the host GridServer.
        let workdir = binary
            .parent()
            .ok_or("the grid publish directory is missing")?;
        let args = vec![
            "/app/OpenSim.Server.GridServer.dll".to_string(),
            "--inimaster".to_string(),
            ini_arg.to_string(),
            "--inidirectory".to_string(),
            empty_arg.to_string(),
            "--console".to_string(),
            "basic".to_string(),
        ];
        let mounts = docker_mounts(crate::docker::NAME_ROBUST)?;
        let secrets: Vec<&str> = passwords.iter().map(String::as_str).collect();
        let attached = crate::docker::launch(crate::docker::LaunchRequest {
            name: crate::docker::NAME_ROBUST,
            image: crate::docker::CSHARP_GRID_IMAGE,
            entrypoint: Some("dotnet"),
            args: &args,
            workdir,
            log: &log,
            pipe_stdin: true,
            detach: false,
            mounts: &mounts,
            track: true,
            redact: &secrets,
        })?;
        proc_from_attached(attached, &log)?
    } else {
        spawn_proc_with_input(
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
        )?
    };
    wait_log(
        &mut proc.child,
        &log,
        PROMPT,
        live::START_TIMEOUT,
        passwords,
    )?;
    live::require_listen(proc.listen_pid, bind, public_port)?;
    live::require_listen(proc.listen_pid, bind, private_port)?;
    Ok(proc)
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
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
    auth_uri: Option<&str>,
    private_port: u16,
    user: AccountSeed<'_>,
    passwords: &[String],
) -> Result<(), String> {
    if let Some(uri) = account_uri {
        provision_remote_account(uri, auth_uri, private_port, user)?;
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
    )?;
    // The in-process account service's auth connector does not store the
    // password. The harness posts it to the Rust service after create user.
    if let Some(uri) = auth_uri {
        store_password(uri, user.uuid, user.password, user.first, user.last)?;
    }
    Ok(())
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
/// password is `POST /auth/plain`, and the inventory skeleton is
/// `POST /xinventory`. The password stays on C# Robust unless `--rust auth`
/// points it at the Rust service. None of these requests is written to a log.
#[cfg(target_os = "linux")]
fn provision_remote_account(
    account_uri: &str,
    auth_uri: Option<&str>,
    private_port: u16,
    user: AccountSeed<'_>,
) -> Result<(), String> {
    let AccountSeed {
        first,
        last,
        password,
        email,
        uuid,
    } = user;
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
    let auth_address = match auth_uri {
        Some(uri) => SocketAddr::from((Ipv4Addr::LOCALHOST, rust_service_port(uri)?)),
        None => SocketAddr::from((ROBUST_BIND, private_port)),
    };
    let auth = post_form(
        auth_address,
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
fn store_password(
    uri: &str,
    uuid: &str,
    password: &str,
    first: &str,
    last: &str,
) -> Result<(), String> {
    let auth = post_form(
        SocketAddr::from((Ipv4Addr::LOCALHOST, rust_service_port(uri)?)),
        "/auth/plain",
        &format!("METHOD=setpassword&PRINCIPAL={uuid}&PASSWORD={password}"),
    )?;
    if auth.contains(password) {
        return Err("the authentication response contains a password".to_string());
    }
    if !auth.contains("Success") {
        return Err(format!("could not store the password for {first} {last}"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn rust_service_port(uri: &str) -> Result<u16, String> {
    let rest = uri
        .strip_prefix("http://127.0.0.1:")
        .ok_or("the service URL is not on 127.0.0.1")?;
    rest.parse::<u16>()
        .map_err(|_| "the service URL has no port".to_string())
}

#[cfg(target_os = "linux")]
fn post_form(address: SocketAddr, path: &str, body: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|error| format!("could not connect to {address}: {error}"))?;
    // OutboundUrlFilter sets NoDelay before connect. This socket exists only
    // after connect; the request write below sees the option.
    stream
        .set_nodelay(true)
        .map_err(|error| format!("could not set TCP_NODELAY: {error}"))?;
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
    seed_prims: bool,
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
    auth_uri: Option<String>,
    metrics_port: Option<u16>,
    basic_physics: bool,
    prim_capture: bool,
}

#[cfg(target_os = "linux")]
fn start_region(
    cwd: &Path,
    publish: &Path,
    source: &Path,
    password: &str,
    spec: RegionStart,
    passwords: &[String],
    docker: bool,
) -> Result<Proc, String> {
    let binary = if spec.ferroan && !docker {
        live::build_binary("ferroan-region")?
    } else if spec.ferroan {
        PathBuf::new()
    } else {
        publish.join("OpenSim.Server.RegionServer")
    };
    if (!docker || !spec.ferroan) && !binary.is_file() {
        return Err("run cargo xtask live build first".to_string());
    }
    prepare_region_cwd(cwd, publish, source)?;
    if spec.seed_prims {
        fs::copy(
            crate::repo_root().join("fixtures/prim-updates/scene.db"),
            cwd.join("OpenSim.db"),
        )
        .map_err(|error| format!("install prim capture store: {error}"))?;
        if spec.uuid == NEIGHBOUR_UUID {
            // Put the existing Q65a linkset within sight of the west border.
            let sql = format!(
                "UPDATE prims SET RegionUUID='{NEIGHBOUR_UUID}', GroupPositionX=GroupPositionX-110"
            );
            let status = std::process::Command::new("sqlite3")
                .arg(cwd.join("OpenSim.db"))
                .arg(sql)
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("could not place the capture prims on the neighbour".into());
            }
        }
    } else if spec.ferroan {
        install_seeded_region_store(cwd)?;
    }
    let mut rewrites = Vec::new();
    if let Some(uri) = &spec.asset_uri {
        rewrites.push(("AssetServerURI", uri.as_str()));
    }
    if let Some(uri) = &spec.account_uri {
        rewrites.push(("UserAccountServerURI", uri.as_str()));
    }
    if let Some(uri) = &spec.auth_uri {
        rewrites.push(("AuthenticationServerURI", uri.as_str()));
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
        prim_capture: spec.prim_capture,
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
        basic_physics: spec.basic_physics,
    };
    let mut ini = region_server_ini(&region)?;
    if spec.login_only {
        // Scene.RegisterRegionWithGrid skips map generation in this supported
        // configuration. Q6 checks login, before Ferroan has terrain/maps.
        ini.push_str("\n[Map]\n    GenerateMaptiles = false\n    MaptileStaticUUID = 00000000-0000-0000-0000-000000000000\n");
    }
    if let Some(port) = spec.metrics_port {
        ini.push_str(&format!(
            "\n[Monitoring]\n    Metrics = true\n    Listen = \"127.0.0.1:{port}\"\n"
        ));
        // Smoke opts in. A normal live run does not export and stays plain text.
        if docker
            && spec.ferroan
            && let Some(endpoint) = crate::docker::smoke_otlp()
        {
            ini.push_str(&format!(
                "    LogFormat = json\n    OtlpEndpoint = \"{endpoint}\"\n"
            ));
        }
    }
    live::write_secret(&config.join("RegionServer.ini"), &ini)?;
    fs::write(regions.join("Regions.ini"), regions_ini(&region))
        .map_err(|error| format!("could not write Regions.ini: {error}"))?;
    let log = cwd.join("region.log");
    let config_arg = config.to_str().ok_or("region config path is not utf-8")?;
    let mut proc = if docker {
        // region-start.sh cds to /app. The prepared working directory holds
        // the harness ini and the config-include links, so the image process
        // uses that directory and the same arguments as the host region.
        let name = if spec.ferroan {
            crate::docker::NAME_HOME
        } else {
            crate::docker::NAME_NEIGHBOUR
        };
        let image = if spec.ferroan {
            crate::docker::REGION_IMAGE
        } else {
            crate::docker::CSHARP_REGION_IMAGE
        };
        let mut args = Vec::new();
        if !spec.ferroan {
            args.push("/app/OpenSim.Server.RegionServer.dll".to_string());
        }
        args.push("--inidirectory".to_string());
        args.push(config_arg.to_string());
        args.push("--background".to_string());
        let mounts = docker_mounts(name)?;
        let secrets: Vec<&str> = passwords.iter().map(String::as_str).collect();
        let attached = crate::docker::launch(crate::docker::LaunchRequest {
            name,
            image,
            entrypoint: (!spec.ferroan).then_some("dotnet"),
            args: &args,
            workdir: cwd,
            log: &log,
            pipe_stdin: false,
            detach: false,
            mounts: &mounts,
            track: true,
            redact: &secrets,
        })?;
        proc_from_attached(attached, &log)?
    } else {
        spawn_proc(
            &binary,
            &["--inidirectory", config_arg, "--background"],
            cwd,
            &log,
            false,
        )?
    };
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
    if spec.basic_physics && !spec.ferroan {
        live::require_basic_physics(&log, cwd, region.name)?;
    }
    live::require_listen(proc.listen_pid, region.http_bind, region.http_port)?;
    live::require_udp(proc.listen_pid, region.udp_bind, region.http_port)?;
    Ok(proc)
}

#[cfg(target_os = "linux")]
fn docker_mounts(name: &str) -> Result<Vec<crate::docker::Mount>, String> {
    let live_root = live::live_root();
    fs::create_dir_all(&live_root)
        .map_err(|error| format!("could not create {}: {error}", live_root.display()))?;
    let logdata = crate::docker::host_log_dir(name);
    fs::create_dir_all(&logdata)
        .map_err(|error| format!("could not create {}: {error}", logdata.display()))?;
    Ok(vec![
        crate::docker::Mount {
            host: live_root.clone(),
            container: live_root,
            read_only: false,
        },
        crate::docker::Mount {
            host: logdata,
            container: PathBuf::from("/app/logdata"),
            read_only: false,
        },
    ])
}

#[cfg(target_os = "linux")]
fn proc_from_attached(attached: crate::docker::Attached, log: &Path) -> Result<Proc, String> {
    let crate::docker::Attached {
        child,
        stdin,
        name,
        listen_pid,
        tracked,
    } = attached;
    let tracked = tracked.ok_or("docker grid launch was not recorded")?;
    let listen_pid = if listen_pid == 0 {
        child.id()
    } else {
        listen_pid
    };
    Ok(Proc {
        child,
        stdin: stdin.map(|pipe| Box::new(pipe) as Box<dyn std::io::Write>),
        log: log.to_path_buf(),
        tracked,
        detached: false,
        container: Some(name),
        listen_pid,
    })
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

/// Copy the committed region-store fixture into this run's directory. The
/// published tree stays without databases. The name is the SQLite file
/// `URI=file:OpenSim.db` opens from the region working directory.
#[cfg(target_os = "linux")]
fn install_seeded_region_store(cwd: &Path) -> Result<(), String> {
    let source = crate::repo_root().join("fixtures/regionstore/objects.db");
    if !source.is_file() {
        return Err(format!("missing {}", source.display()));
    }
    fs::copy(&source, cwd.join("OpenSim.db")).map_err(|error| {
        format!(
            "could not copy {} to {}: {error}",
            source.display(),
            cwd.display()
        )
    })?;
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
    let listen_pid = child.id();
    Ok(Proc {
        child,
        stdin,
        log: log.to_path_buf(),
        tracked,
        detached: false,
        container: None,
        listen_pid,
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

    /// A stand-in server whose stop sends one request through a proxy, as the
    /// C# neighbour's late CloseAgent does.
    #[cfg(target_os = "linux")]
    struct LateCaller(SocketAddr);

    #[cfg(target_os = "linux")]
    impl Drop for LateCaller {
        fn drop(&mut self) {
            use std::io::{Read, Write};
            let mut client = TcpStream::connect(self.0).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            client
                .write_all(b"DELETE /agent/x/ HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut response = Vec::new();
            let _ = client.read_to_end(&mut response);
            assert!(response.is_empty());
        }
    }

    /// Four proxies on closed upstreams, in `run_linux` order: Robust private,
    /// Robust public, home, neighbour. Literal indexes, so a swapped constant
    /// shows.
    #[cfg(target_os = "linux")]
    fn grid_proxies(name: &str) -> (Vec<SocketAddr>, crate::live_proxy::Proxies, Vec<PathBuf>) {
        let binds: Vec<(SocketAddr, SocketAddr, PathBuf)> =
            ["private", "public", "home", "neighbour"]
                .iter()
                .map(|which| {
                    let closed = std::net::TcpListener::bind("127.0.0.1:0")
                        .unwrap()
                        .local_addr()
                        .unwrap();
                    let path = crate::repo_root().join(".scratch").join(format!(
                        "stop-order-{name}-{which}-{}.cap",
                        uuid::Uuid::new_v4()
                    ));
                    ("127.0.0.1:0".parse().unwrap(), closed, path)
                })
                .collect();
        let (addresses, proxies) = crate::live_proxy::Proxies::start(&binds).unwrap();
        (
            addresses,
            proxies,
            binds.into_iter().map(|(_, _, path)| path).collect(),
        )
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn each_server_is_marked_just_before_it_stops() {
        // Each server's stop sends through its own proxy: all are marked.
        let (addresses, proxies, paths) = grid_proxies("own");
        let mut home = Some(LateCaller(addresses[2]));
        let mut neighbour = Some(LateCaller(addresses[3]));
        let mut robust = Some(LateCaller(addresses[0]));
        stop_in_order([&mut home, &mut neighbour, &mut robust], Some(&proxies));
        assert!(home.is_none() && neighbour.is_none() && robust.is_none());
        proxies.finish().unwrap();
        for path in paths {
            let _ = fs::remove_file(path);
        }
        // Home's stop calls the neighbour, which is still meant to be up.
        let (addresses, proxies, paths) = grid_proxies("early");
        let mut home = Some(LateCaller(addresses[3]));
        let mut neighbour: Option<LateCaller> = None;
        let mut robust: Option<LateCaller> = None;
        stop_in_order([&mut home, &mut neighbour, &mut robust], Some(&proxies));
        let error = proxies.finish().unwrap_err();
        assert!(error.contains("recorded 1 failure(s)"), "{error}");
        for path in paths {
            let _ = fs::remove_file(path);
        }
    }

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
        install_seeded_region_store(&root.join("home")).unwrap();
        assert!(root.join("home/OpenSim.db").is_file());
        assert_ne!(
            fs::read(root.join("home/OpenSim.db")).unwrap(),
            b"standalone state"
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
            prim_capture: false,
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
            basic_physics: false,
        }
    }

    #[test]
    fn prim_capture_disables_the_cache_in_the_capture_file_section() {
        let mut region = sample_region(Some("capture-udp.cap"));
        region.prim_capture = true;
        let ini = region_server_ini(&region).unwrap();
        assert_eq!(ini.matches("[ClientStack.LindenUDP]").count(), 1);
        assert!(ini.contains("[ClientStack.LindenUDP]\n    CaptureFile = \"capture-udp.cap\"\n    SupportViewerObjectsCache = false"));
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
        assert!(!ini.contains("physics ="));
        assert!(!ini.contains("ServiceProbe"));
        let mut basic = sample_region(None);
        basic.basic_physics = true;
        let basic_ini = region_server_ini(&basic).unwrap();
        assert!(basic_ini.contains("physics = basicphysics"));
        assert!(
            basic_ini.find("[Startup]").unwrap()
                < basic_ini.find("physics = basicphysics").unwrap()
        );
        assert!(
            basic_ini.find("physics = basicphysics").unwrap()
                < basic_ini.find("ConsoleHistoryFileEnabled").unwrap()
        );
        assert!(ini.contains("DefaultEstateOwnerPassword = \"generated-password\""));
        assert!(!ini.contains("127.0.0.4"));
        let regions = regions_ini(&sample_region(None));
        assert!(regions.contains("Location = 1000,1000"));
        assert!(regions.contains("InternalAddress = 127.0.0.1"));
        assert!(regions.contains("ExternalHostName = 127.0.0.1"));
        assert!(regions.contains("InternalPort = 9000"));
    }

    #[test]
    fn generated_listeners_stay_inside_the_port_base() {
        let base = 25_000u16;
        let home = base + crate::ports::Slot::Home as u16;
        let public = base + crate::ports::Slot::PublicProxy as u16;
        let private = base + crate::ports::Slot::PrivateProxy as u16;
        let robust_public = base + crate::ports::Slot::RobustPublic as u16;
        let robust_private = base + crate::ports::Slot::RobustPrivate as u16;
        let region = RegionIni {
            http_port: home,
            public_proxy: public,
            private_proxy: private,
            ..sample_region(None)
        };
        let ini = region_server_ini(&region).unwrap();
        let regions = regions_ini(&region);
        let robust = robust_ini(
            ROBUST_BIND,
            robust_public,
            robust_private,
            "/tmp/ferroan-robust.db",
            "/tmp/Libraries.xml",
            "/tmp/AssetSets.xml",
            ServiceUris {
                asset: None,
                account: None,
                auth: None,
            },
        )
        .unwrap();
        let rendered = format!("{ini}\n{regions}\n{robust}");
        let allowed = [home, public, private, robust_public, robust_private];
        assert!(
            crate::ports::ports_outside(&rendered, &allowed).is_empty(),
            "{rendered}"
        );
        let hardcoded = format!("{rendered}\nhttp_listener_port = 9000\n");
        assert_eq!(
            crate::ports::ports_outside(&hardcoded, &allowed),
            vec![9000]
        );
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
                auth: None,
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
                auth: None,
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
                    auth: None,
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
                auth: None,
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
                auth: None,
            },
        )
        .unwrap();
        assert!(both.contains("AssetServerURI = \"http://127.0.0.1:9\""));
        assert!(both.contains("UserAccountServerURI = \"http://127.0.0.1:9\""));
        assert!(!both.contains("AssetServiceConnector"));
        assert!(!both.contains("UserAccountServiceConnector"));
    }

    #[test]
    fn a_rust_auth_uri_omits_the_local_handler_and_shares_the_auth_file() {
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
                auth: Some("http://127.0.0.1:9"),
            },
        )
        .unwrap();
        assert!(ini.contains("AuthenticationServerURI = \"http://127.0.0.1:9\""));
        assert!(ini.contains(
            "AuthenticationService = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\""
        ));
        assert!(ini.contains(
            "AuthenticationService = \"OpenSim.Services.Connectors.dll:AuthenticationServicesConnector\""
        ));
        assert!(ini.contains("Data Source=/tmp/auth.db;Version=3;BusyTimeout=15000;"));
        assert!(!ini.contains("AuthenticationServiceConnector"));
        assert!(!ini.contains("LocalServiceModule = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\""));
        assert!(!ini.contains("AllowSetPassword"));
        assert!(!ini.contains("ferroan-robust.db.auth"));
        assert!(ini.contains("UserAccountServiceConnector"));
        let common = "[AuthenticationService]\n    AuthenticationServerURI = \"${Const|PrivURL}:${Const|PrivatePort}\"\n";
        let pointed =
            rewrite_named_server_uri(common, "AuthenticationServerURI", "http://127.0.0.1:9")
                .unwrap();
        assert!(pointed.contains("AuthenticationServerURI = \"http://127.0.0.1:9\""));
        assert!(!pointed.contains("${Const|PrivURL}"));

        let both = robust_ini(
            ROBUST_BIND,
            8002,
            8003,
            "/tmp/ferroan-robust.db",
            "/tmp/Libraries.xml",
            "/tmp/AssetSets.xml",
            ServiceUris {
                asset: None,
                account: Some("http://127.0.0.1:9"),
                auth: Some("http://127.0.0.1:9"),
            },
        )
        .unwrap();
        assert!(both.contains("UserAccountServerURI = \"http://127.0.0.1:9\""));
        assert!(both.contains("AuthenticationServerURI = \"http://127.0.0.1:9\""));
        assert!(both.contains(
            "AuthenticationService = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\""
        ));
        assert!(both.contains("Data Source=/tmp/auth.db;Version=3;BusyTimeout=15000;"));
        assert!(!both.contains("AllowSetPassword"));
        assert!(!both.contains("AuthenticationServicesConnector"));
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
                    auth: None,
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
