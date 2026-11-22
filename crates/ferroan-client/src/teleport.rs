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
//     (Teleport, TeleportAgentWithinRegion)
//   Source/OpenSim.Framework/Util.cs (CompareRegionHandles, RegionHandleToWorldLoc)
//   Source/OpenSim.Region.Framework/Scenes/Scene.cs (PositionIsInCurrentRegion)
//   Source/OpenSim.Framework/RegionInfo.cs (DefaultLandingPoint)
//   Source/OpenSim.Framework/Constants.cs (MinSimulationHeight, MaxSimulationHeight)
//   KanikaBreed/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// SendLocalTeleport's task-queue ordering is f6cc9597ad. The circuit owns that
// queue. A destination outside this region is not a crossing: that is P2.0i.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One teleport that stays inside this region.
//!
//! `HandleTeleportLocationRequest` always asks for `TeleportFlags.ViaLocation`.
//! The same-region landing follows `TeleportAgentWithinRegion`: an illegal
//! position becomes `(128, 128, 128)`, `(128, 128, 22.5)` becomes the default
//! landing point, and Z is lifted to ground plus half the avatar height plus
//! a tenth of a metre, then clamped. `TeleportLocal` uses location id 2.

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
/// `Constants.MinSimulationHeight`.
const MIN_HEIGHT: f32 = -100.0;
/// `Constants.MaxSimulationHeight`.
const MAX_HEIGHT: f32 = 50_000.0;
/// `RegionInfo.DefaultLandingPoint` field initializer.
#[cfg(test)]
const DEFAULT_LANDING: [f32; 3] = [128.0, 128.0, 30.0];
/// Substituted when the offset is outside the region.
const EMERGENCY: [f32; 3] = [128.0, 128.0, 128.0];
/// `TerrainData.DefaultTerrainHeight`, used when the region has no heightmap.
#[cfg(test)]
const DEFAULT_GROUND: f32 = 21.0;

pub const FAIL_PROCESS: &str = "Could not process the teleport";
pub const FAIL_FROZEN: &str = "You are frozen";
/// `SetInTransit` refused a second teleport. Also used when the task queue
/// cannot take another packet without passing `udp.unacked`.
pub const FAIL_INCOMPLETE: &str = "Previous teleport process incomplete.  Please retry shortly.";

/// Height at one metre. The region supplies the heightmap, or flat 21 m.
pub type GroundSample = Arc<dyn Fn(f32, f32) -> f32 + Send + Sync>;

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
    pub flying: bool,
    pub ground: GroundSample,
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
            flying: self.flying,
            ground: Arc::clone(&self.ground),
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
            .field("flying", &self.flying)
            .finish_non_exhaustive()
    }
}

impl TeleportRegion {
    /// A 256 m region whose handle is zero and whose ground is flat 21 m.
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
            flying: false,
            ground: Arc::new(|_, _| DEFAULT_GROUND),
            position: Arc::new(Mutex::new(position)),
        }
    }
}

pub(crate) struct LocalLanding {
    pub position: [f32; 3],
    pub look_at: [f32; 3],
    pub flags: u32,
}

