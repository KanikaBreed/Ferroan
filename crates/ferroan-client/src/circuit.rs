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
//      CompletePing, SendPing, HandleUnacked, OutgoingPacketHandler,
//      SendPacketFinal, LogoutHandler, DeactivateClientDueToTimeout, AddClient)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPClient.cs
//     (constructor, UpdateRoundTrip, TickLastPacketReceived)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (SendLogoutPacket, Close, CloseWithoutChecks, Kick,
//      SendShutdownConnectionNotice, HandleCompleteAgentMovement,
//      HandleAgentPause, HandleAgentResume)
//   Source/OpenSim.Region.ClientStack.LindenUDP/UnackedPacketCollection.cs
//     (Oldest, GetExpiredPackets)
//   Source/OpenSim.Region.Framework/Scenes/Scene.cs
//     (Close, CloseAgent, NewUserConnection)
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs
//     (MakeRootAgent, CompleteMovement)
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Presence/PresenceDetector.cs
//     (OnMakeRootAgent, DoOnMakeRootAgent)
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/GridUser/ActivityDetector.cs
//     (OnMakeRootAgent, DoOnMakeRootAgent)
//   Source/OpenSim.Region.Framework/Scenes/Scene.LoginArrivals.cs
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Socket-error behavior compared with OpenSimUDPBase.cs (no code copied).
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Initial UseCircuitCode ACKs bypass the ordinary sequence counter.
//! Established ACKs are queued. Reliable packets remain until acknowledgment or
//! inactivity, with the pinned server's ping-derived RTO and timer intervals.

use crate::{ArrivalConfig, HandshakeConfig, LandLayer};
use ferroan_diag::{self, Kind, UDP_DATAGRAM, UDP_UNACKED};
use ferroan_protocol::msgs::{
    AgentPause, AgentResume, CompleteAgentMovement, CompletePingCheck, DisableSimulator, KickUser,
    LayerData, LogoutReply, LogoutRequest, PacketAck, StartPingCheck, UseCircuitCode, logout_reply,
    packet_ack, start_ping_check,
};
use ferroan_protocol::packet::{self, MAX_DATAGRAM};
use ferroan_protocol::reliability::{
    MAX_UNACKED, RecentSequences, ReliableWindow, SequenceCounter,
};
use ferroan_protocol::{IncomingPacket, Msg, Writer};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::{Notify, oneshot};
use tokio::time::Instant;
use uuid::Uuid;

const RESEND_AFTER: Duration = Duration::from_millis(1000);
const RESEND_INTERVAL: Duration = Duration::from_millis(100);
const PING_INTERVAL: Duration = Duration::from_secs(5);
const ACK_INTERVAL: Duration = Duration::from_millis(500);
const MAX_PENDING_ACKS: usize = 4096;
const ACKS_PER_PACKET: usize = 255;

