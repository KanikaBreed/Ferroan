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
// Ported to Rust for Ferroan from SceneObjectSerializer.cs (the writer),
// SOPVehicle.ToXml2, PhysicsInertiaData.ToXml2, KeyframeMotion.Serialize,
// SceneObjectPart.SerializeAnimations, PrimitiveBaseShape extra params,
// MediaList.ToXml and RenderMaterialsOvrToRawBin, and DAMap.ToXml, at
// OpenSim-Tranquillity `dev-build`
// @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// PrimFlags, profile and hollow names, and MediaEntry.GetOSD follow
// libopenmetaverse
// @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.
//
// Source/OpenSim.Region.Framework/Scenes/LinksetData.cs at
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b has no licence header.
// LICENSE.txt at that revision says OpenSimulator-originated code is BSD
// (Copyright (c) Contributors, http://opensimulator.org/, the notice above)
// and new Tranquillity files are MPL-2.0:
//
// Copyright (c) 2025 Utopia Skye LLC
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// The file's history in Tranquillity starts in 2023, so which of the two
// applies is not certain. This port keeps both. Both are compatible with
// GPL-3.0-or-later.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! XML2 and original-format object XML from a loaded [`ObjectGroup`].
//!
//! Both forms omit the XML declaration and write no indentation. A part that
//! has task items has inventory serial 1. Folder id is the part id. Region
//! handle, category, ownership cost, buoyancy, force and torque are zero.
//! Sound queueing is false, unsitting is allowed, and a scripted sit is not
//! required. Local id and parent id are the loaded values. An extra-params
//! blob whose entry count is zero is one zero byte, and the flexi, light and
//! sculpt fields stay at the unset defaults. Sit-target state is written when
//! it is not the state the offset and rotation already give. Animation
//! sequence numbers are clocks and are not written. Linkset JSON names
//! `Password` before `Value`.

#[path = "xml_float.rs"]
mod xml_float;

use ferroan_llsd::{Llsd, Map, to_xml_string};
use uuid::Uuid;

use crate::objects::{InventoryItem, ObjectGroup, ObjectPart, ObjectShape};
use crate::payloads::{
    DynAttrs, Keyframe, KeyframeMotion, LinksetData, MediaFace, ObjectAnimation, PhysicsInertia,
    RenderMaterials, Vehicle,
};
use xml_float::format_f32;

/// XML2: the form a crossing sends. No script state.
pub fn to_xml2(group: &ObjectGroup) -> String {
    write_group(group, false)
}

/// Original XML: the form inventory stores. A loaded group has no script state,
/// so none is written.
pub fn to_original_xml(group: &ObjectGroup) -> String {
    write_group(group, true)
}

fn write_group(group: &ObjectGroup, original: bool) -> String {
    let mut out = Out::default();
    let Some(root) = group.parts.first() else {
        return String::new();
    };
    out.start("SceneObjectGroup");
    if original {
        out.start("RootPart");
        write_part(&mut out, root, true);
        out.end("RootPart");
    } else {
        write_part(&mut out, root, true);
    }
    out.start("OtherParts");
    for part in group.parts.iter().skip(1) {
        if part.uuid == root.uuid {
            continue;
        }
        if original {
            out.start("Part");
            write_part(&mut out, part, false);
            out.end("Part");
        } else {
            write_part(&mut out, part, false);
        }
    }
    out.end("OtherParts");
    if let Some(motion) = &root.keyframe {
        out.base64("KeyframeMotion", &keyframe_bytes(motion));
    }
    if let Some(text) = &group.rez_parameter {
        out.elem("StartStr", text);
    }
    out.end("SceneObjectGroup");
    out.s
}

