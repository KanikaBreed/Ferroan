// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-net/src/circuit.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// Sequence numbers, resends, acks and duplicate suppression follow
// llmessage/llcircuit.cpp. Packet-loss statistics and inventory-offer
// receipts are not included.

//! One reliable UDP circuit to a region.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ferroan_protocol::msgs::{self, PacketAck};
use ferroan_protocol::packet::{self, PacketFlags};
use ferroan_protocol::{Msg, Writer};
use tokio::net::UdpSocket;

const MAX_RESENDS: u8 = 4;
const DEDUPE_WINDOW: usize = 2048;
const ACK_FLUSH_INTERVAL: Duration = Duration::from_millis(100);
/// Reliable packets waiting for an ack. Past this, a send is refused.
pub const MAX_UNACKED: usize = 256;
/// Ack ids waiting to be sent. The oldest is dropped past this.
const MAX_PENDING_ACKS: usize = 4096;
/// One PacketAck message lists at most this many ids (the viewer's flush).
const ACKS_PER_PACKET: usize = 250;

/// How a reliable packet is resent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resend {
    /// Fixed resend timeout. `None` uses three round trips, clamped.
    pub timeout: Option<Duration>,
    /// Resends before the packet is given up.
    pub max_retries: u8,
}

impl Resend {
    pub const DEFAULT: Resend = Resend {
        timeout: None,
        max_retries: MAX_RESENDS,
    };
    /// Login `UseCircuitCode` (llstartup.cpp: 3 retries, 5 seconds).
    pub const LOGIN_USE_CIRCUIT_CODE: Resend = Resend {
        timeout: Some(Duration::from_secs(5)),
        max_retries: 3,
    };
}

#[derive(Debug, thiserror::Error)]
pub enum CircuitError {
    #[error("reliable window is full ({0} unacked)")]
    TooManyUnacked(usize),
    #[error("packet is {got} bytes, over the {limit} byte datagram limit")]
    TooLarge { got: usize, limit: usize },
}

struct Unacked {
    payload: Vec<u8>,
    zerocoded: bool,
    sent: Instant,
    retries: u8,
    policy: Resend,
}

pub struct Circuit {
    sock: UdpSocket,
    pub addr: SocketAddr,
    next_seq: u32,
    unacked: HashMap<u32, Unacked>,
    pending_acks: Vec<u32>,
    last_ack_flush: Instant,
    recent: VecDeque<u32>,
    recent_set: HashSet<u32>,
    rtt: Duration,
    ping_id: u8,
    ping_sent: Option<(u8, Instant)>,
    buf: Vec<u8>,
    dropped: Vec<u32>,
}

impl Circuit {
    /// Bind `127.0.0.1` and send to `sim`. The harness regions listen there.
    pub async fn bind(sim: SocketAddr) -> Result<Self, std::io::Error> {
        let sock = UdpSocket::bind("127.0.0.1:0").await?;
        Ok(Self::from_socket(sock, sim))
    }

