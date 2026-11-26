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
//     (DoTeleportInternal, TransferAgent_V2, Fail, NeedsClosing, the unknown-region map block)
//   Source/OpenSim.Framework/Util.cs (IsOutsideView, region-handle conversion)
//   Source/OpenSim.Framework/CapsUtil.cs (GetCapsSeedPath, GetRandomCapsObjectPath)
//   Source/OpenSim.Framework/ChildAgentDataUpdate.cs (NewCallbackURI packed as cb_uri)
//   Source/OpenSim.Services.Connectors/Grid/GridServicesConnector.cs (GetRegionByPosition)
//   Source/OpenSim.Region.Framework/Scenes/Scene.cs (CloseAgent with force false)
//   OpenSim-NGC/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One source-region hop.
//!
//! `QueryAccess`, then `TeleportStart`, then `CreateAgent` with `child` true,
//! then an event-queue `TeleportFinish`. The root is demoted before
//! `UpdateAgent`. An in-view neighbour keeps that child: the hop reuses the
//! caps seed already stored for that destination, and the circuit stays. The
//! hop does not release or close the agent. A destination
//! outside that view, or one whose grid Y is 0, waits for the destination's
//! release DELETE and then closes the circuit without a kick. The grid is asked
//! when the local directory has no handle.

use std::net::{Ipv4Addr, SocketAddr};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Duration;

use ferroan_client::{
    CapsRegistry, EventQueueError, FinishEvent, HopOut, HopRequest, INTERNAL, NOT_FOUND,
    RESOLVE_FAILED, SourceHop, caps_seed, connect_reason, offline_map_block, teleport_finish_event,
};
use ferroan_llsd::Llsd;
#[cfg(test)]
use tokio::sync::Notify;
use uuid::Uuid;

use crate::config::RegionConfig;
use crate::grid::{GridClient, GridError, LocatedRegion};
use crate::simulation::SimulationState;
use crate::simulation_client::{
    AccessQuery, AgentData, Appearance, Circuit, RegionEnd, SimulationClient, TransferAnimations,
    TransferContext,
};

/// One accepted root, plus the region it is leaving.
#[derive(Clone, Debug)]
struct HopSession {
    agent_id: Uuid,
    session_id: Uuid,
    secure_session_id: Uuid,
    circuit_code: u32,
    first_name: String,
    last_name: String,
    source: RegionEnd,
    size_x: u32,
    size_y: u32,
    viewer: String,
    channel: String,
    mac: String,
    id0: String,
    animations: Arc<Mutex<TransferAnimations>>,
    service_urls: Vec<(String, String)>,
    appearance: Option<Appearance>,
}

/// A neighbour the hop can try. `endpoint` is the address the viewer connects to.
#[derive(Clone, Debug)]
pub struct HopDestination {
    pub end: RegionEnd,
    pub endpoint: SocketAddr,
    pub size_x: u32,
    pub size_y: u32,
}

/// Looks up a region handle. Production is empty; the grid is the fallback.
pub trait RegionDirectory: Send + Sync {
    fn find(&self, handle: u64) -> Option<HopDestination>;
}

/// Every foreign handle is unknown until the grid answers.
#[derive(Clone, Copy, Debug)]
pub struct EmptyDirectory;

impl RegionDirectory for EmptyDirectory {
    fn find(&self, _handle: u64) -> Option<HopDestination> {
        None
    }
}

/// Default draw distance clamped into the region view. The view test then
/// steps one metre in, so adjacent 256 m regions stay in view.
const REGION_VIEW_METRES: u64 = 255;

/// The root this hop demotes. Tests record the calls instead of touching a session.
#[derive(Clone)]
pub enum HopPresence {
    /// No session. Unit hops that stay in view use this.
    Idle,
    /// The region's sessions, caps and release table.
    Live(Box<SimulationState>),
    /// A child book with no region session. The hop reads its seeds.
    #[cfg(test)]
    Seeded(Arc<crate::children::ChildBook>),
    #[cfg(test)]
    Watch(Arc<PresenceWatch>),
}

#[cfg(test)]
pub struct PresenceWatch {
    demoted: AtomicBool,
    restored: AtomicBool,
    armed: AtomicBool,
    closed: AtomicBool,
    woke: AtomicBool,
    /// `Some(near)` once the hop's kills were asked for.
    moved: std::sync::Mutex<Option<bool>>,
    budget: Duration,
    release: Notify,
}

#[cfg(test)]
impl PresenceWatch {
    pub(crate) fn new(budget: Duration) -> Arc<Self> {
        Arc::new(Self {
            demoted: AtomicBool::new(false),
            restored: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            woke: AtomicBool::new(false),
            moved: std::sync::Mutex::new(None),
            budget,
            release: Notify::new(),
        })
    }

    pub(crate) fn release(&self) {
        self.release.notify_one();
    }

    pub(crate) fn demoted(&self) -> bool {
        self.demoted.load(Ordering::Acquire)
    }

    pub(crate) fn restored(&self) -> bool {
        self.restored.load(Ordering::Acquire)
    }

    pub(crate) fn armed(&self) -> bool {
        self.armed.load(Ordering::Acquire)
    }

    pub(crate) fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn woke(&self) -> bool {
        self.woke.load(Ordering::Acquire)
    }

