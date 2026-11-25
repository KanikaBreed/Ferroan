// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Speak the remote console's four forms to the C# region and to Ferroan.
//!
//! The step is not part of `cargo xtask check`. Credentials are generated for
//! the run and are not printed. The startup `ReadResponses` drain is not
//! compared: it is scrollback from `lastLineSeen == 0`.
//!
//! Off Linux the server path is not compiled. The compare helpers stay so
//! the unit tests run, and `dead_code` is allowed because nothing calls them
//! from the binary.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::process::ExitCode;

const PROMPT: &str = "Region (Ferroan Test) # ";
const UNKNOWN_ID: &str = "00000000-0000-0000-0000-000000000001";
const REJECTED_PASS: &str = "wrong-console-pass";
const CONSOLE_USER: &str = "console-forms";
const OK_BODY: &str = "<?xml version=\"1.0\"?><ConsoleSession><Result>OK</Result></ConsoleSession>";

pub fn run() -> ExitCode {
    match drive() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("console-forms: {error}");
            ExitCode::from(1)
        }
    }
}

fn drive() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        drive_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok("console-forms: skipped (the live region harness runs on Linux)".to_owned())
    }
}

#[cfg(target_os = "linux")]
fn drive_linux() -> Result<String, String> {
    let estate = crate::live::generate_password()?;
    let console_pass = crate::live::generate_password()?;
    let secrets = vec![
        estate.clone(),
        console_pass.clone(),
        REJECTED_PASS.to_owned(),
    ];
    speak_both(&estate, &console_pass).map_err(|error| scrub(&error, &secrets))
}

#[cfg(target_os = "linux")]
fn speak_both(estate: &str, console_pass: &str) -> Result<String, String> {
    let _claim = crate::grid_state::claim()?;
    let publish = crate::live::publish_dir();
    if !publish.join("OpenSim.Server.RegionServer").is_file() {
        crate::live::ensure_region_built()?;
    }
    if !publish.join("OpenSim.Server.RegionServer").is_file() {
        return Err("run cargo xtask live build first".to_owned());
    }
    prepare_publish(&publish)?;
    let ferroan = crate::live::build_binary("ferroan-region")?;
    let http_port = crate::ports::alloc(crate::ports::Slot::ConsolePublic)?;
    let private_port = crate::ports::alloc(crate::ports::Slot::ConsolePrivate)?;
    let root = if crate::ports::isolated() {
        crate::ports::run_root().join("console-forms")
    } else {
        crate::repo_root().join(".scratch/console-forms")
    };
    let regions = root.join("Regions");
    let config = root.join("config");
    std::fs::create_dir_all(&regions)
        .map_err(|error| format!("could not create {}: {error}", regions.display()))?;
    std::fs::create_dir_all(&config)
        .map_err(|error| format!("could not create {}: {error}", config.display()))?;
    let server_ini = region_ini(
        &regions,
        http_port,
        private_port,
        estate,
        console_pass,
        None,
    )?;
    crate::live::write_secret(&config.join("RegionServer.ini"), &server_ini)?;
    std::fs::write(regions.join("Regions.ini"), regions_ini(http_port))
        .map_err(|error| format!("could not write Regions.ini: {error}"))?;
    let binary = publish.join("OpenSim.Server.RegionServer");
    let csharp_log = root.join("csharp.log");
    let mut csharp = start_region(
        &binary,
        &publish,
        &config,
        &csharp_log,
        &["--console", "rest"],
    )?;
    let csharp_forms = match finish_and_speak(
        &mut csharp,
        http_port,
        &csharp_log,
        console_pass,
        &[estate, console_pass, REJECTED_PASS],
        true,
    ) {
        Ok(forms) => forms,
        Err(error) => {
            stop_region(&mut csharp);
            return Err(error);
        }
    };
    stop_region(&mut csharp);
    clear_sqlite(&publish)?;
    // Ferroan registers with a grid service before it serves the console.
    // The stub answers that one call. C# keeps its in-process grid service.
    let stub = start_grid_stub()?;
    let server_ini = region_ini(
        &regions,
        http_port,
        private_port,
        estate,
        console_pass,
        Some(&stub.base),
    )?;
    crate::live::write_secret(&config.join("RegionServer.ini"), &server_ini)?;
    let ferroan_log = root.join("ferroan.log");
    let mut ferroan_region = start_region(&ferroan, &publish, &config, &ferroan_log, &[])?;
    let ferroan_forms = match finish_and_speak(
        &mut ferroan_region,
        http_port,
        &ferroan_log,
        console_pass,
        &[estate, console_pass, REJECTED_PASS],
        false,
    ) {
        Ok(forms) => forms,
        Err(error) => {
            stop_region(&mut ferroan_region);
            return Err(error);
        }
    };
    stop_region(&mut ferroan_region);
    compare_forms(
        &csharp_forms,
        &ferroan_forms,
        PROMPT,
        env!("CARGO_PKG_VERSION"),
    )?;
    Ok("console-forms: prompt, three version lines and unknown-session replies matched".to_owned())
}

