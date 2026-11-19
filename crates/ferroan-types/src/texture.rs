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
//   OpenMetaverse/Primitives/TextureEntry.cs
//   OpenMetaverse/Helpers.cs (TEOffsetShort, TEOffsetFloat, TERotationShort,
//   TERotationFloat, TEGlowByte, TEGlowFloat)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `Primitive.TextureEntry` bytes.
//!
//! A face field is written only when its attribute bit is set and the stored
//! value differs from the default face. Aurora's `te.rs` is the viewer packer
//! (`LLPrimitive::packTEMessage`): it treats the last face as the default and
//! emits every face. Those bytes do not match OpenMetaverse, so this module
//! follows `TextureEntry.GetBytes` / `FromBytes`.
//!
//! Offset, rotation and glow are stored as the quantized short or byte.
//! `TERotationShort(TERotationFloat(s))` is not the identity, so encoding must
//! write the stored short rather than quantize the public float again.
//!
//! OpenMetaverse does not cap a texture entry. A blob over [`MAX_TEXTURE_BYTES`]
//! is rejected, and a face bitfield is limited to eight 7-bit groups (a 45-face
//! mask fits in seven). A read past the provided slice is [`WireError::Short`]
//! where the C# reader would throw.

use uuid::Uuid;

use crate::wire::{
    self, Color4, WireError, layout_ok, read_color4, read_f32, read_i16, read_uuid, write_color4,
    write_uuid,
};

/// `TextureEntry.MAX_FACES`.
pub const MAX_FACES: usize = 45;

/// Ferroan refuses a texture entry larger than this. The C# reader does not.
pub const MAX_TEXTURE_BYTES: usize = 64 * 1024;

/// `TextureEntry.WHITE_TEXTURE`.
pub const WHITE_TEXTURE: Uuid = Uuid::from_bytes([
    0x57, 0x48, 0xde, 0xcc, 0xf6, 0x29, 0x46, 0x1c, 0x9a, 0x36, 0xa3, 0x5a, 0x22, 0x1f, 0xe2, 0x1f,
]);

const TEXTURE_ID: u32 = 1 << 0;
const RGBA: u32 = 1 << 1;
const REPEAT_U: u32 = 1 << 2;
const REPEAT_V: u32 = 1 << 3;
const OFFSET_U: u32 = 1 << 4;
const OFFSET_V: u32 = 1 << 5;
const ROTATION: u32 = 1 << 6;
const MATERIAL: u32 = 1 << 7;
const MEDIA: u32 = 1 << 8;
const GLOW: u32 = 1 << 9;
const MATERIAL_ID: u32 = 1 << 10;
const ALL: u32 = 0xffff_ffff;

const BUMP_MASK: u8 = 0x1f;
const FULLBRIGHT_MASK: u8 = 0x20;
const SHINY_MASK: u8 = 0xc0;
const MEDIA_MASK: u8 = 0x01;
const TEX_MAP_MASK: u8 = 0x06;

/// Values passed to the public setters. Absent fields are left alone.
/// Setters run in the order the fields are declared here.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextureFaceSet {
    pub texture_id: Option<Uuid>,
    pub rgba: Option<Color4>,
    pub repeat_u: Option<f32>,
    pub repeat_v: Option<f32>,
    pub offset_u: Option<f32>,
    pub offset_v: Option<f32>,
    pub rotation: Option<f32>,
    pub glow: Option<f32>,
    pub bump: Option<u8>,
    pub shiny: Option<u8>,
    pub fullbright: Option<bool>,
    pub media_flags: Option<bool>,
    pub mapping: Option<u8>,
    pub material_id: Option<Uuid>,
}

/// What the public getters return. Unset attributes inherit the default face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureFaceView {
    pub texture_id: Uuid,
    pub material_id: Uuid,
    pub rgba: Color4,
    pub repeat_u: f32,
    pub repeat_v: f32,
    pub offset_u: f32,
    pub offset_v: f32,
    pub rotation: f32,
    pub glow: f32,
    pub bump: u8,
    pub shiny: u8,
    pub fullbright: bool,
    pub media_flags: bool,
    pub mapping: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TextureFace {
    texture_id: Uuid,
    material_id: Uuid,
    rgba: Color4,
    repeat_u: f32,
    repeat_v: f32,
    offset_u: i16,
    offset_v: i16,
    rotation: i16,
    glow: u8,
    material: u8,
    media: u8,
    attributes: u32,
}

