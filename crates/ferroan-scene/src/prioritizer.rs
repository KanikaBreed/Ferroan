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
//   Source/OpenSim.Region.Framework/Scenes/Prioritizer.cs
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Queue index for one viewer's entity update, using the shipped default.
//!
//! The viewer's own id is queue 0. Every other avatar is queue 1, at any
//! distance. An attachment is queue 2, and so is every part of the linkset
//! that avatar is sitting on. Any other prim is measured from the viewer's
//! avatar position to the linkset's absolute position: a child part's own
//! position is not the measurement. Those queues start at 3 and step when the
//! distance reaches about 20, 40, 80, 160, 320, 640, 1280, 2560 and 5120 metres.
//! A few of those steps land a couple of ulps before the round number. A
//! prim whose root has no physical actor, or whose actor is not physical, sits
//! one queue lower. Queue 12 is the last. A distance that is not finite is that
//! last queue. With no presence, only the viewer's own id stays off it.
//!
//! Pass the linkset position and whether the root actor is physical. This
//! function does not read the scene.

use uuid::Uuid;

use crate::entity_queue::{IMMEDIATE_QUEUES, QUEUE_COUNT};

/// The viewer the priority is for.
#[derive(Clone, Debug, PartialEq)]
pub struct PriorityViewer {
    pub agent_id: Uuid,
    /// Avatar position. Absent when this agent has no presence.
    pub position: Option<[f32; 3]>,
    /// Linkset this avatar is sitting on.
    pub sitting_on: Option<Uuid>,
}

/// The entity being queued for that viewer.
#[derive(Clone, Debug, PartialEq)]
pub enum PriorityEntity {
    Avatar {
        id: Uuid,
    },
    Prim {
        id: Uuid,
        /// Linkset identity. Parts of one object share it.
        linkset: Uuid,
        attachment: bool,
        /// True only when the root part has a physics actor and that actor is physical.
        physical: bool,
        /// Linkset absolute position.
        position: [f32; 3],
    },
}

/// Queue index for `entity` as seen by `viewer`. Always `0..QUEUE_COUNT`.
pub fn best_avatar_responsiveness(viewer: &PriorityViewer, entity: &PriorityEntity) -> u8 {
    let entity_id = match entity {
        PriorityEntity::Avatar { id } | PriorityEntity::Prim { id, .. } => *id,
    };
    if entity_id == viewer.agent_id {
        return 0;
    }
    let Some(viewer_pos) = viewer.position else {
        return (QUEUE_COUNT - 1) as u8;
    };
    match entity {
        PriorityEntity::Avatar { .. } => 1,
        PriorityEntity::Prim {
            linkset,
            attachment,
            physical,
            position,
            ..
        } => {
            if *attachment || viewer.sitting_on.is_some_and(|sat| sat == *linkset) {
                2
            } else {
                let mut priority = distance_priority(viewer_pos, *position);
                if !*physical {
                    priority += 1;
                }
                clamp_priority(priority)
            }
        }
    }
}

/// `NumberOfImmediateQueues + 1`, then `log2(distance / 10)` once distance is past 10 m.
///
/// A distance that is not finite overflows the C# queue index, and that enqueue
/// drops the update (TB72). This returns the last queue instead (D55).
fn distance_priority(from: [f32; 3], to: [f32; 3]) -> i32 {
    let distance = meters_between(from, to);
    if !distance.is_finite() {
        return (QUEUE_COUNT - 1) as i32;
    }
    let mut priority = IMMEDIATE_QUEUES as i32 + 1;
    if distance > 10.0 {
        // The pin writes `1.442695f` and `3.321928f`. Those are these f32 values.
        // Multiply and subtract stay separate, matching that expression.
        let tmp = (f64::from(distance).ln() as f32) * std::f32::consts::LOG2_E
            - std::f32::consts::LOG2_10;
        if !tmp.is_finite() {
            return (QUEUE_COUNT - 1) as i32;
        }
        if tmp > 0.0 {
            priority += tmp as i32;
        }
    }
    priority
}

fn meters_between(a: [f32; 3], b: [f32; 3]) -> f32 {
    let x = a[0] - b[0];
    let y = a[1] - b[1];
    let z = a[2] - b[2];
    f64::from(x * x + y * y + z * z).sqrt() as f32
}

fn clamp_priority(priority: i32) -> u8 {
    let last = (QUEUE_COUNT - 1) as i32;
    if priority >= last {
        last as u8
    } else {
        priority as u8
    }
}

