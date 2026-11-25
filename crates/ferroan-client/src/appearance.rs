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
//
// Copyright (c) 2006-2016, openmetaverse.co
// All rights reserved.
//
// - Redistribution and use in source and binary forms, with or without
//   modification, are permitted provided that the following conditions are met:
//
// - Redistributions of source code must retain the above copyright notice, this
//   list of conditions and the following disclaimer.
// - Neither the name of the openmetaverse.co nor the names
//   of its contributors may be used to endorse or promote products derived from
//   this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
//

// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (SendAppearance, CreateAvatarUpdateBlock, SendEntityFullUpdateImmediate)
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs
//     (RotateToLookAt, SendAppearanceToAgentNF, OffsetPosition)
//   Source/OpenSim.Framework/AvatarAppearance.cs
//     (constructor, SetDefaultParams, SetDefaultTexture, SetSize)
//   KanikaBreed/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
//   (the live block for another viewer, Q48: CreateAvatarUpdateBlock's
//   CollisionPlane, Velocity and Rotation, at
//   c401bceecd1684832ae8ba86a31b5ee01a4ded7b)
//   OpenMetaverse/Primitives/TextureEntry.cs (GetBakesBytes, default face)
//   OpenMetaverse/AppearanceManager.cs (DEFAULT_AVATAR_TEXTURE)
//   OpenMetaverse.Types/Quaternion.cs (ToBytes)
//   OpenMetaverse.Types/UtilsConversions.cs (StringToBytes, FloatZeroOneToByte)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The two packets that take an avatar out of the cloud, without baking.
//!
//! `SendAppearanceToAgentNF` uses cofVersion -1 when the region is not baking:
//! `AppearanceData` count 0, then one `AppearanceHover`. `CreateAvatarUpdateBlock`
//! writes one avatar object. Attachment blocks, animations and parcel overlay
//! stay with their own rows.

use ferroan_protocol::msgs::avatar_appearance::{
    AppearanceHover, ObjectData as AppearanceObject, Sender,
};
use ferroan_protocol::msgs::object_update::{ObjectData, RegionData};
use ferroan_protocol::msgs::{AvatarAppearance, ObjectUpdate};
use ferroan_protocol::str_field;
use uuid::Uuid;

/// `PCode.Avatar` in `EnumsPrimitive.cs`.
const PCODE_AVATAR: u8 = 47;
/// `Material.Flesh`.
const MATERIAL_FLESH: u8 = 4;
/// `PhysicsScene.TimeDilation` is 1, and `FloatZeroOneToUshort` maps that to this.
const TIME_DILATION_FULL: u16 = 0xffff;
/// Standing size from `AvatarAppearance`'s constructor. Height is the Z component.
const DEFAULT_HEIGHT: f32 = 1.9;
const SCALE_X: f32 = 0.45;
const SCALE_Y: f32 = 0.6;
/// `AppearanceManager.DEFAULT_AVATAR_TEXTURE`, RFC byte order.
const DEFAULT_TEXTURE: [u8; 16] = [
    0xc2, 0x28, 0xd1, 0xcf, 0x4b, 0x5d, 0x4b, 0xa8, 0x84, 0xf4, 0x89, 0x9a, 0x07, 0x96, 0xaa, 0x97,
];
/// `AvatarAppearance.SetDefaultParams`. The template count is one byte, and this list fits.
const DEFAULT_VISUAL_PARAMS: &[u8] = &[
    33, 61, 85, 23, 58, 127, 63, 85, 63, 42, 0, 85, 63, 36, 85, 95, 153, 63, 34, 0, 63, 109, 88,
    132, 63, 136, 81, 85, 103, 136, 127, 0, 150, 150, 150, 127, 0, 0, 0, 0, 0, 127, 0, 0, 255, 127,
    114, 127, 99, 63, 127, 140, 127, 127, 0, 0, 0, 191, 0, 104, 0, 0, 0, 0, 0, 0, 0, 0, 0, 145,
    216, 133, 0, 127, 0, 127, 170, 0, 0, 127, 127, 109, 85, 127, 127, 63, 85, 42, 150, 150, 150,
    150, 150, 150, 150, 25, 150, 150, 150, 0, 127, 0, 0, 144, 85, 127, 132, 127, 85, 0, 127, 127,
    127, 127, 127, 127, 59, 127, 85, 127, 127, 106, 47, 79, 127, 127, 204, 2, 141, 66, 0, 0, 127,
    127, 0, 0, 0, 0, 127, 0, 159, 0, 0, 178, 127, 36, 85, 131, 127, 127, 127, 153, 95, 0, 140, 75,
    27, 127, 127, 0, 150, 150, 198, 0, 0, 63, 30, 127, 165, 209, 198, 127, 127, 153, 204, 51, 51,
    255, 255, 255, 204, 0, 255, 150, 150, 150, 150, 150, 150, 150, 150, 150, 150, 0, 150, 150, 150,
    150, 150, 0, 127, 127, 150, 150, 150, 150, 150, 150, 150, 150, 0, 0, 150, 51, 132, 150, 150,
    150,
];

