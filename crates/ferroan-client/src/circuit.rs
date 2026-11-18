// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One viewer circuit on a tokio UDP socket.
//!
//! The order follows Tranquillity's receive path. `UseCircuitCode` is acked
//! before the agent is checked, and `RegionHandshake` is sent once when the
//! agent matches. A resend is acked again and does not send a second
//! handshake. Viewer pings get `CompletePingCheck`. `LogoutRequest` gets
//! `LogoutReply`.
//!
//! Read from
//!   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs
//!   HandleUseCircuitCode, SendAckImmediate, CompletePing
//!   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//!   SendRegionHandshake, SendLogoutPacket
//!   OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c
//! Those two files are unchanged at the pinned revision c85783e.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use ferroan_protocol::msgs::{
    AgentHeightWidth, AgentThrottle, CompleteAgentMovement, CompletePingCheck, LogoutReply,
    LogoutRequest, PacketAck, RegionHandshake, RegionHandshakeReply, StartPingCheck,
    UseCircuitCode, logout_reply, packet_ack,
};
use ferroan_protocol::packet::{self, MAX_DATAGRAM};
use ferroan_protocol::{IncomingPacket, Msg, MsgId, Writer};
use tokio::net::UdpSocket;
use uuid::Uuid;

/// How often an unacked reliable packet is sent again.
const RESEND_AFTER: Duration = Duration::from_millis(200);
/// Resends of one reliable packet, then it is left until an ack or logout.
const MAX_RESENDS: u8 = 4;
/// Reliable sequences remembered so a resend is not handled twice.
const DEDUPE_LIMIT: usize = 2048;
/// Reliable packets waiting for an ack.
const MAX_UNACKED: usize = 256;

/// The agent this circuit will accept, and the region id to announce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitExpect {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub circuit_code: u32,
    pub region_id: Uuid,
}

#[derive(Debug, thiserror::Error)]
pub enum CircuitError {
    #[error("udp: {0}")]
    Udp(#[from] std::io::Error),
    #[error("timed out waiting for logout")]
    Timeout,
    #[error("reliable window is full ({0} unacked)")]
    TooManyUnacked(usize),
}

struct Unacked {
    payload: Vec<u8>,
    zerocoded: bool,
    sent: Instant,
    retries: u8,
}

struct State {
    next_seq: u32,
    peer: Option<SocketAddr>,
    unacked: HashMap<u32, Unacked>,
    recent: VecDeque<u32>,
    recent_set: HashSet<u32>,
}

impl State {
    fn new() -> Self {
        Self {
            next_seq: 1,
            peer: None,
            unacked: HashMap::new(),
            recent: VecDeque::new(),
            recent_set: HashSet::new(),
        }
    }

    fn time_until_resend(&self) -> Duration {
        let now = Instant::now();
        let mut soonest: Option<Duration> = None;
        for item in self.unacked.values() {
            if item.retries >= MAX_RESENDS {
                continue;
            }
            let elapsed = now.saturating_duration_since(item.sent);
            let remain = RESEND_AFTER.saturating_sub(elapsed);
            soonest = Some(match soonest {
                Some(so_far) => so_far.min(remain),
                None => remain,
            });
        }
        soonest.unwrap_or(Duration::MAX)
    }

    async fn resend_due(&mut self, sock: &UdpSocket) -> Result<(), CircuitError> {
        let Some(peer) = self.peer else {
            return Ok(());
        };
        let now = Instant::now();
        let due: Vec<u32> = self
            .unacked
            .iter()
            .filter_map(|(&seq, item)| {
                (item.retries < MAX_RESENDS
                    && now.saturating_duration_since(item.sent) >= RESEND_AFTER)
                    .then_some(seq)
            })
            .collect();
        for seq in due {
            let Some(item) = self.unacked.get_mut(&seq) else {
                continue;
            };
            item.retries += 1;
            item.sent = now;
            let payload = item.payload.clone();
            let zerocoded = item.zerocoded;
            let mut out = Vec::new();
            packet::build_raw(&payload, zerocoded, seq, true, true, &[], &mut out);
            sock.send_to(&out, peer).await?;
        }
        Ok(())
    }

    fn apply_ack_ids(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            self.unacked.remove(&id);
        }
    }

