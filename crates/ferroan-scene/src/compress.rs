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
//   Source/OpenSim.Region.Framework/Scenes/TerrainCompressor.cs
//     (CreatePatchFromTerrainData, dct16x16, EncodePatch)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (SendLayerTopRight, SendLayerBottomLeft, SendLayerData)
//   KanikaBreed/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// The commented IDCT in that file is not the live path and is not ported.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Land `LayerData` bodies. Each packet is one patch, in the spiral
//! `SendLayerData` uses, so the 900-byte split does not run.

use crate::bits::BitPack;
use crate::height::Heightmap;

const STRIDE: u32 = 264;
const END_OF_PATCHES: u8 = 97;
const ZERO_CODE: u8 = 0;
const ZERO_EOB: u32 = 2;
const POSITIVE: u32 = 6;
const NEGATIVE: u32 = 7;
const LAND: u8 = 0x4c;
const LAND_EXTENDED: u8 = 0x4d;
const REGION: i32 = 256;

// TerrainCompressor.cs writes these double literals. Shortening them changes
// the f32 product and the captured LayerData bytes.
#[allow(clippy::excessive_precision, clippy::approx_constant)]
mod dct_scale {
    pub(super) const C16_1R: f32 = 0.35185093438159561476_f32 * 2.82842712474619_f32;
    pub(super) const C16_1I: f32 = 0.03465429229977286565_f32 * 2.82842712474619_f32;
    pub(super) const C16_2R: f32 = 0.34675996133053686546_f32 * 2.82842712474619_f32;
    pub(super) const C16_2I: f32 = 0.06897484482073575308_f32 * 2.82842712474619_f32;
    pub(super) const C16_3R: f32 = 0.33832950029358816957_f32 * 2.82842712474619_f32;
    pub(super) const C16_3I: f32 = 0.10263113188058934529_f32 * 2.82842712474619_f32;
    pub(super) const C16_4R: f32 = 0.32664074121909413196_f32 * 2.82842712474619_f32;
    pub(super) const C16_4I: f32 = 0.13529902503654924610_f32 * 2.82842712474619_f32;
    pub(super) const C16_5R: f32 = 0.31180625324666780814_f32 * 2.82842712474619_f32;
    pub(super) const C16_5I: f32 = 0.16666391461943662432_f32 * 2.82842712474619_f32;
    pub(super) const C16_6R: f32 = 0.29396890060483967924_f32 * 2.82842712474619_f32;
    pub(super) const C16_6I: f32 = 0.19642373959677554532_f32 * 2.82842712474619_f32;
    pub(super) const C16_7R: f32 = 0.27330046675043937206_f32 * 2.82842712474619_f32;
    pub(super) const C16_7I: f32 = 0.22429189658565907106_f32 * 2.82842712474619_f32;
    pub(super) const C16_8R: f32 = 0.25_f32 * 2.82842712474619_f32;
    pub(super) const W16_4R: f32 = 0.92387953251128675613_f32;
    pub(super) const W16_4I: f32 = 0.38268343236508977173_f32;
    pub(super) const W16_8R: f32 = 0.70710678118654752440_f32;
}
use dct_scale::{
    C16_1I, C16_1R, C16_2I, C16_2R, C16_3I, C16_3R, C16_4I, C16_4R, C16_5I, C16_5R, C16_6I, C16_6R,
    C16_7I, C16_7R, C16_8R, W16_4I, W16_4R, W16_8R,
};

/// One reliable land packet: layer type plus the bitpack body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainLayer {
    pub kind: u8,
    pub data: Vec<u8>,
}

/// Patches in `SendLayerTopRight` / `SendLayerBottomLeft` order.
pub fn land_layers(map: &Heightmap) -> Vec<TerrainLayer> {
    let large = map.width() > REGION || map.height() > REGION;
    let kind = if large { LAND_EXTENDED } else { LAND };
    let quant = quantize_table();
    let copy = copy_matrix();
    let mut layers = Vec::new();
    for (px, py) in spiral(map.width() / 16, map.height() / 16) {
        let mut bits = BitPack::new();
        bits.pack_bits(STRIDE, 16);
        bits.pack_byte(16);
        bits.pack_byte(kind);
        write_patch(&mut bits, map, px, py, large, &quant, &copy);
        bits.pack_byte(END_OF_PATCHES);
        layers.push(TerrainLayer {
            kind,
            data: bits.bytes().to_vec(),
        });
    }
    layers
}