    pub(crate) fn moved(&self) -> Option<bool> {
        *self
            .moved
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl HopPresence {
    pub fn live(state: SimulationState) -> Self {
        Self::Live(Box::new(state))
    }

    pub fn idle() -> Self {
        Self::Idle
    }

    #[cfg(test)]
    pub(crate) fn watch(watch: Arc<PresenceWatch>) -> Self {
        Self::Watch(watch)
    }

    // Reserve before telling the viewer to leave. A failed create or enqueue
    // releases the place without demoting the root.
    fn prepare_demotion(&self, agent: Uuid) -> Result<impl FnOnce() + '_, &'static str> {
        let reservation = match self {
            Self::Live(state) => Some((state, state.reserve_child_place()?)),
            _ => None,
        };
        Ok(move || {
            if let Some((state, place)) = reservation {
                state.demote_with_place(agent, place);
            }
            #[cfg(test)]
            if let Self::Watch(watch) = self {
                watch.demoted.store(true, Ordering::Release);
            }
        })
    }

    fn restore(&self, agent: Uuid) {
        match self {
            Self::Idle => {}
            #[cfg(test)]
            Self::Seeded(_) => {}
            Self::Live(state) => state.restore_root(agent),
            #[cfg(test)]
            Self::Watch(watch) => watch.restored.store(true, Ordering::Release),
        }
    }

    /// `HasMovedAway` and `MakeChildAgent`: the kills the frame sends once
    /// the destination has the avatar.
    async fn moved_away(&self, agent: Uuid, near: bool, end: &RegionEnd) {
        match self {
            Self::Idle => {}
            #[cfg(test)]
            Self::Seeded(_) => {}
            Self::Live(state) => state.moved_away(agent, near, end).await,
            #[cfg(test)]
            Self::Watch(watch) => {
                *watch
                    .moved
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = Some(near);
            }
        }
    }

    fn child_seed(&self, agent: Uuid, region: Uuid) -> Option<String> {
        match self {
            Self::Idle => None,
            Self::Live(state) => state.child_seed(agent, region),
            #[cfg(test)]
            Self::Seeded(book) => book.seed(agent, region),
            #[cfg(test)]
            Self::Watch(_) => None,
        }
    }

    fn note_child(&self, agent: Uuid, end: &RegionEnd, seed: &str) {
        match self {
            Self::Idle => {}
            Self::Live(state) => state.promote_child(agent, end, seed),
            #[cfg(test)]
            Self::Seeded(book) => book.promote(agent, end, seed),
            #[cfg(test)]
            Self::Watch(_) => {}
        }
    }

    fn take_staged(&self, agent: Uuid) -> Vec<(Uuid, RegionEnd)> {
        match self {
            Self::Idle => Vec::new(),
            Self::Live(state) => state.take_staged_closes(agent),
            #[cfg(test)]
            Self::Seeded(_) | Self::Watch(_) => Vec::new(),
        }
    }

    fn take_children(&self, agent: Uuid) -> Vec<RegionEnd> {
        match self {
            Self::Idle => Vec::new(),
            Self::Live(state) => state.take_open_children(agent),
            #[cfg(test)]
            Self::Seeded(book) => book.begin_logout(agent),
            #[cfg(test)]
            Self::Watch(_) => Vec::new(),
        }
    }

    fn arm(&self, agent: Uuid) {
        match self {
            Self::Idle => {}
            #[cfg(test)]
            Self::Seeded(_) => {}
            Self::Live(state) => state.arm_release(agent),
            #[cfg(test)]
            Self::Watch(watch) => watch.armed.store(true, Ordering::Release),
        }
    }

    fn disarm(&self, agent: Uuid) {
        match self {
            Self::Idle => {}
            #[cfg(test)]
            Self::Seeded(_) => {}
            Self::Live(state) => state.disarm_release(agent),
            #[cfg(test)]
            Self::Watch(watch) => watch.armed.store(false, Ordering::Release),
        }
    }

    async fn wait_release(&self, agent: Uuid) {
        match self {
            Self::Idle => {}
            #[cfg(test)]
            Self::Seeded(_) => {}
            Self::Live(state) => state.wait_release(agent).await,
            #[cfg(test)]
            Self::Watch(watch) => {
                let notified = watch.release.notified();
                tokio::pin!(notified);
                if tokio::time::timeout(watch.budget, notified).await.is_ok() {
                    watch.woke.store(true, Ordering::Release);
                }
            }
        }
    }

    fn close_local(&self, agent: Uuid) {
        match self {
            Self::Idle => {}
            #[cfg(test)]
            Self::Seeded(_) => {}
            Self::Live(state) => state.close_departed(agent),
            #[cfg(test)]
            Self::Watch(watch) => watch.closed.store(true, Ordering::Release),
        }
    }
}

#[derive(Clone)]
struct HopGrid {
    client: GridClient,
    scope: Uuid,
}

/// Login facts the hop needs. The destination caps path is a new object path.
#[derive(Clone, Debug)]
pub struct HopLogin {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub secure_session_id: Uuid,
    pub circuit_code: u32,
    pub first_name: String,
    pub last_name: String,
    pub viewer: String,
    pub channel: String,
    pub mac: String,
    pub id0: String,
    pub animations: Arc<Mutex<TransferAnimations>>,
    pub service_urls: Vec<(String, String)>,
    pub appearance: Option<Appearance>,
}

/// Shared neighbour client and the source region. One per region process.
#[derive(Clone)]
pub struct HopEnv {
    source: RegionEnd,
    size_x: u32,
    size_y: u32,
    client: Option<SimulationClient>,
    caps: CapsRegistry,
    grid: HopGrid,
    presence: HopPresence,
}

impl HopEnv {
    /// An empty directory plus the grid. A client that cannot be built still
    /// reports unknown regions.
    pub fn from_region(config: &RegionConfig, caps: CapsRegistry, presence: HopPresence) -> Self {
        let client = match SimulationClient::new() {
            Ok(client) => Some(client),
            Err(error) => {
                tracing::error!("ferroan-region: simulation client unavailable: {error}");
                None
            }
        };
        Self {
            source: RegionEnd {
                server_uri: config.server_uri.clone(),
                region_id: config.id,
                name: config.name.clone(),
                loc_x: config.location[0].wrapping_mul(256),
                loc_y: config.location[1].wrapping_mul(256),
            },
            size_x: config.size[0],
            size_y: config.size[1],
            client,
            caps,
            grid: HopGrid {
                client: GridClient::new(config),
                scope: config.scope_id,
            },
            presence,
        }
    }

    /// Close children a replaced root left behind. The auth token is the session
    /// that opened them. One take, so the old circuit and the new login cannot
    /// both send the same close. The sends run on a separate task, as
    /// `SendCloseChildAgentConnections` does: an unresponsive neighbour does
    /// not delay the new login.
    pub async fn close_staged(&self, agent: Uuid) {
        let staged = self.presence.take_staged(agent);
        let Some(client) = self.client.clone() else {
            return;
        };
        if staged.is_empty() {
            return;
        }
        let _detached = tokio::spawn(async move {
            for (session, end) in staged {
                if let Err(error) = client.close_agent(&end, agent, &session.to_string()).await {
                    tracing::debug!("replaced root child close: {error}");
                }
            }
        });
    }

    /// Close the in-view neighbour children this root opened. One attempt each.
    /// The error text can carry a URL, so it stays at debug and is not shown.
    pub async fn close_open_child(&self, agent: Uuid, session: Uuid) {
        let ends = self.presence.take_children(agent);
        let Some(client) = &self.client else {
            return;
        };
        let auth = session.to_string();
        for end in ends {
            if let Err(error) = client.close_agent(&end, agent, &auth).await {
                tracing::debug!("logout child close: {error}");
            }
        }
    }

    /// The hop for one accepted root. Events go to that agent's queue.
    pub fn install(&self, login: HopLogin) -> SourceHop {
        let session = HopSession {
            agent_id: login.agent_id,
            session_id: login.session_id,
            secure_session_id: login.secure_session_id,
            circuit_code: login.circuit_code,
            first_name: login.first_name,
            last_name: login.last_name,
            source: self.source.clone(),
            size_x: self.size_x,
            size_y: self.size_y,
            viewer: login.viewer,
            channel: login.channel,
            mac: login.mac,
            id0: login.id0,
            animations: login.animations,
            service_urls: login.service_urls,
            appearance: login.appearance,
        };
        let caps = self.caps.clone();
        let agent_id = login.agent_id;
        drive(
            session,
            self.client.clone(),
            Arc::new(EmptyDirectory),
            Some(self.grid.clone()),
            self.presence.clone(),
            move |event| caps.enqueue_for(agent_id, event),
        )
    }
}

struct HopWork<'a> {
    session: &'a HopSession,
    client: Option<&'a SimulationClient>,
    directory: &'a dyn RegionDirectory,
    grid: Option<&'a HopGrid>,
    presence: &'a HopPresence,
    request: &'a HopRequest,
    out: &'a HopOut,
    enqueue: &'a (dyn Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync),
}

fn drive(
    session: HopSession,
    client: Option<SimulationClient>,
    directory: Arc<dyn RegionDirectory>,
    grid: Option<HopGrid>,
    presence: HopPresence,
    enqueue: impl Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync + 'static,
) -> SourceHop {
    let enqueue = Arc::new(enqueue);
    SourceHop::new(move |request, out| {
        let session = session.clone();
        let client = client.clone();
        let directory = Arc::clone(&directory);
        let grid = grid.clone();
        let presence = presence.clone();
        let enqueue = Arc::clone(&enqueue);
        Box::pin(async move {
            let work = HopWork {
                session: &session,
                client: client.as_ref(),
                directory: directory.as_ref(),
                grid: grid.as_ref(),
                presence: &presence,
                request: &request,
                out: &out,
                enqueue: enqueue.as_ref(),
            };
            perform(work).await;
        })
    })
}

async fn perform(work: HopWork<'_>) {
    let HopWork {
        session,
        client,
        directory,
        grid,
        presence,
        request,
        out,
        enqueue,
    } = work;
    let Some((destination, position)) =
        locate(directory, grid, request.region_handle, request.position).await
    else {
        tracing::debug!("source hop: unknown region");
        out.teleport_failed(NOT_FOUND).await;
        out.map_block(offline_map_block(session.agent_id, request.region_handle))
            .await;
        return;
    };
    let Some((ip, port)) = viewer_endpoint(destination.endpoint) else {
        out.teleport_failed(RESOLVE_FAILED).await;
        return;
    };
    let Some(client) = client else {
        out.teleport_failed(INTERNAL).await;
        return;
    };
    tracing::debug!("source hop: query");
    let outcome = match client
        .query_access(&destination.end, &access_query(session, position))
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::debug!("source hop: query: {error}");
            out.teleport_failed(INTERNAL).await;
            return;
        }
    };
    if !outcome.success {
        out.teleport_failed(&outcome.reason).await;
        return;
    }
    let demote = match presence.prepare_demotion(session.agent_id) {
        Ok(demote) => demote,
        Err(reason) => {
            out.teleport_failed(reason).await;
            return;
        }
    };
    if !out.teleport_start(request.flags).await {
        return;
    }
    let context = TransferContext {
        inbound: outcome.inbound_version,
        outbound: outcome.outbound_version,
        wearables_count: outcome.wearables_count,
    };
    // An in-view destination reuses the seed stored before its admission POST.
    // Outside the view, the destination circuit gets a new path. The viewer
    // seed uses that same path.
    let outside = needs_closing(session, &destination);
    let caps_path = if outside {
        Uuid::new_v4().to_string()
    } else {
        presence
            .child_seed(session.agent_id, destination.end.region_id)
            .unwrap_or_else(|| Uuid::new_v4().to_string())
    };
    let mut circuit = child_circuit(session, request, &caps_path, position);
    tracing::debug!("source hop: create");
    let created = match client
        .create_agent(
            Some(&session.source),
            Some(&destination.end),
            &mut circuit,
            request.flags,
            &context,
        )
        .await
    {
        Ok(created) => created,
        Err(error) => {
            tracing::debug!("source hop: create: {error}");
            out.teleport_failed(INTERNAL).await;
            return;
        }
    };
    if !created.success {
        out.teleport_failed(&created.reason).await;
        return;
    }
    let seed = caps_seed(&destination.end.server_uri, &caps_path);
    let handle = destination_handle(&destination);
    let event = teleport_finish_event(&FinishEvent {
        agent: session.agent_id,
        handle,
        endpoint: ip,
        port,
        flags: request.flags,
        seed,
        size_x: destination.size_x,
        size_y: destination.size_y,
    });
    if enqueue(event).is_err() {
        close_destination(client, &destination.end, session).await;
        out.teleport_failed(&connect_reason(&destination.end.name))
            .await;
        return;
    }
    let callback = if outside {
        presence.arm(session.agent_id);
        Some(release_uri(session))
    } else {
        None
    };
    let update = agent_update(session, request, position, callback);
    demote();
    tracing::debug!("source hop: update");
    let arrived = match client
        .update_agent(&destination.end, &update, &context)
        .await
    {
        Ok(arrived) => arrived,
        Err(error) => {
            tracing::debug!("source hop: update: {error}");
            false
        }
    };
    if !arrived {
        if outside {
            presence.disarm(session.agent_id);
        }
        presence.restore(session.agent_id);
        close_destination(client, &destination.end, session).await;
        out.teleport_failed(&connect_reason(&destination.end.name))
            .await;
        return;
    }
    presence
        .moved_away(session.agent_id, !outside, &destination.end)
        .await;
    if !outside {
        // The hop owns this destination until this generation is root again.
        // Logout does not close the remote root. A late admission still
        // announces once.
        presence.note_child(session.agent_id, &destination.end, &caps_path);
    }
    if outside {
        presence.wait_release(session.agent_id).await;
        presence.close_local(session.agent_id);
        out.leave().await;
    }
}

