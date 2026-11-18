//
// Copyright (c) 2006-2016, openmetaverse.co
// All rights reserved.
//
// - Redistribution and use in source and binary forms, with or without
//   modification, are permitted provided that the following conditions are met:
//
// - Redistributions of source code must retain the above copyright notice, this
//   list of conditions and the following disclaimer.
// - Neither the name of the openmetaverse.co nor the names
//   of its contributors may be used to endorse or promote products derived from
//   this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
//

// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// External process/API integration for D17, written for Ferroan. Ferrobot is
// neither a dependency nor a source of copied code. Its pinned README and
// HTTP API contract specify the command signature and response envelope.
// The companion IM packet follows OpenMetaverse/AgentManager.cs InstantMessage,
// OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.

#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::io::Read;
use std::net::SocketAddr;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ferroan_bot::{Agent, LoginRequest, Session, scenario::Harness};
use ferroan_capture::{Kind, Record};
use ferroan_protocol::msgs::{
    AgentMovementComplete, ChatFromSimulator, ChatFromViewer, CompleteAgentMovement,
    CompletePingCheck, ImprovedInstantMessage, LogoutReply, LogoutRequest, RegionHandshake,
    RegionHandshakeReply, StartPingCheck, UseCircuitCode,
};
use ferroan_protocol::{Msg, parse_packet};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use uuid::Uuid;

const MAX_LOG: usize = 4 * 1024 * 1024;
const MAX_CAPTURE: usize = 32 * 1024 * 1024;
const MAX_API_BODY: usize = 256 * 1024;
const CONNECTED: Duration = Duration::from_secs(120);
const STEP: Duration = Duration::from_millis(100);

pub(crate) struct Config {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub region_name: String,
    pub ferroan: bool,
    pub udp_capture: PathBuf,
    pub http_captures: Vec<PathBuf>,
}

struct Process {
    child: Child,
    tracked: crate::grid_state::Tracked,
    log: PathBuf,
}

impl Process {
    fn running(&mut self) -> Result<(), String> {
        if self
            .child
            .try_wait()
            .map_err(|_| "could not inspect Ferrobot process")?
            .is_some()
        {
            return Err("Ferrobot exited before the check completed".into());
        }
        if fs::metadata(&self.log)
            .map_err(|_| "could not inspect Ferrobot log")?
            .len()
            > MAX_LOG as u64
        {
            return Err("Ferrobot log exceeded the harness limit".into());
        }
        Ok(())
    }

    async fn terminate(&mut self) -> Result<(), String> {
        self.running()?;
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| "could not send SIGTERM to Ferrobot")?;
        if !status.success() {
            return Err("SIGTERM to Ferrobot failed".into());
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|_| "could not reap Ferrobot")?
            {
                self.tracked.forget()?;
                return if status.success() {
                    Ok(())
                } else {
                    Err("Ferrobot exited unsuccessfully after SIGTERM".into())
                };
            }
            if Instant::now() >= deadline {
                return Err("Ferrobot did not exit after SIGTERM".into());
            }
            tokio::time::sleep(STEP).await;
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Only this owned, unreaped Child is signalled. It cannot be a reused PID.
        let _ = self.child.kill();
        if self.child.wait().is_ok() {
            let _ = self.tracked.forget();
        }
    }
}

struct Api {
    http: reqwest::Client,
    url: String,
    secret: String,
}

