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
//   Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/EventQueueGetModule.cs
//     (OnRegisterCaps, HasEvents, GetEvents, NoEvents, ClientClosed)
//   Source/OpenSim.Framework.Servers.HttpServer/PollServiceRequestManager.cs
//     (PoolWorkerJob)
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
// Ferroan changes: bounded event bytes/count and concurrent poll count.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The ordinary, newly registered agent queue. Re-registration markers and
//! crossing queue reuse remain with the crossing task. C# may advance/reset
//! ids for a marker-only batch even though it returns NoEvents; a truly empty
//! poll does not advance. Negative stored ids are negated before delivery,
//! including unchecked i32::MIN negation, then advanced with unchecked addition.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferroan_diag::{self, EVENT_BYTES, EVENT_QUEUE, EVENT_QUEUE_BYTES, Kind};
use ferroan_llsd::{Llsd, to_xml};
use tokio::sync::{Notify, Semaphore};
use tokio::time::Instant;
use uuid::Uuid;

pub(crate) const HOLD: Duration = Duration::from_secs(58);
const MAX_EVENTS: usize = 256;
const MAX_EVENT_BYTES: usize = 64 * 1024;
const MAX_QUEUE_BYTES: usize = 1024 * 1024;
const MAX_PENDING_POLLS: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum EventQueueError {
    #[error("the event queue is closed")]
    Closed,
    #[error("the event queue is full")]
    Full,
    #[error("the event exceeds the size or nesting limit")]
    EventLimit,
}

struct State {
    events: VecDeque<Vec<u8>>,
    bytes: usize,
    next_id: i32,
    closed: bool,
}

#[derive(Clone)]
pub(crate) struct EventQueue {
    state: Arc<Mutex<State>>,
    changed: Arc<Notify>,
    polls: Arc<Semaphore>,
}

pub(crate) enum PollReply {
    Events(Vec<u8>),
    Empty,
    Closed,
    Busy,
}

impl EventQueue {
    pub(crate) fn new() -> Self {
        Self::with_id(random_id())
    }

    fn with_id(next_id: i32) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                events: VecDeque::new(),
                bytes: 0,
                next_id,
                closed: false,
            })),
            changed: Arc::new(Notify::new()),
            polls: Arc::new(Semaphore::new(MAX_PENDING_POLLS)),
        }
    }

    pub(crate) fn enqueue(&self, event: Llsd) -> Result<(), EventQueueError> {
        check_size(&event, 0, &mut 0, &mut 0)?;
        let encoded = to_xml(&event);
        if encoded.len() > MAX_EVENT_BYTES {
            ferroan_diag::refuse(
                EVENT_BYTES,
                Kind::Hard,
                MAX_EVENT_BYTES as u64,
                encoded.len() as u64,
                "event-queue",
            );
            return Err(EventQueueError::EventLimit);
        }
        // to_xml always emits exactly one <llsd>...</llsd> wrapper.
        let inner = encoded[6..encoded.len() - 7].to_vec();
        self.push(inner)
    }

    /// Queue one element that is already encoded. `to_xml` is not applied.
    pub(crate) fn enqueue_bytes(&self, element: &[u8]) -> Result<(), EventQueueError> {
        if element.len() > MAX_EVENT_BYTES {
            ferroan_diag::refuse(
                EVENT_BYTES,
                Kind::Hard,
                MAX_EVENT_BYTES as u64,
                element.len() as u64,
                "event-queue",
            );
            return Err(EventQueueError::EventLimit);
        }
        self.push(element.to_vec())
    }

    pub(crate) fn queued(&self) -> Vec<Vec<u8>> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.events.iter().cloned().collect()
    }

    fn push(&self, element: Vec<u8>) -> Result<(), EventQueueError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.closed {
            return Err(EventQueueError::Closed);
        }
        if state.events.len() >= MAX_EVENTS {
            ferroan_diag::refuse(
                EVENT_QUEUE,
                Kind::Soft,
                MAX_EVENTS as u64,
                state.events.len() as u64,
                "event-queue",
            );
            return Err(EventQueueError::Full);
        }
        if state.bytes + element.len() > MAX_QUEUE_BYTES {
            ferroan_diag::refuse(
                EVENT_QUEUE_BYTES,
                Kind::Soft,
                MAX_QUEUE_BYTES as u64,
                (state.bytes + element.len()) as u64,
                "event-queue",
            );
            return Err(EventQueueError::Full);
        }
        state.bytes += element.len();
        state.events.push_back(element);
        let queued = state.events.len() as u64;
        let queued_bytes = state.bytes as u64;
        drop(state);
        ferroan_diag::observe(EVENT_QUEUE, MAX_EVENTS as u64, queued, "event-queue");
        ferroan_diag::observe(
            EVENT_QUEUE_BYTES,
            MAX_QUEUE_BYTES as u64,
            queued_bytes,
            "event-queue",
        );
        self.changed.notify_waiters();
        Ok(())
    }

    pub(crate) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.closed = true;
        state.events.clear();
        state.bytes = 0;
        drop(state);
        self.changed.notify_waiters();
    }

    pub(crate) async fn poll(&self) -> PollReply {
        let Ok(_permit) = self.polls.try_acquire() else {
            return PollReply::Busy;
        };
        let deadline = Instant::now() + HOLD;
        loop {
            // Register before looking at the queue so an enqueue between the
            // lookup and await cannot leave a poll asleep with events available.
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                if state.closed {
                    return PollReply::Closed;
                }
                // PollServiceRequestManager checks HasEvents before timeout.
                if !state.events.is_empty() {
                    let id = if state.next_id < 0 {
                        state.next_id.wrapping_neg()
                    } else {
                        state.next_id
                    };
                    state.next_id = id.wrapping_add(1);
                    let mut body = Vec::with_capacity(state.bytes + 100);
                    body.extend_from_slice(b"<llsd><map><key>events</key><array>");
                    for event in state.events.drain(..) {
                        body.extend_from_slice(&event);
                    }
                    body.extend_from_slice(
                        format!("</array><key>id</key><integer>{id}</integer></map></llsd>")
                            .as_bytes(),
                    );
                    state.bytes = 0;
                    return PollReply::Events(body);
                }
                if Instant::now() >= deadline {
                    return PollReply::Empty;
                }
            }
            tokio::select! {
                _ = changed => {},
                _ = tokio::time::sleep_until(deadline) => {},
            }
        }
    }
}

