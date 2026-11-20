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
//   OpenMetaverse/Primitives/Primitive.cs (FlexibleData, LightData, LightImage,
//   SculptData, GetExtraParamsBytes, SetExtraParamsFromBytes)
//   OpenMetaverse.Types/EnumsPrimitive.cs (ExtraParamType, SculptType)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Extra parameters on an object update: flexible, light, light image, sculpt.
//!
//! `GetExtraParamsBytes` writes only those four. Extended mesh (`0x70`), render
//! materials (`0x80`) and reflection probes (`0x90`) are viewer blobs.
//! `SetExtraParamsFromBytes` skips them by the declared length, and encoding
//! drops them. Aurora's `extra.rs` parses those viewer blobs; this module does
//! not treat them as something OpenMetaverse wrote.
//!
//! A light stores intensity in the colour alpha byte and forces `Color.A` to 1
//! on decode. Flexible tension, drag, gravity and wind are quantized with
//! `* 10.01` on the way out and `/ 10` on the way back.

use glam::Vec3;
use uuid::Uuid;

use crate::wire::{
    self, Color4, WireError, layout_ok, le_array, read_color4, read_f32, read_uuid, read_vec3,
    write_color4, write_uuid, write_vec3,
};

/// OpenMetaverse reads past a short extra-parameter blob. Ferroan refuses a
/// blob larger than this instead.
pub const MAX_EXTRA_BYTES: usize = 64 * 1024;

const FLEXIBLE: u16 = 0x10;
const LIGHT: u16 = 0x20;
const SCULPT: u16 = 0x30;
const LIGHT_IMAGE: u16 = 0x40;
const MESH: u16 = 0x60;
const SCULPT_MESH: u8 = 5;

/// `Primitive.FlexibleData`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlexibleData {
    pub softness: i32,
    pub gravity: f32,
    pub drag: f32,
    pub wind: f32,
    pub tension: f32,
    pub force: Vec3,
}

/// `Primitive.LightData`. After a decode, `color.a` is 1 and `intensity` holds
/// the alpha byte that was on the wire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightData {
    pub color: Color4,
    pub intensity: f32,
    pub radius: f32,
    pub cutoff: f32,
    pub falloff: f32,
}

/// `Primitive.LightImage`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightImage {
    pub texture: Uuid,
    pub params: Vec3,
}

/// `Primitive.SculptData`. `type_byte` is the raw byte, including invert and
/// mirror. `SculptType` is only the low three bits; those have no separate
/// setters in OpenMetaverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SculptData {
    pub texture: Uuid,
    pub type_byte: u8,
}

/// The four extra-parameter blocks `GetExtraParamsBytes` emits, in that order.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ExtraParams {
    pub flexible: Option<FlexibleData>,
    pub light: Option<LightData>,
    pub light_image: Option<LightImage>,
    pub sculpt: Option<SculptData>,
}

impl SculptData {
    /// `SculptData.Type`, which masks with 7.
    pub fn sculpt_type(self) -> u8 {
        self.type_byte & 7
    }

    pub fn invert(self) -> bool {
        self.type_byte & 64 != 0
    }

    pub fn mirror(self) -> bool {
        self.type_byte & 128 != 0
    }

    fn to_payload(self) -> [u8; 17] {
        let mut data = [0u8; 17];
        layout_ok(write_uuid(&self.texture, &mut data[..16]));
        data[16] = self.type_byte;
        data
    }

    fn read(data: &[u8], pos: usize) -> Result<Self, WireError> {
        // The C# constructor checks the whole buffer, not the bytes left at pos.
        if data.len() < 17 {
            return Ok(Self {
                texture: Uuid::nil(),
                type_byte: 0,
            });
        }
        if pos + 17 > data.len() {
            return Err(WireError::Short {
                need: pos + 17,
                got: data.len(),
            });
        }
        Ok(Self {
            texture: read_uuid(&data[pos..])?,
            type_byte: data[pos + 16],
        })
    }
}

impl FlexibleData {
    fn to_payload(self) -> [u8; 16] {
        let mut data = [0u8; 16];
        data[0] = ((self.softness & 2) << 6) as u8;
        data[1] = ((self.softness & 1) << 7) as u8;
        data[0] |= byte_trunc(self.tension * 10.01) & 0x7f;
        data[1] |= byte_trunc(self.drag * 10.01) & 0x7f;
        data[2] = byte_trunc((self.gravity + 10.0) * 10.01);
        data[3] = byte_trunc(self.wind * 10.01);
        layout_ok(write_vec3(self.force, &mut data[4..16]));
        data
    }

    fn read(data: &[u8], pos: usize) -> Result<Self, WireError> {
        if data.len() < 5 {
            return Ok(Self {
                softness: 0,
                gravity: 0.0,
                drag: 0.0,
                wind: 0.0,
                tension: 0.0,
                force: Vec3::ZERO,
            });
        }
        if pos + 16 > data.len() {
            return Err(WireError::Short {
                need: pos + 16,
                got: data.len(),
            });
        }
        let softness = i32::from((data[pos] & 0x80) >> 6) | i32::from((data[pos + 1] & 0x80) >> 7);
        Ok(Self {
            softness,
            tension: f32::from(data[pos] & 0x7f) / 10.0,
            drag: f32::from(data[pos + 1] & 0x7f) / 10.0,
            gravity: f32::from(data[pos + 2]) / 10.0 - 10.0,
            wind: f32::from(data[pos + 3]) / 10.0,
            force: read_vec3(&data[pos + 4..])?,
        })
    }
}

