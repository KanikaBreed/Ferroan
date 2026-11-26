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
//   Source/OpenSim.Region.CoreModules/World/Terrain/FileLoaders/RAW32.cs
//   Source/OpenSim.Region.Framework/Scenes/TerrainChannel.cs (the indexer)
//   Source/OpenSim.Framework/TerrainData.cs (the setter and the constructor)
//   Source/OpenSim.Framework/TerrainTaintsArray.cs (Set, calclen)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! RAW32 terrain (`.r32`). `LoadStream` sizes a square from the byte length,
//! in multiples of 256 and at least 256, then reads little-endian floats,
//! Y outer and X inner. `TerrainChannel`'s setter drops NaN and infinity.
//! `TerrainData` then skips a write when the cell already compares equal, so
//! the initial +0 stays +0 when the file says -0. A file shorter than that
//! square fails. A square trimmed above 4096 reports `terrain.edge`. Pin8
//! allocates that square. The trimmed square is not cropped to the region
//! corner ([`crate::merge`]). The section reader is `LoadFile` with offsets.
//! `fileHeight` is unused. A changed sample whose taint word lies outside the
//! array `TerrainData` allocated fails, as `TerrainTaintsArray.Set` throws.
//! The console merge reads a map this decoder already returned. The archive
//! walk does not call it yet.

use crate::error::TerrainError;
use crate::height::{Heightmap, MAX_EDGE};

/// `Constants.RegionSize`.
const REGION: i32 = 256;

/// `.r32` and `.f32` are the same RAW32 loader (`TerrainModule.cs` 735–736).
/// `EndsWith` is ordinal, so the suffix case matches.
pub fn is_raw32_name(filename: &str) -> bool {
    filename.ends_with(".r32") || filename.ends_with(".f32")
}

/// Decode one RAW32 blob the way `RAW32.LoadStream` fills a `TerrainChannel`.
pub fn load_raw32(bytes: &[u8]) -> Result<Heightmap, TerrainError> {
    let edge = trimmed_edge(bytes.len())?;
    let mut cells = vec![0.0; square(edge)?];
    let mut pos = 0usize;
    for y in 0..edge {
        for x in 0..edge {
            let value = read_f32(bytes, &mut pos)?;
            store(&mut cells[index(edge, x, y)?], value);
        }
    }
    Ok(Heightmap::from_cells(edge, edge, cells))
}

/// `RAW32.LoadFile` for one section of a larger raw file.
///
/// `file_height` is the C# parameter. The method never reads it. A negative
/// `file_width` with `offset_y` at most 0 matches Pin8: the Y-strip loop does
/// not run, and the X tail does not run, so the section is still read. A
/// positive `offset_y` refuses a negative `file_width`. C# multiplies that
/// strip in an unchecked `int`, which can wrap for a very wide file. This
/// loader does not wrap it. Neither case is `load oar`. Crate-private until
/// the terrain load-tile caller exists.
#[allow(dead_code)]
pub(crate) fn load_raw32_section(
    bytes: &[u8],
    offset_x: i32,
    offset_y: i32,
    file_width: i32,
    file_height: i32,
    section_width: i32,
    section_height: i32,
) -> Result<Heightmap, TerrainError> {
    let _ = file_height;
    let width = section_edge(section_width)?;
    let height = section_edge(section_height)?;
    let width_i = i32::try_from(width).map_err(|_| TerrainError::Raw32)?;
    let height_i = i32::try_from(height).map_err(|_| TerrainError::Raw32)?;
    let mut cells = vec![0.0; width.checked_mul(height).ok_or(TerrainError::Raw32)?];
    let mut pos = 0usize;
    let row_skip = (section_width as usize)
        .checked_mul(4)
        .ok_or(TerrainError::Raw32)?;
    // `offset_y <= 0` never reads the Y strip, so `file_width` is not multiplied.
    let rows = offset_y.max(0);
    if rows > 0 {
        let strip = y_strip_bytes(section_height, file_width, section_width)?;
        for _ in 0..rows {
            skip(bytes, &mut pos, strip);
        }
    }
    for y in 0..height {
        let mut curr_x = 0i32;
        while curr_x < offset_x {
            skip(bytes, &mut pos, row_skip);
            curr_x = curr_x.checked_add(1).ok_or(TerrainError::Raw32)?;
        }
        for x in 0..width {
            let value = read_f32(bytes, &mut pos)?;
            if store(&mut cells[y * width + x], value) {
                let x_i = i32::try_from(x).map_err(|_| TerrainError::Raw32)?;
                let y_i = i32::try_from(y).map_err(|_| TerrainError::Raw32)?;
                taint_word(width_i, height_i, x_i, y_i)?;
            }
        }
        curr_x = curr_x.checked_add(1).ok_or(TerrainError::Raw32)?;
        while curr_x < file_width {
            skip(bytes, &mut pos, row_skip);
            curr_x = curr_x.checked_add(1).ok_or(TerrainError::Raw32)?;
        }
    }
    Ok(Heightmap::from_cells(width_i, height_i, cells))
}