/// `Primitive.TextureEntry`. `faces[i]` is `None` when that face was never
/// created. Getters on a missing face are the default face; the wire form
/// omits it.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureEntry {
    default: Option<TextureFace>,
    faces: [Option<TextureFace>; MAX_FACES],
}

impl TextureFace {
    fn default_face() -> Self {
        Self {
            texture_id: Uuid::nil(),
            material_id: Uuid::nil(),
            rgba: Color4::from_floats(1.0, 1.0, 1.0, 1.0),
            repeat_u: 1.0,
            repeat_v: 1.0,
            offset_u: 0,
            offset_v: 0,
            rotation: 0,
            glow: 0,
            material: 0,
            media: 0,
            attributes: ALL,
        }
    }

    fn child() -> Self {
        Self {
            attributes: 0,
            ..Self::default_face()
        }
    }

    fn apply(&mut self, set: &TextureFaceSet, parent: Option<&TextureFace>) {
        if let Some(value) = set.texture_id {
            self.set_texture_id(value, parent);
        }
        if let Some(value) = set.rgba {
            self.set_rgba(value, parent);
        }
        if let Some(value) = set.repeat_u {
            self.set_repeat_u(value, parent);
        }
        if let Some(value) = set.repeat_v {
            self.set_repeat_v(value, parent);
        }
        if let Some(value) = set.offset_u {
            self.set_offset_u(value, parent);
        }
        if let Some(value) = set.offset_v {
            self.set_offset_v(value, parent);
        }
        if let Some(value) = set.rotation {
            self.set_rotation(value, parent);
        }
        if let Some(value) = set.glow {
            self.set_glow(value, parent);
        }
        if let Some(value) = set.bump {
            self.set_bump(value, parent);
        }
        if let Some(value) = set.shiny {
            self.set_shiny(value, parent);
        }
        if let Some(value) = set.fullbright {
            self.set_fullbright(value, parent);
        }
        if let Some(value) = set.media_flags {
            self.set_media(value, parent);
        }
        if let Some(value) = set.mapping {
            self.set_mapping(value, parent);
        }
        if let Some(value) = set.material_id {
            self.set_material_id(value, parent);
        }
    }

    fn view(self, parent: &TextureFace) -> TextureFaceView {
        let material = self.pick(MATERIAL, self.material, parent.material);
        let media = self.pick(MEDIA, self.media, parent.media);
        TextureFaceView {
            texture_id: self.pick(TEXTURE_ID, self.texture_id, parent.texture_id),
            material_id: self.pick(MATERIAL_ID, self.material_id, parent.material_id),
            rgba: self.pick(RGBA, self.rgba, parent.rgba),
            repeat_u: self.pick(REPEAT_U, self.repeat_u, parent.repeat_u),
            repeat_v: self.pick(REPEAT_V, self.repeat_v, parent.repeat_v),
            offset_u: te_offset_float(self.pick(OFFSET_U, self.offset_u, parent.offset_u)),
            offset_v: te_offset_float(self.pick(OFFSET_V, self.offset_v, parent.offset_v)),
            rotation: te_rotation_float(self.pick(ROTATION, self.rotation, parent.rotation)),
            glow: te_glow_float(self.pick(GLOW, self.glow, parent.glow)),
            bump: material & BUMP_MASK,
            shiny: material & SHINY_MASK,
            fullbright: material & FULLBRIGHT_MASK != 0,
            media_flags: media & MEDIA_MASK != 0,
            mapping: media & TEX_MAP_MASK,
        }
    }

    fn pick<T: Copy>(self, bit: u32, own: T, parent: T) -> T {
        if self.attributes & bit == 0 {
            parent
        } else {
            own
        }
    }

    /// An unset attribute bit means this face inherits from its parent.
    fn inherit(parent: Option<&TextureFace>) -> &TextureFace {
        // INFALLIBLE: an unset attribute bit means this face inherits from its parent.
        #[allow(clippy::expect_used)]
        parent.expect("texture face default")
    }

