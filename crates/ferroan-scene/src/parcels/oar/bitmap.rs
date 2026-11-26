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
//   Source/OpenSim.Region.CoreModules/World/Land/LandObject.cs
//   (RemapLandBitmap, RemoveFromLandBitmap, ConvertLandBitmapToBytes,
//   ConvertBytesToLandBitmap with overrideRegionSize, IsLandBitmapEmpty)
//   OpenSim-Tranquillity `dev-build` @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The land bitmaps an OAR parcel load reshapes, in four-metre cells.
//!
//! A grid is `bool[x, y]`. Bytes hold the cells row by row from the south,
//! west to east, least significant bit first.

/// `Constants.LandUnit`.
const LAND_UNIT: f32 = 4.0;

/// `ConvertLandBitmapToBytes` writes a last partial byte only below this index.
const PARTIAL_BYTE_LIMIT: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Grid {
    width: usize,
    height: usize,
    cells: Vec<bool>,
}

impl Grid {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![false; width * height],
        }
    }

    fn get(&self, x: usize, y: usize) -> bool {
        self.cells[y * self.width + x]
    }

    fn set(&mut self, x: usize, y: usize) {
        self.cells[y * self.width + x] = true;
    }

    /// `IsLandBitmapEmpty`.
    pub(super) fn is_empty(&self) -> bool {
        !self.cells.contains(&true)
    }

    pub(super) fn size(&self) -> [usize; 2] {
        [self.width, self.height]
    }

    /// `ConvertBytesToLandBitmap(overrideRegionSize: true)`: a square whose
    /// side is the whole square root of the bit count. Bits past the square
    /// are dropped.
    pub(super) fn from_bytes(bytes: &[u8]) -> Self {
        let bits = bytes.len() * 8;
        let side = (bits as f64).sqrt() as usize;
        let mut grid = Self::new(side, side);
        if side == 0 {
            return grid;
        }
        for (index, byte) in bytes.iter().enumerate() {
            for bit in 0..8 {
                let n = index * 8 + bit;
                let (x, y) = (n % side, n / side);
                if y >= side {
                    return grid;
                }
                if byte & (1 << bit) != 0 {
                    grid.set(x, y);
                }
            }
        }
        grid
    }

    /// `ConvertLandBitmapToBytes`. The array holds `width * height / 8`
    /// bytes. A last partial byte with a set bit throws below byte 512 and is
    /// dropped from there on.
    pub(super) fn to_bytes(&self) -> Result<Vec<u8>, BitmapTail> {
        let mut out = vec![0; self.width * self.height / 8];
        let mut byte = 0u8;
        let mut written = 0;
        let mut mask = 1u16;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y) {
                    byte |= mask as u8;
                }
                mask <<= 1;
                if mask == 0x100 {
                    mask = 1;
                    out[written] = byte;
                    written += 1;
                    byte = 0;
                }
            }
        }
        if byte != 0 && written < PARTIAL_BYTE_LIMIT {
            return Err(BitmapTail);
        }
        Ok(out)
    }

    /// `RemoveFromLandBitmap`: clear every cell `taken` holds. The answer is
    /// true when nothing is left.
    pub(super) fn remove(&mut self, taken: &Grid) -> Result<bool, SizeMismatch> {
        if self.size() != taken.size() {
            return Err(SizeMismatch {
                base: self.size(),
                taken: taken.size(),
            });
        }
        for (cell, gone) in self.cells.iter_mut().zip(&taken.cells) {
            if *gone {
                *cell = false;
            }
        }
        Ok(self.is_empty())
    }
}

/// `ConvertLandBitmapToBytes` indexed past its array.
#[derive(Debug)]
pub(super) struct BitmapTail;

/// `RemoveFromLandBitmap` threw: the bitmaps differ in size.
#[derive(Debug)]
pub(super) struct SizeMismatch {
    pub(super) base: [usize; 2],
    pub(super) taken: [usize; 2],
}

