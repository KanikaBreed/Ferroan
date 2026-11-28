// Copyright (c) Contributors, http://opensimulator.org/
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
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF
// THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// The sound gate is ported from SceneObjectPart.cs. Cite that file's lines
// 1-26. The play check is CollisionSounds.cs. The text is
// licenses/OpenSimulator-BSD-3-Clause.txt.

// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.PhysicsModules.Jolt/CollisionFrameTracker.cs
//   (lines 45-212)
//   and
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs
//   (DispatchContacts, lines 3253-3416, and StillTouchingAsleep, lines 3541-3549)
//   and
//   Source/OpenSim.Region.Framework/Scenes/SceneObjectPart.cs
//   (lines 3036-3062)
//   and
//   Source/OpenSim.Region.Framework/Scenes/CollisionSounds.cs
//   (lines 170-179 and 219-228)
//   KanikaBreed/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4.
// The actor set remains available under MPL-2.0. The sound gate remains
// available under BSD-3-Clause. Both form part of Ferroan's GPL-3.0-or-later
// work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause

//! One frame of actor-set collision dispatch.
//!
//! `DispatchContacts` turns contact reports into one set per subscribed prim.
//! A duplicate pair in the same heartbeat collapses when substeps are merged.
//! A pair that is asleep on both sides stays. Ground alone is sent again.
//! Overflow ends nobody. Current nonempty sets go out before those ends, so a
//! lower id that ends stays after a higher id that acquires a contact.
//! Actors or contacts past the contact-buffer cap are dropped, that frame is
//! treated as overflow, and actor ids kept across overflow stay inside the cap.
//! Repeat timestamps are kept only for those retained ids.
//! A sound is `abs(speed) > 0.2` on a new contact, for either sign, and the
//! play path also requires `abs(speed) >= 0.2f`.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use ferroan_diag::{Kind, PHYSICS_CONTACTS, drop_bound};

/// `CollisionFrameTracker.MinRepeatSeconds`.
pub const MIN_REPEAT_SECONDS: f64 = 0.13;

/// The double subtracted inside `ShouldDeliverToPart` (`1e-9`).
pub const MIN_REPEAT_EPSILON: f64 = 1e-9;

/// A contact the dispatcher kept. Penetration is 0 and feet stay true, which
/// is the prim constructor `DispatchContacts` uses.
#[derive(Clone, Copy)]
pub struct KeptContact {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub relative_speed: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContactPhase {
    Begin,
    Persist,
    End,
}

/// One report in the heartbeat buffer. Child ids are the struck parts. User
/// ids are the bodies, which name the other side.
pub struct ContactReport {
    pub phase: ContactPhase,
    pub child_a: u32,
    pub user_a: u32,
    pub child_b: u32,
    pub user_b: u32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
    pub relative_speed: f32,
}

#[derive(Clone, Copy)]
pub struct Actor {
    pub id: u32,
    pub subscribed: bool,
    pub physical: bool,
    pub awake: bool,
}

pub struct FrameInput {
    pub time: f64,
    pub merge_substeps: bool,
    pub overflow: bool,
    /// Resolved `JoltScene._contactBufMax`. Extra actors and contacts are dropped.
    pub capacity: usize,
    pub actors: Vec<Actor>,
    pub contacts: Vec<ContactReport>,
}

#[derive(Clone)]
pub struct Delivery {
    pub prim: u32,
    pub colliders: Vec<(u32, KeptContact)>,
}

pub struct FrameOutput {
    pub sends: Vec<Delivery>,
    pub scores: Vec<(u32, i32)>,
    pub tracked: Vec<u32>,
    pub current: Vec<Delivery>,
}

#[derive(Clone)]
struct EventSet {
    colliders: BTreeMap<u32, KeptContact>,
    identity: u64,
}

/// The tracker's accumulate, hold and end decisions, plus one dispatch frame.
pub struct ActorSets {
    current: BTreeMap<u32, EventSet>,
    previous: BTreeMap<u32, EventSet>,
    fresh: BTreeSet<u32>,
    collided_last: BTreeSet<u32>,
    scores: BTreeMap<u32, i32>,
    prev_scored: BTreeSet<u32>,
    last_sent: BTreeMap<u32, f64>,
    now: f64,
    next_identity: u64,
}

impl ActorSets {
    pub fn new() -> Self {
        Self {
            current: BTreeMap::new(),
            previous: BTreeMap::new(),
            fresh: BTreeSet::new(),
            collided_last: BTreeSet::new(),
            scores: BTreeMap::new(),
            prev_scored: BTreeSet::new(),
            last_sent: BTreeMap::new(),
            now: 0.0,
            next_identity: 1,
        }
    }