    fn set_texture_id(&mut self, value: Uuid, parent: Option<&TextureFace>) {
        let current = if self.attributes & TEXTURE_ID == 0 {
            Self::inherit(parent).texture_id
        } else {
            self.texture_id
        };
        if current != value {
            self.texture_id = value;
            self.attributes |= TEXTURE_ID;
        }
    }

    fn set_rgba(&mut self, value: Color4, parent: Option<&TextureFace>) {
        let current = if self.attributes & RGBA == 0 {
            Self::inherit(parent).rgba
        } else {
            self.rgba
        };
        if current != value {
            self.rgba = value;
            self.attributes |= RGBA;
        }
    }

    fn set_repeat_u(&mut self, value: f32, parent: Option<&TextureFace>) {
        let current = if self.attributes & REPEAT_U == 0 {
            Self::inherit(parent).repeat_u
        } else {
            self.repeat_u
        };
        if current != value {
            self.repeat_u = value;
            self.attributes |= REPEAT_U;
        }
    }

    fn set_repeat_v(&mut self, value: f32, parent: Option<&TextureFace>) {
        let current = if self.attributes & REPEAT_V == 0 {
            Self::inherit(parent).repeat_v
        } else {
            self.repeat_v
        };
        if current != value {
            self.repeat_v = value;
            self.attributes |= REPEAT_V;
        }
    }

    fn set_offset_u(&mut self, value: f32, parent: Option<&TextureFace>) {
        let current = if self.attributes & OFFSET_U == 0 {
            Self::inherit(parent).offset_u
        } else {
            self.offset_u
        };
        let quantized = te_offset_short(value);
        if current != quantized {
            self.offset_u = quantized;
            self.attributes |= OFFSET_U;
        }
    }

    fn set_offset_v(&mut self, value: f32, parent: Option<&TextureFace>) {
        let current = if self.attributes & OFFSET_V == 0 {
            Self::inherit(parent).offset_v
        } else {
            self.offset_v
        };
        let quantized = te_offset_short(value);
        if current != quantized {
            self.offset_v = quantized;
            self.attributes |= OFFSET_V;
        }
    }

    fn set_rotation(&mut self, value: f32, parent: Option<&TextureFace>) {
        let current = if self.attributes & ROTATION == 0 {
            Self::inherit(parent).rotation
        } else {
            self.rotation
        };
        let quantized = te_rotation_short(value);
        if current != quantized {
            self.rotation = quantized;
            self.attributes |= ROTATION;
        }
    }

    fn set_glow(&mut self, value: f32, parent: Option<&TextureFace>) {
        let current = if self.attributes & GLOW == 0 {
            Self::inherit(parent).glow
        } else {
            self.glow
        };
        let quantized = te_glow_byte(value);
        if current != quantized {
            self.glow = quantized;
            self.attributes |= GLOW;
        }
    }

    fn set_bump(&mut self, value: u8, parent: Option<&TextureFace>) {
        let old = self.material;
        if self.attributes & MATERIAL == 0 {
            self.material = Self::inherit(parent).material;
        }
        let mixed = (self.material & 0xe0) | value;
        if old != mixed {
            self.material = mixed;
            self.attributes |= MATERIAL;
        }
    }

    fn set_shiny(&mut self, value: u8, parent: Option<&TextureFace>) {
        let old = self.material;
        if self.attributes & MATERIAL == 0 {
            self.material = Self::inherit(parent).material;
        }
        let mixed = (self.material & 0x3f) | value;
        if old != mixed {
            self.material = mixed;
            self.attributes |= MATERIAL;
        }
    }

    fn set_fullbright(&mut self, value: bool, parent: Option<&TextureFace>) {
        let old = self.material;
        if self.attributes & MATERIAL == 0 {
            self.material = Self::inherit(parent).material;
        }
        let mut mixed = self.material & 0xdf;
        if value {
            mixed |= FULLBRIGHT_MASK;
        }
        if old != mixed {
            self.material = mixed;
            self.attributes |= MATERIAL;
        }
    }

