// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Authentication and authorization A/B.
//!
//! `cargo xtask live compare auth` starts the C# connectors and
//! `ferroan-robust` against separate copies of the AuthStore SQLite fixture,
//! sends the same requests, and compares the HTTP. A token is random, so the
//! `<Token>` text is folded before the comparison. One shared database is
//! then written by each side and read by the other. Request bodies are not
//! printed.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use crate::live;
#[cfg(target_os = "linux")]
use crate::repo_root;

#[cfg(target_os = "linux")]
const PRINCIPAL: &str = "11111111-1111-4111-8111-111111111111";
#[cfg(target_os = "linux")]
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
#[cfg(target_os = "linux")]
const WEB_KEY: &str = "33333333-3333-4333-8333-333333333333";
/// `MD5("harness-secret")`, the value login posts after stripping `$1$`.
#[cfg(any(target_os = "linux", test))]
const PRESENTED: &str = "d0f269068995fbfb1fd0c82e1e7d4afa";
#[cfg(target_os = "linux")]
const SALT: &str = "0123456789abcdef0123456789abcdef";
/// `MD5(PRESENTED + ":" + SALT)`.
#[cfg(any(target_os = "linux", test))]
const STORED: &str = "b90b6b949d1ed53d0c113a47dd1dca3f";

/// Captured `AuthorizationRequest`. The response message is this `ID`.
#[cfg(any(target_os = "linux", test))]
const AUTHORIZATION_REQUEST: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<AuthorizationRequest xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n  <ID>11111111-1111-1111-1111-111111111111</ID>\n  <FirstName>Test</FirstName>\n  <SurName>User</SurName>\n  <Email>a@b.invalid</Email>\n  <RegionName>Region</RegionName>\n  <RegionID>22222222-2222-2222-2222-222222222222</RegionID>\n</AuthorizationRequest>";

pub fn compare() -> ExitCode {
    match compare_inner() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("live: {}", redact(&error));
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
        Err("cargo xtask live compare auth runs on Linux".to_string())
    }
}

fn redact(text: &str) -> String {
    text.replace("harness-secret", "[redacted]")
}

#[cfg(any(target_os = "linux", test))]
fn fold_token(body: &str) -> String {
    let open = "<Token>";
    let close = "</Token>";
    let Some(start) = body.find(open) else {
        return body.to_string();
    };
    let value_at = start + open.len();
    let Some(end) = body[value_at..].find(close) else {
        return body.to_string();
    };
    let mut out = String::with_capacity(body.len());
    out.push_str(&body[..value_at]);
    out.push_str("token");
    out.push_str(&body[value_at + end..]);
    out
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
    let fixture = repo_root().join("fixtures/db/sqlite/AuthStore.db");
    if !fixture.is_file() {
        return Err(format!("missing {}", fixture.display()));
    }
    let csharp = run_side("csharp", &fixture)?;
    let ferroan = run_side("ferroan", &fixture)?;
    if csharp.len() != ferroan.len() {
        return Err(format!(
            "auth HTTP count differs: csharp {}, ferroan {}",
            csharp.len(),
            ferroan.len()
        ));
    }
    for (index, (left, right)) in csharp.iter().zip(ferroan.iter()).enumerate() {
        if left != right {
            return Err(format!(
                "auth HTTP differs at record {}\ncsharp:\n{}\nferroan:\n{}",
                index + 1,
                clip(left),
                clip(right)
            ));
        }
    }
    cross_read(&fixture)?;
    Ok(format!(
        "auth: match ({} http records); cross-read ok",
        csharp.len()
    ))
}

#[cfg(target_os = "linux")]
fn clip(text: &str) -> String {
    let text = redact(text);
    if text.len() <= 2000 {
        text
    } else {
        format!("{}…", &text[..2000])
    }
}