    /// Replay one `DispatchContacts` call. Avatar delivery is P3.18c. Volume
    /// detect on these actors is false.
    pub fn dispatch(&mut self, frame: &FrameInput) -> FrameOutput {
        self.begin_frame();
        self.now = frame.time;
        let capacity = frame.capacity;
        let configured = u64::try_from(capacity).unwrap_or(u64::MAX);
        let mut overflow = frame.overflow;
        let mut actor_n = frame.actors.len();
        let mut contact_n = frame.contacts.len();
        if actor_n > capacity {
            drop_bound(
                PHYSICS_CONTACTS,
                Kind::Hard,
                configured,
                u64::try_from(actor_n).unwrap_or(u64::MAX),
                "physics.collision",
            );
            actor_n = capacity;
            overflow = true;
        }
        if contact_n > capacity {
            drop_bound(
                PHYSICS_CONTACTS,
                Kind::Hard,
                configured,
                u64::try_from(contact_n).unwrap_or(u64::MAX),
                "physics.collision",
            );
            contact_n = capacity;
            overflow = true;
        }
        let admitted_actors = &frame.actors[..actor_n];
        let admitted_contacts = &frame.contacts[..contact_n];
        let actors = actor_map(admitted_actors);
        let mut pairs = HashSet::new();
        for contact in admitted_contacts {
            if contact.phase == ContactPhase::End {
                continue;
            }
            if frame.merge_substeps && !pairs.insert(pair_key(contact.child_a, contact.child_b)) {
                continue;
            }
            self.count_contact(contact.child_a);
            self.count_contact(contact.child_b);
            let point = KeptContact {
                position: contact.point,
                normal: contact.normal,
                relative_speed: contact.relative_speed,
            };
            if subscribed(&actors, contact.child_a) {
                self.add_collider(contact.child_a, contact.user_b, point);
            }
            if subscribed(&actors, contact.child_b) {
                let mut toward_b = point;
                toward_b.normal = negate(contact.normal);
                self.add_collider(contact.child_b, contact.user_a, toward_b);
            }
        }
        self.hold_asleep(&actors);
        let mut current_sends = Vec::new();
        let ids: Vec<u32> = self.current.keys().copied().collect();
        for prim in ids {
            let Some(set) = self.current.get(&prim).cloned() else {
                continue;
            };
            if !actors.contains_key(&prim) || !self.should_deliver_to_part(prim, &set) {
                continue;
            }
            current_sends.push(delivery(prim, &set));
        }
        current_sends.sort_by_key(|send| send.prim);
        let ended = self.end_frame(overflow, capacity);
        let mut end_sends = Vec::new();
        for prim in ended {
            if subscribed(&actors, prim) {
                end_sends.push(Delivery {
                    prim,
                    colliders: Vec::new(),
                });
            }
        }
        end_sends.sort_by_key(|send| send.prim);
        let mut sends = current_sends;
        sends.extend(end_sends);
        let mut scores = Vec::new();
        for actor in admitted_actors {
            scores.push((actor.id, self.scores.get(&actor.id).copied().unwrap_or(0)));
        }
        scores.sort_by_key(|(id, _)| *id);
        let current = self
            .current
            .iter()
            .map(|(prim, set)| delivery(*prim, set))
            .collect();
        FrameOutput {
            sends,
            scores,
            tracked: self.collided_last.iter().copied().collect(),
            current,
        }
    }

    fn begin_frame(&mut self) {
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.fresh.clear();
        self.prev_scored = self.scores.keys().copied().collect();
        self.scores.clear();
    }

    fn count_contact(&mut self, struck: u32) {
        if struck == 0 {
            return;
        }
        let entry = self.scores.entry(struck).or_insert(0);
        *entry = entry.wrapping_add(1);
    }

    fn add_collider(&mut self, prim: u32, collider: u32, contact: KeptContact) {
        self.fresh.insert(prim);
        self.hold(prim, collider, contact);
    }

