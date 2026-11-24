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
//   Source/OpenSim.Region.CoreModules/Avatar/Chat/ChatModule.cs
//     (OnChatFromClient, DeliverChatToAvatars, TrySendChatMessage)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (SendChatMessage, SendAgentAlertMessage)
//   KanikaBreed/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// The seat map and the outbound queue are new Rust. Parcel hide, the god
// prefix, object chat and instant messages are not ported. Distances are the
// ChatModule defaults (whisper 10 m, say 20 m, shout 100 m), not INI keys.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Channel 0 agent chat for the avatars seated in one region.
//!
//! A whisper, say or shout is limited to 10 m, 20 m or 100 m. Typing and any
//! other channel-0 type have no distance cap, as `TrySendChatMessage` does
//! when `maxDistSQ` stays negative. Other channels, including the debug
//! channel, are not delivered. The production freeze set stays empty. Seats
//! do not store a position: the caller passes the poses the scene returned.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ferroan_protocol::msgs::{AgentAlertMessage, ChatFromSimulator};
use ferroan_protocol::str_field;
use glam::Vec3;
use tokio::sync::mpsc;
use uuid::Uuid;

const WHISPER: u8 = 0;
const SAY: u8 = 1;
const SHOUT: u8 = 2;
const START_TYPING: u8 = 4;
const STOP_TYPING: u8 = 5;
const SOURCE_AGENT: u8 = 1;
const AUDIBLE_FULLY: u8 = 1;
const WHISPER_SQ: f32 = 10.0 * 10.0;
const SAY_SQ: f32 = 20.0 * 20.0;
const SHOUT_SQ: f32 = 100.0 * 100.0;
/// `ChatModule` cuts a message longer than this many UTF-16 code units.
const CUT_OVER: usize = 1100;
const CUT_TO: usize = 1000;
const OUTBOUND_QUEUE: usize = 64;
const FREEZE_ALERT: &str = "/You may not talk as you are frozen.";

#[derive(Debug, Clone)]
pub(crate) enum ViewerBound {
    Chat(ChatFromSimulator),
    Alert(AgentAlertMessage),
}

struct Seat {
    generation: u64,
    name: String,
    frozen: bool,
    tx: mpsc::Sender<ViewerBound>,
}

struct Hub {
    seats: Mutex<HashMap<Uuid, Seat>>,
    next_generation: AtomicU64,
}

/// Avatars that can hear channel 0. Cloning shares the seat map.
#[derive(Clone)]
pub(crate) struct RegionChat {
    hub: Arc<Hub>,
}

/// One seated avatar. Dropping it removes only this registration.
pub(crate) struct ChatSeat {
    pub(crate) hub: RegionChat,
    /// Removes this avatar when the serve ends. Drop is the only use.
    _guard: SeatGuard,
    pub(crate) rx: mpsc::Receiver<ViewerBound>,
}

struct SeatGuard {
    hub: RegionChat,
    id: Uuid,
    generation: u64,
}

impl Drop for SeatGuard {
    fn drop(&mut self) {
        let mut seats = self
            .hub
            .hub
            .seats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if seats
            .get(&self.id)
            .is_some_and(|seat| seat.generation == self.generation)
        {
            seats.remove(&self.id);
        }
    }
}

impl RegionChat {
    pub(crate) fn new() -> Self {
        Self {
            hub: Arc::new(Hub {
                seats: Mutex::new(HashMap::new()),
                next_generation: AtomicU64::new(1),
            }),
        }
    }

    pub(crate) fn seat(&self, id: Uuid, name: String) -> ChatSeat {
        let (tx, rx) = mpsc::channel(OUTBOUND_QUEUE);
        let generation = self.hub.next_generation.fetch_add(1, Ordering::Relaxed);
        self.hub
            .seats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(
                id,
                Seat {
                    generation,
                    name,
                    frozen: false,
                    tx,
                },
            );
        ChatSeat {
            hub: self.clone(),
            _guard: SeatGuard {
                hub: self.clone(),
                id,
                generation,
            },
            rx,
        }
    }