fn write_part(out: &mut Out, part: &ObjectPart, is_root: bool) {
    out.start_part();
    out.bool("AllowedDrop", part.allowed_drop);
    let (creator, creator_data) = part_creator(&part.creator_id);
    out.uuid("CreatorID", creator);
    if let Some(data) = creator_data.filter(|text| !text.is_empty()) {
        out.elem("CreatorData", &data);
    }
    out.uuid("FolderID", part.uuid);
    let serial: u32 = if part.items.is_empty() { 0 } else { 1 };
    out.display("InventorySerial", serial);
    write_inventory(out, part);
    out.uuid("UUID", part.uuid);
    out.display("LocalId", part.local_id);
    out.elem("Name", &part.name);
    out.display("Material", part.material);
    out.bool("PassTouches", part.pass_touches);
    out.bool("PassCollisions", part.pass_collisions);
    out.elem("RegionHandle", "0");
    out.display("ScriptAccessPin", part.script_access_pin);
    out.vec3("GroupPosition", part.group_position);
    out.vec3("OffsetPosition", part.offset);
    out.quat("RotationOffset", part.rotation);
    out.vec3("Velocity", part.velocity);
    out.vec3("AngularVelocity", part.angular_velocity);
    out.vec3("Acceleration", part.acceleration);
    out.elem("Description", &part.description);
    write_color(out, part.color);
    out.elem("Text", &part.text);
    out.elem("SitName", &part.sit_name);
    out.elem("TouchName", &part.touch_name);
    out.display("LinkNum", part.link_num);
    out.display("ClickAction", part.click_action);
    write_shape(out, &part.shape);
    out.vec3("Scale", part.shape.scale);
    out.quat("SitTargetOrientation", part.sit_target_rotation);
    out.vec3("SitTargetPosition", part.sit_target_position);
    out.vec3("SitTargetPositionLL", part.sit_target_position);
    out.quat("SitTargetOrientationLL", part.sit_target_rotation);
    if part.stand_offset.iter().any(|value| *value != 0.0) {
        out.vec3("StandTarget", part.stand_offset);
    }
    out.display("ParentID", part.parent_id);
    out.display("CreationDate", part.creation_date);
    out.elem("Category", "0");
    out.display("SalePrice", part.sale_price);
    out.display("ObjectSaleType", part.sale_type);
    out.elem("OwnershipCost", "0");
    out.uuid("GroupID", part.group_id);
    out.uuid("OwnerID", part.owner_id);
    out.uuid("LastOwnerID", part.last_owner_id);
    out.uuid("RezzerID", part.rezzer_id);
    out.display("BaseMask", part.base_mask);
    out.display("OwnerMask", part.owner_mask);
    out.display("GroupMask", part.group_mask);
    out.display("EveryoneMask", part.everyone_mask);
    out.display("NextOwnerMask", part.next_owner_mask);
    out.elem("Flags", &prim_flags(part.flags));
    out.uuid("CollisionSound", part.collision_sound);
    out.float("CollisionSoundVolume", part.collision_sound_volume);
    if !part.media_url.is_empty() {
        out.elem("MediaUrl", &part.media_url);
    }
    out.vec3("AttachedPos", part.attached_pos);
    if let Some(attrs) = part
        .dyn_attrs
        .as_ref()
        .filter(|attrs| !attrs.namespaces.is_empty())
    {
        out.start("DynAttrs");
        out.raw(&dyn_xml(attrs));
        out.end("DynAttrs");
    }
    out.base64("TextureAnimation", &part.texture_animation);
    out.base64("ParticleSystem", &part.particles);
    for (index, price) in part.pay_price.iter().enumerate() {
        out.display(&format!("PayPrice{index}"), *price);
    }
    out.float("Buoyancy", 0.0);
    out.vec3("Force", [0.0; 3]);
    out.vec3("Torque", [0.0; 3]);
    out.bool("VolumeDetectActive", part.volume_detect);
    if let Some(vehicle) = &part.vehicle {
        write_vehicle(out, vehicle);
    }
    if let Some(inertia) = &part.phys_inertia {
        write_inertia(out, inertia);
    }
    if is_root && part.rotation_axis_locks != 0 {
        out.display("RotationAxisLocks", part.rotation_axis_locks);
    }
    out.display("PhysicsShapeType", part.physics_shape_type);
    if part.density != 1000.0 {
        out.float("Density", part.density);
    }
    if part.friction != 0.6 {
        out.float("Friction", part.friction);
    }
    if part.restitution != 0.5 {
        out.float("Bounce", part.restitution);
    }
    if part.gravity_modifier != 1.0 {
        out.float("GravityModifier", part.gravity_modifier);
    }
    out.vec3("CameraEyeOffset", part.camera_eye);
    out.vec3("CameraAtOffset", part.camera_at);
    out.uuid("SoundID", part.sound);
    out.float("SoundGain", part.sound_gain);
    out.display("SoundFlags", part.sound_flags);
    out.elem("SoundRadius", "0");
    out.bool("SoundQueueing", false);
    if let Some(anims) = &part.animations {
        let bytes = animation_bytes(anims);
        if !bytes.is_empty() {
            out.elem("SOPAnims", &base64(&bytes));
        }
    }
    if part.sit_active_range.abs() > 1e-5 {
        out.float("SitActRange", part.sit_active_range);
    }
    if let Some(text) = part.linkset.as_ref().and_then(linkset_json) {
        out.elem("LinksetData", &text);
    }
    out.bool("AllowUnsit", true);
    out.bool("ScriptedSitOnly", false);
    let derived = sit_derived(part.sit_target_position, part.sit_target_rotation);
    if part.sit_target_active != derived {
        out.bool("SitTargetActive", part.sit_target_active);
    }
    out.end("SceneObjectPart");
}

fn write_color(out: &mut Out, color: u32) {
    out.start("Color");
    out.display("R", (color >> 16) & 0xff);
    out.display("G", (color >> 8) & 0xff);
    out.display("B", color & 0xff);
    out.display("A", (color >> 24) & 0xff);
    out.end("Color");
}

fn write_inventory(out: &mut Out, part: &ObjectPart) {
    if part.items.is_empty() {
        return;
    }
    out.start("TaskInventory");
    for item in &part.items {
        write_item(out, item);
    }
    out.end("TaskInventory");
}

fn write_item(out: &mut Out, item: &InventoryItem) {
    out.start("TaskInventoryItem");
    out.uuid("AssetID", item.asset_id);
    out.display("BasePermissions", item.base_permissions);
    out.display("CreationDate", item.creation_date);
    let (creator, creator_data) = item_creator(&item.creator_id);
    out.uuid("CreatorID", creator);
    if let Some(data) = creator_data.filter(|text| !text.is_empty()) {
        out.elem("CreatorData", &data);
    }
    out.elem("Description", &item.description);
    out.display("EveryonePermissions", item.everyone_permissions);
    out.display("Flags", item.flags);
    out.uuid("GroupID", item.group_id);
    out.display("GroupPermissions", item.group_permissions);
    out.display("InvType", item.inv_type);
    out.uuid("ItemID", item.item_id);
    out.uuid("OldItemID", Uuid::nil());
    out.uuid("LastOwnerID", item.last_owner_id);
    out.elem("Name", &item.name);
    out.display("NextPermissions", item.next_permissions);
    out.uuid("OwnerID", item.owner_id);
    out.display("CurrentPermissions", item.current_permissions);
    out.uuid("ParentID", item.parent_id);
    out.uuid("ParentPartID", item.parent_part_id);
    out.uuid("PermsGranter", Uuid::nil());
    out.elem("PermsMask", "0");
    out.display("Type", item.asset_type);
    out.bool("OwnerChanged", false);
    if !item.experience_id.is_nil() {
        out.uuid("ExperienceID", item.experience_id);
    }
    out.end("TaskInventoryItem");
}