/// The placement `RemapLandBitmap` applies, in metres and degrees.
#[derive(Clone, Copy, Debug)]
pub(super) struct Placement {
    pub(super) displacement: [f32; 2],
    pub(super) rotation: f32,
    pub(super) bounding_origin: [f32; 2],
    pub(super) bounding_size: [f32; 2],
    pub(super) region_size: [f32; 2],
}

/// `(int)Math.Floor(value / LandUnit)`. The division is in single
/// precision. The conversion saturates, and NaN is 0, as on .NET 10.
fn cells(value: f32) -> i32 {
    f64::from(value / LAND_UNIT).floor() as i32
}

/// `RemapLandBitmap`: rotate the source about the centre of a working grid
/// half again as large, then copy the bounding rectangle to the displacement
/// in a grid the size of the destination region. The answer is true when the
/// result is empty.
pub(super) fn remap(base: &Grid, place: &Placement) -> (Grid, bool) {
    let base_x = base.width as i32;
    let base_y = base.height as i32;
    let offset_x = base_x / 4;
    let offset_y = base_y / 4;
    let tmp_x = base_x + base_x / 2;
    let tmp_y = base_y + base_y / 2;
    let centre_x = tmp_x / 2;
    let centre_y = tmp_y / 2;
    let mut tmp = Grid::new(tmp_x as usize, tmp_y as usize);

    let radians = std::f64::consts::PI * f64::from(place.rotation) / 180.0;
    let cos = radians.cos();
    let sin = radians.sin();
    let mut degrees = place.rotation;
    if degrees < 0.0 {
        degrees += 360.0;
    }

    // Orthogonal turns are exact. The loops run one past each edge, and a
    // write there is caught and dropped.
    for y in 0..=tmp_y {
        for x in 0..=tmp_x {
            let (sx, sy) = if degrees == 0.0 {
                (x - offset_x, y - offset_y)
            } else if degrees == 90.0 {
                (y - offset_x, tmp_y - 1 - x - offset_y)
            } else if degrees == 180.0 {
                (tmp_x - 1 - x - offset_x, tmp_y - 1 - y - offset_y)
            } else if degrees == 270.0 {
                (tmp_x - 1 - y - offset_x, x - offset_y)
            } else {
                let dx = f64::from(x - centre_x);
                let dy = f64::from(y - centre_y);
                (
                    centre_x + (dx * cos + dy * sin).round_ties_even() as i32 - offset_x,
                    centre_y + (dy * cos - dx * sin).round_ties_even() as i32 - offset_y,
                )
            };
            if (0..base_x).contains(&sx)
                && (0..base_y).contains(&sy)
                && base.get(sx as usize, sy as usize)
                && x < tmp_x
                && y < tmp_y
            {
                tmp.set(x as usize, y as usize);
            }
        }
    }

    let new_x = ((place.region_size[0] / LAND_UNIT) as i32).max(0);
    let new_y = ((place.region_size[1] / LAND_UNIT) as i32).max(0);
    let mut out = Grid::new(new_x as usize, new_y as usize);
    let disp_x = cells(place.displacement[0]);
    let disp_y = cells(place.displacement[1]);
    let start_x = cells(place.bounding_origin[0])
        .wrapping_add(offset_x)
        .clamp(0, tmp_x);
    let start_y = cells(place.bounding_origin[1])
        .wrapping_add(offset_y)
        .clamp(0, tmp_y);
    let end_x = cells(place.bounding_origin[0] + place.bounding_size[0])
        .wrapping_add(offset_x)
        .clamp(0, tmp_x);
    let end_y = cells(place.bounding_origin[1] + place.bounding_size[1])
        .wrapping_add(offset_y)
        .clamp(0, tmp_y);

    let mut empty = true;
    for y in start_y..end_y {
        for x in start_x..end_x {
            let dx = x.wrapping_sub(start_x).wrapping_add(disp_x);
            let dy = y.wrapping_sub(start_y).wrapping_add(disp_y);
            if (0..new_x).contains(&dx)
                && (0..new_y).contains(&dy)
                && tmp.get(x as usize, y as usize)
            {
                out.set(dx as usize, dy as usize);
                empty = false;
            }
        }
    }
    (out, empty)
}