    fn set_media(&mut self, value: bool, parent: Option<&TextureFace>) {
        let old = self.media;
        if self.attributes & MEDIA == 0 {
            self.media = Self::inherit(parent).media;
        }
        let mut mixed = self.media & 0xfe;
        if value {
            mixed |= MEDIA_MASK;
        }
        if old != mixed {
            self.media = mixed;
            self.attributes |= MEDIA;
        }
    }

    fn set_mapping(&mut self, value: u8, parent: Option<&TextureFace>) {
        let old = self.media;
        if self.attributes & MEDIA == 0 {
            self.media = Self::inherit(parent).media;
        }
        let mixed = (self.media & 0xf9) | value;
        if old != mixed {
            self.media = mixed;
            self.attributes |= MEDIA;
        }
    }

    fn set_material_id(&mut self, value: Uuid, parent: Option<&TextureFace>) {
        let current = if self.attributes & MATERIAL_ID == 0 {
            Self::inherit(parent).material_id
        } else {
            self.material_id
        };
        if current != value {
            self.material_id = value;
            self.attributes |= MATERIAL_ID;
        }
    }
}

impl TextureEntry {
    /// `new TextureEntry(defaultTextureID)`.
    pub fn new(default_texture: Uuid) -> Self {
        let mut face = TextureFace::default_face();
        face.set_texture_id(default_texture, None);
        Self {
            default: Some(face),
            faces: [None; MAX_FACES],
        }
    }

    fn blank() -> Self {
        Self {
            default: None,
            faces: [None; MAX_FACES],
        }
    }

    /// Setters on the default face. The default face starts with every
    /// attribute bit set, which is what `new TextureEntryFace(null)` does.
    pub fn apply_default(&mut self, set: &TextureFaceSet) {
        self.default_mut().apply(set, None);
    }

    /// `CreateFace` followed by the public setters, in [`TextureFaceSet`] order.
    pub fn apply_face(&mut self, index: usize, set: &TextureFaceSet) -> Result<(), WireError> {
        if index >= MAX_FACES {
            return Err(WireError::Limit {
                limit: MAX_FACES,
                got: index,
            });
        }
        let parent = *self.default_face();
        self.faces[index]
            .get_or_insert_with(TextureFace::child)
            .apply(set, Some(&parent));
        Ok(())
    }

    pub fn default_view(&self) -> Option<TextureFaceView> {
        self.default.as_ref().map(|face| face.view(face))
    }

    /// `None` when `FaceTextures[index]` is null. A missing face still inherits
    /// the default on the wire; this is the face object itself.
    pub fn face_view(&self, index: usize) -> Option<TextureFaceView> {
        let face = self.faces.get(index)?.as_ref()?;
        let parent = self.default.as_ref()?;
        Some(face.view(parent))
    }