fn write_shape(out: &mut Out, shape: &ObjectShape) {
    let extras = decode_extras(&shape.extra_params);
    out.start("Shape");
    out.display("ProfileCurve", shape.profile_curve);
    out.base64("TextureEntry", &shape.texture);
    out.base64("ExtraParams", &extras.bytes);
    out.display("PathBegin", shape.path_begin);
    out.display("PathCurve", shape.path_curve);
    out.display("PathEnd", shape.path_end);
    out.display("PathRadiusOffset", shape.path_radius_offset);
    out.display("PathRevolutions", shape.path_revolutions);
    out.display("PathScaleX", shape.path_scale_x);
    out.display("PathScaleY", shape.path_scale_y);
    out.display("PathShearX", shape.path_shear_x);
    out.display("PathShearY", shape.path_shear_y);
    out.display("PathSkew", shape.path_skew);
    out.display("PathTaperX", shape.path_taper_x);
    out.display("PathTaperY", shape.path_taper_y);
    out.display("PathTwist", shape.path_twist);
    out.display("PathTwistBegin", shape.path_twist_begin);
    out.display("PCode", shape.pcode);
    out.display("ProfileBegin", shape.profile_begin);
    out.display("ProfileEnd", shape.profile_end);
    out.display("ProfileHollow", shape.profile_hollow);
    out.display("State", shape.state);
    out.display("LastAttachPoint", shape.last_attach_point);
    out.elem("ProfileShape", profile_shape(shape.profile_curve));
    out.elem("HollowShape", hollow_shape(shape.profile_curve));
    out.uuid("SculptTexture", extras.sculpt_texture);
    out.display("SculptType", extras.sculpt_type);
    out.display("FlexiSoftness", extras.softness);
    out.float("FlexiTension", extras.tension);
    out.float("FlexiDrag", extras.drag);
    out.float("FlexiGravity", extras.gravity);
    out.float("FlexiWind", extras.wind);
    out.float("FlexiForceX", extras.force[0]);
    out.float("FlexiForceY", extras.force[1]);
    out.float("FlexiForceZ", extras.force[2]);
    out.float("LightColorR", extras.light[0]);
    out.float("LightColorG", extras.light[1]);
    out.float("LightColorB", extras.light[2]);
    out.float("LightColorA", extras.light_a);
    out.float("LightRadius", extras.radius);
    out.float("LightCutoff", extras.cutoff);
    out.float("LightFalloff", extras.falloff);
    out.float("LightIntensity", extras.intensity);
    out.bool("FlexiEntry", extras.flexi);
    out.bool("LightEntry", extras.light_on);
    out.bool("SculptEntry", extras.sculpt);
    if let Some(faces) = shape
        .media_entries
        .as_deref()
        .filter(|faces| !faces.is_empty())
    {
        out.elem("Media", &media_xml(faces));
    }
    if let Some(materials) = &shape.materials {
        out.base64("MatOvrd", &materials_bin(materials));
    }
    out.end("Shape");
}

fn write_vehicle(out: &mut Out, vehicle: &Vehicle) {
    out.start("Vehicle");
    out.display("TYPE", vehicle.kind);
    out.display("FLAGS", vehicle.flags);
    out.vec3("LMDIR", vehicle.linear_motor_direction);
    out.vec3("LMFTIME", vehicle.linear_friction_timescale);
    out.float("LMDTIME", vehicle.linear_motor_decay_timescale);
    out.float("LMTIME", vehicle.linear_motor_timescale);
    out.vec3("LMOFF", vehicle.linear_motor_offset);
    if let Some(value) = vehicle.linear_motor_timescale_axes {
        out.vec3("LMTIMEV", value);
    }
    if let Some(value) = vehicle.linear_motor_decay_timescale_axes {
        out.vec3("LMDTIMEV", value);
    }
    out.vec3("AMDIR", vehicle.angular_motor_direction);
    out.float("AMTIME", vehicle.angular_motor_timescale);
    out.float("AMDTIME", vehicle.angular_motor_decay_timescale);
    out.vec3("AMFTIME", vehicle.angular_friction_timescale);
    if let Some(value) = vehicle.angular_motor_timescale_axes {
        out.vec3("AMTIMEV", value);
    }
    if let Some(value) = vehicle.angular_motor_decay_timescale_axes {
        out.vec3("AMDTIMEV", value);
    }
    out.float("ADEFF", vehicle.angular_deflection_efficiency);
    out.float("ADTIME", vehicle.angular_deflection_timescale);
    out.float("LDEFF", vehicle.linear_deflection_efficiency);
    out.float("LDTIME", vehicle.linear_deflection_timescale);
    out.float("BEFF", vehicle.banking_efficiency);
    out.float("BMIX", vehicle.banking_mix);
    out.float("BTIME", vehicle.banking_timescale);
    out.float("HHEI", vehicle.hover_height);
    out.float("HEFF", vehicle.hover_efficiency);
    out.float("HTIME", vehicle.hover_timescale);
    out.float("VBUO", vehicle.buoyancy);
    out.float("VAEFF", vehicle.vertical_attraction_efficiency);
    out.float("VATIME", vehicle.vertical_attraction_timescale);
    out.float("MLAZ", vehicle.mouselook_azimuth);
    out.float("MLALT", vehicle.mouselook_altitude);
    out.float("BAZ", vehicle.banking_azimuth);
    out.float("DMABOVE", vehicle.disable_motors_above);
    out.float("DMAFTER", vehicle.disable_motors_after);
    out.float("IBMOD", vehicle.inverted_banking_modifier);
    if let Some(value) = vehicle.linear_wind_efficiency {
        out.vec3("LWEFF", value);
    }
    if let Some(value) = vehicle.angular_wind_efficiency {
        out.vec3("AWEFF", value);
    }
    if let Some(value) = vehicle.linear_motor_run_age {
        out.float("LMRAGE", value);
    }
    if let Some(value) = vehicle.linear_motor_run_direction {
        out.vec3("LMRDIR", value);
    }
    if let Some(value) = vehicle.linear_motor_run_target {
        out.vec3("LMRTGT", value);
    }
    if let Some(value) = vehicle.angular_motor_run_age {
        out.float("AMRAGE", value);
    }
    if let Some(value) = vehicle.angular_motor_run_direction {
        out.vec3("AMRDIR", value);
    }
    if let Some(value) = vehicle.angular_motor_run_target {
        out.vec3("AMRTGT", value);
    }
    out.quat("REF_FRAME", vehicle.reference_frame);
    out.end("Vehicle");
}