/// `[ClientStack.LindenUDP]` timing values. RTO settings use milliseconds in
/// Tranquillity configuration; inactivity settings use seconds. Zero RTO values
/// select the C# defaults before this configuration is constructed.
#[derive(Debug, Clone, Copy)]
pub struct CircuitTiming {
    pub default_rto: Duration,
    pub max_rto: Duration,
    pub ack_timeout: Duration,
    pub paused_ack_timeout: Duration,
}
impl Default for CircuitTiming {
    fn default() -> Self {
        Self {
            default_rto: RESEND_AFTER,
            max_rto: Duration::from_secs(3),
            ack_timeout: Duration::from_secs(60),
            paused_ack_timeout: Duration::from_secs(300),
        }
    }
}
impl CircuitTiming {
    pub fn valid(self) -> bool {
        [
            self.default_rto,
            self.max_rto,
            self.ack_timeout,
            self.paused_ack_timeout,
        ]
        .into_iter()
        .all(|value| !value.is_zero() && value <= Duration::from_millis(i32::MAX as u64))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitExpect {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub circuit_code: u32,
    pub region_id: Uuid,
}

/// One validated root transition, before AgentMovementComplete is sent.
///
/// The region dispatcher completes its bounded Presence/GridUser work under
/// the accepted generation's lifecycle lock, then consumes this token. Dropping
/// the token also releases arrival: C#'s detectors do not turn service failures
/// into a failed movement. Dropping the circuit future cancels the continuation.
/// The token carries no credentials and is emitted at most once per circuit.
pub struct RootArrival {
    completed: oneshot::Sender<()>,
}

impl RootArrival {
    pub fn complete(self) {
        let _ = self.completed.send(());
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CircuitError {
    #[error("udp: {0}")]
    Udp(#[from] io::Error),
    #[error("circuit inactivity timeout")]
    Timeout,
    #[error("reliable window is full ({0} unacked)")]
    TooManyUnacked(usize),
    #[error("packet exceeds the datagram limit")]
    TooLarge,
    #[error("arrival position or look-at is not finite")]
    InvalidArrival,
    #[error("circuit timing must be positive and fit the C# millisecond clock")]
    InvalidTiming,
    #[error("circuit closed")]
    Closed,
}

/// Why the region stopped a live circuit. Ordinary logout is not one of these:
/// `LogoutHandler` closes with `sendStop` false and does not send either packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitStop {
    /// `CloseAgent(id, true)`: reliable `DisableSimulator` to the current peer.
    Disable,
    /// `Scene.Close`: reliable `KickUser`, then `DisableSimulator`.
    Shutdown,
}

type PendingUseCircuit = Mutex<Option<(SocketAddr, Vec<u8>)>>;

/// UDP totals for the region metrics listener. Resends are also packets out.
#[derive(Debug, Default)]
pub struct UdpCounters {
    pub packets_in: AtomicU64,
    pub bytes_in: AtomicU64,
    pub packets_out: AtomicU64,
    pub bytes_out: AtomicU64,
    pub resends: AtomicU64,
    pub unacked: AtomicU64,
}

struct CountingSocket {
    inner: UdpSocket,
    udp: Mutex<Option<Arc<UdpCounters>>>,
}

impl CountingSocket {
    async fn bind(address: SocketAddr) -> io::Result<Self> {
        Ok(Self {
            inner: UdpSocket::bind(address).await?,
            udp: Mutex::new(None),
        })
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn set_counters(&self, udp: Arc<UdpCounters>) {
        *self.udp.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(udp);
    }

    fn counters(&self) -> Option<Arc<UdpCounters>> {
        self.udp
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
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
impl DatagramIo for CountingSocket {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let counters = self.counters();
        let result = self.inner.recv_from(buf).await;
        if let (Ok((size, _)), Some(udp)) = (&result, &counters) {
            udp.packets_in.fetch_add(1, Ordering::Relaxed);
            udp.bytes_in
                .fetch_add(u64::try_from(*size).unwrap_or(u64::MAX), Ordering::Relaxed);
        }
        result
    }
    async fn send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<usize> {
        let counters = self.counters();
        let result = self.inner.send_to(buf, to).await;
        if let (Ok(size), Some(udp)) = (&result, &counters) {
            udp.packets_out.fetch_add(1, Ordering::Relaxed);
            udp.bytes_out
                .fetch_add(u64::try_from(*size).unwrap_or(u64::MAX), Ordering::Relaxed);
        }
        result
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
    root_arrival: Option<oneshot::Sender<RootArrival>>,
    connection_opened: Option<oneshot::Sender<()>>,
    arrival_completion: Option<oneshot::Receiver<()>>,
    timing: CircuitTiming,
    rto: Duration,
    resend_at: Instant,
    ping_at: Instant,
    ping_sequence: u8,
    last_ping: Option<Instant>,
    ping_ms: u32,
    last_received: Instant,
    /// Last numeric-minimum unacked sequence. C# does not clear this when the
    /// window becomes empty, so a later ping still reports it.
    last_oldest: u32,
    paused: bool,
    udp: Option<Arc<UdpCounters>>,
    /// This circuit's last addition to the shared unacked gauge.
    published_unacked: u64,
    /// Land packets waiting for a free reliable slot. A 256 metre map is 256
    /// packets, and `AgentMovementComplete` already occupies one, so the burst
    /// does not fit in `udp.unacked`.
    pending_land: VecDeque<LandLayer>,
}
impl State {
    #[cfg(test)]
    fn new(handshake: HandshakeConfig, arrival: ArrivalConfig) -> Self {
        Self::with_timing(handshake, arrival, CircuitTiming::default())
    }
    fn with_timing(
        handshake: HandshakeConfig,
        mut arrival: ArrivalConfig,
        timing: CircuitTiming,
    ) -> Self {
        let now = Instant::now();
        let pending_land = VecDeque::from(std::mem::take(&mut arrival.land));
        Self {
            sequence: SequenceCounter::default(),
            peer: None,
            unacked: ReliableWindow::default(),
            recent: RecentSequences::default(),
            pending_acks: VecDeque::new(),
            ack_at: now + ACK_INTERVAL,
            bytes_since_ack: 0,
            handshake,
            arrival,
            arrived: false,
            root_arrival: None,
            connection_opened: None,
            arrival_completion: None,
            timing,
            rto: timing.default_rto,
            resend_at: now + RESEND_INTERVAL,
            ping_at: now + PING_INTERVAL,
            ping_sequence: 0,
            last_ping: None,
            ping_ms: 20,
            last_received: now,
            last_oldest: 0,
            paused: false,
            udp: None,
            published_unacked: 0,
            pending_land,
        }
    }

    /// Publish this circuit's window length onto the shared gauge.
    /// The gauge is the sum of these contributions. Drop clears this one.
    fn publish_unacked(&mut self) {
        self.set_unacked_gauge(u64::try_from(self.unacked.len()).unwrap_or(u64::MAX));
    }

    fn set_unacked_gauge(&mut self, waiting: u64) {
        let previous = self.published_unacked;
        if let Some(udp) = &self.udp {
            if waiting > previous {
                udp.unacked.fetch_add(waiting - previous, Ordering::Relaxed);
            } else if waiting < previous {
                udp.unacked.fetch_sub(previous - waiting, Ordering::Relaxed);
            }
        }
        self.published_unacked = waiting;
    }
    fn time_until_work(&self) -> Duration {
        if self.peer.is_some() && !self.pending_land.is_empty() && self.unacked.len() < MAX_UNACKED
        {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let mut wait = self.resend_at.saturating_duration_since(now);
        if self.peer.is_some() {
            wait = wait.min(self.ping_at.saturating_duration_since(now));
            if !self.pending_acks.is_empty() {
                wait = wait.min(self.ack_at.saturating_duration_since(now));
            }
        }
        wait
    }
    async fn maintain(&mut self, sock: &impl DatagramIo) -> Result<(), CircuitError> {
        let now = Instant::now();
        if now >= self.resend_at {
            self.resend_at = now + RESEND_INTERVAL;
            let timeout = if self.paused {
                self.timing.paused_ack_timeout
            } else {
                self.timing.ack_timeout
            };
            // AckTimeout starts when the viewer is mapped, which is when C#
            // constructs LLUDPClient inside HandleUseCircuitCode. LoginConnectWait
            // owns an unopened ViaLogin circuit. serve_one has no peer until then
            // either, so it does not inherit a timeout from the moment serving began.
            if let Some(peer) = self.peer
                && now.duration_since(self.last_received) > timeout
            {
                self.send_disable(sock, peer).await?;
                return Err(CircuitError::Timeout);
            }
            if let Some(peer) = self.peer {
                if !self.unacked.is_empty() {
                    self.last_oldest = self.unacked.oldest_or(self.last_oldest);
                }
                let batch = self.unacked.poll(now.into_std(), self.rto);
                let resent = u64::try_from(batch.resend.len()).unwrap_or(u64::MAX);
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
                if let Some(udp) = &self.udp {
                    udp.resends.fetch_add(resent, Ordering::Relaxed);
                }
                self.publish_unacked();
            }
        }
        let Some(peer) = self.peer else {
            return Ok(());
        };
        if Instant::now() >= self.ack_at {
            self.flush_acks(sock, peer).await?;
        }
        if now >= self.ping_at {
            self.ping_at = now + PING_INTERVAL;
            self.send_ping(sock, peer).await?;
        }
        self.drain_land(sock, peer).await?;
        Ok(())
    }

    async fn drain_land(
        &mut self,
        sock: &impl DatagramIo,
        peer: SocketAddr,
    ) -> Result<(), CircuitError> {
        while !self.pending_land.is_empty() && self.unacked.len() < MAX_UNACKED {
            let Some(layer) = self.pending_land.pop_front() else {
                break;
            };
            let mut message = LayerData::default();
            message.layer_id.type_ = layer.kind;
            message.layer_data.data = layer.data;
            self.send_message(sock, peer, &message, true).await?;
        }
        Ok(())
    }
    async fn send_ping(
        &mut self,
        sock: &impl DatagramIo,
        peer: SocketAddr,
    ) -> Result<(), CircuitError> {
        if !self.unacked.is_empty() {
            self.last_oldest = self.unacked.oldest_or(self.last_oldest);
        }
        let ping = StartPingCheck {
            ping_id: start_ping_check::PingID {
                ping_id: self.ping_sequence,
                oldest_unacked: self.unacked.oldest_or(self.last_oldest),
            },
        };
        self.ping_sequence = self.ping_sequence.wrapping_add(1);
        self.send_message(sock, peer, &ping, false).await?;
        self.last_ping = Some(Instant::now());
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
            let count = self.unacked.len() as u64;
            ferroan_diag::observe(UDP_UNACKED, MAX_UNACKED as u64, count, "udp");
            ferroan_diag::refuse(UDP_UNACKED, Kind::Soft, MAX_UNACKED as u64, count, "udp");
            return Err(CircuitError::TooManyUnacked(self.unacked.len()));
        }
        let mut writer = Writer::new();
        M::ID.encode(&mut writer.buf);
        message.encode_body(&mut writer);
        if writer.buf.len() + packet::HEADER_SIZE > MAX_DATAGRAM {
            ferroan_diag::refuse(
                UDP_DATAGRAM,
                Kind::Hard,
                MAX_DATAGRAM as u64,
                (writer.buf.len() + packet::HEADER_SIZE) as u64,
                "udp",
            );
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
                .insert_until_ack(seq, payload, Instant::now().into_std())
                .map_err(|error| CircuitError::TooManyUnacked(error.count))?;
            self.publish_unacked();
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
    async fn send_disable(
        &mut self,
        sock: &impl DatagramIo,
        to: SocketAddr,
    ) -> Result<(), CircuitError> {
        self.send_message(sock, to, &DisableSimulator::default(), true)
            .await
    }
    async fn send_kick(
        &mut self,
        sock: &impl DatagramIo,
        to: SocketAddr,
        expect: &CircuitExpect,
    ) -> Result<(), CircuitError> {
        // Scene.Kick sets TargetIP and TargetPort to 0. The historical capture's
        // loopback address is not what the pin writes.
        let mut kick = KickUser::default();
        kick.user_info.agent_id = expect.agent_id;
        kick.user_info.session_id = expect.session_id;
        kick.user_info.reason = ferroan_protocol::str_field("The simulator is going down.");
        self.send_message(sock, to, &kick, true).await
    }
    async fn finish_close(
        &mut self,
        sock: &impl DatagramIo,
        expect: &CircuitExpect,
        kind: CircuitStop,
    ) -> Result<(), CircuitError> {
        if let Some(peer) = self.peer {
            if kind == CircuitStop::Shutdown {
                self.send_kick(sock, peer, expect).await?;
            }
            self.send_disable(sock, peer).await?;
        }
        Err(CircuitError::Closed)
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
            // SendAckImmediate answers UseCircuitCode before HandleUseCircuitCode
            // decides whether this endpoint may replace the mapped client.
            // The serve loop also keeps the datagram for the next serve: an ACK
            // alone would stop a reliable viewer from resending it.
            if packet.id == UseCircuitCode::ID {
                Self::immediate_ack(sock, from, packet.sequence).await;
            }
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
            self.last_received = Instant::now();
            if let Some(connected) = self.connection_opened.take() {
                let _ = connected.send(());
            }
            if packet.reliable() {
                self.recent.note(packet.sequence);
            }
            let handshake = self
                .handshake
                .build(self.handshake.estate_owner == expect.agent_id, false);
            self.send_message(sock, from, &handshake, true).await?;
            return Ok(false);
        }
        // C# refreshes the mapped endpoint's inactivity clock before ID
        // validation, then validates agent/session IDs before applying ACKs.
        self.last_received = Instant::now();
        // Throttle and height callbacks remain planned. Arrival's circuit-code
        // gate is applied in the movement callback after normal ACK handling.
        if !valid_packet_ids(packet, expect) {
            return Ok(false);
        }
        self.unacked.acknowledge(packet.acks.iter().copied());
        if packet.id == PacketAck::ID
            && let Ok(ack) = packet.decode::<PacketAck>()
        {
            self.unacked
                .acknowledge(ack.packets.iter().map(|block| block.id));
        }
        self.publish_unacked();
        if packet.id == PacketAck::ID {
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
                // MakeRootAgent marks the child->root transition and triggers
                // its detectors before CompleteMovement sends the arrival.
                // Reserve the transition before starting service reporting so
                // duplicates cannot produce a second notification or packet.
                self.arrived = true;
                if let Some(notify) = self.root_arrival.take() {
                    let (completed, completion) = oneshot::channel();
                    let _ = notify.send(RootArrival { completed });
                    self.arrival_completion = Some(completion);
                } else {
                    self.send_arrival(sock, from, expect).await?;
                }
            }
        } else if packet.id == AgentPause::ID {
            self.paused = true;
        } else if packet.id == AgentResume::ID {
            self.paused = false;
            self.send_ping(sock, from).await?;
        } else if packet.id == CompletePingCheck::ID {
            if packet.decode::<CompletePingCheck>().is_ok()
                && let Some(sent) = self.last_ping
            {
                // Pinned C# uses the most recent ping timestamp without matching
                // PingID, ACK RTT samples, Karn filtering or exponential backoff.
                let elapsed_ms = Instant::now().duration_since(sent).as_secs_f64() * 1000.0;
                self.ping_ms = (0.8 * f64::from(self.ping_ms) + 0.2 * elapsed_ms) as u32;
                let sample = Duration::from_millis(u64::from(self.ping_ms) * 5);
                self.rto = if sample > self.timing.max_rto {
                    self.timing.max_rto
                } else {
                    sample.max(Duration::from_millis(250))
                };
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
            // closes without waiting for an acknowledgement.
            return Ok(true);
        }
        Ok(false)
    }
    async fn send_arrival(
        &mut self,
        sock: &impl DatagramIo,
        to: SocketAddr,
        expect: &CircuitExpect,
    ) -> Result<(), CircuitError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        let reply = self.arrival.build(expect, timestamp);
        let avatar = self.arrival.avatar.clone();
        let region_handle = self.arrival.region_handle;
        let position = self.arrival.position;
        let look_at = self.arrival.look_at;
        self.send_message(sock, to, &reply, true).await?;
        // CompleteMovement sends the avatar full update, then appearance.
        // Land stays queued until the reliable window has room.
        if let Some(avatar) = avatar {
            let update = avatar.object_update(region_handle, position, look_at);
            self.send_message(sock, to, &update, true).await?;
            let appearance = avatar.appearance();
            self.send_message(sock, to, &appearance, true).await?;
        }
        Ok(())
    }
}
fn same_agent(agent_id: Uuid, session_id: Uuid, expect: &CircuitExpect) -> bool {
    agent_id == expect.agent_id && session_id == expect.session_id
}
fn valid_packet_ids(packet: &IncomingPacket, expect: &CircuitExpect) -> bool {
    // NeedValidateIDs is every template message whose AgentData block carries
    // both ids. A short body cannot hold them and is dropped before ACKs.
    let Some(check) = ferroan_protocol::id_check(packet.id) else {
        return true;
    };
    let (Some(agent), Some(session)) = (
        uuid_at(&packet.body, check.agent),
        uuid_at(&packet.body, check.session),
    ) else {
        return false;
    };
    same_agent(agent, session, expect)
}

fn uuid_at(body: &[u8], offset: usize) -> Option<Uuid> {
    let bytes = body.get(offset..offset + 16)?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(bytes);
    Some(Uuid::from_bytes(raw))
}
/// Remembered UseCircuitCode datagrams for logins that have not started serving.
/// One slot per source address, capped at the region root-agent limit.
const PENDING_LOGINS: usize = 40;
const INBOX_CAP: usize = 128;

struct Datagram {
    from: SocketAddr,
    bytes: Vec<u8>,
}

struct Inbox {
    queue: Mutex<VecDeque<Datagram>>,
    notify: Notify,
}

impl Inbox {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
        }
    }

    fn push(&self, datagram: Datagram) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if queue.len() >= INBOX_CAP {
            return;
        }
        queue.push_back(datagram);
        self.notify.notify_one();
    }

    fn pop(&self) -> Option<Datagram> {
        self.queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .pop_front()
    }
}

struct UnboundServe {
    id: u64,
    expect: CircuitExpect,
    inbox: Arc<Inbox>,
}

struct Fan {
    bound: HashMap<SocketAddr, u64>,
    senders: HashMap<u64, Arc<Inbox>>,
    unbound: VecDeque<UnboundServe>,
    pending: VecDeque<Datagram>,
}

struct SharedUdp {
    socket: CountingSocket,
    fan: Mutex<Fan>,
}

struct CircuitInner {
    shared: Arc<SharedUdp>,
    handshake: Mutex<HandshakeConfig>,
    timing: Mutex<CircuitTiming>,
    reader: Mutex<Option<tokio::task::JoinHandle<()>>>,
    next_id: AtomicU64,
}

impl Drop for CircuitInner {
    fn drop(&mut self) {
        if let Some(reader) = self
            .reader
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            reader.abort();
        }
    }
}

/// One viewer's packets, taken from the shared UDP socket.
struct SessionSock {
    shared: Arc<SharedUdp>,
    id: u64,
    inbox: Arc<Inbox>,
}

impl Drop for SessionSock {
    fn drop(&mut self) {
        let mut fan = self
            .shared
            .fan
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        fan.bound.retain(|_, id| *id != self.id);
        fan.senders.remove(&self.id);
        fan.unbound.retain(|waiter| waiter.id != self.id);
    }
}

impl DatagramIo for SessionSock {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        loop {
            if let Some(datagram) = self.inbox.pop() {
                if datagram.bytes.len() > buf.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "datagram exceeds the receive buffer",
                    ));
                }
                buf[..datagram.bytes.len()].copy_from_slice(&datagram.bytes);
                return Ok((datagram.bytes.len(), datagram.from));
            }
            self.inbox.notify.notified().await;
        }
    }

    async fn send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<usize> {
        self.shared.socket.send_to(buf, to).await
    }
}