    /// `GetBytes()` with `maxfaces` 45.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_bytes_max(MAX_FACES)
    }

    /// `GetBytes(maxfaces)`. Faces at or above `max_faces` are left out.
    /// A null default face encodes as an empty buffer.
    pub fn to_bytes_max(&self, max_faces: usize) -> Vec<u8> {
        let Some(default) = self.default else {
            return Vec::new();
        };
        let limit = max_faces.min(MAX_FACES);
        let mut out = Vec::new();
        let (last, nulls) = self.write_textures(&mut out, &default, limit);
        self.write_colors(&mut out, &default, last, nulls);
        self.write_repeat(&mut out, &default, last, nulls, true);
        self.write_repeat(&mut out, &default, last, nulls, false);
        self.write_offset(&mut out, &default, last, nulls, true);
        self.write_offset(&mut out, &default, last, nulls, false);
        self.write_rotation(&mut out, &default, last, nulls);
        self.write_material(&mut out, &default, last, nulls);
        self.write_media(&mut out, &default, last, nulls);
        self.write_glow(&mut out, &default, last, nulls);
        self.write_material_id(&mut out, &default, last, nulls);
        out
    }

    /// `new TextureEntry(data, 0, data.Length)`.
    pub fn from_bytes(data: &[u8]) -> Result<Self, WireError> {
        if data.len() > MAX_TEXTURE_BYTES {
            return Err(WireError::Limit {
                limit: MAX_TEXTURE_BYTES,
                got: data.len(),
            });
        }
        if data.len() < 16 {
            return Ok(Self::blank());
        }
        let mut reader = Reader { data, pos: 0 };
        let mut entry = Self::blank();
        entry.default = Some(TextureFace::default_face());
        entry.default_mut().texture_id = reader.read_uuid()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_uuid(),
            TEXTURE_ID,
            |face, value| {
                face.texture_id = value;
            },
        )?;

        entry.default_mut().rgba = reader.read_color()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_color(),
            RGBA,
            |face, value| {
                face.rgba = value;
            },
        )?;

        entry.default_mut().repeat_u = reader.read_f32()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_f32(),
            REPEAT_U,
            |face, value| face.repeat_u = value,
        )?;

        entry.default_mut().repeat_v = reader.read_f32()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_f32(),
            REPEAT_V,
            |face, value| face.repeat_v = value,
        )?;

        entry.default_mut().offset_u = reader.read_i16()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_i16(),
            OFFSET_U,
            |face, value| face.offset_u = value,
        )?;

        entry.default_mut().offset_v = reader.read_i16()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_i16(),
            OFFSET_V,
            |face, value| face.offset_v = value,
        )?;

        entry.default_mut().rotation = reader.read_i16()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_i16(),
            ROTATION,
            |face, value| face.rotation = value,
        )?;

        entry.default_mut().material = reader.read_u8()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_u8(),
            MATERIAL,
            |face, value| face.material = value,
        )?;

        entry.default_mut().media = reader.read_u8()?;
        while reader.pos < reader.data.len() {
            match reader.read_bitfield()? {
                None => break,
                Some((bits, size)) => {
                    let value = reader.read_u8()?;
                    paint(&mut entry, bits, size, MEDIA, value, |face, value| {
                        face.media = value;
                    });
                }
            }
        }

        entry.default_mut().glow = reader.read_u8()?;
        reader.overrides(
            &mut entry,
            |reader| reader.read_u8(),
            GLOW,
            |face, value| {
                face.glow = value;
            },
        )?;

        if reader.remaining() >= 16 {
            entry.default_mut().material_id = reader.read_uuid()?;
            while reader.remaining() >= 16 {
                match reader.read_bitfield()? {
                    None => break,
                    Some((bits, size)) => {
                        let value = reader.read_uuid()?;
                        paint(&mut entry, bits, size, MATERIAL_ID, value, |face, value| {
                            face.material_id = value;
                        });
                    }
                }
            }
        }
        Ok(entry)
    }

    fn default_face(&self) -> &TextureFace {
        // INFALLIBLE: new() and a decoded entry install the default face before a setter runs.
        #[allow(clippy::expect_used)]
        self.default.as_ref().expect("texture entry default")
    }

    fn default_mut(&mut self) -> &mut TextureFace {
        // INFALLIBLE: new() and a decoded entry install the default face before a setter runs.
        #[allow(clippy::expect_used)]
        self.default.as_mut().expect("texture entry default")
    }

    fn write_textures(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        limit: usize,
    ) -> (Option<usize>, u64) {
        push_uuid(out, default.texture_id);
        let mut done = 0u64;
        let mut nulls = 0u64;
        let mut last = None;
        let mut cursor = limit;
        while cursor > 0 {
            cursor -= 1;
            let bit = 1u64 << cursor;
            let Some(face) = &self.faces[cursor] else {
                nulls |= bit;
                continue;
            };
            if last.is_none() {
                last = Some(cursor);
            }
            if done & bit != 0 || face.attributes & TEXTURE_ID == 0 {
                continue;
            }
            let id = face.texture_id;
            if id == default.texture_id {
                continue;
            }
            let cur = self.group_lower(cursor, &mut done, bit, &mut nulls, true, |other| {
                other.attributes & TEXTURE_ID != 0 && other.texture_id == id
            });
            write_face_bitfield(out, cur);
            push_uuid(out, id);
        }
        out.push(0);
        (last, nulls)
    }

    fn write_colors(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        last: Option<usize>,
        nulls: u64,
    ) {
        push_color(out, default.rgba, true);
        self.write_grouped(
            out,
            FaceSpan { last, nulls },
            default.rgba,
            RGBA,
            |face| face.rgba,
            |out, value| {
                push_color(out, value, true);
            },
        );
    }

    fn write_repeat(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        last: Option<usize>,
        nulls: u64,
        horizontal: bool,
    ) {
        let (attr, value) = if horizontal {
            (REPEAT_U, default.repeat_u)
        } else {
            (REPEAT_V, default.repeat_v)
        };
        push_f32(out, value);
        self.write_grouped(
            out,
            FaceSpan { last, nulls },
            value,
            attr,
            |face| {
                if horizontal {
                    face.repeat_u
                } else {
                    face.repeat_v
                }
            },
            push_f32,
        );
    }

    fn write_offset(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        last: Option<usize>,
        nulls: u64,
        horizontal: bool,
    ) {
        let (attr, value) = if horizontal {
            (OFFSET_U, default.offset_u)
        } else {
            (OFFSET_V, default.offset_v)
        };
        push_i16(out, value);
        self.write_grouped(
            out,
            FaceSpan { last, nulls },
            value,
            attr,
            |face| {
                if horizontal {
                    face.offset_u
                } else {
                    face.offset_v
                }
            },
            push_i16,
        );
    }

    fn write_rotation(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        last: Option<usize>,
        nulls: u64,
    ) {
        push_i16(out, default.rotation);
        self.write_grouped(
            out,
            FaceSpan { last, nulls },
            default.rotation,
            ROTATION,
            |face| face.rotation,
            push_i16,
        );
    }

    fn write_material(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        last: Option<usize>,
        nulls: u64,
    ) {
        out.push(default.material);
        self.write_grouped(
            out,
            FaceSpan { last, nulls },
            default.material,
            MATERIAL,
            |face| face.material,
            |out, value| out.push(value),
        );
    }

    fn write_media(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        last: Option<usize>,
        nulls: u64,
    ) {
        out.push(default.media);
        self.write_grouped(
            out,
            FaceSpan { last, nulls },
            default.media,
            MEDIA,
            |face| face.media,
            |out, value| out.push(value),
        );
    }

    fn write_glow(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        last: Option<usize>,
        nulls: u64,
    ) {
        out.push(default.glow);
        self.write_grouped(
            out,
            FaceSpan { last, nulls },
            default.glow,
            GLOW,
            |face| face.glow,
            |out, value| {
                out.push(value);
            },
        );
    }

    fn write_material_id(
        &self,
        out: &mut Vec<u8>,
        default: &TextureFace,
        last: Option<usize>,
        nulls: u64,
    ) {
        push_uuid(out, default.material_id);
        self.write_grouped(
            out,
            FaceSpan { last, nulls },
            default.material_id,
            MATERIAL_ID,
            |face| face.material_id,
            push_uuid,
        );
    }

    fn write_grouped<T: Copy + PartialEq>(
        &self,
        out: &mut Vec<u8>,
        span: FaceSpan,
        default_value: T,
        attr: u32,
        get: impl Fn(&TextureFace) -> T,
        write: impl Fn(&mut Vec<u8>, T),
    ) {
        let mut done = span.nulls;
        let mut nulls_unused = span.nulls;
        let Some(mut cursor) = span.last else {
            out.push(0);
            return;
        };
        loop {
            let bit = 1u64 << cursor;
            if done & bit == 0 {
                let face = match self.faces[cursor].as_ref() {
                    Some(face) => face,
                    None => {
                        // INFALLIBLE: a null face is recorded in the null mask, and this loop skips those bits.
                        #[allow(clippy::panic)]
                        {
                            panic!("null face was not in the null mask");
                        }
                    }
                };
                if face.attributes & attr != 0 {
                    let value = get(face);
                    if value != default_value {
                        let cur = self.group_lower(
                            cursor,
                            &mut done,
                            bit,
                            &mut nulls_unused,
                            false,
                            |other| other.attributes & attr != 0 && get(other) == value,
                        );
                        write_face_bitfield(out, cur);
                        write(out, value);
                    }
                }
            }
            if cursor == 0 {
                break;
            }
            cursor -= 1;
        }
        out.push(0);
    }

    fn group_lower(
        &self,
        index: usize,
        done: &mut u64,
        mut cur: u64,
        nulls: &mut u64,
        record_nulls: bool,
        same: impl Fn(&TextureFace) -> bool,
    ) -> u64 {
        if index == 0 {
            return cur;
        }
        let mut lower = index;
        while lower > 0 {
            lower -= 1;
            let next = 1u64 << lower;
            if *done & next != 0 {
                continue;
            }
            match &self.faces[lower] {
                None => {
                    if record_nulls {
                        *nulls |= next;
                        *done |= next;
                    }
                }
                Some(face) => {
                    if same(face) {
                        *done |= next;
                        cur |= next;
                    }
                }
            }
        }
        cur
    }
}

