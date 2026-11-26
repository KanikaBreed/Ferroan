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
// Ported from LLClientView.SendLandProperties (6757–6893),
// LandObject.GetParcelMaxPrimCount (437) and GetSimulatorMaxPrimCount (463),
// LandObject.UpdateGeometryValues and ConvertLandBitmapToBytes, and
// OpenMetaverse.StructuredData.LLSDxmlEncode2 at Tranquillity
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// libopenmetaverse LLSDxmlEncode.cs uintToByteArray is lines 961–969.

// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `ParcelProperties` event element `EndEventToBytes` returns.
//!
//! `to_xml` stays the capability serializer. This writer follows Encode2 only
//! for the types this message uses.

use std::collections::HashMap;

use base64::Engine;
use uuid::Uuid;

use super::{Parcel, ParcelMap};
use crate::height::Heightmap;
use crate::objects::ObjectGroup;

/// `OpenMetaverse.RegionFlags` bits `SendLandProperties` reads.
const RESTRICT_PUSH: u32 = 1 << 22;
const DENY_ANONYMOUS: u32 = 1 << 23;
const DENY_IDENTIFIED: u32 = 1 << 24;
const DENY_TRANSACTED: u32 = 1 << 25;
const ALLOW_ENVIRONMENT: u32 = 1 << 9;
const DENY_AGE: u32 = 1 << 30;

/// `PrimCounts` for one parcel.
///
/// The tally walks the groups admitted at startup. A stored group is counted
/// even when its root pcode is 9 and `State` is non-zero: `PrimCountModule`
/// skips `IsAttachment`, and a group restored from storage never has that
/// flag. `TemporaryOnRez` on the root is skipped. Owner, group and other
/// follow the land owner and group, and the part count is the group's length.
/// Selected stays zero: a stored group is not selected and holds no sitting
/// avatar. Simulator is that land owner's total on every parcel they own. A
/// parcel with nothing counted stays at zero. Runtime rez, moves, selection
/// and sitting are not in the admitted set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrimCounts {
    pub owner: i32,
    pub group: i32,
    pub others: i32,
    pub selected: i32,
    pub total: i32,
    pub simulator: i32,
}

/// Region values `SendLandProperties` reads besides the parcel map.
///
/// The capture defaults are flags 0, tax free false, capacity 15,000 and
/// object bonus 1. [`ParcelPropertiesSend::notify`] builds that element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParcelRegion {
    /// `EstateManagementModule.GetRegionFlags`. The handshake already applies
    /// the settings bits, including `RestrictPushObject` from `RestrictPushing`.
    pub region_flags: u32,
    /// `EstateSettings.TaxFree`. The element sends the inverse.
    pub tax_free: bool,
    /// `RegionInfo.ObjectCapacity`. The fresh-region default is 15,000.
    pub object_capacity: i32,
    /// `RegionSettings.ObjectBonus`.
    pub object_bonus: f64,
}

impl Default for ParcelRegion {
    fn default() -> Self {
        Self {
            region_flags: 0,
            tax_free: false,
            object_capacity: 15_000,
            object_bonus: 1.0,
        }
    }
}

impl ParcelRegion {
    /// `AllowEnvironmentOverride` is the estate bit the handshake leaves clear.
    /// Bits 23, 24, 25 and 30 are cleared: `GetRegionFlags` never sets them
    /// (D108, TB152), even when the handshake word does.
    pub fn from_handshake(
        handshake_flags: u32,
        object_bonus: f64,
        object_capacity: i32,
        tax_free: bool,
        allow_environment: bool,
    ) -> Self {
        let mut region_flags =
            handshake_flags & !(DENY_ANONYMOUS | DENY_IDENTIFIED | DENY_TRANSACTED | DENY_AGE);
        if allow_environment {
            region_flags |= ALLOW_ENVIRONMENT;
        }
        Self {
            region_flags,
            tax_free,
            object_capacity,
            object_bonus,
        }
    }

    /// Sequence 0 for one parcel. Snap, the request result and the agent's
    /// environment version stay at the notify values.
    pub fn element(self, prims: PrimCounts) -> ParcelPropertiesSend {
        ParcelPropertiesSend {
            sequence_id: 0,
            snap_selection: false,
            request_result: 0,
            region_flags: self.region_flags,
            tax_free: self.tax_free,
            object_capacity: self.object_capacity,
            object_bonus: self.object_bonus,
            prims,
            agent_environment_version: 0,
        }
    }
}