impl LightData {
    fn to_payload(self) -> [u8; 16] {
        let mut data = [0u8; 16];
        let mut color = self.color;
        color.a = self.intensity;
        layout_ok(write_color4(color, false, &mut data[..4]));
        wire::put_f32(&mut data[4..8], self.radius);
        wire::put_f32(&mut data[8..12], self.cutoff);
        wire::put_f32(&mut data[12..16], self.falloff);
        data
    }

    fn read(data: &[u8], pos: usize) -> Result<Self, WireError> {
        if data.len().saturating_sub(pos) < 16 {
            return Ok(Self {
                color: Color4::from_floats(0.0, 0.0, 0.0, 1.0),
                intensity: 0.0,
                radius: 0.0,
                cutoff: 0.0,
                falloff: 0.0,
            });
        }
        let mut color = read_color4(&data[pos..], false)?;
        let intensity = color.a;
        color.a = 1.0;
        Ok(Self {
            color,
            intensity,
            radius: read_f32(&data[pos + 4..]),
            cutoff: read_f32(&data[pos + 8..]),
            falloff: read_f32(&data[pos + 12..]),
        })
    }
}

impl LightImage {
    fn to_payload(self) -> [u8; 28] {
        let mut data = [0u8; 28];
        layout_ok(write_uuid(&self.texture, &mut data[..16]));
        layout_ok(write_vec3(self.params, &mut data[16..28]));
        data
    }

    fn read(data: &[u8], pos: usize) -> Result<Self, WireError> {
        if data.len().saturating_sub(pos) < 28 {
            return Ok(Self {
                texture: Uuid::nil(),
                params: Vec3::ZERO,
            });
        }
        Ok(Self {
            texture: read_uuid(&data[pos..])?,
            params: read_vec3(&data[pos + 16..])?,
        })
    }
}

impl ExtraParams {
    /// `Primitive.GetExtraParamsBytes`. An empty prim is a single zero count.
    pub fn to_bytes(self) -> Vec<u8> {
        let mut parts: Vec<(u16, Vec<u8>)> = Vec::with_capacity(4);
        if let Some(flexible) = self.flexible {
            parts.push((FLEXIBLE, flexible.to_payload().to_vec()));
        }
        if let Some(light) = self.light {
            parts.push((LIGHT, light.to_payload().to_vec()));
        }
        if let Some(image) = self.light_image {
            parts.push((LIGHT_IMAGE, image.to_payload().to_vec()));
        }
        if let Some(sculpt) = self.sculpt {
            let kind = if sculpt.sculpt_type() == SCULPT_MESH {
                MESH
            } else {
                SCULPT
            };
            parts.push((kind, sculpt.to_payload().to_vec()));
        }
        let mut out = Vec::with_capacity(1 + parts.iter().map(|(_, p)| 6 + p.len()).sum::<usize>());
        // INFALLIBLE: at most one of each of the four extra-parameter kinds is stored.
        #[allow(clippy::expect_used)]
        let count = u8::try_from(parts.len()).expect("at most four extra parameters");
        out.push(count);
        for (kind, payload) in parts {
            out.extend_from_slice(&kind.to_le_bytes());
            // INFALLIBLE: each payload is a fixed 16, 17 or 28 byte block.
            #[allow(clippy::expect_used)]
            let len = u32::try_from(payload.len()).expect("payload fits in u32");
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&payload);
        }
        out
    }

    /// `Primitive.SetExtraParamsFromBytes`. Unknown types are skipped.
    pub fn from_bytes(data: &[u8]) -> Result<Self, WireError> {
        if data.len() > MAX_EXTRA_BYTES {
            return Err(crate::wire::refuse_limit(
                ferroan_diag::EXTRA_BYTES,
                MAX_EXTRA_BYTES,
                data.len(),
            ));
        }
        if data.is_empty() {
            return Ok(Self::default());
        }
        let count = data[0];
        let mut pos = 1;
        let mut out = Self::default();
        for _ in 0..count {
            if pos + 6 > data.len() {
                return Err(WireError::Short {
                    need: pos + 6,
                    got: data.len(),
                });
            }
            let kind = u16::from_le_bytes(le_array(&data[pos..pos + 2]));
            let len_u32 = u32::from_le_bytes(le_array(&data[pos + 2..pos + 6]));
            let len = usize::try_from(len_u32).map_err(|_| {
                crate::wire::refuse_limit(ferroan_diag::EXTRA_BYTES, MAX_EXTRA_BYTES, data.len())
            })?;
            pos += 6;
            if len > data.len() - pos {
                return Err(WireError::Short {
                    need: pos + len,
                    got: data.len(),
                });
            }
            match kind {
                FLEXIBLE => out.flexible = Some(FlexibleData::read(data, pos)?),
                LIGHT => out.light = Some(LightData::read(data, pos)?),
                LIGHT_IMAGE => out.light_image = Some(LightImage::read(data, pos)?),
                SCULPT | MESH => out.sculpt = Some(SculptData::read(data, pos)?),
                _ => {}
            }
            pos += len;
        }
        Ok(out)
    }
}

/// `(byte)(value)` for the in-range quantities these fields store.
/// OpenMetaverse truncates toward zero. Values outside 0..256 are not what
/// the C# cast does; the fixtures stay inside that range.
fn byte_trunc(value: f32) -> u8 {
    if value <= 0.0 {
        return 0;
    }
    if value >= 256.0 {
        return 255;
    }
    value as u8
}
