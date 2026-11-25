// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! User-account A/B.
//!
//! `cargo xtask live compare accounts` starts the C# user-account connector
//! and `ferroan-robust` against separate copies of the UserAccount SQLite
//! fixture, sends the same forms, and compares the HTTP. `Created` and
//! `NameChanged` are the current time, so those values are folded before the
//! comparison. The request bodies are not printed.

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

#[cfg(any(target_os = "linux", test))]
const PRINCIPAL: &str = "11111111-1111-4111-8111-111111111111";

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
        Err("cargo xtask live compare accounts runs on Linux".to_string())
    }
}

/// The forms, in order. Callers compare replies and do not log these bodies.
#[cfg(any(target_os = "linux", test))]
fn forms() -> Vec<(&'static str, String)> {
    let id = PRINCIPAL;
    vec![
        (
            "/accounts",
            "METHOD=getaccount&FirstName=GRID&LastName=SERVICES".to_string(),
        ),
        (
            "/accounts",
            format!(
                "METHOD=createuser&FirstName=Test&LastName=User&Password=harness-password&Email=test@example.com&PrincipalID={id}"
            ),
        ),
        (
            "/accounts",
            format!("METHOD=getaccount&UserID={id}"),
        ),
        (
            "/accounts",
            "METHOD=getaccount&Email=test@example.com".to_string(),
        ),
        ("/accounts", "METHOD=getaccounts&query=Test".to_string()),
        (
            "/accounts",
            "METHOD=getaccounts&query=ab%20GRID".to_string(),
        ),
        (
            "/accounts",
            "METHOD=getaccounts&query=Aaa%20Bbb%20Ccc".to_string(),
        ),
        (
            "/accounts",
            format!("METHOD=getmultiaccounts&IDS[]={id}"),
        ),
        (
            "/accounts",
            format!("METHOD=setdisplayname&PrincipalID={id}&DisplayName=Shown"),
        ),
        (
            "/accounts",
            format!("METHOD=getaccount&PrincipalID={id}"),
        ),
        (
            "/accounts",
            format!(
                "METHOD=setaccount&PrincipalID={id}&Email=next@example.com&UserLevel=3"
            ),
        ),
        (
            "/accounts",
            format!("METHOD=getaccount&UserID={id}"),
        ),
        (
            "/accounts",
            "METHOD=createuser&FirstName=Test&LastName=User&Password=harness-password&Email=other@example.com".to_string(),
        ),
        (
            "/accounts",
            "METHOD=getaccount&FirstName=Missing&LastName=Person".to_string(),
        ),
        ("/accounts", "METHOD=no-such-method".to_string()),
        (
            "/accounts",
            "METHOD=getaccount&ScopeID=not-a-uuid".to_string(),
        ),
        (
            "/accounts",
            "METHOD=createuser&FirstName=Only".to_string(),
        ),
    ]
}

#[cfg(any(target_os = "linux", test))]
fn fold_clocks(body: &str) -> String {
    let mut folded = fold_tag(body, "Created");
    folded = fold_tag(&folded, "NameChanged");
    folded
}

/// A unix time is replaced. Zero stays zero, so a missing update still differs.
#[cfg(any(target_os = "linux", test))]
fn fold_tag(body: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find(&open) {
        let value_at = start + open.len();
        let Some(end) = rest[value_at..].find(&close) else {
            out.push_str(rest);
            return out;
        };
        let text = &rest[value_at..value_at + end];
        out.push_str(&rest[..start]);
        out.push_str(&open);
        if text.bytes().all(|byte| byte.is_ascii_digit())
            && text
                .parse::<i64>()
                .ok()
                .is_some_and(|value| value >= 1_000_000_000)
        {
            out.push_str("clock");
        } else {
            out.push_str(text);
        }
        out.push_str(&close);
        rest = &rest[value_at + end + close.len()..];
    }
    out.push_str(rest);
    out
}

fn redact(text: &str) -> String {
    text.replace("harness-password", "[redacted]")
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug)]
struct Reply {
    status: u16,
    content_type: String,
    body: String,
}

