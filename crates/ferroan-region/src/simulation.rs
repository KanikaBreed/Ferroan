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
// JsonWriter.cs: The authors disclaim copyright to this source code. For more
// details, see the COPYING file included with this distribution.
//
// Ported to Rust for Ferroan from Source/OpenSim.Server.Handlers/Simulation/
//   AgentHandlers.cs and Utils.cs, Source/OpenSim.Framework/AgentCircuitData.cs,
//   EntityTransferContext.cs, VersionInfo.cs and CapsUtil.cs, and
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Simulation/LocalSimulationConnector.cs
//   and Source/OpenSim.Region.Framework/Scenes/Scene.cs (NewUserConnection)
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
//   OpenMetaverse.StructuredData/StructuredData.cs (OSD boolean/real conversions)
//   and OpenMetaverse.StructuredData/JSON/JsonWriter.cs (real-number formatting)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: bounded local login admission, concurrent root circuits, gzip limits,
// a single CreateAgent budget under Robust's 30 s connector timeout, and an
// empty JSON 403 for a refused control-plane POST. The release DELETE is
// AgentHandlers.DoAgentDelete when the action is "release", pin c401bceecd.
// Circuit service URLs follow AgentCircuitData.UnpackAgentCircuitData at Pin8,
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// At pin 1d7abb8337,
// Scene.CheckLandPositionAccess admits a god or an estate manager before the
// root-agent count. Those extra roots share region.root-bypass.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The simulation service calls made by C# Robust during a local grid login.
//! The source hop's release DELETE is implemented. A child `CreateAgent` that
//! is not a login, and `UpdateAgent`, are the destination half of a teleport.
//! `DELETE /agent/{id}/{region}/` closes that agent when the auth query is its
//! session id.

use std::{
    collections::HashMap,
    io::Read,
    net::SocketAddr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::config::RegionConfig;
use crate::control_plane::ControlPlaneAccess;
use crate::grid::PresenceClient;
use crate::simulation_client::{
    Appearance, ChildIdentity, RegionEnd, agent_animations, appearance_from_json,
};
use crate::staff::{ROOT_BYPASS_LIMIT, StaffGate};
use crate::status::RegionMeters;
use axum::{
    Router,
    body::to_bytes,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use ferroan_client::{
    CapsError, CapsRegistry, CircuitExpect, CircuitServer, CircuitStop, LoginReservation,
    ROOT_WAIT, ReturnPose, ReturnSlot,
};
use ferroan_diag::{self, HTTP_POST_BODY, Kind, REGION_CHILD_ADMISSIONS, ROOT_AGENTS, ROOT_BYPASS};
use flate2::read::GzDecoder;
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use uuid::Uuid;

const MAX_BODY: usize = 1024 * 1024;
const MAX_FEATURES: usize = 256;
const MIN_VERSION: f32 = 0.3;
const MAX_VERSION: f32 = 0.8;
const VIA_LOGIN: u32 = 1 << 7;
/// `RegionSettings.AgentLimit` at the pin. Not a configuration key.
pub const ROOT_AGENT_LIMIT: usize = 40;
/// Dispatch places reserved for root admissions, including the staff and god bypass.
pub const ADMISSION_QUEUE: usize = ROOT_AGENT_LIMIT + ROOT_BYPASS_LIMIT;
const _: () = assert!(ferroan_client::PENDING_LOGINS == ADMISSION_QUEUE);
/// One region's worth of circuits and one region's worth of overlapping admissions.
pub const DISPATCH_CAPACITY: usize = ADMISSION_QUEUE * 2;
const CHILD_DISPATCH_CAPACITY: usize = DISPATCH_CAPACITY - ADMISSION_QUEUE;
const ADMISSION_SECRET_SLOTS: usize = DISPATCH_CAPACITY * 4;
const _: () = assert!(
    ferroan_log::MAX_SECRETS == ADMISSION_SECRET_SLOTS,
    "log.redact-secrets does not cover the root and bypass limits"
);

#[derive(Debug, thiserror::Error)]
enum SimulationError {
    #[error("invalid simulation JSON")]
    Json,
    #[error("invalid compressed simulation body")]
    Compression,
    #[error("simulation body exceeds its limit")]
    BodyLimit,
    #[error("invalid simulation field: {0}")]
    Field(&'static str),
    #[error("{0}")]
    Refused(String),
}

/// The full child share can queue before a root arrives. Dispatch reservations
/// bound queued and running work together.
pub fn dispatch_queue() -> (mpsc::Sender<AcceptedAgent>, mpsc::Receiver<AcceptedAgent>) {
    mpsc::channel(DISPATCH_CAPACITY)
}

/// A slot covers CreateAgent verification, its queued accept and the circuit through final
/// cleanup. Generations of one agent share a lifecycle lock across all three stages.
#[derive(Default)]
struct DispatchSlots {
    total: usize,
    agents: HashMap<Uuid, (usize, Arc<AgentDispatch>)>,
}

#[derive(Default)]
struct AgentDispatch {
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    // A far hop removes the session row before its circuit finishes.
    // Keep the completion signal as long as any generation holds a slot.
    tail: Mutex<Option<watch::Receiver<()>>>,
}

struct DispatchSlot {
    slots: Arc<Mutex<DispatchSlots>>,
    agent: Uuid,
    lane: Arc<AgentDispatch>,
}

impl DispatchSlot {
    fn reserve(slots: &Arc<Mutex<DispatchSlots>>, agent: Uuid) -> Result<Self, &'static str> {
        let mut state = slots.lock().unwrap_or_else(|poison| poison.into_inner());
        let overlaps = state.total - state.agents.len();
        let limit = if state.agents.contains_key(&agent) && overlaps >= ADMISSION_QUEUE {
            Some((
                ferroan_diag::REGION_OVERLAPPING_LOGINS,
                ADMISSION_QUEUE,
                overlaps,
            ))
        } else if state.total >= DISPATCH_CAPACITY {
            Some((
                ferroan_diag::REGION_DISPATCH,
                DISPATCH_CAPACITY,
                state.total,
            ))
        } else {
            None
        };
        if let Some((id, cap, used)) = limit {
            ferroan_diag::refuse(id, Kind::Hard, cap as u64, (used + 1) as u64, "/agent");
            return Err("Region is busy completing a login or logout");
        }
        state.total += 1;
        let (used, lane) = state.agents.entry(agent).or_default();
        *used += 1;
        Ok(Self {
            slots: Arc::clone(slots),
            agent,
            lane: Arc::clone(lane),
        })
    }
}

impl Drop for DispatchSlot {
    fn drop(&mut self) {
        let mut state = self
            .slots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.total -= 1;
        if let Some((used, _)) = state.agents.get_mut(&self.agent) {
            *used -= 1;
            if *used == 0 {
                state.agents.remove(&self.agent);
            }
        }
    }
}

#[derive(Clone)]
pub struct SimulationState {
    region_id: Uuid,
    caps: CapsRegistry,
    sender: mpsc::Sender<AcceptedAgent>,
    sessions: Arc<Mutex<HashMap<Uuid, ActiveSession>>>,
    region_name: String,
    region_size: [u32; 2],
    presence: PresenceClient,
    control_plane: ControlPlaneAccess,
    dispatch_slots: Arc<Mutex<DispatchSlots>>,
    child_places: Arc<Semaphore>,
    admission_open: Arc<AtomicBool>,
    /// One envelope for the account lookup, lifecycle lock and Presence verification.
    /// Robust's CreateAgent connector gives up at 30 s; this stays under it.
    /// Tests shorten it. It is not a configuration key.
    create_agent_budget: Duration,
    /// Admitted root sessions. A same-agent replacement does not take another
    /// place. Tests lower it. It is not a configuration key.
    root_limit: usize,
    /// Extra roots past `root_limit` for estate staff and grid gods.
    /// Tests lower it. It is not a configuration key.
    bypass_limit: usize,
    staff: StaffGate,
    meters: Arc<RegionMeters>,
    /// Far-hop release waits, keyed by agent. Shared by every clone.
    releases: Arc<Mutex<HashMap<Uuid, ReleaseSlot>>>,
    /// Prim groups loaded at startup. Shared by every clone. Not sent on the circuit.
    objects: Arc<Vec<ferroan_scene::ObjectGroup>>,
    /// The frame, when this process has one. Child hops skip its physics tick.
    scene: Option<Arc<ferroan_scene::TerrainFrame>>,
    /// The UDP listener, once `serve` has bound it. Accept reserves the
    /// circuit code there, before the group and avatar lookups.
    circuit: Arc<OnceLock<CircuitServer>>,
    /// `LoginConnectWaitSeconds`. A reserved pending login lasts until the
    /// session opens or this long after accept. Zero means no connect deadline.
    login_connect_wait: Duration,
    /// Child destinations and seeds for each root circuit generation.
    children: Arc<crate::children::ChildBook>,
    /// Root identities the child driver copies onto neighbour circuits.
    /// A visibility child does not replace one. The matching generation's
    /// lease removes it. A teleport arrives as a child, so the identity is
    /// stored at accept when nothing is held yet, and again when this
    /// generation becomes root.
    roots: Arc<Mutex<HashMap<Uuid, ChildIdentity>>>,
    /// One identity per circuit generation, kept until that lease drops.
    /// `mark_root` publishes it even when an older root identity was held
    /// at accept.
    prepared: Arc<Mutex<HashMap<Uuid, ChildIdentity>>>,
    /// Admission HTTP started by the child drive. Aborting the drive task
    /// does not stop these. Serve end and process shutdown abort them, then
    /// take whatever close the guard staged.
    admissions: Arc<Mutex<HashMap<Uuid, Vec<tokio::task::JoinHandle<()>>>>>,
}

struct ActiveSession {
    session_id: Uuid,
    generation: Uuid,
    /// Set when this circuit's root arrival is accepted, before the root reports.
    root: bool,
    /// Shared with the lease so a superseded child holds its place through cleanup.
    child: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
    /// Set when a far hop closes the agent here. Logout must not tell the grid.
    departed: Arc<AtomicBool>,
    /// Set when this generation is removed while it is still the root.
    rooted_when_replaced: Arc<AtomicBool>,
    cancel: Option<oneshot::Sender<CircuitStop>>,
    /// Cloned by a successor or a failed verification that must wait for logout.
    logout_done: watch::Receiver<()>,
    /// Shared with the circuit that can promote this child.
    slot: ReturnSlot,
}

impl ActiveSession {
    fn is_child(&self) -> bool {
        self.child
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .is_some()
    }

    fn set_child_place(&self, place: Option<OwnedSemaphorePermit>) {
        *self
            .child
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = place;
    }
}

/// Keeps the accepted seed and circuit reserved until logout, timeout or failure.
/// Do not log this value: it includes session credentials received from Robust.
pub struct AcceptedAgent {
    predecessor: Option<watch::Receiver<()>>,
    pub accepted_at: tokio::time::Instant,
    pub expect: CircuitExpect,
    pub secure_session_id: Uuid,
    pub caps_path: String,
    pub start_pos: [f32; 3],
    pub first_name: String,
    pub last_name: String,
    /// `ViaLogin`. A hop child is not one, so its end is not a failed login.
    pub via_login: bool,
    pub client_ip: String,
    pub service_session_id: String,
    /// `TeleportFlags` from the accepted login, including bits other than ViaLogin.
    pub teleport_flags: u32,
    pub viewer: String,
    pub channel: String,
    pub mac: String,
    pub id0: String,
    pub service_urls: Vec<(String, String)>,
    pub appearance: Option<Appearance>,
    /// Same gate the session stores. The circuit waits on it for a return.
    pub return_slot: ReturnSlot,
    /// Set when this agent's seed capability has been answered.
    pub seeds_sent: Arc<AtomicBool>,
    pub object_animations: Arc<AtomicBool>,
    pub supports_pbr: Arc<AtomicBool>,
    cancelled: oneshot::Receiver<CircuitStop>,
    _lease: SessionLease,
    /// Keeps this login's pending `UseCircuitCode` until the session opens
    /// or the connect wait ends. Drop releases it.
    _circuit_hold: Option<LoginReservation>,
    // Drop last, after the UDP serve, frame removal, service writes and generation lease.
    _logout_done: watch::Sender<()>,
}

impl AcceptedAgent {
    pub async fn wait_for_predecessor(&mut self) {
        if let Some(mut done) = self.predecessor.take() {
            let _ = done.changed().await;
        }
    }

    pub async fn lock_lifecycle(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self._lease
            .dispatch_slot
            .lane
            .lifecycle
            .clone()
            .lock_owned()
            .await
    }

    pub fn is_current(&self) -> bool {
        self._lease
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&self._lease.agent_id)
            .is_some_and(|session| session.generation == self._lease.generation)
    }

    /// This root was replaced by a login that holds a different Presence session.
    /// The old session is logged out only when this generation was still the
    /// root. A demoted child, a departed agent, a same-session replay and a
    /// far hop are not this case.
    pub fn replaced_by_other_session(&self) -> bool {
        if !self._lease.rooted_when_replaced.load(Ordering::Acquire) {
            return false;
        }
        self._lease
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&self._lease.agent_id)
            .is_some_and(|session| {
                session.generation != self._lease.generation
                    && session.session_id != self.expect.session_id
            })
    }

    /// A replay may replace a circuit while retaining its Presence session.
    pub fn may_logout_presence(&self) -> bool {
        self._lease
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&self._lease.agent_id)
            .is_none_or(|session| {
                session.generation == self._lease.generation
                    || session.session_id != self.expect.session_id
            })
    }
    /// Completes when a new login or CloseAgent stops this circuit. The
    /// dispatcher must drop the old UDP-serving future before starting a
    /// replacement. A dropped sender is a disable, matching the old close.
    pub async fn replaced(&mut self) -> CircuitStop {
        (&mut self.cancelled).await.unwrap_or(CircuitStop::Disable)
    }

    /// The circuit report is in hand and this generation is still the active one.
    pub fn mark_root(&self) {
        let mut sessions = self
            ._lease
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(session) = sessions.get_mut(&self._lease.agent_id)
            && session.generation == self._lease.generation
        {
            session.root = true;
            session.set_child_place(None);
            session.slot.note_rooted();
            // An in-view return makes this generation the root again. The
            // neighbour it left is an ordinary open child, so logout closes it.
            self._lease.children.release_promoted(self._lease.agent_id);
            publish_agents(&sessions, &self._lease.meters);
            drop(sessions);
            // A teleport's CreateAgent is a child. SendInitialData can arm
            // before this report, so accept stored the identity when none
            // was held. A far-hop return kept the previous identity at
            // accept; publish this generation now, before the return handshake.
            self.publish_prepared_root();
        }
    }

    fn publish_prepared_root(&self) {
        let Some(identity) = self
            ._lease
            .prepared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&self._lease.generation)
            .cloned()
        else {
            return;
        };
        self._lease
            .roots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(self._lease.agent_id, identity);
    }

    /// The grid logout runs only while this generation is still the root.
    /// A demoted child and a far-hop close both leave the presence where it is.
    pub fn still_root(&self) -> bool {
        if self._lease.departed.load(Ordering::Acquire) {
            return false;
        }
        self._lease
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&self._lease.agent_id)
            .is_some_and(|session| session.generation == self._lease.generation && session.root)
    }
}

/// One far hop waiting for the destination's release DELETE.
struct ReleaseSlot {
    notify: Arc<Notify>,
    arrived: bool,
}

fn publish_agents(sessions: &HashMap<Uuid, ActiveSession>, meters: &RegionMeters) {
    let roots = sessions.values().filter(|session| session.root).count() as u64;
    let children = sessions
        .values()
        .filter(|session| session.is_child())
        .count() as u64;
    meters.set_agents(roots, children, sessions.len() as u64);
}

fn occupying(sessions: &HashMap<Uuid, ActiveSession>) -> usize {
    sessions
        .values()
        .filter(|session| !session.is_child())
        .count()
}

struct SessionLease {
    /// Released when this generation is dropped, including a replacement that
    /// outlived the old circuit. A secret still held by another generation stays.
    secrets: ferroan_log::SessionSecrets,
    sessions: Arc<Mutex<HashMap<Uuid, ActiveSession>>>,
    agent_id: Uuid,
    generation: Uuid,
    caps: CapsRegistry,
    _child: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
    dispatch_slot: DispatchSlot,
    meters: Arc<RegionMeters>,
    departed: Arc<AtomicBool>,
    rooted_when_replaced: Arc<AtomicBool>,
    children: Arc<crate::children::ChildBook>,
    roots: Arc<Mutex<HashMap<Uuid, ChildIdentity>>>,
    prepared: Arc<Mutex<HashMap<Uuid, ChildIdentity>>>,
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        // Release redaction holds before `sessions`. `accept` takes `sessions`
        // and then registers secrets, so the opposite order can deadlock.
        drop(std::mem::take(&mut self.secrets));
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.children.end_generation(self.agent_id, self.generation);
        if sessions
            .get(&self.agent_id)
            .is_some_and(|session| session.generation == self.generation)
        {
            self.caps.clear_agent(self.agent_id);
            if let Some(session) = sessions.remove(&self.agent_id) {
                session.slot.close();
            }
            publish_agents(&sessions, &self.meters);
        }
        // The successor is whoever replaced this lease. Captured before the
        // sessions lock drops: a child accepted while this identity was held
        // wrote only `prepared`, and SendInitialData can arm before mark_root.
        let successor = sessions
            .get(&self.agent_id)
            .filter(|session| session.generation != self.generation)
            .map(|session| session.generation);
        drop(sessions);
        // `remember_identity` takes `prepared` then `roots`. Do not take
        // `prepared` while `sessions` or `roots` is held.
        let published = {
            let mut prepared = self
                .prepared
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            prepared.remove(&self.generation);
            successor.and_then(|generation| prepared.get(&generation).cloned())
        };
        let mut roots = self
            .roots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if roots
            .get(&self.agent_id)
            .is_some_and(|identity| identity.generation == self.generation)
        {
            roots.remove(&self.agent_id);
            if let Some(identity) = published {
                roots.insert(self.agent_id, identity);
            }
        }
    }
}

impl SimulationState {
    pub fn new(
        config: &RegionConfig,
        caps: CapsRegistry,
        sender: mpsc::Sender<AcceptedAgent>,
    ) -> Self {
        Self::with_meters(config, caps, sender, Arc::new(RegionMeters::new()))
    }

