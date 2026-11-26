// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The handshake order follows Aurora Viewer
//   crates/aurora-net/src/session.rs
//   crates/aurora-net/src/session/object_cache.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// UseCircuitCode, then CompleteAgentMovement after its ack, with
// AgentThrottle and AgentHeightWidth. RegionHandshakeReply uses the
// viewer's flags for a viewer that has no object cache.
// Arrival evidence is checked against Tranquillity's
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs (CompleteMovement)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs (MoveAgentIntoRegion)
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c.
// These send RegionHandshake before the reliable AgentMovementComplete.

//! Region handshake and logout on one circuit.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ferroan_capture::{Kind, Record};
use ferroan_protocol::msgs::{
    AgentHeightWidth, AgentMovementComplete, AgentThrottle, CoarseLocationUpdate,
    CompleteAgentMovement, CompletePingCheck, ImageData, ImprovedTerseObjectUpdate, LayerData,
    LogoutReply, LogoutRequest, ObjectUpdate, PacketAck, RegionHandshake, RegionHandshakeReply,
    SimStats, SimulatorViewerTimeMessage, StartPingCheck, UseCircuitCode,
};
use ferroan_protocol::{IncomingPacket, Msg, parse_packet};
use uuid::Uuid;

use crate::caps::{self, CapsError};
use crate::circuit::{Circuit, CircuitError, Resend};
use crate::throttle;

/// Identity from a login reply, the fields `UseCircuitCode` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agent {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub circuit_code: u32,
}

