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
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (HandleTeleportLocationRequest, SendLocalTeleport, SendTeleportStart,
//      SendTeleportFailed)
//   Source/OpenSim.Region.CoreModules/Framework/EntityTransfer/EntityTransferModule.cs
//     (Teleport)
//   Source/OpenSim.Framework/Util.cs (CompareRegionHandles, RegionHandleToWorldLoc)
//   Source/OpenSim.Framework/RegionInfo.cs (DefaultLandingPoint)
//   KanikaBreed/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// SendLocalTeleport's task-queue ordering is f6cc9597ad. The circuit owns that
// queue. A destination outside this region starts the circuit's source hop
// when one is installed, and otherwise sends `Could not process the teleport`.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One teleport that stays inside this region.
//!
//! `HandleTeleportLocationRequest` always asks for `TeleportFlags.ViaLocation`.
//! The circuit decides whether the destination is this region and forwards
//! it. The frame (`ferroan-scene`) applies the `TeleportAgentWithinRegion`
//! landing and adds `IsFlying`; it is the only copy of that rule. A circuit
//! with no scene answers `Could not process the teleport`. `TeleportLocal`
//! uses location id 2. A destination outside this region is a source hop
//! when the circuit has one installed. With none installed, the viewer gets
//! `Could not process the teleport`.

use std::sync::{Arc, Mutex};

use ferroan_protocol::msgs::{TeleportFailed, TeleportLocal, TeleportStart, teleport_local};
use ferroan_protocol::str_field;
use glam::Vec3;
use uuid::Uuid;

/// `TeleportFlags.ViaLocation`.
pub const VIA_LOCATION: u32 = 1 << 4;
/// `TeleportFlags.Godlike`.
pub const GODLIKE: u32 = 1 << 9;
/// `TeleportFlags.IsFlying`.
pub const IS_FLYING: u32 = 1 << 13;
/// `LLClientView.SendLocalTeleport` writes this location id.
pub const LOCATION_ID: u32 = 2;
/// `RegionInfo.DefaultLandingPoint` field initializer.
#[cfg(test)]
const DEFAULT_LANDING: [f32; 3] = [128.0, 128.0, 30.0];

pub const FAIL_PROCESS: &str = "Could not process the teleport";
pub const FAIL_FROZEN: &str = "You are frozen";
/// `SetInTransit` refused a second teleport. Also used when the task queue
/// already holds the maximum number of unsent local teleport replies.
pub const FAIL_INCOMPLETE: &str = "Previous teleport process incomplete.  Please retry shortly.";
/// A child agent is only a copy. The avatar is in its root region.
pub const FAIL_CHILD: &str = "You are not in this region. Please try again in a moment.";

/// Region rectangle and the avatar facts `TeleportAgentWithinRegion` reads.
pub struct TeleportRegion {
    pub world_x: u32,
    pub world_y: u32,
    pub size_x: f32,
    pub size_y: f32,
    /// `RegionInfo.DefaultLandingPoint`. The field initializer is `(128, 128, 30)`.
    pub default_landing: [f32; 3],
    /// `Appearance.AvatarHeight / 2`. Login stores an appearance, so this is
    /// not the 0.8 m null-appearance fallback.
    pub half_height: f32,
    pub allow_movement: bool,
    pub godlike: bool,
    /// Login position, then the landed position. The region reads it at logout.
    pub position: Arc<Mutex<[f32; 3]>>,
}

impl Clone for TeleportRegion {
    fn clone(&self) -> Self {
        Self {
            world_x: self.world_x,
            world_y: self.world_y,
            size_x: self.size_x,
            size_y: self.size_y,
            default_landing: self.default_landing,
            half_height: self.half_height,
            allow_movement: self.allow_movement,
            godlike: self.godlike,
            position: Arc::clone(&self.position),
        }
    }
}

impl std::fmt::Debug for TeleportRegion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TeleportRegion")
            .field("world_x", &self.world_x)
            .field("world_y", &self.world_y)
            .field("size_x", &self.size_x)
            .field("size_y", &self.size_y)
            .field("default_landing", &self.default_landing)
            .field("half_height", &self.half_height)
            .field("allow_movement", &self.allow_movement)
            .field("godlike", &self.godlike)
            .finish_non_exhaustive()
    }
}

impl TeleportRegion {
    /// A 256 m region whose handle is zero.
    #[cfg(test)]
    pub(crate) fn flat(position: [f32; 3]) -> Self {
        Self {
            world_x: 0,
            world_y: 0,
            size_x: 256.0,
            size_y: 256.0,
            default_landing: DEFAULT_LANDING,
            half_height: 1.9 / 2.0,
            allow_movement: true,
            godlike: false,
            position: Arc::new(Mutex::new(position)),
        }
    }
}

/// `CompareRegionHandles` for this region's world origin and size.
pub(crate) fn region_offset(
    handle: u64,
    position: [f32; 3],
    region: &TeleportRegion,
) -> Option<[f32; 3]> {
    let hx = (handle >> 32) as u32;
    let hy = handle as u32;
    // uint subtraction, then the sum is widened to double.
    let px = f64::from(hx.wrapping_sub(region.world_x)) + f64::from(position[0]);
    let py = f64::from(hy.wrapping_sub(region.world_y)) + f64::from(position[1]);
    if px >= 0.0 && px < f64::from(region.size_x) && py >= 0.0 && py < f64::from(region.size_y) {
        Some([px as f32, py as f32, position[2]])
    } else {
        None
    }
}

pub(crate) fn start_packet(flags: u32) -> TeleportStart {
    let mut message = TeleportStart::default();
    message.info.teleport_flags = flags;
    message
}

pub(crate) fn local_packet(
    agent: Uuid,
    position: [f32; 3],
    look_at: [f32; 3],
    flags: u32,
) -> TeleportLocal {
    TeleportLocal {
        info: teleport_local::Info {
            agent_id: agent,
            location_id: LOCATION_ID,
            position: Vec3::from_array(position),
            look_at: Vec3::from_array(look_at),
            teleport_flags: flags,
        },
    }
}

pub(crate) fn failed_packet(agent: Uuid, reason: &str) -> TeleportFailed {
    let mut message = TeleportFailed::default();
    message.info.agent_id = agent;
    message.info.reason = str_field(reason);
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_handle_keeps_a_position_inside_a_moved_region() {
        let mut region = TeleportRegion::flat([128.0, 128.0, 25.0]);
        region.world_x = 256_000;
        region.world_y = 256_000;
        let handle = (256_000u64 << 32) | 256_000;
        let Some(offset) = region_offset(handle, [40.0, 200.0, 30.0], &region) else {
            panic!("the home handle is this region");
        };
        assert_eq!(offset, [40.0, 200.0, 30.0]);
        assert!(region_offset(0, [10.0, 10.0, 30.0], &region).is_none());
        assert!(region_offset(handle, [256.0, 10.0, 30.0], &region).is_none());
    }
}
