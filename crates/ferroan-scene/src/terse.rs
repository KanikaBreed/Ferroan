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
// Copyright (c) 2006-2016, openmetaverse.co
// All rights reserved.
//
// - Redistribution and use in source and binary forms, with or without
//   modification, are permitted provided that the following conditions are met:
//
// - Redistributions of source code must retain the above copyright notice, this
//   list of conditions and the following disclaimer.
// - Neither the name of the openmetaverse.co nor the names
//   of its contributors may be used to endorse or promote products derived from
//   this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs
//     (Update, SendTerseUpdateToAllClients, RegionHandShakeReply,
//      SendInitialData's wait, Lookat)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (CreateAvatartImprovedTerseBlock, ProcessEntityUpdates' terse budget and object animation selection)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
//   OpenMetaverse.Types/UtilsConversions.cs (FloatToUInt16, FloatToUnitUInt16),
//   Vector3.cs (ClampedToShortsBytes, Normalize), Quaternion.cs (ToShortsBytes)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Avatar terse updates: when `ScenePresence.Update` sends one, the 60-byte
//! block `CreateAvatartImprovedTerseBlock` writes, and the per-viewer feed the
//! circuit drains.
//!
//! The frame runs [`InitialData`] and [`LastSent::due`] once per tick for each
//! presence. A due presence stores its values and offers the terse flags to
//! every viewer's [`ViewerFeed`], its own on queue 0 and the others on queue 1
//! ([`best_avatar_responsiveness`](crate::best_avatar_responsiveness)). The
//! circuit takes blocks from its feed when the task category asks for more.
//! The block is built then, from the live values on the [`AvatarBoard`], as
//! C# reads the presence when it writes the packet.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::Notify;

use crate::avatars::{AvatarPacket, AvatarPackets};
use crate::entity_queue::{EntityUpdateQueue, QueuedUpdate, TERSE_UPDATE};

use crate::prim_update::{PrimUpdateError, StoredPrimUpdate};
use crate::prioritizer::{PriorityViewer, best_avatar_responsiveness};
use crate::stored_scene::StoredScene;

/// One entry from the viewer's mixed Task queue, resolved at dequeue.
pub enum ViewerUpdate<'a> {
    Full {
        queued: QueuedUpdate,
        part: &'a crate::ObjectPart,
        /// Collection selected for a separate object-animation update at dequeue.
        animations: Option<&'a [crate::ObjectAnimation]>,
        update: Result<&'a StoredPrimUpdate, &'a PrimUpdateError>,
    },
    Terse([u8; AVATAR_TERSE_LEN]),
}

/// `ScenePresence.cs` lines 4056–4060.
const ROTATION_TOLERANCE: f32 = 0.01;
const VELOCITY_TOLERANCE: f32 = 0.1;
const LOW_VELOCITY_SQ: f32 = 0.1;
const POSITION_LARGE_TOLERANCE: f32 = 5.0;
const POSITION_SMALL_TOLERANCE: f32 = 0.05;

/// `ScenePresence.CollisionPlane` while nothing has set it.
pub const UNIT_W: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// The avatar `ObjectData.Data` length.
pub const AVATAR_TERSE_LEN: usize = 60;

/// `ProcessEntityUpdates`: the terse list costs 18 bytes, and each avatar
/// block 63 (it carries no texture entry).
const TERSE_LIST_BYTES: i32 = 18;
const AVATAR_TERSE_BYTES: i32 = 63;

/// One avatar as the terse block and the thresholds read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvatarTerse {
    pub local_id: u32,
    /// `ScenePresence.State`, the `AgentUpdate` state byte.
    pub state: u8,
    pub collision_plane: [f32; 4],
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub rotation: [f32; 4],
    pub angular_velocity: [f32; 3],
    /// Flying or sitting on an object: the block carries the full rotation.
    pub full_rotation: bool,
}

/// The values `SendTerseUpdateToAllClients` stores. A new presence has every
/// one zero, and the collision plane unit W.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LastSent {
    state: u8,
    position: [f32; 3],
    velocity: [f32; 3],
    rotation: [f32; 4],
    collision_plane: [f32; 4],
}