pub(crate) enum TeleportAnswer {
    Local(LocalLanding),
    Failed(&'static str),
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

pub(crate) fn answer(
    region: &TeleportRegion,
    handle: u64,
    position: [f32; 3],
    look_at: [f32; 3],
) -> TeleportAnswer {
    let Some(offset) = region_offset(handle, position, region) else {
        // Neighbour transfer is P2.0i. A viewer still needs a failure packet.
        return TeleportAnswer::Failed(FAIL_PROCESS);
    };
    if !region.allow_movement {
        return TeleportAnswer::Failed(FAIL_FROZEN);
    }
    let mut flags = VIA_LOCATION;
    if region.godlike {
        flags |= GODLIKE;
    }
    if region.flying {
        flags |= IS_FLYING;
    }
    TeleportAnswer::Local(land(region, offset, look_at, flags))
}

fn land(
    region: &TeleportRegion,
    mut position: [f32; 3],
    look_at: [f32; 3],
    flags: u32,
) -> LocalLanding {
    if !inside(position, region.size_x, region.size_y) {
        position = EMERGENCY;
    }
    if position[0] == 128.0 && position[1] == 128.0 && position[2] == 22.5 {
        position = region.default_landing;
    }
    let ground = (region.ground)(position[0], position[1]);
    let limit = ground + region.half_height + 0.1;
    if position[2] < limit && limit.is_finite() {
        position[2] = limit;
    }
    // NaN stays NaN. The bounds are finite and ordered.
    position[2] = position[2].clamp(MIN_HEIGHT, MAX_HEIGHT);
    LocalLanding {
        position,
        look_at: flatten_look(look_at),
        flags,
    }
}

fn inside(position: [f32; 3], size_x: f32, size_y: f32) -> bool {
    position[0] >= 0.0 && position[0] < size_x && position[1] >= 0.0 && position[1] < size_y
}

fn flatten_look(mut look_at: [f32; 3]) -> [f32; 3] {
    look_at[2] = 0.0;
    if look_at[0].abs() < 0.01 && look_at[1].abs() < 0.01 {
        look_at[0] = 1.0;
        look_at[1] = 0.0;
    }
    look_at
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

    fn region() -> TeleportRegion {
        TeleportRegion::flat([128.0, 128.0, 25.0])
    }

    #[test]
    fn the_harness_target_stays_at_the_requested_height() {
        let answer = answer(&region(), 0, [40.0, 200.0, 30.0], [1.0, 0.0, 0.0]);
        let TeleportAnswer::Local(landing) = answer else {
            panic!("expected a local landing");
        };
        assert_eq!(landing.position, [40.0, 200.0, 30.0]);
        assert_eq!(landing.look_at, [1.0, 0.0, 0.0]);
        assert_eq!(landing.flags, VIA_LOCATION);
    }

    #[test]
    fn a_low_target_is_lifted_to_ground_plus_half_height() {
        let answer = answer(&region(), 0, [40.0, 200.0, 10.0], [1.0, 0.0, 0.0]);
        let TeleportAnswer::Local(landing) = answer else {
            panic!("expected a local landing");
        };
        let expect = 21.0 + (1.9 / 2.0) + 0.1;
        assert_eq!(landing.position, [40.0, 200.0, expect]);
    }

    #[test]
    fn the_default_landing_replaces_the_sentinel_and_a_flat_look_faces_plus_x() {
        let answer = answer(&region(), 0, [128.0, 128.0, 22.5], [0.0, 0.0, 4.0]);
        let TeleportAnswer::Local(landing) = answer else {
            panic!("expected a local landing");
        };
        assert_eq!(landing.position, DEFAULT_LANDING);
        assert_eq!(landing.look_at, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn an_offset_outside_the_region_uses_the_emergency_point() {
        // CompareRegionHandles only returns an inside offset. The emergency
        // substitution is the following position check, so call it directly.
        let region = region();
        let landing = land(&region, [-1.0, 10.0, 10.0], [1.0, 0.0, 0.0], VIA_LOCATION);
        assert_eq!(landing.position, EMERGENCY);
    }

    #[test]
    fn height_is_clamped_to_the_simulation_range() {
        let mut high = region();
        high.ground = Arc::new(|_, _| 60_000.0);
        let TeleportAnswer::Local(landing) = answer(&high, 0, [40.0, 40.0, 10.0], [1.0, 0.0, 0.0])
        else {
            panic!("expected a local landing");
        };
        assert_eq!(landing.position[2], MAX_HEIGHT);
        let mut low = region();
        low.ground = Arc::new(|_, _| f32::NAN);
        let TeleportAnswer::Local(landing) = answer(&low, 0, [40.0, 40.0, -500.0], [1.0, 0.0, 0.0])
        else {
            panic!("expected a local landing");
        };
        assert_eq!(landing.position[2], MIN_HEIGHT);
    }

    #[test]
    fn a_non_finite_ground_does_not_replace_z() {
        let mut region = region();
        region.ground = Arc::new(|_, _| f32::INFINITY);
        let TeleportAnswer::Local(landing) =
            answer(&region, 0, [40.0, 40.0, 30.0], [1.0, 0.0, 0.0])
        else {
            panic!("expected a local landing");
        };
        assert_eq!(landing.position[2], 30.0);
    }

    #[test]
    fn another_region_and_a_frozen_avatar_fail_without_a_landing() {
        let outside = answer(&region(), 256, [10.0, 10.0, 30.0], [1.0, 0.0, 0.0]);
        assert!(matches!(outside, TeleportAnswer::Failed(FAIL_PROCESS)));
        let mut frozen = region();
        frozen.allow_movement = false;
        let refused = answer(&frozen, 0, [40.0, 200.0, 30.0], [1.0, 0.0, 0.0]);
        assert!(matches!(refused, TeleportAnswer::Failed(FAIL_FROZEN)));
    }

    #[test]
    fn godlike_and_flying_are_ored_onto_via_location() {
        let mut region = region();
        region.godlike = true;
        region.flying = true;
        let TeleportAnswer::Local(landing) =
            answer(&region, 0, [40.0, 200.0, 30.0], [0.2, -0.4, 9.0])
        else {
            panic!("expected a local landing");
        };
        assert_eq!(landing.flags, VIA_LOCATION | GODLIKE | IS_FLYING);
        assert_eq!(landing.look_at, [0.2, -0.4, 0.0]);
    }

    #[test]
    fn the_same_handle_keeps_a_position_inside_a_moved_region() {
        let mut region = region();
        region.world_x = 256_000;
        region.world_y = 256_000;
        let handle = (256_000u64 << 32) | 256_000;
        let Some(offset) = region_offset(handle, [40.0, 200.0, 30.0], &region) else {
            panic!("the home handle is this region");
        };
        assert_eq!(offset, [40.0, 200.0, 30.0]);
    }
}