/// Each patch's bits as [`land_layers`] packs it, without the layer header
/// and end marker, in the same order. C# packs several patches into one
/// `LayerData`, so captured bodies are compared patch by patch.
#[cfg(test)]
pub(crate) fn land_patch_bits(map: &Heightmap) -> Vec<((i32, i32), Vec<bool>)> {
    let large = map.width() > REGION || map.height() > REGION;
    let quant = quantize_table();
    let copy = copy_matrix();
    spiral(map.width() / 16, map.height() / 16)
        .into_iter()
        .map(|(px, py)| {
            let mut bits = BitPack::new();
            write_patch(&mut bits, map, px, py, large, &quant, &copy);
            ((px, py), bits.bits())
        })
        .collect()
}

fn write_patch(
    output: &mut BitPack,
    map: &Heightmap,
    px: i32,
    py: i32,
    large: bool,
    quant: &[f32; 256],
    copy: &[usize; 256],
) {
    let (zmin, zmax) = map.patch_min_max(px, py);
    let frange = (zmax - zmin) + 1.0;
    let ids = patch_ids(px, py, large);
    if rounds_to_one(frange) {
        output.pack_byte(0);
        output.pack_float(zmin - 0.5);
        output.pack_byte(1);
        output.pack_byte(0);
        output.pack_bits(ids, if large { 32 } else { 10 });
        output.pack_bits(ZERO_EOB, 2);
        return;
    }
    let range = frange as i32;
    let (coeffs, wbits) = compress_patch(map, px, py, range, zmin, quant, copy);
    encode_header(output, zmin, range, ids, large, wbits);
    encode_patch(output, &coeffs, wbits);
}

#[allow(clippy::float_cmp)]
fn rounds_to_one(frange: f32) -> bool {
    // Math.Round(double, digits) default is midpoint-to-even. The float is widened first.
    let rounded = (f64::from(frange) * 100.0).round_ties_even() / 100.0;
    rounded == 1.0
}

fn patch_ids(px: i32, py: i32, large: bool) -> u32 {
    if large {
        ((px << 16) | (py & 0xffff)) as u32
    } else {
        ((px << 5) | (py & 0x1f)) as u32
    }
}

fn compress_patch(
    map: &Heightmap,
    px: i32,
    py: i32,
    range: i32,
    dc: f32,
    quant: &[f32; 256],
    copy: &[usize; 256],
) -> ([i32; 256], i32) {
    let range_f = range as f32;
    let premult = 1024.0_f32 / range_f;
    let sub = 0.5 * range_f + dc;
    let block = map.patch_block(px, py, sub, premult);
    dct16(&block, quant, copy)
}

fn encode_header(output: &mut BitPack, dc: f32, range: i32, ids: u32, large: bool, mut wbits: i32) {
    wbits = wbits.clamp(2, 17);
    // CompressPatch stored wordsize 8 in both nibbles (0x88). The header keeps the high nibble.
    let quant_bits = 0x80 | (wbits - 2);
    output.pack_byte(quant_bits as u8);
    output.pack_float(dc);
    output.pack_bits(range as u32, 16);
    output.pack_bits(ids, if large { 32 } else { 10 });
}

fn encode_patch(output: &mut BitPack, patch: &[i32; 256], wbits: i32) {
    let max_size = (1_i32 << wbits) - 1;
    let mut i = 0;
    while i < 256 {
        let temp = patch[i];
        if temp == 0 {
            let mut j = i + 1;
            while j < 256 && patch[j] == 0 {
                j += 1;
            }
            if j == 256 {
                output.pack_bits(ZERO_EOB, 2);
                return;
            }
            let mut run = j - i;
            while run > 8 {
                output.pack_byte(ZERO_CODE);
                run -= 8;
            }
            if run > 0 {
                output.pack_counted(ZERO_CODE, run as i32);
            }
            i = j;
            continue;
        }
        let magnitude = if temp < 0 {
            output.pack_bits(NEGATIVE, 3);
            temp.saturating_neg()
        } else {
            output.pack_bits(POSITIVE, 3);
            temp
        };
        let packed = if magnitude > max_size {
            max_size
        } else {
            magnitude
        };
        output.pack_bits(packed as u32, wbits);
        i += 1;
    }
}

fn quantize_table() -> [f32; 256] {
    let mut quant = [0.0; 256];
    let scale = 2.0_f32 / 16.0;
    for j in 0..16 {
        for i in 0..16 {
            let tmp = 1.0 + 2.0 * (i + j) as f32;
            quant[j * 16 + i] = scale / tmp;
        }
    }
    quant
}

fn copy_matrix() -> [usize; 256] {
    let mut copy = [0; 256];
    let mut diag = false;
    let mut right = true;
    let mut i = 0_i32;
    let mut j = 0_i32;
    let mut count = 0_usize;
    while i < 16 && j < 16 {
        copy[(j * 16 + i) as usize] = count;
        count += 1;
        if !diag {
            if right {
                if i < 15 {
                    i += 1;
                } else {
                    j += 1;
                }
                right = false;
                diag = true;
            } else if j < 15 {
                j += 1;
                right = true;
                diag = true;
            } else {
                i += 1;
                right = true;
                diag = true;
            }
        } else if right {
            i += 1;
            j -= 1;
            if i == 15 || j == 0 {
                diag = false;
            }
        } else {
            i -= 1;
            j += 1;
            if j == 15 || i == 0 {
                diag = false;
            }
        }
    }
    copy
}

