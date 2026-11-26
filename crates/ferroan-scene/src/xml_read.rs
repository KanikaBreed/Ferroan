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
// Ported to Rust for Ferroan from SceneObjectSerializer.FromXml2Format,
// FromOriginalXmlFormat, Xml2ToSOP, ReadTaskInventory and ReadShape, and
// ExternalRepresentationUtils.ExecuteReadProcessors, at OpenSim-Tranquillity
// `dev-build` @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
//
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Read XML2 and original-format object XML into an [`ObjectGroup`].
//!
//! An element whose name is not in the part, task-inventory or shape table is
//! skipped. Making the part the root clears its parent id and sets its link
//! number to 0, or to 1 when another part is added. XML2 keeps a child link of
//! 0 as the added position and restores any other child link. Original XML
//! restores every child link and trims that child's permission masks.
//!
//! A bad value in the shape or task-item table keeps that field's default and
//! the group. Ten such failures abort that read. A bad value on the part
//! itself drops the group. XML2 finds a `SceneObjectPart` anywhere in the
//! document. Original XML reads `KeyframeMotion` and `StartStr` only when they
//! are the next elements after `OtherParts`, in that order.

use std::collections::HashSet;

use ferroan_diag::{Kind, OBJECT_TASK_ITEMS, OBJECT_XML_BYTES, REGION_LINKSET_PRIMS, refuse};
use ferroan_llsd::from_xml_element;
use ferroan_llsd::xml::{Element, Node};
use uuid::Uuid;

use crate::objects::{
    DEFAULT_BOX_TEXTURE, InventoryItem, MAX_LINKSET_PRIMS, MAX_OBJECT_XML_BYTES, MAX_TASK_ITEMS,
    ObjectGroup, ObjectPart, ObjectRaw, ObjectShape,
};
use crate::payloads::{
    dyn_attrs_from_llsd, parse_animations, parse_keyframe, parse_linkset, parse_materials,
    parse_media, read_inertia, read_vehicle,
};

const PERM_ALL: u32 = 0x8_e000;
const PERM_EXPORT: u32 = 0x1_0000;
const ITEM_PERM: u32 = 0x7fff_ffff;
const FLAG_STRIP: u32 = 0x0000_0080 | 0x0000_0200 | 0x0001_0000;
const FLAG_VOLUME: u32 = 0x0000_4000;
const SHAPE_ERRORS: u32 = 10;
const SCULPT_MESH: u8 = 5;
const PHYS_CONVEX: u8 = 2;

const MATERIAL: [(f32, f32); 8] = [
    (0.8, 0.4),
    (0.3, 0.4),
    (0.2, 0.7),
    (0.6, 0.5),
    (0.9, 0.3),
    (0.4, 0.7),
    (0.9, 0.9),
    (0.6, 0.5),
];

/// XML2. No scene object parts is no group.
pub fn from_xml2(xml: &str) -> Option<ObjectGroup> {
    from_xml2_at(xml, unix_now())
}

/// Original XML. A repeated `xmlns:` prefix is collapsed before the read.
pub fn from_original_xml(xml: &str) -> Option<ObjectGroup> {
    from_original_xml_at(xml, unix_now())
}

/// XML2 with the clock a missing item `CreationDate` uses.
pub(crate) fn from_xml2_at(xml: &str, now: u32) -> Option<ObjectGroup> {
    bounded(xml.len())?;
    let root = ferroan_llsd::xml::parse(xml.as_bytes()).ok()?;
    read_xml2(&root, now)
}

/// Original XML with the clock a missing item `CreationDate` uses.
pub(crate) fn from_original_xml_at(xml: &str, now: u32) -> Option<ObjectGroup> {
    bounded(xml.len())?;
    let xml = sanitize_xml(xml);
    let root = ferroan_llsd::xml::parse(xml.as_bytes()).ok()?;
    read_original(&root, now)
}

pub(super) fn bounded(len: usize) -> Option<()> {
    let bytes = u64::try_from(len).unwrap_or(u64::MAX);
    if bytes > MAX_OBJECT_XML_BYTES {
        refuse(
            OBJECT_XML_BYTES,
            Kind::Hard,
            MAX_OBJECT_XML_BYTES,
            bytes,
            "object-xml",
        );
        return None;
    }
    Some(())
}

pub(super) fn unix_now() -> u32 {
    u32::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0),
    )
    .unwrap_or(u32::MAX)
}

fn read_xml2(root: &Element, now: u32) -> Option<ObjectGroup> {
    let mut part_els = Vec::new();
    gather(root, "SceneObjectPart", &mut part_els);
    if part_els.is_empty() {
        return None;
    }
    let mut parts = Vec::with_capacity(part_els.len());
    for el in part_els {
        parts.push(read_part(el, now)?);
    }
    accept_parts(&mut parts, false)?;
    let mut motions = Vec::new();
    gather(root, "KeyframeMotion", &mut motions);
    if let Some(child) = motions.first() {
        attach_motion(&mut parts, child)?;
    }
    let mut starts = Vec::new();
    gather(root, "StartStr", &mut starts);
    Some(ObjectGroup {
        from_item_id: Uuid::nil(),
        rez_parameter: starts.first().map(|child| child.text()),
        parts,
    })
}

pub(super) fn read_original(root: &Element, now: u32) -> Option<ObjectGroup> {
    if root.name != "SceneObjectGroup" {
        return None;
    }
    let children: Vec<&Element> = root.elements().collect();
    let root_part = children.iter().find(|child| child.name == "RootPart")?;
    let mut parts = vec![read_part(part_element(root_part)?, now)?];
    let other_at = children.iter().position(|child| child.name == "OtherParts");
    if let Some(index) = other_at {
        for wrap in children[index].elements() {
            if wrap.name != "Part" {
                continue;
            }
            if let Some(part) = part_element(wrap) {
                parts.push(read_part(part, now)?);
            }
        }
    }
    accept_parts(&mut parts, true)?;
    let mut rez = None;
    if let Some(index) = other_at {
        let mut cursor = index + 1;
        if children
            .get(cursor)
            .is_some_and(|child| child.name == "KeyframeMotion")
        {
            attach_motion(&mut parts, children[cursor])?;
            cursor += 1;
        }
        if children
            .get(cursor)
            .is_some_and(|child| child.name == "StartStr")
        {
            rez = Some(children[cursor].text());
        }
    }
    Some(ObjectGroup {
        from_item_id: Uuid::nil(),
        rez_parameter: rez,
        parts,
    })
}