    fn hold(&mut self, prim: u32, collider: u32, contact: KeptContact) {
        if !self.current.contains_key(&prim) {
            let identity = self.next_identity;
            self.next_identity = self.next_identity.wrapping_add(1);
            self.current.insert(
                prim,
                EventSet {
                    colliders: BTreeMap::new(),
                    identity,
                },
            );
        }
        let Some(set) = self.current.get_mut(&prim) else {
            return;
        };
        add_point(&mut set.colliders, collider, contact);
    }

    fn hold_all(&mut self, prim: u32, last: &EventSet) {
        self.current.insert(prim, last.clone());
    }

    fn touches(&self, prim: u32, collider: u32) -> bool {
        self.current
            .get(&prim)
            .is_some_and(|set| set.colliders.contains_key(&collider))
    }

    fn hold_asleep(&mut self, actors: &BTreeMap<u32, Actor>) {
        let ids: Vec<u32> = self.previous.keys().copied().collect();
        for prim in ids {
            if !subscribed(actors, prim) {
                continue;
            }
            let Some(last) = self.previous.get(&prim).cloned() else {
                continue;
            };
            if !self.current.contains_key(&prim) && all_asleep(actors, prim, &last) {
                self.hold_all(prim, &last);
                continue;
            }
            for (collider, contact) in &last.colliders {
                if !self.touches(prim, *collider) && still_asleep(actors, prim, *collider) {
                    self.hold(prim, *collider, *contact);
                }
            }
        }
    }

    fn should_deliver(&self, prim: u32, set: &EventSet) -> bool {
        if self.fresh.contains(&prim) || !self.previous.contains_key(&prim) {
            return true;
        }
        let Some(last) = self.previous.get(&prim) else {
            return true;
        };
        if set.identity == last.identity {
            return set.colliders.len() == 1 && set.colliders.contains_key(&0);
        }
        if set.colliders.len() == 1 && set.colliders.contains_key(&0) {
            return true;
        }
        if set.colliders.len() != last.colliders.len() {
            return true;
        }
        set.colliders
            .keys()
            .any(|id| !last.colliders.contains_key(id))
    }

    fn should_deliver_to_part(&mut self, prim: u32, set: &EventSet) -> bool {
        if !self.should_deliver(prim, set) {
            return false;
        }
        if !names_object(set) {
            return true;
        }
        if let Some(last) = self.previous.get(&prim)
            && same_colliders(set, last)
            && let Some(sent) = self.last_sent.get(&prim)
            && self.now - sent < MIN_REPEAT_SECONDS - MIN_REPEAT_EPSILON
        {
            return false;
        }
        self.last_sent.insert(prim, self.now);
        true
    }

    fn end_frame(&mut self, overflow: bool, capacity: usize) -> Vec<u32> {
        if overflow {
            let fresh = self
                .current
                .keys()
                .filter(|id| !self.collided_last.contains(id))
                .count();
            let room = capacity.saturating_sub(self.collided_last.len());
            if fresh > room {
                drop_bound(
                    PHYSICS_CONTACTS,
                    Kind::Hard,
                    u64::try_from(capacity).unwrap_or(u64::MAX),
                    u64::try_from(self.collided_last.len().saturating_add(fresh))
                        .unwrap_or(u64::MAX),
                    "physics.collision",
                );
            }
            let mut left = room;
            for id in self.current.keys().copied() {
                if self.collided_last.contains(&id) {
                    continue;
                }
                if left == 0 {
                    break;
                }
                self.collided_last.insert(id);
                left -= 1;
            }
            self.retain_timestamps();
            return Vec::new();
        }
        let mut ended = Vec::new();
        let last: Vec<u32> = self.collided_last.iter().copied().collect();
        for id in last {
            if !self.current.contains_key(&id) {
                ended.push(id);
                self.last_sent.remove(&id);
            }
        }
        self.collided_last.clear();
        for id in self.current.keys() {
            self.collided_last.insert(*id);
        }
        self.retain_timestamps();
        ended.sort_unstable();
        ended
    }

