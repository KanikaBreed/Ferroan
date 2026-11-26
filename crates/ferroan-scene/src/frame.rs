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
//!
//! Each tick posts one physics heartbeat and reads the pose only after that
//! reply. The pose is not written onto avatars. The stand-in still steps them.
//! A cached position read must not call `Physics::sample`: that call waits
//! for the step, as `TryGetBodyState` does.
//!
//! After each tick the frame runs `ScenePresence.Update`'s terse test on every
//! attached viewer's avatar ([`crate::terse`]) and offers a due update to
//! every attached viewer's feed. The circuit drains its feed. Arrivals,
//! `SendInitialData` and departures also queue the other avatars' full
//! updates, appearances and kills on the feeds ([`crate::avatars`]). Every
//! 50th frame each open viewer gets the minimap dots ([`crate::coarse`]).
//! The frame and circuit share avatar local IDs as in
//! `Source/OpenSim.Region.Framework/Scenes/{SceneBase,ScenePresence}.cs`
//! at Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ferroan_config::Config;
use ferroan_diag::{Kind, REGION_SCENE_INBOUND, refuse};
use ferroan_physics::{Physics, Pose};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::avatars::{AvatarFull, AvatarLook, AvatarPacket, block_rotation};
use crate::coarse::{self, COARSE_EVERY_FRAMES, CoarseState};
use crate::compress::{self, TerrainLayer};
use crate::error::TerrainError;
use crate::height::{DEFAULT_HEIGHT, Heightmap, movement_z};
use crate::motion::{Body, Motion, MotionUpdate, ReturnData, clock_ms};
use crate::prioritizer::{PriorityEntity, PriorityViewer, best_avatar_responsiveness};
use crate::stored_scene::StoredScene;
use crate::terse::{
    AvatarBoard, AvatarTerse, InitialData, InitialStep, LastSent, UNIT_W, ViewerFeed, look_at,
};
use crate::{ObjectGroup, PhysicsScaleLimits, PrimScaleLimits, walk};

/// `Constants.MinSimulationHeight`.
const MIN_HEIGHT: f32 = -100.0;
/// `Constants.MaxSimulationHeight`.
const MAX_HEIGHT: f32 = 50_000.0;
/// Substituted when a local teleport offset is outside the region.
const EMERGENCY: [f32; 3] = [128.0, 128.0, 128.0];
/// The region size the minimap scales to. `ferroan-region` refuses any
/// other size.
const REGION_SIZE: [u32; 2] = [256, 256];
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
    pub always_run: bool,
    pub flying: bool,
    pub camera_center: [f32; 3],
    pub head_rotation: [f32; 4],
    pub body_rotation: [f32; 4],
}

/// What GridUser's logout reads when a presence leaves: `AbsolutePosition`
/// and `Lookat` (`ActivityDetector.OnConnectionClose`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leaving {
    pub position: [f32; 3],
    pub look_at: [f32; 3],
}

/// One viewer attached to the frame: the feed its circuit drains, the seed
/// flag `SendInitialData` waits for (`CapsFlags.SentSeeds`), whether its
/// circuit is open, and what the other viewers are told about its avatar.
pub struct ViewerLink {
    pub feed: Arc<ViewerFeed>,
    pub seeds_sent: Arc<AtomicBool>,
    /// `LLClientView.IsActive`: set once `UseCircuitCode` opened the circuit.
    /// C# has no client before that, so it sends the viewer no minimap dots.
    pub circuit_open: Arc<AtomicBool>,
    pub look: Arc<AvatarLook>,
    /// `TeleportFlags` from the accepted login. Zero asks for terrain and the
    /// overlay again at `SendInitialData` (`ScenePresence.cs` 4338).
    pub teleport_flags: u32,
}

/// A root that hopped away has arrived at its destination
/// (`HasMovedAway`, then `MakeChildAgent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedAway {
    /// The destination is within view range: `HasMovedAway(true)`, which
    /// sends no kill outside a hidden parcel.
    pub near: bool,
    /// Roots with a child agent in the destination (`knowsNeighbourRegion`).
    /// `MakeChildAgent` does not kill the avatar on their viewers.
    pub known_by: Vec<Uuid>,
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
    /// The pose from the latest physics heartbeat. Empty until the first tick.
    #[cfg(test)]
    PhysicsPose {
        reply: oneshot::Sender<Pose>,
    },
    /// The serve ended.
    Remove {
        agent: Uuid,
        reply: oneshot::Sender<Option<Leaving>>,
    },
    /// A viewer's feed. The answer is the avatar's local id.
    Attach {
        agent: Uuid,
        link: ViewerLink,
        reply: oneshot::Sender<u32>,
    },
    /// Seat a login pose for chat. The actor waits for the arrival.
    Pose {
        agent: Uuid,
        pose: Presence,
        reply: oneshot::Sender<Presence>,
    },
    /// An arrival: the record with an actor at rest, or this pose when
    /// there is no record yet, turned to the look the arrival packet sends.
    Arrive {
        agent: Uuid,
        pose: Presence,
        look_at: [f32; 3],
        reply: oneshot::Sender<Presence>,
    },
    /// A hop away reached its destination.
    MovedAway {
        agent: Uuid,
        away: MovedAway,
        reply: oneshot::Sender<()>,
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
    /// `RegionHandshakeReply`.
    HandshakeReply {
        agent: Uuid,
    },
}

struct Avatar {
    pose: Presence,
    body: Body,
}

/// An attached viewer, the terse state of its avatar, and its minimap state.
struct Viewer {
    local_id: u32,
    link: ViewerLink,
    initial: InitialData,
    last: LastSent,
    coarse: CoarseState,
}

/// The presences, the attached viewers and the board their blocks read.
struct Scene {
    presences: HashMap<Uuid, Avatar>,
    viewers: HashMap<Uuid, Viewer>,
    board: AvatarBoard,
    local_ids: Arc<LocalIds>,
}

/// Owns the heightmap and the login presences on `ferroan-scene`.
pub struct TerrainFrame {
    tx: Sender<Request>,
    inbound: SyncSender<Inbound>,
    /// Set by the first post after the frame last drained. Later posts do not
    /// send another wake until the frame clears it.
    wake: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    parcels: Option<crate::parcels::ParcelMap>,
    has_map: bool,
    objects: Arc<StoredScene>,
    local_ids: Arc<LocalIds>,
    board: AvatarBoard,
}

impl TerrainFrame {
    /// Precomputes land packets on the frame thread, then returns.
    pub fn start(map: Heightmap) -> Result<Self, TerrainError> {
        Self::start_region(Some(map), None)
    }

    /// Tick and store login poses when the region has no heightmap.
    pub fn start_without_map() -> Result<Self, TerrainError> {
        Self::start_region(None, None)
    }

    /// Start the frame with its immutable startup parcel map.
    pub fn start_region(
        map: Option<Heightmap>,
        parcels: Option<crate::parcels::ParcelMap>,
    ) -> Result<Self, TerrainError> {
        Self::with_object_settings(
            map,
            parcels,
            Arc::default(),
            None,
            PhysicsScaleLimits::default(),
            true,
            default_jolt_config()?,
        )
    }

    /// Admit this region's restored groups before the first avatar can attach.
    pub fn with_objects(
        map: Option<Heightmap>,
        groups: Arc<Vec<ObjectGroup>>,
        limits: Option<PrimScaleLimits>,
    ) -> Result<Self, TerrainError> {
        Self::with_object_settings(
            map,
            None,
            groups,
            limits,
            PhysicsScaleLimits::default(),
            true,
            default_jolt_config()?,
        )
    }

