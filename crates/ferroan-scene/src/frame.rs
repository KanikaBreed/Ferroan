// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The region frame thread. [D6](../../../docs/DECISIONS.md) keeps it off the
//! async runtime. It ticks at the shipped `FrameTime` and still answers height
//! and layer queries. This row has no physics. The only pose it stores is the
//! login arrival. Chat and teleport keep their own positions.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ferroan_diag::{Kind, REGION_SCENE_INBOUND, refuse};
use uuid::Uuid;

use crate::compress::{self, TerrainLayer};
use crate::error::TerrainError;
use crate::height::{Heightmap, movement_z};

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

/// Owns the heightmap and the login presences on `ferroan-scene`.
pub struct TerrainFrame {
    tx: Sender<Request>,
    inbound: SyncSender<PresencePost>,
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
        push_inbound(&self.inbound, PresencePost { agent, pose, reply })?;
        // Wake the tick wait so the pose is stored before the next frame.
        let _ = self.tx.send(Request::Wake);
        rx.recv().map_err(|_| TerrainError::Stopped)
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

fn push_inbound(tx: &SyncSender<PresencePost>, post: PresencePost) -> Result<(), TerrainError> {
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

fn drain_inbound(rx: &Receiver<PresencePost>, presences: &mut HashMap<Uuid, Presence>) {
    while let Ok(post) = rx.try_recv() {
        presences.insert(post.agent, post.pose);
        let _ = post.reply.send(post.pose);
    }
}

fn run(
    map: Option<Heightmap>,
    rx: Receiver<Request>,
    inbound: Receiver<PresencePost>,
    ready: Sender<()>,
) {
    let layers = map.as_ref().map(compress::land_layers).unwrap_or_default();
    if ready.send(()).is_err() {
        return;
    }
    let mut presences = HashMap::new();
    let mut frames = 0u64;
    let mut next = Instant::now() + frame_wait();
    loop {
        drain_inbound(&inbound, &mut presences);
        let wait = next.saturating_duration_since(Instant::now());
        if wait.is_zero() {
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
                let _ = reply.send(presences.get(&agent).copied());
            }
            Ok(Request::Frames { reply }) => {
                let _ = reply.send(frames);
            }
            Ok(Request::Wake) => {}
            Ok(Request::Stop) => break,
            Err(RecvTimeoutError::Timeout) => {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::height::Heightmap;
    use uuid::Uuid;

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
        assert_eq!(frame.presence(agent).unwrap(), Some(pose));
        std::thread::sleep(Duration::from_millis(400));
        let frames = frame.frames().unwrap();
        assert!(frames >= 2, "frames {frames}");
        assert!(frames < 10, "frames {frames}");
        assert_eq!(FRAME_TIME, 0.0909);
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
                PresencePost {
                    agent: Uuid::from_u128(index as u128),
                    pose,
                    reply,
                },
            )
            .unwrap();
        }
        let (reply, _wait) = mpsc::channel();
        let error = push_inbound(
            &tx,
            PresencePost {
                agent: Uuid::from_u128(999),
                pose,
                reply,
            },
        )
        .unwrap_err();
        assert!(matches!(error, TerrainError::InboundFull));
        assert!(ferroan_diag::metrics().contains("region.scene-inbound"));
    }
}
