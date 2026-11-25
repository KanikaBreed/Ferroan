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
//   Source/OpenSim.Region.Framework/Scenes/Scene.cs
//     (Update's SendCoarseLocations every UpdateCoarseLocationsEveryNFrames)
//   Source/OpenSim.Region.Framework/Scenes/SceneGraph.cs (GetCoarseLocations)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (SendCoarseLocationUpdate)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimap dots: `CoarseLocationUpdate`.
//!
//! Every 50th frame (`Scene.Update`, `Frame % m_update_coarse_locations`,
//! `Scene.cs` lines 1795–1810) C# lists the root presences in the scene's
//! presence order, at most 60 (`SceneGraph.GetCoarseLocations`, lines
//! 233–268), and sends that one list to every presence's viewer, children
//! included (`ForEachScenePresence`). Each viewer's packet
//! (`LLClientView.SendCoarseLocationUpdate`, lines 4822–4890) carries X and
//! Y as bytes, Z divided by four (0 above 1024 m), its own index in `You`
//! and -1 in `Prey`. An empty list is sent once, then not again until the
//! list has someone in it. The packet is unreliable and not zero-coded, on
//! the task category.
//!
//! Ferroan lists the attached viewers' root avatars in local id order, the
//! order they attached. C# reads a `Dictionary` whose freed slots are reused,
//! so after a departure a newcomer can take the leaver's place in its list.
//! Only the order in the packet differs; each viewer still finds itself
//! through `You`.

use uuid::Uuid;

/// Shipped `[Startup] UpdateCoarseLocationsEveryNFrames`
/// (`RegionServerDefaults.ini` line 238), about every 4.5 seconds. The scene
/// constructor's 5 (`Scene.cs` line 389) applies only before that ini read
/// (line 1151), as with `FrameTime`.
pub const COARSE_EVERY_FRAMES: u64 = 50;

/// `GetCoarseLocations(.., 60)` and `Math.Min(.., 60)` in the sender.
pub const MAX_COARSE_LOCATIONS: usize = 60;

/// `Constants.RegionSize`.
const REGION_SIZE: f32 = 256.0;

/// One viewer's `CoarseLocationUpdate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoarseLocations {
    /// X, Y and Z/4 of each listed avatar.
    pub locations: Vec<[u8; 3]>,
    /// This viewer's index in `agents`, or -1.
    pub you: i16,
    /// Always -1: C#'s tracked-avatar index is commented out.
    pub prey: i16,
    pub agents: Vec<Uuid>,
}

/// C#'s unchecked `(byte)` of a float on .NET 10: NaN is 0, anything else
/// truncates toward zero to an `int` and keeps its low eight bits, so -1.5
/// is 255 and 256 is 0.
pub(crate) fn csharp_byte(value: f32) -> u8 {
    if value.is_nan() {
        return 0;
    }
    (value as i32) as u8
}

/// One location as `SendCoarseLocationUpdate` writes it. X and Y are already
/// scaled for the region size.
pub(crate) fn location(position: [f32; 3]) -> [u8; 3] {
    let z = if position[2] > 1024.0 {
        0
    } else {
        csharp_byte(position[2] * 0.25)
    };
    [csharp_byte(position[0]), csharp_byte(position[1]), z]
}

/// `GetCoarseLocations`: X and Y scaled to a 256 m region, at most
/// [`MAX_COARSE_LOCATIONS`]. `roots` are the root presences in scene order.
pub(crate) fn gather(
    roots: impl IntoIterator<Item = (Uuid, [f32; 3])>,
    region_size: [u32; 2],
) -> (Vec<[f32; 3]>, Vec<Uuid>) {
    let scale = |size: u32| {
        let ratio = size as f32 / REGION_SIZE;
        if ratio == 0.0 { 1.0 } else { 1.0 / ratio }
    };
    let (scale_x, scale_y) = (scale(region_size[0]), scale(region_size[1]));
    let mut positions = Vec::new();
    let mut agents = Vec::new();
    for (agent, position) in roots.into_iter().take(MAX_COARSE_LOCATIONS) {
        positions.push([position[0] * scale_x, position[1] * scale_y, position[2]]);
        agents.push(agent);
    }
    (positions, agents)
}

/// What `LLClientView` keeps between sends: `m_couseLocationLastEmpty`.
#[derive(Debug, Default)]
pub(crate) struct CoarseState {
    last_empty: bool,
}