/// What `LandObject.SendLandProperties` passes in besides the parcel map.
///
/// `region_flags` is `EstateManagementModule.GetRegionFlags`. That method does
/// not set the four deny bits this element reads (TB152, D108). The bounding
/// box is the one stored at land load, not a sample taken while encoding.
pub struct ParcelPropertiesSend {
    pub sequence_id: i32,
    pub snap_selection: bool,
    pub request_result: i32,
    pub region_flags: u32,
    /// `EstateSettings.TaxFree`. The element sends the inverse.
    pub tax_free: bool,
    /// `RegionInfo.ObjectCapacity`. The fresh-region default is 15,000.
    pub object_capacity: i32,
    /// `RegionSettings.ObjectBonus`, the double both capacity formulas use.
    pub object_bonus: f64,
    pub prims: PrimCounts,
    /// `ScenePresence.EnvironmentVersion`.
    pub agent_environment_version: i32,
}

impl ParcelPropertiesSend {
    /// Sequence 0 for one parcel, with the region values the Q61a arrival and
    /// walk elements were captured with.
    pub fn notify() -> Self {
        ParcelRegion::default().element(PrimCounts::default())
    }
}

/// `PrimFlags.TemporaryOnRez`. `PrimCountModule.AddObject` skips it on the root.
/// A stored group is counted even when its root pcode is 9 and `State` is
/// non-zero: `IsAttachment` is a field a restored group never has.
const TEMPORARY_ON_REZ: u32 = 0x2000_0000;

/// `PrimCountModule.Recount` over the groups the region admitted.
pub(crate) fn prim_counts(map: &ParcelMap, groups: &[ObjectGroup]) -> HashMap<Uuid, PrimCounts> {
    let mut counts: HashMap<Uuid, PrimCounts> = HashMap::new();
    let mut simwide: HashMap<Uuid, i32> = HashMap::new();
    for parcel in map.parcels() {
        counts.insert(parcel.id, PrimCounts::default());
        simwide.entry(parcel.owner).or_insert(0);
    }
    for group in groups {
        let Some(root) = group.parts.first() else {
            continue;
        };
        if root.flags & TEMPORARY_ON_REZ != 0 {
            continue;
        }
        let Some(land) = map.at_metres(root.group_position[0], root.group_position[1]) else {
            continue;
        };
        let parcel_id = land.id;
        let land_owner = land.owner;
        let land_group = land.group;
        let parts = i32::try_from(group.parts.len()).unwrap_or(i32::MAX);
        if let Some(total) = simwide.get_mut(&land_owner) {
            *total = total.saturating_add(parts);
        }
        let Some(slot) = counts.get_mut(&parcel_id) else {
            continue;
        };
        if root.owner_id == land_owner {
            slot.owner = slot.owner.saturating_add(parts);
        } else if !land_group.is_nil() && root.group_id == land_group {
            slot.group = slot.group.saturating_add(parts);
        } else {
            slot.others = slot.others.saturating_add(parts);
        }
    }
    for parcel in map.parcels() {
        let Some(slot) = counts.get_mut(&parcel.id) else {
            continue;
        };
        slot.total = slot
            .owner
            .saturating_add(slot.group)
            .saturating_add(slot.others);
        slot.simulator = simwide.get(&parcel.owner).copied().unwrap_or(0);
    }
    counts
}

impl ParcelMap {
    /// `LandObject.UpdateGeometryValues`, from `ForceUpdateLandInfo` when the
    /// parcel is loaded. Later encodes read the stored box.
    pub fn update_geometry(&mut self, heightmap: Option<&Heightmap>) {
        let (cells_x, cells_y) = self.cell_size();
        for index in 0..self.parcels.len() {
            let local_id = self.parcels[index].local_id;
            let (min, max) = aabb(&self.cells, cells_x, cells_y, local_id, heightmap);
            self.parcels[index].aabb_min = min;
            self.parcels[index].aabb_max = max;
        }
    }

    /// One `ParcelProperties` element, the bytes `EndEventToBytes` returns.
    pub fn parcel_properties(&self, local_id: i32, send: &ParcelPropertiesSend) -> Option<Vec<u8>> {
        let parcel = self
            .parcels
            .iter()
            .find(|parcel| parcel.local_id == local_id)?;
        let (width, height) = self.cell_size();
        let min = parcel.aabb_min;
        let max = parcel.aabb_max;
        let bitmap = land_bitmap(&self.cells, width, local_id as u32);
        let region = region_metres(width, height);
        let max_prims = if parcel.area > 0 {
            prim_capacity(
                parcel.area,
                send.object_capacity,
                send.object_bonus,
                region[0],
                region[1],
            )
        } else {
            0
        };
        let sim_prims = if parcel.simwide_area > 0 {
            prim_capacity(
                parcel.simwide_area,
                send.object_capacity,
                send.object_bonus,
                region[0],
                region[1],
            )
        } else {
            0
        };
        Some(encode(
            parcel, send, min, max, &bitmap, max_prims, sim_prims,
        ))
    }

