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
// Ported from LLClientView.cs (CreatePrimUpdateBlock), LLUDPZeroEncoder.cs, SceneObjectPart.cs,
// SceneObjectGroup.Inventory.cs, SceneObjectPartInventory.cs and PermissionsModule.cs at Tranquillity
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b, and OSUTF8.cs / UtilsConversions.cs
// at libopenmetaverse a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.

//! Initial full ObjectUpdate block for a restored world prim.

use std::time::{SystemTime, UNIX_EPOCH};

use ferroan_diag::{Action, Hit, Kind, OBJECT_UPDATE_FIELD};
use ferroan_protocol::{Writer, msgs::object_update::ObjectData};
use ferroan_types::{PERM_ALL_EFFECTIVE, PERM_COPY, PERM_MODIFY, PERM_MOVE, PERM_TRANSFER};
use uuid::Uuid;

use crate::{ObjectPart, xml::shape_update_extras};

/// A store value that cannot fit its LLUDP length prefix.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("prim {field} uses {length} bytes (wire maximum {maximum})")]
pub struct PrimUpdateError {
    pub field: &'static str,
    pub length: usize,
    pub maximum: usize,
}

const GRASS: u8 = 95;
const NEW_TREE: u8 = 111;
const TREE: u8 = 255;

const OBJECT_MODIFY: u32 = 0x4;
const OBJECT_COPY: u32 = 0x8;
const ANY_OWNER: u32 = 0x10;
const YOU_OWNER: u32 = 0x20;
const OBJECT_MOVE: u32 = 0x100;
const OBJECT_TRANSFER: u32 = 0x2_0000;
const OWNER_MODIFY: u32 = 0x1000_0000;
const VIEWER_FLAGS: u32 = OBJECT_MODIFY
    | OBJECT_COPY
    | ANY_OWNER
    | YOU_OWNER
    | OBJECT_MOVE
    | OBJECT_TRANSFER
    | OWNER_MODIFY;

/// Owner permissions shared by a restored linkset, computed once before encoding its parts.
pub struct LinksetUpdateFlags {
    root: Uuid,
    effective_owner: u32,
}

impl LinksetUpdateFlags {
    /// SceneObjectGroup.AggregatePerms intersects the owner's rights with the contents
    /// of every part except landmarks, then restores the root's modify and move bits.
    pub fn from_parts(root: &ObjectPart, children: &[ObjectPart]) -> Self {
        let contents = std::iter::once(root)
            .chain(children)
            .flat_map(|part| &part.items)
            .filter(|item| item.inv_type != 3) // InventoryType.Landmark
            .fold(PERM_ALL_EFFECTIVE, |mask, item| {
                mask & item.current_permissions
            });
        let mut owner = root.owner_mask & (contents | PERM_MODIFY | PERM_MOVE);
        if owner & (PERM_COPY | PERM_TRANSFER) == 0 {
            owner |= PERM_TRANSFER;
        }
        owner &= (root.base_mask & PERM_ALL_EFFECTIVE) | PERM_MOVE;
        Self {
            root: root.uuid,
            effective_owner: owner,
        }
    }
}

/// Restore once per part. All viewers see the same local IDs, payload and cache revision.
/// The owner-only flags stand in for GenerateClientFlags until P2.7b.
pub struct StoredPrimUpdate {
    block: ObjectData,
    owner: Uuid,
    owner_flags: u32,
}