impl CoarseState {
    /// `SendCoarseLocationUpdate` for the viewer of `me`. None when the list
    /// is empty again after an empty one was sent.
    pub(crate) fn packet(
        &mut self,
        me: Uuid,
        positions: &[[f32; 3]],
        agents: &[Uuid],
    ) -> Option<CoarseLocations> {
        let total = positions.len().min(MAX_COARSE_LOCATIONS);
        if total == 0 {
            if self.last_empty {
                return None;
            }
            self.last_empty = true;
        } else {
            self.last_empty = false;
        }
        let named = agents.len().min(MAX_COARSE_LOCATIONS).min(total);
        let you = agents[..named]
            .iter()
            .position(|agent| *agent == me)
            .and_then(|index| i16::try_from(index).ok())
            .unwrap_or(-1);
        Some(CoarseLocations {
            locations: positions[..total].iter().map(|at| location(*at)).collect(),
            you,
            prey: -1,
            agents: agents[..named].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(1);
    const B: Uuid = Uuid::from_u128(2);

    #[test]
    fn x_and_y_truncate_and_z_is_a_quarter() {
        assert_eq!(location([128.9, 3.2, 25.96]), [128, 3, 6]);
        assert_eq!(location([0.0, 255.99, 0.87]), [0, 255, 0]);
        assert_eq!(location([10.0, 10.0, 27.96]), [10, 10, 6]);
        assert_eq!(location([10.0, 10.0, 28.0]), [10, 10, 7]);
        assert_eq!(location([10.0, 10.0, 1019.0]), [10, 10, 254]);
    }

    /// The .NET 10 results of the same casts (run on SDK 10.0.114).
    #[test]
    fn out_of_range_values_wrap_as_csharp_casts_them() {
        assert_eq!(location([10.0, 10.0, 1024.0]), [10, 10, 0]);
        assert_eq!(location([10.0, 10.0, 1024.5]), [10, 10, 0]);
        assert_eq!(
            location([10.0, 10.0, 1100.0]),
            [10, 10, 0],
            "the 1024 m cut"
        );
        assert_eq!(location([10.0, 10.0, 1023.9]), [10, 10, 255]);
        assert_eq!(location([10.0, 10.0, -1.0]), [10, 10, 0]);
        assert_eq!(location([10.0, 10.0, -4.0]), [10, 10, 255]);
        assert_eq!(location([10.0, 10.0, -100.0]), [10, 10, 231]);
        assert_eq!(location([-0.5, -1.5, f32::NAN]), [0, 255, 0]);
        assert_eq!(location([256.0, 300.0, 0.0]), [0, 44, 0]);
    }

    #[test]
    fn each_viewer_finds_itself_and_prey_is_unset() {
        let positions = [[128.0, 128.0, 25.96], [131.4, 128.2, 27.96]];
        let agents = [A, B];
        let mine = CoarseState::default()
            .packet(A, &positions, &agents)
            .unwrap();
        let theirs = CoarseState::default()
            .packet(B, &positions, &agents)
            .unwrap();
        assert_eq!(mine.you, 0);
        assert_eq!(theirs.you, 1);
        assert_eq!(mine.prey, -1);
        assert_eq!(mine.locations, [[128, 128, 6], [131, 128, 6]]);
        assert_eq!(mine.locations, theirs.locations);
        assert_eq!(mine.agents, [A, B]);
        let child = CoarseState::default()
            .packet(Uuid::from_u128(3), &positions, &agents)
            .unwrap();
        assert_eq!(child.you, -1, "a viewer whose avatar is not listed");
    }

    #[test]
    fn an_empty_list_is_sent_once_until_someone_is_listed() {
        let mut state = CoarseState::default();
        let empty = state.packet(A, &[], &[]).unwrap();
        assert!(empty.locations.is_empty() && empty.agents.is_empty());
        assert_eq!((empty.you, empty.prey), (-1, -1));
        assert_eq!(state.packet(A, &[], &[]), None);
        assert!(state.packet(A, &[[1.0, 2.0, 3.0]], &[B]).is_some());
        assert!(state.packet(A, &[], &[]).is_some());
        assert_eq!(state.packet(A, &[], &[]), None);
    }

    #[test]
    fn at_most_sixty_are_listed_in_scene_order() {
        let roots: Vec<_> = (0..70u128)
            .map(|n| (Uuid::from_u128(n + 10), [n as f32, 1.0, 0.0]))
            .collect();
        let (positions, agents) = gather(roots, [256, 256]);
        assert_eq!(positions.len(), 60);
        assert_eq!(agents[0], Uuid::from_u128(10));
        assert_eq!(agents[59], Uuid::from_u128(69));
        let last = Uuid::from_u128(69);
        let packet = CoarseState::default()
            .packet(last, &positions, &agents)
            .unwrap();
        assert_eq!(packet.you, 59);
        assert_eq!(packet.locations[59], [59, 1, 0]);
    }

    #[test]
    fn a_larger_region_scales_x_and_y_into_bytes() {
        let (positions, _) = gather([(A, [300.0, 100.0, 40.0])], [512, 256]);
        assert_eq!(positions, [[150.0, 100.0, 40.0]]);
        let (positions, _) = gather([(A, [300.0, 100.0, 40.0])], [0, 256]);
        assert_eq!(positions, [[300.0, 100.0, 40.0]]);
    }
}
