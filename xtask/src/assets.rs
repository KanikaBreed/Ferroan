// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Asset A/B. The region uses the C# asset service, then `ferroan-robust`,
//! through `ferroan-capture http-proxy`. Texture and inventory must pass on
//! both, and the normalized asset HTTP must match.

#[cfg(any(target_os = "linux", test))]
use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
use std::process::ExitCode;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(any(target_os = "linux", test))]
use ferroan_capture::{Kind, read_complete_prefix};

#[cfg(target_os = "linux")]
use crate::live;
#[cfg(target_os = "linux")]
use crate::repo_root;

#[cfg(target_os = "linux")]
pub(crate) const KELP: &str = "2a4880b6-b7a3-690a-2049-bfbe38eafb9f";

pub fn compare() -> ExitCode {
    match compare_inner() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("live: {error}");
            ExitCode::from(1)
        }
    }
}

fn compare_inner() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        compare_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("cargo xtask live assets runs on Linux".to_string())
    }
}

#[cfg(target_os = "linux")]
fn compare_linux() -> Result<String, String> {
    if let Some(pid) = live::standalone_is_up()? {
        return Err(format!(
            "the harness is already up (pid {pid}); run cargo xtask live down first"
        ));
    }
    let _state = crate::grid_state::claim()?;
    live::ensure_region_built()?;
    let seed = seed_database()?;
    let csharp = run_side("csharp", &seed)?;
    let ferroan = run_side("ferroan", &seed)?;
    let left = canonical_http(
        &fs::read(&csharp.capture)
            .map_err(|error| format!("could not read {}: {error}", csharp.capture.display()))?,
    )?;
    let right = canonical_http(
        &fs::read(&ferroan.capture)
            .map_err(|error| format!("could not read {}: {error}", ferroan.capture.display()))?,
    )?;
    if left != right {
        let diff = first_difference(&left, &right);
        return Err(format!(
            "asset HTTP differs\ncsharp: {}\nferroan: {}\n{diff}",
            csharp.capture.display(),
            ferroan.capture.display()
        ));
    }
    Ok(format!(
        "{}\n{}\nassets: match ({} http records)",
        csharp.report,
        ferroan.report,
        left.lines().count()
    ))
}

#[cfg(target_os = "linux")]
struct SideReport {
    report: String,
    capture: PathBuf,
}

#[cfg(target_os = "linux")]
fn run_side(name: &str, seed: &Path) -> Result<SideReport, String> {
    let run = live::scratch_run();
    fs::create_dir_all(&run)
        .map_err(|error| format!("could not create {}: {error}", run.display()))?;
    let database = run.join(format!("assets-{name}.db"));
    fs::copy(seed, &database)
        .map_err(|error| format!("could not copy the asset database: {error}"))?;
    let port = live::allocate_port()?;
    let log = run.join(format!("assets-{name}.log"));
    let mut server = if name == "csharp" {
        start_grid(&database, port, &log, false)?
    } else if name == "ferroan" {
        start_robust(&database, port, &log, false)?
    } else {
        return Err(format!("unknown asset side {name}"));
    };
    wait_service(&mut server, name, port, &log)?;
    let capture = run.join(format!("assets-{name}.cap"));
    let upstream = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let (proxy_port, proxy) = spawn_proxy(upstream, &capture)?;
    let uri = format!("http://127.0.0.1:{proxy_port}");
    println!("live: assets {name} via {uri}");
    let report = live::run_against_assets(&uri, "assets");
    let proxy_result = proxy.finish();
    stop_owned(&mut server);
    proxy_result?;
    let report = report?;
    Ok(SideReport {
        report: format!("assets {name}:\n{report}"),
        capture,
    })
}

#[cfg(target_os = "linux")]
pub(crate) struct Owned {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    stop_on_drop: bool,
    tracked: Option<crate::grid_state::Tracked>,
    container: Option<String>,
    listen_pid: u32,
}

