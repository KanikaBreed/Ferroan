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
//     (MergeWithBounding, 309–434)
//   Source/OpenSim.Region.CoreModules/World/Archiver/ArchiveReadRequest.cs
//     (bounding-size clip, 262–281)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Console-path terrain merge. `HandleLoadOarConsoleCommand` always inserts
//! `bounding-origin`, so `LoadTerrain` takes `MergeWithBounding`. The older
//! `Merge` is not this path.
//!
//! The incoming map is the decoded square. It is not cropped to the region
//! corner (D142): a bounding origin or a rotation can sample past that corner.
//! A trim above 4096 is still [`TerrainError::Edge`] from the decoder.
//! `--rotation-center` is parsed by the console and is not an argument here.

use crate::error::TerrainError;
use crate::height::Heightmap;

/// The temporary-map fill. A cell that is still this value was not taken from
/// the incoming map, and the destination cell is left as it was.
pub const MERGE_MASK: f32 = -65535.0;

/// `ArchiveReadRequest`'s bounding-size clip. An X or Y that is ≤ 0 or larger
/// than the destination region becomes that region's size. The merge itself
/// does not clip.
pub fn clip_bounding_size(region_x: i32, region_y: i32, size_x: f32, size_y: f32) -> (f32, f32) {
    let limit_x = region_x as f32;
    let limit_y = region_y as f32;
    let x = if size_x <= 0.0 || size_x > limit_x {
        limit_x
    } else {
        size_x
    };
    let y = if size_y <= 0.0 || size_y > limit_y {
        limit_y
    } else {
        size_y
    };
    (x, y)
}

