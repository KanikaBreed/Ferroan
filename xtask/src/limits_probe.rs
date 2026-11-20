// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Trip the POST-body, datagram and fetch-folder limits on a Ferroan region
//! and require the log line, the counter and the diagnostic dump.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use ferroan_bot::login::{LoginRequest, http_client};
use ferroan_bot::session::{Agent, Session};
use ferroan_llsd::{Llsd, to_xml};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, UdpSocket};
use uuid::Uuid;

const POST_BYTES: usize = 1024 * 1024 + 1;
const DATAGRAM_BYTES: usize = 4097;
const FOLDERS: usize = 65;
const AGENT: &str = "00000000-0000-0000-0000-000000000001";

pub async fn run(
    login_uri: &str,
    password: &str,
    home_port: u16,
    metrics: &str,
    home_log: &Path,
    private_uri: &str,
) -> Result<String, String> {
    let post = post_body(home_port).await?;
    let udp = send_datagram(home_port).await?;
    let fetch = match fetch_folders(login_uri, password, private_uri).await {
        Ok(status) => status,
        Err(error) => {
            return Err(format!("{error}; post-body status {post}; udp bytes {udp}"));
        }
    };
    let readout = wait_for_readout(home_log, metrics).await?;
    Ok(format!(
        "limits: post-body {post}, udp {udp} bytes, fetch-folders {fetch}; {readout}"
    ))
}

async fn post_body(port: u16) -> Result<u16, String> {
    let socket = TcpSocket::new_v4().map_err(|error| format!("post socket: {error}"))?;
    socket
        .bind(SocketAddr::from((Ipv4Addr::new(127, 0, 0, 1), 0)))
        .map_err(|error| format!("post bind: {error}"))?;
    let mut stream = socket
        .connect(SocketAddr::from((Ipv4Addr::new(127, 0, 0, 2), port)))
        .await
        .map_err(|error| format!("post connect: {error}"))?;
    let header = format!(
        "POST /agent/{AGENT}/ HTTP/1.1\r\nHost: 127.0.0.2:{port}\r\nContent-Type: application/json\r\nContent-Length: {POST_BYTES}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|error| format!("post write: {error}"))?;
    let body = vec![b'x'; POST_BYTES];
    stream
        .write_all(&body)
        .await
        .map_err(|error| format!("post body: {error}"))?;
    let mut buf = [0u8; 128];
    let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf))
        .await
        .map_err(|_| "post timed out".to_string())?
        .map_err(|error| format!("post read: {error}"))?;
    status_code(&buf[..read]).ok_or_else(|| "post reply had no status".to_string())
}

async fn send_datagram(port: u16) -> Result<usize, String> {
    let socket = UdpSocket::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("udp bind: {error}"))?;
    let packet = vec![0u8; DATAGRAM_BYTES];
    let sent = socket
        .send_to(&packet, SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .await
        .map_err(|error| format!("udp send: {error}"))?;
    Ok(sent)
}

