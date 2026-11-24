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
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::config::RegionConfig;
use crate::control_plane::ControlPlaneAccess;
use crate::grid::PresenceClient;
use crate::simulation_client::{Appearance, RegionEnd, appearance_from_json};
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
use ferroan_client::{CapsError, CapsRegistry, CircuitExpect, ROOT_WAIT, ReturnPose, ReturnSlot};
use ferroan_diag::{self, HTTP_POST_BODY, Kind, ROOT_AGENTS, ROOT_BYPASS};
use flate2::read::GzDecoder;
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc, oneshot};
use uuid::Uuid;

const MAX_BODY: usize = 1024 * 1024;
const MAX_FEATURES: usize = 256;
const MIN_VERSION: f32 = 0.3;
const MAX_VERSION: f32 = 0.8;
const VIA_LOGIN: u32 = 1 << 7;
/// `RegionSettings.AgentLimit` at the pin. Not a configuration key.
pub const ROOT_AGENT_LIMIT: usize = 40;
/// Queued accepts: the ordinary roots plus the staff and god bypass.
pub const ADMISSION_QUEUE: usize = ROOT_AGENT_LIMIT + ROOT_BYPASS_LIMIT;
/// Two overlapping logins, four secrets each, for every queued root.
/// [`ferroan_log::MAX_SECRETS`] has to stay equal to this or a full region
/// fails open and leaves a session id in a log line.
const ADMISSION_SECRET_SLOTS: usize = ADMISSION_QUEUE * 2 * 4;
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
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    admission_open: Arc<AtomicBool>,
    /// One envelope for the lifecycle-lock wait and the Presence verify.
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
    /// In-view child this root opened on a neighbour. Logout closes it.
    open_children: Arc<Mutex<HashMap<Uuid, RegionEnd>>>,
}

struct ActiveSession {
    session_id: Uuid,
    generation: Uuid,
    /// Set when this circuit's root arrival is accepted, before the root reports.
    root: bool,
    /// A hop demoted this root. The caps seed and the circuit stay.
    child: bool,
    /// Set when a far hop closes the agent here. Logout must not tell the grid.
    departed: Arc<AtomicBool>,
    cancel: Option<oneshot::Sender<()>>,
    /// Taken by a failed root-replacement verify. Dropping `AcceptedAgent`
    /// completes it after logout notification.
    logout_done: Option<oneshot::Receiver<()>>,
    /// Child update and the promotion back to root. Shared with the circuit.
    slot: ReturnSlot,
}

/// Keeps the accepted seed and circuit reserved until logout, timeout or failure.
/// Do not log this value: it includes session credentials received from Robust.
pub struct AcceptedAgent {
    /// Completed when this agent is dropped, after its logout notification.
    /// Nothing reads the sender; dropping it is the signal.
    #[allow(dead_code)]
    logout_done: Option<oneshot::Sender<()>>,
    pub accepted_at: tokio::time::Instant,
    pub expect: CircuitExpect,
    pub secure_session_id: Uuid,
    pub caps_path: String,
    pub start_pos: [f32; 3],
    pub first_name: String,
    pub last_name: String,
    /// `ViaLogin`. A hop child is not one, so its end is not a failed login.
    pub via_login: bool,
    pub viewer: String,
    pub channel: String,
    pub mac: String,
    pub id0: String,
    pub appearance: Option<Appearance>,
    /// Same gate the session stores. The circuit waits on it for a return.
    pub return_slot: ReturnSlot,
    cancelled: oneshot::Receiver<()>,
    _lease: SessionLease,
}

