// Copyright (c) 2026 Ferroan contributors
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
//! GridCommon.ini.example at Tranquillity `8583218`. Nothing here edits
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
InternalAddress = {advertised}
InternalPort = {port}
AllowAlternatePorts = False
ExternalHostName = {advertised}
"#,
        name = region.name,
        uuid = region.uuid,
        x = region.x,
        y = region.y,
        advertised = ADVERTISED,
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
    live::source_dir()
        .parent()
        .map(|live_root| live_root.join("grid"))
        .unwrap_or_else(|| live::source_dir().join("grid"))
}

#[cfg(target_os = "linux")]
struct Proc {
    child: Child,
    stdin: Option<ChildStdin>,
    log: PathBuf,
}

#[cfg(target_os = "linux")]
struct Grid {
    robust: Option<Proc>,
    home: Option<Proc>,
    neighbour: Option<Proc>,
    proxies: Option<tokio::runtime::Runtime>,
    passwords: Vec<String>,
}

#[cfg(target_os = "linux")]
impl Grid {
    fn stop(&mut self) {
        for slot in [&mut self.home, &mut self.neighbour, &mut self.robust] {
            if let Some(mut proc) = slot.take() {
                drop(proc.stdin.take());
                stop_child(&mut proc.child);
            }
        }
        drop(self.proxies.take());
    }
}

fn stop_child(child: &mut Child) {
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
        self.stop();
    }
}