#[cfg(any(target_os = "linux", test))]
fn parse_response(bytes: &[u8]) -> Result<Reply, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "account response is not utf-8".to_string())?;
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Err("account response has no header end".to_string());
    };
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err(format!("account response is not HTTP/1: {status_line}"));
    }
    let status = parts
        .next()
        .unwrap_or("")
        .parse::<u16>()
        .map_err(|_| format!("account response has no status: {status_line}"))?;
    let mut content_type = String::new();
    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("account response has a bad header: {line}"));
        };
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value.trim().to_string();
        } else if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| format!("account response has a bad content length: {value}"))?,
            );
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            return Err("account response is chunked".to_string());
        }
    }
    let body = match content_length {
        Some(length) if body.len() < length => {
            return Err(format!(
                "account response body is short: {length} bytes expected, {} present",
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

#[cfg(any(target_os = "linux", test))]
fn canonical(reply: &Reply) -> String {
    format!(
        "{}\n{}\n{}",
        reply.status,
        reply.content_type,
        fold_clocks(&reply.body)
    )
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
    let fixture = repo_root().join("fixtures/db/sqlite/UserAccount.db");
    if !fixture.is_file() {
        return Err(format!("missing {}", fixture.display()));
    }
    let csharp = run_side("csharp", &fixture)?;
    let ferroan = run_side("ferroan", &fixture)?;
    if csharp.len() != ferroan.len() {
        return Err(format!(
            "account HTTP count differs: csharp {}, ferroan {}",
            csharp.len(),
            ferroan.len()
        ));
    }
    for (index, (left, right)) in csharp.iter().zip(ferroan.iter()).enumerate() {
        if left != right {
            return Err(format!(
                "account HTTP differs at form {}\ncsharp:\n{}\nferroan:\n{}",
                index + 1,
                clip(left),
                clip(right)
            ));
        }
    }
    Ok(format!("accounts: match ({} http records)", csharp.len()))
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
    let database = run.join(format!("accounts-{name}.db"));
    copy_database(fixture, &database)?;
    let port = live::allocate_port()?;
    let log = run.join(format!("accounts-{name}.log"));
    let mut server = if name == "csharp" {
        start_csharp(&database, port, &log)?
    } else if name == "ferroan" {
        start_robust(&database, port, &log)?
    } else {
        return Err(format!("unknown account side {name}"));
    };
    if let Err(error) = wait_service(&mut server, name, port, &log) {
        stop_running(&mut server);
        return Err(error);
    }
    // GridServer accepts TCP before the connector is registered. A request in
    // that gap is an empty HTML page, not the account handler.
    if name == "csharp" {
        wait_for_log(
            &mut server,
            &log,
            "UserAccountServiceConnector loaded successfully",
        )?;
    }
    println!("live: accounts {name} on http://127.0.0.1:{port}");
    let mut records = Vec::new();
    for (path, body) in forms() {
        match post_form(port, path, &body) {
            Ok(reply) => records.push(canonical(&reply)),
            Err(error) => {
                stop_running(&mut server);
                return Err(format!(
                    "account {name} form failed: {error}\n{}",
                    tail(&log)
                ));
            }
        }
    }
    stop_running(&mut server);
    Ok(records)
}

#[cfg(target_os = "linux")]
fn copy_database(source: &Path, dest: &Path) -> Result<(), String> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = dest.with_file_name(format!(
            "{}{suffix}",
            dest.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("accounts.db")
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
fn start_csharp(database: &Path, port: u16, log: &Path) -> Result<Running, String> {
    let binary = live::grid_server_binary();
    if !binary.is_file() {
        return Err("run cargo xtask live build first".to_string());
    }
    let run = live::scratch_run();
    let empty = run.join("accounts-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_path = run.join("accounts-csharp.ini");
    let db = database
        .to_str()
        .ok_or("account database path is not utf-8")?;
    let ini = format!(
        "[Const]\n    BaseHostname = \"127.0.0.1\"\n    PublicPort = \"{port}\"\n    \
         PrivatePort = \"{port}\"\n\n[Startup]\n    RegistryLocation = \".\"\n    \
         ConsoleHistoryFileEnabled = false\n    EnableRobustSelfsignedCertSupport = false\n\n\
         [Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n\
         [ServiceList]\n    UserAccountServiceConnector = \
         \"{port}/OpenSim.Server.Handlers.dll:UserAccountServiceConnector\"\n\n\
         [DatabaseService]\n    StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n\n[UserAccountService]\n    \
         LocalServiceModule = \"OpenSim.Services.UserAccountService.dll:UserAccountService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n    \
         AllowCreateUser = true\n    AllowSetAccount = true\n"
    );
    live::write_secret(&ini_path, &ini)?;
    spawn_running(
        &binary,
        &[
            "--inimaster",
            ini_path.to_str().ok_or("account ini path is not utf-8")?,
            "--inidirectory",
            empty.to_str().ok_or("account config path is not utf-8")?,
            "--console",
            "basic",
        ],
        binary.parent().unwrap_or(Path::new(".")),
        log,
        true,
    )
}

#[cfg(target_os = "linux")]
fn start_robust(database: &Path, port: u16, log: &Path) -> Result<Running, String> {
    let binary = live::build_binary("ferroan-robust")?;
    let run = live::scratch_run();
    let empty = run.join("accounts-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_path = run.join("accounts-rust.ini");
    let db = database
        .to_str()
        .ok_or("account database path is not utf-8")?;
    let ini = format!(
        "[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n\
         [UserAccountService]\n    \
         LocalServiceModule = \"OpenSim.Services.UserAccountService.dll:UserAccountService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n    \
         AllowCreateUser = true\n    AllowSetAccount = true\n"
    );
    live::write_secret(&ini_path, &ini)?;
    spawn_running(
        &binary,
        &[
            "--inimaster",
            ini_path.to_str().ok_or("account ini path is not utf-8")?,
            "--inidirectory",
            empty.to_str().ok_or("account config path is not utf-8")?,
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
            .map_err(|error| format!("could not check the account service: {error}"))?
            .is_some()
        {
            return Err(format!("the account service exited\n{}", tail(log)));
        }
        if fs::read_to_string(log).unwrap_or_default().contains(needle) {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err(format!(
                "the account service did not report {needle}\n{}",
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
            return Err(format!("the {name} account service exited\n{}", tail(log)));
        }
        if live::require_http_loopback(server.child.id(), port).is_ok() {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err(format!(
                "the {name} account service did not listen on 127.0.0.1:{port}\n{}",
                tail(log)
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(target_os = "linux")]
fn post_form(port: u16, path: &str, body: &str) -> Result<Reply, String> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|error| format!("could not connect to 127.0.0.1:{port}: {error}"))?;
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
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("could not write the account request: {error}"))?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read the account response: {error}"))?;
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
        eprintln!("live: account process cleanup: {error}");
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

#[cfg(test)]
mod tests {
    use super::{canonical, fold_clocks, forms, parse_response};

    #[test]
    fn the_account_script_has_seventeen_forms() {
        let forms = forms();
        assert_eq!(forms.len(), 17);
        assert!(forms.iter().all(|(path, _)| *path == "/accounts"));
        assert!(forms[5].1.contains("ab%20GRID"));
        assert!(forms[7].1.contains("IDS[]="));
    }

    #[test]
    fn clocks_fold_and_zero_stays() {
        let body = "<Created>1700000000</Created><NameChanged>0</NameChanged>";
        assert_eq!(
            fold_clocks(body),
            "<Created>clock</Created><NameChanged>0</NameChanged>"
        );
    }

    #[test]
    fn a_content_length_response_is_read_and_chunked_is_refused() {
        let bytes = b"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: 5\r\n\r\nhelloEXTRA";
        let reply = parse_response(bytes).unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.content_type, "application/xml");
        assert_eq!(reply.body, "hello");
        assert_eq!(canonical(&reply), "200\napplication/xml\nhello");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        assert!(parse_response(chunked).unwrap_err().contains("chunked"));
    }
}
