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
//! (D106). A timeout or a transport failure after logout closes once too.
//! Every new claim is finished by acceptance, failure or refusal. The guard
//! is returned only from [`ChildBook::claim_work`]. Dropping it, including
//! when the admission task is cancelled, follows [`ChildBook::failed`]: before
//! logout the claim is dropped, and after logout or on a stale generation one
//! close is staged with the claim's session. Take those closes only after the
//! admission task has stopped.
//!
//! The admission order is the POST, then [`ANNOUNCE_DELAY`], then
//! [`ChildBook::report`], then the viewer events when the report is
//! [`Admission::Announce`]. Pin8 sleeps and then checks the presence again
//! (`EntityTransferModule.cs` 2780–2786 at `c401bceecd`), so logout during the
//! wait still closes. A completion after the hop promoted the destination
//! still announces once, including when the same session has carried that
//! neighbour forward. The carry keeps the promoted flag until this generation
//! is root again, so a later login does not close a destination that is still
//! the remote root. A different session closes the old root's other open
//! children with the old session id. An in-view hop reads the same seed.
//!
//! Dropping the circuit lease forgets open children and stages nothing. Pin8
//! closes children from a root (`Scene.cs` 3969, `closeChildAgents &&
//! !isChildAgent`) and at hop time (`GetChildAgentsToClose`). A far hop stages
//! a close only for an open child that call would close: outside the
//! destination's view at draw distance 255, or every known child when the
//! destination's grid Y is 0, and never the destination itself
//! (`ScenePresence.cs` 5002–5058). The view uses that child's stored
//! footprint, or 256 m when the size was unknown
//! (`m_knownChildRegionsSizeInfo`, `ScenePresence.cs` 5036–5044). A child
//! that stays in view is forgotten. Ferroan sends no `children_seeds`.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    /// One `CloseAgent`, authenticated with `session`. Do not announce.
    Close { end: RegionEnd, session: Uuid },
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
    /// This child's footprint, in metres. Unknown sizes are stored as
    /// [`DEFAULT_REGION_SIZE`].
    size: [u32; 2],
}

#[derive(Clone, Debug)]
struct StagedClose {
    session: Uuid,
    end: RegionEnd,
}

#[derive(Debug, Default)]
struct Generation {
    logged_out: bool,
    /// The session that attached this generation. Closes use this id.
    session: Uuid,
    by_region: HashMap<Uuid, Destination>,
}

/// How the neighbour POST finished. A timeout and a transport failure both
/// settle through [`ChildBook::failed`], so logout still closes (D106). A
/// refusal does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostReport {
    Accepted,
    Refused,
    TimedOut,
    Transport,
}

impl PostReport {
    /// Classify a finished `CreateAgent`. An HTTP answer with `success` false
    /// is a refusal. A timeout stays a timeout even when the retry also fails.
    pub fn from_create(outcome: &crate::simulation_client::CreateOutcome) -> Self {
        use crate::simulation_client::CreateStop;
        match outcome.stop {
            CreateStop::TimedOut => Self::TimedOut,
            CreateStop::Transport => Self::Transport,
            CreateStop::Answered if outcome.success => Self::Accepted,
            CreateStop::Answered => Self::Refused,
        }
    }
}

/// `InformClientOfNeighbourAsync` sleeps this long after `CreateAgent` returns
/// and before it checks the presence and sends `EnableSimulator`
/// (`EntityTransferModule.cs` 2780–2786 at Pin8 `c401bceecd`).
pub const ANNOUNCE_DELAY: Duration = Duration::from_millis(500);

/// `Constants.RegionSize`. A child whose size was not stored uses this
/// (`ScenePresence.cs` 5049). A stored size is that child's own footprint.
const DEFAULT_REGION_SIZE: u32 = 256;

/// Draw distance `GetChildAgentsToClose` passes to `IsOutsideView`. The
/// comparison then steps one metre in (`Util.cs` 1640).
const CHILD_VIEW_METRES: u64 = 255;

/// Where a far hop is going. Sizes are that region's footprint, in metres.
#[derive(Clone, Debug)]
pub struct FarDestination {
    pub end: RegionEnd,
    pub size_x: u32,
    pub size_y: u32,
}

/// Wait after the admission POST and before [`ChildBook::report`].
/// Logout during the wait still sees a pending claim, so the report closes
/// instead of announcing. Send viewer events only for [`Admission::Announce`].
pub async fn pause_before_announce() {
    tokio::time::sleep(ANNOUNCE_DELAY).await;
}

/// Holds the one [`Claim::New`] returned by [`ChildBook::claim_work`].
/// Dropping the guard, including when the admission task is cancelled, follows
/// [`ChildBook::failed`]: the claim is dropped before logout. After logout, or
/// on a stale generation, a close is staged with this claim's session, unless
/// the current generation already owns that region and this claim is not the
/// remote root. That owner is left alone.
pub struct ClaimWork {
    book: Arc<ChildBook>,
    agent: Uuid,
    generation: Uuid,
    region: Uuid,
    seed: String,
    open: bool,
}

impl ClaimWork {
    /// The caps path stored for this claim.
    pub fn seed(&self) -> &str {
        &self.seed
    }