/// `REGION_HANDSHAKE_SUPPORTS_SELF_APPEARANCE | SEND_ALL_CACHEABLE | CACHE_EMPTY`.
/// The bot keeps no object cache, so the region sends every cacheable object.
const HANDSHAKE_FLAGS: u32 = 0x4 | 0x1 | 0x2;
const WINDOW_HEIGHT: u16 = 1080;
const WINDOW_WIDTH: u16 = 1920;
const PING_EVERY: Duration = Duration::from_secs(5);
/// Packets a scenario has not taken yet. The oldest is dropped past this.
const MAX_INBOX: usize = 256;
/// Per-circuit bound for each observer history, including the raw arrival trace.
pub(crate) const OBSERVER_HISTORY: usize = 256;
/// Minimap updates a scenario has not taken yet. The oldest is dropped past
/// this; a region sends each viewer one list every 50 frames (about 4.5 s).
const MAX_COARSE: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("udp: {0}")]
    Udp(#[from] std::io::Error),
    #[error(transparent)]
    Circuit(#[from] CircuitError),
    #[error(transparent)]
    Seed(#[from] CapsError),
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    #[error("seed task stopped")]
    SeedTask,
    #[error("the previous circuit visit still has too many buffered datagrams")]
    PriorCircuitBacklog,
    #[error("prim arrival capture is full")]
    ArrivalCaptureFull,
}

pub const LOAD_RECEIPT_LIMIT: usize = 2048;
pub(crate) enum PrimTrace {
    Arrival(f32),
    Load,
}

struct ArrivalCapture {
    limit: usize,
    task_bps: f32,
    start: Instant,
    records: Vec<Record>,
}

impl ArrivalCapture {
    fn record(&mut self, kind: Kind, bytes: &[u8]) -> Result<(), SessionError> {
        if self.records.len() >= self.limit {
            ferroan_diag::refuse(
                ferroan_diag::BOT_OBSERVER_HISTORY,
                ferroan_diag::Kind::Hard,
                self.limit as u64,
                self.records.len() as u64 + 1,
                "bot",
            );
            return Err(SessionError::ArrivalCaptureFull);
        }
        self.records.push(Record {
            kind,
            time_ms: self.start.elapsed().as_millis() as u64,
            bytes: bytes.to_vec(),
        });
        Ok(())
    }
}

pub struct Session {
    circuit: Circuit,
    agent: Agent,
    caps: HashMap<String, String>,
    ucc_seq: u32,
    land_rate: Option<f32>,
    movement_sent: bool,
    arrival_capture: Option<ArrivalCapture>,
    handshake_sent: bool,
    handshake_sequence: Option<u32>,
    generation: u64,
    arrived_generation: Option<u64>,
    generation_floor: Option<u32>,
    latest_sequence: Option<u32>,
    logged_out: bool,
    region_id: Option<Uuid>,
    inbox: VecDeque<IncomingPacket>,
    coarse: VecDeque<CoarseLocationUpdate>,
    last_ping: Instant,
    received: u64,
}

impl Session {
    /// Bind `127.0.0.1` and send `UseCircuitCode` with the login resend policy.
    pub async fn connect(sim: SocketAddr, agent: Agent) -> Result<Self, SessionError> {
        let mut circuit = Circuit::bind(sim).await?;
        let mut message = UseCircuitCode::default();
        message.circuit_code.code = agent.circuit_code;
        message.circuit_code.session_id = agent.session_id;
        message.circuit_code.id = agent.agent_id;
        let ucc_seq = circuit
            .send_reliable(&message, Resend::LOGIN_USE_CIRCUIT_CODE)
            .await?;
        Ok(Self {
            circuit,
            agent,
            caps: HashMap::new(),
            ucc_seq,
            land_rate: None,
            movement_sent: false,
            arrival_capture: None,
            handshake_sent: false,
            handshake_sequence: None,
            generation: 0,
            arrived_generation: None,
            generation_floor: None,
            latest_sequence: None,
            logged_out: false,
            region_id: None,
            inbox: VecDeque::new(),
            coarse: VecDeque::new(),
            last_ping: Instant::now(),
            received: 0,
        })
    }

    pub fn caps(&self) -> &HashMap<String, String> {
        &self.caps
    }

    pub fn region_id(&self) -> Option<Uuid> {
        self.region_id
    }

    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.circuit.local_addr()
    }

    /// The region address `UseCircuitCode` was sent to.
    pub fn sim_addr(&self) -> SocketAddr {
        self.circuit.addr
    }

    /// `CompleteAgentMovement` has already been sent on this circuit.
    ///
    /// A region that kept the circuit as a child must see that message
    /// again. A second `UseCircuitCode` closes the child.
    pub fn movement_sent(&self) -> bool {
        self.movement_sent
    }

    pub fn agent(&self) -> Agent {
        self.agent
    }

    /// Send one message. The caller drives the circuit so acks and resends run.
    pub async fn send<M: Msg>(&mut self, message: &M, reliable: bool) -> Result<(), SessionError> {
        self.circuit.send(message, reliable).await?;
        Ok(())
    }

    /// Pump the circuit for `budget`, answering pings and queueing other packets.
    pub async fn drive(&mut self, budget: Duration) -> Result<(), SessionError> {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            self.pump_once(left.min(Duration::from_millis(50))).await?;
        }
        Ok(())
    }

    /// The next packet that was not part of the handshake, if one is waiting.
    pub fn poll_inbox(&mut self) -> Option<IncomingPacket> {
        self.inbox.pop_front()
    }

    /// The minimap updates received since the last call, oldest first. They
    /// skip the inbox, so a scenario that does not read them is unchanged.
    pub fn take_coarse(&mut self) -> Vec<CoarseLocationUpdate> {
        self.coarse.drain(..).collect()
    }

    /// Datagrams received from the region since connect.
    pub fn received(&self) -> u64 {
        self.received
    }

    /// Send `UseCircuitCode`'s follow-up once it is acked, answer the
    /// handshake and pings, and POST the seed. Readiness also requires a fresh
    /// AgentMovementComplete for this agent/session on the current circuit.
    pub async fn run_until_ready(
        &mut self,
        http: &reqwest::Client,
        seed: &str,
        timeout: Duration,
    ) -> Result<(), SessionError> {
        let http = http.clone();
        let seed = seed.to_owned();
        let mut seed_task = if seed.is_empty() {
            None
        } else {
            Some(tokio::spawn(async move {
                caps::request_seed(&http, &seed).await
            }))
        };
        let mut seed_done = seed_task.is_none();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump_once(Duration::from_millis(50)).await?;
            self.send_movement_if_acked().await?;
            if !seed_done && seed_task.as_ref().is_some_and(|task| task.is_finished()) {
                let Some(task) = seed_task.take() else {
                    return Err(SessionError::SeedTask);
                };
                let joined = task.await.map_err(|_| SessionError::SeedTask)?;
                self.caps = joined?;
                seed_done = true;
            }
            if self.movement_sent
                && self.handshake_sent
                && seed_done
                && self.arrived_generation == Some(self.generation)
            {
                return Ok(());
            }
        }
        Err(SessionError::Timeout(
            if self.movement_sent && self.handshake_sent && seed_done {
                "AgentMovementComplete"
            } else {
                "handshake"
            },
        ))
    }

    /// Send `CompleteAgentMovement` again on a circuit the region kept.
    ///
    /// The seed is posted again, and readiness requires a fresh
    /// `RegionHandshake` and `AgentMovementComplete`. Packets and identity from the previous visit cannot
    /// establish that the return hop reached this region.
    pub async fn resume_as_root(
        &mut self,
        http: &reqwest::Client,
        seed: &str,
        timeout: Duration,
    ) -> Result<(), SessionError> {
        // Process already buffered datagrams as the previous visit. The bound
        // also prevents a busy old circuit from delaying the new arrival forever.
        let mut buf = [0u8; ferroan_protocol::packet::MAX_DATAGRAM];
        for index in 0..=MAX_INBOX {
            match self.circuit.try_recv_from(&mut buf) {
                Ok(_) if index == MAX_INBOX => return Err(SessionError::PriorCircuitBacklog),
                Ok((len, from)) => self.receive_datagram(&buf[..len], from).await?,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        self.generation = self.generation.wrapping_add(1);
        self.arrived_generation = None;
        self.generation_floor = self.latest_sequence;
        self.handshake_sequence = None;
        self.handshake_sent = false;
        self.region_id = None;
        self.inbox.clear();
        self.coarse.clear();
        self.send_movement_followup().await?;
        self.run_until_ready(http, seed, timeout).await
    }

    /// `LogoutRequest`, then wait for `LogoutReply`.
    pub async fn logout(&mut self, timeout: Duration) -> Result<(), SessionError> {
        let mut message = LogoutRequest::default();
        message.agent_data.agent_id = self.agent.agent_id;
        message.agent_data.session_id = self.agent.session_id;
        self.circuit.send(&message, true).await?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump_once(Duration::from_millis(50)).await?;
            if self.logged_out {
                return Ok(());
            }
        }
        Err(SessionError::Timeout("logout"))
    }

    async fn send_movement_if_acked(&mut self) -> Result<(), SessionError> {
        if self.movement_sent || self.circuit.is_unacked(self.ucc_seq) {
            return Ok(());
        }
        self.send_movement_followup().await
    }

    async fn send_movement_followup(&mut self) -> Result<(), SessionError> {
        let mut movement = CompleteAgentMovement::default();
        movement.agent_data.agent_id = self.agent.agent_id;
        movement.agent_data.session_id = self.agent.session_id;
        movement.agent_data.circuit_code = self.agent.circuit_code;
        self.circuit.send(&movement, true).await?;

        self.send_viewer_settings().await?;
        self.movement_sent = true;
        Ok(())
    }

    /// Initialize the child viewer's throttle and window without requesting root.
    pub(crate) async fn send_viewer_settings(&mut self) -> Result<(), SessionError> {
        self.send_throttles(self.arrival_capture.as_ref().map(|c| c.task_bps))
            .await?;

        let mut window = AgentHeightWidth::default();
        window.agent_data.agent_id = self.agent.agent_id;
        window.agent_data.session_id = self.agent.session_id;
        window.agent_data.circuit_code = self.agent.circuit_code;
        window.height_width_block.height = WINDOW_HEIGHT;
        window.height_width_block.width = WINDOW_WIDTH;
        self.circuit.send(&window, true).await?;
        Ok(())
    }

    pub(crate) fn set_land_rate(&mut self, rate: Option<f32>) {
        self.land_rate = rate;
    }

    pub(crate) fn capture_prim_arrival(&mut self, trace: PrimTrace) {
        let (task_bps, limit) = match trace {
            PrimTrace::Arrival(rate) => (rate, OBSERVER_HISTORY),
            PrimTrace::Load => (crate::scenario::HANDOFF_TASK_BPS, LOAD_RECEIPT_LIMIT),
        };
        self.arrival_capture = Some(ArrivalCapture {
            limit,
            task_bps,
            start: Instant::now(),
            records: Vec::new(),
        });
    }

    pub(crate) fn take_arrival_records(&mut self) -> Option<Vec<Record>> {
        self.arrival_capture.take().map(|c| c.records)
    }

    pub(crate) async fn send_throttles(
        &mut self,
        task_bps: Option<f32>,
    ) -> Result<(), SessionError> {
        let mut throttle = AgentThrottle::default();
        throttle.agent_data.agent_id = self.agent.agent_id;
        throttle.agent_data.session_id = self.agent.session_id;
        throttle.agent_data.circuit_code = self.agent.circuit_code;
        throttle.throttle.throttles = throttle::initial_throttle_bytes();
        if let Some(rate) = self.land_rate {
            throttle.throttle.throttles[4..8].copy_from_slice(&rate.to_le_bytes());
        }
        if let Some(rate) = task_bps {
            throttle.throttle.throttles[16..20].copy_from_slice(&rate.to_le_bytes());
        }
        self.circuit.send(&throttle, true).await?;
        if let Some(capture) = &mut self.arrival_capture {
            capture.record(Kind::UdpIn, self.circuit.sent_datagram())?;
        }
        Ok(())
    }

    async fn pump_once(&mut self, idle: Duration) -> Result<(), SessionError> {
        self.maybe_ping().await?;
        self.circuit.tick().await;
        let mut buf = [0u8; ferroan_protocol::packet::MAX_DATAGRAM];
        let received = tokio::time::timeout(idle, self.circuit.recv_from(&mut buf)).await;
        let Ok(received) = received else {
            return Ok(());
        };
        let (len, from) = received?;
        self.receive_datagram(&buf[..len], from).await
    }

    async fn receive_datagram(
        &mut self,
        bytes: &[u8],
        from: SocketAddr,
    ) -> Result<(), SessionError> {
        if from != self.circuit.addr {
            return Ok(());
        }
        self.received += 1;
        let Ok(packet) = parse_packet(bytes) else {
            return Ok(());
        };
        if let Some(capture) = &mut self.arrival_capture
            && (packet.id == ObjectUpdate::ID || packet.id == ImprovedTerseObjectUpdate::ID)
        {
            capture.record(Kind::UdpOut, bytes)?;
        }
        if !self
            .circuit
            .on_receive(packet.flags, packet.sequence, &packet.acks)
        {
            self.circuit.flush_acks().await;
            return Ok(());
        }
        self.dispatch(packet).await;
        self.circuit.flush_acks().await;
        Ok(())
    }

    fn accept(&mut self, flags: u8, sequence: u32) {
        self.circuit.accept(flags, sequence);
        if self
            .latest_sequence
            .is_none_or(|previous| newer(sequence, previous))
        {
            self.latest_sequence = Some(sequence);
        }
    }

    async fn maybe_ping(&mut self) -> Result<(), SessionError> {
        if self.last_ping.elapsed() < PING_EVERY {
            return Ok(());
        }
        self.last_ping = Instant::now();
        let (id, oldest) = self.circuit.next_ping();
        let mut ping = StartPingCheck::default();
        ping.ping_id.ping_id = id;
        ping.ping_id.oldest_unacked = oldest;
        self.circuit.send(&ping, false).await?;
        Ok(())
    }

    async fn dispatch(&mut self, packet: IncomingPacket) {
        let flags = packet.flags;
        let sequence = packet.sequence;
        if packet.id == StartPingCheck::ID {
            let Ok(ping) = packet.decode::<StartPingCheck>() else {
                return;
            };
            let mut reply = CompletePingCheck::default();
            reply.ping_id.ping_id = ping.ping_id.ping_id;
            if self.circuit.send(&reply, false).await.is_err() {
                return;
            }
            self.accept(flags, sequence);
            return;
        }
        if packet.id == CompletePingCheck::ID {
            if let Ok(pong) = packet.decode::<CompletePingCheck>() {
                self.circuit.on_ping_reply(pong.ping_id.ping_id);
                self.accept(flags, sequence);
            }
            return;
        }
        if packet.id == PacketAck::ID {
            let Ok(ack) = packet.decode::<PacketAck>() else {
                return;
            };
            self.circuit
                .ack_received(ack.packets.iter().map(|packet| packet.id));
            self.accept(flags, sequence);
            return;
        }
        if packet.id == RegionHandshake::ID {
            let Ok(handshake) = packet.decode::<RegionHandshake>() else {
                return;
            };
            if self
                .generation_floor
                .is_some_and(|floor| !newer(sequence, floor))
            {
                self.accept(flags, sequence);
                return;
            }
            self.region_id = Some(handshake.region_info2.region_id);
            if !self.handshake_sent {
                let mut reply = RegionHandshakeReply::default();
                reply.agent_data.agent_id = self.agent.agent_id;
                reply.agent_data.session_id = self.agent.session_id;
                reply.region_info.flags = HANDSHAKE_FLAGS;
                if self.circuit.send(&reply, true).await.is_err() {
                    return;
                }
                self.handshake_sent = true;
                self.handshake_sequence = Some(sequence);
            }
            self.accept(flags, sequence);
            return;
        }
        if packet.id == LogoutReply::ID {
            if packet.decode::<LogoutReply>().is_ok() {
                self.logged_out = true;
                self.accept(flags, sequence);
            }
            return;
        }
        if packet.id == AgentMovementComplete::ID {
            let Ok(arrival) = packet.decode::<AgentMovementComplete>() else {
                return;
            };
            if arrival.agent_data.agent_id != self.agent.agent_id
                || arrival.agent_data.session_id != self.agent.session_id
                || !packet.reliable()
                || !self.movement_sent
            {
                return;
            }
            // The pinned region sends the handshake before arrival. If UDP
            // reorders them, leave arrival unacked until its reliable resend.
            // The new handshake's sequence also excludes a delayed old visit.
            if !self
                .handshake_sequence
                .is_some_and(|handshake| newer(sequence, handshake))
            {
                return;
            }
            self.arrived_generation = Some(self.generation);
        }
        self.accept(flags, sequence);
        if packet.id == CoarseLocationUpdate::ID {
            if let Ok(message) = packet.decode::<CoarseLocationUpdate>() {
                if self.coarse.len() >= MAX_COARSE {
                    self.coarse.pop_front();
                }
                self.coarse.push_back(message);
            }
            return;
        }
        // Terrain and stats can fill the inbox before a scenario reads
        // AgentMovementComplete. The checks do not use these packets.
        if matches!(
            packet.id,
            id if id == LayerData::ID
                || id == SimStats::ID
                || id == SimulatorViewerTimeMessage::ID
                || id == ImageData::ID
        ) {
            return;
        }
        if self.inbox.len() >= MAX_INBOX {
            self.inbox.pop_front();
        }
        self.inbox.push_back(packet);
    }
}

