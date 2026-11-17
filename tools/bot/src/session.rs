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

//! Region handshake and logout on one circuit.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ferroan_protocol::msgs::{
    AgentHeightWidth, AgentThrottle, CompleteAgentMovement, CompletePingCheck, LogoutReply,
    LogoutRequest, PacketAck, RegionHandshake, RegionHandshakeReply, StartPingCheck,
    UseCircuitCode,
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
}

pub struct Session {
    circuit: Circuit,
    agent: Agent,
    caps: HashMap<String, String>,
    ucc_seq: u32,
    movement_sent: bool,
    handshake_sent: bool,
    logged_out: bool,
    region_id: Option<Uuid>,
    inbox: VecDeque<IncomingPacket>,
    last_ping: Instant,
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
            movement_sent: false,
            handshake_sent: false,
            logged_out: false,
            region_id: None,
            inbox: VecDeque::new(),
            last_ping: Instant::now(),
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

    /// The next packet that was not part of the handshake, if one is waiting.
    pub fn poll_inbox(&mut self) -> Option<IncomingPacket> {
        self.inbox.pop_front()
    }

    /// Send `UseCircuitCode`'s follow-up once it is acked, answer the
    /// handshake and pings, and POST the seed. Returns when all three are done.
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
            if self.movement_sent && self.handshake_sent && seed_done {
                return Ok(());
            }
        }
        Err(SessionError::Timeout("handshake"))
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
        let mut movement = CompleteAgentMovement::default();
        movement.agent_data.agent_id = self.agent.agent_id;
        movement.agent_data.session_id = self.agent.session_id;
        movement.agent_data.circuit_code = self.agent.circuit_code;
        self.circuit.send(&movement, true).await?;

        let mut throttle = AgentThrottle::default();
        throttle.agent_data.agent_id = self.agent.agent_id;
        throttle.agent_data.session_id = self.agent.session_id;
        throttle.agent_data.circuit_code = self.agent.circuit_code;
        throttle.throttle.throttles = throttle::initial_throttle_bytes();
        self.circuit.send(&throttle, true).await?;

        let mut window = AgentHeightWidth::default();
        window.agent_data.agent_id = self.agent.agent_id;
        window.agent_data.session_id = self.agent.session_id;
        window.agent_data.circuit_code = self.agent.circuit_code;
        window.height_width_block.height = WINDOW_HEIGHT;
        window.height_width_block.width = WINDOW_WIDTH;
        self.circuit.send(&window, true).await?;
        self.movement_sent = true;
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
        if from != self.circuit.addr {
            return Ok(());
        }
        let Ok(packet) = parse_packet(&buf[..len]) else {
            return Ok(());
        };
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
            self.circuit.accept(flags, sequence);
            return;
        }
        if packet.id == CompletePingCheck::ID {
            if let Ok(pong) = packet.decode::<CompletePingCheck>() {
                self.circuit.on_ping_reply(pong.ping_id.ping_id);
                self.circuit.accept(flags, sequence);
            }
            return;
        }
        if packet.id == PacketAck::ID {
            let Ok(ack) = packet.decode::<PacketAck>() else {
                return;
            };
            self.circuit
                .ack_received(ack.packets.iter().map(|packet| packet.id));
            self.circuit.accept(flags, sequence);
            return;
        }
        if packet.id == RegionHandshake::ID {
            let Ok(handshake) = packet.decode::<RegionHandshake>() else {
                return;
            };
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
            }
            self.circuit.accept(flags, sequence);
            return;
        }
        if packet.id == LogoutReply::ID {
            if packet.decode::<LogoutReply>().is_ok() {
                self.logged_out = true;
                self.circuit.accept(flags, sequence);
            }
            return;
        }
        self.circuit.accept(flags, sequence);
        if self.inbox.len() >= MAX_INBOX {
            self.inbox.pop_front();
        }
        self.inbox.push_back(packet);
    }
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

    async fn fake_region(sock: UdpSocket) -> Vec<String> {
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
    async fn handshake_answers_the_region_and_logs_out() {
        let sim = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sim_addr = sim.local_addr().unwrap();
        let hit = std::sync::Arc::new(AtomicBool::new(false));
        let seed = serve_seed(hit.clone()).await;
        let region = tokio::spawn(fake_region(sim));
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
}