/// `TerrainChannel.MergeWithBounding`.
///
/// `rotation_degrees` is the console angle. Cosine and sine are taken first.
/// A negative angle then gains 360 before the 0, 90, 180 and 270 tests, so
/// −90 uses the 270 path. Other angles round half to even, which is
/// `Math.Round(double)`. Displacement X and Y are floored. Z is added only
/// after the mask test. The C# rotation loops run one index past the
/// temporary map; that write throws and is discarded, so this loop stays in
/// range.
pub fn merge_with_bounding(
    destination: &mut Heightmap,
    incoming: &Heightmap,
    displacement: [f32; 3],
    mut rotation_degrees: f32,
    bounding_origin: [f32; 2],
    bounding_size: [f32; 2],
) -> Result<(), TerrainError> {
    let base_x = incoming.width();
    let base_y = incoming.height();
    let dest_x = destination.width();
    let dest_y = destination.height();
    if base_x <= 0 || base_y <= 0 || dest_x <= 0 || dest_y <= 0 {
        return Err(TerrainError::Raw32);
    }
    let offset_x = base_x / 4;
    let offset_y = base_y / 4;
    let tmp_x = base_x.checked_add(base_x / 2).ok_or(TerrainError::Raw32)?;
    let tmp_y = base_y.checked_add(base_y / 2).ok_or(TerrainError::Raw32)?;
    let centre_x = tmp_x / 2;
    let centre_y = tmp_y / 2;
    let temp_len = usize::try_from(tmp_x)
        .ok()
        .and_then(|width| {
            usize::try_from(tmp_y)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(TerrainError::Raw32)?;
    let mut temp = vec![MERGE_MASK; temp_len];

    let radians = std::f64::consts::PI * f64::from(rotation_degrees) / 180.0;
    let cos_r = radians.cos();
    let sin_r = radians.sin();
    if rotation_degrees < 0.0 {
        rotation_degrees += 360.0;
    }

    let source = incoming.cells();
    for y in 0..tmp_y {
        for x in 0..tmp_x {
            let (sx, sy) = if rotation_degrees == 0.0 {
                (x - offset_x, y - offset_y)
            } else if rotation_degrees == 90.0 {
                (y - offset_x, tmp_y - 1 - x - offset_y)
            } else if rotation_degrees == 180.0 {
                (tmp_x - 1 - x - offset_x, tmp_y - 1 - y - offset_y)
            } else if rotation_degrees == 270.0 {
                (tmp_x - 1 - y - offset_x, x - offset_y)
            } else {
                let sx = centre_x
                    + round_half_even_to_i32(
                        (f64::from(x - centre_x) * cos_r) + (f64::from(y - centre_y) * sin_r),
                    )
                    - offset_x;
                let sy = centre_y
                    + round_half_even_to_i32(
                        (f64::from(y - centre_y) * cos_r) - (f64::from(x - centre_x) * sin_r),
                    )
                    - offset_y;
                (sx, sy)
            };
            if sx >= 0 && sx < base_x && sy >= 0 && sy < base_y {
                let src = (sy as usize) * (base_x as usize) + (sx as usize);
                let dst = (y as usize) * (tmp_x as usize) + (x as usize);
                if let (Some(value), Some(slot)) = (source.get(src), temp.get_mut(dst)) {
                    *slot = *value;
                }
            }
        }
    }

    let disp_x = floor_to_i32(displacement[0]);
    let disp_y = floor_to_i32(displacement[1]);
    let disp_z = displacement[2];
    let start_x = clamp_window(floor_to_i32(bounding_origin[0]) + offset_x, tmp_x);
    let start_y = clamp_window(floor_to_i32(bounding_origin[1]) + offset_y, tmp_y);
    let end_x = clamp_window(
        floor_to_i32(bounding_origin[0] + bounding_size[0]) + offset_x,
        tmp_x,
    );
    let end_y = clamp_window(
        floor_to_i32(bounding_origin[1] + bounding_size[1]) + offset_y,
        tmp_y,
    );

    for y in start_y..end_y {
        for x in start_x..end_x {
            let dx = x - start_x + disp_x;
            let dy = y - start_y + disp_y;
            if dx >= 0 && dx < dest_x && dy >= 0 && dy < dest_y {
                let src = (y as usize) * (tmp_x as usize) + (x as usize);
                let Some(new_height) = temp.get(src).copied() else {
                    continue;
                };
                if new_height != MERGE_MASK {
                    destination.assign_cell(dx, dy, new_height + disp_z);
                }
            }
        }
    }
    Ok(())
}

/// `(int)Math.Floor` of a float. The float widens to double before the floor.
fn floor_to_i32(value: f32) -> i32 {
    f64::from(value).floor() as i32
}

/// `(int)Math.Round(double)`. Midpoints go to the even integer, then the cast
/// truncates toward zero. The coordinates this merge rounds already sit on an
/// integer inside `i32`.
fn round_half_even_to_i32(value: f64) -> i32 {
    value.round_ties_even() as i32
}

fn clamp_window(value: i32, tmp: i32) -> i32 {
    let mut value = value;
    if value > tmp {
        value = tmp;
    }
    if value < 0 {
        value = 0;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw32::{is_raw32_name, load_raw32};

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/oar-raw32")
            .join(name)
    }

    fn read(name: &str) -> Vec<u8> {
        std::fs::read(fixture(name)).unwrap()
    }

    fn le_bytes(map: &Heightmap) -> Vec<u8> {
        let mut out = Vec::with_capacity(map.cells().len() * 4);
        for value in map.cells() {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    fn map_from(name: &str) -> Heightmap {
        load_raw32(&read(name)).unwrap()
    }

    fn merged(
        source: &str,
        displacement: [f32; 3],
        rotation: f32,
        origin: [f32; 2],
        size: [f32; 2],
    ) -> Vec<u8> {
        let mut destination = map_from("initial.r32");
        let incoming = map_from(source);
        merge_with_bounding(
            &mut destination,
            &incoming,
            displacement,
            rotation,
            origin,
            size,
        )
        .unwrap();
        le_bytes(&destination)
    }

    const REGION: [f32; 2] = [256.0, 256.0];
    const ORIGIN: [f32; 2] = [0.0, 0.0];
    const STILL: [f32; 3] = [0.0, 0.0, 0.0];

    #[test]
    fn half_integers_round_the_way_math_round_does() {
        assert_eq!(round_half_even_to_i32(0.5), 0);
        assert_eq!(round_half_even_to_i32(1.5), 2);
        assert_eq!(round_half_even_to_i32(2.5), 2);
        assert_eq!(round_half_even_to_i32(-0.5), 0);
        assert_eq!(round_half_even_to_i32(-1.5), -2);
        assert_eq!(floor_to_i32(-1.2), -2);
        assert_eq!(floor_to_i32(8.9), 8);
        assert_eq!(floor_to_i32(-0.2), -1);
    }

    #[test]
    fn a_bounding_size_outside_the_region_clips_to_it() {
        assert_eq!(clip_bounding_size(256, 256, 0.0, 512.0), (256.0, 256.0));
        assert_eq!(clip_bounding_size(256, 256, -1.0, 1.0), (256.0, 1.0));
        assert_eq!(clip_bounding_size(256, 256, 256.0, 256.0), (256.0, 256.0));
        assert_eq!(clip_bounding_size(256, 256, 128.0, 64.0), (128.0, 64.0));
    }

    #[test]
    fn each_console_option_matches_the_pin8_load() {
        let initial = read("initial.r32");
        let loaded = read("loaded.r32");
        assert_eq!(merged("member.r32", STILL, 0.0, ORIGIN, REGION), loaded);

        let displacement = merged("member.r32", [8.9, -1.2, 1.5], 0.0, ORIGIN, REGION);
        assert_eq!(displacement, read("displacement.r32"));
        assert_ne!(displacement, loaded);

        let rotation90 = merged("member.r32", STILL, 90.0, ORIGIN, REGION);
        assert_eq!(rotation90, read("rotation90.r32"));
        assert_ne!(rotation90, loaded);

        let rotation45 = merged("member.r32", STILL, 45.0, ORIGIN, REGION);
        assert_eq!(rotation45, read("rotation45.r32"));
        assert_ne!(rotation45, rotation90);

        let rotation270 = merged("member.r32", STILL, 270.0, ORIGIN, REGION);
        let rotation_neg90 = merged("member.r32", STILL, -90.0, ORIGIN, REGION);
        assert_eq!(rotation270, read("rotation270.r32"));
        assert_eq!(rotation_neg90, read("rotation-neg90.r32"));
        assert_eq!(rotation270, rotation_neg90);

        let wide = map_from("wide.r32");
        assert_eq!(wide.width(), 512);
        assert_eq!(wide.height(), 512);
        let bounding = merged("wide.r32", STILL, 0.0, [256.0, 0.0], REGION);
        assert_eq!(bounding, read("bounding.r32"));
        assert_ne!(bounding, initial);
        assert_ne!(bounding, loaded);

        let mask_in = map_from("mask-member.r32");
        assert_eq!(
            mask_in.cells()[10 * 256 + 10].to_bits(),
            MERGE_MASK.to_bits()
        );
        let mask = merged("mask-member.r32", STILL, 0.0, ORIGIN, REGION);
        assert_eq!(mask, read("mask.r32"));
        assert_ne!(mask, loaded);
        let at = (10 * 256 + 10) * 4;
        let kept = f32::from_le_bytes(mask[at..at + 4].try_into().unwrap());
        let island = f32::from_le_bytes(initial[at..at + 4].try_into().unwrap());
        assert_eq!(kept.to_bits(), island.to_bits());
        assert_ne!(island.to_bits(), MERGE_MASK.to_bits());

        assert!(is_raw32_name("terrains/Raw32.r32"));
        assert!(is_raw32_name("terrains/Raw32.f32"));
        assert!(!is_raw32_name("terrains/Raw32.R32"));
        assert!(!is_raw32_name("terrains/Raw32.ter"));
        assert_eq!(read("f32.r32"), loaded);

        let (clip_x, clip_y) = clip_bounding_size(256, 256, 0.0, 512.0);
        let clip = merged("member.r32", STILL, 0.0, ORIGIN, [clip_x, clip_y]);
        assert_eq!(clip, read("clip.r32"));
        assert_eq!(clip, loaded);
        let unclipped = merged("member.r32", STILL, 0.0, ORIGIN, [0.0, 512.0]);
        assert_eq!(unclipped, initial);
        assert_ne!(initial, loaded);
    }
}
