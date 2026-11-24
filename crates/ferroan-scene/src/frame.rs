// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The region frame thread. [D6](../../../docs/DECISIONS.md) keeps it off the
//! async runtime. It ticks at the shipped `FrameTime` and still answers height
//! and layer queries. Each tick steps every avatar that is not sitting on the
//! ground. Login, chat, a local teleport and a hop completion store the pose.
//! An `AgentUpdate` stores movement intent. The reply from those calls is the
//! pose written in that call, before the next tick. Teleport packets stay on
//! the circuit.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ferroan_diag::{Kind, REGION_SCENE_INBOUND, refuse};
use uuid::Uuid;

use crate::compress::{self, TerrainLayer};
use crate::error::TerrainError;
use crate::height::{DEFAULT_HEIGHT, Heightmap, movement_z};
use crate::motion::{Applied, Body, Motion, MotionUpdate};
use crate::walk;

/// `Constants.MinSimulationHeight`.
const MIN_HEIGHT: f32 = -100.0;
/// `Constants.MaxSimulationHeight`.
const MAX_HEIGHT: f32 = 50_000.0;
/// Substituted when a local teleport offset is outside the region.
const EMERGENCY: [f32; 3] = [128.0, 128.0, 128.0];

/// A same-region teleport the circuit already accepted. The frame applies the
/// landing and stores the pose. The circuit sends the packets.
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
#[derive(Clone, Copy, Debug)]
pub struct TeleportStored {
    pub pose: Presence,
    pub look_at: [f32; 3],
    pub flags: u32,
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

enum Request {
    Height {
        x: f32,
        y: f32,
        reply: Sender<f32>,
    },
    Layers {
        reply: Sender<Vec<TerrainLayer>>,
    },
    Read {
        agent: Uuid,
        reply: Sender<Option<Presence>>,
    },
    Motion {
        agent: Uuid,
        reply: Sender<Option<Motion>>,
    },
    Frames {
        reply: Sender<u64>,
    },
    Wake,
    Stop,
}

struct PresencePost {
    agent: Uuid,
    pose: Presence,
    reply: Sender<Presence>,
}

struct LandPost {
    agent: Uuid,
    position: [f32; 3],
    reply: Sender<Presence>,
}

struct PlacesPost {
    agents: Vec<Uuid>,
    reply: Sender<HashMap<Uuid, [f32; 3]>>,
}

struct TeleportPost {
    agent: Uuid,
    request: LocalTeleport,
    reply: Sender<TeleportStored>,
}

struct CompletePost {
    agent: Uuid,
    position: [f32; 3],
    look_at: [f32; 3],
    reply: Sender<Presence>,
}

struct AgentPost {
    agent: Uuid,
    update: MotionUpdate,
    reply: Sender<Applied>,
}

struct AlwaysRunPost {
    agent: Uuid,
    always_run: bool,
    reply: Sender<bool>,
}

struct Avatar {
    pose: Presence,
    body: Body,
}

enum Inbound {
    Pose(PresencePost),
    Land(LandPost),
    Places(PlacesPost),
    Teleport(TeleportPost),
    Complete(CompletePost),
    Agent(AgentPost),
    AlwaysRun(AlwaysRunPost),
}

/// Owns the heightmap and the login presences on `ferroan-scene`.
pub struct TerrainFrame {
    tx: Sender<Request>,
    inbound: SyncSender<Inbound>,
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
        let thread = std::thread::Builder::new()
            .name("ferroan-scene".to_owned())
            .spawn(move || run(map, rx, inbound_rx, ready_tx))
            .map_err(|_| TerrainError::Thread)?;
        ready_rx.recv().map_err(|_| TerrainError::Thread)?;
        Ok(Self {
            tx,
            inbound,
            thread: Some(thread),
            has_map,
        })
    }

    pub fn has_map(&self) -> bool {
        self.has_map
    }