#[cfg(test)]
mod tests {
    use super::{PriorityEntity, PriorityViewer, best_avatar_responsiveness};
    use crate::entity_queue::{EntityUpdateQueue, POSITION};
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn viewer_at(agent: u128, position: [f32; 3], sitting_on: Option<Uuid>) -> PriorityViewer {
        PriorityViewer {
            agent_id: id(agent),
            position: Some(position),
            sitting_on,
        }
    }

    fn avatar(n: u128) -> PriorityEntity {
        PriorityEntity::Avatar { id: id(n) }
    }

    fn prim(
        n: u128,
        linkset: u128,
        attachment: bool,
        physical: bool,
        position: [f32; 3],
    ) -> PriorityEntity {
        PriorityEntity::Prim {
            id: id(n),
            linkset: id(linkset),
            attachment,
            physical,
            position,
        }
    }

    #[test]
    fn the_viewers_own_avatar_is_the_first_queue() {
        let viewer = viewer_at(1, [128.0, 128.0, 25.0], None);
        assert_eq!(best_avatar_responsiveness(&viewer, &avatar(1)), 0);
        let far = viewer_at(1, [0.0, 0.0, 0.0], None);
        assert_eq!(best_avatar_responsiveness(&far, &avatar(1)), 0);
    }

    #[test]
    fn another_avatar_is_the_second_queue_at_any_distance() {
        let near = viewer_at(1, [0.0, 0.0, 0.0], None);
        let far = viewer_at(1, [8000.0, 0.0, 0.0], None);
        assert_eq!(best_avatar_responsiveness(&near, &avatar(2)), 1);
        assert_eq!(best_avatar_responsiveness(&far, &avatar(2)), 1);
    }

    #[test]
    fn an_attachment_stays_on_the_attachment_queue() {
        let viewer = viewer_at(1, [0.0, 0.0, 0.0], None);
        let worn = prim(3, 30, true, false, [4000.0, 0.0, 0.0]);
        let worn_physical = prim(4, 40, true, true, [0.0, 0.0, 0.0]);
        assert_eq!(best_avatar_responsiveness(&viewer, &worn), 2);
        assert_eq!(best_avatar_responsiveness(&viewer, &worn_physical), 2);
    }

    #[test]
    fn the_sat_on_linkset_stays_on_the_attachment_queue() {
        let linkset = 50_u128;
        let sat = viewer_at(1, [0.0, 0.0, 0.0], Some(id(linkset)));
        // Both parts share the root actor's physical flag, and the same queue.
        let root = prim(5, linkset, false, false, [200.0, 0.0, 0.0]);
        let child = prim(6, linkset, false, false, [200.0, 0.0, 0.0]);
        assert_eq!(best_avatar_responsiveness(&sat, &root), 2);
        assert_eq!(best_avatar_responsiveness(&sat, &child), 2);

        let standing = viewer_at(1, [0.0, 0.0, 0.0], None);
        let standing_root = best_avatar_responsiveness(&standing, &root);
        assert_eq!(standing_root, 8);
        assert_eq!(best_avatar_responsiveness(&standing, &child), standing_root);
    }

    #[test]
    fn prims_match_the_csharp_distance_buckets() {
        // Physical, then non-physical. Confirmed with the pin's float formula:
        // (float)Math.Log(distance) * 1.442695f - 3.321928f, truncated toward zero.
        let buckets: &[(f32, u8, u8)] = &[
            (0.0, 3, 4),
            (10.0, 3, 4),
            (10.0001, 3, 4),
            (15.0, 3, 4),
            (19.9, 3, 4),
            (f32::from_bits(0x419f_ffff), 3, 4),
            (20.0, 4, 5),
            (25.0, 4, 5),
            (39.999, 4, 5),
            (f32::from_bits(0x421f_ffff), 4, 5),
            (40.0, 5, 6),
            (f32::from_bits(0x429f_fffd), 5, 6),
            (f32::from_bits(0x429f_fffe), 6, 7),
            (80.0, 6, 7),
            (100.0, 6, 7),
            (160.0, 7, 8),
            (200.0, 7, 8),
            (320.0, 8, 9),
            (640.0, 9, 10),
            (1000.0, 9, 10),
            (1280.0, 10, 11),
            (2560.0, 11, 12),
            (3000.0, 11, 12),
            (f32::from_bits(0x459f_fffb), 11, 12),
            (f32::from_bits(0x459f_fffc), 12, 12),
            (5120.0, 12, 12),
            (6000.0, 12, 12),
            (10240.0, 12, 12),
        ];
        let viewer = viewer_at(1, [0.0, 0.0, 0.0], None);
        for (distance, physical_queue, static_queue) in buckets {
            let at = [*distance, 0.0, 0.0];
            let physical = prim(7, 70, false, true, at);
            let still = prim(8, 80, false, false, at);
            assert_eq!(
                best_avatar_responsiveness(&viewer, &physical),
                *physical_queue,
                "physical {distance}"
            );
            assert_eq!(
                best_avatar_responsiveness(&viewer, &still),
                *static_queue,
                "static {distance}"
            );
        }

        // 6-8-10 triangle, and a 20 m step off a landed avatar.
        let triangle = viewer_at(1, [0.0, 0.0, 0.0], None);
        let ten = prim(9, 90, false, true, [6.0, 8.0, 0.0]);
        assert_eq!(best_avatar_responsiveness(&triangle, &ten), 3);
        let landed = viewer_at(1, [128.0, 128.0, 25.0], None);
        let twenty = prim(10, 100, false, false, [148.0, 128.0, 25.0]);
        assert_eq!(best_avatar_responsiveness(&landed, &twenty), 5);
    }