/// One root avatar the region is about to announce. Wearables and attachments
/// are not on these two packets. Texture bytes are the default bake entry:
/// `ToAvatarAppearance` does not load per-face textures.
#[derive(Clone, Debug)]
pub struct AvatarWire {
    pub agent_id: Uuid,
    /// `ScenePresence.LocalId`. The comparator folds it.
    pub local_id: u32,
    pub first_name: String,
    pub last_name: String,
    /// `ScenePresence.Grouptitle`. Empty when no groups module set a title.
    pub title: String,
    pub visual_params: Vec<u8>,
    /// `AvatarAppearance.AvatarHeight`, the Z of `AvatarSize`.
    pub height: f32,
    /// `AvatarPreferencesHoverZ`. The reader leaves this at 0.
    pub hover: f32,
    /// `TextureEntry.GetBakesBytes` of the default avatar texture.
    pub texture: Vec<u8>,
}

/// The presence values `CreateAvatarUpdateBlock` reads when it writes the
/// block for another viewer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveAvatar {
    pub collision_plane: [f32; 4],
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub rotation: [f32; 4],
}

/// Fields `ToAvatarAppearance` copies onto the constructor defaults.
#[derive(Clone, Debug, Default)]
pub struct StoredAppearance {
    pub visual_params: Option<Vec<u8>>,
    pub height: Option<f32>,
}

impl AvatarWire {
    /// Constructor defaults: `SetDefaultParams`, `SetDefaultTexture`, `SetSize(0.45, 0.6, 1.9)`.
    pub fn standing(agent_id: Uuid, local_id: u32, first_name: &str, last_name: &str) -> Self {
        Self {
            agent_id,
            local_id,
            first_name: first_name.to_owned(),
            last_name: last_name.to_owned(),
            title: String::new(),
            visual_params: DEFAULT_VISUAL_PARAMS.to_vec(),
            height: DEFAULT_HEIGHT,
            hover: 0.0,
            texture: default_bake_texture(),
        }
    }

    pub fn apply_stored(&mut self, stored: &StoredAppearance) {
        if let Some(params) = &stored.visual_params {
            self.visual_params.clone_from(params);
        }
        if let Some(height) = stored.height {
            self.height = height;
        }
    }

    pub fn object_update(
        &self,
        region_handle: u64,
        position: [f32; 3],
        look_at: [f32; 3],
    ) -> ObjectUpdate {
        let object = ObjectData {
            id: self.local_id,
            full_id: self.agent_id,
            p_code: PCODE_AVATAR,
            material: MATERIAL_FLESH,
            scale: [SCALE_X, SCALE_Y, self.height].into(),
            object_data: avatar_blob(position, look_at),
            name_value: name_value(&self.first_name, &self.last_name, &self.title),
            ..ObjectData::default()
        };
        ObjectUpdate {
            region_data: RegionData {
                region_handle,
                time_dilation: TIME_DILATION_FULL,
            },
            object_data: vec![object],
            omitted_trailing_blocks: 0,
        }
    }

    /// `CreateAvatarUpdateBlock` from the live presence values, as another
    /// viewer gets it (`SendEntityFullUpdateImmediate`). `rotation` already
    /// has X and Y zeroed for an avatar that neither flies nor sits.
    pub fn live_update(&self, region_handle: u64, live: &LiveAvatar) -> ObjectUpdate {
        let mut update = self.object_update(region_handle, live.position, [0.0; 3]);
        let mut blob = vec![0u8; 76];
        for (index, component) in live.collision_plane.into_iter().enumerate() {
            let start = index * 4;
            blob[start..start + 4].copy_from_slice(&component.to_le_bytes());
        }
        write_vec3(&mut blob[16..28], live.position);
        write_vec3(&mut blob[28..40], live.velocity);
        write_vec3(&mut blob[52..64], pack_rotation(live.rotation));
        update.object_data[0].object_data = blob;
        update
    }

    /// cofVersion -1: no `AppearanceData` block, one hover vector, no attachment count.
    pub fn appearance(&self) -> AvatarAppearance {
        AvatarAppearance {
            sender: Sender {
                id: self.agent_id,
                is_trial: false,
            },
            object_data: AppearanceObject {
                texture_entry: self.texture.clone(),
            },
            visual_param: self
                .visual_params
                .iter()
                .take(255)
                .map(
                    |param_value| ferroan_protocol::msgs::avatar_appearance::VisualParam {
                        param_value: *param_value,
                    },
                )
                .collect(),
            appearance_data: Vec::new(),
            appearance_hover: vec![AppearanceHover {
                hover_height: [0.0, 0.0, self.hover].into(),
            }],
            attachment_block: Vec::new(),
            omitted_trailing_blocks: 1,
        }
    }
}

