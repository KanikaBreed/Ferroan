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
//   Source/OpenSim.Region.Framework/Scenes/PriorityQueue.cs
//   Source/OpenSim.Region.Framework/Scenes/EntityUpdates.cs
//   Source/OpenSim.Framework/IClientAPI.cs
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One viewer's waiting entity updates.
//!
//! `SendEntityUpdate` keeps one waiting update per local id. A second update
//! for that id merges into the waiting entry: the first entry order stays,
//! and the priority becomes the new one. Field flags are combined. `CancelKill`
//! replaces the waiting flags with a full update that includes animations, or
//! with only the probe bit when that bit is set. A flag word that is only
//! `SendInTransit` drops the waiting entry instead of enqueueing. The caller
//! skips another person's HUD before offering; this queue does not see
//! attachment privacy.
//!
//! Priorities 0 and 1 drain completely before the others.
//! [`best_avatar_responsiveness`](crate::best_avatar_responsiveness) uses 0 for
//! this viewer's avatar and 1 for other avatars. The other eleven queues then
//! take turns, 8, 8, 5, 4, 3, 2, then 1 pull each.
//! Inside one queue the oldest entry order leaves first.
//!
//! A dequeued word with `Kill` set is a kill. Otherwise `UpdateProbe` is a
//! probe and not also a field packet. Remaining field bits that fit the terse
//! mask are a terse update; any other field bit is a full update. `Animations`
//! is reported beside that field packet. Packet bytes, distance culling and
//! deleted-object kills are later rows.
//!
//! C# grows this queue without a bound. Ferroan refuses a new local id when
//! [`MAX_ENTITY_UPDATES`] distinct updates are already waiting on this viewer.
//! A merge into an id that is already waiting still applies.

use std::collections::HashMap;

use ferroan_diag::{Kind, REGION_ENTITY_UPDATES, refuse};

/// Waiting updates on one viewer. One slot per local id.
pub const MAX_ENTITY_UPDATES: usize = 100_000;

/// Queues, including the two that drain before the fair rotation.
pub const QUEUE_COUNT: usize = 13;

pub(crate) const IMMEDIATE_QUEUES: usize = 2;

/// Pulls from each queue per turn. The first two are immediate, so the count is unused.
const QUEUE_PULLS: [i32; QUEUE_COUNT] = [0, 0, 8, 8, 5, 4, 3, 2, 1, 1, 1, 1, 1];

pub const ATTACHMENT_POINT: u32 = 1 << 0;
pub const MATERIAL_OVR: u32 = 1 << 1;
pub const CLICK_ACTION: u32 = 1 << 2;
pub const SCALE: u32 = 1 << 3;
pub const PARENT_ID: u32 = 1 << 4;
pub const PRIM_FLAGS: u32 = 1 << 5;
pub const PRIM_DATA: u32 = 1 << 6;
pub const MEDIA_URL: u32 = 1 << 7;
pub const SCRATCH_PAD: u32 = 1 << 8;
pub const TEXTURES: u32 = 1 << 9;
pub const TEXTURE_ANIM: u32 = 1 << 10;
pub const NAME_VALUE: u32 = 1 << 11;
pub const POSITION: u32 = 1 << 12;
pub const ROTATION: u32 = 1 << 13;
pub const VELOCITY: u32 = 1 << 14;
pub const ACCELERATION: u32 = 1 << 15;
pub const ANGULAR_VELOCITY: u32 = 1 << 16;
pub const COLLISION_PLANE: u32 = 1 << 17;
pub const TEXT: u32 = 1 << 18;
pub const PARTICLES: u32 = 1 << 19;
pub const EXTRA_DATA: u32 = 1 << 20;
pub const SOUND: u32 = 1 << 21;

pub const TERSE_UPDATE: u32 = POSITION | ROTATION | VELOCITY | ACCELERATION | ANGULAR_VELOCITY;