fn write_inertia(out: &mut Out, inertia: &PhysicsInertia) {
    out.start("PhysicsInertia");
    out.float("MASS", inertia.mass);
    out.vec3("CM", inertia.center);
    out.vec3("INERTIA", inertia.inertia);
    out.quat("IROT", inertia.rotation);
    out.end("PhysicsInertia");
}

fn dyn_xml(attrs: &DynAttrs) -> String {
    to_xml_string(&Llsd::Map(attrs.namespaces.clone()))
}

fn media_xml(faces: &[MediaFace]) -> String {
    let items = faces
        .iter()
        .map(|face| Llsd::Map(media_map(face)))
        .collect();
    let mut text = String::from(r#"<OSMedia type="sl" version="0.1"><OSData>"#);
    text.push_str(&to_xml_string(&Llsd::Array(items)));
    text.push_str("</OSData></OSMedia>");
    text
}

fn media_map(face: &MediaFace) -> Map {
    let mut map = Map::new();
    map.insert(
        "alt_image_enable".to_owned(),
        Llsd::Boolean(face.alt_image_enable),
    );
    map.insert("auto_loop".to_owned(), Llsd::Boolean(face.auto_loop));
    map.insert("auto_play".to_owned(), Llsd::Boolean(face.auto_play));
    map.insert("auto_scale".to_owned(), Llsd::Boolean(face.auto_scale));
    map.insert("auto_zoom".to_owned(), Llsd::Boolean(face.auto_zoom));
    map.insert("controls".to_owned(), Llsd::Integer(face.controls));
    map.insert(
        "current_url".to_owned(),
        Llsd::String(face.current_url.clone().unwrap_or_default()),
    );
    map.insert(
        "first_click_interact".to_owned(),
        Llsd::Boolean(face.first_click_interact),
    );
    map.insert(
        "height_pixels".to_owned(),
        Llsd::Integer(face.height_pixels),
    );
    map.insert(
        "home_url".to_owned(),
        Llsd::String(face.home_url.clone().unwrap_or_default()),
    );
    map.insert(
        "perms_control".to_owned(),
        Llsd::Integer(face.perms_control),
    );
    map.insert(
        "perms_interact".to_owned(),
        Llsd::Integer(face.perms_interact),
    );
    let whitelist = face.whitelist.clone().unwrap_or_default();
    map.insert(
        "whitelist".to_owned(),
        Llsd::Array(whitelist.into_iter().map(Llsd::String).collect()),
    );
    map.insert(
        "whitelist_enable".to_owned(),
        Llsd::Boolean(face.whitelist_enable),
    );
    map.insert("width_pixels".to_owned(), Llsd::Integer(face.width_pixels));
    map
}

fn materials_bin(materials: &RenderMaterials) -> Vec<u8> {
    let Some(overrides) = &materials.overrides else {
        return vec![0];
    };
    let kept: Vec<_> = overrides
        .iter()
        .filter(|item| !item.data.is_empty())
        .collect();
    if kept.is_empty() {
        return vec![0];
    }
    let mut out = Vec::new();
    out.push(kept.len() as u8);
    for item in kept {
        let len = item.data.encode_utf16().count();
        out.push(item.te_index);
        out.push(len as u8);
        out.push((len >> 8) as u8);
        out.extend(item.data.as_bytes());
    }
    out
}

fn linkset_json(data: &LinksetData) -> Option<String> {
    let entries = data
        .entries
        .as_ref()
        .filter(|entries| !entries.is_empty())?;
    let mut out = String::from("{\"Data\":{");
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_json_string(&entry.key, &mut out);
        out.push_str(":{\"Password\":");
        push_json_option(entry.password.as_deref(), &mut out);
        out.push_str(",\"Value\":");
        push_json_option(entry.value.as_deref(), &mut out);
        out.push('}');
    }
    out.push_str("},\"BytesFree\":");
    out.push_str(&data.bytes_free.to_string());
    out.push_str(",\"BytesUsed\":");
    out.push_str(&data.bytes_used.to_string());
    out.push('}');
    Some(out)
}

fn keyframe_bytes(motion: &KeyframeMotion) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(b"KFM1");
    out.extend(1u32.to_le_bytes());
    push_vec3(&mut out, motion.serialized_position);
    push_vec3(&mut out, motion.base_position);
    push_quat(&mut out, motion.base_rotation);
    push_keyframe(&mut out, &motion.current_frame);
    push_i32(&mut out, i32_len(motion.frames.len()));
    for frame in &motion.frames {
        push_keyframe(&mut out, frame);
    }
    match &motion.keyframes {
        None => push_i32(&mut out, -1),
        Some(frames) => {
            push_i32(&mut out, i32_len(frames.len()));
            for frame in frames {
                push_keyframe(&mut out, frame);
            }
        }
    }
    push_i32(&mut out, motion.mode);
    push_i32(&mut out, motion.data);
    out.push(u8::from(motion.running));
    push_i32(&mut out, motion.iterations);
    push_i32(&mut out, motion.skip_loops);
    out
}

fn push_keyframe(out: &mut Vec<u8>, frame: &Keyframe) {
    match frame.position {
        Some(value) => {
            out.push(1);
            push_vec3(out, value);
        }
        None => out.push(0),
    }
    match frame.rotation {
        Some(value) => {
            out.push(1);
            push_quat(out, value);
        }
        None => out.push(0),
    }
    push_quat(out, frame.start_rotation);
    push_i32(out, frame.time_ms);
    push_i32(out, frame.time_total);
    push_vec3(out, frame.angular_velocity);
    push_vec3(out, frame.start_position);
}

fn push_vec3(out: &mut Vec<u8>, value: [f32; 3]) {
    for component in value {
        out.extend(component.to_le_bytes());
    }
}

fn push_quat(out: &mut Vec<u8>, value: [f32; 4]) {
    for component in value {
        out.extend(component.to_le_bytes());
    }
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend(value.to_le_bytes());
}