    fn watch(
        book: Arc<ChildBook>,
        agent: Uuid,
        generation: Uuid,
        region: Uuid,
        seed: String,
    ) -> Self {
        Self {
            book,
            agent,
            generation,
            region,
            seed,
            open: true,
        }
    }

    /// The claim was accepted, refused or failed. Drop will not settle it again.
    pub fn settle(mut self) {
        self.open = false;
    }
}

impl Drop for ClaimWork {
    fn drop(&mut self) {
        if self.open {
            self.book.abandon(self.agent, self.generation, self.region);
        }
    }
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
        book.generations.entry(generation).or_default().session = session;
    }

    /// Closes owed to a session a later login replaced, including one staged
    /// when an admission guard dropped after logout. One take sends them.
    /// Call this only after that admission task has stopped: the guard stages
    /// the close as it drops, and an earlier take misses it.
    pub fn take_staged_closes(&self, agent: Uuid) -> Vec<(Uuid, RegionEnd)> {
        let mut books = self.lock();
        let (staged, finished) = {
            let Some(book) = books.agents.get_mut(&agent) else {
                return Vec::new();
            };
            let staged = book
                .staged
                .drain(..)
                .map(|close| (close.session, close.end))
                .collect();
            let finished = book.generations.is_empty() && book.staged.is_empty();
            (staged, finished)
        };
        if finished {
            books.agents.remove(&agent);
        }
        staged
    }

    /// Every staged close, for the moment the admission task has stopped.
    /// An earlier take misses a close the guard stages as it drops.
    pub(crate) fn take_every_staged_close(&self) -> Vec<(Uuid, Uuid, RegionEnd)> {
        let mut books = self.lock();
        let agents: Vec<Uuid> = books.agents.keys().copied().collect();
        let mut found = Vec::new();
        for agent in agents {
            let finished = {
                let Some(book) = books.agents.get_mut(&agent) else {
                    continue;
                };
                for close in book.staged.drain(..) {
                    found.push((agent, close.session, close.end));
                }
                book.generations.is_empty()
            };
            if finished {
                books.agents.remove(&agent);
            }
        }
        found
    }

    /// A close the drive has already decided. The destination is no longer
    /// tracked. The close stays until its DELETE returns, so aborting that
    /// send still leaves it for serve end and shutdown.
    pub fn stage_decided_close(&self, agent: Uuid, session: Uuid, end: RegionEnd) {
        let mut books = self.lock();
        books
            .agents
            .entry(agent)
            .or_default()
            .staged
            .push(StagedClose { session, end });
    }

    /// The DELETE for one decided close returned. One that was not sent stays.
    pub fn ack_decided_close(&self, agent: Uuid, session: Uuid, end: &RegionEnd) {
        let mut books = self.lock();
        let Some(book) = books.agents.get_mut(&agent) else {
            return;
        };
        if let Some(index) = book
            .staged
            .iter()
            .position(|close| close.session == session && close.end.region_id == end.region_id)
        {
            book.staged.remove(index);
        }
    }

    /// Claim `end` for one admission. The seed is stored before the POST.
    /// The child's size is [`DEFAULT_REGION_SIZE`] until a caller knows it.
    pub fn claim(&self, agent: Uuid, generation: Uuid, end: RegionEnd) -> Claim {
        self.claim_sized(
            agent,
            generation,
            end,
            [DEFAULT_REGION_SIZE, DEFAULT_REGION_SIZE],
        )
    }

    /// [`Self::claim`] that remembers this child's footprint. A zero extent is
    /// unknown and falls back to [`DEFAULT_REGION_SIZE`].
    pub fn claim_sized(
        &self,
        agent: Uuid,
        generation: Uuid,
        end: RegionEnd,
        size: [u32; 2],
    ) -> Claim {
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
                size: known_region_size(size),
            },
        );
        Claim::New { seed }
    }

    /// Claim `end` and return the only guard for that claim. Any other
    /// [`Claim`] stores nothing and carries no guard, so a guard cannot drop
    /// a destination another task owns. The child's size is
    /// [`DEFAULT_REGION_SIZE`].
    pub fn claim_work(
        self: &Arc<Self>,
        agent: Uuid,
        generation: Uuid,
        end: RegionEnd,
    ) -> Result<ClaimWork, Claim> {
        self.claim_work_sized(
            agent,
            generation,
            end,
            [DEFAULT_REGION_SIZE, DEFAULT_REGION_SIZE],
        )
    }

    /// [`Self::claim_work`] that remembers this child's footprint.
    pub fn claim_work_sized(
        self: &Arc<Self>,
        agent: Uuid,
        generation: Uuid,
        end: RegionEnd,
        size: [u32; 2],
    ) -> Result<ClaimWork, Claim> {
        let region = end.region_id;
        match self.claim_sized(agent, generation, end, size) {
            Claim::New { seed } => Ok(ClaimWork::watch(
                Arc::clone(self),
                agent,
                generation,
                region,
                seed,
            )),
            other => Err(other),
        }
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
                let close = close_for(&books, agent, generation, end);
                remove_destination(&mut books, agent, generation, region);
                return close;
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
                let close = close_for(&books, agent, generation, end);
                remove_destination(&mut books, agent, generation, region);
                close
            }
            Phase::CloseWhenAccepted => {
                let close = close_for(&books, agent, generation, end);
                remove_destination(&mut books, agent, generation, region);
                close
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
    /// root stays. A replaced generation whose region the current generation
    /// already owns is removed and closes nothing.
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
        // The same-session replacement already owns this region. CloseAgent
        // would accept this claim's session and remove the child that
        // generation is using (`Scene.cs` 5100–5104). Drop the stale entry.
        let current_owns = current != generation && generation_owns(&books, agent, current, region);
        if current_owns && !promoted {
            remove_destination(&mut books, agent, generation, region);
            return Admission::Quiet;
        }
        let stale = generation != current || logged_out || phase == Phase::CloseWhenAccepted;
        if promoted && !stale {
            return Admission::Quiet;
        }
        if stale {
            let close = close_for(&books, agent, generation, end);
            remove_destination(&mut books, agent, generation, region);
            return close;
        }
        // Drop a claim that never opened. An open child, and the remote
        // root, stay. `promoted && !stale` already returned above.
        if !promoted && matches!(phase, Phase::Pending | Phase::CloseWhenAccepted) {
            remove_destination(&mut books, agent, generation, region);
        }
        Admission::Quiet
    }

    /// [`Self::failed`] for a guard that nobody is waiting on. A close is
    /// staged, with the claim's session, instead of being returned. Callers
    /// that still send the close use [`Self::failed`] and then [`ClaimWork::settle`].
    fn abandon(&self, agent: Uuid, generation: Uuid, region: Uuid) {
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
            return;
        };
        // Same rule as `failed`. A guard for a generation the replacement
        // already superseded must not stage a close of that region.
        let current_owns = current != generation && generation_owns(&books, agent, current, region);
        if current_owns && !promoted {
            remove_destination(&mut books, agent, generation, region);
            return;
        }
        let stale = generation != current || logged_out || phase == Phase::CloseWhenAccepted;
        if promoted && !stale {
            return;
        }
        if stale {
            let session = generation_session(&books, agent, generation);
            if let Some(book) = books.agents.get_mut(&agent) {
                book.staged.push(StagedClose { session, end });
            }
            remove_destination(&mut books, agent, generation, region);
            return;
        }
        if !promoted && matches!(phase, Phase::Pending | Phase::CloseWhenAccepted) {
            remove_destination(&mut books, agent, generation, region);
        }
    }

    /// Finish one admission POST. Acceptance follows [`Self::accepted`]. A
    /// refusal drops the claim. A timeout and a transport failure follow
    /// [`Self::failed`], so logout still closes once (D106).
    pub fn report(
        &self,
        agent: Uuid,
        generation: Uuid,
        region: Uuid,
        report: PostReport,
    ) -> Admission {
        match report {
            PostReport::Accepted => self.accepted(agent, generation, region),
            PostReport::Refused => {
                self.refuse(agent, generation, region);
                Admission::Quiet
            }
            PostReport::TimedOut | PostReport::Transport => self.failed(agent, generation, region),
        }
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

    /// The circuit lease dropped. Open children are forgotten and nothing is
    /// staged: Pin8 closes children from a root and at hop time, not when a
    /// child circuit ends. A pending admission still completes, and closes if
    /// logout already won. A promoted destination is the remote root and is
    /// not closed.
    pub fn end_generation(&self, agent: Uuid, generation: Uuid) {
        self.finish_generation(agent, generation, Finish::Forget);
    }

    /// A far hop. Stage a close for each open child [`child_should_close`]
    /// selects, with this generation's session. The same generation's lease
    /// drop must not stage those closes; this is the only path that does.
    /// A seeded hop has no session row, so it closes the current generation.
    /// Production uses [`Self::end_generation_far`].
    #[cfg(test)]
    pub fn close_far(&self, agent: Uuid, destination: &FarDestination) {
        let generation = {
            let books = self.lock();
            books
                .agents
                .get(&agent)
                .map(|book| book.current)
                .unwrap_or_default()
        };
        if !generation.is_nil() {
            self.end_generation_far(agent, generation, destination);
        }
    }

    pub(crate) fn end_generation_far(
        &self,
        agent: Uuid,
        generation: Uuid,
        destination: &FarDestination,
    ) {
        self.finish_generation(agent, generation, Finish::Far(destination));
    }

    fn finish_generation(&self, agent: Uuid, generation: Uuid, mode: Finish<'_>) {
        let mut books = self.lock();
        let session = {
            let Some(book) = books.agents.get(&agent) else {
                return;
            };
            let Some(generation_book) = book.generations.get(&generation) else {
                return;
            };
            let stored = generation_book.session;
            if stored.is_nil() {
                book.session
            } else {
                stored
            }
        };
        let mut stage = Vec::new();
        let empty = {
            let Some(generation_book) = generation_mut(&mut books, agent, generation) else {
                return;
            };
            generation_book.logged_out = true;
            generation_book.by_region.retain(|_, destination| {
                if destination.promoted {
                    return false;
                }
                let closing = match mode {
                    Finish::Forget => false,
                    Finish::Far(hop) => child_should_close(destination, hop),
                };
                match destination.phase {
                    Phase::Pending => {
                        // An in-view child is forgotten. A lease drop, and a
                        // child the far hop would close, still close when the
                        // POST finishes.
                        if matches!(mode, Finish::Forget) || closing {
                            destination.phase = Phase::CloseWhenAccepted;
                            true
                        } else {
                            false
                        }
                    }
                    Phase::CloseWhenAccepted => true,
                    Phase::Open => {
                        if closing {
                            stage.push(destination.end.clone());
                        }
                        false
                    }
                }
            });
            generation_book.by_region.is_empty()
        };
        if let Some(book) = books.agents.get_mut(&agent) {
            for end in stage {
                book.staged.push(StagedClose { session, end });
            }
        }
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
    /// The stored size is left as it is.
    pub fn promote(&self, agent: Uuid, end: &RegionEnd, seed: &str) {
        self.promote_with(agent, end, seed, None);
    }

    /// [`Self::promote`] that stores this neighbour's footprint. A zero extent
    /// stays the 256 m fallback. An existing destination takes the new size.
    pub fn promote_sized(&self, agent: Uuid, end: &RegionEnd, seed: &str, size: [u32; 2]) {
        self.promote_with(agent, end, seed, Some(size));
    }

    fn promote_with(&self, agent: Uuid, end: &RegionEnd, seed: &str, size: Option<[u32; 2]>) {
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
            if let Some(size) = size {
                destination.size = known_region_size(size);
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
                size: known_region_size(size.unwrap_or([DEFAULT_REGION_SIZE, DEFAULT_REGION_SIZE])),
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

    /// Current-generation destinations. `open` is the phase the position
    /// update may use. A pending claim is included so a later pass can tell
    /// it from a neighbour that was never claimed.
    pub fn tracked(&self, agent: Uuid) -> Vec<TrackedChild> {
        let books = self.lock();
        let Some(book) = books.agents.get(&agent) else {
            return Vec::new();
        };
        let Some(generation) = book.generations.get(&book.current) else {
            return Vec::new();
        };
        generation
            .by_region
            .values()
            .map(|destination| TrackedChild {
                end: destination.end.clone(),
                promoted: destination.promoted,
                open: destination.phase == Phase::Open,
            })
            .collect()
    }

    /// Stored footprint of one current-generation destination.
    pub fn destination_size(&self, agent: Uuid, region: Uuid) -> Option<[u32; 2]> {
        let books = self.lock();
        let book = books.agents.get(&agent)?;
        book.generations
            .get(&book.current)?
            .by_region
            .get(&region)
            .map(|destination| destination.size)
    }

    /// `SetNeighbourRegionSizeInfo` (`ScenePresence.cs` 2129–2138). An in-view
    /// neighbour keeps the size this pass reported. A destination that is not
    /// in the list falls back to [`DEFAULT_REGION_SIZE`], which is what a
    /// cleared entry uses on the next close.
    pub fn refresh_sizes(&self, agent: Uuid, visible: &[(Uuid, [u32; 2])]) {
        let mut books = self.lock();
        let Some(book) = books.agents.get_mut(&agent) else {
            return;
        };
        let generation = book.current;
        let Some(generation_book) = book.generations.get_mut(&generation) else {
            return;
        };
        for destination in generation_book.by_region.values_mut() {
            let reported = visible
                .iter()
                .find(|(region, _)| *region == destination.end.region_id)
                .map(|(_, size)| *size);
            destination.size = match reported {
                Some(size) => known_region_size(size),
                None => [DEFAULT_REGION_SIZE, DEFAULT_REGION_SIZE],
            };
        }
    }

    /// The neighbour left the root's view. An open child closes once, with
    /// the session that claimed it. A pending admission closes when its POST
    /// succeeds and is not closed twice. A promoted destination is the remote
    /// root and stays.
    pub fn leave_view(&self, agent: Uuid, region: Uuid) -> Admission {
        let mut books = self.lock();
        let Some(generation) = books.agents.get(&agent).map(|book| book.current) else {
            return Admission::Quiet;
        };
        let Some((promoted, phase, end)) = books
            .agents
            .get(&agent)
            .and_then(|book| book.generations.get(&generation))
            .and_then(|generation_book| generation_book.by_region.get(&region))
            .map(|destination| {
                (
                    destination.promoted,
                    destination.phase,
                    destination.end.clone(),
                )
            })
        else {
            return Admission::Quiet;
        };
        if promoted {
            return Admission::Quiet;
        }
        match phase {
            Phase::Open => {
                let close = close_for(&books, agent, generation, end);
                remove_destination(&mut books, agent, generation, region);
                close
            }
            Phase::Pending | Phase::CloseWhenAccepted => {
                if let Some(destination) = generation_mut(&mut books, agent, generation)
                    .and_then(|generation_book| generation_book.by_region.get_mut(&region))
                {
                    destination.phase = Phase::CloseWhenAccepted;
                }
                Admission::Quiet
            }
        }
    }
}

/// One destination of the current generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedChild {
    pub end: RegionEnd,
    pub promoted: bool,
    pub open: bool,
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
    if book.generations.is_empty() && book.staged.is_empty() {
        books.agents.remove(&agent);
    }
}

#[derive(Clone, Copy)]
enum Finish<'a> {
    /// The circuit lease dropped. Open children are forgotten.
    Forget,
    /// A far hop. Close only what `GetChildAgentsToClose` would.
    Far(&'a FarDestination),
}

/// `GetChildAgentsToClose` (`ScenePresence.cs` 5002–5058). Grid Y 0 closes
/// every known child. The destination itself is never closed. Otherwise the
/// child is closed when `IsOutsideView(255, child, destination)` says it is
/// outside the destination's footprint. The extent is the child's stored
/// size (`m_knownChildRegionsSizeInfo`), which is [`DEFAULT_REGION_SIZE`]
/// when that size was unknown.
fn child_should_close(child: &Destination, destination: &FarDestination) -> bool {
    if child.end.region_id == destination.end.region_id {
        return false;
    }
    if (destination.end.loc_y >> 8) == 0 {
        return true;
    }
    outside_cells(
        (child.end.loc_x >> 8, child.end.loc_y >> 8),
        (destination.end.loc_x >> 8, destination.end.loc_y >> 8),
        (child.size[0], child.size[1]),
        (destination.size_x, destination.size_y),
    )
}

/// A zero extent is unknown. Pin8 falls back to `Constants.RegionSize`.
fn known_region_size(size: [u32; 2]) -> [u32; 2] {
    [
        if size[0] == 0 {
            DEFAULT_REGION_SIZE
        } else {
            size[0]
        },
        if size[1] == 0 {
            DEFAULT_REGION_SIZE
        } else {
            size[1]
        },
    ]
}

/// `Util.IsOutsideView` (`Util.cs` 1636–1656). Coordinates are region grid
/// cells. Sizes are metres. Origins are the cell times [`DEFAULT_REGION_SIZE`],
/// not the region's own size. The draw distance steps in by one metre, so
/// adjacent 256 m regions stay inside.
pub(crate) fn outside_cells(
    child: (u32, u32),
    destination: (u32, u32),
    child_size: (u32, u32),
    destination_size: (u32, u32),
) -> bool {
    let draw = CHILD_VIEW_METRES - 1;
    let child_x = u64::from(child.0) * u64::from(DEFAULT_REGION_SIZE);
    let destination_x = u64::from(destination.0) * u64::from(DEFAULT_REGION_SIZE);
    let child_y = u64::from(child.1) * u64::from(DEFAULT_REGION_SIZE);
    let destination_y = u64::from(destination.1) * u64::from(DEFAULT_REGION_SIZE);
    child_x + u64::from(child_size.0) + draw < destination_x
        || destination_x + u64::from(destination_size.0) + draw < child_x
        || child_y + u64::from(child_size.1) + draw < destination_y
        || destination_y + u64::from(destination_size.1) + draw < child_y
}

fn generation_session(books: &Books, agent: Uuid, generation: Uuid) -> Uuid {
    let Some(book) = books.agents.get(&agent) else {
        return Uuid::nil();
    };
    let stored = book
        .generations
        .get(&generation)
        .map(|generation_book| generation_book.session)
        .unwrap_or_default();
    if stored.is_nil() {
        book.session
    } else {
        stored
    }
}

fn close_for(books: &Books, agent: Uuid, generation: Uuid, end: RegionEnd) -> Admission {
    Admission::Close {
        end,
        session: generation_session(books, agent, generation),
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
    use std::sync::Arc;

    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn end(region: u128) -> RegionEnd {
        RegionEnd {
            server_uri: format!("http://192.0.2.{region}/"),
            region_id: Uuid::from_u128(region),
            name: "Neighbour".to_string(),
            loc_x: 256,
            loc_y: 256,
        }
    }

    fn placed(region: u128, loc_x: u32, loc_y: u32) -> RegionEnd {
        let mut end = end(region);
        end.loc_x = loc_x;
        end.loc_y = loc_y;
        end
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
        assert_eq!(
            admission,
            Admission::Close {
                end: end(5),
                session: session(),
            }
        );
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
        assert_eq!(
            admission,
            Admission::Close {
                end: end(6),
                session: session(),
            }
        );
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
        assert_eq!(
            admission,
            Admission::Close {
                end: end(5),
                session: session(),
            }
        );
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
        assert_eq!(
            book.accepted(agent, older, Uuid::from_u128(2)),
            Admission::Close {
                end: end(2),
                session: session(),
            }
        );
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

    #[test]
    fn a_dropped_claim_can_be_claimed_again() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = Arc::new(book_for(agent, generation));
        let guard = book
            .claim_work(agent, generation, end(5))
            .unwrap_or_else(|_| panic!("claim"));
        assert!(!guard.seed().is_empty());
        drop(guard);
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("a dropped claim should be free");
        };
    }

    #[test]
    fn a_guard_is_not_issued_for_a_claim_another_task_owns() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = Arc::new(book_for(agent, generation));
        let guard = book
            .claim_work(agent, generation, end(5))
            .unwrap_or_else(|_| panic!("claim"));
        assert!(matches!(
            book.claim_work(agent, generation, end(5)),
            Err(Claim::Existing { .. })
        ));
        drop(guard);
        assert!(book.claim_work(agent, generation, end(5)).is_ok());
    }

    #[test]
    fn a_settled_claim_keeps_the_open_child() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = Arc::new(book_for(agent, generation));
        let guard = book
            .claim_work(agent, generation, end(5))
            .unwrap_or_else(|_| panic!("claim"));
        let region = Uuid::from_u128(5);
        assert!(matches!(
            book.accepted(agent, generation, region),
            Admission::Announce { .. }
        ));
        guard.settle();
        assert!(book.seed(agent, region).is_some());
    }

    #[test]
    fn a_timeout_after_logout_closes_with_the_claim_session() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let region = Uuid::from_u128(5);
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("claim");
        };
        book.begin_logout(agent);
        assert_eq!(
            book.report(agent, generation, region, PostReport::TimedOut),
            Admission::Close {
                end: end(5),
                session: session(),
            }
        );
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("claim");
        };
        book.begin_logout(agent);
        assert_eq!(
            book.report(agent, generation, region, PostReport::Transport),
            Admission::Close {
                end: end(5),
                session: session(),
            }
        );
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(5)) else {
            panic!("claim");
        };
        book.begin_logout(agent);
        assert_eq!(
            book.report(agent, generation, region, PostReport::Refused),
            Admission::Quiet
        );
    }

    #[test]
    fn a_stale_close_uses_the_session_that_claimed_it() {
        let agent = Uuid::from_u128(4);
        let older = Uuid::from_u128(8);
        let replacement = Uuid::from_u128(9);
        let claimed = Uuid::from_u128(41);
        let book = ChildBook::new();
        book.attach(agent, older, claimed);
        let Claim::New { .. } = book.claim(agent, older, end(6)) else {
            panic!("claim");
        };
        book.attach(agent, replacement, Uuid::from_u128(70));
        assert_eq!(
            book.accepted(agent, older, Uuid::from_u128(6)),
            Admission::Close {
                end: end(6),
                session: claimed,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pause_before_announce_waits_half_a_second() {
        let started = tokio::time::Instant::now();
        let waiting = tokio::spawn(pause_before_announce());
        tokio::time::sleep(Duration::from_millis(499)).await;
        assert!(!waiting.is_finished());
        tokio::time::sleep(Duration::from_millis(1)).await;
        waiting.await.unwrap();
        assert_eq!(started.elapsed(), ANNOUNCE_DELAY);
    }

    #[test]
    fn a_dropped_lease_forgets_open_children_and_stages_nothing() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(1)) else {
            panic!("open claim");
        };
        assert!(matches!(
            book.accepted(agent, generation, Uuid::from_u128(1)),
            Admission::Announce { .. }
        ));
        let Claim::New { .. } = book.claim(agent, generation, end(2)) else {
            panic!("pending claim");
        };
        book.promote(agent, &end(3), "seed");
        book.end_generation(agent, generation);
        assert!(book.take_staged_closes(agent).is_empty());
        assert!(book.seed(agent, Uuid::from_u128(1)).is_none());
        assert_eq!(
            book.accepted(agent, generation, Uuid::from_u128(2)),
            Admission::Close {
                end: end(2),
                session: session(),
            }
        );
        assert!(book.seed(agent, Uuid::from_u128(3)).is_none());
    }

    #[test]
    fn a_far_hop_closes_a_farther_child_and_forgets_an_adjacent_one() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        // Destination at cell (1, 1). The next cell stays inside a 255 m view.
        // Four cells away does not. The destination itself is an open child.
        let destination = placed(9, 256, 256);
        let adjacent = placed(2, 512, 256);
        let farther = placed(3, 1024, 256);
        for region in [&destination, &adjacent, &farther] {
            let Claim::New { .. } = book.claim(agent, generation, region.clone()) else {
                panic!("claim");
            };
            assert!(matches!(
                book.accepted(agent, generation, region.region_id),
                Admission::Announce { .. }
            ));
        }
        book.close_far(
            agent,
            &FarDestination {
                end: destination.clone(),
                size_x: DEFAULT_REGION_SIZE,
                size_y: DEFAULT_REGION_SIZE,
            },
        );
        let staged = book.take_staged_closes(agent);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, session());
        assert_eq!(staged[0].1.region_id, farther.region_id);
        assert!(book.seed(agent, adjacent.region_id).is_none());
        assert!(book.seed(agent, destination.region_id).is_none());
        assert!(book.take_staged_closes(agent).is_empty());
    }

    #[test]
    fn a_far_hop_to_grid_y_zero_closes_every_child_except_the_destination() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let destination = placed(9, 256, 0);
        let neighbour = placed(2, 512, 256);
        for region in [&destination, &neighbour] {
            let Claim::New { .. } = book.claim(agent, generation, region.clone()) else {
                panic!("claim");
            };
            assert!(matches!(
                book.accepted(agent, generation, region.region_id),
                Admission::Announce { .. }
            ));
        }
        book.close_far(
            agent,
            &FarDestination {
                end: destination.clone(),
                size_x: DEFAULT_REGION_SIZE,
                size_y: DEFAULT_REGION_SIZE,
            },
        );
        let staged = book.take_staged_closes(agent);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, session());
        assert_eq!(staged[0].1.region_id, neighbour.region_id);
        assert!(book.seed(agent, destination.region_id).is_none());
    }

    /// B is 512 m at grid (1000, 1000). A far hop from A's cell (1002, 1000)
    /// to D at (1000, 1002) still sees B: the child's own size reaches D.
    /// The same cell with an unknown size uses 256 m and is outside.
    #[test]
    fn a_varregion_child_stays_inside_a_far_hop() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let wide = placed(2, 1000 * 256, 1000 * 256);
        let unknown = placed(3, 1000 * 256, 1000 * 256);
        let destination = placed(4, 1000 * 256, 1002 * 256);
        let Claim::New { .. } = book.claim_sized(agent, generation, wide.clone(), [512, 512])
        else {
            panic!("wide claim");
        };
        let Claim::New { .. } = book.claim_sized(agent, generation, unknown.clone(), [0, 0]) else {
            panic!("unknown claim");
        };
        for region in [&wide, &unknown] {
            assert!(matches!(
                book.accepted(agent, generation, region.region_id),
                Admission::Announce { .. }
            ));
        }
        book.close_far(
            agent,
            &FarDestination {
                end: destination,
                size_x: DEFAULT_REGION_SIZE,
                size_y: DEFAULT_REGION_SIZE,
            },
        );
        let staged = book.take_staged_closes(agent);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, session());
        assert_eq!(staged[0].1.region_id, unknown.region_id);
        assert!(book.seed(agent, wide.region_id).is_none());
    }

    /// A stored size of 0 would put the child one cell to the left outside.
    /// A zero extent is unknown and falls back to 256 m, so it stays.
    #[test]
    fn a_zero_size_child_beside_the_destination_stays_inside() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let beside = placed(2, 256, 256);
        let destination = placed(9, 512, 256);
        let Claim::New { .. } = book.claim_sized(agent, generation, beside.clone(), [0, 0]) else {
            panic!("claim");
        };
        assert!(matches!(
            book.accepted(agent, generation, beside.region_id),
            Admission::Announce { .. }
        ));
        book.close_far(
            agent,
            &FarDestination {
                end: destination,
                size_x: DEFAULT_REGION_SIZE,
                size_y: DEFAULT_REGION_SIZE,
            },
        );
        assert!(
            book.take_staged_closes(agent).is_empty(),
            "a zero-size neighbour beside the destination was closed"
        );
    }

    #[test]
    fn promote_sized_stores_the_footprint_and_unsized_promote_leaves_it() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let existing = end(1);
        let Claim::New { .. } = book.claim_sized(agent, generation, existing.clone(), [128, 128])
        else {
            panic!("claim");
        };
        book.promote_sized(agent, &existing, "seed", [512, 64]);
        assert_eq!(
            book.destination_size(agent, existing.region_id),
            Some([512, 64])
        );
        book.promote(agent, &existing, "seed");
        assert_eq!(
            book.destination_size(agent, existing.region_id),
            Some([512, 64])
        );
        let fresh = end(2);
        book.promote_sized(agent, &fresh, "seed", [0, 300]);
        assert_eq!(
            book.destination_size(agent, fresh.region_id),
            Some([256, 300])
        );
        let plain = end(3);
        book.promote(agent, &plain, "seed");
        assert_eq!(
            book.destination_size(agent, plain.region_id),
            Some([256, 256])
        );
    }

    #[test]
    fn refresh_sizes_keeps_the_reported_extent_and_clears_the_rest() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let wide = end(1);
        let pending = end(2);
        let Claim::New { .. } = book.claim_sized(agent, generation, wide.clone(), [512, 512])
        else {
            panic!("wide");
        };
        let Claim::New { .. } = book.claim_sized(agent, generation, pending.clone(), [128, 128])
        else {
            panic!("pending");
        };
        book.refresh_sizes(agent, &[(wide.region_id, [0, 400])]);
        assert_eq!(
            book.destination_size(agent, wide.region_id),
            Some([256, 400])
        );
        assert_eq!(
            book.destination_size(agent, pending.region_id),
            Some([256, 256])
        );
    }

    /// claim_work(G1, C), attach(G2, S), claim(G2, C), accepted(G2) announces.
    /// G1's cancelled admission and its timeout must not close C: G2 owns it
    /// on the same session, and CloseAgent would accept that session.
    #[test]
    fn a_replaced_admission_does_not_close_the_child_its_successor_owns() {
        let agent = Uuid::from_u128(4);
        let first = Uuid::from_u128(8);
        let second = Uuid::from_u128(9);
        let region = end(5);
        let shared = session();

        let book = Arc::new(ChildBook::new());
        book.attach(agent, first, shared);
        let guard = book
            .claim_work(agent, first, region.clone())
            .unwrap_or_else(|_| panic!("claim"));
        book.attach(agent, second, shared);
        let Claim::New { .. } = book.claim(agent, second, region.clone()) else {
            panic!("replacement claim");
        };
        assert!(matches!(
            book.accepted(agent, second, region.region_id),
            Admission::Announce { .. }
        ));
        drop(guard);
        assert!(
            book.take_staged_closes(agent).is_empty(),
            "dropping the replaced guard staged a close"
        );
        assert!(book.seed(agent, region.region_id).is_some());

        let book = ChildBook::new();
        book.attach(agent, first, shared);
        let Claim::New { .. } = book.claim(agent, first, region.clone()) else {
            panic!("claim");
        };
        book.attach(agent, second, shared);
        let Claim::New { .. } = book.claim(agent, second, region.clone()) else {
            panic!("replacement claim");
        };
        assert!(matches!(
            book.accepted(agent, second, region.region_id),
            Admission::Announce { .. }
        ));
        assert_eq!(
            book.report(agent, first, region.region_id, PostReport::TimedOut),
            Admission::Quiet
        );
        assert!(book.take_staged_closes(agent).is_empty());
        assert!(book.seed(agent, region.region_id).is_some());
    }

    #[tokio::test]
    async fn aborting_an_admission_post_drops_the_claim_until_logout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = Arc::new(book_for(agent, generation));

        let hold = {
            let book = Arc::clone(&book);
            tokio::spawn(async move {
                let guard = book
                    .claim_work(agent, generation, end(5))
                    .unwrap_or_else(|_| panic!("claim"));
                let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
                stream
                    .write_all(
                        b"POST /agent HTTP/1.1\r\nHost: neighbour\r\nContent-Length: 0\r\n\r\n",
                    )
                    .await
                    .unwrap();
                let mut buf = [0u8; 8];
                let _ = stream.read(&mut buf).await;
                guard.settle();
            })
        };
        let (mut accepted, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 128];
        let _ = accepted.read(&mut request).await.unwrap();
        hold.abort();
        let _ = hold.await;
        assert!(
            book.take_staged_closes(agent).is_empty(),
            "a cancelled admission before logout must not close"
        );
        assert!(book.claim_work(agent, generation, end(5)).is_ok());

        let hold = {
            let book = Arc::clone(&book);
            tokio::spawn(async move {
                let guard = book
                    .claim_work(agent, generation, end(6))
                    .unwrap_or_else(|_| panic!("claim"));
                let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
                stream
                    .write_all(
                        b"POST /agent HTTP/1.1\r\nHost: neighbour\r\nContent-Length: 0\r\n\r\n",
                    )
                    .await
                    .unwrap();
                let mut buf = [0u8; 8];
                let _ = stream.read(&mut buf).await;
                guard.settle();
            })
        };
        let (mut accepted, _) = listener.accept().await.unwrap();
        let _ = accepted.read(&mut request).await.unwrap();
        book.begin_logout(agent);
        // The task is still blocked on the POST. The close is staged when
        // the guard drops, so the take has to wait until the task stops.
        hold.abort();
        let _ = hold.await;
        let staged = book.take_staged_closes(agent);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, session());
        assert_eq!(staged[0].1.region_id, Uuid::from_u128(6));
    }

    #[test]
    fn a_create_timeout_is_not_a_refusal() {
        use crate::simulation_client::{CreateOutcome, CreateStop};

        let timeout = CreateOutcome {
            success: false,
            reason: "Service request failed: connection failed".to_string(),
            stop: CreateStop::TimedOut,
        };
        assert_eq!(PostReport::from_create(&timeout), PostReport::TimedOut);
        let transport = CreateOutcome {
            success: false,
            reason: timeout.reason.clone(),
            stop: CreateStop::Transport,
        };
        assert_eq!(PostReport::from_create(&transport), PostReport::Transport);
        let refused = CreateOutcome {
            success: false,
            reason: String::new(),
            stop: CreateStop::Answered,
        };
        assert_eq!(PostReport::from_create(&refused), PostReport::Refused);
        let accepted = CreateOutcome {
            success: true,
            reason: String::new(),
            stop: CreateStop::Answered,
        };
        assert_eq!(PostReport::from_create(&accepted), PostReport::Accepted);
    }

    #[test]
    fn leave_view_closes_an_open_child_and_holds_a_pending_one() {
        let agent = Uuid::from_u128(4);
        let generation = Uuid::from_u128(8);
        let book = book_for(agent, generation);
        let Claim::New { .. } = book.claim(agent, generation, end(1)) else {
            panic!("open claim");
        };
        assert!(matches!(
            book.accepted(agent, generation, Uuid::from_u128(1)),
            Admission::Announce { .. }
        ));
        let Claim::New { .. } = book.claim(agent, generation, end(2)) else {
            panic!("pending claim");
        };
        book.promote(agent, &end(3), "seed");
        assert_eq!(
            book.leave_view(agent, Uuid::from_u128(1)),
            Admission::Close {
                end: end(1),
                session: session(),
            }
        );
        assert!(book.seed(agent, Uuid::from_u128(1)).is_none());
        assert_eq!(book.leave_view(agent, Uuid::from_u128(2)), Admission::Quiet);
        assert_eq!(
            book.accepted(agent, generation, Uuid::from_u128(2)),
            Admission::Close {
                end: end(2),
                session: session(),
            }
        );
        assert_eq!(book.leave_view(agent, Uuid::from_u128(3)), Admission::Quiet);
        assert_eq!(
            book.seed(agent, Uuid::from_u128(3)).as_deref(),
            Some("seed")
        );
        let tracked = book.tracked(agent);
        assert_eq!(tracked.len(), 1);
        assert!(tracked[0].promoted);
        assert_eq!(book.leave_view(agent, Uuid::from_u128(9)), Admission::Quiet);
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
