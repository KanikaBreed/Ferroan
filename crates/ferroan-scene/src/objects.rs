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
// SceneObjectPart, PrimitiveBaseShape, TaskInventoryItem and DBGuid at
// OpenSim-Tranquillity `dev-build` @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
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
//! Texture entries and extra params stay the stored bytes. Dynattrs, keyframe
//! motion, vehicle, inertia, animations, linkset data, media and the materials
//! override are parsed beside those stored cells. Touch, money and
//! allow-inventory-drop are cleared from the stored flags. A non-zero
//! volume-detect cell sets bit `0x4000`, which viewers read as volume detect.
//! A null media URL loads as an empty string. A stored pseudo-CRC of 0 is
//! absent: the part's clock value is not kept. An animation sequence number is
//! a clock and is absent too. Q65 sends both.
//!
//! Each loaded prim keeps the task-inventory rows stored for its id, in the
//! order they were stored. A repeated item id keeps the first. A null
//! description is an empty string. A creator id with no semicolon is that id,
//! or the zero id when it does not parse. A creator id with a semicolon is the
//! id, the endpoint and the name, and a missing name is `Unknown User`. An
//! experience id that is null, unparsable or shorter than 16 bytes is the zero
//! id. A row that cannot be built is left out; the prim and its other items
//! stay (TB60, D44). Items stay data: nothing is sent and no script is started.
//!
//! PostgreSQL returns each group by group id, the root first and then the
//! children by stored link number. Those stored numbers are not applied: the
//! first part is the root, a single prim stays 0, and each child takes the
//! next count. A root whose id differs from the group id loads as the group
//! id. The shape bytes stay. The raw cells for that part are empty and its
//! default-box flag is set, because those cells are stored under the original
//! id. The item parent id is the loaded prim id. An item stored under the
//! original id of a rewritten root still loads (TB68, D51). A null required
//! number or text leaves that prim out (TB65, D49). An inventory row that
//! cannot be built is left out (TB66, D50). A dynamic-attribute document whose
//! top value is an integer leaves that prim out (TB67, D48). Keyframe motion,
//! the attached position, a volume-detect column and the last attach point are
//! not stored, so those fields stay at the part defaults.

use std::collections::{HashMap, HashSet};

use ferroan_db::{Backend, Pool};
use ferroan_diag::{Kind, OBJECT_TASK_ITEMS, REGION_LINKSET_PRIMS, REGION_OBJECTS, refuse};
use futures_util::TryStreamExt;
use sqlx::Row;
use sqlx::mysql::MySqlRow;
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use crate::error::ObjectError;
use crate::object_properties::{DESCRIPTION_LIMIT, SIT_LIMIT, TOUCH_LIMIT, cut_stored_text};
use crate::payloads::{
    DynAttrs, KeyframeMotion, LinksetData, ObjectAnimation, PhysicsInertia, Vehicle,
    parse_animations, parse_dyn_attrs, parse_inertia, parse_keyframe, parse_linkset,
    parse_materials, parse_media, parse_vehicle,
};

/// Prim count a region load admits. Scene startup has no such cap (D42).
pub const MAX_REGION_OBJECTS: u64 = 100_000;
/// Prim count one linkset admits. A default region allows any size (D42).
pub const MAX_LINKSET_PRIMS: u64 = 10_000;
/// Bytes of one object-XML document. The same ceiling as `http.xml-chars`.
pub const MAX_OBJECT_XML_BYTES: u64 = 50_000_000;
/// Task-inventory rows one part admits from object XML. The pin has no cap.
pub const MAX_TASK_ITEMS: u64 = 10_000;

/// Cleared from the stored flags when the part is loaded.
const FLAG_TOUCH: u32 = 0x0000_0080;
const FLAG_MONEY: u32 = 0x0000_0200;
const FLAG_ALLOW_DROP: u32 = 0x0001_0000;
/// Viewers read this bit as volume detect. A non-zero volume-detect cell sets it.
const FLAG_VOLUME_DETECT: u32 = 0x0000_4000;

/// Plywood face bytes of a prim that has no shape row, captured at pin `1052cbae82`.
/// The same bytes as `PrimitiveBaseShape.DEFAULT_TEXTURE` at the current pin.
pub(crate) const DEFAULT_BOX_TEXTURE: &[u8] = &[
    0x89, 0x55, 0x67, 0x47, 0x24, 0xcb, 0x43, 0xed, 0x92, 0x0b, 0x47, 0xca, 0xed, 0x15, 0x46, 0x5f,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectGroup {
    /// Inventory item that supplied this group. Carried separately in crossing XML.
    pub from_item_id: Uuid,
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
    /// CreatorData text from object XML, unchanged. `None` on a store load, which
    /// still writes `creator_id` through the database spelling.
    pub xml_creator_data: Option<String>,
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
    pub sound_gain: f64,
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
    pub dyn_attrs: Option<DynAttrs>,
    pub keyframe: Option<KeyframeMotion>,
    pub vehicle: Option<Vehicle>,
    pub phys_inertia: Option<PhysicsInertia>,
    pub animations: Option<Vec<ObjectAnimation>>,
    pub linkset: Option<LinksetData>,
    /// Task inventory for this prim, in stored order. Empty when the prim has none.
    pub items: Vec<InventoryItem>,
    /// `None` on a store load. The writer then spells 0 with no items and 1 with
    /// any. A read records the serial after the inventory is assigned and, when
    /// that inventory is not empty, after the part id is copied onto each item.
    pub inventory_serial: Option<u32>,
    /// Stored sit flag. True when the column is absent or null.
    pub allow_unsit: bool,
    /// Stored sit flag. False when the column is absent or null.
    pub scripted_sit_only: bool,
}

/// One task-inventory row as the load leaves it. Nothing here is sent.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryItem {
    pub item_id: Uuid,
    pub parent_part_id: Uuid,
    pub asset_id: Uuid,
    pub parent_id: Uuid,
    pub inv_type: i32,
    pub asset_type: i32,
    pub name: String,
    pub description: String,
    pub creation_date: u32,
    pub creator_id: String,
    /// CreatorData text from object XML, unchanged. `None` on a store load.
    pub xml_creator_data: Option<String>,
    pub owner_id: Uuid,
    pub last_owner_id: Uuid,
    pub group_id: Uuid,
    pub next_permissions: u32,
    pub current_permissions: u32,
    pub base_permissions: u32,
    pub everyone_permissions: u32,
    pub group_permissions: u32,
    pub flags: u32,
    pub experience_id: Uuid,
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
    pub media_entries: Option<Vec<crate::payloads::MediaFace>>,
    pub materials: Option<crate::payloads::RenderMaterials>,
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
    link: Option<i32>,
    start: Option<String>,
    part: Option<ObjectPart>,
}

/// Which region-store driver `StorageProvider` names. The connection string is
/// not enough: a MySQL string can be `Data Source=…;Database=…`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectStoreKind {
    Sqlite,
    Mysql,
    Postgres,
}

/// Open the region store and load one region's prims. A missing SQLite file is an empty list.
/// `kind` is the configured `StorageProvider`, not a guess from the connection string.
pub async fn open_objects(
    connection: &str,
    region: Uuid,
    kind: ObjectStoreKind,
) -> Result<Vec<ObjectGroup>, ObjectError> {
    let backend = match kind {
        ObjectStoreKind::Sqlite => Backend::Sqlite,
        ObjectStoreKind::Mysql => Backend::Mysql,
        ObjectStoreKind::Postgres => Backend::Postgres,
    };
    if kind == ObjectStoreKind::Sqlite {
        let path = ferroan_db::sqlite_filename(connection)
            .map_err(|error| ObjectError::Store(error.to_string()))?;
        if !path.exists() {
            return Ok(Vec::new());
        }
    }
    let pool = Pool::open(backend, connection, &["RegionStore"])
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    let result = load_objects(&pool, region).await;
    pool.close().await;
    result
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
    if pool.postgres().is_some() {
        return load_postgres(pool, region, max_objects, max_linkset).await;
    }
    if pool.mysql().is_some() {
        return load_mysql(pool, region, max_objects, max_linkset).await;
    }
    let Some(sqlite) = pool.sqlite() else {
        return Err(ObjectError::Store(
            "connection is not SQLite, MySQL or PostgreSQL".to_owned(),
        ));
    };
    let region_text = region.as_hyphenated().to_string();
    let mut items = stream_items(sqlite, &region_text, max_objects).await?;
    let mut rows = Vec::new();
    let mut kept = 0u64;
    let mut prims = sqlx::query(PRIM_SQL).bind(&region_text).fetch(sqlite);
    while let Some(row) = prims
        .try_next()
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?
    {
        let Some(prim) = read_prim(&row, &mut items) else {
            continue;
        };
        // A row that fails to build still occupies a link number. Only a built part counts.
        if prim.part.is_some() {
            kept += 1;
            if kept > max_objects {
                refuse(
                    REGION_OBJECTS,
                    Kind::Hard,
                    max_objects,
                    kept,
                    "region-store",
                );
                return Err(ObjectError::Objects {
                    configured: max_objects,
                    observed: kept,
                });
            }
        }
        rows.push(prim);
    }
    drop(prims);
    let groups = assemble(rows);
    enforce(groups, max_objects, max_linkset)
}

async fn stream_items(
    sqlite: &sqlx::SqlitePool,
    region_text: &str,
    max_objects: u64,
) -> Result<HashMap<Uuid, Vec<InventoryItem>>, ObjectError> {
    let mut by_prim: HashMap<Uuid, Vec<InventoryItem>> = HashMap::new();
    let mut seen = HashSet::new();
    let mut counts: HashMap<Uuid, u64> = HashMap::new();
    let mut region_items = 0u64;
    let mut rows = sqlx::query(ITEM_SQL).bind(region_text).fetch(sqlite);
    while let Some(row) = rows
        .try_next()
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?
    {
        let Some(item) = build_item(&row) else {
            let label = optional_text(&row, "itemID").unwrap_or_else(|| "unknown".to_owned());
            log::warn!("inventory item {label} could not be built and was left out");
            continue;
        };
        if !seen.insert(item.item_id) {
            continue;
        }
        note_region_items(&mut region_items, max_objects)?;
        note_task_item(&mut counts, item.parent_part_id)?;
        by_prim.entry(item.parent_part_id).or_default().push(item);
    }
    Ok(by_prim)
}

fn note_task_item(counts: &mut HashMap<Uuid, u64>, prim: Uuid) -> Result<(), ObjectError> {
    let count = counts.entry(prim).or_insert(0);
    *count += 1;
    if *count > MAX_TASK_ITEMS {
        refuse(
            OBJECT_TASK_ITEMS,
            Kind::Hard,
            MAX_TASK_ITEMS,
            *count,
            "region-store",
        );
        return Err(ObjectError::TaskItems {
            configured: MAX_TASK_ITEMS,
            observed: *count,
        });
    }
    Ok(())
}

/// Kept item rows for the whole region. The product is `region.objects` times the per-part cap.
fn note_region_items(count: &mut u64, max_objects: u64) -> Result<(), ObjectError> {
    *count += 1;
    let configured = max_objects.saturating_mul(MAX_TASK_ITEMS);
    if *count > configured {
        refuse(
            OBJECT_TASK_ITEMS,
            Kind::Hard,
            configured,
            *count,
            "region-store",
        );
        return Err(ObjectError::TaskItems {
            configured,
            observed: *count,
        });
    }
    Ok(())
}

fn note_streamed_item(
    region_items: &mut u64,
    part_items: &mut u64,
    max_objects: u64,
) -> Result<(), ObjectError> {
    note_region_items(region_items, max_objects)?;
    *part_items += 1;
    if *part_items > MAX_TASK_ITEMS {
        refuse(
            OBJECT_TASK_ITEMS,
            Kind::Hard,
            MAX_TASK_ITEMS,
            *part_items,
            "region-store",
        );
        return Err(ObjectError::TaskItems {
            configured: MAX_TASK_ITEMS,
            observed: *part_items,
        });
    }
    Ok(())
}

