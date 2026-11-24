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
// Ported to Rust for Ferroan from SQLiteSimulationData, SceneObjectGroup,
// SceneObjectPart and PrimitiveBaseShape at
// OpenSim-NGC/OpenSim-Tranquillity `develop` @ 1052cbae82ae7dc3b8abfcffe33e316498eeaad9.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scalar prims and shapes from a SQLite region store.
//!
//! Groups come back in table order. The root row is the one whose id equals
//! its group id. A single prim keeps link number 0. The first child makes the
//! root 1 and itself 2, and each later child takes the next count. Children
//! whose stored link numbers are exactly 2 through the group's prim count,
//! each once, are attached in that order and then keep the stored number. A
//! null stored number counts as 0. Any other group keeps table order, and one
//! warning names it. A row that cannot be built is skipped, and so is a child
//! whose root was skipped. The rest of the region still loads. Local ids and
//! parent ids stay 0. A missing shape is the default box. A stored rotation
//! whose squared length is at most `1e-6` loads as identity; any longer one is
//! scaled to length 1.
//!
//! Texture entries and extra params stay the stored bytes. Media, the materials
//! override, dynattrs, keyframe motion, vehicle, inertia, animations and
//! linkset data stay the stored text or bytes. Touch, money and
//! allow-inventory-drop are cleared from the stored flags. A non-zero
//! volume-detect cell sets bit `0x4000`, which viewers read as volume detect.
//! A null media URL loads as an empty string. A stored pseudo-CRC of 0 is
//! absent: the part's clock value is not kept. Q65 sends the CRC.

use std::collections::{HashMap, HashSet};

use ferroan_db::{Backend, Pool};
use ferroan_diag::{Kind, REGION_LINKSET_PRIMS, REGION_OBJECTS, refuse};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use crate::error::ObjectError;

/// Prim count a region load admits. Scene startup has no such cap (D42).
pub const MAX_REGION_OBJECTS: u64 = 100_000;
/// Prim count one linkset admits. A default region allows any size (D42).
pub const MAX_LINKSET_PRIMS: u64 = 10_000;

/// Cleared from the stored flags when the part is loaded.
const FLAG_TOUCH: u32 = 0x0000_0080;
const FLAG_MONEY: u32 = 0x0000_0200;
const FLAG_ALLOW_DROP: u32 = 0x0001_0000;
/// Viewers read this bit as volume detect. A non-zero volume-detect cell sets it.
const FLAG_VOLUME_DETECT: u32 = 0x0000_4000;