    /// Use the region's configured scale limits, CollidablePrims setting and
    /// ini. `config` is passed to `Physics::start`, so `[Jolt]` keys and the
    /// warning for a `[Startup] physics` other than `Jolt` apply here.
    pub fn with_object_settings(
        map: Option<Heightmap>,
        parcels: Option<crate::parcels::ParcelMap>,
        groups: Arc<Vec<ObjectGroup>>,
        limits: Option<PrimScaleLimits>,
        physics: PhysicsScaleLimits,
        collidable_prims: bool,
        config: Config,
    ) -> Result<Self, TerrainError> {
        let has_map = map.is_some();
        let (tx, rx) = mpsc::channel();
        let (inbound, inbound_rx) = mpsc::sync_channel(SCENE_INBOUND);
        let (ready_tx, ready_rx) = mpsc::channel();
        let wake = Arc::new(AtomicBool::new(false));
        let frame_wake = Arc::clone(&wake);
        let board = AvatarBoard::default();
        let local_ids = Arc::new(LocalIds::default());
        let objects = Arc::new(StoredScene::admit(
            groups,
            limits,
            physics,
            collidable_prims,
            || local_ids.allocate(),
        ));
        let scene = Scene {
            presences: HashMap::new(),
            viewers: HashMap::new(),
            board: board.clone(),
            local_ids: Arc::clone(&local_ids),
        };
        // Start the packet clock before the first AgentUpdate can be stamped.
        clock_ms(Instant::now());
        let thread = std::thread::Builder::new()
            .name("ferroan-scene".to_owned())
            .spawn(move || run(map, scene, rx, inbound_rx, frame_wake, ready_tx, config))
            .map_err(|_| TerrainError::Thread)?;
        ready_rx.recv().map_err(|_| TerrainError::Thread)?;
        Ok(Self {
            tx,
            inbound,
            wake,
            thread: Some(thread),
            has_map,
            parcels,
            objects,
            local_ids,
            board,
        })
    }

    pub fn parcels(&self) -> Option<&crate::parcels::ParcelMap> {
        self.parcels.as_ref()
    }

    /// Draw from the frame's single region-wide counter (`SceneBase.AllocateLocalId`).
    pub fn allocate_local_id(&self) -> u32 {
        self.local_ids.allocate()
    }

    /// The admitted groups retained by the simulation service.
    pub fn stored_groups(&self) -> Arc<Vec<ObjectGroup>> {
        Arc::clone(&self.objects.groups)
    }

    /// A feed for one viewer, reading this frame's board.
    pub fn new_feed(&self) -> Arc<ViewerFeed> {
        Arc::new(ViewerFeed::with_objects(
            self.board.clone(),
            Arc::clone(&self.objects),
        ))
    }

    /// Attach one viewer before its login pose. The answer is the avatar's
    /// local id, which the full update and every terse block carry. A
    /// second attach for the agent replaces the first and keeps its id.
    pub fn attach(&self, agent: Uuid, link: ViewerLink) -> Result<Pending<u32>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Attach { agent, link, reply })?;
        Ok(answer)
    }

    /// One `RegionHandshakeReply`. A missing viewer is left alone. One
    /// inbound slot, no answer.
    pub fn handshake_reply(&self, agent: Uuid) -> Result<(), TerrainError> {
        self.push(Inbound::HandshakeReply { agent })
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
    pub fn arrive(
        &self,
        agent: Uuid,
        pose: Presence,
        look_at: [f32; 3],
    ) -> Result<Pending<Presence>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Arrive {
            agent,
            pose,
            look_at,
            reply,
        })?;
        Ok(answer)
    }

    /// A hop away reached its destination: the kills C# sends then. The
    /// child flag was already set before the update. A missing viewer is
    /// left alone.
    pub fn moved_away(&self, agent: Uuid, away: MovedAway) -> Result<Pending<()>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::MovedAway { agent, away, reply })?;
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
    /// then the new root at rest facing the look. Its actor waits for the
    /// next `arrive`, so the position stays the one this answer carries.
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
    /// first, then the presence and its viewer go. The answer is where the
    /// presence was and its look, or `None` when there was none. A full
    /// inbound queue cannot keep a ghost.
    pub fn remove(&self, agent: Uuid) -> Result<Pending<Option<Leaving>>, TerrainError> {
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

    /// The pose the last heartbeat reply stored. Crate-internal: a cached
    /// position read must not call [`Physics::sample`].
    #[cfg(test)]
    pub(crate) fn last_physics_pose(&self) -> Result<Pending<Pose>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::PhysicsPose { reply })?;
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

fn drain_inbound(rx: &Receiver<Inbound>, wake: &AtomicBool, scene: &mut Scene) {
    // Clear before draining. A post that still finds the flag set relies on
    // a wake already sent, and every request drains again on receipt.
    wake.swap(false, Ordering::AcqRel);
    while let Ok(post) = rx.try_recv() {
        apply(post, scene);
    }
}