fn dct16(block: &[f32; 256], quant: &[f32; 256], copy: &[usize; 256]) -> ([i32; 256], i32) {
    let mut tmp = [0.0_f32; 256];
    for column in 0..16 {
        let mut row = [0.0; 16];
        let start = column * 16;
        row.copy_from_slice(&block[start..start + 16]);
        let freq = dct_line(&row);
        for (bin, value) in freq.into_iter().enumerate() {
            tmp[bin * 16 + column] = value;
        }
    }
    let mut coeffs = [0_i32; 256];
    let mut wbits = 5;
    for column in 0..16 {
        let mut row = [0.0; 16];
        let start = column * 16;
        row.copy_from_slice(&tmp[start..start + 16]);
        let freq = dct_line(&row);
        for (bin, value) in freq.into_iter().enumerate() {
            let index = bin * 16 + column;
            let scaled = value * quant[index];
            let itmp = scaled as i32;
            if let Some(slot) = coeffs.get_mut(copy[index]) {
                *slot = itmp;
            }
            note_wbits(&mut wbits, itmp);
        }
    }
    (coeffs, wbits)
}

fn note_wbits(wbits: &mut i32, itmp: i32) {
    if *wbits >= 17 {
        return;
    }
    let abs = itmp.saturating_abs();
    let mut max_value = 1_i32 << *wbits;
    while abs > max_value {
        *wbits += 1;
        if *wbits == 17 {
            break;
        }
        max_value = 1_i32 << *wbits;
    }
}

#[allow(clippy::similar_names, clippy::too_many_lines)]
fn dct_line(a: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    let mut x4r = a[0] - a[15];
    let mut xr = a[0] + a[15];
    let mut x4i = a[8] - a[7];
    let mut xi = a[8] + a[7];
    let x0r = xr + xi;
    let x0i = xr - xi;
    let mut x5r = a[2] - a[13];
    xr = a[2] + a[13];
    let mut x5i = a[10] - a[5];
    xi = a[10] + a[5];
    let x1r = xr + xi;
    let x1i = xr - xi;
    let mut x6r = a[4] - a[11];
    xr = a[4] + a[11];
    let mut x6i = a[12] - a[3];
    xi = a[12] + a[3];
    let x2r = xr + xi;
    let x2i = xr - xi;
    let mut x7r = a[6] - a[9];
    xr = a[6] + a[9];
    let mut x7i = a[14] - a[1];
    xi = a[14] + a[1];
    let x3r = xr + xi;
    let x3i = xr - xi;
    xr = x0r + x2r;
    xi = x1r + x3r;
    out[0] = C16_8R * (xr + xi);
    out[8] = C16_8R * (xr - xi);
    xr = x0r - x2r;
    xi = x1r - x3r;
    out[4] = C16_4R * xr - C16_4I * xi;
    out[12] = C16_4R * xi + C16_4I * xr;
    x0r_pair(&mut out, x0i, x1i, x2i, x3i);
    xr = W16_8R * (x6r - x6i);
    xi = W16_8R * (x6i + x6r);
    x6r = x4r - xr;
    x6i = x4i - xi;
    x4r += xr;
    x4i += xi;
    xr = W16_4I * x7r - W16_4R * x7i;
    xi = W16_4I * x7i + W16_4R * x7r;
    x7r = W16_4R * x5r - W16_4I * x5i;
    x7i = W16_4R * x5i + W16_4I * x5r;
    x5r = x7r + xr;
    x5i = x7i + xi;
    x7r -= xr;
    x7i -= xi;
    xr = x4r + x5r;
    xi = x5i + x4i;
    out[1] = C16_1R * xr - C16_1I * xi;
    out[15] = C16_1R * xi + C16_1I * xr;
    xr = x4r - x5r;
    xi = x5i - x4i;
    out[7] = C16_7R * xr - C16_7I * xi;
    out[9] = C16_7R * xi + C16_7I * xr;
    xr = x6r - x7i;
    xi = x7r + x6i;
    out[5] = C16_5R * xr - C16_5I * xi;
    out[11] = C16_5R * xi + C16_5I * xr;
    xr = x6r + x7i;
    xi = x7r - x6i;
    out[3] = C16_3R * xr - C16_3I * xi;
    out[13] = C16_3R * xi + C16_3I * xr;
    out
}