fn use_circuit(bytes: &[u8]) -> Option<IncomingPacket> {
    let packet = packet::parse_packet(bytes).ok()?;
    (packet.id == UseCircuitCode::ID).then_some(packet)
}

fn expect_matches(packet: &IncomingPacket, expect: &CircuitExpect) -> bool {
    let Ok(code) = packet.decode::<UseCircuitCode>() else {
        return false;
    };
    code.circuit_code.code == expect.circuit_code
        && same_agent(code.circuit_code.id, code.circuit_code.session_id, expect)
}

fn datagram_matches(bytes: &[u8], expect: &CircuitExpect) -> bool {
    use_circuit(bytes).is_some_and(|packet| expect_matches(&packet, expect))
}

/// Route one datagram. Returns a source and sequence to ACK when the datagram
/// is kept for a serve that has not started. A matching waiter ACKs it itself.
fn route(shared: &SharedUdp, from: SocketAddr, bytes: Vec<u8>) -> Option<(SocketAddr, u32)> {
    let mut fan = shared
        .fan
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(id) = fan.bound.get(&from).copied() {
        if let Some(inbox) = fan.senders.get(&id) {
            inbox.push(Datagram { from, bytes });
        }
        return None;
    }
    if bytes.len() > MAX_DATAGRAM {
        ferroan_diag::refuse(
            UDP_DATAGRAM,
            Kind::Hard,
            MAX_DATAGRAM as u64,
            bytes.len() as u64,
            "udp",
        );
        ferroan_diag::log_malformed("udp", &bytes, "datagram exceeds udp.datagram");
        return None;
    }
    let packet = use_circuit(&bytes)?;
    if let Some(pos) = fan
        .unbound
        .iter()
        .position(|waiter| expect_matches(&packet, &waiter.expect))
        && pos < fan.unbound.len()
        && let Some(waiter) = fan.unbound.remove(pos)
    {
        fan.bound.insert(from, waiter.id);
        waiter.inbox.push(Datagram { from, bytes });
        return None;
    }
    fan.pending.retain(|queued| queued.from != from);
    if fan.pending.len() >= PENDING_LOGINS {
        fan.pending.pop_front();
    }
    fan.pending.push_back(Datagram { from, bytes });
    Some((from, packet.sequence))
}

