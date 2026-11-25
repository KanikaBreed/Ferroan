// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The region frame thread. [D6](../../../docs/DECISIONS.md) keeps it off the
//! async runtime. It ticks at the shipped `FrameTime` and still answers height
//! and layer queries. Each tick steps every avatar that is not sitting on the
//! ground. Login, chat, a local teleport and a hop completion store the pose.
//! An `AgentUpdate` stores movement intent. Teleport packets stay on the circuit.
//!
//! Nothing here blocks a caller. A call that needs an answer goes on the
//! request channel and returns a [`Pending`] reply, a tokio oneshot the async
//! caller awaits. Each circuit has at most one such call outstanding, so that
//! channel is bounded by the circuits. `AgentUpdate`, `SetAlwaysRun` and the
//! child flag are posted with no answer on the bounded inbound queue, behind
//! one coalesced wake. Every request first applies the posts already queued,
//! so one caller's calls apply in the order it made them. The serve's end
//! removes the presence, so a later login starts fresh.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ferroan_diag::{Kind, REGION_SCENE_INBOUND, refuse};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::compress::{self, TerrainLayer};
use crate::error::TerrainError;
use crate::height::{DEFAULT_HEIGHT, Heightmap, movement_z};
use crate::motion::{Body, Motion, MotionUpdate, ReturnData, clock_ms};
use crate::walk;

/// `Constants.MinSimulationHeight`.
const MIN_HEIGHT: f32 = -100.0;
/// `Constants.MaxSimulationHeight`.
const MAX_HEIGHT: f32 = 50_000.0;
/// Substituted when a local teleport offset is outside the region.
const EMERGENCY: [f32; 3] = [128.0, 128.0, 128.0];
/// `TeleportFlags.IsFlying`.
pub const TELEPORT_IS_FLYING: u32 = 1 << 13;

/// A same-region teleport the circuit already accepted. The frame applies the
/// landing (`TeleportAgentWithinRegion`) and stores the pose. The circuit
/// sends the packets.
#[derive(Clone, Copy, Debug)]
pub struct LocalTeleport {
    pub position: [f32; 3],
    pub look_at: [f32; 3],
    pub half_height: f32,
    pub default_landing: [f32; 3],
    pub size_x: f32,
    pub size_y: f32,
    pub flags: u32,
}

/// What a local teleport stored, plus the look and flags the packet uses.
/// The flags gain `IsFlying` when the avatar was flying.
#[derive(Clone, Copy, Debug)]
pub struct TeleportStored {
    pub pose: Presence,
    pub look_at: [f32; 3],
    pub flags: u32,
}

/// The returning `UpdateAgent` for a hop completion.
#[derive(Clone, Copy, Debug)]
pub struct HopReturn {
    pub position: [f32; 3],
    pub look_at: [f32; 3],
    pub data: ReturnData,
}

/// Shipped `[Startup] FrameTime`. The scene constructor's 0.0908 applies only
/// before that ini read (`Scene.cs` lines 863 and 1145 at pin `c401bceecd`).
pub const FRAME_TIME: f32 = 0.0909;

/// Waiting scene posts. C# `packetInbox` is unbounded. Height queries do not
/// use this queue.
pub const SCENE_INBOUND: usize = 256;

/// Per-avatar pose owned by the frame thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Presence {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub control_flags: u32,
    pub draw_distance: f32,
}

/// Where the departing avatar is, for a source hop: `ScenePresence.Flying`
/// and the camera center and rotations `CopyTo` sends (the body rotation
/// after `IsUsableRotation`, the camera from updates the filter kept).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Departure {
    pub flying: bool,
    pub camera_center: [f32; 3],
    pub head_rotation: [f32; 4],
    pub body_rotation: [f32; 4],
}

/// One answer from the frame thread. The call was already made.
#[derive(Debug)]
pub struct Pending<T>(oneshot::Receiver<T>);

impl<T> Pending<T> {
    /// Await the answer. A frame that stopped first is [`TerrainError::Stopped`].
    pub async fn wait(self) -> Result<T, TerrainError> {
        self.0.await.map_err(|_| TerrainError::Stopped)
    }

    /// Block this thread for the answer. Tests only: it panics on a tokio worker.
    #[cfg(test)]
    pub(crate) fn blocking_wait(self) -> Result<T, TerrainError> {
        self.0.blocking_recv().map_err(|_| TerrainError::Stopped)
    }
}

fn pending<T>() -> (oneshot::Sender<T>, Pending<T>) {
    let (tx, rx) = oneshot::channel();
    (tx, Pending(rx))
}

enum Request {
    Height {
        x: f32,
        y: f32,
        reply: oneshot::Sender<f32>,
    },
    Layers {
        reply: oneshot::Sender<Vec<TerrainLayer>>,
    },
    Read {
        agent: Uuid,
        reply: oneshot::Sender<Option<Presence>>,
    },
    Motion {
        agent: Uuid,
        reply: oneshot::Sender<Option<Motion>>,
    },
    Frames {
        reply: oneshot::Sender<u64>,
    },
    /// The serve ended.
    Remove {
        agent: Uuid,
        reply: oneshot::Sender<bool>,
    },
    /// Seat a login pose for chat. The actor waits for the arrival.
    Pose {
        agent: Uuid,
        pose: Presence,
        reply: oneshot::Sender<Presence>,
    },
    /// An arrival: the record with an actor at rest, or this pose when
    /// there is no record yet.
    Arrive {
        agent: Uuid,
        pose: Presence,
        reply: oneshot::Sender<Presence>,
    },
    Places {
        agents: Vec<Uuid>,
        reply: oneshot::Sender<HashMap<Uuid, [f32; 3]>>,
    },
    Teleport {
        agent: Uuid,
        request: LocalTeleport,
        reply: oneshot::Sender<TeleportStored>,
    },
    Complete {
        agent: Uuid,
        hop: HopReturn,
        reply: oneshot::Sender<Presence>,
    },
    Departure {
        agent: Uuid,
        reply: oneshot::Sender<Option<Departure>>,
    },
    Wake,
    Stop,
}