    /// A refused id can be sent and still stay out of [`Self::collided_last`].
    /// Its timestamp would otherwise survive until a later frame that happens
    /// to retain it. Both stores stay on the ids the overflow rule kept.
    fn retain_timestamps(&mut self) {
        self.last_sent
            .retain(|id, _| self.collided_last.contains(id));
    }
}

impl Default for ActorSets {
    fn default() -> Self {
        Self::new()
    }
}

/// A collision sound. `started` is false when the same collider was already
/// touching. The first compare is the double literal `0.2`. The second is the
/// float literal on the play path.
pub fn collision_sound(speed_bits: u32, started: bool) -> bool {
    if !started {
        return false;
    }
    let speed = f32::from_bits(speed_bits).abs();
    if f64::from(speed) <= 0.2 {
        return false;
    }
    speed >= 0.2
}

fn actor_map(actors: &[Actor]) -> BTreeMap<u32, Actor> {
    actors.iter().map(|actor| (actor.id, *actor)).collect()
}

fn subscribed(actors: &BTreeMap<u32, Actor>, id: u32) -> bool {
    actors.get(&id).is_some_and(|actor| actor.subscribed)
}

fn pair_key(a: u32, b: u32) -> u64 {
    if a <= b {
        ((a as u64) << 32) | u64::from(b)
    } else {
        ((b as u64) << 32) | u64::from(a)
    }
}

fn negate(normal: [f32; 3]) -> [f32; 3] {
    [-normal[0], -normal[1], -normal[2]]
}

fn names_object(set: &EventSet) -> bool {
    set.colliders.keys().any(|id| *id != 0)
}

fn same_colliders(a: &EventSet, b: &EventSet) -> bool {
    a.colliders.len() == b.colliders.len()
        && a.colliders.keys().all(|id| b.colliders.contains_key(id))
}

fn all_asleep(actors: &BTreeMap<u32, Actor>, prim: u32, set: &EventSet) -> bool {
    set.colliders
        .keys()
        .all(|collider| still_asleep(actors, prim, *collider))
}

fn still_asleep(actors: &BTreeMap<u32, Actor>, prim: u32, collider: u32) -> bool {
    let Some(actor) = actors.get(&prim) else {
        return false;
    };
    if actor.awake {
        return false;
    }
    if collider == 0 {
        return actor.physical;
    }
    let Some(other) = actors.get(&collider) else {
        return false;
    };
    if other.awake {
        return false;
    }
    actor.physical || other.physical
}

fn add_point(colliders: &mut BTreeMap<u32, KeptContact>, id: u32, contact: KeptContact) {
    if let Some(current) = colliders.get_mut(&id) {
        if current.relative_speed.abs() < contact.relative_speed.abs() {
            current.relative_speed = contact.relative_speed;
        }
        return;
    }
    colliders.insert(id, contact);
}

fn delivery(prim: u32, set: &EventSet) -> Delivery {
    Delivery {
        prim,
        colliders: set
            .colliders
            .iter()
            .map(|(id, contact)| (*id, *contact))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Actor, ActorSets, ContactPhase, ContactReport, FrameInput, FrameOutput, MIN_REPEAT_SECONDS,
        collision_sound,
    };

    #[test]
    fn actor_sets_match_the_pinned_dispatch() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-file");
        let manifest = std::fs::read_to_string(root.join("manifest.txt")).expect("manifest");
        assert!(manifest.lines().any(|line| line == "collision"));
        let text = std::fs::read_to_string(root.join("collision/replay.txt")).expect("replay");
        assert!(text.contains("source JoltScene.cs 3253-3416\n"));
        assert!(text.contains("tracker CollisionFrameTracker.cs 45-212\n"));
        assert!(text.contains("asleep JoltScene.cs 3541-3549\n"));
        assert!(text.contains("sound SceneObjectPart.cs 3036-3062\n"));
        assert!(text.contains("native not_called\n"));
        assert!(text.contains("scripts not_called\n"));
        let min_repeat = value(&text, "min_repeat ");
        assert_eq!(min_repeat, format!("{:016X}", MIN_REPEAT_SECONDS.to_bits()));
        assert_eq!(
            value(&text, "threshold_float "),
            format!("{:08X}", 0.2f32.to_bits())
        );
        let capacity: usize = value(&text, "contact_cap ").parse().expect("contact cap");
        // JoltScene.cs:308, before AddRegion replaces the field.
        assert_eq!(capacity, 2048);

        let mut cases = text.split("\ncase ");
        let _header = cases.next().expect("header");
        for case_body in cases {
            let (name, body) = case_body.split_once('\n').expect("case name");
            if name == "sound" {
                assert_sounds(body);
                continue;
            }
            let mut replay = ActorSets::new();
            for frame_text in body
                .split("endframe\n")
                .filter(|part| part.contains("frame "))
            {
                let input = parse_frame(frame_text, capacity);
                let output = replay.dispatch(&input);
                let got = format_output(&output);
                let expected = output_section(frame_text);
                assert!(
                    expected.starts_with("phase current\n"),
                    "case {name} keeps the current phase first"
                );
                assert!(
                    expected.contains("\nphase end\n"),
                    "case {name} keeps the end phase"
                );
                assert_eq!(got, expected, "case {name}");
                assert_case_shape(name, &input, &output);
            }
        }
    }