/// Field bits. The material-override bit is not one of them.
pub const FULL_UPDATE: u32 = 0x00ff_ffff & !MATERIAL_OVR;

pub const ANIMATIONS: u32 = 1 << 24;
pub const FULL_UPDATE_WITH_ANIM: u32 = FULL_UPDATE | ANIMATIONS;

pub const UPDATE_PROBE: u32 = 1 << 28;
pub const SEND_IN_TRANSIT: u32 = 1 << 29;
pub const CANCEL_KILL: u32 = 1 << 30;
pub const KILL: u32 = 1 << 31;

const TERSE_MASK: u32 = ATTACHMENT_POINT
    | TEXTURES
    | POSITION
    | ROTATION
    | VELOCITY
    | ACCELERATION
    | ANGULAR_VELOCITY
    | COLLISION_PLANE;

/// How a dequeued flag word is sent. Packet bytes are a later row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldPacket {
    Terse,
    Full,
}

/// Kill and probe replace a field packet. Animations can ride with one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassifiedUpdate {
    pub kill: bool,
    pub probe: bool,
    pub animations: bool,
    pub field: Option<FieldPacket>,
}

/// Classify `flags` the way a dequeue does before any packet is built.
pub fn classify(flags: u32) -> ClassifiedUpdate {
    if flags & KILL != 0 {
        return ClassifiedUpdate {
            kill: true,
            probe: false,
            animations: false,
            field: None,
        };
    }
    if flags & UPDATE_PROBE != 0 {
        return ClassifiedUpdate {
            kill: false,
            probe: true,
            animations: false,
            field: None,
        };
    }
    let animations = flags & ANIMATIONS != 0;
    let field_bits = flags & FULL_UPDATE;
    let field = if field_bits == 0 {
        None
    } else if field_bits & !TERSE_MASK == 0 {
        Some(FieldPacket::Terse)
    } else {
        Some(FieldPacket::Full)
    };
    ClassifiedUpdate {
        kill: false,
        probe: false,
        animations,
        field,
    }
}

/// One waiting update after a dequeue or a lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueuedUpdate {
    pub local_id: u32,
    pub flags: u32,
    pub priority: u8,
    pub entry_order: u64,
}

struct Slot {
    flags: u32,
    priority: u8,
    entry_order: u64,
    heap_index: usize,
}

struct Heap {
    items: Vec<u32>,
    size: usize,
}

