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
//! Each frame ends with `RegionHeartbeatEnd`: every root's animator updates
//! on every second one ([`crate::animation`]). A departure carries the live
//! animator for `CopyTo`, a hop completion applies the sender's (`CopyFrom`),
//! and a hop away that arrived resets it (`MakeChildAgent`).
//!
//! The circuit queues one `ParcelProperties` element after
//! `AgentMovementComplete`. A zero-flag initial-data gate and a walk onto
//! another parcel each queue one as well. The box was stored when the land
//! was loaded. The frame and circuit share avatar local IDs as in
//! `Source/OpenSim.Region.Framework/Scenes/{SceneBase,ScenePresence}.cs`
//! at Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`.
//!
//! Child neighbours are decided here and carried out elsewhere. Arrival and
//! the child half of `CheckForSignificantMovement` only `try_send` a
//! [`ChildNotice`]. The 16 m significant-movement trigger is not this path.
//! HTTP stays off this thread.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ferroan_config::Config;
use ferroan_diag::{Kind, REGION_CHILD_NOTICES, REGION_SCENE_INBOUND, refuse};
use ferroan_physics::{Physics, Pose};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::animation::{Anim, AnimationTransfer};
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

/// Queues one `ParcelProperties` element for an agent. The region passes
/// `parcel-properties`, which is the name a limit readout shows.
pub type ParcelSender = Arc<dyn Fn(Uuid, &[u8]) + Send + Sync>;

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
#[derive(Clone, Debug)]
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

/// Waiting child notices. Pin8's queue is unbounded. A full queue rolls a
/// movement pass back and defers an arrival.
pub const CHILD_NOTICE_QUEUE: usize = 64;

/// Per-avatar pose owned by the frame thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Presence {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub control_flags: u32,
    pub draw_distance: f32,
}

/// A presence as admission and tests read it. `child` is the child-agent flag.
/// `actor` is the physics actor. A view child has the flag and no actor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seat {
    pub pose: Presence,
    pub child: bool,
    pub actor: bool,
}

/// `ScenePresence.CHILDUPDATES_TIME` at Pin8 `c401bceecd` (line 194).
const CHILD_UPDATES_MS: i64 = 2_000;
/// Arrival stores `EnvironmentTickCount() + 10000` (lines 2577 and 4400).
const CHILD_ARRIVAL_STAMP_MS: i64 = 10_000;
/// `Util.EnvironmentTickCountMask + 1` (`Util.cs` 3610 and 3623). A negative
/// tick difference is this wrap, not a hold.
const CHILD_TICK_MODULUS: i64 = 0x4000_0000;
/// `CHILDAGENTSCHECK_MOVEMENT`, the square of 32 m (line 193).
const CHILD_CHECK_DISTANCE_SQ: f32 = 1024.0;
/// `CHILDUPDATES_MOVEMENT`, the square of 10 m (line 192).
const CHILD_UPDATE_DISTANCE_SQ: f32 = 100.0;
/// View and draw changes use a strict 32 m threshold (lines 4757 and 4790).
const CHILD_RANGE_CHANGE: f32 = 32.0;
/// `AvatarAppearance` height when the viewer has not stored one.
const CHILD_DEFAULT_HEIGHT: f32 = 1.9;
/// Shipped `MinRegionViewDistance` / `MaxRegionViewDistance` until the
/// region installs its own limits.
const CHILD_VIEW_MIN: f32 = 96.0;
const CHILD_VIEW_MAX: f32 = 255.0;

/// Which neighbour pass the region task should run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildPass {
    /// `EnableChildAgents`: diff the visible set.
    Enable,
    /// `SendOutChildAgentUpdates`: `AgentPosition` for children already open.
    Position,
}

/// One child pass. The scene thread builds it and does not wait for HTTP.
#[derive(Clone, Debug)]
pub struct ChildNotice {
    pub agent: Uuid,
    pub pass: ChildPass,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub camera: [f32; 3],
    pub draw_distance: f32,
    pub height: f32,
    pub god: bool,
    /// When set, the region answers with [`TerrainFrame::settle_child`].
    /// An arrival sample does not settle, so it cannot clear a later pass.
    pub settle: bool,
    /// The pass this notice belongs to. A settle for another pass is ignored,
    /// which keeps the arrival stamp if an older pass finishes afterwards.
    pub epoch: u64,
}

/// What the child half of `CheckForSignificantMovement` would do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildDue {
    None,
    Enable,
    Position,
}

/// Positions and ranges a settle can restore when the pass did not commit.
#[derive(Clone, Copy, Debug)]
struct ChildSnap {
    update_position: [f32; 3],
    check_position: [f32; 3],
    draw: f32,
    view: f32,
    god: bool,
}

/// The sample `due` compares with the gate. God is false on the frame;
/// the comparison still sees a change when a test supplies one.
struct ChildSample {
    now_ms: i64,
    child: bool,
    position: [f32; 3],
    draw: f32,
    view: f32,
    god: bool,
}

#[derive(Debug)]
struct ChildGate {
    last_ms: i64,
    update_position: [f32; 3],
    check_position: [f32; 3],
    last_draw: f32,
    last_view: f32,
    last_god: bool,
    busy: bool,
    armed: bool,
    known_other: bool,
    arrival_pending: bool,
    epoch: u64,
    inflight: u64,
    saved: Option<ChildSnap>,
}

impl Default for ChildGate {
    fn default() -> Self {
        Self {
            last_ms: 0,
            update_position: [0.0, 0.0, 0.0],
            check_position: [0.0, 0.0, 0.0],
            last_draw: 0.0,
            last_view: 0.0,
            last_god: false,
            busy: false,
            armed: false,
            known_other: false,
            arrival_pending: false,
            epoch: 0,
            inflight: 0,
            saved: None,
        }
    }
}

/// The region task's sender and the view-distance clamp, shared with the
/// scene thread. The scene thread only `try_lock`s the sender.
struct ChildWatch {
    sender: std::sync::Mutex<Option<tokio::sync::mpsc::Sender<ChildNotice>>>,
    view_min: AtomicU32,
    view_max: AtomicU32,
}

impl ChildWatch {
    fn new() -> Self {
        Self {
            sender: std::sync::Mutex::new(None),
            view_min: AtomicU32::new(CHILD_VIEW_MIN.to_bits()),
            view_max: AtomicU32::new(CHILD_VIEW_MAX.to_bits()),
        }
    }

    fn limits(&self) -> (f32, f32) {
        (
            f32::from_bits(self.view_min.load(Ordering::Relaxed)),
            f32::from_bits(self.view_max.load(Ordering::Relaxed)),
        )
    }
}

