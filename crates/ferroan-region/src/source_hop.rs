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
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.CoreModules/Framework/EntityTransfer/EntityTransferModule.cs
//     (DoTeleportInternal, TransferAgent_V2, Fail, the unknown-region map block)
//   Source/OpenSim.Framework/CapsUtil.cs (GetCapsSeedPath, GetRandomCapsObjectPath)
//   OpenSim-NGC/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Viewer half of one source-region hop.
//!
//! `QueryAccess`, then `TeleportStart`, then `CreateAgent` with `child` true,
//! then an event-queue `TeleportFinish`, then `UpdateAgent`. The root stays a
//! root. A failure after a successful create closes the destination agent.
//! The grid directory is empty until the child demotion plugs a real lookup in.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use ferroan_client::{
    CapsRegistry, EventQueueError, FinishEvent, HopOut, HopRequest, INTERNAL, NOT_FOUND,
    RESOLVE_FAILED, SourceHop, caps_seed, connect_reason, offline_map_block, teleport_finish_event,
};
use ferroan_llsd::Llsd;
use uuid::Uuid;

use crate::config::RegionConfig;
use crate::simulation_client::{
    AccessQuery, AgentData, Circuit, RegionEnd, SimulationClient, TransferContext,
};

/// One accepted root, plus the region it is leaving.
#[derive(Clone, Debug)]
struct HopSession {
    agent_id: Uuid,
    session_id: Uuid,
    secure_session_id: Uuid,
    circuit_code: u32,
    first_name: String,
    last_name: String,
    source: RegionEnd,
}

/// A neighbour the hop can try. `endpoint` is the address the viewer connects to.
#[derive(Clone, Debug)]
pub struct HopDestination {
    pub end: RegionEnd,
    pub endpoint: SocketAddr,
    pub size_x: u32,
    pub size_y: u32,
}

/// Looks up a region handle. Production has no grid lookup yet.
pub trait RegionDirectory: Send + Sync {
    fn find(&self, handle: u64) -> Option<HopDestination>;
}

/// Every foreign handle is unknown. The viewer gets the not-found packet.
#[derive(Clone, Copy, Debug)]
pub struct EmptyDirectory;

impl RegionDirectory for EmptyDirectory {
    fn find(&self, _handle: u64) -> Option<HopDestination> {
        None
    }
}

/// Login facts the hop needs. The destination caps path is a new object path.
#[derive(Clone, Debug)]
pub struct HopLogin {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub secure_session_id: Uuid,
    pub circuit_code: u32,
    pub first_name: String,
    pub last_name: String,
}

/// Shared neighbour client and the source region. One per region process.
#[derive(Clone)]
pub struct HopEnv {
    source: RegionEnd,
    client: Option<SimulationClient>,
    caps: CapsRegistry,
}

impl HopEnv {
    /// An empty directory. A client that cannot be built still reports unknown regions.
    pub fn from_region(config: &RegionConfig, caps: CapsRegistry) -> Self {
        let client = match SimulationClient::new() {
            Ok(client) => Some(client),
            Err(error) => {
                tracing::error!("ferroan-region: simulation client unavailable: {error}");
                None
            }
        };
        Self {
            source: RegionEnd {
                server_uri: config.server_uri.clone(),
                region_id: config.id,
                name: config.name.clone(),
                loc_x: config.location[0].wrapping_mul(256),
                loc_y: config.location[1].wrapping_mul(256),
            },
            client,
            caps,
        }
    }

    /// The hop for one accepted root. Events go to that agent's queue.
    pub fn install(&self, login: HopLogin) -> SourceHop {
        let session = HopSession {
            agent_id: login.agent_id,
            session_id: login.session_id,
            secure_session_id: login.secure_session_id,
            circuit_code: login.circuit_code,
            first_name: login.first_name,
            last_name: login.last_name,
            source: self.source.clone(),
        };
        let caps = self.caps.clone();
        let agent_id = login.agent_id;
        drive(
            session,
            self.client.clone(),
            Arc::new(EmptyDirectory),
            move |event| caps.enqueue_for(agent_id, event),
        )
    }
}