    fn cell_size(&self) -> (usize, usize) {
        if self.width == 0 {
            return (0, 0);
        }
        (self.width, self.cells.len() / self.width)
    }
}

/// `GetParcelMaxPrimCount` and `GetSimulatorMaxPrimCount`.
///
/// Both are area × object capacity × object bonus / region area + 0.5, then
/// clamped to the region capacity. The region area is an `int` product. The
/// simulator formula widens that product to `long` before the double division.
fn prim_capacity(area: i32, capacity: i32, bonus: f64, region_x: i32, region_y: i32) -> i32 {
    let region_area = i64::from(region_x.wrapping_mul(region_y));
    if region_area == 0 {
        return 0;
    }
    let raw = f64::from(area) * f64::from(capacity) * bonus / (region_area as f64) + 0.5;
    let mut value = trunc_i32(raw);
    if value > capacity {
        value = capacity;
    }
    value
}

fn trunc_i32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    value as i32
}

fn region_metres(cells_x: usize, cells_y: usize) -> [i32; 2] {
    [
        i32::try_from(cells_x.saturating_mul(4)).unwrap_or(i32::MAX),
        i32::try_from(cells_y.saturating_mul(4)).unwrap_or(i32::MAX),
    ]
}

fn flag(send: &ParcelPropertiesSend, bit: u32) -> bool {
    (send.region_flags & bit) != 0
}

fn encode(
    parcel: &Parcel,
    send: &ParcelPropertiesSend,
    min: [f32; 3],
    max: [f32; 3],
    bitmap: &[u8],
    max_prims: i32,
    sim_prims: i32,
) -> Vec<u8> {
    let mut xml = Xml::default();
    xml.push("<map><key>message</key><string>ParcelProperties</string><key>body</key><map>");
    xml.block_open("ParcelData");
    xml.integer("LocalID", parcel.local_id);
    xml.vector("AABBMax", max);
    xml.vector("AABBMin", min);
    xml.integer("Area", parcel.area);
    xml.integer("AuctionID", parcel.auction as i32);
    xml.uuid("AuthBuyerID", parcel.auth_buyer);
    xml.binary("Bitmap", bitmap);
    xml.integer("Category", parcel.category);
    xml.date("ClaimDate", parcel.claim_date);
    xml.integer("ClaimPrice", parcel.claim_price);
    xml.string("Desc", &parcel.description);
    xml.uint_binary("ParcelFlags", parcel.flags);
    xml.uuid("GroupID", parcel.group);
    xml.integer("GroupPrims", send.prims.group);
    xml.boolean("IsGroupOwned", parcel.group_owned);
    xml.integer("LandingType", i32::from(parcel.landing_type));
    xml.integer("MaxPrims", max_prims);
    xml.uuid("MediaID", parcel.media_id);
    xml.string("MediaURL", &parcel.media_url);
    xml.boolean("MediaAutoScale", parcel.media_auto_scale != 0);
    xml.string("MusicURL", &parcel.music_url);
    xml.string("Name", &parcel.name);
    xml.integer("OtherCleanTime", parcel.other_clean_time);
    xml.integer("OtherCount", 0);
    xml.integer("OtherPrims", send.prims.others);
    xml.uuid("OwnerID", parcel.owner);
    xml.integer("OwnerPrims", send.prims.owner);
    xml.real("ParcelPrimBonus", send.object_bonus as f32);
    xml.real("PassHours", parcel.pass_hours);
    xml.integer("PassPrice", parcel.pass_price);
    xml.integer("PublicCount", 0);
    xml.boolean("RegionDenyAnonymous", flag(send, DENY_ANONYMOUS));
    xml.boolean("RegionDenyIdentified", flag(send, DENY_IDENTIFIED));
    xml.boolean("RegionDenyTransacted", flag(send, DENY_TRANSACTED));
    xml.boolean("RegionPushOverride", flag(send, RESTRICT_PUSH));
    xml.integer("RentPrice", 0);
    xml.integer("RequestResult", send.request_result);
    xml.integer("SalePrice", parcel.sale_price);
    xml.integer("SelectedPrims", send.prims.selected);
    xml.integer("SelfCount", 0);
    xml.integer("SequenceID", send.sequence_id);
    xml.integer("SimWideMaxPrims", sim_prims);
    xml.integer("SimWideTotalPrims", send.prims.simulator);
    xml.boolean("SnapSelection", send.snap_selection);
    xml.uuid("SnapshotID", parcel.snapshot);
    xml.integer("Status", parcel.status);
    xml.integer("TotalPrims", send.prims.total);
    xml.vector("UserLocation", parcel.location);
    xml.vector("UserLookAt", parcel.look_at);
    xml.boolean("SeeAVs", parcel.see_avs);
    xml.boolean("AnyAVSounds", parcel.any_av_sounds);
    xml.boolean("GroupAVSounds", parcel.group_av_sounds);
    xml.block_close();

    xml.block_open("MediaData");
    xml.string("MediaDesc", &parcel.media_description);
    xml.integer("MediaHeight", parcel.media_size[1]);
    xml.integer("MediaWidth", parcel.media_size[0]);
    xml.boolean("MediaLoop", parcel.media_loop);
    xml.string("MediaType", &parcel.media_type);
    xml.boolean("ObscureMedia", false);
    xml.boolean("ObscureMusic", false);
    xml.block_close();

    xml.block_open("ParcelExtendedFlags");
    xml.uint_binary("Flags", if parcel.obscure_media { 1 } else { 0 });
    xml.block_close();

    xml.block_open("AgeVerificationBlock");
    xml.boolean("RegionDenyAgeUnverified", flag(send, DENY_AGE));
    xml.block_close();

    let allow_environment = flag(send, ALLOW_ENVIRONMENT);
    let environment_version = if allow_environment {
        if send.agent_environment_version > 0 {
            -1
        } else {
            parcel.environment_version
        }
    } else {
        -1
    };
    xml.block_open("ParcelEnvironmentBlock");
    xml.integer("ParcelEnvironmentVersion", environment_version);
    xml.boolean("RegionAllowEnvironmentOverride", allow_environment);
    xml.block_close();

    xml.block_open("RegionAllowAccessBlock");
    xml.boolean("RegionAllowAccessOverride", !send.tax_free);
    xml.block_close();
    xml.push("</map></map>");
    xml.0
}