    pub fn with_meters(
        config: &RegionConfig,
        caps: CapsRegistry,
        sender: mpsc::Sender<AcceptedAgent>,
        meters: Arc<RegionMeters>,
    ) -> Self {
        Self {
            region_id: config.id,
            caps,
            sender,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            region_name: config.name.clone(),
            region_size: config.size,
            presence: PresenceClient::new(&config.presence_server_uri),
            control_plane: config.control_plane.clone(),
            dispatch_slots: Default::default(),
            child_places: Arc::new(Semaphore::new(CHILD_DISPATCH_CAPACITY)),
            admission_open: Arc::new(AtomicBool::new(true)),
            create_agent_budget: Duration::from_secs(25),
            root_limit: ROOT_AGENT_LIMIT,
            bypass_limit: ROOT_BYPASS_LIMIT,
            staff: StaffGate::from_config(config),
            meters,
            releases: Arc::new(Mutex::new(HashMap::new())),
            objects: Arc::new(Vec::new()),
            children: Arc::new(crate::children::ChildBook::new()),
            roots: Arc::new(Mutex::new(HashMap::new())),
            prepared: Arc::new(Mutex::new(HashMap::new())),
            admissions: Arc::new(Mutex::new(HashMap::new())),
            scene: None,
            circuit: Arc::new(OnceLock::new()),
            login_connect_wait: config.login_connect_wait,
        }
    }

    /// Point admission at the listener that will serve the accepted login.
    /// A second call keeps the first listener.
    pub fn bind_circuit(&self, circuit: CircuitServer) {
        let _ = self.circuit.set(circuit);
    }

    /// Capabilities for the root whose child events this process queues.
    pub fn caps(&self) -> CapsRegistry {
        self.caps.clone()
    }

    /// The child book admission and the frame driver share.
    pub(crate) fn child_book(&self) -> Arc<crate::children::ChildBook> {
        Arc::clone(&self.children)
    }

    /// The root identity stored at acceptance, if this generation is still it.
    pub fn child_identity(&self, agent: Uuid) -> Option<ChildIdentity> {
        self.roots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&agent)
            .cloned()
    }

    /// Admission tasks one agent may have in flight. A create can hang for
    /// 30 s, so several enables overlap. Past this, another enable is refused
    /// and does not claim. Closes already decided are still sent.
    pub const CHILD_ADMISSION_TASKS: usize = 8;

    /// Drop finished handles. True when another admission would pass
    /// [`Self::CHILD_ADMISSION_TASKS`].
    pub(crate) fn admission_full(&self, agent: Uuid) -> bool {
        let mut admissions = self
            .admissions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(queued) = admissions.get_mut(&agent) else {
            return false;
        };
        queued.retain(|running| !running.is_finished());
        let count = queued.len();
        if count < Self::CHILD_ADMISSION_TASKS {
            return false;
        }
        ferroan_diag::refuse(
            REGION_CHILD_ADMISSIONS,
            Kind::Hard,
            Self::CHILD_ADMISSION_TASKS as u64,
            count.saturating_add(1) as u64,
            "child-admission",
        );
        true
    }

    /// Remember one child-drive admission. A finished handle is dropped.
    pub fn track_admission(&self, agent: Uuid, handle: tokio::task::JoinHandle<()>) {
        let mut admissions = self
            .admissions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let queued = admissions.entry(agent).or_default();
        queued.retain(|running| !running.is_finished());
        queued.push(handle);
    }

    /// Stop this agent's admission HTTP and wait until each guard has dropped.
    /// A logout that already won stages its close here, before the serve-end take.
    pub async fn abort_admissions(&self, agent: Uuid) {
        let handles = self
            .admissions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&agent)
            .unwrap_or_default();
        for handle in &handles {
            handle.abort();
        }
        for handle in handles {
            let _ = handle.await;
        }
    }

    /// Stop every admission the drive spawned. Process shutdown calls this
    /// after the drive task itself has stopped, so no new task is tracked.
    pub async fn abort_every_admission(&self) {
        let handles: Vec<_> = self
            .admissions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .drain()
            .flat_map(|(_, queued)| queued)
            .collect();
        for handle in &handles {
            handle.abort();
        }
        for handle in handles {
            let _ = handle.await;
        }
    }

    /// A current generation with no circuit. Hop tests store a child size
    /// through the live presence without a login.
    #[cfg(test)]
    pub(crate) fn seat_generation(&self, agent: Uuid, generation: Uuid, session: Uuid) {
        let (_, logout_wait) = watch::channel(());
        let (cancel, _) = oneshot::channel();
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(
                agent,
                ActiveSession {
                    session_id: session,
                    generation,
                    root: true,
                    child: Arc::new(Mutex::new(None)),
                    departed: Arc::new(AtomicBool::new(false)),
                    rooted_when_replaced: Arc::new(AtomicBool::new(false)),
                    cancel: Some(cancel),
                    logout_done: logout_wait,
                    slot: ReturnSlot::new(false),
                },
            );
    }

    /// Wait until this agent's admission tasks finish. Tests use it so a
    /// later assertion sees the POST's report.
    #[cfg(test)]
    pub(crate) async fn finish_admissions(&self, agent: Uuid) {
        let handles = self
            .admissions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&agent)
            .unwrap_or_default();
        for handle in handles {
            let _ = handle.await;
        }
    }

    /// Store this generation's child-circuit identity. A visibility child
    /// does not replace an identity already held for the agent. A teleport
    /// is a child with nothing held yet, and `SendInitialData` can arm
    /// before [`AcceptedAgent::mark_root`], so that case is stored now.
    fn remember_identity(&self, generation: Uuid, fields: &AgentFields) {
        let height = fields
            .appearance
            .as_ref()
            .map(|appearance| appearance.height)
            .filter(|height| height.is_finite() && *height > 0.0)
            .unwrap_or(1.9);
        let identity = ChildIdentity {
            agent_id: fields.agent_id,
            session_id: fields.session_id,
            secure_session_id: fields.secure_session_id,
            service_session_id: fields.service_session_id.clone(),
            circuit_code: fields.circuit_code,
            first_name: fields.first_name.clone(),
            last_name: fields.last_name.clone(),
            client_ip: fields.client_ip.clone(),
            viewer: fields.viewer.clone(),
            channel: fields.channel.clone(),
            mac: fields.mac.clone(),
            id0: fields.id0.clone(),
            service_urls: fields.service_urls.clone(),
            generation,
            height,
        };
        self.prepared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(generation, identity.clone());
        let mut roots = self
            .roots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !fields.child || !roots.contains_key(&fields.agent_id) {
            roots.insert(fields.agent_id, identity);
        }
    }

    pub fn keep_objects(&mut self, objects: Arc<Vec<ferroan_scene::ObjectGroup>>) {
        self.objects = objects;
    }

    pub fn keep_scene(&mut self, frame: Arc<ferroan_scene::TerrainFrame>) {
        self.scene = Some(frame);
    }

    fn mark_child(&self, agent: Uuid, child: bool) {
        let Some(frame) = &self.scene else {
            return;
        };
        if let Err(error) = frame.set_child(agent, child) {
            tracing::debug!("scene child flag was not set: {error}");
        }
    }

    pub fn stored_object_counts(&self) -> (usize, usize) {
        let prims = self.objects.iter().map(|group| group.parts.len()).sum();
        (self.objects.len(), prims)
    }

    pub fn apply_estate(&mut self, access: ferroan_scene::EstateAccess) {
        self.staff.apply_store(access);
    }

    /// The god rule `IsAdministrator` uses. The clone shares the level cache.
    pub fn staff_gate(&self) -> StaffGate {
        self.staff.clone()
    }

    pub fn meters(&self) -> Arc<RegionMeters> {
        self.meters.clone()
    }

    pub fn stop_accepting(&self) {
        self.admission_open.store(false, Ordering::Release);
    }

    /// Look up a grid-god level when this agent is not already estate staff.
    /// `None` means the account is not a god, including a failed lookup.
    async fn lookup_level(&self, agent: Uuid) -> Option<i32> {
        if !self.staff.needs_account_level(agent) {
            return None;
        }
        if let Some(level) = self.staff.cached_level(agent) {
            return Some(level);
        }
        let account =
            crate::grid::account_user_level(self.staff.account_uri(), self.staff.scope_id(), agent)
                .await?;
        let level = account.level;
        self.staff.note_account(agent, account);
        Some(level)
    }

    fn admission_refusal(&self, agent: Uuid, level: Option<i32>) -> Option<&'static str> {
        self.admission_for(agent, level, false)
    }

    /// A child does not take a root slot. A closed region still refuses it.
    fn admission_for(&self, agent: Uuid, level: Option<i32>, child: bool) -> Option<&'static str> {
        if !self.admission_open.load(Ordering::Acquire) {
            return Some("Logins to this region are disabled");
        }
        if child {
            return None;
        }
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.refusal_locked(&sessions, agent, level)
    }

    fn refusal_locked(
        &self,
        sessions: &HashMap<Uuid, ActiveSession>,
        agent: Uuid,
        level: Option<i32>,
    ) -> Option<&'static str> {
        let occupying = occupying(sessions);
        if sessions.contains_key(&agent) || occupying < self.root_limit {
            return None;
        }
        if self.staff.bypasses(agent, level) {
            let extra = occupying - self.root_limit;
            if extra < self.bypass_limit {
                return None;
            }
            ferroan_diag::refuse(
                ROOT_BYPASS,
                Kind::Hard,
                self.bypass_limit as u64,
                extra.saturating_add(1) as u64,
                "/agent",
            );
            return Some("The region is full");
        }
        ferroan_diag::refuse(
            ROOT_AGENTS,
            Kind::Hard,
            self.root_limit as u64,
            occupying.saturating_add(1) as u64,
            "/agent",
        );
        Some("The region is full")
    }

    /// Serve using `into_make_service_with_connect_info::<SocketAddr>()`.
    pub fn router(self) -> Router {
        Router::new()
            .route("/agent/{*path}", any(handle))
            .with_state(self)
    }

    pub(crate) fn reserve_child_place(&self) -> Result<OwnedSemaphorePermit, &'static str> {
        self.child_places.clone().try_acquire_owned().map_err(|_| {
            ferroan_diag::refuse(
                ferroan_diag::REGION_DISPATCH_CHILDREN,
                Kind::Hard,
                CHILD_DISPATCH_CAPACITY as u64,
                (CHILD_DISPATCH_CAPACITY + 1) as u64,
                "/agent",
            );
            "Region is busy completing a login or logout"
        })
    }

    fn reserve_admission(
        &self,
        fields: &AgentFields,
    ) -> Result<(DispatchSlot, Option<OwnedSemaphorePermit>), &'static str> {
        let child = fields
            .child
            .then(|| self.reserve_child_place())
            .transpose()?;
        let slot = DispatchSlot::reserve(&self.dispatch_slots, fields.agent_id)?;
        Ok((slot, child))
    }

    #[cfg(test)]
    fn accept(&self, fields: AgentFields) -> Result<(), &'static str> {
        self.accept_with_level(fields, None)
    }

    #[cfg(test)]
    fn accept_with_level(
        &self,
        fields: AgentFields,
        level: Option<i32>,
    ) -> Result<(), &'static str> {
        let (slot, child) = self.reserve_admission(&fields)?;
        self.accept_in_slot(fields, level, slot, child)
    }

    fn accept_in_slot(
        &self,
        fields: AgentFields,
        level: Option<i32>,
        dispatch_slot: DispatchSlot,
        child: Option<OwnedSemaphorePermit>,
    ) -> Result<(), &'static str> {
        if !self.admission_open.load(Ordering::Acquire) {
            return Err("Logins to this region are disabled");
        }
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        // A child does not take a root slot. A login still does.
        if !fields.child
            && let Some(reason) = self.refusal_locked(&sessions, fields.agent_id, level)
        {
            return Err(reason);
        }
        // Reserve before replacing anything: backpressure or a stopped
        // dispatcher must leave the current sessions and capabilities intact.
        let permit = self
            .sender
            .try_reserve()
            .map_err(|_| "Region is not accepting logins")?;
        if let Some(mut previous) = sessions.remove(&fields.agent_id) {
            if previous.root && !previous.departed.load(Ordering::Acquire) {
                previous.rooted_when_replaced.store(true, Ordering::Release);
            }
            if let Some(cancel) = previous.cancel.take() {
                let _ = cancel.send(CircuitStop::Disable);
            }
            self.caps.clear_agent(fields.agent_id);
        }
        let path = format!("/CAPS/{}0000/", fields.caps_path);
        if self.caps.register_agent(&path, fields.agent_id).is_err() {
            publish_agents(&sessions, &self.meters);
            return Err("Could not create seed capability");
        }
        // Register before the login line. The line itself names only the avatar.
        // The lease drops these when the session closes.
        let secrets = ferroan_log::SessionSecrets::login(
            &fields.session_id.to_string(),
            &fields.secure_session_id.to_string(),
            fields.circuit_code,
            &path,
        );
        let first_name = fields.first_name.clone();
        let last_name = fields.last_name.clone();
        let agent_id = fields.agent_id;
        let generation = Uuid::new_v4();
        self.children
            .attach(fields.agent_id, generation, fields.session_id);
        let departed = Arc::new(AtomicBool::new(false));
        let (cancel, cancelled) = oneshot::channel();
        let (logout_done, logout_wait) = watch::channel(());
        let predecessor = dispatch_slot
            .lane
            .tail
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .replace(logout_wait.clone());
        let slot = ReturnSlot::new(fields.child);
        let child = Arc::new(Mutex::new(child));
        let rooted_when_replaced = Arc::new(AtomicBool::new(false));
        sessions.insert(
            fields.agent_id,
            ActiveSession {
                session_id: fields.session_id,
                generation,
                root: false,
                child: Arc::clone(&child),
                departed: Arc::clone(&departed),
                rooted_when_replaced: Arc::clone(&rooted_when_replaced),
                cancel: Some(cancel),
                logout_done: logout_wait,
                slot: slot.clone(),
            },
        );
        publish_agents(&sessions, &self.meters);
        self.remember_identity(generation, &fields);
        let lease = SessionLease {
            secrets,
            sessions: self.sessions.clone(),
            agent_id: fields.agent_id,
            generation,
            caps: self.caps.clone(),
            dispatch_slot,
            _child: child,
            meters: self.meters.clone(),
            departed,
            rooted_when_replaced,
            children: Arc::clone(&self.children),
            roots: Arc::clone(&self.roots),
            prepared: Arc::clone(&self.prepared),
        };
        let (seeds_sent, object_animations, supports_pbr) =
            self.caps.seed_flags(fields.agent_id).unwrap_or_default();
        let accepted_at = tokio::time::Instant::now();
        let expect = CircuitExpect {
            agent_id: fields.agent_id,
            session_id: fields.session_id,
            circuit_code: fields.circuit_code,
            region_id: self.region_id,
        };
        // Before the group lookup, the avatar lookup and the frame attach.
        // Those can outlast the 60 second pending cache. The hold keeps this
        // code until the session opens or the connect wait ends.
        let circuit_hold = self.circuit.get().map(|circuit| {
            let until = if self.login_connect_wait.is_zero() {
                None
            } else {
                Some(accepted_at + self.login_connect_wait)
            };
            circuit.reserve_login(expect, until)
        });
        let accepted = AcceptedAgent {
            _logout_done: logout_done,
            predecessor,
            accepted_at,
            expect,
            secure_session_id: fields.secure_session_id,
            caps_path: fields.caps_path,
            start_pos: fields.start_pos,
            first_name: fields.first_name,
            last_name: fields.last_name,
            via_login: fields.via_login,
            client_ip: fields.client_ip,
            service_session_id: fields.service_session_id,
            teleport_flags: fields.teleport_flags,
            viewer: fields.viewer,
            channel: fields.channel,
            mac: fields.mac,
            id0: fields.id0,
            service_urls: fields.service_urls,
            appearance: fields.appearance,
            return_slot: slot,
            seeds_sent,
            object_animations,
            supports_pbr,
            cancelled,
            _lease: lease,
            _circuit_hold: circuit_hold,
        };
        // A receiver dropped after reservation may drop this lease in send().
        // Release the admission lock first so that cleanup cannot deadlock.
        drop(sessions);
        {
            // The span ends before the circuit starts, so the login trace is
            // the admission itself.
            let span = tracing::info_span!("login", agent_id = %agent_id);
            let _entered = span.enter();
            tracing::info!(
                agent_id = %agent_id,
                "login accepted first_name={first_name} last_name={last_name}"
            );
        }
        permit.send(accepted);
        Ok(())
    }

    /// A failed verify of an existing root closes that root. The session stays
    /// until the dispatcher drops the generation lease after `notify_logout`.
    fn begin_root_close(&self, agent: Uuid) -> Option<watch::Receiver<()>> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let session = sessions.get_mut(&agent)?;
        if !session.root {
            return None;
        }
        if let Some(cancel) = session.cancel.take() {
            let _ = cancel.send(CircuitStop::Disable);
        }
        Some(session.logout_done.clone())
    }

    /// The root becomes a child before `UpdateAgent`. Caps and the circuit stay.
    pub fn demote_child(&self, agent: Uuid) -> Result<(), &'static str> {
        let place = self.reserve_child_place()?;
        self.demote_with_place(agent, place);
        Ok(())
    }

    pub(crate) fn demote_with_place(&self, agent: Uuid, place: OwnedSemaphorePermit) {
        {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(session) = sessions.get_mut(&agent) else {
                return;
            };
            if session.departed.load(Ordering::Acquire) {
                return;
            }
            session.root = false;
            session.set_child_place(Some(place));
            session.slot.arm_child();
            publish_agents(&sessions, &self.meters);
        }
        self.mark_child(agent, true);
    }

    /// `UpdateAgent` failed. The provisional child flag goes back to root.
    pub fn restore_root(&self, agent: Uuid) {
        {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(session) = sessions.get_mut(&agent) else {
                return;
            };
            if session.departed.load(Ordering::Acquire) {
                return;
            }
            session.set_child_place(None);
            session.root = true;
            session.slot.disarm();
            publish_agents(&sessions, &self.meters);
        }
        self.mark_child(agent, false);
    }

    /// Same session, not a login. Keep the circuit and the caps. A stale root
    /// becomes a child instead of being logged out.
    fn reuse_child(
        &self,
        fields: &AgentFields,
        place: OwnedSemaphorePermit,
    ) -> Result<(), &'static str> {
        if !self.admission_open.load(Ordering::Acquire) {
            return Err("Logins to this region are disabled");
        }
        {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(session) = sessions.get_mut(&fields.agent_id) else {
                return Err("Child agent is not in this region");
            };
            if session.session_id != fields.session_id || session.departed.load(Ordering::Acquire) {
                return Err("Child agent is not in this region");
            }
            session.root = false;
            session.set_child_place(Some(place));
            session.slot.arm_child();
            publish_agents(&sessions, &self.meters);
        }
        self.mark_child(fields.agent_id, true);
        let path = format!("/CAPS/{}0000/", fields.caps_path);
        match self.caps.register_agent(&path, fields.agent_id) {
            Ok(()) | Err(CapsError::Occupied) => Ok(()),
            Err(_) => Err("Could not create seed capability"),
        }
    }

    fn same_session(&self, agent: Uuid, session: Uuid) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&agent)
            .is_some_and(|row| row.session_id == session && !row.departed.load(Ordering::Acquire))
    }

    /// Arm the release wait before `UpdateAgent`, so a callback during that
    /// call is not missed. A hop that is inside view does not arm one.
    pub fn arm_release(&self, agent: Uuid) {
        let mut slots = self
            .releases
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        slots.insert(
            agent,
            ReleaseSlot {
                notify: Arc::new(Notify::new()),
                arrived: false,
            },
        );
    }

    pub fn disarm_release(&self, agent: Uuid) {
        self.releases
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&agent);
    }

    /// The destination DELETE. A call before the wait is armed is ignored.
    pub fn note_release(&self, agent: Uuid) {
        let mut slots = self
            .releases
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(slot) = slots.get_mut(&agent) else {
            return;
        };
        slot.arrived = true;
        slot.notify.notify_one();
    }

    /// Wait until the destination releases this agent, or for `budget`.
    pub async fn wait_release_for(&self, agent: Uuid, budget: Duration) {
        let notify = {
            let mut slots = self
                .releases
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(slot) = slots.get(&agent) else {
                return;
            };
            if slot.arrived {
                slots.remove(&agent);
                return;
            }
            Arc::clone(&slot.notify)
        };
        let _ = tokio::time::timeout(budget, notify.notified()).await;
        self.releases
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&agent);
    }

    /// Fifteen seconds, the same bound as the source region's release wait.
    pub async fn wait_release(&self, agent: Uuid) {
        self.wait_release_for(agent, Duration::from_secs(15)).await;
    }

    /// Local close after a far hop. Caps go. The grid presence stays put.
    /// Open children outside `destination`'s view are staged for one close
    /// each. The lease drop that follows does not stage them again.
    pub fn close_departed(&self, agent: Uuid, destination: &crate::children::FarDestination) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(session) = sessions.get(&agent) else {
            return;
        };
        let generation = session.generation;
        self.children
            .end_generation_far(agent, generation, destination);
        session.departed.store(true, Ordering::Release);
        let slot = session.slot.clone();
        self.caps.clear_agent(agent);
        sessions.remove(&agent);
        publish_agents(&sessions, &self.meters);
        drop(sessions);
        slot.close();
    }

    /// The hop away arrived. The frame kills the avatar on root viewers that
    /// have no child in the destination, and out of view range both ways
    /// first. A root knows the destination when its open child is there.
    pub async fn moved_away(&self, agent: Uuid, near: bool, end: &RegionEnd) {
        let Some(frame) = &self.scene else {
            return;
        };
        let known_by = self.children.known_by(end.region_id, agent);
        let away = ferroan_scene::MovedAway { near, known_by };
        let done = match frame.moved_away(agent, away) {
            Ok(pending) => pending.wait().await,
            Err(error) => Err(error),
        };
        if let Err(error) = done {
            tracing::debug!("scene hop kills were not sent: {error}");
        }
    }

    /// The root circuit generation admission and the hop share.
    pub fn root_generation(&self, agent: Uuid) -> Option<Uuid> {
        self.generation_of(agent)
    }

    /// Claim one child destination for `generation` at 256 m on each side.
    /// The returned seed is the admission POST's caps path. `Existing` means
    /// the POST is already owned. The caller passes this same generation to
    /// [`Self::finish_child`] or [`Self::refuse_child`]. A caller that knows
    /// the footprint uses [`Self::claim_child_sized`].
    pub fn claim_child(
        &self,
        agent: Uuid,
        generation: Uuid,
        end: RegionEnd,
    ) -> crate::children::Claim {
        self.children.claim(agent, generation, end)
    }

    /// [`Self::claim_child`] that stores this neighbour's footprint. A zero
    /// extent stays the 256 m fallback.
    pub fn claim_child_sized(
        &self,
        agent: Uuid,
        generation: Uuid,
        end: RegionEnd,
        size: [u32; 2],
    ) -> crate::children::Claim {
        self.children.claim_sized(agent, generation, end, size)
    }

    /// The neighbour refused that destination. No viewer events follow.
    pub fn refuse_child(&self, agent: Uuid, generation: Uuid, region: Uuid) {
        self.children.refuse(agent, generation, region);
    }

    /// The admission POST failed after the seed was stored. Before logout the
    /// claim is dropped, as a refusal is. After logout the child is closed.
    pub fn fail_child(
        &self,
        agent: Uuid,
        generation: Uuid,
        region: Uuid,
    ) -> crate::children::Admission {
        self.children.failed(agent, generation, region)
    }

    /// The admission POST finished for the generation that claimed it.
    pub fn finish_child(
        &self,
        agent: Uuid,
        generation: Uuid,
        region: Uuid,
    ) -> crate::children::Admission {
        self.children.accepted(agent, generation, region)
    }

    /// Caps path already stored for this root's current generation, if any.
    pub fn child_seed(&self, agent: Uuid, region: Uuid) -> Option<String> {
        self.generation_of(agent)?;
        self.children.seed(agent, region)
    }

    /// The in-view hop now owns this destination. Logout does not close it
    /// until this generation is root again. A later admission completion
    /// announces once.
    pub fn promote_child(&self, agent: Uuid, end: &RegionEnd, seed: &str) {
        if self.generation_of(agent).is_none() {
            return;
        }
        self.children.promote(agent, end, seed);
    }

    /// [`Self::promote_child`] that stores this neighbour's footprint. A zero
    /// extent stays the 256 m fallback. An existing destination takes the new size.
    pub fn promote_child_sized(&self, agent: Uuid, end: &RegionEnd, seed: &str, size: [u32; 2]) {
        if self.generation_of(agent).is_none() {
            return;
        }
        self.children.promote_sized(agent, end, seed, size);
    }

    fn generation_of(&self, agent: Uuid) -> Option<Uuid> {
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&agent)
            .map(|session| session.generation)
    }

    /// CloseAgent. The session stays so a root still logs Presence out.
    /// A mismatched token does nothing. The HTTP handler still answers 200.
    fn close_if_authorized(&self, agent: Uuid, auth: &str) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(session) = sessions.get_mut(&agent) else {
            return;
        };
        if session.session_id.to_string() != auth {
            return;
        }
        if let Some(cancel) = session.cancel.take() {
            let _ = cancel.send(CircuitStop::Quiet);
        }
    }

    /// Open children logout should close, once each. A pending admission stays
    /// until its POST finishes, and then closes if logout already won.
    pub fn take_open_children(&self, agent: Uuid) -> Vec<RegionEnd> {
        self.children.begin_logout(agent)
    }

    /// Children a replaced root left for one close each, with that root's session.
    pub fn take_staged_closes(&self, agent: Uuid) -> Vec<(Uuid, RegionEnd)> {
        self.children.take_staged_closes(agent)
    }
}

