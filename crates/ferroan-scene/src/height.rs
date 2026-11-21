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
//   Source/OpenSim.Framework/TerrainData.cs
//     (GetHeight, FromCompressedTerrainSerializationV2D,
//      FromCompressedTerrainSerializationV2DGZip, GetPatchMinMax, GetPatchBlock)
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs (MakeRootAgent)
//   KanikaBreed/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Heightmap stored the way `TerrainData` serializes it: Y outer, X inner.
//! A truncated or unknown blob fails closed. C# would `ClearLand()` to 21
//! and keep running; this row refuses to start on a damaged store.

use std::io::Read;

use ferroan_diag::{Kind, TERRAIN_BLOB, TERRAIN_EDGE};
use flate2::read::GzDecoder;

use crate::error::TerrainError;

/// `TerrainData.DefaultTerrainHeight`.
pub const DEFAULT_HEIGHT: f32 = 21.0;

/// `Constants.MaximumRegionSize`.
pub const MAX_EDGE: i32 = 4096;

/// Eight-byte header plus one float for every cell of a 4096 metre map.
pub const BLOB_CAP: usize = 8 + 4096 * 4096 * 4;

const PATCH: i32 = 16;
const REVISION_V2: i32 = 22;
const REVISION_V2_GZIP: i32 = 23;

/// One region's heightmap. Index is `y * width + x`.
#[derive(Clone, Debug)]
pub struct Heightmap {
    width: i32,
    height: i32,
    cells: Vec<f32>,
}

impl Heightmap {
    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn cells(&self) -> &[f32] {
        &self.cells
    }

    /// `TerrainData.CreateFromDatabaseBlobFactory` for revisions 22 and 23.
    ///
    /// `width` and `height` are the region size. A blob that names a smaller
    /// map leaves the rest at 21. A larger blob is clipped to the region.
    pub fn from_blob(
        revision: i32,
        blob: &[u8],
        width: i32,
        height: i32,
    ) -> Result<Self, TerrainError> {
        check_edge(width)?;
        check_edge(height)?;
        if blob.len() > BLOB_CAP {
            refuse_blob(blob.len());
            return Err(TerrainError::Blob);
        }
        let plain = match revision {
            REVISION_V2 => blob.to_vec(),
            REVISION_V2_GZIP => gunzip(blob)?,
            other => return Err(TerrainError::Revision(other)),
        };
        if plain.len() > BLOB_CAP {
            refuse_blob(plain.len());
            return Err(TerrainError::Blob);
        }
        decode_v2(&plain, width, height)
    }

    /// `TerrainData.GetHeight`. Edges clamp. Interior cells split on `dy > dx`.
    pub fn height_at(&self, x: f32, y: f32) -> f32 {
        let mix = self.width - 1;
        let miy = self.height - 1;
        if x <= 0.0 {
            return self.edge_y(0, y, miy);
        }
        let ix = x as i32;
        if ix >= mix {
            return self.edge_y(mix, y, miy);
        }
        let dx = x - ix as f32;
        if y <= 0.0 {
            let h = self.cell(ix, 0);
            return h + (self.cell(ix + 1, 0) - h) * dx;
        }
        let iy = y as i32;
        if iy >= miy {
            let h = self.cell(ix, miy);
            return h + (self.cell(ix + 1, miy) - h) * dx;
        }
        let dy = y - iy as f32;
        let h0 = self.cell(ix, iy);
        if dy > dx {
            let h2 = self.cell(ix, iy + 1);
            let h1 = (h2 - h0) * dy;
            let h2 = (self.cell(ix + 1, iy + 1) - h2) * dx;
            h0 + h1 + h2
        } else {
            let h2 = self.cell(ix + 1, iy);
            let h1 = (h2 - h0) * dx;
            let h2 = (self.cell(ix + 1, iy + 1) - h2) * dy;
            h0 + h1 + h2
        }
    }

    /// `GetPatchMinMax`, including the last cell of the first column.
    pub(crate) fn patch_min_max(&self, px: i32, py: i32) -> (f32, f32) {
        let x0 = px * PATCH;
        let y0 = py * PATCH;
        let mut min = self.cell(x0, y0);
        let mut max = min;
        for x in x0..x0 + PATCH {
            for y in y0..y0 + PATCH {
                let value = self.cell(x, y);
                if value > max {
                    max = value;
                } else if value < min {
                    min = value;
                }
            }
        }
        (min, max)
    }