/// Where the departing avatar is, for a source hop: `ScenePresence.Flying`
/// and the camera center and rotations `CopyTo` sends (the body rotation
/// after `IsUsableRotation`, the camera from updates the filter kept).
#[derive(Clone, Debug, PartialEq)]
pub struct Departure {
    pub always_run: bool,
    pub flying: bool,
    /// `CopyTo`'s animations, default, implicit default and motion state.
    pub animations: AnimationTransfer,
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
    /// The serve ended. `owner` removes only that generation's presence.
    /// `None` removes whatever is seated.
    Remove {
        agent: Uuid,
        owner: Option<Uuid>,
        reply: oneshot::Sender<Option<Leaving>>,
    },
    /// A viewer's feed. The answer is the avatar's local id.
    Attach {
        agent: Uuid,
        link: ViewerLink,
        reply: oneshot::Sender<u32>,
    },
    /// Seat a login pose for chat. The actor waits for the arrival.
    /// An existing child agent keeps its pose. `owner` is the circuit generation.
    Pose {
        agent: Uuid,
        pose: Presence,
        owner: Uuid,
        reply: oneshot::Sender<Presence>,
    },
    /// Seat a neighbour view child at the posted start. No physics actor and
    /// no `made_child`: the first `SendInitialData` still runs.
    ViewChild {
        agent: Uuid,
        pose: Presence,
        owner: Uuid,
        reply: oneshot::Sender<Presence>,
    },
    /// `MakeChildAgent`: child, no actor, and `made_child` when a viewer is
    /// attached. The position stays where it is.
    MakeChild {
        agent: Uuid,
        reply: oneshot::Sender<()>,
    },
    /// The stored pose and the child and actor flags.
    Seat {
        agent: Uuid,
        reply: oneshot::Sender<Option<Seat>>,
    },
    /// An arrival: the record with an actor at rest, or this pose when
    /// there is no record yet, turned to the look the arrival packet sends.
    Arrive {
        agent: Uuid,
        pose: Presence,
        look_at: [f32; 3],
        reply: oneshot::Sender<Presence>,
    },
    /// The parcel element after `AgentMovementComplete`.
    InitialLand {
        agent: Uuid,
        reply: oneshot::Sender<()>,
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
    /// The region finished one settle-bearing child pass.
    ChildSettled {
        agent: Uuid,
        epoch: u64,
        committed: bool,
    },
    /// The last enable saw at least one neighbour other than this region.
    ChildKnown {
        agent: Uuid,
        others: bool,
    },
    /// Install the sink that queues `ParcelProperties`.
    ParcelOut {
        send: ParcelSender,
        reply: oneshot::Sender<()>,
    },
    /// `RegionSettings.ObjectBonus`, `RegionInfo.ObjectCapacity`, the region
    /// flags and `EstateSettings.TaxFree` for every later element.
    /// `[Startup] enableflyslow`, checked by the region's config.
    FlySlow {
        fly_slow: bool,
        reply: oneshot::Sender<()>,
    },
    ParcelRegion {
        region: crate::parcels::ParcelRegion,
        reply: oneshot::Sender<()>,
    },
    /// `ParcelPropertiesRequest`. The answer follows the queued elements.
    ParcelProperties {
        agent: Uuid,
        query: crate::parcels::PropertiesRequest,
        reply: oneshot::Sender<()>,
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
    /// `AgentAnimation`: each block's id and `StartAnim`, in packet order.
    Animations {
        agent: Uuid,
        changes: Vec<(Uuid, bool)>,
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
    gate: ChildGate,
    /// `ScenePresence.currentParcelUUID`. Absent until a parcel is sent.
    parcel: Option<Uuid>,
    /// `ScenePresence.posLastMove`. Starts at the pose this avatar was built with.
    last_move: [f32; 3],
    /// Circuit generation that seated this presence. A later generation's
    /// remove leaves it in place.
    owner: Uuid,
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
    watch: Arc<ChildWatch>,
    parcels: Option<Arc<crate::parcels::ParcelMap>>,
    parcel_out: Option<ParcelSender>,
    parcel_region: crate::parcels::ParcelRegion,
    /// Groups admitted at startup. The prim tally reads this set.
    groups: Arc<Vec<ObjectGroup>>,
    /// Prim counts for the admitted groups. Filled on the first element.
    /// The set does not change after startup.
    admitted_counts: Option<HashMap<Uuid, crate::parcels::PrimCounts>>,
    /// `LandObject.m_lastSeqId` for each local id, shared by every client.
    parcel_seq: HashMap<i32, i32>,
    /// `Scene.m_useFlySlow`, `[Startup] enableflyslow`.
    fly_slow: bool,
}

/// Owns the heightmap and the login presences on `ferroan-scene`.
pub struct TerrainFrame {
    tx: Sender<Request>,
    inbound: SyncSender<Inbound>,
    /// Set by the first post after the frame last drained. Later posts do not
    /// send another wake until the frame clears it.
    wake: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    parcels: Option<Arc<crate::parcels::ParcelMap>>,
    has_map: bool,
    objects: Arc<StoredScene>,
    local_ids: Arc<LocalIds>,
    board: AvatarBoard,
    watch: Arc<ChildWatch>,
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
        mut parcels: Option<crate::parcels::ParcelMap>,
        groups: Arc<Vec<ObjectGroup>>,
        limits: Option<PrimScaleLimits>,
        physics: PhysicsScaleLimits,
        collidable_prims: bool,
        config: Config,
    ) -> Result<Self, TerrainError> {
        // `ForceUpdateLandInfo` stores the bounding box before the first encode.
        if let Some(parcels) = parcels.as_mut() {
            parcels.update_geometry(map.as_ref());
        }
        let parcels = parcels.map(Arc::new);
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
        let watch = Arc::new(ChildWatch::new());
        let scene = Scene {
            presences: HashMap::new(),
            viewers: HashMap::new(),
            board: board.clone(),
            local_ids: Arc::clone(&local_ids),
            watch: Arc::clone(&watch),
            parcels: parcels.clone(),
            parcel_out: None,
            parcel_region: crate::parcels::ParcelRegion::default(),
            groups: Arc::clone(&objects.groups),
            admitted_counts: None,
            parcel_seq: HashMap::new(),
            fly_slow: false,
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
            watch,
        })
    }

    /// Where child notices go. The scene thread never waits on this sender.
    pub fn watch_children(&self, tx: tokio::sync::mpsc::Sender<ChildNotice>) {
        let mut slot = self
            .watch
            .sender
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *slot = Some(tx);
    }

    /// The region's `MinRegionViewDistance` and `MaxRegionViewDistance`.
    pub fn set_view_limits(&self, min: f32, max: f32) {
        self.watch.view_min.store(min.to_bits(), Ordering::Relaxed);
        self.watch.view_max.store(max.to_bits(), Ordering::Relaxed);
    }

    /// The child pass finished. A pass that did not commit restores the
    /// ranges it had snapshotted. An arrival sample does not call this.
    pub fn settle_child(
        &self,
        agent: Uuid,
        epoch: u64,
        committed: bool,
    ) -> Result<(), TerrainError> {
        self.request(Request::ChildSettled {
            agent,
            epoch,
            committed,
        })
    }

    /// `KnownRegionHandles` had a neighbour other than this region.
    pub fn note_child_neighbours(&self, agent: Uuid, others: bool) -> Result<(), TerrainError> {
        self.request(Request::ChildKnown { agent, others })
    }

    pub fn parcels(&self) -> Option<&crate::parcels::ParcelMap> {
        self.parcels.as_deref()
    }

    /// Install the sink `SendLandUpdateToClient` writes through. The region
    /// points it at one agent's event queue before logins are enabled. A
    /// missing sink still records which parcel the avatar is on.
    pub fn set_parcel_sender(&self, send: ParcelSender) -> Result<Pending<()>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::ParcelOut { send, reply })?;
        Ok(answer)
    }

    /// `Scene.m_useFlySlow`: `startupConfig.GetBoolean("enableflyslow", false)`
    /// (`Scene.cs` line 1002), as `RegionConfig` checked it. Flying with a
    /// direction then plays `FLYSLOW` in place of `FLY`. The region calls this
    /// once at startup, before logins; it is false until then.
    pub fn set_fly_slow(&self, fly_slow: bool) -> Result<Pending<()>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::FlySlow { fly_slow, reply })?;
        Ok(answer)
    }

    /// Install the region values `SendLandProperties` reads. The region calls
    /// this after settings and the estate are loaded, before logins.
    pub fn set_parcel_region(
        &self,
        region: crate::parcels::ParcelRegion,
    ) -> Result<Pending<()>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::ParcelRegion { region, reply })?;
        Ok(answer)
    }

    /// `ParcelPropertiesRequest`. The elements are queued before the answer.
    pub fn parcel_request(
        &self,
        agent: Uuid,
        query: crate::parcels::PropertiesRequest,
    ) -> Result<Pending<()>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::ParcelProperties {
            agent,
            query,
            reply,
        })?;
        Ok(answer)
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
    /// arrival. The answer is the record the frame thread kept. An existing
    /// child agent is left where it is. `owner` is the circuit generation.
    pub fn post_login(
        &self,
        agent: Uuid,
        pose: Presence,
        owner: Uuid,
    ) -> Result<Pending<Presence>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Pose {
            agent,
            pose,
            owner,
            reply,
        })?;
        Ok(answer)
    }

    /// Seat a neighbour view child at `pose` before `CreateAgent` returns.
    /// The child flag is set and the physics actor is not. `SendInitialData`
    /// is not suppressed.
    pub fn seat_view_child(
        &self,
        agent: Uuid,
        pose: Presence,
        owner: Uuid,
    ) -> Result<Pending<Presence>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::ViewChild {
            agent,
            pose,
            owner,
            reply,
        })?;
        Ok(answer)
    }

    /// `MakeChildAgent` for a stale root on the same session. The position
    /// stays. A missing presence is left alone.
    pub fn make_child(&self, agent: Uuid) -> Result<Pending<()>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::MakeChild { agent, reply })?;
        Ok(answer)
    }

    /// The pose and the child and actor flags, or `None` when the agent is absent.
    pub fn seated(&self, agent: Uuid) -> Result<Pending<Option<Seat>>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Seat { agent, reply })?;
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

    /// The parcel element `sendClientInitialLandInfo` queues after
    /// `AgentMovementComplete` (`ScenePresence.cs` 2441, then 2475). A child,
    /// or no parcel, queues nothing.
    pub fn initial_land(&self, agent: Uuid) -> Result<Pending<()>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::InitialLand { agent, reply })?;
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

    /// One forwarded `AgentAnimation`: each block's id and `StartAnim`. A
    /// missing avatar, or a child, is left alone (`avnChangeAnim`). One
    /// inbound slot, no answer.
    pub fn agent_animation(
        &self,
        agent: Uuid,
        changes: Vec<(Uuid, bool)>,
    ) -> Result<(), TerrainError> {
        self.push(Inbound::Animations { agent, changes })
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
        self.remove_if(agent, None)
    }

    /// Remove the presence only when `owner` still seated it. A replacement
    /// generation's view child stays.
    pub fn remove_owned(
        &self,
        agent: Uuid,
        owner: Uuid,
    ) -> Result<Pending<Option<Leaving>>, TerrainError> {
        self.remove_if(agent, Some(owner))
    }

    fn remove_if(
        &self,
        agent: Uuid,
        owner: Option<Uuid>,
    ) -> Result<Pending<Option<Leaving>>, TerrainError> {
        let (reply, answer) = pending();
        self.request(Request::Remove {
            agent,
            owner,
            reply,
        })?;
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
        // C# sends the pack from the packet handler that changed the set.
        scene.send_animation_packs();
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
        Inbound::Animations { agent, changes } => {
            if let Some(avatar) = presences.get_mut(&agent) {
                avatar.body.viewer_animations(&changes);
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
    let fly_slow = scene.fly_slow;
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
        Request::Remove {
            agent,
            owner,
            reply,
        } => {
            if owner.is_some_and(|owner| {
                presences
                    .get(&agent)
                    .is_some_and(|avatar| avatar.owner != owner)
            }) {
                let _ = reply.send(None);
                return true;
            }
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
        Request::Pose {
            agent,
            pose,
            owner,
            reply,
        } => {
            if let Some(avatar) = presences.get(&agent)
                && avatar.body.child_agent()
            {
                // A view child is already seated. Chat must not replace it
                // with a login hold, or the neighbour enable treats it as a root.
                let _ = reply.send(avatar.pose);
            } else {
                let mut avatar = Avatar::new(agent, pose, fly_slow);
                // Chat can read this pose immediately. The actor waits until
                // the arrival packet samples it, or a tick would change the
                // position that packet and GridUser still report.
                avatar.body.set_actor(false);
                avatar.owner = owner;
                let pose = avatar.pose;
                presences.insert(agent, avatar);
                let _ = reply.send(pose);
            }
        }
        Request::ViewChild {
            agent,
            pose,
            owner,
            reply,
        } => {
            // CloseAgent on a different session, or a root becoming this
            // child, kills the old avatar on every other client. The viewer
            // entry stays so the new attach keeps the local id. A same
            // generation re-seat of a view child does not kill.
            let replacing = scene
                .presences
                .get(&agent)
                .is_some_and(|existing| !existing.body.is_child() || existing.owner != owner);
            if replacing
                && let Some(local_id) = scene.viewers.get(&agent).map(|viewer| viewer.local_id)
            {
                scene.board.remove(local_id);
                for (other, _) in scene.viewer_order() {
                    if other != agent && scene.is_open(&other) {
                        scene.send(&other, AvatarPacket::Kill(vec![local_id]));
                    }
                }
            }
            let mut avatar = Avatar::new(agent, pose, fly_slow);
            avatar.body.set_child(true);
            avatar.body.set_actor(false);
            avatar.owner = owner;
            let stored = avatar.pose;
            scene.presences.insert(agent, avatar);
            let _ = reply.send(stored);
        }
        Request::MakeChild { agent, reply } => {
            if let Some(avatar) = presences.get_mut(&agent) {
                avatar.body.set_child(true);
                avatar.body.set_actor(false);
            }
            if let Some(viewer) = scene.viewers.get_mut(&agent) {
                viewer.initial.made_child();
            }
            let _ = reply.send(());
        }
        Request::Seat { agent, reply } => {
            let seat = presences.get(&agent).map(|avatar| Seat {
                pose: avatar.pose,
                child: avatar.body.child_agent(),
                actor: avatar.body.has_actor(),
            });
            let _ = reply.send(seat);
        }
        Request::Arrive {
            agent,
            pose,
            look_at,
            reply,
        } => {
            let avatar = presences
                .entry(agent)
                .or_insert_with(|| Avatar::new(agent, pose, fly_slow));
            arrive_at_rest(avatar);
            // `CompleteMovement` turns a non-crossing arrival to the look it
            // sends in `AgentMovementComplete` (`RotateToLookAt`).
            avatar
                .body
                .turn_to(look_rotation(look_at), &mut avatar.pose);
            let pose = avatar.pose;
            scene.announce(agent);
            // The parcel element follows `AgentMovementComplete`. The circuit
            // posts `InitialLand` after that datagram (`ScenePresence.cs` 2441,
            // then 2475).
            let _ = reply.send(pose);
        }
        Request::InitialLand { agent, reply } => {
            scene.send_parcel_at(agent);
            let _ = reply.send(());
        }
        Request::MovedAway { agent, away, reply } => {
            // `MakeChildAgent` resets the animations.
            if let Some(avatar) = presences.get_mut(&agent) {
                avatar.body.reset_animations();
            }
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
                .or_insert_with(|| Avatar::new(agent, fresh(position), fly_slow));
            let mut flags = request.flags;
            if avatar.body.flying() {
                flags |= TELEPORT_IS_FLYING;
            }
            // `TeleportWithMomentum` stands a sitter up first (`StandUp(false)`).
            if avatar.body.is_sitting() {
                avatar.body.stand_up();
            }
            avatar.pose.position = position;
            avatar
                .body
                .turn_to(look_rotation(look_at), &mut avatar.pose);
            avatar.pose.velocity = [0.0, 0.0, 0.0];
            avatar.body.halt_command();
            avatar.body.clear_contact();
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
                .or_insert_with(|| Avatar::new(agent, fresh(hop.position), fly_slow));
            avatar.pose.position = hop.position;
            avatar.body.apply_return(&hop.data, &mut avatar.pose);
            // CompleteMovement's RotateToLookAt replaces the copied body rotation.
            avatar
                .body
                .turn_to(look_rotation(hop.look_at), &mut avatar.pose);
            hold_for_arrival(avatar);
            // A teleport sets CrossingFlags to 0, so this is not the
            // border-crossing child pass (ScenePresence.cs 2573–2584).
            let _ = reply.send(avatar.pose);
        }
        Request::ChildSettled {
            agent,
            epoch,
            committed,
        } => {
            if let Some(avatar) = presences.get_mut(&agent) {
                settle_gate(&mut avatar.gate, epoch, committed, scene_now_ms());
            }
        }
        Request::ChildKnown { agent, others } => {
            if let Some(avatar) = presences.get_mut(&agent) {
                avatar.gate.known_other = others;
            }
        }
        Request::Departure { agent, reply } => {
            let departure = presences.get(&agent).map(|avatar| {
                let view = avatar.body.view();
                Departure {
                    always_run: view.always_run,
                    flying: avatar.body.flying(),
                    animations: avatar.body.animator().transfer(),
                    camera_center: view.camera_center,
                    head_rotation: view.head_rotation,
                    body_rotation: view.body_rotation,
                }
            });
            let _ = reply.send(departure);
        }
        Request::ParcelOut { send, reply } => {
            scene.parcel_out = Some(send);
            let _ = reply.send(());
        }
        Request::ParcelRegion { region, reply } => {
            scene.parcel_region = region;
            let _ = reply.send(());
        }
        Request::FlySlow { fly_slow, reply } => {
            scene.fly_slow = fly_slow;
            for avatar in scene.presences.values_mut() {
                avatar.body.set_fly_slow(fly_slow);
            }
            let _ = reply.send(());
        }
        Request::ParcelProperties {
            agent,
            query,
            reply,
        } => {
            scene.answer_parcel(agent, query);
            let _ = reply.send(());
        }
        Request::Wake => {}
        Request::Stop => return false,
    }
    true
}

impl Avatar {
    fn new(agent: Uuid, pose: Presence, fly_slow: bool) -> Self {
        let mut body = Body::new(&pose);
        body.set_agent(agent);
        body.set_fly_slow(fly_slow);
        Self {
            body,
            pose,
            gate: ChildGate::default(),
            parcel: None,
            last_move: pose.position,
            owner: Uuid::nil(),
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
    avatar.body.new_actor();
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

fn scene_now_ms() -> i64 {
    i64::try_from(clock_ms(Instant::now())).unwrap_or(i64::MAX)
}

/// `ScenePresence.RegionViewDistance`: draw + 64, clamped to the region limits.
/// A zero result is the child half's early return.
fn region_view(draw: f32, min: f32, max: f32) -> f32 {
    let (low, high) = if min <= max { (min, max) } else { (max, min) };
    if !low.is_finite() || !high.is_finite() {
        return 0.0;
    }
    (draw + 64.0).clamp(low, high)
}

fn length_sq(left: [f32; 3], right: [f32; 3]) -> f32 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    let dz = left[2] - right[2];
    dx * dx + dy * dy + dz * dz
}

/// `Util.EnvironmentTickCountSubtract` (`Util.cs` 3620). TickCount is masked
/// to 30 bits. A negative difference adds the modulus, so arrival's
/// `now + 10000` looks older than the 2 s gate and does not hold it.
fn child_elapsed_ms(now_ms: i64, last_ms: i64) -> i64 {
    let diff = now_ms.wrapping_sub(last_ms);
    if diff >= 0 {
        diff
    } else {
        diff.saturating_add(CHILD_TICK_MODULUS)
    }
}

/// The child half of `CheckForSignificantMovement` (lines 4750–4828).
/// The 16 m trigger is not here: a move whose square is 256 enables nothing
/// and, once a neighbour is known, sends `AgentPosition`.
fn due(gate: &ChildGate, sample: &ChildSample) -> ChildDue {
    if sample.child || !gate.armed || gate.busy || sample.view == 0.0 {
        return ChildDue::None;
    }
    if child_elapsed_ms(sample.now_ms, gate.last_ms) < CHILD_UPDATES_MS {
        return ChildDue::None;
    }
    let view_changed = (sample.view - gate.last_view).abs() > CHILD_RANGE_CHANGE;
    let dx = sample.position[0] - gate.check_position[0];
    let dy = sample.position[1] - gate.check_position[1];
    if view_changed || dx * dx + dy * dy > CHILD_CHECK_DISTANCE_SQ {
        return ChildDue::Enable;
    }
    if !gate.known_other {
        return ChildDue::None;
    }
    let draw_changed = (sample.draw - gate.last_draw).abs() > CHILD_RANGE_CHANGE;
    let moved = length_sq(sample.position, gate.update_position) > CHILD_UPDATE_DISTANCE_SQ;
    if sample.god != gate.last_god || draw_changed || moved {
        ChildDue::Position
    } else {
        ChildDue::None
    }
}

fn child_notice(
    presences: &HashMap<Uuid, Avatar>,
    viewers: &HashMap<Uuid, Viewer>,
    agent: Uuid,
    pass: ChildPass,
    settle: bool,
    epoch: u64,
) -> Option<ChildNotice> {
    let avatar = presences.get(&agent)?;
    let height = viewers
        .get(&agent)
        .map(|viewer| viewer.link.look.height)
        .filter(|height| height.is_finite() && *height > 0.0)
        .unwrap_or(CHILD_DEFAULT_HEIGHT);
    Some(ChildNotice {
        agent,
        pass,
        position: avatar.pose.position,
        velocity: avatar.pose.velocity,
        camera: avatar.body.view().camera_center,
        draw_distance: avatar.pose.draw_distance,
        height,
        god: false,
        settle,
        epoch,
    })
}

fn offer_child(watch: &ChildWatch, notice: ChildNotice) -> bool {
    let slot = match watch.sender.try_lock() {
        Ok(slot) => slot,
        Err(std::sync::TryLockError::Poisoned(poison)) => poison.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return false,
    };
    let Some(sender) = slot.as_ref() else {
        return false;
    };
    match sender.try_send(notice) {
        Ok(()) => true,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            let configured = sender.max_capacity() as u64;
            refuse(
                REGION_CHILD_NOTICES,
                Kind::Soft,
                configured,
                configured.saturating_add(1),
                "child-notice",
            );
            false
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
    }
}

fn snap_of(gate: &ChildGate) -> ChildSnap {
    ChildSnap {
        update_position: gate.update_position,
        check_position: gate.check_position,
        draw: gate.last_draw,
        view: gate.last_view,
        god: gate.last_god,
    }
}

fn restore_snap(gate: &mut ChildGate, saved: ChildSnap) {
    gate.update_position = saved.update_position;
    gate.check_position = saved.check_position;
    gate.last_draw = saved.draw;
    gate.last_view = saved.view;
    gate.last_god = saved.god;
}

fn stamp_arrival(gate: &mut ChildGate, pose: &Presence, view: f32, now_ms: i64) {
    gate.last_ms = now_ms.saturating_add(CHILD_ARRIVAL_STAMP_MS);
    gate.update_position = pose.position;
    gate.check_position = pose.position;
    gate.last_draw = pose.draw_distance;
    gate.last_view = view;
    gate.last_god = false;
    gate.busy = false;
    gate.armed = true;
    gate.arrival_pending = false;
    gate.saved = None;
    // Drop any pass that has not settled yet. Its later settle must not
    // replace this stamp.
    gate.epoch = gate.epoch.wrapping_add(1);
    gate.inflight = 0;
}

fn advance_enable(gate: &mut ChildGate, pose: &Presence, view: f32, epoch: u64) {
    gate.saved = Some(snap_of(gate));
    gate.check_position = pose.position;
    gate.update_position = pose.position;
    gate.last_draw = pose.draw_distance;
    gate.last_view = view;
    gate.last_god = false;
    gate.busy = true;
    gate.epoch = epoch;
    gate.inflight = epoch;
}

fn advance_position(gate: &mut ChildGate, pose: &Presence, epoch: u64) {
    gate.saved = Some(snap_of(gate));
    gate.update_position = pose.position;
    gate.last_draw = pose.draw_distance;
    gate.last_god = false;
    gate.busy = true;
    gate.epoch = epoch;
    gate.inflight = epoch;
}

fn rollback_gate(gate: &mut ChildGate) {
    gate.busy = false;
    gate.inflight = 0;
    if let Some(saved) = gate.saved.take() {
        restore_snap(gate, saved);
    }
}

fn settle_gate(gate: &mut ChildGate, epoch: u64, committed: bool, now_ms: i64) {
    if epoch == 0 || gate.inflight != epoch {
        return;
    }
    gate.busy = false;
    gate.inflight = 0;
    gate.last_ms = now_ms;
    if committed {
        gate.saved = None;
    } else if let Some(saved) = gate.saved.take() {
        restore_snap(gate, saved);
    }
}

/// Arrival's `EnableChildAgents`. A full channel leaves the pass pending
/// and does not stamp the arrival time.
fn schedule_arrival(scene: &mut Scene, agent: Uuid, now_ms: i64) {
    let away = scene
        .presences
        .get(&agent)
        .is_none_or(|avatar| avatar.body.child_agent());
    if away {
        return;
    }
    let (min, max) = scene.watch.limits();
    let Some(notice) = child_notice(
        &scene.presences,
        &scene.viewers,
        agent,
        ChildPass::Enable,
        false,
        0,
    ) else {
        return;
    };
    if offer_child(&scene.watch, notice) {
        if let Some(avatar) = scene.presences.get_mut(&agent) {
            let view = region_view(avatar.pose.draw_distance, min, max);
            stamp_arrival(&mut avatar.gate, &avatar.pose, view, now_ms);
        }
    } else if let Some(avatar) = scene.presences.get_mut(&agent) {
        avatar.gate.arrival_pending = true;
    }
}

fn consider_children(scene: &mut Scene, now_ms: i64) {
    let agents: Vec<Uuid> = scene.presences.keys().copied().collect();
    for agent in agents {
        consider_child(scene, agent, now_ms);
    }
}

fn consider_child(scene: &mut Scene, agent: Uuid, now_ms: i64) {
    let away = scene
        .presences
        .get(&agent)
        .is_none_or(|avatar| avatar.body.child_agent());
    if away {
        if let Some(avatar) = scene.presences.get_mut(&agent) {
            avatar.gate.arrival_pending = false;
        }
        return;
    }
    if scene
        .presences
        .get(&agent)
        .is_some_and(|avatar| avatar.gate.arrival_pending)
    {
        schedule_arrival(scene, agent, now_ms);
        return;
    }
    let (min, max) = scene.watch.limits();
    let Some(avatar) = scene.presences.get(&agent) else {
        return;
    };
    let view = region_view(avatar.pose.draw_distance, min, max);
    let sample = ChildSample {
        now_ms,
        child: false,
        position: avatar.pose.position,
        draw: avatar.pose.draw_distance,
        view,
        god: false,
    };
    let decision = due(&avatar.gate, &sample);
    let pass = match decision {
        ChildDue::None => return,
        ChildDue::Enable => ChildPass::Enable,
        ChildDue::Position => ChildPass::Position,
    };
    let epoch = avatar.gate.epoch.wrapping_add(1);
    let Some(notice) = child_notice(&scene.presences, &scene.viewers, agent, pass, true, epoch)
    else {
        return;
    };
    let Some(avatar) = scene.presences.get_mut(&agent) else {
        return;
    };
    match decision {
        ChildDue::Enable => advance_enable(&mut avatar.gate, &avatar.pose, view, epoch),
        ChildDue::Position => advance_position(&mut avatar.gate, &avatar.pose, epoch),
        ChildDue::None => return,
    }
    if !offer_child(&scene.watch, notice)
        && let Some(avatar) = scene.presences.get_mut(&agent)
    {
        rollback_gate(&mut avatar.gate);
    }
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
        // A local teleport stands a sitter up (`StandUp`), which sends.
        scene.send_animation_packs();
    }
}

/// One frame: step every avatar, run `ScenePresence.Update` on each, count
/// the frame, and send the minimap dots on every 50th. Returns the count.
fn tick(scene: &mut Scene, map: Option<&Heightmap>, frames: u64) -> u64 {
    step_avatars(&mut scene.presences, map);
    consider_children(scene, scene_now_ms());
    // The delayed stop's `AddNewMovement` can change the movement state.
    scene.send_animation_packs();
    // `EventManagerOnClientMovement` after the avatar has moved. A teleport
    // only stores the new position; this is the send on the next tick.
    note_parcel_crossings(scene);
    update_presences(scene);
    heartbeat_end(&mut scene.presences);
    scene.send_animation_packs();
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

    /// `SendLandUpdateToClient`: one sequence-0 element for the parcel under
    /// the avatar. A child, or no parcel, sends nothing and leaves the stored
    /// id alone. The circuit posts this after `AgentMovementComplete`, and
    /// the zero-flag initial-data gate posts it for a root.
    fn send_parcel_at(&mut self, agent: Uuid) {
        let position = {
            let Some(avatar) = self.presences.get(&agent) else {
                return;
            };
            if avatar.body.is_child() {
                return;
            }
            avatar.pose.position
        };
        let Some(parcels) = self.parcels.clone() else {
            return;
        };
        let Some((id, local_id)) = parcels
            .at_metres(position[0], position[1])
            .map(|parcel| (parcel.id, parcel.local_id))
        else {
            return;
        };
        if let Some(avatar) = self.presences.get_mut(&agent) {
            avatar.parcel = Some(id);
        }
        self.deliver_notify(agent, local_id, id);
    }

    /// `SendLandUpdateToClient` calls `SendLandProperties(0, false, 0)`, which
    /// stores sequence 0 on that parcel (`LandObject.cs` 509–519).
    fn deliver_notify(&mut self, agent: Uuid, local_id: i32, parcel_id: Uuid) {
        let Some(element) = self.parcel_element(local_id, parcel_id) else {
            return;
        };
        self.parcel_seq.insert(local_id, 0);
        self.emit_parcel(agent, &element);
    }

    /// The element for one parcel: installed region values and the tally of
    /// admitted groups. The tally is computed once; the admitted set does not
    /// change.
    fn parcel_element(&mut self, local_id: i32, parcel_id: Uuid) -> Option<Vec<u8>> {
        let parcels = Arc::clone(self.parcels.as_ref()?);
        if self.admitted_counts.is_none() {
            self.admitted_counts = Some(crate::parcels::prim_counts(&parcels, &self.groups));
        }
        let prims = self
            .admitted_counts
            .as_ref()
            .and_then(|counts| counts.get(&parcel_id).copied())
            .unwrap_or_default();
        parcels.parcel_properties(local_id, &self.parcel_region.element(prims))
    }

    fn emit_parcel(&self, agent: Uuid, element: &[u8]) {
        if let Some(send) = &self.parcel_out {
            send(agent, element);
        }
    }

    /// `ClientOnParcelPropertiesRequest`. A child is not special: the packet
    /// handler does not check one. The sequence clock is the land object's.
    fn answer_parcel(&mut self, agent: Uuid, query: crate::parcels::PropertiesRequest) {
        let Some(parcels) = self.parcels.clone() else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i32;
        let elements = parcels.properties_request(agent, query, now, &mut self.parcel_seq);
        for element in &elements {
            self.emit_parcel(agent, element);
        }
    }

    /// `CompleteMovement`: the avatar's full update to every other viewer
    /// (`SendInitialAvatarDataToAllAgents`), then its appearance and its
    /// animations to each (`SendAppearanceToAgentNF`,
    /// `SendAnimPackToAgentNF`). Its own viewer gets the full update and the
    /// appearance from its circuit, with the arrival packet, and its
    /// animations here (`SendAnimPackToAgent`): the circuit takes this feed
    /// only after it has queued those two.
    fn announce(&self, agent: Uuid) {
        let Some(full) = self.full_of(agent) else {
            return;
        };
        let animations = self.animations_of(agent).unwrap_or_default();
        self.send(
            &agent,
            AvatarPacket::Animations {
                agent,
                animations: animations.clone(),
            },
        );
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
            self.send(
                other,
                AvatarPacket::Animations {
                    agent,
                    animations: animations.clone(),
                },
            );
        }
    }

    /// `Animator.GetArrays`: the default first unless it is nil, then the others.
    fn animations_of(&self, agent: Uuid) -> Option<Vec<Anim>> {
        self.presences
            .get(&agent)
            .map(|avatar| avatar.body.animator().animations().arrays())
    }

    /// `ScenePresenceAnimator.SendAnimPack` for every avatar whose animator
    /// owes one: a root's set to every open viewer, its own and children
    /// included (`ForEachScenePresence`). A child owes nothing and sends
    /// nothing (`IsChildAgent`).
    fn send_animation_packs(&mut self) {
        let mut due = Vec::new();
        for (agent, avatar) in &mut self.presences {
            if avatar.body.take_animation_pack() && !avatar.body.is_child() {
                due.push((*agent, avatar.body.animator().animations().arrays()));
            }
        }
        if due.is_empty() {
            return;
        }
        let order = self.viewer_order();
        due.sort_by_key(|(agent, _)| {
            order
                .iter()
                .position(|(viewer, _)| viewer == agent)
                .unwrap_or(usize::MAX)
        });
        for (agent, animations) in due {
            for (viewer, _) in &order {
                if self.is_open(viewer) {
                    self.send(
                        viewer,
                        AvatarPacket::Animations {
                            agent,
                            animations: animations.clone(),
                        },
                    );
                }
            }
        }
    }

    /// `SendOtherAgentsAvatarFullToMe`: every other root avatar's full update,
    /// then its appearance, then its animations, to this viewer. A root then
    /// arms one child pass, which is `EnableChildAgents` inside
    /// `SendInitialData` (`ScenePresence.cs` 4395–4407), after the handshake
    /// reply, the seeds and four more frames. A border crossing is a separate
    /// site.
    fn send_initial_data(&mut self, agent: Uuid) {
        // ScenePresence.cs 4337 logs before the teleport-flag check.
        tracing::info!("SendInitialData for \"{agent}\"");
        // ScenePresence.cs 4338: flags of zero request terrain, one parcel element
        // and the overlay again. A child still gets the overlay.
        let again = self
            .viewers
            .get(&agent)
            .is_some_and(|viewer| viewer.link.teleport_flags == 0);
        if again && self.is_root(&agent) {
            self.send_parcel_at(agent);
        }
        if again && let Some(viewer) = self.viewers.get(&agent) {
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
            if let Some(animations) = self.animations_of(other) {
                self.send(
                    &agent,
                    AvatarPacket::Animations {
                        agent: other,
                        animations,
                    },
                );
            }
        }
        if let (Some(viewer), Some(avatar)) = (self.viewers.get(&agent), self.presences.get(&agent))
        {
            viewer.link.feed.offer_stored_parts(&PriorityViewer {
                agent_id: agent,
                position: Some(avatar.pose.position),
                sitting_on: None,
            });
        }
        schedule_arrival(self, agent, scene_now_ms());
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

/// `RegionHeartbeatEnd` for every root: the animator updates on every second
/// one. C# subscribes a presence in `CompleteMovement` and drops it in
/// `MakeChildAgent`; here a child, and a pose the arrival has not sampled, is
/// skipped.
fn heartbeat_end(presences: &mut HashMap<Uuid, Avatar>) {
    let now = clock_ms(Instant::now());
    for avatar in presences.values_mut() {
        if !avatar.body.is_child() {
            avatar.body.heartbeat(now);
        }
    }
}

/// `ScenePresence.MOVEMENT`: more than half a metre, as a squared length.
const MOVEMENT_SQ: f32 = 0.25;

/// `CheckForSignificantMovement` (`ScenePresence.cs` 4726–4734). A NaN
/// difference does not pass.
fn significant_move(from: [f32; 3], to: [f32; 3]) -> bool {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let dz = to[2] - from[2];
    dx * dx + dy * dy + dz * dz > MOVEMENT_SQ
}

/// `EventManagerOnClientMovement` for a root whose parcel `GlobalID` changed.
///
/// `Update` returns before that check while `NeedInitialData` is still
/// positive (`ScenePresence.cs` 4067–4074) and while the avatar is a child.
/// A crossing freeze is not stored, so that part of the return is not applied.
/// The position is recorded only after a move of more than half a metre. The
/// same id, or no parcel, sends nothing. Ban lines stay out.
fn note_parcel_crossings(scene: &mut Scene) {
    let Some(parcels) = scene.parcels.clone() else {
        return;
    };
    let mut moved = Vec::new();
    for (agent, avatar) in &scene.presences {
        if avatar.body.is_child() {
            continue;
        }
        if scene
            .viewers
            .get(agent)
            .is_some_and(|viewer| viewer.initial.waiting())
        {
            continue;
        }
        if !significant_move(avatar.last_move, avatar.pose.position) {
            continue;
        }
        let parcel = parcels
            .at_metres(avatar.pose.position[0], avatar.pose.position[1])
            .map(|parcel| (parcel.id, parcel.local_id));
        moved.push((*agent, avatar.pose.position, parcel));
    }
    let mut crossed = Vec::new();
    for (agent, position, parcel) in moved {
        let Some(avatar) = scene.presences.get_mut(&agent) else {
            continue;
        };
        avatar.last_move = position;
        let Some((id, local_id)) = parcel else {
            continue;
        };
        if avatar.parcel == Some(id) {
            continue;
        }
        avatar.parcel = Some(id);
        crossed.push((agent, id, local_id));
    }
    for (agent, id, local_id) in crossed {
        scene.deliver_notify(agent, local_id, id);
    }
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
            .post_login(agent, pose, Uuid::nil())
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
                animations: crate::animation::AnimationTransfer::default(),
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

    /// The agent ids of the full updates, appearances and animations, and
    /// the kills, in the order a feed holds them.
    fn packet_kinds(feed: &ViewerFeed) -> Vec<(char, u128)> {
        feed.take_avatar_packets(usize::MAX)
            .into_iter()
            .filter_map(|packet| match packet {
                AvatarPacket::Full(full) => Some(('F', full.agent.as_u128())),
                AvatarPacket::Appearance { agent, .. } => Some(('A', agent.as_u128())),
                AvatarPacket::Animations { agent, .. } => Some(('M', agent.as_u128())),
                AvatarPacket::Kill(ids) => Some(('K', u128::from(ids[0]))),
                // The frame thread's clock decides when a minimap list falls.
                AvatarPacket::Coarse(_) => None,
            })
            .collect()
    }

    /// `CompleteMovement` sends the newcomer's full update, then its
    /// appearance and its animations, to each viewer already there, a
    /// child's included. The newcomer learns of the
    /// other root at its `SendInitialData`: the reply, the seeds, four frames.
    #[test]
    fn an_arrival_is_announced_and_the_newcomer_hears_of_the_others() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let first = Uuid::from_u128(60);
        let second = Uuid::from_u128(61);
        let (_, first_feed, _) = attach(&frame, first);
        login(&frame, first, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, first);
        // Its own full update and appearance are the circuit's; its
        // animations follow them (`SendAnimPackToAgent`).
        assert_eq!(packet_kinds(&first_feed), [('M', 60)]);
        // A viewer whose avatar is away here (a child) hears of arrivals
        // too: `CompleteMovement` loops over every presence.
        let child = Uuid::from_u128(59);
        let (_, child_feed, _) = attach(&frame, child);
        login(&frame, child, pose_at([130.0, 128.0, 25.0]));
        reuse(&frame, child);
        frame.set_child(child, true).unwrap();
        let _ = first_feed.take_avatar_packets(usize::MAX);
        let _ = child_feed.take_avatar_packets(usize::MAX);
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
        assert_eq!(packet_kinds(&child_feed), [('F', 61), ('A', 61), ('M', 61)]);
        let announced = first_feed.take_avatar_packets(usize::MAX);
        assert_eq!(announced.len(), 3);
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
        // A new set: `STAND` sequence 1, no object.
        let standing = vec![Anim {
            animation: crate::animation::STAND,
            sequence: 1,
            object_id: Uuid::nil(),
        }];
        assert_eq!(
            announced[2],
            AvatarPacket::Animations {
                agent: second,
                animations: standing.clone()
            }
        );
        assert_eq!(
            second_feed.take_avatar_packets(usize::MAX),
            [AvatarPacket::Animations {
                agent: second,
                animations: standing
            }]
        );
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
        assert_eq!(sent, [('F', 60), ('A', 60), ('M', 60)]);
        assert!(
            seen >= before + 4,
            "SendInitialData ran {} frames after the reply, before the fourth",
            seen - before
        );
        // Once only.
        wait_frames(&frame, 3);
        assert!(packet_kinds(&second_feed).is_empty());
        assert!(packet_kinds(&first_feed).is_empty());
        assert!(packet_kinds(&child_feed).is_empty());
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
        assert_eq!(packet_kinds(&feed), [('M', newcomer.as_u128())]);
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
            watch: Arc::new(ChildWatch::new()),
            parcels: None,
            parcel_out: None,
            parcel_region: crate::parcels::ParcelRegion::default(),
            groups: Arc::default(),
            admitted_counts: None,
            parcel_seq: HashMap::new(),
            fly_slow: false,
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

    /// The follow-up after arrival, the zero-flag gate and a GlobalID change
    /// each queue one element. `Arrive` itself queues none. Heights are the
    /// samples stored at land load, so the bytes match the Q61a files. A held
    /// login, a same-parcel move, the teleport itself and a child send nothing.
    #[tokio::test]
    async fn parcel_properties_follow_arrival_the_gate_and_a_walk() {
        let parcels = loaded_region_parcels().await;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/parcel-properties");
        let arrival = std::fs::read(root.join("arrival.bin")).unwrap();
        let walk = std::fs::read(root.join("walk.bin")).unwrap();

        let mut scene = scene_with(&parcels);
        let sent = record_parcels(&mut scene);
        let agent = Uuid::from_u128(1);
        let _ = open_viewer_flags(&mut scene, agent, true, 1 << 7);
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Pose {
                agent,
                pose: pose_at([128.0, 128.0, 25.0]),
                owner: Uuid::nil(),
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        note_parcel_crossings(&mut scene);
        assert!(sent.lock().unwrap().is_empty());
        arrive_in(&mut scene, agent, [128.0, 128.0, 25.0]);
        assert!(sent.lock().unwrap().is_empty());
        scene.send_parcel_at(agent);
        assert_eq!(
            sent.lock().unwrap().as_slice(),
            std::slice::from_ref(&arrival)
        );
        // The walk runs after initial data. Flags are not zero, so this
        // queues no second element.
        arm_initial(&mut scene, agent);
        assert_eq!(sent.lock().unwrap().len(), 1);

        scene.presences.get_mut(&agent).unwrap().pose.position = [120.0, 128.0, 25.0];
        note_parcel_crossings(&mut scene);
        assert_eq!(sent.lock().unwrap().len(), 1);

        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Teleport {
                agent,
                request: LocalTeleport {
                    position: [6.0, 2.0, 30.0],
                    look_at: [1.0, 0.0, 0.0],
                    half_height: 1.0,
                    default_landing: [128.0, 128.0, 23.0],
                    size_x: 256.0,
                    size_y: 256.0,
                    flags: 0,
                },
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        assert_eq!(sent.lock().unwrap().len(), 1);
        note_parcel_crossings(&mut scene);
        {
            let queued = sent.lock().unwrap();
            assert_eq!(queued.len(), 2);
            assert!(
                queued[1]
                    .windows(b"<key>Name</key><string>West</string>".len())
                    .any(|window| window == b"<key>Name</key><string>West</string>")
            );
            assert!(
                queued[1]
                    .windows(b"<key>SequenceID</key><integer />".len())
                    .any(|window| window == b"<key>SequenceID</key><integer />")
            );
        }

        scene.presences.get_mut(&agent).unwrap().pose.position = [10.0, 2.0, 30.0];
        note_parcel_crossings(&mut scene);
        assert_eq!(sent.lock().unwrap().last().unwrap(), &walk);
        note_parcel_crossings(&mut scene);
        assert_eq!(sent.lock().unwrap().len(), 3);

        scene
            .presences
            .get_mut(&agent)
            .unwrap()
            .body
            .set_child(true);
        scene.presences.get_mut(&agent).unwrap().pose.position = [128.0, 128.0, 25.0];
        note_parcel_crossings(&mut scene);
        assert_eq!(sent.lock().unwrap().len(), 3);

        let mut gated = scene_with(&parcels);
        let gated_sent = record_parcels(&mut gated);
        let root_agent = Uuid::from_u128(2);
        let (feed, _) = open_viewer_flags(&mut gated, root_agent, true, 0);
        arrive_in(&mut gated, root_agent, [128.0, 128.0, 25.0]);
        assert!(gated_sent.lock().unwrap().is_empty());
        gated.send_parcel_at(root_agent);
        arm_initial(&mut gated, root_agent);
        {
            let queued = gated_sent.lock().unwrap();
            assert_eq!(queued.len(), 2);
            assert_eq!(queued[0], arrival);
            assert_eq!(queued[1], arrival);
        }
        assert!(feed.take_land_pass());

        let mut login = scene_with(&parcels);
        let login_sent = record_parcels(&mut login);
        let login_agent = Uuid::from_u128(3);
        let (feed, _) = open_viewer_flags(&mut login, login_agent, true, 1 << 7);
        arrive_in(&mut login, login_agent, [128.0, 128.0, 25.0]);
        assert!(login_sent.lock().unwrap().is_empty());
        login.send_parcel_at(login_agent);
        arm_initial(&mut login, login_agent);
        assert_eq!(login_sent.lock().unwrap().as_slice(), &[arrival]);
        assert!(!feed.take_land_pass());
    }

    /// The send path reads the installed region and the admitted groups.
    /// Bonus 2 makes West's max prims 7. Restrict pushing sets the push
    /// override. The two-part group on West is the owner's count.
    #[tokio::test]
    async fn a_west_send_uses_bonus_push_override_and_the_stored_tally() {
        let parcels = loaded_region_parcels().await;
        let owner = parcels.at_metres(6.0, 2.0).unwrap().owner;
        let mut scene = scene_with(&parcels);
        scene.parcel_region =
            crate::parcels::ParcelRegion::from_handshake(1 << 22, 2.0, 15_000, false, false);
        let mut root = crate::xml::blank_part();
        root.owner_id = owner;
        root.group_position = [6.0, 2.0, 30.0];
        root.shape.pcode = 9;
        let child = crate::xml::blank_part();
        scene.groups = Arc::new(vec![crate::ObjectGroup {
            from_item_id: Uuid::nil(),
            rez_parameter: None,
            parts: vec![root, child],
        }]);
        let sent = record_parcels(&mut scene);
        let agent = Uuid::from_u128(9);
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Pose {
                agent,
                pose: pose_at([6.0, 2.0, 30.0]),
                owner: Uuid::nil(),
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        {
            let avatar = scene.presences.get_mut(&agent).unwrap();
            avatar.body.set_actor(true);
            avatar.body.set_child(false);
        }
        scene.send_parcel_at(agent);
        let xml = String::from_utf8(sent.lock().unwrap()[0].clone()).unwrap();
        assert!(
            xml.contains("<key>Name</key><string>West</string>"),
            "{xml}"
        );
        // The stored West is 48 m², so bonus 2 is 22. A 16 m² West is 7.
        assert!(
            xml.contains("<key>MaxPrims</key><integer>22</integer>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>ParcelPrimBonus</key><real>2</real>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>RegionPushOverride</key><boolean>1</boolean>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>OwnerPrims</key><integer>2</integer>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>TotalPrims</key><integer>2</integer>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>SimWideTotalPrims</key><integer>2</integer>"),
            "{xml}"
        );
        assert!(xml.contains("<key>SelectedPrims</key><integer />"), "{xml}");
    }

    /// West at 7.8 m, East at 8.2 m. The step is 0.4 m, so no element is
    /// queued and the last significant position stays put. 8.4 m is 0.6 m
    /// from that position and does queue East. A longer step while initial
    /// data is still owed also queues nothing.
    #[tokio::test]
    async fn a_crossing_under_half_a_metre_sends_nothing() {
        let parcels = loaded_region_parcels().await;
        let mut scene = scene_with(&parcels);
        let sent = record_parcels(&mut scene);
        let agent = Uuid::from_u128(11);
        let _ = open_viewer_flags(&mut scene, agent, true, 1 << 7);
        arrive_in(&mut scene, agent, [7.8, 2.0, 30.0]);
        assert!(sent.lock().unwrap().is_empty());
        scene.send_parcel_at(agent);
        let arrived = sent.lock().unwrap().len();
        assert_eq!(arrived, 1);
        scene.presences.get_mut(&agent).unwrap().pose.position = [8.4, 2.0, 30.0];
        note_parcel_crossings(&mut scene);
        assert_eq!(sent.lock().unwrap().len(), arrived);
        assert_eq!(
            scene.presences.get(&agent).unwrap().last_move,
            [7.8, 2.0, 30.0]
        );
        scene.presences.get_mut(&agent).unwrap().pose.position = [7.8, 2.0, 30.0];
        arm_initial(&mut scene, agent);
        assert_eq!(sent.lock().unwrap().len(), arrived);

        scene.presences.get_mut(&agent).unwrap().pose.position = [8.2, 2.0, 30.0];
        note_parcel_crossings(&mut scene);
        assert_eq!(sent.lock().unwrap().len(), arrived);
        assert_eq!(
            scene.presences.get(&agent).unwrap().last_move,
            [7.8, 2.0, 30.0]
        );

        scene.presences.get_mut(&agent).unwrap().pose.position = [8.4, 2.0, 30.0];
        note_parcel_crossings(&mut scene);
        let queued = sent.lock().unwrap();
        assert_eq!(queued.len(), arrived + 1);
        assert!(
            queued
                .last()
                .unwrap()
                .windows(b"<key>Name</key><string>East</string>".len())
                .any(|window| window == b"<key>Name</key><string>East</string>")
        );
    }

    /// A return-slot child at the zero-flag gate gets the land pass and no
    /// element. Either the child check on the send or the root check on the
    /// gate is enough to keep the element out.
    #[tokio::test]
    async fn a_child_at_the_gate_gets_a_land_pass_and_no_element() {
        let parcels = loaded_region_parcels().await;
        let mut scene = scene_with(&parcels);
        let sent = record_parcels(&mut scene);
        let agent = Uuid::from_u128(12);
        let (feed, _) = open_viewer_flags(&mut scene, agent, true, 0);
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Pose {
                agent,
                pose: pose_at([6.0, 2.0, 30.0]),
                owner: Uuid::nil(),
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        scene
            .presences
            .get_mut(&agent)
            .unwrap()
            .body
            .set_child(true);
        arm_initial(&mut scene, agent);
        assert!(sent.lock().unwrap().is_empty());
        assert!(feed.take_land_pass());
    }

    /// The constructor samples the heightmap it is given. A flat map sampled
    /// afterwards is a different box, and the frame keeps the first one.
    #[tokio::test]
    async fn with_object_settings_stores_the_load_time_box() {
        let height = crate::InitialTerrain::PinheadIsland
            .generate(256, 256)
            .unwrap();
        let raw = region_parcel_map().await;
        let mut expected = region_parcel_map().await;
        expected.update_geometry(Some(&height));
        let mut later = region_parcel_map().await;
        let flat = crate::Heightmap::from_cells(256, 256, vec![3.0; 256 * 256]);
        later.update_geometry(Some(&flat));
        let frame = TerrainFrame::with_object_settings(
            Some(height),
            Some(raw),
            Arc::default(),
            None,
            crate::PhysicsScaleLimits::default(),
            true,
            default_jolt_config().unwrap(),
        )
        .unwrap();
        let stored = frame.parcels().unwrap();
        assert_eq!(stored.parcels().len(), expected.parcels().len());
        let mut differed = false;
        for ((got, want), flat_parcel) in stored
            .parcels()
            .iter()
            .zip(expected.parcels())
            .zip(later.parcels())
        {
            assert_eq!(got.aabb_min, want.aabb_min, "{}", got.name);
            assert_eq!(got.aabb_max, want.aabb_max, "{}", got.name);
            if got.aabb_min != flat_parcel.aabb_min || got.aabb_max != flat_parcel.aabb_max {
                differed = true;
            }
        }
        assert!(differed);
    }

    /// One cell, the two-parcel span and a snap reuse the land sequence.
    /// An out-of-range bound sends nothing.
    #[tokio::test]
    async fn a_parcel_request_sends_the_captured_cell_and_span() {
        let parcels = loaded_region_parcels().await;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/parcel-properties");
        let cell = std::fs::read(root.join("request-cell.bin")).unwrap();
        let west = std::fs::read(root.join("request-span-west.bin")).unwrap();
        let east = std::fs::read(root.join("request-span-east.bin")).unwrap();
        let mut scene = scene_with(&parcels);
        let sent = record_parcels(&mut scene);
        let agent = uuid::uuid!("4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55");
        let ask = |scene: &mut Scene, query: crate::parcels::PropertiesRequest| {
            let (reply, _answer) = oneshot::channel();
            assert!(answer(
                Request::ParcelProperties {
                    agent,
                    query,
                    reply
                },
                scene,
                None,
                &[],
                0,
                no_physics(),
            ));
        };
        ask(
            &mut scene,
            crate::parcels::PropertiesRequest {
                west: 5.0,
                south: 1.0,
                east: 6.0,
                north: 2.0,
                sequence_id: 7,
                snap_selection: false,
            },
        );
        assert_eq!(sent.lock().unwrap().as_slice(), &[cell]);
        sent.lock().unwrap().clear();
        ask(
            &mut scene,
            crate::parcels::PropertiesRequest {
                west: 4.0,
                south: 0.0,
                east: 12.0,
                north: 4.0,
                sequence_id: 8,
                snap_selection: false,
            },
        );
        assert_eq!(sent.lock().unwrap().as_slice(), &[west, east]);
        sent.lock().unwrap().clear();
        ask(
            &mut scene,
            crate::parcels::PropertiesRequest {
                west: 5.0,
                south: 1.0,
                east: 6.0,
                north: 2.0,
                sequence_id: 0,
                snap_selection: true,
            },
        );
        let snap = sent.lock().unwrap()[0].clone();
        let xml = String::from_utf8(snap).unwrap();
        assert!(
            xml.contains("<key>SequenceID</key><integer>8</integer>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>SnapSelection</key><boolean>1</boolean>"),
            "{xml}"
        );
        sent.lock().unwrap().clear();
        ask(
            &mut scene,
            crate::parcels::PropertiesRequest {
                west: -1.0,
                south: 0.0,
                east: 4.0,
                north: 4.0,
                sequence_id: 9,
                snap_selection: false,
            },
        );
        assert!(sent.lock().unwrap().is_empty());
    }

    /// A request stores sequence 8. Arrival stores 0, so the next snap reads 0.
    #[tokio::test]
    async fn an_arrival_resets_the_parcel_sequence_before_a_snap() {
        let parcels = loaded_region_parcels().await;
        let mut scene = scene_with(&parcels);
        let sent = record_parcels(&mut scene);
        let agent = uuid::uuid!("4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55");
        let ask = |scene: &mut Scene, sequence_id: i32, snap_selection: bool| {
            let (reply, _answer) = oneshot::channel();
            assert!(answer(
                Request::ParcelProperties {
                    agent,
                    query: crate::parcels::PropertiesRequest {
                        west: 128.0,
                        south: 128.0,
                        east: 132.0,
                        north: 132.0,
                        sequence_id,
                        snap_selection,
                    },
                    reply,
                },
                scene,
                None,
                &[],
                0,
                no_physics(),
            ));
        };
        ask(&mut scene, 8, false);
        let requested = String::from_utf8(sent.lock().unwrap()[0].clone()).unwrap();
        assert!(
            requested.contains("<key>SequenceID</key><integer>8</integer>"),
            "{requested}"
        );
        sent.lock().unwrap().clear();
        arrive_in(&mut scene, agent, [128.0, 128.0, 25.0]);
        assert!(sent.lock().unwrap().is_empty());
        scene.send_parcel_at(agent);
        assert_eq!(sent.lock().unwrap().len(), 1);
        sent.lock().unwrap().clear();
        ask(&mut scene, 0, true);
        let snap = String::from_utf8(sent.lock().unwrap()[0].clone()).unwrap();
        assert!(snap.contains("<key>SequenceID</key><integer />"), "{snap}");
        assert!(
            !snap.contains("<key>SequenceID</key><integer>8</integer>"),
            "{snap}"
        );
    }

    /// Arrival uses the same text bound as a request and a walk (D127).
    #[tokio::test]
    async fn an_oversized_arrival_is_refused() {
        let mut map = region_parcel_map().await;
        let local_id = map.at_metres(128.0, 128.0).unwrap().local_id;
        map.set_description(local_id, "x".repeat(9_000));
        let parcels = Arc::new(map);
        let mut scene = scene_with(&parcels);
        let sent = record_parcels(&mut scene);
        let agent = Uuid::from_u128(21);
        let _ = open_viewer_flags(&mut scene, agent, true, 1 << 7);
        arrive_in(&mut scene, agent, [128.0, 128.0, 25.0]);
        assert!(sent.lock().unwrap().is_empty());
        let _guard = crate::parcels::limits_lock();
        let before = crate::parcels::limit_hits("parcel.text", "refuse");
        scene.send_parcel_at(agent);
        assert!(sent.lock().unwrap().is_empty());
        assert_eq!(
            crate::parcels::limit_hits("parcel.text", "refuse"),
            before + 1
        );
        drop(_guard);
    }

    fn scene_with(parcels: &Arc<crate::parcels::ParcelMap>) -> Scene {
        let mut scene = bare_scene();
        scene.parcels = Some(Arc::clone(parcels));
        scene
    }

    fn record_parcels(scene: &mut Scene) -> Arc<std::sync::Mutex<Vec<Vec<u8>>>> {
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = Arc::clone(&sent);
        scene.parcel_out = Some(Arc::new(move |_agent, element| {
            record.lock().unwrap().push(element.to_vec());
        }));
        sent
    }

    fn arm_initial(scene: &mut Scene, agent: Uuid) {
        scene
            .viewers
            .get(&agent)
            .unwrap()
            .link
            .seeds_sent
            .store(true, Ordering::Release);
        apply(Inbound::HandshakeReply { agent }, scene);
        for _ in 0..4 {
            update_presences(scene);
        }
    }

    async fn region_parcel_map() -> crate::parcels::ParcelMap {
        let copy = crate::test_store::CopiedStore::new("regionstore/objects.db");
        let pool = ferroan_db::Pool::open(
            ferroan_db::Backend::Sqlite,
            &copy.connection(),
            &["RegionStore"],
        )
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../../../fixtures/regionstore/parcels.sql"))
            .execute(pool.sqlite().unwrap())
            .await
            .unwrap();
        pool.close().await;
        let region = uuid::uuid!("11111111-1111-1111-1111-111111111111");
        let estate = uuid::uuid!("4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55");
        crate::parcels::open_parcels_at(
            Some(&copy.connection()),
            region,
            [256, 256],
            estate,
            1_795_674_348,
        )
        .await
        .unwrap()
    }

    async fn loaded_region_parcels() -> Arc<crate::parcels::ParcelMap> {
        let mut map = region_parcel_map().await;
        let height = crate::InitialTerrain::PinheadIsland
            .generate(256, 256)
            .unwrap();
        map.update_geometry(Some(&height));
        assert_eq!(map.at_metres(128.0, 128.0).unwrap().name, "Your Parcel");
        assert_eq!(map.at_metres(120.0, 128.0).unwrap().name, "Your Parcel");
        assert_eq!(map.at_metres(6.0, 2.0).unwrap().name, "West");
        // East is the 4 m cell at x = 8..12. Metre 12 is the next cell.
        assert_eq!(map.at_metres(10.0, 2.0).unwrap().name, "East");
        Arc::new(map)
    }


    /// A Default-flag child gets terrain, the other root, and does not move.
    /// A later login pose does not replace it.
    #[test]
    fn a_view_child_sends_initial_data_and_does_not_move() {
        let mut scene = bare_scene();
        let child = Uuid::from_u128(4);
        let root = Uuid::from_u128(5);
        let start = [-56.0, 128.0, 6.916];
        arrive_in(&mut scene, root, [16.0, 128.0, 25.0]);
        open_viewer_flags(&mut scene, root, true, 1 << 7);
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::ViewChild {
                agent: child,
                pose: pose_at(start),
                owner: Uuid::from_u128(9),
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Pose {
                agent: child,
                pose: pose_at([1.0, 1.0, 1.0]),
                owner: Uuid::from_u128(8),
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        assert_eq!(scene.presences.get(&child).unwrap().pose.position, start);
        let (feed, _) = open_viewer_flags(&mut scene, child, true, 0);
        scene
            .viewers
            .get(&child)
            .unwrap()
            .link
            .seeds_sent
            .store(true, Ordering::Release);
        apply(Inbound::HandshakeReply { agent: child }, &mut scene);
        for _ in 0..3 {
            update_presences(&mut scene);
            assert!(!feed.take_land_pass());
        }
        update_presences(&mut scene);
        assert!(feed.take_land_pass());
        let packets = feed.take_avatar_packets(usize::MAX);
        assert!(
            packets
                .iter()
                .any(|packet| { matches!(packet, AvatarPacket::Full(full) if full.agent == root) })
        );
        for _ in 0..4 {
            update_presences(&mut scene);
            step_avatars(&mut scene.presences, None);
        }
        let avatar = scene.presences.get(&child).unwrap();
        assert_eq!(avatar.pose.position, start);
        assert!(avatar.body.child_agent());
        assert!(!avatar.body.has_actor());
    }

    /// Replacing a live root kills it on the other viewer and keeps the
    /// viewer entry. Seating the same view child again does not kill.
    #[test]
    fn a_view_child_replacing_a_root_kills_it_on_the_other_viewer() {
        let mut scene = bare_scene();
        let leaving = Uuid::from_u128(4);
        let staying = Uuid::from_u128(5);
        let old_owner = Uuid::from_u128(1);
        let new_owner = Uuid::from_u128(2);
        arrive_in(&mut scene, leaving, [200.0, 128.0, 25.0]);
        arrive_in(&mut scene, staying, [16.0, 128.0, 25.0]);
        scene.presences.get_mut(&leaving).unwrap().owner = old_owner;
        let (leaving_feed, _) = open_viewer_flags(&mut scene, leaving, true, 1 << 7);
        let (staying_feed, _) = open_viewer_flags(&mut scene, staying, true, 1 << 7);
        let leaving_id = scene.viewers.get(&leaving).unwrap().local_id;
        let _ = leaving_feed.take_avatar_packets(usize::MAX);
        let _ = staying_feed.take_avatar_packets(usize::MAX);
        let (reply, answer_rx) = oneshot::channel();
        assert!(answer(
            Request::ViewChild {
                agent: leaving,
                pose: pose_at([-56.0, 128.0, 6.916]),
                owner: new_owner,
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        assert_eq!(
            answer_rx.blocking_recv().unwrap().position,
            [-56.0, 128.0, 6.916]
        );
        assert_eq!(packet_kinds(&staying_feed), [('K', u128::from(leaving_id))]);
        assert!(packet_kinds(&leaving_feed).is_empty());
        assert_eq!(scene.viewers.get(&leaving).unwrap().local_id, leaving_id);
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::Remove {
                agent: leaving,
                owner: Some(old_owner),
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        let _ = staying_feed.take_avatar_packets(usize::MAX);
        let (reply, _answer) = oneshot::channel();
        assert!(answer(
            Request::ViewChild {
                agent: leaving,
                pose: pose_at([-56.0, 128.0, 6.916]),
                owner: new_owner,
                reply,
            },
            &mut scene,
            None,
            &[],
            0,
            no_physics(),
        ));
        assert!(packet_kinds(&staying_feed).is_empty());
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
                owner: None,
                reply,
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
        // Its own animations, from `CompleteMovement`.
        assert_eq!(packet_kinds(&feed), [('M', coming.as_u128())]);
        scene.send_initial_data(coming);
        assert_eq!(
            packet_kinds(&feed),
            [
                ('F', there.as_u128()),
                ('A', there.as_u128()),
                ('M', there.as_u128())
            ]
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
                owner: None,
                reply,
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

    /// `CopyTo` reads the live animator: the heartbeat walks an arrival
    /// whose actor touched the ground after its first forward update.
    /// `MakeChildAgent` resets the set, and a return's `CopyFrom` replaces it.
    #[test]
    fn the_departure_carries_the_live_animator_and_a_return_copies_it() {
        use crate::animation::{
            Anim, AnimationTransfer, MOTION_FLYING, MOTION_ON_SURFACE, STAND, default_animation,
        };
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(23);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, agent);
        let depart = |frame: &TerrainFrame| {
            frame
                .departure(agent)
                .unwrap()
                .blocking_wait()
                .unwrap()
                .unwrap()
                .animations
        };
        let standing = depart(&frame);
        assert_eq!(standing.default_animation.unwrap().animation, STAND);
        assert_eq!(standing.default_animation.unwrap().sequence, 1);
        assert_eq!(standing.motion_state, MOTION_ON_SURFACE);

        frame
            .agent_update(agent, walking_update(1_000, 1, CONTROL_AT_POS))
            .unwrap();
        wait_frames(&frame, 4);
        let walking = depart(&frame);
        let walk = walking.default_animation.unwrap();
        assert_eq!(walk.animation, default_animation("WALK").unwrap());
        assert_eq!(walk.object_id, agent);
        assert_eq!(walking.animation_state, Some(walk));
        assert!(walking.animations.is_empty());
        assert_eq!(walking.motion_state, MOTION_ON_SURFACE);

        frame.set_child(agent, true).unwrap();
        frame
            .moved_away(
                agent,
                MovedAway {
                    near: true,
                    known_by: Vec::new(),
                },
            )
            .unwrap()
            .blocking_wait()
            .unwrap();
        let reset = depart(&frame);
        assert_eq!(reset.default_animation.unwrap().animation, STAND);
        assert_eq!(reset.default_animation.unwrap().sequence, 1);

        let fly = default_animation("FLY").unwrap();
        let extra = Anim {
            animation: Uuid::from_u128(500),
            sequence: 41,
            object_id: Uuid::from_u128(501),
        };
        let mut back = standing_return([128.0, 128.0, 40.0]);
        back.data.animations = AnimationTransfer {
            animations: vec![extra],
            default_animation: Some(Anim {
                animation: fly,
                sequence: 40,
                object_id: Uuid::from_u128(502),
            }),
            animation_state: None,
            motion_state: MOTION_FLYING,
        };
        complete(&frame, agent, back);
        let copied = depart(&frame);
        let expected = Anim {
            animation: fly,
            sequence: 40,
            object_id: Uuid::nil(),
        };
        assert_eq!(copied.default_animation, Some(expected));
        assert_eq!(copied.animation_state, Some(expected));
        assert_eq!(copied.animations, [extra]);
        assert_eq!(copied.motion_state, MOTION_FLYING);
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

    /// `TeleportWithMomentum` stands a ground sitter up (`StandUp(false)`,
    /// `ScenePresence.cs` lines 1867–1868) before it moves the avatar, so the
    /// arrival plays `STAND`, not the ground sit.
    #[test]
    fn a_local_teleport_stands_a_ground_sitter_up() {
        use crate::animation::{STAND, default_animation};
        use crate::motion::CONTROL_SIT_ON_GROUND;
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(24);
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, agent);
        frame
            .agent_update(agent, walking_update(1_000, 1, CONTROL_SIT_ON_GROUND))
            .unwrap();
        let default = |frame: &TerrainFrame| {
            frame
                .departure(agent)
                .unwrap()
                .blocking_wait()
                .unwrap()
                .unwrap()
                .animations
                .default_animation
                .unwrap()
                .animation
        };
        assert!(motion(&frame, agent).sit_ground);
        assert_eq!(
            default(&frame),
            default_animation("SIT_GROUND_CONSTRAINED").unwrap()
        );
        let moved = teleport(&frame, agent, local([40.0, 40.0, 40.0], [1.0, 0.0, 0.0]));
        assert_eq!(moved.flags, 16);
        assert!(!motion(&frame, agent).sit_ground);
        assert_eq!(default(&frame), STAND);
    }

    /// The `AvatarAnimation` sets a feed holds, dropping the rest.
    fn animations_in(feed: &ViewerFeed) -> Vec<(Uuid, Vec<Anim>)> {
        feed.take_avatar_packets(usize::MAX)
            .into_iter()
            .filter_map(|packet| match packet {
                AvatarPacket::Animations { agent, animations } => Some((agent, animations)),
                _ => None,
            })
            .collect()
    }

    /// Wait until `feed` holds an animation set, then take them all.
    fn next_animations(feed: &ViewerFeed) -> Vec<(Uuid, Vec<Anim>)> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let seen = animations_in(feed);
            if !seen.is_empty() {
                return seen;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no animation pack arrived"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A closed viewer link: attached, but its `UseCircuitCode` has not come.
    fn attach_closed(frame: &TerrainFrame, agent: Uuid) -> Arc<ViewerFeed> {
        let feed = frame.new_feed();
        let link = ViewerLink {
            feed: Arc::clone(&feed),
            seeds_sent: Arc::new(AtomicBool::new(false)),
            circuit_open: Arc::new(AtomicBool::new(false)),
            look: look_of("closed"),
            teleport_flags: 1 << 7,
        };
        frame.attach(agent, link).unwrap().blocking_wait().unwrap();
        feed
    }

    /// `SendAnimPack` (`ForEachScenePresence`): a root's new movement state
    /// goes to its own viewer and every other open one, children included,
    /// as soon as the update that changed it applies; the release stands on
    /// the heartbeat with the next number. A child's animator sends nothing,
    /// and nothing goes to a viewer whose circuit is not open.
    #[test]
    fn a_walk_and_its_stop_reach_every_open_viewer_children_included() {
        use crate::animation::{STAND, default_animation};
        let frame = TerrainFrame::start_without_map().unwrap();
        let walker = Uuid::from_u128(110);
        let watcher = Uuid::from_u128(111);
        let child = Uuid::from_u128(112);
        let (_, walker_feed, _) = attach(&frame, walker);
        let (_, watcher_feed, _) = attach(&frame, watcher);
        let (_, child_feed, _) = attach(&frame, child);
        let closed_feed = attach_closed(&frame, Uuid::from_u128(113));
        for agent in [walker, watcher, child] {
            login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
            reuse(&frame, agent);
        }
        frame.set_child(child, true).unwrap();
        wait_frames(&frame, 3);
        let feeds = [&walker_feed, &watcher_feed, &child_feed];
        for feed in feeds {
            let _ = feed.take_avatar_packets(usize::MAX);
        }
        frame
            .agent_update(walker, walking_update(1_000, 1, CONTROL_AT_POS))
            .unwrap();
        // The answer comes after the post has applied, and so sent.
        let _ = frames(&frame);
        let walk = {
            let seen = animations_in(&walker_feed);
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].0, walker);
            assert_eq!(seen[0].1.len(), 1);
            seen[0].1[0]
        };
        assert_eq!(walk.animation, default_animation("WALK").unwrap());
        assert_eq!(walk.object_id, walker);
        for feed in [&watcher_feed, &child_feed] {
            assert_eq!(animations_in(feed), [(walker, vec![walk])]);
        }
        frame
            .agent_update(walker, walking_update(1_100, 2, 0))
            .unwrap();
        let stand = Anim {
            animation: STAND,
            sequence: walk.sequence + 1,
            object_id: walker,
        };
        assert_eq!(next_animations(&walker_feed), [(walker, vec![stand])]);
        for feed in [&watcher_feed, &child_feed] {
            assert_eq!(animations_in(feed), [(walker, vec![stand])]);
        }
        frame
            .agent_update(child, walking_update(1_000, 1, CONTROL_AT_POS))
            .unwrap();
        wait_frames(&frame, 3);
        for feed in feeds {
            assert!(animations_in(feed).is_empty());
        }
        assert!(animations_in(&closed_feed).is_empty());
    }

    /// `HandleAgentAnimation`: one pack per packet to every open viewer, with
    /// the started animation after the default and no object, and one even
    /// when nothing changed. A child is refused: no change and no pack.
    #[test]
    fn a_viewer_animation_packet_sends_one_pack_and_a_child_is_refused() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let dancer = Uuid::from_u128(120);
        let watcher = Uuid::from_u128(121);
        let (_, dancer_feed, _) = attach(&frame, dancer);
        let (_, watcher_feed, _) = attach(&frame, watcher);
        for agent in [dancer, watcher] {
            login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
            reuse(&frame, agent);
        }
        wait_frames(&frame, 3);
        for feed in [&dancer_feed, &watcher_feed] {
            let _ = feed.take_avatar_packets(usize::MAX);
        }
        let dance = Uuid::from_u128(0xda);
        frame.agent_animation(dancer, vec![(dance, true)]).unwrap();
        let _ = frames(&frame);
        let started = animations_in(&dancer_feed);
        assert_eq!(started.len(), 1);
        let set = &started[0].1;
        assert_eq!(set.len(), 2);
        assert_eq!(set[1].animation, dance);
        assert_eq!(set[1].object_id, Uuid::nil());
        assert_eq!(animations_in(&watcher_feed), started);
        frame.agent_animation(dancer, vec![(dance, false)]).unwrap();
        frame.agent_animation(dancer, Vec::new()).unwrap();
        let _ = frames(&frame);
        let default = vec![set[0]];
        let stopped = vec![(dancer, default.clone()), (dancer, default)];
        assert_eq!(animations_in(&dancer_feed), stopped);
        assert_eq!(animations_in(&watcher_feed), stopped);

        frame.set_child(dancer, true).unwrap();
        frame.agent_animation(dancer, vec![(dance, true)]).unwrap();
        let _ = frames(&frame);
        frame.set_child(dancer, false).unwrap();
        frame.agent_animation(dancer, Vec::new()).unwrap();
        let _ = frames(&frame);
        // The refused start left the set as it was.
        assert_eq!(animations_in(&watcher_feed), [(dancer, vec![set[0]])]);
    }

    /// The checked `enableflyslow` reaches every avatar: one already there
    /// and one that arrives later.
    #[test]
    fn the_region_fly_slow_setting_reaches_the_avatars() {
        use crate::animation::default_animation;
        let frame = TerrainFrame::start_without_map().unwrap();
        let fly = |agent: Uuid| {
            reuse(&frame, agent);
            frame
                .agent_update(
                    agent,
                    walking_update(1_000, 1, CONTROL_FLY | CONTROL_AT_POS),
                )
                .unwrap();
            frame
                .departure(agent)
                .unwrap()
                .blocking_wait()
                .unwrap()
                .unwrap()
                .animations
                .default_animation
                .unwrap()
                .animation
        };
        let before = Uuid::from_u128(25);
        login(&frame, before, pose_at([128.0, 128.0, 40.0]));
        assert_eq!(fly(before), default_animation("FLY").unwrap());
        let early = Uuid::from_u128(26);
        login(&frame, early, pose_at([128.0, 128.0, 40.0]));
        frame.set_fly_slow(true).unwrap().blocking_wait().unwrap();
        let later = Uuid::from_u128(27);
        login(&frame, later, pose_at([128.0, 128.0, 40.0]));
        let slow = default_animation("FLYSLOW").unwrap();
        assert_eq!(fly(early), slow);
        assert_eq!(fly(later), slow);
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
                    animations: crate::animation::AnimationTransfer::default(),
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

    fn armed_gate(position: [f32; 3], draw: f32, view: f32) -> ChildGate {
        ChildGate {
            armed: true,
            update_position: position,
            check_position: position,
            last_draw: draw,
            last_view: view,
            last_ms: 0,
            ..ChildGate::default()
        }
    }

    fn sample_at(now_ms: i64, position: [f32; 3], draw: f32, view: f32, god: bool) -> ChildSample {
        ChildSample {
            now_ms,
            child: false,
            position,
            draw,
            view,
            god,
        }
    }

    #[test]
    fn child_gate_follows_the_pin8_child_half() {
        let here = [128.0, 128.0, 25.0];
        let mut gate = ChildGate::default();
        let sample = sample_at(5_000, here, 255.0, 255.0, false);
        assert_eq!(due(&gate, &sample), ChildDue::None);
        gate = armed_gate(here, 255.0, 255.0);
        gate.busy = true;
        assert_eq!(due(&gate, &sample), ChildDue::None);
        gate.busy = false;
        let mut hidden = sample_at(5_000, here, 255.0, 0.0, false);
        assert_eq!(due(&gate, &hidden), ChildDue::None);
        hidden.view = 255.0;
        hidden.child = true;
        assert_eq!(due(&gate, &hidden), ChildDue::None);
        // Arrival stores now+10s. EnvironmentTickCountSubtract wraps that
        // future value, so a 16 m move still sends AgentPosition. A stamp
        // in the past holds until 2000 ms have passed.
        gate.last_ms = 12_000;
        gate.known_other = true;
        let early = sample_at(5_000, [128.0 + 16.0, 128.0, 25.0], 255.0, 255.0, false);
        assert_eq!(due(&gate, &early), ChildDue::Position);
        gate.known_other = false;
        assert_eq!(due(&gate, &early), ChildDue::None);
        gate.last_ms = 4_000;
        gate.known_other = true;
        let held = sample_at(5_999, [128.0 + 16.0, 128.0, 25.0], 255.0, 255.0, false);
        assert_eq!(due(&gate, &held), ChildDue::None);
        gate.known_other = false;
        // Equality does not trip either distance. A 16 m move is the position
        // arm only, and only once a neighbour is known.
        gate.last_ms = 0;
        let exact = sample_at(
            5_000,
            [128.0 + 32.0, 128.0, 25.0],
            255.0,
            255.0 + 32.0,
            false,
        );
        assert_eq!(due(&gate, &exact), ChildDue::None);
        let sixteen = sample_at(5_000, [128.0 + 16.0, 128.0, 25.0], 255.0, 255.0, false);
        assert_eq!(due(&gate, &sixteen), ChildDue::None);
        gate.known_other = true;
        assert_eq!(due(&gate, &sixteen), ChildDue::Position);
        gate.known_other = false;
        let enable = sample_at(5_000, [128.0 + 33.0, 128.0, 25.0], 255.0, 255.0, false);
        assert_eq!(due(&gate, &enable), ChildDue::Enable);
        let view = sample_at(5_000, here, 255.0, 255.0 + 32.1, false);
        assert_eq!(due(&gate, &view), ChildDue::Enable);
        gate.known_other = true;
        let draw = sample_at(5_000, here, 255.0 + 32.1, 255.0, false);
        assert_eq!(due(&gate, &draw), ChildDue::Position);
        let god = sample_at(5_000, here, 255.0, 255.0, true);
        assert_eq!(due(&gate, &god), ChildDue::Position);
        let lifted = sample_at(5_000, [128.0, 128.0, 25.0 + 11.0], 255.0, 255.0, false);
        assert_eq!(due(&gate, &lifted), ChildDue::Position);
        gate.known_other = false;
        assert_eq!(due(&gate, &draw), ChildDue::None);
        assert_eq!(due(&gate, &god), ChildDue::None);
        assert_eq!(due(&gate, &lifted), ChildDue::None);
    }

    fn wait_enable(rx: &mut tokio::sync::mpsc::Receiver<ChildNotice>, agent: Uuid) -> ChildNotice {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(notice) = rx.try_recv()
                && notice.agent == agent
                && notice.pass == ChildPass::Enable
            {
                return notice;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "SendInitialData did not arm an enable"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A login and a hop return each arm one enable, on the `SendInitialData`
    /// frame. `CompleteMovement` and the arrival sample do not arm one.
    #[test]
    fn send_initial_data_arms_one_enable_per_arrival() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        frame.watch_children(tx);
        let agent = Uuid::from_u128(81);
        let (_, _, seeds) = attach(&frame, agent);
        seeds.store(true, Ordering::Release);
        login(&frame, agent, pose_at([16.0, 128.0, 30.0]));
        let _ = frame
            .arrive(agent, pose_at([16.0, 128.0, 30.0]), EAST)
            .unwrap()
            .blocking_wait()
            .unwrap();
        std::thread::sleep(Duration::from_millis(250));
        assert!(
            rx.try_recv().is_err(),
            "arrival armed before SendInitialData"
        );
        frame.handshake_reply(agent).unwrap();
        let first = wait_enable(&mut rx, agent);
        assert!(!first.settle);
        std::thread::sleep(Duration::from_millis(300));
        assert!(rx.try_recv().is_err(), "a login armed a second enable");

        frame.set_child(agent, true).unwrap();
        let _ = complete(&frame, agent, standing_return([20.0, 128.0, 25.0]));
        let _ = frame
            .arrive(agent, pose_at([20.0, 128.0, 25.0]), EAST)
            .unwrap()
            .blocking_wait()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            rx.try_recv().is_err(),
            "a hop return armed before SendInitialData"
        );
        frame.handshake_reply(agent).unwrap();
        let second = wait_enable(&mut rx, agent);
        assert_eq!(second.pass, ChildPass::Enable);
        assert!(!second.settle);
        std::thread::sleep(Duration::from_millis(300));
        assert!(rx.try_recv().is_err(), "a hop return armed twice");
    }

    #[test]
    fn arrival_offers_enable_without_blocking_when_the_channel_is_full() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(ChildNotice {
            agent: Uuid::nil(),
            pass: ChildPass::Enable,
            position: [0.0, 0.0, 0.0],
            velocity: [0.0, 0.0, 0.0],
            camera: [0.0, 0.0, 0.0],
            draw_distance: 0.0,
            height: 1.9,
            god: false,
            settle: false,
            epoch: 0,
        })
        .unwrap();
        frame.watch_children(tx);
        let agent = Uuid::from_u128(79);
        let (_, _, seeds) = attach(&frame, agent);
        seeds.store(true, Ordering::Release);
        let started = std::time::Instant::now();
        let stored = frame
            .arrive(agent, pose_at([20.0, 30.0, 40.0]), EAST)
            .unwrap()
            .blocking_wait()
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(stored.position, [20.0, 30.0, 40.0]);
        frame.handshake_reply(agent).unwrap();
        // SendInitialData is several frames after the reply. Leave the filler
        // in the queue until that offer is refused, or the retry succeeds
        // into the slot the filler occupied and nothing is reported.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !ferroan_diag::metrics().contains("region.child-notices") {
            assert!(
                std::time::Instant::now() < deadline,
                "a full child-notice queue was not reported"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let filler = rx.try_recv().unwrap();
        assert!(filler.agent.is_nil());
        let notice = loop {
            if let Ok(notice) = rx.try_recv()
                && notice.agent == agent
            {
                break notice;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "arrival enable was not retried"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(notice.pass, ChildPass::Enable);
        assert!(!notice.settle);
        // The scene keeps stepping while the channel is full, so this notice
        // carries the pose at the retry. The arrival answer above is the login
        // position. Nothing walks, so the horizontal position stays.
        assert_eq!(notice.position[0], 20.0);
        assert_eq!(notice.position[1], 30.0);
        assert!(notice.position[2].is_finite());
    }
}