async fn handle(
    State(state): State<SimulationState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_owned();
    let path: Vec<_> = request.uri().path().trim_matches('/').split('/').collect();
    let agent = path.get(1).and_then(|s| Uuid::parse_str(s).ok());
    let Some(agent) = agent else {
        return bare(StatusCode::NOT_FOUND);
    };
    let region = path.get(2).and_then(|s| Uuid::parse_str(s).ok());
    if method == "DELETE" && (agent.is_nil() || region.is_none_or(|id| id.is_nil())) {
        return bare(StatusCode::BAD_REQUEST);
    }
    if method == "DELETE" && path.get(3).copied() == Some("release") {
        if region == Some(state.region_id) {
            state.note_release(agent);
        }
        tracing::debug!("source hop: release");
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/plain"),
                (header::CONNECTION, "close"),
            ],
            format!("OpenSim agent {agent}"),
        )
            .into_response();
    }
    if method == "DELETE" {
        if region == Some(state.region_id)
            && let Some(auth) = auth_query(request.uri().query())
        {
            state.close_if_authorized(agent, auth);
        }
        tracing::debug!("agent close");
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/plain"),
                (header::CONNECTION, "close"),
            ],
            format!("OpenSim agent {agent}"),
        )
            .into_response();
    }
    if method == "PUT" {
        return update_agent(&state, agent, request).await;
    }
    if method != "POST" && method != "QUERYACCESS" {
        return bare(StatusCode::METHOD_NOT_ALLOWED);
    }
    if method == "POST"
        && (request.headers().contains_key("x-secondlife-shard")
            || !state.control_plane.allows(peer.ip()).await)
    {
        return forbidden();
    }
    if agent.is_nil() || (method == "QUERYACCESS" && region.is_none_or(|id| id.is_nil())) {
        return bare(StatusCode::BAD_REQUEST);
    }
    let gzip = is_gzip(request.headers());
    let Ok(body) = to_bytes(request.into_body(), MAX_BODY).await else {
        ferroan_diag::refuse(
            HTTP_POST_BODY,
            Kind::Hard,
            MAX_BODY as u64,
            (MAX_BODY as u64).saturating_add(1),
            "/agent",
        );
        return bare(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let args = match decode_body(&body, gzip) {
        Ok(args) => args,
        Err(SimulationError::BodyLimit) => return bare(StatusCode::PAYLOAD_TOO_LARGE),
        Err(_) => return bare(StatusCode::BAD_REQUEST),
    };
    if method == "QUERYACCESS" {
        return match query_access(&args, region == Some(state.region_id)) {
            Ok(mut answer) => {
                if answer["success"] == true {
                    let level = state.lookup_level(agent).await;
                    if let Some(reason) = state.admission_refusal(agent, level) {
                        answer["success"] = json!(false);
                        answer["reason"] = json!(reason);
                    }
                }
                json_reply(answer, false)
            }
            Err(_) => bare(StatusCode::BAD_REQUEST),
        };
    }
    let mut fields = match AgentFields::parse(&args, agent, state.region_id) {
        Ok(fields) => fields,
        Err(SimulationError::Refused(reason)) => return create_refused(&reason, peer),
        Err(_) => return bare(StatusCode::BAD_REQUEST),
    };
    fields.clamp_position(state.region_size);
    // Scene.NewUserConnection checks LoginsEnabled before VerifyUserPresence.
    // The root-agent bound uses that same early point. A god level looked up
    // here is the level accept uses, including when the level cache is full.
    // A child does not take a root slot. A closed region still refuses it.
    let deadline = tokio::time::Instant::now() + state.create_agent_budget;
    let remaining = || deadline.saturating_duration_since(tokio::time::Instant::now());
    let busy = "Region is busy completing a login or logout";
    let Ok(level) = tokio::time::timeout_at(deadline, state.lookup_level(agent)).await else {
        return create_refused(busy, peer);
    };
    if let Some(reason) = state.admission_for(agent, level, fields.child) {
        return create_refused(reason, peer);
    }
    let (slot, child) = match state.reserve_admission(&fields) {
        Ok(slot) => slot,
        Err(reason) => return create_refused(reason, peer),
    };
    let gate = Arc::clone(&slot.lane.lifecycle);
    // Same session, not a login: keep the circuit and the caps. A failed
    // presence check must not log this avatar out.
    if !fields.via_login
        && state.same_session(agent, fields.session_id)
        && let Some(place) = child
    {
        let lock_wait = remaining();
        if lock_wait.is_zero() {
            return create_refused(busy, peer);
        }
        let Ok(_lifecycle) = tokio::time::timeout(lock_wait, gate.lock()).await else {
            return create_refused(busy, peer);
        };
        let answer = match state.reuse_child(&fields, place) {
            Ok(()) => {
                json!({"reason":"Authorized", "success":true, "your_ip":peer.ip().to_string()})
            }
            Err(reason) => json!({"reason":reason, "your_ip":peer.ip().to_string()}),
        };
        return json_reply(answer, false);
    }
    // SimulationServiceConnector.CreateAgent posts with a 30 s timeout. The
    // account lookup, lock wait and Presence verify share one shorter budget so Robust
    // does not time out and retry a login this region then accepts.
    let lock_wait = remaining();
    if lock_wait.is_zero() {
        return create_refused(busy, peer);
    }
    let Ok(lifecycle) = tokio::time::timeout(lock_wait, gate.lock()).await else {
        return create_refused(busy, peer);
    };
    let verify_budget = remaining();
    if verify_budget.is_zero() {
        // Do not call Presence with a zero timeout: reqwest treats that as unlimited.
        // A budget that expired before verify does not close the current root.
        return create_refused(busy, peer);
    }
    let verified = state
        .presence
        .verify_within(fields.agent_id, fields.session_id, verify_budget)
        .await
        .unwrap_or(false);
    if !verified {
        // Successive logins still verify before replacing. Only a failed verify
        // of an existing root closes that root and waits for its LoggedOut.
        let done = state.begin_root_close(agent);
        drop(lifecycle);
        if let Some(mut done) = done {
            let wait = remaining();
            if !wait.is_zero() {
                let _ = tokio::time::timeout(wait, done.changed()).await;
            }
        }
        return create_refused(
            &format!(
                "Failed to verify user presence in the grid for {} {}, access denied to region {}.",
                fields.first_name, fields.last_name, state.region_name
            ),
            peer,
        );
    }
    let _lifecycle = lifecycle;
    // A child keeps the connection open. A login still closes it.
    let child_create = fields.child;
    let answer = match state.accept_in_slot(fields, level, slot, child) {
        Ok(()) => json!({"reason":"Authorized", "success":true, "your_ip":peer.ip().to_string()}),
        // The C# serializer omits false values in CreateAgent replies.
        Err(reason) => json!({"reason":reason, "your_ip":peer.ip().to_string()}),
    };
    json_reply(answer, !child_create)
}

async fn update_agent(state: &SimulationState, agent: Uuid, request: Request) -> Response {
    if agent.is_nil() {
        return bare(StatusCode::BAD_REQUEST);
    }
    let gzip = is_gzip(request.headers());
    let Ok(body) = to_bytes(request.into_body(), MAX_BODY).await else {
        ferroan_diag::refuse(
            HTTP_POST_BODY,
            Kind::Hard,
            MAX_BODY as u64,
            (MAX_BODY as u64).saturating_add(1),
            "/agent",
        );
        return bare(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let args = match decode_body(&body, gzip) {
        Ok(args) => args,
        Err(SimulationError::BodyLimit) => return bare(StatusCode::PAYLOAD_TOO_LARGE),
        Err(_) => return bare(StatusCode::BAD_REQUEST),
    };
    let allowed = match args.get("message_type").and_then(Value::as_str) {
        Some("AgentData") => apply_agent_data(state, agent, &args).await,
        Some("AgentPosition") => apply_agent_position(state, agent, &args),
        _ => false,
    };
    plain_flag(allowed)
}

async fn apply_agent_data(state: &SimulationState, agent: Uuid, args: &Map<String, Value>) -> bool {
    if json_uuid(args.get("agent_uuid")) != Some(agent) {
        return false;
    }
    let Some(session) = json_uuid(args.get("session_uuid")) else {
        return false;
    };
    let ChildGate::Child(slot) = child_gate(state, agent, session) else {
        return false;
    };
    let Some(position) = json_vec3(args.get("position")) else {
        return false;
    };
    let velocity = json_vec3(args.get("velocity")).unwrap_or([0.0; 3]);
    let look_at = arrival_look(json_vec3(args.get("at_axis")).unwrap_or([0.0; 3]), velocity);
    let origin = json_uuid(args.get("region_id")).unwrap_or_else(Uuid::nil);
    let callback = args
        .get("cb_uri")
        .and_then(Value::as_str)
        .map(str::to_owned);
    // `AgentData.Unpack`. The frame applies these (`CopyFrom`) when the
    // child becomes root.
    if !slot.store_update(
        ReturnPose {
            position,
            look_at,
            origin,
            velocity,
            far: json_real(args.get("far")),
            body_rotation: json_quat(args.get("body_rotation")),
            head_rotation: json_quat(args.get("head_rotation")),
            control_flags: json_u32_text(args.get("control_flags")),
            always_run: json_bool(args.get("always_run")),
            animations: agent_animations(args),
        },
        callback,
    ) {
        return false;
    }
    if args.get("wait_for_root").and_then(Value::as_bool) == Some(true) {
        slot.wait_root(ROOT_WAIT).await
    } else {
        true
    }
}

fn apply_agent_position(state: &SimulationState, agent: Uuid, args: &Map<String, Value>) -> bool {
    if json_uuid(args.get("agent_uuid")) != Some(agent) {
        return false;
    }
    let Some(session) = json_uuid(args.get("session_uuid")) else {
        return false;
    };
    match child_gate(state, agent, session) {
        ChildGate::Root => true,
        ChildGate::Child(slot) => {
            let Some(position) = json_vec3(args.get("position")) else {
                return false;
            };
            slot.store_position(position)
        }
        ChildGate::Missing | ChildGate::Mismatch => false,
    }
}

enum ChildGate {
    Missing,
    Mismatch,
    Root,
    Child(ReturnSlot),
}

fn child_gate(state: &SimulationState, agent: Uuid, session: Uuid) -> ChildGate {
    let sessions = state
        .sessions
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(row) = sessions.get(&agent) else {
        return ChildGate::Missing;
    };
    if row.session_id != session || row.departed.load(Ordering::Acquire) {
        return ChildGate::Mismatch;
    }
    if !row.is_child() {
        return ChildGate::Root;
    }
    ChildGate::Child(row.slot.clone())
}

fn json_uuid(value: Option<&Value>) -> Option<Uuid> {
    value
        .and_then(Value::as_str)
        .and_then(|text| Uuid::parse_str(text).ok())
}

fn json_vec3(value: Option<&Value>) -> Option<[f32; 3]> {
    let text = value?.as_str()?.trim();
    let inner = text.strip_prefix('<')?.strip_suffix('>')?;
    let mut out = [0.0; 3];
    let mut parts = inner.split(',');
    for slot in &mut out {
        *slot = parts
            .next()?
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|number| number.is_finite())?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// `osd.AsReal()`: a number, or text that parses as one. Absent is 0.
fn json_real(value: Option<&Value>) -> f32 {
    match value {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(0.0) as f32,
        Some(Value::String(text)) => text.trim().parse::<f64>().unwrap_or(0.0) as f32,
        Some(Value::Bool(flag)) => f32::from(u8::from(*flag)),
        _ => 0.0,
    }
}

/// `UInt32.TryParse(osd.AsString())`. Anything else is 0.
fn json_u32_text(value: Option<&Value>) -> u32 {
    match value {
        Some(Value::String(text)) => text.parse().unwrap_or(0),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .unwrap_or(0),
        _ => 0,
    }
}

/// `osd.AsBoolean()`. Absent is false.
fn json_bool(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => text == "1" || text.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// `Quaternion.TryParse(osd.AsString())`. An absent key keeps `AgentData`'s
/// zero field; text that does not parse is the identity.
fn json_quat(value: Option<&Value>) -> [f32; 4] {
    let Some(value) = value else {
        return [0.0; 4];
    };
    let parsed = (|| {
        let inner = value
            .as_str()?
            .trim()
            .strip_prefix('<')?
            .strip_suffix('>')?;
        let mut out = [0.0; 4];
        let mut parts = inner.split(',');
        for slot in &mut out {
            *slot = parts.next()?.trim().parse::<f32>().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        Some(out)
    })();
    parsed.unwrap_or([0.0, 0.0, 0.0, 1.0])
}

/// OpenMetaverse `Vector3.Normalize` on x86-64 (the SSE4.1 path): a length
/// squared above 1e-6 divides by the square root, anything else is zero.
fn om_normalize(v: [f32; 3]) -> [f32; 3] {
    let length_squared = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if length_squared > 1.0e-6 {
        let length = length_squared.sqrt();
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        [0.0; 3]
    }
}

/// `CompleteMovement`'s arrival look (`ScenePresence.cs` 2343–2356 at pin
/// `c401bceecd`): `Lookat` (the copied at axis with Z cleared, normalised),
/// with Z cleared and normalised again; when that is under 0.01 in X and Y,
/// the copied velocity the same way; when that is too, `(0.99, 0.042, 0)`.
/// `AgentMovementComplete` sends it (2441) and the body turns to it (2405).
/// A landmark teleport sends a zero look, which `SetLookAt` stores as unit Z
/// (TB79), so it reaches the fallback.
fn arrival_look(at_axis: [f32; 3], velocity: [f32; 3]) -> [f32; 3] {
    let flat = |v: [f32; 3]| [v[0], v[1], 0.0];
    let short = |v: [f32; 3]| v[0].abs() < 0.01 && v[1].abs() < 0.01;
    let camera = om_normalize(flat(at_axis));
    let mut look = om_normalize(flat(camera));
    if short(look) {
        look = om_normalize(flat(velocity));
        if short(look) {
            look = [0.99, 0.042, 0.0];
        }
    }
    look
}

fn plain_flag(value: bool) -> Response {
    let text = if value { "True" } else { "False" };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain"),
            (header::CONNECTION, "close"),
        ],
        text,
    )
        .into_response()
}

/// DELETE the source callback after this child becomes root. The URI is not logged.
pub async fn release_callback(uri: String) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        tracing::debug!("destination release failed");
        return;
    };
    match client.delete(uri).send().await {
        Ok(response) if response.status().is_success() => {
            tracing::debug!("destination release");
        }
        _ => tracing::debug!("destination release failed"),
    }
}

fn forbidden() -> Response {
    // AgentSimpleHandler sets Content-Type application/json, then Authorize
    // answers 403 with an empty buffer. A string body would replace that type.
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::FORBIDDEN;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        header::CONNECTION,
        header::HeaderValue::from_static("close"),
    );
    response
}

fn create_refused(reason: &str, peer: SocketAddr) -> Response {
    json_reply(
        json!({"reason":reason, "your_ip":peer.ip().to_string()}),
        true,
    )
}

fn bare(status: StatusCode) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CONNECTION, "close"),
        ],
        "false",
    )
        .into_response()
}