impl Api {
    async fn command(&self, command: &str, args: &[&str]) -> Result<String, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock precedes the Unix epoch")?
            .as_secs()
            .to_string();
        let joined = args.join("~#~");
        let signing = signature(command, &joined, &now, &self.secret);
        let mut form = ferroan_http::FieldMap::new();
        for (name, value) in [
            ("commandName", command),
            ("args", &joined),
            ("unixtime", &now),
            ("signing", &signing),
        ] {
            form.insert(name.into(), ferroan_http::Field::Str(value.into()));
        }
        let body = ferroan_http::build_query_string(&form)
            .map_err(|_| "could not encode Ferrobot command")?;
        let mut reply = self
            .http
            .post(&self.url)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|_| "Ferrobot API request failed")?;
        if !reply.status().is_success() {
            return Err("Ferrobot API returned an HTTP error".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = reply
            .chunk()
            .await
            .map_err(|_| "could not read Ferrobot API response")?
        {
            if bytes.len() + chunk.len() > MAX_API_BODY {
                return Err("Ferrobot API response exceeded the limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        parse_reply(&bytes, command)
    }
}

fn signature(command: &str, arguments: &str, time: &str, secret: &str) -> String {
    let mut digest = Sha1::new();
    for part in [command, arguments, time, secret] {
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn parse_reply(bytes: &[u8], command: &str) -> Result<String, String> {
    if bytes == b"No bot connected yet" {
        return Err("Ferrobot API reports no bot connected yet".into());
    }
    let outer: Value = serde_json::from_slice(bytes)
        .map_err(|_| "Ferrobot API refused the command or returned invalid JSON")?;
    if outer.get("Key").and_then(Value::as_bool) != Some(true) {
        return Err("Ferrobot did not run the command".into());
    }
    let value = outer
        .get("Value")
        .and_then(Value::as_str)
        .ok_or("Ferrobot API envelope has no Value")?;
    let inner: Value =
        serde_json::from_str(value).map_err(|_| "Ferrobot command reply is invalid JSON")?;
    if inner.get("status").and_then(Value::as_bool) != Some(true)
        || inner.get("command").and_then(Value::as_str) != Some(command)
    {
        return Err("Ferrobot command reported failure".into());
    }
    inner
        .get("reply")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "Ferrobot command has no reply".into())
}

fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|_| "could not open harness evidence")?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read harness evidence")?;
    if bytes.len() > limit {
        return Err("harness evidence exceeded its size limit".into());
    }
    Ok(bytes)
}

fn api_port(log: &[u8]) -> Option<u16> {
    String::from_utf8_lossy(log).lines().find_map(|line| {
        let suffix = line.split_once("HTTP API on port ")?.1;
        let (port, address) = suffix.split_once(' ')?;
        if address.trim() != "(127.0.0.1)" {
            return None;
        }
        port.parse().ok().filter(|port| *port != 0)
    })
}

fn start(config: &Config, harness: &Harness, secret: &str) -> Result<Process, String> {
    fs::create_dir_all(&config.root).map_err(|_| "could not create Ferrobot run directory")?;
    let environment = json!({
        "FERRO_USERNAME": format!("{} {}", harness.owner.first_name, harness.owner.last_name),
        "FERRO_PASSWORD": harness.owner.password,
        "FERRO_LOGIN_URI": harness.login_uri,
        "FERRO_START": format!("{}/128/128/25", config.region_name),
        "FERRO_SHARED_SECRET": secret,
        "FERRO_HTTP": "true", "FERRO_HTTP_BIND": "127.0.0.1", "FERRO_HTTP_PORT": "0",
        "FERRO_RELOG": "false", "FERRO_IM_COMMANDS": "false",
        "FERRO_HEALTH_FILE": config.root.join("health"),
        "RUST_LOG": "info",
    });
    let env_path = config.root.join("environment.json");
    crate::live::write_secret(&env_path, &environment.to_string())?;
    let saved: Value = serde_json::from_slice(&read_limited(&env_path, 16 * 1024)?)
        .map_err(|_| "could not read generated Ferrobot environment")?;
    let log = config.root.join("ferrobot.log");
    crate::live::write_secret(&log, "")?;
    let output = File::options()
        .append(true)
        .open(&log)
        .map_err(|_| "could not open Ferrobot log")?;
    let binary = fs::canonicalize(&config.executable)
        .map_err(|_| "could not resolve Ferrobot executable")?;
    let cwd =
        fs::canonicalize(&config.root).map_err(|_| "could not resolve Ferrobot run directory")?;
    let launch = crate::grid_state::prepare_spawn(&binary, &[], &cwd)?;
    let mut command = Command::new(&binary);
    command
        .env_clear()
        .current_dir(&cwd)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(
            output
                .try_clone()
                .map_err(|_| "could not clone Ferrobot log")?,
        )
        .stderr(output);
    for (name, value) in saved
        .as_object()
        .ok_or("invalid generated Ferrobot environment")?
    {
        command.env(
            name,
            value.as_str().ok_or("invalid generated Ferrobot setting")?,
        );
    }
    launch.configure(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            launch.cancel()?;
            return Err("could not start Ferrobot".into());
        }
    };
    let tracked = match launch.record(&child) {
        Ok(tracked) => tracked,
        Err(error) => {
            let _ = child.kill();
            if child.wait().is_ok() {
                let _ = launch.cancel();
            }
            return Err(error);
        }
    };
    Ok(Process {
        child,
        tracked,
        log,
    })
}

pub(crate) async fn run(harness: &Harness, config: &Config) -> Result<String, String> {
    let secret = crate::live::generate_password()?;
    let mut process = start(config, harness, &secret)?;
    let result = checks(harness, config, &secret, &mut process).await;
    result.map_err(|error| {
        let tail = read_limited(&process.log, MAX_LOG)
            .map(|bytes| {
                let clean = crate::live::redact_secrets(
                    &String::from_utf8_lossy(&bytes),
                    &[&secret, &harness.owner.password, &harness.chat.password],
                );
                let mut lines: Vec<_> = clean.lines().rev().take(16).collect();
                lines.reverse();
                let tail = lines.join("\n");
                let start = tail
                    .char_indices()
                    .rev()
                    .nth(4095)
                    .map_or(0, |(index, _)| index);
                tail[start..].to_owned()
            })
            .unwrap_or_else(|_| "Ferrobot log unavailable or over limit".into());
        format!("{error}\n{tail}")
    })
}

async fn checks(
    harness: &Harness,
    config: &Config,
    secret: &str,
    process: &mut Process,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(45);
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| "could not create Ferrobot API client")?;
    let mut last_status = "Ferrobot API listener has not appeared".to_owned();
    let api = loop {
        process.running()?;
        if let Some(port) = api_port(&read_limited(&process.log, MAX_LOG)?) {
            let api = Api {
                http: http.clone(),
                url: format!("http://127.0.0.1:{port}/api/Run"),
                secret: secret.into(),
            };
            match api.command("SimName", &[]).await {
                Ok(name) if name == config.region_name => break api,
                Ok(_) => last_status = "Ferrobot reports a different region".into(),
                Err(error) => last_status = error,
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Ferrobot did not log into the expected region: {last_status}"
            ));
        }
        tokio::time::sleep(STEP).await;
    };
    let expected =
        Uuid::parse_str(crate::live::ESTATE_OWNER).map_err(|_| "invalid harness owner UUID")?;
    let agent = loop {
        if let Ok(agent) = records(&config.udp_capture)
            .and_then(|records| login_evidence(&records, expected, harness.region_id))
        {
            break agent;
        }
        process.running()?;
        if Instant::now() >= deadline {
            return Err("capture has no completed Ferrobot region arrival".into());
        }
        tokio::time::sleep(STEP).await;
    };
    let mut lines = vec!["ferrobot login: pass".to_owned()];
    println!("ferrobot: checking a 120-second connected session");
    let until = Instant::now() + CONNECTED;
    while Instant::now() < until {
        process.running()?;
        if api.command("SimName", &[]).await? != config.region_name {
            return Err("Ferrobot changed region or disconnected during the hold".into());
        }
        tokio::time::sleep(
            Duration::from_secs(2).min(until.saturating_duration_since(Instant::now())),
        )
        .await;
    }
    process.running()?;
    connected_evidence(config)?;
    lines.push("ferrobot connected: pass (120 s; pings and event-queue polls)".into());
    if config.ferroan {
        lines.push("ferrobot local chat: pending (P2.0g)".into());
        lines.push("ferrobot IM: pending (region IM support)".into());
    } else {
        companion(harness, &api, agent.agent_id, &mut lines).await?;
    }
    process.terminate().await?;
    logout_evidence(&records(&config.udp_capture)?, agent)?;
    lines.push("ferrobot logout: pass (SIGTERM; LogoutReply)".into());
    Ok(lines.join("\n"))
}

