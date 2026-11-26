// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One viewer circuit observing a known peer. Pin8 LLClientView sends a full
//! UUID and local ID in CreateAvatarUpdateBlock, only a local ID in a terse
//! update or kill, and the source UUID in SendAnimations.

use super::{STEP, Session, world};
use ferroan_protocol::msgs::{
    AvatarAnimation, AvatarAppearance, CoarseLocationUpdate, UUIDNameReply,
};
use ferroan_protocol::{IncomingPacket, Msg};
use glam::Vec3;
use std::future::Future;
use uuid::Uuid;

pub(super) struct ObserverSession {
    pub(super) session: Session,
    pub(super) seen: ObservedAvatar,
    peer: Uuid,
}

impl ObserverSession {
    pub(super) fn new(session: Session, peer: Uuid) -> Self {
        Self {
            session,
            peer,
            seen: ObservedAvatar::default(),
        }
    }

    pub(super) async fn step(&mut self) -> Result<(), String> {
        self.session.drive(STEP).await.map_err(|e| e.to_string())?;
        self.seen.note(&mut self.session, self.peer)
    }

    /// Keep acknowledging traffic and recording the peer while it logs in or out.
    pub(super) async fn during<T>(
        &mut self,
        action: impl Future<Output = T>,
    ) -> (T, Result<(), String>) {
        tokio::pin!(action);
        loop {
            tokio::select! {
                result = &mut action => return (result, Ok(())),
                step = self.step() => if let Err(error) = step {
                    // The other circuit must finish its login or logout.
                    return (action.await, Err(error));
                },
            }
        }
    }
}

use crate::session::OBSERVER_HISTORY as HISTORY;

fn remember<T>(history: &mut Vec<T>, entry: T) -> Result<(), String> {
    if history.len() >= HISTORY {
        ferroan_diag::refuse(
            ferroan_diag::BOT_OBSERVER_HISTORY,
            ferroan_diag::Kind::Hard,
            HISTORY as u64,
            history.len() as u64 + 1,
            "bot",
        );
        return Err("observer history is full".into());
    }
    history.push(entry);
    Ok(())
}

/// What one bot has seen of the other avatar, of its own, and on its minimap.
#[derive(Debug, Default)]
pub(super) struct ObservedAvatar {
    /// The local id of the other avatar's full `ObjectUpdate`.
    pub(super) local_id: Option<u32>,
    appearance: bool,
    pub(super) prim_arrival: Option<super::prim_arrival::ObservedPrims>,
    /// Local ids in every `KillObject` this bot received.
    pub(super) killed: Vec<u32>,
    /// Every `UUIDNameReply` block: the id, then the first and last name.
    pub(super) names: Vec<(Uuid, String, String)>,
    /// The other avatar's position in its latest full or terse update.
    pub(super) position: Option<Vec3>,
    /// Latest peer position delivered in an avatar terse block for its known local ID.
    pub(super) peer_terse: Option<Vec3>,
    /// Last animation message for the peer, including animation UUIDs and sequences.
    pub(super) animations: Option<AvatarAnimation>,
    /// Own validated movement completion, with the landing look direction.
    pub(super) arrival: Option<(Vec3, Vec3)>,
    own_local_id: Option<u32>,
    /// This bot's own position in its latest full or terse update.
    pub(super) own_position: Option<Vec3>,
    /// The latest `CoarseLocationUpdate`.
    pub(super) coarse: Option<CoarseLocationUpdate>,
    /// The other avatar's minimap cells, X and Y, each change in order.
    pub(super) other_cells: Vec<(u8, u8)>,
    /// This bot's own minimap cells, the same way.
    pub(super) own_cells: Vec<(u8, u8)>,
    /// The first malformed minimap update.
    coarse_error: Option<String>,
}

impl ObservedAvatar {
    fn note(&mut self, session: &mut Session, other: Uuid) -> Result<(), String> {
        let me = session.agent().agent_id;
        while let Some(packet) = session.poll_inbox() {
            self.record(&packet, me, other)?;
        }
        for coarse in session.take_coarse() {
            self.note_coarse(me, other, coarse)?;
        }
        Ok(())
    }

