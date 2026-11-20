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
//   OpenMetaverse.Types/UUID.cs
//   OpenMetaverse.Types/Vector2.cs
//   OpenMetaverse.Types/Vector3.cs
//   OpenMetaverse.Types/Vector3d.cs
//   OpenMetaverse.Types/Vector4.cs
//   OpenMetaverse.Types/Quaternion.cs
//   OpenMetaverse.Types/Color4.cs
//   OpenMetaverse.Types/UtilsConversions.cs
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Packet bytes for the OpenMetaverse value types.
//!
//! Vectors and quaternions are `glam` values. The bytes are written one field
//! at a time, little-endian, because a `glam` vector is not the wire layout.
//! UUID bytes are network order, which is what `UUID.ToBytes` writes on a
//! little-endian machine. `GetBytes` and `ToBytes` on a quaternion use
//! different tests before they substitute zeros; both are kept.

use glam::{DVec3, Quat, Vec2, Vec3, Vec4};
use uuid::Uuid;

pub(crate) fn refuse_limit(id: &'static str, limit: usize, got: usize) -> WireError {
    ferroan_diag::refuse(
        id,
        ferroan_diag::Kind::Hard,
        limit as u64,
        got as u64,
        "wire",
    );
    WireError::Limit { limit, got }
}

/// A read or write was shorter than the OpenMetaverse layout, or a Ferroan
/// size cap was hit. OpenMetaverse does not cap TextureEntry or extra-parameter
/// blobs; the cap is recorded in the crate README.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WireError {
    #[error("need {need} bytes, got {got}")]
    Short { need: usize, got: usize },
    #[error("limit of {limit} exceeded ({got})")]
    Limit { limit: usize, got: usize },
}

/// RGBA in 0..1, the range `Color4` stores. The four-byte form is quantized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color4 {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color4 {
    /// `new Color4(byte, byte, byte, byte)`: each byte times `1/255`.
    pub fn from_bytes(r: u8, g: u8, b: u8, a: u8) -> Self {
        const QUANTA: f32 = 1.0 / 255.0;
        Self {
            r: f32::from(r) * QUANTA,
            g: f32::from(g) * QUANTA,
            b: f32::from(b) * QUANTA,
            a: f32::from(a) * QUANTA,
        }
    }

    pub fn from_floats(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

pub(crate) fn need(src: &[u8], n: usize) -> Result<(), WireError> {
    if src.len() < n {
        Err(WireError::Short {
            need: n,
            got: src.len(),
        })
    } else {
        Ok(())
    }
}

pub(crate) fn put_f32(dest: &mut [u8], value: f32) {
    dest[..4].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn put_i16(dest: &mut [u8], value: i16) {
    dest[..2].copy_from_slice(&value.to_le_bytes());
}

fn put_f64(dest: &mut [u8], value: f64) {
    dest[..8].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn put_u16(dest: &mut [u8], value: u16) {
    dest[..2].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn read_f32(src: &[u8]) -> f32 {
    f32::from_le_bytes(le_array(src))
}

fn read_f64(src: &[u8]) -> f64 {
    f64::from_le_bytes(le_array(src))
}

pub(crate) fn read_u16(src: &[u8]) -> u16 {
    u16::from_le_bytes(le_array(src))
}

/// Copy a fixed little-endian field. Callers reject a short slice first.
pub(crate) fn le_array<const N: usize>(src: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    match src.get(..N) {
        Some(chunk) => out.copy_from_slice(chunk),
        None => {
            // INFALLIBLE: every caller rejects a short slice before reading.
            #[allow(clippy::panic)]
            {
                panic!("wire slice shorter than {N}");
            }
        }
    }
    out
}

/// A write into a stack array of the field's wire size.
pub(crate) fn layout_ok(result: Result<(), WireError>) {
    if let Err(error) = result {
        // INFALLIBLE: the destination is a stack array of the field's wire size.
        #[allow(clippy::panic)]
        {
            panic!("fixed wire buffer was short: {error}");
        }
    }
}

/// `UUID.GetBytes`. Sixteen bytes, network order.
pub fn write_uuid(id: &Uuid, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 16)?;
    dest[..16].copy_from_slice(id.as_bytes());
    Ok(())
}

pub fn read_uuid(src: &[u8]) -> Result<Uuid, WireError> {
    need(src, 16)?;
    Ok(Uuid::from_bytes(le_array(src)))
}

pub fn write_vec2(v: Vec2, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 8)?;
    put_f32(&mut dest[..4], v.x);
    put_f32(&mut dest[4..8], v.y);
    Ok(())
}

pub fn read_vec2(src: &[u8]) -> Result<Vec2, WireError> {
    need(src, 8)?;
    Ok(Vec2::new(read_f32(src), read_f32(&src[4..])))
}

pub fn write_vec3(v: Vec3, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 12)?;
    put_f32(&mut dest[..4], v.x);
    put_f32(&mut dest[4..8], v.y);
    put_f32(&mut dest[8..12], v.z);
    Ok(())
}

pub fn read_vec3(src: &[u8]) -> Result<Vec3, WireError> {
    need(src, 12)?;
    Ok(Vec3::new(
        read_f32(src),
        read_f32(&src[4..]),
        read_f32(&src[8..]),
    ))
}

pub fn write_dvec3(v: DVec3, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 24)?;
    put_f64(&mut dest[..8], v.x);
    put_f64(&mut dest[8..16], v.y);
    put_f64(&mut dest[16..24], v.z);
    Ok(())
}