fn records(path: &Path) -> Result<Vec<Record>, String> {
    ferroan_capture::read_complete_prefix(&read_limited(path, MAX_CAPTURE)?)
        .map_err(|_| "invalid Ferrobot traffic capture".into())
}

fn login_evidence(records: &[Record], expected: Uuid, region: Uuid) -> Result<Agent, String> {
    let mut agent = None;
    let mut movement = false;
    let mut handshake = false;
    let mut handshake_reply = false;
    let mut arrival = false;
    for record in records
        .iter()
        .filter(|record| matches!(record.kind, Kind::UdpIn | Kind::UdpOut))
    {
        let packet = parse_packet(&record.bytes).map_err(|_| "invalid captured UDP packet")?;
        if record.kind == Kind::UdpIn && packet.id == UseCircuitCode::ID {
            let message = packet
                .decode::<UseCircuitCode>()
                .map_err(|_| "invalid captured UseCircuitCode")?;
            if message.circuit_code.id == expected {
                let opened = Agent {
                    agent_id: expected,
                    session_id: message.circuit_code.session_id,
                    circuit_code: message.circuit_code.code,
                };
                if agent != Some(opened) {
                    movement = false;
                    handshake = false;
                    handshake_reply = false;
                    arrival = false;
                }
                agent = Some(opened);
            }
        }
        let Some(agent) = agent else {
            continue;
        };
        if record.kind == Kind::UdpIn && packet.id == CompleteAgentMovement::ID {
            let value = packet
                .decode::<CompleteAgentMovement>()
                .map_err(|_| "invalid captured movement request")?;
            movement |= value.agent_data.agent_id == agent.agent_id
                && value.agent_data.session_id == agent.session_id
                && value.agent_data.circuit_code == agent.circuit_code;
        }
        if record.kind == Kind::UdpOut && packet.id == RegionHandshake::ID {
            let value = packet
                .decode::<RegionHandshake>()
                .map_err(|_| "invalid captured region handshake")?;
            handshake |= value.region_info2.region_id == region;
        }
        if record.kind == Kind::UdpIn && packet.id == RegionHandshakeReply::ID {
            let message = packet
                .decode::<RegionHandshakeReply>()
                .map_err(|_| "invalid captured RegionHandshakeReply")?;
            if message.agent_data.agent_id == agent.agent_id
                && message.agent_data.session_id == agent.session_id
            {
                handshake_reply = true;
            }
        }
        if record.kind == Kind::UdpOut && packet.id == AgentMovementComplete::ID && movement {
            let value = packet
                .decode::<AgentMovementComplete>()
                .map_err(|_| "invalid captured arrival")?;
            arrival |= packet.reliable()
                && value.agent_data.agent_id == agent.agent_id
                && value.agent_data.session_id == agent.session_id;
        }
        if movement && handshake && handshake_reply && arrival {
            return Ok(agent);
        }
    }
    Err("capture has no completed Ferrobot region arrival".into())
}