impl Heap {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            size: 0,
        }
    }

    fn len(&self) -> usize {
        self.size
    }

    fn add(&mut self, slots: &mut HashMap<u32, Slot>, local_id: u32) {
        if self.size == self.items.len() {
            self.items.push(local_id);
        } else {
            self.items[self.size] = local_id;
        }
        if let Some(slot) = slots.get_mut(&local_id) {
            slot.heap_index = self.size;
        }
        self.bubble_up(slots, self.size);
        self.size += 1;
    }

    fn remove_at(&mut self, slots: &mut HashMap<u32, Slot>, index: usize) -> bool {
        if self.size == 0 || index >= self.size {
            return false;
        }
        self.size -= 1;
        if self.size > 0 && index != self.size {
            let moved = self.items[self.size];
            self.items[index] = moved;
            if let Some(slot) = slots.get_mut(&moved) {
                slot.heap_index = index;
            }
            if !self.bubble_up(slots, index) {
                self.bubble_down(slots, index);
            }
        }
        true
    }

    fn bubble_up(&mut self, slots: &mut HashMap<u32, Slot>, index: usize) -> bool {
        let Some(&item_id) = self.items.get(index) else {
            return false;
        };
        let Some(item_order) = slots.get(&item_id).map(|slot| slot.entry_order) else {
            return false;
        };
        let mut current = index;
        while current > 0 {
            let parent = (current - 1) / 2;
            let parent_id = self.items[parent];
            let Some(parent_order) = slots.get(&parent_id).map(|slot| slot.entry_order) else {
                break;
            };
            if parent_order <= item_order {
                break;
            }
            self.items[current] = parent_id;
            if let Some(slot) = slots.get_mut(&parent_id) {
                slot.heap_index = current;
            }
            current = parent;
        }
        if current != index {
            self.items[current] = item_id;
            if let Some(slot) = slots.get_mut(&item_id) {
                slot.heap_index = current;
            }
            true
        } else {
            false
        }
    }

    fn bubble_down(&mut self, slots: &mut HashMap<u32, Slot>, index: usize) {
        if self.size < 2 {
            return;
        }
        let Some(&item_id) = self.items.get(index) else {
            return;
        };
        let Some(item_order) = slots.get(&item_id).map(|slot| slot.entry_order) else {
            return;
        };
        let mut current = index;
        let child_limit = self.size - 1;
        loop {
            if current >= self.size / 2 {
                break;
            }
            let mut child = (2 * current) + 1;
            if child >= self.size {
                break;
            }
            let mut child_id = self.items[child];
            let Some(mut child_order) = slots.get(&child_id).map(|slot| slot.entry_order) else {
                break;
            };
            if child < child_limit {
                let right = child + 1;
                if let Some(&right_id) = self.items.get(right)
                    && let Some(right_order) = slots.get(&right_id).map(|slot| slot.entry_order)
                    && child_order > right_order
                {
                    child_id = right_id;
                    child_order = right_order;
                    child = right;
                }
            }
            if child_order >= item_order {
                break;
            }
            self.items[current] = child_id;
            if let Some(slot) = slots.get_mut(&child_id) {
                slot.heap_index = current;
            }
            current = child;
        }
        if current != index {
            self.items[current] = item_id;
            if let Some(slot) = slots.get_mut(&item_id) {
                slot.heap_index = current;
            }
        }
    }
}

/// Per-viewer entity update queue. Priorities outside `0..QUEUE_COUNT` are refused.
pub struct EntityUpdateQueue {
    heaps: [Heap; QUEUE_COUNT],
    slots: HashMap<u32, Slot>,
    next_queue: usize,
    count_from_queue: i32,
    next_request: u64,
}

