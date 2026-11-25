// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-net/src/circuit.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// Extracted from Ferroan's bot circuit: sequence allocation, bounded duplicate
// history and reliable-packet resend accounting, without networking or framing.
//
// Compared with Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs
//   (HandleUnacked, ResendUnacked) and UnackedPacketCollection.cs
//   (GetExpiredPackets), OpenSim-NGC/OpenSim-Tranquillity
//   @ 858321883f5487a5d192d36ea9d7b90fa468310c.
// C# retries until its client inactivity timeout, without a retry-count cutoff.
// The count-limited Resend policies remain Ferroan/Aurora bot policies;
// insert_until_ack selects the server's inactivity-bounded lifetime instead.

//! Shared LLUDP reliability state. Callers retain their own ack scheduling,
//! packet framing, transport errors and decisions about retry exhaustion.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use ferroan_diag::{self, Kind, UDP_RETRIES, UDP_UNACKED};

pub const MAX_UNACKED: usize = 256;
pub const DEDUPE_WINDOW: usize = 2048;

/// Sequence allocation for ordinary datagrams. Out-of-band server acknowledgments
/// can bypass this counter without consuming the next ordinary sequence.
pub struct SequenceCounter {
    next: u32,
}

impl Default for SequenceCounter {
    fn default() -> Self {
        Self::new(1)
    }
}

impl SequenceCounter {
    pub fn new(first: u32) -> Self {
        Self { next: first }
    }

    pub fn take(&mut self) -> u32 {
        let sequence = self.next;
        self.next = self.next.wrapping_add(1);
        sequence
    }

    pub fn peek(&self) -> u32 {
        self.next
    }
}

#[derive(Default)]
pub struct RecentSequences {
    order: VecDeque<u32>,
    seen: HashSet<u32>,
}

impl RecentSequences {
    pub fn contains(&self, sequence: u32) -> bool {
        self.seen.contains(&sequence)
    }