#[cfg(target_os = "linux")]
fn run_side(name: &str, fixture: &Path) -> Result<Vec<String>, String> {
    let run = live::scratch_run();
    fs::create_dir_all(&run)
        .map_err(|error| format!("could not create {}: {error}", run.display()))?;
    let database = run.join(format!("auth-{name}.db"));
    copy_database(fixture, &database)?;
    let asset = run.join(format!("auth-assets-{name}.db"));
    fs::File::create(&asset)
        .map_err(|error| format!("could not create {}: {error}", asset.display()))?;
    let port = live::allocate_port()?;
    let log = run.join(format!("auth-{name}.log"));
    let mut server = if name == "csharp" {
        start_csharp(&database, &asset, port, &log)?
    } else if name == "ferroan" {
        start_robust(&database, &asset, port, &log)?
    } else {
        return Err(format!("unknown auth side {name}"));
    };
    if let Err(error) = wait_service(&mut server, name, port, &log) {
        stop_running(&mut server);
        return Err(error);
    }
    if name == "csharp" {
        wait_for_log(
            &mut server,
            &log,
            "AuthorizationServerConnector loaded successfully",
        )?;
    }
    println!("live: auth {name} on http://127.0.0.1:{port}");
    let driven = drive(&mut server, name, port, &log);
    stop_running(&mut server);
    driven
}

/// Same requests on each side. `setpassword` creates the row; `setauthinfo`
/// then stores the fixed hash so `getauthinfo` can match. The token from
/// `authenticate` is used for `verify` and `release` on that side only.
#[cfg(target_os = "linux")]
fn drive(server: &mut Running, name: &str, port: u16, log: &Path) -> Result<Vec<String>, String> {
    let mut records = Vec::new();
    let form = |step: &str, path: &str, body: &str| -> Result<Reply, String> {
        post_form(port, path, body)
            .map_err(|error| format!("auth {name} {step} failed: {error}\n{}", tail(log)))
    };
    let push = |records: &mut Vec<String>, reply: Reply| {
        records.push(fold_token(&canonical(&reply)));
    };
    let set_password = format!("METHOD=setpassword&PRINCIPAL={PRINCIPAL}&PASSWORD=harness-secret");
    push(
        &mut records,
        checked(
            server,
            log,
            form("setpassword", "/auth/plain", &set_password)?,
        )?,
    );
    let set_info = format!(
        "METHOD=setauthinfo&PRINCIPAL={PRINCIPAL}&AccountType=UserAccount&PasswordHash={STORED}&PasswordSalt={SALT}&WebLoginKey={WEB_KEY}"
    );
    push(
        &mut records,
        checked(server, log, form("setauthinfo", "/auth/plain", &set_info)?)?,
    );
    let info = format!("METHOD=getauthinfo&PRINCIPAL={PRINCIPAL}");
    push(
        &mut records,
        checked(server, log, form("getauthinfo", "/auth/plain", &info)?)?,
    );
    let authenticate = format!("METHOD=authenticate&PRINCIPAL={PRINCIPAL}&PASSWORD={PRESENTED}");
    let authed = checked(
        server,
        log,
        form("authenticate", "/auth/plain", &authenticate)?,
    )?;
    let token = token_of(&authed.body).ok_or_else(|| {
        format!(
            "auth {name} authenticate returned no token\n{}",
            clip(&canonical(&authed))
        )
    })?;
    push(&mut records, authed);
    let verify = format!("METHOD=verify&PRINCIPAL={PRINCIPAL}&TOKEN={token}");
    push(
        &mut records,
        checked(server, log, form("verify", "/auth/plain", &verify)?)?,
    );
    let release = format!("METHOD=release&PRINCIPAL={PRINCIPAL}&TOKEN={token}");
    push(
        &mut records,
        checked(server, log, form("release", "/auth/plain", &release)?)?,
    );
    push(
        &mut records,
        checked(
            server,
            log,
            form("verify-after-release", "/auth/plain", &verify)?,
        )?,
    );
    let wrong = format!(
        "METHOD=authenticate&PRINCIPAL={PRINCIPAL}&PASSWORD=00000000000000000000000000000000"
    );
    push(
        &mut records,
        checked(server, log, form("wrong-password", "/auth/plain", &wrong)?)?,
    );
    push(
        &mut records,
        checked(
            server,
            log,
            form(
                "missing-password",
                "/auth/plain",
                &format!("METHOD=authenticate&PRINCIPAL={PRINCIPAL}"),
            )?,
        )?,
    );
    push(
        &mut records,
        checked(
            server,
            log,
            form("missing-principal", "/auth/plain", "METHOD=authenticate")?,
        )?,
    );
    push(
        &mut records,
        checked(
            server,
            log,
            form(
                "bad-uuid",
                "/auth/plain",
                "METHOD=authenticate&PRINCIPAL=not-a-uuid&PASSWORD=x",
            )?,
        )?,
    );
    push(
        &mut records,
        checked(
            server,
            log,
            form(
                "unknown-method",
                "/auth/plain",
                &format!("METHOD=no-such-method&PRINCIPAL={PRINCIPAL}"),
            )?,
        )?,
    );
    let bad_life =
        format!("METHOD=authenticate&PRINCIPAL={PRINCIPAL}&PASSWORD={PRESENTED}&LIFETIME=nope");
    push(
        &mut records,
        checked(server, log, form("bad-lifetime", "/auth/plain", &bad_life)?)?,
    );
    push(
        &mut records,
        checked(server, log, form("crypt", "/auth/crypt", "ignored")?)?,
    );
    push(
        &mut records,
        checked(server, log, form("bare", "/auth", "")?)?,
    );
    let authorized = post_xml(port, "/authorization", AUTHORIZATION_REQUEST)
        .map_err(|error| format!("auth {name} authorization failed: {error}\n{}", tail(log)))?;
    push(&mut records, checked(server, log, authorized)?);
    // Exercise the R13 serializer and form cases on both running connectors.
    let edges: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/auth-edges/cases.json"))
            .map_err(|error| error.to_string())?;
    for case in edges["xml"]
        .as_array()
        .ok_or("auth fixture is missing XML cases")?
    {
        let request = case["request"]
            .as_str()
            .ok_or("auth fixture is missing a request")?;
        let reply = post_xml(port, "/authorization", request)?;
        push(&mut records, checked(server, log, reply)?);
    }
    let list = edges["list"]["request"]
        .as_str()
        .ok_or("auth fixture is missing the list form")?;
    push(
        &mut records,
        checked(server, log, form("list", "/auth/plain", list)?)?,
    );
    push(
        &mut records,
        checked(server, log, form("read-list", "/auth/plain", &info)?)?,
    );
    Ok(records)
}

