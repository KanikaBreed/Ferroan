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
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (ProcessEntityUpdates' terse packets, HandleQueueEmpty, HandleHasUpdates,
//      SendKillObject, SendCoarseLocationUpdate's header)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPClient.cs
//     (DequeueOutgoing's empty categories, BeginFireQueueEmpty,
//      GetCatBytesCanSend)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One viewer's entity updates, as the circuit sees them. The region fills
//! the feed. When Task asks for more, the circuit takes full prim updates and
//! avatar terse blocks, charging their separate estimated list and block sizes.
//! A negotiated material override follows those packets, unencoded, and is not
//! charged against that estimate.
//!
//! C# asks when the task outbox holds fewer than five packets and the feed
//! has updates, at most every 20 ms, with a budget of what the task bucket
//! drips in 30 ms. The packets are reliable and zero-coded on the task
//! category ("zero code is not as spec").
//!
//! Other avatars' full updates, appearances and kills do not wait for that:
//! C# writes them straight onto the task outbox
//! (`SendEntityFullUpdateImmediate`, `SendAppearance`, `SendKillObject`).
//! The minimap dots (`SendCoarseLocationUpdate`) go the same way, unreliable.
//! The circuit queues them, in order, on every pass.

use std::sync::Arc;
use std::time::Duration;

use ferroan_protocol::msgs::improved_terse_object_update::{ObjectData, RegionData};
use ferroan_protocol::msgs::{
    AvatarAppearance, CoarseLocationUpdate, ImprovedTerseObjectUpdate, KillObject, ObjectUpdate,
    kill_object,
};
use tokio::sync::Notify;

/// The avatar `ObjectData.Data` length (`CreateAvatartImprovedTerseBlock`).
pub const AVATAR_TERSE_LEN: usize = 60;

/// `LLUDPClient.MIN_CALLBACK_MS`.
pub(crate) const QUEUE_EMPTY_INTERVAL: Duration = Duration::from_millis(20);
/// `HandleQueueEmpty` asks for what the task bucket sends in this time.
pub(crate) const QUEUE_EMPTY_MS: i32 = 30;
/// `DequeueOutgoing` reports a category empty below this many packets.
pub(crate) const QUEUE_EMPTY_BELOW: usize = 5;

/// `LLUDPServer.MAXPAYLOAD`.
const MAX_PAYLOAD: usize = 1200;
/// Header, region data and the block count in front of the first block.
const FIRST_BLOCK_AT: usize = 18;
/// Size byte, block and the empty texture entry.
const AVATAR_BLOCK_BYTES: usize = 1 + AVATAR_TERSE_LEN + 2;

/// `Utils.FloatZeroOneToushort(1.0)`. The region does not dilate time.
const TIME_DILATION_FULL: u16 = 0xffff;

/// `SendKillObject` puts at most this many ids in one packet.
const KILLS_PER_PACKET: usize = 200;

/// One packet C# writes straight onto the task outbox.
#[derive(Clone, Debug, PartialEq)]
pub enum AvatarPacket {
    /// `SendEntityFullUpdateImmediate`: reliable and zero-coded.
    Full(ObjectUpdate),
    /// `SendAppearance`: reliable and zero-coded.
    Appearance(AvatarAppearance),
    /// `SendKillObject`: reliable, not zero-coded.
    Kill(KillObject),
    /// `SendCoarseLocationUpdate`: unreliable, not zero-coded.
    Coarse(CoarseLocationUpdate),
}

/// `SendKillObject`: the ids in order, 200 to a packet.
pub fn kill_packets(ids: &[u32]) -> Vec<KillObject> {
    ids.chunks(KILLS_PER_PACKET)
        .map(|chunk| KillObject {
            object_data: chunk
                .iter()
                .map(|id| kill_object::ObjectData { id: *id })
                .collect(),
            omitted_trailing_blocks: 0,
        })
        .collect()
}

/// `SendCoarseLocationUpdate`'s body: one location per listed avatar (X, Y,
/// Z/4), the viewer's own index and `Prey`, then the agents. Both counts are
/// written, empty or not.
pub fn coarse_update(
    locations: &[[u8; 3]],
    you: i16,
    prey: i16,
    agents: &[uuid::Uuid],
) -> CoarseLocationUpdate {
    use ferroan_protocol::msgs::coarse_location_update::{AgentData, Index, Location};
    CoarseLocationUpdate {
        location: locations
            .iter()
            .map(|[x, y, z]| Location {
                x: *x,
                y: *y,
                z: *z,
            })
            .collect(),
        index: Index { you, prey },
        agent_data: agents
            .iter()
            .map(|agent| AgentData { agent_id: *agent })
            .collect(),
        omitted_trailing_blocks: 0,
    }
}

/// Object-property payloads from one `ProcessEntityPropertyRequests` pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropertySend {
    /// Zero-coded `ObjectProperties` payloads, message id included.
    pub objects: Vec<Vec<u8>>,
    /// Raw `ObjectPropertiesFamily` payloads. The circuit zero-codes them.
    pub families: Vec<Vec<u8>>,
}