async fn locate(
    directory: &dyn RegionDirectory,
    grid: Option<&HopGrid>,
    handle: u64,
    position: [f32; 3],
) -> Option<(HopDestination, [f32; 3])> {
    if let Some(found) = directory.find(handle) {
        return Some((found, position));
    }
    let grid = grid?;
    match grid_destination(grid, handle, position).await {
        Ok(found) => found,
        Err(_) => {
            tracing::debug!("source hop: grid lookup failed");
            None
        }
    }
}

async fn grid_destination(
    grid: &HopGrid,
    handle: u64,
    mut position: [f32; 3],
) -> Result<Option<(HopDestination, [f32; 3])>, GridError> {
    let (world_x, world_y) = handle_world(handle);
    let (x, y, adjust) = if world_y == 0 {
        let Ok(x) = i32::try_from(world_x) else {
            return Ok(None);
        };
        (x, 0, false)
    } else {
        let (Some(x), Some(y)) = (
            grid_corner(world_x, position[0]),
            grid_corner(world_y, position[1]),
        ) else {
            return Ok(None);
        };
        (x, y, true)
    };
    let Some(found) = grid.client.region_by_position(grid.scope, x, y).await? else {
        return Ok(None);
    };
    if adjust {
        position[0] += world_x as f32 - found.loc_x as f32;
        position[1] += world_y as f32 - found.loc_y as f32;
    }
    Ok(Some((destination_from(found), position)))
}

fn destination_from(found: LocatedRegion) -> HopDestination {
    HopDestination {
        end: RegionEnd {
            server_uri: found.server_uri,
            region_id: found.region_id,
            name: found.name,
            loc_x: found.loc_x,
            loc_y: found.loc_y,
        },
        endpoint: found.endpoint,
        size_x: found.size_x,
        size_y: found.size_y,
    }
}

fn grid_corner(world: u32, offset: f32) -> Option<i32> {
    let metres = f64::from(world) + f64::from(offset);
    if !metres.is_finite() || metres < 0.0 || metres > f64::from(u32::MAX) {
        return None;
    }
    i32::try_from((metres as u32) & 0xffff_ff00).ok()
}

fn handle_cells(handle: u64) -> (u32, u32) {
    (
        ((handle >> 40) & 0x00ff_ffff) as u32,
        ((handle >> 8) & 0x00ff_ffff) as u32,
    )
}

fn handle_world(handle: u64) -> (u32, u32) {
    ((handle >> 32) as u32, handle as u32)
}

/// Region handle of the resolved destination, in metres.
fn destination_handle(destination: &HopDestination) -> u64 {
    (u64::from(destination.end.loc_x) << 32) | u64::from(destination.end.loc_y)
}

fn needs_closing(session: &HopSession, destination: &HopDestination) -> bool {
    if destination.end.loc_y == 0 {
        return true;
    }
    let (new_x, new_y) = handle_cells(destination_handle(destination));
    outside_view(
        (session.source.loc_x >> 8, session.source.loc_y >> 8),
        (new_x, new_y),
        (session.size_x, session.size_y),
        (destination.size_x, destination.size_y),
    )
}

fn outside_view(
    old: (u32, u32),
    new: (u32, u32),
    old_size: (u32, u32),
    new_size: (u32, u32),
) -> bool {
    let draw = REGION_VIEW_METRES - 1;
    let old_x = u64::from(old.0) * 256;
    let new_x = u64::from(new.0) * 256;
    let old_y = u64::from(old.1) * 256;
    let new_y = u64::from(new.1) * 256;
    old_x + u64::from(old_size.0) + draw < new_x
        || new_x + u64::from(new_size.0) + draw < old_x
        || old_y + u64::from(old_size.1) + draw < new_y
        || new_y + u64::from(new_size.1) + draw < old_y
}

fn release_uri(session: &HopSession) -> String {
    let base = session.source.server_uri.trim_end_matches('/');
    format!(
        "{base}/agent/{}/{}/release/",
        session.agent_id, session.source.region_id
    )
}

fn viewer_endpoint(endpoint: SocketAddr) -> Option<(Ipv4Addr, u16)> {
    match endpoint {
        SocketAddr::V4(address) if address.port() != 0 => Some((*address.ip(), address.port())),
        _ => None,
    }
}

fn access_query(session: &HopSession, position: [f32; 3]) -> AccessQuery {
    AccessQuery {
        agent_id: session.agent_id,
        agent_home_uri: session
            .service_urls
            .iter()
            .find(|(key, _)| key == "HomeURI")
            .map(|(_, value)| value.clone()),
        via_teleport: true,
        position,
        features: Vec::new(),
        context: TransferContext::negotiated(),
    }
}

fn child_circuit(
    session: &HopSession,
    request: &HopRequest,
    caps_path: &str,
    position: [f32; 3],
) -> Circuit {
    Circuit {
        agent_id: session.agent_id,
        base_folder: Uuid::nil(),
        caps_path: caps_path.to_owned(),
        children_seeds: Vec::new(),
        child: true,
        circuit_code: session.circuit_code,
        first_name: session.first_name.clone(),
        last_name: session.last_name.clone(),
        inventory_folder: Uuid::nil(),
        secure_session_id: session.secure_session_id,
        session_id: session.session_id,
        service_session_id: String::new(),
        start_pos: position,
        client_ip: request.client_ip.clone(),
        viewer: session.viewer.clone(),
        channel: session.channel.clone(),
        mac: session.mac.clone(),
        id0: session.id0.clone(),
        far: 0.0,
        appearance: session.appearance.clone(),
        service_urls: session.service_urls.clone(),
    }
}