#[cfg(target_os = "linux")]
impl Owned {
    pub(crate) fn pid(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn detach(mut self) {
        self.stop_on_drop = false;
    }
}

#[cfg(target_os = "linux")]
impl Drop for Owned {
    fn drop(&mut self) {
        if self.stop_on_drop {
            stop_owned(self);
        }
    }
}

/// The caller retains this guard until the region and its run state are ready.
#[cfg(target_os = "linux")]
pub(crate) fn start_persistent() -> Result<(Owned, u16), String> {
    let seed = seed_database()?;
    let run = live::scratch_run();
    let database = run.join("assets-live.db");
    // A prior run may have left WAL files beside the database. Replacing only
    // its main file would mix the new seed with those old transactions.
    for suffix in ["", "-wal", "-shm"] {
        let path = run.join(format!("assets-live.db{suffix}"));
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("could not remove {}: {error}", path.display())),
        }
    }
    fs::copy(seed, &database)
        .map_err(|error| format!("could not copy the asset database: {error}"))?;
    let port = live::allocate_port()?;
    let log = run.join("assets-live.log");
    let mut server = start_robust(&database, port, &log, false)?;
    wait_service(&mut server, "ferroan", port, &log)?;
    Ok((server, port))
}

#[cfg(target_os = "linux")]
pub(crate) fn wait_service(
    server: &mut Owned,
    name: &str,
    port: u16,
    log: &Path,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if server
            .child
            .try_wait()
            .map_err(|error| format!("could not check {name}: {error}"))?
            .is_some()
        {
            return Err(format!("the {name} asset service exited\n{}", tail(log)));
        }
        if live::require_http_loopback(server.listen_pid, port).is_ok() {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err(format!(
                "the {name} asset service did not listen on 127.0.0.1:{port}\n{}",
                tail(log)
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// Only the recorded process from this worktree may be stopped by `live down`.
#[cfg(target_os = "linux")]
pub(crate) fn persistent_pid_live(pid: u32, start_time: u64) -> bool {
    if live::process_start_time(pid) != Some(start_time) {
        return false;
    }
    let Ok(bytes) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let args: Vec<_> = bytes.split(|byte| *byte == 0).collect();
    let ini = live::scratch_run().join("robust.ini");
    args.first().is_some_and(|arg| {
        arg.rsplit(|byte| *byte == b'/').next() == Some(b"ferroan-robust".as_slice())
    }) && args
        .windows(2)
        .any(|pair| pair[0] == b"--inimaster" && pair[1] == ini.as_os_str().as_encoded_bytes())
}

#[cfg(target_os = "linux")]
pub(crate) fn stop_persistent(pid: u32, start_time: u64) -> Result<(), String> {
    if !persistent_pid_live(pid, start_time) {
        return Ok(());
    }
    let _ = crate::signal::signal_group(crate::signal::Signal::Term, pid);
    let deadline = Instant::now() + Duration::from_secs(20);
    while persistent_pid_live(pid, start_time) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    if persistent_pid_live(pid, start_time) {
        let _ = crate::signal::signal_group(crate::signal::Signal::Kill, pid);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while persistent_pid_live(pid, start_time) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    if persistent_pid_live(pid, start_time) {
        Err(format!("asset service pid {pid} did not exit"))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn seed_database() -> Result<PathBuf, String> {
    let run = live::scratch_run();
    fs::create_dir_all(&run)
        .map_err(|error| format!("could not create {}: {error}", run.display()))?;
    let seed = run.join("asset-seed.db");
    if library_texture_stored(&seed)? {
        return Ok(seed);
    }
    let fixture = repo_root().join("fixtures/db/sqlite/AssetStore.db");
    fs::copy(&fixture, &seed)
        .map_err(|error| format!("could not copy {}: {error}", fixture.display()))?;
    let port = live::allocate_port()?;
    let log = run.join("asset-seed.log");
    let mut server = start_robust(&seed, port, &log, true)?;
    let started = Instant::now();
    loop {
        if server
            .child
            .try_wait()
            .map_err(|error| format!("could not check the asset loader: {error}"))?
            .is_some()
        {
            stop_owned(&mut server);
            return Err(format!("the asset loader exited\n{}", tail(&log)));
        }
        if live::require_http_loopback(server.listen_pid, port).is_ok() {
            break;
        }
        if started.elapsed() > Duration::from_secs(180) {
            stop_owned(&mut server);
            return Err(format!(
                "the asset loader did not store the library texture\n{}",
                tail(&log)
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
    stop_owned(&mut server);
    let _ = Command::new("sqlite3")
        .arg(&seed)
        .arg("PRAGMA wal_checkpoint(TRUNCATE);")
        .output();
    if !library_texture_stored(&seed)? {
        return Err("the library texture disappeared after the loader stopped".to_string());
    }
    Ok(seed)
}

#[cfg(target_os = "linux")]
pub(crate) fn library_texture_stored(database: &Path) -> Result<bool, String> {
    if !database.is_file() {
        return Ok(false);
    }
    let output = Command::new("sqlite3")
        .arg(database)
        .arg(format!(
            "SELECT COUNT(*) FROM assets WHERE UUID = '{KELP}';"
        ))
        .output()
        .map_err(|error| format!("could not query the asset database: {error}"))?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "1")
}

#[cfg(target_os = "linux")]
fn start_grid(database: &Path, port: u16, log: &Path, load: bool) -> Result<Owned, String> {
    let binary = live::grid_server_binary();
    if !binary.is_file() {
        return Err("run cargo xtask live build first".to_string());
    }
    let run = live::scratch_run();
    let empty = run.join("grid-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_path = run.join(if load {
        "grid-seed.ini"
    } else {
        "grid-serve.ini"
    });
    let loader = if load {
        let assets = live::asset_set_path();
        if !assets.is_file() {
            return Err(format!("missing {}", assets.display()));
        }
        format!(
            "    DefaultAssetLoader = \"OpenSim.Framework.AssetLoader.Filesystem.dll\"\n    \
             AssetLoaderArgs = \"{}\"\n",
            assets.display()
        )
    } else {
        "    AssetLoaderEnabled = false\n".to_string()
    };
    let db = database.display();
    let ini = format!(
        "[Const]\n    BaseHostname = \"127.0.0.1\"\n    PublicPort = \"{port}\"\n    \
         PrivatePort = \"{port}\"\n\n[Startup]\n    RegistryLocation = \".\"\n    \
         ConsoleHistoryFileEnabled = false\n    \
         EnableRobustSelfsignedCertSupport = false\n\n[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n\
         [ServiceList]\n    \
         AssetServiceConnector = \"{port}/OpenSim.Server.Handlers.dll:AssetServiceConnector\"\n\n\
         [DatabaseService]\n    StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n\n[AssetService]\n    \
         LocalServiceModule = \"OpenSim.Services.AssetService.dll:AssetService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n{loader}"
    );
    live::write_secret(&ini_path, &ini)?;
    spawn_owned(
        &binary,
        &[
            "--inimaster",
            ini_path.to_str().ok_or("grid ini path is not utf-8")?,
            "--inidirectory",
            empty.to_str().ok_or("grid config path is not utf-8")?,
            "--console",
            "basic",
        ],
        binary.parent().unwrap_or(Path::new(".")),
        log,
        true,
        true,
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn seed_library_database() -> Result<PathBuf, String> {
    seed_database()
}

/// Databases one `ferroan-robust` may open for the grid `--rust` services.
#[cfg(target_os = "linux")]
pub(crate) struct GridServiceDatabases<'a> {
    pub(crate) asset: Option<&'a Path>,
    pub(crate) account: Option<&'a Path>,
    pub(crate) auth: Option<&'a Path>,
}

/// One `ferroan-robust` for the grid `--rust` services named here.
///
/// An asset database is the seeded library store. An account database is a
/// copy of the UserAccount fixture, not Robust's file. An auth database is a
/// copy of the AuthStore fixture. Asset-only keeps the ini the harness used
/// before accounts were wired.
#[cfg(target_os = "linux")]
pub(crate) fn start_grid_services(
    databases: GridServiceDatabases<'_>,
    port: u16,
    metrics_port: Option<u16>,
    log: &Path,
    ini_path: &Path,
    docker: bool,
) -> Result<Owned, String> {
    let GridServiceDatabases {
        asset: asset_database,
        account: account_database,
        auth: auth_database,
    } = databases;
    if asset_database.is_none() && account_database.is_none() && auth_database.is_none() {
        return Err("ferroan-robust needs an asset, account or auth database".to_string());
    }
    let binary = if docker {
        PathBuf::new()
    } else {
        live::build_binary("ferroan-robust")?
    };
    let empty = ini_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("rust-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let mut ini = format!("[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n");
    if let Some(database) = asset_database {
        let db = database.display();
        ini.push_str(&format!(
            "[AssetService]\n    \
             LocalServiceModule = \"OpenSim.Services.AssetService.dll:AssetService\"\n    \
             StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
             ConnectionString = \"Data Source={db};Version=3;\"\n"
        ));
    }
    if let Some(database) = account_database {
        if asset_database.is_some() {
            ini.push('\n');
        }
        let db = database.display();
        ini.push_str(&format!(
            "[UserAccountService]\n    \
             LocalServiceModule = \"OpenSim.Services.UserAccountService.dll:UserAccountService\"\n    \
             StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
             ConnectionString = \"Data Source={db};Version=3;\"\n    \
             AllowCreateUser = true\n"
        ));
    }
    if let Some(database) = auth_database {
        if asset_database.is_some() || account_database.is_some() {
            ini.push('\n');
        }
        let db = database.display();
        ini.push_str(&format!(
            "[AuthenticationService]\n    \
             LocalServiceModule = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\"\n    \
             StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
             ConnectionString = \"Data Source={db};Version=3;\"\n    \
             AllowSetPassword = true\n"
        ));
    }
    if let Some(metrics) = metrics_port {
        ini.push_str(&format!(
            "\n[Monitoring]\n    Metrics = true\n    Listen = \"127.0.0.1:{metrics}\"\n"
        ));
    }
    live::write_secret(ini_path, &ini)?;
    let ini_arg = ini_path.to_str().ok_or("service ini path is not utf-8")?;
    let empty_arg = empty.to_str().ok_or("service config path is not utf-8")?;
    let mut server = if docker {
        let workdir = ini_path.parent().ok_or("service ini path has no parent")?;
        let args = vec![
            "--inimaster".to_string(),
            ini_arg.to_string(),
            "--inidirectory".to_string(),
            empty_arg.to_string(),
        ];
        let live_root = live::live_root();
        let logdata = crate::docker::host_log_dir(crate::docker::NAME_ASSETS);
        let mounts = vec![
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
        ];
        let attached = crate::docker::launch(crate::docker::LaunchRequest {
            name: crate::docker::NAME_ASSETS,
            image: crate::docker::ROBUST_IMAGE,
            entrypoint: None,
            args: &args,
            workdir,
            log,
            pipe_stdin: false,
            detach: false,
            mounts: &mounts,
            track: true,
            redact: &[],
        })?;
        owned_from_attached(attached)?
    } else {
        spawn_owned(
            &binary,
            &["--inimaster", ini_arg, "--inidirectory", empty_arg],
            &repo_root(),
            log,
            false,
            true,
        )?
    };
    if let Err(error) = wait_service(&mut server, "ferroan", port, log) {
        stop_owned(&mut server);
        return Err(error);
    }
    Ok(server)
}

#[cfg(target_os = "linux")]
fn start_robust(database: &Path, port: u16, log: &Path, load: bool) -> Result<Owned, String> {
    let binary = live::build_binary("ferroan-robust")?;
    let run = live::scratch_run();
    let empty = run.join("grid-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_path = run.join(if load {
        "robust-seed.ini"
    } else {
        "robust.ini"
    });
    let loader = if load {
        let assets = live::asset_set_path();
        if !assets.is_file() {
            return Err(format!("missing {}", assets.display()));
        }
        let text = assets.display().to_string();
        if text.contains('"') {
            return Err("asset set path contains a quote".to_string());
        }
        format!(
            "    DefaultAssetLoader = \"OpenSim.Framework.AssetLoader.Filesystem.dll\"\n    \
             AssetLoaderArgs = \"{text}\"\n"
        )
    } else {
        String::new()
    };
    let db = database.display();
    let ini = format!(
        "[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n[AssetService]\n    \
         LocalServiceModule = \"OpenSim.Services.AssetService.dll:AssetService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n{loader}"
    );
    live::write_secret(&ini_path, &ini)?;
    spawn_owned(
        &binary,
        &[
            "--inimaster",
            ini_path.to_str().ok_or("robust ini path is not utf-8")?,
            "--inidirectory",
            empty.to_str().ok_or("robust config path is not utf-8")?,
        ],
        &repo_root(),
        log,
        false,
        true,
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn spawn_owned(
    binary: &Path,
    args: &[&str],
    cwd: &Path,
    log: &Path,
    pipe_stdin: bool,
    track: bool,
) -> Result<Owned, String> {
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
    command.process_group(0);
    let (mut child, tracked) = if track {
        let (child, tracked) = live::spawn_tracked(&mut command, binary, args, cwd)?;
        (child, Some(tracked))
    } else {
        (
            command
                .spawn()
                .map_err(|error| format!("could not start {}: {error}", binary.display()))?,
            None,
        )
    };
    let stdin = child.stdin.take();
    let listen_pid = child.id();
    Ok(Owned {
        child,
        stdin,
        stop_on_drop: true,
        tracked,
        container: None,
        listen_pid,
    })
}

#[cfg(target_os = "linux")]
fn owned_from_attached(attached: crate::docker::Attached) -> Result<Owned, String> {
    let crate::docker::Attached {
        child,
        stdin,
        name,
        listen_pid,
        tracked,
    } = attached;
    Ok(Owned {
        child,
        stdin,
        stop_on_drop: true,
        tracked,
        container: Some(name),
        listen_pid,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn spawn_proxy(
    upstream: SocketAddr,
    out: &Path,
) -> Result<(u16, crate::live_proxy::Proxies), String> {
    let (addresses, proxies) = crate::live_proxy::Proxies::start(&[(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        upstream,
        out.to_path_buf(),
    )])?;
    Ok((
        addresses.first().ok_or("asset proxy did not bind")?.port(),
        proxies,
    ))
}

#[cfg(target_os = "linux")]
fn stop_owned(owned: &mut Owned) {
    owned.stop_on_drop = false;
    if let Some(name) = owned.container.take() {
        crate::docker::remove_container(&name);
    }
    if matches!(owned.child.try_wait(), Ok(Some(_))) {
        if let Some(tracked) = owned.tracked.take() {
            let _ = tracked.forget();
        }
        return;
    }
    let pid = owned.child.id();
    drop(owned.stdin.take());
    let _ = crate::signal::signal_group(crate::signal::Signal::Term, pid);
    let started = Instant::now();
    while matches!(owned.child.try_wait(), Ok(None)) && started.elapsed() < Duration::from_secs(20)
    {
        thread::sleep(Duration::from_millis(200));
    }
    if matches!(owned.child.try_wait(), Ok(None)) {
        let _ = crate::signal::signal_group(crate::signal::Signal::Kill, pid);
    }
    let _ = owned.child.wait();
    if let Some(tracked) = owned.tracked.take()
        && let Err(error) = tracked.forget()
    {
        eprintln!("live: asset process cleanup: {error}");
    }
}

#[cfg(all(target_os = "linux", test))]
fn pid_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "linux")]
pub(crate) fn tail(path: &Path) -> String {
    let Ok(bytes) = fs::read(path) else {
        return format!("missing {}", path.display());
    };
    let text = String::from_utf8_lossy(&bytes);
    let start = text.len().saturating_sub(2000);
    text[start..].to_string()
}

/// Method, path and body of each asset exchange, with generated ids folded.
#[cfg(any(target_os = "linux", test))]
pub fn canonical_http(bytes: &[u8]) -> Result<String, String> {
    let records = read_complete_prefix(bytes).map_err(|error| error.to_string())?;
    let mut ids = BTreeMap::new();
    let mut next = 1u32;
    let mut lines = Vec::new();
    let mut index = 0;
    while index < records.len() {
        if records[index].kind != Kind::HttpRequest {
            index += 1;
            continue;
        }
        let request = &records[index];
        let response = records
            .get(index + 1)
            .filter(|record| record.kind == Kind::HttpResponse);
        index += if response.is_some() { 2 } else { 1 };
        let (start, body) = split_message(&request.bytes);
        let start = fold_ids(&start, &mut ids, &mut next);
        let body = fold_body(&body, &mut ids, &mut next);
        let status = response
            .map(|record| status_code(&record.bytes))
            .unwrap_or_else(|| "none".to_string());
        let response_body = response
            .map(|record| fold_body(&split_message(&record.bytes).1, &mut ids, &mut next))
            .unwrap_or_default();
        lines.push(format!("{start} -> {status}\n{body}\n{response_body}"));
    }
    if lines.is_empty() {
        return Err("the asset proxy recorded no HTTP".to_string());
    }
    Ok(lines.join("\n"))
}

#[cfg(any(target_os = "linux", test))]
fn split_message(bytes: &[u8]) -> (String, Vec<u8>) {
    let Some(at) = find_crlf_crlf(bytes) else {
        return (
            String::from_utf8_lossy(bytes).trim().to_string(),
            Vec::new(),
        );
    };
    let start = String::from_utf8_lossy(&bytes[..at])
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    (start, bytes[at + 4..].to_vec())
}

#[cfg(any(target_os = "linux", test))]
fn find_crlf_crlf(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

#[cfg(any(target_os = "linux", test))]
fn status_code(bytes: &[u8]) -> String {
    let (start, _) = split_message(bytes);
    start.split_whitespace().nth(1).unwrap_or("?").to_string()
}

#[cfg(any(target_os = "linux", test))]
fn fold_body(body: &[u8], ids: &mut BTreeMap<String, String>, next: &mut u32) -> String {
    match String::from_utf8(body.to_vec()) {
        Ok(text) => fold_ids(&text, ids, next),
        Err(_) => format!("binary {} {:016x}", body.len(), digest(body)),
    }
}

#[cfg(any(target_os = "linux", test))]
fn fold_ids(text: &str, ids: &mut BTreeMap<String, String>, next: &mut u32) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if uuid_at(bytes, index) {
            let key = text[index..index + 36].to_ascii_lowercase();
            let placeholder = ids.entry(key).or_insert_with(|| {
                let value = format!("00000000-0000-4000-8000-{next:012x}");
                *next += 1;
                value
            });
            out.extend_from_slice(placeholder.as_bytes());
            index += 36;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

#[cfg(any(target_os = "linux", test))]
fn uuid_at(bytes: &[u8], start: usize) -> bool {
    if start > 0 && bytes[start - 1].is_ascii_hexdigit() {
        return false;
    }
    let groups = [8usize, 4, 4, 4, 12];
    let mut index = start;
    for (nth, length) in groups.iter().enumerate() {
        if nth > 0 {
            if bytes.get(index) != Some(&b'-') {
                return false;
            }
            index += 1;
        }
        let Some(end) = index.checked_add(*length) else {
            return false;
        };
        if end > bytes.len()
            || !bytes[index..end]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return false;
        }
        index = end;
    }
    !bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_hexdigit())
}

#[cfg(any(target_os = "linux", test))]
fn digest(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(target_os = "linux")]
pub(crate) fn first_difference(left: &str, right: &str) -> String {
    for (index, (one, other)) in left.lines().zip(right.lines()).enumerate() {
        if one != other {
            return format!("first difference at line {index}\ncsharp: {one}\nferroan: {other}");
        }
    }
    format!(
        "different lengths (csharp {} lines, ferroan {} lines)",
        left.lines().count(),
        right.lines().count()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_capture::{Kind, Record, write_capture};

    #[test]
    fn generated_ids_and_headers_fold_to_the_same_exchange() {
        let csharp = capture(
            b"POST /assets/ HTTP/1.1\r\nDate: Mon\r\n\r\n<ID>aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</ID>",
            b"HTTP/1.1 200 OK\r\nServer: csharp\r\n\r\n<ID>aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</ID>",
        );
        let ferroan = capture(
            b"POST /assets/ HTTP/1.1\r\nDate: Tue\r\n\r\n<ID>11111111-2222-3333-4444-555555555555</ID>",
            b"HTTP/1.1 200 OK\r\nServer: ferroan\r\n\r\n<ID>11111111-2222-3333-4444-555555555555</ID>",
        );
        assert_eq!(
            canonical_http(&csharp).unwrap(),
            canonical_http(&ferroan).unwrap()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_failed_start_drops_and_reaps_the_owned_process() {
        let dir = repo_root()
            .join(".scratch")
            .join(format!("asset-process-test-{}", uuid::Uuid::new_v4()));
        let owned = spawn_owned(
            Path::new("/bin/sleep"),
            &["30"],
            &repo_root(),
            &dir.join("child.log"),
            false,
            false,
        )
        .unwrap();
        let pid = owned.pid();
        assert!(pid_exists(pid));
        drop(owned);
        assert!(!pid_exists(pid));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn persistent_stop_refuses_an_unrelated_process_and_reused_identity() {
        let dir = repo_root()
            .join(".scratch")
            .join(format!("asset-owner-test-{}", uuid::Uuid::new_v4()));
        let mut owned = spawn_owned(
            Path::new("/bin/sleep"),
            &["30"],
            &repo_root(),
            &dir.join("child.log"),
            false,
            false,
        )
        .unwrap();
        let pid = owned.pid();
        let start = live::process_start_time(pid).unwrap();
        stop_persistent(pid, start).unwrap();
        stop_persistent(pid, start + 1).unwrap();
        assert!(owned.child.try_wait().unwrap().is_none());
        drop(owned);
        fs::remove_dir_all(dir).unwrap();
    }

    fn capture(request: &[u8], response: &[u8]) -> Vec<u8> {
        write_capture(&[
            Record {
                kind: Kind::HttpRequest,
                time_ms: 1,
                bytes: request.to_vec(),
            },
            Record {
                kind: Kind::HttpResponse,
                time_ms: 2,
                bytes: response.to_vec(),
            },
        ])
        .unwrap()
    }
}