fn connected_evidence(config: &Config) -> Result<(), String> {
    let captured = records(&config.udp_capture)?;
    ping_evidence(&captured)?;
    let mut queue_found = false;
    for path in &config.http_captures {
        queue_found |= event_queue_evidence(&records(path)?)?;
    }
    if !queue_found {
        return Err(
            "capture has no completed repeated Ferrobot event-queue polls during the hold".into(),
        );
    }
    Ok(())
}

fn ping_evidence(captured: &[Record]) -> Result<(), String> {
    let mut starts = Vec::new();
    let mut completions = Vec::new();
    for record in captured {
        if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
            continue;
        }
        let packet = parse_packet(&record.bytes).map_err(|_| "invalid captured UDP packet")?;
        if packet.id == StartPingCheck::ID {
            let ping = packet
                .decode::<StartPingCheck>()
                .map_err(|_| "invalid captured ping")?;
            starts.push((record.kind, record.time_ms, ping.ping_id.ping_id));
        } else if packet.id == CompletePingCheck::ID {
            let ping = packet
                .decode::<CompletePingCheck>()
                .map_err(|_| "invalid captured ping reply")?;
            completions.push((record.kind, record.time_ms, ping.ping_id.ping_id));
        }
    }
    let mut matched: Vec<_> = starts
        .iter()
        .filter(|(direction, time, id)| {
            completions.iter().any(|(other, reply_time, reply_id)| {
                direction != other
                    && id == reply_id
                    && reply_time >= time
                    && reply_time - time <= 10_000
            })
        })
        .map(|entry| entry.1)
        .collect();
    matched.sort_unstable();
    matched.dedup();
    if matched.len() < 20
        || matched
            .last()
            .copied()
            .unwrap_or(0)
            .saturating_sub(matched.first().copied().unwrap_or(0))
            < 110_000
        || matched.windows(2).any(|pair| pair[1] - pair[0] > 15_000)
    {
        return Err("capture does not prove ping replies throughout the 120-second session".into());
    }
    Ok(())
}