fn json_reply(value: Value, close: bool) -> Response {
    let mut response = (
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response();
    if close || value.get("success") != Some(&Value::Bool(true)) {
        response.headers_mut().insert(
            header::CONNECTION,
            header::HeaderValue::from_static("close"),
        );
    }
    response
}

fn is_gzip(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v == "application/x-gzip")
        || headers
            .get(header::CONTENT_ENCODING)
            .is_some_and(|v| v == "gzip")
        || headers
            .get("x-content-encoding")
            .is_some_and(|v| v == "gzip")
}

fn decode_body(body: &[u8], gzip: bool) -> Result<Map<String, Value>, SimulationError> {
    if body.len() > MAX_BODY {
        ferroan_diag::refuse(
            HTTP_POST_BODY,
            Kind::Hard,
            MAX_BODY as u64,
            body.len() as u64,
            "/agent",
        );
        return Err(SimulationError::BodyLimit);
    }
    let mut decoded = Vec::new();
    let bytes = if gzip {
        GzDecoder::new(body)
            .take((MAX_BODY + 1) as u64)
            .read_to_end(&mut decoded)
            .map_err(|_| SimulationError::Compression)?;
        if decoded.len() > MAX_BODY {
            ferroan_diag::refuse(
                HTTP_POST_BODY,
                Kind::Hard,
                MAX_BODY as u64,
                decoded.len() as u64,
                "/agent",
            );
            return Err(SimulationError::BodyLimit);
        }
        decoded.as_slice()
    } else {
        body
    };
    match serde_json::from_slice(bytes) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => {
            ferroan_diag::log_malformed("/agent", bytes, "json value is not an object");
            ferroan_diag::log_boundary("/agent", "json value is not an object");
            Err(SimulationError::Json)
        }
        Err(error) => {
            ferroan_diag::log_malformed("/agent", bytes, &error.to_string());
            ferroan_diag::log_boundary("/agent", &error.to_string());
            Err(SimulationError::Json)
        }
    }
}

fn query_access(args: &Map<String, Value>, known_region: bool) -> Result<Value, SimulationError> {
    let number = |key| match args.get(key) {
        Some(Value::String(text)) => {
            text.trim().replace(',', "").parse::<f64>().unwrap_or(0.0) as f32
        }
        Some(Value::Bool(value)) => u8::from(*value) as f32,
        value => value.and_then(Value::as_f64).unwrap_or(0.0) as f32,
    };
    let supported_min = number("simulation_service_supported_min");
    let supported_max = number("simulation_service_supported_max");
    let accepted_min = number("simulation_service_accepted_min");
    let accepted_max = number("simulation_service_accepted_max");
    let mut version = 0.0;
    let mut inbound = 0.0;
    let mut outbound = 0.0;
    let deny = |reason: String| Ok(json!({"success":false,"reason":reason}));
    if supported_min == 0.0 {
        version = args
            .get("my_version")
            .and_then(Value::as_str)
            .and_then(|v| v.split('/').nth(1))
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0);
        if version == 0.0 {
            return deny("Your region is running a old version of opensim no longer supported. Consider updating it".into());
        }
        if !(MIN_VERSION..=MAX_VERSION).contains(&version) {
            return deny(format!(
                "Your region protocol version is {version} and we accept only {MIN_VERSION} - {MAX_VERSION}. No version overlap."
            ));
        }
    } else {
        if supported_min > MAX_VERSION || supported_max < MIN_VERSION {
            return deny(format!(
                "Your region provide protocol versions {supported_min} - {supported_max} and we accept only {MIN_VERSION} - {MAX_VERSION}. No version overlap."
            ));
        }
        if accepted_min > MAX_VERSION || accepted_max < MIN_VERSION {
            return deny(format!(
                "You require region protocol versions {accepted_min} - {accepted_max} and we provide only {MIN_VERSION} - {MAX_VERSION}. No version overlap."
            ));
        }
        outbound = supported_max.min(MAX_VERSION);
        inbound = accepted_max.min(MAX_VERSION);
    }
    let features = match args.get("features") {
        Some(Value::Array(values)) if values.len() <= MAX_FEATURES => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .map(|id| Value::String(id.to_string()))
                    .ok_or(SimulationError::Field("features"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::Array(_)) => return Err(SimulationError::Field("features")),
        _ => Vec::new(),
    };
    let reason = if !known_region {
        "Communications failure"
    } else if args
        .get("viaTeleport")
        .is_some_and(|value| !osd_boolean(value))
    {
        // Explicit Ferroan capability limit, not a C# refusal condition.
        "Region crossings are not implemented"
    } else {
        ""
    };
    Ok(json!({
        "success":reason.is_empty(),
        "reason":reason,
        "version":format!("SIMULATION/{version}"),
        "negotiated_inbound_version":wire_version(inbound),
        "negotiated_outbound_version":wire_version(outbound),
        "features":features,
        "carried_riders":false,
    }))
}

fn wire_version(version: f32) -> f64 {
    // OSD.FromReal widens the single; JsonWriter uses Convert.ToString(double),
    // not a fixed 15-decimal rounding. serde_json emits the shortest double too.
    f64::from(version)
}

fn osd_boolean(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => {
            !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
        }
        _ => false,
    }
}

#[derive(Clone)]
struct AgentFields {
    agent_id: Uuid,
    session_id: Uuid,
    secure_session_id: Uuid,
    circuit_code: u32,
    caps_path: String,
    start_pos: [f32; 3],
    first_name: String,
    last_name: String,
    /// The teleport child. It does not take a root slot.
    child: bool,
    /// `ViaLogin`. A child login stays refused.
    via_login: bool,
    client_ip: String,
    service_session_id: String,
    /// The full `teleport_flags` word. ViaLogin is one bit of it.
    teleport_flags: u32,
    viewer: String,
    channel: String,
    mac: String,
    id0: String,
    service_urls: Vec<(String, String)>,
    appearance: Option<Appearance>,
}

impl AgentFields {
    fn parse(
        args: &Map<String, Value>,
        path_agent: Uuid,
        region_id: Uuid,
    ) -> Result<Self, SimulationError> {
        let uuid = |key| {
            required_str(args, key).and_then(|s| {
                Uuid::parse_str(s)
                    .ok()
                    .filter(|v| !v.is_nil())
                    .ok_or(SimulationError::Field(key))
            })
        };
        let agent_id = uuid("agent_id")?;
        if agent_id != path_agent {
            return Err(SimulationError::Field("agent_id"));
        }
        let destination = Uuid::parse_str(required_str(args, "destination_uuid")?)
            .map_err(|_| SimulationError::Field("destination_uuid"))?;
        if destination != region_id {
            return Err(SimulationError::Refused(format!(
                "Did not find region {}",
                args.get("destination_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            )));
        }
        let session_id = uuid("session_id")?;
        let secure_session_id = uuid("secure_session_id")?;
        let circuit_code = required_str(args, "circuit_code")?
            .parse()
            .map_err(|_| SimulationError::Field("circuit_code"))?;
        let flags: u32 = required_str(args, "teleport_flags")?
            .parse()
            .map_err(|_| SimulationError::Field("teleport_flags"))?;
        let child = args.get("child").and_then(Value::as_bool) == Some(true);
        let via_login = flags & VIA_LOGIN != 0;
        // A child login stays refused. A non-child without ViaLogin stays refused.
        // The hop is a child whose flags do not include ViaLogin.
        if child && via_login {
            return Err(SimulationError::Refused(
                "Child agent connections are not implemented".to_owned(),
            ));
        }
        if !child && !via_login {
            return Err(SimulationError::Refused(
                "Only login agent connections are supported".to_owned(),
            ));
        }
        let caps_path = required_str(args, "caps_path")?;
        if caps_path.is_empty()
            || caps_path.len() > 128
            || !caps_path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(SimulationError::Field("caps_path"));
        }
        let position = required_str(args, "start_pos")?
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>');
        let mut coordinates = position.split(',');
        let mut start_pos = [0.0; 3];
        for coordinate in &mut start_pos {
            *coordinate = coordinates
                .next()
                .and_then(|c| c.trim().parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .ok_or(SimulationError::Field("start_pos"))?;
        }
        if coordinates.next().is_some() {
            return Err(SimulationError::Field("start_pos"));
        }
        Ok(Self {
            agent_id,
            session_id,
            secure_session_id,
            circuit_code,
            caps_path: caps_path.to_owned(),
            start_pos,
            first_name: required_str(args, "first_name")?.to_owned(),
            last_name: required_str(args, "last_name")?.to_owned(),
            child,
            via_login,
            client_ip: optional_string(args, "client_ip"),
            service_session_id: optional_string(args, "service_session_id"),
            teleport_flags: flags,
            viewer: optional_string(args, "viewer"),
            channel: optional_string(args, "channel"),
            mac: optional_string(args, "mac"),
            id0: optional_string(args, "id0"),
            service_urls: circuit_service_urls(args),
            appearance: args.get("packed_appearance").map(appearance_from_json),
        })
    }

    fn clamp_position(&mut self, size: [u32; 2]) {
        // Scene.NewUserConnection clamps root-login X/Y, preserving Z for
        // CompleteMovement's later terrain/landing correction (P2.0e).
        for (coordinate, extent) in self.start_pos[..2].iter_mut().zip(size) {
            if *coordinate < 0.0 {
                *coordinate = 1.0;
            } else if *coordinate >= extent as f32 {
                *coordinate = extent as f32 - 1.0;
            }
        }
    }
}

/// AgentCircuitData prefers the serviceurls map, falling back to legacy key/value pairs.
fn circuit_service_urls(args: &Map<String, Value>) -> Vec<(String, String)> {
    let mut legacy = Map::new();
    let urls = if let Some(map) = args.get("serviceurls").and_then(Value::as_object) {
        map
    } else if let Some(flat) = args.get("service_urls").and_then(Value::as_array) {
        for [key, value] in flat.as_chunks::<2>().0 {
            if let (Some(key), Value::String(_)) = (key.as_str(), value) {
                legacy.insert(key.to_owned(), value.clone());
            }
        }
        &legacy
    } else {
        return Vec::new();
    };
    urls.iter()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect()
}

fn optional_string(args: &Map<String, Value>, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn auth_query(query: Option<&str>) -> Option<&str> {
    let query = query?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "auth" && !value.is_empty()).then_some(value)
    })
}

fn required_str<'a>(
    args: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, SimulationError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or(SimulationError::Field(key))
}

#[cfg(test)]
mod tests {
    mod child_neighbours;
    use super::*;
    use std::io::Write;

    const QUERY_REQUEST: &str =
        include_str!("../../../fixtures/region-login/query-access.request.json");
    const QUERY_RESPONSE: &str =
        include_str!("../../../fixtures/region-login/query-access.response.json");
    const CREATE_REQUEST: &str =
        include_str!("../../../fixtures/region-login/create-agent.request.json");
    const CREATE_RESPONSE: &str =
        include_str!("../../../fixtures/region-login/create-agent.response.json");

    #[test]
    fn stored_objects_stay_shared_after_clone() {
        let region = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let caps = CapsRegistry::new("http://127.0.0.1:9").unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        let mut state =
            SimulationState::new(&test_config("http://127.0.0.1:9", region), caps, sender);
        assert_eq!(state.stored_object_counts(), (0, 0));
        let groups = Arc::new(vec![ferroan_scene::ObjectGroup {
            from_item_id: Uuid::nil(),
            rez_parameter: None,
            parts: Vec::new(),
        }]);
        state.keep_objects(Arc::clone(&groups));
        let cloned = state.clone();
        assert_eq!(cloned.stored_object_counts(), (1, 0));
        assert!(Arc::ptr_eq(&state.objects, &cloned.objects));
    }

    fn test_config(base: &str, region: Uuid) -> RegionConfig {
        let server = ferroan_config::Config::parse(&format!(
            "[Network]\nbind_ip_address=127.0.0.1\nhttp_listener_port=9000\n[GridService]\nGridServerURI={base}\n[PresenceService]\nPresenceServerURI={base}\n[GridUserService]\nGridUserServerURI={base}\n[Map]\nGenerateMaptiles=false\n"
        )).unwrap();
        let regions = ferroan_config::Config::parse(&format!(
            "[Fixture Region]\nRegionUUID={region}\nLocation=1000,1000\nInternalAddress=127.0.0.1\nInternalPort=9000\nExternalHostName=127.0.0.1\n"
        )).unwrap();
        RegionConfig::from_configs(&server, &regions).unwrap()
    }

    fn presence_reply(user: Uuid) -> String {
        format!(
            "<?xml version=\"1.0\"?><ServerResponse><result type=\"List\"><UserID>{user}</UserID></result></ServerResponse>"
        )
    }

