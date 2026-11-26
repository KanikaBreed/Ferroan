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
//   Source/OpenSim.Region.CoreModules/Framework/EntityTransfer/EntityTransferModule.cs
//     (EnableChildAgents, InformClientOfNeighbourAsync, GetChildSeed reuse)
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs
//     (CloseChildAgents, closeAllChildAgents, knowsNeighbourRegion)
//   Source/OpenSim.Framework/CapsUtil.cs (GetCapsSeedPath, GetRandomCapsObjectPath)
//   OpenSim-NGC/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One owner for each child destination of a root circuit generation.
//!
//! The seed is stored before the admission POST, as `EnableChildAgents` does.
//! A second claim for that destination does not start another POST. Refusal
//! drops the claim and produces no viewer events. A failed POST before logout
//! drops the claim the same way, so the next pass can POST again. Logout
//! during the POST closes an accepted child once and does not announce it
//! (D106). A timeout or a transport failure after logout closes once too. A
//! completion after the hop promoted the destination still announces once,
//! including when the same session has carried that neighbour forward. The
//! carry keeps the promoted flag until this generation is root again, so a
//! later login does not close a destination that is still the remote root.
//! A different session closes the old root's other open children with the
//! old session id. An in-view hop reads the same seed.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Mutex;

use ferroan_llsd::Llsd;
use uuid::Uuid;

use crate::simulation_client::RegionEnd;

/// Destinations remembered for one root circuit generation.
/// Pin8's seed map is unbounded. A draw distance covers a handful of neighbours.
pub const CHILD_DESTINATIONS: usize = 64;

/// What the caller may do with a destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// This caller owns the one admission POST. The seed is already stored.
    New { seed: String },
    /// A pending or open admission already owns this destination. Do not POST.
    Existing { seed: String },
    /// This generation already holds [`CHILD_DESTINATIONS`]. Nothing was stored.
    Bound,
    /// No generation owns this agent, this generation is not current, or it
    /// has logged out. Nothing was stored.
    Unavailable,
}

/// What an admission completion may tell the viewer and the neighbour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    /// `EnableSimulator`, then `EstablishAgentCommunication`, using `seed`.
    Announce { seed: String, end: RegionEnd },
    /// One `CloseAgent`. Do not announce.
    Close { end: RegionEnd },
    /// Refusal, a stale generation, a destination another generation owns,
    /// or a completion that must not be sent again.
    Quiet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Pending,
    Open,
    /// Logout won while the POST was in flight. Success closes once.
    CloseWhenAccepted,
}

#[derive(Clone, Debug)]
struct Destination {
    end: RegionEnd,
    seed: String,
    phase: Phase,
    /// The in-view hop made this neighbour the remote root.
    promoted: bool,
    /// The viewer was told once. A second completion stays quiet.
    announced: bool,
}

#[derive(Clone, Debug)]
struct StagedClose {
    session: Uuid,
    end: RegionEnd,
}

#[derive(Debug, Default)]
struct Generation {
    logged_out: bool,
    by_region: HashMap<Uuid, Destination>,
}

#[derive(Debug, Default)]
struct AgentBook {
    current: Uuid,
    session: Uuid,
    generations: HashMap<Uuid, Generation>,
    /// Open children a later login must close with the session that opened them.
    staged: Vec<StagedClose>,
}

#[derive(Debug, Default)]
struct Books {
    agents: HashMap<Uuid, AgentBook>,
}

/// Shared by admission, the source hop, logout and the frame's kill decision.
#[derive(Debug, Default)]
pub struct ChildBook {
    inner: Mutex<Books>,
}

impl ChildBook {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Books> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The root circuit generation that now owns this agent's children.
    /// Open children move only with the session that opened them. A neighbour
    /// the hop promoted stays promoted across that carry, until
    /// [`Self::release_promoted`] clears it when this generation is root
    /// again. A different session does not queue a close for a destination
    /// that is still the remote root. It closes the other open children with
    /// the old session id and starts empty. A pending admission stays with
    /// the generation that claimed it.
    pub fn attach(&self, agent: Uuid, generation: Uuid, session: Uuid) {
        let mut books = self.lock();
        let book = books.agents.entry(agent).or_default();
        let previous = book.current;
        let previous_session = book.session;
        let same_session = previous_session.is_nil() || previous_session == session;
        if previous != generation && !previous.is_nil() {
            let mut moved = Vec::new();
            let mut close = Vec::new();
            let empty = if let Some(older) = book.generations.get_mut(&previous) {
                older.by_region.retain(|region, destination| {
                    if destination.phase != Phase::Open {
                        return true;
                    }
                    if same_session {
                        let mut carried = destination.clone();
                        carried.phase = Phase::Open;
                        moved.push((*region, carried));
                    } else if !destination.promoted {
                        close.push(destination.end.clone());
                    }
                    false
                });
                if !same_session {
                    older.logged_out = true;
                }
                older.by_region.is_empty()
            } else {
                false
            };
            if empty {
                book.generations.remove(&previous);
            }
            if same_session {
                let generation_book = book.generations.entry(generation).or_default();
                for (region, destination) in moved {
                    generation_book
                        .by_region
                        .entry(region)
                        .or_insert(destination);
                }
            } else {
                for end in close {
                    book.staged.push(StagedClose {
                        session: previous_session,
                        end,
                    });
                }
            }
        }
        book.current = generation;
        book.session = session;
        book.generations.entry(generation).or_default();
    }

