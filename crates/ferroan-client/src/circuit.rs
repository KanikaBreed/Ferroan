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
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs
//     (PacketReceived, HandleUseCircuitCode, SendAckImmediate, SendAcks,
//      CompletePing, SendPacketFinal, LogoutHandler)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (SendLogoutPacket, CloseWithoutChecks, HandleCompleteAgentMovement)
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs (MakeRootAgent)
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
// Socket-error behavior compared with OpenSimUDPBase.cs (no code copied).
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Initial UseCircuitCode ACKs bypass the ordinary sequence counter.
//! Established ACKs are queued. Shared reliability state removes exhausted
//! packets; C# itself retries until its inactivity timeout.

use crate::{ArrivalConfig, HandshakeConfig};
use ferroan_protocol::msgs::{
    AgentHeightWidth, AgentThrottle, CompleteAgentMovement, CompletePingCheck, LogoutReply,
    LogoutRequest, PacketAck, RegionHandshakeReply, StartPingCheck, UseCircuitCode, logout_reply,
    packet_ack,
};
use ferroan_protocol::packet::{self, MAX_DATAGRAM};
use ferroan_protocol::reliability::{
    MAX_UNACKED, RecentSequences, ReliableWindow, Resend, SequenceCounter,
};
use ferroan_protocol::{IncomingPacket, Msg, Writer};
use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use uuid::Uuid;

const RESEND_AFTER: Duration = Duration::from_millis(200);
const MAX_RESENDS: u8 = 4;
const ACK_INTERVAL: Duration = Duration::from_millis(500);
const MAX_PENDING_ACKS: usize = 4096;
const ACKS_PER_PACKET: usize = 255;

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
    Udp(#[from] io::Error),
    #[error("timed out waiting for logout")]
    Timeout,
    #[error("reliable window is full ({0} unacked)")]
    TooManyUnacked(usize),
    #[error("packet exceeds the datagram limit")]
    TooLarge,
    #[error("arrival position or look-at is not finite")]
    InvalidArrival,
}