fn i32_len(len: usize) -> i32 {
    i32::try_from(len).unwrap_or(i32::MAX)
}

fn animation_bytes(anims: &[ObjectAnimation]) -> Vec<u8> {
    if anims.is_empty() {
        return vec![0];
    }
    let mut out = Vec::new();
    let count = u16::try_from(anims.len()).unwrap_or(u16::MAX);
    out.extend(count.to_le_bytes());
    for anim in anims.iter().take(usize::from(count)) {
        out.extend(anim.id.as_bytes());
        if anim.name.is_empty() {
            out.push(0);
            continue;
        }
        let bytes = utf8_limit(&anim.name, 63);
        let len = u8::try_from(bytes.len()).unwrap_or(u8::MAX);
        out.push(len);
        out.extend(&bytes[..usize::from(len)]);
    }
    out
}

fn utf8_limit(text: &str, max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in text.chars() {
        let mut buf = [0u8; 4];
        let encoded = ch.encode_utf8(&mut buf);
        if out.len() + encoded.len() > max {
            break;
        }
        out.extend(encoded.as_bytes());
    }
    out
}

fn part_creator(identification: &str) -> (Uuid, Option<String>) {
    if identification.is_empty() {
        return (Uuid::nil(), None);
    }
    let mut parts = identification.split(';');
    let id = parts
        .next()
        .and_then(|text| Uuid::parse_str(text).ok())
        .unwrap_or(Uuid::nil());
    let Some(home) = parts.next() else {
        return (id, None);
    };
    let mut data = home.to_owned();
    if !data.ends_with('/') {
        data.push('/');
    }
    data.push(';');
    data.push_str(parts.next().unwrap_or("Unknown User"));
    (id, Some(data))
}

fn item_creator(identification: &str) -> (Uuid, Option<String>) {
    if identification.is_empty() {
        return (Uuid::nil(), None);
    }
    if !identification.contains(';') {
        return (Uuid::parse_str(identification).unwrap_or(Uuid::nil()), None);
    }
    let parts: Vec<&str> = identification.split(';').collect();
    let id = parts
        .first()
        .and_then(|text| Uuid::parse_str(text).ok())
        .unwrap_or(Uuid::nil());
    let home = parts.get(1).copied().unwrap_or("");
    let name = parts.get(2).copied().unwrap_or("Unknown User");
    (id, Some(format!("{home};{name}")))
}

fn sit_derived(offset: [f32; 3], rotation: [f32; 4]) -> bool {
    offset.iter().any(|value| *value != 0.0)
        || rotation[0] != 0.0
        || rotation[1] != 0.0
        || rotation[2] != 0.0
}

const PRIM_FLAG_NAMES: &[(u32, &str)] = &[
    (0x0000_0001, "Physics"),
    (0x0000_0002, "CreateSelected"),
    (0x0000_0004, "ObjectModify"),
    (0x0000_0008, "ObjectCopy"),
    (0x0000_0010, "ObjectAnyOwner"),
    (0x0000_0020, "ObjectYouOwner"),
    (0x0000_0040, "Scripted"),
    (0x0000_0080, "Touch"),
    (0x0000_0100, "ObjectMove"),
    (0x0000_0200, "Money"),
    (0x0000_0400, "Phantom"),
    (0x0000_0800, "InventoryEmpty"),
    (0x0000_1000, "JointHinge"),
    (0x0000_2000, "JointP2P"),
    (0x0000_4000, "JointLP2P"),
    (0x0000_8000, "JointWheel"),
    (0x0001_0000, "AllowInventoryDrop"),
    (0x0002_0000, "ObjectTransfer"),
    (0x0004_0000, "ObjectGroupOwned"),
    (0x0008_0000, "ObjectYouOfficer"),
    (0x0010_0000, "CameraDecoupled"),
    (0x0020_0000, "AnimSource"),
    (0x0040_0000, "CameraSource"),
    (0x0080_0000, "CastShadows"),
    (0x0100_0000, "DieAtEdge"),
    (0x0200_0000, "ReturnAtEdge"),
    (0x0400_0000, "Sandbox"),
    (0x0800_0000, "Flying"),
    (0x1000_0000, "ObjectOwnerModify"),
    (0x2000_0000, "TemporaryOnRez"),
    (0x4000_0000, "Temporary"),
    (0x8000_0000, "ZlibCompressed"),
];

fn prim_flags(value: u32) -> String {
    flag_names(value, PRIM_FLAG_NAMES).replace(',', "")
}

fn flag_names(value: u32, names: &[(u32, &str)]) -> String {
    if value == 0 {
        return "None".to_owned();
    }
    let mut left = value;
    let mut chosen = Vec::new();
    for &(bit, name) in names.iter().rev() {
        if bit != 0 && left & bit == bit {
            left -= bit;
            chosen.insert(0, name);
        }
    }
    if left != 0 {
        return value.to_string();
    }
    chosen.join(", ")
}

fn profile_shape(curve: u8) -> &'static str {
    match curve & 0x0f {
        0 => "Circle",
        1 => "Square",
        2 => "IsometricTriangle",
        3 => "EquilateralTriangle",
        4 => "RightTriangle",
        5 => "HalfCircle",
        _ => "Square",
    }
}

fn hollow_shape(curve: u8) -> &'static str {
    match curve & 0xf0 {
        0 => "Same",
        16 => "Circle",
        32 => "Square",
        48 => "Triangle",
        _ => "Same",
    }
}

struct Extras {
    bytes: Vec<u8>,
    softness: i32,
    tension: f32,
    drag: f32,
    gravity: f32,
    wind: f32,
    force: [f32; 3],
    light: [f32; 3],
    light_a: f32,
    radius: f32,
    cutoff: f32,
    falloff: f32,
    intensity: f32,
    flexi: bool,
    light_on: bool,
    sculpt: bool,
    sculpt_texture: Uuid,
    sculpt_type: u8,
    projection: Option<Projection>,
    mesh_flags: Option<u32>,
    probe: Option<Probe>,
    materials: Vec<(u8, Uuid)>,
}