fn aabb(
    cells: &[u32],
    cells_x: usize,
    cells_y: usize,
    local_id: i32,
    heightmap: Option<&Heightmap>,
) -> ([f32; 3], [f32; 3]) {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let id = local_id as u32;
    for y in 0..cells_y {
        for x in 0..cells_x {
            if cells[y * cells_x + x] != id {
                continue;
            }
            let x = x as i32;
            let y = y as i32;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if min_x == i32::MAX {
        return ([0.0; 3], [0.0; 3]);
    }
    let region = region_metres(cells_x, cells_y);
    let mut tx = min_x.saturating_mul(4);
    if tx >= region[0] {
        tx = region[0] - 1;
    }
    let mut ty = min_y.saturating_mul(4);
    if ty >= region[1] {
        ty = region[1] - 1;
    }
    let z_min = sample(heightmap, tx, ty);
    max_x = max_x.saturating_add(1);
    max_y = max_y.saturating_add(1);
    let mut bx = max_x.saturating_mul(4);
    if bx > region[0] {
        bx = region[0];
    }
    let mut by = max_y.saturating_mul(4);
    if by > region[1] {
        by = region[1];
    }
    let z_max = sample(heightmap, bx - 1, by - 1);
    ([tx as f32, ty as f32, z_min], [bx as f32, by as f32, z_max])
}

fn sample(heightmap: Option<&Heightmap>, x: i32, y: i32) -> f32 {
    let Some(heightmap) = heightmap else {
        return 0.0;
    };
    if x < 0 || y < 0 || x >= heightmap.width() || y >= heightmap.height() {
        return 0.0;
    }
    heightmap.cells()[(y as usize) * (heightmap.width() as usize) + (x as usize)]
}

/// `ConvertLandBitmapToBytes`: Y outer, X inner, the low bit of each byte first.
fn land_bitmap(cells: &[u32], width: usize, local_id: u32) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    let height = cells.len() / width;
    let mut bytes = vec![0u8; width.saturating_mul(height) / 8];
    let mut temp = 0u32;
    let mut mask = 1u32;
    let mut byte_num = 0usize;
    for y in 0..height {
        for x in 0..width {
            if cells[y * width + x] == local_id {
                temp |= mask;
            }
            mask <<= 1;
            if mask == 0x100 {
                mask = 1;
                if let Some(slot) = bytes.get_mut(byte_num) {
                    *slot = temp as u8;
                }
                byte_num += 1;
                temp = 0;
            }
        }
    }
    if temp != 0
        && byte_num < 512
        && let Some(slot) = bytes.get_mut(byte_num)
    {
        *slot = temp as u8;
    }
    bytes
}

#[derive(Default)]
struct Xml(Vec<u8>);

impl Xml {
    fn push(&mut self, text: &str) {
        self.0.extend_from_slice(text.as_bytes());
    }

    fn block_open(&mut self, name: &str) {
        self.push("<key>");
        self.push(name);
        self.push("</key><array><map>");
    }

    fn block_close(&mut self) {
        self.push("</map></array>");
    }

    fn key(&mut self, name: &str) {
        self.push("<key>");
        self.push(name);
        self.push("</key>");
    }

    fn integer(&mut self, name: &str, value: i32) {
        self.key(name);
        if value == 0 {
            self.push("<integer />");
        } else {
            self.push("<integer>");
            self.push(&value.to_string());
            self.push("</integer>");
        }
    }

    fn uint_binary(&mut self, name: &str, value: u32) {
        self.binary(name, &value.to_be_bytes());
    }

    fn boolean(&mut self, name: &str, value: bool) {
        self.key(name);
        self.push(if value {
            "<boolean>1</boolean>"
        } else {
            "<boolean>0</boolean>"
        });
    }

    fn real_body(&mut self, value: f32) {
        if value == 0.0 {
            self.push("<real>0</real>");
            return;
        }
        self.push("<real>");
        self.push(&format_single(value));
        self.push("</real>");
    }

    fn real(&mut self, name: &str, value: f32) {
        self.key(name);
        self.real_body(value);
    }

    fn vector(&mut self, name: &str, value: [f32; 3]) {
        self.key(name);
        self.push("<array>");
        for component in value {
            self.real_body(component);
        }
        self.push("</array>");
    }

    fn uuid(&mut self, name: &str, value: Uuid) {
        self.key(name);
        if value.is_nil() {
            self.push("<uuid />");
        } else {
            self.push("<uuid>");
            self.push(&value.to_string());
            self.push("</uuid>");
        }
    }

    fn string(&mut self, name: &str, value: &str) {
        self.key(name);
        if value.is_empty() {
            self.push("<string />");
        } else {
            self.push("<string>");
            escape(value, &mut self.0);
            self.push("</string>");
        }
    }

    fn binary(&mut self, name: &str, value: &[u8]) {
        self.key(name);
        if value.is_empty() {
            self.push("<binary />");
        } else {
            self.push("<binary>");
            let encoded = base64::engine::general_purpose::STANDARD.encode(value);
            self.push(&encoded);
            self.push("</binary>");
        }
    }

    fn date(&mut self, name: &str, unix: i32) {
        self.key(name);
        match format_unix(unix) {
            None => self.push("<date />"),
            Some(text) => {
                self.push("<date>");
                self.push(&text);
                self.push("</date>");
            }
        }
    }
}

/// `OSDParser.EscapeToXML` (`XmlLLSD.cs` 309–337). Apostrophe becomes
/// `&apos;`. A backslash is left as itself.
fn escape(value: &str, out: &mut Vec<u8>) {
    let mut text = String::new();
    ferroan_llsd::xml::escape(value, &mut text);
    out.extend_from_slice(text.as_bytes());
}

/// `float.ToString(CultureInfo.InvariantCulture)`, the general format.
///
/// Finite values take the spelling of [`crate::xml::format_f32`].
/// Encode2 writes `Infinity` and `-Infinity`, not U+221E.
fn format_single(value: f32) -> String {
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
    crate::xml::format_f32(value)
}

/// `Util.ToDateTime` then Encode2's date text. Unix 0 is the empty epoch.
fn format_unix(seconds: i32) -> Option<String> {
    if seconds == 0 {
        return None;
    }
    let days = seconds.div_euclid(86_400);
    let tod = seconds.rem_euclid(86_400);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    let (year, month, day) = civil_from_days(days);
    if hour == 0 && minute == 0 && second == 0 {
        Some(format!("{year:04}-{month:02}-{day:02}"))
    } else {
        Some(format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
        ))
    }
}