/// Posts with no answer. These share the bounded inbound queue.
enum Inbound {
    /// The avatar is away, or has come back, for the physics tick.
    Child {
        agent: Uuid,
        child: bool,
    },
    Agent {
        agent: Uuid,
        update: MotionUpdate,
    },
    AlwaysRun {
        agent: Uuid,
        always_run: bool,
    },
}

struct Avatar {
    pose: Presence,
    body: Body,
}

/// Owns the heightmap and the login presences on `ferroan-scene`.
pub struct TerrainFrame {
    tx: Sender<Request>,
    inbound: SyncSender<Inbound>,
    /// Set by the first post after the frame last drained. Later posts do not
    /// send another wake until the frame clears it.
    wake: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    has_map: bool,
}

impl TerrainFrame {
    /// Precomputes land packets on the frame thread, then returns.
    pub fn start(map: Heightmap) -> Result<Self, TerrainError> {
        Self::spawn(Some(map))
    }

    /// Tick and store login poses when the region has no heightmap.
    pub fn start_without_map() -> Result<Self, TerrainError> {
        Self::spawn(None)
    }

    fn spawn(map: Option<Heightmap>) -> Result<Self, TerrainError> {
        let has_map = map.is_some();
        let (tx, rx) = mpsc::channel();
        let (inbound, inbound_rx) = mpsc::sync_channel(SCENE_INBOUND);
        let (ready_tx, ready_rx) = mpsc::channel();
        let wake = Arc::new(AtomicBool::new(false));
        let frame_wake = Arc::clone(&wake);
        // Start the packet clock before the first AgentUpdate can be stamped.
        clock_ms(Instant::now());
        let thread = std::thread::Builder::new()
            .name("ferroan-scene".to_owned())
            .spawn(move || run(map, rx, inbound_rx, frame_wake, ready_tx))
            .map_err(|_| TerrainError::Thread)?;
        ready_rx.recv().map_err(|_| TerrainError::Thread)?;
        Ok(Self {
            tx,
            inbound,
            wake,
            thread: Some(thread),
            has_map,
        })
    }

    pub fn has_map(&self) -> bool {
        self.has_map
    }

    pub fn ground(&self, x: f32, y: f32) -> Result<Pending<f32>, TerrainError> {
        if !self.has_map {
            return Err(TerrainError::NoMap);
        }
        let (reply, answer) = pending();
        self.request(Request::Height { x, y, reply })?;
        Ok(answer)
    }

    pub fn layers(&self) -> Result<Pending<Vec<TerrainLayer>>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Layers { reply })?;
        Ok(answer)
    }

    /// Seat one login pose so chat can read it. The actor waits for the
    /// arrival. The answer is the record the frame thread kept.
    pub fn post_login(
        &self,
        agent: Uuid,
        pose: Presence,
    ) -> Result<Pending<Presence>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Pose { agent, pose, reply })?;
        Ok(answer)
    }

    /// An arrival. An existing record keeps its position and gets a physics
    /// actor at rest. With no record, `pose` is stored with an actor. Either
    /// way the actor exists after this one call.
    pub fn arrive(&self, agent: Uuid, pose: Presence) -> Result<Pending<Presence>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Arrive { agent, pose, reply })?;
        Ok(answer)
    }

    /// A forwarded same-region `TeleportLocationRequest`.
    pub fn teleport_local(
        &self,
        agent: Uuid,
        request: LocalTeleport,
    ) -> Result<Pending<TeleportStored>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Teleport {
            agent,
            request,
            reply,
        })?;
        Ok(answer)
    }

    /// A hop completion: the returning `UpdateAgent` as `CopyFrom` stores it,
    /// then the new root actor at rest facing the look.
    pub fn complete_hop(
        &self,
        agent: Uuid,
        hop: HopReturn,
    ) -> Result<Pending<Presence>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Complete { agent, hop, reply })?;
        Ok(answer)
    }

    /// The departing avatar's flight and pose, after every post already
    /// queued. A missing avatar is `None`.
    pub fn departure(&self, agent: Uuid) -> Result<Pending<Option<Departure>>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Departure { agent, reply })?;
        Ok(answer)
    }

    /// Skip the physics tick while the avatar is a child. A missing avatar
    /// is left alone. One inbound slot, no answer.
    pub fn set_child(&self, agent: Uuid, child: bool) -> Result<(), TerrainError> {
        self.push(Inbound::Child { agent, child })
    }

    /// One forwarded `AgentUpdate`. A missing avatar is dropped. One inbound
    /// slot, no answer.
    pub fn agent_update(&self, agent: Uuid, update: MotionUpdate) -> Result<(), TerrainError> {
        self.push(Inbound::Agent { agent, update })
    }

    /// One forwarded `SetAlwaysRun`. A missing avatar is left alone. One
    /// inbound slot, no answer.
    pub fn set_always_run(&self, agent: Uuid, always_run: bool) -> Result<(), TerrainError> {
        self.push(Inbound::AlwaysRun { agent, always_run })
    }

    /// Positions for one forwarded `ChatFromViewer`. Missing agents are omitted.
    pub fn chat_positions(
        &self,
        agents: &[Uuid],
    ) -> Result<Pending<HashMap<Uuid, [f32; 3]>>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Places {
            agents: agents.to_vec(),
            reply,
        })?;
        Ok(answer)
    }

    /// The serve for this agent ended. Every post already queued applies
    /// first, then the presence goes. The answer says whether one was there.
    /// A full inbound queue cannot keep a ghost.
    pub fn remove(&self, agent: Uuid) -> Result<Pending<bool>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Remove { agent, reply })?;
        Ok(answer)
    }

    /// The movement intent. This does not take an inbound slot.
    pub fn motion(&self, agent: Uuid) -> Result<Pending<Option<Motion>>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Motion { agent, reply })?;
        Ok(answer)
    }

    /// The stored pose. This does not take an inbound slot.
    pub fn presence(&self, agent: Uuid) -> Result<Pending<Option<Presence>>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Read { agent, reply })?;
        Ok(answer)
    }

    pub fn frames(&self) -> Result<Pending<u64>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Frames { reply })?;
        Ok(answer)
    }

    fn request(&self, request: Request) -> Result<(), TerrainError> {
        self.tx.send(request).map_err(|_| TerrainError::Stopped)
    }

    fn push(&self, post: Inbound) -> Result<(), TerrainError> {
        push_inbound(&self.inbound, post)?;
        // Wake the tick wait so the post is applied before the next frame.
        // One wake covers every post until the frame drains again.
        if !self.wake.swap(true, Ordering::AcqRel) {
            let _ = self.tx.send(Request::Wake);
        }
        Ok(())
    }
}