struct Projection {
    texture: Uuid,
    fov: f32,
    focus: f32,
    ambiance: f32,
}

struct Probe {
    ambiance: f32,
    clip: f32,
    flags: u8,
}

impl Extras {
    fn blank() -> Self {
        Self {
            bytes: Vec::new(),
            softness: 0,
            tension: 0.0,
            drag: 0.0,
            gravity: 0.0,
            wind: 0.0,
            force: [0.0; 3],
            light: [0.0; 3],
            light_a: 1.0,
            radius: 0.0,
            cutoff: 0.0,
            falloff: 0.0,
            intensity: 1.0,
            flexi: false,
            light_on: false,
            sculpt: false,
            sculpt_texture: Uuid::nil(),
            sculpt_type: 0,
            projection: None,
            mesh_flags: None,
            probe: None,
            materials: Vec::new(),
        }
    }
}

fn decode_extras(data: &[u8]) -> Extras {
    let mut extras = Extras::blank();
    if data.len() > 1 && data[0] != 0 {
        parse_extras(&mut extras, data);
    }
    extras.bytes = encode_extras(&extras);
    extras
}

fn parse_extras(extras: &mut Extras, data: &[u8]) {
    let count = usize::from(data[0]);
    let mut index = 1usize;
    for _ in 0..count {
        let Some(kind) = data.get(index).copied() else {
            break;
        };
        match kind {
            0x10 => {
                if !advance(&mut index, 6) {
                    break;
                }
                read_flexi(extras, data, index);
                if !advance(&mut index, 16) {
                    break;
                }
            }
            0x20 => {
                if !advance(&mut index, 6) {
                    break;
                }
                read_light(extras, data, index);
                if !advance(&mut index, 16) {
                    break;
                }
            }
            0x30 | 0x60 => {
                if !advance(&mut index, 6) {
                    break;
                }
                read_sculpt(extras, data, index);
                if !advance(&mut index, 17) {
                    break;
                }
            }
            0x40 => {
                if !advance(&mut index, 6) {
                    break;
                }
                read_projection(extras, data, index);
                if !advance(&mut index, 28) {
                    break;
                }
            }
            0x70 => {
                if !advance(&mut index, 6) {
                    break;
                }
                extras.mesh_flags = Some(le_u32(data, index).unwrap_or(0));
                if !advance(&mut index, 4) {
                    break;
                }
            }
            0x90 => {
                if !advance(&mut index, 6) {
                    break;
                }
                read_probe(extras, data, index);
                if !advance(&mut index, 9) {
                    break;
                }
            }
            0x80 => {
                if !advance(&mut index, 2) {
                    break;
                }
                let Some(size) = le_i32(data, index) else {
                    break;
                };
                if !advance(&mut index, 4) {
                    break;
                }
                if size > 17 {
                    read_material_entries(extras, data, index, size);
                }
                if size < 0 || !advance(&mut index, size as usize + 4) {
                    break;
                }
            }
            _ => break,
        }
    }
}

fn advance(index: &mut usize, count: usize) -> bool {
    match index.checked_add(count) {
        Some(next) => {
            *index = next;
            true
        }
        None => false,
    }
}

fn read_flexi(extras: &mut Extras, data: &[u8], pos: usize) {
    if data.len().saturating_sub(pos) < 16 {
        extras.flexi = false;
        extras.softness = 0;
        extras.tension = 0.0;
        extras.drag = 0.0;
        extras.gravity = 0.0;
        extras.wind = 0.0;
        extras.force = [0.0; 3];
        return;
    }
    let first = data[pos];
    let second = data[pos + 1];
    extras.flexi = true;
    extras.softness = i32::from((first & 0x80) >> 6) | i32::from((second & 0x80) >> 7);
    extras.tension = f32::from(first & 0x7f) / 10.0;
    extras.drag = f32::from(second & 0x7f) / 10.0;
    extras.gravity = f32::from(data[pos + 2]) / 10.0 - 10.0;
    extras.wind = f32::from(data[pos + 3]) / 10.0;
    extras.force = [
        le_f32(data, pos + 4),
        le_f32(data, pos + 8),
        le_f32(data, pos + 12),
    ];
}

fn read_light(extras: &mut Extras, data: &[u8], pos: usize) {
    extras.light_a = 1.0;
    if data.len().saturating_sub(pos) < 16 {
        extras.light_on = false;
        extras.light = [0.0; 3];
        extras.radius = 0.0;
        extras.cutoff = 0.0;
        extras.falloff = 0.0;
        extras.intensity = 0.0;
        return;
    }
    let quanta = 1.0 / 255.0;
    extras.light_on = true;
    extras.light = [
        f32::from(data[pos]) * quanta,
        f32::from(data[pos + 1]) * quanta,
        f32::from(data[pos + 2]) * quanta,
    ];
    extras.intensity = f32::from(data[pos + 3]) * quanta;
    extras.radius = le_f32(data, pos + 4);
    extras.cutoff = le_f32(data, pos + 8);
    extras.falloff = le_f32(data, pos + 12);
}

fn read_sculpt(extras: &mut Extras, data: &[u8], pos: usize) {
    if data.len().saturating_sub(pos) < 17 {
        extras.sculpt = false;
        extras.sculpt_texture = Uuid::nil();
        extras.sculpt_type = 0;
        return;
    }
    extras.sculpt = true;
    extras.sculpt_texture = uuid_at(data, pos);
    extras.sculpt_type = data[pos + 16];
}

fn read_projection(extras: &mut Extras, data: &[u8], pos: usize) {
    if data.len().saturating_sub(pos) < 28 {
        extras.projection = None;
        return;
    }
    extras.projection = Some(Projection {
        texture: uuid_at(data, pos),
        fov: le_f32(data, pos + 16),
        focus: le_f32(data, pos + 20),
        ambiance: le_f32(data, pos + 24),
    });
}

