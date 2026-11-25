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
//   Source/OpenSim.Region.Framework/Scenes/TerrainChannel.cs
//     (TerrainChannel(String, int, int, int), PinHeadIsland, FlatLand)
//   Source/OpenSim.Region.Framework/Scenes/TerrainUtil.cs (SphericalFactor)
//   Source/OpenSim.Framework/TerrainData.cs (ClearLand, DefaultTerrainHeight)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The terrain a region builds when its store has none: `[Terrain]
//! InitialTerrain`, `pinhead-island` by default (`RegionServerDefaults.ini`).
//! `TerrainChannel(String, int, int, int)` makes `flat` (every cell 21) for
//! the exact string `flat` and the pinhead island for any other value.

use crate::error::TerrainError;
use crate::height::{DEFAULT_HEIGHT, Heightmap, check_edge};

/// `Scene.LoadWorldMap`'s default when `[Terrain] InitialTerrain` is absent.
pub const DEFAULT_INITIAL_TERRAIN: &str = "pinhead-island";

/// The two maps `TerrainChannel` can build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialTerrain {
    /// `FlatLand`: `ClearLand()` to 21.
    Flat,
    /// `PinHeadIsland`.
    PinheadIsland,
}

impl InitialTerrain {
    /// `type.Equals("flat")`, ordinal and case-sensitive. Every other name,
    /// `pinhead-island` included, is the island.
    pub fn from_name(name: &str) -> Self {
        if name == "flat" {
            Self::Flat
        } else {
            Self::PinheadIsland
        }
    }

    /// The map for a region of `width` by `height` metres.
    pub fn generate(self, width: i32, height: i32) -> Result<Heightmap, TerrainError> {
        check_edge(width)?;
        check_edge(height)?;
        let len = (width as usize)
            .checked_mul(height as usize)
            .ok_or(TerrainError::Edge)?;
        let cells = match self {
            Self::Flat => vec![DEFAULT_HEIGHT; len],
            Self::PinheadIsland => pinhead_island(width, height, len),
        };
        Ok(Heightmap::from_cells(width, height, cells))
    }
}

/// `TerrainUtil.SphericalFactor`, in `float`.
fn spherical_factor(dx: f32, dy: f32, size: f32) -> f32 {
    let a = (dx * dx + dy * dy) / (size * size);
    if a >= 1.0 { 0.0 } else { 1.0 - a }
}

/// `PinHeadIsland`: a 25 m bump of radius 50 on a 10 m shelf of radius 100,
/// centred on the region, zero outside. The centre is the `float` half size.
fn pinhead_island(width: i32, height: i32, len: usize) -> Vec<f32> {
    let cx = width as f32 * 0.5;
    let cy = height as f32 * 0.5;
    let mut cells = vec![0.0; len];
    for x in 0..width {
        for y in 0..height {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let mut h = 25.0 * spherical_factor(dx, dy, 50.0);
            let b = 10.0 * spherical_factor(dx, dy, 100.0);
            if h < b {
                h = b;
            }
            let index = (y as usize) * (width as usize) + (x as usize);
            if let Some(slot) = cells.get_mut(index) {
                *slot = h;
            }
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/walk");
        std::fs::read(root.join(name)).unwrap()
    }

    #[test]
    fn only_the_exact_name_flat_is_flat() {
        assert_eq!(InitialTerrain::from_name("flat"), InitialTerrain::Flat);
        assert_eq!(
            InitialTerrain::from_name("pinhead-island"),
            InitialTerrain::PinheadIsland
        );
        assert_eq!(
            InitialTerrain::from_name("Flat"),
            InitialTerrain::PinheadIsland
        );
        assert_eq!(InitialTerrain::from_name(""), InitialTerrain::PinheadIsland);
    }

    #[test]
    fn flat_is_twenty_one_everywhere() {
        let map = InitialTerrain::Flat.generate(32, 16).unwrap();
        assert_eq!(map.cells().len(), 512);
        assert!(map.cells().iter().all(|h| *h == 21.0));
    }

    #[test]
    fn the_edge_rules_still_apply() {
        assert!(InitialTerrain::PinheadIsland.generate(0, 256).is_err());
        assert!(InitialTerrain::PinheadIsland.generate(256, 250).is_err());
    }

    /// `fixtures/walk/terrain.bin` is the `terrain` row C# stored for a new
    /// 256 m region: the island after the three-decimal rounding of
    /// `ToCompressedTerrainSerializationV2DGzip`.
    #[test]
    fn the_island_is_the_csharp_row_bit_for_bit() {
        let stored = Heightmap::from_blob(23, &fixture("terrain.bin"), 256, 256).unwrap();
        let island = InitialTerrain::PinheadIsland.generate(256, 256).unwrap();
        let mut differ = 0;
        for (index, (ours, theirs)) in island.cells().iter().zip(stored.cells()).enumerate() {
            let rounded = crate::height::round3_away(*ours);
            if rounded.to_bits() != theirs.to_bits() {
                differ += 1;
                if differ < 5 {
                    eprintln!(
                        "cell {index}: generated {ours} rounds to {rounded}, C# stored {theirs}"
                    );
                }
            }
        }
        assert_eq!(differ, 0);
        assert_eq!(island.height_at(128.0, 128.0), 25.0);
        assert_eq!(island.cells()[0], 0.0);
        // The round trip through Ferroan's own blob gives the stored cells.
        let (revision, blob) = island.to_blob().unwrap();
        assert_eq!(revision, 23);
        let back = Heightmap::from_blob(revision, &blob, 256, 256).unwrap();
        assert!(
            back.cells()
                .iter()
                .zip(stored.cells())
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }

    #[test]
    fn rounding_matches_net_away_from_zero() {
        use crate::height::round3_away;
        assert_eq!(round3_away(1.0006), 1.001);
        assert_eq!(round3_away(-1.0006), -1.001);
        assert_eq!(round3_away(2.0004), 2.0);
        assert_eq!(round3_away(25.0), 25.0);
        assert_eq!(round3_away(0.0), 0.0);
        // 0.0004999999 scales to just under 0.5: BitDecrement(0.5) keeps it
        // at 0, a plain 0.5 would round it up.
        assert_eq!(round3_away(f32::from_bits(0x3a03_126e)), 0.0);
    }
}