#[cfg(target_os = "linux")]
fn finish_and_speak(
    running: &mut Running,
    port: u16,
    log: &std::path::Path,
    console_pass: &str,
    secrets: &[&str],
    wait_for_prompt: bool,
) -> Result<Forms, String> {
    wait_log(
        &mut running.child,
        log,
        "INITIALIZATION COMPLETE FOR Ferroan Test - LOGINS ENABLED",
        crate::live::START_TIMEOUT,
        secrets,
    )?;
    if wait_for_prompt {
        wait_log(
            &mut running.child,
            log,
            "Region (Ferroan Test) #",
            std::time::Duration::from_secs(30),
            secrets,
        )?;
    }
    crate::live::require_listen(running.child.id(), std::net::Ipv4Addr::LOCALHOST, port)
        .map_err(|error| format!("{error}\n{}", crate::live::log_tail_secrets(log, secrets)))?;
    speak(port, console_pass)
        .map_err(|error| format!("{error}\n{}", crate::live::log_tail_secrets(log, secrets)))
}

#[cfg(target_os = "linux")]
fn prepare_publish(publish: &std::path::Path) -> Result<(), String> {
    for name in ["StandaloneCommon.ini", "FlotsamCache.ini"] {
        crate::live::copy_example(publish, name)?;
    }
    if publish
        .join("config-include/GridCommon.ini.example")
        .is_file()
    {
        crate::live::copy_example(publish, "GridCommon.ini")?;
    }
    clear_sqlite(publish)
}