impl Default for LastSent {
    fn default() -> Self {
        Self {
            state: 0,
            position: [0.0; 3],
            velocity: [0.0; 3],
            rotation: [0.0; 4],
            collision_plane: UNIT_W,
        }
    }
}

impl LastSent {
    /// `ScenePresence.Update` lines 4100–4130, for a presence that is not
    /// sitting on an object. Vector tolerances are per axis
    /// (`Vector3.ApproxEquals` and `ApproxZero`). The rotation test reads X, Y
    /// and Z; the plane test reads X, Y and W.
    pub fn due(&self, now: &AvatarTerse) -> bool {
        let turned = (0..3)
            .any(|axis| (now.rotation[axis] - self.rotation[axis]).abs() > ROTATION_TOLERANCE);
        let plane = [0, 1, 3].into_iter().any(|axis| {
            (now.collision_plane[axis] - self.collision_plane[axis]).abs()
                > POSITION_SMALL_TOLERANCE
        });
        if now.state != self.state || turned || plane {
            return true;
        }
        let velocity_moved = (0..3)
            .any(|axis| (now.velocity[axis] - self.velocity[axis]).abs() > VELOCITY_TOLERANCE);
        let stopped = now.velocity == [0.0; 3] && self.velocity != [0.0; 3];
        if velocity_moved || stopped {
            return true;
        }
        let moved = |tolerance: f32| {
            (0..3).any(|axis| (now.position[axis] - self.position[axis]).abs() > tolerance)
        };
        let speed_sq = now.velocity.iter().map(|axis| axis * axis).sum::<f32>();
        moved(POSITION_LARGE_TOLERANCE)
            || (moved(POSITION_SMALL_TOLERANCE) && speed_sq < LOW_VELOCITY_SQ)
    }

    pub fn store(&mut self, now: &AvatarTerse) {
        self.state = now.state;
        self.position = now.position;
        self.velocity = now.velocity;
        self.rotation = now.rotation;
        self.collision_plane = now.collision_plane;
    }
}

/// `NeedInitialData`: `ScenePresence.Update` returns until `SendInitialData`
/// has run (lines 4067–4071 and 4266–4290). It starts at 1. The viewer's
/// first `RegionHandshakeReply` makes it 2. Once the seed capability has been
/// answered, each frame adds one and returns, and the frame that reaches 6
/// runs `SendInitialData`, then returns too. The threshold test runs from the
/// next frame. A far departure sets it to done and forgets the reply
/// (`HasMovedAway(false)`, line 7602); for a near hop the reply is forgotten
/// by the return's `CompleteMovement` before it sends a new handshake (lines
/// 2428–2429). Either way the next reply starts the wait again. Ferroan
/// forgets it when the avatar becomes a child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitialData {
    need: i32,
    handshake: bool,
}

impl Default for InitialData {
    fn default() -> Self {
        Self {
            need: 1,
            handshake: false,
        }
    }
}

impl InitialData {
    /// `RegionHandShakeReply`. Only the first reply counts.
    pub fn handshake_reply(&mut self) {
        if !self.handshake {
            self.handshake = true;
            self.need = 2;
        }
    }

    /// `MakeChildAgent`.
    pub fn made_child(&mut self) {
        self.need = -1;
        self.handshake = false;
    }

    /// One frame: return early, run `SendInitialData` and return, or run
    /// the threshold test.
    pub fn frame(&mut self, seeds_sent: bool) -> InitialStep {
        if self.need <= 0 {
            return InitialStep::Test;
        }
        if self.need < 2 || !seeds_sent {
            return InitialStep::Wait;
        }
        self.need += 1;
        if self.need >= 6 {
            self.need = -1;
            return InitialStep::Send;
        }
        InitialStep::Wait
    }
}

/// What `ScenePresence.Update` does on one frame before the threshold test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialStep {
    /// `Update` returns early.
    Wait,
    /// This frame runs `SendInitialData`, then returns.
    Send,
    /// The threshold test runs.
    Test,
}

