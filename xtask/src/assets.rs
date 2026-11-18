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
use std::io::Write;
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
const KELP: &str = "2a4880b6-b7a3-690a-2049-bfbe38eafb9f";

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
        start_robust(&database, port, &log)?
    } else {
        return Err(format!("unknown asset side {name}"));
    };
    let started = Instant::now();
    loop {
        if !pid_exists(server.child.id()) {
            stop_owned(&mut server);
            return Err(format!("the {name} asset service exited\n{}", tail(&log)));
        }
        if live::require_http_loopback(server.child.id(), port).is_ok() {
            break;
        }
        if started.elapsed() > Duration::from_secs(60) {
            stop_owned(&mut server);
            return Err(format!(
                "the {name} asset service did not listen on 127.0.0.1:{port}\n{}",
                tail(&log)
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
    let capture = run.join(format!("assets-{name}.cap"));
    let upstream = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let (proxy_port, proxy) = spawn_proxy(upstream, &capture)?;
    let uri = format!("http://127.0.0.1:{proxy_port}");
    println!("live: assets {name} via {uri}");
    let report = live::run_against_assets(&uri, "assets");
    drop(proxy);
    stop_owned(&mut server);
    let report = report?;
    Ok(SideReport {
        report: format!("assets {name}:\n{report}"),
        capture,
    })
}

#[cfg(target_os = "linux")]
struct Owned {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
}

#[cfg(target_os = "linux")]
fn seed_database() -> Result<PathBuf, String> {
    let run = live::scratch_run();
    fs::create_dir_all(&run)
        .map_err(|error| format!("could not create {}: {error}", run.display()))?;
    let seed = run.join("asset-seed.db");
    if asset_present(&seed)? {
        return Ok(seed);
    }
    let fixture = repo_root().join("fixtures/db/sqlite/AssetStore.db");
    fs::copy(&fixture, &seed)
        .map_err(|error| format!("could not copy {}: {error}", fixture.display()))?;
    let port = live::allocate_port()?;
    let log = run.join("asset-seed.log");
    let mut server = start_grid(&seed, port, &log, true)?;
    let started = Instant::now();
    loop {
        if asset_present(&seed)? {
            break;
        }
        if !pid_exists(server.child.id()) || started.elapsed() > Duration::from_secs(180) {
            stop_owned(&mut server);
            return Err(format!(
                "the asset loader did not store the library texture\n{}",
                tail(&log)
            ));
        }
        thread::sleep(Duration::from_millis(500));
    }
    stop_owned(&mut server);
    let _ = Command::new("sqlite3")
        .arg(&seed)
        .arg("PRAGMA wal_checkpoint(TRUNCATE);")
        .output();
    if !asset_present(&seed)? {
        return Err("the library texture disappeared after the loader stopped".to_string());
    }
    Ok(seed)
}

#[cfg(target_os = "linux")]
fn asset_present(database: &Path) -> Result<bool, String> {
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
         EnableRobustSelfsignedCertSupport = false\n\n[Network]\n    port = {port}\n\n\
         [ServiceList]\n    \
         AssetServiceConnector = \"{port}/OpenSim.Server.Handlers.dll:AssetServiceConnector\"\n\n\
         [DatabaseService]\n    StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n\n[AssetService]\n    \
         LocalServiceModule = \"OpenSim.Services.AssetService.dll:AssetService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n{loader}"
    );
    write_secret(&ini_path, &ini)?;
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
    )
}

#[cfg(target_os = "linux")]
fn start_robust(database: &Path, port: u16, log: &Path) -> Result<Owned, String> {
    let binary = robust_binary()?;
    let run = live::scratch_run();
    let empty = run.join("grid-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_path = run.join("robust.ini");
    let db = database.display();
    let ini = format!(
        "[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n[AssetService]\n    \
         LocalServiceModule = \"OpenSim.Services.AssetService.dll:AssetService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={db};Version=3;\"\n"
    );
    write_secret(&ini_path, &ini)?;
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
    )
}

#[cfg(target_os = "linux")]
fn robust_binary() -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args(["build", "-p", "ferroan-robust", "--offline"])
        .current_dir(repo_root())
        .output()
        .map_err(|error| format!("could not build ferroan-robust: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo build -p ferroan-robust failed\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let debug = repo_root().join("target/debug/ferroan-robust");
    let release = repo_root().join("target/release/ferroan-robust");
    if debug.is_file() {
        Ok(debug)
    } else if release.is_file() {
        Ok(release)
    } else {
        Err("ferroan-robust was not produced".to_string())
    }
}

#[cfg(target_os = "linux")]
fn spawn_owned(
    binary: &Path,
    args: &[&str],
    cwd: &Path,
    log: &Path,
    pipe_stdin: bool,
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
    let mut child = command
        .process_group(0)
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", binary.display()))?;
    let stdin = child.stdin.take();
    Ok(Owned { child, stdin })
}

#[cfg(target_os = "linux")]
fn spawn_proxy(upstream: SocketAddr, out: &Path) -> Result<(u16, tokio::runtime::Runtime), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start tokio: {error}"))?;
    let path = out.to_path_buf();
    let port = runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| format!("could not bind the asset proxy: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("could not read the asset proxy port: {error}"))?
            .port();
        tokio::spawn(async move {
            let _ = ferroan_capture::serve_on(listener, upstream, &path).await;
        });
        Ok::<u16, String>(port)
    })?;
    Ok((port, runtime))
}

#[cfg(target_os = "linux")]
fn stop_owned(owned: &mut Owned) {
    let pid = owned.child.id();
    drop(owned.stdin.take());
    let _ = Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .status();
    let started = Instant::now();
    while pid_exists(pid) && started.elapsed() < Duration::from_secs(20) {
        thread::sleep(Duration::from_millis(200));
    }
    if pid_exists(pid) {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .status();
    }
    let _ = owned.child.wait();
}

#[cfg(target_os = "linux")]
fn pid_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "linux")]
fn write_secret(path: &Path, text: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

#[cfg(target_os = "linux")]
fn tail(path: &Path) -> String {
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
fn first_difference(left: &str, right: &str) -> String {
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