impl Drop for TerrainFrame {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn push_inbound(tx: &SyncSender<Inbound>, post: Inbound) -> Result<(), TerrainError> {
    match tx.try_send(post) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            refuse(
                REGION_SCENE_INBOUND,
                Kind::Hard,
                SCENE_INBOUND as u64,
                SCENE_INBOUND as u64 + 1,
                "scene-inbound",
            );
            Err(TerrainError::InboundFull)
        }
        Err(TrySendError::Disconnected(_)) => Err(TerrainError::Stopped),
    }
}

fn frame_wait() -> Duration {
    Duration::from_secs_f32(FRAME_TIME)
}

/// The next tick is one frame after the last one was due, as `Scene.Update`
/// sleeps for what is left of `FrameTime`. A tick that started more than a
/// frame late runs the next one at once and does not try to catch up further.
fn next_tick(due: Instant, started: Instant) -> Instant {
    let next = due + frame_wait();
    if next < started { started } else { next }
}

fn drain_inbound(rx: &Receiver<Inbound>, wake: &AtomicBool, presences: &mut HashMap<Uuid, Avatar>) {
    // Clear before draining. A post that still finds the flag set relies on
    // a wake already sent, and every request drains again on receipt.
    wake.swap(false, Ordering::AcqRel);
    while let Ok(post) = rx.try_recv() {
        apply(post, presences);
    }
}

fn apply(post: Inbound, presences: &mut HashMap<Uuid, Avatar>) {
    match post {
        Inbound::Child { agent, child } => {
            if let Some(avatar) = presences.get_mut(&agent) {
                avatar.body.set_child(child);
            }
        }
        Inbound::Agent { agent, update } => {
            if let Some(avatar) = presences.get_mut(&agent) {
                avatar.body.apply(&update, &mut avatar.pose);
            }
        }
        Inbound::AlwaysRun { agent, always_run } => {
            if let Some(avatar) = presences.get_mut(&agent) {
                avatar.body.set_always_run(always_run);
            }
        }
    }
}

/// One answered call. Returns false for `Stop`.
fn answer(
    request: Request,
    presences: &mut HashMap<Uuid, Avatar>,
    map: Option<&Heightmap>,
    layers: &[TerrainLayer],
    frames: u64,
) -> bool {
    match request {
        Request::Height { x, y, reply } => {
            let height = map.map(|map| map.height_at(x, y)).unwrap_or(0.0);
            let _ = reply.send(height);
        }
        Request::Layers { reply } => {
            let _ = reply.send(layers.to_vec());
        }
        Request::Read { agent, reply } => {
            let _ = reply.send(presences.get(&agent).map(|avatar| avatar.pose));
        }
        Request::Motion { agent, reply } => {
            let _ = reply.send(presences.get(&agent).map(|avatar| avatar.body.view()));
        }
        Request::Frames { reply } => {
            let _ = reply.send(frames);
        }
        Request::Remove { agent, reply } => {
            let _ = reply.send(presences.remove(&agent).is_some());
        }
        Request::Pose { agent, pose, reply } => {
            let mut avatar = Avatar::new(pose);
            // Chat can read this pose immediately. The actor waits until
            // the arrival packet samples it, or a tick would change the
            // position that packet and GridUser still report.
            avatar.body.set_actor(false);
            let pose = avatar.pose;
            presences.insert(agent, avatar);
            let _ = reply.send(pose);
        }
        Request::Arrive { agent, pose, reply } => {
            let avatar = presences.entry(agent).or_insert_with(|| Avatar::new(pose));
            arrive_at_rest(avatar);
            let _ = reply.send(avatar.pose);
        }
        Request::Places { agents, reply } => {
            let mut found = HashMap::new();
            for agent in agents {
                if let Some(avatar) = presences.get(&agent) {
                    found.insert(agent, avatar.pose.position);
                }
            }
            let _ = reply.send(found);
        }
        Request::Teleport {
            agent,
            request,
            reply,
        } => {
            let position = place_teleport(map, &request);
            let look_at = flatten_look(request.look_at);
            let avatar = presences
                .entry(agent)
                .or_insert_with(|| Avatar::new(fresh(position)));
            let mut flags = request.flags;
            if avatar.body.flying() {
                flags |= TELEPORT_IS_FLYING;
            }
            avatar.pose.position = position;
            avatar
                .body
                .turn_to(look_rotation(look_at), &mut avatar.pose);
            avatar.pose.velocity = [0.0, 0.0, 0.0];
            avatar.body.halt_command();
            let _ = reply.send(TeleportStored {
                pose: avatar.pose,
                look_at,
                flags,
            });
        }
        Request::Complete { agent, hop, reply } => {
            let avatar = presences
                .entry(agent)
                .or_insert_with(|| Avatar::new(fresh(hop.position)));
            avatar.pose.position = hop.position;
            avatar.body.apply_return(&hop.data, &mut avatar.pose);
            // CompleteMovement's RotateToLookAt replaces the copied body rotation.
            avatar
                .body
                .turn_to(look_rotation(hop.look_at), &mut avatar.pose);
            arrive_at_rest(avatar);
            let _ = reply.send(avatar.pose);
        }
        Request::Departure { agent, reply } => {
            let departure = presences.get(&agent).map(|avatar| {
                let view = avatar.body.view();
                Departure {
                    flying: avatar.body.flying(),
                    camera_center: view.camera_center,
                    head_rotation: view.head_rotation,
                    body_rotation: view.body_rotation,
                }
            });
            let _ = reply.send(departure);
        }
        Request::Wake => {}
        Request::Stop => return false,
    }
    true
}