    /// Test hook. Production never freezes an avatar; parcel freeze is not ported.
    #[cfg(test)]
    pub(crate) fn freeze(&self, id: Uuid, frozen: bool) {
        if let Some(seat) = self
            .hub
            .seats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get_mut(&id)
        {
            seat.frozen = frozen;
        }
    }

    /// Agents currently seated. Chat asks the scene for these poses.
    pub(crate) fn seated_ids(&self) -> Vec<Uuid> {
        self.hub
            .seats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .keys()
            .copied()
            .collect()
    }

    /// Deliver one viewer chat line. The caller has already checked the circuit identity.
    /// `places` are the scene poses. An agent with no pose is not in the scene.
    pub(crate) fn publish(
        &self,
        from: Uuid,
        chat_type: u8,
        channel: i32,
        message: &str,
        places: &HashMap<Uuid, [f32; 3]>,
    ) {
        if channel != 0 {
            return;
        }
        let seats = self
            .hub
            .seats
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(speaker) = seats.get(&from) else {
            return;
        };
        let Some(&from_pos) = places.get(&from) else {
            return;
        };
        let name = speaker.name.clone();
        let frozen = speaker.frozen;
        let speaker_tx = speaker.tx.clone();
        if frozen {
            // A frozen sender is told only for real chat. Typing while frozen
            // is not delivered and is not alerted, as C# does since TB37 was
            // fixed (#488, Pin7 `1052cbae82`).
            if !is_typing(chat_type) {
                let _ = speaker_tx.try_send(ViewerBound::Alert(freeze_alert(from)));
            }
            return;
        }
        let text = cut_message(message);
        let packet = chat_packet(from, &name, from_pos, chat_type, text);
        let limit = range_limit(chat_type);
        let targets: Vec<mpsc::Sender<ViewerBound>> = seats
            .iter()
            .filter_map(|(id, seat)| {
                let &to = places.get(id)?;
                if limit.is_some_and(|max_sq| distance_squared(from_pos, to) > max_sq) {
                    return None;
                }
                Some(seat.tx.clone())
            })
            .collect();
        drop(seats);
        for tx in targets {
            let _ = tx.try_send(ViewerBound::Chat(packet.clone()));
        }
    }
}

fn is_typing(chat_type: u8) -> bool {
    chat_type == START_TYPING || chat_type == STOP_TYPING
}

fn range_limit(chat_type: u8) -> Option<f32> {
    match chat_type {
        WHISPER => Some(WHISPER_SQ),
        SAY => Some(SAY_SQ),
        SHOUT => Some(SHOUT_SQ),
        _ => None,
    }
}

fn distance_squared(from: [f32; 3], to: [f32; 3]) -> f32 {
    let dx = from[0] - to[0];
    let dy = from[1] - to[1];
    let dz = from[2] - to[2];
    dx * dx + dy * dy + dz * dz
}

/// Keep a message of 1100 UTF-16 code units. A longer one is cut to 1000.
/// A code unit that would cross the cut is left out, so the result stays valid UTF-8.
fn cut_message(message: &str) -> &str {
    if message.encode_utf16().count() <= CUT_OVER {
        return message;
    }
    let mut units = 0usize;
    for (index, ch) in message.char_indices() {
        let width = ch.len_utf16();
        if units + width > CUT_TO {
            return &message[..index];
        }
        units += width;
    }
    message
}

fn chat_packet(
    from: Uuid,
    name: &str,
    position: [f32; 3],
    chat_type: u8,
    message: &str,
) -> ChatFromSimulator {
    let mut packet = ChatFromSimulator::default();
    packet.chat_data.from_name = str_field(name);
    packet.chat_data.source_id = from;
    packet.chat_data.owner_id = from;
    packet.chat_data.source_type = SOURCE_AGENT;
    packet.chat_data.chat_type = chat_type;
    packet.chat_data.audible = AUDIBLE_FULLY;
    packet.chat_data.position = Vec3::from_array(position);
    packet.chat_data.message = str_field(message);
    packet
}