    #[tokio::test]
    async fn captured_refusals_and_query_variants_match_csharp_http() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../fixtures/region-parity/exchanges.json"
        ))
        .unwrap();
        let exchanges = fixture["exchanges"].as_object().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let caps = CapsRegistry::new(&base).unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let state = SimulationState::new(&test_config(&base, region), caps.clone(), sender);
        let app = state.router().route(
            "/presence",
            axum::routing::post(|| async {
                "<ServerResponse><result>null</result></ServerResponse>"
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        for (name, exchange) in exchanges {
            let body = exchange["request"].to_string();
            let mut request = client
                .request(
                    reqwest::Method::from_bytes(exchange["method"].as_str().unwrap().as_bytes())
                        .unwrap(),
                    format!("{base}{}", exchange["path"].as_str().unwrap()),
                )
                .header(
                    "content-type",
                    exchange["request_content_type"].as_str().unwrap(),
                );
            if exchange["request_x_content_encoding"] == "gzip" {
                let mut compressed =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                compressed.write_all(body.as_bytes()).unwrap();
                request = request
                    .header("x-content-encoding", "gzip")
                    .body(compressed.finish().unwrap());
            } else {
                request = request.body(body);
            }
            let response = request.send().await.unwrap();
            assert_eq!(
                response.status().as_u16(),
                exchange["status"].as_u64().unwrap() as u16,
                "{name}"
            );
            assert_eq!(
                response.headers()["content-type"],
                exchange["content_type"].as_str().unwrap(),
                "{name}"
            );
            let actual: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
            assert_eq!(actual, exchange["response"], "{name}");
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(caps.register("/CAPS/denied-left-no-seed0000/").is_ok());
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn mismatched_or_failed_presence_never_admits_or_replaces() {
        // Source-derived variants of the captured unknown-session denial:
        // VerifyUserPresence also rejects a different UserID or connector error.
        for reply in [
            presence_reply(Uuid::from_u128(999)),
            "malformed service reply".to_owned(),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
            let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
            let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
            let caps = CapsRegistry::new(&base).unwrap();
            let (sender, mut receiver) = mpsc::channel(1);
            let state = SimulationState::new(&test_config(&base, region), caps, sender);
            state
                .accept(AgentFields::parse(&args, agent, region).unwrap())
                .unwrap();
            let mut current = receiver.recv().await.unwrap();
            let app = state.router().route(
                "/presence",
                axum::routing::post(move || {
                    let reply = reply.clone();
                    async move { reply }
                }),
            );
            let task = tokio::spawn(async move {
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap();
            });
            let response = reqwest::Client::new()
                .post(format!("{base}/agent/{agent}/"))
                .body(CREATE_REQUEST)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let actual: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
            let fixture: Value = serde_json::from_str(include_str!(
                "../../../fixtures/region-parity/exchanges.json"
            ))
            .unwrap();
            assert_eq!(actual, fixture["exchanges"]["unknown-session"]["response"]);
            assert!(current.is_current());
            assert!(matches!(
                current.cancelled.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            assert!(matches!(
                receiver.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
            task.abort();
            let _ = task.await;
        }
    }

    #[test]
    fn login_position_clamps_xy_but_preserves_z() {
        for (input, output) in [
            ([-0.1, -100.0, -10.0], [1.0, 1.0, -10.0]),
            ([256.0, 1024.0, 200.0], [255.0, 511.0, 200.0]),
            ([0.0, 511.5, 0.0], [0.0, 511.5, 0.0]),
        ] {
            let mut fields = synthetic_agent();
            fields.start_pos = input;
            fields.clamp_position([256, 512]);
            assert_eq!(fields.start_pos, output);
        }
    }

    #[tokio::test]
    async fn unsupported_semantics_use_refusal_envelopes_and_query_honors_capacity() {
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let mut state =
            SimulationState::new(&test_config("http://127.0.0.1:8003", region), caps, sender);
        let peer = "127.0.0.1:1234".parse().unwrap();
        for (key, value, reason) in [
            (
                "child",
                json!(true),
                "Child agent connections are not implemented",
            ),
            (
                "teleport_flags",
                json!("0"),
                "Only login agent connections are supported",
            ),
        ] {
            let mut request_args = args.clone();
            request_args.insert(key.into(), value);
            let request = Request::builder()
                .method("POST")
                .uri(format!("/agent/{agent}/"))
                .body(axum::body::Body::from(
                    Value::Object(request_args).to_string(),
                ))
                .unwrap();
            let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap(),
                json!({"reason":reason,"your_ip":"127.0.0.1"})
            );
        }
        let mut query = decode_body(QUERY_REQUEST.as_bytes(), false).unwrap();
        for via in [json!(false), json!("false"), json!(0)] {
            query.insert("viaTeleport".into(), via);
            let answer = query_access(&query, true).unwrap();
            assert_eq!(answer["success"], false);
            assert_eq!(answer["reason"], "Region crossings are not implemented");
        }
        state.root_limit = 1;
        state
            .accept(AgentFields::parse(&args, agent, region).unwrap())
            .unwrap();
        let current = receiver.recv().await.unwrap();
        assert_eq!(state.admission_refusal(agent, None), None);
        let other = Uuid::from_u128(999);
        let request = Request::builder()
            .method("QUERYACCESS")
            .uri(format!("/agent/{other}/{region}/"))
            .body(axum::body::Body::from(QUERY_REQUEST))
            .unwrap();
        let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
        let answer: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(answer["success"], false);
        assert_eq!(answer["reason"], "The region is full");
        assert!(ferroan_diag::render().contains("region.root-agents"));
        drop(current);
    }

    #[tokio::test]
    async fn configured_region_and_captured_create_agent_drive_arrival_send_path() {
        use ferroan_client::CircuitServer;
        use ferroan_protocol::msgs::{
            AgentMovementComplete, CompleteAgentMovement, UseCircuitCode,
        };
        use ferroan_protocol::{Msg, build_packet, parse_packet};
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let config = test_config("http://127.0.0.1:8003", region);
        let mut fields = AgentFields::parse(&args, agent, region).unwrap();
        fields.clamp_position(config.size);
        let expect = CircuitExpect {
            agent_id: agent,
            session_id: fields.session_id,
            circuit_code: fields.circuit_code,
            region_id: region,
        };
        let arrival = config.arrival(fields.start_pos);
        let server = CircuitServer::bind()
            .await
            .unwrap()
            .with_handshake(config.handshake.clone())
            .with_timing(config.circuit_timing)
            .unwrap()
            .with_throttle(config.circuit_throttle);
        let address = server.local_addr().unwrap();
        let task =
            tokio::spawn(async move { server.serve_one_with_arrival(expect, arrival).await });
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut use_circuit = UseCircuitCode::default();
        use_circuit.circuit_code.id = expect.agent_id;
        use_circuit.circuit_code.session_id = expect.session_id;
        use_circuit.circuit_code.code = expect.circuit_code;
        let mut bytes = Vec::new();
        build_packet(&use_circuit, 1, true, false, &[], &mut bytes);
        socket.send_to(&bytes, address).await.unwrap();
        let mut movement = CompleteAgentMovement::default();
        movement.agent_data.agent_id = expect.agent_id;
        movement.agent_data.session_id = expect.session_id;
        movement.agent_data.circuit_code = expect.circuit_code;
        bytes.clear();
        build_packet(&movement, 2, true, false, &[], &mut bytes);
        socket.send_to(&bytes, address).await.unwrap();
        let incoming = tokio::time::timeout(Duration::from_secs(2), async {
            let mut buffer = [0; 4096];
            for _ in 0..16 {
                let (size, _) = socket.recv_from(&mut buffer).await.unwrap();
                let packet = parse_packet(&buffer[..size]).unwrap();
                if packet.id == AgentMovementComplete::ID {
                    return packet;
                }
            }
            panic!("arrival missing from bounded login exchange")
        })
        .await;
        task.abort();
        let _ = task.await;
        let packet = incoming.unwrap();
        let mut actual = packet.decode::<AgentMovementComplete>().unwrap();
        let capture = include_bytes!("../../../fixtures/arrival/agent-movement-complete.bin");
        let captured = parse_packet(capture).unwrap();
        let expected = captured.decode::<AgentMovementComplete>().unwrap();
        assert_eq!(packet.flags, captured.flags);
        assert_eq!(packet.sequence, captured.sequence);
        assert_eq!(packet.acks, captured.acks);
        assert_eq!(actual.agent_data.agent_id, fields.agent_id);
        assert_eq!(actual.agent_data.session_id, fields.session_id);
        assert_eq!(actual.data.position.z, fields.start_pos[2]);
        assert_ne!(actual.data.position.z, expected.data.position.z);
        assert_eq!(
            actual.sim_data.channel_version,
            format!("{}\0", config.channel_version).as_bytes()
        );
        assert_ne!(
            actual.sim_data.channel_version,
            expected.sim_data.channel_version
        );
        // The HTTP and UDP fixtures use different identity folds. Retain the
        // captured UDP fold and timestamp normalization; only terrain-adjusted
        // Z (P2.0e) and the actual server product/version intentionally differ.
        actual.scrub();
        actual.data.timestamp = 0;
        actual.data.position.z = expected.data.position.z;
        actual.sim_data.channel_version = expected.sim_data.channel_version.clone();
        bytes.clear();
        build_packet(
            &actual,
            packet.sequence,
            true,
            false,
            &packet.acks,
            &mut bytes,
        );
        assert_eq!(bytes, capture);
    }

    #[tokio::test]
    async fn logout_ownership_tracks_generation_and_session_and_shutdown_refuses_admission() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            caps,
            sender,
        );
        state.accept(synthetic_agent()).unwrap();
        let old = receiver.recv().await.unwrap();
        state.accept(synthetic_agent()).unwrap();
        let same_session = receiver.recv().await.unwrap();
        assert!(!old.is_current());
        assert!(!old.may_logout_presence());
        assert!(same_session.is_current());
        let mut next = synthetic_agent();
        next.session_id = Uuid::from_u128(7);
        state.accept(next).unwrap();
        let current = receiver.recv().await.unwrap();
        assert!(old.may_logout_presence());
        assert!(same_session.may_logout_presence());
        assert!(!same_session.is_current());
        assert!(current.is_current());
        let guard = current.lock_lifecycle().await;
        assert!(
            same_session
                ._lease
                .dispatch_slot
                .lane
                .lifecycle
                .try_lock()
                .is_err()
        );
        state.stop_accepting();
        assert_eq!(
            state.accept(synthetic_agent()),
            Err("Logins to this region are disabled")
        );
        drop(old);
        drop(same_session);
        assert!(current.is_current());
        drop(current);
        drop(guard);
        assert!(state.sessions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn captured_csharp_login_requests_install_circuit_and_seed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let caps = CapsRegistry::new(&base).unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let expected = AgentFields::parse(&args, agent, region).unwrap();
        let state = SimulationState::new(&test_config(&base, region), caps.clone(), sender);
        let app = state.router().merge(caps.router()).route(
            "/presence",
            axum::routing::post(move || async move { presence_reply(agent) }),
        );
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let query = client
            .request(
                reqwest::Method::from_bytes(b"QUERYACCESS").unwrap(),
                format!("{base}/agent/{agent}/{region}/"),
            )
            .header("content-type", "application/json")
            .body(QUERY_REQUEST)
            .send()
            .await
            .unwrap();
        assert_eq!(query.status(), StatusCode::OK);
        // These fixtures retain captured values after identity normalization and
        // serde_json reserialization. This compares our normalized JSON output;
        // it does not establish the C# body's original whitespace/key order.
        assert_eq!(query.text().await.unwrap(), QUERY_RESPONSE.trim());
        let mut compressed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        compressed.write_all(CREATE_REQUEST.as_bytes()).unwrap();
        let response = client
            .post(format!("{base}/agent/{agent}/"))
            .header("content-type", "application/json")
            .header("x-content-encoding", "gzip")
            .body(compressed.finish().unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("connection").unwrap(), "close");
        let mut answer: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(answer["your_ip"], "127.0.0.1");
        answer["your_ip"] = Value::String("192.0.2.1".into());
        assert_eq!(answer.to_string(), CREATE_RESPONSE.trim());
        let mut accepted = receiver.recv().await.unwrap();
        assert_eq!(
            accepted.expect,
            CircuitExpect {
                agent_id: agent,
                session_id: expected.session_id,
                circuit_code: expected.circuit_code,
                region_id: region,
            }
        );
        assert_eq!(accepted.secure_session_id, expected.secure_session_id);
        assert_eq!(accepted.start_pos, [128.0, 128.0, 0.0]);
        let seed = format!("{base}/CAPS/{}0000/", expected.caps_path);
        let response = client
            .post(&seed)
            .header("content-type", "application/llsd+xml")
            .body("<llsd><array><string>EventQueueGet</string></array></llsd>")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.text().await.unwrap().contains("EventQueueGet"));

        // Replay the captured CreateAgent schema with a fresh session for the
        // same avatar. Its old lease remains alive while the new seed is used.
        let mut next_args = args.clone();
        let next_session = Uuid::from_u128(101);
        let next_caps = Uuid::from_u128(102).to_string();
        next_args.insert("session_id".into(), json!(next_session.to_string()));
        next_args.insert(
            "secure_session_id".into(),
            json!(Uuid::from_u128(103).to_string()),
        );
        next_args.insert("circuit_code".into(), json!("43"));
        next_args.insert("caps_path".into(), json!(next_caps));
        let response = client
            .post(format!("{base}/agent/{agent}/"))
            .header("content-type", "application/json")
            .body(Value::Object(next_args).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let answer: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(answer["success"], true);
        tokio::time::timeout(std::time::Duration::from_secs(1), accepted.replaced())
            .await
            .unwrap();
        let replacement = receiver.recv().await.unwrap();
        assert_eq!(replacement.expect.agent_id, agent);
        assert_eq!(replacement.expect.session_id, next_session);
        assert_eq!(replacement.expect.circuit_code, 43);
        drop(accepted);
        assert_eq!(
            client.post(&seed).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let next_seed = format!("{base}/CAPS/{next_caps}0000/");
        assert_eq!(
            client
                .post(&next_seed)
                .header("content-type", "application/llsd+xml")
                .body("<llsd><array><string>EventQueueGet</string></array></llsd>")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        drop(replacement);
        assert_eq!(
            client.post(&next_seed).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        task.abort();
        let _ = task.await;
    }

    #[test]
    fn captured_agent_rejects_mismatched_identity_and_invalid_capability_path() {
        let mut args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        assert!(AgentFields::parse(&args, Uuid::nil(), region).is_err());
        assert!(AgentFields::parse(&args, agent, Uuid::nil()).is_err());
        args.insert("caps_path".into(), Value::String("../escape".into()));
        assert!(AgentFields::parse(&args, agent, region).is_err());
    }

    fn far_destination() -> crate::children::FarDestination {
        crate::children::FarDestination {
            end: RegionEnd {
                server_uri: "http://127.0.0.1:9/".to_string(),
                region_id: Uuid::from_u128(99),
                name: "Away".to_string(),
                loc_x: 768,
                loc_y: 768,
            },
            size_x: 256,
            size_y: 256,
        }
    }

    fn synthetic_agent() -> AgentFields {
        AgentFields {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            secure_session_id: Uuid::from_u128(3),
            circuit_code: 42,
            caps_path: Uuid::from_u128(4).to_string(),
            start_pos: [128.0, 128.0, 25.0],
            first_name: "Synthetic".into(),
            last_name: "Test".into(),
            child: false,
            via_login: true,
            client_ip: String::new(),
            service_session_id: String::new(),
            teleport_flags: VIA_LOGIN,
            viewer: String::new(),
            channel: String::new(),
            mac: String::new(),
            id0: String::new(),
            service_urls: Vec::new(),
            appearance: None,
        }
    }

    #[tokio::test]
    async fn admission_lease_replaces_same_agent_and_cleans_closed_queue() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let mut state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            caps.clone(),
            sender,
        );
        state.root_limit = 1;
        state.accept(synthetic_agent()).unwrap();
        let mut accepted = receiver.recv().await.unwrap();
        assert_eq!(accepted.expect.circuit_code, 42);
        let held = format!("/CAPS/{}0000/", synthetic_agent().caps_path);
        let mut other = synthetic_agent();
        other.agent_id = Uuid::from_u128(6);
        assert_eq!(state.accept(other), Err("The region is full"));
        assert!(caps.register(&held).is_err());
        let mut next = synthetic_agent();
        next.session_id = Uuid::from_u128(7);
        next.circuit_code = 43;
        next.caps_path = Uuid::from_u128(8).to_string();
        state.accept(next).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), accepted.replaced())
            .await
            .unwrap();
        let replacement = receiver.recv().await.unwrap();
        assert_eq!(replacement.expect.session_id, Uuid::from_u128(7));
        assert_eq!(replacement.expect.circuit_code, 43);
        drop(accepted);
        // An old generation's cleanup must not release the new reservation.
        let replacement_path = format!("/CAPS/{}0000/", Uuid::from_u128(8));
        assert!(caps.register(&replacement_path).is_err());
        assert!(
            state
                .sessions
                .lock()
                .unwrap()
                .contains_key(&synthetic_agent().agent_id)
        );
        drop(replacement);
        assert!(state.sessions.lock().unwrap().is_empty());
        state.accept(synthetic_agent()).unwrap();
        drop(receiver.recv().await.unwrap());
        drop(receiver);
        assert!(state.accept(synthetic_agent()).is_err());
        assert!(state.sessions.lock().unwrap().is_empty());
        caps.register("/CAPS/after-closed-queue0000/").unwrap();
    }

    #[tokio::test]
    async fn accept_reserves_the_login_until_the_accepted_agent_is_dropped() {
        let region = Uuid::from_u128(5);
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let state =
            SimulationState::new(&test_config("http://127.0.0.1:8003", region), caps, sender);
        let circuit = CircuitServer::bind().await.unwrap();
        assert_eq!(circuit.reserved_logins(), 0);
        state.bind_circuit(circuit.clone());
        state.accept(synthetic_agent()).unwrap();
        let accepted = receiver.recv().await.unwrap();
        assert_eq!(circuit.reserved_logins(), 1);
        drop(accepted);
        assert_eq!(circuit.reserved_logins(), 0);
    }

    #[tokio::test]
    async fn a_demoted_root_keeps_caps_and_frees_a_root_slot() {
        let caps = CapsRegistry::new("http://127.0.0.1:9").unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        let mut state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps.clone(),
            sender,
        );
        state.root_limit = 1;
        state.accept(synthetic_agent()).unwrap();
        let agent = receiver.recv().await.unwrap();
        agent.mark_root();
        let path = format!("/CAPS/{}0000/", synthetic_agent().caps_path);
        assert!(
            caps.enqueue_for(agent.expect.agent_id, ferroan_llsd::Llsd::new_map())
                .is_ok()
        );
        state.demote_child(agent.expect.agent_id).unwrap();
        let lines = state.meters().diag_lines();
        assert!(lines.contains("root_agents: 0\n"));
        assert!(lines.contains("child_agents: 1\n"));
        assert!(lines.contains("circuits: 1\n"));
        assert!(
            caps.enqueue_for(agent.expect.agent_id, ferroan_llsd::Llsd::new_map())
                .is_ok()
        );
        assert!(!agent.still_root());
        let mut extra = synthetic_agent();
        extra.agent_id = Uuid::from_u128(6);
        extra.session_id = Uuid::from_u128(7);
        extra.circuit_code = 43;
        extra.caps_path = Uuid::from_u128(8).to_string();
        state.accept(extra).unwrap();
        state.restore_root(agent.expect.agent_id);
        assert!(agent.still_root());
        assert!(state.meters().diag_lines().contains("root_agents: 1\n"));
        state.demote_child(agent.expect.agent_id).unwrap();
        state.close_departed(agent.expect.agent_id, &far_destination());
        assert!(
            caps.enqueue_for(synthetic_agent().agent_id, ferroan_llsd::Llsd::new_map())
                .is_err()
        );
        assert!(!agent.still_root());
        assert!(state.meters().diag_lines().contains("child_agents: 0\n"));
        drop(agent);
        assert!(caps.register(&path).is_ok());
    }

    #[tokio::test]
    async fn a_release_delete_wakes_the_wait_and_a_miss_is_bounded() {
        let region = Uuid::from_u128(5);
        let agent = Uuid::from_u128(1);
        let caps = CapsRegistry::new("http://127.0.0.1:9").unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        let state = SimulationState::new(&test_config("http://127.0.0.1:9", region), caps, sender);
        let started = std::time::Instant::now();
        state
            .wait_release_for(agent, Duration::from_millis(30))
            .await;
        assert!(started.elapsed() < Duration::from_secs(2));
        state.arm_release(agent);
        let waiting = state.clone();
        let parked = tokio::spawn(async move {
            waiting.wait_release(agent).await;
        });
        let app = state.clone().router();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http = format!("http://{}", listener.local_addr().unwrap());
        let serving = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let response = client
            .delete(format!("{http}/agent/{agent}/{region}/release/"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert_eq!(body, format!("OpenSim agent {agent}"));
        tokio::time::timeout(Duration::from_secs(2), parked)
            .await
            .unwrap()
            .unwrap();
        serving.abort();
    }

    #[tokio::test]
    async fn two_roots_keep_separate_caps_and_replacement_stays_inside_the_bound() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(2);
        let mut state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            caps.clone(),
            sender,
        );
        state.root_limit = 2;
        state.accept(synthetic_agent()).unwrap();
        let mut second = synthetic_agent();
        second.agent_id = Uuid::from_u128(6);
        second.session_id = Uuid::from_u128(7);
        second.secure_session_id = Uuid::from_u128(8);
        second.circuit_code = 43;
        second.caps_path = Uuid::from_u128(9).to_string();
        state.accept(second).unwrap();
        let mut first = receiver.recv().await.unwrap();
        let other = receiver.recv().await.unwrap();
        assert!(first.is_current());
        assert!(other.is_current());
        let lines = state.meters().diag_lines();
        assert!(lines.contains("circuits: 2\n"));
        assert!(lines.contains("root_agents: 0\n"));
        first.mark_root();
        other.mark_root();
        assert!(state.meters().diag_lines().contains("root_agents: 2\n"));
        let mut extra = synthetic_agent();
        extra.agent_id = Uuid::from_u128(11);
        extra.session_id = Uuid::from_u128(12);
        extra.circuit_code = 44;
        extra.caps_path = Uuid::from_u128(13).to_string();
        assert_eq!(state.accept(extra), Err("The region is full"));
        assert!(first.is_current());
        assert!(other.is_current());
        let mut again = synthetic_agent();
        again.session_id = Uuid::from_u128(14);
        again.circuit_code = 45;
        again.caps_path = Uuid::from_u128(15).to_string();
        state.accept(again).unwrap();
        tokio::time::timeout(Duration::from_secs(1), first.replaced())
            .await
            .unwrap();
        let replacement = receiver.recv().await.unwrap();
        assert!(!first.is_current());
        assert!(other.is_current());
        assert!(replacement.is_current());
        let lines = state.meters().diag_lines();
        assert!(lines.contains("circuits: 2\n"));
        assert!(lines.contains("root_agents: 1\n"));
        let first_path = format!("/CAPS/{}0000/", synthetic_agent().caps_path);
        let second_path = format!("/CAPS/{}0000/", Uuid::from_u128(9));
        assert!(caps.register(&first_path).is_ok());
        assert!(caps.register(&second_path).is_err());
        drop(first);
        assert!(other.is_current());
        assert!(state.meters().diag_lines().contains("circuits: 2\n"));
        drop(other);
        let lines = state.meters().diag_lines();
        assert!(lines.contains("root_agents: 0\n"));
        assert!(lines.contains("circuits: 1\n"));
        assert!(replacement.is_current());
        drop(replacement);
        assert!(state.meters().diag_lines().contains("circuits: 0\n"));
    }

    fn refuse_count(snapshot: &str, limit: &str) -> u64 {
        let prefix = format!("ferroan_limit_hits_total{{limit=\"{limit}\",action=\"refuse\"}} ");
        snapshot
            .lines()
            .find_map(|line| line.strip_prefix(&prefix)?.parse().ok())
            .unwrap_or(0)
    }

    #[test]
    fn dispatch_caps_replacements_and_leaves_room_for_other_agents() {
        let slots = Arc::new(Mutex::new(DispatchSlots::default()));
        let agent = Uuid::from_u128(1);
        let before = ferroan_diag::metrics();
        let mut held = Vec::new();
        for _ in 0..=ADMISSION_QUEUE {
            held.push(DispatchSlot::reserve(&slots, agent).unwrap());
        }
        assert!(DispatchSlot::reserve(&slots, agent).is_err());
        for n in 2..=ADMISSION_QUEUE {
            held.push(DispatchSlot::reserve(&slots, Uuid::from_u128(n as u128)).unwrap());
        }
        assert_eq!(held.len(), DISPATCH_CAPACITY);
        assert!(DispatchSlot::reserve(&slots, Uuid::from_u128(999)).is_err());
        let after = ferroan_diag::metrics();
        for limit in [
            ferroan_diag::REGION_DISPATCH,
            ferroan_diag::REGION_OVERLAPPING_LOGINS,
        ] {
            assert!(refuse_count(&after, limit) > refuse_count(&before, limit));
        }
        drop(held);
        let state = slots.lock().unwrap();
        assert_eq!(state.total, 0);
        assert!(state.agents.is_empty());
    }

    #[test]
    fn a_full_child_share_leaves_room_for_a_root_login() {
        let (sender, mut receiver) = dispatch_queue();
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            CapsRegistry::new("http://127.0.0.1:8002").unwrap(),
            sender,
        );
        let child = |n: u32| {
            let mut fields = other_agent(u128::from(n), n);
            fields.child = true;
            fields.via_login = false;
            fields.teleport_flags = 0;
            fields
        };
        for n in 10..10 + CHILD_DISPATCH_CAPACITY as u32 {
            state.accept(child(n)).unwrap();
        }
        let extra = 1000;
        let busy = "Region is busy completing a login or logout";
        let before = refuse_count(
            &ferroan_diag::metrics(),
            ferroan_diag::REGION_DISPATCH_CHILDREN,
        );
        assert_eq!(state.accept(child(extra)), Err(busy));
        assert!(
            refuse_count(
                &ferroan_diag::metrics(),
                ferroan_diag::REGION_DISPATCH_CHILDREN
            ) > before
        );
        // All the children are still queued. Neither their slots nor the queue
        // prevent this ordinary root from being accepted.
        state.accept(synthetic_agent()).unwrap();
        let promoted = receiver.try_recv().unwrap();
        promoted.mark_root();
        let place = state.reserve_child_place().unwrap();
        state.demote_with_place(promoted.expect.agent_id, place);
        assert_eq!(state.accept(child(extra)), Err(busy));
        state.restore_root(promoted.expect.agent_id);
        state.accept(child(extra)).unwrap();

        // A removed child remains charged until its worker has finished cleanup.
        let leaving = receiver.try_recv().unwrap();
        state.close_departed(leaving.expect.agent_id, &far_destination());
        assert_eq!(state.accept(child(extra + 1)), Err(busy));
        drop(leaving);
        state.accept(child(extra + 1)).unwrap();
    }

    fn other_agent(id: u128, code: u32) -> AgentFields {
        let mut fields = synthetic_agent();
        fields.agent_id = Uuid::from_u128(id);
        fields.session_id = Uuid::from_u128(id.saturating_add(1_000));
        fields.secure_session_id = Uuid::from_u128(id.saturating_add(2_000));
        fields.circuit_code = code;
        fields.caps_path = Uuid::from_u128(id.saturating_add(3_000)).to_string();
        fields
    }

    #[test]
    fn estate_manager_enters_a_full_region_and_an_ordinary_account_does_not() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, receiver) = mpsc::channel(4);
        let mut state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            caps,
            sender,
        );
        state.root_limit = 1;
        state.bypass_limit = 1;
        let manager = Uuid::from_u128(20);
        let ordinary = Uuid::from_u128(21);
        let god = Uuid::from_u128(22);
        state.staff.set_managers(vec![manager]);
        state.staff.set_allow_grid_gods(true);
        state.staff.note_level(god, crate::staff::GOD_USER_LEVEL);
        // The queued accepts keep both sessions. Dropping one would free its seat.
        state.accept(synthetic_agent()).unwrap();
        state.accept(other_agent(manager.as_u128(), 50)).unwrap();
        let _held = receiver;
        let agents_before = refuse_count(&ferroan_diag::metrics(), "region.root-agents");
        assert_eq!(
            state.accept(other_agent(ordinary.as_u128(), 51)),
            Err("The region is full")
        );
        assert!(refuse_count(&ferroan_diag::metrics(), "region.root-agents") > agents_before);
        let bypass_before = refuse_count(&ferroan_diag::metrics(), "region.root-bypass");
        assert_eq!(
            state.accept(other_agent(god.as_u128(), 52)),
            Err("The region is full")
        );
        assert_eq!(
            refuse_count(&ferroan_diag::metrics(), "region.root-bypass"),
            bypass_before + 1
        );
    }

    // Keep socket I/O on the real clock. Advance only the interval under test.
    async fn advance_clock(elapsed: Duration) {
        tokio::time::pause();
        tokio::time::advance(elapsed).await;
        tokio::time::resume();
    }

    #[tokio::test]
    async fn a_demoted_account_loses_the_bypass_after_its_cache_lifetime() {
        for (local_field, ttl) in [("", 3600), ("<LocalToGrid>False</LocalToGrid>", 172800)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let requests = hits.clone();
            let app = axum::Router::new().route("/accounts", axum::routing::post(move || {
                let level = if requests.fetch_add(1, Ordering::Relaxed) == 0 { 200 } else { 199 };
                async move { format!("<ServerResponse><result type=\"List\"><UserLevel>{level}</UserLevel>{local_field}</result></ServerResponse>") }
            }));
            let service = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let mut config = test_config(&base, Uuid::from_u128(5));
            config.user_account_server_uri = base.clone();
            config.allow_grid_gods = true;
            let (sender, _receiver) = mpsc::channel(1);
            let mut state =
                SimulationState::new(&config, CapsRegistry::new(&base).unwrap(), sender);
            state.root_limit = 0;
            let agent = Uuid::from_u128(80);
            let level = state.lookup_level(agent).await;
            assert_eq!(state.admission_refusal(agent, level), None);
            advance_clock(Duration::from_secs(ttl - 1)).await;
            assert_eq!(state.lookup_level(agent).await, Some(200));
            assert_eq!(hits.load(Ordering::Relaxed), 1);

            // The first sweep after the lifetime forces another getaccount.
            advance_clock(Duration::from_secs(2)).await;
            let level = state.lookup_level(agent).await;
            assert_eq!(level, Some(199));
            assert_eq!(
                state.admission_refusal(agent, level),
                Some("The region is full")
            );
            assert_eq!(hits.load(Ordering::Relaxed), 2);
            service.abort();
        }
    }

    #[tokio::test]
    async fn getaccount_level_200_admits_a_grid_god_to_a_full_region() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/accounts",
            axum::routing::post(|| async {
                "<?xml version=\"1.0\"?><ServerResponse><result type=\"List\"><UserLevel>200</UserLevel></result></ServerResponse>"
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let region = Uuid::from_u128(5);
        let mut config = test_config(&format!("http://{addr}"), region);
        config.user_account_server_uri = format!("http://{addr}");
        config.allow_grid_gods = true;
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, _receiver) = mpsc::channel(4);
        let mut state = SimulationState::new(&config, caps, sender);
        state.root_limit = 1;
        state.bypass_limit = 1;
        state.accept(synthetic_agent()).unwrap();
        let god = Uuid::from_u128(80);
        let level = state.lookup_level(god).await;
        assert_eq!(level, Some(200));
        assert_eq!(state.admission_refusal(god, level), None);
        state
            .accept_with_level(other_agent(god.as_u128(), 80), level)
            .unwrap();
    }

    #[tokio::test]
    async fn a_closed_session_forgets_its_redaction_secrets() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            caps,
            sender,
        );
        let mut fields = synthetic_agent();
        fields.agent_id = Uuid::from_u128(0x13c2_0100);
        fields.session_id = Uuid::from_u128(0x13c2_0101);
        fields.secure_session_id = Uuid::from_u128(0x13c2_0102);
        fields.circuit_code = 424242;
        fields.caps_path = Uuid::from_u128(0x13c2_0103).to_string();
        let session = fields.session_id.to_string();
        let secure = fields.secure_session_id.to_string();
        let path = format!("/CAPS/{}0000/", fields.caps_path);
        state.accept(fields).unwrap();
        let mut first = receiver.recv().await.unwrap();
        let hidden = ferroan_log::redact(&format!("{session} {secure} 424242 {path}"));
        assert!(!hidden.contains(&session));
        assert!(!hidden.contains(&secure));
        assert!(!hidden.contains("424242"));
        assert!(!hidden.contains(&path));

        let mut next = synthetic_agent();
        next.agent_id = Uuid::from_u128(0x13c2_0100);
        next.session_id = Uuid::parse_str(&session).unwrap();
        next.secure_session_id = Uuid::from_u128(0x13c2_0104);
        next.circuit_code = 424243;
        next.caps_path = Uuid::from_u128(0x13c2_0105).to_string();
        let next_secure = next.secure_session_id.to_string();
        let next_path = format!("/CAPS/{}0000/", next.caps_path);
        state.accept(next).unwrap();
        tokio::time::timeout(Duration::from_secs(1), first.replaced())
            .await
            .unwrap();
        let replacement = receiver.recv().await.unwrap();
        drop(first);
        let after_old = ferroan_log::redact(&format!(
            "{session} {secure} 424242 {path} {next_secure} 424243 {next_path}"
        ));
        assert!(!after_old.contains(&session));
        assert!(after_old.contains(&secure));
        assert!(after_old.contains("424242"));
        assert!(after_old.contains(&path));
        assert!(!after_old.contains(&next_secure));
        assert!(!after_old.contains("424243"));
        assert!(!after_old.contains(&next_path));
        drop(replacement);
        let gone = ferroan_log::redact(&format!("{session} {next_secure} 424243 {next_path}"));
        assert!(gone.contains(&session));
        assert!(gone.contains(&next_secure));
        assert!(gone.contains("424243"));
        assert!(gone.contains(&next_path));
    }

    #[tokio::test]
    async fn a_full_or_closed_dispatch_queue_preserves_the_current_login() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            caps.clone(),
            sender,
        );
        state.accept(synthetic_agent()).unwrap();
        let held = format!("/CAPS/{}0000/", synthetic_agent().caps_path);
        // First login is still queued; reserving a replacement must fail
        // without changing its capabilities or sending a cancellation.
        assert_eq!(
            state.accept(synthetic_agent()),
            Err("Region is not accepting logins")
        );
        let mut accepted = receiver.recv().await.unwrap();
        assert!(matches!(
            accepted.cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(caps.register(&held).is_err());
        receiver.close();
        assert_eq!(
            state.accept(synthetic_agent()),
            Err("Region is not accepting logins")
        );
        assert!(matches!(
            accepted.cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(caps.register(&held).is_err());
        drop(accepted);
        caps.register("/CAPS/after-drop0000/").unwrap();
    }

    #[test]
    fn malformed_and_oversize_bodies_are_rejected() {
        assert!(matches!(
            decode_body(b"[]", false),
            Err(SimulationError::Json)
        ));
        assert!(matches!(
            decode_body(b"not gzip", true),
            Err(SimulationError::Compression)
        ));
        assert!(matches!(
            decode_body(&vec![b' '; MAX_BODY + 1], false),
            Err(SimulationError::BodyLimit)
        ));
        let mut compressed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        compressed.write_all(&vec![b' '; MAX_BODY + 1]).unwrap();
        assert!(matches!(
            decode_body(&compressed.finish().unwrap(), true),
            Err(SimulationError::BodyLimit)
        ));
    }

    #[test]
    fn protocol_negotiation_refuses_non_overlapping_versions() {
        let args = json!({
            "simulation_service_supported_min":0.9,
            "simulation_service_supported_max":1.0,
            "simulation_service_accepted_min":0.3,
            "simulation_service_accepted_max":0.8,
        });
        let answer = query_access(args.as_object().unwrap(), true).unwrap();
        assert_eq!(answer["success"], false);
        assert_eq!(
            answer["reason"],
            "Your region provide protocol versions 0.9 - 1 and we accept only 0.3 - 0.8. No version overlap."
        );
        let answer = query_access(&Map::new(), true).unwrap();
        assert_eq!(answer["success"], false);
        assert_eq!(
            answer["reason"],
            "Your region is running a old version of opensim no longer supported. Consider updating it"
        );
    }

    fn login_robustness() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/pin3/login-robustness.json")).unwrap()
    }

    fn presence_server(
        reply: String,
        delay: Duration,
        hits: Arc<std::sync::atomic::AtomicUsize>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = axum::Router::new().route(
            "/presence",
            axum::routing::post(move || {
                let reply = reply.clone();
                let hits = hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::Relaxed);
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    reply
                }
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (base, task)
    }

    async fn post_create(base: &str, body: &str) -> (Value, std::time::Duration) {
        let started = std::time::Instant::now();
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap()
            .post(base)
            .header("content-type", "application/json")
            .body(body.to_owned())
            .send()
            .await
            .expect("CreateAgent exceeded its budget");
        let elapsed = started.elapsed();
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        (value, elapsed)
    }

    #[test]
    fn create_agent_budget_stays_under_the_robust_connector_timeout() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            caps,
            sender,
        );
        assert_eq!(state.create_agent_budget, Duration::from_secs(25));
    }

    #[tokio::test]
    async fn the_account_lookup_and_presence_share_the_create_agent_deadline() {
        for exhaust_in_lookup in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let account_started = Arc::new(Notify::new());
            let release_account = Arc::new(Notify::new());
            let presence_started = Arc::new(Notify::new());
            let started = account_started.clone();
            let release = release_account.clone();
            let verifying = presence_started.clone();
            let app = axum::Router::new()
                .route("/accounts", axum::routing::post(move || {
                    let started = started.clone();
                    let release = release.clone();
                    async move {
                        started.notify_one();
                        release.notified().await;
                        "<ServerResponse><result type=\"List\"><UserLevel>200</UserLevel></result></ServerResponse>"
                    }
                }))
                .route("/presence", axum::routing::post(move || {
                    verifying.notify_one();
                    std::future::pending::<String>()
                }));
            let service = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let region = Uuid::from_u128(5);
            let mut config = test_config(&base, region);
            config.allow_grid_gods = true;
            config.user_account_server_uri = base.clone();
            let (sender, _receiver) = mpsc::channel(1);
            let mut state =
                SimulationState::new(&config, CapsRegistry::new(&base).unwrap(), sender);
            state.root_limit = 0;
            state.create_agent_budget = Duration::from_secs(3);
            let request = Request::builder()
                .method("POST")
                .uri(format!("/agent/{}/", Uuid::from_u128(1)))
                .body(axum::body::Body::from(create_body_for(
                    synthetic_agent(),
                    region,
                )))
                .unwrap();
            let response = tokio::spawn(handle(
                State(state),
                ConnectInfo("127.0.0.1:1234".parse().unwrap()),
                request,
            ));
            let guard = Duration::from_secs(1);
            tokio::time::timeout(guard, account_started.notified())
                .await
                .unwrap();
            if exhaust_in_lookup {
                advance_clock(Duration::from_millis(3100)).await;
            } else {
                advance_clock(Duration::from_secs(2)).await;
                release_account.notify_one();
                tokio::time::timeout(guard, presence_started.notified())
                    .await
                    .unwrap();
                advance_clock(Duration::from_millis(1100)).await;
            }
            let response = tokio::time::timeout(guard, response)
                .await
                .unwrap()
                .unwrap();
            let bytes = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            if exhaust_in_lookup {
                assert_eq!(
                    result["reason"],
                    "Region is busy completing a login or logout"
                );
            } else {
                assert!(
                    result["reason"]
                        .as_str()
                        .unwrap()
                        .starts_with("Failed to verify user presence")
                );
            }
            service.abort();
        }
    }

    #[tokio::test]
    async fn delayed_presence_create_agent_finishes_inside_the_robust_timeout() {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (base, task) = presence_server(
            "<ServerResponse><result>null</result></ServerResponse>".into(),
            Duration::from_secs(30),
            hits.clone(),
        );
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        let mut state = SimulationState::new(&test_config(&base, region), caps, sender);
        state.create_agent_budget = Duration::from_millis(200);
        let app = state.router();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http = format!("http://{}", listener.local_addr().unwrap());
        let serving = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let (value, elapsed) = post_create(&format!("{http}/agent/{agent}/"), CREATE_REQUEST).await;
        assert!(
            elapsed < Duration::from_secs(5),
            "CreateAgent took {elapsed:?}"
        );
        assert_ne!(value.get("success"), Some(&Value::Bool(true)));
        assert!(
            value["reason"]
                .as_str()
                .unwrap()
                .contains("Failed to verify user presence")
        );
        assert!(hits.load(Ordering::Relaxed) >= 1);
        serving.abort();
        task.abort();
    }

    #[tokio::test]
    async fn exhausted_create_agent_budget_refuses_busy_without_closing_a_root() {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (base, task) = presence_server(
            presence_reply(Uuid::from_u128(1)),
            Duration::ZERO,
            hits.clone(),
        );
        let region = Uuid::from_u128(5);
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let mut state = SimulationState::new(&test_config(&base, region), caps, sender);
        state.create_agent_budget = Duration::ZERO;
        state.accept(synthetic_agent()).unwrap();
        let mut root = receiver.recv().await.unwrap();
        root.mark_root();
        let peer = "127.0.0.1:1234".parse().unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/agent/{}/", Uuid::from_u128(1)))
            .body(axum::body::Body::from(create_body_for(
                synthetic_agent(),
                region,
            )))
            .unwrap();
        let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
        let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value["reason"],
            "Region is busy completing a login or logout"
        );
        assert_eq!(hits.load(Ordering::Relaxed), 0);
        assert!(matches!(
            root.cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        state.create_agent_budget = Duration::from_millis(200);
        let (release_tx, release_rx) = oneshot::channel();
        let lifecycle = root._lease.dispatch_slot.lane.lifecycle.clone();
        let holder = tokio::spawn(async move {
            let guard = lifecycle.lock_owned().await;
            let _ = release_rx.await;
            drop(guard);
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if root._lease.dispatch_slot.lane.lifecycle.try_lock().is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("lifecycle lock was not taken");
        let started = std::time::Instant::now();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/agent/{}/", Uuid::from_u128(1)))
            .body(axum::body::Body::from(create_body_for(
                synthetic_agent(),
                region,
            )))
            .unwrap();
        let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
        assert!(started.elapsed() < Duration::from_secs(2));
        let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value["reason"],
            "Region is busy completing a login or logout"
        );
        assert_eq!(hits.load(Ordering::Relaxed), 0);
        assert!(matches!(
            root.cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let _ = release_tx.send(());
        holder.await.unwrap();
        task.abort();
    }

    fn create_body_for(fields: AgentFields, region: Uuid) -> String {
        json!({
            "agent_id": fields.agent_id.to_string(),
            "session_id": fields.session_id.to_string(),
            "secure_session_id": fields.secure_session_id.to_string(),
            "circuit_code": fields.circuit_code.to_string(),
            "caps_path": fields.caps_path,
            "start_pos": format!("<{}, {}, {}>", fields.start_pos[0], fields.start_pos[1], fields.start_pos[2]),
            "first_name": fields.first_name,
            "last_name": fields.last_name,
            "destination_uuid": region.to_string(),
            "destination_name": "Fixture Region",
            "teleport_flags": "128",
            "child": false,
        })
        .to_string()
    }

    #[tokio::test]
    async fn admission_is_refused_before_presence_verify() {
        let fixture = login_robustness();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (base, task) = presence_server(
            presence_reply(Uuid::from_u128(1)),
            Duration::ZERO,
            hits.clone(),
        );
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        let state = SimulationState::new(&test_config(&base, region), caps, sender);
        state.stop_accepting();
        let peer = "127.0.0.1:1234".parse().unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/agent/{agent}/"))
            .body(axum::body::Body::from(CREATE_REQUEST))
            .unwrap();
        let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
        let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["reason"], fixture["logins_disabled"]["reason"]);
        assert_eq!(hits.load(Ordering::Relaxed), 0);

        let (sender, mut occupied) = mpsc::channel(1);
        let mut open = SimulationState::new(
            &test_config(&base, region),
            CapsRegistry::new("http://127.0.0.1:8002").unwrap(),
            sender,
        );
        open.root_limit = 1;
        let mut other = synthetic_agent();
        other.agent_id = Uuid::from_u128(9);
        open.accept(other).unwrap();
        let _occupied = occupied.recv().await.unwrap();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/agent/{agent}/"))
            .body(axum::body::Body::from(CREATE_REQUEST))
            .unwrap();
        let response = handle(State(open), ConnectInfo(peer), request).await;
        let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["reason"], "The region is full");
        assert_eq!(hits.load(Ordering::Relaxed), 0);
        task.abort();
    }

    #[tokio::test]
    async fn refused_post_matches_csharp_json_403() {
        let fixture = login_robustness();
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        let state =
            SimulationState::new(&test_config("http://127.0.0.1:8003", region), caps, sender);
        for case in fixture["refused_post"].as_array().unwrap() {
            let peer: SocketAddr = if case["name"] == "untrusted" {
                "192.0.2.1:9".parse().unwrap()
            } else {
                "127.0.0.1:9".parse().unwrap()
            };
            let mut builder = Request::builder()
                .method("POST")
                .uri(format!("/agent/{agent}/"));
            if case["name"] == "script-marker" {
                builder = builder.header("x-secondlife-shard", "OpenSim");
            }
            let request = builder
                .body(axum::body::Body::from(CREATE_REQUEST))
                .unwrap();
            let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
            assert_eq!(
                response.status().as_u16(),
                case["status"].as_u64().unwrap() as u16
            );
            assert_eq!(
                response.headers()["content-type"],
                case["content_type"].as_str().unwrap()
            );
            assert_eq!(response.headers()["connection"], "close");
            assert_eq!(case["keep_alive"], false);
            let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
            assert_eq!(body.len(), case["body_length"].as_u64().unwrap() as usize);
        }
    }

    #[tokio::test]
    async fn a_child_create_does_not_take_a_root_slot() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        let mut state = SimulationState::new(
            &test_config("http://127.0.0.1:8003", Uuid::from_u128(5)),
            caps.clone(),
            sender,
        );
        state.root_limit = 1;
        state.accept(synthetic_agent()).unwrap();
        let mut root = receiver.recv().await.unwrap();
        root.mark_root();
        let mut child = other_agent(6, 43);
        child.child = true;
        child.via_login = false;
        child.teleport_flags = 0;
        state.accept(child).unwrap();
        let accepted = receiver.recv().await.unwrap();
        assert!(accepted.return_slot.is_child());
        assert!(!accepted.still_root());
        assert!(state.meters().diag_lines().contains("root_agents: 1\n"));
        assert!(state.meters().diag_lines().contains("child_agents: 1\n"));
        assert_eq!(state.accept(other_agent(7, 44)), Err("The region is full"));
        let mut again = synthetic_agent();
        again.child = true;
        again.via_login = false;
        again.teleport_flags = 0;
        again.caps_path = Uuid::from_u128(99).to_string();
        state
            .reuse_child(&again, state.reserve_child_place().unwrap())
            .unwrap();
        assert!(matches!(
            root.cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(!root.still_root());
        assert!(
            caps.register(&format!("/CAPS/{}0000/", synthetic_agent().caps_path))
                .is_err()
        );
        assert!(
            caps.register(&format!("/CAPS/{}0000/", Uuid::from_u128(99)))
                .is_err()
        );
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_return_create_keeps_the_circuit_and_update_waits_for_root() {
        let region = Uuid::from_u128(5);
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(2);
        let mut state = SimulationState::new(
            &test_config("http://127.0.0.1:9", region),
            caps.clone(),
            sender,
        );
        state.root_limit = 0;
        let peer = "127.0.0.1:1234".parse().unwrap();
        let login = synthetic_agent();
        let agent = login.agent_id;
        let session = login.session_id;
        // A closed region refuses the child before presence.
        state.stop_accepting();
        let closed = handle(
            State(state.clone()),
            ConnectInfo(peer),
            child_request(&login, region, "closed-seed"),
        )
        .await;
        let closed_body = to_bytes(closed.into_body(), MAX_BODY).await.unwrap();
        let closed_value: Value = serde_json::from_slice(&closed_body).unwrap();
        assert_eq!(closed_value["reason"], "Logins to this region are disabled");
        state.admission_open.store(true, Ordering::Release);
        // A full region still reaches presence for a new child, and still
        // refuses a login before that.
        let full_login = handle(
            State(state.clone()),
            ConnectInfo(peer),
            Request::builder()
                .method("POST")
                .uri(format!("/agent/{agent}/"))
                .body(axum::body::Body::from(create_body_for(
                    login.clone(),
                    region,
                )))
                .unwrap(),
        )
        .await;
        let full_body = to_bytes(full_login.into_body(), MAX_BODY).await.unwrap();
        let full_value: Value = serde_json::from_slice(&full_body).unwrap();
        assert_eq!(full_value["reason"], "The region is full");
        state.root_limit = 1;
        state.accept(login.clone()).unwrap();
        let mut accepted = receiver.recv().await.unwrap();
        accepted.mark_root();
        let response = handle(
            State(state.clone()),
            ConnectInfo(peer),
            child_request(&login, region, "return-seed"),
        )
        .await;
        assert!(response.headers().get("connection").is_none());
        let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["success"], true);
        assert!(matches!(
            accepted.cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(receiver.try_recv().is_err());
        assert!(!accepted.still_root());
        assert!(
            caps.register(&format!("/CAPS/{}0000/", login.caps_path))
                .is_err()
        );
        assert!(caps.register("/CAPS/return-seed0000/").is_err());

        let wrong = put_agent(
            &state,
            peer,
            &json!({
                "message_type": "AgentData",
                "agent_uuid": agent.to_string(),
                "session_uuid": Uuid::from_u128(8).to_string(),
                "region_id": region.to_string(),
                "position": "<40, 40, 40>",
                "at_axis": "<0, 1, 0>",
                "wait_for_root": false,
            }),
        )
        .await;
        assert_eq!(wrong, "False");
        assert_eq!(accepted.return_slot.position(), [128.0, 128.0, 30.0]);

        let moved = put_agent(
            &state,
            peer,
            &json!({
                "message_type": "AgentPosition",
                "agent_uuid": agent.to_string(),
                "session_uuid": session.to_string(),
                "position": "<40, 40, 40>",
            }),
        )
        .await;
        assert_eq!(moved, "True");
        assert_eq!(accepted.return_slot.position(), [40.0, 40.0, 40.0]);

        let nil_origin = put_agent(
            &state,
            peer,
            &json!({
                "message_type": "AgentData",
                "agent_uuid": agent.to_string(),
                "session_uuid": session.to_string(),
                "region_id": Uuid::nil().to_string(),
                "position": "<1, 2, 3>",
                "at_axis": "<0, 1, 0>",
                "wait_for_root": true,
            }),
        )
        .await;
        assert_eq!(nil_origin, "False");
        assert_eq!(accepted.return_slot.position(), [40.0, 40.0, 40.0]);

        state.restore_root(agent);
        let rooted = put_agent(
            &state,
            peer,
            &json!({
                "message_type": "AgentData",
                "agent_uuid": agent.to_string(),
                "session_uuid": session.to_string(),
                "region_id": region.to_string(),
                "position": "<9, 9, 9>",
                "wait_for_root": true,
            }),
        )
        .await;
        assert_eq!(rooted, "False");
        assert_eq!(accepted.return_slot.position(), [40.0, 40.0, 40.0]);
        let rooted_position = put_agent(
            &state,
            peer,
            &json!({
                "message_type": "AgentPosition",
                "agent_uuid": agent.to_string(),
                "session_uuid": session.to_string(),
                "position": "<8, 8, 8>",
            }),
        )
        .await;
        assert_eq!(rooted_position, "True");
        assert_eq!(accepted.return_slot.position(), [40.0, 40.0, 40.0]);

        state.demote_child(agent).unwrap();
        let state_wait = state.clone();
        let waiting = tokio::spawn(async move {
            put_agent(
                &state_wait,
                peer,
                &json!({
                    "message_type": "AgentData",
                    "agent_uuid": agent.to_string(),
                    "session_uuid": session.to_string(),
                    "region_id": Uuid::from_u128(70).to_string(),
                    "position": "<64, 64, 12>",
                    "at_axis": "<1, 0, 0>",
                    "velocity": "<1.5, -2, 0>",
                    "far": 192.0,
                    "body_rotation": "<0, 0, 0.6, 0.8>",
                    "head_rotation": "<0, 0, 0.5, 0.8660254>",
                    "control_flags": "8192",
                    "always_run": true,
                    "default_animation": { "animation": agent.to_string(), "seq_num": 8 },
                    "animation_state": { "animation": session.to_string(), "seq_num": 9 },
                    "animations": [{ "animation": agent.to_string(), "object_id": session.to_string(), "seq_num": 10 }],
                    "motion_state": 2,
                    "wait_for_root": true,
                    "cb_uri": "http://127.0.0.1:9/agent/release/",
                }),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if accepted.return_slot.position() == [64.0, 64.0, 12.0] {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the child update was not stored");
        let stored = accepted.return_slot.pose();
        assert_eq!(stored.velocity, [1.5, -2.0, 0.0]);
        assert_eq!(stored.far, 192.0);
        assert_eq!(stored.body_rotation, [0.0, 0.0, 0.6, 0.8]);
        assert_eq!(stored.head_rotation, [0.0, 0.0, 0.5, 0.866_025_4]);
        assert_eq!(stored.control_flags, 8192);
        assert!(stored.always_run);
        // The animator fields ride with the pose to the frame's `CopyFrom`.
        let animations = &stored.animations;
        assert_eq!(animations.default_animation.unwrap().animation, agent);
        assert_eq!(animations.default_animation.unwrap().sequence, 8);
        assert_eq!(animations.animation_state.unwrap().sequence, 9);
        assert_eq!(animations.animations[0].object_id, session);
        assert_eq!(animations.motion_state, 2);
        accepted.mark_root();
        let flag = tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(flag, "True");
        assert_eq!(
            accepted.return_slot.take_callback().as_deref(),
            Some("http://127.0.0.1:9/agent/release/")
        );
        let garbage = put_agent(&state, peer, &json!({"message_type": "nope"})).await;
        assert_eq!(garbage, "False");
        let not_object = Request::builder()
            .method("PUT")
            .uri(format!("/agent/{agent}/"))
            .body(axum::body::Body::from("[1]"))
            .unwrap();
        let response = handle(State(state.clone()), ConnectInfo(peer), not_object).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn the_arrival_look_falls_back_as_complete_movement_does() {
        assert_eq!(
            arrival_look([0.0, 2.0, 5.0], [9.0, 0.0, 0.0]),
            [0.0, 1.0, 0.0]
        );
        let look = arrival_look([3.0, 4.0, 0.0], [0.0; 3]);
        assert!((look[0] - 0.6).abs() < 1.0e-6 && (look[1] - 0.8).abs() < 1.0e-6);
        // A landmark teleport: unit Z, so the copied velocity decides.
        assert_eq!(
            arrival_look([0.0, 0.0, 1.0], [0.0, -2.0, 1.0]),
            [0.0, -1.0, 0.0]
        );
        // Neither: the fixed heading.
        assert_eq!(arrival_look([0.0, 0.0, 1.0], [0.0; 3]), [0.99, 0.042, 0.0]);
        // A short axis that still normalises keeps its heading; one too
        // short to normalise is zero and falls through.
        assert_eq!(arrival_look([0.005, 0.0, 0.0], [0.0; 3]), [1.0, 0.0, 0.0]);
        assert_eq!(
            arrival_look([0.0005, 0.0, 0.0], [0.0; 3]),
            [0.99, 0.042, 0.0]
        );
    }

    #[test]
    fn circuit_urls_prefer_the_map_and_keep_the_last_legacy_value() {
        let mut args = json!({"service_urls":["HomeURI", "/", "HomeURI", "legacy/", "odd"]});
        assert_eq!(
            circuit_service_urls(args.as_object().unwrap()),
            [("HomeURI".into(), "legacy/".into())]
        );
        args["serviceurls"] = json!({"HomeURI": "map/"});
        assert_eq!(
            circuit_service_urls(args.as_object().unwrap()),
            [("HomeURI".into(), "map/".into())]
        );
        args["serviceurls"] = json!({});
        assert!(circuit_service_urls(args.as_object().unwrap()).is_empty());
    }

    #[test]
    fn returning_agent_data_fields_parse_as_agent_data_unpack_does() {
        assert_eq!(json_quat(None), [0.0; 4]);
        assert_eq!(json_quat(Some(&json!("<1, 2>"))), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(json_quat(Some(&json!(7))), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            json_quat(Some(&json!("<0, 0, 1, 0>"))),
            [0.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(json_real(Some(&json!(64.5))), 64.5);
        assert_eq!(json_real(Some(&json!("96"))), 96.0);
        assert_eq!(json_real(None), 0.0);
        assert_eq!(json_u32_text(Some(&json!("4294967295"))), u32::MAX);
        assert_eq!(json_u32_text(Some(&json!("-1"))), 0);
        assert_eq!(json_u32_text(Some(&json!(17))), 17);
        assert_eq!(json_u32_text(None), 0);
        assert!(json_bool(Some(&json!(true))));
        assert!(json_bool(Some(&json!("True"))));
        assert!(!json_bool(Some(&json!("no"))));
        assert!(!json_bool(None));
    }

    fn child_request(fields: &AgentFields, region: Uuid, caps_path: &str) -> Request {
        let body = json!({
            "agent_id": fields.agent_id.to_string(),
            "session_id": fields.session_id.to_string(),
            "secure_session_id": fields.secure_session_id.to_string(),
            "circuit_code": fields.circuit_code.to_string(),
            "caps_path": caps_path,
            "start_pos": "<128, 128, 30>",
            "first_name": fields.first_name,
            "last_name": fields.last_name,
            "destination_uuid": region.to_string(),
            "destination_name": "Fixture Region",
            "teleport_flags": "16",
            "child": true,
        });
        Request::builder()
            .method("POST")
            .uri(format!("/agent/{}/", fields.agent_id))
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    fn captured_agent_update() -> Value {
        let bytes = include_bytes!("../../../fixtures/neighbour-teleport/proxy-home.cap");
        let records = ferroan_capture::read_complete_prefix(bytes).unwrap();
        for record in records {
            if record.kind != ferroan_capture::Kind::HttpRequest {
                continue;
            }
            let Some(split) = record.bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let body = &record.bytes[split + 4..];
            let Ok(value) = serde_json::from_slice::<Value>(body) else {
                continue;
            };
            if value.get("message_type").and_then(Value::as_str) == Some("AgentData") {
                return value;
            }
        }
        panic!("proxy-home.cap has no AgentData");
    }

    fn agent_body(patch: &Value) -> Value {
        let mut body = captured_agent_update();
        let Some(map) = body.as_object_mut() else {
            return body;
        };
        if let Value::Object(patch) = patch {
            for (key, value) in patch {
                map.insert(key.clone(), value.clone());
            }
        }
        body
    }

    async fn put_agent(state: &SimulationState, peer: SocketAddr, body: &Value) -> String {
        let body = agent_body(body);
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/agent/{}/", state_agent(&body)))
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
        assert_eq!(response.status(), StatusCode::OK);
        String::from_utf8(
            to_bytes(response.into_body(), MAX_BODY)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    fn state_agent(body: &Value) -> Uuid {
        body.get("agent_uuid")
            .and_then(Value::as_str)
            .and_then(|text| Uuid::parse_str(text).ok())
            .unwrap_or_else(|| Uuid::from_u128(1))
    }

    #[test]
    fn login_create_keeps_viewer_and_appearance() {
        let args = decode_body(CREATE_REQUEST.as_bytes(), false).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let fields = AgentFields::parse(&args, agent, region).unwrap();
        assert_eq!(fields.viewer, "Ferroan Bot 0.1.0");
        assert_eq!(fields.channel, "Ferroan Bot");
        assert!(!fields.mac.is_empty());
        assert!(!fields.id0.is_empty());
        assert!(fields.via_login);
        let appearance = fields.appearance.expect("packed appearance");
        assert!((appearance.height - 1.9).abs() < 0.01);
        assert!(appearance.has_texture);
    }

    #[tokio::test]
    async fn close_agent_checks_the_session_and_leaves_the_root() {
        let region = Uuid::from_u128(5);
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let state = SimulationState::new(&test_config("http://127.0.0.1:9", region), caps, sender);
        let login = synthetic_agent();
        let agent = login.agent_id;
        let session = login.session_id;
        state.accept(login).unwrap();
        let mut accepted = receiver.recv().await.unwrap();
        accepted.mark_root();
        let peer = "127.0.0.1:9".parse().unwrap();
        let wrong = delete_agent(&state, peer, agent, region, "not-the-session").await;
        assert_eq!(wrong, format!("OpenSim agent {agent}"));
        assert!(matches!(
            accepted.cancelled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let other = delete_agent(
            &state,
            peer,
            agent,
            Uuid::from_u128(8),
            &session.to_string(),
        )
        .await;
        assert_eq!(other, format!("OpenSim agent {agent}"));
        assert!(accepted.still_root());
        let closed = delete_agent(&state, peer, agent, region, &session.to_string()).await;
        assert_eq!(closed, format!("OpenSim agent {agent}"));
        let kind = tokio::time::timeout(Duration::from_secs(1), accepted.replaced())
            .await
            .unwrap();
        assert_eq!(kind, CircuitStop::Quiet);
        assert!(accepted.is_current());
        assert!(accepted.still_root());
    }

    #[tokio::test]
    async fn a_nil_agent_or_region_delete_answers_false() {
        let region = Uuid::from_u128(5);
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        let state = SimulationState::new(&test_config("http://127.0.0.1:9", region), caps, sender);
        let agent = Uuid::from_u128(1);
        let peer = "127.0.0.1:9".parse().unwrap();
        for uri in [
            format!("/agent/{}/", Uuid::nil()),
            format!("/agent/{agent}/"),
            format!("/agent/{agent}/{}/", Uuid::nil()),
            format!("/agent/{}/{region}/", Uuid::nil()),
        ] {
            let request = Request::builder()
                .method("DELETE")
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap();
            let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = to_bytes(response.into_body(), MAX_BODY).await.unwrap();
            assert_eq!(body.as_ref(), b"false");
        }
    }

    #[tokio::test]
    async fn a_demoted_or_departed_generation_is_not_a_replaced_root() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps,
            sender,
        );
        let login = synthetic_agent();
        state.accept(login.clone()).unwrap();
        let mut rooted = receiver.recv().await.unwrap();
        rooted.mark_root();
        let mut other = login.clone();
        other.session_id = Uuid::from_u128(70);
        other.secure_session_id = Uuid::from_u128(71);
        other.circuit_code = 70;
        other.caps_path = Uuid::from_u128(72).to_string();
        state.accept(other.clone()).unwrap();
        assert!(rooted.replaced_by_other_session());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), rooted.replaced())
                .await
                .unwrap(),
            CircuitStop::Disable
        );
        drop(rooted);
        drop(receiver.recv().await.unwrap());

        state.accept(login.clone()).unwrap();
        let demoted = receiver.recv().await.unwrap();
        demoted.mark_root();
        state.demote_child(login.agent_id).unwrap();
        let mut next = other.clone();
        next.session_id = Uuid::from_u128(73);
        next.secure_session_id = Uuid::from_u128(74);
        next.circuit_code = 73;
        next.caps_path = Uuid::from_u128(75).to_string();
        state.accept(next).unwrap();
        assert!(!demoted.replaced_by_other_session());
        drop(demoted);
        drop(receiver.recv().await.unwrap());

        state.accept(login.clone()).unwrap();
        let departed = receiver.recv().await.unwrap();
        departed.mark_root();
        state.close_departed(login.agent_id, &far_destination());
        let mut after = other;
        after.session_id = Uuid::from_u128(76);
        after.secure_session_id = Uuid::from_u128(77);
        after.circuit_code = 76;
        after.caps_path = Uuid::from_u128(78).to_string();
        state.accept(after).unwrap();
        assert!(!departed.replaced_by_other_session());
    }

    #[tokio::test]
    async fn close_and_drop_clear_the_open_child() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(2);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps,
            sender,
        );
        state.accept(synthetic_agent()).unwrap();
        let accepted = receiver.recv().await.unwrap();
        let agent = accepted.expect.agent_id;
        let end = RegionEnd {
            server_uri: "http://127.0.0.1:9/".to_string(),
            region_id: Uuid::from_u128(7),
            name: "Neighbour".to_string(),
            loc_x: 256,
            loc_y: 256,
        };
        let generation = state.root_generation(agent).unwrap();
        assert!(matches!(
            state.claim_child(agent, generation, end.clone()),
            crate::children::Claim::New { .. }
        ));
        assert!(matches!(
            state.finish_child(agent, generation, end.region_id),
            crate::children::Admission::Announce { .. }
        ));
        state.close_departed(agent, &far_destination());
        assert!(state.take_open_children(agent).is_empty());

        state.accept(synthetic_agent()).unwrap();
        let old = receiver.recv().await.unwrap();
        let mut next = synthetic_agent();
        next.session_id = Uuid::from_u128(70);
        next.circuit_code = 70;
        next.caps_path = Uuid::from_u128(71).to_string();
        state.accept(next).unwrap();
        let mut current = receiver.recv().await.unwrap();
        let mut old = old;
        assert!(matches!(
            old.predecessor.as_mut().unwrap().has_changed(),
            Ok(false)
        ));
        drop(accepted);
        old.wait_for_predecessor().await;
        assert!(matches!(
            current.predecessor.as_mut().unwrap().has_changed(),
            Ok(false)
        ));
        let generation = state.root_generation(agent).unwrap();
        assert!(matches!(
            state.claim_child(agent, generation, end),
            crate::children::Claim::New { .. }
        ));
        assert!(matches!(
            state.finish_child(agent, generation, Uuid::from_u128(7)),
            crate::children::Admission::Announce { .. }
        ));
        drop(old);
        current.wait_for_predecessor().await;
        assert_eq!(
            state
                .take_open_children(agent)
                .into_iter()
                .map(|end| end.region_id)
                .collect::<Vec<_>>(),
            vec![Uuid::from_u128(7)]
        );
        drop(current);
    }

    #[tokio::test]
    async fn promote_child_stays_open_for_the_same_session() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(2);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps,
            sender,
        );
        state.accept(synthetic_agent()).unwrap();
        let rooted = receiver.recv().await.unwrap();
        let agent = rooted.expect.agent_id;
        let end = RegionEnd {
            server_uri: "http://127.0.0.1:9/".to_string(),
            region_id: Uuid::from_u128(7),
            name: "Neighbour".to_string(),
            loc_x: 256,
            loc_y: 256,
        };
        state.promote_child(agent, &end, "seed");
        rooted.mark_root();
        assert_eq!(
            state
                .take_open_children(agent)
                .into_iter()
                .map(|end| end.region_id)
                .collect::<Vec<_>>(),
            vec![end.region_id]
        );

        state.accept(synthetic_agent()).unwrap();
        let first = receiver.recv().await.unwrap();
        state.promote_child(agent, &end, "seed");
        state.accept(synthetic_agent()).unwrap();
        let second = receiver.recv().await.unwrap();
        drop(first);
        assert!(state.take_open_children(agent).is_empty());
        second.mark_root();
        assert_eq!(
            state
                .take_open_children(agent)
                .into_iter()
                .map(|end| end.region_id)
                .collect::<Vec<_>>(),
            vec![end.region_id]
        );
        drop(second);
        drop(rooted);
    }

    #[tokio::test]
    async fn a_different_session_does_not_inherit_the_open_child() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(2);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps,
            sender,
        );
        state.accept(synthetic_agent()).unwrap();
        let old = receiver.recv().await.unwrap();
        let agent = old.expect.agent_id;
        let end = RegionEnd {
            server_uri: "http://127.0.0.1:9/".to_string(),
            region_id: Uuid::from_u128(7),
            name: "Neighbour".to_string(),
            loc_x: 256,
            loc_y: 256,
        };
        let generation = state.root_generation(agent).unwrap();
        assert!(matches!(
            state.claim_child(agent, generation, end.clone()),
            crate::children::Claim::New { .. }
        ));
        assert!(matches!(
            state.finish_child(agent, generation, end.region_id),
            crate::children::Admission::Announce { .. }
        ));
        let mut next = synthetic_agent();
        next.session_id = Uuid::from_u128(70);
        next.circuit_code = 70;
        next.caps_path = Uuid::from_u128(71).to_string();
        state.accept(next).unwrap();
        let current = receiver.recv().await.unwrap();
        assert!(state.take_open_children(agent).is_empty());
        let staged = state.take_staged_closes(agent);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, Uuid::from_u128(2));
        assert_eq!(staged[0].1.region_id, end.region_id);
        drop(old);
        drop(current);
    }

    /// A with neighbours B and E. After A-B-A, B is an ordinary open child.
    /// An in-view hop promotes E, and E then closes A's child circuit. That
    /// lease drop must not stage a close of B: the next login, child arrivals
    /// included, would send it with the old session.
    #[tokio::test]
    async fn a_child_circuit_drop_after_an_in_view_hop_stages_nothing() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(4);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps,
            sender,
        );
        state.accept(synthetic_agent()).unwrap();
        let root = receiver.recv().await.unwrap();
        root.mark_root();
        let agent = root.expect.agent_id;
        let neighbour = |id: u128, name: &str, loc_x: u32| RegionEnd {
            server_uri: format!("http://{name}/"),
            region_id: Uuid::from_u128(id),
            name: name.to_string(),
            loc_x,
            loc_y: 256,
        };
        let b = neighbour(11, "B", 256);
        let e = neighbour(12, "E", 512);
        let generation = state.root_generation(agent).unwrap();
        assert!(matches!(
            state.claim_child(agent, generation, b.clone()),
            crate::children::Claim::New { .. }
        ));
        assert!(matches!(
            state.finish_child(agent, generation, b.region_id),
            crate::children::Admission::Announce { .. }
        ));
        // Round trip A-B-A. The return carries B, then the new root releases it
        // so B is an ordinary open child. `child_seed` does not consume it.
        state.promote_child(agent, &b, "seed-b");
        state.accept(synthetic_agent()).unwrap();
        let returned = receiver.recv().await.unwrap();
        drop(root);
        returned.mark_root();
        assert!(state.child_seed(agent, b.region_id).is_some());
        // In-view hop A-E. E is the remote root. B stays open.
        state.promote_child(agent, &e, "seed-e");
        assert!(state.child_seed(agent, b.region_id).is_some());
        state.demote_child(agent).unwrap();
        // E closes A's child circuit.
        drop(returned);
        assert!(
            state.take_staged_closes(agent).is_empty(),
            "dropping the child circuit staged a close"
        );
        state.accept(synthetic_agent()).unwrap();
        let again = receiver.recv().await.unwrap();
        assert!(state.take_staged_closes(agent).is_empty());
        drop(again);
    }

    /// Production far hop: `close_departed` stages only the child outside the
    /// destination's view, with this session. Dropping the lease afterwards
    /// stages nothing more.
    #[tokio::test]
    async fn a_far_hop_stages_only_the_outside_child() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, mut receiver) = mpsc::channel(2);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps,
            sender,
        );
        state.accept(synthetic_agent()).unwrap();
        let accepted = receiver.recv().await.unwrap();
        accepted.mark_root();
        let agent = accepted.expect.agent_id;
        // `far_destination` is cell (3, 3). The next cell stays inside.
        // Cell (7, 3) is outside: 768 + 256 + 254 < 1792.
        let neighbour = |id: u128, loc_x: u32| RegionEnd {
            server_uri: format!("http://192.0.2.{id}/"),
            region_id: Uuid::from_u128(id),
            name: "Neighbour".to_string(),
            loc_x,
            loc_y: 768,
        };
        let adjacent = neighbour(11, 1024);
        let outside = neighbour(12, 1792);
        let generation = state.root_generation(agent).unwrap();
        for region in [&adjacent, &outside] {
            assert!(matches!(
                state.claim_child(agent, generation, region.clone()),
                crate::children::Claim::New { .. }
            ));
            assert!(matches!(
                state.finish_child(agent, generation, region.region_id),
                crate::children::Admission::Announce { .. }
            ));
        }
        state.close_departed(agent, &far_destination());
        let staged = state.take_staged_closes(agent);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, Uuid::from_u128(2));
        assert_eq!(staged[0].1.region_id, outside.region_id);
        drop(accepted);
        assert!(
            state.take_staged_closes(agent).is_empty(),
            "the lease drop staged another close"
        );
    }

    /// `MakeChildAgent` spares a root whose open child is in the destination.
    #[tokio::test]
    async fn a_hop_away_spares_a_root_with_a_child_in_the_destination() {
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, _receiver) = mpsc::channel(2);
        let mut state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps,
            sender,
        );
        let frame = Arc::new(ferroan_scene::TerrainFrame::start_without_map().unwrap());
        state.keep_scene(Arc::clone(&frame));
        let end = RegionEnd {
            server_uri: "http://127.0.0.1:9/".to_string(),
            region_id: Uuid::from_u128(7),
            name: "Neighbour".to_string(),
            loc_x: 256,
            loc_y: 256,
        };
        let mut feeds = Vec::new();
        for id in [80u128, 81, 82] {
            let agent = Uuid::from_u128(id);
            let feed = frame.new_feed();
            let link = ferroan_scene::ViewerLink {
                feed: Arc::clone(&feed),
                seeds_sent: Arc::new(AtomicBool::new(false)),
                circuit_open: Arc::new(AtomicBool::new(true)),
                look: Arc::new(ferroan_scene::AvatarLook {
                    first_name: "Test".to_string(),
                    last_name: "User".to_string(),
                    title: String::new(),
                    visual_params: Vec::new(),
                    texture: Vec::new(),
                    height: 1.9,
                    hover: 0.0,
                }),
                teleport_flags: 1 << 7,
                rights: ferroan_scene::DeleteRights::default(),
            };
            let local_id = frame.attach(agent, link).unwrap().wait().await.unwrap();
            let pose = ferroan_scene::Presence {
                position: [128.0, 128.0, 25.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                velocity: [0.0; 3],
                control_flags: 0,
                draw_distance: 255.0,
            };
            frame
                .arrive(agent, pose, [1.0, 0.0, 0.0])
                .unwrap()
                .wait()
                .await
                .unwrap();
            feeds.push((local_id, feed));
        }
        for (_, feed) in &feeds {
            let _ = feed.take_avatar_packets(usize::MAX);
        }
        let hopper = Uuid::from_u128(80);
        // 81 has a child in the destination; 82 does not.
        let mut watcher = synthetic_agent();
        watcher.agent_id = Uuid::from_u128(81);
        watcher.session_id = Uuid::from_u128(81);
        watcher.circuit_code = 81;
        watcher.caps_path = Uuid::from_u128(81).to_string();
        state.accept(watcher).unwrap();
        let watcher_generation = state.root_generation(Uuid::from_u128(81)).unwrap();
        assert!(matches!(
            state.claim_child(Uuid::from_u128(81), watcher_generation, end.clone()),
            crate::children::Claim::New { .. }
        ));
        assert!(matches!(
            state.finish_child(Uuid::from_u128(81), watcher_generation, end.region_id),
            crate::children::Admission::Announce { .. }
        ));
        state.moved_away(hopper, true, &end).await;
        assert!(feeds[1].1.take_avatar_packets(usize::MAX).is_empty());
        assert_eq!(
            feeds[2].1.take_avatar_packets(usize::MAX),
            [ferroan_scene::AvatarPacket::Kill(vec![feeds[0].0])]
        );
        assert!(feeds[0].1.take_avatar_packets(usize::MAX).is_empty());
    }

    #[tokio::test]
    async fn an_oversized_put_is_refused() {
        let before = refuse_count(&ferroan_diag::metrics(), HTTP_POST_BODY);
        let caps = CapsRegistry::new("http://127.0.0.1:8002").unwrap();
        let (sender, _receiver) = mpsc::channel(1);
        let state = SimulationState::new(
            &test_config("http://127.0.0.1:9", Uuid::from_u128(5)),
            caps,
            sender,
        );
        let peer = "127.0.0.1:9".parse().unwrap();
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/agent/{}/", Uuid::from_u128(1)))
            .body(axum::body::Body::from(vec![b' '; MAX_BODY + 1]))
            .unwrap();
        let response = handle(State(state), ConnectInfo(peer), request).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let after = refuse_count(&ferroan_diag::metrics(), HTTP_POST_BODY);
        assert!(after > before);
    }

    async fn delete_agent(
        state: &SimulationState,
        peer: SocketAddr,
        agent: Uuid,
        region: Uuid,
        auth: &str,
    ) -> String {
        let request = Request::builder()
            .method("DELETE")
            .uri(format!("/agent/{agent}/{region}/?auth={auth}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let response = handle(State(state.clone()), ConnectInfo(peer), request).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/plain");
        String::from_utf8(
            to_bytes(response.into_body(), MAX_BODY)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }
}