fn gather<'a>(el: &'a Element, name: &str, out: &mut Vec<&'a Element>) {
    if el.name == name {
        out.push(el);
    }
    for child in el.elements() {
        gather(child, name, out);
    }
}

fn attach_motion(parts: &mut [ObjectPart], child: &Element) -> Option<()> {
    let bytes = decode_b64(&child.text())?;
    if let Some(mut motion) = parse_keyframe(&bytes) {
        // Writing the motion stores the group's position in place of the
        // position that was in the blob. The other positions stay as read.
        motion.serialized_position = parts[0].group_position;
        parts[0].keyframe = Some(motion);
    }
    Some(())
}

fn accept_parts(parts: &mut [ObjectPart], original: bool) -> Option<()> {
    let count = u64::try_from(parts.len()).unwrap_or(u64::MAX);
    if count > MAX_LINKSET_PRIMS {
        refuse(
            REGION_LINKSET_PRIMS,
            Kind::Hard,
            MAX_LINKSET_PRIMS,
            count,
            "object-xml",
        );
        return None;
    }
    let mut seen = HashSet::new();
    for part in parts.iter() {
        if !seen.insert(part.uuid) {
            return None;
        }
    }
    finish_parts(parts, original);
    Some(())
}

fn part_element(el: &Element) -> Option<&Element> {
    el.elements().find(|child| child.name == "SceneObjectPart")
}

fn finish_parts(parts: &mut [ObjectPart], original: bool) {
    let Some((root, children)) = parts.split_first_mut() else {
        return;
    };
    root.link_num = if children.is_empty() { 0 } else { 1 };
    root.parent_id = 0;
    for (offset, part) in children.iter_mut().enumerate() {
        if !original && part.link_num == 0 {
            part.link_num = i32::try_from(offset + 2).unwrap_or(i32::MAX);
        }
        if original {
            part.base_mask &= PERM_ALL | PERM_EXPORT;
            part.owner_mask &= PERM_ALL | PERM_EXPORT;
            part.group_mask &= PERM_ALL;
            part.everyone_mask &= PERM_ALL | PERM_EXPORT;
            part.next_owner_mask &= PERM_ALL;
        }
    }
}

fn read_part(el: &Element, now: u32) -> Option<ObjectPart> {
    let mut part = blank_part();
    let mut creator = Uuid::nil();
    let mut creator_data = None;
    let mut sit_explicit = None;
    let mut serial = 0u32;
    for child in el.elements() {
        apply_part(
            &mut part,
            child,
            &mut creator,
            &mut creator_data,
            &mut sit_explicit,
            &mut serial,
            now,
        )?;
    }
    part.creator_id = combine_creator(creator, creator_data.as_deref());
    part.xml_creator_data = creator_data.filter(|text| !text.is_empty());
    let derived = super::sit_derived(part.sit_target_position, part.sit_target_rotation);
    part.sit_target_active = sit_explicit.unwrap_or(derived);
    part.inventory_serial = Some(serial);
    Some(part)
}