fn read_probe(extras: &mut Extras, data: &[u8], pos: usize) {
    if data.len().saturating_sub(pos) < 9 {
        return;
    }
    extras.probe = Some(Probe {
        ambiance: le_f32(data, pos).clamp(0.0, 100.0),
        clip: le_f32(data, pos + 4).clamp(0.0, 1024.0),
        flags: data[pos + 8],
    });
}

fn read_material_entries(extras: &mut Extras, data: &[u8], pos: usize, size: i32) {
    let Some(count) = data.get(pos).copied() else {
        return;
    };
    let count = usize::from(count);
    let need = 1usize.saturating_add(17usize.saturating_mul(count));
    if (size as usize) < need || data.len().saturating_sub(pos) < need {
        return;
    }
    let mut entries = Vec::with_capacity(count);
    let mut cursor = pos + 1;
    for _ in 0..count {
        let te_index = data[cursor];
        cursor += 1;
        entries.push((te_index, uuid_at(data, cursor)));
        cursor += 16;
    }
    extras.materials = entries;
}

fn encode_extras(extras: &Extras) -> Vec<u8> {
    let mut body = Vec::new();
    let mut count = 0u8;
    if extras.flexi {
        count += 1;
        push_entry(&mut body, 0x10, &flexi_bytes(extras));
    }
    if extras.light_on {
        count += 1;
        push_entry(&mut body, 0x20, &light_bytes(extras));
    }
    if extras.sculpt {
        count += 1;
        push_entry(&mut body, 0x30, &sculpt_bytes(extras));
    }
    if let Some(projection) = &extras.projection {
        count += 1;
        push_entry(&mut body, 0x40, &projection_bytes(projection));
    }
    if let Some(flags) = extras.mesh_flags {
        count += 1;
        push_entry(&mut body, 0x70, &flags.to_le_bytes());
    }
    if let Some(probe) = &extras.probe {
        count += 1;
        push_entry(&mut body, 0x90, &probe_bytes(probe));
    }
    if !extras.materials.is_empty() {
        count += 1;
        push_entry(&mut body, 0x80, &extra_materials(extras));
    }
    if count == 0 {
        return vec![0];
    }
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(count);
    out.extend(body);
    out
}

fn push_entry(out: &mut Vec<u8>, kind: u8, data: &[u8]) {
    out.push(kind);
    out.push(0);
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend(len.to_le_bytes());
    out.extend(data);
}

fn flexi_bytes(extras: &Extras) -> [u8; 16] {
    let mut data = [0u8; 16];
    let tension = (extras.tension * 10.01) as u8;
    let drag = (extras.drag * 10.01) as u8;
    data[0] = (((extras.softness & 2) as u8) << 6) | (tension & 0x7f);
    data[1] = (((extras.softness & 1) as u8) << 7) | (drag & 0x7f);
    data[2] = ((extras.gravity + 10.0) * 10.01) as u8;
    data[3] = (extras.wind * 10.01) as u8;
    data[4..8].copy_from_slice(&extras.force[0].to_le_bytes());
    data[8..12].copy_from_slice(&extras.force[1].to_le_bytes());
    data[12..16].copy_from_slice(&extras.force[2].to_le_bytes());
    data
}

fn light_bytes(extras: &Extras) -> [u8; 16] {
    let mut data = [0u8; 16];
    data[0] = unit_byte(extras.light[0]);
    data[1] = unit_byte(extras.light[1]);
    data[2] = unit_byte(extras.light[2]);
    data[3] = unit_byte(extras.intensity);
    data[4..8].copy_from_slice(&extras.radius.to_le_bytes());
    data[8..12].copy_from_slice(&extras.cutoff.to_le_bytes());
    data[12..16].copy_from_slice(&extras.falloff.to_le_bytes());
    data
}

fn sculpt_bytes(extras: &Extras) -> [u8; 17] {
    let mut data = [0u8; 17];
    data[..16].copy_from_slice(extras.sculpt_texture.as_bytes());
    data[16] = extras.sculpt_type;
    data
}

fn projection_bytes(projection: &Projection) -> [u8; 28] {
    let mut data = [0u8; 28];
    data[..16].copy_from_slice(projection.texture.as_bytes());
    data[16..20].copy_from_slice(&projection.fov.to_le_bytes());
    data[20..24].copy_from_slice(&projection.focus.to_le_bytes());
    data[24..28].copy_from_slice(&projection.ambiance.to_le_bytes());
    data
}

fn probe_bytes(probe: &Probe) -> [u8; 9] {
    let mut data = [0u8; 9];
    data[..4].copy_from_slice(&probe.ambiance.to_le_bytes());
    data[4..8].copy_from_slice(&probe.clip.to_le_bytes());
    data[8] = probe.flags;
    data
}

fn extra_materials(extras: &Extras) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 17 * extras.materials.len());
    data.push(extras.materials.len() as u8);
    for (te_index, id) in &extras.materials {
        data.push(*te_index);
        data.extend(id.as_bytes());
    }
    data
}

fn unit_byte(value: f32) -> u8 {
    if value <= 0.0 {
        0
    } else if value >= 1.0 {
        255
    } else {
        (255.0 * value) as u8
    }
}

fn le_f32(data: &[u8], index: usize) -> f32 {
    le_u32(data, index).map(f32::from_bits).unwrap_or(0.0)
}