fn random_id() -> i32 {
    // First four UUIDv4 bytes are uniform random bytes (version bits are later).
    // Rejection avoids bias from reducing a 32-bit draw modulo 30,000,000.
    const LIMIT: u32 = 30_000_000;
    const ACCEPT_BELOW: u32 = u32::MAX - u32::MAX % LIMIT;
    loop {
        let bytes = *Uuid::new_v4().as_bytes();
        let draw = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if draw < ACCEPT_BELOW {
            return (draw % LIMIT) as i32;
        }
    }
}

fn check_size(
    value: &Llsd,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
) -> Result<(), EventQueueError> {
    *nodes += 1;
    if depth > 32 || *nodes > 4096 {
        return Err(EventQueueError::EventLimit);
    }
    match value {
        Llsd::String(text) | Llsd::Uri(text) => *bytes = bytes.saturating_add(text.len()),
        Llsd::Binary(data) => *bytes = bytes.saturating_add(data.len()),
        Llsd::Array(values) => {
            for value in values {
                check_size(value, depth + 1, nodes, bytes)?;
            }
        }
        Llsd::Map(values) => {
            for (key, value) in values {
                *bytes = bytes.saturating_add(key.len());
                check_size(value, depth + 1, nodes, bytes)?;
            }
        }
        _ => {}
    }
    if *bytes > MAX_EVENT_BYTES {
        return Err(EventQueueError::EventLimit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_llsd::from_xml;

    fn event() -> Llsd {
        let mut event = Llsd::new_map();
        event.insert("message", "TestEvent");
        event.insert("body", Llsd::new_map());
        event
    }

    #[tokio::test]
    async fn raw_elements_keep_the_empty_forms() {
        let queue = EventQueue::with_id(1);
        let element = b"<map><key>AuctionID</key><integer /><key>AuthBuyerID</key><uuid /></map>";
        queue.enqueue_bytes(element).unwrap();
        let PollReply::Events(body) = queue.poll().await else {
            panic!("expected events");
        };
        assert!(body.windows(element.len()).any(|window| window == element));
        assert!(
            !body
                .windows(b"<integer>0</integer>".len())
                .any(|window| window == b"<integer>0</integer>")
        );
    }

    fn id(reply: PollReply) -> i32 {
        let PollReply::Events(body) = reply else {
            panic!("expected events");
        };
        from_xml(&body).unwrap().get("id").as_i32()
    }

    #[tokio::test(start_paused = true)]
    async fn empty_polls_hold_without_advancing_the_id_and_wake_on_close() {
        let queue = EventQueue::with_id(7);
        let polling = queue.clone();
        let task = tokio::spawn(async move { polling.poll().await });
        tokio::task::yield_now().await;
        tokio::time::advance(HOLD - Duration::from_millis(1)).await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(matches!(task.await.unwrap(), PollReply::Empty));
        queue.enqueue(event()).unwrap();
        assert_eq!(id(queue.poll().await), 7);
        let polling = queue.clone();
        let task = tokio::spawn(async move { polling.poll().await });
        tokio::task::yield_now().await;
        queue.close();
        assert!(matches!(task.await.unwrap(), PollReply::Closed));
    }

    #[tokio::test(start_paused = true)]
    async fn events_win_at_the_deadline_and_ids_follow_csharp_unchecked_arithmetic() {
        let queue = EventQueue::with_id(i32::MAX);
        let polling = queue.clone();
        let task = tokio::spawn(async move { polling.poll().await });
        tokio::task::yield_now().await;
        tokio::time::advance(HOLD).await;
        queue.enqueue(event()).unwrap();
        queue.enqueue(event()).unwrap();
        assert_eq!(id(task.await.unwrap()), i32::MAX);
        queue.enqueue(event()).unwrap();
        assert_eq!(id(queue.poll().await), i32::MIN);
        queue.enqueue(event()).unwrap();
        assert_eq!(id(queue.poll().await), i32::MAX);
    }

    #[tokio::test(start_paused = true)]
    async fn event_count_bytes_depth_and_waiting_polls_are_bounded() {
        let queue = EventQueue::new();
        for _ in 0..MAX_EVENTS {
            queue.enqueue(event()).unwrap();
        }
        assert!(matches!(queue.enqueue(event()), Err(EventQueueError::Full)));
        let _ = queue.poll().await;
        let large = Llsd::String("x".repeat(MAX_EVENT_BYTES / 2));
        for _ in 0..31 {
            queue.enqueue(large.clone()).unwrap();
        }
        assert!(matches!(queue.enqueue(large), Err(EventQueueError::Full)));
        let _ = queue.poll().await;
        assert!(matches!(
            queue.enqueue(Llsd::String("x".repeat(MAX_EVENT_BYTES + 1))),
            Err(EventQueueError::EventLimit)
        ));
        let mut nested = event();
        for _ in 0..34 {
            nested = Llsd::Array(vec![nested]);
        }
        assert!(matches!(
            queue.enqueue(nested),
            Err(EventQueueError::EventLimit)
        ));
        let mut tasks = Vec::new();
        for _ in 0..MAX_PENDING_POLLS {
            let queue = queue.clone();
            tasks.push(tokio::spawn(async move { queue.poll().await }));
        }
        tokio::task::yield_now().await;
        assert!(matches!(queue.poll().await, PollReply::Busy));
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        queue.enqueue(event()).unwrap();
        assert!(matches!(queue.poll().await, PollReply::Events(_)));
        queue.close();
        assert!(matches!(
            queue.enqueue(event()),
            Err(EventQueueError::Closed)
        ));
    }

    #[test]
    fn initial_ids_are_random_in_the_csharp_range_including_zero() {
        let ids: Vec<_> = (0..32).map(|_| random_id()).collect();
        assert!(ids.iter().all(|id| (0..30_000_000).contains(id)));
        assert!(ids.iter().any(|id| *id != ids[0]));
    }

    #[tokio::test]
    async fn zero_is_a_valid_id_and_each_batch_advances_once() {
        let queue = EventQueue::with_id(0);
        queue.enqueue(event()).unwrap();
        queue.enqueue(event()).unwrap();
        assert_eq!(id(queue.poll().await), 0);
        queue.enqueue(event()).unwrap();
        assert_eq!(id(queue.poll().await), 1);
    }
}