    /// Remember a decoded packet, returning false when it was already recorded.
    /// Callers decide whether the resend flag is required for duplicate rejection.
    pub fn note(&mut self, sequence: u32) -> bool {
        if self.seen.contains(&sequence) {
            return false;
        }
        if self.order.len() >= DEDUPE_WINDOW
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        self.order.push_back(sequence);
        self.seen.insert(sequence);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resend {
    /// Fixed timeout, or the caller's current RTT-derived timeout when None.
    pub timeout: Option<Duration>,
    pub max_retries: u8,
}

impl Resend {
    pub const DEFAULT: Self = Self {
        timeout: None,
        max_retries: 4,
    };
    /// Aurora's login UseCircuitCode policy, from llstartup.cpp.
    pub const LOGIN_USE_CIRCUIT_CODE: Self = Self {
        timeout: Some(Duration::from_secs(5)),
        max_retries: 3,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("reliable window is full ({count} unacked)")]
pub struct WindowFull {
    pub count: usize,
}

struct Unacked<T> {
    payload: T,
    sent: Instant,
    retries: u8,
    policy: Resend,
    retry_limit: Option<u8>,
}

pub struct ReliableWindow<T> {
    packets: HashMap<u32, Unacked<T>>,
}

impl<T> Default for ReliableWindow<T> {
    fn default() -> Self {
        Self {
            packets: HashMap::new(),
        }
    }
}

pub struct ResendBatch<T> {
    pub resend: Vec<(u32, T)>,
    /// Exhausted packets are removed before this batch is returned, freeing slots.
    pub dropped: Vec<u32>,
}

impl<T> ReliableWindow<T> {
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    pub fn contains(&self, sequence: u32) -> bool {
        self.packets.contains_key(&sequence)
    }

    pub fn retry_count(&self, sequence: u32) -> Option<u8> {
        self.packets.get(&sequence).map(|packet| packet.retries)
    }

    pub fn insert(
        &mut self,
        sequence: u32,
        payload: T,
        now: Instant,
        policy: Resend,
    ) -> Result<(), WindowFull> {
        if self.packets.len() >= MAX_UNACKED && !self.packets.contains_key(&sequence) {
            let count = self.packets.len() as u64;
            ferroan_diag::observe(UDP_UNACKED, MAX_UNACKED as u64, count, "udp");
            ferroan_diag::refuse(UDP_UNACKED, Kind::Soft, MAX_UNACKED as u64, count, "udp");
            return Err(WindowFull {
                count: self.packets.len(),
            });
        }
        self.packets.insert(
            sequence,
            Unacked {
                payload,
                sent: now,
                retries: 0,
                policy,
                retry_limit: Some(policy.max_retries),
            },
        );
        ferroan_diag::observe(
            UDP_UNACKED,
            MAX_UNACKED as u64,
            self.packets.len() as u64,
            "udp",
        );
        Ok(())
    }

    /// Server policy: retain reliable packets until acknowledged or the owning
    /// circuit expires. The caller supplies its current ping-derived RTO to poll.
    /// Retry accounting saturates without changing that policy or the window bound.
    pub fn insert_until_ack(
        &mut self,
        sequence: u32,
        payload: T,
        now: Instant,
    ) -> Result<(), WindowFull> {
        self.insert(sequence, payload, now, Resend::DEFAULT)?;
        if let Some(packet) = self.packets.get_mut(&sequence) {
            packet.retry_limit = None;
        }
        Ok(())
    }

    pub fn acknowledge(&mut self, sequences: impl IntoIterator<Item = u32>) {
        for sequence in sequences {
            self.packets.remove(&sequence);
        }
    }

    /// Count one resend that actually left the socket. A refused throttle does
    /// not call this, so the packet stays due and keeps its retry count.
    pub fn commit_resend(&mut self, sequence: u32, now: Instant) {
        if let Some(packet) = self.packets.get_mut(&sequence) {
            packet.retries = packet.retries.saturating_add(1);
            packet.sent = now;
        }
    }

    pub fn oldest_or(&self, fallback: u32) -> u32 {
        // Pinned UnackedPacketCollection uses a SortedDictionary<uint,...>;
        // its first entry is numeric minimum, including across sequence wrap.
        self.packets.keys().min().copied().unwrap_or(fallback)
    }

    /// Includes the expiry of packets at their retry limit so they cannot remain
    /// permanently in a full window after the final resend.
    pub fn time_until_due(&self, now: Instant, fallback: Duration) -> Duration {
        self.packets
            .values()
            .map(|packet| {
                packet
                    .policy
                    .timeout
                    .unwrap_or(fallback)
                    .saturating_sub(now.saturating_duration_since(packet.sent))
            })
            .min()
            .unwrap_or(Duration::MAX)
    }
}

impl<T: Clone> ReliableWindow<T> {
    /// Update accounting before the caller attempts each send, matching the bot's
    /// bounded best-effort policy even if its UDP send fails.
    pub fn poll(&mut self, now: Instant, fallback: Duration) -> ResendBatch<T> {
        let mut batch = ResendBatch {
            resend: Vec::new(),
            dropped: Vec::new(),
        };
        let mut crossed: Option<(u8, u8)> = None;
        for (&sequence, packet) in &mut self.packets {
            if now.saturating_duration_since(packet.sent)
                < packet.policy.timeout.unwrap_or(fallback)
            {
                continue;
            }
            if let Some(limit) = packet.retry_limit
                && packet.retries >= limit
            {
                batch.dropped.push(sequence);
                if crossed.is_none_or(|(retries, _)| packet.retries >= retries) {
                    crossed = Some((packet.retries, limit));
                }
            } else {
                packet.retries = packet.retries.saturating_add(1);
                packet.sent = now;
                batch.resend.push((sequence, packet.payload.clone()));
            }
        }
        self.acknowledge(batch.dropped.iter().copied());
        if let Some((retries, limit)) = crossed {
            let observed = u64::from(retries);
            #[cfg(test)]
            reported_retry_observed().store(observed, std::sync::atomic::Ordering::Relaxed);
            ferroan_diag::drop_bound(UDP_RETRIES, Kind::Soft, u64::from(limit), observed, "udp");
        }
        batch
    }

    /// Due packets without consuming a retry. Exhausted packets are still
    /// removed. The caller commits each resend it manages to put on the wire.
    pub fn poll_uncommitted(&mut self, now: Instant, fallback: Duration) -> ResendBatch<T> {
        let mut batch = ResendBatch {
            resend: Vec::new(),
            dropped: Vec::new(),
        };
        let mut crossed: Option<(u8, u8)> = None;
        let due: Vec<u32> = self
            .packets
            .iter()
            .filter(|(_, packet)| {
                now.saturating_duration_since(packet.sent)
                    >= packet.policy.timeout.unwrap_or(fallback)
            })
            .map(|(sequence, _)| *sequence)
            .collect();
        for sequence in due {
            let Some(packet) = self.packets.get(&sequence) else {
                continue;
            };
            if let Some(limit) = packet.retry_limit
                && packet.retries >= limit
            {
                if crossed.is_none_or(|(retries, _)| packet.retries >= retries) {
                    crossed = Some((packet.retries, limit));
                }
                batch.dropped.push(sequence);
            } else {
                batch.resend.push((sequence, packet.payload.clone()));
            }
        }
        self.acknowledge(batch.dropped.iter().copied());
        if let Some((retries, limit)) = crossed {
            let observed = u64::from(retries);
            #[cfg(test)]
            reported_retry_observed().store(observed, std::sync::atomic::Ordering::Relaxed);
            ferroan_diag::drop_bound(UDP_RETRIES, Kind::Soft, u64::from(limit), observed, "udp");
        }
        batch
    }
}

#[cfg(test)]
fn reported_retry_observed() -> &'static std::sync::atomic::AtomicU64 {
    use std::sync::atomic::AtomicU64;
    static LAST: AtomicU64 = AtomicU64::new(0);
    &LAST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_allocation_wraps_without_consuming_unallocated_acks() {
        let mut sequence = SequenceCounter::new(u32::MAX);
        assert_eq!(sequence.peek(), u32::MAX);
        assert_eq!(sequence.take(), u32::MAX);
        assert_eq!(sequence.peek(), 0);
        assert_eq!(sequence.take(), 0);
        assert_eq!(sequence.take(), 1);
    }

    #[test]
    fn duplicate_history_is_bounded_without_refreshing_repeated_sequences() {
        let mut recent = RecentSequences::default();
        for sequence in 0..DEDUPE_WINDOW as u32 {
            assert!(recent.note(sequence));
        }
        assert!(!recent.note(0));
        assert!(recent.note(DEDUPE_WINDOW as u32));
        assert!(!recent.contains(0));
        assert!(recent.contains(1));
        assert_eq!(recent.order.len(), DEDUPE_WINDOW);
        assert_eq!(recent.seen.len(), DEDUPE_WINDOW);
    }

    #[test]
    fn exhausted_retries_free_a_full_window_and_acknowledged_packets_do_not_resend() {
        let _lock = retry_observe_lock();
        let now = Instant::now();
        let step = Duration::from_millis(200);
        let policy = Resend {
            timeout: Some(step),
            max_retries: 4,
        };
        let mut window = ReliableWindow::default();
        for sequence in 0..MAX_UNACKED as u32 {
            window.insert(sequence, vec![1, 2, 3], now, policy).unwrap();
        }
        assert_eq!(
            window.insert(999, vec![], now, policy).unwrap_err().count,
            MAX_UNACKED
        );
        window.acknowledge([0]);
        for retry in 1..=4 {
            let batch = window.poll(now + step * retry, step);
            assert_eq!(batch.resend.len(), MAX_UNACKED - 1);
            assert!(batch.dropped.is_empty());
            assert_eq!(window.retry_count(1), Some(retry as u8));
        }
        assert_eq!(window.time_until_due(now + step * 4, step), step);
        let batch = window.poll(now + step * 5, step);
        assert!(batch.resend.is_empty());
        assert_eq!(batch.dropped.len(), MAX_UNACKED - 1);
        assert_eq!(
            reported_retry_observed().load(std::sync::atomic::Ordering::Relaxed),
            4
        );
        assert_ne!(
            reported_retry_observed().load(std::sync::atomic::Ordering::Relaxed),
            batch.dropped.len() as u64
        );
        assert!(window.is_empty());
        window.insert(999, vec![], now, policy).unwrap();
        assert_eq!(window.oldest_or(1000), 999);
    }

    #[test]
    fn fixed_login_policy_and_dynamic_rtt_policy_keep_separate_deadlines() {
        let now = Instant::now();
        let mut window = ReliableWindow::default();
        window
            .insert(1, (), now, Resend::LOGIN_USE_CIRCUIT_CODE)
            .unwrap();
        window.insert(2, (), now, Resend::DEFAULT).unwrap();
        let batch = window.poll(now + Duration::from_secs(4), Duration::from_secs(1));
        assert_eq!(batch.resend, vec![(2, ())]);
        assert_eq!(window.retry_count(1), Some(0));
        assert_eq!(
            window.time_until_due(now + Duration::from_secs(4), Duration::from_secs(1)),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn oldest_matches_csharp_numeric_order_across_sequence_wrap() {
        let now = Instant::now();
        let mut window = ReliableWindow::default();
        for sequence in [u32::MAX - 1, u32::MAX, 0, 1] {
            window.insert_until_ack(sequence, (), now).unwrap();
        }
        assert_eq!(window.oldest_or(99), 0);
        window.acknowledge([u32::MAX - 1, 0]);
        assert_eq!(window.oldest_or(99), 1);
        window.acknowledge([u32::MAX]);
        assert_eq!(window.oldest_or(99), 1);
        window.insert_until_ack(0, (), now).unwrap();
        assert_eq!(window.oldest_or(99), 0);
        window.acknowledge([1, 0]);
        assert_eq!(window.oldest_or(99), 99);
    }

    #[test]
    fn server_policy_retains_packet_after_retry_counter_saturates() {
        let now = Instant::now();
        let mut window = ReliableWindow::default();
        window.insert_until_ack(1, (), now).unwrap();
        for retry in 1..=300 {
            let batch = window.poll(now + Duration::from_secs(retry), Duration::from_secs(1));
            assert_eq!(batch.resend, vec![(1, ())]);
            assert!(batch.dropped.is_empty());
        }
        assert_eq!(window.retry_count(1), Some(u8::MAX));
        assert_eq!(window.len(), 1);
        window.acknowledge([1]);
        assert!(window.is_empty());
    }

    #[test]
    fn a_dropped_packet_reports_the_retry_count_that_crossed() {
        use std::sync::atomic::Ordering;
        let _lock = retry_observe_lock();
        let now = Instant::now();
        let step = Duration::from_millis(50);
        let policy = Resend {
            timeout: Some(step),
            max_retries: 2,
        };
        let mut window = ReliableWindow::default();
        window.insert(7, (), now, policy).unwrap();
        for retry in 1..=2 {
            let batch = window.poll(now + step * retry, step);
            assert!(batch.dropped.is_empty());
            assert_eq!(window.retry_count(7), Some(retry as u8));
        }
        let batch = window.poll(now + step * 3, step);
        assert_eq!(batch.dropped.len(), 1);
        assert_eq!(reported_retry_observed().load(Ordering::Relaxed), 2);
        assert_ne!(
            reported_retry_observed().load(Ordering::Relaxed),
            batch.dropped.len() as u64
        );
    }

    #[test]
    fn uncommitted_poll_keeps_the_retry_until_commit() {
        let _lock = retry_observe_lock();
        let now = Instant::now();
        let step = Duration::from_secs(1);
        let policy = Resend {
            timeout: Some(step),
            max_retries: 1,
        };
        let mut window = ReliableWindow::default();
        window.insert(4, 9_u8, now, policy).unwrap();
        let due = now + step;
        let batch = window.poll_uncommitted(due, step);
        assert_eq!(batch.resend, vec![(4, 9_u8)]);
        assert!(batch.dropped.is_empty());
        assert_eq!(window.retry_count(4), Some(0));
        let again = window.poll_uncommitted(due, step);
        assert_eq!(again.resend, vec![(4, 9_u8)]);
        assert_eq!(window.retry_count(4), Some(0));
        window.commit_resend(4, due);
        assert_eq!(window.retry_count(4), Some(1));
        assert!(window.poll_uncommitted(due, step).resend.is_empty());
        let dropped = window.poll_uncommitted(due + step, step);
        assert_eq!(dropped.dropped, vec![4]);
        assert_eq!(window.retry_count(4), None);
    }

    fn retry_observe_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}