/// `LoadStream`'s dimension trim. A square above [`MAX_EDGE`] reports
/// `terrain.edge` and returns [`TerrainError::Edge`]. Pin8 allocates that
/// square. This loader refuses it.
fn trimmed_edge(byte_len: usize) -> Result<i32, TerrainError> {
    let floats = byte_len / 4;
    let dimension = (floats as f64).sqrt();
    // A real slice cannot make this square root exceed `i32::MAX`. The guard
    // still refuses a `usize` argument that is not a real allocation.
    if !dimension.is_finite() || dimension < 0.0 || dimension > f64::from(i32::MAX) {
        return Err(TerrainError::Raw32);
    }
    let dim = dimension.floor() as i32;
    let mut trimmed = dim - dim.rem_euclid(REGION);
    if trimmed < REGION {
        trimmed = REGION;
    }
    crate::height::check_edge(trimmed)?;
    Ok(trimmed)
}

fn square(edge: i32) -> Result<usize, TerrainError> {
    let side = usize::try_from(edge).map_err(|_| TerrainError::Raw32)?;
    side.checked_mul(side).ok_or(TerrainError::Raw32)
}

fn index(edge: i32, x: i32, y: i32) -> Result<usize, TerrainError> {
    let width = usize::try_from(edge).map_err(|_| TerrainError::Raw32)?;
    let x = usize::try_from(x).map_err(|_| TerrainError::Raw32)?;
    let y = usize::try_from(y).map_err(|_| TerrainError::Raw32)?;
    y.checked_mul(width)
        .and_then(|row| row.checked_add(x))
        .ok_or(TerrainError::Raw32)
}

/// A section side. `0..=MAX_EDGE` is accepted, including a side under 16 or
/// not a multiple of 16. A side past [`MAX_EDGE`] reports `terrain.edge`. A
/// negative side is [`TerrainError::Raw32`] and does not report that limit.
fn section_edge(side: i32) -> Result<usize, TerrainError> {
    if side > MAX_EDGE {
        // 4096 itself stays `Ok`: `check_edge` accepts it. Only a larger side
        // reaches this arm, and that call reports `terrain.edge`.
        crate::height::check_edge(side)?;
        return Err(TerrainError::Edge);
    }
    if side < 0 {
        return Err(TerrainError::Raw32);
    }
    Ok(side as usize)
}

fn y_strip_bytes(
    section_height: i32,
    file_width: i32,
    section_width: i32,
) -> Result<usize, TerrainError> {
    if section_height < 0 || file_width < 0 || section_width < 0 {
        return Err(TerrainError::Raw32);
    }
    let floats = (section_height as usize)
        .checked_mul(file_width as usize)
        .and_then(|n| n.checked_mul(section_width as usize))
        .ok_or(TerrainError::Raw32)?;
    floats.checked_mul(4).ok_or(TerrainError::Raw32)
}