impl AcceptedAgent {
    pub async fn lock_lifecycle(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self._lease.lifecycle.clone().lock_owned().await
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
    /// The old session is logged out. A same-session replay, a far hop and a
    /// demotion are not this case.
    pub fn replaced_by_other_session(&self) -> bool {
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
    /// Completes when a new login replaces this circuit. The dispatcher must
    /// drop the old UDP-serving future before starting the replacement.
    pub async fn replaced(&mut self) {
        let _ = (&mut self.cancelled).await;
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
            session.child = false;
            session.slot.note_rooted();
            publish_agents(&sessions, &self._lease.meters);
        }
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
    let children = sessions.values().filter(|session| session.child).count() as u64;
    meters.set_agents(roots, children, sessions.len() as u64);
}

fn occupying(sessions: &HashMap<Uuid, ActiveSession>) -> usize {
    sessions.values().filter(|session| !session.child).count()
}

struct SessionLease {
    /// Released when this generation is dropped, including a replacement that
    /// outlived the old circuit. A secret still held by another generation stays.
    secrets: ferroan_log::SessionSecrets,
    sessions: Arc<Mutex<HashMap<Uuid, ActiveSession>>>,
    agent_id: Uuid,
    generation: Uuid,
    caps: CapsRegistry,
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    meters: Arc<RegionMeters>,
    departed: Arc<AtomicBool>,
    open_children: Arc<Mutex<HashMap<Uuid, RegionEnd>>>,
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
        if sessions
            .get(&self.agent_id)
            .is_some_and(|session| session.generation == self.generation)
        {
            self.open_children
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .remove(&self.agent_id);
            self.caps.clear_agent(self.agent_id);
            if let Some(session) = sessions.remove(&self.agent_id) {
                session.slot.close();
            }
            publish_agents(&sessions, &self.meters);
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
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
            admission_open: Arc::new(AtomicBool::new(true)),
            create_agent_budget: Duration::from_secs(25),
            root_limit: ROOT_AGENT_LIMIT,
            bypass_limit: ROOT_BYPASS_LIMIT,
            staff: StaffGate::from_config(config),
            meters,
            releases: Arc::new(Mutex::new(HashMap::new())),
            objects: Arc::new(Vec::new()),
            open_children: Arc::new(Mutex::new(HashMap::new())),
            scene: None,
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
        let level =
            crate::grid::account_user_level(self.staff.account_uri(), self.staff.scope_id(), agent)
                .await;
        if let Some(level) = level {
            self.staff.note_level(agent, level);
        }
        level
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

    #[cfg(test)]
    fn accept(&self, fields: AgentFields) -> Result<(), &'static str> {
        self.accept_with_level(fields, None)
    }

    fn accept_with_level(
        &self,
        fields: AgentFields,
        level: Option<i32>,
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
            if let Some(cancel) = previous.cancel.take() {
                let _ = cancel.send(());
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
        let departed = Arc::new(AtomicBool::new(false));
        let (cancel, cancelled) = oneshot::channel();
        let (logout_done, logout_wait) = oneshot::channel();
        let slot = ReturnSlot::new(fields.child);
        sessions.insert(
            fields.agent_id,
            ActiveSession {
                session_id: fields.session_id,
                generation,
                root: false,
                child: fields.child,
                departed: Arc::clone(&departed),
                cancel: Some(cancel),
                logout_done: Some(logout_wait),
                slot: slot.clone(),
            },
        );
        publish_agents(&sessions, &self.meters);
        let lease = SessionLease {
            secrets,
            sessions: self.sessions.clone(),
            agent_id: fields.agent_id,
            generation,
            caps: self.caps.clone(),
            lifecycle: self.lifecycle.clone(),
            meters: self.meters.clone(),
            departed,
            open_children: self.open_children.clone(),
        };
        let accepted = AcceptedAgent {
            logout_done: Some(logout_done),
            accepted_at: tokio::time::Instant::now(),
            expect: CircuitExpect {
                agent_id: fields.agent_id,
                session_id: fields.session_id,
                circuit_code: fields.circuit_code,
                region_id: self.region_id,
            },
            secure_session_id: fields.secure_session_id,
            caps_path: fields.caps_path,
            start_pos: fields.start_pos,
            first_name: fields.first_name,
            last_name: fields.last_name,
            via_login: fields.via_login,
            viewer: fields.viewer,
            channel: fields.channel,
            mac: fields.mac,
            id0: fields.id0,
            appearance: fields.appearance,
            return_slot: slot,
            cancelled,
            _lease: lease,
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
    fn begin_root_close(&self, agent: Uuid) -> Option<oneshot::Receiver<()>> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let session = sessions.get_mut(&agent)?;
        if !session.root {
            return None;
        }
        if let Some(cancel) = session.cancel.take() {
            let _ = cancel.send(());
        }
        session.logout_done.take()
    }

    /// The root becomes a child before `UpdateAgent`. Caps and the circuit stay.
    pub fn demote_child(&self, agent: Uuid) {
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
            session.child = true;
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
            session.child = false;
            session.root = true;
            session.slot.disarm();
            publish_agents(&sessions, &self.meters);
        }
        self.mark_child(agent, false);
    }

    /// Same session, not a login. Keep the circuit and the caps. A stale root
    /// becomes a child instead of being logged out.
    fn reuse_child(&self, fields: &AgentFields) -> Result<(), &'static str> {
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
            session.child = true;
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
    pub fn close_departed(&self, agent: Uuid) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(session) = sessions.get(&agent) else {
            return;
        };
        self.open_children
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&agent);
        session.departed.store(true, Ordering::Release);
        let slot = session.slot.clone();
        self.caps.clear_agent(agent);
        sessions.remove(&agent);
        publish_agents(&sessions, &self.meters);
        drop(sessions);
        slot.close();
    }

    /// Remember a neighbour child that stays in view. A later hop replaces it.
    pub fn note_open_child(&self, agent: Uuid, end: RegionEnd) {
        self.open_children
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(agent, end);
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
            let _ = cancel.send(());
        }
    }