    fn record(&mut self, packet: &IncomingPacket, me: Uuid, other: Uuid) -> Result<(), String> {
        if let Some(trace) = &mut self.prim_arrival {
            trace.note(packet)?;
        }
        if world::chat_heard(packet).is_some() {
            return Err("the region sent ChatFromSimulator".to_string());
        }
        // Pin8 LLClientView.SendKillObject removes pending updates for killed IDs.
        let killed_peer = self.local_id.filter(|id| self.killed.contains(id));
        for object in world::objects_in(packet) {
            if killed_peer == Some(object.local_id) {
                return Err(format!(
                    "full update after KillObject for peer {}",
                    object.local_id
                ));
            }
            if object.pcode != world::PCODE_AVATAR {
                continue;
            }
            if object.full_id == other {
                match self.local_id {
                    Some(first) if first != object.local_id => {
                        return Err(format!(
                            "peer local id changed from {first} to {}",
                            object.local_id
                        ));
                    }
                    None => self.local_id = Some(object.local_id),
                    _ => {}
                }
                self.position = Some(Vec3::from(object.position));
            } else if object.full_id == me {
                self.own_local_id = Some(object.local_id);
                self.own_position = Some(Vec3::from(object.position));
            }
        }
        for (local_id, avatar, position) in world::terse_in(packet) {
            if killed_peer == Some(local_id) {
                return Err(format!("terse update after KillObject for peer {local_id}"));
            }
            if !avatar {
                continue;
            }
            if Some(local_id) == self.local_id {
                self.peer_terse = Some(position);
                self.position = Some(position);
            } else if Some(local_id) == self.own_local_id {
                self.own_position = Some(position);
            }
        }
        if packet.id == AvatarAppearance::ID
                && let Ok(message) = packet.decode::<AvatarAppearance>()
                && message.sender.id == other
                && !message.visual_param.is_empty()
                // A viewer drops an appearance for an avatar it does not
                // know yet: only one after the full update counts.
                && self.local_id.is_some()
        {
            self.appearance = true;
        }
        if packet.id == AvatarAnimation::ID {
            let message = packet
                .decode::<AvatarAnimation>()
                .map_err(|e| e.to_string())?;
            if message.sender.id == other {
                self.animations = Some(message);
            }
        }
        if let Some(pos) = world::movement_position(packet)
            && let Some((look, _)) = world::movement_look(packet)
        {
            self.arrival = Some((pos, look));
        }
        for id in world::killed(packet) {
            remember(&mut self.killed, id)?;
        }
        if packet.id == UUIDNameReply::ID {
            let reply = packet
                .decode::<UUIDNameReply>()
                .map_err(|_| "a UUIDNameReply did not decode".to_string())?;
            for block in reply.uuid_name_block {
                remember(
                    &mut self.names,
                    (
                        block.id,
                        ferroan_protocol::field_str(&block.first_name),
                        ferroan_protocol::field_str(&block.last_name),
                    ),
                )?;
            }
        }
        Ok(())
    }

    /// `SendCoarseLocationUpdate`: one location per listed agent, this bot's
    /// own index in `You` when it is listed, and `Prey` -1.
    fn note_coarse(
        &mut self,
        me: Uuid,
        other: Uuid,
        coarse: CoarseLocationUpdate,
    ) -> Result<(), String> {
        let agents: Vec<Uuid> = coarse.agent_data.iter().map(|data| data.agent_id).collect();
        let you = agents
            .iter()
            .position(|agent| *agent == me)
            .map_or(-1, |index| index as i16);
        if self.coarse_error.is_none() {
            if coarse.location.len() != agents.len() {
                self.coarse_error = Some(format!(
                    "a minimap update lists {} locations for {} agents",
                    coarse.location.len(),
                    agents.len()
                ));
            } else if coarse.index.you != you || coarse.index.prey != -1 {
                self.coarse_error = Some(format!(
                    "a minimap update has You {} and Prey {} for index {you}",
                    coarse.index.you, coarse.index.prey
                ));
            }
        }
        for (agent, cells) in [(other, &mut self.other_cells), (me, &mut self.own_cells)] {
            if let Some(index) = agents.iter().position(|listed| *listed == agent)
                && let Some(location) = coarse.location.get(index)
            {
                let cell = (location.x, location.y);
                if cells.last() != Some(&cell) {
                    remember(cells, cell)?;
                }
            }
        }
        self.coarse = Some(coarse);
        Ok(())
    }

    /// The latest minimap update lists exactly `expected`, each at the cell of
    /// the position this bot last saw for it.
    pub(super) fn dots_match(
        &self,
        me: Uuid,
        other: Uuid,
        expected: &[Uuid],
    ) -> Result<(), String> {
        if let Some(error) = &self.coarse_error {
            return Err(error.clone());
        }
        let coarse = self.coarse.as_ref().ok_or("no minimap update")?;
        let agents: Vec<Uuid> = coarse.agent_data.iter().map(|data| data.agent_id).collect();
        if agents.len() != expected.len() || expected.iter().any(|agent| !agents.contains(agent)) {
            return Err(format!("the minimap lists {} agent(s)", agents.len()));
        }
        for (index, agent) in agents.iter().enumerate() {
            let seen = if *agent == me {
                self.own_position
            } else if *agent == other {
                self.position
            } else {
                None
            };
            let seen = seen.ok_or("no position for a listed agent")?;
            let location = &coarse.location[index];
            let got = [location.x, location.y, location.z];
            if !coarse_cell_near(got, seen) {
                return Err(format!(
                    "a minimap dot is at {},{},{} for an avatar seen at {:.2},{:.2},{:.2}",
                    got[0], got[1], got[2], seen.x, seen.y, seen.z
                ));
            }
        }
        Ok(())
    }