    /// `true` the first time `sequence` is seen.
    fn note(&mut self, sequence: u32) -> bool {
        if self.recent_set.contains(&sequence) {
            return false;
        }
        if self.recent.len() >= DEDUPE_LIMIT
            && let Some(old) = self.recent.pop_front()
        {
            self.recent_set.remove(&old);
        }
        self.recent.push_back(sequence);
        self.recent_set.insert(sequence);
        true
    }

    async fn send_message<M: Msg>(
        &mut self,
        sock: &UdpSocket,
        to: SocketAddr,
        message: &M,
        reliable: bool,
    ) -> Result<(), CircuitError> {
        if reliable && self.unacked.len() >= MAX_UNACKED {
            return Err(CircuitError::TooManyUnacked(self.unacked.len()));
        }
        let mut writer = Writer::new();
        M::ID.encode(&mut writer.buf);
        message.encode_body(&mut writer);
        let payload = writer.buf;
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let mut out = Vec::new();
        packet::build_raw(&payload, M::ZEROCODED, seq, reliable, false, &[], &mut out);
        sock.send_to(&out, to).await?;
        if reliable {
            self.unacked.insert(
                seq,
                Unacked {
                    payload,
                    zerocoded: M::ZEROCODED,
                    sent: Instant::now(),
                    retries: 0,
                },
            );
        }
        Ok(())
    }

    async fn send_ack(
        &mut self,
        sock: &UdpSocket,
        to: SocketAddr,
        sequence: u32,
    ) -> Result<(), CircuitError> {
        let mut ack = PacketAck::default();
        ack.packets.push(packet_ack::Packets { id: sequence });
        // Unreliable, matching SendAckImmediate. A lost ack is the viewer's resend.
        self.send_message(sock, to, &ack, false).await
    }

    async fn handle(
        &mut self,
        sock: &UdpSocket,
        from: SocketAddr,
        expect: &CircuitExpect,
        packet: &IncomingPacket,
    ) -> Result<bool, CircuitError> {
        if self.peer.is_some_and(|peer| peer != from) {
            return Ok(false);
        }
        if self.peer.is_none() {
            return self.try_open(sock, from, expect, packet).await;
        }
        self.apply_ack_ids(packet.acks.iter().copied());
        if packet.id == PacketAck::ID
            && let Ok(ack) = packet.decode::<PacketAck>()
        {
            self.apply_ack_ids(ack.packets.iter().map(|block| block.id));
        }
        if packet.reliable() {
            let fresh = self.note(packet.sequence);
            self.send_ack(sock, from, packet.sequence).await?;
            if !fresh {
                return Ok(false);
            }
        }
        if packet.id == PacketAck::ID || packet.id == UseCircuitCode::ID {
            return Ok(false);
        }
        self.on_established(sock, from, expect, packet).await
    }

    async fn try_open(
        &mut self,
        sock: &UdpSocket,
        from: SocketAddr,
        expect: &CircuitExpect,
        packet: &IncomingPacket,
    ) -> Result<bool, CircuitError> {
        if packet.id != UseCircuitCode::ID {
            return Ok(false);
        }
        // Ack even when the agent is refused, so the viewer stops resending.
        if packet.reliable() {
            self.send_ack(sock, from, packet.sequence).await?;
        }
        let Ok(code) = packet.decode::<UseCircuitCode>() else {
            return Ok(false);
        };
        if code.circuit_code.code != expect.circuit_code
            || code.circuit_code.id != expect.agent_id
            || code.circuit_code.session_id != expect.session_id
        {
            return Ok(false);
        }
        // The refused packet was not recorded. This viewer also starts at sequence 1.
        self.peer = Some(from);
        if packet.reliable() {
            self.note(packet.sequence);
        }
        self.send_handshake(sock, from, expect).await?;
        Ok(false)
    }

    async fn send_handshake(
        &mut self,
        sock: &UdpSocket,
        to: SocketAddr,
        expect: &CircuitExpect,
    ) -> Result<(), CircuitError> {
        let mut handshake = RegionHandshake::default();
        handshake.region_info2.region_id = expect.region_id;
        // LLClientView.SendRegionHandshake always writes these two.
        handshake.region_info3.cpu_class_id = 9;
        handshake.region_info3.cpu_ratio = 1;
        self.send_message(sock, to, &handshake, true).await
    }