    /// The neighbour logout should close, once.
    pub fn take_open_child(&self, agent: Uuid) -> Option<RegionEnd> {
        self.open_children
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&agent)
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
    let level = state.lookup_level(agent).await;
    if let Some(reason) = state.admission_for(agent, level, fields.child) {
        return create_refused(reason, peer);
    }
    // Same session, not a login: keep the circuit and the caps. A failed
    // presence check must not log this avatar out.
    if fields.child && !fields.via_login && state.same_session(agent, fields.session_id) {
        let busy = "Region is busy completing a login or logout";
        let deadline = tokio::time::Instant::now() + state.create_agent_budget;
        let lock_wait = deadline.saturating_duration_since(tokio::time::Instant::now());
        if lock_wait.is_zero() {
            return create_refused(busy, peer);
        }
        let Ok(_lifecycle) = tokio::time::timeout(lock_wait, state.lifecycle.lock()).await else {
            return create_refused(busy, peer);
        };
        let answer = match state.reuse_child(&fields) {
            Ok(()) => {
                json!({"reason":"Authorized", "success":true, "your_ip":peer.ip().to_string()})
            }
            Err(reason) => json!({"reason":reason, "your_ip":peer.ip().to_string()}),
        };
        return json_reply(answer, false);
    }
    // SimulationServiceConnector.CreateAgent posts with a 30 s timeout. The
    // lock wait and the Presence verify share one shorter budget so Robust
    // does not time out and retry a login this region then accepts.
    let deadline = tokio::time::Instant::now() + state.create_agent_budget;
    let remaining = || deadline.saturating_duration_since(tokio::time::Instant::now());
    let busy = "Region is busy completing a login or logout";
    let lock_wait = remaining();
    if lock_wait.is_zero() {
        return create_refused(busy, peer);
    }
    let Ok(lifecycle) = tokio::time::timeout(lock_wait, state.lifecycle.lock()).await else {
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
        drop(lifecycle);
        // Successive logins still verify before replacing. Only a failed verify
        // of an existing root closes that root and waits for its LoggedOut.
        if let Some(done) = state.begin_root_close(agent) {
            let wait = remaining();
            if !wait.is_zero() {
                let _ = tokio::time::timeout(wait, done).await;
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
    let answer = match state.accept_with_level(fields, level) {
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
    let look_at = json_vec3(args.get("at_axis"))
        .map(look_axis)
        .unwrap_or([1.0, 0.0, 0.0]);
    let origin = json_uuid(args.get("region_id")).unwrap_or_else(Uuid::nil);
    let callback = args
        .get("cb_uri")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if !slot.store_update(
        ReturnPose {
            position,
            look_at,
            origin,
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
    if !row.child {
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

/// Flatten Z. A missing or near-zero heading stays the arrival default.
fn look_axis(at_axis: [f32; 3]) -> [f32; 3] {
    if at_axis[0].is_finite()
        && at_axis[1].is_finite()
        && at_axis[0].abs() + at_axis[1].abs() > 1.0e-6
    {
        [at_axis[0], at_axis[1], 0.0]
    } else {
        [1.0, 0.0, 0.0]
    }
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
    viewer: String,
    channel: String,
    mac: String,
    id0: String,
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
            viewer: optional_string(args, "viewer"),
            channel: optional_string(args, "channel"),
            mac: optional_string(args, "mac"),
            id0: optional_string(args, "id0"),
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
            .unwrap();
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
        assert!(state.lifecycle.try_lock().is_err());
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
            viewer: String::new(),
            channel: String::new(),
            mac: String::new(),
            id0: String::new(),
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
        state.demote_child(agent.expect.agent_id);
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
        state.demote_child(agent.expect.agent_id);
        state.close_departed(agent.expect.agent_id);
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
        let lifecycle = state.lifecycle.clone();
        let holder = tokio::spawn(async move {
            let guard = lifecycle.lock_owned().await;
            let _ = release_rx.await;
            drop(guard);
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state.lifecycle.try_lock().is_err() {
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
        again.caps_path = Uuid::from_u128(99).to_string();
        state.reuse_child(&again).unwrap();
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

        state.demote_child(agent);
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
        tokio::time::timeout(Duration::from_secs(1), accepted.replaced())
            .await
            .unwrap();
        assert!(accepted.is_current());
        assert!(accepted.still_root());
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
        state.note_open_child(agent, end.clone());
        state.close_departed(agent);
        assert!(state.take_open_child(agent).is_none());

        state.accept(synthetic_agent()).unwrap();
        let old = receiver.recv().await.unwrap();
        let mut next = synthetic_agent();
        next.session_id = Uuid::from_u128(70);
        next.circuit_code = 70;
        next.caps_path = Uuid::from_u128(71).to_string();
        state.accept(next).unwrap();
        let current = receiver.recv().await.unwrap();
        state.note_open_child(agent, end);
        drop(old);
        assert_eq!(
            state.take_open_child(agent).map(|end| end.region_id),
            Some(Uuid::from_u128(7))
        );
        drop(current);
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