    /// Closes owed to a session a later login replaced. One take sends them.
    pub fn take_staged_closes(&self, agent: Uuid) -> Vec<(Uuid, RegionEnd)> {
        let mut books = self.lock();
        let Some(book) = books.agents.get_mut(&agent) else {
            return Vec::new();
        };
        book.staged
            .drain(..)
            .map(|close| (close.session, close.end))
            .collect()
    }

    /// Claim `end` for one admission. The seed is stored before the POST.
    pub fn claim(&self, agent: Uuid, generation: Uuid, end: RegionEnd) -> Claim {
        let mut books = self.lock();
        let Some(book) = books.agents.get_mut(&agent) else {
            return Claim::Unavailable;
        };
        if book.current != generation {
            return Claim::Unavailable;
        }
        let Some(generation_book) = book.generations.get_mut(&generation) else {
            return Claim::Unavailable;
        };
        if generation_book.logged_out {
            return Claim::Unavailable;
        }
        if let Some(destination) = generation_book.by_region.get(&end.region_id) {
            return Claim::Existing {
                seed: destination.seed.clone(),
            };
        }
        let count = generation_book.by_region.len();
        if count >= CHILD_DESTINATIONS {
            report_destination_bound(count);
            return Claim::Bound;
        }
        let seed = Uuid::new_v4().to_string();
        generation_book.by_region.insert(
            end.region_id,
            Destination {
                end,
                seed: seed.clone(),
                phase: Phase::Pending,
                promoted: false,
                announced: false,
            },
        );
        Claim::New { seed }
    }

    /// The neighbour refused the POST. Drop the claim. No viewer events.
    pub fn refuse(&self, agent: Uuid, generation: Uuid, region: Uuid) {
        let mut books = self.lock();
        let empty = {
            let Some(generation_book) = generation_mut(&mut books, agent, generation) else {
                return;
            };
            let drop_claim = generation_book
                .by_region
                .get(&region)
                .is_some_and(|destination| {
                    !destination.promoted
                        && matches!(destination.phase, Phase::Pending | Phase::CloseWhenAccepted)
                });
            if drop_claim {
                generation_book.by_region.remove(&region);
            }
            generation_book.by_region.is_empty()
        };
        // The circuit generation stays. Only a finished generation is removed.
        if empty {
            forget_finished(&mut books, agent, generation);
        }
    }

    /// The admission POST finished. The caller announces, closes, or does neither.
    pub fn accepted(&self, agent: Uuid, generation: Uuid, region: Uuid) -> Admission {
        let mut books = self.lock();
        let current = books
            .agents
            .get(&agent)
            .map(|book| book.current)
            .unwrap_or_default();
        let promoted_elsewhere = promoted_on_another(&books, agent, generation, region);
        let current_owns = current != generation && generation_owns(&books, agent, current, region);
        let Some((promoted, phase, logged_out, announced, seed, end)) = books
            .agents
            .get(&agent)
            .and_then(|book| book.generations.get(&generation))
            .and_then(|generation_book| {
                generation_book.by_region.get(&region).map(|destination| {
                    (
                        destination.promoted,
                        destination.phase,
                        generation_book.logged_out,
                        destination.announced,
                        destination.seed.clone(),
                        destination.end.clone(),
                    )
                })
            })
        else {
            // The same session carried the hop's neighbour. Announce that
            // copy once; the claiming generation no longer holds it.
            return announce_carried(&mut books, agent, region);
        };
        if promoted_elsewhere || (current_owns && !promoted) {
            if !promoted {
                remove_destination(&mut books, agent, generation, region);
            }
            return Admission::Quiet;
        }
        // The hop already owns this neighbour. Tell the viewer once. Logout
        // still does not close it until the generation is root again. Logout
        // that already won closes and does not announce (D106).
        if promoted {
            if logged_out || phase == Phase::CloseWhenAccepted {
                remove_destination(&mut books, agent, generation, region);
                return Admission::Close { end };
            }
            if !announced {
                if let Some(destination) = generation_mut(&mut books, agent, generation)
                    .and_then(|generation_book| generation_book.by_region.get_mut(&region))
                {
                    destination.announced = true;
                    destination.phase = Phase::Open;
                }
                return Admission::Announce { seed, end };
            }
            return Admission::Quiet;
        }
        let stale = generation != current || logged_out;
        match phase {
            Phase::Pending if stale => {
                remove_destination(&mut books, agent, generation, region);
                Admission::Close { end }
            }
            Phase::CloseWhenAccepted => {
                remove_destination(&mut books, agent, generation, region);
                Admission::Close { end }
            }
            Phase::Pending => {
                if let Some(destination) = generation_mut(&mut books, agent, generation)
                    .and_then(|generation_book| generation_book.by_region.get_mut(&region))
                {
                    destination.phase = Phase::Open;
                    destination.announced = true;
                }
                Admission::Announce { seed, end }
            }
            Phase::Open => Admission::Quiet,
        }
    }