pub fn read_dvec3(src: &[u8]) -> Result<DVec3, WireError> {
    need(src, 24)?;
    Ok(DVec3::new(
        read_f64(src),
        read_f64(&src[8..]),
        read_f64(&src[16..]),
    ))
}

pub fn write_vec4(v: Vec4, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 16)?;
    put_f32(&mut dest[..4], v.x);
    put_f32(&mut dest[4..8], v.y);
    put_f32(&mut dest[8..12], v.z);
    put_f32(&mut dest[12..16], v.w);
    Ok(())
}

pub fn read_vec4(src: &[u8]) -> Result<Vec4, WireError> {
    need(src, 16)?;
    Ok(Vec4::new(
        read_f32(src),
        read_f32(&src[4..]),
        read_f32(&src[8..]),
        read_f32(&src[12..]),
    ))
}

/// `Vector3.ClampedToShortsBytes`: scale so the largest component fits in
/// `range`, then quantize each axis with [`float_to_u16_range`].
pub fn write_vec3_clamped(v: Vec3, range: f32, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 6)?;
    let mut peak = v.x.abs();
    if v.y.abs() > peak {
        peak = v.y.abs();
    }
    if v.z.abs() > peak {
        peak = v.z.abs();
    }
    let (x, y, z) = if peak > range {
        let scale = range / peak;
        (v.x * scale, v.y * scale, v.z * scale)
    } else {
        (v.x, v.y, v.z)
    };
    put_u16(&mut dest[..2], float_to_u16_range(x, range));
    put_u16(&mut dest[2..4], float_to_u16_range(y, range));
    put_u16(&mut dest[4..6], float_to_u16_range(z, range));
    Ok(())
}

pub fn read_vec3_clamped(src: &[u8]) -> Result<[u16; 3], WireError> {
    need(src, 6)?;
    Ok([read_u16(src), read_u16(&src[2..]), read_u16(&src[4..])])
}

fn length_squared(q: Quat) -> f32 {
    q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w
}

/// Twelve little-endian floats, X Y Z, after the OpenMetaverse normalize step.
///
/// `get_bytes` follows `Quaternion.GetBytes` (`W < 0.9999`). Otherwise this is
/// `Quaternion.ToBytes` (`norm < 0.9999`). A negative `W` flips the scale, so
/// the omitted component reconstructs as positive.
fn pack_quaternion(q: Quat, get_bytes: bool) -> [f32; 3] {
    let norm = length_squared(q);
    let normalize = if get_bytes {
        norm > 1e-6 || q.w < 0.9999
    } else {
        norm > 1e-6 || norm < 0.9999
    };
    if !normalize {
        return [0.0, 0.0, 0.0];
    }
    let scale = if q.w < 0.0 {
        -1.0 / norm.sqrt()
    } else {
        1.0 / norm.sqrt()
    };
    [scale * q.x, scale * q.y, scale * q.z]
}

pub fn write_quaternion_get_bytes(q: Quat, dest: &mut [u8]) -> Result<(), WireError> {
    write_packed(pack_quaternion(q, true), dest)
}

pub fn write_quaternion_to_bytes(q: Quat, dest: &mut [u8]) -> Result<(), WireError> {
    write_packed(pack_quaternion(q, false), dest)
}

fn write_packed(xyz: [f32; 3], dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 12)?;
    put_f32(&mut dest[..4], xyz[0]);
    put_f32(&mut dest[4..8], xyz[1]);
    put_f32(&mut dest[8..12], xyz[2]);
    Ok(())
}

/// `Quaternion.FromBytes(..., normalized: true)`. `W` is the positive square
/// root of whatever is left of 1.
pub fn read_quaternion_packed(src: &[u8]) -> Result<Quat, WireError> {
    need(src, 12)?;
    let x = read_f32(src);
    let y = read_f32(&src[4..]);
    let z = read_f32(&src[8..]);
    let sum = 1.0 - x * x - y * y - z * z;
    let w = if sum > 1e-6 { sum.sqrt() } else { 0.0 };
    Ok(Quat::from_xyzw(x, y, z, w))
}

/// `Quaternion.ToShortsBytes`: each component through [`float_to_unit_u16`].
pub fn write_quaternion_shorts(q: Quat, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 8)?;
    put_u16(&mut dest[..2], float_to_unit_u16(q.x));
    put_u16(&mut dest[2..4], float_to_unit_u16(q.y));
    put_u16(&mut dest[4..6], float_to_unit_u16(q.z));
    put_u16(&mut dest[6..8], float_to_unit_u16(q.w));
    Ok(())
}