impl StoredPrimUpdate {
    /// The frame supplies this part's local ID and its root's local ID.
    pub fn from_store(
        part: &ObjectPart,
        linkset: &LinksetUpdateFlags,
        local_id: u32,
        root_local_id: u32,
    ) -> Result<Self, PrimUpdateError> {
        let shape = &part.shape;
        let is_root = part.uuid == linkset.root;
        let vegetation = matches!(shape.pcode, GRASS | NEW_TREE | TREE);
        let mut motion = Writer::new();
        motion.vec3(
            if is_root {
                part.group_position
            } else {
                part.offset
            }
            .into(),
        );
        if vegetation {
            motion.buf.resize(36, 0); // position, velocity and acceleration
            if shape.pcode != GRASS {
                motion.quat(glam::Quat::from_array(part.rotation));
            }
            motion.buf.resize(60, 0);
        } else {
            motion.vec3(part.velocity.into());
            motion.vec3(part.acceleration.into());
            motion.quat(glam::Quat::from_array(part.rotation));
            motion.vec3(part.angular_velocity.into());
        }

        let mut flags = part.flags & !(VIEWER_FLAGS | 0x2); // restored prims are never CreateSelected
        if is_root {
            flags &= !0x40; // Scene.LoadPrimsFromStorage clears Scripted on the root
        }
        if part.items.is_empty() {
            flags |= 0x800; // InventoryEmpty
        }
        if !part.owner_id.is_nil() {
            flags |= ANY_OWNER;
        }
        let mut block = ObjectData {
            id: local_id,
            full_id: part.uuid,
            crc: restored_crc(part.pseudo_crc, is_root && linkset.effective_owner != 0),
            state: shape.state,
            p_code: shape.pcode,
            scale: shape.scale.into(),
            object_data: motion.buf,
            parent_id: if is_root { 0 } else { root_local_id },
            update_flags: flags,
            text_color: vec![0; 4],
            ..Default::default()
        };
        if vegetation {
            block.data.push(shape.state);
        } else {
            fill_prim(&mut block, part)?;
        }
        Ok(Self {
            block,
            owner: part.owner_id,
            owner_flags: owner_flags(linkset.effective_owner),
        })
    }

    pub fn block_for(&self, viewer: Uuid) -> ObjectData {
        let mut block = self.block.clone();
        if viewer == self.owner && !viewer.is_nil() {
            block.update_flags |= self.owner_flags;
        }
        block
    }
}

fn owner_flags(mask: u32) -> u32 {
    let mut flags = YOU_OWNER | ANY_OWNER;
    if mask & PERM_COPY != 0 {
        flags |= OBJECT_COPY;
    }
    if mask & PERM_TRANSFER != 0 {
        flags |= OBJECT_TRANSFER;
    }
    if mask & PERM_MOVE != 0 {
        flags |= OBJECT_MOVE;
        if mask & PERM_MODIFY != 0 {
            flags |= OBJECT_MODIFY | OWNER_MODIFY;
        }
    }
    flags
}

fn restored_crc(stored: Option<i32>, root_update: bool) -> u32 {
    // SceneObjectPart uses the low 32 bits of DateTime.UtcNow.Ticks for a missing revision.
    let crc = stored.filter(|value| *value != 0).map_or_else(
        || {
            const EPOCH_TICKS: u64 = 621_355_968_000_000_000;
            let unix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            EPOCH_TICKS.wrapping_add((unix.as_nanos() / 100) as u64) as u32
        },
        |crc| crc as u32,
    );
    // On the first GenerateClientFlags after restore, AggregatePerms schedules a
    // full update for the root when effective permissions change. All three other
    // effective masks are intersected with the owner mask, and all start at zero.
    crc.wrapping_add(u32::from(root_update))
}

fn check_size(field: &'static str, length: usize, maximum: usize) -> Result<(), PrimUpdateError> {
    if length <= maximum {
        return Ok(());
    }
    ferroan_diag::refuse(
        OBJECT_UPDATE_FIELD,
        Kind::Hard,
        maximum as u64,
        length as u64,
        field,
    );
    Err(PrimUpdateError {
        field,
        length,
        maximum,
    })
}

fn short_text(field: &'static str, text: &str) -> Vec<u8> {
    const MAX: usize = 254;
    let prefix = &text.as_bytes()[..text.len().min(MAX + 1)];
    let len = if let Some(nul) = prefix.iter().position(|b| *b == 0) {
        nul
    } else if text.len() > MAX {
        ferroan_diag::hit(&Hit {
            id: OBJECT_UPDATE_FIELD,
            kind: Kind::Hard,
            configured: MAX as u64,
            observed: text.len() as u64,
            action: Action::Truncate,
            route: field,
            agent_id: None,
        });
        text.floor_char_boundary(MAX)
    } else {
        text.len()
    };
    let mut bytes = text.as_bytes()[..len].to_vec();
    if !bytes.is_empty() {
        bytes.push(0);
    }
    bytes
}