    async fn on_established(
        &mut self,
        sock: &UdpSocket,
        from: SocketAddr,
        expect: &CircuitExpect,
        packet: &IncomingPacket,
    ) -> Result<bool, CircuitError> {
        if packet.id == StartPingCheck::ID {
            let Ok(ping) = packet.decode::<StartPingCheck>() else {
                return Ok(false);
            };
            let mut reply = CompletePingCheck::default();
            reply.ping_id.ping_id = ping.ping_id.ping_id;
            self.send_message(sock, from, &reply, false).await?;
            return Ok(false);
        }
        if packet.id == LogoutRequest::ID {
            let Ok(request) = packet.decode::<LogoutRequest>() else {
                return Ok(false);
            };
            if request.agent_data.agent_id != expect.agent_id
                || request.agent_data.session_id != expect.session_id
            {
                return Ok(false);
            }
            let mut reply = LogoutReply::default();
            reply.agent_data.agent_id = expect.agent_id;
            reply.agent_data.session_id = expect.session_id;
            // The template says the block is present even when the list is empty.
            reply.inventory_data.push(logout_reply::InventoryData {
                item_id: Uuid::nil(),
            });
            self.send_message(sock, from, &reply, true).await?;
            return Ok(true);
        }
        if is_session_packet(packet.id) {
            let _accepted = session_packet(packet, expect);
        }
        Ok(false)
    }
}

fn is_session_packet(id: MsgId) -> bool {
    id == RegionHandshakeReply::ID
        || id == CompleteAgentMovement::ID
        || id == AgentThrottle::ID
        || id == AgentHeightWidth::ID
}

fn same_agent(agent_id: Uuid, session_id: Uuid, expect: &CircuitExpect) -> bool {
    agent_id == expect.agent_id && session_id == expect.session_id
}

fn same_circuit(
    agent_id: Uuid,
    session_id: Uuid,
    circuit_code: u32,
    expect: &CircuitExpect,
) -> bool {
    same_agent(agent_id, session_id, expect) && circuit_code == expect.circuit_code
}

/// Decode a viewer packet this circuit accepts. A mismatch is ignored.
fn session_packet(packet: &IncomingPacket, expect: &CircuitExpect) -> bool {
    if packet.id == RegionHandshakeReply::ID {
        return packet
            .decode::<RegionHandshakeReply>()
            .is_ok_and(|message| {
                same_agent(
                    message.agent_data.agent_id,
                    message.agent_data.session_id,
                    expect,
                )
            });
    }
    if packet.id == CompleteAgentMovement::ID {
        return packet
            .decode::<CompleteAgentMovement>()
            .is_ok_and(|message| {
                same_circuit(
                    message.agent_data.agent_id,
                    message.agent_data.session_id,
                    message.agent_data.circuit_code,
                    expect,
                )
            });
    }
    if packet.id == AgentThrottle::ID {
        return packet.decode::<AgentThrottle>().is_ok_and(|message| {
            same_circuit(
                message.agent_data.agent_id,
                message.agent_data.session_id,
                message.agent_data.circuit_code,
                expect,
            )
        });
    }
    if packet.id == AgentHeightWidth::ID {
        return packet.decode::<AgentHeightWidth>().is_ok_and(|message| {
            same_circuit(
                message.agent_data.agent_id,
                message.agent_data.session_id,
                message.agent_data.circuit_code,
                expect,
            )
        });
    }
    false
}

/// A UDP socket bound to `127.0.0.1`. [`CircuitServer::serve_one`] runs one circuit.
pub struct CircuitServer {
    socket: UdpSocket,
}