    /// The POST failed after the seed was stored. While this generation is
    /// still logged in, drop the claim the way [`Self::refuse`] does, so the
    /// next pass can POST again. Logout that already won closes once,
    /// including a timeout or a transport failure (D106). The hop's remote
    /// root stays.
    pub fn failed(&self, agent: Uuid, generation: Uuid, region: Uuid) -> Admission {
        let mut books = self.lock();
        let current = books
            .agents
            .get(&agent)
            .map(|book| book.current)
            .unwrap_or_default();
        let Some((promoted, phase, logged_out, end)) = books
            .agents
            .get(&agent)
            .and_then(|book| book.generations.get(&generation))
            .and_then(|generation_book| {
                generation_book.by_region.get(&region).map(|destination| {
                    (
                        destination.promoted,
                        destination.phase,
                        generation_book.logged_out,
                        destination.end.clone(),
                    )
                })
            })
        else {
            return Admission::Quiet;
        };
        let stale = generation != current || logged_out || phase == Phase::CloseWhenAccepted;
        if promoted && !stale {
            return Admission::Quiet;
        }
        if stale {
            remove_destination(&mut books, agent, generation, region);
            return Admission::Close { end };
        }
        // Drop a claim that never opened. An open child, and the remote
        // root, stay. `promoted && !stale` already returned above.
        if !promoted && matches!(phase, Phase::Pending | Phase::CloseWhenAccepted) {
            remove_destination(&mut books, agent, generation, region);
        }
        Admission::Quiet
    }

    /// Logout of the current generation. Open children are returned for one
    /// close each. A pending admission closes later, if the POST succeeds.
    pub fn begin_logout(&self, agent: Uuid) -> Vec<RegionEnd> {
        let mut books = self.lock();
        let Some(book) = books.agents.get_mut(&agent) else {
            return Vec::new();
        };
        let generation = book.current;
        let Some(generation_book) = book.generations.get_mut(&generation) else {
            return Vec::new();
        };
        generation_book.logged_out = true;
        let mut close = Vec::new();
        let mut finished = Vec::new();
        for (region, destination) in &mut generation_book.by_region {
            if destination.promoted {
                continue;
            }
            match destination.phase {
                Phase::Open => {
                    close.push(destination.end.clone());
                    finished.push(*region);
                }
                Phase::Pending => destination.phase = Phase::CloseWhenAccepted,
                Phase::CloseWhenAccepted => {}
            }
        }
        for region in finished {
            generation_book.by_region.remove(&region);
        }
        close
    }

    /// The circuit generation is gone. Pending admissions still complete.
    /// Open records are dropped here; logout already closed the ones it owned.
    /// A promoted destination is not closed.
    pub fn end_generation(&self, agent: Uuid, generation: Uuid) {
        let mut books = self.lock();
        let empty = {
            let Some(generation_book) = generation_mut(&mut books, agent, generation) else {
                return;
            };
            generation_book.logged_out = true;
            generation_book.by_region.retain(|_, destination| {
                if destination.promoted {
                    return false;
                }
                match destination.phase {
                    Phase::Pending => {
                        destination.phase = Phase::CloseWhenAccepted;
                        true
                    }
                    Phase::CloseWhenAccepted => true,
                    Phase::Open => false,
                }
            });
            generation_book.by_region.is_empty()
        };
        if empty {
            drop_generation(&mut books, agent, generation);
        }
    }

    /// Caps object path for an in-view hop, when this generation already claimed it.
    pub fn seed(&self, agent: Uuid, region: Uuid) -> Option<String> {
        let books = self.lock();
        let book = books.agents.get(&agent)?;
        let destination = book
            .generations
            .get(&book.current)?
            .by_region
            .get(&region)?;
        Some(destination.seed.clone())
    }