    fn assert_sounds(body: &str) {
        let mut probes = 0u32;
        for line in body.lines() {
            let Some(rest) = line.strip_prefix("probe ") else {
                continue;
            };
            let mut words = rest.split(' ');
            let _kind = words.next().expect("kind");
            let when = words.next().expect("when");
            let bits = u32::from_str_radix(words.next().expect("bits"), 16).expect("speed bits");
            let played = words.next().expect("played") == "1";
            let started = when == "fresh";
            assert_eq!(collision_sound(bits, started), played, "{line}");
            probes += 1;
        }
        assert!(probes > 8);
    }

    fn assert_case_shape(name: &str, input: &FrameInput, output: &FrameOutput) {
        match name {
            "substep-merge" => {
                assert!(input.merge_substeps);
                assert_eq!(score(output, 1), 1);
                assert_eq!(score(output, 2), 1);
                assert_eq!(
                    send_speed(output, 1, 2),
                    input.contacts[0].relative_speed.to_bits()
                );
            }
            "substep-keep" => {
                assert!(!input.merge_substeps);
                assert_eq!(score(output, 1), 2);
                assert_eq!(score(output, 2), 2);
                assert_eq!(
                    send_speed(output, 1, 2),
                    input.contacts[1].relative_speed.to_bits()
                );
            }
            "end-ignored" => {
                assert!(output.sends.is_empty());
                assert!(output.tracked.is_empty());
                assert_eq!(score(output, 1), 0);
            }
            "held-terrain" if input.contacts.is_empty() => {
                assert!(send_names(output, 1, 0));
                assert!(output.tracked.contains(&1));
            }
            "awake-ground-ends" | "static-ground-ends" if input.contacts.is_empty() => {
                assert!(send_empty(output, 1));
            }
            "sleeping-pair" if input.contacts.is_empty() => {
                assert!(output.sends.is_empty());
                assert!(current_names(output, 4, 5));
                assert!(current_names(output, 5, 4));
                assert!(output.tracked.contains(&4) && output.tracked.contains(&5));
            }
            "waking-pair" if input.contacts.is_empty() => {
                assert!(send_empty(output, 4));
                assert!(send_empty(output, 5));
            }
            "overflow-partial" if input.overflow => {
                assert!(send_names(output, 1, 2));
                assert!(!send_names(output, 1, 3));
                assert!(
                    !output
                        .sends
                        .iter()
                        .any(|send| send.prim == 3 && send.colliders.is_empty())
                );
                assert!(
                    !output
                        .sends
                        .iter()
                        .any(|send| send.prim == 4 && send.colliders.is_empty())
                );
                for id in [3, 4, 5] {
                    assert!(output.tracked.contains(&id));
                }
            }
            "repeat-window" => {
                if (input.time - 100.0).abs() < 1e-15 {
                    assert!(send_names(output, 1, 2));
                }
            }
            "score-unsubscribed" => {
                assert!(send_names(output, 1, 2));
                assert!(!output.sends.iter().any(|send| send.prim == 2));
                assert_eq!(score(output, 1), 1);
                assert_eq!(score(output, 2), 1);
            }
            "phase-order" if input.time == 1.0 => {
                let pos = |prim: u32, empty: bool| {
                    output
                        .sends
                        .iter()
                        .position(|send| send.prim == prim && send.colliders.is_empty() == empty)
                        .expect("phase send")
                };
                let current_2 = pos(2, false);
                let current_3 = pos(3, false);
                let end_1 = pos(1, true);
                assert!(current_2 < current_3);
                assert!(current_3 < end_1);
            }
            _ => {}
        }
    }