fn freeze_alert(agent: Uuid) -> AgentAlertMessage {
    let mut alert = AgentAlertMessage::default();
    alert.agent_data.agent_id = agent;
    alert.alert_data.modal = false;
    alert.alert_data.message = str_field(FREEZE_ALERT);
    alert
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_protocol::{Msg, field_str};

    fn take(rx: &mut mpsc::Receiver<ViewerBound>) -> Vec<ViewerBound> {
        let mut out = Vec::new();
        while let Ok(item) = rx.try_recv() {
            out.push(item);
        }
        out
    }

    fn chat_of(items: &[ViewerBound]) -> Vec<&ChatFromSimulator> {
        items
            .iter()
            .filter_map(|item| match item {
                ViewerBound::Chat(packet) => Some(packet),
                ViewerBound::Alert(_) => None,
            })
            .collect()
    }

    fn alerts_of(items: &[ViewerBound]) -> Vec<&AgentAlertMessage> {
        items
            .iter()
            .filter_map(|item| match item {
                ViewerBound::Alert(packet) => Some(packet),
                ViewerBound::Chat(_) => None,
            })
            .collect()
    }

    fn seat_pair() -> (RegionChat, ChatSeat, ChatSeat, Uuid, Uuid) {
        let hub = RegionChat::new();
        let speaker = Uuid::from_u128(1);
        let listener = Uuid::from_u128(2);
        let left = hub.seat(speaker, "Test User".to_string());
        let right = hub.seat(listener, "Chat Bot".to_string());
        (hub, left, right, speaker, listener)
    }

    fn places(at: &[(Uuid, [f32; 3])]) -> HashMap<Uuid, [f32; 3]> {
        at.iter().copied().collect()
    }

    #[test]
    fn a_say_in_range_reaches_both_avatars_with_the_speaker_fields() {
        let (hub, mut speaker, mut listener, speaker_id, listener_id) = seat_pair();
        let here = places(&[
            (speaker_id, [0.0, 0.0, 0.0]),
            (listener_id, [0.0, 0.0, 0.0]),
        ]);
        hub.publish(speaker_id, SAY, 0, "ferroan harness chat", &here);
        for items in [take(&mut speaker.rx), take(&mut listener.rx)] {
            let heard = chat_of(&items);
            assert_eq!(heard.len(), 1);
            assert!(alerts_of(&items).is_empty());
            let packet = heard[0];
            assert_eq!(field_str(&packet.chat_data.from_name), "Test User");
            assert_eq!(packet.chat_data.source_id, speaker_id);
            assert_eq!(packet.chat_data.owner_id, speaker_id);
            assert_eq!(packet.chat_data.source_type, SOURCE_AGENT);
            assert_eq!(packet.chat_data.chat_type, SAY);
            assert_eq!(packet.chat_data.audible, AUDIBLE_FULLY);
            assert_eq!(packet.chat_data.position, Vec3::ZERO);
            assert_eq!(packet.chat_data.message, b"ferroan harness chat\0".to_vec());
        }
        const {
            assert!(!ChatFromSimulator::ZEROCODED);
            assert!(!AgentAlertMessage::ZEROCODED);
        }
    }

    #[test]
    fn whisper_say_and_shout_stop_at_their_squared_distances() {
        let (hub, mut speaker, mut listener, speaker_id, listener_id) = seat_pair();
        let cases = [
            (WHISPER, [10.0, 0.0, 0.0], [10.0, 1.0, 0.0]),
            (SAY, [20.0, 0.0, 0.0], [20.0, 1.0, 0.0]),
            (SHOUT, [100.0, 0.0, 0.0], [100.0, 1.0, 0.0]),
        ];
        for (chat_type, inside, outside) in cases {
            let near = places(&[(speaker_id, [0.0, 0.0, 0.0]), (listener_id, inside)]);
            hub.publish(speaker_id, chat_type, 0, "near", &near);
            assert_eq!(chat_of(&take(&mut listener.rx)).len(), 1, "{chat_type}");
            assert_eq!(chat_of(&take(&mut speaker.rx)).len(), 1, "{chat_type}");
            let far = places(&[(speaker_id, [0.0, 0.0, 0.0]), (listener_id, outside)]);
            hub.publish(speaker_id, chat_type, 0, "far", &far);
            assert!(chat_of(&take(&mut listener.rx)).is_empty(), "{chat_type}");
            let own_items = take(&mut speaker.rx);
            let own = chat_of(&own_items);
            assert_eq!(own.len(), 1, "{chat_type}");
            assert_eq!(field_str(&own[0].chat_data.message), "far");
        }
    }

    #[test]
    fn typing_has_no_distance_cap_and_other_channels_are_dropped() {
        let (hub, mut speaker, mut listener, speaker_id, listener_id) = seat_pair();
        let far = places(&[
            (speaker_id, [0.0, 0.0, 0.0]),
            (listener_id, [500.0, 0.0, 0.0]),
        ]);
        hub.publish(speaker_id, START_TYPING, 0, "", &far);
        hub.publish(speaker_id, STOP_TYPING, 0, "", &far);
        assert_eq!(chat_of(&take(&mut listener.rx)).len(), 2);
        assert_eq!(chat_of(&take(&mut speaker.rx)).len(), 2);
        hub.publish(speaker_id, SAY, 1, "private", &far);
        hub.publish(speaker_id, SAY, 2_147_483_647, "debug", &far);
        assert!(take(&mut listener.rx).is_empty());
        assert!(take(&mut speaker.rx).is_empty());
        hub.publish(Uuid::from_u128(9), SAY, 0, "missing", &far);
        assert!(take(&mut listener.rx).is_empty());
    }

    #[test]
    fn a_message_longer_than_1100_utf16_units_is_cut_to_1000() {
        let (hub, mut speaker, mut listener, speaker_id, listener_id) = seat_pair();
        let here = places(&[
            (speaker_id, [0.0, 0.0, 0.0]),
            (listener_id, [0.0, 0.0, 0.0]),
        ]);
        let kept = "a".repeat(1100);
        hub.publish(speaker_id, SAY, 0, &kept, &here);
        let kept_items = take(&mut listener.rx);
        let heard = chat_of(&kept_items);
        assert_eq!(field_str(&heard[0].chat_data.message), kept);
        let _ = take(&mut speaker.rx);
        let long = "b".repeat(1101);
        hub.publish(speaker_id, SAY, 0, &long, &here);
        let cut_items = take(&mut listener.rx);
        let heard = chat_of(&cut_items);
        assert_eq!(field_str(&heard[0].chat_data.message), "b".repeat(1000));
    }

    #[test]
    fn a_frozen_sender_is_alerted_only_for_real_chat() {
        let (hub, mut speaker, mut listener, speaker_id, listener_id) = seat_pair();
        let here = places(&[
            (speaker_id, [0.0, 0.0, 0.0]),
            (listener_id, [0.0, 0.0, 0.0]),
        ]);
        hub.freeze(speaker_id, true);
        hub.publish(speaker_id, SAY, 0, "hello", &here);
        let own = take(&mut speaker.rx);
        assert!(chat_of(&own).is_empty());
        let alerts = alerts_of(&own);
        assert_eq!(alerts.len(), 1);
        assert!(!alerts[0].alert_data.modal);
        assert_eq!(alerts[0].agent_data.agent_id, speaker_id);
        assert_eq!(
            field_str(&alerts[0].alert_data.message),
            "/You may not talk as you are frozen."
        );
        assert!(take(&mut listener.rx).is_empty());
        hub.publish(speaker_id, START_TYPING, 0, "", &here);
        hub.publish(speaker_id, STOP_TYPING, 0, "", &here);
        assert!(take(&mut speaker.rx).is_empty());
        assert!(take(&mut listener.rx).is_empty());
        hub.freeze(speaker_id, false);
        hub.publish(speaker_id, SAY, 0, "after", &here);
        assert_eq!(chat_of(&take(&mut listener.rx)).len(), 1);
        assert!(alerts_of(&take(&mut speaker.rx)).is_empty());
    }
}