fn event_queue_evidence(records: &[Record]) -> Result<bool, String> {
    // Ferrobot runs one sequential EQ poll loop. Other CAPS requests may
    // interleave in this capture, so do not blindly pair adjacent HTTP records.
    let polls: Vec<_> = records
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            if record.kind != Kind::HttpRequest {
                return None;
            }
            let (headers, _) = http_parts(&record.bytes)?;
            let line = headers.lines().next()?;
            let path = line.strip_prefix("POST /CE/")?.split_whitespace().next()?;
            Some((index, record.time_ms, path))
        })
        .collect();
    if polls.is_empty() {
        return Ok(false);
    }
    if polls.len() < 3
        || polls
            .last()
            .map(|poll| poll.1)
            .unwrap_or(0)
            .saturating_sub(polls[0].1)
            < 55_000
    {
        return Err("capture has too few completed Ferrobot event-queue polls".into());
    }
    for pair in polls.windows(2) {
        let (start, time, path) = pair[0];
        let (next, _, next_path) = pair[1];
        if path != next_path {
            return Err("Ferrobot replaced its event queue during the connected check".into());
        }
        let replied = records[start + 1..next].iter().any(|record| {
            if record.kind != Kind::HttpResponse || record.time_ms < time {
                return false;
            }
            let Some((headers, body)) = http_parts(&record.bytes) else {
                return false;
            };
            let status = headers
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1));
            let mime = headers.lines().skip(1).find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-type")
                    .then_some(value.trim())
            });
            match (status, mime) {
                (Some("502"), Some("text/html")) => {
                    body.is_empty() && record.time_ms - time >= 55_000
                }
                (Some("200"), Some("application/xml")) => {
                    ferroan_llsd::from_xml(body).is_ok_and(|value| {
                        matches!(value.get("events"), ferroan_llsd::Llsd::Array(_))
                            && matches!(value.get("id"), ferroan_llsd::Llsd::Integer(_))
                    })
                }
                _ => false,
            }
        });
        if !replied {
            return Err(
                "Ferrobot poll restarted without a captured valid event-queue reply".into(),
            );
        }
    }
    Ok(true)
}

fn http_parts(bytes: &[u8]) -> Option<(&str, &[u8])> {
    let end = bytes.windows(4).position(|part| part == b"\r\n\r\n")?;
    Some((std::str::from_utf8(&bytes[..end]).ok()?, &bytes[end + 4..]))
}