    /// The hop reached this neighbour. Logout does not close it while it is
    /// the remote root. A later admission completion announces once.
    pub fn promote(&self, agent: Uuid, end: &RegionEnd, seed: &str) {
        let mut books = self.lock();
        let Some(book) = books.agents.get_mut(&agent) else {
            return;
        };
        let generation = book.current;
        let Some(generation_book) = book.generations.get_mut(&generation) else {
            return;
        };
        if let Some(destination) = generation_book.by_region.get_mut(&end.region_id) {
            destination.promoted = true;
            destination.phase = Phase::Open;
            destination.end = end.clone();
            if !seed.is_empty() {
                destination.seed = seed.to_owned();
            }
            return;
        }
        if generation_book.by_region.len() >= CHILD_DESTINATIONS {
            report_destination_bound(generation_book.by_region.len());
            return;
        }
        generation_book.by_region.insert(
            end.region_id,
            Destination {
                end: end.clone(),
                seed: seed.to_owned(),
                phase: Phase::Open,
                promoted: true,
                announced: false,
            },
        );
    }

    /// This generation is the root again. The neighbour the hop promoted is an
    /// ordinary open child, so logout closes it.
    pub fn release_promoted(&self, agent: Uuid) {
        let mut books = self.lock();
        let Some(book) = books.agents.get_mut(&agent) else {
            return;
        };
        let generation = book.current;
        let Some(generation_book) = book.generations.get_mut(&generation) else {
            return;
        };
        for destination in generation_book.by_region.values_mut() {
            destination.promoted = false;
        }
    }

    /// Roots whose current generation already knows `region`. The avatar that
    /// is leaving is not included. Pending claims count: the seed is stored first.
    pub fn known_by(&self, region: Uuid, except: Uuid) -> Vec<Uuid> {
        let books = self.lock();
        let mut known = Vec::new();
        for (agent, book) in &books.agents {
            if *agent == except {
                continue;
            }
            let knows = book
                .generations
                .get(&book.current)
                .and_then(|generation| generation.by_region.get(&region))
                .is_some_and(|destination| {
                    matches!(destination.phase, Phase::Pending | Phase::Open)
                });
            if knows {
                known.push(*agent);
            }
        }
        known
    }
}

/// The claiming generation no longer holds the destination because a
/// same-session carry moved it. Announce the current copy once when the hop
/// promoted it and the viewer has not been told.
fn announce_carried(books: &mut Books, agent: Uuid, region: Uuid) -> Admission {
    let Some(book) = books.agents.get(&agent) else {
        return Admission::Quiet;
    };
    let current = book.current;
    let Some(destination) = book
        .generations
        .get(&current)
        .and_then(|generation_book| generation_book.by_region.get(&region))
    else {
        return Admission::Quiet;
    };
    if !destination.promoted || destination.announced {
        return Admission::Quiet;
    }
    let seed = destination.seed.clone();
    let end = destination.end.clone();
    if let Some(destination) = generation_mut(books, agent, current)
        .and_then(|generation_book| generation_book.by_region.get_mut(&region))
    {
        destination.announced = true;
        destination.phase = Phase::Open;
    }
    Admission::Announce { seed, end }
}

fn report_destination_bound(count: usize) {
    ferroan_diag::refuse(
        ferroan_diag::REGION_CHILD_DESTINATIONS,
        ferroan_diag::Kind::Hard,
        CHILD_DESTINATIONS as u64,
        count.saturating_add(1) as u64,
        "child-admission",
    );
}

fn generation_mut(books: &mut Books, agent: Uuid, generation: Uuid) -> Option<&mut Generation> {
    books
        .agents
        .get_mut(&agent)?
        .generations
        .get_mut(&generation)
}

fn remove_destination(books: &mut Books, agent: Uuid, generation: Uuid, region: Uuid) {
    let empty = {
        let Some(generation_book) = generation_mut(books, agent, generation) else {
            return;
        };
        generation_book.by_region.remove(&region);
        generation_book.by_region.is_empty()
    };
    if empty {
        forget_finished(books, agent, generation);
    }
}

/// Drop an empty generation that is no longer the root circuit.
/// The current generation stays so a later claim can use it.
fn forget_finished(books: &mut Books, agent: Uuid, generation: Uuid) {
    let current = books
        .agents
        .get(&agent)
        .is_some_and(|book| book.current == generation);
    if !current {
        drop_generation(books, agent, generation);
    }
}

fn drop_generation(books: &mut Books, agent: Uuid, generation: Uuid) {
    let Some(book) = books.agents.get_mut(&agent) else {
        return;
    };
    book.generations.remove(&generation);
    if book.generations.is_empty() {
        books.agents.remove(&agent);
    }
}