    pub fn ground(&self, x: f32, y: f32) -> Result<f32, TerrainError> {
        if !self.has_map {
            return Err(TerrainError::NoMap);
        }
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Request::Height { x, y, reply })
            .map_err(|_| TerrainError::Stopped)?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    pub fn layers(&self) -> Result<Vec<TerrainLayer>, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Request::Layers { reply })
            .map_err(|_| TerrainError::Stopped)?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    /// Store one login pose and return the record the frame thread kept.
    pub fn post_login(&self, agent: Uuid, pose: Presence) -> Result<Presence, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.push(Inbound::Pose(PresencePost { agent, pose, reply }))?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    /// A local or neighbour landing. An existing record keeps its other fields.
    pub fn land(&self, agent: Uuid, position: [f32; 3]) -> Result<Presence, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.push(Inbound::Land(LandPost {
            agent,
            position,
            reply,
        }))?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    /// A forwarded same-region `TeleportLocationRequest`. One inbound slot.
    pub fn teleport_local(
        &self,
        agent: Uuid,
        request: LocalTeleport,
    ) -> Result<TeleportStored, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.push(Inbound::Teleport(TeleportPost {
            agent,
            request,
            reply,
        }))?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    /// A hop completion. Position and heading change. The other fields stay.
    pub fn complete_hop(
        &self,
        agent: Uuid,
        position: [f32; 3],
        look_at: [f32; 3],
    ) -> Result<Presence, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.push(Inbound::Complete(CompletePost {
            agent,
            position,
            look_at,
            reply,
        }))?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    /// One forwarded `AgentUpdate`. A missing avatar is dropped. One inbound slot.
    pub fn agent_update(&self, agent: Uuid, update: MotionUpdate) -> Result<Applied, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.push(Inbound::Agent(AgentPost {
            agent,
            update,
            reply,
        }))?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    /// One forwarded `SetAlwaysRun`. A missing avatar reports false. One inbound slot.
    pub fn set_always_run(&self, agent: Uuid, always_run: bool) -> Result<bool, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.push(Inbound::AlwaysRun(AlwaysRunPost {
            agent,
            always_run,
            reply,
        }))?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    /// The movement intent. This does not take an inbound slot.
    pub fn motion(&self, agent: Uuid) -> Result<Option<Motion>, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Request::Motion { agent, reply })
            .map_err(|_| TerrainError::Stopped)?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    /// Positions for one forwarded `ChatFromViewer`. Missing agents are omitted.
    pub fn chat_positions(&self, agents: &[Uuid]) -> Result<HashMap<Uuid, [f32; 3]>, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.push(Inbound::Places(PlacesPost {
            agents: agents.to_vec(),
            reply,
        }))?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    fn push(&self, post: Inbound) -> Result<(), TerrainError> {
        push_inbound(&self.inbound, post)?;
        // Wake the tick wait so the post is applied before the next frame.
        let _ = self.tx.send(Request::Wake);
        Ok(())
    }

    pub fn presence(&self, agent: Uuid) -> Result<Option<Presence>, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Request::Read { agent, reply })
            .map_err(|_| TerrainError::Stopped)?;
        rx.recv().map_err(|_| TerrainError::Stopped)
    }

    pub fn frames(&self) -> Result<u64, TerrainError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Request::Frames { reply })
            .map_err(|_| TerrainError::Stopped)?;
        rx.recv().map_err(|_| TerrainError::Stopped)
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