/// `Utils.FloatToUInt16(value, range)`: the quotient is a float, the product
/// a double, then truncated.
fn float_to_u16(value: f32, range: f32) -> u16 {
    let value = value + range;
    if value <= 0.0 {
        return 0;
    }
    let value = value / range;
    if value >= 2.0 {
        return u16::MAX;
    }
    (f64::from(value) * 32767.5) as u16
}

/// `Utils.FloatToUnitUInt16`.
fn float_to_unit_u16(value: f32) -> u16 {
    if value <= -1.0 {
        return 0;
    }
    if value >= 1.0 {
        return u16::MAX;
    }
    (f64::from(value) * 32767.5 + 32767.0) as u16
}

/// `Vector3.ClampedToShortsBytes`: the largest axis is scaled into `range`.
fn clamped_shorts(value: [f32; 3], range: f32, out: &mut [u8]) {
    let mut largest = value[0].abs();
    if value[1].abs() > largest {
        largest = value[1].abs();
    }
    if value[2].abs() > largest {
        largest = value[2].abs();
    }
    let scale = if largest > range {
        Some(range / largest)
    } else {
        None
    };
    for (axis, component) in value.into_iter().enumerate() {
        let component = match scale {
            Some(scale) => component * scale,
            None => component,
        };
        let packed = float_to_u16(component, range);
        out[axis * 2..axis * 2 + 2].copy_from_slice(&packed.to_le_bytes());
    }
}

/// `CreateAvatartImprovedTerseBlock` (`LLClientView.cs` lines 7352–7411),
/// without the size byte and the empty texture entry the packet adds.
pub fn avatar_block(avatar: &AvatarTerse) -> [u8; AVATAR_TERSE_LEN] {
    let mut out = [0u8; AVATAR_TERSE_LEN];
    out[0..4].copy_from_slice(&avatar.local_id.to_le_bytes());
    out[4] = avatar.state;
    out[5] = 1;
    let plane = if avatar.collision_plane == [0.0; 4] {
        UNIT_W
    } else {
        avatar.collision_plane
    };
    for (index, component) in plane.into_iter().enumerate() {
        let at = 6 + index * 4;
        out[at..at + 4].copy_from_slice(&component.to_le_bytes());
    }
    for (index, component) in avatar.position.into_iter().enumerate() {
        let at = 22 + index * 4;
        out[at..at + 4].copy_from_slice(&component.to_le_bytes());
    }
    clamped_shorts(avatar.velocity, 128.0, &mut out[34..40]);
    // Acceleration is written as zero.
    out[40..46].copy_from_slice(&[0xff, 0x7f, 0xff, 0x7f, 0xff, 0x7f]);
    let rotation = avatar.rotation;
    if avatar.full_rotation {
        for (index, component) in rotation.into_iter().enumerate() {
            let at = 46 + index * 2;
            out[at..at + 2].copy_from_slice(&float_to_unit_u16(component).to_le_bytes());
        }
    } else {
        out[46..50].copy_from_slice(&[0xff, 0x7f, 0xff, 0x7f]);
        let (z, w) = (rotation[2], rotation[3]);
        let length = z * z + w * w;
        if length > -1e-6 && length < 1e-6 {
            out[50..54].copy_from_slice(&0xffff_7fff_u32.to_le_bytes());
        } else {
            let scale = 1.0 / length.sqrt();
            out[50..52].copy_from_slice(&float_to_unit_u16(z * scale).to_le_bytes());
            out[52..54].copy_from_slice(&float_to_unit_u16(w * scale).to_le_bytes());
        }
    }
    clamped_shorts(avatar.angular_velocity, 64.0, &mut out[54..60]);
    out
}

/// `ScenePresence.Lookat` (lines 648–655): the stored camera at axis, which
/// C# never normalises (TB86), with Z dropped, then `Vector3.Normalize` on
/// its SSE4.1 path (`Vector3.cs` lines 227–243), which divides each axis by
/// the length and gives zero at a length squared of 1e-6 or less. The
/// scalar path multiplies by the reciprocal, which can differ in the last
/// bit; x86-64 servers take the SSE path.
pub fn look_at(camera_at: [f32; 3]) -> [f32; 3] {
    let (x, y) = (camera_at[0], camera_at[1]);
    let length = x * x + y * y;
    if length > 1e-6 {
        let length = length.sqrt();
        [x / length, y / length, 0.0]
    } else {
        [0.0; 3]
    }
}