fn fill_prim(block: &mut ObjectData, part: &ObjectPart) -> Result<(), PrimUpdateError> {
    let shape = &part.shape;
    check_size("TextureEntry", shape.texture.len(), u16::MAX.into())?;
    check_size("TextureAnim", part.texture_animation.len(), u8::MAX.into())?;
    check_size("PSBlock", part.particles.len(), u8::MAX.into())?;
    // The store setter reads at most 255 records and 255 material entries per record;
    // the wire limit applies to the bytes the shape getter actually emits.
    let (sculpt_type, extra) = shape_update_extras(&shape.extra_params);
    check_size("ExtraParams", extra.len(), u8::MAX.into())?;

    block.material = part.material;
    block.click_action = part.click_action;
    block.path_curve = shape.path_curve;
    block.profile_curve = shape.profile_curve;
    block.path_begin = shape.path_begin;
    block.path_end = shape.path_end;
    block.path_scale_x = shape.path_scale_x;
    block.path_scale_y = shape.path_scale_y;
    block.path_shear_x = shape.path_shear_x;
    block.path_shear_y = shape.path_shear_y;
    block.path_twist = shape.path_twist;
    block.path_twist_begin = shape.path_twist_begin;
    block.path_radius_offset = shape.path_radius_offset;
    block.path_taper_x = shape.path_taper_x;
    block.path_taper_y = shape.path_taper_y;
    block.path_revolutions = shape.path_revolutions;
    block.path_skew = shape.path_skew;
    block.profile_begin = shape.profile_begin;
    block.profile_end = shape.profile_end;
    block.profile_hollow = shape.profile_hollow;
    if sculpt_type == 5 {
        block.profile_curve &= 0x0f;
        if block.profile_begin == 1 {
            block.profile_begin = 9375;
        }
        if block.profile_hollow == 1 {
            block.profile_hollow = 27500;
        }
        if block.profile_curve == 0 {
            block.path_scale_y = block.path_scale_y.max(150);
        }
    }

    block.texture_entry.clone_from(&shape.texture);
    block.texture_anim.clone_from(&part.texture_animation);
    if extra.len() >= 2 {
        block.extra_params = extra;
    }
    block.text = short_text("Text", &part.text);
    if !block.text.is_empty() {
        let [a, r, g, b] = part.color.to_be_bytes();
        block.text_color = vec![r, g, b, !a];
    }
    block.media_url = short_text("MediaURL", &part.media_url);
    block.ps_block.clone_from(&part.particles);
    let has_sound = !part.sound.is_nil() || part.sound_flags != 0;
    if has_sound {
        block.sound = part.sound;
        block.gain = part.sound_gain as f32;
        block.flags = part.sound_flags;
        // SoundRadius is not persisted; a restored part starts at zero.
    }
    if has_sound || !part.particles.is_empty() {
        block.owner_id = part.owner_id;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObjectGroup, ObjectStoreKind, open_objects, test_store::CopiedStore};
    use std::collections::BTreeMap;

    fn single(part: &ObjectPart) -> Result<StoredPrimUpdate, PrimUpdateError> {
        StoredPrimUpdate::from_store(part, &LinksetUpdateFlags::from_parts(part, &[]), 1, 1)
    }

    const FIXTURE: &str = "../../fixtures/prim-updates";
    async fn load_capture_store() -> Vec<ObjectGroup> {
        let copy = CopiedStore::new("prim-updates/scene.db");
        let region = Uuid::from_u128(0x6d1a7f4e_0c3b_4f53_9a1e_5f0e3c2a9b01);
        open_objects(&copy.connection(), region, ObjectStoreKind::Sqlite)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn stored_parts_match_every_byte_of_the_pin8_full_updates() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        let map: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("map.json")).unwrap()).unwrap();
        let owner = &map["owner"];
        let store_owner: Uuid = owner["store_uuid"].as_str().unwrap().parse().unwrap();
        let capture_owner: Uuid = owner["capture_uuid"].as_str().unwrap().parse().unwrap();
        let mut aliases = BTreeMap::new();
        for entry in map["parts"].as_array().unwrap() {
            let store: Uuid = entry["store_uuid"].as_str().unwrap().parse().unwrap();
            let alias: Uuid = entry["capture_uuid"].as_str().unwrap().parse().unwrap();
            aliases.insert(store, (alias, entry));
        }
        let groups = load_capture_store().await;
        assert_eq!(
            groups.iter().map(|g| g.parts.len()).sum::<usize>(),
            aliases.len()
        );
        for group in groups {
            let (root, children) = group.parts.split_first().unwrap();
            let flags = LinksetUpdateFlags::from_parts(root, children);
            let root_local = aliases[&root.uuid].0.as_u128() as u32;
            for part in &group.parts {
                let (alias, entry) = aliases[&part.uuid];
                let local = alias.as_u128() as u32;
                let mut block = StoredPrimUpdate::from_store(part, &flags, local, root_local)
                    .unwrap()
                    .block_for(store_owner);
                assert_eq!(block.full_id, part.uuid);
                block.full_id = alias;
                if !block.owner_id.is_nil() {
                    assert_eq!(block.owner_id, store_owner);
                    block.owner_id = capture_owner;
                }
                let mut writer = Writer::new();
                block.encode(&mut writer);
                let expected = std::fs::read(dir.join(entry["block"].as_str().unwrap())).unwrap();
                assert_eq!(writer.buf, expected, "full block for {}", part.name);
            }
        }
    }

    #[tokio::test]
    async fn only_trees_keep_rotation_and_only_ordinary_prims_keep_motion() {
        let mut part = load_capture_store().await.remove(0).parts.remove(0);
        part.rotation = [1., 0., 0., 0.];
        part.velocity = [1.; 3];
        part.acceleration = [2.; 3];
        part.angular_velocity = [3.; 3];
        part.shape.state = 42;
        for code in [9, GRASS, NEW_TREE, TREE] {
            part.shape.pcode = code;
            let block = single(&part).unwrap().block_for(part.owner_id);
            let floats: Vec<_> = block
                .object_data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|f| f32::from_le_bytes(*f))
                .collect();
            assert_eq!(floats[..3], part.group_position);
            assert_eq!(
                floats[3..6],
                if code == 9 { part.velocity } else { [0.; 3] }
            );
            assert_eq!(
                floats[6..9],
                if code == 9 {
                    part.acceleration
                } else {
                    [0.; 3]
                }
            );
            assert_eq!(
                floats[9..12],
                if code == GRASS { [0.; 3] } else { [1., 0., 0.] }
            );
            assert_eq!(
                floats[12..],
                if code == 9 {
                    part.angular_velocity
                } else {
                    [0.; 3]
                }
            );
            if code != 9 {
                assert_eq!(block.data, [42]);
                assert!(block.owner_id.is_nil());
                assert!(block.texture_entry.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn sound_or_particles_send_owner_independently_of_viewer_permissions() {
        let mut part = load_capture_store().await.remove(0).parts.remove(0);
        part.sound_gain = 0.625;
        part.base_mask = PERM_MODIFY;
        part.owner_mask = PERM_MODIFY; // modify on a locked root gives no modify privilege
        part.flags |= VIEWER_FLAGS | 2;
        part.pseudo_crc = None;
        for (sound_id, sound_flags, particles) in
            [(0, 0, false), (0, 0, true), (0, 2, false), (42, 0, false)]
        {
            part.sound = Uuid::from_u128(sound_id);
            part.sound_flags = sound_flags;
            part.particles = if particles { vec![7] } else { vec![] };
            let prim = single(&part).unwrap();
            let owner = prim.block_for(part.owner_id);
            let other = prim.block_for(Uuid::from_u128(1));
            assert_eq!(owner.crc, other.crc);
            assert_eq!(
                owner.update_flags & (VIEWER_FLAGS | 2),
                YOU_OWNER | ANY_OWNER
            );
            assert_eq!(other.update_flags & (VIEWER_FLAGS | 2), ANY_OWNER);
            let sound = sound_id != 0 || sound_flags != 0;
            assert_eq!(
                other.owner_id,
                if sound || particles {
                    part.owner_id
                } else {
                    Uuid::nil()
                }
            );
            assert_eq!(other.gain, if sound { 0.625 } else { 0. });
            assert_eq!(other.flags, sound_flags);
        }
    }

    #[tokio::test]
    async fn first_permission_aggregation_revises_only_a_changed_root() {
        let mut group = load_capture_store().await.remove(0);
        let (root, children) = group.parts.split_first_mut().unwrap();
        root.pseudo_crc = Some(-1);
        root.base_mask = 0;
        root.owner_mask = 0;
        let encode = |part: &ObjectPart, flags: &LinksetUpdateFlags| {
            StoredPrimUpdate::from_store(part, flags, 1, 1)
                .unwrap()
                .block_for(part.owner_id)
        };
        let flags = LinksetUpdateFlags::from_parts(root, children);
        assert_eq!(encode(root, &flags).crc, u32::MAX);

        root.base_mask = PERM_ALL_EFFECTIVE;
        root.owner_mask = PERM_ALL_EFFECTIVE;
        let flags = LinksetUpdateFlags::from_parts(root, children);
        assert_eq!(encode(root, &flags).crc, 0);
        children[0].pseudo_crc = Some(-1);
        assert_eq!(encode(&children[0], &flags).crc, u32::MAX);
    }

    #[tokio::test]
    async fn child_inventory_restricts_copy_except_for_landmarks() {
        let store = CopiedStore::new("regionstore/objects.db");
        let groups = open_objects(
            &store.connection(),
            Uuid::from_bytes([0x11; 16]),
            ObjectStoreKind::Sqlite,
        )
        .await
        .unwrap();
        let mut item = groups
            .into_iter()
            .flat_map(|g| g.parts)
            .flat_map(|p| p.items)
            .next()
            .unwrap();
        let mut group = load_capture_store().await.remove(0);
        let (root, children) = group.parts.split_first_mut().unwrap();
        item.inv_type = 6;
        item.current_permissions = 0;
        children[0].items = vec![item];
        for (inv_type, can_copy) in [(6, false), (3, true)] {
            children[0].items[0].inv_type = inv_type;
            let flags = LinksetUpdateFlags::from_parts(root, children);
            let prim = StoredPrimUpdate::from_store(root, &flags, 1, 1).unwrap();
            let bits = prim.block_for(root.owner_id).update_flags;
            assert_eq!(bits & OBJECT_COPY != 0, can_copy);
            assert_eq!(
                bits & (OBJECT_MODIFY | OBJECT_MOVE | OBJECT_TRANSFER),
                OBJECT_MODIFY | OBJECT_MOVE | OBJECT_TRANSFER
            );
        }
    }

    #[test]
    fn limited_text_ends_at_nul_or_a_complete_utf8_character() {
        assert!(short_text("Text", "\0hidden").is_empty());
        assert_eq!(short_text("Text", "hi\0hidden"), b"hi\0");
        for (prefix, suffix) in [(250, "😀"), (251, ""), (252, ""), (253, ""), (254, "")] {
            let text = format!("{}😀more", "a".repeat(prefix));
            assert_eq!(
                short_text("MediaURL", &text),
                format!("{}{suffix}\0", "a".repeat(prefix)).as_bytes()
            );
        }
    }

    #[tokio::test]
    async fn binary_fields_must_fit_the_wire_length() {
        let mut part = load_capture_store().await.remove(0).parts.remove(0);
        for (field, max) in [
            ("TextureEntry", 65535),
            ("TextureAnim", 255),
            ("PSBlock", 255),
        ] {
            let bytes = match field {
                "TextureEntry" => &mut part.shape.texture,
                "TextureAnim" => &mut part.texture_animation,
                _ => &mut part.particles,
            };
            bytes.resize(max + 1, 0);
            let error = single(&part).err().unwrap();
            assert_eq!(
                error,
                PrimUpdateError {
                    field,
                    length: max + 1,
                    maximum: max
                }
            );
            part.shape.texture.clear();
            part.texture_animation.clear();
            part.particles.clear();
        }
        // A glTF material entry carries one face byte and a UUID.
        part.shape.extra_params = vec![1, 0x80, 0, 0x11, 1, 0, 0, 16];
        part.shape.extra_params.resize(7 + 273, 0);
        let error = single(&part).err().unwrap();
        assert_eq!(
            error,
            PrimUpdateError {
                field: "ExtraParams",
                length: 280,
                maximum: 255
            }
        );
    }
}