#[cfg(target_os = "linux")]
fn clear_sqlite(publish: &std::path::Path) -> Result<(), String> {
    let entries = std::fs::read_dir(publish)
        .map_err(|error| format!("could not read {}: {error}", publish.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not read {}: {error}", publish.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if crate::live::is_sqlite_file(&name) {
            std::fs::remove_file(entry.path())
                .map_err(|error| format!("could not remove {}: {error}", entry.path().display()))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn region_ini(
    regions: &std::path::Path,
    public_port: u16,
    private_port: u16,
    estate: &str,
    console_pass: &str,
    grid_base: Option<&str>,
) -> Result<String, String> {
    let dir = regions.to_str().ok_or("regions directory is not utf-8")?;
    if dir.contains(['"', '\n', '\r'])
        || estate.contains(['"', '\n', '\r'])
        || console_pass.contains(['"', '\n', '\r'])
        || grid_base.is_some_and(|value| value.contains(['"', '\n', '\r']))
    {
        return Err("a configuration value contains a quote or a line break".to_owned());
    }
    let services = match grid_base {
        Some(base) => format!(
            "\n[GridService]\n    GridServerURI = \"{base}\"\n\n\
             [PresenceService]\n    PresenceServerURI = \"{base}\"\n\n\
             [GridUserService]\n    GridUserServerURI = \"{base}\"\n"
        ),
        None => String::new(),
    };
    Ok(format!(
        r#"[Const]
    BaseHostname = "127.0.0.1"
    BaseURL = http://${{Const|BaseHostname}}
    PublicPort = "{public_port}"
    PrivURL = ${{Const|BaseURL}}
    PrivatePort = "{private_port}"

[Startup]
    regionload_regionsdir = "{dir}"
    ConsolePrompt = "Region (\R) "

[Network]
    http_listener_port = {public_port}
    bind_ip_address = "127.0.0.1"
    ConsoleUser = "{CONSOLE_USER}"
    ConsolePass = "{console_pass}"

[Map]
    GenerateMaptiles = false

[Estates]
    DefaultEstateName = "Ferroan Test Estate"
    DefaultEstateOwnerName = "Test User"
    DefaultEstateOwnerEMail = "test@example.invalid"
    DefaultEstateOwnerUUID = "{owner}"
    DefaultEstateOwnerPassword = "{estate}"

[Architecture]
    Include-Architecture = "config-include/Standalone.ini"

[ClientStack.LindenUDP]
    ServiceProbe = true
{services}"#,
        owner = crate::live::ESTATE_OWNER,
    ))
}

/// Answers Ferroan's startup `POST /grid` with `Result>Success`. Presence and
/// grid-user calls get the same document; a refusal there is nonfatal.
#[cfg(target_os = "linux")]
struct GridStub {
    base: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
impl Drop for GridStub {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(target_os = "linux")]
fn start_grid_stub() -> Result<GridStub, String> {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    const BODY: &str =
        "<?xml version=\"1.0\"?>\n<ServerResponse>\n  <Result>Success</Result>\n</ServerResponse>";
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("could not bind the grid stub: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("could not set the grid stub nonblocking: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("could not read the grid stub port: {error}"))?
        .port();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut bytes = [0_u8; 4096];
                    let _ = stream.read(&mut bytes);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                        BODY.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    Ok(GridStub {
        base: format!("http://127.0.0.1:{port}"),
        stop,
        thread: Some(thread),
    })
}

#[cfg(target_os = "linux")]
fn regions_ini(public_port: u16) -> String {
    format!(
        r#"[Ferroan Test]
RegionUUID = {uuid}
Location = 1000,1000
InternalAddress = 127.0.0.1
InternalPort = {public_port}
AllowAlternatePorts = False
ExternalHostName = 127.0.0.1
"#,
        uuid = crate::live::REGION_UUID,
    )
}

#[cfg(target_os = "linux")]
struct Running {
    child: std::process::Child,
    tracked: Option<crate::grid_state::Tracked>,
    stopped: bool,
}

#[cfg(target_os = "linux")]
impl Drop for Running {
    fn drop(&mut self) {
        stop_region(self);
    }
}

#[cfg(target_os = "linux")]
fn start_region(
    binary: &std::path::Path,
    publish: &std::path::Path,
    config: &std::path::Path,
    log: &std::path::Path,
    extra: &[&str],
) -> Result<Running, String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let log_file = std::fs::File::create(log)
        .map_err(|error| format!("could not create {}: {error}", log.display()))?;
    let err_file = log_file
        .try_clone()
        .map_err(|error| format!("could not open {}: {error}", log.display()))?;
    let config_arg = config.to_str().ok_or("region config path is not UTF-8")?;
    let mut args = vec!["--inidirectory", config_arg];
    args.extend_from_slice(extra);
    let mut command = Command::new(binary);
    command
        .current_dir(publish)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(err_file))
        .args(&args)
        .process_group(0);
    let (child, tracked) = crate::live::spawn_tracked(&mut command, binary, &args, publish)?;
    Ok(Running {
        child,
        tracked: Some(tracked),
        stopped: false,
    })
}

#[cfg(target_os = "linux")]
fn stop_region(running: &mut Running) {
    if running.stopped {
        return;
    }
    running.stopped = true;
    let pid = running.child.id();
    if matches!(running.child.try_wait(), Ok(None)) {
        let _ = crate::signal::signal_group(crate::signal::Signal::Term, pid);
        let started = std::time::Instant::now();
        while matches!(running.child.try_wait(), Ok(None))
            && started.elapsed() < std::time::Duration::from_secs(20)
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if matches!(running.child.try_wait(), Ok(None)) {
            let _ = crate::signal::signal_group(crate::signal::Signal::Kill, pid);
            let _ = running.child.wait();
        }
    }
    if let Some(tracked) = running.tracked.take() {
        let _ = tracked.forget();
    }
}

#[cfg(target_os = "linux")]
fn wait_log(
    child: &mut std::process::Child,
    log: &std::path::Path,
    needle: &str,
    timeout: std::time::Duration,
    secrets: &[&str],
) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        if crate::live::log_contains(log, needle)? {
            return Ok(());
        }
        match child
            .try_wait()
            .map_err(|error| format!("could not wait for the region: {error}"))?
        {
            Some(status) => {
                return Err(format!(
                    "the region exited ({status}) before {needle}\n{}",
                    crate::live::log_tail_secrets(log, secrets)
                ));
            }
            None if started.elapsed() >= timeout => {
                return Err(format!(
                    "timed out waiting for {needle}\n{}",
                    crate::live::log_tail_secrets(log, secrets)
                ));
            }
            None => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
}

#[cfg(target_os = "linux")]
fn speak(port: u16, console_pass: &str) -> Result<Forms, String> {
    let wrong_password = post_form(
        port,
        "/StartSession/",
        &form(&[("USER", CONSOLE_USER), ("PASS", REJECTED_PASS)]),
    )?;
    let start = post_form(
        port,
        "/StartSession/",
        &form(&[("USER", CONSOLE_USER), ("PASS", console_pass)]),
    )?;
    let session = between(&start.body, "<SessionID>", "</SessionID>")?;
    // Scrollback from the new session is not compared.
    let _drain = post_form(port, &format!("/ReadResponses/{session}/"), "")?;
    let command = post_form(
        port,
        "/SessionCommand/",
        &form(&[("ID", session), ("COMMAND", "show version")]),
    )?;
    let version = read_until_version(port, session)?;
    let close_known = post_form(port, "/CloseSession/", &form(&[("ID", session)]))?;
    let command_unknown = post_form(
        port,
        "/SessionCommand/",
        &form(&[("ID", UNKNOWN_ID), ("COMMAND", "show version")]),
    )?;
    let close_unknown = post_form(port, "/CloseSession/", &form(&[("ID", UNKNOWN_ID)]))?;
    let read_unknown = post_form(port, &format!("/ReadResponses/{UNKNOWN_ID}/"), "")?;
    let forms = Forms {
        wrong_password,
        start,
        command,
        version,
        close_known,
        command_unknown,
        close_unknown,
        read_unknown,
    };
    refuse_secrets(&forms, console_pass)?;
    Ok(forms)
}

#[cfg(target_os = "linux")]
fn read_until_version(port: u16, session: &str) -> Result<Reply, String> {
    let started = std::time::Instant::now();
    let path = format!("/ReadResponses/{session}/");
    let mut last = String::new();
    while started.elapsed() < std::time::Duration::from_secs(90) {
        let reply = post_form(port, &path, "")?;
        if reply.body.contains("Input=\"true\"") {
            return Ok(reply);
        }
        last = preview(&reply.body);
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(format!("show version did not arrive\n{last}"))
}

#[cfg(target_os = "linux")]
fn refuse_secrets(forms: &Forms, console_pass: &str) -> Result<(), String> {
    for (name, reply) in forms.named() {
        if reply.body.contains(console_pass) || reply.body.contains(REJECTED_PASS) {
            return Err(format!("{name} carries a console password"));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn post_form(port: u16, path: &str, body: &str) -> Result<Reply, String> {
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    use std::time::Duration;

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|error| format!("could not connect to 127.0.0.1:{port}: {error}"))?;
    stream
        .set_nodelay(true)
        .map_err(|error| format!("could not set TCP_NODELAY: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(40)))
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
        .map_err(|error| format!("could not write {path}: {error}"))?;
    let mut bytes = Vec::new();
    let mut tmp = [0_u8; 8192];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&tmp[..n]);
                if bytes.len() > 2_000_000 {
                    return Err(format!("{path} response exceeds 2 MiB"));
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(error) => return Err(format!("could not read {path}: {error}")),
        }
    }
    parse_reply(&bytes).map_err(|error| format!("{path}: {error}"))
}

#[cfg(target_os = "linux")]
fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(target_os = "linux")]
fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

struct Reply {
    status: u16,
    media: String,
    body: String,
}

struct Forms {
    wrong_password: Reply,
    start: Reply,
    command: Reply,
    version: Reply,
    close_known: Reply,
    command_unknown: Reply,
    close_unknown: Reply,
    read_unknown: Reply,
}

impl Forms {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn named(&self) -> [(&str, &Reply); 8] {
        [
            ("wrong password", &self.wrong_password),
            ("StartSession", &self.start),
            ("SessionCommand", &self.command),
            ("version", &self.version),
            ("CloseSession", &self.close_known),
            ("unknown SessionCommand", &self.command_unknown),
            ("unknown CloseSession", &self.close_unknown),
            ("unknown ReadResponses", &self.read_unknown),
        ]
    }
}

/// Wildcards, each with its reason:
/// - session id: a new UUID per session (the element and UUID shape are compared)
/// - `HelpTree` children: Ferroan has not ported the region command catalogue;
///   the element and each `Command`'s schema are compared
/// - version text: it is Ferroan's, and must contain `Ferroan` and the version
/// - line number: the scrollback depth differs
fn compare_forms(
    csharp: &Forms,
    ferroan: &Forms,
    prompt: &str,
    ferroan_version: &str,
) -> Result<(), String> {
    same_exact(
        &csharp.wrong_password,
        &ferroan.wrong_password,
        "wrong password",
        401,
        "text/plain",
        "",
    )?;
    compare_start(&csharp.start, &ferroan.start, prompt)?;
    same_exact(
        &csharp.command,
        &ferroan.command,
        "SessionCommand",
        200,
        "text/xml",
        OK_BODY,
    )?;
    compare_version(&csharp.version, &ferroan.version, prompt, ferroan_version)?;
    same_exact(
        &csharp.close_known,
        &ferroan.close_known,
        "CloseSession",
        200,
        "text/xml",
        OK_BODY,
    )?;
    same_exact(
        &csharp.command_unknown,
        &ferroan.command_unknown,
        "unknown SessionCommand",
        404,
        "text/plain",
        "",
    )?;
    // Since Pin8 both unknown-session forms are refused (TB31, Tranquillity #528).
    same_exact(
        &csharp.close_unknown,
        &ferroan.close_unknown,
        "unknown CloseSession",
        404,
        "text/plain",
        "",
    )?;
    same_exact(
        &csharp.read_unknown,
        &ferroan.read_unknown,
        "unknown ReadResponses",
        404,
        "text/plain",
        "",
    )?;
    Ok(())
}

fn same_exact(
    csharp: &Reply,
    ferroan: &Reply,
    what: &str,
    status: u16,
    media: &str,
    body: &str,
) -> Result<(), String> {
    for (name, reply) in [("C#", csharp), ("Ferroan", ferroan)] {
        if reply.status != status || reply.media != media || reply.body.trim() != body {
            return Err(format!(
                "{what}: {name} is {} {} {:?}",
                reply.status,
                reply.media,
                preview(&reply.body)
            ));
        }
    }
    Ok(())
}

fn compare_start(csharp: &Reply, ferroan: &Reply, prompt: &str) -> Result<(), String> {
    for (name, reply) in [("C#", csharp), ("Ferroan", ferroan)] {
        if reply.status != 200 || reply.media != "text/xml" {
            return Err(format!(
                "StartSession: {name} is {} {}",
                reply.status, reply.media
            ));
        }
        help_schema(&reply.body)?;
    }
    let csharp_prompt = between(&csharp.body, "<Prompt>", "</Prompt>")?;
    let ferroan_prompt = between(&ferroan.body, "<Prompt>", "</Prompt>")?;
    if csharp_prompt != prompt || ferroan_prompt != prompt {
        return Err(format!(
            "StartSession prompt is {csharp_prompt:?} and {ferroan_prompt:?}; expected {prompt:?}"
        ));
    }
    let csharp_id = between(&csharp.body, "<SessionID>", "</SessionID>")?;
    let ferroan_id = between(&ferroan.body, "<SessionID>", "</SessionID>")?;
    if !uuid_shape(csharp_id) || !uuid_shape(ferroan_id) {
        return Err("StartSession SessionID is not a UUID".to_owned());
    }
    let csharp_order = child_order(&csharp.body)?;
    let ferroan_order = child_order(&ferroan.body)?;
    if csharp_order != ["SessionID", "Prompt", "HelpTree"]
        || ferroan_order != ["SessionID", "Prompt", "HelpTree"]
    {
        return Err(format!(
            "StartSession children are {csharp_order:?} and {ferroan_order:?}"
        ));
    }
    Ok(())
}

fn compare_version(
    csharp: &Reply,
    ferroan: &Reply,
    prompt: &str,
    ferroan_version: &str,
) -> Result<(), String> {
    for (name, reply) in [("C#", csharp), ("Ferroan", ferroan)] {
        if reply.status != 200 || reply.media != "application/xml" {
            return Err(format!(
                "version poll: {name} is {} {}",
                reply.status, reply.media
            ));
        }
    }
    let csharp_lines = version_lines(&csharp.body)?;
    let ferroan_lines = version_lines(&ferroan.body)?;
    if csharp_lines[0].text != "show version" || ferroan_lines[0].text != "show version" {
        return Err(format!(
            "version echo is {:?} and {:?}",
            csharp_lines[0].text, ferroan_lines[0].text
        ));
    }
    if csharp_lines[2].text != prompt || ferroan_lines[2].text != prompt {
        return Err(format!(
            "version prompt line is {:?} and {:?}",
            csharp_lines[2].text, ferroan_lines[2].text
        ));
    }
    if !ferroan_lines[1].text.contains("Ferroan")
        || !ferroan_lines[1].text.contains(ferroan_version)
    {
        return Err(format!(
            "Ferroan version text {:?} does not contain Ferroan and {ferroan_version}",
            ferroan_lines[1].text
        ));
    }
    // The version text and the line number are the wildcards. Flags are not.
    for (index, expected) in [
        ("", "false", "false", "true"),
        ("", "false", "false", "false"),
        ("", "true", "true", "false"),
    ]
    .into_iter()
    .enumerate()
    {
        for (name, lines) in [("C#", &csharp_lines), ("Ferroan", &ferroan_lines)] {
            let line = &lines[index];
            if line.level != expected.0
                || line.prompt != expected.1
                || line.command != expected.2
                || line.input != expected.3
                || !attributes_in_order(&line.open)
                || line.number.is_empty()
            {
                return Err(format!(
                    "version line {index} on {name} is Number={:?} Level={:?} Prompt={} Command={} Input={}",
                    line.number, line.level, line.prompt, line.command, line.input
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ConsoleLine {
    number: String,
    level: String,
    prompt: String,
    command: String,
    input: String,
    text: String,
    open: String,
}

fn version_lines(body: &str) -> Result<[ConsoleLine; 3], String> {
    let lines = console_lines(body)?;
    let Some(index) = lines
        .iter()
        .position(|line| line.input == "true" && line.text == "show version")
    else {
        return Err(format!(
            "version poll has no show version echo ({})",
            preview(body)
        ));
    };
    let Some(window) = lines.get(index..index + 3) else {
        return Err("version poll ends before the prompt line".to_owned());
    };
    Ok([window[0].clone(), window[1].clone(), window[2].clone()])
}

fn console_lines(body: &str) -> Result<Vec<ConsoleLine>, String> {
    let mut lines = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("<Line ") {
        let after = &rest[start..];
        let Some(end_tag) = after.find('>') else {
            return Err("a Line tag is unclosed".to_owned());
        };
        let open = &after[..=end_tag];
        let Some(close) = after[end_tag + 1..].find("</Line>") else {
            return Err("a Line element is unclosed".to_owned());
        };
        let text = &after[end_tag + 1..end_tag + 1 + close];
        lines.push(ConsoleLine {
            number: attr(open, "Number")?,
            level: attr(open, "Level")?,
            prompt: attr(open, "Prompt")?,
            command: attr(open, "Command")?,
            input: attr(open, "Input")?,
            text: xml_unescape(text),
            open: open.to_owned(),
        });
        rest = &after[end_tag + 1 + close + "</Line>".len()..];
    }
    Ok(lines)
}

fn attributes_in_order(open: &str) -> bool {
    let mut cursor = 0;
    for name in ["Number", "Level", "Prompt", "Command", "Input"] {
        let needle = format!("{name}=\"");
        let Some(found) = open[cursor..].find(&needle) else {
            return false;
        };
        cursor += found + needle.len();
    }
    true
}

fn attr(open: &str, name: &str) -> Result<String, String> {
    let needle = format!("{name}=\"");
    let Some(start) = open.find(&needle) else {
        return Err(format!("Line is missing {name}"));
    };
    let rest = &open[start + needle.len()..];
    let Some(end) = rest.find('"') else {
        return Err(format!("Line {name} is unclosed"));
    };
    Ok(xml_unescape(&rest[..end]))
}

fn child_order(body: &str) -> Result<[&str; 3], String> {
    let session = body
        .find("<SessionID>")
        .ok_or("StartSession has no SessionID")?;
    let prompt = body.find("<Prompt>").ok_or("StartSession has no Prompt")?;
    let help = body
        .find("<HelpTree>")
        .ok_or("StartSession has no HelpTree")?;
    if session < prompt && prompt < help {
        Ok(["SessionID", "Prompt", "HelpTree"])
    } else {
        Err("StartSession children are out of order".to_owned())
    }
}

fn help_schema(body: &str) -> Result<(), String> {
    let inner = between(body, "<HelpTree>", "</HelpTree>")?;
    let mut rest = inner;
    let mut commands = 0usize;
    while let Some(start) = rest.find("<Command>") {
        let after = &rest[start + "<Command>".len()..];
        let Some(end) = after.find("</Command>") else {
            return Err("HelpTree Command is unclosed".to_owned());
        };
        let command = &after[..end];
        let mut cursor = 0;
        for name in ["Module", "Shared", "HelpText", "LongHelp", "Description"] {
            let open = format!("<{name}>");
            let close = format!("</{name}>");
            let Some(at) = command[cursor..].find(&open) else {
                return Err(format!("HelpTree Command is missing {name}"));
            };
            let after_open = cursor + at + open.len();
            let Some(close_at) = command[after_open..].find(&close) else {
                return Err(format!("HelpTree Command is missing {name}"));
            };
            cursor = after_open + close_at + close.len();
        }
        commands += 1;
        rest = &after[end + "</Command>".len()..];
    }
    if commands == 0 {
        return Err("HelpTree has no Command".to_owned());
    }
    Ok(())
}

fn between<'a>(body: &'a str, start: &str, end: &str) -> Result<&'a str, String> {
    let Some(at) = body.find(start) else {
        return Err(format!("missing {start}"));
    };
    let rest = &body[at + start.len()..];
    let Some(stop) = rest.find(end) else {
        return Err(format!("missing {end}"));
    };
    Ok(&rest[..stop])
}

fn uuid_shape(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && parts[0].len() == 8
        && parts[1].len() == 4
        && parts[2].len() == 4
        && parts[3].len() == 4
        && parts[4].len() == 12
        && parts
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn xml_unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn preview(body: &str) -> String {
    let folded =
        if let (Some(start), Some(end)) = (body.find("<HelpTree>"), body.find("</HelpTree>")) {
            format!(
                "{}<HelpTree>...</HelpTree>{}",
                &body[..start],
                &body[end + "</HelpTree>".len()..]
            )
        } else {
            body.to_owned()
        };
    let mut out: String = folded.chars().take(500).collect();
    if folded.chars().count() > 500 {
        out.push('…');
    }
    out
}

fn scrub(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_owned();
    let mut secrets = secrets.to_vec();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    for secret in secrets {
        if secret.len() >= 8 {
            out = out.replace(&secret, "[redacted]");
        }
    }
    let head: String = out.chars().take(1200).collect();
    head.replace('\n', " ")
}

#[cfg(target_os = "linux")]
fn parse_reply(bytes: &[u8]) -> Result<Reply, String> {
    let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err("the response has no header end".to_owned());
    };
    let header = String::from_utf8_lossy(&bytes[..split]);
    let body = String::from_utf8_lossy(&bytes[split + 4..]).into_owned();
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or("the response has no status")?;
    let media = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-type") {
                Some(
                    value
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase(),
                )
            } else {
                None
            }
        })
        .unwrap_or_default();
    Ok(Reply {
        status,
        media,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, media: &str, body: &str) -> Reply {
        Reply {
            status,
            media: media.to_owned(),
            body: body.to_owned(),
        }
    }

    fn sample(version: &str, number: u64, session: &str, prompt: &str) -> Forms {
        let start = format!(
            "<?xml version=\"1.0\"?><ConsoleSession><SessionID>{session}</SessionID><Prompt>{prompt}</Prompt><HelpTree><Level Name=\"show\"><Level Name=\"version\"><Command><Module>General</Module><Shared>False</Shared><HelpText>show version</HelpText><LongHelp>Show</LongHelp><Description></Description></Command></Level></Level></HelpTree></ConsoleSession>"
        );
        let lines = format!(
            "<?xml version=\"1.0\"?><ConsoleSession><Line Number=\"{number}\" Level=\"\" Prompt=\"false\" Command=\"false\" Input=\"true\">show version</Line><Line Number=\"{next}\" Level=\"\" Prompt=\"false\" Command=\"false\" Input=\"false\">{version}</Line><Line Number=\"{last}\" Level=\"\" Prompt=\"true\" Command=\"true\" Input=\"false\">{prompt}</Line></ConsoleSession>",
            next = number + 1,
            last = number + 2,
        );
        Forms {
            wrong_password: reply(401, "text/plain", ""),
            start: reply(200, "text/xml", &start),
            command: reply(200, "text/xml", OK_BODY),
            version: reply(200, "application/xml", &lines),
            close_known: reply(200, "text/xml", OK_BODY),
            command_unknown: reply(404, "text/plain", ""),
            close_unknown: reply(404, "text/plain", ""),
            read_unknown: reply(404, "text/plain", ""),
        }
    }

    #[test]
    fn the_named_wildcards_are_the_only_folds() {
        let csharp = sample(
            "Version: OpenSim-NGC Tranquillity Release 1.2.225-alpha",
            213,
            "00000000-0000-0000-0000-0000000000a1",
            PROMPT,
        );
        let mut ferroan = sample(
            "Ferroan 0.1.0 (ferroan-region)",
            4,
            "00000000-0000-0000-0000-0000000000b2",
            PROMPT,
        );
        ferroan.start.body = ferroan.start.body.replace("Name=\"show\"", "Name=\"diag\"");
        compare_forms(&csharp, &ferroan, PROMPT, "0.1.0").expect("match");

        ferroan.start.body = ferroan.start.body.replace(PROMPT, "Ferroan# ");
        assert!(compare_forms(&csharp, &ferroan, PROMPT, "0.1.0").is_err());
    }

    #[test]
    fn a_version_line_with_the_wrong_flags_fails() {
        let csharp = sample(
            "Version: C#",
            10,
            "00000000-0000-0000-0000-0000000000a1",
            PROMPT,
        );
        let mut ferroan = sample(
            "Ferroan 0.1.0 (ferroan-region)",
            1,
            "00000000-0000-0000-0000-0000000000b2",
            PROMPT,
        );
        ferroan.version.body = ferroan
            .version
            .body
            .replace("Input=\"true\"", "Input=\"false\"");
        assert!(compare_forms(&csharp, &ferroan, PROMPT, "0.1.0").is_err());
    }

    #[test]
    fn secrets_are_scrubbed() {
        let text = scrub(
            "saw word-secret-value in the log",
            &["word-secret-value".to_owned()],
        );
        assert!(!text.contains("word-secret-value"));
        assert!(text.contains("[redacted]"));
    }

    #[test]
    fn help_schema_rejects_a_command_missing_a_child() {
        let body = "<HelpTree><Command><Module>General</Module><Shared>False</Shared><HelpText>show</HelpText><LongHelp>Show</LongHelp></Command></HelpTree>";
        assert!(help_schema(body).is_err());
    }
}
