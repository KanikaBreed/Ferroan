/*
 * Copyright (c) 2006-2016, openmetaverse.co
 * All rights reserved.
 *
 * - Redistribution and use in source and binary forms, with or without
 *   modification, are permitted provided that the following conditions are met:
 *
 * - Redistributions of source code must retain the above copyright notice, this
 *   list of conditions and the following disclaimer.
 * - Neither the name of the openmetaverse.co nor the names
 *   of its contributors may be used to endorse or promote products derived from
 *   this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 * ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
 * LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
 * CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
 * SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
 * INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
 * CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
 * ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
 * POSSIBILITY OF SUCH DAMAGE.
 */

// Ported to Rust for Ferroan from
//   OpenMetaverse/Primitives/Primitive.cs (ConstructionData profile nibbles,
//   PackBeginCut, PackEndCut, PackPathScale, PackPathShear, PackPathTwist,
//   PackPathTaper, PackPathRevolutions, PackProfileHollow, and the Unpack*
//   partners)
//   OpenMetaverse.Types/UtilsConversions.cs (SwapNibbles)
//   OpenMetaverse.Types/EnumsPrimitive.cs (PathCurve, ProfileCurve, HoleType)
//   libopenmetaverse/data/message_template.msg (ObjectAdd ObjectData field order)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Prim shape fields from an `ObjectAdd` / `ObjectShape` object block.
//!
//! `MathF.Round` rounds a midpoint to the even integer. The following cast
//! keeps the low bits of that integer (`(byte)256f` is `0`). Rust float casts
//! saturate, so these functions round first and then wrap the integer.
//!
//! Aurora's `pack.rs` uses `f32::round` (midpoints away from zero) and clamps.
//! Those bytes are not OpenMetaverse's, so that packer is not copied.

use crate::wire::{WireError, put_u16, read_u16};

/// Bytes of the shape fields in `ObjectAdd`'s `ObjectData` block, from
/// `PathCurve` through `ProfileHollow`.
pub const SHAPE_LEN: usize = 23;

const CUT_QUANTA: f32 = 0.00002;
const CUT_INV: f32 = 1.0 / CUT_QUANTA;
const SCALE_QUANTA: f32 = 0.01;
const SCALE_INV: f32 = 1.0 / SCALE_QUANTA;
const SHEAR_QUANTA: f32 = 0.01;
const SHEAR_INV: f32 = 1.0 / SHEAR_QUANTA;
const TAPER_QUANTA: f32 = 0.01;
const TAPER_INV: f32 = 1.0 / TAPER_QUANTA;
const REV_QUANTA: f32 = 0.015;
const REV_INV: f32 = 1.0 / REV_QUANTA;
const HOLLOW_QUANTA: f32 = 0.00002;
const HOLLOW_INV: f32 = 1.0 / HOLLOW_QUANTA;

const PROFILE_MASK: u8 = 0x0f;
const HOLE_MASK: u8 = 0xf0;

/// `PathCurve.Line`. A new box uses this.
pub const PATH_LINE: u8 = 0x10;
/// `PathCurve.Circle`.
pub const PATH_CIRCLE: u8 = 0x20;
/// `ProfileCurve.Square`.
pub const PROFILE_SQUARE: u8 = 0x01;
/// `HoleType.Triangle`.
pub const HOLE_TRIANGLE: u8 = 0x30;

/// Float shape parameters. `to_bytes` quantizes them. `from_bytes` stores the
/// unpacked floats, which are not always the floats that were packed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeParams {
    pub path_curve: u8,
    pub profile_curve: u8,
    pub path_begin: f32,
    pub path_end: f32,
    pub path_scale_x: f32,
    pub path_scale_y: f32,
    pub path_shear_x: f32,
    pub path_shear_y: f32,
    pub path_twist: f32,
    pub path_twist_begin: f32,
    pub path_radius_offset: f32,
    pub path_taper_x: f32,
    pub path_taper_y: f32,
    pub path_revolutions: f32,
    pub path_skew: f32,
    pub profile_begin: f32,
    pub profile_end: f32,
    pub profile_hollow: f32,
}

impl ShapeParams {
    /// `Primitive.Pack*` of these floats, in `ObjectAdd` field order.
    pub fn to_bytes(self) -> [u8; SHAPE_LEN] {
        let mut out = [0u8; SHAPE_LEN];
        out[0] = self.path_curve;
        out[1] = self.profile_curve;
        put_u16(&mut out[2..4], pack_begin_cut(self.path_begin));
        put_u16(&mut out[4..6], pack_end_cut(self.path_end));
        out[6] = pack_path_scale(self.path_scale_x);
        out[7] = pack_path_scale(self.path_scale_y);
        out[8] = pack_path_shear(self.path_shear_x) as u8;
        out[9] = pack_path_shear(self.path_shear_y) as u8;
        out[10] = pack_path_twist(self.path_twist) as u8;
        out[11] = pack_path_twist(self.path_twist_begin) as u8;
        out[12] = pack_path_twist(self.path_radius_offset) as u8;
        out[13] = pack_path_taper(self.path_taper_x) as u8;
        out[14] = pack_path_taper(self.path_taper_y) as u8;
        out[15] = pack_path_revolutions(self.path_revolutions);
        out[16] = pack_path_twist(self.path_skew) as u8;
        put_u16(&mut out[17..19], pack_begin_cut(self.profile_begin));
        put_u16(&mut out[19..21], pack_end_cut(self.profile_end));
        put_u16(&mut out[21..23], pack_profile_hollow(self.profile_hollow));
        out
    }