impl Avatar {
    fn new(pose: Presence) -> Self {
        Self {
            body: Body::new(&pose),
            pose,
        }
    }
}

/// A new physics actor on arrival or return: `MakeRootAgent` adds one at rest
/// for a teleport. The record stays; the command does not. The tick may run
/// after this returns.
fn arrive_at_rest(avatar: &mut Avatar) {
    avatar.pose.velocity = [0.0, 0.0, 0.0];
    avatar.body.halt_command();
    avatar.body.set_child(false);
    avatar.body.set_actor(true);
}

fn fresh(position: [f32; 3]) -> Presence {
    Presence {
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        velocity: [0.0, 0.0, 0.0],
        control_flags: 0,
        draw_distance: 255.0,
    }
}

/// `TeleportAgentWithinRegion` after the circuit has kept the offset inside
/// this region. Ground height is this frame's map, or the flat default.
/// This is the only copy of the landing rule.
fn place_teleport(map: Option<&Heightmap>, request: &LocalTeleport) -> [f32; 3] {
    let mut position = request.position;
    if !(position[0] >= 0.0
        && position[0] < request.size_x
        && position[1] >= 0.0
        && position[1] < request.size_y)
    {
        position = EMERGENCY;
    }
    if position[0] == 128.0 && position[1] == 128.0 && position[2] == 22.5 {
        position = request.default_landing;
    }
    let ground = map
        .map(|map| map.height_at(position[0], position[1]))
        .unwrap_or(DEFAULT_HEIGHT);
    let limit = ground + request.half_height + 0.1;
    if position[2] < limit && limit.is_finite() {
        position[2] = limit;
    }
    // NaN stays NaN. The bounds are finite and ordered.
    position[2] = position[2].clamp(MIN_HEIGHT, MAX_HEIGHT);
    position
}

fn flatten_look(mut look_at: [f32; 3]) -> [f32; 3] {
    look_at[2] = 0.0;
    if look_at[0].abs() < 0.01 && look_at[1].abs() < 0.01 {
        look_at[0] = 1.0;
        look_at[1] = 0.0;
    }
    look_at
}

/// `RotateToLookAt` when the avatar is not seated. A level heading becomes
/// that yaw. A heading with no horizontal length stays the identity.
fn look_rotation(look_at: [f32; 3]) -> [f32; 4] {
    let horizontal = look_at[0] * look_at[0] + look_at[1] * look_at[1];
    if horizontal < 0.0001 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let cosine = (look_at[0] / horizontal.sqrt()).clamp(-1.0, 1.0);
    let half = cosine.acos() * 0.5;
    let sine = if look_at[1] >= 0.0 {
        half.sin()
    } else {
        -half.sin()
    };
    [0.0, 0.0, sine, half.cos()]
}