/// `GetBakesBytes` when every baked face is absent. Each section is the default
/// value and a zero bitfield terminator. White inverted is four zero bytes.
fn default_bake_texture() -> Vec<u8> {
    let mut out = vec![0u8; 64];
    out[..16].copy_from_slice(&DEFAULT_TEXTURE);
    // RepeatU and RepeatV are 1. Color, offsets, rotation, material, media,
    // glow and the material id stay zero, which the cleared buffer already is.
    let one = 1.0f32.to_le_bytes();
    out[22..26].copy_from_slice(&one);
    out[27..31].copy_from_slice(&one);
    out
}

fn name_value(first: &str, last: &str, title: &str) -> Vec<u8> {
    // HideTitle is false, so the title is Grouptitle, including the space when it is empty.
    str_field(&format!(
        "FirstName STRING RW SV {first}\nLastName STRING RW SV {last}\nTitle STRING RW SV {title}"
    ))
}

/// The 76-byte avatar `ObjectData`: foot plane, position, zero velocity and
/// acceleration, Z-only rotation, zero angular velocity.
fn avatar_blob(position: [f32; 3], look_at: [f32; 3]) -> Vec<u8> {
    let mut blob = vec![0u8; 76];
    blob[12..16].copy_from_slice(&1.0f32.to_le_bytes());
    write_vec3(&mut blob[16..28], position);
    write_vec3(&mut blob[52..64], packed_look(look_at));
    blob
}

fn write_vec3(dest: &mut [u8], value: [f32; 3]) {
    for (index, component) in value.into_iter().enumerate() {
        let start = index * 4;
        dest[start..start + 4].copy_from_slice(&component.to_le_bytes());
    }
}

