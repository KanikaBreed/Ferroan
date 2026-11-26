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
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (CreateObjectPropertiesBlock, CreateObjectPropertiesFamilyBlock,
//      ProcessEntityPropertyRequests)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
//   osUTF8 and Utils.StringToBytesNoTerm
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Object property blocks and the per-viewer property queue.
//!
//! `CreateObjectPropertiesBlock` and `CreateObjectPropertiesFamilyBlock`
//! (`LLClientView.cs` 6396–6488) and `ProcessEntityPropertyRequests`
//! (6222–6395) at Tranquillity `c401bceecd1684832ae8ba86a31b5ee01a4ded7b`.
//! String limits follow `osUTF8` (`SceneObjectPart.cs` 693–716, 1151–1201)
//! and `Utils.StringToBytesNoTerm` at libopenmetaverse `a3596616`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

use ferroan_diag::{Action, Hit, Kind, OBJECT_UPDATE_FIELD, hit};
use ferroan_protocol::msgs::ObjectPropertiesFamily;
use ferroan_protocol::msgs::object_properties::ObjectData as ObjectBlock;
use ferroan_protocol::msgs::object_properties_family::ObjectData as FamilyBlock;
use ferroan_protocol::{Msg, Writer};
use uuid::Uuid;

use crate::entity_queue::{merge_flags, normalize_new};
use crate::objects::ObjectPart;
use crate::xml::format_f32;

/// `ObjectPropertyUpdateFlags.Family`.
pub(crate) const FAMILY: u8 = 1;
/// `ObjectPropertyUpdateFlags.Object`.
pub(crate) const OBJECT: u8 = 2;
/// Family flag cost in `ProcessEntityPropertyRequests`.
pub(crate) const FAMILY_COST: i32 = 100;
/// Object flag cost in `ProcessEntityPropertyRequests`.
pub(crate) const OBJECT_COST: i32 = 200;

const NAME_LIMIT: usize = 63;
const DESCRIPTION_LIMIT: usize = 127;
const TOUCH_LIMIT: usize = 36;
const SIT_LIMIT: usize = 36;

/// One waiting property request. A second enqueue for the same local id ORs
/// the flags and the family request flags. Drain order is the first enqueue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PropertySlot {
    pub local_id: u32,
    pub flags: u8,
    pub request_flags: u32,
}

#[derive(Default)]
pub(crate) struct PropertyQueue {
    order: VecDeque<u32>,
    slots: HashMap<u32, PropertySlot>,
}

impl PropertyQueue {
    pub(crate) fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub(crate) fn enqueue_family(&mut self, local_id: u32, request_flags: u32) {
        self.enqueue(local_id, FAMILY, request_flags);
    }

    pub(crate) fn enqueue_object(&mut self, local_id: u32) {
        self.enqueue(local_id, OBJECT, 0);
    }

    fn enqueue(&mut self, local_id: u32, flag: u8, request_flags: u32) {
        if let Some(slot) = self.slots.get_mut(&local_id) {
            slot.flags |= flag;
            // `EntityUpdate.UpdateFromNew` on the property queue's flag word.
            slot.request_flags = merge_flags(slot.request_flags, request_flags);
            return;
        }
        self.slots.insert(
            local_id,
            PropertySlot {
                local_id,
                flags: flag,
                // `EntityUpdate.Update` on the first insert.
                request_flags: normalize_new(request_flags),
            },
        );
        self.order.push_back(local_id);
    }

    pub(crate) fn remove(&mut self, local_ids: &[u32]) {
        for id in local_ids {
            if self.slots.remove(id).is_some() {
                self.order.retain(|queued| queued != id);
            }
        }
    }

    pub(crate) fn pop(&mut self) -> Option<PropertySlot> {
        let id = self.order.pop_front()?;
        self.slots.remove(&id)
    }
}

/// `osUTF8` then `AddShortLimitedUTF8`. Empty is an empty vector, which
/// `var1` writes as one zero byte. A nonempty value keeps a trailing NUL.
pub(crate) fn limited_text(field: &'static str, text: &str, max: usize) -> Vec<u8> {
    let prefix = &text.as_bytes()[..text.len().min(max + 1)];
    let len = if let Some(nul) = prefix.iter().position(|byte| *byte == 0) {
        nul
    } else if text.len() > max {
        // C# cuts once, when the value is stored. A later encode of the same
        // field and text does not report the cut again.
        report_cut(field, text, max);
        text.floor_char_boundary(max)
    } else {
        text.len()
    };
    let mut bytes = text.as_bytes()[..len].to_vec();
    if !bytes.is_empty() {
        bytes.push(0);
    }
    bytes
}