fn logout_evidence(records: &[Record], agent: Agent) -> Result<(), String> {
    let mut requested = false;
    for record in records {
        if !matches!(record.kind, Kind::UdpIn | Kind::UdpOut) {
            continue;
        }
        let packet = parse_packet(&record.bytes).map_err(|_| "invalid captured UDP packet")?;
        if record.kind == Kind::UdpIn && packet.id == LogoutRequest::ID {
            let value = packet
                .decode::<LogoutRequest>()
                .map_err(|_| "invalid captured logout request")?;
            requested = value.agent_data.agent_id == agent.agent_id
                && value.agent_data.session_id == agent.session_id;
        }
        if requested && record.kind == Kind::UdpOut && packet.id == LogoutReply::ID {
            let value = packet
                .decode::<LogoutReply>()
                .map_err(|_| "invalid captured logout reply")?;
            if value.agent_data.agent_id == agent.agent_id
                && value.agent_data.session_id == agent.session_id
            {
                return Ok(());
            }
        }
    }
    Err("Ferrobot SIGTERM has no matching captured LogoutRequest and LogoutReply".into())
}

async fn companion(
    harness: &Harness,
    api: &Api,
    ferrobot: Uuid,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    let http =
        ferroan_bot::login::http_client().map_err(|_| "could not create companion HTTP client")?;
    let login = ferroan_bot::login(
        &http,
        &LoginRequest {
            login_uri: harness.login_uri.clone(),
            username: format!("{} {}", harness.chat.first_name, harness.chat.last_name),
            password: harness.chat.password.clone(),
            start: "last".into(),
        },
    )
    .await
    .map_err(|_| "companion login failed")?;
    let mut session = Session::connect(
        SocketAddr::from((login.sim_ip, login.sim_port)),
        Agent {
            agent_id: login.agent_id,
            session_id: login.session_id,
            circuit_code: login.circuit_code,
        },
    )
    .await
    .map_err(|_| "companion circuit failed")?;
    session
        .run_until_ready(&http, &login.seed_capability, Duration::from_secs(30))
        .await
        .map_err(|_| "companion handshake failed")?;
    let result = exchange(&mut session, harness, api, ferrobot, lines).await;
    let logout = session.logout(Duration::from_secs(10)).await;
    result?;
    logout.map_err(|_| "companion LogoutReply was not received")?;
    Ok(())
}

fn string_field(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

async fn exchange(
    session: &mut Session,
    harness: &Harness,
    api: &Api,
    ferrobot: Uuid,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    let outgoing = format!("ferroan-to-ferrobot-{}", Uuid::new_v4());
    let incoming = format!("ferrobot-to-ferroan-{}", Uuid::new_v4());
    let agent = session.agent();
    let mut chat = ChatFromViewer::default();
    chat.agent_data.agent_id = agent.agent_id;
    chat.agent_data.session_id = agent.session_id;
    chat.chat_data.message = string_field(&outgoing);
    chat.chat_data.type_ = 1;
    session
        .send(&chat, true)
        .await
        .map_err(|_| "companion chat send failed")?;
    wait_history(session, api, "LocalChatHistory", &[], &outgoing).await?;
    lines.push("ferrobot chat receive: pass".into());
    api.command("Say", &["0", &incoming]).await?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        session
            .drive(STEP)
            .await
            .map_err(|_| "companion receive failed")?;
        let mut heard = false;
        while let Some(packet) = session.poll_inbox() {
            if packet.id == ChatFromSimulator::ID {
                let value = packet
                    .decode::<ChatFromSimulator>()
                    .map_err(|_| "invalid companion chat packet")?;
                if value.chat_data.source_id == ferrobot
                    && value.chat_data.chat_type == 1
                    && value
                        .chat_data
                        .message
                        .strip_suffix(&[0])
                        .unwrap_or(&value.chat_data.message)
                        == incoming.as_bytes()
                {
                    heard = true;
                }
            }
        }
        if heard {
            break;
        }
        if Instant::now() >= deadline {
            return Err("companion did not hear Ferrobot local chat".into());
        }
    }
    lines.push("ferrobot chat send: pass".into());
    let text = format!("ferroan-im-to-ferrobot-{}", Uuid::new_v4());
    let mut im = ImprovedInstantMessage::default();
    im.agent_data.agent_id = agent.agent_id;
    im.agent_data.session_id = agent.session_id;
    im.message_block.to_agent_id = ferrobot;
    im.message_block.id = Uuid::from_u128(agent.agent_id.as_u128() ^ ferrobot.as_u128());
    im.message_block.from_agent_name = string_field(&format!(
        "{} {}",
        harness.chat.first_name, harness.chat.last_name
    ));
    im.message_block.message = string_field(&text);
    im.message_block.offline = 1;
    session
        .send(&im, true)
        .await
        .map_err(|_| "companion IM send failed")?;
    wait_history(
        session,
        api,
        "getimchat",
        &[&agent.agent_id.to_string()],
        &text,
    )
    .await?;
    lines.push("ferrobot IM: pass".into());
    Ok(())
}