fn generation_owns(books: &Books, agent: Uuid, generation: Uuid, region: Uuid) -> bool {
    books
        .agents
        .get(&agent)
        .and_then(|book| book.generations.get(&generation))
        .is_some_and(|generation_book| generation_book.by_region.contains_key(&region))
}

fn promoted_on_another(books: &Books, agent: Uuid, generation: Uuid, region: Uuid) -> bool {
    let Some(book) = books.agents.get(&agent) else {
        return false;
    };
    book.generations.iter().any(|(id, generation_book)| {
        *id != generation
            && generation_book
                .by_region
                .get(&region)
                .is_some_and(|destination| destination.promoted)
    })
}

/// Fields the viewer reads from the neighbour announcements.
#[derive(Clone, Debug)]
pub struct NeighbourView {
    pub handle: u64,
    pub address: Ipv4Addr,
    pub port: u16,
    pub size_x: u32,
    pub size_y: u32,
    pub agent: Uuid,
    pub seed: String,
}

fn u32_binary(value: u32) -> Llsd {
    Llsd::Binary(value.to_be_bytes().to_vec())
}

/// `EnableSimulator` then `EstablishAgentCommunication`.
/// `InformClientOfNeighbourAsync` sends them in that order after acceptance.
pub fn neighbour_events(view: &NeighbourView) -> [Llsd; 2] {
    let mut simulator = Llsd::new_map();
    simulator.insert("Handle", Llsd::Binary(view.handle.to_be_bytes().to_vec()));
    simulator.insert("IP", Llsd::Binary(view.address.octets().to_vec()));
    simulator.insert("Port", Llsd::Integer(i32::from(view.port)));
    simulator.insert("RegionSizeX", u32_binary(view.size_x));
    simulator.insert("RegionSizeY", u32_binary(view.size_y));
    let mut enable_body = Llsd::new_map();
    enable_body.insert("SimulatorInfo", Llsd::Array(vec![simulator]));
    let mut enable = Llsd::new_map();
    enable.insert("message", Llsd::String("EnableSimulator".to_owned()));
    enable.insert("body", enable_body);

    let mut establish_body = Llsd::new_map();
    establish_body.insert("agent-id", Llsd::Uuid(view.agent));
    establish_body.insert(
        "sim-ip-and-port",
        Llsd::String(format!("{}:{}", view.address, view.port)),
    );
    establish_body.insert("seed-capability", Llsd::String(view.seed.clone()));
    let mut establish = Llsd::new_map();
    establish.insert(
        "message",
        Llsd::String("EstablishAgentCommunication".to_owned()),
    );
    establish.insert("body", establish_body);
    [enable, establish]
}