/// `TerrainChannel`'s setter, then `TerrainData`'s. Returns whether the cell
/// changed, which is when C# marks the patch tainted.
fn store(cell: &mut f32, value: f32) -> bool {
    if !value.is_finite() {
        return false;
    }
    // IEEE equality: -0 compares equal to the ClearLand +0, so it is not stored.
    if *cell == value {
        return false;
    }
    *cell = value;
    true
}

/// `TerrainData` sizes the taint array by `Size / 16` patches, and
/// `TerrainTaintsArray` allocates one `int` per 32 bits (`calclen`). `Set`
/// indexes that word array. A bit past the patch count still lands in the
/// last allocated word; a word past the array throws.
fn taint_word(width: i32, height: i32, x: i32, y: i32) -> Result<(), TerrainError> {
    const PATCH: i32 = 16;
    let taint_x = width / PATCH;
    let taint_y = height / PATCH;
    let bits = taint_x.checked_mul(taint_y).ok_or(TerrainError::Raw32)?;
    let words = if bits > 0 { ((bits - 1) >> 5) + 1 } else { 0 };
    let bit = (x / PATCH)
        .checked_add(
            (y / PATCH)
                .checked_mul(taint_x)
                .ok_or(TerrainError::Raw32)?,
        )
        .ok_or(TerrainError::Raw32)?;
    let word = bit >> 5;
    if word < 0 || word >= words {
        return Err(TerrainError::Raw32);
    }
    Ok(())
}

fn read_f32(bytes: &[u8], pos: &mut usize) -> Result<f32, TerrainError> {
    let end = pos.checked_add(4).ok_or(TerrainError::Raw32)?;
    let Some(chunk) = bytes.get(*pos..end) else {
        return Err(TerrainError::Raw32);
    };
    let mut buf = [0u8; 4];
    buf.copy_from_slice(chunk);
    *pos = end;
    Ok(f32::from_le_bytes(buf))
}