async fn wait_history(
    session: &mut Session,
    api: &Api,
    command: &str,
    args: &[&str],
    text: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        session
            .drive(STEP)
            .await
            .map_err(|_| "companion circuit failed while awaiting history")?;
        while session.poll_inbox().is_some() {}
        let reply = api.command(command, args).await?;
        let history: Vec<String> =
            serde_json::from_str(&reply).map_err(|_| "Ferrobot returned invalid chat history")?;
        if history.iter().any(|line| line.ends_with(text)) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Ferrobot did not receive the companion message".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn datagram<M: Msg>(kind: Kind, time_ms: u64, message: &M) -> Record {
        let mut bytes = Vec::new();
        ferroan_protocol::build_packet(message, 1, true, false, &[], &mut bytes);
        Record {
            kind,
            time_ms,
            bytes,
        }
    }

    #[test]
    fn login_capture_requires_movement_and_arrival_for_the_current_session() {
        let mut records =
            ferroan_capture::read_capture(include_bytes!("../../fixtures/arrival/sequence.cap"))
                .unwrap();
        let region = records
            .iter()
            .find_map(|record| {
                let packet = parse_packet(&record.bytes).unwrap();
                (packet.id == RegionHandshake::ID).then(|| {
                    packet
                        .decode::<RegionHandshake>()
                        .unwrap()
                        .region_info2
                        .region_id
                })
            })
            .unwrap();
        let agent = Uuid::from_u128(1);
        assert!(
            login_evidence(&records, agent, region).is_err(),
            "no handshake reply yet"
        );
        let mut reply = RegionHandshakeReply::default();
        reply.agent_data.agent_id = agent;
        reply.agent_data.session_id = Uuid::from_u128(2);
        records.push(datagram(Kind::UdpIn, 100, &reply));
        assert!(login_evidence(&records, agent, region).is_ok());
        assert!(login_evidence(&records, agent, Uuid::nil()).is_err());
        records
            .retain(|record| parse_packet(&record.bytes).unwrap().id != AgentMovementComplete::ID);
        assert!(login_evidence(&records, agent, region).is_err());
        let mut arrival = AgentMovementComplete::default();
        arrival.agent_data.agent_id = agent;
        arrival.agent_data.session_id = Uuid::from_u128(99);
        records.push(datagram(Kind::UdpOut, 101, &arrival));
        assert!(login_evidence(&records, agent, region).is_err());
    }

    #[test]
    fn ping_evidence_rejects_only_endpoints_or_a_long_gap() {
        let exchange = |time, id| {
            let mut start = StartPingCheck::default();
            start.ping_id.ping_id = id;
            let mut reply = CompletePingCheck::default();
            reply.ping_id.ping_id = id;
            [
                datagram(Kind::UdpIn, time, &start),
                datagram(Kind::UdpOut, time + 10, &reply),
            ]
        };
        let regular: Vec<_> = (0..24)
            .flat_map(|id| exchange(u64::from(id) * 5000, id))
            .collect();
        assert!(ping_evidence(&regular).is_ok());
        let endpoints: Vec<_> = [exchange(0, 0), exchange(115_000, 1)]
            .into_iter()
            .flatten()
            .collect();
        assert!(ping_evidence(&endpoints).is_err());
        let gap: Vec<_> = regular
            .into_iter()
            .filter(|record| !(45_000..60_000).contains(&record.time_ms))
            .collect();
        assert!(ping_evidence(&gap).is_err());
    }

    fn http_record(kind: Kind, time_ms: u64, text: &str) -> Record {
        Record {
            kind,
            time_ms,
            bytes: text.as_bytes().to_vec(),
        }
    }

    #[test]
    fn event_queue_evidence_requires_completed_valid_replies_between_polls() {
        const POLL: &str = "POST /CE/test HTTP/1.1\r\n\r\n";
        const EMPTY: &str = "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/html\r\n\r\n";
        let mut records = vec![
            http_record(Kind::HttpRequest, 0, POLL),
            http_record(Kind::HttpResponse, 58_000, EMPTY),
            http_record(Kind::HttpRequest, 58_010, POLL),
            // An unrelated response may interleave with the held EQ response.
            http_record(Kind::HttpResponse, 60_000, "HTTP/1.1 404 Not Found\r\n\r\n"),
            http_record(Kind::HttpResponse, 116_010, EMPTY),
            http_record(Kind::HttpRequest, 116_020, POLL),
        ];
        assert!(event_queue_evidence(&records).unwrap());
        records[4].bytes = b"HTTP/1.1 503 Service Unavailable\r\n\r\n".to_vec();
        assert!(event_queue_evidence(&records).is_err());
        records[4].bytes = b"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\n\r\n<llsd><map><key>caps</key><string>unrelated</string></map></llsd>".to_vec();
        assert!(
            event_queue_evidence(&records).is_err(),
            "unrelated successful CAPS response is not EQ evidence"
        );
        records[4].bytes = b"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\n\r\n<llsd><map><key>events</key><array/><key>id</key><integer>7</integer></map></llsd>".to_vec();
        assert!(event_queue_evidence(&records).unwrap());
        records[4] = http_record(Kind::HttpResponse, 59_000, EMPTY);
        assert!(
            event_queue_evidence(&records).is_err(),
            "quick empty error is not a held poll"
        );
        assert!(!event_queue_evidence(&[]).unwrap());
    }

    #[test]
    fn command_reply_requires_both_envelopes_and_never_echoes_untrusted_text() {
        let valid = json!({"Key":true,"Value":json!({"command":"SimName","status":true,"reply":"Fixture Region"}).to_string()});
        assert_eq!(
            parse_reply(valid.to_string().as_bytes(), "SimName").unwrap(),
            "Fixture Region"
        );
        for value in [
            json!({"Key":false,"Value":"private"}),
            json!({"Key":true,"Value":json!({"command":"SimName","status":false,"reply":"private"}).to_string()}),
            json!({"Key":true,"Value":json!({"command":"Name","status":true,"reply":"private"}).to_string()}),
        ] {
            let error = parse_reply(value.to_string().as_bytes(), "SimName").unwrap_err();
            assert!(!error.contains("private"));
        }
        assert!(parse_reply(b"No bot connected yet", "SimName").is_err());
    }

    #[test]
    fn api_port_requires_a_loopback_listener_and_valid_port() {
        assert_eq!(
            api_port(b"[INFO] HTTP API on port 43123 (127.0.0.1)\n"),
            Some(43123)
        );
        assert_eq!(api_port(b"HTTP API on port 43123 (0.0.0.0)"), None);
        assert_eq!(api_port(b"HTTP API on port 0 (127.0.0.1)"), None);
    }
}