    pub fn from_socket(sock: UdpSocket, sim: SocketAddr) -> Self {
        Self {
            sock,
            addr: sim,
            next_seq: 1,
            unacked: HashMap::new(),
            pending_acks: Vec::new(),
            last_ack_flush: Instant::now(),
            recent: VecDeque::with_capacity(DEDUPE_WINDOW),
            recent_set: HashSet::with_capacity(DEDUPE_WINDOW),
            rtt: Duration::from_millis(300),
            ping_id: 0,
            ping_sent: None,
            buf: Vec::with_capacity(1500),
            dropped: Vec::new(),
        }
    }

    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.sock.local_addr()
    }

    pub async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), std::io::Error> {
        self.sock.recv_from(buf).await
    }

    fn alloc_seq(&mut self) -> u32 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }

    /// `try_send_to` only succeeds after the socket has been polled ready.
    async fn raw_send(&mut self) -> bool {
        if self.sock.writable().await.is_err() {
            return false;
        }
        self.sock.try_send_to(&self.buf, self.addr).is_ok()
    }

    pub async fn send<M: Msg>(&mut self, msg: &M, reliable: bool) -> Result<u32, CircuitError> {
        self.send_with(msg, reliable.then_some(Resend::DEFAULT))
            .await
    }

    pub async fn send_reliable<M: Msg>(
        &mut self,
        msg: &M,
        policy: Resend,
    ) -> Result<u32, CircuitError> {
        self.send_with(msg, Some(policy)).await
    }

    async fn send_with<M: Msg>(
        &mut self,
        msg: &M,
        reliable: Option<Resend>,
    ) -> Result<u32, CircuitError> {
        if reliable.is_some() && self.unacked.len() >= MAX_UNACKED {
            return Err(CircuitError::TooManyUnacked(self.unacked.len()));
        }
        let mut writer = Writer::new();
        M::ID.encode(&mut writer.buf);
        msg.encode_body(&mut writer);
        let payload = writer.buf;
        if payload.len() + packet::HEADER_SIZE > packet::MAX_DATAGRAM {
            return Err(CircuitError::TooLarge {
                got: payload.len() + packet::HEADER_SIZE,
                limit: packet::MAX_DATAGRAM,
            });
        }
        let zerocoded = M::ZEROCODED;
        let seq = self.alloc_seq();
        let mut buf = std::mem::take(&mut self.buf);
        let appended = packet::build_raw(
            &payload,
            zerocoded,
            seq,
            reliable.is_some(),
            false,
            &self.pending_acks,
            &mut buf,
        );
        self.pending_acks.drain(..appended);
        self.buf = buf;
        self.raw_send().await;
        if let Some(policy) = reliable {
            self.unacked.insert(
                seq,
                Unacked {
                    payload,
                    zerocoded,
                    sent: Instant::now(),
                    retries: 0,
                    policy,
                },
            );
        }
        Ok(seq)
    }

    pub fn is_unacked(&self, seq: u32) -> bool {
        self.unacked.contains_key(&seq)
    }

    pub fn unacked_count(&self) -> usize {
        self.unacked.len()
    }

    /// Header bookkeeping for a datagram. `false` means a resend of a packet
    /// already processed: it is acked again and must not be processed.
    /// A new packet is acked only from [`Circuit::accept`], after it decoded.
    pub fn on_receive(&mut self, flags: u8, seq: u32, acks: &[u32]) -> bool {
        self.ack_received(acks.iter().copied());
        if flags & PacketFlags::RESENT != 0 && self.recent_set.contains(&seq) {
            if flags & PacketFlags::RELIABLE != 0 {
                self.push_ack(seq);
            }
            return false;
        }
        true
    }

    /// The packet decoded. Ack it when it is reliable, and remember it so a
    /// later resend is not processed again.
    pub fn accept(&mut self, flags: u8, seq: u32) {
        if flags & PacketFlags::RELIABLE != 0 {
            self.push_ack(seq);
        }
        if self.recent_set.contains(&seq) {
            return;
        }
        if self.recent.len() >= DEDUPE_WINDOW
            && let Some(old) = self.recent.pop_front()
        {
            self.recent_set.remove(&old);
        }
        self.recent.push_back(seq);
        self.recent_set.insert(seq);
    }

    fn push_ack(&mut self, seq: u32) {
        if self.pending_acks.len() >= MAX_PENDING_ACKS {
            self.pending_acks.remove(0);
        }
        self.pending_acks.push(seq);
    }

    pub fn take_dropped(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.dropped)
    }

    pub fn ack_received(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            self.unacked.remove(&id);
        }
    }

    /// Resend timed-out reliable packets and flush acks that have waited.
    pub async fn tick(&mut self) {
        let now = Instant::now();
        let ping_timeout = (self.rtt * 3).clamp(Duration::from_millis(500), Duration::from_secs(3));
        let mut to_resend = Vec::new();
        let mut to_drop = Vec::new();
        for (seq, unacked) in &self.unacked {
            let timeout = unacked.policy.timeout.unwrap_or(ping_timeout);
            if now.saturating_duration_since(unacked.sent) > timeout {
                if unacked.retries >= unacked.policy.max_retries {
                    to_drop.push(*seq);
                } else {
                    to_resend.push(*seq);
                }
            }
        }
        for seq in to_drop {
            self.unacked.remove(&seq);
            self.dropped.push(seq);
        }
        for seq in to_resend {
            let Some(unacked) = self.unacked.get_mut(&seq) else {
                continue;
            };
            unacked.retries += 1;
            unacked.sent = now;
            let payload = unacked.payload.clone();
            let zerocoded = unacked.zerocoded;
            let mut buf = std::mem::take(&mut self.buf);
            packet::build_raw(&payload, zerocoded, seq, true, true, &[], &mut buf);
            self.buf = buf;
            self.raw_send().await;
        }
        if !self.pending_acks.is_empty()
            && now.saturating_duration_since(self.last_ack_flush) >= ACK_FLUSH_INTERVAL
        {
            self.flush_acks().await;
        }
    }

    pub async fn flush_acks(&mut self) {
        self.last_ack_flush = Instant::now();
        while !self.pending_acks.is_empty() {
            let count = self.pending_acks.len().min(ACKS_PER_PACKET);
            let ids: Vec<u32> = self.pending_acks.drain(..count).collect();
            let message = PacketAck {
                packets: ids
                    .into_iter()
                    .map(|id| msgs::packet_ack::Packets { id })
                    .collect(),
            };
            let mut writer = Writer::new();
            PacketAck::ID.encode(&mut writer.buf);
            message.encode_body(&mut writer);
            let seq = self.alloc_seq();
            let mut buf = std::mem::take(&mut self.buf);
            packet::build_raw(&writer.buf, false, seq, false, false, &[], &mut buf);
            self.buf = buf;
            self.raw_send().await;
        }
    }

    /// The next ping id and the oldest unacked sequence.
    pub fn next_ping(&mut self) -> (u8, u32) {
        self.ping_id = self.ping_id.wrapping_add(1);
        self.ping_sent = Some((self.ping_id, Instant::now()));
        let oldest = self.unacked.keys().min().copied().unwrap_or(self.next_seq);
        (self.ping_id, oldest)
    }

    pub fn on_ping_reply(&mut self, id: u8) -> Option<Duration> {
        let (sent_id, at) = self.ping_sent?;
        if sent_id != id {
            return None;
        }
        let rtt = at.elapsed();
        self.rtt = (self.rtt * 7 + rtt) / 8;
        self.ping_sent = None;
        Some(rtt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_protocol::msgs::UseCircuitCode;
    use ferroan_protocol::{build_packet, parse_packet};

    const RELIABLE: u8 = PacketFlags::RELIABLE;
    const RESENT: u8 = PacketFlags::RELIABLE | PacketFlags::RESENT;

    async fn circuit() -> Circuit {
        Circuit::bind("127.0.0.1:9".parse().unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn an_undecoded_packet_is_not_acked_and_its_resend_is_processed() {
        let mut c = circuit().await;
        assert!(c.on_receive(RELIABLE, 7, &[]));
        assert!(c.pending_acks.is_empty());
        assert!(c.on_receive(RESENT, 7, &[]));
        c.accept(RESENT, 7);
        assert_eq!(c.pending_acks, vec![7]);
    }

    #[tokio::test]
    async fn a_duplicate_resend_is_acked_and_not_processed_again() {
        let mut c = circuit().await;
        assert!(c.on_receive(RELIABLE, 3, &[]));
        c.accept(RELIABLE, 3);
        assert!(!c.on_receive(RESENT, 3, &[]));
        assert_eq!(c.pending_acks, vec![3, 3]);
        assert!(c.on_receive(RELIABLE, 3, &[]));
    }

    #[tokio::test]
    async fn an_unreliable_packet_is_not_acked() {
        let mut c = circuit().await;
        assert!(c.on_receive(0, 1, &[]));
        c.accept(0, 1);
        assert!(c.pending_acks.is_empty());
    }

    #[tokio::test]
    async fn login_use_circuit_code_keeps_its_own_resend_policy() {
        let mut c = circuit().await;
        let message = UseCircuitCode::default();
        let login = c
            .send_reliable(&message, Resend::LOGIN_USE_CIRCUIT_CODE)
            .await
            .unwrap();
        let other = c.send(&message, true).await.unwrap();
        for unacked in c.unacked.values_mut() {
            unacked.sent -= Duration::from_secs(4);
        }
        c.tick().await;
        assert_eq!(c.unacked[&login].retries, 0);
        assert_eq!(c.unacked[&other].retries, 1);
        for _ in 0..4 {
            c.unacked.get_mut(&login).unwrap().sent -= Duration::from_secs(6);
            c.tick().await;
        }
        assert!(!c.is_unacked(login));
        assert_eq!(c.take_dropped(), vec![login]);
        assert!(c.take_dropped().is_empty());
    }

    #[tokio::test]
    async fn a_resend_is_flagged_and_an_appended_ack_clears_it() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut c = circuit().await;
        c.addr = peer.local_addr().unwrap();
        let seq = c
            .send_reliable(&UseCircuitCode::default(), Resend::LOGIN_USE_CIRCUIT_CODE)
            .await
            .unwrap();
        let mut buf = [0u8; 4096];
        let (n, _) = peer.recv_from(&mut buf).await.unwrap();
        let first = parse_packet(&buf[..n]).unwrap();
        assert_eq!(first.sequence, seq);
        assert!(first.reliable());
        assert!(!first.resent());
        c.unacked.get_mut(&seq).unwrap().sent -= Duration::from_secs(6);
        c.tick().await;
        let (n, _) = peer.recv_from(&mut buf).await.unwrap();
        let resent = parse_packet(&buf[..n]).unwrap();
        assert_eq!(resent.sequence, seq);
        assert!(resent.resent());
        let mut ack = Vec::new();
        build_packet(&PacketAck::default(), 1, false, false, &[seq], &mut ack);
        peer.send_to(&ack, c.local_addr().unwrap()).await.unwrap();
        let (n, _) = c.recv_from(&mut buf).await.unwrap();
        let incoming = parse_packet(&buf[..n]).unwrap();
        assert!(c.on_receive(incoming.flags, incoming.sequence, &incoming.acks));
        assert!(!c.is_unacked(seq));
    }
}