#[cfg(target_os = "linux")]
fn run_linux(scenario: &str, ferroan: bool) -> Result<String, String> {
    if ferroan && scenario != "login" {
        return Err(
            "the Ferroan grid region currently supports the login scenario only".to_string(),
        );
    }
    if scenario != "neighbour" && ferroan_bot::scenario::selection(scenario).is_none() {
        return Err(format!(
            "unknown scenario {scenario}; expected all, neighbour or {}",
            ferroan_bot::scenario::NAMES.join(", ")
        ));
    }
    if let Some(pid) = live::standalone_is_up()? {
        return Err(format!(
            "the standalone region is already up (pid {pid}); cargo xtask live down stops it"
        ));
    }
    live::ensure_region_built()?;
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
    ensure_grid_common(&publish)?;
    let source = live::source_dir();
    println!("live: grid region {HOME_NAME} on {HOME_BIND}:{home_port}");
    grid.home = Some(start_region(
        &root.join("home"),
        &publish,
        &source,
        &estate,
        RegionStart {
            ferroan,
            login_only: scenario == "login",
            name: HOME_NAME,
            uuid: live::REGION_UUID,
            x: HOME_X,
            y: HOME_Y,
            http_bind: HOME_BIND,
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
            login_only: scenario == "login",
            name: NEIGHBOUR_NAME,
            uuid: NEIGHBOUR_UUID,
            x: NEIGHBOUR_X,
            y: NEIGHBOUR_Y,
            http_bind: NEIGHBOUR_BIND,
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
    let report = run_bot(
        scenario,
        &format!("http://{ROBUST_BIND}:{robust_public}/"),
        &estate,
        &chat,
        &home_log,
        ferroan,
    );
    grid.stop();
    let report = report?;
    let proxy_report = proxy_report(&captures, &grid.passwords)?;
    if ferroan {
        let comparison = login_capture::compare(&root)?;
        Ok(format!(
            "{}\nlogout: LogoutReply\n{comparison}\n{proxy_report}",
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
fn run_bot(
    scenario: &str,
    login_uri: &str,
    estate: &str,
    chat: &str,
    region_log: &Path,
    strict_logout: bool,
) -> Result<BotReport, String> {
    let region_id = uuid::Uuid::parse_str(live::REGION_UUID)
        .map_err(|_| "the region uuid is not a uuid".to_string())?;
    let neighbour_id = uuid::Uuid::parse_str(NEIGHBOUR_UUID)
        .map_err(|_| "the neighbour uuid is not a uuid".to_string())?;
    let library = uuid::Uuid::parse_str("2a4880b6-b7a3-690a-2049-bfbe38eafb9f")
        .map_err(|_| "the library texture uuid is not a uuid".to_string())?;
    let http_trace = ferroan_bot::trace::HttpTrace::new();
    let harness = ferroan_bot::scenario::Harness {
        login_uri: login_uri.to_string(),
        region_id,
        owner: ferroan_bot::scenario::Account {
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            password: estate.to_string(),
        },
        chat: ferroan_bot::scenario::Account {
            first_name: "Chat".to_string(),
            last_name: "Bot".to_string(),
            password: chat.to_string(),
        },
        texture: live::texture_bytes()?,
        library_texture: library,
        http_trace: std::sync::Arc::clone(&http_trace),
        region_log: region_log.to_path_buf(),
        require_logout_reply: strict_logout,
        neighbour: Some(ferroan_bot::scenario::Neighbour {
            region_id: neighbour_id,
            x: NEIGHBOUR_X,
            y: NEIGHBOUR_Y,
            home_x: HOME_X,
            home_y: HOME_Y,
        }),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start tokio: {error}"))?;
    let lines = runtime.block_on(ferroan_bot::scenario::run(scenario, &harness))?;
    Ok(BotReport {
        lines,
        http: http_trace.records(),
    })
}

#[cfg(target_os = "linux")]
fn start_proxies(
    binds: &[(SocketAddr, SocketAddr, PathBuf)],
) -> Result<tokio::runtime::Runtime, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start tokio: {error}"))?;
    runtime.block_on(async {
        for (listen, upstream, path) in binds {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
            }
            let listener = tokio::net::TcpListener::bind(*listen)
                .await
                .map_err(|error| format!("could not bind the proxy on {listen}: {error}"))?;
            let upstream = *upstream;
            let path = path.clone();
            tokio::spawn(async move {
                let _ = ferroan_capture::serve_on(listener, upstream, &path).await;
            });
        }
        Ok::<(), String>(())
    })?;
    Ok(runtime)
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
        let output = Command::new("cargo")
            .args(["build", "-p", "ferroan-region", "--locked"])
            .current_dir(crate::repo_root())
            .output()
            .map_err(|error| format!("could not build ferroan-region: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "could not build ferroan-region\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        crate::repo_root().join("target/debug/ferroan-region")
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
    live::require_udp(proc.child.id(), ADVERTISED, region.http_port)?;
    Ok(proc)
}

#[cfg(target_os = "linux")]
fn prepare_region_cwd(cwd: &Path, publish: &Path, source: &Path) -> Result<(), String> {
    fs::create_dir_all(cwd)
        .map_err(|error| format!("could not create {}: {error}", cwd.display()))?;
    // Plugin loading uses Assembly.LoadFrom with a path relative to the
    // working directory, and configDir() is that same directory. Link the
    // published tree. Regions and Estates stay real directories: both regions
    // would otherwise write one Regions.ini and one estate store.
    let entries = fs::read_dir(publish)
        .map_err(|error| format!("could not read {}: {error}", publish.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not read {}: {error}", publish.display()))?;
        let name = entry.file_name();
        if name == "Regions" || name == "Estates" {
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
fn ensure_grid_common(publish: &Path) -> Result<(), String> {
    let dest = publish.join("config-include/GridCommon.ini");
    if dest.is_file() {
        return Ok(());
    }
    let source = dest.with_file_name("GridCommon.ini.example");
    fs::copy(&source, &dest).map(|_| ()).map_err(|error| {
        format!(
            "could not copy {} to {}: {error}",
            source.display(),
            dest.display()
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
    let mut child = command
        .process_group(0)
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", binary.display()))?;
    let stdin = child.stdin.take();
    Ok(Proc {
        child,
        stdin,
        log: log.to_path_buf(),
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
    redact(&live::log_tail(log, ""), passwords)
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

fn redact(text: &str, passwords: &[String]) -> String {
    let mut out = text.to_string();
    for password in passwords {
        if !password.is_empty() {
            out = out.replace(password, "<redacted>");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_region<'a>(capture: Option<&'a str>) -> RegionIni<'a> {
        RegionIni {
            name: HOME_NAME,
            uuid: live::REGION_UUID,
            x: HOME_X,
            y: HOME_Y,
            http_bind: HOME_BIND,
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