fn apply_part(
    part: &mut ObjectPart,
    child: &Element,
    creator: &mut Uuid,
    creator_data: &mut Option<String>,
    sit_explicit: &mut Option<bool>,
    serial: &mut u32,
    now: u32,
) -> Option<()> {
    match child.name.as_str() {
        "AllowedDrop" => part.allowed_drop = read_bool(child),
        "CreatorID" => *creator = uuid_of(child),
        "CreatorData" => {
            let text = child.text();
            if !text.is_empty() {
                *creator_data = Some(text);
            }
        }
        "InventorySerial" => *serial = need_i32(child)? as u32,
        "TaskInventory" => {
            part.items = read_inventory(child, now)?;
            *serial = serial.wrapping_add(1);
        }
        "UUID" => {
            part.uuid = uuid_of(child);
            if !part.items.is_empty() {
                let id = part.uuid;
                for item in &mut part.items {
                    item.parent_id = id;
                    item.parent_part_id = id;
                }
                *serial = serial.wrapping_add(1);
            }
        }
        "LocalId" => part.local_id = need_i64(child)? as u32,
        "Name" => part.name = child.text(),
        "Material" => set_material(part, need_i32(child)?),
        "PassTouches" => part.pass_touches = read_bool(child),
        "PassCollisions" => part.pass_collisions = read_bool(child),
        "ScriptAccessPin" => part.script_access_pin = need_i32(child)?,
        "GroupPosition" => part.group_position = vec3_of(child)?,
        "OffsetPosition" => part.offset = vec3_of(child)?,
        "RotationOffset" => part.rotation = normalize_quat(quat_of(child)?),
        "Velocity" => part.velocity = vec3_of(child)?,
        "AngularVelocity" => part.angular_velocity = vec3_of(child)?,
        "Acceleration" => part.acceleration = vec3_of(child)?,
        "Description" => part.description = child.text(),
        "Color" => part.color = color_of(child)?,
        "Text" => part.text = child.text(),
        "SitName" => part.sit_name = child.text(),
        "TouchName" => part.touch_name = child.text(),
        "LinkNum" => part.link_num = need_i32(child)?,
        "ClickAction" => part.click_action = need_i32(child)? as u8,
        "Shape" => part.shape = read_shape(child)?,
        "Scale" => part.shape.scale = vec3_of(child)?,
        "SitTargetOrientation" => {
            part.sit_target_rotation = quat_of(child)?;
            *sit_explicit = None;
        }
        "SitTargetPosition" => {
            part.sit_target_position = vec3_of(child)?;
            *sit_explicit = None;
        }
        "SitTargetPositionLL" => part.sit_target_position = vec3_of(child)?,
        "SitTargetOrientationLL" => part.sit_target_rotation = quat_of(child)?,
        "StandTarget" => part.stand_offset = vec3_of(child)?,
        "ParentID" => part.parent_id = need_u32(child)?,
        "CreationDate" => part.creation_date = need_i32(child)?,
        "SalePrice" => part.sale_price = need_i32(child)?,
        "ObjectSaleType" => part.sale_type = need_i32(child)? as u8,
        "GroupID" => part.group_id = uuid_of(child),
        "OwnerID" => part.owner_id = uuid_of(child),
        "LastOwnerID" => part.last_owner_id = uuid_of(child),
        "RezzerID" => part.rezzer_id = uuid_of(child),
        "BaseMask" => part.base_mask = need_i64(child)? as u32,
        "OwnerMask" => part.owner_mask = need_i64(child)? as u32,
        "GroupMask" => part.group_mask = need_i64(child)? as u32,
        "EveryoneMask" => part.everyone_mask = need_i64(child)? as u32,
        "NextOwnerMask" => part.next_owner_mask = need_i64(child)? as u32,
        "Flags" => {
            let flags = parse_flags(&child.text())?;
            part.flags = flags;
            part.volume_detect = flags & FLAG_VOLUME != 0;
        }
        "CollisionSound" => part.collision_sound = uuid_of(child),
        "CollisionSoundVolume" => part.collision_sound_volume = need_f32(child)?,
        "MediaUrl" => part.media_url = child.text(),
        "AttachedPos" => part.attached_pos = vec3_of(child)?,
        "DynAttrs" => part.dyn_attrs = read_dyn(child)?,
        "TextureAnimation" => part.texture_animation = decode_b64(&child.text())?,
        "ParticleSystem" => part.particles = decode_b64(&child.text())?,
        "PayPrice0" => part.pay_price[0] = need_i32(child)?,
        "PayPrice1" => part.pay_price[1] = need_i32(child)?,
        "PayPrice2" => part.pay_price[2] = need_i32(child)?,
        "PayPrice3" => part.pay_price[3] = need_i32(child)?,
        "PayPrice4" => part.pay_price[4] = need_i32(child)?,
        "VolumeDetectActive" => {
            let on = read_bool(child);
            if on {
                part.flags |= FLAG_VOLUME;
            } else {
                part.flags &= !FLAG_VOLUME;
            }
            part.volume_detect = on;
        }
        "Vehicle" => part.vehicle = read_vehicle(child).ok().flatten(),
        "PhysicsInertia" => part.phys_inertia = read_inertia(child).ok().flatten(),
        "RotationAxisLocks" => part.rotation_axis_locks = need_i32(child)? as u8,
        "PhysicsShapeType" => set_physics_shape(part, need_i32(child)?),
        "Density" => set_ranged(part, need_f32(child)?, Range::Density),
        "Friction" => set_ranged(part, need_f32(child)?, Range::Friction),
        "Bounce" => set_ranged(part, need_f32(child)?, Range::Bounce),
        "GravityModifier" => set_ranged(part, need_f32(child)?, Range::Gravity),
        "CameraEyeOffset" => part.camera_eye = vec3_of(child)?,
        "CameraAtOffset" => part.camera_at = vec3_of(child)?,
        "SoundID" => part.sound = uuid_of(child),
        "SoundGain" => part.sound_gain = need_f64(child)?,
        "SoundFlags" => part.sound_flags = need_i32(child)? as u8,
        "SOPAnims" => {
            let text = child.text();
            // A blob shorter than a count is an empty animation list and is
            // written back as a single zero byte. A zero count, a blank
            // element, or a blob that does not parse leaves the list unset.
            part.animations = if text.is_empty() {
                None
            } else {
                decode_b64(&text).and_then(|bytes| parse_animations(&bytes))
            };
        }
        "SitActRange" => part.sit_active_range = need_f32(child)?,
        "LinksetData" => part.linkset = parse_linkset(&child.text()),
        "SitTargetActive" => *sit_explicit = Some(read_bool(child)),
        // Known elements the object model does not keep. The writer emits the
        // fixed defaults. Crossing targets stay for Q77 and Q78.
        "FolderID" | "RegionHandle" | "Category" | "OwnershipCost" | "Buoyancy" | "Force"
        | "Torque" | "SoundRadius" | "SoundQueueing" | "AllowUnsit" | "ScriptedSitOnly"
        | "CTMoveTarget" | "CTMoveTau" | "CTHoverHeight" | "CTHoverType" | "CTHoverTau"
        | "CTHoverRepel" | "CTLookTarget" | "CTLookStrength" | "CTLookDamping" | "CTDragAgent"
        | "CTDragAnchor" | "CTDragTarget" => {}
        _ => {}
    }
    Some(())
}

fn read_inventory(el: &Element, now: u32) -> Option<Vec<InventoryItem>> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for child in el.elements() {
        if child.name != "TaskInventoryItem" {
            continue;
        }
        let item = read_item(child, now)?;
        if !seen.insert(item.item_id) {
            return None;
        }
        items.push(item);
    }
    let count = u64::try_from(items.len()).unwrap_or(u64::MAX);
    if count > MAX_TASK_ITEMS {
        refuse(
            OBJECT_TASK_ITEMS,
            Kind::Hard,
            MAX_TASK_ITEMS,
            count,
            "object-xml",
        );
        return None;
    }
    Some(items)
}

fn read_item(el: &Element, now: u32) -> Option<InventoryItem> {
    let mut item = blank_item(now);
    let mut creator = Uuid::nil();
    let mut creator_data = None;
    let mut errors = 0u32;
    for child in el.elements() {
        if apply_item(&mut item, child, &mut creator, &mut creator_data).is_none() {
            errors += 1;
            if errors == SHAPE_ERRORS {
                return None;
            }
        }
    }
    item.creator_id = combine_item_creator(creator, creator_data.as_deref());
    item.xml_creator_data = creator_data.filter(|text| !text.is_empty());
    Some(item)
}