async fn read_loop(shared: Arc<SharedUdp>) {
    let mut buf = [0u8; MAX_DATAGRAM + 1];
    loop {
        let received = shared.socket.recv_from(&mut buf).await;
        let (len, from) = match received {
            Ok(pair) => pair,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(1)).await;
                continue;
            }
        };
        let bytes = buf[..len].to_vec();
        if let Some((to, sequence)) = route(&shared, from, bytes) {
            State::immediate_ack(&shared.socket, to, sequence).await;
        }
    }
}

/// One UDP listener with the region settings to announce.
///
/// Several logins share the socket. Each serve receives only its own peer,
/// plus the UseCircuitCode that opened it. A login packet that arrives before
/// its serve is acknowledged and kept for that serve.
pub struct CircuitServer {
    inner: Arc<CircuitInner>,
}

impl Clone for CircuitServer {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl CircuitServer {
    pub async fn bind() -> Result<Self, CircuitError> {
        Self::bind_at(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await
    }
    pub async fn bind_at(address: SocketAddr) -> Result<Self, CircuitError> {
        Ok(Self {
            inner: Arc::new(CircuitInner {
                shared: Arc::new(SharedUdp {
                    socket: CountingSocket::bind(address).await?,
                    fan: Mutex::new(Fan {
                        bound: HashMap::new(),
                        senders: HashMap::new(),
                        unbound: VecDeque::new(),
                        pending: VecDeque::new(),
                    }),
                }),
                handshake: Mutex::new(HandshakeConfig::for_region(Uuid::nil())),
                timing: Mutex::new(CircuitTiming::default()),
                reader: Mutex::new(None),
                next_id: AtomicU64::new(1),
            }),
        })
    }
    pub fn with_udp_counters(self, udp: Arc<UdpCounters>) -> Self {
        self.inner.shared.socket.set_counters(udp);
        self
    }
    pub fn with_handshake(self, handshake: HandshakeConfig) -> Self {
        *self
            .inner
            .handshake
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = handshake;
        self
    }
    pub fn with_timing(self, timing: CircuitTiming) -> Result<Self, CircuitError> {
        if !timing.valid() {
            return Err(CircuitError::InvalidTiming);
        }
        *self
            .inner
            .timing
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = timing;
        Ok(self)
    }
    pub fn local_addr(&self) -> Result<SocketAddr, CircuitError> {
        Ok(self.inner.shared.socket.local_addr()?)
    }

    fn ensure_reader(&self) {
        let mut reader = self
            .inner
            .reader
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if reader.is_some() {
            return;
        }
        let shared = Arc::clone(&self.inner.shared);
        *reader = Some(tokio::spawn(async move { read_loop(shared).await }));
    }

    fn open_session(&self, expect: CircuitExpect) -> SessionSock {
        self.ensure_reader();
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let inbox = Arc::new(Inbox::new());
        let mut fan = self
            .inner
            .shared
            .fan
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        fan.senders.insert(id, Arc::clone(&inbox));
        if let Some(pos) = fan
            .pending
            .iter()
            .position(|queued| datagram_matches(&queued.bytes, &expect))
            && pos < fan.pending.len()
            && let Some(datagram) = fan.pending.remove(pos)
        {
            fan.bound.insert(datagram.from, id);
            inbox.push(datagram);
        } else {
            fan.unbound.push_back(UnboundServe {
                id,
                expect,
                inbox: Arc::clone(&inbox),
            });
        }
        SessionSock {
            shared: Arc::clone(&self.inner.shared),
            id,
            inbox,
        }
    }
    /// Serve one authorized viewer until LogoutReply. Datagram errors are
    /// tolerated until inactivity. The outer deadline is only a test guard.
    #[cfg(test)]
    pub async fn serve_one(
        &self,
        expect: CircuitExpect,
        timeout: Duration,
    ) -> Result<(), CircuitError> {
        tokio::time::timeout(
            timeout,
            self.serve_one_with_arrival(expect, ArrivalConfig::default()),
        )
        .await
        .map_err(|_| CircuitError::Timeout)?
    }

    /// Serve an accepted login with its actual start position and the region's
    /// current handle and channel/version. Sends arrival once after the viewer
    /// completes movement; later scene traffic remains with its task.
    pub async fn serve_one_with_arrival(
        &self,
        expect: CircuitExpect,
        arrival: ArrivalConfig,
    ) -> Result<(), CircuitError> {
        self.serve_with_reporting(expect, arrival, None, None, None)
            .await
    }

    /// Serve one accepted generation and notify its dispatcher exactly once
    /// when validated movement makes it root. The dispatcher must complete or
    /// drop the token after bounded service work before arrival can be sent.
    /// Keep the receiver local to this generation; do not forward it to a
    /// detached task that could outlive circuit replacement or logout.
    pub async fn serve_one_with_arrival_reporting(
        &self,
        expect: CircuitExpect,
        arrival: ArrivalConfig,
        root_arrival: oneshot::Sender<RootArrival>,
    ) -> Result<(), CircuitError> {
        self.serve_with_reporting(expect, arrival, Some(root_arrival), None, None)
            .await
    }

    /// A ViaLogin circuit waits for the region's login-connect deadline until
    /// its first valid UseCircuitCode; thereafter ordinary inactivity applies.
    pub async fn serve_login(
        &self,
        expect: CircuitExpect,
        arrival: ArrivalConfig,
        root_arrival: oneshot::Sender<RootArrival>,
        connected: oneshot::Sender<()>,
        stop: oneshot::Receiver<CircuitStop>,
    ) -> Result<(), CircuitError> {
        self.serve_with_reporting(
            expect,
            arrival,
            Some(root_arrival),
            Some(connected),
            Some(stop),
        )
        .await
    }

    async fn serve_with_reporting(
        &self,
        expect: CircuitExpect,
        arrival: ArrivalConfig,
        root_arrival: Option<oneshot::Sender<RootArrival>>,
        connected: Option<oneshot::Sender<()>>,
        stop: Option<oneshot::Receiver<CircuitStop>>,
    ) -> Result<(), CircuitError> {
        if !arrival.valid() {
            return Err(CircuitError::InvalidArrival);
        }
        let session = self.open_session(expect);
        let mut handshake = self
            .inner
            .handshake
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        if handshake.region_id.is_nil() {
            handshake.region_id = expect.region_id;
        }
        let timing = *self
            .inner
            .timing
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        serve_socket_reporting(
            &session,
            expect,
            handshake,
            arrival,
            timing,
            self.inner.shared.socket.counters(),
            ServeExtra {
                root_arrival,
                connected,
                // A login that arrived early is already queued on `session`.
                pending: None,
                stop,
            },
        )
        .await
    }
}

/// Signals that belong to one serve and are absent in the plain test helper.
struct ServeExtra<'a> {
    root_arrival: Option<oneshot::Sender<RootArrival>>,
    connected: Option<oneshot::Sender<()>>,
    pending: Option<&'a PendingUseCircuit>,
    stop: Option<oneshot::Receiver<CircuitStop>>,
}

#[cfg(test)]
async fn serve_socket(
    sock: &impl DatagramIo,
    expect: CircuitExpect,
    handshake: HandshakeConfig,
    arrival: ArrivalConfig,
    timing: CircuitTiming,
) -> Result<(), CircuitError> {
    serve_socket_reporting(
        sock,
        expect,
        handshake,
        arrival,
        timing,
        None,
        ServeExtra {
            root_arrival: None,
            connected: None,
            pending: None,
            stop: None,
        },
    )
    .await
}

fn take_stop(stop: &mut Option<oneshot::Receiver<CircuitStop>>) -> Option<CircuitStop> {
    let rx = stop.as_mut()?;
    match rx.try_recv() {
        Ok(kind) => {
            *stop = None;
            Some(kind)
        }
        Err(oneshot::error::TryRecvError::Empty) => None,
        Err(oneshot::error::TryRecvError::Closed) => {
            *stop = None;
            None
        }
    }
}

async fn serve_socket_reporting(
    sock: &impl DatagramIo,
    expect: CircuitExpect,
    handshake: HandshakeConfig,
    arrival: ArrivalConfig,
    timing: CircuitTiming,
    udp: Option<Arc<UdpCounters>>,
    mut extra: ServeExtra<'_>,
) -> Result<(), CircuitError> {
    let mut state = State::with_timing(handshake, arrival, timing);
    state.udp = udp;
    state.root_arrival = extra.root_arrival;
    state.connection_opened = extra.connected;
    // One byte past the cap so Linux `recvfrom` can report an oversize datagram
    // instead of silently truncating it to `MAX_DATAGRAM`.
    let mut buf = [0u8; MAX_DATAGRAM + 1];
    loop {
        state.maintain(sock).await?;
        if let Some(kind) = take_stop(&mut extra.stop) {
            return state.finish_close(sock, &expect, kind).await;
        }
        // Only a serve that has not mapped a peer may consume a UseCircuitCode
        // saved while the previous circuit was still running.
        if state.peer.is_none() {
            let queued = extra.pending.and_then(|slot| {
                // INFALLIBLE: the pending slot is locked only to push or take one datagram.
                #[allow(clippy::unwrap_used)]
                slot.lock().unwrap().take()
            });
            if let Some((from, bytes)) = queued {
                if let Ok(packet) = packet::parse_packet(&bytes)
                    && state
                        .handle(sock, from, &expect, &packet, bytes.len())
                        .await?
                {
                    return Ok(());
                }
                continue;
            }
        }
        let wait = state.time_until_work();
        if wait.is_zero() {
            continue;
        }
        // Reporting belongs to the scene transition, not the UDP transport.
        // Keep ACKs, pings, retries and inactivity alive while services finish.
        // A dropped stop sender is not a shutdown: the branch then stays pending.
        let received = tokio::select! {
            biased;
            stopped = std::future::poll_fn(|cx| match extra.stop.as_mut() {
                Some(rx) => std::pin::Pin::new(rx).poll(cx),
                None => std::task::Poll::Pending,
            }), if extra.stop.is_some() => {
                extra.stop = None;
                if let Ok(kind) = stopped {
                    return state.finish_close(sock, &expect, kind).await;
                }
                continue;
            }
            _ = async {
                match &mut state.arrival_completion {
                    Some(completion) => { let _ = completion.await; }
                    None => std::future::pending::<()>().await,
                }
            } => {
                state.arrival_completion = None;
                if let Some(peer) = state.peer {
                    state.send_arrival(sock, peer, &expect).await?;
                }
                continue;
            }
            received = tokio::time::timeout(wait, sock.recv_from(&mut buf)) => received,
        };
        let (len, from) = match received {
            Ok(Ok(received)) => received,
            Ok(Err(_)) => {
                // Includes Windows WSAECONNRESET; back off to avoid spinning
                // on a persistently failing socket until inactivity expires.
                tokio::time::sleep(Duration::from_millis(1)).await;
                continue;
            }
            Err(_) => continue,
        };
        if len > MAX_DATAGRAM {
            ferroan_diag::refuse(
                UDP_DATAGRAM,
                Kind::Hard,
                MAX_DATAGRAM as u64,
                len as u64,
                "udp",
            );
            ferroan_diag::log_malformed("udp", &buf[..len], "datagram exceeds udp.datagram");
            continue;
        }
        let Ok(packet) = packet::parse_packet(&buf[..len]) else {
            continue;
        };
        if state.peer.is_some_and(|peer| peer != from) && packet.id == UseCircuitCode::ID {
            // Store before the ACK so a test that waits for the ACK can already
            // see the datagram queued for the replacement serve.
            if let Some(slot) = extra.pending {
                // INFALLIBLE: the pending slot is locked only to push or take one datagram.
                #[allow(clippy::unwrap_used)]
                {
                    *slot.lock().unwrap() = Some((from, buf[..len].to_vec()));
                }
            }
            State::immediate_ack(sock, from, packet.sequence).await;
            continue;
        }
        if state.handle(sock, from, &expect, &packet, len).await? {
            return Ok(());
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.set_unacked_gauge(0);
    }
}

#[cfg(test)]
#[path = "circuit_root_tests.rs"]
mod root_tests;

#[cfg(test)]
#[path = "circuit_timing_tests.rs"]
mod timing_tests;

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
    async fn two_viewers_complete_the_handshake_together() {
        let server = CircuitServer::bind().await.unwrap();
        let addr = server.local_addr().unwrap();
        let first = expect_agent();
        let second = CircuitExpect {
            agent_id: Uuid::from_u128(3),
            session_id: Uuid::from_u128(4),
            circuit_code: 43,
            region_id: first.region_id,
        };
        let left_server = server.clone();
        let right_server = server.clone();
        let left_task =
            tokio::spawn(
                async move { left_server.serve_one(first, Duration::from_secs(20)).await },
            );
        let right_task = tokio::spawn(async move {
            right_server
                .serve_one(second, Duration::from_secs(20))
                .await
        });
        let http = ferroan_bot::login::http_client().unwrap();
        let mut left = ferroan_bot::Session::connect(
            addr,
            ferroan_bot::Agent {
                agent_id: first.agent_id,
                session_id: first.session_id,
                circuit_code: first.circuit_code,
            },
        )
        .await
        .unwrap();
        let mut right = ferroan_bot::Session::connect(
            addr,
            ferroan_bot::Agent {
                agent_id: second.agent_id,
                session_id: second.session_id,
                circuit_code: second.circuit_code,
            },
        )
        .await
        .unwrap();
        let (left_ready, right_ready) = tokio::join!(
            left.run_until_ready(&http, "", Duration::from_secs(8)),
            right.run_until_ready(&http, "", Duration::from_secs(8)),
        );
        left_ready.unwrap();
        right_ready.unwrap();
        assert_eq!(left.region_id(), Some(first.region_id));
        assert_eq!(right.region_id(), Some(second.region_id));
        let (left_out, right_out) = tokio::join!(
            left.logout(Duration::from_secs(8)),
            right.logout(Duration::from_secs(8)),
        );
        left_out.unwrap();
        right_out.unwrap();
        left_task.await.unwrap().unwrap();
        right_task.await.unwrap().unwrap();
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
        while start.elapsed() < Duration::from_millis(1400) {
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

    #[tokio::test]
    async fn foreign_use_circuit_code_is_acked_and_served_next() {
        let server = std::sync::Arc::new(CircuitServer::bind().await.unwrap());
        let addr = server.local_addr().unwrap();
        let expect = expect_agent();
        let first = {
            let server = std::sync::Arc::clone(&server);
            tokio::spawn(async move { server.serve_one(expect, Duration::from_secs(5)).await })
        };
        let viewer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        send(&viewer, addr, 1, &use_circuit(1, 2, 42), true).await;
        let mut buf = [0u8; 4096];
        let mut saw_handshake = false;
        let start = Instant::now();
        while !saw_handshake && start.elapsed() < Duration::from_secs(2) {
            if let Some(packet) = recv(&viewer, &mut buf, Duration::from_millis(200)).await
                && packet.id == RegionHandshake::ID
            {
                saw_handshake = true;
            }
        }
        assert!(saw_handshake);

        let other = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        send(&other, addr, 4, &use_circuit(1, 2, 42), true).await;
        let mut acked = false;
        let start = Instant::now();
        while !acked && start.elapsed() < Duration::from_secs(2) {
            if let Some(packet) = recv(&other, &mut buf, Duration::from_millis(200)).await
                && packet.id == PacketAck::ID
            {
                assert_eq!(packet.sequence, 0);
                assert!(!packet.reliable());
                acked = true;
            }
        }
        assert!(acked, "foreign UseCircuitCode was not acked");

        let mut ping = StartPingCheck::default();
        ping.ping_id.ping_id = 3;
        send(&viewer, addr, 2, &ping, false).await;
        let mut pong = false;
        let start = Instant::now();
        while !pong && start.elapsed() < Duration::from_secs(2) {
            if let Some(packet) = recv(&viewer, &mut buf, Duration::from_millis(200)).await
                && packet.id == CompletePingCheck::ID
            {
                pong = true;
            }
        }
        assert!(pong, "mapped viewer lost the circuit");

        let mut request = LogoutRequest::default();
        request.agent_data.agent_id = expect.agent_id;
        request.agent_data.session_id = expect.session_id;
        send(&viewer, addr, 3, &request, true).await;
        first.await.unwrap().unwrap();

        let second = {
            let server = std::sync::Arc::clone(&server);
            tokio::spawn(async move { server.serve_one(expect, Duration::from_secs(5)).await })
        };
        let mut handed = false;
        let start = Instant::now();
        while !handed && start.elapsed() < Duration::from_secs(2) {
            if let Some(packet) = recv(&other, &mut buf, Duration::from_millis(200)).await
                && packet.id == RegionHandshake::ID
            {
                handed = true;
            }
        }
        assert!(handed, "queued UseCircuitCode was not served");
        let mut request = LogoutRequest::default();
        request.agent_data.agent_id = expect.agent_id;
        request.agent_data.session_id = expect.session_id;
        send(&other, addr, 5, &request, true).await;
        second.await.unwrap().unwrap();
    }

    #[test]
    fn the_unacked_gauge_is_the_sum_of_open_circuits_and_clears_on_drop() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use ferroan_protocol::reliability::Resend;

        use super::{Payload, State, UdpCounters};
        use crate::{ArrivalConfig, HandshakeConfig};

        let udp = Arc::new(UdpCounters::default());
        let mut first = State::new(
            HandshakeConfig::for_region(Uuid::nil()),
            ArrivalConfig::default(),
        );
        let mut second = State::new(
            HandshakeConfig::for_region(Uuid::nil()),
            ArrivalConfig::default(),
        );
        first.udp = Some(Arc::clone(&udp));
        second.udp = Some(Arc::clone(&udp));
        let now = std::time::Instant::now();
        let payload = Payload {
            bytes: vec![1],
            zerocoded: false,
        };
        first
            .unacked
            .insert(1, payload.clone(), now, Resend::DEFAULT)
            .unwrap();
        first
            .unacked
            .insert(2, payload.clone(), now, Resend::DEFAULT)
            .unwrap();
        second
            .unacked
            .insert(3, payload, now, Resend::DEFAULT)
            .unwrap();
        first.publish_unacked();
        second.publish_unacked();
        assert_eq!(udp.unacked.load(Ordering::Relaxed), 3);
        first.unacked.acknowledge([1]);
        first.publish_unacked();
        assert_eq!(udp.unacked.load(Ordering::Relaxed), 2);
        drop(first);
        assert_eq!(udp.unacked.load(Ordering::Relaxed), 1);
        drop(second);
        assert_eq!(udp.unacked.load(Ordering::Relaxed), 0);
    }
}