async fn fetch_folders(login_uri: &str, password: &str, private_uri: &str) -> Result<u16, String> {
    let http = http_client().map_err(|error| error.to_string())?;
    let response = crate::grid::login_probe::login_when_settled(
        private_uri,
        crate::live::ESTATE_OWNER,
        &http,
        &LoginRequest {
            login_uri: login_uri.to_owned(),
            username: "Test User".to_owned(),
            password: password.to_owned(),
            start: "uri:Ferroan Test&128&128&25".to_owned(),
        },
    )
    .await?;
    let mut session = Session::connect(
        SocketAddr::from((response.sim_ip, response.sim_port)),
        Agent {
            agent_id: response.agent_id,
            session_id: response.session_id,
            circuit_code: response.circuit_code,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let outcome = async {
        session
            .run_until_ready(&http, &response.seed_capability, Duration::from_secs(30))
            .await
            .map_err(|error| error.to_string())?;
        let seed = post_seed(&http, &response.seed_capability).await?;
        let url = seed
            .get("FetchInventoryDescendents2")
            .ok_or_else(|| "FetchInventoryDescendents2 was not granted".to_string())?;
        let status = post_status(&http, url, &folder_batch(response.agent_id)).await?;
        if status != 400 {
            return Err(format!("fetch-folders status {status}"));
        }
        Ok(status)
    }
    .await;
    let logged_out = session.logout(Duration::from_secs(20)).await;
    let _ = crate::grid::login_probe::offline(private_uri).await;
    match (outcome, logged_out) {
        (Ok(status), Ok(())) => Ok(status),
        (Ok(_), Err(error)) => Err(format!("logout failed: {error}")),
        (Err(error), _) => Err(error),
    }
}

fn folder_batch(owner: Uuid) -> Llsd {
    let mut folders = Llsd::new_array();
    for index in 0..FOLDERS {
        let mut one = Llsd::new_map();
        one.insert("folder_id", Uuid::from_u128((index as u128) + 1));
        one.insert("owner_id", owner);
        one.insert("fetch_folders", true);
        one.insert("fetch_items", true);
        one.insert("sort_order", 0);
        folders.push(one);
    }
    let mut request = Llsd::new_map();
    request.insert("folders", folders);
    request
}

async fn post_seed(
    http: &reqwest::Client,
    seed: &str,
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut names = Llsd::new_array();
    names.push("FetchInventoryDescendents2");
    let bytes = post_bytes(http, seed, &names).await?;
    let body =
        ferroan_llsd::from_xml(&bytes).map_err(|_| "the seed reply is not LLSD".to_string())?;
    let Some(map) = body.as_map() else {
        return Err("the seed reply is not an LLSD map".to_string());
    };
    let mut caps = std::collections::HashMap::new();
    for (name, value) in map {
        let text = value.to_string_value();
        if !text.is_empty() {
            caps.insert(name.clone(), text);
        }
    }
    Ok(caps)
}

async fn post_status(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<u16, String> {
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/llsd+xml")
        .body(to_xml(body))
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status().as_u16();
    let _ = response
        .bytes()
        .await
        .map_err(|error| error.without_url().to_string())?;
    Ok(status)
}

async fn post_bytes(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<Vec<u8>, String> {
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/llsd+xml")
        .body(to_xml(body))
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| error.without_url().to_string())?
        .to_vec();
    if !status.is_success() {
        return Err(format!("seed status {}", status.as_u16()));
    }
    Ok(bytes)
}

async fn wait_for_readout(log: &Path, metrics: &str) -> Result<String, String> {
    let mut log_text = String::new();
    for _ in 0..8 {
        log_text = fs_read(log).unwrap_or_default();
        if log_text.contains("limit http.post-body")
            && log_text.contains("limit udp.datagram")
            && log_text.contains("limit inventory.fetch-folders")
        {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    for id in [
        "limit http.post-body",
        "limit udp.datagram",
        "limit inventory.fetch-folders",
    ] {
        if !log_text.contains(id) {
            return Err(format!("region log is missing {id}"));
        }
    }
    let series = get_text(&format!("{}/metrics", metrics.trim_end_matches('/'))).await?;
    for needle in [
        "ferroan_limit_hits_total{limit=\"http.post-body\",action=\"refuse\"}",
        "ferroan_limit_hits_total{limit=\"udp.datagram\",action=\"refuse\"}",
        "ferroan_limit_hits_total{limit=\"inventory.fetch-folders\",action=\"refuse\"}",
    ] {
        if !series.contains(needle) {
            return Err(format!("metrics missing {needle}"));
        }
    }
    let dump = get_text(&format!("{}/diag", metrics.trim_end_matches('/'))).await?;
    for id in ["http.post-body", "udp.datagram", "inventory.fetch-folders"] {
        if !dump.contains(id) {
            return Err(format!("diag dump missing {id}"));
        }
    }
    let dir = crate::live::live_root().join("diag");
    std::fs::create_dir_all(&dir).map_err(|error| format!("diag dir: {error}"))?;
    let path = dir.join("limits.txt");
    std::fs::write(&path, &dump).map_err(|error| format!("diag write: {error}"))?;
    Ok(format!("log, metrics and {}", path.display()))
}

fn fs_read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

async fn get_text(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.without_url().to_string())?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if !status.is_success() {
        return Err(format!("GET {url} status {}", status.as_u16()));
    }
    Ok(body)
}

fn status_code(bytes: &[u8]) -> Option<u16> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut words = text.split_whitespace();
    let _http = words.next()?;
    words.next()?.parse().ok()
}