fn drive(
    session: HopSession,
    client: Option<SimulationClient>,
    directory: Arc<dyn RegionDirectory>,
    enqueue: impl Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync + 'static,
) -> SourceHop {
    let enqueue = Arc::new(enqueue);
    SourceHop::new(move |request, out| {
        let session = session.clone();
        let client = client.clone();
        let directory = Arc::clone(&directory);
        let enqueue = Arc::clone(&enqueue);
        Box::pin(async move {
            perform(
                &session,
                client.as_ref(),
                directory.as_ref(),
                &request,
                &out,
                enqueue.as_ref(),
            )
            .await;
        })
    })
}

async fn perform(
    session: &HopSession,
    client: Option<&SimulationClient>,
    directory: &dyn RegionDirectory,
    request: &HopRequest,
    out: &HopOut,
    enqueue: &(dyn Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync),
) {
    let _ = request.look_at;
    let Some(destination) = directory.find(request.region_handle) else {
        tracing::debug!("source hop: unknown region");
        out.teleport_failed(NOT_FOUND).await;
        out.map_block(offline_map_block(session.agent_id, request.region_handle))
            .await;
        return;
    };
    let Some((ip, port)) = viewer_endpoint(destination.endpoint) else {
        out.teleport_failed(RESOLVE_FAILED).await;
        return;
    };
    let Some(client) = client else {
        out.teleport_failed(INTERNAL).await;
        return;
    };
    tracing::debug!("source hop: query");
    let outcome = match client
        .query_access(&destination.end, &access_query(session, request))
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::debug!("source hop: query: {error}");
            out.teleport_failed(INTERNAL).await;
            return;
        }
    };
    if !outcome.success {
        out.teleport_failed(&outcome.reason).await;
        return;
    }
    if !out.teleport_start(request.flags).await {
        return;
    }
    let context = TransferContext {
        inbound: outcome.inbound_version,
        outbound: outcome.outbound_version,
        wearables_count: outcome.wearables_count,
    };
    // No child seed is stored yet, so the destination circuit gets a new path.
    // The viewer seed uses that same path.
    let caps_path = Uuid::new_v4().to_string();
    let mut circuit = child_circuit(session, request, &caps_path);
    tracing::debug!("source hop: create");
    let created = match client
        .create_agent(
            Some(&session.source),
            Some(&destination.end),
            &mut circuit,
            request.flags,
            &context,
        )
        .await
    {
        Ok(created) => created,
        Err(error) => {
            tracing::debug!("source hop: create: {error}");
            out.teleport_failed(INTERNAL).await;
            return;
        }
    };
    if !created.success {
        out.teleport_failed(&created.reason).await;
        return;
    }
    let seed = caps_seed(&destination.end.server_uri, &caps_path);
    let event = teleport_finish_event(&FinishEvent {
        agent: session.agent_id,
        handle: request.region_handle,
        endpoint: ip,
        port,
        flags: request.flags,
        seed,
        size_x: destination.size_x,
        size_y: destination.size_y,
    });
    if enqueue(event).is_err() {
        close_destination(client, &destination.end, session).await;
        out.teleport_failed(&connect_reason(&destination.end.name))
            .await;
        return;
    }
    let update = agent_update(session, request, &destination);
    tracing::debug!("source hop: update");
    let arrived = match client
        .update_agent(&destination.end, &update, &context)
        .await
    {
        Ok(arrived) => arrived,
        Err(error) => {
            tracing::debug!("source hop: update: {error}");
            false
        }
    };
    if !arrived {
        close_destination(client, &destination.end, session).await;
        out.teleport_failed(&connect_reason(&destination.end.name))
            .await;
    }
}