fn assemble(mut rows: Vec<PrimRow>) -> Vec<ObjectGroup> {
    let (order, restore) = child_order(&rows);
    let mut groups = Vec::new();
    let mut index: HashMap<Uuid, usize> = HashMap::new();
    for row in &mut rows {
        if row.uuid_text != row.group_text {
            continue;
        }
        let Some(part) = row.part.take() else {
            log::warn!("prim {} could not be built and was skipped", row.uuid_text);
            continue;
        };
        if index.contains_key(&part.uuid) {
            log::warn!("prim {} could not be built and was skipped", row.uuid_text);
            continue;
        }
        index.insert(part.uuid, groups.len());
        groups.push(ObjectGroup {
            from_item_id: Uuid::nil(),
            rez_parameter: row.start.clone(),
            parts: vec![part],
        });
    }

    for index_in_rows in order {
        let row = &mut rows[index_in_rows];
        let Some(mut part) = row.part.take() else {
            log::warn!("prim {} could not be built and was skipped", row.uuid_text);
            continue;
        };
        let Some(group_id) = parse_uuid(&row.group_text) else {
            log::warn!("prim {} has no root and was skipped", row.uuid_text);
            continue;
        };
        if group_id.is_nil() {
            log::warn!("prim {} has no scene group and was left out", row.uuid_text);
            continue;
        }
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

fn read_prim(row: &SqliteRow, items: &mut HashMap<Uuid, Vec<InventoryItem>>) -> Option<PrimRow> {
    // Root and link order compare the stored text, the same way the SQLite load does.
    let uuid_text = text_or_empty(row, "UUID");
    if uuid_text.is_empty() {
        log::warn!("prim could not be built and was skipped");
        return None;
    }
    let group_text = text_or_empty(row, "SceneGroupID");
    let group = parse_uuid(&group_text).unwrap_or(Uuid::nil());
    if group.is_nil() {
        log::warn!("prim {uuid_text} has no scene group and was left out");
        return None;
    }
    let link = optional_i32(row, "LinkNumber");
    let start = optional_text(row, "StartStr");
    if sqlite_is_null(row, "pseudocrc") {
        log::warn!("prim {uuid_text} has a null pseudo-CRC and was skipped");
        return Some(PrimRow {
            uuid_text,
            group_text,
            link,
            start,
            part: None,
        });
    }
    if parse_uuid(&uuid_text).is_none() {
        log::warn!("prim {uuid_text} could not be built and was skipped");
        return Some(PrimRow {
            uuid_text,
            group_text,
            link,
            start,
            part: None,
        });
    }
    let mut part = match build_part(row) {
        Ok(part) => part,
        Err(()) => {
            log::warn!("prim {uuid_text} could not be built and was skipped");
            return Some(PrimRow {
                uuid_text,
                group_text,
                link,
                start,
                part: None,
            });
        }
    };
    part.items = items.remove(&part.uuid).unwrap_or_default();
    Some(PrimRow {
        uuid_text,
        group_text,
        link,
        start,
        part: Some(part),
    })
}

/// True when the cell is SQL NULL. A missing column is not null.
fn sqlite_is_null(row: &SqliteRow, name: &str) -> bool {
    match row.try_get::<Option<i64>, _>(name) {
        Ok(Some(_)) => return false,
        Ok(None) => return true,
        Err(_) => {}
    }
    match row.try_get::<Option<f64>, _>(name) {
        Ok(Some(_)) => return false,
        Ok(None) => return true,
        Err(_) => {}
    }
    match row.try_get::<Option<String>, _>(name) {
        Ok(Some(_)) => return false,
        Ok(None) => return true,
        Err(_) => {}
    }
    matches!(row.try_get::<Option<Vec<u8>>, _>(name), Ok(None))
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

fn build_part(row: &SqliteRow) -> Result<ObjectPart, ()> {
    let uuid = required_uuid(row, "UUID")?;
    let (mut shape, media, mat_ovrd) = if sqlite_is_null(row, "ShapeUUID") {
        (default_box(), None, None)
    } else {
        let built = build_shape(row)?;
        let media = optional_text(row, "Media");
        let mat_ovrd = optional_blob(row, "MatOvrd")?;
        (built, media, mat_ovrd)
    };
    shape.media_entries = match &media {
        Some(text) => parse_media(text),
        None => None,
    };
    shape.materials = match &mat_ovrd {
        Some(bytes) => parse_materials(bytes),
        None => None,
    };
    let dyn_text = optional_text(row, "DynAttrs");
    let dyn_attrs = match &dyn_text {
        Some(text) => Some(parse_dyn_attrs(text)?),
        None => None,
    };
    let keyframe_motion = optional_blob(row, "KeyframeMotion")?;
    let keyframe = match &keyframe_motion {
        Some(data) if !data.is_empty() => parse_keyframe(data),
        _ => None,
    };
    let vehicle_text = optional_text(row, "Vehicle");
    let vehicle = match &vehicle_text {
        Some(text) if !text.is_empty() => parse_vehicle(text)?,
        _ => None,
    };
    let inertia_text = optional_text(row, "PhysInertia");
    let phys_inertia = match &inertia_text {
        Some(text) if !text.is_empty() => parse_inertia(text)?,
        _ => None,
    };
    let sopanims = optional_blob(row, "sopanims")?;
    let animations = match &sopanims {
        Some(data) if !data.is_empty() => parse_animations(data),
        _ => None,
    };
    let linkset_text = optional_text(row, "linksetdata");
    let linkset = match &linkset_text {
        Some(text) => parse_linkset(text),
        None => None,
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
        description: cut_stored_text(
            "Description",
            required_text(row, "Description")?,
            DESCRIPTION_LIMIT,
        ),
        sit_name: cut_stored_text("SitName", required_text(row, "SitName")?, SIT_LIMIT),
        touch_name: cut_stored_text("TouchName", required_text(row, "TouchName")?, TOUCH_LIMIT),
        flags,
        creator_id: part_creator_identification(&required_text(row, "CreatorID")?),
        xml_creator_data: None,
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
        sound_gain: f64::from(required_f32(row, "LoopedSoundGain")?),
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
            dyn_attrs: dyn_text,
            keyframe_motion,
            vehicle: vehicle_text,
            phys_inertia: inertia_text,
            sopanims,
            linkset_data: linkset_text,
        },
        dyn_attrs,
        keyframe,
        vehicle,
        phys_inertia,
        animations,
        linkset,
        items: Vec::new(),
        inventory_serial: None,
        allow_unsit: true,
        scripted_sit_only: false,
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
        media_entries: None,
        materials: None,
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
        media_entries: None,
        materials: None,
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

/// OpenMetaverse `UUID.TryParse`: dashed or compact hex, with ASCII space
/// padding. A tab, braces and `urn:uuid:` fail, and the result is nil.
pub fn omv_uuid(text: &str) -> Uuid {
    let text = text.trim_matches(' ');
    if matches!(text.len(), 32 | 36)
        && let Ok(id) = Uuid::parse_str(text)
    {
        return id;
    }
    Uuid::nil()
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

const PRIM_SQL: &str = "SELECT p.UUID, p.RegionUUID, p.SceneGroupID, p.CreationDate, p.Name, p.Text, \
p.Description, p.SitName, p.TouchName, p.ColorR, p.ColorG, p.ColorB, p.ColorA, p.ObjectFlags, \
p.CreatorID, p.OwnerID, p.GroupID, p.LastOwnerID, p.RezzerID, p.OwnerMask, p.NextOwnerMask, \
p.GroupMask, p.EveryoneMask, p.BaseMask, p.PositionX, p.PositionY, p.PositionZ, p.GroupPositionX, \
p.GroupPositionY, p.GroupPositionZ, p.VelocityX, p.VelocityY, p.VelocityZ, p.AngularVelocityX, \
p.AngularVelocityY, p.AngularVelocityZ, p.AccelerationX, p.AccelerationY, p.AccelerationZ, \
p.RotationX, p.RotationY, p.RotationZ, p.RotationW, p.SitTargetOffsetX, p.SitTargetOffsetY, \
p.SitTargetOffsetZ, p.SitTargetOrientX, p.SitTargetOrientY, p.SitTargetOrientZ, p.SitTargetOrientW, \
p.SitTargetActive, p.standtargetx, p.standtargety, p.standtargetz, p.sitactrange, p.ClickAction, \
p.PayPrice, p.PayButton1, p.PayButton2, p.PayButton3, p.PayButton4, p.LoopedSound, \
p.LoopedSoundGain, p.TextureAnimation, p.ParticleSystem, p.CameraEyeOffsetX, p.CameraEyeOffsetY, \
p.CameraEyeOffsetZ, p.CameraAtOffsetX, p.CameraAtOffsetY, p.CameraAtOffsetZ, p.ForceMouselook, \
p.ScriptAccessPin, p.AllowedDrop, p.DieAtEdge, p.SalePrice, p.SaleType, p.Material, \
p.CollisionSound, p.CollisionSoundVolume, p.VolumeDetect, p.MediaURL, p.AttachedPosX, \
p.AttachedPosY, p.AttachedPosZ, p.DynAttrs, p.PhysicsShapeType, p.Density, p.GravityModifier, \
p.Friction, p.Restitution, p.KeyframeMotion, p.PassCollisions, p.PassTouches, p.RotationAxisLocks, \
p.Vehicle, p.PhysInertia, p.pseudocrc, p.sopanims, p.linksetdata, p.StartStr, p.LinkNumber, \
s.UUID AS ShapeUUID, s.ScaleX, s.ScaleY, s.ScaleZ, s.PCode, s.PathBegin, s.PathEnd, s.PathScaleX, \
s.PathScaleY, s.PathShearX, s.PathShearY, s.PathSkew, s.PathCurve, s.PathRadiusOffset, \
s.PathRevolutions, s.PathTaperX, s.PathTaperY, s.PathTwist, s.PathTwistBegin, s.ProfileBegin, \
s.ProfileEnd, s.ProfileCurve, s.ProfileHollow, s.Texture, s.ExtraParams, s.State, s.Media, \
s.LastAttachPoint, s.MatOvrd \
FROM prims p LEFT JOIN primshapes s ON p.UUID = s.UUID COLLATE NOCASE \
WHERE p.RegionUUID = ? COLLATE NOCASE ORDER BY p.rowid";

const ITEM_SQL: &str = "SELECT itemID, primID, assetID, parentFolderID, invType, assetType, name, \
description, creationDate, creatorID, ownerID, lastOwnerID, groupID, nextPermissions, \
currentPermissions, basePermissions, everyonePermissions, groupPermissions, flags, experienceID \
FROM primitems WHERE lower(primID) IN (SELECT lower(UUID) FROM prims WHERE RegionUUID = ? COLLATE NOCASE) \
ORDER BY primitems.rowid";

fn build_item(row: &SqliteRow) -> Option<InventoryItem> {
    let creator = required_text(row, "creatorID").ok()?;
    Some(InventoryItem {
        item_id: required_uuid(row, "itemID").ok()?,
        parent_part_id: required_uuid(row, "primID").ok()?,
        asset_id: required_uuid(row, "assetID").ok()?,
        parent_id: required_uuid(row, "parentFolderID").ok()?,
        inv_type: required_i32(row, "invType").ok()?,
        asset_type: required_i32(row, "assetType").ok()?,
        name: required_text(row, "name").ok()?,
        description: optional_text(row, "description").unwrap_or_default(),
        creation_date: required_u32(row, "creationDate").ok()?,
        creator_id: creator_identification(&creator),
        xml_creator_data: None,
        owner_id: required_uuid(row, "ownerID").ok()?,
        last_owner_id: required_uuid(row, "lastOwnerID").ok()?,
        group_id: required_uuid(row, "groupID").ok()?,
        next_permissions: required_u32(row, "nextPermissions").ok()?,
        current_permissions: required_u32(row, "currentPermissions").ok()?,
        base_permissions: required_u32(row, "basePermissions").ok()?,
        everyone_permissions: required_u32(row, "everyonePermissions").ok()?,
        group_permissions: required_u32(row, "groupPermissions").ok()?,
        flags: required_u32(row, "flags").ok()?,
        experience_id: experience_id(row)?,
    })
}

/// Task-item creator string: a bare id, or `id;endpoint;name`, with no trailing slash.
fn creator_identification(stored: &str) -> String {
    let zero = Uuid::nil().hyphenated().to_string();
    if stored.is_empty() {
        return zero;
    }
    if !stored.contains(';') {
        return parse_uuid(stored)
            .unwrap_or(Uuid::nil())
            .hyphenated()
            .to_string();
    }
    let mut parts = stored.split(';');
    let id = parts
        .next()
        .and_then(parse_uuid)
        .unwrap_or(Uuid::nil())
        .hyphenated();
    let endpoint = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("Unknown User");
    format!("{id};{endpoint};{name}")
}

/// Part creator string. An endpoint gains a trailing `/` and, with no name, `Unknown User`.
/// Segments after the name are dropped. Empty or unparsable text is the zero id.
fn part_creator_identification(stored: &str) -> String {
    let zero = Uuid::nil().hyphenated().to_string();
    if stored.is_empty() {
        return zero;
    }
    let mut parts = stored.split(';');
    let id = parts
        .next()
        .and_then(parse_uuid)
        .unwrap_or(Uuid::nil())
        .hyphenated()
        .to_string();
    let Some(endpoint) = parts.next() else {
        return id;
    };
    let mut data = endpoint.to_owned();
    if !data.ends_with('/') {
        data.push('/');
    }
    data.push(';');
    match parts.next() {
        Some(name) => data.push_str(name),
        None => data.push_str("Unknown User"),
    }
    format!("{id};{data}")
}

/// A null, unparsable or short experience id is the zero id. Any other stored
/// form is unreadable.
fn experience_id(row: &SqliteRow) -> Option<Uuid> {
    match row.try_get::<Option<String>, _>("experienceID") {
        Ok(None) => Some(Uuid::nil()),
        Ok(Some(text)) => Some(parse_uuid(&text).unwrap_or(Uuid::nil())),
        Err(_) => match row.try_get::<Option<Vec<u8>>, _>("experienceID") {
            Ok(Some(bytes)) if bytes.len() >= 16 => {
                let mut id = [0u8; 16];
                id.copy_from_slice(&bytes[..16]);
                Some(Uuid::from_bytes(id))
            }
            Ok(_) => Some(Uuid::nil()),
            Err(_) => None,
        },
    }
}

const MYSQL_PRIM_SQL: &str = "SELECT UUID, SceneGroupID, CreationDate, Name, Text, Description, \
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
SalePrice, SaleType, Material, CollisionSound, CollisionSoundVolume, MediaURL, AttachedPosX, \
AttachedPosY, AttachedPosZ, DynAttrs, PhysicsShapeType, Density, GravityModifier, Friction, \
Restitution, KeyframeMotion, PassCollisions, PassTouches, RotationAxisLocks, Vehicle, PhysInertia, \
pseudocrc, sopanims, linksetdata, StartStr, LinkNumber, AllowUnsit, ScriptedSitOnly \
FROM prims WHERE RegionUUID = ? ORDER BY LinkNumber";

const MYSQL_SHAPE_SQL: &str = "SELECT primshapes.UUID, primshapes.Shape, primshapes.ScaleX, primshapes.ScaleY, \
primshapes.ScaleZ, primshapes.PCode, primshapes.PathBegin, primshapes.PathEnd, primshapes.PathScaleX, \
primshapes.PathScaleY, primshapes.PathShearX, primshapes.PathShearY, primshapes.PathSkew, \
primshapes.PathCurve, primshapes.PathRadiusOffset, primshapes.PathRevolutions, primshapes.PathTaperX, \
primshapes.PathTaperY, primshapes.PathTwist, primshapes.PathTwistBegin, primshapes.ProfileBegin, \
primshapes.ProfileEnd, primshapes.ProfileCurve, primshapes.ProfileHollow, primshapes.Texture, \
primshapes.ExtraParams, primshapes.State, primshapes.Media, primshapes.LastAttachPoint, primshapes.MatOvrd \
FROM prims LEFT JOIN primshapes ON prims.UUID = primshapes.UUID \
WHERE prims.RegionUUID = ? AND primshapes.UUID IS NOT NULL";

const MYSQL_ITEM_IDS_SQL: &str = "SELECT DISTINCT primitems.primID FROM prims \
INNER JOIN primitems ON prims.UUID = primitems.primID WHERE prims.RegionUUID = ?";

const MYSQL_ITEM_SQL: &str = "SELECT itemID, primID, assetID, parentFolderID, invType, assetType, \
name, description, creationDate, creatorID, ownerID, lastOwnerID, groupID, nextPermissions, \
currentPermissions, basePermissions, everyonePermissions, groupPermissions, flags, experienceID \
FROM primitems WHERE primID = ?";

struct MysqlBuilt {
    uuid: Uuid,
    group: Uuid,
    start: Option<String>,
    part: ObjectPart,
}

async fn load_mysql(
    pool: &Pool,
    region: Uuid,
    max_objects: u64,
    max_linkset: u64,
) -> Result<Vec<ObjectGroup>, ObjectError> {
    let Some(mysql) = pool.mysql() else {
        return Err(ObjectError::Store("connection is not MySQL".to_owned()));
    };
    let region_text = region.as_hyphenated().to_string();
    let mut shapes: HashMap<String, MySqlRow> = HashMap::new();
    let mut shape_rows = sqlx::query(MYSQL_SHAPE_SQL).bind(&region_text).fetch(mysql);
    while let Some(row) = shape_rows
        .try_next()
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?
    {
        let id = mysql_text(&row, "UUID").unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        shapes.insert(id.to_ascii_lowercase(), row);
    }
    drop(shape_rows);
    let mut built = Vec::new();
    let mut kept = 0u64;
    let mut prims = sqlx::query(MYSQL_PRIM_SQL).bind(&region_text).fetch(mysql);
    while let Some(row) = prims
        .try_next()
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?
    {
        let label = mysql_text(&row, "UUID").unwrap_or_else(|| "unknown".to_owned());
        let group = parse_uuid(&mysql_text(&row, "SceneGroupID").unwrap_or_default())
            .unwrap_or(Uuid::nil());
        if group.is_nil() {
            log::warn!("prim {label} has no scene group and was left out");
            continue;
        }
        match mysql_part(&row, &shapes) {
            Ok(prim) => {
                kept += 1;
                if kept > max_objects {
                    refuse(
                        REGION_OBJECTS,
                        Kind::Hard,
                        max_objects,
                        kept,
                        "region-store",
                    );
                    return Err(ObjectError::Objects {
                        configured: max_objects,
                        observed: kept,
                    });
                }
                built.push(prim);
            }
            Err(()) => log::warn!("prim {label} could not be built and was skipped"),
        }
    }
    drop(prims);
    let mut groups = assemble_mysql(built);
    let mut located: HashMap<Uuid, (usize, usize)> = HashMap::new();
    for (slot, group) in groups.iter().enumerate() {
        for (index, part) in group.parts.iter().enumerate() {
            located.insert(part.uuid, (slot, index));
        }
    }
    let mut wanted = Vec::new();
    let mut id_rows = sqlx::query(MYSQL_ITEM_IDS_SQL)
        .bind(&region_text)
        .fetch(mysql);
    while let Some(id_row) = id_rows
        .try_next()
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?
    {
        if let Some(prim_text) = mysql_text(&id_row, "primID") {
            wanted.push(prim_text);
        }
    }
    drop(id_rows);
    let mut region_items = 0u64;
    for prim_text in wanted {
        let Some(prim_id) = parse_uuid(&prim_text) else {
            continue;
        };
        let Some(&(slot, index)) = located.get(&prim_id) else {
            continue;
        };
        let mut part_items = 0u64;
        let mut items = Vec::new();
        let mut rows = sqlx::query(MYSQL_ITEM_SQL).bind(&prim_text).fetch(mysql);
        while let Some(row) = rows
            .try_next()
            .await
            .map_err(|error| ObjectError::Store(error.to_string()))?
        {
            match mysql_item(&row, prim_id) {
                Some(item) => {
                    note_streamed_item(&mut region_items, &mut part_items, max_objects)?;
                    items.push(item);
                }
                None => {
                    let label = mysql_text(&row, "itemID").unwrap_or_else(|| "unknown".to_owned());
                    log::warn!("inventory item {label} could not be built and was left out");
                }
            }
        }
        drop(rows);
        groups[slot].parts[index].items = items;
    }
    enforce(groups, max_objects, max_linkset)
}

fn assemble_mysql(rows: Vec<MysqlBuilt>) -> Vec<ObjectGroup> {
    let mut groups = Vec::new();
    let mut index: HashMap<Uuid, usize> = HashMap::new();
    let mut rows: Vec<Option<MysqlBuilt>> = rows.into_iter().map(Some).collect();
    for slot in &mut rows {
        let Some(row) = slot.as_ref() else {
            continue;
        };
        if row.uuid != row.group {
            continue;
        }
        let Some(row) = slot.take() else {
            continue;
        };
        if index.contains_key(&row.uuid) {
            log::warn!("prim {} could not be built and was skipped", row.uuid);
            continue;
        }
        index.insert(row.uuid, groups.len());
        let mut part = row.part;
        part.link_num = 0;
        groups.push(ObjectGroup {
            from_item_id: Uuid::nil(),
            rez_parameter: row.start,
            parts: vec![part],
        });
    }
    for slot in &mut rows {
        let Some(row) = slot.take() else {
            continue;
        };
        if row.uuid == row.group {
            continue;
        }
        let Some(&group_slot) = index.get(&row.group) else {
            log::warn!("prim {} has no root and was skipped", row.uuid);
            continue;
        };
        let mut part = row.part;
        let original = part.link_num;
        let count = groups[group_slot].parts.len().saturating_add(1);
        let Ok(link_num) = i32::try_from(count) else {
            log::warn!("prim {} could not be built and was skipped", row.uuid);
            continue;
        };
        part.link_num = link_num;
        if link_num == 2 {
            groups[group_slot].parts[0].link_num = 1;
        }
        if original != 0 {
            part.link_num = original;
        }
        groups[group_slot].parts.push(part);
    }
    groups
}

fn mysql_part(row: &MySqlRow, shapes: &HashMap<String, MySqlRow>) -> Result<MysqlBuilt, ()> {
    let uuid_text = mysql_text(row, "UUID").ok_or(())?;
    let uuid = parse_uuid(&uuid_text).ok_or(())?;
    let group =
        parse_uuid(&mysql_text(row, "SceneGroupID").unwrap_or_default()).unwrap_or(Uuid::nil());
    let shape_row = shapes.get(&uuid.as_hyphenated().to_string().to_ascii_lowercase());
    let shape_missing = match shape_row {
        None => true,
        Some(shape) => mysql_i64(shape, "Shape").is_none(),
    };
    let (mut shape, media, mat_ovrd) = if shape_missing {
        let mut boxed = default_box();
        boxed.default_box = shape_row.is_none();
        (boxed, None, None)
    } else {
        let shape = shape_row.ok_or(())?;
        let built = mysql_shape(shape)?;
        let media = mysql_text(shape, "Media");
        let mat_ovrd = mysql_blob(shape, "MatOvrd")?;
        (built, media, mat_ovrd)
    };
    if !shape_missing {
        shape.media_entries = match &media {
            Some(text) => parse_media(text),
            None => None,
        };
        shape.materials = match &mat_ovrd {
            Some(bytes) => parse_materials(bytes),
            None => parse_materials_null(),
        };
    }
    let stored_media = shape_row.and_then(|shape| mysql_text(shape, "Media"));
    let stored_mat = match shape_row {
        Some(shape) => mysql_blob(shape, "MatOvrd")?,
        None => None,
    };
    let dyn_text = mysql_text(row, "DynAttrs");
    let dyn_attrs = match &dyn_text {
        Some(text) => Some(parse_dyn_attrs(text)?),
        None => None,
    };
    let keyframe_motion = mysql_blob(row, "KeyframeMotion")?;
    let keyframe = match &keyframe_motion {
        Some(data) if !data.is_empty() => parse_keyframe(data),
        _ => None,
    };
    let vehicle_text = mysql_text(row, "Vehicle").filter(|text| !text.is_empty());
    let vehicle = match &vehicle_text {
        Some(text) => parse_vehicle(text)?,
        None => None,
    };
    let inertia_text = mysql_text(row, "PhysInertia").filter(|text| !text.is_empty());
    let phys_inertia = match &inertia_text {
        Some(text) => parse_inertia(text)?,
        None => None,
    };
    let sopanims = mysql_blob(row, "sopanims")?;
    let animations = match &sopanims {
        Some(data) if !data.is_empty() => parse_animations(data),
        _ => None,
    };
    let linkset_text = mysql_text(row, "linksetdata");
    let linkset = linkset_text.as_deref().and_then(parse_linkset);
    let sound = mysql_uuid(row, "LoopedSound").ok_or(())?;
    let explicit_sit =
        mysql_i64(row, "SitTargetActive").and_then(|value| i32::try_from(value).ok());
    let offset = mysql_vec3(row, "PositionX", "PositionY", "PositionZ", 0.0)?;
    let sit_target_position = mysql_vec3(
        row,
        "SitTargetOffsetX",
        "SitTargetOffsetY",
        "SitTargetOffsetZ",
        0.0,
    )?;
    let sit_target_rotation = mysql_quat(
        row,
        "SitTargetOrientX",
        "SitTargetOrientY",
        "SitTargetOrientZ",
        "SitTargetOrientW",
        0.0,
        1.0,
    )?;
    let pseudo = mysql_i64(row, "pseudocrc")
        .and_then(|value| i32::try_from(value).ok())
        .filter(|value| *value != 0);
    let flags = mysql_u32(row, "ObjectFlags", 0)? & !(FLAG_TOUCH | FLAG_MONEY | FLAG_ALLOW_DROP);
    let creator = mysql_text(row, "CreatorID").ok_or(())?;
    let part = ObjectPart {
        uuid,
        link_num: mysql_i32(row, "LinkNumber")?,
        local_id: 0,
        parent_id: 0,
        creation_date: mysql_i32_default(row, "CreationDate", 0)?,
        name: mysql_text(row, "Name").unwrap_or_default(),
        text: mysql_text(row, "Text").unwrap_or_default(),
        color: mysql_color(row)?,
        description: cut_stored_text(
            "Description",
            mysql_text(row, "Description").unwrap_or_default(),
            DESCRIPTION_LIMIT,
        ),
        sit_name: cut_stored_text(
            "SitName",
            mysql_text(row, "SitName").unwrap_or_default(),
            SIT_LIMIT,
        ),
        touch_name: cut_stored_text(
            "TouchName",
            mysql_text(row, "TouchName").unwrap_or_default(),
            TOUCH_LIMIT,
        ),
        flags,
        creator_id: part_creator_identification(&creator),
        xml_creator_data: None,
        owner_id: mysql_uuid(row, "OwnerID").unwrap_or(Uuid::nil()),
        group_id: mysql_uuid(row, "GroupID").unwrap_or(Uuid::nil()),
        last_owner_id: mysql_uuid(row, "LastOwnerID").unwrap_or(Uuid::nil()),
        rezzer_id: match mysql_text(row, "RezzerID") {
            Some(text) => parse_uuid(&text).unwrap_or(Uuid::nil()),
            None => Uuid::nil(),
        },
        owner_mask: mysql_u32(row, "OwnerMask", 0)?,
        next_owner_mask: mysql_u32(row, "NextOwnerMask", 0)?,
        group_mask: mysql_u32(row, "GroupMask", 0)?,
        everyone_mask: mysql_u32(row, "EveryoneMask", 0)?,
        base_mask: mysql_u32(row, "BaseMask", 0)?,
        offset,
        group_position: mysql_vec3(
            row,
            "GroupPositionX",
            "GroupPositionY",
            "GroupPositionZ",
            0.0,
        )?,
        velocity: mysql_vec3(row, "VelocityX", "VelocityY", "VelocityZ", 0.0)?,
        angular_velocity: mysql_vec3(
            row,
            "AngularVelocityX",
            "AngularVelocityY",
            "AngularVelocityZ",
            0.0,
        )?,
        acceleration: mysql_vec3(row, "AccelerationX", "AccelerationY", "AccelerationZ", 0.0)?,
        rotation: loaded_rotation(mysql_quat(
            row,
            "RotationX",
            "RotationY",
            "RotationZ",
            "RotationW",
            0.0,
            1.0,
        )?),
        sit_target_position,
        sit_target_rotation,
        sit_target_active: sit_active(explicit_sit, sit_target_position, sit_target_rotation),
        stand_offset: mysql_vec3(row, "standtargetx", "standtargety", "standtargetz", 0.0)?,
        sit_active_range: mysql_f32(row, "sitactrange", 0.0)?,
        click_action: mysql_i64(row, "ClickAction")
            .map(|value| value as u8)
            .unwrap_or(0),
        pay_price: [
            mysql_i32(row, "PayPrice")?,
            mysql_i32(row, "PayButton1")?,
            mysql_i32(row, "PayButton2")?,
            mysql_i32(row, "PayButton3")?,
            mysql_i32(row, "PayButton4")?,
        ],
        sound,
        sound_gain: f64::from(mysql_f32(row, "LoopedSoundGain", 0.0)?),
        sound_flags: u8::from(!sound.is_nil()),
        texture_animation: mysql_blob(row, "TextureAnimation")?.unwrap_or_default(),
        particles: mysql_blob(row, "ParticleSystem")?.unwrap_or_default(),
        camera_eye: mysql_vec3(
            row,
            "CameraEyeOffsetX",
            "CameraEyeOffsetY",
            "CameraEyeOffsetZ",
            0.0,
        )?,
        camera_at: mysql_vec3(
            row,
            "CameraAtOffsetX",
            "CameraAtOffsetY",
            "CameraAtOffsetZ",
            0.0,
        )?,
        force_mouselook: mysql_flag(row, "ForceMouselook")?,
        script_access_pin: mysql_i32(row, "ScriptAccessPin")?,
        allowed_drop: mysql_flag(row, "AllowedDrop")?,
        die_at_edge: mysql_flag(row, "DieAtEdge")?,
        sale_price: mysql_i32(row, "SalePrice")?,
        sale_type: mysql_i64(row, "SaleType").ok_or(())? as u8,
        material: mysql_i64(row, "Material").ok_or(())? as u8,
        collision_sound: mysql_uuid(row, "CollisionSound").ok_or(())?,
        collision_sound_volume: mysql_f32_required(row, "CollisionSoundVolume")?,
        volume_detect: (flags & FLAG_VOLUME_DETECT) != 0,
        media_url: mysql_text(row, "MediaURL").unwrap_or_default(),
        attached_pos: mysql_attached(row)?,
        physics_shape_type: mysql_parse_int(row, "PhysicsShapeType")? as u8,
        density: mysql_f32(row, "Density", 1000.0)?,
        gravity_modifier: mysql_f32(row, "GravityModifier", 1.0)?,
        friction: mysql_f32(row, "Friction", 0.6)?,
        restitution: mysql_f32(row, "Restitution", 0.5)?,
        pass_collisions: mysql_flag(row, "PassCollisions")?,
        pass_touches: mysql_flag(row, "PassTouches")?,
        rotation_axis_locks: mysql_parse_int(row, "RotationAxisLocks")? as u8,
        pseudo_crc: pseudo,
        shape,
        raw: ObjectRaw {
            media: stored_media,
            mat_ovrd: stored_mat,
            dyn_attrs: dyn_text,
            keyframe_motion,
            vehicle: mysql_text(row, "Vehicle"),
            phys_inertia: mysql_text(row, "PhysInertia"),
            sopanims,
            linkset_data: linkset_text,
        },
        dyn_attrs,
        keyframe,
        vehicle,
        phys_inertia,
        animations,
        linkset,
        items: Vec::new(),
        inventory_serial: None,
        allow_unsit: mysql_bool_default(row, "AllowUnsit", true),
        scripted_sit_only: mysql_bool_default(row, "ScriptedSitOnly", false),
    };
    Ok(MysqlBuilt {
        uuid,
        group,
        start: mysql_text(row, "StartStr"),
        part,
    })
}

fn parse_materials_null() -> Option<crate::payloads::RenderMaterials> {
    parse_materials(&[])
}

fn mysql_shape(row: &MySqlRow) -> Result<ObjectShape, ()> {
    let texture = match mysql_blob(row, "Texture")? {
        Some(bytes) => bytes,
        None => DEFAULT_BOX_TEXTURE.to_vec(),
    };
    let extra = match mysql_blob(row, "ExtraParams")? {
        Some(bytes) => bytes,
        None => vec![0],
    };
    Ok(ObjectShape {
        default_box: false,
        scale: mysql_vec3_required(row, "ScaleX", "ScaleY", "ScaleZ")?,
        pcode: mysql_u8(row, "PCode", 9)?,
        path_begin: mysql_u16(row, "PathBegin", 0)?,
        path_end: mysql_u16(row, "PathEnd", 0)?,
        path_scale_x: mysql_u8(row, "PathScaleX", 100)?,
        path_scale_y: mysql_u8(row, "PathScaleY", 100)?,
        path_shear_x: mysql_u8(row, "PathShearX", 0)?,
        path_shear_y: mysql_u8(row, "PathShearY", 0)?,
        path_skew: mysql_i8(row, "PathSkew", 0)?,
        path_curve: mysql_u8(row, "PathCurve", 16)?,
        path_radius_offset: mysql_i8(row, "PathRadiusOffset", 0)?,
        path_revolutions: mysql_u8(row, "PathRevolutions", 0)?,
        path_taper_x: mysql_i8(row, "PathTaperX", 0)?,
        path_taper_y: mysql_i8(row, "PathTaperY", 0)?,
        path_twist: mysql_i8(row, "PathTwist", 0)?,
        path_twist_begin: mysql_i8(row, "PathTwistBegin", 0)?,
        profile_begin: mysql_u16(row, "ProfileBegin", 0)?,
        profile_end: mysql_u16(row, "ProfileEnd", 0)?,
        profile_curve: profile_curve(mysql_u8(row, "ProfileCurve", 1)?),
        profile_hollow: mysql_u16(row, "ProfileHollow", 0)?,
        state: mysql_u8(row, "State", 0)?,
        last_attach_point: mysql_i64(row, "LastAttachPoint").ok_or(())? as u8,
        texture,
        extra_params: extra,
        media_entries: None,
        materials: None,
    })
}

fn mysql_item(row: &MySqlRow, prim: Uuid) -> Option<InventoryItem> {
    let creator = mysql_text(row, "creatorID")?;
    let flags = mysql_u32_strict(row, "flags")?;
    Some(InventoryItem {
        item_id: mysql_uuid(row, "itemID").unwrap_or(Uuid::nil()),
        parent_part_id: mysql_uuid(row, "primID").unwrap_or(prim),
        asset_id: mysql_uuid(row, "assetID").unwrap_or(Uuid::nil()),
        parent_id: prim,
        inv_type: mysql_i32_default(row, "invType", -1).ok()?,
        asset_type: mysql_i32_default(row, "assetType", -1).ok()?,
        name: mysql_text(row, "name").unwrap_or_default(),
        description: mysql_text(row, "description").unwrap_or_default(),
        creation_date: mysql_u32_column(row, "creationDate", 0)?,
        creator_id: creator_identification(&creator),
        xml_creator_data: None,
        owner_id: mysql_uuid(row, "ownerID").unwrap_or(Uuid::nil()),
        last_owner_id: mysql_uuid(row, "lastOwnerID").unwrap_or(Uuid::nil()),
        group_id: mysql_uuid(row, "groupID").unwrap_or(Uuid::nil()),
        next_permissions: mysql_u32_column(row, "nextPermissions", 0)?,
        current_permissions: mysql_u32_column(row, "currentPermissions", 0)?,
        base_permissions: mysql_u32_column(row, "basePermissions", 0)?,
        everyone_permissions: mysql_u32_column(row, "everyonePermissions", 0)?,
        group_permissions: mysql_u32_column(row, "groupPermissions", 0)?,
        flags,
        experience_id: mysql_uuid(row, "experienceID").unwrap_or(Uuid::nil()),
    })
}

fn mysql_attached(row: &MySqlRow) -> Result<[f32; 3], ()> {
    if mysql_f64(row, "AttachedPosX").is_none() {
        return Ok([0.0, 0.0, 0.0]);
    }
    Ok([
        mysql_f32_required(row, "AttachedPosX")?,
        mysql_f32(row, "AttachedPosY", 0.0)?,
        mysql_f32(row, "AttachedPosZ", 0.0)?,
    ])
}

fn mysql_color(row: &MySqlRow) -> Result<u32, ()> {
    let alpha = mysql_i32(row, "ColorA")? as u32;
    let red = mysql_i32(row, "ColorR")? as u32;
    let green = mysql_i32(row, "ColorG")? as u32;
    let blue = mysql_i32(row, "ColorB")? as u32;
    Ok((alpha << 24) | (red << 16) | (green << 8) | blue)
}

fn mysql_vec3(row: &MySqlRow, x: &str, y: &str, z: &str, fallback: f32) -> Result<[f32; 3], ()> {
    Ok([
        mysql_f32(row, x, fallback)?,
        mysql_f32(row, y, fallback)?,
        mysql_f32(row, z, fallback)?,
    ])
}

fn mysql_vec3_required(row: &MySqlRow, x: &str, y: &str, z: &str) -> Result<[f32; 3], ()> {
    Ok([
        mysql_f32_required(row, x)?,
        mysql_f32_required(row, y)?,
        mysql_f32_required(row, z)?,
    ])
}

fn mysql_quat(
    row: &MySqlRow,
    x: &str,
    y: &str,
    z: &str,
    w: &str,
    fallback: f32,
    fallback_w: f32,
) -> Result<[f32; 4], ()> {
    Ok([
        mysql_f32(row, x, fallback)?,
        mysql_f32(row, y, fallback)?,
        mysql_f32(row, z, fallback)?,
        mysql_f32(row, w, fallback_w)?,
    ])
}

fn mysql_text(row: &MySqlRow, name: &str) -> Option<String> {
    if let Ok(value) = row.try_get::<Option<String>, _>(name) {
        return value;
    }
    if let Ok(Some(bytes)) = row.try_get::<Option<Vec<u8>>, _>(name) {
        return String::from_utf8(bytes).ok();
    }
    None
}

fn mysql_uuid(row: &MySqlRow, name: &str) -> Option<Uuid> {
    let text = mysql_text(row, name)?;
    Some(parse_uuid(&text).unwrap_or(Uuid::nil()))
}

fn mysql_i64(row: &MySqlRow, name: &str) -> Option<i64> {
    if let Ok(value) = row.try_get::<Option<i8>, _>(name) {
        return value.map(i64::from);
    }
    if let Ok(value) = row.try_get::<Option<i16>, _>(name) {
        return value.map(i64::from);
    }
    if let Ok(value) = row.try_get::<Option<i32>, _>(name) {
        return value.map(i64::from);
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(name) {
        return value;
    }
    if let Ok(value) = row.try_get::<Option<u32>, _>(name) {
        return value.map(i64::from);
    }
    if let Ok(value) = row.try_get::<Option<u64>, _>(name) {
        return value.and_then(|number| i64::try_from(number).ok());
    }
    if let Ok(value) = row.try_get::<Option<f32>, _>(name) {
        return value.and_then(|number| whole_i64(f64::from(number)));
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(name) {
        return value.and_then(whole_i64);
    }
    if let Ok(Some(text)) = row.try_get::<Option<String>, _>(name) {
        return text.trim().parse().ok();
    }
    None
}

fn whole_i64(value: f64) -> Option<i64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || !(i64::MIN as f64..=i64::MAX as f64).contains(&value)
    {
        return None;
    }
    Some(value as i64)
}

fn mysql_f64(row: &MySqlRow, name: &str) -> Option<f64> {
    if let Ok(value) = row.try_get::<Option<f32>, _>(name) {
        return value.map(f64::from);
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(name) {
        return value;
    }
    mysql_i64(row, name).map(|number| number as f64)
}

fn mysql_f32(row: &MySqlRow, name: &str, fallback: f32) -> Result<f32, ()> {
    match mysql_f64(row, name) {
        Some(value) => {
            let narrow = value as f32;
            if narrow.is_finite() {
                Ok(narrow)
            } else {
                Err(())
            }
        }
        None => Ok(fallback),
    }
}

fn mysql_f32_required(row: &MySqlRow, name: &str) -> Result<f32, ()> {
    let value = mysql_f64(row, name).ok_or(())?;
    let narrow = value as f32;
    if narrow.is_finite() {
        Ok(narrow)
    } else {
        Err(())
    }
}

fn mysql_i32(row: &MySqlRow, name: &str) -> Result<i32, ()> {
    let value = mysql_i64(row, name).ok_or(())?;
    i32::try_from(value).map_err(|_| ())
}

fn mysql_i32_default(row: &MySqlRow, name: &str, fallback: i32) -> Result<i32, ()> {
    match mysql_i64(row, name) {
        Some(value) => i32::try_from(value).map_err(|_| ()),
        None => Ok(fallback),
    }
}

fn mysql_u32(row: &MySqlRow, name: &str, fallback: u32) -> Result<u32, ()> {
    match mysql_i64(row, name) {
        Some(value) => Ok(value as u32),
        None => Ok(fallback),
    }
}

fn mysql_u32_strict(row: &MySqlRow, name: &str) -> Option<u32> {
    let value = mysql_i64(row, name)?;
    u32::try_from(value).ok()
}

/// A null cell keeps the column default. A value that is not a `u32` leaves the row out.
fn mysql_u32_column(row: &MySqlRow, name: &str, fallback: u32) -> Option<u32> {
    match mysql_i64(row, name) {
        Some(value) => u32::try_from(value).ok(),
        None => Some(fallback),
    }
}

fn mysql_bool_default(row: &MySqlRow, name: &str, fallback: bool) -> bool {
    match mysql_i64(row, name) {
        Some(value) => value != 0,
        None => fallback,
    }
}

fn mysql_u16(row: &MySqlRow, name: &str, fallback: u16) -> Result<u16, ()> {
    match mysql_i64(row, name) {
        Some(value) => Ok(value as u16),
        None => Ok(fallback),
    }
}

fn mysql_u8(row: &MySqlRow, name: &str, fallback: u8) -> Result<u8, ()> {
    match mysql_i64(row, name) {
        Some(value) => Ok(value as u8),
        None => Ok(fallback),
    }
}

fn mysql_i8(row: &MySqlRow, name: &str, fallback: i8) -> Result<i8, ()> {
    match mysql_i64(row, name) {
        Some(value) => Ok(value as i8),
        None => Ok(fallback),
    }
}

fn mysql_flag(row: &MySqlRow, name: &str) -> Result<bool, ()> {
    Ok((mysql_i64(row, name).ok_or(())? as i8) != 0)
}

fn mysql_parse_int(row: &MySqlRow, name: &str) -> Result<i32, ()> {
    if let Some(text) = mysql_text(row, name) {
        return text.trim().parse::<i32>().map_err(|_| ());
    }
    mysql_i32(row, name)
}

fn mysql_blob(row: &MySqlRow, name: &str) -> Result<Option<Vec<u8>>, ()> {
    match row.try_get::<Option<Vec<u8>>, _>(name) {
        Ok(value) => Ok(value),
        Err(_) => match row.try_get::<Option<String>, _>(name) {
            Ok(value) => Ok(value.map(String::into_bytes)),
            Err(_) => Err(()),
        },
    }
}

const PG_PRIM_SQL: &str = "SELECT \"UUID\", \"SceneGroupID\", \"CreationDate\", \"Name\", \"Text\", \
\"Description\", \"SitName\", \"TouchName\", \"ColorR\", \"ColorG\", \"ColorB\", \"ColorA\", \
\"ObjectFlags\", \"CreatorID\", \"OwnerID\", \"GroupID\", \"LastOwnerID\", \"RezzerID\", \
\"OwnerMask\", \"NextOwnerMask\", \"GroupMask\", \"EveryoneMask\", \"BaseMask\", \"PositionX\", \
\"PositionY\", \"PositionZ\", \"GroupPositionX\", \"GroupPositionY\", \"GroupPositionZ\", \
\"VelocityX\", \"VelocityY\", \"VelocityZ\", \"AngularVelocityX\", \"AngularVelocityY\", \
\"AngularVelocityZ\", \"AccelerationX\", \"AccelerationY\", \"AccelerationZ\", \"RotationX\", \
\"RotationY\", \"RotationZ\", \"RotationW\", \"SitTargetOffsetX\", \"SitTargetOffsetY\", \
\"SitTargetOffsetZ\", \"SitTargetOrientX\", \"SitTargetOrientY\", \"SitTargetOrientZ\", \
\"SitTargetOrientW\", \"SitTargetActive\", \"standtargetx\", \"standtargety\", \"standtargetz\", \
\"sitactrange\", \"ClickAction\", \"PayPrice\", \"PayButton1\", \"PayButton2\", \"PayButton3\", \
\"PayButton4\", \"LoopedSound\", \"LoopedSoundGain\", \"TextureAnimation\", \"ParticleSystem\", \
\"CameraEyeOffsetX\", \"CameraEyeOffsetY\", \"CameraEyeOffsetZ\", \"CameraAtOffsetX\", \
\"CameraAtOffsetY\", \"CameraAtOffsetZ\", \"ForceMouselook\", \"ScriptAccessPin\", \"AllowedDrop\", \
\"DieAtEdge\", \"SalePrice\", \"SaleType\", \"Material\", \"CollisionSound\", \
\"CollisionSoundVolume\", \"MediaURL\", \"DynAttrs\", \"PhysicsShapeType\", \"Density\", \
\"GravityModifier\", \"Friction\", \"Restitution\", \"PassCollisions\", \"PassTouches\", \
\"RotationAxisLocks\", \"Vehicle\", \"PhysInertia\", \"pseudocrc\", \"sopanims\", \"linksetdata\", \
\"StartStr\" FROM prims WHERE \"RegionUUID\" = $1 ORDER BY \"SceneGroupID\" ASC, \
CASE WHEN \"UUID\" = \"SceneGroupID\" THEN 0 ELSE 1 END ASC, \"LinkNumber\" ASC";

const PG_SHAPE_SQL: &str = "SELECT \"UUID\", \"Shape\", \"ScaleX\", \"ScaleY\", \"ScaleZ\", \"PCode\", \
\"PathBegin\", \"PathEnd\", \"PathScaleX\", \"PathScaleY\", \"PathShearX\", \"PathShearY\", \
\"PathSkew\", \"PathCurve\", \"PathRadiusOffset\", \"PathRevolutions\", \"PathTaperX\", \
\"PathTaperY\", \"PathTwist\", \"PathTwistBegin\", \"ProfileBegin\", \"ProfileEnd\", \
\"ProfileCurve\", \"ProfileHollow\", \"Texture\", \"ExtraParams\", \"State\", \"Media\", \"MatOvrd\" \
FROM primshapes WHERE \"UUID\" IN (SELECT \"UUID\" FROM prims WHERE \"RegionUUID\" = $1)";

const PG_ITEM_IDS_SQL: &str = "SELECT DISTINCT \"primID\" FROM primitems WHERE \"primID\" IN \
(SELECT \"UUID\" FROM prims WHERE \"RegionUUID\" = $1)";

const PG_ITEM_SQL: &str = "SELECT \"itemID\", \"primID\", \"assetID\", \"parentFolderID\", \
\"invType\", \"assetType\", \"name\", \"description\", \"creationDate\", \"creatorID\", \"ownerID\", \
\"lastOwnerID\", \"groupID\", \"nextPermissions\", \"currentPermissions\", \"basePermissions\", \
\"everyonePermissions\", \"groupPermissions\", \"flags\", \"experienceID\" FROM primitems \
WHERE \"primID\" = $1";

struct PgBuilt {
    original: Uuid,
    group_id: Uuid,
    start: Option<String>,
    part: ObjectPart,
}

async fn load_postgres(
    pool: &Pool,
    region: Uuid,
    max_objects: u64,
    max_linkset: u64,
) -> Result<Vec<ObjectGroup>, ObjectError> {
    let Some(postgres) = pool.postgres() else {
        return Err(ObjectError::Store(
            "connection is not PostgreSQL".to_owned(),
        ));
    };
    let mut shapes: HashMap<Uuid, PgRow> = HashMap::new();
    let mut shape_rows = sqlx::query(PG_SHAPE_SQL).bind(region).fetch(postgres);
    while let Some(row) = shape_rows
        .try_next()
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?
    {
        if let Some(id) = pg_uuid(&row, "UUID") {
            shapes.insert(id, row);
        }
    }
    drop(shape_rows);
    let mut built = Vec::new();
    let mut kept = 0u64;
    let mut prims = sqlx::query(PG_PRIM_SQL).bind(region).fetch(postgres);
    while let Some(row) = prims
        .try_next()
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?
    {
        let label = pg_uuid(&row, "UUID")
            .map(|id| id.hyphenated().to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        if !pg_scene_group_present(&row) {
            log::warn!("prim {label} has no scene group and was left out");
            continue;
        }
        match pg_part(&row, &shapes) {
            Ok(prim) => {
                kept += 1;
                if kept > max_objects {
                    refuse(
                        REGION_OBJECTS,
                        Kind::Hard,
                        max_objects,
                        kept,
                        "region-store",
                    );
                    return Err(ObjectError::Objects {
                        configured: max_objects,
                        observed: kept,
                    });
                }
                built.push(prim);
            }
            Err(()) => log::warn!("prim {label} could not be built and was skipped"),
        }
    }
    drop(prims);
    let (mut groups, originals) = assemble_postgres(built);
    let mut wanted = Vec::new();
    let mut id_rows = sqlx::query(PG_ITEM_IDS_SQL).bind(region).fetch(postgres);
    while let Some(id_row) = id_rows
        .try_next()
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?
    {
        if let Some(original) = pg_uuid(&id_row, "primID") {
            wanted.push(original);
        }
    }
    drop(id_rows);
    let mut region_items = 0u64;
    for original in wanted {
        let Some(&(slot, index)) = originals.get(&original) else {
            continue;
        };
        let current = groups[slot].parts[index].uuid;
        // Items are stored under the original prim id. The parent id is the loaded prim id.
        let mut part_items = 0u64;
        let mut items = Vec::new();
        let mut rows = sqlx::query(PG_ITEM_SQL).bind(original).fetch(postgres);
        while let Some(row) = rows
            .try_next()
            .await
            .map_err(|error| ObjectError::Store(error.to_string()))?
        {
            match pg_item(&row, current) {
                Some(item) => {
                    note_streamed_item(&mut region_items, &mut part_items, max_objects)?;
                    items.push(item);
                }
                None => {
                    let label = pg_uuid(&row, "itemID")
                        .map(|id| id.hyphenated().to_string())
                        .unwrap_or_else(|| "unknown".to_owned());
                    log::warn!("inventory item {label} could not be built and was left out");
                }
            }
        }
        drop(rows);
        groups[slot].parts[index].items = items;
    }
    enforce(groups, max_objects, max_linkset)
}

fn assemble_postgres(rows: Vec<PgBuilt>) -> (Vec<ObjectGroup>, HashMap<Uuid, (usize, usize)>) {
    let mut groups = Vec::new();
    let mut originals = HashMap::new();
    let mut batch = Vec::new();
    let mut current: Option<Uuid> = None;
    for row in rows {
        if current != Some(row.group_id) {
            flush_pg_batch(&mut batch, &mut groups, &mut originals);
            current = Some(row.group_id);
        }
        batch.push(row);
    }
    flush_pg_batch(&mut batch, &mut groups, &mut originals);
    (groups, originals)
}

/// A group whose stored root row is missing stays out. Children are not promoted.
fn flush_pg_batch(
    batch: &mut Vec<PgBuilt>,
    groups: &mut Vec<ObjectGroup>,
    originals: &mut HashMap<Uuid, (usize, usize)>,
) {
    if batch.is_empty() {
        return;
    }
    let group_id = batch[0].group_id;
    if group_id.is_nil() {
        for row in batch.drain(..) {
            log::warn!("prim {} has no scene group and was left out", row.original);
        }
        return;
    }
    let Some(root_at) = batch.iter().position(|row| row.part.uuid == group_id) else {
        for row in batch.drain(..) {
            log::warn!("prim {} has no root and was skipped", row.original);
        }
        return;
    };
    let mut root = batch.remove(root_at);
    root.part.link_num = 0;
    root.part.parent_id = 0;
    let slot = groups.len();
    originals.insert(root.original, (slot, 0));
    let start = root.start;
    let mut parts = vec![root.part];
    for row in batch.drain(..) {
        let count = parts.len().saturating_add(1);
        let Ok(link_num) = i32::try_from(count) else {
            log::warn!("prim {} could not be built and was skipped", row.original);
            continue;
        };
        let mut part = row.part;
        part.link_num = link_num;
        part.parent_id = 0;
        if link_num == 2 {
            parts[0].link_num = 1;
        }
        let index = parts.len();
        originals.insert(row.original, (slot, index));
        parts.push(part);
    }
    groups.push(ObjectGroup {
        from_item_id: Uuid::nil(),
        rez_parameter: start,
        parts,
    });
}

fn pg_part(row: &PgRow, shapes: &HashMap<Uuid, PgRow>) -> Result<PgBuilt, ()> {
    let uuid = pg_uuid_required(row, "UUID")?;
    let group_id = pg_uuid_required(row, "SceneGroupID")?;
    let shape_row = shapes.get(&uuid);
    let shape_missing = match shape_row {
        None => true,
        Some(shape) => pg_i64(shape, "Shape").is_none(),
    };
    let (mut shape, media, mat_ovrd) = if shape_missing {
        (default_box(), None, None)
    } else {
        let shape = shape_row.ok_or(())?;
        let built = pg_shape(shape)?;
        let media = pg_text(shape, "Media");
        let mat_ovrd = pg_blob(shape, "MatOvrd")?;
        (built, media, mat_ovrd)
    };
    if !shape_missing {
        shape.media_entries = match &media {
            Some(text) => parse_media(text),
            None => None,
        };
        shape.materials = match &mat_ovrd {
            Some(bytes) => parse_materials(bytes),
            None => None,
        };
    }
    let stored_media = shape_row.and_then(|shape| pg_text(shape, "Media"));
    let stored_mat = match shape_row {
        Some(shape) => pg_blob(shape, "MatOvrd")?,
        None => None,
    };
    let dyn_text = pg_text(row, "DynAttrs").filter(|text| !text.is_empty());
    let dyn_attrs = match &dyn_text {
        Some(text) => Some(parse_dyn_attrs(text)?),
        None => None,
    };
    let vehicle_text = pg_text(row, "Vehicle").filter(|text| !text.is_empty());
    let vehicle = match &vehicle_text {
        Some(text) => parse_vehicle(text)?,
        None => None,
    };
    let inertia_text = pg_text(row, "PhysInertia").filter(|text| !text.is_empty());
    let phys_inertia = match &inertia_text {
        Some(text) => parse_inertia(text)?,
        None => None,
    };
    let sopanims = pg_blob(row, "sopanims")?;
    let animations = match &sopanims {
        Some(data) if !data.is_empty() => parse_animations(data),
        _ => None,
    };
    let linkset_text = pg_text(row, "linksetdata");
    let linkset = linkset_text.as_deref().and_then(parse_linkset);
    let sound = pg_uuid_required(row, "LoopedSound")?;
    let explicit_sit = pg_i64(row, "SitTargetActive").and_then(|value| i32::try_from(value).ok());
    let offset = pg_vec3_req(row, "PositionX", "PositionY", "PositionZ")?;
    let sit_target_position = pg_vec3_req(
        row,
        "SitTargetOffsetX",
        "SitTargetOffsetY",
        "SitTargetOffsetZ",
    )?;
    let sit_target_rotation = pg_quat_req(
        row,
        "SitTargetOrientX",
        "SitTargetOrientY",
        "SitTargetOrientZ",
        "SitTargetOrientW",
    )?;
    let pseudo = pg_i32_req(row, "pseudocrc")?;
    let pseudo = (pseudo != 0).then_some(pseudo);
    let flags = pg_u32_req(row, "ObjectFlags")? & !(FLAG_TOUCH | FLAG_MONEY | FLAG_ALLOW_DROP);
    let creator = pg_text_req(row, "CreatorID")?;
    let part = ObjectPart {
        uuid,
        link_num: 0,
        local_id: 0,
        parent_id: 0,
        creation_date: pg_i32_req(row, "CreationDate")?,
        name: pg_text_req(row, "Name")?,
        text: pg_text_req(row, "Text")?,
        color: pg_color(row)?,
        description: cut_stored_text(
            "Description",
            pg_text_req(row, "Description")?,
            DESCRIPTION_LIMIT,
        ),
        sit_name: cut_stored_text("SitName", pg_text_req(row, "SitName")?, SIT_LIMIT),
        touch_name: cut_stored_text("TouchName", pg_text_req(row, "TouchName")?, TOUCH_LIMIT),
        flags,
        creator_id: part_creator_identification(&creator),
        xml_creator_data: None,
        owner_id: pg_uuid_required(row, "OwnerID")?,
        group_id: pg_uuid_required(row, "GroupID")?,
        last_owner_id: pg_uuid_required(row, "LastOwnerID")?,
        rezzer_id: pg_uuid(row, "RezzerID").unwrap_or(Uuid::nil()),
        owner_mask: pg_u32_req(row, "OwnerMask")?,
        next_owner_mask: pg_u32_req(row, "NextOwnerMask")?,
        group_mask: pg_u32_req(row, "GroupMask")?,
        everyone_mask: pg_u32_req(row, "EveryoneMask")?,
        base_mask: pg_u32_req(row, "BaseMask")?,
        offset,
        group_position: pg_vec3_req(row, "GroupPositionX", "GroupPositionY", "GroupPositionZ")?,
        velocity: pg_vec3_req(row, "VelocityX", "VelocityY", "VelocityZ")?,
        angular_velocity: pg_vec3_req(
            row,
            "AngularVelocityX",
            "AngularVelocityY",
            "AngularVelocityZ",
        )?,
        acceleration: pg_vec3_req(row, "AccelerationX", "AccelerationY", "AccelerationZ")?,
        rotation: loaded_rotation(pg_quat_req(
            row,
            "RotationX",
            "RotationY",
            "RotationZ",
            "RotationW",
        )?),
        sit_target_position,
        sit_target_rotation,
        sit_target_active: sit_active(explicit_sit, sit_target_position, sit_target_rotation),
        stand_offset: pg_vec3_req(row, "standtargetx", "standtargety", "standtargetz")?,
        sit_active_range: pg_f32_req(row, "sitactrange")?,
        click_action: match pg_i64(row, "ClickAction") {
            Some(value) => u8::try_from(value).map_err(|_| ())?,
            None => 0,
        },
        pay_price: [
            pg_i32_req(row, "PayPrice")?,
            pg_i32_req(row, "PayButton1")?,
            pg_i32_req(row, "PayButton2")?,
            pg_i32_req(row, "PayButton3")?,
            pg_i32_req(row, "PayButton4")?,
        ],
        sound,
        sound_gain: f64::from(pg_f32_req(row, "LoopedSoundGain")?),
        sound_flags: u8::from(!sound.is_nil()),
        texture_animation: pg_blob(row, "TextureAnimation")?.unwrap_or_default(),
        particles: pg_blob(row, "ParticleSystem")?.unwrap_or_default(),
        camera_eye: pg_vec3_req(
            row,
            "CameraEyeOffsetX",
            "CameraEyeOffsetY",
            "CameraEyeOffsetZ",
        )?,
        camera_at: pg_vec3_req(row, "CameraAtOffsetX", "CameraAtOffsetY", "CameraAtOffsetZ")?,
        force_mouselook: pg_bool_or_int(row, "ForceMouselook")?,
        script_access_pin: pg_i32_req(row, "ScriptAccessPin")?,
        allowed_drop: pg_bool_or_int(row, "AllowedDrop")?,
        die_at_edge: pg_bool_or_int(row, "DieAtEdge")?,
        sale_price: pg_i32_req(row, "SalePrice")?,
        sale_type: pg_u8_req(row, "SaleType")?,
        material: pg_u8_req(row, "Material")?,
        collision_sound: pg_uuid_required(row, "CollisionSound")?,
        collision_sound_volume: pg_f32_req(row, "CollisionSoundVolume")?,
        volume_detect: (flags & FLAG_VOLUME_DETECT) != 0,
        media_url: pg_text(row, "MediaURL").unwrap_or_default(),
        attached_pos: [0.0, 0.0, 0.0],
        physics_shape_type: pg_u8_req(row, "PhysicsShapeType")?,
        density: pg_f32_req(row, "Density")?,
        gravity_modifier: pg_f32_req(row, "GravityModifier")?,
        friction: pg_f32_req(row, "Friction")?,
        restitution: pg_f32_req(row, "Restitution")?,
        pass_collisions: pg_bool(row, "PassCollisions")?,
        pass_touches: pg_bool(row, "PassTouches")?,
        rotation_axis_locks: pg_u8_req(row, "RotationAxisLocks")?,
        pseudo_crc: pseudo,
        shape,
        raw: ObjectRaw {
            media: stored_media,
            mat_ovrd: stored_mat,
            dyn_attrs: dyn_text,
            keyframe_motion: None,
            vehicle: pg_text(row, "Vehicle"),
            phys_inertia: pg_text(row, "PhysInertia"),
            sopanims,
            linkset_data: linkset_text,
        },
        dyn_attrs,
        keyframe: None,
        vehicle,
        phys_inertia,
        animations,
        linkset,
        items: Vec::new(),
        inventory_serial: None,
        allow_unsit: true,
        scripted_sit_only: false,
    };
    Ok(PgBuilt {
        original: uuid,
        group_id,
        start: pg_text(row, "StartStr"),
        part,
    })
}

fn pg_shape(row: &PgRow) -> Result<ObjectShape, ()> {
    let texture = pg_blob(row, "Texture")?.ok_or(())?;
    let extra = pg_blob(row, "ExtraParams")?.ok_or(())?;
    Ok(ObjectShape {
        default_box: false,
        scale: pg_vec3_req(row, "ScaleX", "ScaleY", "ScaleZ")?,
        pcode: pg_u8_req(row, "PCode")?,
        path_begin: pg_u16_req(row, "PathBegin")?,
        path_end: pg_u16_req(row, "PathEnd")?,
        path_scale_x: pg_u8_req(row, "PathScaleX")?,
        path_scale_y: pg_u8_req(row, "PathScaleY")?,
        path_shear_x: pg_u8_req(row, "PathShearX")?,
        path_shear_y: pg_u8_req(row, "PathShearY")?,
        path_skew: pg_i8_req(row, "PathSkew")?,
        path_curve: pg_u8_req(row, "PathCurve")?,
        path_radius_offset: pg_i8_req(row, "PathRadiusOffset")?,
        path_revolutions: pg_u8_req(row, "PathRevolutions")?,
        path_taper_x: pg_i8_req(row, "PathTaperX")?,
        path_taper_y: pg_i8_req(row, "PathTaperY")?,
        path_twist: pg_i8_req(row, "PathTwist")?,
        path_twist_begin: pg_i8_req(row, "PathTwistBegin")?,
        profile_begin: pg_u16_req(row, "ProfileBegin")?,
        profile_end: pg_u16_req(row, "ProfileEnd")?,
        profile_curve: profile_curve(pg_u8_req(row, "ProfileCurve")?),
        profile_hollow: pg_u16_req(row, "ProfileHollow")?,
        state: match pg_i64(row, "State") {
            Some(value) => u8::try_from(value).map_err(|_| ())?,
            None => 0,
        },
        last_attach_point: 0,
        texture,
        extra_params: extra,
        media_entries: None,
        materials: None,
    })
}

fn pg_item(row: &PgRow, prim: Uuid) -> Option<InventoryItem> {
    let creator = pg_text_req(row, "creatorID").ok()?;
    let flags = pg_u32_req(row, "flags").ok()?;
    Some(InventoryItem {
        item_id: pg_uuid_required(row, "itemID").ok()?,
        parent_part_id: pg_uuid_required(row, "primID").ok()?,
        asset_id: pg_uuid_required(row, "assetID").ok()?,
        parent_id: prim,
        inv_type: pg_i32_req(row, "invType").ok()?,
        asset_type: pg_i32_req(row, "assetType").ok()?,
        name: pg_text_req(row, "name").ok()?,
        description: pg_text_req(row, "description").ok()?,
        creation_date: pg_u32_req(row, "creationDate").ok()?,
        creator_id: creator_identification(&creator),
        xml_creator_data: None,
        owner_id: pg_uuid_required(row, "ownerID").ok()?,
        last_owner_id: pg_uuid_required(row, "lastOwnerID").ok()?,
        group_id: pg_uuid_required(row, "groupID").ok()?,
        next_permissions: pg_u32_req(row, "nextPermissions").ok()?,
        current_permissions: pg_u32_req(row, "currentPermissions").ok()?,
        base_permissions: pg_u32_req(row, "basePermissions").ok()?,
        everyone_permissions: pg_u32_req(row, "everyonePermissions").ok()?,
        group_permissions: pg_u32_req(row, "groupPermissions").ok()?,
        flags,
        experience_id: pg_uuid(row, "experienceID").unwrap_or(Uuid::nil()),
    })
}

fn pg_text(row: &PgRow, name: &str) -> Option<String> {
    row.try_get::<Option<String>, _>(name).ok().flatten()
}

fn pg_text_req(row: &PgRow, name: &str) -> Result<String, ()> {
    pg_text(row, name).ok_or(())
}

fn pg_scene_group_present(row: &PgRow) -> bool {
    match row.try_get::<Option<Uuid>, _>("SceneGroupID") {
        Ok(Some(id)) => !id.is_nil(),
        Ok(None) => false,
        Err(_) => match pg_text(row, "SceneGroupID") {
            Some(text) => parse_uuid(&text).is_some_and(|id| !id.is_nil()),
            None => false,
        },
    }
}

fn pg_uuid(row: &PgRow, name: &str) -> Option<Uuid> {
    if let Ok(value) = row.try_get::<Option<Uuid>, _>(name) {
        return value;
    }
    pg_text(row, name).and_then(|text| parse_uuid(&text))
}

fn pg_uuid_required(row: &PgRow, name: &str) -> Result<Uuid, ()> {
    match row.try_get::<Option<Uuid>, _>(name) {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(()),
        Err(_) => match pg_text(row, name) {
            Some(text) => parse_uuid(&text).ok_or(()),
            None => Err(()),
        },
    }
}

fn pg_i64(row: &PgRow, name: &str) -> Option<i64> {
    if let Ok(value) = row.try_get::<Option<i16>, _>(name) {
        return value.map(i64::from);
    }
    if let Ok(value) = row.try_get::<Option<i32>, _>(name) {
        return value.map(i64::from);
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(name) {
        return value;
    }
    if let Ok(value) = row.try_get::<Option<f32>, _>(name) {
        return value.and_then(|number| whole_i64(f64::from(number)));
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(name) {
        return value.and_then(whole_i64);
    }
    if let Ok(Some(text)) = row.try_get::<Option<String>, _>(name) {
        return text.trim().parse().ok();
    }
    None
}

fn pg_f64(row: &PgRow, name: &str) -> Option<f64> {
    if let Ok(value) = row.try_get::<Option<f32>, _>(name) {
        return value.map(f64::from);
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(name) {
        return value;
    }
    pg_i64(row, name).map(|number| number as f64)
}

fn pg_f32_req(row: &PgRow, name: &str) -> Result<f32, ()> {
    let value = pg_f64(row, name).ok_or(())?;
    let narrow = value as f32;
    if narrow.is_finite() {
        Ok(narrow)
    } else {
        Err(())
    }
}

fn pg_i32_req(row: &PgRow, name: &str) -> Result<i32, ()> {
    i32::try_from(pg_i64(row, name).ok_or(())?).map_err(|_| ())
}

fn pg_u32_req(row: &PgRow, name: &str) -> Result<u32, ()> {
    u32::try_from(pg_i64(row, name).ok_or(())?).map_err(|_| ())
}

fn pg_u16_req(row: &PgRow, name: &str) -> Result<u16, ()> {
    u16::try_from(pg_i64(row, name).ok_or(())?).map_err(|_| ())
}

fn pg_u8_req(row: &PgRow, name: &str) -> Result<u8, ()> {
    u8::try_from(pg_i64(row, name).ok_or(())?).map_err(|_| ())
}

fn pg_i8_req(row: &PgRow, name: &str) -> Result<i8, ()> {
    i8::try_from(pg_i64(row, name).ok_or(())?).map_err(|_| ())
}

fn pg_bool(row: &PgRow, name: &str) -> Result<bool, ()> {
    match row.try_get::<Option<bool>, _>(name) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(()),
        Err(_) => Ok(pg_i64(row, name).ok_or(())? != 0),
    }
}

fn pg_bool_or_int(row: &PgRow, name: &str) -> Result<bool, ()> {
    if let Ok(value) = row.try_get::<Option<bool>, _>(name) {
        return value.ok_or(());
    }
    Ok(pg_i64(row, name).ok_or(())? != 0)
}

fn pg_blob(row: &PgRow, name: &str) -> Result<Option<Vec<u8>>, ()> {
    match row.try_get::<Option<Vec<u8>>, _>(name) {
        Ok(value) => Ok(value),
        Err(_) => match row.try_get::<Option<String>, _>(name) {
            Ok(value) => Ok(value.map(String::into_bytes)),
            Err(_) => Err(()),
        },
    }
}

fn pg_color(row: &PgRow) -> Result<u32, ()> {
    let alpha = pg_i32_req(row, "ColorA")? as u32;
    let red = pg_i32_req(row, "ColorR")? as u32;
    let green = pg_i32_req(row, "ColorG")? as u32;
    let blue = pg_i32_req(row, "ColorB")? as u32;
    Ok((alpha << 24) | (red << 16) | (green << 8) | blue)
}

fn pg_vec3_req(row: &PgRow, x: &str, y: &str, z: &str) -> Result<[f32; 3], ()> {
    Ok([
        pg_f32_req(row, x)?,
        pg_f32_req(row, y)?,
        pg_f32_req(row, z)?,
    ])
}

fn pg_quat_req(row: &PgRow, x: &str, y: &str, z: &str, w: &str) -> Result<[f32; 4], ()> {
    Ok([
        pg_f32_req(row, x)?,
        pg_f32_req(row, y)?,
        pg_f32_req(row, z)?,
        pg_f32_req(row, w)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_store::CopiedStore;
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

    fn expect_f32(path: &str, got: f32, value: &Value) {
        let parsed = value.as_f64().unwrap_or_else(|| panic!("{path}")) as f32;
        assert_eq!(
            parsed.to_bits(),
            got.to_bits(),
            "{path} dump {value} load {got}"
        );
    }

    fn expect_i32(path: &str, got: i32, value: &Value) {
        assert_eq!(i64::from(got), json_i64(value, path), "{path}");
    }

    fn expect_bool(path: &str, got: bool, value: &Value) {
        assert_eq!(
            got,
            value.as_bool().unwrap_or_else(|| panic!("{path}")),
            "{path}"
        );
    }

    fn expect_opt_f32(path: &str, got: Option<f32>, value: &Value) {
        match got {
            None => assert!(value.is_null(), "{path}"),
            Some(number) => expect_f32(path, number, value),
        }
    }

    fn expect_opt_f32s(path: &str, got: Option<&[f32]>, value: &Value) {
        match got {
            None => assert!(value.is_null(), "{path}"),
            Some(items) => expect_f32s(path, items, value),
        }
    }

    fn llsd_value(value: &ferroan_llsd::Llsd) -> Value {
        use ferroan_llsd::Llsd;
        match value {
            Llsd::Undef => Value::Null,
            Llsd::Boolean(flag) => Value::Bool(*flag),
            Llsd::Integer(number) => Value::from(*number),
            Llsd::Real(number) => serde_json::Number::from_f64(*number)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Llsd::String(text) | Llsd::Uri(text) => Value::String(text.clone()),
            Llsd::Uuid(id) => Value::String(id.to_string()),
            Llsd::Date(_) | Llsd::Binary(_) => Value::String(value.to_osd_string()),
            Llsd::Array(items) => Value::Array(items.iter().map(llsd_value).collect()),
            Llsd::Map(map) => {
                let mut obj = Map::new();
                for (key, child) in map {
                    obj.insert(key.clone(), llsd_value(child));
                }
                Value::Object(obj)
            }
        }
    }

    fn expect_dyn_attrs(got: &Option<DynAttrs>, value: &Value) {
        match got {
            None => assert!(value.is_null(), "dyn_attrs"),
            Some(attrs) => assert_eq!(
                value,
                &llsd_value(&ferroan_llsd::Llsd::Map(attrs.namespaces.clone())),
                "dyn_attrs"
            ),
        }
    }

    fn expect_frame(path: &str, frame: &crate::payloads::Keyframe, value: &Value) {
        let mut obj = value.as_object().unwrap().clone();
        expect_opt_f32s(
            &format!("{path}.position"),
            frame.position.as_ref().map(|item| item.as_slice()),
            &take(&mut obj, "position"),
        );
        expect_opt_f32s(
            &format!("{path}.rotation"),
            frame.rotation.as_ref().map(|item| item.as_slice()),
            &take(&mut obj, "rotation"),
        );
        expect_f32s(
            &format!("{path}.start_rotation"),
            &frame.start_rotation,
            &take(&mut obj, "start_rotation"),
        );
        expect_i32(
            &format!("{path}.time_ms"),
            frame.time_ms,
            &take(&mut obj, "time_ms"),
        );
        expect_i32(
            &format!("{path}.time_total"),
            frame.time_total,
            &take(&mut obj, "time_total"),
        );
        expect_f32s(
            &format!("{path}.angular_velocity"),
            &frame.angular_velocity,
            &take(&mut obj, "angular_velocity"),
        );
        expect_f32s(
            &format!("{path}.start_position"),
            &frame.start_position,
            &take(&mut obj, "start_position"),
        );
        assert!(
            obj.is_empty(),
            "{path} uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    fn expect_frames(path: &str, frames: &[crate::payloads::Keyframe], value: &Value) {
        let items = value.as_array().unwrap_or_else(|| panic!("{path}"));
        assert_eq!(items.len(), frames.len(), "{path}");
        for (index, (frame, item)) in frames.iter().zip(items).enumerate() {
            expect_frame(&format!("{path}[{index}]"), frame, item);
        }
    }

    fn expect_keyframe(got: &Option<KeyframeMotion>, value: &Value) {
        let Some(motion) = got else {
            assert!(value.is_null(), "keyframe");
            return;
        };
        let mut obj = value.as_object().unwrap().clone();
        expect_f32s(
            "serialized_position",
            &motion.serialized_position,
            &take(&mut obj, "serialized_position"),
        );
        expect_f32s(
            "base_position",
            &motion.base_position,
            &take(&mut obj, "base_position"),
        );
        expect_f32s(
            "base_rotation",
            &motion.base_rotation,
            &take(&mut obj, "base_rotation"),
        );
        expect_frame(
            "current_frame",
            &motion.current_frame,
            &take(&mut obj, "current_frame"),
        );
        expect_frames("frames", &motion.frames, &take(&mut obj, "frames"));
        match &motion.keyframes {
            None => assert!(take(&mut obj, "keyframes").is_null(), "keyframes"),
            Some(frames) => expect_frames("keyframes", frames, &take(&mut obj, "keyframes")),
        }
        expect_i32("mode", motion.mode, &take(&mut obj, "mode"));
        expect_i32("data", motion.data, &take(&mut obj, "data"));
        expect_bool("running", motion.running, &take(&mut obj, "running"));
        expect_i32(
            "iterations",
            motion.iterations,
            &take(&mut obj, "iterations"),
        );
        expect_i32(
            "skip_loops",
            motion.skip_loops,
            &take(&mut obj, "skip_loops"),
        );
        assert!(
            obj.is_empty(),
            "keyframe uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    fn expect_vehicle(got: &Option<Vehicle>, value: &Value) {
        let Some(vehicle) = got else {
            assert!(value.is_null(), "vehicle");
            return;
        };
        let mut obj = value.as_object().unwrap().clone();
        expect_i32("type", vehicle.kind, &take(&mut obj, "type"));
        expect_i32("flags", vehicle.flags, &take(&mut obj, "flags"));
        expect_f32s(
            "linear_motor_direction",
            &vehicle.linear_motor_direction,
            &take(&mut obj, "linear_motor_direction"),
        );
        expect_f32s(
            "linear_friction_timescale",
            &vehicle.linear_friction_timescale,
            &take(&mut obj, "linear_friction_timescale"),
        );
        expect_f32(
            "linear_motor_decay_timescale",
            vehicle.linear_motor_decay_timescale,
            &take(&mut obj, "linear_motor_decay_timescale"),
        );
        expect_f32(
            "linear_motor_timescale",
            vehicle.linear_motor_timescale,
            &take(&mut obj, "linear_motor_timescale"),
        );
        expect_f32s(
            "linear_motor_offset",
            &vehicle.linear_motor_offset,
            &take(&mut obj, "linear_motor_offset"),
        );
        expect_f32s(
            "angular_motor_direction",
            &vehicle.angular_motor_direction,
            &take(&mut obj, "angular_motor_direction"),
        );
        expect_f32(
            "angular_motor_timescale",
            vehicle.angular_motor_timescale,
            &take(&mut obj, "angular_motor_timescale"),
        );
        expect_f32(
            "angular_motor_decay_timescale",
            vehicle.angular_motor_decay_timescale,
            &take(&mut obj, "angular_motor_decay_timescale"),
        );
        expect_f32s(
            "angular_friction_timescale",
            &vehicle.angular_friction_timescale,
            &take(&mut obj, "angular_friction_timescale"),
        );
        for (got, key) in [
            (
                vehicle.angular_deflection_efficiency,
                "angular_deflection_efficiency",
            ),
            (
                vehicle.angular_deflection_timescale,
                "angular_deflection_timescale",
            ),
            (
                vehicle.linear_deflection_efficiency,
                "linear_deflection_efficiency",
            ),
            (
                vehicle.linear_deflection_timescale,
                "linear_deflection_timescale",
            ),
            (vehicle.banking_efficiency, "banking_efficiency"),
            (vehicle.banking_mix, "banking_mix"),
            (vehicle.banking_timescale, "banking_timescale"),
            (vehicle.hover_height, "hover_height"),
            (vehicle.hover_efficiency, "hover_efficiency"),
            (vehicle.hover_timescale, "hover_timescale"),
            (vehicle.buoyancy, "buoyancy"),
            (
                vehicle.vertical_attraction_efficiency,
                "vertical_attraction_efficiency",
            ),
            (
                vehicle.vertical_attraction_timescale,
                "vertical_attraction_timescale",
            ),
            (vehicle.mouselook_azimuth, "mouselook_azimuth"),
            (vehicle.mouselook_altitude, "mouselook_altitude"),
            (vehicle.banking_azimuth, "banking_azimuth"),
            (vehicle.disable_motors_above, "disable_motors_above"),
            (vehicle.disable_motors_after, "disable_motors_after"),
            (
                vehicle.inverted_banking_modifier,
                "inverted_banking_modifier",
            ),
        ] {
            expect_f32(key, got, &take(&mut obj, key));
        }
        expect_f32s(
            "reference_frame",
            &vehicle.reference_frame,
            &take(&mut obj, "reference_frame"),
        );
        for (got, key) in [
            (
                vehicle.linear_motor_timescale_axes,
                "linear_motor_timescale_axes",
            ),
            (
                vehicle.linear_motor_decay_timescale_axes,
                "linear_motor_decay_timescale_axes",
            ),
            (
                vehicle.angular_motor_timescale_axes,
                "angular_motor_timescale_axes",
            ),
            (
                vehicle.angular_motor_decay_timescale_axes,
                "angular_motor_decay_timescale_axes",
            ),
            (vehicle.linear_wind_efficiency, "linear_wind_efficiency"),
            (vehicle.angular_wind_efficiency, "angular_wind_efficiency"),
            (
                vehicle.linear_motor_run_direction,
                "linear_motor_run_direction",
            ),
            (vehicle.linear_motor_run_target, "linear_motor_run_target"),
            (
                vehicle.angular_motor_run_direction,
                "angular_motor_run_direction",
            ),
            (vehicle.angular_motor_run_target, "angular_motor_run_target"),
        ] {
            expect_opt_f32s(
                key,
                got.as_ref().map(|item| item.as_slice()),
                &take(&mut obj, key),
            );
        }
        expect_opt_f32(
            "linear_motor_run_age",
            vehicle.linear_motor_run_age,
            &take(&mut obj, "linear_motor_run_age"),
        );
        expect_opt_f32(
            "angular_motor_run_age",
            vehicle.angular_motor_run_age,
            &take(&mut obj, "angular_motor_run_age"),
        );
        assert!(
            obj.is_empty(),
            "vehicle uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    fn expect_inertia(got: &Option<PhysicsInertia>, value: &Value) {
        let Some(inertia) = got else {
            assert!(value.is_null(), "phys_inertia");
            return;
        };
        let mut obj = value.as_object().unwrap().clone();
        expect_f32("mass", inertia.mass, &take(&mut obj, "mass"));
        expect_f32s("center", &inertia.center, &take(&mut obj, "center"));
        expect_f32s("inertia", &inertia.inertia, &take(&mut obj, "inertia"));
        expect_f32s("rotation", &inertia.rotation, &take(&mut obj, "rotation"));
        assert!(
            obj.is_empty(),
            "phys_inertia uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    fn expect_animations(got: &Option<Vec<ObjectAnimation>>, value: &Value) {
        let Some(list) = got else {
            assert!(value.is_null(), "animations");
            return;
        };
        let items = value.as_array().unwrap_or_else(|| panic!("animations"));
        assert_eq!(items.len(), list.len(), "animations");
        for (index, (animation, item)) in list.iter().zip(items).enumerate() {
            let mut obj = item.as_object().unwrap().clone();
            expect_uuid(
                &format!("animations[{index}].id"),
                animation.id,
                &take(&mut obj, "id"),
            );
            assert_eq!(
                obj.remove("name")
                    .and_then(|name| name.as_str().map(str::to_owned)),
                Some(animation.name.clone()),
                "animations[{index}].name"
            );
            assert!(obj.is_empty(), "animations[{index}] {obj:?}");
        }
    }

    fn expect_linkset(got: &Option<LinksetData>, value: &Value) {
        let Some(linkset) = got else {
            assert!(value.is_null(), "linkset");
            return;
        };
        let mut obj = value.as_object().unwrap().clone();
        match &linkset.entries {
            None => assert!(take(&mut obj, "entries").is_null(), "entries"),
            Some(entries) => {
                let items = take(&mut obj, "entries");
                let items = items.as_array().unwrap_or_else(|| panic!("entries"));
                assert_eq!(items.len(), entries.len(), "entries");
                for (index, (entry, item)) in entries.iter().zip(items).enumerate() {
                    let mut row = item.as_object().unwrap().clone();
                    assert_eq!(
                        row.remove("key")
                            .and_then(|key| key.as_str().map(str::to_owned)),
                        Some(entry.key.clone()),
                        "entries[{index}].key"
                    );
                    expect_opt_text(
                        &format!("entries[{index}].value"),
                        &entry.value,
                        &take(&mut row, "value"),
                    );
                    expect_opt_text(
                        &format!("entries[{index}].password"),
                        &entry.password,
                        &take(&mut row, "password"),
                    );
                    assert!(row.is_empty(), "entries[{index}] {row:?}");
                }
            }
        }
        expect_i32(
            "bytes_used",
            linkset.bytes_used,
            &take(&mut obj, "bytes_used"),
        );
        expect_i32(
            "bytes_free",
            linkset.bytes_free,
            &take(&mut obj, "bytes_free"),
        );
        assert!(
            obj.is_empty(),
            "linkset uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    fn expect_media(got: &Option<Vec<crate::payloads::MediaFace>>, value: &Value) {
        let Some(faces) = got else {
            assert!(value.is_null(), "media_entries");
            return;
        };
        let items = value.as_array().unwrap_or_else(|| panic!("media_entries"));
        assert_eq!(items.len(), faces.len(), "media_entries");
        for (index, (face, item)) in faces.iter().zip(items).enumerate() {
            let mut obj = item.as_object().unwrap().clone();
            let path = format!("media[{index}]");
            expect_bool(
                &format!("{path}.alt_image_enable"),
                face.alt_image_enable,
                &take(&mut obj, "alt_image_enable"),
            );
            expect_bool(
                &format!("{path}.auto_loop"),
                face.auto_loop,
                &take(&mut obj, "auto_loop"),
            );
            expect_bool(
                &format!("{path}.auto_play"),
                face.auto_play,
                &take(&mut obj, "auto_play"),
            );
            expect_bool(
                &format!("{path}.auto_scale"),
                face.auto_scale,
                &take(&mut obj, "auto_scale"),
            );
            expect_bool(
                &format!("{path}.auto_zoom"),
                face.auto_zoom,
                &take(&mut obj, "auto_zoom"),
            );
            expect_i32(
                &format!("{path}.controls"),
                face.controls,
                &take(&mut obj, "controls"),
            );
            expect_opt_text(
                &format!("{path}.current_url"),
                &face.current_url,
                &take(&mut obj, "current_url"),
            );
            expect_bool(
                &format!("{path}.first_click_interact"),
                face.first_click_interact,
                &take(&mut obj, "first_click_interact"),
            );
            expect_i32(
                &format!("{path}.height_pixels"),
                face.height_pixels,
                &take(&mut obj, "height_pixels"),
            );
            expect_opt_text(
                &format!("{path}.home_url"),
                &face.home_url,
                &take(&mut obj, "home_url"),
            );
            expect_i32(
                &format!("{path}.perms_control"),
                face.perms_control,
                &take(&mut obj, "perms_control"),
            );
            expect_i32(
                &format!("{path}.perms_interact"),
                face.perms_interact,
                &take(&mut obj, "perms_interact"),
            );
            match &face.whitelist {
                None => assert!(take(&mut obj, "whitelist").is_null(), "{path}.whitelist"),
                Some(list) => {
                    let dumped = take(&mut obj, "whitelist");
                    let dumped = dumped.as_array().unwrap();
                    let texts: Vec<&str> =
                        dumped.iter().map(|item| item.as_str().unwrap()).collect();
                    assert_eq!(
                        texts,
                        list.iter().map(String::as_str).collect::<Vec<_>>(),
                        "{path}.whitelist"
                    );
                }
            }
            expect_bool(
                &format!("{path}.whitelist_enable"),
                face.whitelist_enable,
                &take(&mut obj, "whitelist_enable"),
            );
            expect_i32(
                &format!("{path}.width_pixels"),
                face.width_pixels,
                &take(&mut obj, "width_pixels"),
            );
            assert!(obj.is_empty(), "{path} {obj:?}");
        }
    }

    fn expect_materials(got: &Option<crate::payloads::RenderMaterials>, value: &Value) {
        let Some(materials) = got else {
            assert!(value.is_null(), "materials");
            return;
        };
        let mut obj = value.as_object().unwrap().clone();
        match &materials.overrides {
            None => assert!(take(&mut obj, "overrides").is_null(), "overrides"),
            Some(overrides) => {
                let items = take(&mut obj, "overrides");
                let items = items.as_array().unwrap();
                assert_eq!(items.len(), overrides.len(), "overrides");
                for (index, (entry, item)) in overrides.iter().zip(items).enumerate() {
                    let mut row = item.as_object().unwrap().clone();
                    expect_i32(
                        &format!("materials[{index}].te_index"),
                        i32::from(entry.te_index),
                        &take(&mut row, "te_index"),
                    );
                    assert_eq!(
                        row.remove("data")
                            .and_then(|data| data.as_str().map(str::to_owned)),
                        Some(entry.data.clone()),
                        "materials[{index}].data"
                    );
                    assert!(row.is_empty(), "materials[{index}] {row:?}");
                }
            }
        }
        assert!(obj.is_empty(), "materials {obj:?}");
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
        assert_eq!(
            part.allow_unsit,
            take(&mut obj, "allow_unsit").as_bool().unwrap(),
            "{label} allow_unsit"
        );
        assert_eq!(
            part.scripted_sit_only,
            take(&mut obj, "scripted_sit_only").as_bool().unwrap(),
            "{label} scripted_sit_only"
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
        assert_eq!(
            f64::from(gain).to_bits(),
            part.sound_gain.to_bits(),
            "sound_gain"
        );
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
        expect_dyn_attrs(&part.dyn_attrs, &take(&mut obj, "dyn_attrs"));
        expect_keyframe(&part.keyframe, &take(&mut obj, "keyframe"));
        expect_vehicle(&part.vehicle, &take(&mut obj, "vehicle"));
        expect_inertia(&part.phys_inertia, &take(&mut obj, "phys_inertia"));
        expect_animations(&part.animations, &take(&mut obj, "animations"));
        expect_linkset(&part.linkset, &take(&mut obj, "linkset"));
        expect_items(&part.items, &take(&mut obj, "items"));
        assert!(
            obj.is_empty(),
            "{label} uncompared {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    fn expect_items(items: &[InventoryItem], value: &Value) {
        let dumped = value.as_array().unwrap_or_else(|| panic!("items"));
        assert_eq!(dumped.len(), items.len(), "items");
        for (index, (item, dumped_item)) in items.iter().zip(dumped).enumerate() {
            let mut obj = dumped_item.as_object().unwrap().clone();
            let path = format!("items[{index}]");
            expect_uuid(&path, item.item_id, &take(&mut obj, "item_id"));
            expect_uuid(
                &path,
                item.parent_part_id,
                &take(&mut obj, "parent_part_id"),
            );
            expect_uuid(&path, item.asset_id, &take(&mut obj, "asset_id"));
            expect_uuid(&path, item.parent_id, &take(&mut obj, "parent_id"));
            assert_eq!(
                i64::from(item.inv_type),
                json_i64(&take(&mut obj, "inv_type"), "inv_type"),
                "{path}"
            );
            assert_eq!(
                i64::from(item.asset_type),
                json_i64(&take(&mut obj, "asset_type"), "asset_type"),
                "{path}"
            );
            assert_eq!(
                item.name,
                take(&mut obj, "name").as_str().unwrap(),
                "{path}"
            );
            assert_eq!(
                item.description,
                take(&mut obj, "description").as_str().unwrap(),
                "{path}"
            );
            assert_eq!(
                u64::from(item.creation_date),
                json_i64(&take(&mut obj, "creation_date"), "creation_date") as u64,
                "{path}"
            );
            assert_eq!(
                item.creator_id,
                take(&mut obj, "creator_id").as_str().unwrap(),
                "{path}"
            );
            expect_uuid(&path, item.owner_id, &take(&mut obj, "owner_id"));
            expect_uuid(&path, item.last_owner_id, &take(&mut obj, "last_owner_id"));
            expect_uuid(&path, item.group_id, &take(&mut obj, "group_id"));
            for (got, key) in [
                (item.next_permissions, "next_permissions"),
                (item.current_permissions, "current_permissions"),
                (item.base_permissions, "base_permissions"),
                (item.everyone_permissions, "everyone_permissions"),
                (item.group_permissions, "group_permissions"),
                (item.flags, "flags"),
            ] {
                assert_eq!(
                    u64::from(got),
                    json_i64(&take(&mut obj, key), key) as u64,
                    "{path}.{key}"
                );
            }
            expect_uuid(&path, item.experience_id, &take(&mut obj, "experience_id"));
            assert!(
                obj.is_empty(),
                "{path} uncompared {:?}",
                obj.keys().collect::<Vec<_>>()
            );
        }
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
        let texture = take(&mut obj, "texture");
        if texture.is_null() {
            assert_eq!(shape.texture, DEFAULT_BOX_TEXTURE, "texture");
        } else {
            expect_bytes("texture", &shape.texture, &texture);
        }
        let extra = take(&mut obj, "extra_params");
        if extra.is_null() {
            assert_eq!(shape.extra_params, vec![0], "extra_params");
        } else {
            expect_bytes("extra_params", &shape.extra_params, &extra);
        }
        expect_media(&shape.media_entries, &take(&mut obj, "media_entries"));
        expect_materials(&shape.materials, &take(&mut obj, "materials"));
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
        assert_eq!(dump["pin"], "41f18f983bb2bfc82e033fed410d361ba57d5aa4");
        assert_eq!(dump["region"], REGION);
        let store = CopiedStore::new("regionstore/objects.db");
        let groups = open_objects(
            &store.connection(),
            Uuid::parse_str(REGION).unwrap(),
            ObjectStoreKind::Sqlite,
        )
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
        assert!(!ids.contains(&Uuid::parse_str("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1").unwrap()));
        assert_eq!(groups[0].parts[0].name, "showcase");
        assert!(groups[0].parts[0].allow_unsit);
        assert!(!groups[0].parts[0].scripted_sit_only);
        assert!(groups[0].parts[0].xml_creator_data.is_none());
        assert_eq!(groups[0].parts[0].link_num, 0);
        assert_eq!(groups[0].parts[0].flags, 0x4401);
        assert_eq!(groups[0].parts[0].pay_price, [10, 20, 30, 40, 50]);
        assert!(!groups[0].parts[0].sit_target_active);
        assert_eq!(groups[0].parts[0].shape.extra_params, vec![0x00, 0xaa]);
        assert_eq!(groups[0].rez_parameter.as_deref(), Some("rez-arg"));
        assert_eq!(
            groups[0].parts[0].creator_id,
            "00000000-0000-0000-0000-000000000004;https://host/agent/;Unknown User"
        );
        assert_eq!(
            groups[1]
                .parts
                .iter()
                .map(|part| (part.name.as_str(), part.link_num))
                .collect::<Vec<_>>(),
            vec![("link-restore", 1), ("child-three", 3)]
        );
        assert_eq!(
            groups[1].parts[0].creator_id,
            "00000000-0000-0000-0000-000000000004;https://host/;Name"
        );
        assert!(!ids.contains(&Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2").unwrap()));
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
        let showcase = &groups[0].parts[0];
        assert!(
            showcase
                .dyn_attrs
                .as_ref()
                .unwrap()
                .namespaces
                .contains_key("kept")
        );
        assert!(showcase.keyframe.is_some());
        assert_eq!(showcase.vehicle.as_ref().unwrap().kind, 2);
        assert_eq!(
            showcase.phys_inertia.as_ref().unwrap().mass.to_bits(),
            2.5f32.to_bits()
        );
        assert_eq!(showcase.animations.as_ref().unwrap().len(), 1);
        assert_eq!(showcase.linkset.as_ref().unwrap().bytes_used, 36);
        assert_eq!(showcase.shape.media_entries.as_ref().unwrap().len(), 2);
        assert!(
            showcase
                .shape
                .materials
                .as_ref()
                .unwrap()
                .overrides
                .is_some()
        );
        let sibling = &groups[1].parts[1];
        assert!(sibling.items.is_empty());
        assert_eq!(
            showcase
                .items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["note", "script", "box"]
        );
        assert_eq!(showcase.items[0].description, "");
        assert_eq!(showcase.items[0].experience_id, Uuid::nil());
        assert_eq!(
            showcase.items[1].creator_id,
            "00000000-0000-0000-0000-000000000004;https://profile.example/agent;Unknown User"
        );
        assert_eq!(
            showcase.items[2].creator_id,
            "00000000-0000-0000-0000-000000000004;https://profile.example/agent;Ada Lovelace"
        );
        assert_ne!(showcase.items[2].experience_id, Uuid::nil());
        assert!(groups[1].parts[0].items.is_empty());
        assert_eq!(groups[3].parts[0].items.len(), 1);
        assert_eq!(
            groups[3].parts[0].items[0].creator_id,
            "00000000-0000-0000-0000-000000000000"
        );
        let loaded: Vec<Uuid> = groups
            .iter()
            .flat_map(|group| group.parts.iter())
            .flat_map(|part| part.items.iter().map(|item| item.item_id))
            .collect();
        assert!(
            !loaded.contains(&Uuid::parse_str("f0000000-0000-0000-0000-000000000001").unwrap())
        );
        assert!(
            !loaded.contains(&Uuid::parse_str("e0000000-0000-0000-0000-000000000001").unwrap())
        );
        assert!(
            !loaded.contains(&Uuid::parse_str("b2222222-2222-2222-2222-222222222222").unwrap())
        );
    }

    #[tokio::test]
    async fn regionstore_unreadable_inventory_row_is_left_out() {
        let store = CopiedStore::new("regionstore/objects.db");
        let connection = store.connection();
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        sqlx::query(
            "INSERT INTO primitems (itemID, primID, assetID, parentFolderID, invType, assetType, \
             name, description, creationDate, creatorID, ownerID, lastOwnerID, groupID, \
             nextPermissions, currentPermissions, basePermissions, everyonePermissions, \
             groupPermissions, flags, experienceID) VALUES \
             ('d1000000-0000-0000-0000-000000000001', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'kept', '', 1700000200, '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, 0, \
             '00000000-0000-0000-0000-000000000000'), \
             ('d1000000-0000-0000-0000-000000000002', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, NULL, '', 1700000201, '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, 0, \
             '00000000-0000-0000-0000-000000000000')",
        )
        .execute(sqlite)
        .await
        .unwrap();
        let groups = load_objects(&pool, Uuid::parse_str(REGION).unwrap())
            .await
            .unwrap();
        let items = &groups[3].parts[0].items;
        assert!(items.iter().any(|item| item.name == "kept"));
        assert!(
            items.iter().all(|item| item.item_id
                != Uuid::parse_str("d1000000-0000-0000-0000-000000000002").unwrap())
        );
        assert_eq!(groups[3].parts[0].name, "no-shape");
        pool.close().await;
    }

    /// Description, SitName and TouchName follow the part setters at load.
    /// Name and an inventory item's description stay whole.
    #[tokio::test]
    async fn a_loaded_prim_cuts_description_sit_and_touch_once() {
        let store = CopiedStore::new("regionstore/objects.db");
        let connection = store.connection();
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        let name = "N".repeat(80);
        let description = "a".repeat(200);
        let sit = "s".repeat(40);
        let touch = "t".repeat(50);
        let item_text = "i".repeat(200);
        sqlx::query(
            "UPDATE prims SET Name = ?, Description = ?, SitName = ?, TouchName = ? \
             WHERE Name = 'no-shape'",
        )
        .bind(&name)
        .bind(&description)
        .bind(sit)
        .bind(touch)
        .execute(sqlite)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE primitems SET description = ? \
             WHERE primID IN (SELECT UUID FROM prims WHERE Name = ?)",
        )
        .bind(&item_text)
        .bind(&name)
        .execute(sqlite)
        .await
        .unwrap();
        let groups = load_objects(&pool, Uuid::parse_str(REGION).unwrap())
            .await
            .unwrap();
        let part = groups
            .iter()
            .flat_map(|group| group.parts.iter())
            .find(|part| part.name == name)
            .unwrap();
        assert_eq!(part.name.len(), 80);
        assert_eq!(part.description, "a".repeat(DESCRIPTION_LIMIT));
        assert_eq!(part.sit_name.len(), SIT_LIMIT);
        assert_eq!(part.touch_name.len(), TOUCH_LIMIT);
        assert!(part.items.iter().any(|item| item.description == item_text));
        pool.close().await;
    }

    #[test]
    fn part_creator_identification_follows_the_part_rule() {
        let zero = "00000000-0000-0000-0000-000000000000";
        let id = "00000000-0000-0000-0000-000000000004";
        assert_eq!(part_creator_identification(""), zero);
        assert_eq!(part_creator_identification(id), id);
        assert_eq!(
            part_creator_identification(&format!("{id};https://profile.example/agent")),
            format!("{id};https://profile.example/agent/;Unknown User")
        );
        assert_eq!(
            part_creator_identification(&format!("{id};https://profile.example/agent/;Ada")),
            format!("{id};https://profile.example/agent/;Ada")
        );
        assert_eq!(
            part_creator_identification(&format!("{id};https://example.com;Ada;extra")),
            format!("{id};https://example.com/;Ada")
        );
        assert_eq!(
            part_creator_identification(&format!("{id};")),
            format!("{id};/;Unknown User")
        );
        assert_eq!(part_creator_identification("not-a-uuid"), zero);
        assert_eq!(
            part_creator_identification("not-a-uuid;http://x"),
            format!("{zero};http://x/;Unknown User")
        );
        assert_eq!(
            creator_identification(&format!("{id};https://profile.example/agent")),
            format!("{id};https://profile.example/agent;Unknown User")
        );
        assert_eq!(
            creator_identification(&format!("{id};")),
            format!("{id};;Unknown User")
        );
    }

    #[test]
    fn the_task_item_cap_refuses_the_next_row() {
        let mut counts = HashMap::new();
        let prim = Uuid::nil();
        for _ in 0..MAX_TASK_ITEMS {
            note_task_item(&mut counts, prim).unwrap();
        }
        match note_task_item(&mut counts, prim) {
            Err(ObjectError::TaskItems {
                configured,
                observed,
            }) => {
                assert_eq!(configured, MAX_TASK_ITEMS);
                assert_eq!(observed, MAX_TASK_ITEMS + 1);
            }
            other => panic!("task items: {other:?}"),
        }
        let mut region_items = 0u64;
        match note_region_items(&mut region_items, 0) {
            Err(ObjectError::TaskItems {
                configured,
                observed,
            }) => {
                assert_eq!(configured, 0);
                assert_eq!(observed, 1);
            }
            other => panic!("region items: {other:?}"),
        }
    }

    #[tokio::test]
    async fn regionstore_streams_this_region_only() {
        let store = CopiedStore::new("regionstore/objects.db");
        let connection = store.connection();
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        let creator = "00000000-0000-0000-0000-000000000004;https://profile.example/agent";
        sqlx::query("UPDATE prims SET CreatorID = ? WHERE Name = 'showcase'")
            .bind(creator)
            .execute(sqlite)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE prims SET RegionUUID = '11111111-1111-1111-1111-111111111111' \
             WHERE Name = 'showcase'",
        )
        .execute(sqlite)
        .await
        .unwrap();
        sqlx::query("UPDATE prims SET RegionUUID = upper(RegionUUID) WHERE Name = 'link-restore'")
            .execute(sqlite)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE primshapes SET UUID = upper(UUID) \
             WHERE UUID = 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1'",
        )
        .execute(sqlite)
        .await
        .unwrap();
        sqlx::query("UPDATE prims SET pseudocrc = NULL WHERE Name = 'no-shape'")
            .execute(sqlite)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE prims SET SceneGroupID = '00000000-0000-0000-0000-000000000000' \
             WHERE Name = 'child-two'",
        )
        .execute(sqlite)
        .await
        .unwrap();
        let groups = load_objects(&pool, Uuid::parse_str(REGION).unwrap())
            .await
            .unwrap();
        let showcase = groups
            .iter()
            .flat_map(|group| group.parts.iter())
            .find(|part| part.name == "showcase")
            .unwrap();
        assert_eq!(
            showcase.creator_id,
            "00000000-0000-0000-0000-000000000004;https://profile.example/agent/;Unknown User"
        );
        assert!(!showcase.shape.default_box);
        assert!(groups.iter().all(|group| {
            group
                .parts
                .iter()
                .all(|part| part.name != "no-shape" && part.name != "child-two")
        }));
        assert!(
            groups
                .iter()
                .any(|group| { group.parts.iter().any(|part| part.name == "link-restore") })
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn regionstore_root_test_compares_the_stored_text() {
        let store = CopiedStore::new("regionstore/objects.db");
        let connection = store.connection();
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        sqlx::query("UPDATE prims SET UUID = upper(UUID) WHERE Name = 'link-restore'")
            .execute(sqlite)
            .await
            .unwrap();
        let groups = load_objects(&pool, Uuid::parse_str(REGION).unwrap())
            .await
            .unwrap();
        assert!(groups.iter().all(|group| {
            group.parts.iter().all(|part| {
                part.name != "link-restore"
                    && part.name != "child-three"
                    && part.name != "child-two"
            })
        }));
        assert!(
            groups
                .iter()
                .any(|group| group.parts.iter().any(|part| part.name == "showcase"))
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn regionstore_limits_refuse_a_smaller_cap() {
        let store = CopiedStore::new("regionstore/objects.db");
        let connection = store.connection();
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let region = Uuid::parse_str(REGION).unwrap();
        let objects = load_objects_limited(&pool, region, 1, MAX_LINKSET_PRIMS).await;
        match objects {
            Err(ObjectError::Objects {
                configured: 1,
                observed: 2,
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
        pool.close().await;
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
        let groups = open_objects(
            &connection,
            Uuid::parse_str(REGION).unwrap(),
            ObjectStoreKind::Sqlite,
        )
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
        let store = CopiedStore::new("regionstore/objects.db");
        let connection = store.connection();
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

    fn mysql_cross_connection() -> Option<String> {
        if std::env::var("FERROAN_REGIONSTORE_CROSS").ok().as_deref() != Some("1") {
            return None;
        }
        Some(std::env::var("FERROAN_REGIONSTORE_MYSQL").unwrap())
    }

    fn mysql_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        &LOCK
    }

    fn mysql_dump() -> Value {
        let text = std::fs::read_to_string(fixture_dir().join("objects-mysql.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn expect_groups(groups: &[ObjectGroup], dump: &Value) {
        let dumped = dump["groups"].as_array().unwrap();
        assert_eq!(groups.len(), dumped.len());
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
            }
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn regionstore_mysql_load_matches_the_committed_dump() {
        let Some(connection) = mysql_cross_connection() else {
            eprintln!("regionstore mysql load skipped");
            return;
        };
        let _guard = mysql_test_lock().lock().unwrap();
        let dump = mysql_dump();
        assert_eq!(dump["pin"], "41f18f983bb2bfc82e033fed410d361ba57d5aa4");
        assert_eq!(dump["region"], REGION);
        let groups = open_objects(
            &connection,
            Uuid::parse_str(REGION).unwrap(),
            ObjectStoreKind::Mysql,
        )
        .await
        .unwrap();
        expect_groups(&groups, &dump);
        let mut ids = BTreeSet::new();
        for group in &groups {
            for part in &group.parts {
                ids.insert(part.uuid);
            }
        }
        assert!(!ids.contains(&Uuid::parse_str("99999999-9999-9999-9999-999999999992").unwrap()));
        assert!(!ids.contains(&Uuid::parse_str("ffffffff-ffff-ffff-ffff-fffffffffff1").unwrap()));
        let showcase_group = groups
            .iter()
            .find(|group| group.parts[0].name == "showcase")
            .unwrap();
        let showcase = &showcase_group.parts[0];
        for xml in [
            crate::to_xml2(showcase_group),
            crate::to_original_xml(showcase_group),
        ] {
            assert!(
                xml.contains(
                    "<AllowUnsit>false</AllowUnsit><ScriptedSitOnly>true</ScriptedSitOnly>"
                )
            );
        }
        assert_eq!(showcase.flags, 0x401);
        assert!(!showcase.volume_detect);
        assert!(!showcase.allow_unsit);
        assert!(showcase.scripted_sit_only);
        assert_eq!(
            showcase.creator_id,
            "00000000-0000-0000-0000-000000000004;https://host/agent/;Unknown User"
        );
        assert_eq!(
            showcase
                .items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["script", "box", "note"]
        );
        let null_prim = groups
            .iter()
            .flat_map(|group| group.parts.iter())
            .find(|part| {
                part.uuid == Uuid::parse_str("99999999-9999-9999-9999-999999999991").unwrap()
            })
            .unwrap();
        assert!(!null_prim.shape.default_box);
        assert_eq!(null_prim.shape.scale, [0.0, 0.0, 0.0]);
        assert_eq!(null_prim.shape.texture, DEFAULT_BOX_TEXTURE);
        assert_eq!(null_prim.shape.extra_params, vec![0]);
        assert_eq!(null_prim.sale_price, 10);
        assert_eq!(null_prim.material, 3);
        assert!(!null_prim.sit_target_active);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn regionstore_mysql_unreadable_rows_are_left_out() {
        let Some(connection) = mysql_cross_connection() else {
            eprintln!("regionstore mysql unreadable rows skipped");
            return;
        };
        let _guard = mysql_test_lock().lock().unwrap();
        let pool = Pool::open(Backend::Mysql, &connection, &["RegionStore"])
            .await
            .unwrap();
        let mysql = pool.mysql().unwrap();
        let region = Uuid::parse_str(REGION).unwrap();
        reset_mysql_extras(mysql).await;
        sqlx::query(
            "INSERT INTO primitems (itemID, primID, assetID, parentFolderID, invType, assetType, \
             name, description, creationDate, creatorID, ownerID, lastOwnerID, groupID, \
             nextPermissions, currentPermissions, basePermissions, everyonePermissions, \
             groupPermissions, flags, experienceID) VALUES \
             ('d1000000-0000-0000-0000-000000000001', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'kept', '', 1700000200, '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, 1, \
             '00000000-0000-0000-0000-000000000000'), \
             ('d1000000-0000-0000-0000-000000000002', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'dropped', '', 1700000201, '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, -1, \
             '00000000-0000-0000-0000-000000000000'), \
             ('d1000000-0000-0000-0000-000000000003', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'wrapped', '', 1700000202, '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', -1, 1, 1, 0, 0, 1, \
             '00000000-0000-0000-0000-000000000000'), \
             ('d1000000-0000-0000-0000-000000000004', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'dated', '', NULL, '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, 1, \
             '00000000-0000-0000-0000-000000000000')",
        )
        .execute(mysql)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO prims (UUID, RegionUUID, SceneGroupID, DynAttrs) VALUES \
             ('eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1', '11111111-1111-1111-1111-111111111111', \
             'eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1', '<llsd><integer>1</integer></llsd>')",
        )
        .execute(mysql)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE primitems SET experienceID = 'nope' WHERE itemID = \
             'c0000000-0000-0000-0000-000000000003'",
        )
        .execute(mysql)
        .await
        .unwrap();
        let loaded = load_objects(&pool, region).await;
        reset_mysql_extras(mysql).await;
        let groups = loaded.unwrap();
        let no_shape = groups
            .iter()
            .find_map(|group| group.parts.iter().find(|part| part.name == "no-shape"))
            .unwrap();
        assert!(no_shape.items.iter().any(|item| item.name == "kept"));
        assert!(no_shape.items.iter().all(|item| {
            item.item_id != Uuid::parse_str("d1000000-0000-0000-0000-000000000002").unwrap()
                && item.name != "wrapped"
        }));
        let dated = no_shape
            .items
            .iter()
            .find(|item| item.name == "dated")
            .unwrap();
        assert_eq!(dated.creation_date, 0);
        let note = groups
            .iter()
            .find_map(|group| {
                group
                    .parts
                    .iter()
                    .find(|part| part.name == "showcase")
                    .and_then(|part| part.items.iter().find(|item| item.name == "note"))
            })
            .unwrap();
        assert_eq!(note.experience_id, Uuid::nil());
        assert!(groups.iter().all(|group| {
            group.parts.iter().all(|part| {
                part.uuid != Uuid::parse_str("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1").unwrap()
            })
        }));
        assert!(groups.len() >= 5);
    }

    async fn reset_mysql_extras(mysql: &sqlx::MySqlPool) {
        sqlx::query(
            "DELETE FROM primitems WHERE itemID IN \
             ('d1000000-0000-0000-0000-000000000001', 'd1000000-0000-0000-0000-000000000002', \
              'd1000000-0000-0000-0000-000000000003', 'd1000000-0000-0000-0000-000000000004')",
        )
        .execute(mysql)
        .await
        .unwrap();
        sqlx::query("DELETE FROM prims WHERE UUID = 'eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1'")
            .execute(mysql)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE primitems SET experienceID = '00000000-0000-0000-0000-000000000000' \
             WHERE itemID = 'c0000000-0000-0000-0000-000000000003'",
        )
        .execute(mysql)
        .await
        .unwrap();
    }

    fn postgres_cross_connection() -> Option<String> {
        if std::env::var("FERROAN_REGIONSTORE_CROSS").ok().as_deref() != Some("1") {
            return None;
        }
        std::env::var("FERROAN_REGIONSTORE_POSTGRES").ok()
    }

    fn postgres_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        &LOCK
    }

    fn without_dropped_root(dump: &Value) -> Value {
        let mut comparable = dump.clone();
        let kept = comparable["groups"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|group| {
                group["parts"].as_array().unwrap().iter().all(|part| {
                    !matches!(
                        part["name"].as_str(),
                        Some("rewrite-root" | "rewrite-child" | "zero-group")
                    )
                })
            })
            .cloned()
            .collect();
        comparable["groups"] = Value::Array(kept);
        comparable
    }

    fn postgres_dump() -> Value {
        let text = std::fs::read_to_string(fixture_dir().join("objects-postgres.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn regionstore_postgres_load_matches_the_committed_dump() {
        let Some(connection) = postgres_cross_connection() else {
            eprintln!("regionstore postgres load skipped");
            return;
        };
        let _guard = postgres_test_lock().lock().unwrap();
        let dump = postgres_dump();
        assert_eq!(dump["pin"], "41f18f983bb2bfc82e033fed410d361ba57d5aa4");
        assert_eq!(dump["region"], REGION);
        let groups = open_objects(
            &connection,
            Uuid::parse_str(REGION).unwrap(),
            ObjectStoreKind::Postgres,
        )
        .await
        .unwrap();
        expect_groups(&groups, &without_dropped_root(&dump));
        assert!(groups.iter().all(|group| {
            group.parts.iter().all(|part| {
                part.name != "rewrite-root"
                    && part.name != "rewrite-child"
                    && part.name != "zero-group"
            })
        }));
        let showcase = groups
            .iter()
            .flat_map(|group| group.parts.iter())
            .find(|part| part.name == "showcase")
            .unwrap();
        assert_eq!(showcase.flags, 0x401);
        assert!(!showcase.volume_detect);
        assert!(showcase.keyframe.is_none());
        assert_eq!(showcase.shape.last_attach_point, 0);
        assert_eq!(
            showcase
                .items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["note", "script", "box"]
        );
        assert!(
            showcase
                .items
                .iter()
                .all(|item| item.parent_id == showcase.uuid)
        );
        let second = groups
            .iter()
            .flat_map(|group| group.parts.iter())
            .find(|part| part.name == "child-second")
            .unwrap();
        assert_eq!(second.link_num, 3);
        assert_eq!(
            showcase.creator_id,
            "00000000-0000-0000-0000-000000000004;https://host/agent/;Unknown User"
        );
        // A zero scene-group id makes the pin throw, so the oracle seed leaves
        // the row out. Insert it here and keep every other group.
        let pool = Pool::open(Backend::Postgres, &connection, &["RegionStore"])
            .await
            .unwrap();
        let postgres = pool.postgres().unwrap();
        let inserted = sqlx::query(
            "INSERT INTO prims SELECT r.* FROM prims p \
             CROSS JOIN LATERAL jsonb_populate_record(NULL::prims, \
             to_jsonb(p) || jsonb_build_object( \
             'UUID', '66666666-6666-6666-6666-666666666661'::uuid, \
             'SceneGroupID', '00000000-0000-0000-0000-000000000000'::uuid, \
             'Name', 'zero-group')) AS r \
             WHERE p.\"UUID\" = 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1'::uuid",
        )
        .execute(postgres)
        .await
        .unwrap();
        assert_eq!(inserted.rows_affected(), 1u64);
        let shaped = sqlx::query(
            "INSERT INTO primshapes SELECT r.* FROM primshapes p \
             CROSS JOIN LATERAL jsonb_populate_record(NULL::primshapes, \
             to_jsonb(p) || jsonb_build_object( \
             'UUID', '66666666-6666-6666-6666-666666666661'::uuid)) AS r \
             WHERE p.\"UUID\" = 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1'::uuid",
        )
        .execute(postgres)
        .await
        .unwrap();
        assert_eq!(shaped.rows_affected(), 1u64);
        let again = open_objects(
            &connection,
            Uuid::parse_str(REGION).unwrap(),
            ObjectStoreKind::Postgres,
        )
        .await
        .unwrap();
        let zero_group = Uuid::parse_str("66666666-6666-6666-6666-666666666661").unwrap();
        assert_eq!(again.len(), groups.len());
        assert!(again.iter().all(|group| {
            group
                .parts
                .iter()
                .all(|part| part.name != "zero-group" && part.uuid != zero_group)
        }));
        sqlx::query(
            "DELETE FROM primshapes WHERE \"UUID\" = '66666666-6666-6666-6666-666666666661'",
        )
        .execute(postgres)
        .await
        .unwrap();
        sqlx::query("DELETE FROM prims WHERE \"UUID\" = '66666666-6666-6666-6666-666666666661'")
            .execute(postgres)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn regionstore_postgres_unreadable_rows_are_left_out() {
        let Some(connection) = postgres_cross_connection() else {
            eprintln!("regionstore postgres unreadable rows skipped");
            return;
        };
        let _guard = postgres_test_lock().lock().unwrap();
        let pool = Pool::open(Backend::Postgres, &connection, &["RegionStore"])
            .await
            .unwrap();
        let postgres = pool.postgres().unwrap();
        let region = Uuid::parse_str(REGION).unwrap();
        reset_postgres_extras(postgres).await;
        sqlx::query(
            "INSERT INTO prims SELECT r.* FROM prims p \
             CROSS JOIN LATERAL jsonb_populate_record(NULL::prims, \
             to_jsonb(p) || jsonb_build_object( \
             'UUID', '77777777-7777-7777-7777-777777777771'::uuid, \
             'SceneGroupID', '77777777-7777-7777-7777-777777777771'::uuid, \
             'Name', NULL)) AS r \
             WHERE p.\"UUID\" = 'dddddddd-dddd-dddd-dddd-ddddddddddd1'::uuid",
        )
        .execute(postgres)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO prims SELECT r.* FROM prims p \
             CROSS JOIN LATERAL jsonb_populate_record(NULL::prims, \
             to_jsonb(p) || jsonb_build_object( \
             'UUID', '77777777-7777-7777-7777-777777777772'::uuid, \
             'SceneGroupID', '77777777-7777-7777-7777-777777777772'::uuid, \
             'CreationDate', NULL)) AS r \
             WHERE p.\"UUID\" = 'dddddddd-dddd-dddd-dddd-ddddddddddd1'::uuid",
        )
        .execute(postgres)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO prims SELECT r.* FROM prims p \
             CROSS JOIN LATERAL jsonb_populate_record(NULL::prims, \
             to_jsonb(p) || jsonb_build_object( \
             'UUID', 'eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1'::uuid, \
             'SceneGroupID', 'eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1'::uuid, \
             'DynAttrs', '<llsd><integer>1</integer></llsd>')) AS r \
             WHERE p.\"UUID\" = 'dddddddd-dddd-dddd-dddd-ddddddddddd1'::uuid",
        )
        .execute(postgres)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO primitems (\"itemID\", \"primID\", \"assetID\", \"parentFolderID\", \
             \"invType\", \"assetType\", \"name\", \"description\", \"creationDate\", \"creatorID\", \
             \"ownerID\", \"lastOwnerID\", \"groupID\", \"nextPermissions\", \"currentPermissions\", \
             \"basePermissions\", \"everyonePermissions\", \"groupPermissions\", \"flags\") VALUES \
             ('d1000000-0000-0000-0000-000000000001', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'kept', '', '1700000200', '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, 0), \
             ('d1000000-0000-0000-0000-000000000002', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'dropped', NULL, '1700000201', '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, 0), \
             ('d1000000-0000-0000-0000-000000000003', 'dddddddd-dddd-dddd-dddd-ddddddddddd1', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'negative', '', '1700000202', '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, -1), \
             ('d1000000-0000-0000-0000-000000000004', '88888888-8888-8888-8888-888888888881', \
             '00000000-0000-0000-0000-000000000010', '00000000-0000-0000-0000-000000000011', \
             7, 7, 'rewrite-note', '', '1700000203', '00000000-0000-0000-0000-000000000004', \
             '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000003', 1, 1, 1, 0, 0, 0)",
        )
        .execute(postgres)
        .await
        .unwrap();
        let loaded = load_objects(&pool, region).await;
        reset_postgres_extras(postgres).await;
        let groups = loaded.unwrap();
        assert!(groups.iter().all(|group| {
            group.parts.iter().all(|part| {
                part.name != "unparsed"
                    && part.uuid != Uuid::parse_str("77777777-7777-7777-7777-777777777771").unwrap()
                    && part.uuid != Uuid::parse_str("77777777-7777-7777-7777-777777777772").unwrap()
                    && part.uuid != Uuid::parse_str("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1").unwrap()
            })
        }));
        let no_shape = groups
            .iter()
            .find_map(|group| group.parts.iter().find(|part| part.name == "no-shape"))
            .unwrap();
        assert!(no_shape.items.iter().any(|item| item.name == "kept"));
        assert!(no_shape.items.iter().all(|item| item.name != "dropped"));
        assert!(no_shape.items.iter().all(|item| item.name != "negative"));
        assert!(groups.iter().all(|group| {
            group.parts.iter().all(|part| {
                part.name != "rewrite-root"
                    && part.name != "rewrite-child"
                    && part.items.iter().all(|item| item.name != "rewrite-note")
            })
        }));
        let kept = without_dropped_root(&postgres_dump());
        assert_eq!(
            groups.len(),
            kept["groups"].as_array().unwrap().len(),
            "the rewrite group and the zero group are left out"
        );
    }

    async fn reset_postgres_extras(postgres: &sqlx::PgPool) {
        sqlx::query(
            "DELETE FROM primitems WHERE \"itemID\" IN \
             ('d1000000-0000-0000-0000-000000000001', \
              'd1000000-0000-0000-0000-000000000002', \
              'd1000000-0000-0000-0000-000000000003', \
              'd1000000-0000-0000-0000-000000000004')",
        )
        .execute(postgres)
        .await
        .unwrap();
        sqlx::query(
            "DELETE FROM primshapes WHERE \"UUID\" = '66666666-6666-6666-6666-666666666661'",
        )
        .execute(postgres)
        .await
        .unwrap();
        sqlx::query(
            "DELETE FROM prims WHERE \"UUID\" IN \
             ('77777777-7777-7777-7777-777777777771', \
              '77777777-7777-7777-7777-777777777772', \
              '66666666-6666-6666-6666-666666666661', \
              'eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1')",
        )
        .execute(postgres)
        .await
        .unwrap();
    }
}