impl EntityUpdateQueue {
    pub fn new() -> Self {
        Self {
            heaps: std::array::from_fn(|_| Heap::new()),
            slots: HashMap::new(),
            next_queue: IMMEDIATE_QUEUES,
            count_from_queue: QUEUE_PULLS[IMMEDIATE_QUEUES],
            next_request: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn get(&self, local_id: u32) -> Option<QueuedUpdate> {
        self.slots.get(&local_id).map(|slot| QueuedUpdate {
            local_id,
            flags: slot.flags,
            priority: slot.priority,
            entry_order: slot.entry_order,
        })
    }

    /// Queue `flags` for `local_id`, or merge them into the waiting entry.
    ///
    /// A flag word that is only `SendInTransit` removes that id. A new id is
    /// refused when the viewer already holds [`MAX_ENTITY_UPDATES`] entries.
    pub fn enqueue(&mut self, priority: u8, local_id: u32, flags: u32) -> bool {
        if priority as usize >= QUEUE_COUNT {
            return false;
        }
        if flags == SEND_IN_TRANSIT {
            self.remove(&[local_id]);
            return true;
        }
        if let Some((old_priority, index, existing)) = self
            .slots
            .get(&local_id)
            .map(|slot| (slot.priority, slot.heap_index, slot.flags))
        {
            let merged = merge_flags(existing, flags);
            if priority != old_priority {
                let removed = self.heaps[old_priority as usize].remove_at(&mut self.slots, index);
                if let Some(slot) = self.slots.get_mut(&local_id) {
                    slot.flags = merged;
                    slot.priority = priority;
                }
                if removed {
                    self.heaps[priority as usize].add(&mut self.slots, local_id);
                }
            } else if let Some(slot) = self.slots.get_mut(&local_id) {
                slot.flags = merged;
                slot.priority = priority;
            }
            return true;
        }
        if self.slots.len() >= MAX_ENTITY_UPDATES {
            refuse(
                REGION_ENTITY_UPDATES,
                Kind::Hard,
                MAX_ENTITY_UPDATES as u64,
                MAX_ENTITY_UPDATES as u64 + 1,
                "entity-updates",
            );
            return false;
        }
        let entry_order = self.next_request;
        self.next_request = self.next_request.wrapping_add(1);
        self.slots.insert(
            local_id,
            Slot {
                flags: normalize_new(flags),
                priority,
                entry_order,
                heap_index: 0,
            },
        );
        self.heaps[priority as usize].add(&mut self.slots, local_id);
        true
    }

    pub fn remove(&mut self, local_ids: &[u32]) {
        for &local_id in local_ids {
            let Some((priority, index)) = self
                .slots
                .get(&local_id)
                .map(|slot| (slot.priority, slot.heap_index))
            else {
                continue;
            };
            self.heaps[priority as usize].remove_at(&mut self.slots, index);
            self.slots.remove(&local_id);
        }
    }

    /// Reassign priorities. `handler` returns the new priority, or `None` to stop.
    pub fn reprioritize(&mut self, mut handler: impl FnMut(u32) -> Option<u8>) {
        let ids: Vec<u32> = self.slots.keys().copied().collect();
        for local_id in ids {
            let Some(priority) = handler(local_id) else {
                break;
            };
            if priority as usize >= QUEUE_COUNT {
                break;
            }
            let Some((old_priority, index)) = self
                .slots
                .get(&local_id)
                .map(|slot| (slot.priority, slot.heap_index))
            else {
                continue;
            };
            if priority == old_priority {
                continue;
            }
            let removed = self.heaps[old_priority as usize].remove_at(&mut self.slots, index);
            if let Some(slot) = self.slots.get_mut(&local_id) {
                slot.priority = priority;
            }
            if removed {
                self.heaps[priority as usize].add(&mut self.slots, local_id);
            }
        }
    }

    pub fn dequeue(&mut self) -> Option<QueuedUpdate> {
        for queue in 0..IMMEDIATE_QUEUES {
            if self.heaps[queue].len() > 0 {
                return self.take_next(queue);
            }
        }
        if self.count_from_queue > 0 && self.heaps[self.next_queue].len() > 0 {
            self.count_from_queue -= 1;
            return self.take_next(self.next_queue);
        }
        for _ in IMMEDIATE_QUEUES..QUEUE_COUNT {
            self.next_queue += 1;
            if self.next_queue >= QUEUE_COUNT {
                self.next_queue = IMMEDIATE_QUEUES;
            }
            if self.heaps[self.next_queue].len() == 0 {
                continue;
            }
            self.count_from_queue = QUEUE_PULLS[self.next_queue] - 1;
            return self.take_next(self.next_queue);
        }
        None
    }

    fn take_next(&mut self, queue: usize) -> Option<QueuedUpdate> {
        let local_id = *self.heaps[queue].items.first()?;
        if self.heaps[queue].len() == 0 {
            return None;
        }
        let queued = self.get(local_id)?;
        self.heaps[queue].remove_at(&mut self.slots, 0);
        self.slots.remove(&local_id);
        Some(queued)
    }
}

impl Default for EntityUpdateQueue {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_new(flags: u32) -> u32 {
    if flags & CANCEL_KILL != 0 {
        if flags & UPDATE_PROBE != 0 {
            UPDATE_PROBE
        } else {
            FULL_UPDATE_WITH_ANIM
        }
    } else {
        flags
    }
}

fn merge_flags(mut existing: u32, new_flags: u32) -> u32 {
    if new_flags & UPDATE_PROBE != 0 {
        existing &= !UPDATE_PROBE;
    }
    if new_flags & CANCEL_KILL != 0 {
        if new_flags & UPDATE_PROBE != 0 {
            UPDATE_PROBE
        } else {
            FULL_UPDATE_WITH_ANIM
        }
    } else {
        existing | new_flags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(queue: &mut EntityUpdateQueue) -> Vec<u32> {
        let mut out = Vec::new();
        while let Some(update) = queue.dequeue() {
            out.push(update.local_id);
        }
        out
    }

    #[test]
    fn the_same_entity_updated_twice_keeps_one_merged_entry() {
        let mut queue = EntityUpdateQueue::new();
        assert!(queue.enqueue(5, 7, POSITION));
        assert!(queue.enqueue(5, 8, ROTATION));
        assert!(queue.enqueue(3, 7, VELOCITY));
        assert!(!queue.enqueue(QUEUE_COUNT as u8, 7, SCALE));

        assert_eq!(queue.len(), 2);
        let merged = queue.get(7).unwrap();
        assert_eq!(merged.flags, POSITION | VELOCITY);
        assert_eq!(merged.priority, 3);
        assert_eq!(merged.entry_order, 0);
        assert_eq!(queue.get(8).unwrap().entry_order, 1);
        assert_eq!(queue.get(8).unwrap().flags, ROTATION);
        assert_eq!(ids(&mut queue), vec![7, 8]);
    }

    #[test]
    fn a_kill_after_an_update_is_one_kill() {
        let mut queue = EntityUpdateQueue::new();
        assert!(queue.enqueue(2, 1, TERSE_UPDATE));
        assert!(queue.enqueue(2, 1, KILL));
        assert_eq!(queue.len(), 1);
        let update = queue.dequeue().unwrap();
        assert_eq!(update.flags, TERSE_UPDATE | KILL);
        assert_eq!(
            classify(update.flags),
            ClassifiedUpdate {
                kill: true,
                probe: false,
                animations: false,
                field: None,
            }
        );
        assert!(queue.dequeue().is_none());
    }

    #[test]
    fn a_full_update_after_a_terse_update_is_full() {
        let mut queue = EntityUpdateQueue::new();
        assert!(queue.enqueue(4, 1, TERSE_UPDATE));
        assert!(queue.enqueue(4, 1, FULL_UPDATE));
        let update = queue.get(1).unwrap();
        assert_eq!(update.flags, FULL_UPDATE);
        assert_eq!(
            classify(update.flags),
            ClassifiedUpdate {
                kill: false,
                probe: false,
                animations: false,
                field: Some(FieldPacket::Full),
            }
        );
        assert_eq!(classify(TERSE_UPDATE).field, Some(FieldPacket::Terse));
        assert_eq!(
            classify(POSITION | ROTATION).field,
            Some(FieldPacket::Terse)
        );
    }

    #[test]
    fn an_older_merged_entry_stays_ahead() {
        let mut queue = EntityUpdateQueue::new();
        assert!(queue.enqueue(2, 1, POSITION));
        assert!(queue.enqueue(2, 2, POSITION));
        assert!(queue.enqueue(2, 3, POSITION));
        assert!(queue.enqueue(2, 4, POSITION));
        assert!(queue.enqueue(2, 5, POSITION));
        assert!(queue.enqueue(2, 1, ROTATION));
        queue.remove(&[3]);
        assert_eq!(queue.get(1).unwrap().flags, POSITION | ROTATION);
        assert_eq!(ids(&mut queue), vec![1, 2, 4, 5]);
    }

    #[test]
    fn immediate_queues_drain_before_the_fair_rotation() {
        let mut queue = EntityUpdateQueue::new();
        assert!(queue.enqueue(4, 10, POSITION));
        assert!(queue.enqueue(0, 11, POSITION));
        assert!(queue.enqueue(2, 12, POSITION));
        assert_eq!(ids(&mut queue), vec![11, 12, 10]);

        let mut last = EntityUpdateQueue::new();
        assert!(last.enqueue(12, 40, SCALE));
        assert_eq!(ids(&mut last), vec![40]);
    }

    #[test]
    fn each_fair_queue_gives_up_its_count_then_the_next() {
        let mut queue = EntityUpdateQueue::new();
        for id in 1..=9 {
            assert!(queue.enqueue(2, id, POSITION));
        }
        assert!(queue.enqueue(3, 100, POSITION));
        assert_eq!(ids(&mut queue), vec![1, 2, 3, 4, 5, 6, 7, 8, 100, 9]);
    }

    #[test]
    fn a_send_in_transit_flag_alone_drops_the_waiting_update() {
        let mut queue = EntityUpdateQueue::new();
        assert!(queue.enqueue(2, 1, POSITION));
        assert!(queue.enqueue(2, 1, SEND_IN_TRANSIT));
        assert!(queue.is_empty());
        assert!(queue.enqueue(2, 2, POSITION | SEND_IN_TRANSIT));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.get(2).unwrap().flags, POSITION | SEND_IN_TRANSIT);
    }

    #[test]
    fn cancel_kill_replaces_a_queued_kill_with_a_full_update() {
        let mut queue = EntityUpdateQueue::new();
        assert!(queue.enqueue(2, 1, KILL | POSITION));
        assert!(queue.enqueue(2, 1, FULL_UPDATE));
        assert!(classify(queue.get(1).unwrap().flags).kill);

        assert!(queue.enqueue(2, 1, CANCEL_KILL));
        let replaced = queue.get(1).unwrap();
        assert_eq!(replaced.flags, FULL_UPDATE_WITH_ANIM);
        assert_eq!(
            classify(replaced.flags),
            ClassifiedUpdate {
                kill: false,
                probe: false,
                animations: true,
                field: Some(FieldPacket::Full),
            }
        );

        let mut fresh = EntityUpdateQueue::new();
        assert!(fresh.enqueue(2, 4, CANCEL_KILL | UPDATE_PROBE));
        assert_eq!(fresh.get(4).unwrap().flags, UPDATE_PROBE);
        assert!(classify(UPDATE_PROBE).probe);
    }

    #[test]
    fn two_viewers_keep_their_own_queues() {
        let mut first = EntityUpdateQueue::new();
        let mut second = EntityUpdateQueue::new();
        assert!(first.enqueue(2, 1, POSITION));
        assert!(second.enqueue(2, 1, KILL));
        assert_eq!(first.get(1).unwrap().flags, POSITION);
        assert!(classify(second.get(1).unwrap().flags).kill);
    }

    #[test]
    fn a_reprioritize_moves_an_entry_onto_an_immediate_queue() {
        let mut queue = EntityUpdateQueue::new();
        assert!(queue.enqueue(6, 1, POSITION));
        assert!(queue.enqueue(6, 2, POSITION));
        queue.reprioritize(|local_id| Some(if local_id == 2 { 0 } else { 6 }));
        assert_eq!(ids(&mut queue), vec![2, 1]);
    }

    #[test]
    fn a_new_local_id_past_the_queue_depth_is_refused() {
        let mut queue = EntityUpdateQueue::new();
        for id in 0..MAX_ENTITY_UPDATES as u32 {
            assert!(queue.enqueue(2, id, POSITION));
        }
        assert_eq!(queue.len(), MAX_ENTITY_UPDATES);
        assert!(!queue.enqueue(2, MAX_ENTITY_UPDATES as u32, POSITION));
        assert!(queue.enqueue(2, 0, ROTATION));
        assert_eq!(queue.len(), MAX_ENTITY_UPDATES);
        assert_eq!(queue.get(0).unwrap().flags, POSITION | ROTATION);
        assert!(ferroan_diag::metrics().contains("region.entity-updates"));
    }
}