/// One `object.update-field` hit per distinct field and stored text.
fn report_cut(field: &'static str, text: &str, max: usize) {
    static SEEN: OnceLock<Mutex<HashSet<(&'static str, String)>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut seen = seen.lock().unwrap_or_else(|poison| poison.into_inner());
    if !seen.insert((field, text.to_owned())) {
        return;
    }
    hit(&Hit {
        id: OBJECT_UPDATE_FIELD,
        kind: Kind::Hard,
        configured: max as u64,
        observed: text.len() as u64,
        action: Action::Truncate,
        route: field,
        agent_id: None,
    });
}

/// `CreatorIdentification`: the text before `;`, or `UUID.Zero` when that
/// text does not parse. The parser is OpenMetaverse `UUID.TryParse`.
pub(crate) fn creator_uuid(value: &str) -> Uuid {
    let head = value.split(';').next().unwrap_or("");
    crate::omv_uuid(head)
}

/// Store loads leave the serial unset. The object XML writer then spells 0
/// with no items and 1 with any, which is what `RestoreInventoryItems` sends.
pub(crate) fn inventory_serial(part: &ObjectPart) -> i16 {
    let serial = match part.inventory_serial {
        Some(value) => value,
        None if part.items.is_empty() => 0,
        None => 1,
    };
    serial as i16
}

fn owner_on_wire(part: &ObjectPart) -> Uuid {
    if part.owner_id == part.group_id {
        Uuid::nil()
    } else {
        part.owner_id
    }
}

fn creation_stamp(date: i32) -> u64 {
    (date as u64).wrapping_mul(1_000_000)
}

pub(crate) fn object_block(part: &ObjectPart, root: &ObjectPart) -> Vec<u8> {
    let block = ObjectBlock {
        object_id: part.uuid,
        creator_id: creator_uuid(&part.creator_id),
        owner_id: owner_on_wire(part),
        group_id: part.group_id,
        creation_date: creation_stamp(part.creation_date),
        base_mask: root.base_mask,
        owner_mask: root.owner_mask,
        group_mask: root.group_mask,
        everyone_mask: root.everyone_mask,
        next_owner_mask: root.next_owner_mask,
        ownership_cost: 0,
        sale_type: root.sale_type,
        sale_price: root.sale_price,
        aggregate_perms: 0,
        aggregate_perm_textures: 0,
        aggregate_perm_textures_owner: 0,
        category: 0,
        inventory_serial: inventory_serial(part),
        item_id: Uuid::nil(),
        folder_id: Uuid::nil(),
        from_task_id: Uuid::nil(),
        last_owner_id: part.last_owner_id,
        name: limited_text("Name", &part.name, NAME_LIMIT),
        description: limited_text("Description", &part.description, DESCRIPTION_LIMIT),
        touch_name: limited_text("TouchName", &part.touch_name, TOUCH_LIMIT),
        sit_name: limited_text("SitName", &part.sit_name, SIT_LIMIT),
        texture_id: Vec::new(),
    };
    let mut writer = Writer::new();
    block.encode(&mut writer);
    writer.buf
}

pub(crate) fn family_payload(part: &ObjectPart, root: &ObjectPart, request_flags: u32) -> Vec<u8> {
    let message = ObjectPropertiesFamily {
        object_data: FamilyBlock {
            request_flags,
            object_id: part.uuid,
            owner_id: owner_on_wire(part),
            group_id: part.group_id,
            base_mask: root.base_mask,
            owner_mask: root.owner_mask,
            group_mask: root.group_mask,
            everyone_mask: root.everyone_mask,
            next_owner_mask: root.next_owner_mask,
            ownership_cost: 0,
            sale_type: root.sale_type,
            sale_price: root.sale_price,
            category: 0,
            last_owner_id: part.last_owner_id,
            name: limited_text("Name", &part.name, NAME_LIMIT),
            description: limited_text("Description", &part.description, DESCRIPTION_LIMIT),
        },
    };
    let mut writer = Writer::new();
    ObjectPropertiesFamily::ID.encode(&mut writer.buf);
    message.encode_body(&mut writer);
    writer.buf
}

/// One part in `ObjectPhysicsProperties`. `GravityMultiplier` is `GravityModifier`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsPart {
    pub local_id: u32,
    pub density: f32,
    pub friction: f32,
    pub gravity_modifier: f32,
    pub restitution: f32,
    pub shape: u8,
}