    #[test]
    fn contact_admission_stops_at_the_buffer_cap() {
        let _guard = limits_lock();
        let before = limit_hits();
        let mut replay = ActorSets::new();
        let first = replay.dispatch(&FrameInput {
            time: 0.0,
            merge_substeps: false,
            overflow: false,
            capacity: 2,
            actors: vec![actor(1), actor(2), actor(3), actor(4)],
            contacts: vec![persist(1, 2), persist(3, 4), persist(1, 3)],
        });
        assert_eq!(limit_hits(), before + 2);
        assert!(first.sends.iter().all(|send| !send.colliders.is_empty()));
        assert_eq!(first.tracked, vec![1, 2]);
        assert_eq!(first.scores.len(), 2);

        let second = replay.dispatch(&FrameInput {
            time: 1.0,
            merge_substeps: false,
            overflow: true,
            capacity: 2,
            actors: vec![actor(5), actor(6), actor(7), actor(8)],
            contacts: vec![persist(5, 6), persist(7, 8), persist(5, 7)],
        });
        assert_eq!(limit_hits(), before + 5);
        assert!(second.sends.iter().all(|send| !send.colliders.is_empty()));
        assert_eq!(second.tracked, vec![1, 2]);
    }

    #[test]
    fn repeat_timestamps_follow_the_retained_ids() {
        let _guard = limits_lock();
        let mut replay = ActorSets::new();
        let settled = replay.dispatch(&FrameInput {
            time: 0.0,
            merge_substeps: false,
            overflow: false,
            capacity: 2,
            actors: vec![actor(1), actor(2)],
            contacts: vec![persist(1, 2)],
        });
        assert_eq!(settled.tracked, vec![1, 2]);
        assert_eq!(replay.last_sent.len(), 2);

        for (step, base) in [(1.0, 3u32), (2.0, 7u32), (3.0, 11u32)] {
            let _ = replay.dispatch(&FrameInput {
                time: step,
                merge_substeps: false,
                overflow: true,
                capacity: 2,
                actors: vec![
                    actor(base),
                    actor(base + 1),
                    actor(base + 2),
                    actor(base + 3),
                ],
                contacts: vec![persist(base, base + 1)],
            });
            assert_eq!(
                replay.collided_last.iter().copied().collect::<Vec<_>>(),
                vec![1, 2]
            );
            assert!(
                replay.last_sent.len() <= 2,
                "step {step} kept {} timestamps",
                replay.last_sent.len()
            );
            assert!(
                replay
                    .last_sent
                    .keys()
                    .all(|id| replay.collided_last.contains(id))
            );
            assert!(!replay.last_sent.contains_key(&base));
            assert!(!replay.last_sent.contains_key(&(base + 1)));
        }

        let recovered = replay.dispatch(&FrameInput {
            time: 4.0,
            merge_substeps: false,
            overflow: false,
            capacity: 2,
            actors: vec![actor(11), actor(12)],
            contacts: vec![persist(11, 12)],
        });
        assert_eq!(recovered.tracked, vec![11, 12]);
        let stamps: Vec<u32> = replay.last_sent.keys().copied().collect();
        assert_eq!(stamps, vec![11, 12]);
    }

    fn actor(id: u32) -> Actor {
        Actor {
            id,
            subscribed: true,
            physical: true,
            awake: true,
        }
    }

    fn persist(a: u32, b: u32) -> ContactReport {
        ContactReport {
            phase: ContactPhase::Persist,
            child_a: a,
            user_a: a,
            child_b: b,
            user_b: b,
            point: [1.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            relative_speed: 0.5,
        }
    }

    fn limit_hits() -> u64 {
        let needle =
            "ferroan_limit_hits_total{limit=\"physics.contacts\",action=\"drop\"} ".to_string();
        ferroan_diag::metrics()
            .lines()
            .find_map(|line| line.strip_prefix(&needle)?.parse().ok())
            .unwrap_or(0)
    }

    fn limits_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn score(output: &FrameOutput, id: u32) -> i32 {
        output
            .scores
            .iter()
            .find(|(actor, _)| *actor == id)
            .expect("score")
            .1
    }

    fn send_speed(output: &FrameOutput, prim: u32, collider: u32) -> u32 {
        let send = output
            .sends
            .iter()
            .find(|send| send.prim == prim)
            .expect("send");
        let (_, contact) = send
            .colliders
            .iter()
            .find(|(id, _)| *id == collider)
            .expect("collider");
        contact.relative_speed.to_bits()
    }

    fn send_names(output: &FrameOutput, prim: u32, collider: u32) -> bool {
        output
            .sends
            .iter()
            .any(|send| send.prim == prim && send.colliders.iter().any(|(id, _)| *id == collider))
    }

    fn send_empty(output: &FrameOutput, prim: u32) -> bool {
        output
            .sends
            .iter()
            .any(|send| send.prim == prim && send.colliders.is_empty())
    }

    fn current_names(output: &FrameOutput, prim: u32, collider: u32) -> bool {
        output
            .current
            .iter()
            .any(|set| set.prim == prim && set.colliders.iter().any(|(id, _)| *id == collider))
    }

    fn parse_frame(text: &str, capacity: usize) -> FrameInput {
        let mut time = 0.0f64;
        let mut merge_substeps = false;
        let mut overflow = false;
        let mut actors = Vec::new();
        let mut contacts = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("time ") {
                time = f64::from_bits(u64::from_str_radix(rest, 16).expect("time"));
            } else if let Some(rest) = line.strip_prefix("merge ") {
                merge_substeps = rest == "1";
            } else if let Some(rest) = line.strip_prefix("overflow ") {
                overflow = rest == "1";
            } else if let Some(rest) = line.strip_prefix("actor ") {
                let mut words = rest.split(' ');
                actors.push(Actor {
                    id: words.next().expect("id").parse().expect("actor"),
                    subscribed: words.next().expect("subscribed") == "1",
                    physical: words.next().expect("physical") == "1",
                    awake: words.next().expect("awake") == "1",
                });
            } else if let Some(rest) = line.strip_prefix("contact ") {
                contacts.push(parse_contact(rest));
            }
        }
        FrameInput {
            time,
            merge_substeps,
            overflow,
            capacity,
            actors,
            contacts,
        }
    }