fn run(
    map: Option<Heightmap>,
    rx: Receiver<Request>,
    inbound: Receiver<Inbound>,
    wake: Arc<AtomicBool>,
    ready: Sender<()>,
) {
    let layers = map.as_ref().map(compress::land_layers).unwrap_or_default();
    if ready.send(()).is_err() {
        return;
    }
    let mut presences: HashMap<Uuid, Avatar> = HashMap::new();
    let mut frames = 0u64;
    let mut next = Instant::now() + frame_wait();
    loop {
        drain_inbound(&inbound, &wake, &mut presences);
        let now = Instant::now();
        let wait = next.saturating_duration_since(now);
        if wait.is_zero() {
            step_avatars(&mut presences, map.as_ref());
            frames = frames.saturating_add(1);
            next = next_tick(next, now);
            continue;
        }
        let request = match rx.recv_timeout(wait) {
            Ok(request) => request,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        // A call sees every post its caller made before it.
        drain_inbound(&inbound, &wake, &mut presences);
        if !answer(request, &mut presences, map.as_ref(), &layers, frames) {
            break;
        }
    }
}

/// Login Z from `MakeRootAgent` once the frame has reported the ground height.
pub fn landed_z(start_z: f32, ground: f32) -> f32 {
    movement_z(start_z, ground)
}

fn step_avatars(presences: &mut HashMap<Uuid, Avatar>, map: Option<&Heightmap>) {
    // The same monotonic clock the circuit stamps each AgentUpdate with.
    let now = clock_ms(Instant::now());
    let (extent_x, extent_y) = match map {
        Some(map) => (map.width() as f32, map.height() as f32),
        None => (walk::DEFAULT_EXTENT, walk::DEFAULT_EXTENT),
    };
    for avatar in presences.values_mut() {
        let terrain = walk::terrain_height(map, avatar.pose.position, extent_x, extent_y);
        avatar.body.step_physics(
            &mut avatar.pose,
            terrain,
            extent_x,
            extent_y,
            FRAME_TIME,
            now,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::height::Heightmap;
    use crate::motion::{CONTROL_AT_POS, CONTROL_FLY};
    use crate::walk::AVATAR_SIZE_Z;
    use uuid::Uuid;

    fn z_is_written_or_snapped(z: f32, written: f32) -> bool {
        z == written || z == AVATAR_SIZE_Z
    }

    fn pose_at(position: [f32; 3]) -> Presence {
        Presence {
            position,
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0, 0.0, 0.0],
            control_flags: 0,
            draw_distance: 255.0,
        }
    }

    fn login(frame: &TerrainFrame, agent: Uuid, pose: Presence) -> Presence {
        frame
            .post_login(agent, pose)
            .unwrap()
            .blocking_wait()
            .unwrap()
    }

    fn read(frame: &TerrainFrame, agent: Uuid) -> Presence {
        frame
            .presence(agent)
            .unwrap()
            .blocking_wait()
            .unwrap()
            .unwrap()
    }

    fn motion(frame: &TerrainFrame, agent: Uuid) -> Motion {
        frame
            .motion(agent)
            .unwrap()
            .blocking_wait()
            .unwrap()
            .unwrap()
    }

    fn frames(frame: &TerrainFrame) -> u64 {
        frame.frames().unwrap().blocking_wait().unwrap()
    }

    fn places(frame: &TerrainFrame, agents: &[Uuid]) -> HashMap<Uuid, [f32; 3]> {
        frame
            .chat_positions(agents)
            .unwrap()
            .blocking_wait()
            .unwrap()
    }

    /// An arrival that has a record. The posted pose is never used.
    fn reuse(frame: &TerrainFrame, agent: Uuid) -> Presence {
        frame
            .arrive(agent, pose_at([1.0, 1.0, 1.0]))
            .unwrap()
            .blocking_wait()
            .unwrap()
    }

    fn flying(frame: &TerrainFrame, agent: Uuid) -> bool {
        frame
            .departure(agent)
            .unwrap()
            .blocking_wait()
            .unwrap()
            .is_some_and(|departure| departure.flying)
    }

    fn teleport(frame: &TerrainFrame, agent: Uuid, request: LocalTeleport) -> TeleportStored {
        frame
            .teleport_local(agent, request)
            .unwrap()
            .blocking_wait()
            .unwrap()
    }

    fn local(position: [f32; 3], look_at: [f32; 3]) -> LocalTeleport {
        LocalTeleport {
            position,
            look_at,
            half_height: 0.95,
            default_landing: [128.0, 128.0, 30.0],
            size_x: 256.0,
            size_y: 256.0,
            flags: 16,
        }
    }

    fn standing_return(position: [f32; 3]) -> HopReturn {
        HopReturn {
            position,
            look_at: [1.0, 0.0, 0.0],
            data: ReturnData {
                velocity: [0.0, 0.0, 0.0],
                far: 255.0,
                body_rotation: [0.0, 0.0, 0.0, 1.0],
                head_rotation: [0.0, 0.0, 0.0, 1.0],
                control_flags: 0,
                always_run: false,
            },
        }
    }

    fn complete(frame: &TerrainFrame, agent: Uuid, hop: HopReturn) -> Presence {
        frame
            .complete_hop(agent, hop)
            .unwrap()
            .blocking_wait()
            .unwrap()
    }

    fn walking_update(now_ms: u64, sequence: u32, controls: u32) -> MotionUpdate {
        MotionUpdate {
            now_ms,
            sequence,
            child: false,
            in_transit: false,
            body_rotation: [0.0, 0.0, 0.0, 1.0],
            head_rotation: [0.0, 0.0, 0.0, 1.0],
            state: 0,
            camera_center: [128.0, 128.0, 26.0],
            camera_at: [1.0, 0.0, 0.0],
            camera_left: [0.0, 1.0, 0.0],
            camera_up: [0.0, 0.0, 1.0],
            far: 96.0,
            control_flags: controls,
            flags: 0,
        }
    }

    fn wait_frames(frame: &TerrainFrame, extra: u64) {
        let start = frames(frame);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while frames(frame) < start.saturating_add(extra) {
            assert!(std::time::Instant::now() < deadline, "frame did not tick");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_frame_thread_returns_the_height() {
        let mut plain = Vec::new();
        plain.extend_from_slice(&16i32.to_le_bytes());
        plain.extend_from_slice(&16i32.to_le_bytes());
        for _ in 0..256 {
            plain.extend_from_slice(&21.0f32.to_le_bytes());
        }
        let map = Heightmap::from_blob(22, &plain, 16, 16).unwrap();
        let frame = TerrainFrame::start(map).unwrap();
        let ground = frame.ground(4.0, 4.0).unwrap().blocking_wait().unwrap();
        assert_eq!(ground, 21.0);
        assert!((landed_z(10.0, ground) - (21.0 + 0.01 + 0.8)).abs() < f32::EPSILON);
        assert_eq!(frame.layers().unwrap().blocking_wait().unwrap().len(), 1);
    }

    #[test]
    fn the_frame_ticks_and_stores_the_login_pose() {
        let frame = TerrainFrame::start_without_map().unwrap();
        assert!(!frame.has_map());
        assert!(matches!(frame.ground(1.0, 1.0), Err(TerrainError::NoMap)));
        let agent = Uuid::from_u128(7);
        let pose = pose_at([4.0, 5.0, 26.0]);
        assert_eq!(login(&frame, agent, pose), pose);
        assert_eq!(read(&frame, agent).position, [4.0, 5.0, 26.0]);
        let before = frames(&frame);
        std::thread::sleep(Duration::from_millis(400));
        let after = frames(&frame);
        assert!(after > before, "frames {before} -> {after}");
        assert!(after < 10, "frames {after}");
        assert_eq!(FRAME_TIME, 0.0909);
        assert_eq!(read(&frame, agent).position, [4.0, 5.0, 26.0]);
        let released = reuse(&frame, agent);
        assert_eq!(released.position, [4.0, 5.0, 26.0]);
        std::thread::sleep(Duration::from_millis(400));
        let snapped = read(&frame, agent);
        assert_eq!(snapped.position, [4.0, 5.0, AVATAR_SIZE_Z]);
        assert_eq!(snapped.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(snapped.rotation, pose.rotation);
    }

    #[test]
    fn the_next_tick_is_due_one_frame_after_the_last_without_drift() {
        let due = Instant::now();
        // A tick that started a little late keeps the schedule.
        let late = due + Duration::from_millis(30);
        assert_eq!(next_tick(due, late), due + frame_wait());
        // One that started more than a frame late runs again at once.
        let overrun = due + Duration::from_millis(200);
        assert_eq!(next_tick(due, overrun), overrun);
        // Ten on-time ticks are ten frames from the first, not ten frames plus the work.
        let mut next = due;
        for _ in 0..10 {
            next = next_tick(next, next);
        }
        assert_eq!(next, due + frame_wait() * 10);
    }

    #[test]
    fn a_full_scene_inbound_channel_reports_the_limit() {
        let (tx, _rx) = mpsc::sync_channel(SCENE_INBOUND);
        for index in 0..SCENE_INBOUND {
            push_inbound(
                &tx,
                Inbound::AlwaysRun {
                    agent: Uuid::from_u128(index as u128),
                    always_run: true,
                },
            )
            .unwrap();
        }
        let error = push_inbound(
            &tx,
            Inbound::Agent {
                agent: Uuid::from_u128(999),
                update: walking_update(1_000, 1, 0),
            },
        )
        .unwrap_err();
        assert!(matches!(error, TerrainError::InboundFull));
        assert!(ferroan_diag::metrics().contains("region.scene-inbound"));
    }

    #[test]
    fn posts_without_an_answer_are_applied_before_a_later_query() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(12);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        // Many posts share one wake. Each is applied before the query that follows.
        for sequence in 1..=100 {
            frame
                .agent_update(
                    agent,
                    walking_update(1_000 + u64::from(sequence) * 30, sequence, CONTROL_AT_POS),
                )
                .unwrap();
        }
        frame.set_always_run(agent, true).unwrap();
        frame
            .agent_update(agent, walking_update(5_000, 101, CONTROL_AT_POS))
            .unwrap();
        let stored = motion(&frame, agent);
        assert!(stored.always_run);
        assert!((stored.target_velocity[0] - 4.096).abs() < 1.0e-4);
        assert_eq!(read(&frame, agent).draw_distance, 96.0);
        // A missing avatar is left alone and the post still succeeds.
        frame
            .agent_update(Uuid::from_u128(99), walking_update(5_100, 1, 0))
            .unwrap();
        frame.set_always_run(Uuid::from_u128(99), true).unwrap();
        assert!(
            frame
                .motion(Uuid::from_u128(99))
                .unwrap()
                .blocking_wait()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_removed_presence_is_gone_and_a_new_login_starts_fresh() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(13);
        let other = Uuid::from_u128(14);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        login(&frame, other, pose_at([130.0, 128.0, 25.0]));
        reuse(&frame, agent);
        frame
            .agent_update(agent, walking_update(1_000, 500, CONTROL_AT_POS))
            .unwrap();
        frame.set_always_run(agent, true).unwrap();
        teleport(&frame, agent, local([40.0, 200.0, 30.0], [1.0, 0.0, 0.0]));
        // A post queued just before the removal is applied, then the record goes.
        frame
            .agent_update(agent, walking_update(1_100, 501, CONTROL_AT_POS))
            .unwrap();
        assert!(frame.remove(agent).unwrap().blocking_wait().unwrap());
        assert!(!frame.remove(agent).unwrap().blocking_wait().unwrap());
        assert!(
            frame
                .presence(agent)
                .unwrap()
                .blocking_wait()
                .unwrap()
                .is_none()
        );
        assert!(!places(&frame, &[agent, other]).contains_key(&agent));
        assert!(
            frame
                .departure(agent)
                .unwrap()
                .blocking_wait()
                .unwrap()
                .is_none()
        );
        assert!(!flying(&frame, agent));
        // The other avatar stays.
        assert_eq!(places(&frame, &[other]).len(), 1);

        // A re-login gets the pose it posts, a new filter (sequence 1 is kept)
        // and no always-run.
        let fresh = login(&frame, agent, pose_at([10.0, 20.0, 25.0]));
        assert_eq!(fresh.position, [10.0, 20.0, 25.0]);
        assert_eq!(fresh.draw_distance, 255.0);
        let view = motion(&frame, agent);
        assert!(!view.always_run);
        assert_eq!(view.target_velocity, [0.0, 0.0, 0.0]);
        frame
            .agent_update(agent, walking_update(2_000, 1, CONTROL_AT_POS))
            .unwrap();
        let walking = motion(&frame, agent);
        assert_eq!(walking.control_flags, CONTROL_AT_POS);
        assert!(walking.target_velocity[0] > 0.0);
        assert_eq!(read(&frame, agent).position, [10.0, 20.0, 25.0]);
    }

    #[test]
    fn an_arrival_with_no_record_stores_the_pose_with_an_actor() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(17);
        // No seat was posted first: the one arrival call stores and releases.
        let stored = frame
            .arrive(agent, pose_at([20.0, 30.0, 40.0]))
            .unwrap()
            .blocking_wait()
            .unwrap();
        assert_eq!(stored.position, [20.0, 30.0, 40.0]);
        frame
            .agent_update(
                agent,
                walking_update(1_000, 1, CONTROL_FLY | CONTROL_AT_POS),
            )
            .unwrap();
        assert!(flying(&frame, agent));
        wait_frames(&frame, 2);
        assert!(read(&frame, agent).position[0] > 20.0);
    }

    #[test]
    fn answered_calls_do_not_wait_behind_a_full_inbound_queue() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(18);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, agent);
        // A flood of posts with no answer may fill the inbound queue. The
        // answered calls do not use it, so none of them is refused.
        for sequence in 0..(SCENE_INBOUND as u32 * 4) {
            let _ = frame.agent_update(agent, walking_update(1_000, sequence + 1, 0));
        }
        let moved = teleport(&frame, agent, local([40.0, 40.0, 40.0], [1.0, 0.0, 0.0]));
        assert_eq!(moved.pose.position, [40.0, 40.0, 40.0]);
        assert_eq!(reuse(&frame, agent).position, [40.0, 40.0, 40.0]);
        assert_eq!(places(&frame, &[agent]).len(), 1);
    }

    #[test]
    fn a_departure_reports_the_kept_camera_and_a_usable_body_rotation() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(19);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, agent);
        let mut first = walking_update(1_000, 1, 0);
        first.body_rotation = [0.0, 0.0, 0.6, 0.8];
        first.head_rotation = [0.0, 0.0, 0.0, 1.0];
        first.camera_center = [100.0, 100.0, 30.0];
        frame.agent_update(agent, first).unwrap();
        // Movement significant (controls change), camera not (moved 0.005),
        // and a body rotation that is not usable.
        let mut second = walking_update(1_100, 2, CONTROL_AT_POS);
        second.body_rotation = [0.0, 0.0, 0.0, 0.0];
        second.head_rotation = [0.0, 0.0, 0.6, 0.8];
        second.camera_center = [100.005, 100.0, 30.0];
        frame.agent_update(agent, second).unwrap();
        let departure = frame
            .departure(agent)
            .unwrap()
            .blocking_wait()
            .unwrap()
            .unwrap();
        assert!(!departure.flying);
        assert_eq!(departure.camera_center, [100.0, 100.0, 30.0]);
        assert_eq!(departure.body_rotation, [0.0, 0.0, 0.6, 0.8]);
        assert_eq!(departure.head_rotation, [0.0, 0.0, 0.6, 0.8]);
    }

    #[test]
    fn a_landing_is_the_position_chat_reads() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let speaker = Uuid::from_u128(1);
        let listener = Uuid::from_u128(2);
        login(&frame, speaker, pose_at([128.0, 128.0, 25.0]));
        login(&frame, listener, pose_at([130.0, 128.0, 25.0]));
        teleport(&frame, speaker, local([40.0, 200.0, 30.0], [1.0, 0.0, 0.0]));
        let found = places(&frame, &[speaker, listener]);
        let speaker_at = found.get(&speaker).copied().unwrap();
        let listener_at = found.get(&listener).copied().unwrap();
        assert_eq!(speaker_at[0], 40.0);
        assert_eq!(speaker_at[1], 200.0);
        assert!(z_is_written_or_snapped(speaker_at[2], 30.0));
        assert_eq!(listener_at[0], 130.0);
        assert_eq!(listener_at[1], 128.0);
        assert!(z_is_written_or_snapped(listener_at[2], 25.0));
        assert!(places(&frame, &[Uuid::from_u128(9)]).is_empty());
    }

    #[test]
    fn a_local_teleport_and_a_hop_completion_write_the_pose() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(3);
        login(
            &frame,
            agent,
            Presence {
                position: [128.0, 128.0, 25.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                velocity: [1.0, 0.0, 0.0],
                control_flags: 4,
                draw_distance: 64.0,
            },
        );
        let moved = teleport(&frame, agent, local([40.0, 200.0, 30.0], [0.0, 0.0, 1.0]));
        assert_eq!(moved.pose.position, [40.0, 200.0, 30.0]);
        assert_eq!(moved.look_at, [1.0, 0.0, 0.0]);
        assert_eq!(moved.flags, 16);
        assert_eq!(moved.pose.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(moved.pose.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(moved.pose.control_flags, 4);
        assert_eq!(moved.pose.draw_distance, 64.0);
        let lifted = teleport(&frame, agent, local([10.0, 10.0, 1.0], [1.0, 0.0, 0.0]));
        let ground = 21.0 + 0.95 + 0.1;
        assert!((lifted.pose.position[2] - ground).abs() < 0.001);
        assert_eq!(lifted.pose.position[0], 10.0);
        let emergency = teleport(&frame, agent, local([-1.0, 10.0, 30.0], [1.0, 0.0, 0.0]));
        assert_eq!(emergency.pose.position, [128.0, 128.0, 128.0]);
        let sentinel = teleport(&frame, agent, local([128.0, 128.0, 22.5], [0.0, 0.0, 4.0]));
        assert_eq!(sentinel.pose.position, [128.0, 128.0, 30.0]);
        assert_eq!(sentinel.look_at, [1.0, 0.0, 0.0]);
        let low = teleport(&frame, agent, local([40.0, 40.0, -500.0], [1.0, 0.0, 0.0]));
        assert!((low.pose.position[2] - ground).abs() < 0.001);
        let high = teleport(
            &frame,
            agent,
            local([40.0, 40.0, 60_000.0], [1.0, 0.0, 0.0]),
        );
        assert_eq!(high.pose.position[2], MAX_HEIGHT);
        let tilted = teleport(&frame, agent, local([40.0, 40.0, 30.0], [0.2, -0.4, 9.0]));
        assert_eq!(tilted.look_at, [0.2, -0.4, 0.0]);
        let yaw = teleport(&frame, agent, local([50.0, 50.0, 40.0], [0.0, 1.0, 0.0]));
        assert_eq!(yaw.pose.position, [50.0, 50.0, 40.0]);
        let quarter = std::f32::consts::FRAC_1_SQRT_2;
        assert!((yaw.pose.rotation[2] - quarter).abs() < 1e-5);
        assert!((yaw.pose.rotation[3] - quarter).abs() < 1e-5);
        assert_eq!(yaw.pose.rotation[0], 0.0);
        assert_eq!(yaw.pose.rotation[1], 0.0);
        assert_eq!(motion(&frame, agent).body_rotation, yaw.pose.rotation);

        let traveller = Uuid::from_u128(4);
        login(&frame, traveller, pose_at([8.0, 8.0, 22.0]));
        let completed = complete(&frame, traveller, standing_return([128.0, 128.0, 30.0]));
        assert_eq!(completed.position, [128.0, 128.0, 30.0]);
        assert_eq!(completed.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(completed.control_flags, 0);
        assert_eq!(completed.draw_distance, 255.0);
        let chat = places(&frame, &[traveller])
            .get(&traveller)
            .copied()
            .unwrap();
        assert_eq!(chat[0], 128.0);
        assert_eq!(chat[1], 128.0);
        assert!(z_is_written_or_snapped(chat[2], 30.0));
    }

    #[test]
    fn a_flying_avatar_teleports_with_is_flying() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(15);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        // Before the arrival samples the pose there is no actor, so no flight.
        frame
            .agent_update(agent, walking_update(1_000, 1, CONTROL_FLY))
            .unwrap();
        assert!(!flying(&frame, agent));
        assert_eq!(
            teleport(&frame, agent, local([40.0, 40.0, 40.0], [1.0, 0.0, 0.0])).flags,
            16
        );
        reuse(&frame, agent);
        assert!(flying(&frame, agent));
        let flown = teleport(&frame, agent, local([50.0, 50.0, 60.0], [1.0, 0.0, 0.0]));
        assert_eq!(flown.flags, 16 | TELEPORT_IS_FLYING);
        // Still flying on the landing: the fly flag stays.
        assert!(motion(&frame, agent).flying);
        frame
            .agent_update(agent, walking_update(2_000, 2, 0))
            .unwrap();
        assert!(!flying(&frame, agent));
        let walked = teleport(&frame, agent, local([60.0, 60.0, 30.0], [1.0, 0.0, 0.0]));
        assert_eq!(walked.flags, 16);
        // A child has no actor and does not fly.
        frame
            .agent_update(agent, walking_update(3_000, 3, CONTROL_FLY))
            .unwrap();
        frame.set_child(agent, true).unwrap();
        assert!(!flying(&frame, agent));
        assert!(!flying(&frame, Uuid::from_u128(99)));
    }

    #[test]
    fn a_hop_completion_takes_the_returning_agent_data() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(16);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, agent);
        // Before departure: walking, far 96, always-run on.
        frame
            .agent_update(agent, walking_update(1_000, 1, CONTROL_AT_POS))
            .unwrap();
        frame.set_always_run(agent, true).unwrap();
        frame.set_child(agent, true).unwrap();
        let yaw = std::f32::consts::FRAC_1_SQRT_2;
        let stored = complete(
            &frame,
            agent,
            HopReturn {
                position: [20.0, 30.0, 40.0],
                look_at: [0.0, 1.0, 0.0],
                data: ReturnData {
                    velocity: [3.0, 0.0, 0.0],
                    far: 200.0,
                    body_rotation: [0.0, 0.0, 1.0, 0.0],
                    head_rotation: [0.0, 0.0, 0.25, 0.75],
                    control_flags: CONTROL_FLY,
                    always_run: false,
                },
            },
        );
        assert_eq!(stored.position, [20.0, 30.0, 40.0]);
        assert_eq!(stored.draw_distance, 200.0);
        assert_eq!(stored.control_flags, CONTROL_FLY);
        // The new root actor starts at rest, facing the look.
        assert_eq!(stored.velocity, [0.0, 0.0, 0.0]);
        assert!((stored.rotation[2] - yaw).abs() < 1e-5);
        assert!((stored.rotation[3] - yaw).abs() < 1e-5);
        let view = motion(&frame, agent);
        assert!(!view.always_run);
        assert!(view.flying);
        assert_eq!(view.head_rotation, [0.0, 0.0, 0.25, 0.75]);
        assert_eq!(view.target_velocity, [0.0, 0.0, 0.0]);
        assert_eq!(view.draw_distance, 200.0);
        assert!(flying(&frame, agent));
        // A far below 32 is clamped as `DrawDistance` is.
        let mut near = standing_return([20.0, 30.0, 40.0]);
        near.data.far = 8.0;
        assert_eq!(complete(&frame, agent, near).draw_distance, 32.0);
    }

    #[test]
    fn a_return_rests_and_a_child_is_not_stepped() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(21);
        login(
            &frame,
            agent,
            Presence {
                position: [8.0, 8.0, 25.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                velocity: [2.0, 0.0, 0.0],
                control_flags: 0,
                draw_distance: 64.0,
            },
        );
        frame.set_child(Uuid::from_u128(99), true).unwrap();
        frame
            .agent_update(agent, walking_update(1_000, 1, CONTROL_AT_POS))
            .unwrap();
        assert!(motion(&frame, agent).target_velocity[0] > 0.0);
        let completed = complete(&frame, agent, standing_return([128.0, 128.0, 30.0]));
        assert_eq!(completed.position, [128.0, 128.0, 30.0]);
        assert_eq!(completed.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(motion(&frame, agent).target_velocity, [0.0, 0.0, 0.0]);
        wait_frames(&frame, 2);
        let later = read(&frame, agent);
        assert_eq!(later.position[0], 128.0);
        assert_eq!(later.position[1], 128.0);
        assert_eq!(later.velocity, [0.0, 0.0, 0.0]);
        assert!(z_is_written_or_snapped(later.position[2], 30.0));
        frame
            .agent_update(agent, walking_update(2_000, 2, CONTROL_AT_POS))
            .unwrap();
        assert!(motion(&frame, agent).target_velocity[0] > 0.0);
        frame.set_child(agent, true).unwrap();
        let parked = read(&frame, agent).position;
        wait_frames(&frame, 2);
        assert_eq!(read(&frame, agent).position, parked);
        let reused = reuse(&frame, agent);
        assert_eq!(reused.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(reused.position[0], parked[0]);
        assert_eq!(reused.position[1], parked[1]);
        assert_eq!(motion(&frame, agent).target_velocity, [0.0, 0.0, 0.0]);
        wait_frames(&frame, 2);
        let home = read(&frame, agent);
        assert_eq!(home.position[0], parked[0]);
        assert_eq!(home.position[1], parked[1]);
        assert_eq!(home.velocity, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_teleport_stops_the_command_and_keeps_always_run() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(11);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        frame
            .agent_update(agent, walking_update(1_000, 1, CONTROL_AT_POS))
            .unwrap();
        frame.set_always_run(agent, true).unwrap();
        assert!(motion(&frame, agent).target_velocity[0] > 0.0);
        teleport(&frame, agent, local([40.0, 200.0, 30.0], [1.0, 0.0, 0.0]));
        let stopped = motion(&frame, agent);
        assert_eq!(stopped.target_velocity, [0.0, 0.0, 0.0]);
        assert!(stopped.always_run);
        assert_eq!(read(&frame, agent).velocity, [0.0, 0.0, 0.0]);
    }
}