/// One `ProcessEntityPropertyRequests` pass: object blocks, family payloads, physics parts.
pub struct PropertyDrain {
    /// One `ObjectProperties` variable block, without the count byte.
    pub objects: Vec<Vec<u8>>,
    /// Message id and body of one `ObjectPropertiesFamily`.
    pub families: Vec<Vec<u8>>,
    /// Parts taken with the object flag, in dequeue order.
    pub physics: Vec<PhysicsPart>,
}

pub(crate) fn physics_of(part: &ObjectPart) -> PhysicsPart {
    PhysicsPart {
        local_id: part.local_id,
        density: part.density,
        friction: part.friction,
        gravity_modifier: part.gravity_modifier,
        restitution: part.restitution,
        shape: part.physics_shape_type,
    }
}

/// The inner event map `StartEvent` / `EndEventToBytes` produces. `GetEvents`
/// writes this element into the events array.
pub fn physics_event(parts: &[PhysicsPart]) -> Vec<u8> {
    let mut xml = Vec::new();
    push(
        &mut xml,
        "<map><key>message</key><string>ObjectPhysicsProperties</string><key>body</key><map><key>ObjectData</key><array>",
    );
    for part in parts {
        push(&mut xml, "<map>");
        integer(&mut xml, "LocalID", part.local_id as i32);
        real(&mut xml, "Density", part.density);
        real(&mut xml, "Friction", part.friction);
        real(&mut xml, "GravityMultiplier", part.gravity_modifier);
        real(&mut xml, "Restitution", part.restitution);
        integer(&mut xml, "PhysicsShapeType", i32::from(part.shape));
        push(&mut xml, "</map>");
    }
    push(&mut xml, "</array></map></map>");
    xml
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

fn key(out: &mut Vec<u8>, name: &str) {
    push(out, "<key>");
    push(out, name);
    push(out, "</key>");
}

fn integer(out: &mut Vec<u8>, name: &str, value: i32) {
    key(out, name);
    if value == 0 {
        push(out, "<integer />");
    } else {
        push(out, "<integer>");
        push(out, &value.to_string());
        push(out, "</integer>");
    }
}

fn real(out: &mut Vec<u8>, name: &str, value: f32) {
    key(out, name);
    if value == 0.0 {
        push(out, "<real>0</real>");
        return;
    }
    push(out, "<real>");
    push(out, &spell_real(value));
    push(out, "</real>");
}

/// `float.ToString(InvariantCulture)`. `NaN` and `Infinity` are that spelling.
fn spell_real(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity".to_owned()
        } else {
            "Infinity".to_owned()
        };
    }
    format_f32(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_one_zero_and_a_value_keeps_its_nul() {
        assert!(limited_text("Name", "", 63).is_empty());
        assert_eq!(limited_text("Name", "Box", 63), b"Box\0");
        let long = "a".repeat(80);
        let limited = limited_text("Name", &long, 63);
        assert_eq!(limited.len(), 64);
        assert_eq!(limited[63], 0);
    }

    #[test]
    fn a_creator_suffix_is_dropped_and_a_bad_token_is_zero() {
        let id = Uuid::from_u128(9);
        assert_eq!(creator_uuid(&format!("{id};https://grid.example/")), id);
        assert_eq!(creator_uuid(&format!("  {id}  ")), id);
        assert_eq!(creator_uuid(&id.simple().to_string()), id);
        assert!(creator_uuid(&format!("{{{id}}}")).is_nil());
        assert!(creator_uuid("").is_nil());
        assert!(creator_uuid("not-a-uuid").is_nil());
    }

    #[test]
    fn request_flags_follow_update_and_a_family_select_merge() {
        use crate::entity_queue::{CANCEL_KILL, FULL_UPDATE_WITH_ANIM, UPDATE_PROBE};

        let mut queue = PropertyQueue::default();
        queue.enqueue_family(1, CANCEL_KILL | 0x4);
        assert_eq!(queue.pop().unwrap().request_flags, FULL_UPDATE_WITH_ANIM);
        queue.enqueue_family(2, CANCEL_KILL | UPDATE_PROBE);
        assert_eq!(queue.pop().unwrap().request_flags, UPDATE_PROBE);
        queue.enqueue_family(3, 0x15);
        queue.enqueue_object(3);
        queue.enqueue_family(3, CANCEL_KILL);
        let slot = queue.pop().unwrap();
        assert_eq!(slot.local_id, 3);
        assert_eq!(slot.flags, FAMILY | OBJECT);
        assert_eq!(slot.request_flags, FULL_UPDATE_WITH_ANIM);
    }

    #[test]
    fn a_child_keeps_its_own_identity_and_the_root_masks() {
        use ferroan_protocol::Reader;
        use ferroan_protocol::msgs::object_properties::ObjectData;

        let mut root = crate::xml::blank_part();
        root.uuid = Uuid::from_u128(1);
        root.owner_id = Uuid::from_u128(2);
        root.last_owner_id = Uuid::from_u128(3);
        root.base_mask = 0x11;
        root.owner_mask = 0x22;
        root.group_mask = 0x33;
        root.everyone_mask = 0x44;
        root.next_owner_mask = 0x55;
        root.sale_type = 3;
        root.sale_price = 9;
        root.name = "RootName".into();
        root.touch_name = "root-touch".into();
        root.sit_name = "root-sit".into();

        let mut child = root.clone();
        child.uuid = Uuid::from_u128(4);
        child.last_owner_id = Uuid::from_u128(5);
        child.base_mask = 0x99;
        child.owner_mask = 0x88;
        child.group_mask = 0x77;
        child.everyone_mask = 0x66;
        child.next_owner_mask = 0x55aa;
        child.sale_type = 1;
        child.sale_price = 100;
        child.name = "ChildName".into();
        child.description = "ChildDesc".into();
        child.touch_name = "child-touch".into();
        child.sit_name = "child-sit".into();
        child.items.push(crate::xml::blank_item(1));

        let bytes = object_block(&child, &root);
        assert!(bytes.windows(10).any(|window| window == b"ChildName\0"));
        assert!(bytes.windows(12).any(|window| window == b"child-touch\0"));
        assert!(bytes.windows(10).any(|window| window == b"child-sit\0"));
        assert!(!bytes.windows(8).any(|window| window == b"RootName"));
        let mut reader = Reader::new(&bytes);
        let block = ObjectData::decode(&mut reader).unwrap();
        assert_eq!(block.object_id, child.uuid);
        assert_eq!(block.last_owner_id, child.last_owner_id);
        assert_eq!(block.base_mask, root.base_mask);
        assert_eq!(block.owner_mask, root.owner_mask);
        assert_eq!(block.group_mask, root.group_mask);
        assert_eq!(block.everyone_mask, root.everyone_mask);
        assert_eq!(block.next_owner_mask, root.next_owner_mask);
        assert_eq!(block.sale_type, root.sale_type);
        assert_eq!(block.sale_price, root.sale_price);
        assert_eq!(block.inventory_serial, 1);
        assert_ne!(block.last_owner_id, child.owner_id);

        let payload = family_payload(&child, &root, 0x15);
        let (_, len) = ferroan_protocol::MsgId::decode(&payload).unwrap();
        let family =
            ferroan_protocol::msgs::ObjectPropertiesFamily::decode(&payload[len..]).unwrap();
        assert_eq!(family.object_data.request_flags, 0x15);
        assert_eq!(family.object_data.object_id, child.uuid);
        assert_eq!(family.object_data.base_mask, root.base_mask);
        assert_eq!(family.object_data.sale_price, root.sale_price);
    }

    #[test]
    fn the_queue_merges_one_slot_and_drains_oldest_first() {
        let mut queue = PropertyQueue::default();
        queue.enqueue_object(2);
        queue.enqueue_family(1, 0x4);
        queue.enqueue_family(2, 0x1);
        queue.enqueue_object(2);
        let first = queue.pop().unwrap();
        assert_eq!(first.local_id, 2);
        assert_eq!(first.flags, FAMILY | OBJECT);
        assert_eq!(first.request_flags, 0x1);
        assert_eq!(queue.pop().unwrap().local_id, 1);
        assert!(queue.pop().is_none());
        queue.enqueue_object(3);
        queue.remove(&[3, 9]);
        assert!(queue.is_empty());
    }

    #[test]
    fn a_zero_local_id_and_a_zero_real_use_the_empty_spellings() {
        let xml = physics_event(&[PhysicsPart {
            local_id: 0,
            density: 0.0,
            friction: 0.6,
            gravity_modifier: 1.0,
            restitution: -0.0,
            shape: 0,
        }]);
        let text = String::from_utf8(xml).unwrap();
        assert!(text.starts_with(
            "<map><key>message</key><string>ObjectPhysicsProperties</string><key>body</key><map><key>ObjectData</key><array><map>"
        ));
        assert!(text.contains("<key>LocalID</key><integer />"));
        assert!(text.contains("<key>Density</key><real>0</real>"));
        assert!(text.contains("<key>Restitution</key><real>0</real>"));
        assert!(text.contains("<key>PhysicsShapeType</key><integer />"));
        assert!(text.ends_with("</map></array></map></map>"));
        assert!(text.contains(&format!(
            "<key>Friction</key><real>{}</real>",
            format_f32(0.6)
        )));
    }

    #[tokio::test]
    async fn stored_parts_match_every_byte_of_the_pin8_object_properties() {
        use std::collections::BTreeMap;

        use crate::test_store::CopiedStore;
        use crate::{ObjectStoreKind, open_objects};

        let copy = CopiedStore::new("prim-updates/scene.db");
        let region = Uuid::from_u128(0x6d1a7f4e_0c3b_4f53_9a1e_5f0e3c2a9b01);
        let groups = open_objects(&copy.connection(), region, ObjectStoreKind::Sqlite)
            .await
            .unwrap();
        let map_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/prim-updates/map.json");
        let map: serde_json::Value =
            serde_json::from_slice(&std::fs::read(map_path).unwrap()).unwrap();
        let mut uuid_of = BTreeMap::new();
        let owner_store: Uuid = map["owner"]["store_uuid"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let owner_capture: Uuid = map["owner"]["capture_uuid"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        uuid_of.insert(owner_store, owner_capture);
        for entry in map["parts"].as_array().unwrap() {
            let store: Uuid = entry["store_uuid"].as_str().unwrap().parse().unwrap();
            let capture: Uuid = entry["capture_uuid"].as_str().unwrap().parse().unwrap();
            uuid_of.insert(store, capture);
        }
        let alias = |id: Uuid| uuid_of.get(&id).copied().unwrap_or(id);
        let alias_creator = |value: &str| {
            let (head, rest) = value.split_once(';').unwrap_or((value, ""));
            let id = crate::omv_uuid(head);
            match uuid_of.get(&id) {
                Some(capture) if rest.is_empty() => capture.to_string(),
                Some(capture) => format!("{capture};{rest}"),
                None => value.to_owned(),
            }
        };
        let mut aliased = Vec::new();
        for group in &groups {
            let root_id = group.parts[0].uuid;
            for part in &group.parts {
                let mut part = part.clone();
                let mut root = group
                    .parts
                    .iter()
                    .find(|candidate| candidate.uuid == root_id)
                    .unwrap()
                    .clone();
                for item in [&mut part, &mut root] {
                    item.uuid = alias(item.uuid);
                    item.local_id = u32::try_from(item.uuid.as_u128()).unwrap();
                    item.owner_id = alias(item.owner_id);
                    item.group_id = alias(item.group_id);
                    item.last_owner_id = alias(item.last_owner_id);
                    item.creator_id = alias_creator(&item.creator_id);
                }
                aliased.push((part.uuid, part, root));
            }
        }
        aliased.sort_by_key(|(id, _, _)| *id);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/object-properties");
        let roots: Vec<_> = aliased
            .iter()
            .filter(|(_, part, root)| part.uuid == root.uuid)
            .collect();
        assert_eq!(roots.len(), 7);
        for (index, (_, part, root)) in roots.iter().enumerate() {
            let payload = family_payload(part, root, 0);
            let (_, len) = ferroan_protocol::MsgId::decode(&payload).unwrap();
            let expected = std::fs::read(dir.join(format!("family-{}.bin", index + 1))).unwrap();
            assert_eq!(&payload[len..], expected.as_slice(), "family {}", index + 1);
        }
        let mut object = vec![u8::try_from(aliased.len()).unwrap()];
        let mut physics = Vec::new();
        for (_, part, root) in &aliased {
            object.extend(object_block(part, root));
            physics.push(physics_of(part));
        }
        assert_eq!(
            object,
            std::fs::read(dir.join("object-1.bin")).unwrap(),
            "object properties"
        );
        assert_eq!(
            physics_event(&physics),
            std::fs::read(dir.join("physics-1.xml")).unwrap(),
            "physics event"
        );
        let kill = std::fs::read(dir.join("kill-1.bin")).unwrap();
        assert_eq!(kill, std::fs::read(dir.join("kill-2.bin")).unwrap());
        assert_eq!(kill, vec![1, 1, 0, 0, 0]);
    }
}