    /// `GetPatchBlock`: Y outer, X inner, after the DC and scale.
    pub(crate) fn patch_block(&self, px: i32, py: i32, sub: f32, premult: f32) -> [f32; 256] {
        let mut block = [0.0; 256];
        let mut k = 0;
        let x0 = px * PATCH;
        let y0 = py * PATCH;
        for y in y0..y0 + PATCH {
            for x in x0..x0 + PATCH {
                block[k] = (self.cell(x, y) - sub) * premult;
                k += 1;
            }
        }
        block
    }

    fn edge_y(&self, x: i32, y: f32, miy: i32) -> f32 {
        if y <= 0.0 {
            return self.cell(x, 0);
        }
        let iy = y as i32;
        // The right edge compares `y`, the top edge compares `iy`. Both are C#.
        if x == self.width - 1 {
            if y >= miy as f32 {
                return self.cell(x, miy);
            }
        } else if iy >= miy {
            return self.cell(x, miy);
        }
        let dy = y - iy as f32;
        let h = self.cell(x, iy);
        h + (self.cell(x, iy + 1) - h) * dy
    }

    fn cell(&self, x: i32, y: i32) -> f32 {
        let index = (y as usize) * (self.width as usize) + (x as usize);
        self.cells.get(index).copied().unwrap_or(DEFAULT_HEIGHT)
    }
}

/// `MakeRootAgent` with no appearance and no physics: half-height is 0.8.
/// The avatar is lifted only when `height + 0.01 + 0.8` is above the start Z.
pub fn movement_z(start_z: f32, ground: f32) -> f32 {
    let lifted = ground + 0.01 + 0.8;
    if lifted > start_z { lifted } else { start_z }
}

fn check_edge(side: i32) -> Result<(), TerrainError> {
    if (16..=MAX_EDGE).contains(&side) && side % PATCH == 0 {
        return Ok(());
    }
    ferroan_diag::refuse(
        TERRAIN_EDGE,
        Kind::Hard,
        MAX_EDGE as u64,
        u64::try_from(side).unwrap_or(u64::MAX),
        "terrain",
    );
    Err(TerrainError::Edge)
}

fn refuse_blob(observed: usize) {
    ferroan_diag::refuse(
        TERRAIN_BLOB,
        Kind::Hard,
        BLOB_CAP as u64,
        observed as u64,
        "terrain",
    );
}

fn gunzip(blob: &[u8]) -> Result<Vec<u8>, TerrainError> {
    let mut decoder = GzDecoder::new(blob);
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match decoder.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if out.len().saturating_add(n) > BLOB_CAP {
                    refuse_blob(out.len().saturating_add(n));
                    return Err(TerrainError::Blob);
                }
                out.extend_from_slice(&buf[..n]);
            }
            // ToCompressedTerrainSerializationV2DGzip copies the member and
            // calls ToArray before GZipStream.Dispose writes the trailer.
            // A complete heightmap is the blob a region saved.
            Err(_) if v2_complete(&out) => break,
            Err(_) => return Err(TerrainError::Damaged),
        }
    }
    if !v2_complete(&out) {
        return Err(TerrainError::Damaged);
    }
    Ok(out)
}

fn v2_complete(plain: &[u8]) -> bool {
    if plain.len() < 8 {
        return false;
    }
    let Ok(x_bytes) = plain[0..4].try_into() else {
        return false;
    };
    let Ok(y_bytes) = plain[4..8].try_into() else {
        return false;
    };
    let sx = i32::from_le_bytes(x_bytes);
    let sy = i32::from_le_bytes(y_bytes);
    if sx <= 0 || sy <= 0 {
        return false;
    }
    let need = 8_u64.saturating_add((sx as u64).saturating_mul(sy as u64).saturating_mul(4));
    plain.len() as u64 >= need && need <= BLOB_CAP as u64
}

fn decode_v2(plain: &[u8], width: i32, height: i32) -> Result<Heightmap, TerrainError> {
    if plain.len() < 8 {
        return Err(TerrainError::Damaged);
    }
    let hm_x = i32::from_le_bytes(plain[0..4].try_into().map_err(|_| TerrainError::Damaged)?);
    let hm_y = i32::from_le_bytes(plain[4..8].try_into().map_err(|_| TerrainError::Damaged)?);
    if hm_x <= 0 || hm_y <= 0 {
        return Err(TerrainError::Damaged);
    }
    let floats = (hm_x as u64).saturating_mul(hm_y as u64);
    if floats.saturating_mul(4).saturating_add(8) > BLOB_CAP as u64 {
        refuse_blob(BLOB_CAP.saturating_add(1));
        return Err(TerrainError::Blob);
    }
    Ok(Heightmap {
        width,
        height,
        cells: read_clipped(plain, width, height)?,
    })
}