    pub(super) fn named(&self, id: Uuid) -> Vec<(String, String)> {
        self.names
            .iter()
            .filter(|(named, _, _)| *named == id)
            .map(|(_, first, last)| (first.clone(), last.clone()))
            .collect()
    }

    pub(super) fn seen(&self) -> bool {
        self.local_id.is_some() && self.appearance
    }

    pub(super) fn peer_kills(&self) -> usize {
        self.killed
            .iter()
            .filter(|&&id| Some(id) == self.local_id)
            .count()
    }

    pub(super) fn check_departure(&self, leaving_kills: usize) -> Result<(), String> {
        let staying = self.peer_kills();
        if staying != 1 || leaving_kills != 0 {
            return Err(format!(
                "walker kills: observer {staying}, leaving viewer {leaving_kills}"
            ));
        }
        Ok(())
    }
}

/// `SendCoarseLocationUpdate`'s cell for `position`: X and Y as bytes, Z
/// divided by four. Up to 0.1 m either way is allowed, because an update
/// below the terse thresholds is not sent.
fn coarse_cell_near(cell: [u8; 3], position: Vec3) -> bool {
    let byte = |value: f32| (value as i32) as u8;
    let near = |got: u8, value: f32| got == byte(value - 0.1) || got == byte(value + 0.1);
    near(cell[0], position.x) && near(cell[1], position.y) && near(cell[2], position.z * 0.25)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_capture::{Kind, read_capture};
    use ferroan_protocol::msgs::{ImprovedTerseObjectUpdate, ObjectUpdate};
    use ferroan_protocol::{build_packet, parse_packet};

    const CAPTURE: &[u8] = include_bytes!("../../../../fixtures/avatars/avatars.cap");
    const OWNER: Uuid = Uuid::from_u128(0x4f0c8d2e_7a51_4b8e_9c3d_2e6f1a0b7c55);
    const WALKER: Uuid = Uuid::from_u128(0x8c4e1a70_2b59_4d6f_9e13_5a7c0d8b2f44);
    const WALKER_LOCAL: u32 = 1008931090;

    fn captured_packets() -> impl Iterator<Item = IncomingPacket> {
        read_capture(CAPTURE)
            .unwrap()
            .into_iter()
            .filter(|record| record.kind == Kind::UdpOut)
            .map(|record| parse_packet(&record.bytes).unwrap())
    }

    #[test]
    fn pin8_walker_is_identified_before_its_terse_updates_and_kill() {
        let mut watcher = ObservedAvatar::default();
        let full = captured_packets()
            .find(|p| {
                p.id == ObjectUpdate::ID && world::objects_in(p).iter().any(|b| b.full_id == WALKER)
            })
            .unwrap();
        let terse = captured_packets()
            .find(|p| {
                world::terse_in(p)
                    .iter()
                    .any(|(id, _, _)| *id == WALKER_LOCAL)
            })
            .unwrap();
        watcher.record(&terse, OWNER, WALKER).unwrap();
        assert!(watcher.peer_terse.is_none());
        watcher.record(&full, OWNER, WALKER).unwrap();
        assert_eq!(watcher.local_id, Some(WALKER_LOCAL));
        assert!(watcher.peer_terse.is_none());

        let mut wrong_type = terse.decode::<ImprovedTerseObjectUpdate>().unwrap();
        for block in &mut wrong_type.object_data {
            block.data[5] = 0;
        }
        let mut bytes = Vec::new();
        build_packet(&wrong_type, 1, false, false, &[], &mut bytes);
        watcher
            .record(&parse_packet(&bytes).unwrap(), OWNER, WALKER)
            .unwrap();
        assert!(watcher.peer_terse.is_none());

        // The redacted server capture interleaves both circuits. Only the live
        // observer can establish which viewer received a particular datagram.
        let mut unknown = ObservedAvatar::default();
        for packet in captured_packets() {
            watcher.record(&packet, OWNER, WALKER).unwrap();
            unknown.record(&packet, OWNER, Uuid::nil()).unwrap();
        }
        assert!(watcher.seen());
        assert!(watcher.peer_terse.unwrap().x > 130.0);
        assert_eq!(watcher.peer_kills(), 1);
        assert!(unknown.local_id.is_none());
        assert!(unknown.peer_terse.is_none());
        assert_eq!(unknown.peer_kills(), 0);
    }

    #[test]
    fn departure_requires_one_kill_only_on_the_staying_circuit() {
        let full = captured_packets()
            .find(|p| world::objects_in(p).iter().any(|b| b.full_id == WALKER))
            .unwrap();
        let kill = captured_packets()
            .find(|p| world::killed(p).contains(&WALKER_LOCAL))
            .unwrap();
        let mut observer = ObservedAvatar::default();
        observer.record(&full, OWNER, WALKER).unwrap();
        assert!(observer.check_departure(0).is_err());
        observer.record(&kill, OWNER, WALKER).unwrap();
        observer.check_departure(0).unwrap();
        assert!(observer.check_departure(1).is_err());
        observer.record(&kill, OWNER, WALKER).unwrap();
        assert!(observer.check_departure(0).is_err());
    }

    #[test]
    fn killed_peer_cannot_reappear_in_full_or_terse_updates() {
        let full = captured_packets()
            .find(|p| world::objects_in(p).iter().any(|b| b.full_id == WALKER))
            .unwrap();
        let terse = captured_packets()
            .find(|p| {
                world::terse_in(p)
                    .iter()
                    .any(|(id, _, _)| *id == WALKER_LOCAL)
            })
            .unwrap();
        let kill = captured_packets()
            .find(|p| world::killed(p).contains(&WALKER_LOCAL))
            .unwrap();
        let mut observer = ObservedAvatar::default();
        observer.record(&full, OWNER, WALKER).unwrap();
        observer.record(&terse, OWNER, WALKER).unwrap();
        observer.record(&kill, OWNER, WALKER).unwrap();
        for (kind, packet) in [("full", full), ("terse", terse)] {
            assert_eq!(
                observer.record(&packet, OWNER, WALKER).unwrap_err(),
                format!("{kind} update after KillObject for peer {WALKER_LOCAL}")
            );
        }
    }

    #[test]
    fn another_full_update_cannot_replace_a_killed_peers_local_id() {
        let mut observer = ObservedAvatar::default();
        let full = captured_packets()
            .find(|p| world::objects_in(p).iter().any(|b| b.full_id == WALKER))
            .unwrap();
        let kill = captured_packets()
            .find(|p| world::killed(p).contains(&WALKER_LOCAL))
            .unwrap();
        observer.record(&full, OWNER, WALKER).unwrap();
        observer.record(&kill, OWNER, WALKER).unwrap();
        let mut update = full.decode::<ObjectUpdate>().unwrap();
        update
            .object_data
            .iter_mut()
            .find(|b| b.full_id == WALKER)
            .unwrap()
            .id += 1;
        let mut bytes = Vec::new();
        build_packet(&update, 3, true, false, &[], &mut bytes);
        assert!(
            observer
                .record(&parse_packet(&bytes).unwrap(), OWNER, WALKER)
                .is_err()
        );
        assert_eq!(observer.local_id, Some(WALKER_LOCAL));
        assert_eq!(observer.peer_kills(), 1);
    }

    #[test]
    fn pin8_animation_list_is_recorded_only_for_its_sender() {
        let packet = captured_packets()
            .find(|p| {
                p.id == AvatarAnimation::ID
                    && !p
                        .decode::<AvatarAnimation>()
                        .unwrap()
                        .animation_list
                        .is_empty()
            })
            .unwrap();
        let expected = packet.decode::<AvatarAnimation>().unwrap();
        let peer = expected.sender.id;
        let mut watcher = ObservedAvatar::default();
        watcher.record(&packet, peer, Uuid::from_u128(1)).unwrap();
        assert!(watcher.animations.is_none());
        watcher.record(&packet, Uuid::from_u128(1), peer).unwrap();
        let seen = watcher.animations.unwrap();
        assert_eq!(seen.sender.id, peer);
        assert_eq!(seen.animation_list.len(), expected.animation_list.len());
        for (actual, expected) in seen.animation_list.iter().zip(expected.animation_list) {
            assert_eq!(actual.anim_id, expected.anim_id);
            assert_eq!(actual.anim_sequence_id, expected.anim_sequence_id);
        }
    }

    #[test]
    fn full_observer_history_fails_without_discarding_the_evidence() {
        let mut entries: Vec<usize> = (0..HISTORY).collect();
        assert!(remember(&mut entries, HISTORY).is_err());
        assert_eq!(entries.len(), HISTORY);
        assert_eq!(entries[0], 0);
    }
}