#[cfg(target_os = "linux")]
fn checked(server: &mut Running, log: &Path, reply: Reply) -> Result<Reply, String> {
    if server
        .child
        .try_wait()
        .map_err(|error| format!("could not check the auth service: {error}"))?
        .is_some()
    {
        return Err(format!("the auth service exited\n{}", tail(log)));
    }
    Ok(reply)
}

#[cfg(target_os = "linux")]
fn token_of(body: &str) -> Option<String> {
    let start = body.find("<Token>")? + "<Token>".len();
    let end = body[start..].find("</Token>")? + start;
    let token = &body[start..end];
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Rust writes principal A. C# reads it, then writes principal B. Rust reads B.
#[cfg(target_os = "linux")]
fn cross_read(fixture: &Path) -> Result<(), String> {
    let run = live::scratch_run();
    let database = run.join("auth-cross.db");
    copy_database(fixture, &database)?;
    let asset = run.join("auth-cross-assets.db");
    fs::File::create(&asset)
        .map_err(|error| format!("could not create {}: {error}", asset.display()))?;
    let presented = format!("METHOD=authenticate&PRINCIPAL={{id}}&PASSWORD={PRESENTED}");
    let set_password =
        |id: &str| format!("METHOD=setpassword&PRINCIPAL={id}&PASSWORD=harness-secret");
    write_then_read(
        "ferroan",
        "csharp",
        &database,
        &asset,
        &set_password(PRINCIPAL),
        &presented.replace("{id}", PRINCIPAL),
    )?;
    write_then_read(
        "csharp",
        "ferroan",
        &database,
        &asset,
        &set_password(OTHER),
        &presented.replace("{id}", OTHER),
    )?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_then_read(
    writer: &str,
    reader: &str,
    database: &Path,
    asset: &Path,
    write_body: &str,
    read_body: &str,
) -> Result<(), String> {
    let port = live::allocate_port()?;
    let log = live::scratch_run().join(format!("auth-cross-{writer}.log"));
    let mut server = start_named(writer, database, asset, port, &log)?;
    if let Err(error) = wait_service(&mut server, writer, port, &log) {
        stop_running(&mut server);
        return Err(error);
    }
    if writer == "csharp" {
        wait_for_log(
            &mut server,
            &log,
            "AuthorizationServerConnector loaded successfully",
        )?;
    }
    let wrote = post_form(port, "/auth/plain", write_body);
    stop_running(&mut server);
    let wrote = wrote?;
    if !wrote.body.contains("Success") {
        return Err(format!(
            "auth cross-read {writer} setpassword failed\n{}",
            clip(&canonical(&wrote))
        ));
    }
    let port = live::allocate_port()?;
    let log = live::scratch_run().join(format!("auth-cross-{reader}.log"));
    let mut server = start_named(reader, database, asset, port, &log)?;
    if let Err(error) = wait_service(&mut server, reader, port, &log) {
        stop_running(&mut server);
        return Err(error);
    }
    if reader == "csharp" {
        wait_for_log(
            &mut server,
            &log,
            "AuthorizationServerConnector loaded successfully",
        )?;
    }
    let read = post_form(port, "/auth/plain", read_body);
    stop_running(&mut server);
    let read = read?;
    if token_of(&read.body).is_none() {
        return Err(format!(
            "auth cross-read {reader} did not accept the password stored by {writer}\n{}",
            clip(&canonical(&read))
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn start_named(
    name: &str,
    database: &Path,
    asset: &Path,
    port: u16,
    log: &Path,
) -> Result<Running, String> {
    if name == "csharp" {
        start_csharp(database, asset, port, log)
    } else if name == "ferroan" {
        start_robust(database, asset, port, log)
    } else {
        Err(format!("unknown auth side {name}"))
    }
}

#[cfg(target_os = "linux")]
fn copy_database(source: &Path, dest: &Path) -> Result<(), String> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = dest.with_file_name(format!(
            "{}{suffix}",
            dest.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("auth.db")
        ));
        match fs::remove_file(&sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("could not remove {}: {error}", sidecar.display()));
            }
        }
    }
    fs::copy(source, dest).map(|_| ()).map_err(|error| {
        format!(
            "could not copy {} to {}: {error}",
            source.display(),
            dest.display()
        )
    })
}

#[cfg(target_os = "linux")]
struct Running {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    tracked: Option<crate::grid_state::Tracked>,
    stop_on_drop: bool,
}

#[cfg(target_os = "linux")]
impl Drop for Running {
    fn drop(&mut self) {
        if self.stop_on_drop {
            stop_running(self);
        }
    }
}

#[cfg(target_os = "linux")]
fn ini_services(auth: &str, asset: &str, port: u16, service_list: bool) -> String {
    let list = if service_list {
        format!(
            "[Const]\n    BaseHostname = \"127.0.0.1\"\n    PublicPort = \"{port}\"\n    \
             PrivatePort = \"{port}\"\n\n[Startup]\n    RegistryLocation = \".\"\n    \
             ConsoleHistoryFileEnabled = false\n    EnableRobustSelfsignedCertSupport = false\n\n\
             [ServiceList]\n    AuthenticationServiceConnector = \
             \"{port}/OpenSim.Server.Handlers.dll:AuthenticationServiceConnector\"\n    \
             AuthorizationServerConnector = \
             \"{port}/OpenSim.Server.Handlers.dll:AuthorizationServerConnector\"\n\n\
             [DatabaseService]\n    StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
             ConnectionString = \"Data Source={auth};Version=3;\"\n\n"
        )
    } else {
        String::new()
    };
    format!(
        "{list}[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n\
         [AuthenticationService]\n    \
         LocalServiceModule = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={auth};Version=3;\"\n    \
         AllowSetPassword = true\n    AllowGetAuthInfo = true\n    AllowSetAuthInfo = true\n\n\
         [AuthorizationService]\n    \
         LocalServiceModule = \"OpenSim.Services.AuthorizationService.dll:AuthorizationService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={asset};Version=3;\"\n"
    )
}

#[cfg(target_os = "linux")]
fn start_csharp(database: &Path, asset: &Path, port: u16, log: &Path) -> Result<Running, String> {
    let binary = live::grid_server_binary();
    if !binary.is_file() {
        return Err("run cargo xtask live build first".to_string());
    }
    let run = live::scratch_run();
    let empty = run.join("auth-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_path = run.join("auth-csharp.ini");
    let auth = database.to_str().ok_or("auth database path is not utf-8")?;
    let asset = asset.to_str().ok_or("auth asset path is not utf-8")?;
    live::write_secret(&ini_path, &ini_services(auth, asset, port, true))?;
    spawn_running(
        &binary,
        &[
            "--inimaster",
            ini_path.to_str().ok_or("auth ini path is not utf-8")?,
            "--inidirectory",
            empty.to_str().ok_or("auth config path is not utf-8")?,
            "--console",
            "basic",
        ],
        binary.parent().unwrap_or(Path::new(".")),
        log,
        true,
    )
}

#[cfg(target_os = "linux")]
fn start_robust(database: &Path, asset: &Path, port: u16, log: &Path) -> Result<Running, String> {
    let binary = live::build_binary("ferroan-robust")?;
    let run = live::scratch_run();
    let empty = run.join("auth-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_path = run.join("auth-rust.ini");
    let auth = database.to_str().ok_or("auth database path is not utf-8")?;
    let asset = asset.to_str().ok_or("auth asset path is not utf-8")?;
    live::write_secret(&ini_path, &ini_services(auth, asset, port, false))?;
    spawn_running(
        &binary,
        &[
            "--inimaster",
            ini_path.to_str().ok_or("auth ini path is not utf-8")?,
            "--inidirectory",
            empty.to_str().ok_or("auth config path is not utf-8")?,
        ],
        &repo_root(),
        log,
        false,
    )
}

#[cfg(target_os = "linux")]
fn spawn_running(
    binary: &Path,
    args: &[&str],
    cwd: &Path,
    log: &Path,
    pipe_stdin: bool,
) -> Result<Running, String> {
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
    let (mut child, tracked) = live::spawn_tracked(&mut command, binary, args, cwd)?;
    let stdin = child.stdin.take();
    Ok(Running {
        child,
        stdin,
        tracked: Some(tracked),
        stop_on_drop: true,
    })
}

#[cfg(target_os = "linux")]
fn wait_for_log(server: &mut Running, log: &Path, needle: &str) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        if server
            .child
            .try_wait()
            .map_err(|error| format!("could not check the auth service: {error}"))?
            .is_some()
        {
            return Err(format!("the auth service exited\n{}", tail(log)));
        }
        if fs::read_to_string(log).unwrap_or_default().contains(needle) {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err(format!(
                "the auth service did not report {needle}\n{}",
                tail(log)
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(target_os = "linux")]
fn wait_service(server: &mut Running, name: &str, port: u16, log: &Path) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        if server
            .child
            .try_wait()
            .map_err(|error| format!("could not check {name}: {error}"))?
            .is_some()
        {
            return Err(format!("the {name} auth service exited\n{}", tail(log)));
        }
        if live::require_http_loopback(server.child.id(), port).is_ok() {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err(format!(
                "the {name} auth service did not listen on 127.0.0.1:{port}\n{}",
                tail(log)
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(target_os = "linux")]
fn post_form(port: u16, path: &str, body: &str) -> Result<Reply, String> {
    post_bytes(
        port,
        path,
        "application/x-www-form-urlencoded",
        body.as_bytes(),
    )
}

#[cfg(target_os = "linux")]
fn post_xml(port: u16, path: &str, body: &str) -> Result<Reply, String> {
    post_bytes(port, path, "application/xml", body.as_bytes())
}

#[cfg(target_os = "linux")]
fn post_bytes(port: u16, path: &str, content_type: &str, body: &[u8]) -> Result<Reply, String> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|error| format!("could not connect to 127.0.0.1:{port}: {error}"))?;
    stream
        .set_nodelay(true)
        .map_err(|error| format!("could not set TCP_NODELAY: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|error| format!("could not set the read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| format!("could not set the write timeout: {error}"))?;
    let header = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|error| format!("could not write the auth request: {error}"))?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read the auth response: {error}"))?;
    parse_response(&bytes)
}

#[cfg(target_os = "linux")]
fn stop_running(owned: &mut Running) {
    owned.stop_on_drop = false;
    if matches!(owned.child.try_wait(), Ok(Some(_))) {
        if let Some(tracked) = owned.tracked.take() {
            let _ = tracked.forget();
        }
        return;
    }
    let pid = owned.child.id();
    drop(owned.stdin.take());
    let _ = crate::signal::signal_group(crate::signal::Signal::Term, pid);
    let started = std::time::Instant::now();
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
        eprintln!("live: auth process cleanup: {error}");
    }
}

#[cfg(target_os = "linux")]
fn tail(path: &Path) -> String {
    let Ok(bytes) = fs::read(path) else {
        return format!("missing {}", path.display());
    };
    let text = String::from_utf8_lossy(&bytes);
    let start = text.len().saturating_sub(2000);
    redact(&text[start..])
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct Reply {
    status: u16,
    content_type: String,
    body: String,
}

#[cfg(target_os = "linux")]
fn parse_response(bytes: &[u8]) -> Result<Reply, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "auth response is not utf-8".to_string())?;
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Err("auth response has no header end".to_string());
    };
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err(format!("auth response is not HTTP/1: {status_line}"));
    }
    let status = parts
        .next()
        .unwrap_or("")
        .parse::<u16>()
        .map_err(|_| format!("auth response has no status: {status_line}"))?;
    let mut content_type = String::new();
    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("auth response has a bad header: {line}"));
        };
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value.trim().to_string();
        } else if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| format!("auth response has a bad content length: {value}"))?,
            );
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            return Err("auth response is chunked".to_string());
        }
    }
    let body = match content_length {
        Some(length) if body.len() < length => {
            return Err(format!(
                "auth response body is short: {length} bytes expected, {} present",
                body.len()
            ));
        }
        Some(length) => body[..length].to_string(),
        None => body.to_string(),
    };
    Ok(Reply {
        status,
        content_type,
        body,
    })
}

#[cfg(target_os = "linux")]
fn canonical(reply: &Reply) -> String {
    format!("{}\n{}\n{}", reply.status, reply.content_type, reply.body)
}

#[cfg(test)]
mod tests {
    use super::{AUTHORIZATION_REQUEST, PRESENTED, STORED, fold_token};

    #[test]
    fn a_token_folds_and_the_authorization_id_is_stable() {
        let body =
            "<Result>Success</Result>\n  <Token>11111111-1111-4111-8111-111111111111</Token>\n";
        assert_eq!(
            fold_token(body),
            "<Result>Success</Result>\n  <Token>token</Token>\n"
        );
        assert_eq!(
            fold_token("<Result>Failure</Result>"),
            "<Result>Failure</Result>"
        );
        assert!(AUTHORIZATION_REQUEST.contains("11111111-1111-1111-1111-111111111111"));
        assert_eq!(PRESENTED.len(), 32);
        assert_eq!(STORED.len(), 32);
        assert!(!AUTHORIZATION_REQUEST.contains("harness-secret"));
    }
}