fn apply(post: Inbound, scene: &mut Scene) {
    let presences = &mut scene.presences;
    match post {
        Inbound::Child { agent, child } => {
            if let Some(avatar) = presences.get_mut(&agent) {
                avatar.body.set_child(child);
            }
            if child && let Some(viewer) = scene.viewers.get_mut(&agent) {
                viewer.initial.made_child();
            }
        }
        Inbound::HandshakeReply { agent } => {
            if let Some(viewer) = scene.viewers.get_mut(&agent) {
                viewer.initial.handshake_reply();
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
    scene: &mut Scene,
    map: Option<&Heightmap>,
    layers: &[TerrainLayer],
    frames: u64,
    physics_pose: Pose,
) -> bool {
    #[cfg(not(test))]
    let _ = physics_pose;
    let presences = &mut scene.presences;
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
        #[cfg(test)]
        Request::PhysicsPose { reply } => {
            let _ = reply.send(physics_pose);
        }
        Request::Remove { agent, reply } => {
            let root = presences
                .get(&agent)
                .is_some_and(|avatar| !avatar.body.is_child());
            if let Some(viewer) = scene.viewers.remove(&agent) {
                scene.board.remove(viewer.local_id);
                // `RemoveClient` kills a root on every client.
                if root {
                    scene.kill_everywhere(viewer.local_id);
                }
            }
            let presences = &mut scene.presences;
            let leaving = presences.remove(&agent).map(|avatar| Leaving {
                position: avatar.pose.position,
                look_at: look_at(avatar.body.camera_at_axis()),
            });
            let _ = reply.send(leaving);
        }
        Request::Attach { agent, link, reply } => {
            let local_id = match scene.viewers.get(&agent) {
                Some(viewer) => viewer.local_id,
                None => scene.local_ids.allocate(),
            };
            scene.viewers.insert(
                agent,
                Viewer {
                    local_id,
                    link,
                    initial: InitialData::default(),
                    last: LastSent::default(),
                    coarse: CoarseState::default(),
                },
            );
            let _ = reply.send(local_id);
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
        Request::Arrive {
            agent,
            pose,
            look_at,
            reply,
        } => {
            let avatar = presences.entry(agent).or_insert_with(|| Avatar::new(pose));
            arrive_at_rest(avatar);
            // `CompleteMovement` turns a non-crossing arrival to the look it
            // sends in `AgentMovementComplete` (`RotateToLookAt`).
            avatar
                .body
                .turn_to(look_rotation(look_at), &mut avatar.pose);
            let pose = avatar.pose;
            scene.announce(agent);
            let _ = reply.send(pose);
        }
        Request::MovedAway { agent, away, reply } => {
            scene.moved_away(agent, &away);
            let _ = reply.send(());
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
            // C# sends a terse update here without the thresholds
            // (`ScenePresence.cs` line 1880). Ferroan leaves it to the next
            // frame's test, so a landing under 0.05 m away sends nothing.
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
            hold_for_arrival(avatar);
            let _ = reply.send(avatar.pose);
        }
        Request::Departure { agent, reply } => {
            let departure = presences.get(&agent).map(|avatar| {
                let view = avatar.body.view();
                Departure {
                    always_run: view.always_run,
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

/// A new physics actor on arrival: `MakeRootAgent` adds one at rest
/// for a teleport. The record stays; the command does not. The tick may run
/// after this returns.
fn arrive_at_rest(avatar: &mut Avatar) {
    avatar.pose.velocity = [0.0, 0.0, 0.0];
    avatar.body.halt_command();
    avatar.body.set_child(false);
    avatar.body.set_actor(true);
}

/// A hop completion: root and at rest, as `arrive_at_rest`, but the actor
/// waits for the arrival packet's sample (`Request::Arrive`), as a login
/// pose does. The circuit reports this position to GridUser and Presence
/// before it sends `AgentMovementComplete`. A tick in between would move
/// the avatar, and the packet would not carry the landing GridUser got.
fn hold_for_arrival(avatar: &mut Avatar) {
    avatar.pose.velocity = [0.0, 0.0, 0.0];
    avatar.body.halt_command();
    avatar.body.set_child(false);
    avatar.body.set_actor(false);
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

/// `[Startup] physics = Jolt` and no `[Jolt]` keys. Callers that have the
/// region's ini pass that to [`TerrainFrame::with_object_settings`] instead.
fn default_jolt_config() -> Result<Config, TerrainError> {
    Config::parse("[Startup]\nphysics = Jolt\n").map_err(|_| TerrainError::Thread)
}

fn run(
    map: Option<Heightmap>,
    mut scene: Scene,
    rx: Receiver<Request>,
    inbound: Receiver<Inbound>,
    wake: Arc<AtomicBool>,
    ready: Sender<()>,
    config: Config,
) {
    let layers = map.as_ref().map(compress::land_layers).unwrap_or_default();
    // The frame's own period, not `[Startup] FrameTime`. The region's ini
    // supplies `[Jolt]` and `[Startup] physics`. Drop detaches this thread;
    // the frame does not join it.
    let Ok(physics) = Physics::start(&config, FRAME_TIME) else {
        return;
    };
    if ready.send(()).is_err() {
        return;
    }
    let mut frames = 0u64;
    let mut physics_pose = Pose {
        ready: false,
        last_count: 0,
        total_steps: 0,
    };
    let mut next = Instant::now() + frame_wait();
    loop {
        drain_inbound(&inbound, &wake, &mut scene);
        let now = Instant::now();
        let wait = next.saturating_duration_since(now);
        if wait.is_zero() {
            // Pose after the reply, then the stand-in. P3.64 reads a box pose.
            // P3.76 is what removes the stand-in. A refused heartbeat is logged.
            // The time is not dropped without a trace.
            match physics.frame_tick(f64::from(FRAME_TIME)) {
                Ok(pose) => physics_pose = pose,
                Err(error) => log::error!("ferroan-scene: physics heartbeat: {error}"),
            }
            frames = tick(&mut scene, map.as_ref(), frames);
            next = next_tick(next, now);
            continue;
        }
        let request = match rx.recv_timeout(wait) {
            Ok(request) => request,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        // A call sees every post its caller made before it.
        drain_inbound(&inbound, &wake, &mut scene);
        if !answer(
            request,
            &mut scene,
            map.as_ref(),
            &layers,
            frames,
            physics_pose,
        ) {
            break;
        }
    }
}

/// One frame: step every avatar, run `ScenePresence.Update` on each, count
/// the frame, and send the minimap dots on every 50th. Returns the count.
fn tick(scene: &mut Scene, map: Option<&Heightmap>, frames: u64) -> u64 {
    step_avatars(&mut scene.presences, map);
    update_presences(scene);
    // `Scene.Update` counts the frame first, then tests it.
    let frames = frames.saturating_add(1);
    if frames.is_multiple_of(COARSE_EVERY_FRAMES) {
        scene.send_coarse_locations();
    }
    frames
}

/// `Scene.AllocateLocalId` starts at a random value below a quarter of the
/// range (`Scene.cs` line 869). The clock stands in for the random source.
fn first_local_id() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    nanos % (u32::MAX / 4)
}

/// One id space for the region, including its frame and outbound avatar packets.
#[derive(Debug)]
struct LocalIds(AtomicU32);

impl Default for LocalIds {
    fn default() -> Self {
        Self(AtomicU32::new(first_local_id()))
    }
}

impl LocalIds {
    /// SceneBase.AllocateLocalId, Pin8 c401bceecd. Skip zero on wrap.
    fn allocate(&self) -> u32 {
        loop {
            let id = self.0.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            if id != 0 {
                return id;
            }
        }
    }
}

/// What `CreateAvatartImprovedTerseBlock` and the thresholds read from one
/// presence. The stand-in never sits on an object and sets no collision plane.
fn terse_of(local_id: u32, avatar: &Avatar) -> AvatarTerse {
    let view = avatar.body.view();
    AvatarTerse {
        local_id,
        state: view.state,
        collision_plane: UNIT_W,
        position: avatar.pose.position,
        velocity: avatar.pose.velocity,
        rotation: avatar.pose.rotation,
        angular_velocity: view.angular_velocity,
        full_rotation: avatar.body.flying(),
    }
}

/// `ScenePresence.Update` for each attached viewer's avatar, after the tick:
/// the board gets the live values, and a due update goes to every attached
/// viewer (`SendTerseUpdateToAllClients`), the avatar's own included. A
/// viewer whose wait ends on this frame runs `SendInitialData` instead. A
/// child, and a login or return the arrival has not sampled, is not tested.
/// The other side's priority is `BestAvatarResponsiveness`.
fn update_presences(scene: &mut Scene) {
    let mut due = Vec::new();
    let mut initial = Vec::new();
    for (agent, viewer) in &mut scene.viewers {
        let avatar = scene.presences.get(agent);
        if let Some(avatar) = avatar {
            scene.board.write(terse_of(viewer.local_id, avatar));
        }
        let seeds = viewer.link.seeds_sent.load(Ordering::Acquire);
        match viewer.initial.frame(seeds) {
            InitialStep::Wait => continue,
            InitialStep::Send => {
                initial.push(*agent);
                continue;
            }
            InitialStep::Test => {}
        }
        let Some(avatar) = avatar else {
            continue;
        };
        if avatar.body.is_child() {
            continue;
        }
        let now = terse_of(viewer.local_id, avatar);
        if viewer.last.due(&now) {
            viewer.last.store(&now);
            due.push((*agent, viewer.local_id));
        }
    }
    for agent in initial {
        scene.send_initial_data(agent);
    }
    for (avatar_agent, local_id) in due {
        for (viewer_agent, viewer) in &scene.viewers {
            let looker = PriorityViewer {
                agent_id: *viewer_agent,
                position: scene
                    .presences
                    .get(viewer_agent)
                    .map(|avatar| avatar.pose.position),
                sitting_on: None,
            };
            let entity = PriorityEntity::Avatar { id: avatar_agent };
            let priority = best_avatar_responsiveness(&looker, &entity);
            viewer.link.feed.offer_terse(priority, local_id);
        }
    }
}

impl Scene {
    /// Attached viewers in local id order, which is the order they attached.
    fn viewer_order(&self) -> Vec<(Uuid, u32)> {
        let mut order: Vec<_> = self
            .viewers
            .iter()
            .map(|(agent, viewer)| (*agent, viewer.local_id))
            .collect();
        order.sort_by_key(|(_, local_id)| *local_id);
        order
    }

    fn is_root(&self, agent: &Uuid) -> bool {
        self.presences
            .get(agent)
            .is_some_and(|avatar| !avatar.body.is_child())
    }

    /// What `CreateAvatarUpdateBlock` reads now. The stand-in sets no
    /// collision plane and never sits on an object.
    fn full_of(&self, agent: Uuid) -> Option<AvatarFull> {
        let viewer = self.viewers.get(&agent)?;
        let avatar = self.presences.get(&agent)?;
        Some(AvatarFull {
            agent,
            local_id: viewer.local_id,
            look: Arc::clone(&viewer.link.look),
            collision_plane: UNIT_W,
            position: avatar.pose.position,
            velocity: avatar.pose.velocity,
            rotation: block_rotation(avatar.pose.rotation, avatar.body.flying()),
        })
    }

    fn send(&self, to: &Uuid, packet: AvatarPacket) {
        if let Some(viewer) = self.viewers.get(to) {
            viewer.link.feed.send_avatar_packet(packet);
        }
    }

    /// `CompleteMovement`: the avatar's full update to every other viewer
    /// (`SendInitialAvatarDataToAllAgents`), then its appearance to each
    /// (`SendAppearanceToAgentNF`). Its own viewer gets both from its
    /// circuit, with the arrival packet.
    fn announce(&self, agent: Uuid) {
        let Some(full) = self.full_of(agent) else {
            return;
        };
        let others: Vec<Uuid> = self
            .viewer_order()
            .into_iter()
            .map(|(other, _)| other)
            .filter(|other| *other != agent && self.is_open(other))
            .collect();
        for other in &others {
            self.send(other, AvatarPacket::Full(full.clone()));
        }
        for other in &others {
            self.send(
                other,
                AvatarPacket::Appearance {
                    agent,
                    look: Arc::clone(&full.look),
                },
            );
        }
    }

    /// `SendOtherAgentsAvatarFullToMe`: every other root avatar's full update,
    /// then its appearance, to this viewer.
    fn send_initial_data(&self, agent: Uuid) {
        // ScenePresence.cs 4337 logs before the teleport-flag check.
        tracing::info!("SendInitialData for \"{agent}\"");
        // ScenePresence.cs 4338: flags of zero request terrain and the overlay again.
        if let Some(viewer) = self.viewers.get(&agent)
            && viewer.link.teleport_flags == 0
        {
            viewer.link.feed.request_land_pass();
        }
        for (other, _) in self.viewer_order() {
            if other == agent || !self.is_root(&other) {
                continue;
            }
            let Some(full) = self.full_of(other) else {
                continue;
            };
            let look = Arc::clone(&full.look);
            self.send(&agent, AvatarPacket::Full(full));
            self.send(&agent, AvatarPacket::Appearance { agent: other, look });
        }
        if let (Some(viewer), Some(avatar)) = (self.viewers.get(&agent), self.presences.get(&agent))
        {
            viewer.link.feed.offer_stored_parts(&PriorityViewer {
                agent_id: agent,
                position: Some(avatar.pose.position),
                sitting_on: None,
            });
        }
    }

    /// `SendCoarseLocations`: the root avatars in scene order, the same list
    /// to every open viewer, children included (`ForEachScenePresence`). The
    /// position is the frame's, after this tick. C# reads it on a pool
    /// thread a little later and skips a frame while the last send is still
    /// running; this send is done before the next tick.
    fn send_coarse_locations(&mut self) {
        let order = self.viewer_order();
        let roots = order.iter().filter_map(|(agent, _)| {
            let avatar = self.presences.get(agent)?;
            (!avatar.body.is_child()).then_some((*agent, avatar.pose.position))
        });
        let (positions, agents) = coarse::gather(roots, REGION_SIZE);
        for (agent, _) in &order {
            let Some(viewer) = self.viewers.get_mut(agent) else {
                continue;
            };
            if !viewer.link.circuit_open.load(Ordering::Acquire) {
                continue;
            }
            if let Some(packet) = viewer.coarse.packet(*agent, &positions, &agents) {
                viewer
                    .link
                    .feed
                    .send_avatar_packet(AvatarPacket::Coarse(packet));
            }
        }
    }

    /// `RemoveClient`: the leaving root's local id on every remaining viewer.
    fn kill_everywhere(&self, local_id: u32) {
        for (other, _) in self.viewer_order() {
            if self.is_open(&other) {
                self.send(&other, AvatarPacket::Kill(vec![local_id]));
            }
        }
    }

    /// C# has a presence, and a client, only from `UseCircuitCode` on: a
    /// viewer attached at login whose circuit is not open yet is not told.
    fn is_open(&self, agent: &Uuid) -> bool {
        self.viewers
            .get(agent)
            .is_some_and(|viewer| viewer.link.circuit_open.load(Ordering::Acquire))
    }

    /// `HasMovedAway`, then `MakeChildAgent`. Out of view range, the avatar
    /// is killed on every other viewer and every other root on it. Then it is
    /// killed on every root viewer that has no child in the destination; out
    /// of view range that is a second kill.
    fn moved_away(&mut self, agent: Uuid, away: &MovedAway) {
        let Some(local_id) = self.viewers.get(&agent).map(|viewer| viewer.local_id) else {
            return;
        };
        let order = self.viewer_order();
        if !away.near {
            if let Some(viewer) = self.viewers.get_mut(&agent) {
                viewer.initial.made_child();
            }
            for (other, other_id) in &order {
                if *other == agent {
                    continue;
                }
                self.send(other, AvatarPacket::Kill(vec![local_id]));
                if self.is_root(other) {
                    self.send(&agent, AvatarPacket::Kill(vec![*other_id]));
                }
            }
        }
        for (other, _) in &order {
            if *other == agent || !self.is_root(other) || away.known_by.contains(other) {
                continue;
            }
            self.send(other, AvatarPacket::Kill(vec![local_id]));
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
    mod stored_arrival;
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

    fn no_physics() -> Pose {
        Pose {
            ready: false,
            last_count: 0,
            total_steps: 0,
        }
    }

    fn frames(frame: &TerrainFrame) -> u64 {
        frame.frames().unwrap().blocking_wait().unwrap()
    }

    fn substep_counts() -> Vec<i64> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/jolt/substeps.json");
        let text = std::fs::read_to_string(path).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["sequence"]["counts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|count| count.as_i64().unwrap())
            .collect()
    }

    fn wait_for_physics_steps(frame: &TerrainFrame, at_least: i64) -> Pose {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let pose = frame.last_physics_pose().unwrap().blocking_wait().unwrap();
            if pose.total_steps >= at_least {
                return pose;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "frame reached {} steps",
                pose.total_steps
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_frame_steps_follow_the_substep_fixture() {
        let counts = substep_counts();
        let ticks = 12;
        let expected: i64 = counts.iter().take(ticks).sum();
        let frame = TerrainFrame::start_without_map().unwrap();
        let pose = wait_for_physics_steps(&frame, expected);
        let mut total = 0i64;
        let mut matched = None;
        for (index, count) in counts.iter().copied().enumerate() {
            total += count;
            if total == pose.total_steps {
                matched = Some((index, count));
                break;
            }
            if total > pose.total_steps {
                break;
            }
        }
        let (index, count) = matched.expect("total_steps left the fixture sequence");
        assert!(pose.ready);
        assert_eq!(pose.last_count, i32::try_from(count).unwrap());
        assert!(count == 4 || count == 5, "{count}");
        assert!(index + 1 >= ticks, "saw only {} ticks", index + 1);
    }

    #[test]
    fn a_zero_step_rate_takes_one_step_on_each_frame_tick() {
        let config = ferroan_config::Config::parse(
            "[Startup]\nphysics = Jolt\n[Jolt]\nPhysicsStepRate = 0\n",
        )
        .unwrap();
        let frame = TerrainFrame::with_object_settings(
            None,
            None,
            std::sync::Arc::default(),
            None,
            crate::PhysicsScaleLimits::default(),
            true,
            config,
        )
        .unwrap();
        let pose = wait_for_physics_steps(&frame, 4);
        assert!(pose.ready);
        assert_eq!(pose.last_count, 1);
        assert!(pose.total_steps >= 4);
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
            .arrive(agent, pose_at([1.0, 1.0, 1.0]), EAST)
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

    /// A look that turns an arrival to the identity rotation.
    const EAST: [f32; 3] = [1.0, 0.0, 0.0];

    fn look_of(first_name: &str) -> Arc<AvatarLook> {
        Arc::new(AvatarLook {
            first_name: first_name.to_owned(),
            last_name: "Resident".to_owned(),
            title: String::new(),
            visual_params: vec![1, 2, 3],
            texture: vec![4, 5],
            height: 1.9,
            hover: 0.0,
        })
    }

    fn attach(frame: &TerrainFrame, agent: Uuid) -> (u32, Arc<ViewerFeed>, Arc<AtomicBool>) {
        let feed = frame.new_feed();
        let seeds = Arc::new(AtomicBool::new(false));
        let link = ViewerLink {
            feed: Arc::clone(&feed),
            seeds_sent: Arc::clone(&seeds),
            circuit_open: Arc::new(AtomicBool::new(true)),
            look: look_of(&agent.as_u128().to_string()),
            teleport_flags: 1 << 7,
        };
        let local_id = frame.attach(agent, link).unwrap().blocking_wait().unwrap();
        (local_id, feed, seeds)
    }

    /// Wait for a terse update in `feed`, then read the frame counter. The
    /// update was offered in a tick the counter already includes, so the
    /// reading bounds the frames that passed before it from above.
    fn frames_at_first_terse(frame: &TerrainFrame, feed: &ViewerFeed) -> u64 {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !feed.has_updates() {
            assert!(
                std::time::Instant::now() < deadline,
                "no terse update arrived"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        frames(frame)
    }

    fn local_ids(blocks: &[[u8; crate::terse::AVATAR_TERSE_LEN]]) -> Vec<u32> {
        blocks
            .iter()
            .map(|block| u32::from_le_bytes(block[0..4].try_into().unwrap()))
            .collect()
    }

    /// `ScenePresence.Update` sends nothing until `SendInitialData` has run:
    /// the reply, the seeds, then four frames. A due update then goes to
    /// every attached viewer, the walker's own first.
    #[test]
    fn a_terse_update_waits_for_initial_data_and_reaches_every_viewer() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let walker = Uuid::from_u128(46);
        let watcher = Uuid::from_u128(47);
        let (first_id, _, _) = attach(&frame, walker);
        let (watcher_id, watcher_feed, _) = attach(&frame, watcher);
        assert_ne!(first_id, watcher_id);
        // Attaching again keeps the id and replaces the feed.
        let (walker_id, walker_feed, walker_seeds) = attach(&frame, walker);
        assert_eq!(walker_id, first_id);
        login(&frame, walker, pose_at([128.0, 128.0, 25.0]));
        login(&frame, watcher, pose_at([120.0, 128.0, 25.0]));
        reuse(&frame, walker);
        reuse(&frame, watcher);
        // Seeds without a reply: nothing.
        walker_seeds.store(true, Ordering::Release);
        wait_frames(&frame, 8);
        assert!(!walker_feed.has_updates());
        // The frame thread ticks on the wall clock, so frames are counted,
        // not slept: the counter is read before the reply is posted, and the
        // first test runs in the fifth frame after the reply is applied
        // (four frames of `SendInitialData`'s wait, which return early).
        let before = frames(&frame);
        frame.handshake_reply(walker).unwrap();
        let seen = frames_at_first_terse(&frame, &walker_feed);
        assert!(
            seen >= before + 5,
            "the first frames return early ({} frames)",
            seen - before
        );
        // The first test is due against the zero last values.
        assert_eq!(local_ids(&walker_feed.take_terse(0)), [walker_id]);
        assert_eq!(local_ids(&watcher_feed.take_terse(0)), [walker_id]);
        // At rest, nothing more is due. The watcher never replied.
        wait_frames(&frame, 3);
        assert!(!walker_feed.has_updates());
        assert!(!watcher_feed.has_updates());
        // Walking starts: the velocity is due.
        let now = clock_ms(Instant::now());
        frame
            .agent_update(walker, walking_update(now, 2, CONTROL_AT_POS))
            .unwrap();
        wait_frames(&frame, 2);
        let blocks = walker_feed.take_terse(0);
        assert_eq!(local_ids(&blocks), [walker_id]);
        assert_ne!(
            &blocks[0][34..36],
            &[0xff, 0x7f],
            "the block carries a velocity"
        );
        assert!(watcher_feed.has_updates());
        // A child is not tested, and the next reply waits again.
        frame.set_child(walker, true).unwrap();
        wait_frames(&frame, 2);
        let _ = walker_feed.take_terse(0);
        let _ = watcher_feed.take_terse(0);
        wait_frames(&frame, 6);
        assert!(!walker_feed.has_updates());
        // Root again: the wait starts over at the next reply, so a due walk
        // is not sent for the frames `SendInitialData` takes. The reply is
        // posted before the arrival, which applies it first: had a tick run
        // between the arrival and the reply, the stop after the walk would
        // have been tested with the wait still over.
        let before = frames(&frame);
        frame.handshake_reply(walker).unwrap();
        reuse(&frame, walker);
        // A turn is due on every frame until it is sent.
        let mut turn = walking_update(clock_ms(Instant::now()), 3, 0);
        turn.body_rotation = [
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ];
        frame.agent_update(walker, turn).unwrap();
        let seen = frames_at_first_terse(&frame, &walker_feed);
        assert!(
            seen >= before + 5,
            "the re-armed wait returns early ({} frames)",
            seen - before
        );
        assert_eq!(local_ids(&walker_feed.take_terse(0)), [walker_id]);
        // Leaving removes the viewer and its board entry.
        frame.remove(walker).unwrap().blocking_wait().unwrap();
        watcher_feed.offer_terse(1, walker_id);
        assert!(watcher_feed.take_terse(0).is_empty());
    }

    /// `BestAvatarResponsiveness` puts a viewer's own avatar on queue 0 and
    /// the other on queue 1, so its own update leaves first even when the
    /// other avatar's was offered earlier.
    #[test]
    fn each_viewer_takes_its_own_avatar_first() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let left = Uuid::from_u128(50);
        let right = Uuid::from_u128(51);
        let (left_id, left_feed, left_seeds) = attach(&frame, left);
        let (right_id, right_feed, right_seeds) = attach(&frame, right);
        for (agent, seeds) in [(left, &left_seeds), (right, &right_seeds)] {
            login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
            reuse(&frame, agent);
            seeds.store(true, Ordering::Release);
            frame.handshake_reply(agent).unwrap();
        }
        wait_frames(&frame, 7);
        let _ = left_feed.take_terse(0);
        let _ = right_feed.take_terse(0);
        let turn = |agent: Uuid, sequence: u32, z: f32, w: f32| {
            let mut update = walking_update(clock_ms(Instant::now()), sequence, 0);
            update.body_rotation = [0.0, 0.0, z, w];
            frame.agent_update(agent, update).unwrap();
        };
        // The right avatar turns a frame before the left one.
        turn(
            right,
            2,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        wait_frames(&frame, 1);
        turn(
            left,
            2,
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        wait_frames(&frame, 2);
        assert_eq!(local_ids(&left_feed.take_terse(0)), [left_id, right_id]);
        assert_eq!(local_ids(&right_feed.take_terse(0)), [right_id, left_id]);
        // And the other way round.
        turn(left, 3, 0.0, 1.0);
        wait_frames(&frame, 1);
        turn(right, 3, 0.0, 1.0);
        wait_frames(&frame, 2);
        assert_eq!(local_ids(&left_feed.take_terse(0)), [left_id, right_id]);
        assert_eq!(local_ids(&right_feed.take_terse(0)), [right_id, left_id]);
    }

    /// A login is a child agent in C# until `CompleteMovement`: the wait
    /// runs, but nothing is sent before the arrival samples the pose.
    #[test]
    fn nothing_is_sent_before_the_arrival() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(49);
        let (local_id, feed, seeds) = attach(&frame, agent);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        seeds.store(true, Ordering::Release);
        frame.handshake_reply(agent).unwrap();
        wait_frames(&frame, 8);
        assert!(!feed.has_updates());
        reuse(&frame, agent);
        wait_frames(&frame, 2);
        assert_eq!(local_ids(&feed.take_terse(0)), [local_id]);
    }

    /// The agent ids of the full updates and appearances, and the kills, in
    /// the order a feed holds them.
    fn packet_kinds(feed: &ViewerFeed) -> Vec<(char, u128)> {
        feed.take_avatar_packets(usize::MAX)
            .into_iter()
            .filter_map(|packet| match packet {
                AvatarPacket::Full(full) => Some(('F', full.agent.as_u128())),
                AvatarPacket::Appearance { agent, .. } => Some(('A', agent.as_u128())),
                AvatarPacket::Kill(ids) => Some(('K', u128::from(ids[0]))),
                // The frame thread's clock decides when a minimap list falls.
                AvatarPacket::Coarse(_) => None,
            })
            .collect()
    }

    /// `CompleteMovement` sends the newcomer's full update, then its
    /// appearance, to the viewer already there. The newcomer learns of the
    /// other root at its `SendInitialData`: the reply, the seeds, four frames.
    #[test]
    fn an_arrival_is_announced_and_the_newcomer_hears_of_the_others() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let first = Uuid::from_u128(60);
        let second = Uuid::from_u128(61);
        let (_, first_feed, _) = attach(&frame, first);
        login(&frame, first, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, first);
        assert!(
            packet_kinds(&first_feed).is_empty(),
            "its own update is the circuit's"
        );
        let (_, second_feed, second_seeds) = attach(&frame, second);
        login(&frame, second, pose_at([120.0, 128.0, 25.0]));
        // The newcomer turns to the arrival look before it is announced.
        let look = [0.99, 0.042, 0.0];
        let arrived = frame
            .arrive(second, pose_at([1.0, 1.0, 1.0]), look)
            .unwrap()
            .blocking_wait()
            .unwrap();
        assert_eq!(arrived.rotation, look_rotation(look));
        let announced = first_feed.take_avatar_packets(usize::MAX);
        assert_eq!(announced.len(), 2);
        let AvatarPacket::Full(full) = &announced[0] else {
            panic!("the full update comes first");
        };
        assert_eq!(full.agent, second);
        assert_eq!(full.position, [120.0, 128.0, 25.0]);
        assert_eq!(full.velocity, [0.0; 3]);
        assert_eq!(full.collision_plane, UNIT_W);
        assert_eq!(full.rotation, look_rotation(look));
        assert_eq!(full.look.first_name, "61");
        assert!(matches!(
            &announced[1],
            AvatarPacket::Appearance { agent, .. } if *agent == second
        ));
        assert!(packet_kinds(&second_feed).is_empty());
        // The wait: the reply and the seeds, then four frames. The frame
        // thread ticks on the wall clock, so the test does not count frames
        // by sleeping. The counter is read before the reply is posted, so
        // every frame of the wait comes after that reading; the packets are
        // queued in a tick that is already counted when the counter is read
        // after them.
        second_seeds.store(true, Ordering::Release);
        let before = frames(&frame);
        frame.handshake_reply(second).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let sent = loop {
            let kinds = packet_kinds(&second_feed);
            if !kinds.is_empty() {
                break kinds;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "SendInitialData never ran"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        let seen = frames(&frame);
        assert_eq!(sent, [('F', 60), ('A', 60)]);
        assert!(
            seen >= before + 4,
            "SendInitialData ran {} frames after the reply, before the fourth",
            seen - before
        );
        // Once only.
        wait_frames(&frame, 3);
        assert!(packet_kinds(&second_feed).is_empty());
        assert!(packet_kinds(&first_feed).is_empty());
    }

    /// `SendOtherAgentsAvatarFullToMe` names root agents only.
    #[test]
    fn initial_data_skips_a_child_and_an_avatar_not_yet_arrived() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let child = Uuid::from_u128(62);
        let waiting = Uuid::from_u128(63);
        let newcomer = Uuid::from_u128(64);
        attach(&frame, child);
        login(&frame, child, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, child);
        frame.set_child(child, true).unwrap();
        attach(&frame, waiting);
        login(&frame, waiting, pose_at([128.0, 128.0, 25.0]));
        let (_, feed, seeds) = attach(&frame, newcomer);
        login(&frame, newcomer, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, newcomer);
        seeds.store(true, Ordering::Release);
        frame.handshake_reply(newcomer).unwrap();
        wait_frames(&frame, 7);
        assert!(packet_kinds(&feed).is_empty());
    }

    /// `RemoveClient` kills a root on every remaining viewer and drops the
    /// terse update still waiting for it there. A child leaves no kill.
    #[test]
    fn a_root_that_leaves_is_killed_on_the_others_and_a_child_is_not() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let staying = Uuid::from_u128(65);
        let leaving = Uuid::from_u128(66);
        let child = Uuid::from_u128(67);
        let (_, feed, _) = attach(&frame, staying);
        let (leaving_id, _, _) = attach(&frame, leaving);
        let (child_id, _, _) = attach(&frame, child);
        for agent in [staying, leaving, child] {
            login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
            reuse(&frame, agent);
        }
        frame.set_child(child, true).unwrap();
        let _ = feed.take_avatar_packets(usize::MAX);
        feed.offer_terse(1, leaving_id);
        frame.remove(leaving).unwrap().blocking_wait().unwrap();
        assert_eq!(packet_kinds(&feed), [('K', u128::from(leaving_id))]);
        assert!(
            !feed.has_updates(),
            "the kill drops the waiting terse update"
        );
        frame.remove(child).unwrap().blocking_wait().unwrap();
        assert!(
            packet_kinds(&feed).is_empty(),
            "no kill for child {child_id}"
        );
    }

    /// `MakeChildAgent` kills the avatar on root viewers with no child in the
    /// destination. Out of view range, `HasMovedAway(false)` first kills it
    /// on every other viewer and every other root on its own viewer.
    #[test]
    fn a_hop_away_kills_where_the_destination_is_unknown() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let hopper = Uuid::from_u128(68);
        let knows = Uuid::from_u128(69);
        let unaware = Uuid::from_u128(70);
        let (hopper_id, hopper_feed, _) = attach(&frame, hopper);
        let (knows_id, knows_feed, _) = attach(&frame, knows);
        let (unaware_id, unaware_feed, _) = attach(&frame, unaware);
        for agent in [hopper, knows, unaware] {
            login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
            reuse(&frame, agent);
        }
        for feed in [&hopper_feed, &knows_feed, &unaware_feed] {
            let _ = feed.take_avatar_packets(usize::MAX);
        }
        frame.set_child(hopper, true).unwrap();
        let near = MovedAway {
            near: true,
            known_by: vec![knows],
        };
        frame
            .moved_away(hopper, near)
            .unwrap()
            .blocking_wait()
            .unwrap();
        assert!(packet_kinds(&knows_feed).is_empty());
        assert_eq!(packet_kinds(&unaware_feed), [('K', u128::from(hopper_id))]);
        assert!(packet_kinds(&hopper_feed).is_empty());
        let far = MovedAway {
            near: false,
            known_by: vec![knows],
        };
        frame
            .moved_away(hopper, far)
            .unwrap()
            .blocking_wait()
            .unwrap();
        assert_eq!(packet_kinds(&knows_feed), [('K', u128::from(hopper_id))]);
        assert_eq!(
            packet_kinds(&unaware_feed),
            [('K', u128::from(hopper_id)), ('K', u128::from(hopper_id))]
        );
        assert_eq!(
            packet_kinds(&hopper_feed),
            [('K', u128::from(knows_id)), ('K', u128::from(unaware_id))]
        );
    }

    /// GridUser's logout reads the walked position and `Lookat`, the camera
    /// at axis with Z dropped and normalised.
    #[test]
    fn leaving_reports_the_position_and_the_flattened_look() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(48);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, agent);
        let mut update = walking_update(clock_ms(Instant::now()), 2, 0);
        // Not a unit axis: C# stores it as sent (TB86).
        update.camera_at = [0.3, 0.7, 0.2];
        frame.agent_update(agent, update).unwrap();
        wait_frames(&frame, 2);
        let stored = read(&frame, agent);
        let leaving = frame
            .remove(agent)
            .unwrap()
            .blocking_wait()
            .unwrap()
            .unwrap();
        assert_eq!(leaving.position, stored.position);
        assert_eq!(leaving.look_at, look_at([0.3, 0.7, 0.2]));
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

    /// A scene driven by hand, without the frame thread, so the frame count
    /// is exact.
    fn bare_scene() -> Scene {
        Scene {
            presences: HashMap::new(),
            viewers: HashMap::new(),
            board: AvatarBoard::default(),
            local_ids: Arc::default(),
        }
    }

    fn open_viewer(
        scene: &mut Scene,
        agent: Uuid,
        open: bool,
    ) -> (Arc<ViewerFeed>, Arc<AtomicBool>) {
        // ViaLogin. SendInitialData does not request a second overlay.
        open_viewer_flags(scene, agent, open, 1 << 7)
    }

    fn open_viewer_flags(
        scene: &mut Scene,
        agent: Uuid,
        open: bool,
        teleport_flags: u32,
    ) -> (Arc<ViewerFeed>, Arc<AtomicBool>) {
        let feed = Arc::new(ViewerFeed::new(scene.board.clone()));
        let circuit_open = Arc::new(AtomicBool::new(open));
        let link = ViewerLink {
            feed: Arc::clone(&feed),
            seeds_sent: Arc::new(AtomicBool::new(false)),
            circuit_open: Arc::clone(&circuit_open),
            look: look_of(&agent.as_u128().to_string()),
            teleport_flags,
        };
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Attach { agent, link, reply },
            scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        (feed, circuit_open)
    }

    /// Handshake, seeds, then four frames. The fourth runs `SendInitialData`.
    /// Zero teleport flags request another land pass. ViaLogin does not.
    /// A second reply after that frame does not request another pass.
    #[test]
    fn send_initial_data_requests_land_only_when_teleport_flags_are_zero() {
        for flags in [0, 1 << 7] {
            let mut scene = bare_scene();
            let agent = Uuid::from_u128(1);
            let (feed, _) = open_viewer_flags(&mut scene, agent, true, flags);
            scene
                .viewers
                .get(&agent)
                .unwrap()
                .link
                .seeds_sent
                .store(true, Ordering::Release);
            apply(Inbound::HandshakeReply { agent }, &mut scene);
            for _ in 0..3 {
                update_presences(&mut scene);
                assert!(!feed.take_land_pass());
            }
            update_presences(&mut scene);
            assert_eq!(feed.take_land_pass(), flags == 0);
            // A second reply must not re-arm the gate. One frame cannot
            // reach SendInitialData; four frames can.
            apply(Inbound::HandshakeReply { agent }, &mut scene);
            for _ in 0..4 {
                update_presences(&mut scene);
                assert!(!feed.take_land_pass());
            }
        }
    }

    fn arrive_in(scene: &mut Scene, agent: Uuid, position: [f32; 3]) {
        let (reply, _answer) = oneshot::channel();
        let request = Request::Arrive {
            agent,
            pose: pose_at(position),
            look_at: EAST,
            reply,
        };
        assert!(answer(request, scene, None, &[], 0, no_physics()));
    }

    /// The minimap packets a feed holds, dropping the rest.
    fn dots(feed: &ViewerFeed) -> Vec<crate::coarse::CoarseLocations> {
        feed.take_avatar_packets(usize::MAX)
            .into_iter()
            .filter_map(|packet| match packet {
                AvatarPacket::Coarse(coarse) => Some(coarse),
                _ => None,
            })
            .collect()
    }

    fn cell(scene: &Scene, agent: Uuid) -> [u8; 3] {
        crate::coarse::location(scene.presences[&agent].pose.position)
    }

    /// C# has no presence for a viewer before its `UseCircuitCode`: an
    /// arrival and a departure meanwhile tell it nothing, and its own
    /// `SendInitialData` then sends the other root once.
    #[test]
    fn a_viewer_not_yet_open_hears_of_the_others_once_at_its_initial_data() {
        let mut scene = bare_scene();
        let there = Uuid::from_u128(90);
        let leaving = Uuid::from_u128(91);
        let coming = Uuid::from_u128(92);
        let _ = open_viewer(&mut scene, there, true);
        let _ = open_viewer(&mut scene, leaving, true);
        let (feed, open) = open_viewer(&mut scene, coming, false);
        arrive_in(&mut scene, there, [128.0, 128.0, 25.0]);
        arrive_in(&mut scene, leaving, [129.0, 128.0, 25.0]);
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Remove {
                agent: leaving,
                reply
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        assert!(packet_kinds(&feed).is_empty());
        open.store(true, Ordering::Release);
        arrive_in(&mut scene, coming, [130.0, 128.0, 25.0]);
        scene.send_initial_data(coming);
        assert_eq!(
            packet_kinds(&feed),
            [('F', there.as_u128()), ('A', there.as_u128())]
        );
    }

    const EVERY: u64 = COARSE_EVERY_FRAMES;

    /// `Scene.Update` counts the frame, then sends on `Frame % 50 == 0`.
    #[test]
    fn minimap_dots_go_out_on_every_fiftieth_frame_only() {
        let mut scene = bare_scene();
        let agent = Uuid::from_u128(80);
        let (feed, _) = open_viewer(&mut scene, agent, true);
        arrive_in(&mut scene, agent, [128.0, 128.0, 25.0]);
        let _ = feed.take_avatar_packets(usize::MAX);
        let mut frames = 0;
        let mut sent_on = Vec::new();
        for _ in 0..151 {
            frames = tick(&mut scene, None, frames);
            if !dots(&feed).is_empty() {
                sent_on.push(frames);
            }
        }
        assert_eq!(sent_on, [50, 100, 150]);
    }

    /// One list of roots in attach order goes to every open viewer, a child
    /// included; each finds itself through `You`, and `Prey` is -1.
    #[test]
    fn each_viewer_gets_the_roots_and_its_own_index() {
        let mut scene = bare_scene();
        let first = Uuid::from_u128(81);
        let second = Uuid::from_u128(82);
        let child = Uuid::from_u128(83);
        let waiting = Uuid::from_u128(84);
        let (first_feed, _) = open_viewer(&mut scene, first, true);
        let (second_feed, _) = open_viewer(&mut scene, second, true);
        let (child_feed, _) = open_viewer(&mut scene, child, true);
        let (waiting_feed, _) = open_viewer(&mut scene, waiting, true);
        arrive_in(&mut scene, first, [128.0, 128.0, 25.0]);
        arrive_in(&mut scene, second, [131.6, 40.2, 27.96]);
        arrive_in(&mut scene, child, [10.0, 10.0, 25.0]);
        apply(
            Inbound::Child {
                agent: child,
                child: true,
            },
            &mut scene,
        );
        let frames = (0..EVERY).fold(0, |frames, _| tick(&mut scene, None, frames));
        assert_eq!(frames, EVERY);
        let expected = vec![cell(&scene, first), cell(&scene, second)];
        assert_eq!(expected[1][..2], [131, 40]);
        for (feed, you) in [
            (&first_feed, 0),
            (&second_feed, 1),
            (&child_feed, -1),
            (&waiting_feed, -1),
        ] {
            let got = dots(feed);
            assert_eq!(got.len(), 1);
            assert_eq!(got[0].agents, [first, second]);
            assert_eq!(got[0].locations, expected);
            assert_eq!((got[0].you, got[0].prey), (you, -1));
        }
    }

    /// C# has no client before `UseCircuitCode`: a viewer whose circuit is
    /// not open gets no dots, and its first list comes after it opens.
    #[test]
    fn a_viewer_gets_dots_only_once_its_circuit_is_open() {
        let mut scene = bare_scene();
        let there = Uuid::from_u128(85);
        let coming = Uuid::from_u128(86);
        let _ = open_viewer(&mut scene, there, true);
        arrive_in(&mut scene, there, [128.0, 128.0, 25.0]);
        let (feed, open) = open_viewer(&mut scene, coming, false);
        let mut frames = (0..2 * EVERY).fold(0, |frames, _| tick(&mut scene, None, frames));
        assert!(dots(&feed).is_empty());
        open.store(true, Ordering::Release);
        frames = (0..EVERY).fold(frames, |frames, _| tick(&mut scene, None, frames));
        assert_eq!(frames, 3 * EVERY);
        let got = dots(&feed);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].agents, [there]);
        assert_eq!(got[0].you, -1, "not root until it arrives");
    }

    /// An empty list goes once; a root that leaves drops out of the next one.
    #[test]
    fn an_empty_list_goes_once_and_a_leaver_drops_out() {
        let mut scene = bare_scene();
        let staying = Uuid::from_u128(87);
        let leaving = Uuid::from_u128(88);
        let (feed, _) = open_viewer(&mut scene, staying, true);
        let mut frames = 0;
        let mut lists = Vec::new();
        for _ in 0..3 * EVERY {
            frames = tick(&mut scene, None, frames);
            lists.extend(dots(&feed));
        }
        assert_eq!(lists.len(), 1, "the empty list goes once");
        assert!(lists[0].agents.is_empty() && lists[0].locations.is_empty());
        assert_eq!(lists[0].you, -1);
        let _ = open_viewer(&mut scene, leaving, true);
        arrive_in(&mut scene, staying, [128.0, 128.0, 25.0]);
        arrive_in(&mut scene, leaving, [129.0, 128.0, 25.0]);
        frames = (0..EVERY).fold(frames, |frames, _| tick(&mut scene, None, frames));
        assert_eq!(dots(&feed)[0].agents, [staying, leaving]);
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Remove {
                agent: leaving,
                reply
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        let _ = (0..EVERY).fold(frames, |frames, _| tick(&mut scene, None, frames));
        let got = dots(&feed);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].agents, [staying]);
        assert_eq!(got[0].you, 0);
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
        assert!(
            frame
                .remove(agent)
                .unwrap()
                .blocking_wait()
                .unwrap()
                .is_some()
        );
        assert!(
            frame
                .remove(agent)
                .unwrap()
                .blocking_wait()
                .unwrap()
                .is_none()
        );
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
            .arrive(agent, pose_at([20.0, 30.0, 40.0]), EAST)
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
        frame.set_always_run(agent, true).unwrap();
        let departure = frame
            .departure(agent)
            .unwrap()
            .blocking_wait()
            .unwrap()
            .unwrap();
        assert!(departure.always_run);
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
        // The actor flies once the arrival packet has sampled the pose.
        assert!(!flying(&frame, agent));
        assert_eq!(reuse(&frame, agent).position, [20.0, 30.0, 40.0]);
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

    /// QR29: the circuit reports the completion's position to GridUser and
    /// Presence, then samples it for `AgentMovementComplete`. Ticks in between
    /// must not move it, or the return lands at the snapped height.
    #[test]
    fn a_hop_completion_holds_the_landing_until_the_arrival_samples_it() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(29);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, agent);
        frame.set_child(agent, true).unwrap();
        let landing = [128.0, 128.0, 30.0];
        assert_eq!(
            complete(&frame, agent, standing_return(landing)).position,
            landing
        );
        wait_frames(&frame, 3);
        assert_eq!(read(&frame, agent).position, landing);
        // A walk posted before the sample is dropped by the arrival.
        frame
            .agent_update(agent, walking_update(3_000, 3, CONTROL_AT_POS))
            .unwrap();
        wait_frames(&frame, 2);
        assert_eq!(read(&frame, agent).position, landing);
        let sampled = reuse(&frame, agent);
        assert_eq!(sampled.position, landing);
        assert_eq!(sampled.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(motion(&frame, agent).target_velocity, [0.0, 0.0, 0.0]);
        // After the sample the actor is stepped again.
        wait_frames(&frame, 2);
        let stepped = read(&frame, agent).position;
        assert_eq!([stepped[0], stepped[1]], [128.0, 128.0]);
        assert_eq!(stepped[2], AVATAR_SIZE_Z);
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