    #[test]
    fn a_non_finite_distance_is_the_last_queue() {
        let viewer = viewer_at(1, [0.0, 0.0, 0.0], None);
        let infinite = prim(7, 70, false, true, [f32::INFINITY, 0.0, 0.0]);
        let missing = prim(8, 80, false, false, [f32::NAN, 0.0, 0.0]);
        let huge = prim(9, 90, false, true, [f32::MAX, 0.0, 0.0]);
        assert_eq!(best_avatar_responsiveness(&viewer, &infinite), 12);
        assert_eq!(best_avatar_responsiveness(&viewer, &missing), 12);
        assert_eq!(best_avatar_responsiveness(&viewer, &huge), 12);
    }

    #[test]
    fn without_a_presence_only_the_viewers_avatar_stays_off_the_last_queue() {
        let viewer = PriorityViewer {
            agent_id: id(1),
            position: None,
            sitting_on: Some(id(50)),
        };
        assert_eq!(best_avatar_responsiveness(&viewer, &avatar(1)), 0);
        assert_eq!(best_avatar_responsiveness(&viewer, &avatar(2)), 12);
        let sat_upon = prim(5, 50, false, true, [0.0, 0.0, 0.0]);
        assert_eq!(best_avatar_responsiveness(&viewer, &sat_upon), 12);
    }

    #[test]
    fn an_entity_with_the_viewers_id_is_first() {
        let viewer = viewer_at(1, [0.0, 0.0, 0.0], None);
        let same_id = prim(1, 70, false, false, [3000.0, 0.0, 0.0]);
        assert_eq!(best_avatar_responsiveness(&viewer, &same_id), 0);
    }

    #[test]
    fn those_priorities_leave_the_queue_in_responsiveness_order() {
        let viewer = viewer_at(1, [0.0, 0.0, 0.0], None);
        let updates = [
            (60_u32, prim(16, 160, false, false, [100.0, 0.0, 0.0]), 7_u8),
            (50, prim(15, 150, false, false, [0.0, 0.0, 0.0]), 4),
            (40, prim(14, 140, false, true, [0.0, 0.0, 0.0]), 3),
            (30, prim(13, 130, true, false, [50.0, 0.0, 0.0]), 2),
            (20, avatar(2), 1),
            (10, avatar(1), 0),
        ];
        let mut queue = EntityUpdateQueue::new();
        for (local_id, entity, expected) in &updates {
            let priority = best_avatar_responsiveness(&viewer, entity);
            assert_eq!(priority, *expected);
            assert!(queue.enqueue(priority, *local_id, POSITION));
        }
        let mut order = Vec::new();
        while let Some(update) = queue.dequeue() {
            order.push(update.local_id);
        }
        assert_eq!(order, vec![10, 20, 30, 40, 50, 60]);
    }

    #[test]
    fn moving_the_viewer_reprioritizes_a_waiting_prim() {
        let near = viewer_at(1, [0.0, 0.0, 0.0], None);
        let object = prim(8, 80, false, false, [0.0, 0.0, 0.0]);
        let mut queue = EntityUpdateQueue::new();
        let priority = best_avatar_responsiveness(&near, &object);
        assert_eq!(priority, 4);
        assert!(queue.enqueue(priority, 9, POSITION));

        let far = viewer_at(1, [-100.0, 0.0, 0.0], None);
        let next = best_avatar_responsiveness(&far, &object);
        assert_eq!(next, 7);
        queue.reprioritize(|local_id| if local_id == 9 { Some(next) } else { Some(0) });
        assert_eq!(queue.get(9).map(|update| update.priority), Some(7));
    }
}