fn viewer_endpoint(endpoint: SocketAddr) -> Option<(Ipv4Addr, u16)> {
    match endpoint {
        SocketAddr::V4(address) if address.port() != 0 => Some((*address.ip(), address.port())),
        _ => None,
    }
}

fn access_query(session: &HopSession, request: &HopRequest) -> AccessQuery {
    AccessQuery {
        agent_id: session.agent_id,
        agent_home_uri: None,
        via_teleport: true,
        position: request.position,
        features: Vec::new(),
        context: TransferContext::negotiated(),
    }
}

fn child_circuit(session: &HopSession, request: &HopRequest, caps_path: &str) -> Circuit {
    Circuit {
        agent_id: session.agent_id,
        base_folder: Uuid::nil(),
        caps_path: caps_path.to_owned(),
        children_seeds: Vec::new(),
        child: true,
        circuit_code: session.circuit_code,
        first_name: session.first_name.clone(),
        last_name: session.last_name.clone(),
        inventory_folder: Uuid::nil(),
        secure_session_id: session.secure_session_id,
        session_id: session.session_id,
        service_session_id: String::new(),
        start_pos: request.position,
        client_ip: request.client_ip.clone(),
        viewer: String::new(),
        channel: String::new(),
        mac: String::new(),
        id0: String::new(),
        far: 0.0,
        appearance: None,
        service_urls: Vec::new(),
    }
}

fn agent_update(
    session: &HopSession,
    request: &HopRequest,
    destination: &HopDestination,
) -> AgentData {
    AgentData {
        region_id: destination.end.region_id,
        circuit_code: session.circuit_code,
        agent_id: session.agent_id,
        session_id: session.session_id,
        position: request.position,
        velocity: [0.0; 3],
        center: [0.0; 3],
        size: [0.0; 3],
        at_axis: [0.0; 3],
        left_axis: [0.0; 3],
        up_axis: [0.0; 3],
        wait_for_root: true,
        far: 0.0,
        throttles: Vec::new(),
        locomotion_state: 0,
        head_rotation: [0.0; 4],
        body_rotation: [0.0; 4],
        control_flags: 0,
        god_data: true,
        agent_access: 0,
        default_animation: None,
        animation_state: None,
        motion_state: 0,
        appearance: None,
        sit_offset: [0.0; 3],
        children_seeds: Vec::new(),
    }
}