/// Serial-number comparison across the LLUDP u32 wrap, within half its range.
fn newer(sequence: u32, previous: u32) -> bool {
    let distance = sequence.wrapping_sub(previous);
    distance != 0 && distance < (1 << 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_protocol::msgs::{self, message_info};
    use ferroan_protocol::{Msg, build_packet};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, UdpSocket};

    fn agent() -> Agent {
        Agent {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
        }
    }

    async fn send_msg<M: Msg>(
        sock: &UdpSocket,
        to: SocketAddr,
        seq: &mut u32,
        msg: &M,
        reliable: bool,
    ) {
        let mut bytes = Vec::new();
        build_packet(msg, *seq, reliable, false, &[], &mut bytes);
        *seq = seq.wrapping_add(1);
        sock.send_to(&bytes, to).await.unwrap();
    }

    fn arrival() -> AgentMovementComplete {
        let mut packet = AgentMovementComplete::default();
        packet.agent_data.agent_id = agent().agent_id;
        packet.agent_data.session_id = agent().session_id;
        packet
    }

    async fn fake_region(sock: UdpSocket, send_arrival: bool) -> Vec<String> {
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        let mut seq = 1u32;
        let mut handshake_sent = false;
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            let Ok(Ok((len, from))) =
                tokio::time::timeout(Duration::from_millis(200), sock.recv_from(&mut buf)).await
            else {
                continue;
            };
            let Ok(packet) = parse_packet(&buf[..len]) else {
                continue;
            };
            let name = message_info(packet.id).map(|info| info.name).unwrap_or("");
            seen.push(name.to_owned());
            if packet.reliable() {
                let mut ack = PacketAck::default();
                ack.packets.push(msgs::packet_ack::Packets {
                    id: packet.sequence,
                });
                send_msg(&sock, from, &mut seq, &ack, false).await;
            }
            if name == "UseCircuitCode" && !handshake_sent {
                let code = packet.decode::<UseCircuitCode>().unwrap();
                assert_eq!(code.circuit_code.code, 42);
                assert_eq!(code.circuit_code.id, Uuid::from_u128(1));
                assert_eq!(code.circuit_code.session_id, Uuid::from_u128(2));
                let mut ping = StartPingCheck::default();
                ping.ping_id.ping_id = 3;
                send_msg(&sock, from, &mut seq, &ping, false).await;
                let mut handshake = RegionHandshake::default();
                handshake.region_info2.region_id = Uuid::from_u128(9);
                send_msg(&sock, from, &mut seq, &handshake, true).await;
                handshake_sent = true;
            }
            if name == "RegionHandshakeReply" {
                let reply = packet.decode::<RegionHandshakeReply>().unwrap();
                assert_eq!(reply.region_info.flags, HANDSHAKE_FLAGS);
                assert_eq!(reply.agent_data.agent_id, Uuid::from_u128(1));
            }
            if name == "CompletePingCheck" {
                let pong = packet.decode::<CompletePingCheck>().unwrap();
                assert_eq!(pong.ping_id.ping_id, 3);
            }
            if name == "AgentThrottle" {
                let throttle = packet.decode::<AgentThrottle>().unwrap();
                assert_eq!(throttle.throttle.throttles.len(), 28);
                assert_eq!(throttle.agent_data.circuit_code, 42);
            }
            if name == "AgentHeightWidth" {
                let window = packet.decode::<AgentHeightWidth>().unwrap();
                assert_eq!(window.height_width_block.height, WINDOW_HEIGHT);
                assert_eq!(window.height_width_block.width, WINDOW_WIDTH);
            }
            if name == "CompleteAgentMovement" {
                let movement = packet.decode::<CompleteAgentMovement>().unwrap();
                assert_eq!(movement.agent_data.circuit_code, 42);
                if send_arrival {
                    send_msg(&sock, from, &mut seq, &arrival(), true).await;
                }
            }
            if name == "LogoutRequest" {
                let mut reply = LogoutReply::default();
                reply.agent_data.agent_id = Uuid::from_u128(1);
                reply.agent_data.session_id = Uuid::from_u128(2);
                send_msg(&sock, from, &mut seq, &reply, true).await;
                break;
            }
        }
        seen
    }

    async fn serve_seed(hit: std::sync::Arc<AtomicBool>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let request = crate::body::read_http_request(&mut sock).await;
            let text = String::from_utf8_lossy(&request);
            assert!(text.to_ascii_lowercase().contains("application/llsd+xml"));
            assert!(text.contains("GetTexture"));
            assert!(text.contains("EventQueueGet"));
            hit.store(true, Ordering::SeqCst);
            let body = b"<?xml version=\"1.0\"?><llsd><map><key>GetTexture</key><string>http://127.0.0.1/texture</string><key>EventQueueGet</key><string>http://127.0.0.1/events</string></map></llsd>";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(header.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        });
        format!("http://127.0.0.1:{port}/seed")
    }

    #[tokio::test]
    async fn prim_arrival_records_a_resend_even_when_the_inbox_drops_it() {
        let sim = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer = sim.local_addr().unwrap();
        let mut session = Session::connect(peer, agent()).await.unwrap();
        session.capture_prim_arrival(PrimTrace::Arrival(8000.0));
        let mut first = Vec::new();
        build_packet(&ObjectUpdate::default(), 41, true, false, &[], &mut first);
        let mut resend = first.clone();
        resend[0] |= ferroan_protocol::PacketFlags::RESENT;
        session.receive_datagram(&first, peer).await.unwrap();
        session.receive_datagram(&resend, peer).await.unwrap();
        assert_eq!(session.inbox.len(), 1);
        let records = session.take_arrival_records().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].bytes, first);
        assert_eq!(records[1].bytes, resend);
    }

    async fn handshaken_session() -> (Session, UdpSocket, u32) {
        let sim = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = Session::connect(sim.local_addr().unwrap(), agent())
            .await
            .unwrap();
        let to = session.local_addr().unwrap();
        let mut seq = 1;
        let mut ack = PacketAck::default();
        ack.packets.push(msgs::packet_ack::Packets {
            id: session.ucc_seq,
        });
        send_msg(&sim, to, &mut seq, &ack, false).await;
        let mut handshake = RegionHandshake::default();
        handshake.region_info2.region_id = Uuid::from_u128(9);
        send_msg(&sim, to, &mut seq, &handshake, true).await;
        session.pump_once(Duration::from_millis(100)).await.unwrap();
        session.pump_once(Duration::from_millis(100)).await.unwrap();
        session.send_movement_if_acked().await.unwrap();
        assert!(session.handshake_sent && session.movement_sent);
        (session, sim, seq)
    }

    #[tokio::test]
    async fn seed_and_handshake_without_arrival_do_not_establish_readiness() {
        let sim = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = Session::connect(sim.local_addr().unwrap(), agent())
            .await
            .unwrap();
        let region = tokio::spawn(fake_region(sim, false));
        let hit = std::sync::Arc::new(AtomicBool::new(false));
        let seed = serve_seed(hit.clone()).await;
        let http = crate::login::http_client().unwrap();
        let result = session
            .run_until_ready(&http, &seed, Duration::from_millis(200))
            .await;
        assert!(matches!(
            result,
            Err(SessionError::Timeout("AgentMovementComplete"))
        ));
        assert!(hit.load(Ordering::SeqCst));
        assert!(session.handshake_sent && session.movement_sent);
        assert_eq!(session.arrived_generation, None);
        region.abort();
    }

    #[tokio::test]
    async fn arrival_requires_current_agent_session_peer_and_reliable_packet() {
        let (mut session, sim, mut seq) = handshaken_session().await;
        let to = session.local_addr().unwrap();
        let http = crate::login::http_client().unwrap();
        let mut wrong = arrival();
        wrong.agent_data.agent_id = Uuid::from_u128(111);
        send_msg(&sim, to, &mut seq, &wrong, true).await;
        wrong = arrival();
        wrong.agent_data.session_id = Uuid::from_u128(222);
        send_msg(&sim, to, &mut seq, &wrong, true).await;
        send_msg(&sim, to, &mut seq, &arrival(), false).await;
        let unrelated = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        send_msg(&unrelated, to, &mut seq, &arrival(), true).await;
        assert!(
            session
                .run_until_ready(&http, "", Duration::from_millis(100))
                .await
                .is_err()
        );
        assert_eq!(session.arrived_generation, None);
        assert!(session.poll_inbox().is_none());
        send_msg(&sim, to, &mut seq, &arrival(), true).await;
        session
            .run_until_ready(&http, "", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(session.poll_inbox().unwrap().id, AgentMovementComplete::ID);
    }

    #[tokio::test]
    async fn reordered_arrival_completes_only_when_resent_after_the_handshake() {
        let (mut session, sim, mut seq) = handshaken_session().await;
        session.handshake_sent = false;
        session.handshake_sequence = None;
        let to = session.local_addr().unwrap();
        let mut handshake_seq = seq;
        seq += 1;
        let arrival_seq = seq;
        send_msg(&sim, to, &mut seq, &arrival(), true).await;
        let mut handshake = RegionHandshake::default();
        handshake.region_info2.region_id = Uuid::from_u128(9);
        send_msg(&sim, to, &mut handshake_seq, &handshake, true).await;
        let http = crate::login::http_client().unwrap();
        assert!(
            session
                .run_until_ready(&http, "", Duration::from_millis(100))
                .await
                .is_err()
        );
        let mut bytes = Vec::new();
        build_packet(&arrival(), arrival_seq, true, true, &[], &mut bytes);
        sim.send_to(&bytes, to).await.unwrap();
        session
            .run_until_ready(&http, "", Duration::from_secs(1))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn return_visit_rejects_prior_arrival_and_delayed_old_sequence() {
        let (mut session, sim, mut seq) = handshaken_session().await;
        let to = session.local_addr().unwrap();
        let http = crate::login::http_client().unwrap();
        send_msg(&sim, to, &mut seq, &arrival(), true).await;
        session
            .run_until_ready(&http, "", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(session.arrived_generation, Some(0));
        let region = tokio::spawn(async move {
            let mut buf = [0; 4096];
            let mut movements = 0;
            loop {
                let (len, from) =
                    tokio::time::timeout(Duration::from_secs(2), sim.recv_from(&mut buf))
                        .await
                        .unwrap()
                        .unwrap();
                let packet = parse_packet(&buf[..len]).unwrap();
                if packet.id != CompleteAgentMovement::ID {
                    continue;
                }
                movements += 1;
                if movements != 2 {
                    continue;
                }
                let mut handshake = RegionHandshake::default();
                handshake.region_info2.region_id = Uuid::from_u128(10);
                send_msg(&sim, from, &mut 10, &handshake, true).await;
                send_msg(&sim, from, &mut 9, &arrival(), true).await;
                return sim;
            }
        });
        assert!(
            session
                .resume_as_root(&http, "", Duration::from_millis(200))
                .await
                .is_err()
        );
        assert_eq!(session.region_id(), Some(Uuid::from_u128(10)));
        assert_eq!(session.arrived_generation, None);
        assert!(session.poll_inbox().is_none());
        let sim = region.await.unwrap();
        send_msg(&sim, to, &mut 11, &arrival(), true).await;
        session
            .run_until_ready(&http, "", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(session.arrived_generation, Some(1));
    }

    #[tokio::test]
    async fn buffered_previous_visit_packets_cannot_complete_a_return() {
        let (mut session, sim, mut seq) = handshaken_session().await;
        let to = session.local_addr().unwrap();
        let mut old_handshake = RegionHandshake::default();
        old_handshake.region_info2.region_id = Uuid::from_u128(9);
        send_msg(&sim, to, &mut seq, &old_handshake, true).await;
        send_msg(&sim, to, &mut seq, &arrival(), true).await;
        // Let the reactor mark the queued datagrams readable before resuming.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let http = crate::login::http_client().unwrap();
        assert!(
            session
                .resume_as_root(&http, "", Duration::from_millis(100))
                .await
                .is_err()
        );
        assert_eq!(session.handshake_sequence, None);
        assert_eq!(session.arrived_generation, None);
        assert!(session.poll_inbox().is_none());
    }

    #[test]
    fn arrival_sequence_order_handles_wrap_and_rejects_old_or_equal_values() {
        assert!(newer(0, u32::MAX));
        assert!(!newer(u32::MAX, 0));
        assert!(!newer(7, 7));
        assert!(!newer(6, 7));
    }

    #[tokio::test]
    async fn handshake_answers_the_region_and_logs_out() {
        let sim = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sim_addr = sim.local_addr().unwrap();
        let hit = std::sync::Arc::new(AtomicBool::new(false));
        let seed = serve_seed(hit.clone()).await;
        let region = tokio::spawn(fake_region(sim, true));
        let http = crate::login::http_client().unwrap();
        let mut session = Session::connect(sim_addr, agent()).await.unwrap();
        session
            .run_until_ready(&http, &seed, Duration::from_secs(8))
            .await
            .unwrap();
        assert_eq!(session.region_id(), Some(Uuid::from_u128(9)));
        assert_eq!(
            session.caps().get("GetTexture").map(String::as_str),
            Some("http://127.0.0.1/texture")
        );
        assert!(hit.load(Ordering::SeqCst));
        session.logout(Duration::from_secs(8)).await.unwrap();
        let seen = region.await.unwrap();
        for name in [
            "UseCircuitCode",
            "CompletePingCheck",
            "RegionHandshakeReply",
            "CompleteAgentMovement",
            "AgentThrottle",
            "AgentHeightWidth",
            "LogoutRequest",
        ] {
            assert!(
                seen.iter().any(|got| got == name),
                "missing {name} in {seen:?}"
            );
        }
    }

    #[tokio::test]
    async fn resume_requires_fresh_region_evidence_even_after_seed_success() {
        let sim = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut session = Session::connect(sim.local_addr().unwrap(), agent())
            .await
            .unwrap();
        session.region_id = Some(Uuid::from_u128(9));
        session.handshake_sent = true;
        session.handshake_sequence = Some(10);
        session.arrived_generation = Some(session.generation);
        session.movement_sent = true;
        let mut stale = Vec::new();
        build_packet(&arrival(), 42, false, false, &[], &mut stale);
        session.inbox.push_back(parse_packet(&stale).unwrap());
        let hit = std::sync::Arc::new(AtomicBool::new(false));
        let seed = serve_seed(hit.clone()).await;
        let http = crate::login::http_client().unwrap();
        let result = session
            .resume_as_root(&http, &seed, Duration::from_millis(300))
            .await;
        assert!(hit.load(Ordering::SeqCst), "the seed request completed");
        assert!(matches!(result, Err(SessionError::Timeout("handshake"))));
        assert_eq!(session.region_id(), None);
        assert!(session.poll_inbox().is_none());
    }

    #[tokio::test]
    async fn resume_as_root_sends_complete_agent_movement_again() {
        let sim = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sim_addr = sim.local_addr().unwrap();
        let hit = std::sync::Arc::new(AtomicBool::new(false));
        let seed = serve_seed(hit).await;
        let region = tokio::spawn(async move {
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            let mut seq = 1u32;
            let mut handshake_sent = false;
            let mut movement = 0u32;
            let deadline = Instant::now() + Duration::from_secs(8);
            while Instant::now() < deadline {
                let Ok(Ok((len, from))) =
                    tokio::time::timeout(Duration::from_millis(200), sim.recv_from(&mut buf)).await
                else {
                    continue;
                };
                let Ok(packet) = parse_packet(&buf[..len]) else {
                    continue;
                };
                let name = message_info(packet.id).map(|info| info.name).unwrap_or("");
                seen.push(name.to_owned());
                if packet.reliable() {
                    let mut ack = PacketAck::default();
                    ack.packets.push(msgs::packet_ack::Packets {
                        id: packet.sequence,
                    });
                    send_msg(&sim, from, &mut seq, &ack, false).await;
                }
                if name == "UseCircuitCode" && !handshake_sent {
                    let mut handshake = RegionHandshake::default();
                    handshake.region_info2.region_id = Uuid::from_u128(9);
                    send_msg(&sim, from, &mut seq, &handshake, true).await;
                    handshake_sent = true;
                }
                if name == "CompleteAgentMovement" {
                    movement += 1;
                    if movement == 2 {
                        let mut handshake = RegionHandshake::default();
                        handshake.region_info2.region_id = Uuid::from_u128(10);
                        send_msg(&sim, from, &mut seq, &handshake, true).await;
                    }
                    send_msg(&sim, from, &mut seq, &arrival(), true).await;
                }
                if seen
                    .iter()
                    .filter(|got| got.as_str() == "RegionHandshakeReply")
                    .count()
                    >= 2
                {
                    break;
                }
            }
            (seen, movement)
        });
        let http = crate::login::http_client().unwrap();
        let mut session = Session::connect(sim_addr, agent()).await.unwrap();
        assert_eq!(session.sim_addr(), sim_addr);
        session
            .run_until_ready(&http, &seed, Duration::from_secs(8))
            .await
            .unwrap();
        assert!(session.movement_sent());
        let again = std::sync::Arc::new(AtomicBool::new(false));
        let seed_again = serve_seed(again.clone()).await;
        session
            .resume_as_root(&http, &seed_again, Duration::from_secs(8))
            .await
            .unwrap();
        assert!(again.load(Ordering::SeqCst));
        assert_eq!(session.region_id(), Some(Uuid::from_u128(10)));
        session.drive(Duration::from_millis(800)).await.unwrap();
        let (seen, movement) = region.await.unwrap();
        assert!(
            movement >= 2,
            "CompleteAgentMovement count {movement} in {seen:?}"
        );
        assert!(
            seen.iter()
                .filter(|got| got.as_str() == "RegionHandshakeReply")
                .count()
                >= 2,
            "expected a second handshake reply in {seen:?}"
        );
    }
}
