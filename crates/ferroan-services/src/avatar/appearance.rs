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
// Ported from Source/OpenSim.Services.Interfaces/IAvatarService.cs (AvatarData),
// Source/OpenSim.Framework/AvatarAppearance.cs and AvatarWearable.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{BTreeMap, HashMap};

use ferroan_http::{MAX_FIELDS, MAX_QUERY_BYTES};
use uuid::{Uuid, uuid};

use super::service::{check_bound, check_fields, numeric_text, parse_height, try_id};
use super::{AvatarData, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppearanceItem {
    pub item: Uuid,
    pub asset: Uuid,
}

const DEFAULT_ITEMS: [AppearanceItem; 6] = [
    AppearanceItem {
        item: uuid!("66c41e39-38f9-f75a-024e-585989bfaba9"),
        asset: uuid!("66c41e39-38f9-f75a-024e-585989bfab73"),
    },
    AppearanceItem {
        item: uuid!("77c41e39-38f9-f75a-024e-585989bfabc9"),
        asset: uuid!("77c41e39-38f9-f75a-024e-585989bbabbb"),
    },
    AppearanceItem {
        item: uuid!("d342e6c1-b9d2-11dc-95ff-0800200c9a66"),
        asset: uuid!("d342e6c0-b9d2-11dc-95ff-0800200c9a66"),
    },
    AppearanceItem {
        item: uuid!("cdc31054-eed8-4021-994f-4e0c6e861b50"),
        asset: uuid!("4bb6fa4d-1cd2-498a-a84c-95c1a0e745a7"),
    },
    AppearanceItem {
        item: uuid!("77c41e39-38f9-f75a-0000-585989bf0000"),
        asset: uuid!("00000000-38f9-1111-024e-222222111110"),
    },
    AppearanceItem {
        item: uuid!("77c41e39-38f9-f75a-0000-5859892f1111"),
        asset: uuid!("00000000-38f9-1111-024e-222222111120"),
    },
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WearableLayers(Vec<AppearanceItem>);

impl WearableLayers {
    pub fn items(&self) -> &[AppearanceItem] {
        &self.0
    }

    /// AvatarWearable.Add replaces an existing asset without moving its layer.
    pub fn add(&mut self, item: AppearanceItem) {
        if item.item.is_nil() {
            return;
        }
        if let Some(existing) = self.0.iter_mut().find(|other| other.item == item.item) {
            *existing = item;
        } else if self.0.len() < 5 {
            self.0.push(item);
        }
    }
}

/// The appearance fields persisted by the avatar service. The order of items
/// within each wearable type and attachment point is significant.
#[derive(Debug, Clone, PartialEq)]
pub struct AvatarAppearance {
    pub serial: i32,
    pub height: f32,
    pub wearables: Vec<WearableLayers>,
    pub visual_params: Vec<u8>,
    pub attachments: BTreeMap<i32, Vec<AppearanceItem>>,
}

impl Default for AvatarAppearance {
    fn default() -> Self {
        let mut wearables = vec![WearableLayers::default(); 15];
        for (slot, item) in wearables.iter_mut().zip(DEFAULT_ITEMS) {
            slot.add(item);
        }
        Self {
            serial: 0,
            height: 1.9,
            wearables,
            visual_params: ferroan_types::DEFAULT_VISUAL_PARAMS.to_vec(),
            attachments: BTreeMap::new(),
        }
    }
}

fn malformed() -> StoreError {
    StoreError::Row("malformed avatar appearance".into())
}

fn number(value: &str) -> Result<i32, StoreError> {
    numeric_text(value).parse().map_err(|_| malformed())
}

// OpenMetaverse accepts leading ASCII spaces and reads the first UUID;
// trailing text does not take part in the parse.
fn id(value: &str) -> Option<Uuid> {
    let value = value.trim_start_matches(' ');
    let len = if value.as_bytes().get(8) == Some(&b'-') {
        36
    } else {
        32
    };
    Uuid::parse_str(value.get(..len)?).ok()
}

impl AvatarData {
    pub fn to_appearance(&self) -> Result<Option<AvatarAppearance>, StoreError> {
        check_fields(
            self.fields.len(),
            self.fields
                .iter()
                .map(|(k, v)| k.len().saturating_add(v.len())),
        )?;
        match self.decode() {
            Ok(appearance) => Ok(Some(appearance)),
            Err(StoreError::Row(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn decode(&self) -> Result<AvatarAppearance, StoreError> {
        let mut appearance = AvatarAppearance::default();
        if self.fields.is_empty() {
            return Ok(appearance);
        }
        appearance.wearables.fill(WearableLayers::default());
        let fields: HashMap<_, _> = self
            .fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if let Some(value) = fields.get("Serial") {
            appearance.serial = number(value)?;
        }
        if let Some(value) = fields.get("AvatarHeight") {
            let height = parse_height(value).ok_or_else(malformed)?;
            appearance.height = if height == 0.0 {
                1.9
            } else {
                height.clamp(0.1, 32.0)
            };
        }
        if let Some(value) = fields.get("VisualParams") {
            appearance.visual_params = value
                .split(',')
                .map(|v| number(v).map(|i| i as u8))
                .collect::<Result<_, _>>()?;
        }
        for (slot, key) in [
            "Body",
            "Skin",
            "Hair",
            "Eyes",
            "Shirt",
            "Pants",
            "Shoes",
            "Socks",
            "Jacket",
            "Gloves",
            "UnderShirt",
            "UnderPants",
            "Skirt",
        ]
        .iter()
        .enumerate()
        {
            if let Some(value) = fields.get(format!("{key}Item").as_str()) {
                let asset = fields
                    .get(format!("{key}Asset").as_str())
                    .and_then(|v| id(v))
                    .ok_or_else(malformed)?;
                appearance.wearables[slot].add(AppearanceItem {
                    item: id(value).ok_or_else(malformed)?,
                    asset,
                });
            }
        }
        let mut layers = Vec::new();
        let mut attachments = AttachmentRows::default();
        for (key, value) in &self.fields {
            if let Some(suffix) = key.strip_prefix("Wearable ").filter(|s| !s.is_empty()) {
                let mut indices = suffix.split(':');
                let ty = usize::try_from(number(indices.next().ok_or_else(malformed)?)?)
                    .map_err(|_| malformed())?;
                check_bound(ferroan_diag::AVATAR_ROWS, MAX_FIELDS, ty + 1)?;
                let index = indices.next().map(number).transpose()?.unwrap_or(0);
                let mut ids = value.split(':');
                let item = ids.next().and_then(id).ok_or_else(malformed)?;
                let asset = ids.next().and_then(id).ok_or_else(malformed)?;
                layers.push((ty, index, AppearanceItem { item, asset }));
            } else if let Some(point) = key.strip_prefix("_ap_").and_then(|v| number(v).ok()) {
                attachments.read(point, value);
            }
        }
        layers.sort_by_key(|(ty, index, _)| (*ty, *index));
        for (ty, _, item) in layers {
            if ty >= appearance.wearables.len() {
                appearance.wearables.resize_with(ty + 1, Default::default);
            }
            appearance.wearables[ty].add(item);
        }
        for (slot, default) in appearance.wearables.iter_mut().zip(&DEFAULT_ITEMS[..4]) {
            if slot.items().is_empty() {
                slot.add(*default);
            }
        }
        appearance.attachments = attachments.finish();
        Ok(appearance)
    }

    pub fn from_appearance(appearance: &AvatarAppearance) -> Result<Self, StoreError> {
        check_bound(
            ferroan_diag::AVATAR_ROWS,
            MAX_FIELDS,
            appearance.wearables.len(),
        )?;
        let rows = appearance
            .wearables
            .iter()
            .map(|w| w.items().len())
            .sum::<usize>()
            .saturating_add(appearance.attachments.len())
            .saturating_add(3);
        check_bound(ferroan_diag::AVATAR_ROWS, MAX_FIELDS, rows)?;
        let mut fields = vec![
            ("Serial".into(), appearance.serial.to_string()),
            (
                "AvatarHeight".into(),
                crate::griduser::format_f32(appearance.height),
            ),
        ];
        for (ty, layers) in appearance.wearables.iter().enumerate() {
            for (index, layer) in layers.items().iter().enumerate() {
                fields.push((
                    format!("Wearable {ty}:{index}"),
                    format!("{}:{}", layer.item, layer.asset),
                ));
            }
        }
        check_bound(
            ferroan_diag::AVATAR_ROW_BYTES,
            MAX_QUERY_BYTES,
            appearance.visual_params.len(),
        )?;
        let mut visual = String::new();
        for byte in &appearance.visual_params {
            if !visual.is_empty() {
                visual.push(',');
            }
            visual.push_str(&byte.to_string());
        }
        fields.push(("VisualParams".into(), visual));
        // Bound all point lists together before formatting any UUIDs.
        let count = appearance
            .attachments
            .values()
            .map(Vec::len)
            .fold(0usize, usize::saturating_add);
        check_bound(
            ferroan_diag::AVATAR_ROW_BYTES,
            MAX_QUERY_BYTES,
            count.saturating_mul(36),
        )?;
        for (point, items) in &appearance.attachments {
            let mut ids = String::new();
            for item in items.iter().filter(|i| !i.item.is_nil()) {
                if !ids.is_empty() {
                    ids.push(',');
                }
                ids.push_str(
                    item.item
                        .as_hyphenated()
                        .encode_lower(&mut Uuid::encode_buffer()),
                );
            }
            if !ids.is_empty() {
                fields.push((format!("_ap_{point}"), ids));
            }
        }
        check_fields(
            fields.len(),
            fields.iter().map(|(k, v)| k.len().saturating_add(v.len())),
        )?;
        Ok(Self {
            avatar_type: 1,
            fields,
        })
    }
}

// SetAttachment clears the raw point, then appends at point & 127. An item
// already attached anywhere moves to its new position, even within one point.
#[derive(Default)]
struct AttachmentRows {
    by_item: HashMap<Uuid, (i32, usize)>,
    cleared: HashMap<i32, usize>,
    sequence: usize,
}

impl AttachmentRows {
    fn read(&mut self, point: i32, value: &str) {
        if point != 0 {
            self.cleared.insert(point, self.sequence);
        }
        for item in value.split(',').filter_map(try_id) {
            self.sequence += 1;
            if item.is_nil() {
                self.cleared.insert(point | 128, self.sequence);
            } else {
                self.by_item.insert(item, (point & 127, self.sequence));
            }
        }
    }

    fn finish(self) -> BTreeMap<i32, Vec<AppearanceItem>> {
        let mut ordered: Vec<_> = self.by_item.into_iter().collect();
        ordered.sort_unstable_by_key(|(_, (_, seq))| *seq);
        let mut points = BTreeMap::<_, Vec<_>>::new();
        for (item, (point, seq)) in ordered {
            if seq > self.cleared.get(&point).copied().unwrap_or(0) {
                points.entry(point).or_default().push(AppearanceItem {
                    item,
                    asset: Uuid::nil(),
                });
            }
        }
        points
    }
}