fn x0r_pair(out: &mut [f32; 16], x0i: f32, x1i: f32, x2i: f32, x3i: f32) {
    let x0r = W16_8R * (x1i - x3i);
    let x2r = W16_8R * (x1i + x3i);
    let mut xr = x0i + x0r;
    let mut xi = x2r + x2i;
    out[2] = C16_2R * xr - C16_2I * xi;
    out[14] = C16_2R * xi + C16_2I * xr;
    xr = x0i - x0r;
    xi = x2r - x2i;
    out[6] = C16_6R * xr - C16_6I * xi;
    out[10] = C16_6R * xi + C16_6I * xr;
}

fn spiral(nx: i32, ny: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    if nx > 0 && ny > 0 {
        top_right(&mut out, 0, 0, nx - 1, ny - 1);
    }
    out
}

fn top_right(out: &mut Vec<(i32, i32)>, x1: i32, y1: i32, x2: i32, y2: i32) {
    let mut x = x1;
    while x <= x2 {
        out.push((x, y1));
        x += 1;
    }
    let mut y = y1 + 1;
    while y <= y2 {
        out.push((x2, y));
        y += 1;
    }
    if x2 - x1 > 0 && y2 - y1 > 0 {
        bottom_left(out, x1, y1 + 1, x2 - 1, y2);
    }
}

fn bottom_left(out: &mut Vec<(i32, i32)>, x1: i32, y1: i32, x2: i32, y2: i32) {
    let mut x = x2;
    while x >= x1 {
        out.push((x, y2));
        x -= 1;
    }
    let mut y = y2 - 1;
    while y >= y1 {
        out.push((x1, y));
        y -= 1;
    }
    if x2 - x1 > 0 && y2 - y1 > 0 {
        top_right(out, x1 + 1, y1, x2, y2 - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::height::Heightmap;

    fn filled(side: i32, height: f32) -> Heightmap {
        Heightmap::from_blob(22, &plain(side, move |_, _| height), side, side).unwrap()
    }

    fn plain(side: i32, height: impl Fn(i32, i32) -> f32) -> Vec<u8> {
        let mut plain = Vec::new();
        plain.extend_from_slice(&side.to_le_bytes());
        plain.extend_from_slice(&side.to_le_bytes());
        for y in 0..side {
            for x in 0..side {
                plain.extend_from_slice(&height(x, y).to_le_bytes());
            }
        }
        plain
    }

    #[test]
    fn spiral_visits_each_patch_once() {
        let patches = spiral(16, 16);
        assert_eq!(patches.len(), 256);
        assert_eq!(patches[0], (0, 0));
        assert!(patches.contains(&(15, 15)));
        assert_eq!(spiral(1, 1), vec![(0, 0)]);
    }

    #[test]
    fn extreme_on_the_first_columns_last_cell_is_not_flat() {
        let flat = filled(16, 21.0);
        let extreme = Heightmap::from_blob(
            22,
            &plain(16, |x, y| if x == 0 && y == 15 { 40.0 } else { 21.0 }),
            16,
            16,
        )
        .unwrap();
        let flat_layers = land_layers(&flat);
        let extreme_layers = land_layers(&extreme);
        assert_eq!(flat_layers.len(), 1);
        assert_eq!(extreme_layers.len(), 1);
        assert_ne!(flat_layers[0].data, extreme_layers[0].data);
        assert_eq!(extreme_layers[0].kind, LAND);
    }

    #[test]
    fn captured_csharp_region_layers_match() {
        // Captured from pin c70fee205f64ee8672a8d607cef6a733931da951.
        // TerrainData 256² cleared to 21, then [0,15] and [128,128] set to 40.
        // GetDatabaseBlob wrote revision 23. Layer bodies are SendLayerData's
        // bitpack, one patch per packet, spiral order.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/terrain");
        let blob = std::fs::read(root.join("terrain.bin")).unwrap();
        let packed = std::fs::read(root.join("layers.bin")).unwrap();
        let map = Heightmap::from_blob(23, &blob, 256, 256).unwrap();
        assert_eq!(map.height_at(0.0, 15.0), 40.0);
        assert_eq!(map.height_at(128.0, 128.0), 40.0);
        let lifted = crate::movement_z(25.0, map.height_at(128.0, 128.0));
        assert_eq!(lifted.to_bits(), 1_109_605_744);
        let layers = land_layers(&map);
        let count = u32::from_le_bytes(packed[0..4].try_into().unwrap()) as usize;
        let mut offset = 4;
        assert_eq!(layers.len(), count);
        for (index, layer) in layers.iter().enumerate() {
            let len = u32::from_le_bytes(packed[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            let body = &packed[offset..offset + len];
            offset += len;
            assert_eq!(layer.kind, LAND);
            assert_eq!(
                layer.data,
                body,
                "patch {index} differs (rust {} csharp {})",
                layer.data.len(),
                body.len()
            );
        }
    }
}