/// Horizontal look, normalised the way OpenMetaverse normalises a vector.
/// A length squared at or below 1e-6, or a non-finite component, is a zero
/// look: unit X, unit Y and unit Z. The division is skipped, so a tiny look
/// cannot underflow to infinity. The pinned server stores unit Z as the at
/// axis and keeps the up axis `CopyTo` already stored (TB79, D64).
fn look_axes(look_at: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let up = [0.0, 0.0, 1.0];
    let zero = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], up);
    if !look_at[0].is_finite() || !look_at[1].is_finite() {
        return zero;
    }
    let length_squared = look_at[0] * look_at[0] + look_at[1] * look_at[1];
    // OpenMetaverse `Normalize` treats a length squared that is not above 1e-6
    // as the zero vector and does not divide.
    if length_squared <= 1.0e-6 {
        return zero;
    }
    let length = length_squared.sqrt();
    let at = [look_at[0] / length, look_at[1] / length, 0.0];
    (at, [-at[1], at[0], 0.0], up)
}

/// `AgentManager.ControlFlags.AGENT_CONTROL_FLY`.
const CONTROL_FLY: u32 = 1 << 13;

/// A flying departure tells the destination to fly: `IsFlying` in the hop's
/// teleport flags sets `AGENT_CONTROL_FLY` in the `AgentData` it sends
/// (`EntityTransferModule.cs` lines 1171–1172 and 1401–1402).
fn fly_control(teleport_flags: u32) -> u32 {
    if teleport_flags & ferroan_client::IS_FLYING != 0 {
        CONTROL_FLY
    } else {
        0
    }
}

/// Position is the destination the viewer asked for. `region_id` is this
/// region. The camera center and rotations are the last viewer update.
fn agent_update(
    session: &HopSession,
    request: &HopRequest,
    position: [f32; 3],
    callback: Option<String>,
) -> AgentData {
    let (at_axis, left_axis, up_axis) = look_axes(request.look_at);
    let animations = session
        .animations
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    AgentData {
        region_id: session.source.region_id,
        circuit_code: session.circuit_code,
        agent_id: session.agent_id,
        session_id: session.session_id,
        position,
        velocity: [0.0; 3],
        center: request.pose.center,
        size: [0.0; 3],
        at_axis,
        left_axis,
        up_axis,
        wait_for_root: true,
        // Default draw. A smaller far clamps the destination view inside this
        // region, so it never tells the viewer about the region just left.
        far: f32::from(u16::try_from(REGION_VIEW_METRES).unwrap_or(0)),
        throttles: Vec::new(),
        locomotion_state: 0,
        head_rotation: request.pose.head_rotation,
        body_rotation: request.pose.body_rotation,
        control_flags: fly_control(request.flags),
        // The region does not enable viewer god UI. Estate or admission rights alone never set it.
        viewer_ui_is_god: Some(false),
        always_run: request.pose.always_run,
        // CopyTo(false) for teleports. Only CopyTo(true) for a crossing copies the group.
        active_group_id: Uuid::nil(),
        active_group_name: String::new(),
        active_group_title: None,
        animations: animations.extra.clone(),
        agent_access: 0,
        default_animation: Some(animations.default_animation),
        animation_state: Some(animations.animation_state),
        motion_state: i32::from(animations.motion_state),
        appearance: None,
        sit_offset: [0.0; 3],
        children_seeds: Vec::new(),
        callback,
    }
}