/// The live avatar values every viewer's block is built from. The frame
/// writes each presence after its tick and removes it with the presence.
#[derive(Clone, Default)]
pub struct AvatarBoard {
    avatars: Arc<Mutex<HashMap<u32, AvatarTerse>>>,
}

impl AvatarBoard {
    fn lock(&self) -> MutexGuard<'_, HashMap<u32, AvatarTerse>> {
        self.avatars
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn write(&self, avatar: AvatarTerse) {
        self.lock().insert(avatar.local_id, avatar);
    }

    pub(crate) fn remove(&self, local_id: u32) {
        self.lock().remove(&local_id);
    }

    pub fn get(&self, local_id: u32) -> Option<AvatarTerse> {
        self.lock().get(&local_id).copied()
    }
}

/// One viewer's entity updates. The frame offers; the circuit takes. A
/// waiting update wakes the circuit through [`ViewerFeed::wake`]. Avatar
/// packets C# writes straight onto the task outbox wait beside the queue
/// and are taken first ([`ViewerFeed::take_avatar_packets`]).
pub struct ViewerFeed {
    queue: Mutex<EntityUpdateQueue>,
    objects: Arc<StoredScene>,
    packets: Mutex<AvatarPackets>,
    board: AvatarBoard,
    wake: Arc<Notify>,
    /// `SendInitialData` with zero teleport flags. The circuit takes it once.
    land_pass: AtomicBool,
}

impl ViewerFeed {
    pub fn new(board: AvatarBoard) -> Self {
        Self::with_objects(board, Arc::default())
    }

    pub(crate) fn with_objects(board: AvatarBoard, objects: Arc<StoredScene>) -> Self {
        Self {
            objects,
            queue: Mutex::new(EntityUpdateQueue::new()),
            packets: Mutex::new(AvatarPackets::default()),
            board,
            wake: Arc::new(Notify::new()),
            land_pass: AtomicBool::new(false),
        }
    }

    /// `ScenePresence.SendInitialData` asked for terrain and the overlay again.
    /// The circuit's entity wake is the same notify an avatar offer uses.
    pub fn request_land_pass(&self) {
        self.land_pass.store(true, Ordering::Release);
        self.wake.notify_one();
    }

    /// Take that request. A later call is false until the frame asks again.
    pub fn take_land_pass(&self) -> bool {
        self.land_pass.swap(false, Ordering::AcqRel)
    }

    fn queue(&self) -> MutexGuard<'_, EntityUpdateQueue> {
        self.queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn packets(&self) -> MutexGuard<'_, AvatarPackets> {
        self.packets
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Queue one avatar packet for the circuit. A kill first drops the
    /// waiting updates for those ids (`SendKillObject` removes them from
    /// `m_entityUpdates`).
    pub(crate) fn send_avatar_packet(&self, packet: AvatarPacket) {
        if let AvatarPacket::Kill(ids) = &packet {
            self.queue().remove(ids);
        }
        if self.packets().push(packet) {
            self.wake.notify_one();
        }
    }

    /// The oldest `max` waiting avatar packets, in the order the frame sent
    /// them. The rest stay for the next pass.
    pub fn take_avatar_packets(&self, max: usize) -> Vec<AvatarPacket> {
        self.packets().take(max)
    }

    /// `SendEntityUpdate` for an avatar's terse flags. A full queue refuses
    /// the update (`region.entity-updates`); priorities 0 and 1 are kept.
    pub(crate) fn offer_terse(&self, priority: u8, local_id: u32) {
        let queued = self.queue().enqueue(priority, local_id, TERSE_UPDATE);
        if queued {
            self.wake.notify_one();
        }
    }