fn skip(bytes: &[u8], pos: &mut usize, count: usize) {
    let room = bytes.len().saturating_sub(*pos);
    *pos += count.min(room);
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::GzDecoder;

    use super::*;

    fn le_bytes(map: &Heightmap) -> Vec<u8> {
        let mut out = Vec::with_capacity(map.cells().len() * 4);
        for value in map.cells() {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/oar-raw32")
            .join(name)
    }

    fn read(name: &str) -> Vec<u8> {
        std::fs::read(fixture(name)).unwrap()
    }

    #[test]
    fn load_oar_heightmap_matches_pin8() {
        let oar = read("region.oar");
        let member = read("member.r32");
        let mut plain = Vec::new();
        GzDecoder::new(oar.as_slice())
            .read_to_end(&mut plain)
            .unwrap();
        assert!(
            plain.windows(member.len()).any(|window| window == member),
            "the archive member is not the file load oar decoded"
        );
        let first = f32::from_le_bytes(member[0..4].try_into().unwrap());
        assert!(first.is_nan());
        let map = load_raw32(&member).unwrap();
        assert_eq!(map.width(), 256);
        assert_eq!(map.height(), 256);
        assert_eq!(map.cells()[0].to_bits(), 0);
        assert_eq!(le_bytes(&map), read("loaded.r32"));
    }

    #[test]
    fn a_three_hundred_square_trims_to_the_first_region() {
        let input = read("trim-in.r32");
        assert_eq!(input.len(), 300 * 300 * 4);
        let map = load_raw32(&input).unwrap();
        assert_eq!(map.width(), 256);
        assert_eq!(le_bytes(&map), read("trim-out.r32"));
    }

    #[test]
    fn an_offset_section_matches_pin8() {
        let map = load_raw32_section(&read("section-in.r32"), 1, 1, 2, 99, 16, 16).unwrap();
        assert_eq!(map.width(), 16);
        assert_eq!(map.height(), 16);
        assert_eq!(map.cells()[0].to_bits(), 0);
        assert_eq!(map.cells()[1].to_bits(), 0);
        assert_eq!(le_bytes(&map), read("section-out.r32"));
    }

    #[test]
    fn a_zero_section_smaller_than_a_patch_matches_pin8() {
        let map = load_raw32_section(&read("tiny-in.r32"), 0, 0, 1, 1, 4, 4).unwrap();
        assert_eq!(map.width(), 4);
        assert_eq!(map.height(), 4);
        assert_eq!(le_bytes(&map), read("tiny-out.r32"));
    }

    #[test]
    fn a_changed_sample_outside_the_taint_words_fails() {
        let err = load_raw32_section(&read("taint-in.r32"), 0, 0, 1, 1, 4, 4).unwrap_err();
        assert!(matches!(err, TerrainError::Raw32));
        let note = std::fs::read_to_string(fixture("taint.txt")).unwrap();
        assert!(note.contains("IndexOutOfRangeException"), "{note}");
    }

    #[test]
    fn a_sample_in_the_allocated_taint_word_matches_pin8() {
        let map = load_raw32_section(&read("word-in.r32"), 0, 0, 1, 1, 17, 16).unwrap();
        assert_eq!(map.width(), 17);
        assert_eq!(map.height(), 16);
        assert_eq!(le_bytes(&map), read("word-out.r32"));
    }

    #[test]
    fn a_short_file_fails() {
        let err = load_raw32(&read("short.r32")).unwrap_err();
        assert!(matches!(err, TerrainError::Raw32));
        let note = std::fs::read_to_string(fixture("short.txt")).unwrap();
        assert!(note.contains("EndOfStreamException"), "{note}");
    }

    #[test]
    fn a_square_above_the_region_cap_is_refused() {
        assert_eq!(trimmed_edge(4096 * 4096 * 4).unwrap(), 4096);
        assert_eq!(trimmed_edge(4351 * 4351 * 4).unwrap(), 4096);
        let _guard = crate::parcels::limits_lock();
        let before = crate::parcels::limit_hits("terrain.edge", "refuse");
        assert_eq!(section_edge(4096).unwrap(), 4096);
        assert_eq!(crate::parcels::limit_hits("terrain.edge", "refuse"), before);
        let edge: i64 = 4352;
        let len = usize::try_from(edge * edge * 4).unwrap();
        let err = trimmed_edge(len).unwrap_err();
        assert!(matches!(err, TerrainError::Edge));
        assert_eq!(
            crate::parcels::limit_hits("terrain.edge", "refuse"),
            before + 1
        );
        let err = section_edge(4097).unwrap_err();
        assert!(matches!(err, TerrainError::Edge));
        assert_eq!(
            crate::parcels::limit_hits("terrain.edge", "refuse"),
            before + 2
        );
        let err = section_edge(-1).unwrap_err();
        assert!(matches!(err, TerrainError::Raw32));
        assert_eq!(
            crate::parcels::limit_hits("terrain.edge", "refuse"),
            before + 2
        );
    }

    #[test]
    fn a_negative_file_width_with_no_y_offset_reads_the_section() {
        let mut bytes = vec![0u8; 16 * 16 * 4];
        bytes[..4].copy_from_slice(&1.5f32.to_le_bytes());
        let map = load_raw32_section(&bytes, 0, 0, -1, 0, 16, 16).unwrap();
        assert_eq!(map.width(), 16);
        assert_eq!(map.height(), 16);
        assert_eq!(map.cells()[0].to_bits(), 1.5f32.to_bits());
        let err = load_raw32_section(&bytes, 0, 1, -1, 1, 16, 16).unwrap_err();
        assert!(matches!(err, TerrainError::Raw32));
    }
}