async fn close_destination(
    client: &SimulationClient,
    destination: &RegionEnd,
    session: &HopSession,
) {
    let auth = session.session_id.to_string();
    if let Err(error) = client
        .close_agent(destination, session.agent_id, &auth)
        .await
    {
        // The viewer packet still goes out. The error text has no URL.
        tracing::debug!("source hop: close: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Read;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::{
        EmptyDirectory, HopDestination, HopSession, RegionDirectory, drive, viewer_endpoint,
    };
    use crate::simulation_client::{RegionEnd, SimulationClient};
    use ferroan_client::{
        ArrivalConfig, CONNECT_INNER, CapsServer, CircuitExpect, CircuitServer, EventQueueError,
        NOT_FOUND, connect_reason,
    };
    use ferroan_llsd::Llsd;
    use ferroan_protocol::msgs::{
        MapBlockReply, TeleportFailed, TeleportFinish, TeleportLocal, TeleportLocationRequest,
        TeleportStart, UseCircuitCode,
    };
    use ferroan_protocol::{Msg, build_packet, parse_packet};
    use flate2::read::GzDecoder;
    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream, UdpSocket};
    use tokio::sync::Notify;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    const HANDLE: u64 = 256_u64 << 32;
    const POSITION: [f32; 3] = [128.0, 128.0, 30.0];
    const QUERY_OK: &str = "{\"success\":true,\"reason\":\"\",\"version\":\"SIMULATION/0.8\",\"negotiated_inbound_version\":0.8,\"negotiated_outbound_version\":0.8}";
    const CREATE_OK: &str = "{\"success\":true,\"reason\":\"\",\"service_session_id\":\"\"}";

    struct Listed(HashMap<u64, HopDestination>);

    impl RegionDirectory for Listed {
        fn find(&self, handle: u64) -> Option<HopDestination> {
            self.0.get(&handle).cloned()
        }
    }

    struct Hit {
        header: String,
        body: Vec<u8>,
    }

    struct Opened {
        running: JoinHandle<Result<(), ferroan_client::CircuitError>>,
        script: Option<JoinHandle<()>>,
        collecting: JoinHandle<()>,
        hits: Arc<Mutex<Vec<Hit>>>,
        gathered: Arc<Mutex<Vec<Vec<u8>>>>,
        socket: Arc<UdpSocket>,
        position: Arc<Mutex<[f32; 3]>>,
        release: Option<Arc<Notify>>,
        events: String,
        caps: Arc<Mutex<CapsServer>>,
    }

    impl Drop for Opened {
        fn drop(&mut self) {
            self.running.abort();
            if let Some(script) = self.script.take() {
                script.abort();
            }
            self.collecting.abort();
        }
    }

    fn session() -> HopSession {
        HopSession {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            secure_session_id: Uuid::from_u128(3),
            circuit_code: 42,
            first_name: "Hop".to_string(),
            last_name: "Test".to_string(),
            source: RegionEnd {
                server_uri: "http://127.0.0.1:8/".to_string(),
                region_id: Uuid::from_u128(9),
                name: "Home".to_string(),
                loc_x: 0,
                loc_y: 0,
            },
        }
    }

    fn destination(addr: SocketAddr) -> HopDestination {
        HopDestination {
            end: RegionEnd {
                server_uri: format!("http://{addr}/"),
                region_id: Uuid::from_u128(7),
                name: "Neighbour".to_string(),
                loc_x: 256,
                loc_y: 0,
            },
            endpoint: "127.0.0.1:9100".parse().unwrap(),
            size_x: 256,
            size_y: 256,
        }
    }

    fn listed(addr: SocketAddr) -> Arc<dyn RegionDirectory> {
        let mut regions = HashMap::new();
        regions.insert(HANDLE, destination(addr));
        Arc::new(Listed(regions))
    }

    async fn listen() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, addr)
    }

    fn enqueue_caps(
        caps: &Arc<Mutex<CapsServer>>,
    ) -> impl Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync + 'static {
        let caps = Arc::clone(caps);
        move |event: Llsd| caps.lock().unwrap().enqueue_event(event)
    }

    async fn open(
        directory: Arc<dyn RegionDirectory>,
        client: Option<SimulationClient>,
        listener: Option<TcpListener>,
        steps: Vec<(u16, String)>,
        hold_query: bool,
        enqueue: impl Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync + 'static,
    ) -> Opened {
        let release = hold_query.then(Notify::new).map(Arc::new);
        let hits = Arc::new(Mutex::new(Vec::new()));
        let script = match listener {
            Some(listener) => {
                Some(spawn_neighbour(listener, steps, Arc::clone(&hits), release.clone()).await)
            }
            None => None,
        };
        let caps_server = CapsServer::bind().await.unwrap();
        let events = caps_server.event_queue_url().to_owned();
        let caps = Arc::new(Mutex::new(caps_server));
        let hop = drive(session(), client, directory, enqueue);
        let position = Arc::new(Mutex::new([128.0, 128.0, 25.0]));
        let region = ferroan_client::TeleportRegion {
            world_x: 0,
            world_y: 0,
            size_x: 256.0,
            size_y: 256.0,
            default_landing: [128.0, 128.0, 30.0],
            half_height: 0.9,
            allow_movement: true,
            godlike: false,
            flying: false,
            ground: Arc::new(|_, _| 21.0),
            position: Arc::clone(&position),
        };
        let expect = CircuitExpect {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
            region_id: Uuid::from_u128(9),
        };
        let arrival = ArrivalConfig {
            position: [128.0, 128.0, 25.0],
            look_at: [1.0, 0.0, 0.0],
            region_handle: 0,
            channel_version: "Ferroan hop test".to_string(),
            land: Vec::new(),
            avatar: None,
            teleport: Some(region),
            source_hop: Some(hop),
        };
        let server = CircuitServer::bind().await.unwrap();
        let udp = server.local_addr().unwrap();
        let running =
            tokio::spawn(async move { server.serve_one_with_arrival(expect, arrival).await });
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        socket.connect(udp).await.unwrap();
        let gathered = Arc::new(Mutex::new(Vec::new()));
        let reading = Arc::clone(&socket);
        let store = Arc::clone(&gathered);
        let collecting = tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            while let Ok(len) = reading.recv(&mut buf).await {
                store.lock().unwrap().push(buf[..len].to_vec());
            }
        });
        send_view(&socket, expect, 1, login_packet(expect)).await;
        send_view(
            &socket,
            expect,
            2,
            teleport_packet(expect, HANDLE, POSITION),
        )
        .await;
        Opened {
            running,
            script,
            collecting,
            hits,
            gathered,
            socket,
            position,
            release,
            events,
            caps,
        }
    }

    async fn spawn_neighbour(
        listener: TcpListener,
        steps: Vec<(u16, String)>,
        hits: Arc<Mutex<Vec<Hit>>>,
        release: Option<Arc<Notify>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            for (index, (status, text)) in steps.into_iter().enumerate() {
                let accepted =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept()).await;
                let Ok(Ok((mut stream, _))) = accepted else {
                    break;
                };
                let (header, body) = read_http(&mut stream).await;
                hits.lock().unwrap().push(Hit { header, body });
                if index == 0
                    && let Some(release) = &release
                {
                    let wait = release.notified();
                    tokio::pin!(wait);
                    if tokio::time::timeout(Duration::from_secs(5), wait)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                let raw = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = stream.write_all(raw.as_bytes()).await;
            }
        })
    }

    async fn read_http(stream: &mut TcpStream) -> (String, Vec<u8>) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let count = stream.read(&mut tmp).await.unwrap_or(0);
            if count == 0 {
                break 0;
            }
            buf.extend_from_slice(&tmp[..count]);
            if let Some(position) = buf.windows(4).position(|part| part == b"\r\n\r\n") {
                break position;
            }
            if buf.len() > 1024 * 1024 {
                break buf.len();
            }
        };
        let header = String::from_utf8_lossy(&buf[..header_end.min(buf.len())]).into_owned();
        let mut body = if header_end + 4 <= buf.len() {
            buf[header_end + 4..].to_vec()
        } else {
            Vec::new()
        };
        let length = content_length(&header);
        while body.len() < length {
            let count = stream.read(&mut tmp).await.unwrap_or(0);
            if count == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..count]);
        }
        if body.len() > length {
            body.truncate(length);
        }
        (header, body)
    }

    fn content_length(header: &str) -> usize {
        header
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    fn login_packet(expect: CircuitExpect) -> UseCircuitCode {
        let mut message = UseCircuitCode::default();
        message.circuit_code.id = expect.agent_id;
        message.circuit_code.session_id = expect.session_id;
        message.circuit_code.code = expect.circuit_code;
        message
    }

    fn teleport_packet(
        expect: CircuitExpect,
        handle: u64,
        position: [f32; 3],
    ) -> TeleportLocationRequest {
        let mut message = TeleportLocationRequest::default();
        message.agent_data.agent_id = expect.agent_id;
        message.agent_data.session_id = expect.session_id;
        message.info.region_handle = handle;
        message.info.position = position.into();
        message.info.look_at = [1.0, 0.0, 0.0].into();
        message
    }

    async fn send_view<M: ferroan_protocol::Msg>(
        socket: &UdpSocket,
        _expect: CircuitExpect,
        sequence: u32,
        message: M,
    ) {
        let mut bytes = Vec::new();
        build_packet(&message, sequence, true, false, &[], &mut bytes);
        socket.send(&bytes).await.unwrap();
    }

    async fn until(
        gathered: &Mutex<Vec<Vec<u8>>>,
        mut ready: impl FnMut(&[Vec<u8>]) -> bool,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if ready(&gathered.lock().unwrap()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn reasons(gathered: &[Vec<u8>]) -> Vec<String> {
        gathered
            .iter()
            .filter_map(|bytes| {
                let packet = parse_packet(bytes).ok()?;
                if packet.id != TeleportFailed::ID {
                    return None;
                }
                let message = packet.decode::<TeleportFailed>().ok()?;
                Some(ferroan_protocol::field_str(&message.info.reason))
            })
            .collect()
    }

    fn has_id(gathered: &[Vec<u8>], id: ferroan_protocol::MsgId) -> bool {
        gathered
            .iter()
            .any(|bytes| parse_packet(bytes).is_ok_and(|packet| packet.id == id))
    }

    fn request_parts(header: &str) -> (String, String) {
        let mut parts = header.lines().next().unwrap_or("").split_whitespace();
        (
            parts.next().unwrap_or("").to_string(),
            parts.next().unwrap_or("").to_string(),
        )
    }

    fn plain(hit: &Hit) -> Vec<u8> {
        let gzip = hit
            .header
            .to_ascii_lowercase()
            .contains("x-content-encoding: gzip");
        if !gzip {
            return hit.body.clone();
        }
        let mut decoder = GzDecoder::new(hit.body.as_slice());
        let mut plain = Vec::new();
        decoder.read_to_end(&mut plain).unwrap();
        plain
    }

    async fn finish_script(opened: &mut Opened) -> Vec<Hit> {
        let Some(mut handle) = opened.script.take() else {
            return Vec::new();
        };
        match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
            Ok(Ok(())) => std::mem::take(&mut *opened.hits.lock().unwrap()),
            _ => {
                handle.abort();
                panic!("neighbour script did not finish");
            }
        }
    }

    async fn poll_finish(opened: &Opened) -> Llsd {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let response = http
            .post(&opened.events)
            .body(Vec::new())
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let bytes = response.bytes().await.unwrap();
        ferroan_llsd::from_xml(&bytes).unwrap()
    }

    fn client() -> Option<SimulationClient> {
        Some(SimulationClient::new().unwrap())
    }

    #[tokio::test]
    async fn a_successful_hop_sends_start_then_the_finish_event() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let events = caps.lock().unwrap().event_queue_url().to_owned();
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "true".to_string()),
            ],
            false,
            enqueue_caps(&caps),
        )
        .await;
        opened.events = events;
        opened.caps = caps;
        let hits = finish_script(&mut opened).await;
        assert!(
            until(&opened.gathered, |packets| has_id(
                packets,
                TeleportStart::ID
            ))
            .await,
            "TeleportStart did not arrive"
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(!has_id(&packets, TeleportFinish::ID));
        assert!(!has_id(&packets, TeleportLocal::ID));
        assert!(!has_id(&packets, MapBlockReply::ID));
        assert!(reasons(&packets).is_empty());
        let start = packets.iter().find_map(|bytes| {
            let packet = parse_packet(bytes).ok()?;
            if packet.id != TeleportStart::ID {
                return None;
            }
            Some(packet)
        });
        let start = start.unwrap();
        assert!(start.reliable());
        assert_eq!(start.flags & ferroan_protocol::PacketFlags::ZEROCODED, 0);
        let message = start.decode::<TeleportStart>().unwrap();
        assert_eq!(message.info.teleport_flags, 16);
        assert_eq!(hits.len(), 3);
        let (method, _) = request_parts(&hits[0].header);
        assert_eq!(method, "QUERYACCESS");
        let query: Value = serde_json::from_slice(&hits[0].body).unwrap();
        assert_eq!(query["viaTeleport"], true);
        assert_eq!(query["position"], "<128, 128, 30>");
        let (method, _) = request_parts(&hits[1].header);
        assert_eq!(method, "POST");
        let create: Value = serde_json::from_slice(&plain(&hits[1])).unwrap();
        assert_eq!(create["child"], true);
        assert_eq!(create["teleport_flags"], "16");
        assert_eq!(create["start_pos"], "<128, 128, 30>");
        assert_eq!(create["client_ip"], "127.0.0.1");
        let (method, _) = request_parts(&hits[2].header);
        assert_eq!(method, "PUT");
        let update: Value = serde_json::from_slice(&plain(&hits[2])).unwrap();
        assert_eq!(update["message_type"], "AgentData");
        assert_eq!(update["wait_for_root"], true);
        assert!(update["god_data"].is_object());
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
        let event = poll_finish(&opened).await;
        let info = event.get("events").at(0);
        assert_eq!(info.get("message").as_str(), "TeleportFinish");
        let body = info.get("body").get("Info").at(0);
        assert_eq!(body.get("LocationID").as_u32(), 4);
        assert_eq!(body.get("SimAccess").as_i32(), 13);
        assert_eq!(body.get("TeleportFlags").as_u32(), 16);
        assert_eq!(body.get("RegionSizeX").as_u32(), 256);
        assert_eq!(body.get("RegionSizeY").as_u32(), 256);
        assert_eq!(body.get("RegionHandle").as_u64(), HANDLE);
        assert_eq!(body.get("SimIP").as_binary(), [127, 0, 0, 1]);
        assert_eq!(body.get("SimPort").as_i32(), 9100);
        assert!(body.get("SeedCapability").as_str().contains("/CAPS/"));
        assert!(body.get("SeedCapability").as_str().ends_with("0000/"));
    }

    #[tokio::test]
    async fn query_refusal_stays_and_does_not_start() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![(
                200,
                "{\"success\":false,\"reason\":\"no access\"}".to_string(),
            )],
            false,
            enqueue_caps(&caps),
        )
        .await;
        let hits = finish_script(&mut opened).await;
        assert!(
            until(&opened.gathered, |packets| reasons(packets)
                .iter()
                .any(|reason| reason == "no access"))
            .await
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(!has_id(&packets, TeleportStart::ID));
        assert!(!has_id(&packets, MapBlockReply::ID));
        assert_eq!(hits.len(), 1);
        assert_eq!(request_parts(&hits[0].header).0, "QUERYACCESS");
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
    }

    #[tokio::test]
    async fn create_refusal_sends_the_connector_reason_and_does_not_close() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, "{\"success\":false,\"reason\":\"full\"}".to_string()),
            ],
            false,
            enqueue_caps(&caps),
        )
        .await;
        let hits = finish_script(&mut opened).await;
        assert!(
            until(&opened.gathered, |packets| reasons(packets)
                .iter()
                .any(|reason| reason == "full"))
            .await
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(has_id(&packets, TeleportStart::ID));
        assert!(!has_id(&packets, MapBlockReply::ID));
        assert_eq!(hits.len(), 2);
        assert_eq!(request_parts(&hits[1].header).0, "POST");
        assert!(
            hits.iter()
                .all(|hit| request_parts(&hit.header).0 != "DELETE")
        );
    }

    #[tokio::test]
    async fn update_refusal_closes_and_sends_the_connection_sentence() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "false".to_string()),
                (200, "closed".to_string()),
            ],
            false,
            enqueue_caps(&caps),
        )
        .await;
        let hits = finish_script(&mut opened).await;
        let expected = connect_reason("Neighbour");
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| reason == &expected)
            })
            .await
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(has_id(&packets, TeleportStart::ID));
        assert!(
            reasons(&packets)
                .iter()
                .any(|reason| reason.contains(CONNECT_INNER))
        );
        assert_eq!(hits.len(), 4);
        let (method, target) = request_parts(&hits[3].header);
        assert_eq!(method, "DELETE");
        assert!(target.contains("auth=00000000-0000-0000-0000-000000000002"));
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
    }

    #[tokio::test]
    async fn an_enqueue_failure_closes_without_update() {
        let (listener, addr) = listen().await;
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "closed".to_string()),
            ],
            false,
            |_event| Err(EventQueueError::Closed),
        )
        .await;
        let hits = finish_script(&mut opened).await;
        let expected = connect_reason("Neighbour");
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| reason == &expected)
            })
            .await
        );
        assert_eq!(hits.len(), 3);
        assert_eq!(request_parts(&hits[2].header).0, "DELETE");
        assert!(hits.iter().all(|hit| request_parts(&hit.header).0 != "PUT"));
    }

    #[tokio::test]
    async fn an_unknown_region_sends_not_found_and_one_offline_map_block() {
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let opened = open(
            Arc::new(EmptyDirectory),
            None,
            None,
            Vec::new(),
            false,
            enqueue_caps(&caps),
        )
        .await;
        assert!(
            until(&opened.gathered, |packets| has_id(
                packets,
                MapBlockReply::ID
            ))
            .await,
            "map block did not arrive"
        );
        let packets = opened.gathered.lock().unwrap().clone();
        let found = reasons(&packets);
        assert!(!found.is_empty());
        assert!(found.iter().all(|reason| reason == NOT_FOUND));
        assert!(!has_id(&packets, TeleportStart::ID));
        assert!(!has_id(&packets, TeleportFinish::ID));
        let maps: Vec<_> = packets
            .iter()
            .filter_map(|bytes| {
                let packet = parse_packet(bytes).ok()?;
                if packet.id != MapBlockReply::ID {
                    return None;
                }
                Some(packet.decode::<MapBlockReply>().unwrap())
            })
            .collect();
        assert!(!maps.is_empty());
        assert_eq!(maps[0].agent_data.flags, 0);
        assert_eq!(maps[0].agent_data.agent_id, Uuid::from_u128(1));
        assert_eq!(maps[0].data.len(), 1);
        assert_eq!(maps[0].data[0].x, 1);
        assert_eq!(maps[0].data[0].y, 0);
        assert_eq!(maps[0].data[0].access, 254);
        assert!(maps[0].data[0].name.is_empty());
        assert!(maps[0].size.is_empty());
        assert_eq!(maps[0].omitted_trailing_blocks, 0);
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
    }

    #[tokio::test]
    async fn a_second_request_during_query_is_refused_and_does_not_start() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![(
                200,
                "{\"success\":false,\"reason\":\"no access\"}".to_string(),
            )],
            true,
            enqueue_caps(&caps),
        )
        .await;
        let deadline = Instant::now() + Duration::from_secs(3);
        while opened.hits.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "query did not arrive");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let expect = CircuitExpect {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
            region_id: Uuid::from_u128(9),
        };
        send_view(
            &opened.socket,
            expect,
            3,
            teleport_packet(expect, HANDLE, POSITION),
        )
        .await;
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| {
                    reason == "Previous teleport process incomplete.  Please retry shortly."
                })
            })
            .await,
            "in-transit refusal did not arrive"
        );
        assert_eq!(opened.hits.lock().unwrap().len(), 1);
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(!has_id(&packets, TeleportStart::ID));
        opened.release.take().unwrap().notify_one();
        let hits = finish_script(&mut opened).await;
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| reason == "no access")
            })
            .await
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(!has_id(&packets, TeleportStart::ID));
        assert_eq!(hits.len(), 1);
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
    }

    #[test]
    fn a_missing_ipv4_endpoint_does_not_ask_the_neighbour() {
        let endpoint: SocketAddr = "127.0.0.1:0".parse().unwrap();
        assert!(viewer_endpoint(endpoint).is_none());
        let endpoint: SocketAddr = "[::1]:9000".parse().unwrap();
        assert!(viewer_endpoint(endpoint).is_none());
    }
}