// This private boundary permits deterministic peer-error tests without
// depending on OS-specific ICMP timing. It adds no alternative transport.
trait DatagramIo: Sync {
    fn recv_from(
        &self,
        buf: &mut [u8],
    ) -> impl Future<Output = io::Result<(usize, SocketAddr)>> + Send;
    fn send_to(&self, buf: &[u8], to: SocketAddr)
    -> impl Future<Output = io::Result<usize>> + Send;
}
impl DatagramIo for UdpSocket {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        UdpSocket::recv_from(self, buf).await
    }
    async fn send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<usize> {
        UdpSocket::send_to(self, buf, to).await
    }
}
#[derive(Clone)]
struct Payload {
    bytes: Vec<u8>,
    zerocoded: bool,
}
struct State {
    sequence: SequenceCounter,
    peer: Option<SocketAddr>,
    unacked: ReliableWindow<Payload>,
    recent: RecentSequences,
    pending_acks: VecDeque<u32>,
    ack_at: Instant,
    bytes_since_ack: usize,
    handshake: HandshakeConfig,
    arrival: ArrivalConfig,
    arrived: bool,
}
impl State {
    fn new(handshake: HandshakeConfig, arrival: ArrivalConfig) -> Self {
        Self {
            sequence: SequenceCounter::default(),
            peer: None,
            unacked: ReliableWindow::default(),
            recent: RecentSequences::default(),
            pending_acks: VecDeque::new(),
            ack_at: Instant::now() + ACK_INTERVAL,
            bytes_since_ack: 0,
            handshake,
            arrival,
            arrived: false,
        }
    }
    fn time_until_work(&self) -> Duration {
        let now = Instant::now();
        let resend = self.unacked.time_until_due(now, RESEND_AFTER);
        if self.pending_acks.is_empty() {
            resend
        } else {
            resend.min(self.ack_at.saturating_duration_since(now))
        }
    }
    async fn maintain(&mut self, sock: &impl DatagramIo) -> Result<(), CircuitError> {
        let Some(peer) = self.peer else {
            return Ok(());
        };
        let batch = self.unacked.poll(Instant::now(), RESEND_AFTER);
        for (seq, payload) in batch.resend {
            let mut bytes = Vec::new();
            packet::build_raw(
                &payload.bytes,
                payload.zerocoded,
                seq,
                true,
                true,
                &[],
                &mut bytes,
            );
            let _ = sock.send_to(&bytes, peer).await;
        }
        if Instant::now() >= self.ack_at {
            self.flush_acks(sock, peer).await?;
        }
        Ok(())
    }
    async fn send_message<M: Msg>(
        &mut self,
        sock: &impl DatagramIo,
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
        if writer.buf.len() + packet::HEADER_SIZE > MAX_DATAGRAM {
            return Err(CircuitError::TooLarge);
        }
        let payload = Payload {
            bytes: writer.buf,
            zerocoded: M::ZEROCODED,
        };
        let seq = self.sequence.take();
        let mut bytes = Vec::new();
        packet::build_raw(
            &payload.bytes,
            payload.zerocoded,
            seq,
            reliable,
            false,
            &[],
            &mut bytes,
        );
        // Like OpenSimUDPBase, retain the listener after a peer send error.
        // Reliable packets still enter the retry window when sending fails.
        let _ = sock.send_to(&bytes, to).await;
        if reliable {
            self.unacked
                .insert(
                    seq,
                    payload,
                    Instant::now(),
                    Resend {
                        timeout: Some(RESEND_AFTER),
                        max_retries: MAX_RESENDS,
                    },
                )
                .map_err(|error| CircuitError::TooManyUnacked(error.count))?;
        }
        Ok(())
    }
    async fn immediate_ack(sock: &impl DatagramIo, to: SocketAddr, sequence: u32) {
        let ack = PacketAck {
            omitted_trailing_blocks: 0,
            packets: vec![packet_ack::Packets { id: sequence }],
        };
        let mut bytes = Vec::new();
        // SendAckImmediate bypasses CurrentSequence: the default header is zero.
        packet::build_packet(&ack, 0, false, false, &[], &mut bytes);
        let _ = sock.send_to(&bytes, to).await;
    }
    async fn flush_acks(
        &mut self,
        sock: &impl DatagramIo,
        to: SocketAddr,
    ) -> Result<(), CircuitError> {
        while !self.pending_acks.is_empty() {
            let n = self.pending_acks.len().min(ACKS_PER_PACKET);
            let ack = PacketAck {
                omitted_trailing_blocks: 0,
                packets: self
                    .pending_acks
                    .drain(..n)
                    .map(|id| packet_ack::Packets { id })
                    .collect(),
            };
            self.send_message(sock, to, &ack, false).await?;
        }
        self.ack_at = Instant::now() + ACK_INTERVAL;
        Ok(())
    }
    async fn handle(
        &mut self,
        sock: &impl DatagramIo,
        from: SocketAddr,
        expect: &CircuitExpect,
        packet: &IncomingPacket,
        wire_len: usize,
    ) -> Result<bool, CircuitError> {
        if self.peer.is_some_and(|peer| peer != from) {
            return Ok(false);
        }
        if self.peer.is_none() {
            if packet.id != UseCircuitCode::ID {
                return Ok(false);
            }
            Self::immediate_ack(sock, from, packet.sequence).await;
            let Ok(code) = packet.decode::<UseCircuitCode>() else {
                return Ok(false);
            };
            if code.circuit_code.code != expect.circuit_code
                || !same_agent(code.circuit_code.id, code.circuit_code.session_id, expect)
            {
                return Ok(false);
            }
            self.peer = Some(from);
            if packet.reliable() {
                self.recent.note(packet.sequence);
            }
            let handshake = self
                .handshake
                .build(self.handshake.estate_owner == expect.agent_id, false);
            self.send_message(sock, from, &handshake, true).await?;
            return Ok(false);
        }
        // C# validates agent/session IDs before applying appended ACKs.
        // Throttle and height callbacks remain planned. Arrival's circuit-code
        // gate is applied in the movement callback after normal ACK handling.
        if !valid_packet_ids(packet, expect) {
            return Ok(false);
        }
        self.unacked.acknowledge(packet.acks.iter().copied());
        if packet.id == PacketAck::ID {
            if let Ok(ack) = packet.decode::<PacketAck>() {
                self.unacked
                    .acknowledge(ack.packets.iter().map(|block| block.id));
            }
            return Ok(false);
        }
        if packet.reliable() {
            if self.pending_acks.len() == MAX_PENDING_ACKS {
                self.pending_acks.pop_front();
            }
            self.pending_acks.push_back(packet.sequence);
            self.bytes_since_ack = self.bytes_since_ack.saturating_add(wire_len);
            if self.bytes_since_ack > packet::MTU * 2 {
                self.bytes_since_ack -= packet::MTU * 2;
                self.flush_acks(sock, from).await?;
            }
            if !self.recent.note(packet.sequence) {
                return Ok(false);
            }
        }
        if packet.id == CompleteAgentMovement::ID {
            // LLClientView invokes CompleteMovement only for this circuit.
            // ScenePresence.MakeRootAgent suppresses a second root arrival.
            if !self.arrived
                && let Ok(movement) = packet.decode::<CompleteAgentMovement>()
                && movement.agent_data.circuit_code == expect.circuit_code
            {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as u32;
                let reply = self.arrival.build(expect, timestamp);
                self.send_message(sock, from, &reply, true).await?;
                self.arrived = true;
            }
        } else if packet.id == StartPingCheck::ID {
            if let Ok(ping) = packet.decode::<StartPingCheck>() {
                let mut reply = CompletePingCheck::default();
                reply.ping_id.ping_id = ping.ping_id.ping_id;
                self.send_message(sock, from, &reply, false).await?;
            }
        } else if packet.id == LogoutRequest::ID {
            let mut reply = LogoutReply::default();
            reply.agent_data.agent_id = expect.agent_id;
            reply.agent_data.session_id = expect.session_id;
            reply.inventory_data.push(logout_reply::InventoryData {
                item_id: Uuid::nil(),
            });
            self.send_message(sock, from, &reply, true).await?;
            // LogoutHandler sends a reliable reply then CloseWithoutChecks
            // flushes and closes; it does not wait for an acknowledgement.
            return Ok(true);
        }
        Ok(false)
    }
}
fn same_agent(agent_id: Uuid, session_id: Uuid, expect: &CircuitExpect) -> bool {
    agent_id == expect.agent_id && session_id == expect.session_id
}
fn valid_packet_ids(packet: &IncomingPacket, expect: &CircuitExpect) -> bool {
    macro_rules! check {
        ($message:ty) => {
            packet.decode::<$message>().is_ok_and(|message| {
                same_agent(
                    message.agent_data.agent_id,
                    message.agent_data.session_id,
                    expect,
                )
            })
        };
    }
    if packet.id == RegionHandshakeReply::ID {
        check!(RegionHandshakeReply)
    } else if packet.id == CompleteAgentMovement::ID {
        check!(CompleteAgentMovement)
    } else if packet.id == AgentThrottle::ID {
        check!(AgentThrottle)
    } else if packet.id == AgentHeightWidth::ID {
        check!(AgentHeightWidth)
    } else if packet.id == LogoutRequest::ID {
        check!(LogoutRequest)
    } else {
        true
    }
}
/// One UDP listener with the region settings to announce.
pub struct CircuitServer {
    socket: UdpSocket,
    handshake: HandshakeConfig,
}
impl CircuitServer {
    pub async fn bind() -> Result<Self, CircuitError> {
        Self::bind_at(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await
    }
    pub async fn bind_at(address: SocketAddr) -> Result<Self, CircuitError> {
        Ok(Self {
            socket: UdpSocket::bind(address).await?,
            handshake: HandshakeConfig::for_region(Uuid::nil()),
        })
    }
    pub fn with_handshake(mut self, handshake: HandshakeConfig) -> Self {
        self.handshake = handshake;
        self
    }
    pub fn local_addr(&self) -> Result<SocketAddr, CircuitError> {
        Ok(self.socket.local_addr()?)
    }
    /// Serve one authorized viewer until LogoutReply. Datagram errors are
    /// tolerated within the circuit's bounded lifetime.
    #[cfg(test)]
    pub async fn serve_one(
        &self,
        expect: CircuitExpect,
        timeout: Duration,
    ) -> Result<(), CircuitError> {
        self.serve_one_with_arrival(expect, ArrivalConfig::default(), timeout)
            .await
    }

    /// Serve an accepted login with its actual start position and the region's
    /// current handle and channel/version. Sends arrival once after the viewer
    /// completes movement; later scene traffic remains with its task.
    pub async fn serve_one_with_arrival(
        &self,
        expect: CircuitExpect,
        arrival: ArrivalConfig,
        timeout: Duration,
    ) -> Result<(), CircuitError> {
        if !arrival.valid() {
            return Err(CircuitError::InvalidArrival);
        }
        let mut handshake = self.handshake.clone();
        if handshake.region_id.is_nil() {
            handshake.region_id = expect.region_id;
        }
        serve_socket(&self.socket, expect, handshake, arrival, timeout).await
    }
}
async fn serve_socket(
    sock: &impl DatagramIo,
    expect: CircuitExpect,
    handshake: HandshakeConfig,
    arrival: ArrivalConfig,
    timeout: Duration,
) -> Result<(), CircuitError> {
    let mut state = State::new(handshake, arrival);
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; MAX_DATAGRAM];
    loop {
        state.maintain(sock).await?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CircuitError::Timeout);
        }
        let wait = remaining.min(state.time_until_work());
        if wait.is_zero() {
            continue;
        }
        let (len, from) = match tokio::time::timeout(wait, sock.recv_from(&mut buf)).await {
            Ok(Ok(received)) => received,
            Ok(Err(_)) => {
                // Includes Windows WSAECONNRESET; back off to avoid spinning
                // on a persistently failing socket until the bounded deadline.
                tokio::time::sleep(Duration::from_millis(1)).await;
                continue;
            }
            Err(_) => continue,
        };
        let Ok(packet) = packet::parse_packet(&buf[..len]) else {
            continue;
        };
        if state.handle(sock, from, &expect, &packet, len).await? {
            return Ok(());
        }
    }
}

#[cfg(test)]
#[path = "circuit_tests.rs"]
mod regression_tests;

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