/// Plywood face bytes of a prim that has no shape row, captured at pin `1052cbae82`.
const DEFAULT_BOX_TEXTURE: &[u8] = &[
    0x89, 0x55, 0x67, 0x47, 0x24, 0xcb, 0x43, 0xed, 0x92, 0x0b, 0x47, 0xca, 0xed, 0x15, 0x46, 0x5f,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectGroup {
    pub rez_parameter: Option<String>,
    pub parts: Vec<ObjectPart>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectPart {
    pub uuid: Uuid,
    pub link_num: i32,
    pub local_id: u32,
    pub parent_id: u32,
    pub creation_date: i32,
    pub name: String,
    pub text: String,
    pub color: u32,
    pub description: String,
    pub sit_name: String,
    pub touch_name: String,
    pub flags: u32,
    pub creator_id: String,
    pub owner_id: Uuid,
    pub group_id: Uuid,
    pub last_owner_id: Uuid,
    pub rezzer_id: Uuid,
    pub owner_mask: u32,
    pub next_owner_mask: u32,
    pub group_mask: u32,
    pub everyone_mask: u32,
    pub base_mask: u32,
    pub offset: [f32; 3],
    pub group_position: [f32; 3],
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub acceleration: [f32; 3],
    pub rotation: [f32; 4],
    pub sit_target_position: [f32; 3],
    pub sit_target_rotation: [f32; 4],
    pub sit_target_active: bool,
    pub stand_offset: [f32; 3],
    pub sit_active_range: f32,
    pub click_action: u8,
    pub pay_price: [i32; 5],
    pub sound: Uuid,
    pub sound_gain: f32,
    pub sound_flags: u8,
    pub texture_animation: Vec<u8>,
    pub particles: Vec<u8>,
    pub camera_eye: [f32; 3],
    pub camera_at: [f32; 3],
    pub force_mouselook: bool,
    pub script_access_pin: i32,
    pub allowed_drop: bool,
    pub die_at_edge: bool,
    pub sale_price: i32,
    pub sale_type: u8,
    pub material: u8,
    pub collision_sound: Uuid,
    pub collision_sound_volume: f32,
    pub volume_detect: bool,
    /// Empty when the stored cell is null.
    pub media_url: String,
    pub attached_pos: [f32; 3],
    pub physics_shape_type: u8,
    pub density: f32,
    pub gravity_modifier: f32,
    pub friction: f32,
    pub restitution: f32,
    pub pass_collisions: bool,
    pub pass_touches: bool,
    pub rotation_axis_locks: u8,
    /// `None` when the stored value is null or 0. The clock substitute is not kept.
    pub pseudo_crc: Option<i32>,
    pub shape: ObjectShape,
    pub raw: ObjectRaw,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectShape {
    pub default_box: bool,
    pub scale: [f32; 3],
    pub pcode: u8,
    pub path_begin: u16,
    pub path_end: u16,
    pub path_scale_x: u8,
    pub path_scale_y: u8,
    pub path_shear_x: u8,
    pub path_shear_y: u8,
    pub path_skew: i8,
    pub path_curve: u8,
    pub path_radius_offset: i8,
    pub path_revolutions: u8,
    pub path_taper_x: i8,
    pub path_taper_y: i8,
    pub path_twist: i8,
    pub path_twist_begin: i8,
    pub profile_begin: u16,
    pub profile_end: u16,
    pub profile_curve: u8,
    pub profile_hollow: u16,
    pub state: u8,
    pub last_attach_point: u8,
    pub texture: Vec<u8>,
    pub extra_params: Vec<u8>,
}

/// Stored text and bytes. They are not parsed here.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectRaw {
    pub media: Option<String>,
    pub mat_ovrd: Option<Vec<u8>>,
    pub dyn_attrs: Option<String>,
    pub keyframe_motion: Option<Vec<u8>>,
    pub vehicle: Option<String>,
    pub phys_inertia: Option<String>,
    pub sopanims: Option<Vec<u8>>,
    pub linkset_data: Option<String>,
}

struct PrimRow {
    uuid_text: String,
    group_text: String,
    region_text: String,
    link: Option<i32>,
    start: Option<String>,
    part: Result<ObjectPart, ()>,
}

/// Open the region store and load one region's prims. A missing file is an empty list.
pub async fn open_objects(connection: &str, region: Uuid) -> Result<Vec<ObjectGroup>, ObjectError> {
    let path = ferroan_db::sqlite_filename(connection)
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let pool = Pool::open(Backend::Sqlite, connection, &["RegionStore"])
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    load_objects(&pool, region).await
}

/// Load with [`MAX_REGION_OBJECTS`] and [`MAX_LINKSET_PRIMS`].
pub async fn load_objects(pool: &Pool, region: Uuid) -> Result<Vec<ObjectGroup>, ObjectError> {
    load_objects_limited(pool, region, MAX_REGION_OBJECTS, MAX_LINKSET_PRIMS).await
}

/// Load one region. `max_objects` and `max_linkset` are the hard caps.
pub async fn load_objects_limited(
    pool: &Pool,
    region: Uuid,
    max_objects: u64,
    max_linkset: u64,
) -> Result<Vec<ObjectGroup>, ObjectError> {
    let Some(sqlite) = pool.sqlite() else {
        return Err(ObjectError::Store("connection is not SQLite".to_owned()));
    };
    let prims = sqlx::query(PRIM_SQL)
        .fetch_all(sqlite)
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    let shape_rows = sqlx::query(SHAPE_SQL)
        .fetch_all(sqlite)
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    let mut shapes = HashMap::new();
    for row in &shape_rows {
        let id: String = row
            .try_get("UUID")
            .map_err(|error| ObjectError::Store(error.to_string()))?;
        shapes.insert(id, row);
    }
    let mut rows = Vec::new();
    for row in &prims {
        rows.push(read_prim(row, &shapes));
    }
    let wanted = region.as_hyphenated().to_string();
    let rows: Vec<PrimRow> = rows
        .into_iter()
        .filter(|row| row.region_text.eq_ignore_ascii_case(&wanted))
        .collect();
    let groups = assemble(&rows);
    enforce(groups, max_objects, max_linkset)
}

fn assemble(rows: &[PrimRow]) -> Vec<ObjectGroup> {
    let mut groups = Vec::new();
    let mut index: HashMap<Uuid, usize> = HashMap::new();
    for row in rows {
        if row.uuid_text != row.group_text {
            continue;
        }
        let Ok(part) = &row.part else {
            log::warn!("prim {} could not be built and was skipped", row.uuid_text);
            continue;
        };
        if index.contains_key(&part.uuid) {
            log::warn!("prim {} could not be built and was skipped", row.uuid_text);
            continue;
        }
        index.insert(part.uuid, groups.len());
        groups.push(ObjectGroup {
            rez_parameter: row.start.clone(),
            parts: vec![part.clone()],
        });
    }

    let (order, restore) = child_order(rows);
    for index_in_rows in order {
        let row = &rows[index_in_rows];
        let Ok(mut part) = row.part.clone() else {
            log::warn!("prim {} could not be built and was skipped", row.uuid_text);
            continue;
        };
        let Some(group_id) = parse_uuid(&row.group_text) else {
            log::warn!("prim {} has no root and was skipped", row.uuid_text);
            continue;
        };
        let Some(&slot) = index.get(&group_id) else {
            log::warn!("prim {} has no root and was skipped", row.uuid_text);
            continue;
        };
        let count = groups[slot].parts.len().saturating_add(1);
        let Ok(link_num) = i32::try_from(count) else {
            log::warn!("prim {} could not be built and was skipped", row.uuid_text);
            continue;
        };
        part.link_num = link_num;
        if link_num == 2 {
            groups[slot].parts[0].link_num = 1;
        }
        if restore.contains(&row.group_text) {
            part.link_num = row.link.unwrap_or(0);
        }
        groups[slot].parts.push(part);
    }
    groups
}

fn child_order(rows: &[PrimRow]) -> (Vec<usize>, HashSet<String>) {
    let mut by_group: HashMap<String, Vec<usize>> = HashMap::new();
    let mut group_order = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if row.uuid_text == row.group_text {
            continue;
        }
        if !by_group.contains_key(&row.group_text) {
            group_order.push(row.group_text.clone());
        }
        by_group
            .entry(row.group_text.clone())
            .or_default()
            .push(index);
    }
    let mut restore = HashSet::new();
    let mut ordered = Vec::new();
    for group in group_order {
        let children = &by_group[&group];
        let last = children.len().saturating_add(1);
        let mut seen = vec![false; last.saturating_add(1)];
        let mut usable = true;
        for index in children {
            let number = rows[*index].link.unwrap_or(0);
            let Some(slot) = usize::try_from(number).ok() else {
                usable = false;
                break;
            };
            if number < 2 || slot > last || seen.get(slot).copied().unwrap_or(true) {
                usable = false;
                break;
            }
            seen[slot] = true;
        }
        if usable {
            restore.insert(group);
            let mut sorted = children.clone();
            sorted.sort_by_key(|index| rows[*index].link.unwrap_or(0));
            ordered.extend(sorted);
        } else {
            log::warn!(
                "object {group} has prims whose stored link numbers are missing, repeated or out of range; its prims are numbered in the order they were stored"
            );
            ordered.extend(children.iter().copied());
        }
    }
    (ordered, restore)
}

fn enforce(
    groups: Vec<ObjectGroup>,
    max_objects: u64,
    max_linkset: u64,
) -> Result<Vec<ObjectGroup>, ObjectError> {
    let prims = groups.iter().map(|group| group.parts.len() as u64).sum();
    if prims > max_objects {
        refuse(
            REGION_OBJECTS,
            Kind::Hard,
            max_objects,
            prims,
            "region-store",
        );
        return Err(ObjectError::Objects {
            configured: max_objects,
            observed: prims,
        });
    }
    for group in &groups {
        let count = group.parts.len() as u64;
        if count > max_linkset {
            refuse(
                REGION_LINKSET_PRIMS,
                Kind::Hard,
                max_linkset,
                count,
                "region-store",
            );
            return Err(ObjectError::Linkset {
                configured: max_linkset,
                observed: count,
            });
        }
    }
    Ok(groups)
}

fn read_prim(row: &SqliteRow, shapes: &HashMap<String, &SqliteRow>) -> PrimRow {
    let uuid_text = text_or_empty(row, "UUID");
    let group_text = text_or_empty(row, "SceneGroupID");
    let region_text = text_or_empty(row, "RegionUUID");
    let link = optional_i32(row, "LinkNumber");
    let start = optional_text(row, "StartStr");
    let part = build_part(row, shapes);
    PrimRow {
        uuid_text,
        group_text,
        region_text,
        link,
        start,
        part,
    }
}

fn text_or_empty(row: &SqliteRow, name: &str) -> String {
    row.try_get::<Option<String>, _>(name)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn optional_text(row: &SqliteRow, name: &str) -> Option<String> {
    row.try_get::<Option<String>, _>(name).ok().flatten()
}

fn optional_i32(row: &SqliteRow, name: &str) -> Option<i32> {
    let value = optional_i64(row, name)?;
    i32::try_from(value).ok()
}

fn build_part(row: &SqliteRow, shapes: &HashMap<String, &SqliteRow>) -> Result<ObjectPart, ()> {
    let uuid = required_uuid(row, "UUID")?;
    let shape_row = shapes.get(&uuid.as_hyphenated().to_string()).copied();
    let (shape, media, mat_ovrd) = match shape_row {
        Some(shape) => {
            let built = build_shape(shape)?;
            let media = optional_text(shape, "Media");
            let mat_ovrd = optional_blob(shape, "MatOvrd")?;
            (built, media, mat_ovrd)
        }
        None => (default_box(), None, None),
    };
    let sound = required_uuid(row, "LoopedSound")?;
    let explicit_sit = optional_i32(row, "SitTargetActive");
    let offset = vec3(row, "PositionX", "PositionY", "PositionZ")?;
    let sit_target_position = vec3(
        row,
        "SitTargetOffsetX",
        "SitTargetOffsetY",
        "SitTargetOffsetZ",
    )?;
    let sit_target_rotation = quat(
        row,
        "SitTargetOrientX",
        "SitTargetOrientY",
        "SitTargetOrientZ",
        "SitTargetOrientW",
    )?;
    let pseudo = optional_i32(row, "pseudocrc").filter(|value| *value != 0);
    let mut flags =
        required_u32(row, "ObjectFlags")? & !(FLAG_TOUCH | FLAG_MONEY | FLAG_ALLOW_DROP);
    if flag_i16(row, "VolumeDetect")? {
        flags |= FLAG_VOLUME_DETECT;
    }
    Ok(ObjectPart {
        uuid,
        link_num: 0,
        local_id: 0,
        parent_id: 0,
        creation_date: required_i32(row, "CreationDate")?,
        name: optional_text(row, "Name").unwrap_or_default(),
        text: required_text(row, "Text")?,
        color: color(row)?,
        description: required_text(row, "Description")?,
        sit_name: required_text(row, "SitName")?,
        touch_name: required_text(row, "TouchName")?,
        flags,
        creator_id: required_text(row, "CreatorID")?,
        owner_id: required_uuid(row, "OwnerID")?,
        group_id: required_uuid(row, "GroupID")?,
        last_owner_id: required_uuid(row, "LastOwnerID")?,
        rezzer_id: match optional_text(row, "RezzerID") {
            Some(text) => parse_uuid(&text).ok_or(())?,
            None => Uuid::nil(),
        },
        owner_mask: required_u32(row, "OwnerMask")?,
        next_owner_mask: required_u32(row, "NextOwnerMask")?,
        group_mask: required_u32(row, "GroupMask")?,
        everyone_mask: required_u32(row, "EveryoneMask")?,
        base_mask: required_u32(row, "BaseMask")?,
        offset,
        group_position: vec3(row, "GroupPositionX", "GroupPositionY", "GroupPositionZ")?,
        velocity: vec3(row, "VelocityX", "VelocityY", "VelocityZ")?,
        angular_velocity: vec3(
            row,
            "AngularVelocityX",
            "AngularVelocityY",
            "AngularVelocityZ",
        )?,
        acceleration: vec3(row, "AccelerationX", "AccelerationY", "AccelerationZ")?,
        rotation: loaded_rotation(quat(
            row,
            "RotationX",
            "RotationY",
            "RotationZ",
            "RotationW",
        )?),
        sit_target_position,
        sit_target_rotation,
        sit_target_active: sit_active(explicit_sit, sit_target_position, sit_target_rotation),
        stand_offset: vec3(row, "standtargetx", "standtargety", "standtargetz")?,
        sit_active_range: required_f32(row, "sitactrange")?,
        click_action: required_u8(row, "ClickAction")?,
        pay_price: [
            required_i32(row, "PayPrice")?,
            required_i32(row, "PayButton1")?,
            required_i32(row, "PayButton2")?,
            required_i32(row, "PayButton3")?,
            required_i32(row, "PayButton4")?,
        ],
        sound,
        sound_gain: required_f32(row, "LoopedSoundGain")?,
        sound_flags: u8::from(!sound.is_nil()),
        texture_animation: base64_cell(row, "TextureAnimation")?,
        particles: base64_cell(row, "ParticleSystem")?,
        camera_eye: vec3(
            row,
            "CameraEyeOffsetX",
            "CameraEyeOffsetY",
            "CameraEyeOffsetZ",
        )?,
        camera_at: vec3(row, "CameraAtOffsetX", "CameraAtOffsetY", "CameraAtOffsetZ")?,
        force_mouselook: flag_i16(row, "ForceMouselook")?,
        script_access_pin: required_i32(row, "ScriptAccessPin")?,
        allowed_drop: flag_i16(row, "AllowedDrop")?,
        die_at_edge: flag_i16(row, "DieAtEdge")?,
        sale_price: required_i32(row, "SalePrice")?,
        sale_type: required_u8(row, "SaleType")?,
        material: required_u8(row, "Material")?,
        collision_sound: required_uuid(row, "CollisionSound")?,
        collision_sound_volume: required_f32(row, "CollisionSoundVolume")?,
        volume_detect: (flags & FLAG_VOLUME_DETECT) != 0,
        media_url: optional_text(row, "MediaURL").unwrap_or_default(),
        attached_pos: vec3(row, "AttachedPosX", "AttachedPosY", "AttachedPosZ")?,
        physics_shape_type: required_u8(row, "PhysicsShapeType")?,
        density: required_f32(row, "Density")?,
        gravity_modifier: required_f32(row, "GravityModifier")?,
        friction: required_f32(row, "Friction")?,
        restitution: required_f32(row, "Restitution")?,
        pass_collisions: flag_bool(row, "PassCollisions")?,
        pass_touches: flag_bool(row, "PassTouches")?,
        rotation_axis_locks: required_u8(row, "RotationAxisLocks")?,
        pseudo_crc: pseudo,
        shape,
        raw: ObjectRaw {
            media,
            mat_ovrd,
            dyn_attrs: optional_text(row, "DynAttrs"),
            keyframe_motion: optional_blob(row, "KeyframeMotion")?,
            vehicle: optional_text(row, "Vehicle"),
            phys_inertia: optional_text(row, "PhysInertia"),
            sopanims: optional_blob(row, "sopanims")?,
            linkset_data: optional_text(row, "linksetdata"),
        },
    })
}

fn build_shape(row: &SqliteRow) -> Result<ObjectShape, ()> {
    Ok(ObjectShape {
        default_box: false,
        scale: vec3(row, "ScaleX", "ScaleY", "ScaleZ")?,
        pcode: required_u8(row, "PCode")?,
        path_begin: required_u16(row, "PathBegin")?,
        path_end: required_u16(row, "PathEnd")?,
        path_scale_x: required_u8(row, "PathScaleX")?,
        path_scale_y: required_u8(row, "PathScaleY")?,
        path_shear_x: required_u8(row, "PathShearX")?,
        path_shear_y: required_u8(row, "PathShearY")?,
        path_skew: required_i8(row, "PathSkew")?,
        path_curve: required_u8(row, "PathCurve")?,
        path_radius_offset: required_i8(row, "PathRadiusOffset")?,
        path_revolutions: required_u8(row, "PathRevolutions")?,
        path_taper_x: required_i8(row, "PathTaperX")?,
        path_taper_y: required_i8(row, "PathTaperY")?,
        path_twist: required_i8(row, "PathTwist")?,
        path_twist_begin: required_i8(row, "PathTwistBegin")?,
        profile_begin: required_u16(row, "ProfileBegin")?,
        profile_end: required_u16(row, "ProfileEnd")?,
        profile_curve: profile_curve(required_u8(row, "ProfileCurve")?),
        profile_hollow: required_u16(row, "ProfileHollow")?,
        state: required_u8(row, "State")?,
        last_attach_point: required_u8(row, "LastAttachPoint")?,
        texture: required_blob(row, "Texture")?,
        extra_params: required_blob(row, "ExtraParams")?,
    })
}

fn default_box() -> ObjectShape {
    ObjectShape {
        default_box: true,
        scale: [0.5, 0.5, 0.5],
        pcode: 9,
        path_begin: 0,
        path_end: 0,
        path_scale_x: 100,
        path_scale_y: 100,
        path_shear_x: 0,
        path_shear_y: 0,
        path_skew: 0,
        path_curve: 16,
        path_radius_offset: 0,
        path_revolutions: 0,
        path_taper_x: 0,
        path_taper_y: 0,
        path_twist: 0,
        path_twist_begin: 0,
        profile_begin: 0,
        profile_end: 0,
        profile_curve: 1,
        profile_hollow: 0,
        state: 0,
        last_attach_point: 0,
        texture: DEFAULT_BOX_TEXTURE.to_vec(),
        extra_params: vec![0],
    }
}

fn profile_curve(value: u8) -> u8 {
    let hollow = match value & 0xf0 {
        hollow @ (0 | 16 | 32 | 48) => hollow,
        _ => 0,
    };
    let profile = value & 0x0f;
    let profile = if profile <= 5 { profile } else { 1 };
    hollow | profile
}

fn sit_active(explicit: Option<i32>, offset: [f32; 3], rotation: [f32; 4]) -> bool {
    if let Some(value) = explicit {
        return value != 0;
    }
    let offset_set = offset.iter().any(|value| *value != 0.0);
    let rotation_set = rotation[0] != 0.0 || rotation[1] != 0.0 || rotation[2] != 0.0;
    offset_set || rotation_set
}

fn color(row: &SqliteRow) -> Result<u32, ()> {
    let alpha = required_i32(row, "ColorA")? as u32;
    let red = required_i32(row, "ColorR")? as u32;
    let green = required_i32(row, "ColorG")? as u32;
    let blue = required_i32(row, "ColorB")? as u32;
    Ok((alpha << 24) | (red << 16) | (green << 8) | blue)
}

fn vec3(row: &SqliteRow, x: &str, y: &str, z: &str) -> Result<[f32; 3], ()> {
    Ok([
        required_f32(row, x)?,
        required_f32(row, y)?,
        required_f32(row, z)?,
    ])
}

fn quat(row: &SqliteRow, x: &str, y: &str, z: &str, w: &str) -> Result<[f32; 4], ()> {
    Ok([
        required_f32(row, x)?,
        required_f32(row, y)?,
        required_f32(row, z)?,
        required_f32(row, w)?,
    ])
}

/// A stored rotation whose squared length is at most `1e-6` loads as identity.
/// Any longer stored rotation is scaled to length 1.
fn loaded_rotation(value: [f32; 4]) -> [f32; 4] {
    let mag = value
        .iter()
        .map(|component| component * component)
        .sum::<f32>();
    if mag > 1e-6 {
        let scale = mag.sqrt().recip();
        value.map(|component| component * scale)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

fn required_text(row: &SqliteRow, name: &str) -> Result<String, ()> {
    optional_text(row, name).ok_or(())
}

fn required_uuid(row: &SqliteRow, name: &str) -> Result<Uuid, ()> {
    parse_uuid(&required_text(row, name)?).ok_or(())
}

fn parse_uuid(text: &str) -> Option<Uuid> {
    Uuid::parse_str(text.trim()).ok()
}

fn required_f32(row: &SqliteRow, name: &str) -> Result<f32, ()> {
    let value = optional_f64(row, name).ok_or(())?;
    if !value.is_finite() {
        return Err(());
    }
    let narrow = value as f32;
    if !narrow.is_finite() {
        return Err(());
    }
    Ok(narrow)
}

fn required_i32(row: &SqliteRow, name: &str) -> Result<i32, ()> {
    let value = optional_i64(row, name).ok_or(())?;
    i32::try_from(value).map_err(|_| ())
}

fn required_u32(row: &SqliteRow, name: &str) -> Result<u32, ()> {
    let value = optional_i64(row, name).ok_or(())?;
    u32::try_from(value).map_err(|_| ())
}

fn required_u16(row: &SqliteRow, name: &str) -> Result<u16, ()> {
    let value = optional_i64(row, name).ok_or(())?;
    u16::try_from(value).map_err(|_| ())
}

fn required_u8(row: &SqliteRow, name: &str) -> Result<u8, ()> {
    let value = optional_i64(row, name).ok_or(())?;
    u8::try_from(value).map_err(|_| ())
}

fn required_i8(row: &SqliteRow, name: &str) -> Result<i8, ()> {
    let value = optional_i64(row, name).ok_or(())?;
    i8::try_from(value).map_err(|_| ())
}

fn flag_i16(row: &SqliteRow, name: &str) -> Result<bool, ()> {
    let value = optional_i64(row, name).ok_or(())?;
    let narrow = i16::try_from(value).map_err(|_| ())?;
    Ok(narrow != 0)
}

fn flag_bool(row: &SqliteRow, name: &str) -> Result<bool, ()> {
    if let Ok(Some(value)) = row.try_get::<Option<bool>, _>(name) {
        return Ok(value);
    }
    Ok(optional_i64(row, name).ok_or(())? != 0)
}

fn optional_f64(row: &SqliteRow, name: &str) -> Option<f64> {
    if let Ok(Some(value)) = row.try_get::<Option<f64>, _>(name) {
        return Some(value);
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(name) {
        return value.map(|number| number as f64);
    }
    if let Ok(Some(text)) = row.try_get::<Option<String>, _>(name) {
        return text.trim().parse::<f64>().ok();
    }
    if let Ok(None) = row.try_get::<Option<String>, _>(name) {
        return None;
    }
    None
}

fn optional_i64(row: &SqliteRow, name: &str) -> Option<i64> {
    let value = optional_f64(row, name)?;
    if value.fract() != 0.0 || !(i64::MIN as f64..=i64::MAX as f64).contains(&value) {
        return None;
    }
    Some(value as i64)
}

fn optional_blob(row: &SqliteRow, name: &str) -> Result<Option<Vec<u8>>, ()> {
    row.try_get::<Option<Vec<u8>>, _>(name).map_err(|_| ())
}

fn required_blob(row: &SqliteRow, name: &str) -> Result<Vec<u8>, ()> {
    optional_blob(row, name)?.ok_or(())
}

fn base64_cell(row: &SqliteRow, name: &str) -> Result<Vec<u8>, ()> {
    match optional_text(row, name) {
        None => Ok(Vec::new()),
        Some(text) => decode_base64(&text).ok_or(()),
    }
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut index = 0;
    while index < bytes.len() {
        let chunk = &bytes[index..index + 4];
        index += 4;
        if chunk[0] == b'=' || chunk[1] == b'=' || (chunk[2] == b'=' && chunk[3] != b'=') {
            return None;
        }
        let mut values = [0u8; 4];
        let mut pad = 0u8;
        for (slot, byte) in chunk.iter().copied().enumerate() {
            values[slot] = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => {
                    pad += 1;
                    0
                }
                _ => return None,
            };
        }
        if pad > 2 {
            return None;
        }
        let triple = ((values[0] as u32) << 18)
            | ((values[1] as u32) << 12)
            | ((values[2] as u32) << 6)
            | (values[3] as u32);
        out.push((triple >> 16) as u8);
        if pad < 2 {
            out.push((triple >> 8) as u8);
        }
        if pad < 1 {
            out.push(triple as u8);
        }
    }
    Some(out)
}

const PRIM_SQL: &str = "SELECT UUID, RegionUUID, SceneGroupID, CreationDate, Name, Text, Description, \
SitName, TouchName, ColorR, ColorG, ColorB, ColorA, ObjectFlags, CreatorID, OwnerID, GroupID, \
LastOwnerID, RezzerID, OwnerMask, NextOwnerMask, GroupMask, EveryoneMask, BaseMask, PositionX, \
PositionY, PositionZ, GroupPositionX, GroupPositionY, GroupPositionZ, VelocityX, VelocityY, \
VelocityZ, AngularVelocityX, AngularVelocityY, AngularVelocityZ, AccelerationX, AccelerationY, \
AccelerationZ, RotationX, RotationY, RotationZ, RotationW, SitTargetOffsetX, SitTargetOffsetY, \
SitTargetOffsetZ, SitTargetOrientX, SitTargetOrientY, SitTargetOrientZ, SitTargetOrientW, \
SitTargetActive, standtargetx, standtargety, standtargetz, sitactrange, ClickAction, PayPrice, \
PayButton1, PayButton2, PayButton3, PayButton4, LoopedSound, LoopedSoundGain, TextureAnimation, \
ParticleSystem, CameraEyeOffsetX, CameraEyeOffsetY, CameraEyeOffsetZ, CameraAtOffsetX, \
CameraAtOffsetY, CameraAtOffsetZ, ForceMouselook, ScriptAccessPin, AllowedDrop, DieAtEdge, \
SalePrice, SaleType, Material, CollisionSound, CollisionSoundVolume, VolumeDetect, MediaURL, \
AttachedPosX, AttachedPosY, AttachedPosZ, DynAttrs, PhysicsShapeType, Density, GravityModifier, \
Friction, Restitution, KeyframeMotion, PassCollisions, PassTouches, RotationAxisLocks, Vehicle, \
PhysInertia, pseudocrc, sopanims, linksetdata, StartStr, LinkNumber FROM prims ORDER BY rowid";

const SHAPE_SQL: &str = "SELECT UUID, ScaleX, ScaleY, ScaleZ, PCode, PathBegin, PathEnd, PathScaleX, \
PathScaleY, PathShearX, PathShearY, PathSkew, PathCurve, PathRadiusOffset, PathRevolutions, \
PathTaperX, PathTaperY, PathTwist, PathTwistBegin, ProfileBegin, ProfileEnd, ProfileCurve, \
ProfileHollow, Texture, ExtraParams, State, Media, LastAttachPoint, MatOvrd FROM primshapes \
ORDER BY rowid";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::process::Command;

    use serde_json::{Map, Value};

    const REGION: &str = "11111111-1111-1111-1111-111111111111";

    fn fixture_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/regionstore")
    }

    fn committed_dump() -> Value {
        let text = std::fs::read_to_string(fixture_dir().join("objects.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn copied_store() -> String {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-regionstore-{}-{}",
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

    fn json_i64(value: &Value, path: &str) -> i64 {
        if let Some(number) = value.as_i64() {
            return number;
        }
        let number = value
            .as_f64()
            .unwrap_or_else(|| panic!("{path} is not a number"));
        assert!(
            number.fract() == 0.0 && (i64::MIN as f64..=i64::MAX as f64).contains(&number),
            "{path} is not an integer"
        );
        number as i64
    }

    fn take(obj: &mut Map<String, Value>, key: &str) -> Value {
        obj.remove(key).unwrap_or_else(|| panic!("missing {key}"))
    }

    fn expect_f32s(path: &str, got: &[f32], value: &Value) {
        let items = value.as_array().unwrap_or_else(|| panic!("{path}"));
        assert_eq!(items.len(), got.len(), "{path}");
        for (index, (item, number)) in items.iter().zip(got).enumerate() {
            let parsed = item.as_f64().unwrap_or_else(|| panic!("{path}[{index}]")) as f32;
            assert_eq!(
                parsed.to_bits(),
                number.to_bits(),
                "{path}[{index}] dump {item} load {number}"
            );
        }
    }

    fn expect_bytes(path: &str, got: &[u8], value: &Value) {
        let text = value.as_str().unwrap_or_else(|| panic!("{path}"));
        assert!(text.len().is_multiple_of(2), "{path}");
        let mut parsed = Vec::with_capacity(text.len() / 2);
        let mut index = 0;
        while index < text.len() {
            let pair = &text[index..index + 2];
            parsed.push(u8::from_str_radix(pair, 16).unwrap_or_else(|_| panic!("{path}")));
            index += 2;
        }
        assert_eq!(parsed, got, "{path}");
    }

    fn expect_opt_bytes(path: &str, got: &Option<Vec<u8>>, value: &Value) {
        match got {
            None => assert!(value.is_null(), "{path}"),
            Some(bytes) => expect_bytes(path, bytes, value),
        }
    }

    fn expect_opt_text(path: &str, got: &Option<String>, value: &Value) {
        match got {
            None => assert!(value.is_null(), "{path}"),
            Some(text) => assert_eq!(value.as_str(), Some(text.as_str()), "{path}"),
        }
    }

    fn expect_uuid(path: &str, got: Uuid, value: &Value) {
        let text = value.as_str().unwrap_or_else(|| panic!("{path}"));
        assert_eq!(got, Uuid::parse_str(text).unwrap(), "{path}");
    }

    fn expect_part(part: &ObjectPart, value: &Value) {
        let mut obj = value.as_object().unwrap().clone();
        let label = take(&mut obj, "name").as_str().unwrap().to_owned();
        assert_eq!(part.name, label);
        expect_uuid("uuid", part.uuid, &take(&mut obj, "uuid"));
        assert_eq!(
            i64::from(part.link_num),
            json_i64(&take(&mut obj, "link_num"), "link_num")
        );
        assert_eq!(
            u64::from(part.local_id),
            json_i64(&take(&mut obj, "local_id"), "local_id") as u64
        );
        assert_eq!(
            u64::from(part.parent_id),
            json_i64(&take(&mut obj, "parent_id"), "parent_id") as u64
        );
        assert_eq!(
            i64::from(part.creation_date),
            json_i64(&take(&mut obj, "creation_date"), "creation_date")
        );
        assert_eq!(part.text, take(&mut obj, "text").as_str().unwrap());
        assert_eq!(
            u64::from(part.color),
            json_i64(&take(&mut obj, "color"), "color") as u64
        );
        assert_eq!(
            part.description,
            take(&mut obj, "description").as_str().unwrap()
        );
        assert_eq!(part.sit_name, take(&mut obj, "sit_name").as_str().unwrap());
        assert_eq!(
            part.touch_name,
            take(&mut obj, "touch_name").as_str().unwrap()
        );
        assert_eq!(
            u64::from(part.flags),
            json_i64(&take(&mut obj, "flags"), "flags") as u64
        );
        assert_eq!(
            part.creator_id,
            take(&mut obj, "creator_id").as_str().unwrap()
        );
        expect_uuid("owner_id", part.owner_id, &take(&mut obj, "owner_id"));
        expect_uuid("group_id", part.group_id, &take(&mut obj, "group_id"));
        expect_uuid(
            "last_owner_id",
            part.last_owner_id,
            &take(&mut obj, "last_owner_id"),
        );
        expect_uuid("rezzer_id", part.rezzer_id, &take(&mut obj, "rezzer_id"));
        for (got, key) in [
            (part.owner_mask, "owner_mask"),
            (part.next_owner_mask, "next_owner_mask"),
            (part.group_mask, "group_mask"),
            (part.everyone_mask, "everyone_mask"),
            (part.base_mask, "base_mask"),
        ] {
            assert_eq!(
                u64::from(got),
                json_i64(&take(&mut obj, key), key) as u64,
                "{label} {key}"
            );
        }
        expect_f32s("offset", &part.offset, &take(&mut obj, "offset"));
        expect_f32s(
            "group_position",
            &part.group_position,
            &take(&mut obj, "group_position"),
        );
        expect_f32s("velocity", &part.velocity, &take(&mut obj, "velocity"));
        expect_f32s(
            "angular_velocity",
            &part.angular_velocity,
            &take(&mut obj, "angular_velocity"),
        );
        expect_f32s(
            "acceleration",
            &part.acceleration,
            &take(&mut obj, "acceleration"),
        );
        expect_f32s("rotation", &part.rotation, &take(&mut obj, "rotation"));
        expect_f32s(
            "sit_target_position",
            &part.sit_target_position,
            &take(&mut obj, "sit_target_position"),
        );
        expect_f32s(
            "sit_target_rotation",
            &part.sit_target_rotation,
            &take(&mut obj, "sit_target_rotation"),
        );
        assert_eq!(
            part.sit_target_active,
            take(&mut obj, "sit_target_active").as_bool().unwrap()
        );
        expect_f32s(
            "stand_offset",
            &part.stand_offset,
            &take(&mut obj, "stand_offset"),
        );
        let range = take(&mut obj, "sit_active_range").as_f64().unwrap() as f32;
        assert_eq!(
            range.to_bits(),
            part.sit_active_range.to_bits(),
            "sit_active_range"
        );
        assert_eq!(
            i64::from(part.click_action),
            json_i64(&take(&mut obj, "click_action"), "click_action")
        );
        let pay = take(&mut obj, "pay_price");
        let pay = pay.as_array().unwrap();
        assert_eq!(pay.len(), part.pay_price.len());
        for (index, (item, number)) in pay.iter().zip(part.pay_price).enumerate() {
            assert_eq!(
                json_i64(item, "pay_price"),
                i64::from(number),
                "pay_price[{index}]"
            );
        }
        expect_uuid("sound", part.sound, &take(&mut obj, "sound"));
        let gain = take(&mut obj, "sound_gain").as_f64().unwrap() as f32;
        assert_eq!(gain.to_bits(), part.sound_gain.to_bits(), "sound_gain");
        assert_eq!(
            i64::from(part.sound_flags),
            json_i64(&take(&mut obj, "sound_flags"), "sound_flags")
        );
        expect_bytes(
            "texture_animation",
            &part.texture_animation,
            &take(&mut obj, "texture_animation"),
        );
        expect_bytes("particles", &part.particles, &take(&mut obj, "particles"));
        expect_f32s(
            "camera_eye",
            &part.camera_eye,
            &take(&mut obj, "camera_eye"),
        );
        expect_f32s("camera_at", &part.camera_at, &take(&mut obj, "camera_at"));
        assert_eq!(
            part.force_mouselook,
            take(&mut obj, "force_mouselook").as_bool().unwrap()
        );
        assert_eq!(
            i64::from(part.script_access_pin),
            json_i64(&take(&mut obj, "script_access_pin"), "script_access_pin")
        );
        assert_eq!(
            part.allowed_drop,
            take(&mut obj, "allowed_drop").as_bool().unwrap()
        );
        assert_eq!(
            part.die_at_edge,
            take(&mut obj, "die_at_edge").as_bool().unwrap()
        );
        assert_eq!(
            i64::from(part.sale_price),
            json_i64(&take(&mut obj, "sale_price"), "sale_price")
        );
        assert_eq!(
            i64::from(part.sale_type),
            json_i64(&take(&mut obj, "sale_type"), "sale_type")
        );
        assert_eq!(
            i64::from(part.material),
            json_i64(&take(&mut obj, "material"), "material")
        );
        expect_uuid(
            "collision_sound",
            part.collision_sound,
            &take(&mut obj, "collision_sound"),
        );
        let volume = take(&mut obj, "collision_sound_volume").as_f64().unwrap() as f32;
        assert_eq!(volume.to_bits(), part.collision_sound_volume.to_bits());
        assert_eq!(
            part.volume_detect,
            take(&mut obj, "volume_detect").as_bool().unwrap()
        );
        assert_eq!(
            part.media_url,
            take(&mut obj, "media_url").as_str().unwrap()
        );
        expect_f32s(
            "attached_pos",
            &part.attached_pos,
            &take(&mut obj, "attached_pos"),
        );
        assert_eq!(
            i64::from(part.physics_shape_type),
            json_i64(&take(&mut obj, "physics_shape_type"), "physics_shape_type")
        );
        for (got, key) in [
            (part.density, "density"),
            (part.gravity_modifier, "gravity_modifier"),
            (part.friction, "friction"),
            (part.restitution, "restitution"),
        ] {
            let parsed = take(&mut obj, key).as_f64().unwrap() as f32;
            assert_eq!(parsed.to_bits(), got.to_bits(), "{label} {key}");
        }
        assert_eq!(
            part.pass_collisions,
            take(&mut obj, "pass_collisions").as_bool().unwrap()
        );
        assert_eq!(
            part.pass_touches,
            take(&mut obj, "pass_touches").as_bool().unwrap()
        );
        assert_eq!(
            i64::from(part.rotation_axis_locks),
            json_i64(
                &take(&mut obj, "rotation_axis_locks"),
                "rotation_axis_locks"
            )
        );
        match &part.pseudo_crc {
            None => assert!(take(&mut obj, "pseudo_crc").is_null(), "{label} pseudo_crc"),
            Some(crc) => assert_eq!(
                i64::from(*crc),
                json_i64(&take(&mut obj, "pseudo_crc"), "pseudo_crc")
            ),
        }
        expect_shape(&part.shape, &take(&mut obj, "shape"));
        expect_raw(&part.raw, &take(&mut obj, "raw"));
        assert!(
            obj.is_empty(),
            "{label} uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    fn expect_shape(shape: &ObjectShape, value: &Value) {
        let mut obj = value.as_object().unwrap().clone();
        assert_eq!(
            shape.default_box,
            take(&mut obj, "default_box").as_bool().unwrap()
        );
        expect_f32s("scale", &shape.scale, &take(&mut obj, "scale"));
        for (got, key) in [
            (u64::from(shape.pcode), "pcode"),
            (u64::from(shape.path_begin), "path_begin"),
            (u64::from(shape.path_end), "path_end"),
            (u64::from(shape.path_scale_x), "path_scale_x"),
            (u64::from(shape.path_scale_y), "path_scale_y"),
            (u64::from(shape.path_shear_x), "path_shear_x"),
            (u64::from(shape.path_shear_y), "path_shear_y"),
            (u64::from(shape.path_curve), "path_curve"),
            (u64::from(shape.path_revolutions), "path_revolutions"),
            (u64::from(shape.profile_begin), "profile_begin"),
            (u64::from(shape.profile_end), "profile_end"),
            (u64::from(shape.profile_curve), "profile_curve"),
            (u64::from(shape.profile_hollow), "profile_hollow"),
            (u64::from(shape.state), "state"),
            (u64::from(shape.last_attach_point), "last_attach_point"),
        ] {
            assert_eq!(got, json_i64(&take(&mut obj, key), key) as u64, "{key}");
        }
        for (got, key) in [
            (i64::from(shape.path_skew), "path_skew"),
            (i64::from(shape.path_radius_offset), "path_radius_offset"),
            (i64::from(shape.path_taper_x), "path_taper_x"),
            (i64::from(shape.path_taper_y), "path_taper_y"),
            (i64::from(shape.path_twist), "path_twist"),
            (i64::from(shape.path_twist_begin), "path_twist_begin"),
        ] {
            assert_eq!(got, json_i64(&take(&mut obj, key), key), "{key}");
        }
        expect_bytes("texture", &shape.texture, &take(&mut obj, "texture"));
        expect_bytes(
            "extra_params",
            &shape.extra_params,
            &take(&mut obj, "extra_params"),
        );
        assert!(
            obj.is_empty(),
            "shape uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    fn expect_raw(raw: &ObjectRaw, value: &Value) {
        let mut obj = value.as_object().unwrap().clone();
        expect_opt_text("media", &raw.media, &take(&mut obj, "media"));
        expect_opt_bytes("mat_ovrd", &raw.mat_ovrd, &take(&mut obj, "mat_ovrd"));
        expect_opt_text("dyn_attrs", &raw.dyn_attrs, &take(&mut obj, "dyn_attrs"));
        expect_opt_bytes(
            "keyframe_motion",
            &raw.keyframe_motion,
            &take(&mut obj, "keyframe_motion"),
        );
        expect_opt_text("vehicle", &raw.vehicle, &take(&mut obj, "vehicle"));
        expect_opt_text(
            "phys_inertia",
            &raw.phys_inertia,
            &take(&mut obj, "phys_inertia"),
        );
        expect_opt_bytes("sopanims", &raw.sopanims, &take(&mut obj, "sopanims"));
        expect_opt_text(
            "linkset_data",
            &raw.linkset_data,
            &take(&mut obj, "linkset_data"),
        );
        assert!(
            obj.is_empty(),
            "raw uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn regionstore_load_matches_the_committed_dump() {
        let dump = committed_dump();
        assert_eq!(dump["pin"], "1052cbae82ae7dc3b8abfcffe33e316498eeaad9");
        assert_eq!(dump["region"], REGION);
        let groups = open_objects(&copied_store(), Uuid::parse_str(REGION).unwrap())
            .await
            .unwrap();
        let dumped = dump["groups"].as_array().unwrap();
        assert_eq!(groups.len(), dumped.len());
        assert_eq!(groups.len(), 4);
        let mut ids = BTreeSet::new();
        for (group, dumped_group) in groups.iter().zip(dumped) {
            expect_opt_text(
                "rez_parameter",
                &group.rez_parameter,
                &dumped_group["rez_parameter"],
            );
            let parts = dumped_group["parts"].as_array().unwrap();
            assert_eq!(group.parts.len(), parts.len());
            for (part, dumped_part) in group.parts.iter().zip(parts) {
                expect_part(part, dumped_part);
                ids.insert(part.uuid);
            }
        }
        assert!(!ids.contains(&Uuid::parse_str("ffffffff-ffff-ffff-ffff-fffffffffff1").unwrap()));
        assert_eq!(groups[0].parts[0].name, "showcase");
        assert_eq!(groups[0].parts[0].link_num, 0);
        assert_eq!(groups[0].parts[0].flags, 0x4401);
        assert_eq!(groups[0].parts[0].pay_price, [10, 20, 30, 40, 50]);
        assert!(!groups[0].parts[0].sit_target_active);
        assert_eq!(groups[0].parts[0].shape.extra_params, vec![0x00, 0xaa]);
        assert_eq!(groups[0].rez_parameter.as_deref(), Some("rez-arg"));
        assert_eq!(
            groups[1]
                .parts
                .iter()
                .map(|part| (part.name.as_str(), part.link_num))
                .collect::<Vec<_>>(),
            vec![("link-restore", 1), ("child-two", 2), ("child-three", 3)]
        );
        assert!(groups[1].parts[1].sit_target_active);
        assert_eq!(groups[1].parts[1].sit_target_position, [0.0, 0.0, 0.0]);
        assert_eq!(
            groups[2]
                .parts
                .iter()
                .map(|part| (part.name.as_str(), part.link_num))
                .collect::<Vec<_>>(),
            vec![("link-table", 1), ("child-first", 2), ("child-second", 3)]
        );
        assert_eq!(groups[3].parts[0].name, "no-shape");
        assert!(groups[3].parts[0].shape.default_box);
        assert_eq!(groups[3].parts[0].shape.extra_params, vec![0]);
        assert_eq!(groups[3].parts[0].shape.texture, DEFAULT_BOX_TEXTURE);
        assert!(groups[3].parts[0].sit_target_active);
        assert_eq!(groups[3].parts[0].pay_price, [0, 0, 0, 0, 0]);
        assert_eq!(groups[3].parts[0].pseudo_crc, None);
        assert_eq!(groups[3].parts[0].media_url, "");
    }

    #[tokio::test]
    async fn regionstore_limits_refuse_a_smaller_cap() {
        let connection = copied_store();
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let region = Uuid::parse_str(REGION).unwrap();
        let objects = load_objects_limited(&pool, region, 1, MAX_LINKSET_PRIMS).await;
        match objects {
            Err(ObjectError::Objects {
                configured: 1,
                observed: 8,
            }) => {}
            other => panic!("object cap: {other:?}"),
        }
        let linkset = load_objects_limited(&pool, region, MAX_REGION_OBJECTS, 2).await;
        match linkset {
            Err(ObjectError::Linkset {
                configured: 2,
                observed: 3,
            }) => {}
            other => panic!("linkset cap: {other:?}"),
        }
    }

    #[tokio::test]
    async fn regionstore_missing_file_is_empty() {
        let path = std::env::temp_dir().join(format!(
            "ferroan-regionstore-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let connection = format!("Data Source={};Version=3", path.display());
        let groups = open_objects(&connection, Uuid::parse_str(REGION).unwrap())
            .await
            .unwrap();
        assert!(groups.is_empty());
    }

    #[tokio::test]
    async fn regionstore_oracle_reload_matches_the_committed_dump() {
        if std::env::var("FERROAN_REGIONSTORE").ok().as_deref() != Some("1") {
            eprintln!("regionstore oracle reload skipped");
            return;
        }
        let dll = std::env::var("FERROAN_REGIONSTORE_ORACLE").unwrap();
        let dump = committed_dump();
        let pin = dump["pin"].as_str().unwrap();
        let connection = copied_store();
        let path = ferroan_db::sqlite_filename(&connection).unwrap();
        let output = Command::new("dotnet")
            .arg(&dll)
            .args(["load", &path.display().to_string(), REGION, pin])
            .env("MSBUILDDISABLENODEREUSE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let fresh: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(fresh, dump);
    }
}