pub fn read_quaternion_shorts(src: &[u8]) -> Result<[u16; 4], WireError> {
    need(src, 8)?;
    Ok([
        read_u16(src),
        read_u16(&src[2..]),
        read_u16(&src[4..]),
        read_u16(&src[6..]),
    ])
}

/// Four little-endian floats, X Y Z W. `Utils.FloatToBytes` of each component.
pub fn write_quaternion_full(q: Quat, dest: &mut [u8]) -> Result<(), WireError> {
    write_vec4(Vec4::new(q.x, q.y, q.z, q.w), dest)
}

pub fn read_quaternion_full(src: &[u8]) -> Result<Quat, WireError> {
    let v = read_vec4(src)?;
    Ok(Quat::from_xyzw(v.x, v.y, v.z, v.w))
}

/// `Color4.ToBytes`. `inverted` stores `255 - byte` for each channel.
pub fn write_color4(color: Color4, inverted: bool, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 4)?;
    let bytes = [
        float_zero_one_to_byte(color.r),
        float_zero_one_to_byte(color.g),
        float_zero_one_to_byte(color.b),
        float_zero_one_to_byte(color.a),
    ];
    if inverted {
        dest[..4].copy_from_slice(&[
            255 - bytes[0],
            255 - bytes[1],
            255 - bytes[2],
            255 - bytes[3],
        ]);
    } else {
        dest[..4].copy_from_slice(&bytes);
    }
    Ok(())
}

/// `Color4.FromBytes`.
pub fn read_color4(src: &[u8], inverted: bool) -> Result<Color4, WireError> {
    need(src, 4)?;
    const QUANTA: f32 = 1.0 / 255.0;
    let byte = |index: usize| -> f32 {
        let raw = src[index];
        let value = if inverted { 255 - raw } else { raw };
        f32::from(value) * QUANTA
    };
    Ok(Color4 {
        r: byte(0),
        g: byte(1),
        b: byte(2),
        a: byte(3),
    })
}

/// `Color4.GetFloatBytes`: four little-endian floats.
pub fn write_color4_floats(color: Color4, dest: &mut [u8]) -> Result<(), WireError> {
    write_vec4(Vec4::new(color.r, color.g, color.b, color.a), dest)
}

pub fn read_color4_floats(src: &[u8]) -> Result<Color4, WireError> {
    let v = read_vec4(src)?;
    Ok(Color4 {
        r: v.x,
        g: v.y,
        b: v.z,
        a: v.w,
    })
}

/// `Utils.FloatToUInt16(value, range)`. The final scale is a double multiply
/// by `32767.5`, then a truncation toward zero.
pub fn float_to_u16_range(mut value: f32, range: f32) -> u16 {
    value += range;
    if value <= 0.0 {
        return 0;
    }
    value /= range;
    if value >= 2.0 {
        return 0xffff;
    }
    (f64::from(value) * 32767.5) as u16
}

/// `Utils.FloatToUInt16(value, lower, upper)`. The final multiply stays in
/// single precision (`value * 0xffff`).
pub fn float_to_u16_bounds(mut value: f32, lower: f32, upper: f32) -> u16 {
    value -= lower;
    if value <= 0.0 {
        return 0;
    }
    value /= upper - lower;
    if value >= 1.0 {
        return 0xffff;
    }
    (value * 65535.0) as u16
}

/// `Utils.FloatToUnitUInt16`. The `-1` comparison is against a double.
pub fn float_to_unit_u16(value: f32) -> u16 {
    if f64::from(value) <= -1.0 {
        return 0;
    }
    if value >= 1.0 {
        return 0xffff;
    }
    (f64::from(value) * 32767.5 + 32767.0) as u16
}

/// `Utils.UInt16ToFloat`. A result closer to zero than one step becomes `0`.
pub fn u16_to_float(val: u16, lower: f32, upper: f32) -> f32 {
    let delta = (upper - lower) * (1.0 / f32::from(u16::MAX));
    let mut fval = f32::from(val) * delta;
    fval += lower;
    if fval.abs() < delta { 0.0 } else { fval }
}

/// `Utils.FloatZeroOneToByte`.
pub fn float_zero_one_to_byte(val: f32) -> u8 {
    if val <= 0.0 {
        return 0;
    }
    if val >= 1.0 {
        return 255;
    }
    (val * 255.0) as u8
}

pub fn write_u16(value: u16, dest: &mut [u8]) -> Result<(), WireError> {
    need(dest, 2)?;
    put_u16(dest, value);
    Ok(())
}

pub(crate) fn read_i16(src: &[u8]) -> i16 {
    i16::from_le_bytes(le_array(src))
}

pub fn read_u16_value(src: &[u8]) -> Result<u16, WireError> {
    need(src, 2)?;
    Ok(read_u16(src))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_buffer_is_an_error() {
        assert_eq!(
            read_uuid(&[0; 15]),
            Err(WireError::Short { need: 16, got: 15 })
        );
        assert_eq!(
            read_vec3(&[0; 11]),
            Err(WireError::Short { need: 12, got: 11 })
        );
        assert_eq!(
            read_u16_value(&[0]),
            Err(WireError::Short { need: 2, got: 1 })
        );
    }
}