fn paint<T: Copy>(
    entry: &mut TextureEntry,
    bits: u64,
    size: usize,
    attr: u32,
    value: T,
    write: impl Fn(&mut TextureFace, T),
) {
    let count = size.min(MAX_FACES);
    for (index, slot) in entry.faces.iter_mut().enumerate().take(count) {
        if bits & (1u64 << index) == 0 {
            continue;
        }
        let face = slot.get_or_insert_with(TextureFace::child);
        write(face, value);
        face.attributes |= attr;
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos + len;
        if end > self.data.len() {
            return Err(WireError::Short {
                need: end,
                got: self.data.len(),
            });
        }
        let start = self.pos;
        self.pos = end;
        Ok(&self.data[start..end])
    }

    fn read_u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn read_i16(&mut self) -> Result<i16, WireError> {
        Ok(read_i16(self.take(2)?))
    }

    fn read_f32(&mut self) -> Result<f32, WireError> {
        Ok(read_f32(self.take(4)?))
    }

    fn read_uuid(&mut self) -> Result<Uuid, WireError> {
        read_uuid(self.take(16)?)
    }

    fn read_color(&mut self) -> Result<Color4, WireError> {
        read_color4(self.take(4)?, true)
    }

    /// `None` when the slice is exhausted or the next byte is the terminator.
    /// A continuation past the slice is [`WireError::Short`]. More than eight
    /// groups is [`WireError::Limit`].
    fn read_bitfield(&mut self) -> Result<Option<(u64, usize)>, WireError> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let mut byte = self.data[self.pos];
        self.pos += 1;
        if byte == 0 {
            return Ok(None);
        }
        let mut bits = u64::from(byte & 0x7f);
        let mut size = 7usize;
        let mut groups = 1usize;
        while byte & 0x80 != 0 {
            if self.pos >= self.data.len() {
                return Err(WireError::Short {
                    need: self.pos + 1,
                    got: self.data.len(),
                });
            }
            if groups >= 8 {
                return Err(WireError::Limit {
                    limit: 8,
                    got: groups + 1,
                });
            }
            byte = self.data[self.pos];
            self.pos += 1;
            bits = (bits << 7) | u64::from(byte & 0x7f);
            size += 7;
            groups += 1;
        }
        Ok(Some((bits, size)))
    }

    fn overrides<T: Copy>(
        &mut self,
        entry: &mut TextureEntry,
        mut read_value: impl FnMut(&mut Self) -> Result<T, WireError>,
        attr: u32,
        write: impl Fn(&mut TextureFace, T),
    ) -> Result<(), WireError> {
        loop {
            match self.read_bitfield()? {
                None => return Ok(()),
                Some((bits, size)) => {
                    let value = read_value(self)?;
                    paint(entry, bits, size, attr, value, &write);
                }
            }
        }
    }
}