fn apply_item(
    item: &mut InventoryItem,
    child: &Element,
    creator: &mut Uuid,
    creator_data: &mut Option<String>,
) -> Option<()> {
    match child.name.as_str() {
        "AssetID" => item.asset_id = uuid_of(child),
        "BasePermissions" => item.base_permissions = need_i64(child)? as u32,
        "CreationDate" => item.creation_date = need_i64(child)? as u32,
        "CreatorID" => *creator = uuid_of(child),
        "CreatorData" => {
            let text = child.text();
            if !text.is_empty() {
                *creator_data = Some(text);
            }
        }
        "Description" => item.description = child.text(),
        "EveryonePermissions" => item.everyone_permissions = need_i64(child)? as u32,
        "Flags" => item.flags = need_i64(child)? as u32,
        "GroupID" => item.group_id = uuid_of(child),
        "GroupPermissions" => item.group_permissions = need_i64(child)? as u32,
        "InvType" => item.inv_type = need_i32(child)?,
        "ItemID" => item.item_id = uuid_of(child),
        "LastOwnerID" => item.last_owner_id = uuid_of(child),
        "Name" => item.name = child.text(),
        "NextPermissions" => item.next_permissions = need_i64(child)? as u32,
        "OwnerID" => item.owner_id = uuid_of(child),
        "CurrentPermissions" => item.current_permissions = need_i64(child)? as u32,
        "ParentID" => item.parent_id = uuid_of(child),
        "ParentPartID" => item.parent_part_id = uuid_of(child),
        "Type" => item.asset_type = need_i32(child)?,
        "ExperienceID" => item.experience_id = uuid_of(child),
        "OldItemID" | "PermsGranter" | "PermsMask" | "OwnerChanged" => {}
        _ => {}
    }
    Some(())
}

fn read_shape(el: &Element) -> Option<ObjectShape> {
    let mut shape = blank_shape();
    // The sculpt, flexi and light elements edit this value. Encoding after
    // every one would drop a sculpt type that is stored while sculpt is off.
    let mut extras = super::decode_extras(&shape.extra_params);
    let mut errors = 0u32;
    for child in el.elements() {
        if apply_shape(&mut shape, &mut extras, child).is_none() {
            errors += 1;
            if errors == SHAPE_ERRORS {
                return None;
            }
        }
    }
    shape.extra_params = super::encode_extras(&extras);
    Some(shape)
}

fn apply_shape(shape: &mut ObjectShape, extras: &mut super::Extras, child: &Element) -> Option<()> {
    match child.name.as_str() {
        "ProfileCurve" => set_profile_byte(shape, need_i32(child)? as u8),
        // Shorter than one face id: the entry is dropped and the written
        // element is empty. A longer entry is kept as the bytes given.
        // A bad encoding keeps the plywood default.
        "TextureEntry" => {
            let bytes = decode_b64(&child.text())?;
            shape.texture = if bytes.len() < 16 { Vec::new() } else { bytes };
        }
        "ExtraParams" => {
            let bytes = decode_b64(&child.text())?;
            *extras = super::decode_extras(&bytes);
        }
        "PathBegin" => shape.path_begin = need_i32(child)? as u16,
        "PathCurve" => shape.path_curve = need_i32(child)? as u8,
        "PathEnd" => shape.path_end = need_i32(child)? as u16,
        "PathRadiusOffset" => shape.path_radius_offset = need_i32(child)? as i8,
        "PathRevolutions" => shape.path_revolutions = need_i32(child)? as u8,
        "PathScaleX" => shape.path_scale_x = need_i32(child)? as u8,
        "PathScaleY" => shape.path_scale_y = need_i32(child)? as u8,
        "PathShearX" => shape.path_shear_x = need_i32(child)? as u8,
        "PathShearY" => shape.path_shear_y = need_i32(child)? as u8,
        "PathSkew" => shape.path_skew = need_i32(child)? as i8,
        "PathTaperX" => shape.path_taper_x = need_i32(child)? as i8,
        "PathTaperY" => shape.path_taper_y = need_i32(child)? as i8,
        "PathTwist" => shape.path_twist = need_i32(child)? as i8,
        "PathTwistBegin" => shape.path_twist_begin = need_i32(child)? as i8,
        "PCode" => shape.pcode = need_i32(child)? as u8,
        "ProfileBegin" => shape.profile_begin = need_i32(child)? as u16,
        "ProfileEnd" => shape.profile_end = need_i32(child)? as u16,
        "ProfileHollow" => shape.profile_hollow = need_i32(child)? as u16,
        "Scale" => shape.scale = vec3_of(child)?,
        "State" => shape.state = need_i32(child)? as u8,
        "LastAttachPoint" => shape.last_attach_point = need_i32(child)? as u8,
        "ProfileShape" => {
            let profile = profile_nibble(&child.text())?;
            shape.profile_curve = (shape.profile_curve & 0xf0) | profile;
        }
        "HollowShape" => {
            let hollow = hollow_nibble(&child.text())?;
            shape.profile_curve = (shape.profile_curve & 0x0f) | hollow;
        }
        "Media" => shape.media_entries = parse_media(&child.text()),
        "MatOvrd" => {
            shape.materials = decode_b64(&child.text()).and_then(|bytes| parse_materials(&bytes));
        }
        // SculptData is not in the shape table. The other sculpt, flexi and
        // light elements update the fields the extra-params blob is rebuilt from.
        "SculptData" => {}
        "SculptTexture" => extras.sculpt_texture = uuid_of(child),
        "SculptType" => extras.sculpt_type = need_i32(child)? as u8,
        "SculptEntry" => extras.sculpt = read_bool(child),
        "FlexiEntry" => extras.flexi = read_bool(child),
        "LightEntry" => extras.light_on = read_bool(child),
        "FlexiSoftness" => extras.softness = need_i32(child)?,
        "FlexiTension" => extras.tension = need_f32(child)?,
        "FlexiDrag" => extras.drag = need_f32(child)?,
        "FlexiGravity" => extras.gravity = need_f32(child)?,
        "FlexiWind" => extras.wind = need_f32(child)?,
        "FlexiForceX" => extras.force[0] = need_f32(child)?,
        "FlexiForceY" => extras.force[1] = need_f32(child)?,
        "FlexiForceZ" => extras.force[2] = need_f32(child)?,
        "LightColorR" => extras.light[0] = need_f32(child)?.clamp(0.0, 1.0),
        "LightColorG" => extras.light[1] = need_f32(child)?.clamp(0.0, 1.0),
        "LightColorB" => extras.light[2] = need_f32(child)?.clamp(0.0, 1.0),
        "LightColorA" => extras.light_a = need_f32(child)?.clamp(0.0, 1.0),
        "LightRadius" => extras.radius = need_f32(child)?,
        "LightCutoff" => extras.cutoff = need_f32(child)?,
        "LightFalloff" => extras.falloff = need_f32(child)?,
        "LightIntensity" => extras.intensity = need_f32(child)?,
        _ => {}
    }
    Some(())
}