async fn close_destination(
    client: &SimulationClient,
    destination: &RegionEnd,
    session: &HopSession,
) {
    let auth = session.session_id.to_string();
    if let Err(error) = client
        .close_agent(destination, session.agent_id, &auth)
        .await
    {
        // The viewer packet still goes out. The error text has no URL.
        tracing::debug!("source hop: close: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Read;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::{
        EmptyDirectory, HopDestination, HopGrid, HopPresence, HopSession, PresenceWatch,
        RegionDirectory, drive, outside_view, viewer_endpoint,
    };
    use crate::simulation_client::{RegionEnd, SimulationClient};
    use ferroan_client::{
        ArrivalConfig, CONNECT_INNER, CapsServer, CircuitExpect, CircuitServer, EventQueueError,
        NOT_FOUND, connect_reason,
    };
    use ferroan_llsd::Llsd;
    use ferroan_protocol::msgs::{
        DisableSimulator, KickUser, MapBlockReply, TeleportFailed, TeleportFinish, TeleportLocal,
        TeleportLocationRequest, TeleportStart, UseCircuitCode,
    };
    use ferroan_protocol::{Msg, build_packet, parse_packet};
    use flate2::read::GzDecoder;
    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream, UdpSocket};
    use tokio::sync::Notify;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    #[test]
    fn look_axes_normalise_a_heading_and_keep_up_on_a_zero() {
        let (at, left, up) = super::look_axes([0.0, 2.0, 4.0]);
        assert_eq!(at, [0.0, 1.0, 0.0]);
        assert_eq!(left, [-1.0, 0.0, 0.0]);
        assert_eq!(up, [0.0, 0.0, 1.0]);
        let (at, left, up) = super::look_axes([0.0, 0.0, 1.0]);
        assert_eq!(at, [1.0, 0.0, 0.0]);
        assert_ne!(at, [0.0, 0.0, 1.0]);
        assert!(!left[0].is_sign_negative());
        assert_eq!(left, [0.0, 1.0, 0.0]);
        assert_eq!(up, [0.0, 0.0, 1.0]);
        let (at, left, up) = super::look_axes([1.0e-4, 0.0, 0.0]);
        assert_eq!(
            (at, left, up),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0])
        );
        let (at, left, up) = super::look_axes([0.002, 0.0, 0.0]);
        assert_eq!(at, [1.0, 0.0, 0.0]);
        assert!(left[0].is_finite() && left[1].is_finite() && up[2].is_finite());
        assert!(!at[0].is_nan() && !left.iter().any(|part| part.is_nan()));
        let (at, _, up) = super::look_axes([f32::NAN, 1.0, 0.0]);
        assert_eq!(at, [1.0, 0.0, 0.0]);
        assert_eq!(up, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn destination_origin_stays_in_view_inside_a_large_region() {
        let mut home = session();
        home.source.loc_x = 0;
        home.source.loc_y = 256;
        let mut dest = destination("127.0.0.1:9".parse().unwrap());
        dest.end.loc_x = 256;
        dest.end.loc_y = 256;
        dest.size_x = 768;
        dest.size_y = 256;
        assert!(outside_view((0, 1), (2, 1), (256, 256), (256, 256)));
        assert!(!super::needs_closing(&home, &dest));
        dest.end.loc_x = 1024;
        assert!(super::needs_closing(&home, &dest));
    }

    const HANDLE: u64 = 256_u64 << 32;
    const POSITION: [f32; 3] = [128.0, 128.0, 30.0];
    const QUERY_OK: &str = "{\"success\":true,\"reason\":\"\",\"version\":\"SIMULATION/0.8\",\"negotiated_inbound_version\":0.8,\"negotiated_outbound_version\":0.8}";
    const CREATE_OK: &str = "{\"success\":true,\"reason\":\"\",\"service_session_id\":\"\"}";

    struct Listed(HashMap<u64, HopDestination>);

    impl RegionDirectory for Listed {
        fn find(&self, handle: u64) -> Option<HopDestination> {
            self.0.get(&handle).cloned()
        }
    }

    struct Hit {
        header: String,
        body: Vec<u8>,
    }

    struct Opened {
        running: JoinHandle<Result<(), ferroan_client::CircuitError>>,
        script: Option<JoinHandle<()>>,
        collecting: JoinHandle<()>,
        hits: Arc<Mutex<Vec<Hit>>>,
        gathered: Arc<Mutex<Vec<Vec<u8>>>>,
        socket: Arc<UdpSocket>,
        position: Arc<Mutex<[f32; 3]>>,
        release: Option<Arc<Notify>>,
        events: String,
        caps: Arc<Mutex<CapsServer>>,
    }

    impl Drop for Opened {
        fn drop(&mut self) {
            self.running.abort();
            if let Some(script) = self.script.take() {
                script.abort();
            }
            self.collecting.abort();
        }
    }

    fn session() -> HopSession {
        HopSession {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            secure_session_id: Uuid::from_u128(3),
            circuit_code: 42,
            first_name: "Hop".to_string(),
            last_name: "Test".to_string(),
            source: RegionEnd {
                server_uri: "http://127.0.0.1:8/".to_string(),
                region_id: Uuid::from_u128(9),
                name: "Home".to_string(),
                loc_x: 0,
                loc_y: 0,
            },
            size_x: 256,
            size_y: 256,
            viewer: "Ferroan Bot 0.1.0".to_string(),
            channel: "Ferroan Bot".to_string(),
            mac: "aabb".to_string(),
            id0: "ccdd".to_string(),
            animations: Default::default(),
            service_urls: vec![("HomeURI".into(), "/".into())],
            appearance: Some(crate::simulation_client::Appearance {
                serial: 1,
                height: 1.8,
                aphz: 0.0,
                has_texture: false,
                wearables: Vec::new(),
                te8: Vec::new(),
                textures: Vec::new(),
                baked: Vec::new(),
                visual_params: Vec::new(),
                attachments: Vec::new(),
            }),
        }
    }

    fn destination(addr: SocketAddr) -> HopDestination {
        HopDestination {
            end: RegionEnd {
                server_uri: format!("http://{addr}/"),
                region_id: Uuid::from_u128(7),
                name: "Neighbour".to_string(),
                loc_x: 256,
                loc_y: 256,
            },
            endpoint: "127.0.0.1:9100".parse().unwrap(),
            size_x: 256,
            size_y: 256,
        }
    }

    fn listed(addr: SocketAddr) -> Arc<dyn RegionDirectory> {
        listed_for(addr, HANDLE, 256)
    }

    fn listed_for(addr: SocketAddr, handle: u64, loc_y: u32) -> Arc<dyn RegionDirectory> {
        let mut dest = destination(addr);
        // Closing follows the destination origin, so a far request handle
        // needs an origin that is actually outside the view.
        dest.end.loc_x = (handle >> 32) as u32;
        dest.end.loc_y = loc_y;
        let mut regions = HashMap::new();
        regions.insert(handle, dest);
        Arc::new(Listed(regions))
    }

    struct HopChoice {
        presence: HopPresence,
        handle: u64,
        grid: Option<HopGrid>,
    }

    fn idle_choice() -> HopChoice {
        HopChoice {
            presence: HopPresence::idle(),
            handle: HANDLE,
            grid: None,
        }
    }

    async fn listen() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, addr)
    }

    fn enqueue_caps(
        caps: &Arc<Mutex<CapsServer>>,
    ) -> impl Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync + 'static {
        let caps = Arc::clone(caps);
        move |event: Llsd| caps.lock().unwrap().enqueue_event(event)
    }

    async fn open(
        directory: Arc<dyn RegionDirectory>,
        client: Option<SimulationClient>,
        listener: Option<TcpListener>,
        steps: Vec<(u16, String)>,
        hold_query: bool,
        enqueue: impl Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync + 'static,
    ) -> Opened {
        open_at(
            directory,
            client,
            listener,
            steps,
            hold_query,
            enqueue,
            idle_choice(),
        )
        .await
    }

    async fn open_at(
        directory: Arc<dyn RegionDirectory>,
        client: Option<SimulationClient>,
        listener: Option<TcpListener>,
        steps: Vec<(u16, String)>,
        hold_query: bool,
        enqueue: impl Fn(Llsd) -> Result<(), EventQueueError> + Send + Sync + 'static,
        choice: HopChoice,
    ) -> Opened {
        let release = hold_query.then(Notify::new).map(Arc::new);
        let hits = Arc::new(Mutex::new(Vec::new()));
        let script = match listener {
            Some(listener) => {
                Some(spawn_neighbour(listener, steps, Arc::clone(&hits), release.clone()).await)
            }
            None => None,
        };
        let caps_server = CapsServer::bind().await.unwrap();
        let events = caps_server.event_queue_url().to_owned();
        let caps = Arc::new(Mutex::new(caps_server));
        let hop = drive(
            session(),
            client,
            directory,
            choice.grid,
            choice.presence,
            enqueue,
        );
        let position = Arc::new(Mutex::new([128.0, 128.0, 25.0]));
        let region = ferroan_client::TeleportRegion {
            world_x: 0,
            world_y: 0,
            size_x: 256.0,
            size_y: 256.0,
            default_landing: [128.0, 128.0, 30.0],
            half_height: 0.9,
            allow_movement: true,
            godlike: false,
            position: Arc::clone(&position),
        };
        let expect = CircuitExpect {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
            region_id: Uuid::from_u128(9),
        };
        let arrival = ArrivalConfig {
            position: [128.0, 128.0, 25.0],
            look_at: [1.0, 0.0, 0.0],
            region_handle: 0,
            channel_version: "Ferroan hop test".to_string(),
            land: Vec::new(),
            overlay: Vec::new(),
            teleport_flags: 1 << 7,
            crossing_flags: 0,
            avatar: None,
            teleport: Some(region),
            source_hop: Some(hop),
            return_slot: None,
            updates: None,
        };
        let server = CircuitServer::bind().await.unwrap();
        let udp = server.local_addr().unwrap();
        let running =
            tokio::spawn(async move { server.serve_one_with_arrival(expect, arrival).await });
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        socket.connect(udp).await.unwrap();
        let gathered = Arc::new(Mutex::new(Vec::new()));
        let reading = Arc::clone(&socket);
        let store = Arc::clone(&gathered);
        let collecting = tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            while let Ok(len) = reading.recv(&mut buf).await {
                store.lock().unwrap().push(buf[..len].to_vec());
            }
        });
        send_view(&socket, expect, 1, login_packet(expect)).await;
        send_view(
            &socket,
            expect,
            2,
            teleport_packet(expect, choice.handle, POSITION),
        )
        .await;
        Opened {
            running,
            script,
            collecting,
            hits,
            gathered,
            socket,
            position,
            release,
            events,
            caps,
        }
    }

    async fn spawn_neighbour(
        listener: TcpListener,
        steps: Vec<(u16, String)>,
        hits: Arc<Mutex<Vec<Hit>>>,
        release: Option<Arc<Notify>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            for (index, (status, text)) in steps.into_iter().enumerate() {
                let accepted =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept()).await;
                let Ok(Ok((mut stream, _))) = accepted else {
                    break;
                };
                let (header, body) = read_http(&mut stream).await;
                hits.lock().unwrap().push(Hit { header, body });
                if index == 0
                    && let Some(release) = &release
                {
                    let wait = release.notified();
                    tokio::pin!(wait);
                    if tokio::time::timeout(Duration::from_secs(5), wait)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                let raw = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = stream.write_all(raw.as_bytes()).await;
            }
        })
    }

    async fn read_http(stream: &mut TcpStream) -> (String, Vec<u8>) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let count = stream.read(&mut tmp).await.unwrap_or(0);
            if count == 0 {
                break 0;
            }
            buf.extend_from_slice(&tmp[..count]);
            if let Some(position) = buf.windows(4).position(|part| part == b"\r\n\r\n") {
                break position;
            }
            if buf.len() > 1024 * 1024 {
                break buf.len();
            }
        };
        let header = String::from_utf8_lossy(&buf[..header_end.min(buf.len())]).into_owned();
        let mut body = if header_end + 4 <= buf.len() {
            buf[header_end + 4..].to_vec()
        } else {
            Vec::new()
        };
        let length = content_length(&header);
        while body.len() < length {
            let count = stream.read(&mut tmp).await.unwrap_or(0);
            if count == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..count]);
        }
        if body.len() > length {
            body.truncate(length);
        }
        (header, body)
    }

    fn content_length(header: &str) -> usize {
        header
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    fn login_packet(expect: CircuitExpect) -> UseCircuitCode {
        let mut message = UseCircuitCode::default();
        message.circuit_code.id = expect.agent_id;
        message.circuit_code.session_id = expect.session_id;
        message.circuit_code.code = expect.circuit_code;
        message
    }

    fn teleport_packet(
        expect: CircuitExpect,
        handle: u64,
        position: [f32; 3],
    ) -> TeleportLocationRequest {
        let mut message = TeleportLocationRequest::default();
        message.agent_data.agent_id = expect.agent_id;
        message.agent_data.session_id = expect.session_id;
        message.info.region_handle = handle;
        message.info.position = position.into();
        message.info.look_at = [1.0, 0.0, 0.0].into();
        message
    }

    async fn send_view<M: ferroan_protocol::Msg>(
        socket: &UdpSocket,
        _expect: CircuitExpect,
        sequence: u32,
        message: M,
    ) {
        let mut bytes = Vec::new();
        build_packet(&message, sequence, true, false, &[], &mut bytes);
        socket.send(&bytes).await.unwrap();
    }

    async fn until(
        gathered: &Mutex<Vec<Vec<u8>>>,
        mut ready: impl FnMut(&[Vec<u8>]) -> bool,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if ready(&gathered.lock().unwrap()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn reasons(gathered: &[Vec<u8>]) -> Vec<String> {
        gathered
            .iter()
            .filter_map(|bytes| {
                let packet = parse_packet(bytes).ok()?;
                if packet.id != TeleportFailed::ID {
                    return None;
                }
                let message = packet.decode::<TeleportFailed>().ok()?;
                Some(ferroan_protocol::field_str(&message.info.reason))
            })
            .collect()
    }

    fn has_id(gathered: &[Vec<u8>], id: ferroan_protocol::MsgId) -> bool {
        gathered
            .iter()
            .any(|bytes| parse_packet(bytes).is_ok_and(|packet| packet.id == id))
    }

    fn request_parts(header: &str) -> (String, String) {
        let mut parts = header.lines().next().unwrap_or("").split_whitespace();
        (
            parts.next().unwrap_or("").to_string(),
            parts.next().unwrap_or("").to_string(),
        )
    }

    fn plain(hit: &Hit) -> Vec<u8> {
        let gzip = hit
            .header
            .to_ascii_lowercase()
            .contains("x-content-encoding: gzip");
        if !gzip {
            return hit.body.clone();
        }
        let mut decoder = GzDecoder::new(hit.body.as_slice());
        let mut plain = Vec::new();
        decoder.read_to_end(&mut plain).unwrap();
        plain
    }

    async fn finish_script(opened: &mut Opened) -> Vec<Hit> {
        let Some(mut handle) = opened.script.take() else {
            return Vec::new();
        };
        match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
            Ok(Ok(())) => std::mem::take(&mut *opened.hits.lock().unwrap()),
            _ => {
                handle.abort();
                panic!("neighbour script did not finish");
            }
        }
    }

    async fn poll_finish(opened: &Opened) -> Llsd {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let response = http
            .post(&opened.events)
            .body(Vec::new())
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let bytes = response.bytes().await.unwrap();
        ferroan_llsd::from_xml(&bytes).unwrap()
    }

    fn client() -> Option<SimulationClient> {
        Some(SimulationClient::new().unwrap())
    }

    #[test]
    fn departure_copies_the_run_flag_and_current_animation_set() {
        let session = session();
        let extra = crate::simulation_client::Anim {
            animation: Uuid::from_u128(123),
            object_id: Uuid::from_u128(8),
            sequence: 3,
        };
        session.animations.lock().unwrap().extra.push(extra);
        let request = ferroan_client::HopRequest {
            agent_id: session.agent_id,
            session_id: session.session_id,
            region_handle: HANDLE,
            position: [128.0, 128.0, 30.0],
            look_at: [1.0, 0.0, 0.0],
            flags: ferroan_client::VIA_LOCATION,
            client_ip: "127.0.0.1".into(),
            pose: ferroan_client::HopPose {
                always_run: true,
                ..Default::default()
            },
        };
        let data = super::agent_update(&session, &request, request.position, None);
        assert!(data.always_run);
        assert_eq!(data.animations, [extra]);
    }

    #[tokio::test]
    async fn a_successful_hop_sends_start_then_the_finish_event() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let events = caps.lock().unwrap().event_queue_url().to_owned();
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "true".to_string()),
            ],
            false,
            enqueue_caps(&caps),
        )
        .await;
        opened.events = events;
        opened.caps = caps;
        let hits = finish_script(&mut opened).await;
        assert!(
            until(&opened.gathered, |packets| has_id(
                packets,
                TeleportStart::ID
            ))
            .await,
            "TeleportStart did not arrive"
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(!has_id(&packets, TeleportFinish::ID));
        assert!(!has_id(&packets, TeleportLocal::ID));
        assert!(!has_id(&packets, MapBlockReply::ID));
        assert!(reasons(&packets).is_empty());
        let start = packets.iter().find_map(|bytes| {
            let packet = parse_packet(bytes).ok()?;
            if packet.id != TeleportStart::ID {
                return None;
            }
            Some(packet)
        });
        let start = start.unwrap();
        assert!(start.reliable());
        assert_eq!(start.flags & ferroan_protocol::PacketFlags::ZEROCODED, 0);
        let message = start.decode::<TeleportStart>().unwrap();
        assert_eq!(message.info.teleport_flags, 16);
        assert_eq!(hits.len(), 3);
        let (method, _) = request_parts(&hits[0].header);
        assert_eq!(method, "QUERYACCESS");
        let query: Value = serde_json::from_slice(&hits[0].body).unwrap();
        assert_eq!(query["agent_home_uri"], "/");
        assert_eq!(query["viaTeleport"], true);
        assert_eq!(query["position"], "<128, 128, 30>");
        let (method, _) = request_parts(&hits[1].header);
        assert_eq!(method, "POST");
        let create: Value = serde_json::from_slice(&plain(&hits[1])).unwrap();
        assert_eq!(create["serviceurls"]["HomeURI"], "/");
        assert_eq!(create["child"], true);
        assert_eq!(create["teleport_flags"], "16");
        assert_eq!(create["start_pos"], "<128, 128, 30>");
        assert_eq!(create["client_ip"], "127.0.0.1");
        assert_eq!(create["viewer"], "Ferroan Bot 0.1.0");
        assert_eq!(create["channel"], "Ferroan Bot");
        assert_eq!(create["mac"], "aabb");
        assert_eq!(create["id0"], "ccdd");
        assert_eq!(
            create["packed_appearance"]["height"].as_f64(),
            Some(f64::from(1.8f32))
        );
        assert!(create["packed_appearance"].get("wearables").is_none());
        let (method, _) = request_parts(&hits[2].header);
        assert_eq!(method, "PUT");
        let update: Value = serde_json::from_slice(&plain(&hits[2])).unwrap();
        assert_eq!(update["message_type"], "AgentData");
        assert_eq!(update["wait_for_root"], true);
        assert_eq!(update["at_axis"], "<1, 0, 0>");
        assert_eq!(update["left_axis"], "<-0, 1, 0>");
        assert_eq!(update["up_axis"], "<0, 0, 1>");
        assert_eq!(update["center"], "<0, 0, 0>");
        assert_eq!(update["head_rotation"], "<0, 0, 0, 1>");
        assert_eq!(update["body_rotation"], "<0, 0, 0, 1>");
        assert_eq!(update["region_id"], Uuid::from_u128(9).to_string());
        assert_ne!(update["region_id"], Uuid::from_u128(7).to_string());
        assert_eq!(update["far"].as_f64(), Some(255.0));
        assert_eq!(update["god_data"], serde_json::json!({}));
        assert!(update.get("god_level").is_none());
        assert!(update.get("active_group_id").is_none());
        assert!(update.get("active_group_name").is_none());
        assert!(update.get("active_group_title").is_none());
        let stand =
            serde_json::json!({"animation":"2408fe9e-df1d-1d7d-f4ff-1384fa7b350f", "seq_num":1});
        assert_eq!(update["default_animation"], stand);
        assert_eq!(update["animation_state"], stand);
        assert_eq!(update["motion_state"], 6);
        assert!(update.get("cb_uri").is_none());
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
        let event = poll_finish(&opened).await;
        let info = event.get("events").at(0);
        assert_eq!(info.get("message").as_str(), "TeleportFinish");
        let body = info.get("body").get("Info").at(0);
        assert!(matches!(body.get("LocationID"), Llsd::Binary(bytes) if bytes.len() == 4));
        assert!(matches!(body.get("TeleportFlags"), Llsd::Binary(bytes) if bytes.len() == 4));
        assert!(matches!(body.get("RegionSizeX"), Llsd::Binary(bytes) if bytes.len() == 4));
        assert!(matches!(body.get("RegionSizeY"), Llsd::Binary(bytes) if bytes.len() == 4));
        assert!(matches!(body.get("SimPort"), Llsd::Integer(_)));
        assert!(matches!(body.get("SimAccess"), Llsd::Integer(_)));
        assert_eq!(body.get("LocationID").as_u32(), 4);
        assert_eq!(body.get("SimAccess").as_i32(), 13);
        assert_eq!(body.get("TeleportFlags").as_u32(), 16);
        assert_eq!(body.get("RegionSizeX").as_u32(), 256);
        assert_eq!(body.get("RegionSizeY").as_u32(), 256);
        assert_eq!(body.get("RegionHandle").as_u64(), (256_u64 << 32) | 256);
        assert_eq!(body.get("SimIP").as_binary(), [127, 0, 0, 1]);
        assert_eq!(body.get("SimPort").as_i32(), 9100);
        assert!(body.get("SeedCapability").as_str().contains("/CAPS/"));
        assert!(body.get("SeedCapability").as_str().ends_with("0000/"));
    }

    #[tokio::test]
    async fn an_in_view_hop_reuses_the_admission_seed() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let events = caps.lock().unwrap().event_queue_url().to_owned();
        let book = Arc::new(crate::children::ChildBook::new());
        let agent = Uuid::from_u128(1);
        let generation = Uuid::from_u128(11);
        book.attach(agent, generation, Uuid::from_u128(2));
        let claimed = book.claim(
            agent,
            generation,
            RegionEnd {
                server_uri: format!("http://{addr}/"),
                region_id: Uuid::from_u128(7),
                name: "Neighbour".to_string(),
                loc_x: 256,
                loc_y: 256,
            },
        );
        let crate::children::Claim::New { seed } = claimed else {
            panic!("admission did not own the destination");
        };
        let mut opened = open_at(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "true".to_string()),
            ],
            false,
            enqueue_caps(&caps),
            HopChoice {
                presence: HopPresence::Seeded(Arc::clone(&book)),
                handle: HANDLE,
                grid: None,
            },
        )
        .await;
        opened.events = events;
        opened.caps = caps;
        let hits = finish_script(&mut opened).await;
        assert_eq!(hits.len(), 3);
        let create: Value = serde_json::from_slice(&plain(&hits[1])).unwrap();
        assert_eq!(create["child"], true);
        assert_eq!(create["teleport_flags"], "16");
        assert_eq!(create["caps_path"], seed);
        let event = poll_finish(&opened).await;
        let info = event.get("events").at(0);
        assert_eq!(info.get("message").as_str(), "TeleportFinish");
        assert_eq!(
            info.get("body")
                .get("Info")
                .at(0)
                .get("SeedCapability")
                .as_str(),
            ferroan_client::caps_seed(&format!("http://{addr}/"), &seed)
        );
    }

    #[test]
    fn a_flying_departure_sends_the_fly_control() {
        assert_eq!(super::fly_control(ferroan_client::VIA_LOCATION), 0);
        assert_eq!(
            super::fly_control(ferroan_client::VIA_LOCATION | ferroan_client::IS_FLYING),
            super::CONTROL_FLY
        );
    }

    #[tokio::test]
    async fn query_refusal_stays_and_does_not_start() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![(
                200,
                "{\"success\":false,\"reason\":\"no access\"}".to_string(),
            )],
            false,
            enqueue_caps(&caps),
        )
        .await;
        let hits = finish_script(&mut opened).await;
        assert!(
            until(&opened.gathered, |packets| reasons(packets)
                .iter()
                .any(|reason| reason == "no access"))
            .await
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(!has_id(&packets, TeleportStart::ID));
        assert!(!has_id(&packets, MapBlockReply::ID));
        assert_eq!(hits.len(), 1);
        assert_eq!(request_parts(&hits[0].header).0, "QUERYACCESS");
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
    }

    #[tokio::test]
    async fn create_refusal_sends_the_connector_reason_and_does_not_close() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, "{\"success\":false,\"reason\":\"full\"}".to_string()),
            ],
            false,
            enqueue_caps(&caps),
        )
        .await;
        let hits = finish_script(&mut opened).await;
        assert!(
            until(&opened.gathered, |packets| reasons(packets)
                .iter()
                .any(|reason| reason == "full"))
            .await
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(has_id(&packets, TeleportStart::ID));
        assert!(!has_id(&packets, MapBlockReply::ID));
        assert_eq!(hits.len(), 2);
        assert_eq!(request_parts(&hits[1].header).0, "POST");
        assert!(
            hits.iter()
                .all(|hit| request_parts(&hit.header).0 != "DELETE")
        );
    }

    #[tokio::test]
    async fn update_refusal_closes_and_sends_the_connection_sentence() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "false".to_string()),
                (200, "closed".to_string()),
            ],
            false,
            enqueue_caps(&caps),
        )
        .await;
        let hits = finish_script(&mut opened).await;
        let expected = connect_reason("Neighbour");
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| reason == &expected)
            })
            .await
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(has_id(&packets, TeleportStart::ID));
        assert!(
            reasons(&packets)
                .iter()
                .any(|reason| reason.contains(CONNECT_INNER))
        );
        assert_eq!(hits.len(), 4);
        let (method, target) = request_parts(&hits[3].header);
        assert_eq!(method, "DELETE");
        assert!(target.contains("auth=00000000-0000-0000-0000-000000000002"));
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
    }

    #[tokio::test]
    async fn an_enqueue_failure_closes_without_update() {
        let (listener, addr) = listen().await;
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "closed".to_string()),
            ],
            false,
            |_event| Err(EventQueueError::Closed),
        )
        .await;
        let hits = finish_script(&mut opened).await;
        let expected = connect_reason("Neighbour");
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| reason == &expected)
            })
            .await
        );
        assert_eq!(hits.len(), 3);
        assert_eq!(request_parts(&hits[2].header).0, "DELETE");
        assert!(hits.iter().all(|hit| request_parts(&hit.header).0 != "PUT"));
    }

    #[tokio::test]
    async fn an_unknown_region_sends_not_found_and_one_offline_map_block() {
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let opened = open(
            Arc::new(EmptyDirectory),
            None,
            None,
            Vec::new(),
            false,
            enqueue_caps(&caps),
        )
        .await;
        assert!(
            until(&opened.gathered, |packets| has_id(
                packets,
                MapBlockReply::ID
            ))
            .await,
            "map block did not arrive"
        );
        let packets = opened.gathered.lock().unwrap().clone();
        let found = reasons(&packets);
        assert!(!found.is_empty());
        assert!(found.iter().all(|reason| reason == NOT_FOUND));
        assert!(!has_id(&packets, TeleportStart::ID));
        assert!(!has_id(&packets, TeleportFinish::ID));
        let maps: Vec<_> = packets
            .iter()
            .filter_map(|bytes| {
                let packet = parse_packet(bytes).ok()?;
                if packet.id != MapBlockReply::ID {
                    return None;
                }
                Some(packet.decode::<MapBlockReply>().unwrap())
            })
            .collect();
        assert!(!maps.is_empty());
        assert_eq!(maps[0].agent_data.flags, 0);
        assert_eq!(maps[0].agent_data.agent_id, Uuid::from_u128(1));
        assert_eq!(maps[0].data.len(), 1);
        assert_eq!(maps[0].data[0].x, 1);
        assert_eq!(maps[0].data[0].y, 0);
        assert_eq!(maps[0].data[0].access, 254);
        assert!(maps[0].data[0].name.is_empty());
        assert!(maps[0].size.is_empty());
        assert_eq!(maps[0].omitted_trailing_blocks, 0);
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
    }

    #[tokio::test]
    async fn a_second_request_during_query_is_refused_and_does_not_start() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let mut opened = open(
            listed(addr),
            client(),
            Some(listener),
            vec![(
                200,
                "{\"success\":false,\"reason\":\"no access\"}".to_string(),
            )],
            true,
            enqueue_caps(&caps),
        )
        .await;
        let deadline = Instant::now() + Duration::from_secs(3);
        while opened.hits.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "query did not arrive");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let expect = CircuitExpect {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 42,
            region_id: Uuid::from_u128(9),
        };
        send_view(
            &opened.socket,
            expect,
            3,
            teleport_packet(expect, HANDLE, POSITION),
        )
        .await;
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| {
                    reason == "Previous teleport process incomplete.  Please retry shortly."
                })
            })
            .await,
            "in-transit refusal did not arrive"
        );
        assert_eq!(opened.hits.lock().unwrap().len(), 1);
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(!has_id(&packets, TeleportStart::ID));
        opened.release.take().unwrap().notify_one();
        let hits = finish_script(&mut opened).await;
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| reason == "no access")
            })
            .await
        );
        let packets = opened.gathered.lock().unwrap().clone();
        assert!(!has_id(&packets, TeleportStart::ID));
        assert_eq!(hits.len(), 1);
        assert_eq!(*opened.position.lock().unwrap(), [128.0, 128.0, 25.0]);
    }

    #[test]
    fn a_missing_ipv4_endpoint_does_not_ask_the_neighbour() {
        let endpoint: SocketAddr = "127.0.0.1:0".parse().unwrap();
        assert!(viewer_endpoint(endpoint).is_none());
        let endpoint: SocketAddr = "[::1]:9000".parse().unwrap();
        assert!(viewer_endpoint(endpoint).is_none());
    }

    #[test]
    fn an_adjacent_region_stays_in_view_and_two_cells_do_not() {
        assert!(!outside_view((0, 0), (1, 0), (256, 256), (256, 256)));
        assert!(!outside_view((0, 0), (1, 1), (256, 256), (256, 256)));
        assert!(outside_view((0, 0), (3, 0), (256, 256), (256, 256)));
    }

    fn watched(budget: Duration, handle: u64) -> (Arc<PresenceWatch>, HopChoice) {
        let watch = PresenceWatch::new(budget);
        let choice = HopChoice {
            presence: HopPresence::watch(Arc::clone(&watch)),
            handle,
            grid: None,
        };
        (watch, choice)
    }

    fn no_kick(packets: &[Vec<u8>]) {
        assert!(!has_id(packets, KickUser::ID));
        assert!(!has_id(packets, DisableSimulator::ID));
    }

    #[tokio::test]
    async fn an_in_view_hop_demotes_and_keeps_the_circuit() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let (watch, choice) = watched(Duration::from_secs(2), HANDLE);
        let mut opened = open_at(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "true".to_string()),
            ],
            false,
            enqueue_caps(&caps),
            choice,
        )
        .await;
        let hits = finish_script(&mut opened).await;
        assert_eq!(hits.len(), 3);
        let update: Value = serde_json::from_slice(&plain(&hits[2])).unwrap();
        assert!(update.get("cb_uri").is_none());
        assert!(watch.demoted());
        assert!(!watch.restored());
        assert!(!watch.armed());
        assert!(!watch.closed());
        // The kills follow the update's answer.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while watch.moved().is_none() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            watch.moved(),
            Some(true),
            "a near hop kills after it arrives"
        );
        assert!(!opened.running.is_finished());
        no_kick(&opened.gathered.lock().unwrap());
    }

    #[tokio::test]
    async fn an_update_failure_restores_the_root() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let (watch, choice) = watched(Duration::from_secs(2), HANDLE);
        let mut opened = open_at(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "false".to_string()),
                (200, "closed".to_string()),
            ],
            false,
            enqueue_caps(&caps),
            choice,
        )
        .await;
        let hits = finish_script(&mut opened).await;
        let expected = connect_reason("Neighbour");
        assert!(
            until(&opened.gathered, |packets| {
                reasons(packets).iter().any(|reason| reason == &expected)
            })
            .await
        );
        assert_eq!(request_parts(&hits[3].header).0, "DELETE");
        assert!(watch.demoted());
        assert!(watch.restored());
        assert!(!watch.armed());
        assert!(!watch.closed());
        assert_eq!(watch.moved(), None, "a failed update kills nothing");
        assert!(!opened.running.is_finished());
    }

    #[tokio::test]
    async fn an_enqueue_failure_does_not_demote() {
        let (listener, addr) = listen().await;
        let (watch, choice) = watched(Duration::from_secs(2), HANDLE);
        let mut opened = open_at(
            listed(addr),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "closed".to_string()),
            ],
            false,
            |_event| Err(EventQueueError::Closed),
            choice,
        )
        .await;
        let _hits = finish_script(&mut opened).await;
        assert!(!watch.demoted());
        assert!(!watch.closed());
        assert!(!opened.running.is_finished());
    }

    const FAR_HANDLE: u64 = 3 << 40;

    #[tokio::test]
    async fn a_far_hop_sends_the_callback_and_closes_when_released() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let (watch, choice) = watched(Duration::from_secs(5), FAR_HANDLE);
        let mut opened = open_at(
            listed_for(addr, FAR_HANDLE, 256),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "true".to_string()),
            ],
            false,
            enqueue_caps(&caps),
            choice,
        )
        .await;
        let hits = finish_script(&mut opened).await;
        watch.release();
        let ended = tokio::time::timeout(Duration::from_secs(2), &mut opened.running).await;
        assert!(ended.is_ok(), "circuit did not close after release");
        assert!(watch.woke());
        assert!(watch.demoted());
        assert!(!watch.restored());
        assert!(watch.armed());
        assert!(watch.closed());
        assert_eq!(watch.moved(), Some(false));
        assert_eq!(hits.len(), 3);
        assert!(
            hits.iter()
                .all(|hit| request_parts(&hit.header).0 != "DELETE")
        );
        let update: Value = serde_json::from_slice(&plain(&hits[2])).unwrap();
        let callback = update["cb_uri"].as_str().unwrap();
        assert!(callback.starts_with("http://127.0.0.1:8/agent/"));
        assert!(callback.ends_with("/release/"));
        assert!(callback.contains(&Uuid::from_u128(1).to_string()));
        assert!(callback.contains(&Uuid::from_u128(9).to_string()));
        no_kick(&opened.gathered.lock().unwrap());
    }

    #[tokio::test]
    async fn a_region_at_grid_y_zero_closes_when_the_release_does_not_arrive() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let (watch, choice) = watched(Duration::from_millis(40), HANDLE);
        let mut opened = open_at(
            listed_for(addr, HANDLE, 0),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "true".to_string()),
            ],
            false,
            enqueue_caps(&caps),
            choice,
        )
        .await;
        let hits = finish_script(&mut opened).await;
        let ended = tokio::time::timeout(Duration::from_secs(2), &mut opened.running).await;
        assert!(
            ended.is_ok(),
            "circuit did not close after the release budget"
        );
        assert!(!watch.woke());
        assert!(watch.closed());
        assert!(watch.armed());
        assert_eq!(hits.len(), 3);
        assert!(
            hits.iter()
                .all(|hit| request_parts(&hit.header).0 != "DELETE")
        );
        let update: Value = serde_json::from_slice(&plain(&hits[2])).unwrap();
        assert!(update["cb_uri"].as_str().unwrap().ends_with("/release/"));
        no_kick(&opened.gathered.lock().unwrap());
    }

    #[tokio::test]
    async fn a_grid_lookup_runs_the_in_view_hop() {
        let (listener, addr) = listen().await;
        let caps = Arc::new(Mutex::new(CapsServer::bind().await.unwrap()));
        let region_id = Uuid::from_u128(7);
        let mut region = ferroan_http::FieldMap::new();
        for (key, value) in [
            ("uuid", region_id.to_string()),
            ("locX", "256".to_string()),
            ("locY", "256".to_string()),
            ("sizeX", "256".to_string()),
            ("sizeY", "256".to_string()),
            ("regionName", "Neighbour".to_string()),
            ("serverIP", "127.0.0.1".to_string()),
            ("serverPort", "9100".to_string()),
            ("serverURI", format!("http://{addr}")),
        ] {
            region.insert(key.to_owned(), ferroan_http::Field::Str(value));
        }
        let mut root = ferroan_http::FieldMap::new();
        root.insert("result".to_owned(), ferroan_http::Field::Map(region));
        let xml = ferroan_http::build_xml_response(&root).unwrap();
        let app = axum::Router::new().fallback(move |body: String| {
            assert!(body.contains("METHOD=get_region_by_position"), "{body}");
            assert!(body.contains("X=256"), "{body}");
            assert!(body.contains("Y=0"), "{body}");
            let xml = xml.clone();
            async move {
                (
                    axum::http::StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "application/xml")],
                    xml,
                )
            }
        });
        let grid_socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let grid_base = format!("http://{}", grid_socket.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(grid_socket, app).await.unwrap() });
        let (watch, mut choice) = watched(Duration::from_secs(2), HANDLE);
        choice.grid = Some(HopGrid {
            client: crate::grid::GridClient::from_base(&grid_base),
            scope: Uuid::from_u128(3),
        });
        let mut opened = open_at(
            Arc::new(EmptyDirectory),
            client(),
            Some(listener),
            vec![
                (200, QUERY_OK.to_string()),
                (200, CREATE_OK.to_string()),
                (200, "true".to_string()),
            ],
            false,
            enqueue_caps(&caps),
            choice,
        )
        .await;
        let hits = finish_script(&mut opened).await;
        assert_eq!(hits.len(), 3);
        let update: Value = serde_json::from_slice(&plain(&hits[2])).unwrap();
        assert!(update.get("cb_uri").is_none());
        assert_eq!(update["position"], "<128, 128, 30>");
        assert!(watch.demoted());
        assert!(!watch.closed());
        assert!(!opened.running.is_finished());
        serving.abort();
    }
}