fn le_u32(data: &[u8], index: usize) -> Option<u32> {
    let bytes = data.get(index..index + 4)?;
    let bytes = <[u8; 4]>::try_from(bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn le_i32(data: &[u8], index: usize) -> Option<i32> {
    le_u32(data, index).map(|bits| i32::from_le_bytes(bits.to_le_bytes()))
}

fn uuid_at(data: &[u8], index: usize) -> Uuid {
    let Some(bytes) = data.get(index..index + 16) else {
        return Uuid::nil();
    };
    let Ok(bytes) = <[u8; 16]>::try_from(bytes) else {
        return Uuid::nil();
    };
    Uuid::from_bytes(bytes)
}

fn push_json_option(value: Option<&str>, out: &mut String) {
    match value {
        None => out.push_str("null"),
        Some(text) => push_json_string(text, out),
    }
}

fn push_json_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other
                if (other as u32) < 0x20
                    || matches!(other, '<' | '>' | '&' | '\u{2028}' | '\u{2029}')
                    || (other as u32) > 0x7f =>
            {
                let mut units = [0u16; 2];
                let encoded = other.encode_utf16(&mut units);
                for unit in &*encoded {
                    out.push_str(&format!("\\u{unit:04X}"));
                }
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// `XmlTextWriter.WriteBase64` and `Convert.ToBase64String` on the pin's
/// runtime emit one continuous string. No 76-column line break.
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut index = 0usize;
    while index < data.len() {
        let first = data[index];
        let second = data.get(index + 1).copied();
        let third = data.get(index + 2).copied();
        let value = (u32::from(first) << 16)
            | (u32::from(second.unwrap_or(0)) << 8)
            | u32::from(third.unwrap_or(0));
        let chars = [
            TABLE[((value >> 18) & 0x3f) as usize],
            TABLE[((value >> 12) & 0x3f) as usize],
            if second.is_some() {
                TABLE[((value >> 6) & 0x3f) as usize]
            } else {
                b'='
            },
            if third.is_some() {
                TABLE[(value & 0x3f) as usize]
            } else {
                b'='
            },
        ];
        for byte in chars {
            out.push(byte as char);
        }
        index += 3;
    }
    out
}

struct Open {
    start: usize,
    empty: bool,
}

#[derive(Default)]
struct Out {
    s: String,
    /// Open elements. An element that receives no content is rewritten to
    /// `<Name />`, which is how XmlTextWriter spells an empty
    /// `WriteElementString` and an empty element pair.
    open: Vec<Open>,
}

impl Out {
    fn note_content(&mut self) {
        if let Some(frame) = self.open.last_mut() {
            frame.empty = false;
        }
    }

    fn start(&mut self, name: &str) {
        self.note_content();
        self.open.push(Open {
            start: self.s.len(),
            empty: true,
        });
        self.s.push('<');
        self.s.push_str(name);
        self.s.push('>');
    }

    fn start_part(&mut self) {
        self.note_content();
        self.open.push(Open {
            start: self.s.len(),
            empty: true,
        });
        self.s.push_str(
            "<SceneObjectPart xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">",
        );
    }

    fn end(&mut self, name: &str) {
        let Some(frame) = self.open.pop() else {
            return;
        };
        if frame.empty {
            self.s.truncate(frame.start);
            self.s.push('<');
            self.s.push_str(name);
            self.s.push_str(" />");
            return;
        }
        self.s.push_str("</");
        self.s.push_str(name);
        self.s.push('>');
    }

    fn elem(&mut self, name: &str, text: &str) {
        self.start(name);
        if !text.is_empty() {
            self.note_content();
            escape(text, &mut self.s);
        }
        self.end(name);
    }

    fn display(&mut self, name: &str, value: impl std::fmt::Display) {
        self.elem(name, &value.to_string());
    }

    fn bool(&mut self, name: &str, value: bool) {
        self.elem(name, if value { "true" } else { "false" });
    }

    fn float(&mut self, name: &str, value: f32) {
        self.elem(name, &format_f32(value));
    }

    fn uuid(&mut self, name: &str, id: Uuid) {
        self.start(name);
        self.elem("UUID", &id.to_string());
        self.end(name);
    }

    fn vec3(&mut self, name: &str, value: [f32; 3]) {
        self.start(name);
        self.float("X", value[0]);
        self.float("Y", value[1]);
        self.float("Z", value[2]);
        self.end(name);
    }

    fn quat(&mut self, name: &str, value: [f32; 4]) {
        self.start(name);
        self.float("X", value[0]);
        self.float("Y", value[1]);
        self.float("Z", value[2]);
        self.float("W", value[3]);
        self.end(name);
    }

    fn base64(&mut self, name: &str, data: &[u8]) {
        // WriteBase64 always opens and closes a full element, even when the
        // encoded text is empty.
        self.start(name);
        self.note_content();
        self.s.push_str(&base64(data));
        self.end(name);
    }

    fn raw(&mut self, text: &str) {
        if !text.is_empty() {
            self.note_content();
        }
        self.s.push_str(text);
    }
}

fn escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{to_original_xml, to_xml2};
    use crate::open_objects;
    use uuid::Uuid;

    const REGION: &str = "11111111-1111-1111-1111-111111111111";
    const PIN: &str = "c401bceecd1684832ae8ba86a31b5ee01a4ded7b";

    fn fixture_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/regionstore")
    }

    fn copied_store() -> String {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-object-xml-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("objects.db");
        std::fs::copy(fixture_dir().join("objects.db"), &path).unwrap();
        format!("Data Source={};Version=3", path.display())
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
        let window = 160;
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

    #[tokio::test]
    async fn regionstore_xml_matches_the_serializer_oracle() {
        let text = std::fs::read_to_string(fixture_dir().join("objects-xml.json")).unwrap();
        let dump: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(dump["pin"], PIN);
        assert_eq!(dump["region"], REGION);
        let groups = open_objects(&copied_store(), Uuid::parse_str(REGION).unwrap())
            .await
            .unwrap();
        let rows = dump["groups"].as_array().unwrap();
        assert_eq!(groups.len(), rows.len());
        assert_eq!(groups.len(), 4);
        for (group, row) in groups.iter().zip(rows) {
            let id = group.parts[0].uuid.to_string();
            assert_eq!(row["id"].as_str().unwrap(), id);
            assert_same(
                &format!("{id} xml2"),
                &to_xml2(group),
                row["xml2"].as_str().unwrap(),
            );
            assert_same(
                &format!("{id} original"),
                &to_original_xml(group),
                row["original"].as_str().unwrap(),
            );
        }
    }
}