fn write_face_bitfield(out: &mut Vec<u8>, bitfield: u64) {
    if bitfield == 0 {
        return;
    }
    if bitfield >= 0x80 {
        if bitfield >= 0x4000 {
            if bitfield >= 0x20_0000 {
                if bitfield >= 0x1000_0000 {
                    if bitfield >= 0x8_0000_0000 {
                        if bitfield >= 1u64 << 42 {
                            if bitfield >= 1u64 << 49 {
                                out.push(((bitfield >> 49) as u8) | 0x80);
                            }
                            out.push(((bitfield >> 42) as u8) | 0x80);
                        }
                        out.push(((bitfield >> 35) as u8) | 0x80);
                    }
                    out.push(((bitfield >> 28) as u8) | 0x80);
                }
                out.push(((bitfield >> 21) as u8) | 0x80);
            }
            out.push(((bitfield >> 14) as u8) | 0x80);
        }
        out.push(((bitfield >> 7) as u8) | 0x80);
    }
    out.push((bitfield & 0x7f) as u8);
}

fn push_uuid(out: &mut Vec<u8>, id: Uuid) {
    let mut buf = [0u8; 16];
    layout_ok(write_uuid(&id, &mut buf));
    out.extend_from_slice(&buf);
}

fn push_color(out: &mut Vec<u8>, color: Color4, inverted: bool) {
    let mut buf = [0u8; 4];
    layout_ok(write_color4(color, inverted, &mut buf));
    out.extend_from_slice(&buf);
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    let mut buf = [0u8; 4];
    wire::put_f32(&mut buf, value);
    out.extend_from_slice(&buf);
}

