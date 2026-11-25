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
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs
//     (CompleteMovement's SendInitialAvatarDataToAllAgents and appearance
//      loop, SendOtherAgentsAvatarFullToMe, HasMovedAway, MakeChildAgent's
//      kills, SendKillTo)
//   Source/OpenSim.Region.Framework/Scenes/Scene.cs (RemoveClient's kill)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (SendEntityFullUpdateImmediate, CreateAvatarUpdateBlock's live values,
//      SendKillObject)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! What one viewer is told about the other avatars, outside the entity
//! queue. C# writes these straight onto the viewer's task category:
//! `SendEntityFullUpdateImmediate`, `SendAppearance` and `SendKillObject`.
//! The frame decides when and to whom; the circuit writes the packets in
//! the order they were queued.
//!
//! - An arrival (`CompleteMovement`) sends the avatar's full update to every
//!   other attached viewer, then its appearance to each of them. Its own
//!   viewer gets both from its circuit.
//! - `SendInitialData` sends every other root avatar's full update and
//!   appearance to the viewer that has just finished its wait.
//! - A root that leaves the region (`RemoveClient`) is killed on every other
//!   viewer. A hop away kills it on every root viewer that has no child in
//!   the destination (`MakeChildAgent`), and a hop out of view range kills
//!   it, and every other root on it, both ways first (`HasMovedAway(false)`).
//!
//! - Every 50th frame each viewer whose circuit is open gets the minimap
//!   dots ([`crate::coarse`]), behind whatever is already waiting.
//!
//! Animations and attachments are later rows (P2.3b, P2.4).

use std::collections::VecDeque;
use std::sync::Arc;

use ferroan_diag::{Kind, REGION_AVATAR_PACKETS, refuse};
use uuid::Uuid;

use crate::coarse::CoarseLocations;

/// Waiting avatar packets per viewer. C# writes them to the outbox with no
/// bound; Ferroan refuses the next one past this (`region.avatar-packets`).
/// The circuit takes them only while its reliable window has room, so a
/// viewer that stops acknowledging leaves them here.
pub const MAX_AVATAR_PACKETS: usize = 4096;

/// What `CreateAvatarUpdateBlock` and `SendAppearance` read that the frame
/// does not own: the name, the group title and the stored appearance.
#[derive(Clone, Debug, PartialEq)]
pub struct AvatarLook {
    pub first_name: String,
    pub last_name: String,
    /// `ScenePresence.Grouptitle`. Empty when no groups module set a title.
    pub title: String,
    pub visual_params: Vec<u8>,
    /// `TextureEntry.GetBakesBytes`.
    pub texture: Vec<u8>,
    /// `AvatarAppearance.AvatarHeight`, the Z of `AvatarSize`.
    pub height: f32,
    /// `AvatarPreferencesHoverZ`.
    pub hover: f32,
}

/// One avatar as `CreateAvatarUpdateBlock` reads it when the packet is
/// built: the live presence values at that moment.
#[derive(Clone, Debug, PartialEq)]
pub struct AvatarFull {
    pub agent: Uuid,
    pub local_id: u32,
    pub look: Arc<AvatarLook>,
    pub collision_plane: [f32; 4],
    /// `OffsetPosition`: the region position of an avatar that does not sit.
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    /// `Rotation`, with X and Y already zeroed when the avatar neither flies
    /// nor sits on an object.
    pub rotation: [f32; 4],
}

/// One packet for one viewer, in queue order.
#[derive(Clone, Debug, PartialEq)]
pub enum AvatarPacket {
    /// `SendEntityFullUpdateImmediate` of an avatar.
    Full(AvatarFull),
    /// `SendAppearanceToAgentNF` with cofVersion -1.
    Appearance { agent: Uuid, look: Arc<AvatarLook> },
    /// `SendKillObject`.
    Kill(Vec<u32>),
    /// `SendCoarseLocationUpdate`.
    Coarse(CoarseLocations),
}

/// The packets waiting for one viewer's circuit.
#[derive(Debug, Default)]
pub(crate) struct AvatarPackets {
    waiting: VecDeque<AvatarPacket>,
}

impl AvatarPackets {
    /// Queue one packet. Past [`MAX_AVATAR_PACKETS`] the packet is refused.
    /// A minimap update replaces one still waiting at the back: the circuit
    /// has not taken the older list, and the viewer keeps only the newest.
    pub(crate) fn push(&mut self, packet: AvatarPacket) -> bool {
        if let AvatarPacket::Coarse(_) = &packet
            && let Some(last @ AvatarPacket::Coarse(_)) = self.waiting.back_mut()
        {
            *last = packet;
            return true;
        }
        if self.waiting.len() >= MAX_AVATAR_PACKETS {
            refuse(
                REGION_AVATAR_PACKETS,
                Kind::Hard,
                MAX_AVATAR_PACKETS as u64,
                self.waiting.len() as u64 + 1,
                "avatar-packets",
            );
            return false;
        }
        self.waiting.push_back(packet);
        true
    }

    /// The oldest `max` packets, in order.
    pub(crate) fn take(&mut self, max: usize) -> Vec<AvatarPacket> {
        let count = max.min(self.waiting.len());
        self.waiting.drain(..count).collect()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}

/// The rotation `CreateAvatarUpdateBlock` writes: X and Y are zeroed when the
/// avatar neither flies nor sits on an object.
pub(crate) fn block_rotation(rotation: [f32; 4], full: bool) -> [f32; 4] {
    if full {
        rotation
    } else {
        [0.0, 0.0, rotation[2], rotation[3]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standing_block_keeps_only_the_yaw() {
        let tilted = [0.1, 0.2, 0.3, 0.9];
        assert_eq!(block_rotation(tilted, false), [0.0, 0.0, 0.3, 0.9]);
        assert_eq!(block_rotation(tilted, true), tilted);
    }

    #[test]
    fn packets_past_the_bound_are_refused_and_the_rest_keep_their_order() {
        let mut packets = AvatarPackets::default();
        for id in 0..MAX_AVATAR_PACKETS as u32 {
            assert!(packets.push(AvatarPacket::Kill(vec![id])));
        }
        assert!(!packets.push(AvatarPacket::Kill(vec![u32::MAX])));
        let first = packets.take(1);
        assert_eq!(first, [AvatarPacket::Kill(vec![0])]);
        let taken = packets.take(usize::MAX);
        assert_eq!(taken.len(), MAX_AVATAR_PACKETS - 1);
        assert_eq!(taken[0], AvatarPacket::Kill(vec![1]));
        assert_eq!(
            taken[MAX_AVATAR_PACKETS - 2],
            AvatarPacket::Kill(vec![MAX_AVATAR_PACKETS as u32 - 1])
        );
        assert!(packets.is_empty());
    }

    fn coarse(you: i16) -> AvatarPacket {
        AvatarPacket::Coarse(CoarseLocations {
            locations: Vec::new(),
            you,
            prey: -1,
            agents: Vec::new(),
        })
    }

    #[test]
    fn a_waiting_minimap_update_is_replaced_only_at_the_back() {
        let mut packets = AvatarPackets::default();
        assert!(packets.push(coarse(0)));
        assert!(packets.push(coarse(1)));
        assert!(packets.push(AvatarPacket::Kill(vec![7])));
        assert!(packets.push(coarse(2)));
        assert_eq!(
            packets.take(usize::MAX),
            [coarse(1), AvatarPacket::Kill(vec![7]), coarse(2)]
        );
    }
}