fn read_dyn(el: &Element) -> Option<Option<crate::payloads::DynAttrs>> {
    let inner = el.elements().next()?;
    let value = from_xml_element(inner).ok()?;
    let attrs = dyn_attrs_from_llsd(value).ok()?;
    if attrs.namespaces.is_empty() {
        Some(None)
    } else {
        Some(Some(attrs))
    }
}

fn set_material(part: &mut ObjectPart, value: i32) {
    let value = value as u8;
    if value > 7 {
        return;
    }
    part.material = value;
    let (friction, bounce) = MATERIAL[usize::from(value)];
    part.friction = friction;
    part.restitution = bounce;
}

enum Range {
    Density,
    Friction,
    Bounce,
    Gravity,
}

fn set_ranged(part: &mut ObjectPart, value: f32, range: Range) {
    match range {
        Range::Density if (1.0..=22_587.0).contains(&value) => part.density = value,
        Range::Friction if (0.0..=255.0).contains(&value) => part.friction = value,
        Range::Bounce if (0.0..=1.0).contains(&value) => part.restitution = value,
        Range::Gravity if (-1.0..=28.0).contains(&value) => part.gravity_modifier = value,
        Range::Density | Range::Friction | Range::Bounce | Range::Gravity => {}
    }
}

fn set_physics_shape(part: &mut ObjectPart, value: i32) {
    let value = value as u8;
    if value <= PHYS_CONVEX {
        // The part has no group while it is read, so `none` stays `none`.
        part.physics_shape_type = value;
    } else if shape_is_mesh(&part.shape) {
        part.physics_shape_type = PHYS_CONVEX;
    } else {
        part.physics_shape_type = 0;
    }
}

fn shape_is_mesh(shape: &ObjectShape) -> bool {
    let extras = super::decode_extras(&shape.extra_params);
    extras.sculpt && extras.sculpt_type == SCULPT_MESH
}

fn set_profile_byte(shape: &mut ObjectShape, value: u8) {
    let hollow = match value & 0xf0 {
        hollow @ (0 | 16 | 32 | 48) => hollow,
        _ => 0,
    };
    let profile = value & 0x0f;
    let profile = if profile <= 5 { profile } else { 1 };
    shape.profile_curve = hollow | profile;
}

fn profile_nibble(name: &str) -> Option<u8> {
    Some(match name.trim() {
        "Circle" => 0,
        "Square" => 1,
        "IsometricTriangle" => 2,
        "EquilateralTriangle" => 3,
        "RightTriangle" => 4,
        "HalfCircle" => 5,
        _ => return None,
    })
}

fn hollow_nibble(name: &str) -> Option<u8> {
    Some(match name.trim() {
        "Same" => 0,
        "Circle" => 16,
        "Square" => 32,
        "Triangle" => 48,
        _ => return None,
    })
}

fn parse_flags(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || text == "None" {
        return Some(0);
    }
    if let Ok(number) = text.parse::<u32>() {
        return Some(number & !FLAG_STRIP);
    }
    let mut value = 0u32;
    for token in text.split(|c: char| c == ',' || c.is_whitespace()) {
        if token.is_empty() {
            continue;
        }
        let bit = super::PRIM_FLAG_NAMES
            .iter()
            .find(|(_, name)| *name == token)?
            .0;
        value |= bit;
    }
    Some(value & !FLAG_STRIP)
}

fn combine_creator(id: Uuid, data: Option<&str>) -> String {
    let Some(data) = data.map(str::trim).filter(|text| !text.is_empty()) else {
        return id.to_string();
    };
    let (home, name) = data.split_once(';').unwrap_or((data, "Unknown User"));
    let home = home.strip_suffix('/').unwrap_or(home);
    format!("{id};{home};{name}")
}

fn combine_item_creator(id: Uuid, data: Option<&str>) -> String {
    match data.map(str::trim).filter(|text| !text.is_empty()) {
        Some(data) => format!("{id};{data}"),
        None => id.to_string(),
    }
}

fn read_bool(el: &Element) -> bool {
    let value = el.text().trim().to_ascii_lowercase();
    value == "true" || value == "1"
}

fn uuid_of(el: &Element) -> Uuid {
    let text = if let Some(inner) = el.child("UUID").or_else(|| el.child("Guid")) {
        inner.text()
    } else {
        el.text()
    };
    let text = text.trim();
    // A leading brace is not a uuid here. The pin's parser returns the zero id.
    if text.starts_with('{') || text.ends_with('}') {
        return Uuid::nil();
    }
    Uuid::parse_str(text).unwrap_or(Uuid::nil())
}

fn vec3_of(el: &Element) -> Option<[f32; 3]> {
    let mut values = [0.0; 3];
    let mut count = 0usize;
    for node in &el.children {
        match node {
            Node::Text(text) if text.trim().is_empty() => {}
            Node::Text(_) => return None,
            Node::Element(child) => {
                if count == 3 {
                    return None;
                }
                values[count] = need_f32(child)?;
                count += 1;
            }
        }
    }
    if count != 3 {
        return None;
    }
    Some(values)
}

fn quat_of(el: &Element) -> Option<[f32; 4]> {
    let mut value = [0.0; 4];
    for child in el.elements() {
        match child.name.to_ascii_lowercase().as_str() {
            "x" => value[0] = need_f32(child)?,
            "y" => value[1] = need_f32(child)?,
            "z" => value[2] = need_f32(child)?,
            "w" => value[3] = need_f32(child)?,
            // An unknown child is skipped. The pin's reader never advances on
            // one, so this is the fixed behaviour (TB73).
            _ => {}
        }
    }
    Some(value)
}