impl CircuitServer {
    /// Bind `127.0.0.1` on a free port.
    pub async fn bind() -> Result<Self, CircuitError> {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        Ok(Self { socket })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, CircuitError> {
        Ok(self.socket.local_addr()?)
    }

    /// Serve the viewer that presents `expect`, until `LogoutReply` is sent.
    ///
    /// Datagrams from any other address are ignored once the circuit is open.
    /// `timeout` covers the whole exchange, including logout.
    pub async fn serve_one(
        self,
        expect: CircuitExpect,
        timeout: Duration,
    ) -> Result<(), CircuitError> {
        let sock = self.socket;
        let mut state = State::new();
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; MAX_DATAGRAM];
        loop {
            state.resend_due(&sock).await?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(CircuitError::Timeout);
            }
            let wait = remaining.min(state.time_until_resend());
            if wait.is_zero() {
                continue;
            }
            let received = match tokio::time::timeout(wait, sock.recv_from(&mut buf)).await {
                Ok(Ok(value)) => value,
                Ok(Err(error)) => return Err(error.into()),
                Err(_) => continue,
            };
            let (len, from) = received;
            let Ok(packet) = packet::parse_packet(&buf[..len]) else {
                continue;
            };
            if state.handle(&sock, from, &expect, &packet).await? {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    use ferroan_protocol::msgs::{
        CompletePingCheck, LogoutReply, LogoutRequest, PacketAck, RegionHandshake, StartPingCheck,
        UseCircuitCode,
    };
    use ferroan_protocol::{Msg, build_packet, parse_packet};
    use tokio::net::UdpSocket;
    use uuid::Uuid;

    use super::{CircuitExpect, CircuitServer};

    fn expect_agent() -> CircuitExpect {
        CircuitExpect {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
            region_id: Uuid::from_u128(9),
        }
    }

    fn use_circuit(agent: u128, session: u128, code: u32) -> UseCircuitCode {
        let mut message = UseCircuitCode::default();
        message.circuit_code.code = code;
        message.circuit_code.id = Uuid::from_u128(agent);
        message.circuit_code.session_id = Uuid::from_u128(session);
        message
    }

    async fn send<M: Msg>(sock: &UdpSocket, to: SocketAddr, seq: u32, message: &M, reliable: bool) {
        let mut bytes = Vec::new();
        build_packet(message, seq, reliable, false, &[], &mut bytes);
        sock.send_to(&bytes, to).await.unwrap();
    }

    async fn recv(
        sock: &UdpSocket,
        buf: &mut [u8],
        budget: Duration,
    ) -> Option<ferroan_protocol::IncomingPacket> {
        let Ok(Ok((len, _))) = tokio::time::timeout(budget, sock.recv_from(buf)).await else {
            return None;
        };
        parse_packet(&buf[..len]).ok()
    }

    #[tokio::test]
    async fn the_bot_completes_the_handshake_and_logs_out() {
        let server = CircuitServer::bind().await.unwrap();
        let addr = server.local_addr().unwrap();
        let expect = expect_agent();
        let running =
            tokio::spawn(async move { server.serve_one(expect, Duration::from_secs(20)).await });
        let http = ferroan_bot::login::http_client().unwrap();
        let mut session = ferroan_bot::Session::connect(
            addr,
            ferroan_bot::Agent {
                agent_id: expect.agent_id,
                session_id: expect.session_id,
                circuit_code: expect.circuit_code,
            },
        )
        .await
        .unwrap();
        session
            .run_until_ready(&http, "", Duration::from_secs(8))
            .await
            .unwrap();
        assert_eq!(session.region_id(), Some(expect.region_id));
        session.logout(Duration::from_secs(8)).await.unwrap();
        running.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_refused_agent_gets_no_handshake_and_a_ping_is_answered() {
        let server = CircuitServer::bind().await.unwrap();
        let addr = server.local_addr().unwrap();
        let expect = expect_agent();
        let running =
            tokio::spawn(async move { server.serve_one(expect, Duration::from_secs(4)).await });

        let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        send(&stranger, addr, 1, &use_circuit(99, 2, 42), true).await;
        let mut buf = [0u8; 4096];
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(350) {
            if let Some(packet) = recv(&stranger, &mut buf, Duration::from_millis(50)).await {
                assert_ne!(
                    packet.id,
                    RegionHandshake::ID,
                    "refused agent was handed a region"
                );
            }
        }

        let viewer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        send(&viewer, addr, 1, &use_circuit(1, 2, 42), true).await;
        let mut handshake = None;
        let start = Instant::now();
        while handshake.is_none() && start.elapsed() < Duration::from_secs(2) {
            let Some(packet) = recv(&viewer, &mut buf, Duration::from_millis(200)).await else {
                continue;
            };
            if packet.id == RegionHandshake::ID {
                handshake = Some(packet);
            }
        }
        let handshake = handshake.expect("RegionHandshake");
        assert!(handshake.reliable());
        let decoded = handshake.decode::<RegionHandshake>().unwrap();
        assert_eq!(decoded.region_info2.region_id, Uuid::from_u128(9));
        assert_eq!(decoded.region_info3.cpu_class_id, 9);
        assert_eq!(decoded.region_info3.cpu_ratio, 1);

        let mut ping = StartPingCheck::default();
        ping.ping_id.ping_id = 11;
        ping.ping_id.oldest_unacked = 1;
        send(&viewer, addr, 2, &ping, false).await;
        let mut pong = None;
        let start = Instant::now();
        while pong.is_none() && start.elapsed() < Duration::from_secs(2) {
            let Some(packet) = recv(&viewer, &mut buf, Duration::from_millis(200)).await else {
                continue;
            };
            if packet.id == CompletePingCheck::ID {
                pong = Some(packet);
            }
        }
        let pong = pong
            .expect("CompletePingCheck")
            .decode::<CompletePingCheck>()
            .unwrap();
        assert_eq!(pong.ping_id.ping_id, 11);

        let mut request = LogoutRequest::default();
        request.agent_data.agent_id = Uuid::from_u128(1);
        request.agent_data.session_id = Uuid::from_u128(2);
        send(&viewer, addr, 3, &request, true).await;
        let mut reply = None;
        let start = Instant::now();
        while reply.is_none() && start.elapsed() < Duration::from_secs(2) {
            let Some(packet) = recv(&viewer, &mut buf, Duration::from_millis(200)).await else {
                continue;
            };
            if packet.id == LogoutReply::ID {
                reply = Some(packet);
            }
        }
        let reply = reply.expect("LogoutReply").decode::<LogoutReply>().unwrap();
        assert_eq!(reply.agent_data.agent_id, Uuid::from_u128(1));
        assert_eq!(reply.agent_data.session_id, Uuid::from_u128(2));
        assert_eq!(reply.inventory_data.len(), 1);
        assert_eq!(reply.inventory_data[0].item_id, Uuid::nil());
        running.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn an_unacked_handshake_is_resent_and_not_sent_twice() {
        let server = CircuitServer::bind().await.unwrap();
        let addr = server.local_addr().unwrap();
        let expect = expect_agent();
        let running =
            tokio::spawn(async move { server.serve_one(expect, Duration::from_secs(4)).await });
        let viewer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        send(&viewer, addr, 1, &use_circuit(1, 2, 42), true).await;
        // Same sequence again: a viewer resend, not a second circuit.
        send(&viewer, addr, 1, &use_circuit(1, 2, 42), true).await;

        let mut buf = [0u8; 4096];
        let mut handshake_seq = None;
        let mut resent = false;
        let mut packet_acks = 0u32;
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(800) {
            let Some(packet) = recv(&viewer, &mut buf, Duration::from_millis(100)).await else {
                continue;
            };
            if packet.id == PacketAck::ID {
                packet_acks += 1;
                assert!(!packet.resent());
            }
            if packet.id == RegionHandshake::ID {
                match handshake_seq {
                    None => handshake_seq = Some(packet.sequence),
                    Some(seq) => assert_eq!(packet.sequence, seq, "a second handshake was sent"),
                }
                if packet.resent() {
                    resent = true;
                }
            }
        }
        assert!(handshake_seq.is_some(), "missing RegionHandshake");
        assert!(resent, "RegionHandshake was not resent");
        assert!(packet_acks >= 1, "UseCircuitCode was not acked");

        let mut request = LogoutRequest::default();
        request.agent_data.agent_id = Uuid::from_u128(1);
        request.agent_data.session_id = Uuid::from_u128(2);
        send(&viewer, addr, 2, &request, true).await;
        let start = Instant::now();
        let mut saw_logout = false;
        while !saw_logout && start.elapsed() < Duration::from_secs(2) {
            if let Some(packet) = recv(&viewer, &mut buf, Duration::from_millis(200)).await
                && packet.id == LogoutReply::ID
            {
                saw_logout = true;
            }
        }
        assert!(saw_logout);
        running.await.unwrap().unwrap();
    }
}