fn read_clipped(plain: &[u8], width: i32, height: i32) -> Result<Vec<f32>, TerrainError> {
    let mut hm_x = i32::from_le_bytes(plain[0..4].try_into().map_err(|_| TerrainError::Damaged)?);
    let mut hm_y = i32::from_le_bytes(plain[4..8].try_into().map_err(|_| TerrainError::Damaged)?);
    if hm_x <= 0 || hm_y <= 0 {
        return Err(TerrainError::Damaged);
    }
    let stored_x = hm_x as usize;
    let stored_y = hm_y as usize;
    let need = 8usize
        .checked_add(
            stored_x
                .checked_mul(stored_y)
                .and_then(|n| n.checked_mul(4))
                .ok_or(TerrainError::Damaged)?,
        )
        .ok_or(TerrainError::Damaged)?;
    if plain.len() < need {
        return Err(TerrainError::Damaged);
    }
    // `BinaryReader` reads the clipped counts in file order. It does not skip
    // to the next stored row when the blob is wider than the region.
    if hm_x > width {
        hm_x = width;
    }
    if hm_y > height {
        hm_y = height;
    }
    let len = (width as usize)
        .checked_mul(height as usize)
        .ok_or(TerrainError::Edge)?;
    let mut cells = vec![DEFAULT_HEIGHT; len];
    let mut offset = 8;
    for yy in 0..hm_y {
        for xx in 0..hm_x {
            let value = f32::from_le_bytes(
                plain
                    .get(offset..offset + 4)
                    .ok_or(TerrainError::Damaged)?
                    .try_into()
                    .map_err(|_| TerrainError::Damaged)?,
            );
            offset += 4;
            let index = (yy as usize) * (width as usize) + (xx as usize);
            if let Some(slot) = cells.get_mut(index) {
                *slot = value;
            }
        }
    }
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(side: i32, height: f32) -> Heightmap {
        let n = (side as usize) * (side as usize);
        Heightmap {
            width: side,
            height: side,
            cells: vec![height; n],
        }
    }

    #[test]
    fn bilinear_matches_the_interior_split() {
        let mut map = flat(16, 10.0);
        map.cells[0] = 10.0;
        map.cells[1] = 20.0;
        map.cells[16] = 30.0;
        map.cells[17] = 40.0;
        let dy_gt = map.height_at(0.2, 0.5);
        let dx_gt = map.height_at(0.5, 0.2);
        assert!((dy_gt - 22.0).abs() < 0.001);
        assert!((dx_gt - 19.0).abs() < 0.001);
        assert_eq!(map.height_at(-1.0, -1.0), 10.0);
        assert_eq!(map.height_at(100.0, 100.0), 10.0);
    }

    #[test]
    fn movement_lifts_only_when_the_ground_is_above_the_start() {
        let lifted = movement_z(10.0, 21.0);
        assert!((lifted - (21.0 + 0.01 + 0.8)).abs() < f32::EPSILON);
        assert_eq!(movement_z(30.0, 21.0), 30.0);
    }

    #[test]
    fn patch_minmax_includes_the_last_cell_of_the_first_column() {
        let mut map = flat(16, 21.0);
        map.cells[15 * 16] = 40.0;
        let (min, max) = map.patch_min_max(0, 0);
        assert_eq!(min, 21.0);
        assert_eq!(max, 40.0);
    }

    #[test]
    fn gzip_roundtrip_keeps_the_extreme_cell() {
        let mut plain = Vec::new();
        plain.extend_from_slice(&16i32.to_le_bytes());
        plain.extend_from_slice(&16i32.to_le_bytes());
        for y in 0..16 {
            for x in 0..16 {
                let h: f32 = if x == 0 && y == 15 { 40.0 } else { 21.0 };
                plain.extend_from_slice(&h.to_le_bytes());
            }
        }
        let mut gzip = Vec::new();
        {
            use flate2::Compression;
            use flate2::write::GzEncoder;
            use std::io::Write;
            let mut enc = GzEncoder::new(&mut gzip, Compression::default());
            enc.write_all(&plain).unwrap();
            enc.finish().unwrap();
        }
        let map = Heightmap::from_blob(23, &gzip, 16, 16).unwrap();
        assert_eq!(map.cells[15 * 16], 40.0);
        assert_eq!(map.cells[0], 21.0);
    }

    #[test]
    fn unknown_revision_and_short_blob_fail_closed() {
        assert!(matches!(
            Heightmap::from_blob(11, &[0; 8], 16, 16),
            Err(TerrainError::Revision(11))
        ));
        assert!(matches!(
            Heightmap::from_blob(22, &[0; 4], 16, 16),
            Err(TerrainError::Damaged)
        ));
        assert!(matches!(
            Heightmap::from_blob(22, &[0; 8], 15, 16),
            Err(TerrainError::Edge)
        ));
    }
}