    /// `Primitive.Unpack*` of a 23-byte shape block.
    pub fn from_bytes(data: &[u8]) -> Result<Self, WireError> {
        if data.len() != SHAPE_LEN {
            return Err(WireError::Short {
                need: SHAPE_LEN,
                got: data.len(),
            });
        }
        Ok(Self {
            path_curve: data[0],
            profile_curve: data[1],
            path_begin: unpack_begin_cut(read_u16(&data[2..4])),
            path_end: unpack_end_cut(read_u16(&data[4..6])),
            path_scale_x: unpack_path_scale(data[6]),
            path_scale_y: unpack_path_scale(data[7]),
            path_shear_x: unpack_path_shear(data[8] as i8),
            path_shear_y: unpack_path_shear(data[9] as i8),
            path_twist: unpack_path_twist(data[10] as i8),
            path_twist_begin: unpack_path_twist(data[11] as i8),
            path_radius_offset: unpack_path_twist(data[12] as i8),
            path_taper_x: unpack_path_taper(data[13] as i8),
            path_taper_y: unpack_path_taper(data[14] as i8),
            path_revolutions: unpack_path_revolutions(data[15]),
            path_skew: unpack_path_twist(data[16] as i8),
            profile_begin: unpack_begin_cut(read_u16(&data[17..19])),
            profile_end: unpack_end_cut(read_u16(&data[19..21])),
            profile_hollow: unpack_profile_hollow(read_u16(&data[21..23])),
        })
    }
}

/// Low nibble of `ConstructionData.profileCurve`.
pub fn profile_curve(profile: u8) -> u8 {
    profile & PROFILE_MASK
}

/// High nibble of `ConstructionData.profileCurve` (`HoleType` already shifted).
pub fn profile_hole(profile: u8) -> u8 {
    profile & HOLE_MASK
}

/// `ConstructionData.ProfileCurve` setter. The value is stored as given.
pub fn set_profile_curve(profile: u8, curve: u8) -> u8 {
    (profile & HOLE_MASK) | curve
}

/// `ConstructionData.ProfileHole` setter. The value is stored as given.
pub fn set_profile_hole(profile: u8, hole: u8) -> u8 {
    (profile & PROFILE_MASK) | hole
}

/// `Utils.SwapNibbles`. `AttachmentPoint` is stored in `State` this way.
pub fn swap_nibbles(value: u8) -> u8 {
    ((value & 0xf0) >> 4) | ((value & 0x0f) << 4)
}

/// `Primitive.PackBeginCut`.
pub fn pack_begin_cut(begin_cut: f32) -> u16 {
    round_i32(begin_cut * CUT_INV) as u16
}

/// `Primitive.PackEndCut`.
pub fn pack_end_cut(end_cut: f32) -> u16 {
    let rounded = round_i32(end_cut * CUT_INV) as u16;
    (50_000i32 - i32::from(rounded)) as u16
}

/// `Primitive.PackPathScale`.
pub fn pack_path_scale(path_scale: f32) -> u8 {
    let rounded = round_i32(path_scale * SCALE_INV) as u8;
    (200i32 - i32::from(rounded)) as u8
}

/// `Primitive.PackPathShear`.
pub fn pack_path_shear(path_shear: f32) -> i8 {
    round_i32(path_shear * SHEAR_INV) as i8
}

/// `Primitive.PackPathTwist`. Also packs twist-begin, radius offset and skew.
pub fn pack_path_twist(path_twist: f32) -> i8 {
    round_i32(path_twist * SCALE_INV) as i8
}

/// `Primitive.PackPathTaper`.
pub fn pack_path_taper(path_taper: f32) -> i8 {
    round_i32(path_taper * TAPER_INV) as i8
}

/// `Primitive.PackPathRevolutions`.
pub fn pack_path_revolutions(path_revolutions: f32) -> u8 {
    round_i32((path_revolutions - 1.0) * REV_INV) as u8
}

/// `Primitive.PackProfileHollow`.
pub fn pack_profile_hollow(profile_hollow: f32) -> u16 {
    round_i32(profile_hollow * HOLLOW_INV) as u16
}

/// `Primitive.UnpackBeginCut`.
pub fn unpack_begin_cut(begin_cut: u16) -> f32 {
    f32::from(begin_cut) * CUT_QUANTA
}

/// `Primitive.UnpackEndCut`.
pub fn unpack_end_cut(end_cut: u16) -> f32 {
    (50_000 - i32::from(end_cut)) as f32 * CUT_QUANTA
}

/// `Primitive.UnpackPathScale`.
pub fn unpack_path_scale(path_scale: u8) -> f32 {
    (200 - i32::from(path_scale)) as f32 * SCALE_QUANTA
}

/// `Primitive.UnpackPathShear`.
pub fn unpack_path_shear(path_shear: i8) -> f32 {
    f32::from(path_shear) * SHEAR_QUANTA
}

/// `Primitive.UnpackPathTwist`.
pub fn unpack_path_twist(path_twist: i8) -> f32 {
    f32::from(path_twist) * SCALE_QUANTA
}

/// `Primitive.UnpackPathTaper`.
pub fn unpack_path_taper(path_taper: i8) -> f32 {
    f32::from(path_taper) * TAPER_QUANTA
}

/// `Primitive.UnpackPathRevolutions`.
pub fn unpack_path_revolutions(path_revolutions: u8) -> f32 {
    f32::from(path_revolutions) * REV_QUANTA + 1.0
}

/// `Primitive.UnpackProfileHollow`.
pub fn unpack_profile_hollow(profile_hollow: u16) -> f32 {
    f32::from(profile_hollow) * HOLLOW_QUANTA
}

/// `MathF.Round` then the integer cast. The rounded value has to fit in `i32`,
/// which the shape quanta in the fixtures do. Out of that range, Rust saturates
/// and C# leaves the result unspecified.
fn round_i32(value: f32) -> i32 {
    value.round_ties_even() as i32
}
