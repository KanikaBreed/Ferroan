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
// Ported to Rust for Ferroan from TerrainData, OpenSimTerrainCompressor,
// LLClientView.SendLayerData and OpenMetaverse.BitPack at
// KanikaBreed/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Read-only region terrain, the prim load, region settings and the frame thread.
//! The frame ticks at the shipped `FrameTime`, stores the login presence, and
//! answers a forwarded chat with those positions. A landing updates the position.
//! An `AgentUpdate` stores movement intent on that presence.
//! Each viewer has an entity-update queue. A second update for the same local
//! id merges flags. This crate does not write those packets.
//!
//! Estate edits, terrain upload, physics and wind `LayerData` are later rows.
//! `ferroan-region` keeps this load at startup. `ObjectUpdate` is a later row.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod bits;
mod compress;
pub mod entity_queue;
mod error;
mod estate;
mod frame;
mod height;
mod motion;
mod objects;
mod payloads;
mod settings;
mod store;

pub use compress::TerrainLayer;
pub use entity_queue::{
    ANIMATIONS, CANCEL_KILL, ClassifiedUpdate, EntityUpdateQueue, FULL_UPDATE,
    FULL_UPDATE_WITH_ANIM, FieldPacket, KILL, MAX_ENTITY_UPDATES, POSITION, QueuedUpdate, ROTATION,
    SCALE, SEND_IN_TRANSIT, TERSE_UPDATE, UPDATE_PROBE, VELOCITY, classify,
};
pub use error::{ObjectError, TerrainError};
pub use estate::{EstateAccess, EstateError, MAX_ESTATE_MANAGERS, open_estate};
pub use frame::{LocalTeleport, Presence, TeleportStored, TerrainFrame, landed_z};
pub use height::{Heightmap, movement_z};
pub use motion::{Applied, Motion, MotionUpdate};
pub use objects::{
    InventoryItem, MAX_LINKSET_PRIMS, MAX_REGION_OBJECTS, ObjectGroup, ObjectPart, ObjectRaw,
    ObjectShape, load_objects, load_objects_limited, open_objects,
};
pub use payloads::{
    DynAttrs, Keyframe, KeyframeMotion, LinksetData, LinksetEntry, MaterialOverride, MediaFace,
    ObjectAnimation, PhysicsInertia, RenderMaterials, Vehicle,
};
pub use settings::{RegionSettings, SettingsSource, SpawnPoint, open_region_settings};
pub use store::open_terrain;
