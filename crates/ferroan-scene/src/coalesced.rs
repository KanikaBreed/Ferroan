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
// Ported from OpenSim.Region.Framework/Scenes/{CoalescedSceneObjects,Scene,
// SceneObjectGroup,SceneObjectPart,Serialization/CoalescedSceneObjectsSerializer}.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b, and OpenMetaverse.Types/
// Matrix4.cs, Quaternion.cs and Vector3.cs at a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    format_f32, to_original_xml,
    xml_read::{bounded, read_original, sanitize_xml, unix_now},
};
use crate::{ObjectGroup, objects::MAX_REGION_OBJECTS};
use ferroan_diag::{Kind, OBJECT_COALESCED_PRIMS, refuse};
use uuid::Uuid;

type Vector = [f32; 3];
type Rotation = [f32; 4];

/// Scene.GetCombinedBoundingBox combines the bounds of each linkset in region
/// coordinates. Its offsets are measured from the minimum to each root.
pub fn to_coalesced_xml(groups: &[ObjectGroup]) -> Option<String> {
    let mut lower = [f32::MAX; 3];
    let mut upper = [f32::MIN; 3];
    let mut total: usize = 0;
    for group in groups {
        let root = group.parts.first()?;
        total = total.saturating_add(group.parts.len());
        check_prim_count(total, MAX_REGION_OBJECTS)?;
        let (low, high) = linkset_bounds(root, &group.parts);
        for i in 0..3 {
            extend(
                &mut lower[i],
                &mut upper[i],
                low[i] + root.group_position[i],
                high[i] + root.group_position[i],
            );
        }
    }
    let [x, y, z] = std::array::from_fn(|i| format_f32(upper[i] - lower[i]));
    let mut xml = format!(r#"<CoalescedObject x="{x}" y="{y}" z="{z}""#);
    if groups.is_empty() {
        xml.push_str(" />");
        return Some(xml);
    }
    xml.push('>');
    for group in groups {
        let position = group.parts.first()?.group_position;
        let [x, y, z] = std::array::from_fn(|i| format_f32(position[i] - lower[i]));
        let wrapper = format!(r#"<SceneObjectGroup offsetx="{x}" offsety="{y}" offsetz="{z}">"#);
        let member = to_original_xml(group);
        bounded(
            xml.len()
                .saturating_add(wrapper.len())
                .saturating_add(member.len())
                .saturating_add("</SceneObjectGroup></CoalescedObject>".len()),
        )?;
        xml.push_str(&wrapper);
        xml.push_str(&member);
        xml.push_str("</SceneObjectGroup>");
    }
    xml.push_str("</CoalescedObject>");
    Some(xml)
}

// SceneObjectGroup.GetAxisAlignedBoundingBoxRaw treats every prim as a rotated box.
fn linkset_bounds(root: &crate::ObjectPart, parts: &[crate::ObjectPart]) -> (Vector, Vector) {
    let mut low = [f32::MAX; 3];
    let mut high = [f32::MIN; 3];
    let root_axes = axes(root.rotation);
    for part in parts {
        let (position, rotation) = if part.parent_id == 0 {
            (part.group_position, part.rotation)
        } else {
            (
                std::array::from_fn(|i| root.group_position[i] + dot(root_axes[i], part.offset)),
                product(root.rotation, part.rotation),
            )
        };
        let half = part.shape.scale.map(|s| s * 0.5);
        for (i, axis) in axes(rotation).into_iter().enumerate() {
            // Keep Pin8's world-position add/subtract before combining linksets.
            let offset = position[i] - root.group_position[i];
            let extent = dot(axis.map(f32::abs), half);
            extend(&mut low[i], &mut high[i], offset - extent, offset + extent);
        }
    }
    (low, high)
}

// Pin8 retains the first endpoint on ties, including oppositely signed zero.
fn extend(low: &mut f32, high: &mut f32, min: f32, max: f32) {
    if min < *low {
        *low = min;
    }
    if max > *high {
        *high = max;
    }
}

fn check_prim_count(count: usize, limit: u64) -> Option<()> {
    let observed = u64::try_from(count).unwrap_or(u64::MAX);
    if observed > limit {
        refuse(
            OBJECT_COALESCED_PRIMS,
            Kind::Hard,
            limit,
            observed,
            "coalesced",
        );
        return None;
    }
    Some(())
}

fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn axes([x, y, z, w]: Rotation) -> [Vector; 3] {
    let (xx, yy, zz) = (x + x, y + y, z + z);
    [
        [1.0 - y * yy - z * zz, x * yy - w * zz, x * zz + w * yy],
        [x * yy + w * zz, 1.0 - x * xx - z * zz, y * zz - w * xx],
        [x * zz - w * yy, y * zz + w * xx, 1.0 - x * xx - y * yy],
    ]
}

fn product(a: Rotation, b: Rotation) -> Rotation {
    let [x, y, z, w] = a;
    let [i, j, k, r] = b;
    if w.abs() > 1.0 - 1e-6 {
        return b;
    }
    if r.abs() > 1.0 - 1e-6 {
        return a;
    }
    [
        w * i + x * r + y * k - z * j,
        w * j + y * r + z * i - x * k,
        w * k + z * r + x * j - y * i,
        w * r - x * i - y * j - z * k,
    ]
}

/// Read members in document order as TryFromXml does, skipping unreadable members.
/// The rez path additionally requires valid wrapper sizes and offsets to place each
/// linkset. Its caller needs those values as well as the members.
pub fn from_coalesced_xml(xml: &str) -> Option<Vec<ObjectGroup>> {
    bounded(xml.len())?;
    let root = ferroan_llsd::xml::parse_document(xml.as_bytes()).ok()?;
    if root.name != "CoalescedObject" || root.attr("xmlns").is_some_and(|ns| !ns.is_empty()) {
        return None;
    }
    read_members(&root, MAX_REGION_OBJECTS)
}

fn read_members(root: &ferroan_llsd::xml::Element, limit: u64) -> Option<Vec<ObjectGroup>> {
    let now = unix_now();
    let mut groups = Vec::new();
    let mut total: usize = 0;
    for wrapper in root
        .elements()
        .filter(|el| el.name == "SceneObjectGroup" && el.attr("xmlns").is_none_or(str::is_empty))
    {
        let member = wrapper.child("SceneObjectGroup").unwrap_or(wrapper);
        if let Some(group) = read_original(member, now) {
            total = total.saturating_add(group.parts.len());
            check_prim_count(total, limit)?;
            groups.push(group);
        }
    }
    Some(groups)
}

/// Decode and sanitize the inventory asset as ModifySerializedObject does (D83).
pub fn from_coalesced_xml_data(data: &[u8]) -> Option<Vec<ObjectGroup>> {
    bounded(data.len())?;
    let text = String::from_utf8_lossy(data.strip_suffix(&[0]).unwrap_or(data));
    bounded(text.len())?;
    from_coalesced_xml(&sanitize_xml(&text))
}

impl ObjectGroup {
    /// The extra crossing field travels beside the group's XML2.
    pub fn extra_to_xml_string(&self) -> String {
        format!("<ExtraFromItemID>{}</ExtraFromItemID>", self.from_item_id)
    }

    /// Read the first literal tag. The closing tag is optional in Pin8.
    pub fn extra_from_xml_string(&mut self, text: &str) -> Option<()> {
        bounded(text.len())?;
        self.from_item_id = text
            .split_once("<ExtraFromItemID>")
            .and_then(|(_, value)| {
                let id = value
                    .split_once("</ExtraFromItemID>")
                    .map_or(value, |(id, _)| id)
                    .trim_matches(' ');
                if !matches!(id.len(), 32 | 36) {
                    return None;
                }
                Uuid::parse_str(id).ok()
            })
            .unwrap_or_default();
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesced_prim_budget_counts_children_across_member_groups() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/regionstore/coalesced.json"))
                .unwrap();
        let root =
            ferroan_llsd::xml::parse_document(fixture["xml"].as_str().unwrap().as_bytes()).unwrap();
        assert_eq!(read_members(&root, 3).unwrap().len(), 2);
        assert!(read_members(&root, 2).is_none());
    }
}