fn drain_inbound(
    rx: &Receiver<Inbound>,
    presences: &mut HashMap<Uuid, Avatar>,
    map: Option<&Heightmap>,
) {
    while let Ok(post) = rx.try_recv() {
        match post {
            Inbound::Pose(post) => {
                let avatar = Avatar::new(post.pose);
                let pose = avatar.pose;
                presences.insert(post.agent, avatar);
                let _ = post.reply.send(pose);
            }
            Inbound::Land(post) => {
                let avatar = presences
                    .entry(post.agent)
                    .or_insert_with(|| Avatar::new(fresh(post.position)));
                avatar.pose.position = post.position;
                let _ = post.reply.send(avatar.pose);
            }
            Inbound::Places(post) => {
                let mut found = HashMap::new();
                for agent in post.agents {
                    if let Some(avatar) = presences.get(&agent) {
                        found.insert(agent, avatar.pose.position);
                    }
                }
                let _ = post.reply.send(found);
            }
            Inbound::Teleport(post) => {
                let position = place_teleport(map, &post.request);
                let look_at = flatten_look(post.request.look_at);
                let avatar = presences
                    .entry(post.agent)
                    .or_insert_with(|| Avatar::new(fresh(position)));
                avatar.pose.position = position;
                avatar.pose.rotation = look_rotation(look_at);
                avatar.pose.velocity = [0.0, 0.0, 0.0];
                avatar.body.halt_command();
                let _ = post.reply.send(TeleportStored {
                    pose: avatar.pose,
                    look_at,
                    flags: post.request.flags,
                });
            }
            Inbound::Complete(post) => {
                let avatar = presences
                    .entry(post.agent)
                    .or_insert_with(|| Avatar::new(fresh(post.position)));
                avatar.pose.position = post.position;
                avatar.pose.rotation = look_rotation(post.look_at);
                let _ = post.reply.send(avatar.pose);
            }
            Inbound::Agent(post) => {
                let applied = if let Some(avatar) = presences.get_mut(&post.agent) {
                    avatar.body.apply(&post.update, &mut avatar.pose)
                } else {
                    Applied {
                        dropped: true,
                        movement: false,
                        camera: false,
                    }
                };
                let _ = post.reply.send(applied);
            }
            Inbound::AlwaysRun(post) => {
                let applied = if let Some(avatar) = presences.get_mut(&post.agent) {
                    avatar.body.set_always_run(post.always_run);
                    true
                } else {
                    false
                };
                let _ = post.reply.send(applied);
            }
        }
    }
}