    pub(crate) fn offer_stored_parts(&self, viewer: &PriorityViewer) {
        let mut queued = false;
        for (entity, local_id, flags) in self.objects.initial_parts() {
            let priority = best_avatar_responsiveness(viewer, &entity);
            queued |= self.queue().enqueue(priority, local_id, flags);
        }
        if queued {
            self.wake.notify_one();
        }
    }

    /// Dequeue one full or terse entry in Q42 priority order. A caller can
    /// stop at its batch budget without consuming the following entry.
    pub fn take_update(&self) -> Option<ViewerUpdate<'_>> {
        self.take_update_for_viewer(false)
    }

    /// Select a separate animation update only when this viewer supports it
    /// and the queued part still has an animation collection.
    pub fn take_update_for_viewer(
        &self,
        supports_object_animations: bool,
    ) -> Option<ViewerUpdate<'_>> {
        let mut queue = self.queue();
        while let Some(queued) = queue.dequeue() {
            if let Some((part, update)) = self.objects.get(queued.local_id) {
                let animations = if supports_object_animations
                    && queued.flags & crate::entity_queue::ANIMATIONS != 0
                {
                    part.animations.as_deref()
                } else {
                    None
                };
                return Some(ViewerUpdate::Full {
                    queued,
                    part,
                    animations,
                    update,
                });
            }
            if let Some(avatar) = self.board.get(queued.local_id) {
                return Some(ViewerUpdate::Terse(avatar_block(&avatar)));
            }
        }
        None
    }

    /// Whether an avatar-only consumer can dequeue a terse entry without taking stored parts.
    pub fn has_avatar_terse(&self) -> bool {
        self.queue().has_avatar_terse()
    }

    /// Notified after an offer. A permit is kept while nobody waits.
    pub fn wake(&self) -> Arc<Notify> {
        Arc::clone(&self.wake)
    }

    /// `HandleHasUpdates` for the task category.
    pub fn has_updates(&self) -> bool {
        !self.queue().is_empty()
    }

    /// `ProcessEntityUpdates(maxUpdatesBytes)` for avatar terse updates. A
    /// budget of 0 or less is a flush. An avatar that left the region is
    /// skipped. Each block is built from the board as it is taken.
    pub fn take_terse(&self, max_bytes: i32) -> Vec<[u8; AVATAR_TERSE_LEN]> {
        let mut budget = if max_bytes <= 0 { i32::MAX } else { max_bytes };
        let mut blocks = Vec::new();
        let mut queue = self.queue();
        while budget > 0 {
            let Some(update) = queue.dequeue_avatar_terse() else {
                break;
            };
            let Some(avatar) = self.board.get(update.local_id) else {
                continue;
            };
            if blocks.is_empty() {
                budget = budget.saturating_sub(TERSE_LIST_BYTES);
            }
            budget = budget.saturating_sub(AVATAR_TERSE_BYTES);
            blocks.push(avatar_block(&avatar));
        }
        blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn animation_dequeue_requires_the_flag_viewer_support_and_a_collection() {
        use crate::entity_queue::{FULL_UPDATE, FULL_UPDATE_WITH_ANIM};
        use crate::test_store::CopiedStore;
        use crate::{ObjectAnimation, ObjectStoreKind, TerrainFrame, open_objects};
        use uuid::Uuid;

        let store = CopiedStore::new("prim-updates/scene.db");
        let region = Uuid::from_u128(0x6d1a7f4e_0c3b_4f53_9a1e_5f0e3c2a9b01);
        let mut groups = open_objects(&store.connection(), region, ObjectStoreKind::Sqlite)
            .await
            .unwrap();
        let animation = ObjectAnimation {
            id: Uuid::from_u128(1),
            name: "turn".into(),
        };
        for (group, collection) in
            groups
                .iter_mut()
                .zip([None, Some(vec![]), Some(vec![animation.clone()])])
        {
            group.parts[0].animations = collection;
        }
        let frame = TerrainFrame::with_objects(None, Arc::new(groups), None).unwrap();
        let groups = frame.stored_groups();
        let feed = frame.new_feed();

        // Inject the flag even on the part with no collection, independently
        // of the SendUpdateToClient filter that normally clears it at offer.
        for (index, flags, supported, expected) in [
            (0, FULL_UPDATE_WITH_ANIM, true, None),
            (1, FULL_UPDATE_WITH_ANIM, true, Some(&[][..])),
            (
                2,
                FULL_UPDATE_WITH_ANIM,
                true,
                Some(std::slice::from_ref(&animation)),
            ),
            (2, FULL_UPDATE_WITH_ANIM, false, None),
            (2, FULL_UPDATE, true, None),
        ] {
            let id = groups[index].parts[0].local_id;
            assert!(feed.queue().enqueue(4, id, flags));
            let Some(ViewerUpdate::Full {
                queued, animations, ..
            }) = feed.take_update_for_viewer(supported)
            else {
                panic!("missing full update for {id}");
            };
            assert_eq!(queued.flags, flags);
            assert_eq!(animations, expected);
        }
    }

    fn standing(position: [f32; 3], velocity: [f32; 3]) -> AvatarTerse {
        AvatarTerse {
            local_id: 7,
            state: 0,
            collision_plane: UNIT_W,
            position,
            velocity,
            rotation: [0.0, 0.0, 0.0, 1.0],
            angular_velocity: [0.0; 3],
            full_rotation: false,
        }
    }

    fn sent(now: &AvatarTerse) -> LastSent {
        let mut last = LastSent::default();
        last.store(now);
        last
    }

    #[test]
    fn a_fresh_presence_is_due_and_a_still_one_is_not() {
        let now = standing([128.0, 128.0, 26.0], [0.0; 3]);
        assert!(LastSent::default().due(&now));
        assert!(!sent(&now).due(&now));
    }

    #[test]
    fn rotation_past_a_hundredth_on_x_y_or_z_is_due() {
        let base = standing([10.0, 10.0, 22.0], [0.0; 3]);
        let last = sent(&base);
        for axis in 0..3 {
            let mut now = base;
            now.rotation[axis] += 0.0101;
            assert!(last.due(&now), "axis {axis}");
            let mut near = base;
            near.rotation[axis] += 0.009;
            assert!(!last.due(&near), "axis {axis}");
        }
        // W is not compared.
        let mut w = base;
        w.rotation[3] = 0.5;
        assert!(!last.due(&w));
    }

    #[test]
    fn state_and_the_collision_plane_are_due() {
        let base = standing([10.0, 10.0, 22.0], [0.0; 3]);
        let last = sent(&base);
        let mut state = base;
        state.state = 4;
        assert!(last.due(&state));
        let mut plane = base;
        plane.collision_plane[3] = 1.06;
        assert!(last.due(&plane));
        // Z of the plane is not compared.
        let mut z = base;
        z.collision_plane[2] = 0.5;
        assert!(!last.due(&z));
    }

    #[test]
    fn velocity_past_a_tenth_or_a_stop_is_due() {
        let moving = standing([10.0, 10.0, 22.0], [2.4, 0.0, 0.0]);
        let last = sent(&moving);
        let mut faster = moving;
        faster.velocity[0] = 2.51;
        assert!(last.due(&faster));
        let mut close = moving;
        close.velocity[0] = 2.45;
        assert!(!last.due(&close));
        let stopped = standing(moving.position, [0.0; 3]);
        assert!(last.due(&stopped));
        // A stop is due once.
        assert!(!sent(&stopped).due(&stopped));
    }

    #[test]
    fn five_metres_on_one_axis_or_five_centimetres_when_slow_is_due() {
        let moving = standing([10.0, 10.0, 22.0], [2.4, 0.0, 0.0]);
        let last = sent(&moving);
        let mut walked = moving;
        walked.position[0] = 15.01;
        assert!(last.due(&walked));
        let mut short = moving;
        short.position[0] = 14.9;
        short.position[1] = 14.9;
        assert!(!last.due(&short));
        // Below a squared speed of 0.1, 5 cm on one axis is enough.
        let slow = standing([10.0, 10.0, 22.0], [0.3, 0.0, 0.0]);
        let last = sent(&slow);
        let mut crept = slow;
        crept.position[2] = 22.051;
        assert!(last.due(&crept));
        let mut still = slow;
        still.position[2] = 22.04;
        assert!(!last.due(&still));
        // At a squared speed of 0.1 or more it is not.
        let quick = standing([10.0, 10.0, 22.0], [0.32, 0.0, 0.0]);
        let mut moved = quick;
        moved.position[2] = 22.06;
        assert!(!sent(&quick).due(&moved));
    }

    #[test]
    fn the_initial_data_wait_needs_the_reply_and_the_seeds_then_four_frames() {
        use InitialStep::{Send, Test, Wait};
        let mut wait = InitialData::default();
        // No reply yet: seeds alone do not count.
        assert_eq!(wait.frame(true), Wait);
        wait.handshake_reply();
        assert_eq!(wait.frame(false), Wait);
        assert_eq!(wait.frame(false), Wait);
        // Three frames return early, the fourth runs SendInitialData.
        for _ in 0..3 {
            assert_eq!(wait.frame(true), Wait);
        }
        assert_eq!(wait.frame(true), Send);
        assert_eq!(wait.frame(true), Test);
        assert_eq!(wait.frame(false), Test);
        // A second reply does not restart it.
        wait.handshake_reply();
        assert_eq!(wait.frame(true), Test);
        // A child forgets the reply; the next one waits again.
        wait.made_child();
        assert_eq!(wait.frame(true), Test);
        wait.handshake_reply();
        assert_eq!(wait.frame(true), Wait);
    }

    #[test]
    fn the_block_packs_a_standing_turn_and_a_flying_rotation() {
        let mut avatar = standing([134.0, 128.25, 26.0], [2.4, -0.5, 0.0]);
        avatar.local_id = 0x0102_0304;
        avatar.rotation = [0.1, 0.2, 0.3, 0.9];
        let block = avatar_block(&avatar);
        assert_eq!(&block[0..4], &[4, 3, 2, 1]);
        assert_eq!(block[4], 0);
        assert_eq!(block[5], 1);
        assert_eq!(
            &block[6..22],
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 63]
        );
        assert_eq!(&block[22..26], &134.0f32.to_le_bytes());
        let u16_at = |at: usize| u16::from_le_bytes([block[at], block[at + 1]]);
        assert_eq!(u16_at(34), float_to_u16(2.4, 128.0));
        assert_eq!(u16_at(36), float_to_u16(-0.5, 128.0));
        assert_eq!([u16_at(40), u16_at(42), u16_at(44)], [0x7fff; 3]);
        assert_eq!([u16_at(46), u16_at(48)], [0x7fff; 2]);
        let scale = 1.0 / (0.3f32 * 0.3 + 0.9 * 0.9).sqrt();
        assert_eq!(u16_at(50), float_to_unit_u16(0.3 * scale));
        assert_eq!(u16_at(52), float_to_unit_u16(0.9 * scale));
        assert_eq!(u16_at(54), float_to_u16(0.0, 64.0));
        // No Z or W: Z 0x7fff, W 0xffff.
        avatar.rotation = [0.0, 0.0, 0.0, 0.0];
        let block = avatar_block(&avatar);
        assert_eq!(&block[50..54], &[0xff, 0x7f, 0xff, 0xff]);
        // Flying: the full rotation.
        avatar.rotation = [0.1, 0.2, 0.3, 0.9];
        avatar.full_rotation = true;
        let block = avatar_block(&avatar);
        let u16_at = |at: usize| u16::from_le_bytes([block[at], block[at + 1]]);
        assert_eq!(u16_at(46), float_to_unit_u16(0.1));
        assert_eq!(u16_at(48), float_to_unit_u16(0.2));
        // A zero plane is written as unit W.
        avatar.collision_plane = [0.0; 4];
        assert_eq!(&avatar_block(&avatar)[18..22], &1.0f32.to_le_bytes());
    }

    #[test]
    fn a_fast_velocity_is_scaled_into_the_range() {
        let avatar = standing([1.0, 1.0, 1.0], [256.0, 64.0, 0.0]);
        let block = avatar_block(&avatar);
        let u16_at = |at: usize| u16::from_le_bytes([block[at], block[at + 1]]);
        assert_eq!(u16_at(34), u16::MAX);
        assert_eq!(u16_at(36), float_to_u16(32.0, 128.0));
    }

    /// Dividing by the length, as the SSE path does, is not multiplying by
    /// its reciprocal in the last bit.
    #[test]
    fn look_at_divides_by_the_length() {
        let axis = [0.3, 0.7, 0.2];
        let length = (0.3f32 * 0.3 + 0.7 * 0.7).sqrt();
        let looked = look_at(axis);
        assert_eq!(looked[0].to_bits(), (0.3f32 / length).to_bits());
        assert_eq!(looked[1].to_bits(), (0.7f32 / length).to_bits());
        let reciprocal = 1.0 / length;
        assert_ne!(
            [looked[0].to_bits(), looked[1].to_bits()],
            [
                (0.3f32 * reciprocal).to_bits(),
                (0.7f32 * reciprocal).to_bits()
            ]
        );
    }

    #[test]
    fn look_at_drops_z_and_a_vertical_camera_is_zero() {
        assert_eq!(look_at([3.0, 4.0, 9.0]), [0.6, 0.8, 0.0]);
        assert_eq!(look_at([0.0, 0.0, 1.0]), [0.0; 3]);
        assert_eq!(look_at([0.0005, 0.0, 1.0]), [0.0; 3]);
    }

    /// A feed the circuit does not drain refuses the packet past its bound
    /// and reports `region.avatar-packets`. A kill still drops its id's
    /// waiting terse update.
    #[test]
    fn an_undrained_feed_refuses_past_its_bound_and_reports_it() {
        let feed = ViewerFeed::new(AvatarBoard::default());
        feed.offer_terse(1, 7);
        for id in 0..crate::avatars::MAX_AVATAR_PACKETS as u32 {
            feed.send_avatar_packet(AvatarPacket::Kill(vec![id + 100]));
        }
        assert!(feed.has_updates());
        feed.send_avatar_packet(AvatarPacket::Kill(vec![7]));
        assert!(ferroan_diag::metrics().contains("region.avatar-packets"));
        assert!(
            !feed.has_updates(),
            "the kill still drops the waiting update"
        );
        assert!(feed.take_avatar_packets(0).is_empty());
        let taken = feed.take_avatar_packets(usize::MAX);
        assert_eq!(taken.len(), crate::avatars::MAX_AVATAR_PACKETS);
        assert_eq!(taken.last(), Some(&AvatarPacket::Kill(vec![4195])));
    }

    #[test]
    fn a_feed_takes_its_own_avatar_first_and_within_the_budget() {
        let board = AvatarBoard::default();
        let feed = ViewerFeed::new(board.clone());
        for local_id in [1, 2, 3] {
            let mut avatar = standing([local_id as f32, 0.0, 0.0], [0.0; 3]);
            avatar.local_id = local_id;
            board.write(avatar);
        }
        feed.offer_terse(1, 2);
        feed.offer_terse(1, 3);
        feed.offer_terse(0, 1);
        // A second offer merges.
        feed.offer_terse(0, 1);
        assert!(feed.has_updates());
        // 18 + 63 fits one block, then the budget is spent.
        let first = feed.take_terse(81);
        assert_eq!(first.len(), 1);
        assert_eq!(&first[0][0..4], &1u32.to_le_bytes());
        // A block is built from the board when it is taken.
        let mut moved = standing([9.0, 0.0, 0.0], [0.0; 3]);
        moved.local_id = 2;
        board.write(moved);
        let rest = feed.take_terse(0);
        assert_eq!(rest.len(), 2);
        assert_eq!(&rest[0][22..26], &9.0f32.to_le_bytes());
        assert!(!feed.has_updates());
        // An avatar that left is skipped.
        feed.offer_terse(1, 3);
        board.remove(3);
        assert!(feed.take_terse(0).is_empty());
    }
}