fn push_i16(out: &mut Vec<u8>, value: i16) {
    let mut buf = [0u8; 2];
    wire::put_i16(&mut buf, value);
    out.extend_from_slice(&buf);
}

/// Literal `6.2831853f` from `Helpers.TERotationShort` and `TERotationFloat`.
/// That is not [`f32::consts::TAU`].
#[allow(clippy::approx_constant)]
const TE_TWO_PI: f32 = 6.2831853;

/// The highest face a field walk may reach, and the null-face mask from the
/// texture pass. Later fields start from this so they do not read a null face.
#[derive(Clone, Copy)]
struct FaceSpan {
    last: Option<usize>,
    nulls: u64,
}

/// `Helpers.TEOffsetShort`.
fn te_offset_short(mut offset: f32) -> i16 {
    offset = clamp_f32(offset, -1.0, 1.0);
    offset *= 32767.0;
    if offset >= 0.0 {
        offset += 0.5;
    } else {
        offset -= 0.5;
    }
    offset as i16
}

/// `Helpers.TEOffsetFloat`.
fn te_offset_float(offset: i16) -> f32 {
    f32::from(offset) * (1.0 / 32767.0)
}

/// `Helpers.TERotationShort`. `Math.IEEERemainder` rounds halfway cases to even.
fn te_rotation_short(rotation: f32) -> i16 {
    const SCALED: f32 = 32768.0 / TE_TWO_PI;
    let mut rotation = ieee_remainder(rotation, TE_TWO_PI);
    rotation *= SCALED;
    if rotation >= 0.0 {
        rotation += 0.5;
    } else {
        rotation -= 0.5;
    }
    rotation as i16
}

/// `Helpers.TERotationFloat`.
fn te_rotation_float(rotation: i16) -> f32 {
    f32::from(rotation) * (TE_TWO_PI / 32768.0)
}

/// `Helpers.TEGlowByte` for a glow in `0..=1`, which is the range the fixtures use.
fn te_glow_byte(glow: f32) -> u8 {
    let scaled = glow * 255.0;
    if scaled <= 0.0 {
        return 0;
    }
    if scaled >= 255.0 {
        return 255;
    }
    scaled as u8
}

/// `Helpers.TEGlowFloat`.
fn te_glow_float(glow: u8) -> f32 {
    f32::from(glow) * (1.0 / 255.0)
}

fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
    if value < max {
        if value > min { value } else { min }
    } else {
        max
    }
}

fn ieee_remainder(x: f32, y: f32) -> f32 {
    let x = f64::from(x);
    let y = f64::from(y);
    let quotient = (x / y).round_ties_even();
    (x - y * quotient) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorter_than_a_uuid_has_no_default() {
        let entry = TextureEntry::from_bytes(&[0; 8]).unwrap();
        assert!(entry.default_view().is_none());
        assert!(entry.to_bytes().is_empty());
    }

    #[test]
    fn a_uuid_without_the_rest_is_short() {
        let err = TextureEntry::from_bytes(&[0; 16]).unwrap_err();
        assert!(matches!(err, WireError::Short { .. }));
    }

    #[test]
    fn over_the_cap_is_rejected() {
        let err = TextureEntry::from_bytes(&vec![0; MAX_TEXTURE_BYTES + 1]).unwrap_err();
        assert_eq!(
            err,
            WireError::Limit {
                limit: MAX_TEXTURE_BYTES,
                got: MAX_TEXTURE_BYTES + 1,
            }
        );
    }
}