/// Howard Hinnant's `civil_from_days`. `days` counts from 1970-01-01.
fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = i64::from(days) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year as i32, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::initial::InitialTerrain;
    use crate::objects::ObjectGroup;
    use crate::parcels::open_parcels_at;
    use crate::test_store::CopiedStore;

    const REGION: Uuid = uuid::uuid!("11111111-1111-1111-1111-111111111111");
    /// The harness estate owner. The capture's default parcel is claimed by it.
    const ESTATE: Uuid = uuid::uuid!("4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55");
    /// Region start of the published capture: 2026-11-26T06:25:48Z.
    const CLAIM: i32 = 1_795_674_348;

    #[test]
    fn capacity_rounds_and_clamps_like_land_object() {
        assert_eq!(prim_capacity(65_472, 15_000, 1.0, 256, 256), 14_985);
        assert_eq!(prim_capacity(48, 15_000, 1.0, 256, 256), 11);
        assert_eq!(prim_capacity(16, 15_000, 1.0, 256, 256), 4);
        // West is 16 m². Bonus 2 on a 256 m region is 7.824, truncated to 7.
        assert_eq!(prim_capacity(16, 15_000, 2.0, 256, 256), 7);
        assert_eq!(prim_capacity(65_536, 15_000, 1.0, 256, 256), 15_000);
        assert_eq!(prim_capacity(65_536, 15_000, 2.0, 256, 256), 15_000);
        assert_eq!(prim_capacity(0, 15_000, 1.0, 256, 256), 0);
    }

    #[test]
    fn restrict_pushing_and_bonus_two_match_the_west_element() {
        let mut west = super::super::Parcel::estate_land(ESTATE, CLAIM);
        west.name = "West".into();
        let mut map = ParcelMap::empty([256, 256]).unwrap();
        map.insert(super::super::StoredParcel {
            data: west,
            bitmap: vec![1],
        });
        let local_id = map.parcels()[0].local_id;
        let region = ParcelRegion::from_handshake(1 << 22, 2.0, 15_000, false, true);
        assert_eq!(region.region_flags & (1 << 22), 1 << 22);
        assert_eq!(region.region_flags & (1 << 9), 1 << 9);
        let bytes = map
            .parcel_properties(local_id, &region.element(PrimCounts::default()))
            .unwrap();
        let xml = String::from_utf8(bytes).unwrap();
        assert!(
            xml.contains("<key>Name</key><string>West</string>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>ParcelPrimBonus</key><real>2</real>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>MaxPrims</key><integer>7</integer>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>RegionPushOverride</key><boolean>1</boolean>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>RegionAllowEnvironmentOverride</key><boolean>1</boolean>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>RegionAllowAccessOverride</key><boolean>1</boolean>"),
            "{xml}"
        );
        let taxed = ParcelRegion::from_handshake(0, 1.0, 15_000, true, false);
        let taxed = map
            .parcel_properties(local_id, &taxed.element(PrimCounts::default()))
            .unwrap();
        let taxed = String::from_utf8(taxed).unwrap();
        assert!(
            taxed.contains("<key>RegionAllowAccessOverride</key><boolean>0</boolean>"),
            "{taxed}"
        );
        assert!(
            taxed.contains("<key>RegionPushOverride</key><boolean>0</boolean>"),
            "{taxed}"
        );
        let denied = ParcelRegion::from_handshake(
            (1 << 22) | (1 << 23) | (1 << 24) | (1 << 25) | (1 << 30),
            2.0,
            15_000,
            false,
            true,
        );
        let denied = map
            .parcel_properties(local_id, &denied.element(PrimCounts::default()))
            .unwrap();
        let denied = String::from_utf8(denied).unwrap();
        assert!(
            denied.contains("<key>RegionDenyAnonymous</key><boolean>0</boolean>"),
            "{denied}"
        );
        assert!(
            denied.contains("<key>RegionDenyIdentified</key><boolean>0</boolean>"),
            "{denied}"
        );
        assert!(
            denied.contains("<key>RegionDenyTransacted</key><boolean>0</boolean>"),
            "{denied}"
        );
        assert!(
            denied.contains("<key>RegionDenyAgeUnverified</key><boolean>0</boolean>"),
            "{denied}"
        );
        assert!(
            denied.contains("<key>RegionPushOverride</key><boolean>1</boolean>"),
            "{denied}"
        );
        assert!(
            denied.contains("<key>RegionAllowEnvironmentOverride</key><boolean>1</boolean>"),
            "{denied}"
        );
    }

    #[test]
    fn admitted_groups_tally_owner_group_other_and_simwide() {
        let alice = uuid::uuid!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
        let bob = uuid::uuid!("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        let group = uuid::uuid!("cccccccc-cccc-cccc-cccc-cccccccccccc");
        let mut west = super::super::Parcel::estate_land(alice, CLAIM);
        west.group = group;
        west.name = "West".into();
        let east = super::super::Parcel::estate_land(alice, CLAIM);
        let mut map = ParcelMap::empty([16, 16]).unwrap();
        map.insert(super::super::StoredParcel {
            data: west,
            bitmap: vec![0b0001],
        });
        map.insert(super::super::StoredParcel {
            data: east,
            bitmap: vec![0b0010],
        });
        let placed = |owner: Uuid,
                      group_id: Uuid,
                      position: [f32; 3],
                      parts: usize,
                      state: u8,
                      flags: u32| {
            if parts == 0 {
                return ObjectGroup {
                    from_item_id: Uuid::nil(),
                    rez_parameter: None,
                    parts: Vec::new(),
                };
            }
            let mut root = crate::xml::blank_part();
            root.owner_id = owner;
            root.group_id = group_id;
            root.group_position = position;
            root.shape.pcode = 9;
            root.shape.state = state;
            root.flags = flags;
            let mut pieces = vec![root];
            for _ in 1..parts {
                pieces.push(crate::xml::blank_part());
            }
            ObjectGroup {
                from_item_id: Uuid::nil(),
                rez_parameter: None,
                parts: pieces,
            }
        };
        let groups = vec![
            placed(alice, Uuid::nil(), [1.0, 1.0, 0.0], 2, 0, 0),
            placed(bob, group, [1.0, 1.0, 0.0], 3, 0, 0),
            placed(bob, Uuid::nil(), [1.0, 1.0, 0.0], 1, 0, 0),
            placed(alice, Uuid::nil(), [5.0, 1.0, 0.0], 4, 0, 0),
            placed(alice, Uuid::nil(), [1.0, 1.0, 0.0], 9, 1, 0),
            placed(alice, Uuid::nil(), [1.0, 1.0, 0.0], 8, 0, 0x2000_0000),
            placed(alice, Uuid::nil(), [1.0, 1.0, 0.0], 0, 0, 0),
            placed(alice, Uuid::nil(), [-1.0, 1.0, 0.0], 5, 0, 0),
        ];
        let counts = prim_counts(&map, &groups);
        let west_id = map.at_metres(1.0, 1.0).unwrap().id;
        let east_id = map.at_metres(5.0, 1.0).unwrap().id;
        assert_eq!(
            counts[&west_id],
            PrimCounts {
                owner: 11,
                group: 3,
                others: 1,
                selected: 0,
                total: 15,
                simulator: 19,
            }
        );
        assert_eq!(
            counts[&east_id],
            PrimCounts {
                owner: 4,
                group: 0,
                others: 0,
                selected: 0,
                total: 4,
                simulator: 19,
            }
        );
    }

    #[test]
    fn claim_dates_use_encode2() {
        assert_eq!(format_unix(0), None);
        assert_eq!(
            format_unix(1_700_000_000).as_deref(),
            Some("2023-11-14T22:13:20Z")
        );
        assert_eq!(format_unix(CLAIM).as_deref(), Some("2026-11-26T06:25:48Z"));
        assert_eq!(format_unix(86_400).as_deref(), Some("1970-01-02"));
    }

    #[test]
    fn strings_escape_apostrophe_and_reals_use_an_exponent() {
        let mut escaped = Vec::new();
        escape("Bob's Place", &mut escaped);
        assert_eq!(escaped, b"Bob&apos;s Place");
        escaped.clear();
        escape(r"C:\land", &mut escaped);
        assert_eq!(escaped, br"C:\land");
        assert_eq!(format_single(-4.371139e-8), "-4.371139E-08");
        assert_eq!(format_single(1e-5), "1E-05");
        assert_eq!(format_single(1e9), "1E+09");
        assert_eq!(format_single(f32::INFINITY), "Infinity");
        assert_eq!(format_single(f32::NEG_INFINITY), "-Infinity");
    }

    #[tokio::test]
    async fn bounding_box_heights_follow_each_metre_corner() {
        let mut map = ParcelMap::empty([32, 32]).unwrap();
        let mut bitmap = vec![0u8; 8];
        for (x, y) in [(1u32, 0u32), (2, 0), (1, 3), (2, 3)] {
            let n = y * 8 + x;
            bitmap[n as usize / 8] |= 1 << (n % 8);
        }
        map.insert(super::super::StoredParcel {
            data: super::super::Parcel::estate_land(ESTATE, CLAIM),
            bitmap,
        });
        let mut heights = vec![0.0f32; 32 * 32];
        heights[4] = 12.5;
        heights[4 * 32] = 99.0;
        heights[15 * 32 + 11] = 40.5;
        heights[11 * 32 + 15] = 7.0;
        let heightmap = super::Heightmap::from_cells(32, 32, heights);
        map.update_geometry(Some(&heightmap));
        let local_id = map.parcels()[0].local_id;
        let send = ParcelPropertiesSend {
            sequence_id: 0,
            snap_selection: false,
            request_result: 0,
            region_flags: 0,
            tax_free: false,
            object_capacity: 15_000,
            object_bonus: 1.0,
            prims: PrimCounts::default(),
            agent_environment_version: 0,
        };
        let bytes = map.parcel_properties(local_id, &send).unwrap();
        let xml = String::from_utf8(bytes).unwrap();
        assert!(
            xml.contains(
                "<key>AABBMin</key><array><real>4</real><real>0</real><real>12.5</real></array>"
            ),
            "{xml}"
        );
        assert!(
            xml.contains(
                "<key>AABBMax</key><array><real>12</real><real>16</real><real>40.5</real></array>"
            ),
            "{xml}"
        );
        assert!(!xml.contains("<real>99</real>"));
        assert!(!xml.contains("<real>7</real>"));
    }

    #[tokio::test]
    async fn parcel_element_bytes_grow_with_the_region() {
        let cases = [(2048u32, 46_750usize), (2560, 71_326), (4096, 177_823)];
        for (edge, expected) in cases {
            let mut map = open_parcels_at(None, REGION, [edge, edge], ESTATE, CLAIM)
                .await
                .unwrap();
            map.update_geometry(None);
            let local_id = map.parcels()[0].local_id;
            let send = ParcelPropertiesSend {
                sequence_id: 0,
                snap_selection: false,
                request_result: 0,
                region_flags: 0,
                tax_free: false,
                object_capacity: 15_000,
                object_bonus: 1.0,
                prims: PrimCounts::default(),
                agent_environment_version: 0,
            };
            let bytes = map.parcel_properties(local_id, &send).unwrap();
            assert_eq!(bytes.len(), expected, "{edge} m");
        }
    }

    #[tokio::test]
    async fn send_land_properties_matches_each_captured_element() {
        let copy = CopiedStore::new("regionstore/objects.db");
        let pool = ferroan_db::Pool::open(
            ferroan_db::Backend::Sqlite,
            &copy.connection(),
            &["RegionStore"],
        )
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../../../../fixtures/regionstore/parcels.sql"))
            .execute(pool.sqlite().unwrap())
            .await
            .unwrap();
        pool.close().await;
        let mut map = open_parcels_at(Some(&copy.connection()), REGION, [256, 256], ESTATE, CLAIM)
            .await
            .unwrap();
        let height = InitialTerrain::PinheadIsland.generate(256, 256).unwrap();
        map.update_geometry(Some(&height));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/parcel-properties");
        let moments = [
            ("arrival.bin", "Your Parcel", 0, 0, "KAAgSw=="),
            ("request-cell.bin", "West", 7, 0, "gACABA=="),
            ("request-span-west.bin", "West", 8, 1, "gACABA=="),
            ("request-span-east.bin", "East", 8, 1, "gACABA=="),
            ("walk.bin", "East", 0, 0, "gACABA=="),
        ];
        let mut saw_empty_uuid = false;
        for (file, name, sequence, result, flags) in moments {
            let local_id = map
                .parcels()
                .iter()
                .find(|parcel| parcel.name == name)
                .unwrap()
                .local_id;
            let send = ParcelPropertiesSend {
                sequence_id: sequence,
                snap_selection: false,
                request_result: result,
                region_flags: 0,
                tax_free: false,
                object_capacity: 15_000,
                object_bonus: 1.0,
                prims: PrimCounts::default(),
                agent_environment_version: 0,
            };
            let got = map.parcel_properties(local_id, &send).unwrap();
            let expected = std::fs::read(root.join(file)).unwrap();
            assert_eq!(
                got,
                expected,
                "{file} first difference at {}",
                mismatch(&got, &expected)
            );
            assert!(
                got.windows(b"<integer />".len())
                    .any(|window| window == b"<integer />"),
                "{file} zero integer"
            );
            // West's buyer, group, media and snapshot ids are all set. Arrival
            // and East carry Encode2's empty uuid.
            saw_empty_uuid |= got
                .windows(b"<uuid />".len())
                .any(|window| window == b"<uuid />");
            let flags = format!("<binary>{flags}</binary>");
            assert!(
                got.windows(flags.len())
                    .any(|window| window == flags.as_bytes()),
                "{file} flags binary"
            );
            assert!(
                got.windows(b"<key>Bitmap</key><binary>".len())
                    .any(|window| window == b"<key>Bitmap</key><binary>"),
                "{file} bitmap"
            );
            assert!(
                got.windows(b"<key>AABBMax</key><array><real>".len())
                    .any(|window| window == b"<key>AABBMax</key><array><real>"),
                "{file} vector"
            );
        }
        assert!(saw_empty_uuid, "an empty uuid");
    }

    fn mismatch(got: &[u8], expected: &[u8]) -> String {
        let index = got
            .iter()
            .zip(expected.iter())
            .position(|(left, right)| left != right)
            .unwrap_or(got.len().min(expected.len()));
        let end = (index + 48).min(got.len());
        let text = String::from_utf8_lossy(&got[index..end]);
        format!("{index} ({text})")
    }
}