/// The region's side of one viewer's entity updates.
pub trait EntityUpdates: Send + Sync {
    /// `HandleHasUpdates` for the task category.
    fn has_updates(&self) -> bool;
    /// `ProcessEntityUpdates(max_bytes)` for stored full and avatar terse updates.
    fn take_updates(&self, batch: &mut crate::TaskBatch);
    /// At most `max` waiting avatar packets, the oldest first, in the order
    /// the region sent them. The rest wait for a later pass.
    fn take_avatar_packets(&self, max: usize) -> Vec<AvatarPacket>;
    /// `SendInitialData` asked for another terrain and overlay pass.
    /// A feed that never reaches that gate leaves this false.
    fn take_land_pass(&self) -> bool {
        false
    }
    /// `ProcessEntityPropertyRequests` with a fresh task budget.
    fn take_properties(&self, _budget: i32) -> PropertySend {
        PropertySend::default()
    }
}

/// The feed and the wake an offer notifies.
#[derive(Clone)]
pub struct EntityFeed {
    pub updates: Arc<dyn EntityUpdates>,
    pub wake: Arc<Notify>,
}

impl std::fmt::Debug for EntityFeed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EntityFeed")
    }
}

/// The terse packets for `blocks`, in order. A block that would end at or
/// past `MAXPAYLOAD` starts the next packet. C# keeps the last such block in
/// the packet without counting it, so the viewer never reads it (TB85); here
/// it starts a packet like the others (D69).
pub(crate) fn terse_packets(
    region_handle: u64,
    blocks: &[[u8; AVATAR_TERSE_LEN]],
) -> Vec<ImprovedTerseObjectUpdate> {
    let mut packets = Vec::new();
    let mut current: Vec<ObjectData> = Vec::new();
    let mut end = FIRST_BLOCK_AT;
    for block in blocks {
        end += AVATAR_BLOCK_BYTES;
        if end >= MAX_PAYLOAD && !current.is_empty() {
            packets.push(packet(region_handle, std::mem::take(&mut current)));
            end = FIRST_BLOCK_AT + AVATAR_BLOCK_BYTES;
        }
        current.push(ObjectData {
            data: block.to_vec(),
            texture_entry: Vec::new(),
        });
    }
    if !current.is_empty() {
        packets.push(packet(region_handle, current));
    }
    packets
}

fn packet(region_handle: u64, object_data: Vec<ObjectData>) -> ImprovedTerseObjectUpdate {
    ImprovedTerseObjectUpdate {
        region_data: RegionData {
            region_handle,
            time_dilation: TIME_DILATION_FULL,
        },
        object_data,
        omitted_trailing_blocks: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: u32) -> [u8; AVATAR_TERSE_LEN] {
        let mut out = [0u8; AVATAR_TERSE_LEN];
        out[0..4].copy_from_slice(&id.to_le_bytes());
        out
    }

    /// The list keeps its order, `You` and `Prey`, and an empty list still
    /// writes both counts.
    #[test]
    fn a_minimap_list_keeps_its_order_and_indexes() {
        let first = uuid::Uuid::from_u128(1);
        let second = uuid::Uuid::from_u128(2);
        let message = coarse_update(&[[128, 128, 6], [131, 40, 0]], 1, -1, &[first, second]);
        assert_eq!(message.location[1].x, 131);
        assert_eq!(message.location[1].y, 40);
        assert_eq!((message.index.you, message.index.prey), (1, -1));
        assert_eq!(message.agent_data[0].agent_id, first);
        assert_eq!(message.agent_data[1].agent_id, second);
        let empty = coarse_update(&[], -1, -1, &[]);
        let mut writer = ferroan_protocol::Writer::new();
        ferroan_protocol::Msg::encode_body(&empty, &mut writer);
        assert_eq!(writer.buf, [0, 0xff, 0xff, 0xff, 0xff, 0]);
    }

    #[test]
    fn kills_go_two_hundred_to_a_packet_in_order() {
        let ids: Vec<u32> = (1..=401).collect();
        let packets = kill_packets(&ids);
        assert_eq!(
            packets
                .iter()
                .map(|packet| packet.object_data.len())
                .collect::<Vec<_>>(),
            [200, 200, 1]
        );
        assert_eq!(packets[2].object_data[0].id, 401);
        assert_eq!(kill_packets(&[7])[0].object_data[0].id, 7);
        assert!(kill_packets(&[]).is_empty());
    }

    #[test]
    fn eighteen_blocks_fit_one_packet_and_the_nineteenth_starts_another() {
        let blocks: Vec<_> = (0..40).map(block).collect();
        let packets = terse_packets(7, &blocks[..18]);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].object_data.len(), 18);
        let packets = terse_packets(7, &blocks[..19]);
        assert_eq!(
            packets
                .iter()
                .map(|packet| packet.object_data.len())
                .collect::<Vec<_>>(),
            [18, 1]
        );
        let packets = terse_packets(7, &blocks);
        assert_eq!(
            packets
                .iter()
                .map(|packet| packet.object_data.len())
                .collect::<Vec<_>>(),
            [18, 18, 4]
        );
        // Order is kept across packets.
        let ids: Vec<u32> = packets
            .iter()
            .flat_map(|packet| &packet.object_data)
            .map(|object| u32::from_le_bytes(object.data[0..4].try_into().unwrap()))
            .collect();
        assert_eq!(ids, (0..40).collect::<Vec<_>>());
        assert!(terse_packets(7, &[]).is_empty());
    }
}