/// Viewer events for an accepted admission, or none when the book says quiet or close.
pub fn admission_events(admission: &Admission, view: &NeighbourView) -> Vec<Llsd> {
    match admission {
        Admission::Announce { seed, end } => {
            let mut announced = view.clone();
            announced.seed = ferroan_client::caps_seed(&end.server_uri, seed);
            neighbour_events(&announced).to_vec()
        }
        Admission::Close { .. } | Admission::Quiet => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(region: u128) -> RegionEnd {
        RegionEnd {
            server_uri: format!("http://192.0.2.{region}/"),
            region_id: Uuid::from_u128(region),
            name: "Neighbour".to_string(),
            loc_x: 256,
            loc_y: 256,
        }
    }

    fn session() -> Uuid {
        Uuid::from_u128(2)
    }

    fn book_for(agent: Uuid, generation: Uuid) -> ChildBook {
        let book = ChildBook::new();
        book.attach(agent, generation, session());
        book
    }

    #[test]
    fn two_destinations_each_have_one_owner_and_one_post() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let first = book.claim(agent, generation, end(1));
        let second = book.claim(agent, generation, end(2));
        let again = book.claim(agent, generation, end(1));
        let Claim::New { seed: first_seed } = &first else {
            panic!("first destination was not claimed");
        };
        let Claim::New { seed: second_seed } = &second else {
            panic!("second destination was not claimed");
        };
        assert_ne!(first_seed, second_seed);
        assert_eq!(
            again,
            Claim::Existing {
                seed: first_seed.clone()
            }
        );
        let posts = [&first, &second, &again]
            .into_iter()
            .filter(|claim| matches!(claim, Claim::New { .. }))
            .count();
        assert_eq!(posts, 2);
        let first_seed = first_seed.clone();
        let admitted = book.accepted(agent, generation, Uuid::from_u128(1));
        let Admission::Announce { seed, .. } = &admitted else {
            panic!("acceptance did not announce");
        };
        assert_eq!(seed, &first_seed);
        let events = admission_events(
            &admitted,
            &NeighbourView {
                handle: 1,
                address: Ipv4Addr::new(192, 0, 2, 1),
                port: 9001,
                size_x: 256,
                size_y: 256,
                agent,
                seed: String::new(),
            },
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].get("message").as_str(), "EnableSimulator");
        assert_eq!(
            events[1].get("message").as_str(),
            "EstablishAgentCommunication"
        );
        assert_eq!(
            events[1].get("body").get("seed-capability").as_str(),
            ferroan_client::caps_seed(&end(1).server_uri, &first_seed)
        );
    }

    #[test]
    fn refusal_drops_the_claim_and_emits_no_events() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(3)) else {
            panic!("claim");
        };
        book.refuse(agent, generation, Uuid::from_u128(3));
        let admission = book.accepted(agent, generation, Uuid::from_u128(3));
        assert_eq!(admission, Admission::Quiet);
        assert!(admission_events(&admission, &dummy_view(agent)).is_empty());
        let Claim::New { .. } = book.claim(agent, generation, end(3)) else {
            panic!("the refused destination should be free");
        };
    }

    #[test]
    fn logout_during_admission_closes_once_and_does_not_announce() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("claim");
        };
        assert!(book.begin_logout(agent).is_empty());
        let admission = book.accepted(agent, generation, Uuid::from_u128(5));
        assert!(matches!(admission, Admission::Close { .. }));
        assert!(admission_events(&admission, &dummy_view(agent)).is_empty());
        assert_eq!(
            book.accepted(agent, generation, Uuid::from_u128(5)),
            Admission::Quiet
        );
    }

    #[test]
    fn a_refused_post_after_logout_does_not_close() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("claim");
        };
        book.begin_logout(agent);
        book.refuse(agent, generation, Uuid::from_u128(5));
        assert_eq!(
            book.accepted(agent, generation, Uuid::from_u128(5)),
            Admission::Quiet
        );
    }

    #[test]
    fn a_stale_completion_does_not_announce_or_close_the_promoted_root() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let replacement = Uuid::from_u128(9);
        let book = book_for(agent, older);
        let region = end(6);
        let Claim::New { seed } = book.claim(agent, older, region.clone()) else {
            panic!("claim");
        };
        book.attach(agent, replacement, session());
        book.promote(agent, &region, &seed);
        let admission = book.accepted(agent, older, region.region_id);
        assert_eq!(admission, Admission::Quiet);
        assert!(admission_events(&admission, &dummy_view(agent)).is_empty());
        assert!(book.begin_logout(agent).is_empty());
    }

    #[test]
    fn a_stale_completion_closes_an_orphan_the_replacement_does_not_own() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let replacement = Uuid::from_u128(9);
        let book = book_for(agent, older);
        let Claim::New { .. } = book.claim(agent, older, end(6)) else {
            panic!("claim");
        };
        book.attach(agent, replacement, session());
        let admission = book.accepted(agent, older, Uuid::from_u128(6));
        assert!(matches!(admission, Admission::Close { .. }));
        assert!(admission_events(&admission, &dummy_view(agent)).is_empty());
        assert_eq!(
            book.accepted(agent, older, Uuid::from_u128(6)),
            Admission::Quiet
        );
    }

    #[test]
    fn the_destination_bound_is_reported() {
        let before = refuse_hits();
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        for id in 1..=CHILD_DESTINATIONS {
            let Claim::New { .. } = book.claim(agent, generation, end(id as u128)) else {
                panic!("destination {id} was not claimed");
            };
        }
        assert_eq!(
            book.claim(agent, generation, end((CHILD_DESTINATIONS + 1) as u128)),
            Claim::Bound
        );
        assert_eq!(refuse_hits(), before + 1);
        book.promote(agent, &end(1), "seed");
        assert_eq!(refuse_hits(), before + 1);
        book.promote(agent, &end((CHILD_DESTINATIONS + 2) as u128), "seed");
        assert_eq!(refuse_hits(), before + 2);
        assert!(
            book.seed(agent, Uuid::from_u128((CHILD_DESTINATIONS + 2) as u128))
                .is_none()
        );
    }

    #[test]
    fn an_in_view_seed_is_the_one_the_admission_stored() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let region = end(7);
        let Claim::New { seed } = book.claim(agent, generation, region.clone()) else {
            panic!("claim");
        };
        assert_eq!(
            book.seed(agent, region.region_id).as_deref(),
            Some(seed.as_str())
        );
        book.promote(agent, &region, &seed);
        assert_eq!(
            book.seed(agent, region.region_id).as_deref(),
            Some(seed.as_str())
        );
        let admission = book.accepted(agent, generation, region.region_id);
        let Admission::Announce {
            seed: announced, ..
        } = &admission
        else {
            panic!("a completion after the hop should announce once");
        };
        assert_eq!(announced, &seed);
        assert_eq!(
            book.accepted(agent, generation, region.region_id),
            Admission::Quiet
        );
    }

    #[test]
    fn a_completion_after_promote_announces_once_and_logout_spares_the_remote_root() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let region = end(6);
        let Claim::New { seed } = book.claim(agent, generation, region.clone()) else {
            panic!("claim");
        };
        book.promote(agent, &region, &seed);
        let admission = book.accepted(agent, generation, region.region_id);
        assert!(matches!(admission, Admission::Announce { .. }));
        assert!(!admission_events(&admission, &dummy_view(agent)).is_empty());
        assert_eq!(
            book.accepted(agent, generation, region.region_id),
            Admission::Quiet
        );
        assert!(book.begin_logout(agent).is_empty());
    }

    #[test]
    fn a_same_session_carry_keeps_the_promoted_neighbour_until_release() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let carried = Uuid::from_u128(9);
        let book = book_for(agent, older);
        let region = end(6);
        let Claim::New { seed } = book.claim(agent, older, region.clone()) else {
            panic!("claim");
        };
        book.promote(agent, &region, &seed);
        book.attach(agent, carried, session());
        assert!(book.begin_logout(agent).is_empty());
        book.release_promoted(agent);
        let close = book.begin_logout(agent);
        assert_eq!(
            close.iter().map(|end| end.region_id).collect::<Vec<_>>(),
            vec![region.region_id]
        );
    }

    #[test]
    fn a_later_session_does_not_queue_a_close_of_the_carried_root() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let carried = Uuid::from_u128(9);
        let later = Uuid::from_u128(10);
        let book = book_for(agent, older);
        let region = end(6);
        let Claim::New { seed } = book.claim(agent, older, region.clone()) else {
            panic!("claim");
        };
        book.promote(agent, &region, &seed);
        book.attach(agent, carried, session());
        book.attach(agent, later, Uuid::from_u128(70));
        assert!(book.take_staged_closes(agent).is_empty());
    }

    #[test]
    fn a_carried_promoted_claim_announces_once() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let carried = Uuid::from_u128(9);
        let book = book_for(agent, older);
        let region = end(6);
        let Claim::New { seed } = book.claim(agent, older, region.clone()) else {
            panic!("claim");
        };
        book.promote(agent, &region, &seed);
        book.attach(agent, carried, session());
        let admission = book.accepted(agent, older, region.region_id);
        let Admission::Announce {
            seed: announced, ..
        } = &admission
        else {
            panic!("a carried promotion should announce");
        };
        assert_eq!(announced, &seed);
        assert!(!admission_events(&admission, &dummy_view(agent)).is_empty());
        assert_eq!(
            book.accepted(agent, older, region.region_id),
            Admission::Quiet
        );
    }

    #[test]
    fn releasing_the_promoted_neighbour_lets_logout_close_it() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let region = end(6);
        book.promote(agent, &region, "seed");
        assert!(book.begin_logout(agent).is_empty());
        let book = book_for(agent, generation);
        book.promote(agent, &region, "seed");
        book.release_promoted(agent);
        let close = book.begin_logout(agent);
        assert_eq!(
            close.iter().map(|end| end.region_id).collect::<Vec<_>>(),
            vec![region.region_id]
        );
    }

    #[test]
    fn claim_after_logout_stores_nothing() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        book.begin_logout(agent);
        assert_eq!(book.claim(agent, generation, end(5)), Claim::Unavailable);
        assert!(book.seed(agent, Uuid::from_u128(5)).is_none());
    }

    #[test]
    fn a_failed_post_after_logout_closes_once() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("claim");
        };
        book.begin_logout(agent);
        let admission = book.failed(agent, generation, Uuid::from_u128(5));
        assert!(matches!(admission, Admission::Close { .. }));
        assert!(admission_events(&admission, &dummy_view(agent)).is_empty());
        assert_eq!(
            book.failed(agent, generation, Uuid::from_u128(5)),
            Admission::Quiet
        );
    }

    #[test]
    fn fail_child_then_claim_child_returns_new() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("claim");
        };
        let admission = book.failed(agent, generation, Uuid::from_u128(5));
        assert_eq!(admission, Admission::Quiet);
        assert!(admission_events(&admission, &dummy_view(agent)).is_empty());
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("fail_child then claim_child returns New");
        };
        assert!(book.begin_logout(agent).is_empty());
    }

    #[test]
    fn a_different_session_closes_open_children_with_the_old_session() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let replacement = Uuid::from_u128(9);
        let book = book_for(agent, older);
        let region = end(1);
        let Claim::New { .. } = book.claim(agent, older, region.clone()) else {
            panic!("claim");
        };
        assert!(matches!(
            book.accepted(agent, older, region.region_id),
            Admission::Announce { .. }
        ));
        book.attach(agent, replacement, Uuid::from_u128(70));
        assert!(book.seed(agent, region.region_id).is_none());
        assert!(book.begin_logout(agent).is_empty());
        let staged = book.take_staged_closes(agent);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, session());
        assert_eq!(staged[0].1.region_id, region.region_id);
        assert!(book.take_staged_closes(agent).is_empty());
    }

    #[test]
    fn a_different_session_does_not_close_the_remote_root() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let replacement = Uuid::from_u128(9);
        let book = book_for(agent, older);
        let region = end(6);
        book.promote(agent, &region, "seed");
        book.attach(agent, replacement, Uuid::from_u128(70));
        assert!(book.take_staged_closes(agent).is_empty());
        assert!(book.begin_logout(agent).is_empty());
        assert!(book.seed(agent, region.region_id).is_none());
    }

    #[test]
    fn a_replacement_keeps_an_open_child_and_leaves_a_pending_admission() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let replacement = Uuid::from_u128(9);
        let book = book_for(agent, older);
        let Claim::New { .. } = book.claim(agent, older, end(1)) else {
            panic!("open claim");
        };
        assert!(matches!(
            book.accepted(agent, older, Uuid::from_u128(1)),
            Admission::Announce { .. }
        ));
        let Claim::New { .. } = book.claim(agent, older, end(2)) else {
            panic!("pending claim");
        };
        book.attach(agent, replacement, session());
        assert!(book.seed(agent, Uuid::from_u128(1)).is_some());
        assert!(book.seed(agent, Uuid::from_u128(2)).is_none());
        let close = book.begin_logout(agent);
        assert_eq!(
            close.iter().map(|end| end.region_id).collect::<Vec<_>>(),
            vec![Uuid::from_u128(1)]
        );
        assert!(matches!(
            book.accepted(agent, older, Uuid::from_u128(2)),
            Admission::Close { .. }
        ));
    }

    #[test]
    fn neighbour_events_match_the_child_view_capture() {
        let captured =
            ferroan_llsd::from_xml(include_bytes!("../../../fixtures/child-view/events.xml"))
                .unwrap();
        let events = captured.get("events").as_array();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].get("message").as_str(), "EnableSimulator");
        assert_eq!(
            events[1].get("message").as_str(),
            "EstablishAgentCommunication"
        );
        let info = events[0].get("body").get("SimulatorInfo").at(0);
        let body = events[1].get("body");
        let octets: [u8; 4] = info.get("IP").as_binary().try_into().unwrap();
        let built = neighbour_events(&NeighbourView {
            handle: info.get("Handle").as_u64(),
            address: Ipv4Addr::from(octets),
            port: u16::try_from(info.get("Port").as_i32()).unwrap(),
            size_x: info.get("RegionSizeX").as_u32(),
            size_y: info.get("RegionSizeY").as_u32(),
            agent: body.get("agent-id").as_uuid(),
            seed: body.get("seed-capability").as_str().to_owned(),
        });
        assert_eq!(built[0], events[0]);
        assert_eq!(built[1], events[1]);
        let keys = |value: &Llsd| value.as_map().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&built[0]), keys(&events[0]));
        assert_eq!(keys(&built[1]), keys(&events[1]));
        assert_eq!(
            keys(built[0].get("body").get("SimulatorInfo").at(0)),
            keys(info)
        );
        assert_eq!(keys(built[1].get("body")), keys(body));

        let trace: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/child-view/trace.json")).unwrap();
        let caps_path = trace["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|request| request["method"] == "POST")
            .unwrap()["fields"]["caps_path"]
            .as_str()
            .unwrap();
        let host = body.get("sim-ip-and-port").as_str();
        assert_eq!(
            body.get("seed-capability").as_str(),
            ferroan_client::caps_seed(&format!("http://{host}"), caps_path)
        );
    }

    fn dummy_view(agent: Uuid) -> NeighbourView {
        NeighbourView {
            handle: 0,
            address: Ipv4Addr::LOCALHOST,
            port: 1,
            size_x: 256,
            size_y: 256,
            agent,
            seed: String::new(),
        }
    }

    fn refuse_hits() -> u64 {
        let snapshot = ferroan_diag::metrics();
        let needle = "limit=\"region.child-destinations\",action=\"refuse\"} ";
        snapshot
            .lines()
            .find_map(|line| line.split_once(needle).map(|(_, count)| count))
            .and_then(|count| count.parse().ok())
            .unwrap_or(0)
    }
}