/// `Quaternion.ToBytes`: X, Y and Z scaled by the reciprocal length, negated
/// when W is negative. The branch that writes zeros is reached only by NaN.
fn pack_rotation(rotation: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = rotation;
    let norm = x * x + y * y + z * z + w * w;
    if norm > 1e-6 || norm < 0.9999 {
        let scale = if w < 0.0 {
            -1.0 / norm.sqrt()
        } else {
            1.0 / norm.sqrt()
        };
        [scale * x, scale * y, scale * z]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// `RotateToLookAt` for an unparented body, then `Quaternion.ToBytes`.
/// X and Y are forced to 0 before the pack when the avatar is not flying.
fn packed_look(look: [f32; 3]) -> [f32; 3] {
    let horizontal = look[0] * look[0] + look[1] * look[1];
    if horizontal < 0.0001 {
        return [0.0, 0.0, 0.0];
    }
    let cosine = look[0] / horizontal.sqrt();
    let angle = cosine.acos() * 0.5;
    let z = if look[1] >= 0.0 {
        angle.sin()
    } else {
        -angle.sin()
    };
    let w = angle.cos();
    let norm = z * z + w * w;
    if norm > 1e-6 || norm < 0.9999 {
        let scale = if w < 0.0 {
            -1.0 / norm.sqrt()
        } else {
            1.0 / norm.sqrt()
        };
        [0.0, 0.0, scale * z]
    } else {
        [0.0, 0.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use ferroan_protocol::Msg;

    use super::*;

    /// The block another viewer gets carries the live plane, position,
    /// velocity and rotation; everything else is the arrival block.
    #[test]
    fn a_live_update_writes_the_presence_values() {
        let wire = AvatarWire::standing(Uuid::from_u128(7), 7, "Test", "User");
        let look = [0.99, 0.042, 0.0];
        let arrival = wire.object_update(99, [128.0, 128.0, 25.96], look);
        let half = (0.99f32 / (0.99f32 * 0.99 + 0.042 * 0.042).sqrt()).acos() * 0.5;
        let still = LiveAvatar {
            collision_plane: [0.0, 0.0, 0.0, 1.0],
            position: [128.0, 128.0, 25.96],
            velocity: [0.0; 3],
            rotation: [0.0, 0.0, half.sin(), half.cos()],
        };
        assert_eq!(wire.live_update(99, &still), arrival);
        let moving = LiveAvatar {
            collision_plane: [0.0, 0.0, 1.0, 21.5],
            position: [130.0, 128.0, 22.0],
            velocity: [2.0, 0.5, 0.0],
            rotation: [0.0, 0.0, -0.6, -0.8],
        };
        let update = wire.live_update(99, &moving);
        let blob = &update.object_data[0].object_data;
        assert_eq!(&blob[8..12], &1.0f32.to_le_bytes());
        assert_eq!(&blob[12..16], &21.5f32.to_le_bytes());
        assert_eq!(&blob[16..20], &130.0f32.to_le_bytes());
        assert_eq!(&blob[28..32], &2.0f32.to_le_bytes());
        assert_eq!(&blob[40..52], &[0u8; 12]);
        // A negative W flips the packed axes.
        assert_eq!(&blob[60..64], &0.6f32.to_le_bytes());
        assert_eq!(&blob[64..76], &[0u8; 12]);
    }

    #[test]
    fn default_bake_is_the_64_byte_face_entry() {
        let texture = default_bake_texture();
        assert_eq!(texture.len(), 64);
        assert_eq!(&texture[..16], &DEFAULT_TEXTURE);
        assert_eq!(texture[16], 0);
        assert_eq!(&texture[17..22], &[0, 0, 0, 0, 0]);
        assert_eq!(&texture[22..26], &1.0f32.to_le_bytes());
        assert_eq!(texture[26], 0);
        assert_eq!(&texture[27..31], &1.0f32.to_le_bytes());
        assert_eq!(&texture[31..], &[0u8; 33]);
    }

    #[test]
    fn standing_packets_use_defaults_and_omit_the_bake_block() {
        let wire = AvatarWire::standing(Uuid::from_u128(7), 7, "Test", "User");
        assert_eq!(wire.visual_params, DEFAULT_VISUAL_PARAMS);
        assert!(wire.visual_params.len() <= 255);
        let update = wire.object_update(99, [128.0, 128.0, 25.0], [0.99, 0.042, 0.0]);
        assert_eq!(update.region_data.time_dilation, TIME_DILATION_FULL);
        assert_eq!(update.object_data.len(), 1);
        let object = &update.object_data[0];
        assert_eq!(object.p_code, PCODE_AVATAR);
        assert_eq!(object.material, MATERIAL_FLESH);
        assert_eq!(object.object_data.len(), 76);
        assert_eq!(&object.object_data[12..16], &1.0f32.to_le_bytes());
        assert_eq!(&object.object_data[52..60], &[0u8; 8]);
        let name = String::from_utf8(object.name_value.clone()).unwrap();
        assert!(name.contains("FirstName STRING RW SV Test\n"));
        assert!(name.contains("LastName STRING RW SV User\n"));
        assert!(name.ends_with("Title STRING RW SV \0"));
        let appearance = wire.appearance();
        assert!(appearance.appearance_data.is_empty());
        assert_eq!(appearance.appearance_hover.len(), 1);
        assert!(appearance.attachment_block.is_empty());
        assert_eq!(appearance.omitted_trailing_blocks, 1);
        assert_eq!(appearance.visual_param.len(), DEFAULT_VISUAL_PARAMS.len());
        assert_eq!(appearance.object_data.texture_entry.len(), 64);
    }

    /// Grid login at `c70fee205f`. See `fixtures/appearance/README.md`.
    #[test]
    fn captured_grid_login_matches_the_standing_avatar() {
        let captured_params = include_bytes!("../../../fixtures/appearance/visual-params.bin");
        assert_eq!(captured_params, DEFAULT_VISUAL_PARAMS);
        let agent = Uuid::parse_str("4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55").unwrap();
        let wire = AvatarWire::standing(agent, 0, "Test", "User");
        let appearance = wire.appearance();
        let mut encoded = ferroan_protocol::Writer::new();
        appearance.encode_body(&mut encoded);
        assert_body(
            &encoded.buf,
            include_bytes!("../../../fixtures/appearance/avatar-appearance.body"),
        );

        // Home is map tile 1000,1000. Packed Z is folded to 0; X and Y stay.
        let handle = (1_000u64 * 256) << 32 | (1_000 * 256);
        let mut update = wire.object_update(handle, [128.0, 128.0, 0.0], [0.99, 0.042, 0.0]);
        let object = &mut update.object_data[0];
        object.crc = 0;
        object.name_value = b"Redacted".to_vec();
        let mut encoded = ferroan_protocol::Writer::new();
        update.encode_body(&mut encoded);
        assert_body(
            &encoded.buf,
            include_bytes!("../../../fixtures/appearance/object-update.body"),
        );
    }

    fn assert_body(built: &[u8], captured: &[u8]) {
        if built == captured {
            return;
        }
        let offset = built
            .iter()
            .zip(captured)
            .position(|(left, right)| left != right)
            .unwrap_or(built.len().min(captured.len()));
        panic!(
            "body differs at {offset} (lengths {} and {})",
            built.len(),
            captured.len()
        );
    }
}
