// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The terrain frame thread. [D6](../../../docs/DECISIONS.md) keeps the region
//! frame off the async runtime. This row has no physics, so the thread only
//! serves the heightmap and the precomputed land packets.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use crate::compress::{self, TerrainLayer};
use crate::error::TerrainError;
use crate::height::{Heightmap, movement_z};

enum Request {
    Height { x: f32, y: f32, reply: Sender<f32> },
    Layers { reply: Sender<Vec<TerrainLayer>> },
    Stop,
}

/// Owns the heightmap on `ferroan-scene` and answers login queries.
pub struct TerrainFrame {
    tx: Sender<Request>,
    thread: Option<JoinHandle<()>>,
}

impl TerrainFrame {
    /// Precomputes land packets on the frame thread, then returns.
    pub fn start(map: Heightmap) -> Result<Self, TerrainError> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("ferroan-scene".to_owned())
            .spawn(move || run(map, rx, ready_tx))
            .map_err(|_| TerrainError::Thread)?;
        ready_rx.recv().map_err(|_| TerrainError::Thread)?;
        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }

    pub fn ground(&self, x: f32, y: f32) -> Result<f32, TerrainError> {
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
}

impl Drop for TerrainFrame {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(map: Heightmap, rx: Receiver<Request>, ready: Sender<()>) {
    let layers = compress::land_layers(&map);
    if ready.send(()).is_err() {
        return;
    }
    while let Ok(request) = rx.recv() {
        match request {
            Request::Height { x, y, reply } => {
                let _ = reply.send(map.height_at(x, y));
            }
            Request::Layers { reply } => {
                let _ = reply.send(layers.clone());
            }
            Request::Stop => break,
        }
    }
}

/// Login Z from `MakeRootAgent` once the frame has reported the ground height.
pub fn landed_z(start_z: f32, ground: f32) -> f32 {
    movement_z(start_z, ground)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::height::Heightmap;

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
}