    fn parse_contact(rest: &str) -> ContactReport {
        let mut words = rest.split(' ');
        let phase = match words.next().expect("phase") {
            "begin" => ContactPhase::Begin,
            "persist" => ContactPhase::Persist,
            "end" => ContactPhase::End,
            other => panic!("phase {other}"),
        };
        let id =
            |words: &mut std::str::Split<'_, char>| words.next().expect("id").parse().expect("id");
        let child_a = id(&mut words);
        let user_a = id(&mut words);
        let child_b = id(&mut words);
        let user_b = id(&mut words);
        let bits = |words: &mut std::str::Split<'_, char>| {
            f32::from_bits(u32::from_str_radix(words.next().expect("bits"), 16).expect("float"))
        };
        ContactReport {
            phase,
            child_a,
            user_a,
            child_b,
            user_b,
            point: [bits(&mut words), bits(&mut words), bits(&mut words)],
            normal: [bits(&mut words), bits(&mut words), bits(&mut words)],
            relative_speed: bits(&mut words),
        }
    }

    fn output_section(frame_text: &str) -> String {
        let mut out = String::new();
        let mut started = false;
        for line in frame_text.lines() {
            if line.starts_with("phase ") || line.starts_with("send ") || line.starts_with("score ")
            {
                started = true;
            }
            if started && !line.is_empty() {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    fn format_output(output: &FrameOutput) -> String {
        let mut text = String::new();
        text.push_str("phase current\n");
        for send in output
            .sends
            .iter()
            .filter(|send| !send.colliders.is_empty())
        {
            text.push_str(&format!("send {}\n", send.prim));
            for (id, contact) in &send.colliders {
                write_contact(&mut text, "sent", *id, contact);
            }
        }
        text.push_str("phase end\n");
        for send in output.sends.iter().filter(|send| send.colliders.is_empty()) {
            text.push_str(&format!("send {} empty\n", send.prim));
        }
        for (id, score) in &output.scores {
            text.push_str(&format!("score {id} {score}\n"));
        }
        text.push_str("tracked");
        for id in &output.tracked {
            text.push(' ');
            text.push_str(&id.to_string());
        }
        text.push('\n');
        for set in &output.current {
            text.push_str(&format!("current {}\n", set.prim));
            for (id, contact) in &set.colliders {
                write_contact(&mut text, "held", *id, contact);
            }
        }
        text
    }

    fn write_contact(text: &mut String, label: &str, id: u32, contact: &super::KeptContact) {
        text.push_str(label);
        text.push(' ');
        text.push_str(&id.to_string());
        for value in contact
            .position
            .iter()
            .chain(contact.normal.iter())
            .chain(std::iter::once(&contact.relative_speed))
        {
            text.push(' ');
            text.push_str(&format!("{:08X}", value.to_bits()));
        }
        text.push('\n');
    }

    fn value<'a>(text: &'a str, prefix: &str) -> &'a str {
        text.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .expect("prefix")
    }
}