impl Avatar {
    fn new(pose: Presence) -> Self {
        Self {
            body: Body::new(&pose),
            pose,
        }
    }
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
        drain_inbound(&inbound, &mut presences, map.as_ref());
        let wait = next.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            step_avatars(&mut presences, map.as_ref());
            frames = frames.saturating_add(1);
            next = Instant::now() + frame_wait();
            continue;
        }
        match rx.recv_timeout(wait) {
            Ok(Request::Height { x, y, reply }) => {
                let height = map.as_ref().map(|map| map.height_at(x, y)).unwrap_or(0.0);
                let _ = reply.send(height);
            }
            Ok(Request::Layers { reply }) => {
                let _ = reply.send(layers.clone());
            }
            Ok(Request::Read { agent, reply }) => {
                let _ = reply.send(presences.get(&agent).map(|avatar| avatar.pose));
            }
            Ok(Request::Motion { agent, reply }) => {
                let _ = reply.send(presences.get(&agent).map(|avatar| avatar.body.view()));
            }
            Ok(Request::Frames { reply }) => {
                let _ = reply.send(frames);
            }
            Ok(Request::Wake) => {}
            Ok(Request::Stop) => break,
            Err(RecvTimeoutError::Timeout) => {
                step_avatars(&mut presences, map.as_ref());
                frames = frames.saturating_add(1);
                next = Instant::now() + frame_wait();
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Login Z from `MakeRootAgent` once the frame has reported the ground height.
pub fn landed_z(start_z: f32, ground: f32) -> f32 {
    movement_z(start_z, ground)
}

fn step_avatars(presences: &mut HashMap<Uuid, Avatar>, map: Option<&Heightmap>) {
    let now = unix_ms();
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

fn unix_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::height::Heightmap;
    use crate::walk::AVATAR_SIZE_Z;
    use uuid::Uuid;

    fn z_is_written_or_snapped(z: f32, written: f32) -> bool {
        z == written || z == AVATAR_SIZE_Z
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
        let ground = frame.ground(4.0, 4.0).unwrap();
        assert_eq!(ground, 21.0);
        assert!((landed_z(10.0, ground) - (21.0 + 0.01 + 0.8)).abs() < f32::EPSILON);
        assert_eq!(frame.layers().unwrap().len(), 1);
    }

    #[test]
    fn the_frame_ticks_and_stores_the_login_pose() {
        let frame = TerrainFrame::start_without_map().unwrap();
        assert!(!frame.has_map());
        assert!(matches!(frame.ground(1.0, 1.0), Err(TerrainError::NoMap)));
        let agent = Uuid::from_u128(7);
        let pose = Presence {
            position: [4.0, 5.0, 26.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0, 0.0, 0.0],
            control_flags: 0,
            draw_distance: 255.0,
        };
        assert_eq!(frame.post_login(agent, pose).unwrap(), pose);
        let immediate = frame.presence(agent).unwrap().unwrap();
        assert_eq!(immediate.position[0], 4.0);
        assert_eq!(immediate.position[1], 5.0);
        assert!(z_is_written_or_snapped(immediate.position[2], 26.0));
        let before = frame.frames().unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let frames = frame.frames().unwrap();
        assert!(frames > before, "frames {before} -> {frames}");
        assert!(frames < 10, "frames {frames}");
        assert_eq!(FRAME_TIME, 0.0909);
        let snapped = frame.presence(agent).unwrap().unwrap();
        assert_eq!(snapped.position, [4.0, 5.0, AVATAR_SIZE_Z]);
        assert_eq!(snapped.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(snapped.rotation, pose.rotation);
    }

    #[test]
    fn a_full_scene_inbound_channel_reports_the_limit() {
        let (tx, _rx) = mpsc::sync_channel(SCENE_INBOUND);
        let pose = Presence {
            position: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0, 0.0, 0.0],
            control_flags: 0,
            draw_distance: 255.0,
        };
        for index in 0..SCENE_INBOUND {
            let (reply, _wait) = mpsc::channel();
            push_inbound(
                &tx,
                Inbound::Pose(PresencePost {
                    agent: Uuid::from_u128(index as u128),
                    pose,
                    reply,
                }),
            )
            .unwrap();
        }
        let (reply, _wait) = mpsc::channel();
        let error = push_inbound(
            &tx,
            Inbound::Pose(PresencePost {
                agent: Uuid::from_u128(999),
                pose,
                reply,
            }),
        )
        .unwrap_err();
        assert!(matches!(error, TerrainError::InboundFull));
        assert!(ferroan_diag::metrics().contains("region.scene-inbound"));
    }

    #[test]
    fn a_landing_is_the_position_chat_reads() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let speaker = Uuid::from_u128(1);
        let listener = Uuid::from_u128(2);
        let pose = Presence {
            position: [128.0, 128.0, 25.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [1.0, 0.0, 0.0],
            control_flags: 4,
            draw_distance: 255.0,
        };
        frame.post_login(speaker, pose).unwrap();
        frame
            .post_login(
                listener,
                Presence {
                    position: [130.0, 128.0, 25.0],
                    ..pose
                },
            )
            .unwrap();
        let landed = frame.land(speaker, [40.0, 200.0, 30.0]).unwrap();
        assert_eq!(landed.position, [40.0, 200.0, 30.0]);
        assert_eq!(landed.rotation, pose.rotation);
        assert_eq!(landed.velocity, pose.velocity);
        assert_eq!(landed.control_flags, pose.control_flags);
        assert_eq!(landed.draw_distance, pose.draw_distance);
        let places = frame.chat_positions(&[speaker, listener]).unwrap();
        let speaker_at = places.get(&speaker).copied().unwrap();
        let listener_at = places.get(&listener).copied().unwrap();
        assert_eq!(speaker_at[0], 40.0);
        assert_eq!(speaker_at[1], 200.0);
        assert!(z_is_written_or_snapped(speaker_at[2], 30.0));
        assert_eq!(listener_at[0], 130.0);
        assert_eq!(listener_at[1], 128.0);
        assert!(z_is_written_or_snapped(listener_at[2], 25.0));
        assert!(
            frame
                .chat_positions(&[Uuid::from_u128(9)])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_local_teleport_and_a_hop_completion_write_the_pose() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(3);
        frame
            .post_login(
                agent,
                Presence {
                    position: [128.0, 128.0, 25.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    velocity: [1.0, 0.0, 0.0],
                    control_flags: 4,
                    draw_distance: 64.0,
                },
            )
            .unwrap();
        let moved = frame
            .teleport_local(
                agent,
                LocalTeleport {
                    position: [40.0, 200.0, 30.0],
                    look_at: [0.0, 0.0, 1.0],
                    half_height: 0.95,
                    default_landing: [128.0, 128.0, 30.0],
                    size_x: 256.0,
                    size_y: 256.0,
                    flags: 16,
                },
            )
            .unwrap();
        assert_eq!(moved.pose.position, [40.0, 200.0, 30.0]);
        assert_eq!(moved.look_at, [1.0, 0.0, 0.0]);
        assert_eq!(moved.flags, 16);
        assert_eq!(moved.pose.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(moved.pose.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(moved.pose.control_flags, 4);
        assert_eq!(moved.pose.draw_distance, 64.0);
        let lifted = frame
            .teleport_local(
                agent,
                LocalTeleport {
                    position: [10.0, 10.0, 1.0],
                    look_at: [1.0, 0.0, 0.0],
                    half_height: 0.95,
                    default_landing: [128.0, 128.0, 30.0],
                    size_x: 256.0,
                    size_y: 256.0,
                    flags: 16,
                },
            )
            .unwrap();
        let ground = 21.0 + 0.95 + 0.1;
        assert!((lifted.pose.position[2] - ground).abs() < 0.001);
        assert_eq!(lifted.pose.position[0], 10.0);
        let emergency = frame
            .teleport_local(
                agent,
                LocalTeleport {
                    position: [-1.0, 10.0, 30.0],
                    look_at: [1.0, 0.0, 0.0],
                    half_height: 0.95,
                    default_landing: [128.0, 128.0, 30.0],
                    size_x: 256.0,
                    size_y: 256.0,
                    flags: 16,
                },
            )
            .unwrap();
        assert_eq!(emergency.pose.position, [128.0, 128.0, 128.0]);
        let sentinel = frame
            .teleport_local(
                agent,
                LocalTeleport {
                    position: [128.0, 128.0, 22.5],
                    look_at: [0.0, 0.0, 4.0],
                    half_height: 0.95,
                    default_landing: [128.0, 128.0, 30.0],
                    size_x: 256.0,
                    size_y: 256.0,
                    flags: 16,
                },
            )
            .unwrap();
        assert_eq!(sentinel.pose.position, [128.0, 128.0, 30.0]);
        assert_eq!(sentinel.look_at, [1.0, 0.0, 0.0]);
        let yaw = frame
            .teleport_local(
                agent,
                LocalTeleport {
                    position: [50.0, 50.0, 40.0],
                    look_at: [0.0, 1.0, 0.0],
                    half_height: 0.95,
                    default_landing: [128.0, 128.0, 30.0],
                    size_x: 256.0,
                    size_y: 256.0,
                    flags: 16,
                },
            )
            .unwrap();
        assert_eq!(yaw.pose.position, [50.0, 50.0, 40.0]);
        let quarter = std::f32::consts::FRAC_1_SQRT_2;
        assert!((yaw.pose.rotation[2] - quarter).abs() < 1e-5);
        assert!((yaw.pose.rotation[3] - quarter).abs() < 1e-5);
        assert_eq!(yaw.pose.rotation[0], 0.0);
        assert_eq!(yaw.pose.rotation[1], 0.0);
        let traveller = Uuid::from_u128(4);
        frame
            .post_login(
                traveller,
                Presence {
                    position: [8.0, 8.0, 22.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    velocity: [2.0, 0.0, 0.0],
                    control_flags: 7,
                    draw_distance: 80.0,
                },
            )
            .unwrap();
        let completed = frame
            .complete_hop(traveller, [128.0, 128.0, 30.0], [1.0, 0.0, 0.0])
            .unwrap();
        assert_eq!(completed.position, [128.0, 128.0, 30.0]);
        assert!(
            completed.velocity == [2.0, 0.0, 0.0] || completed.velocity == [0.0, 0.0, 0.0],
            "{:?}",
            completed.velocity
        );
        assert_eq!(completed.control_flags, 7);
        assert_eq!(completed.draw_distance, 80.0);
        let completed = frame
            .complete_hop(agent, [128.0, 128.0, 30.0], [1.0, 0.0, 0.0])
            .unwrap();
        assert_eq!(completed.position, [128.0, 128.0, 30.0]);
        assert_eq!(completed.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(completed.control_flags, 4);
        assert_eq!(completed.draw_distance, 64.0);
        let chat = frame
            .chat_positions(&[agent])
            .unwrap()
            .get(&agent)
            .copied()
            .unwrap();
        assert_eq!(chat[0], 128.0);
        assert_eq!(chat[1], 128.0);
        assert!(z_is_written_or_snapped(chat[2], 30.0));
    }

    #[test]
    fn an_agent_update_is_stored_on_the_frame_and_a_teleport_stops_the_command() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(11);
        frame
            .post_login(
                agent,
                Presence {
                    position: [128.0, 128.0, 25.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    velocity: [0.0, 0.0, 0.0],
                    control_flags: 0,
                    draw_distance: 255.0,
                },
            )
            .unwrap();
        assert!(frame.motion(Uuid::from_u128(99)).unwrap().is_none());
        let applied = frame
            .agent_update(
                agent,
                crate::motion::MotionUpdate {
                    now_ms: 1_000,
                    sequence: 1,
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
                    control_flags: crate::motion::CONTROL_AT_POS,
                    flags: 0,
                },
            )
            .unwrap();
        assert!(applied.movement);
        let motion = frame.motion(agent).unwrap().unwrap();
        assert!(motion.target_velocity[0] > 0.0);
        assert_eq!(frame.presence(agent).unwrap().unwrap().draw_distance, 96.0);
        assert!(frame.set_always_run(agent, true).unwrap());
        assert!(frame.motion(agent).unwrap().unwrap().always_run);
        assert!(!frame.set_always_run(Uuid::from_u128(99), true).unwrap());
        let dropped = frame
            .agent_update(
                Uuid::from_u128(99),
                crate::motion::MotionUpdate {
                    now_ms: 1_100,
                    sequence: 1,
                    child: false,
                    in_transit: false,
                    body_rotation: [0.0, 0.0, 0.0, 1.0],
                    head_rotation: [0.0, 0.0, 0.0, 1.0],
                    state: 0,
                    camera_center: [0.0, 0.0, 0.0],
                    camera_at: [1.0, 0.0, 0.0],
                    camera_left: [0.0, 1.0, 0.0],
                    camera_up: [0.0, 0.0, 1.0],
                    far: 32.0,
                    control_flags: 0,
                    flags: 0,
                },
            )
            .unwrap();
        assert!(dropped.dropped);
        frame
            .teleport_local(
                agent,
                LocalTeleport {
                    position: [40.0, 200.0, 30.0],
                    look_at: [1.0, 0.0, 0.0],
                    half_height: 0.95,
                    default_landing: [128.0, 128.0, 30.0],
                    size_x: 256.0,
                    size_y: 256.0,
                    flags: 16,
                },
            )
            .unwrap();
        let stopped = frame.motion(agent).unwrap().unwrap();
        assert_eq!(stopped.target_velocity, [0.0, 0.0, 0.0]);
        assert!(stopped.always_run);
        assert_eq!(
            frame.presence(agent).unwrap().unwrap().velocity,
            [0.0, 0.0, 0.0]
        );
    }
}