fn normalize_quat(value: [f32; 4]) -> [f32; 4] {
    let mag = value[0] * value[0] + value[1] * value[1] + value[2] * value[2] + value[3] * value[3];
    if mag > 1e-6 {
        let scale = mag.sqrt().recip();
        [
            value[0] * scale,
            value[1] * scale,
            value[2] * scale,
            value[3] * scale,
        ]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

fn color_of(el: &Element) -> Option<u32> {
    let channel = |name: &str| -> Option<u32> {
        let Some(child) = el.child(name) else {
            return Some(0);
        };
        let value = child.text().trim().parse::<f32>().ok()?;
        Some(value as i32 as u32)
    };
    let red = channel("R")?;
    let green = channel("G")?;
    let blue = channel("B")?;
    let alpha = channel("A")?;
    Some((alpha << 24) | (red << 16) | (green << 8) | blue)
}

fn need_f32(el: &Element) -> Option<f32> {
    el.text().trim().parse().ok()
}

fn need_f64(el: &Element) -> Option<f64> {
    el.text().trim().parse().ok()
}

fn need_i32(el: &Element) -> Option<i32> {
    el.text().trim().parse().ok()
}

fn need_i64(el: &Element) -> Option<i64> {
    el.text().trim().parse().ok()
}

fn need_u32(el: &Element) -> Option<u32> {
    el.text().trim().parse().ok()
}

fn decode_b64(text: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut index = 0usize;
    while index < bytes.len() {
        let first = value(bytes[index])?;
        let second = value(bytes[index + 1])?;
        let third_pad = bytes[index + 2] == b'=';
        let fourth_pad = bytes[index + 3] == b'=';
        if third_pad && !fourth_pad {
            return None;
        }
        let third = if third_pad {
            0
        } else {
            value(bytes[index + 2])?
        };
        let fourth = if fourth_pad {
            0
        } else {
            value(bytes[index + 3])?
        };
        out.push((first << 2) | (second >> 4));
        if !third_pad {
            out.push((second << 4) | (third >> 2));
        }
        if !fourth_pad {
            out.push((third << 6) | fourth);
        }
        index += 4;
    }
    Some(out)
}

pub(super) fn sanitize_xml(xml: &str) -> String {
    let Some(start) = xml.find("xmlns:xmlns:") else {
        return xml.to_owned();
    };
    if start == 0 {
        return xml.to_owned();
    }
    let bytes = xml.as_bytes();
    let mut end = start + 12;
    while end + 5 < bytes.len() && bytes[end + 5] == b':' {
        end += 6;
    }
    let bad = &xml[start..end];
    xml.replace(bad, "xmlns:")
}

fn combine_blank_raw() -> ObjectRaw {
    ObjectRaw {
        media: None,
        mat_ovrd: None,
        dyn_attrs: None,
        keyframe_motion: None,
        vehicle: None,
        phys_inertia: None,
        sopanims: None,
        linkset_data: None,
    }
}

fn blank_shape() -> ObjectShape {
    ObjectShape {
        default_box: false,
        scale: [0.0; 3],
        pcode: 9,
        path_begin: 0,
        path_end: 0,
        path_scale_x: 0,
        path_scale_y: 0,
        path_shear_x: 0,
        path_shear_y: 0,
        path_skew: 0,
        path_curve: 0,
        path_radius_offset: 0,
        path_revolutions: 0,
        path_taper_x: 0,
        path_taper_y: 0,
        path_twist: 0,
        path_twist_begin: 0,
        profile_begin: 0,
        profile_end: 0,
        profile_curve: 0,
        profile_hollow: 0,
        state: 0,
        last_attach_point: 0,
        texture: DEFAULT_BOX_TEXTURE.to_vec(),
        extra_params: vec![0],
        media_entries: None,
        materials: None,
    }
}

fn blank_item(now: u32) -> InventoryItem {
    InventoryItem {
        item_id: Uuid::nil(),
        parent_part_id: Uuid::nil(),
        asset_id: Uuid::nil(),
        parent_id: Uuid::nil(),
        inv_type: 0,
        asset_type: 0,
        name: String::new(),
        description: String::new(),
        creation_date: now,
        creator_id: String::new(),
        xml_creator_data: None,
        owner_id: Uuid::nil(),
        last_owner_id: Uuid::nil(),
        group_id: Uuid::nil(),
        next_permissions: ITEM_PERM,
        current_permissions: ITEM_PERM,
        base_permissions: ITEM_PERM,
        everyone_permissions: ITEM_PERM,
        group_permissions: ITEM_PERM,
        flags: 0,
        experience_id: Uuid::nil(),
    }
}

pub(crate) fn blank_part() -> ObjectPart {
    ObjectPart {
        uuid: Uuid::nil(),
        link_num: 0,
        local_id: 0,
        parent_id: 0,
        creation_date: 0,
        name: String::new(),
        text: String::new(),
        color: 0,
        description: String::new(),
        sit_name: String::new(),
        touch_name: String::new(),
        flags: 0,
        creator_id: String::new(),
        xml_creator_data: None,
        owner_id: Uuid::nil(),
        group_id: Uuid::nil(),
        last_owner_id: Uuid::nil(),
        rezzer_id: Uuid::nil(),
        owner_mask: 0,
        next_owner_mask: 0,
        group_mask: 0,
        everyone_mask: 0,
        base_mask: 0,
        offset: [0.0; 3],
        group_position: [0.0; 3],
        velocity: [0.0; 3],
        angular_velocity: [0.0; 3],
        acceleration: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        sit_target_position: [0.0; 3],
        sit_target_rotation: [0.0; 4],
        sit_target_active: false,
        stand_offset: [0.0; 3],
        sit_active_range: 0.0,
        click_action: 0,
        pay_price: [-2; 5],
        sound: Uuid::nil(),
        sound_gain: 0.0,
        sound_flags: 0,
        texture_animation: Vec::new(),
        particles: Vec::new(),
        camera_eye: [0.0; 3],
        camera_at: [0.0; 3],
        force_mouselook: false,
        script_access_pin: 0,
        allowed_drop: false,
        die_at_edge: false,
        sale_price: 0,
        sale_type: 0,
        material: 3,
        collision_sound: Uuid::nil(),
        collision_sound_volume: 0.0,
        volume_detect: false,
        media_url: String::new(),
        attached_pos: [0.0; 3],
        physics_shape_type: 0,
        density: 1000.0,
        gravity_modifier: 1.0,
        friction: 0.6,
        restitution: 0.5,
        pass_collisions: false,
        pass_touches: false,
        rotation_axis_locks: 0,
        pseudo_crc: None,
        shape: blank_shape(),
        raw: combine_blank_raw(),
        dyn_attrs: None,
        keyframe: None,
        vehicle: None,
        phys_inertia: None,
        animations: None,
        linkset: None,
        items: Vec::new(),
        inventory_serial: None,
        allow_unsit: true,
        scripted_sit_only: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{from_original_xml, from_original_xml_at, from_xml2, from_xml2_at};
    use crate::objects::{MAX_LINKSET_PRIMS, MAX_OBJECT_XML_BYTES, MAX_TASK_ITEMS};
    use crate::xml::{to_original_xml, to_xml2};

    const PIN: &str = "c401bceecd1684832ae8ba86a31b5ee01a4ded7b";

    fn fixture(name: &str) -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/regionstore")
            .join(name);
        let text = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn insert_unknown(xml: &str) -> String {
        let at = xml.find("<SceneObjectPart").unwrap();
        let end = xml[at..].find('>').unwrap() + at;
        let mut with_part = String::new();
        with_part.push_str(&xml[..=end]);
        with_part.push_str("<NotAField>skip</NotAField>");
        with_part.push_str(&xml[end + 1..]);
        let shape = with_part.find("<Shape>").unwrap();
        let mut out = String::new();
        out.push_str(&with_part[..shape + "<Shape>".len()]);
        out.push_str("<SculptData>nope</SculptData>");
        out.push_str(&with_part[shape + "<Shape>".len()..]);
        out
    }

    fn assert_same(label: &str, got: &str, expected: &str) {
        if got == expected {
            return;
        }
        let at = got
            .bytes()
            .zip(expected.bytes())
            .position(|(left, right)| left != right)
            .unwrap_or(got.len().min(expected.len()));
        let window = 180;
        panic!(
            "{label} differs at byte {at} (got {}, expected {})\n got {:?}\n exp {:?}",
            got.len(),
            expected.len(),
            got.get(at..got.len().min(at + window)).unwrap_or(""),
            expected
                .get(at..expected.len().min(at + window))
                .unwrap_or("")
        );
    }

    #[test]
    fn regionstore_xml_reader_matches_the_serializer_oracle() {
        let written = fixture("objects-xml.json");
        let read = fixture("objects-xml-read.json");
        assert_eq!(read["pin"], PIN);
        assert_eq!(written["region"], read["region"]);
        let groups = written["groups"].as_array().unwrap();
        let expected = read["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 4);
        assert_eq!(expected.len(), 4);
        for (input, row) in groups.iter().zip(expected) {
            let id = input["id"].as_str().unwrap();
            assert_eq!(row["id"].as_str().unwrap(), id);
            let xml2 = input["xml2"].as_str().unwrap();
            let original = input["original"].as_str().unwrap();
            let got2 = to_xml2(&from_xml2(xml2).unwrap());
            let got_original = to_original_xml(&from_original_xml(original).unwrap());
            assert_same(&format!("{id} xml2"), &got2, row["xml2"].as_str().unwrap());
            assert_same(
                &format!("{id} original"),
                &got_original,
                row["original"].as_str().unwrap(),
            );
        }
        let unknown = &read["unknown"];
        assert_eq!(unknown["element"], "NotAField");
        assert_eq!(unknown["shape_element"], "SculptData");
        let dirty2 = insert_unknown(groups[0]["xml2"].as_str().unwrap());
        let dirty_original = insert_unknown(groups[0]["original"].as_str().unwrap());
        assert!(dirty2.contains("<NotAField>"));
        assert!(dirty2.contains("<SculptData>"));
        let got2 = to_xml2(&from_xml2(&dirty2).unwrap());
        let got_original = to_original_xml(&from_original_xml(&dirty_original).unwrap());
        assert_same("unknown xml2", &got2, unknown["xml2"].as_str().unwrap());
        assert_same(
            "unknown original",
            &got_original,
            unknown["original"].as_str().unwrap(),
        );
        assert!(!got2.contains("NotAField"));
        assert!(!got2.contains("SculptData"));
        assert_eq!(got2, expected[0]["xml2"].as_str().unwrap());
        assert_eq!(got_original, expected[0]["original"].as_str().unwrap());
    }

    /// The review probes. `refuse` drops the group even when the pin returned
    /// a damaged one. `mask`, `media`, `skip` and `clock` are the reads that
    /// deliberately do not copy the pin.
    #[test]
    fn regionstore_xml_reader_matches_the_probe_oracle() {
        let probes = fixture("objects-xml-probes.json");
        assert_eq!(probes["pin"], PIN);
        let clean = fixture("objects-xml-read.json");
        let clean_xml2 = clean["groups"][0]["xml2"].as_str().unwrap();
        let cases = probes["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 41);
        let mut failures = Vec::new();
        for case in cases {
            let name = case["name"].as_str().unwrap();
            check_probe(name, "xml2", case, clean_xml2, &mut failures);
            check_probe(name, "original", case, clean_xml2, &mut failures);
        }
        assert!(
            failures.is_empty(),
            "{} probe mismatch(es):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    fn check_probe(
        name: &str,
        form: &str,
        case: &serde_json::Value,
        clean_xml2: &str,
        failures: &mut Vec<String>,
    ) {
        let expect_key = format!("{form}_expect");
        let input_key = format!("{form}_in");
        let expect = case[&expect_key].as_str().unwrap();
        let input = case[&input_key].as_str().unwrap();
        let label = format!("{name} {form}");
        match (form, expect) {
            (_, "match") => {
                let Some(group) = read_form(form, input, None) else {
                    failures.push(format!("{label} returned no group"));
                    return;
                };
                let got = write_form(form, &group);
                let Some(expected) = case[form].as_str() else {
                    failures.push(format!("{label} fixture has no output"));
                    return;
                };
                if got != expected {
                    failures.push(diff_line(&label, &got, expected));
                }
            }
            (_, "refuse") => {
                if read_form(form, input, None).is_some() {
                    failures.push(format!("{label} kept the group"));
                }
            }
            ("xml2", "mask") => {
                let Some(group) = read_form(form, input, None) else {
                    failures.push(format!("{label} returned no group"));
                    return;
                };
                let got = write_form(form, &group);
                if !got.contains("<BaseMask>4294967295</BaseMask>") {
                    failures.push(format!("{label} dropped the unsigned mask"));
                }
                if !case["xml2"].is_null() {
                    failures.push(format!("{label} pin returned a group"));
                }
            }
            ("xml2", "media") => {
                let Some(group) = read_form(form, input, None) else {
                    failures.push(format!("{label} returned no group"));
                    return;
                };
                let got = write_form(form, &group);
                if !got.contains("<MatOvrd>") || !got.contains("<SoundGain>0.5</SoundGain>") {
                    failures.push(format!("{label} lost a field after the media element"));
                }
                if !got.contains("<X>1</X>") {
                    failures.push(format!("{label} cleared the scale"));
                }
                let csharp = case["xml2"].as_str().unwrap_or("");
                if csharp.contains("<MatOvrd>") {
                    failures.push(format!("{label} pin kept MatOvrd"));
                }
            }
            ("xml2", "skip") => {
                let Some(group) = read_form(form, input, None) else {
                    failures.push(format!("{label} returned no group"));
                    return;
                };
                let got = write_form(form, &group);
                if got != clean_xml2 {
                    failures.push(diff_line(&label, &got, clean_xml2));
                }
            }
            ("xml2", "clock") => {
                let Some(csharp) = case["xml2"].as_str() else {
                    failures.push(format!("{label} fixture has no output"));
                    return;
                };
                let date = first_creation(csharp);
                let Some(group) = read_form(form, input, Some(date)) else {
                    failures.push(format!("{label} returned no group"));
                    return;
                };
                let got = write_form(form, &group);
                if got != csharp {
                    failures.push(diff_line(&label, &got, csharp));
                }
            }
            _ => failures.push(format!("{label} has expect {expect}")),
        }
    }

    fn read_form(form: &str, input: &str, now: Option<u32>) -> Option<crate::objects::ObjectGroup> {
        match (form, now) {
            ("xml2", Some(now)) => from_xml2_at(input, now),
            ("xml2", None) => from_xml2(input),
            ("original", Some(now)) => from_original_xml_at(input, now),
            ("original", None) => from_original_xml(input),
            _ => None,
        }
    }

    fn write_form(form: &str, group: &crate::objects::ObjectGroup) -> String {
        match form {
            "xml2" => to_xml2(group),
            "original" => to_original_xml(group),
            _ => String::new(),
        }
    }

    fn first_creation(xml: &str) -> u32 {
        let start = xml.find("<CreationDate>").unwrap() + "<CreationDate>".len();
        let end = xml[start..].find("</CreationDate>").unwrap() + start;
        xml[start..end].parse().unwrap()
    }

    fn diff_line(label: &str, got: &str, expected: &str) -> String {
        let at = got
            .bytes()
            .zip(expected.bytes())
            .position(|(left, right)| left != right)
            .unwrap_or(got.len().min(expected.len()));
        let window = 120;
        format!(
            "{label} differs at byte {at} (got {}, expected {}) got {:?} exp {:?}",
            got.len(),
            expected.len(),
            got.get(at..got.len().min(at + window)).unwrap_or(""),
            expected
                .get(at..expected.len().min(at + window))
                .unwrap_or("")
        )
    }

    /// The pin's reader never finishes this element. This input is not sent
    /// to the oracle.
    #[test]
    fn an_unknown_quaternion_child_is_skipped() {
        let xml = "\
<SceneObjectPart>\
<UUID>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1</UUID>\
<RotationOffset><Nope/><X>0</X><Y>0</Y><Z>0</Z><W>1</W></RotationOffset>\
</SceneObjectPart>";
        let group = from_xml2(xml).unwrap();
        assert_eq!(group.parts[0].rotation, [0.0, 0.0, 0.0, 1.0]);
        let bare = "\
<SceneObjectPart>\
<UUID>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1</UUID>\
<RotationOffset>text<W>1</W></RotationOffset>\
</SceneObjectPart>";
        let group = from_xml2(bare).unwrap();
        assert_eq!(group.parts[0].rotation, [0.0, 0.0, 0.0, 1.0]);
        let bad = "\
<SceneObjectPart>\
<UUID>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1</UUID>\
<RotationOffset><X>abc</X></RotationOffset>\
</SceneObjectPart>";
        assert!(from_xml2(bad).is_none());
    }

    #[test]
    fn a_missing_item_creation_date_uses_the_supplied_instant() {
        let xml = "\
<SceneObjectPart>\
<UUID>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1</UUID>\
<TaskInventory>\
<TaskInventoryItem>\
<ItemID>00000000-0000-0000-0000-000000000001</ItemID>\
</TaskInventoryItem>\
</TaskInventory>\
</SceneObjectPart>";
        let group = from_xml2_at(xml, 1_700_000_999).unwrap();
        assert_eq!(group.parts[0].items[0].creation_date, 1_700_000_999);
        assert_eq!(group.parts[0].items[0].base_permissions, 0x7fff_ffff);
        let written = to_xml2(&group);
        assert!(written.contains("<CreationDate>1700000999</CreationDate>"));
        assert!(written.contains("<BasePermissions>2147483647</BasePermissions>"));
    }

    #[test]
    fn object_xml_bounds_refuse_through_diagnostics() {
        let over = " ".repeat(usize::try_from(MAX_OBJECT_XML_BYTES).unwrap() + 1);
        assert!(from_xml2(&over).is_none());
        assert!(ferroan_diag::metrics().contains("object.xml-bytes"));

        assert!(from_xml2(&parts_xml(MAX_LINKSET_PRIMS)).is_some());
        assert!(from_xml2(&parts_xml(MAX_LINKSET_PRIMS + 1)).is_none());
        assert!(ferroan_diag::metrics().contains("region.linkset-prims"));

        assert!(from_xml2(&items_xml(MAX_TASK_ITEMS)).is_some());
        assert!(from_xml2(&items_xml(MAX_TASK_ITEMS + 1)).is_none());
        assert!(ferroan_diag::metrics().contains("object.task-items"));
    }

    fn parts_xml(count: u64) -> String {
        let mut xml = String::from("<SceneObjectGroup>");
        for index in 0..count {
            xml.push_str(&format!(
                "<SceneObjectPart><UUID>{:08x}-0000-4000-8000-{:012x}</UUID></SceneObjectPart>",
                1, index
            ));
        }
        xml.push_str("</SceneObjectGroup>");
        xml
    }

    fn items_xml(count: u64) -> String {
        let mut xml = String::from(
            "<SceneObjectPart><UUID>aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1</UUID><TaskInventory>",
        );
        for index in 0..count {
            xml.push_str(&format!(
                "<TaskInventoryItem><ItemID>{:08x}-0000-4000-8000-{:012x}</ItemID></TaskInventoryItem>",
                2,
                index
            ));
        }
        xml.push_str("</TaskInventory></SceneObjectPart>");
        xml
    }
}
