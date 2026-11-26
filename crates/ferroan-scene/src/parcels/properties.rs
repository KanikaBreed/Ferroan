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
//! for the types this message uses. A `ParcelPropertiesRequest` selects the
//! parcels `ClientOnParcelPropertiesRequest` would send.

use std::collections::HashMap;

use base64::Engine;
use ferroan_diag::{Kind, PARCEL_REPLY, PARCEL_TEXT};
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
/// `Constants.LandUnit`. Parcel edges sit on this grid.
const LAND_UNIT: i32 = 4;
/// `LandChannel.LAND_RESULT_SINGLE` and `LAND_RESULT_MULTIPLE`.
const LAND_RESULT_SINGLE: i32 = 0;
const LAND_RESULT_MULTIPLE: i32 = 1;
/// `ParcelFlags.UseBanList` in OpenMetaverse 1.2.9.
const USE_BAN_LIST: u32 = 1024;
/// `AccessList.Ban`. The inner test compares equal to this value.
const ACCESS_BAN: u32 = 2;
/// One multi-parcel reply sends at most this many elements (D127).
const REPLY_CAP: usize = 256;
/// Bytes of parcel text allowed above the empty element for the region (D127).
const TEXT_BYTES: u64 = 8192;

/// One `ParcelPropertiesRequest` after the circuit has decoded it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropertiesRequest {
    pub west: f32,
    pub south: f32,
    pub east: f32,
    pub north: f32,
    pub sequence_id: i32,
    pub snap_selection: bool,
}

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

    /// Elements for one `ParcelPropertiesRequest`, in send order.
    ///
    /// `last_sequence` is `LandObject.m_lastSeqId`, shared by every client.
    /// Snap with sequence 0 reads it. Any other request stores the sequence
    /// it sends. A scan that selects more than one parcel is capped (D127).
    /// An element past [`text_limit`](Self::text_limit) is not sent.
    pub fn properties_request(
        &self,
        agent: Uuid,
        request: PropertiesRequest,
        now: i32,
        last_sequence: &mut HashMap<i32, i32>,
    ) -> Vec<Vec<u8>> {
        if self.parcels.is_empty() {
            return Vec::new();
        }
        let [west, south, east, north] = rounded_bounds(request);
        let [size_x, size_y] = region_metres(self.cell_size().0, self.cell_size().1);
        if west < 0 || south < 0 || east < 0 || north < 0 {
            return Vec::new();
        }
        if west >= size_x || south >= size_y || east > size_x || north > size_y {
            return Vec::new();
        }
        let (local_ids, multi) = if east - west <= LAND_UNIT && north - south <= LAND_UNIT {
            let Some(parcel) = self.at_metres(west as f32, south as f32) else {
                return Vec::new();
            };
            (vec![parcel.local_id], false)
        } else {
            (
                self.scan_request(agent, west, south, east, north, now),
                true,
            )
        };
        let mut local_ids = local_ids;
        if multi && local_ids.len() > 1 {
            let cap = reply_cap(size_x, size_y);
            if local_ids.len() > cap {
                ferroan_diag::drop_bound(
                    PARCEL_REPLY,
                    Kind::Hard,
                    cap as u64,
                    local_ids.len() as u64,
                    "parcel-properties",
                );
                local_ids.truncate(cap);
            }
        }
        if local_ids.is_empty() {
            return Vec::new();
        }
        let result = if local_ids.len() > 1 {
            LAND_RESULT_MULTIPLE
        } else {
            LAND_RESULT_SINGLE
        };
        let limit = self.text_limit();
        let mut elements = Vec::with_capacity(local_ids.len());
        for local_id in local_ids {
            let sequence_id = sequence_for(last_sequence, local_id, request);
            let send = ParcelPropertiesSend {
                sequence_id,
                snap_selection: request.snap_selection,
                request_result: result,
                ..ParcelPropertiesSend::notify()
            };
            let Some(element) = self.parcel_properties(local_id, &send) else {
                continue;
            };
            if element.len() as u64 > limit {
                ferroan_diag::refuse(
                    PARCEL_TEXT,
                    Kind::Hard,
                    limit,
                    element.len() as u64,
                    "parcel-properties",
                );
                continue;
            }
            elements.push(element);
        }
        elements
    }

    /// Empty strings plus this region's bitmap, then [`TEXT_BYTES`].
    fn text_limit(&self) -> u64 {
        let (width, height) = self.cell_size();
        let bitmap = vec![0u8; width.saturating_mul(height) / 8];
        let mut parcel = Parcel::estate_land(Uuid::nil(), 0);
        parcel.name.clear();
        parcel.media_type.clear();
        parcel.local_id = 0;
        parcel.area = 0;
        parcel.flags = 0;
        let bytes = encode(
            &parcel,
            &ParcelPropertiesSend::notify(),
            [0.0; 3],
            [0.0; 3],
            &bitmap,
            0,
            0,
        );
        bytes.len() as u64 + TEXT_BYTES
    }

    /// Land units, x outer and y inner. The first sight of a local id is kept
    /// unless that agent is banned from the parcel.
    fn scan_request(
        &self,
        agent: Uuid,
        west: i32,
        south: i32,
        east: i32,
        north: i32,
        now: i32,
    ) -> Vec<i32> {
        let mut ids = Vec::new();
        let x0 = west / LAND_UNIT;
        let y0 = south / LAND_UNIT;
        let x1 = east / LAND_UNIT;
        let y1 = north / LAND_UNIT;
        for x in x0..x1 {
            for y in y0..y1 {
                let Some(parcel) = self.at_cell(x as usize, y as usize) else {
                    continue;
                };
                if ids.contains(&parcel.local_id) || self.is_banned(parcel, agent, now) {
                    continue;
                }
                ids.push(parcel.local_id);
            }
        }
        ids
    }

    /// `LandObject.IsBannedFromLand` (`LandObject.cs` 873–902).
    ///
    /// Exemptions are `TaxFree`, the installed administrator rule, the estate
    /// owner, every estate manager and the parcel owner. The first `Ban`
    /// entry for the agent decides expiry. A later entry is not read.
    fn is_banned(&self, parcel: &Parcel, agent: Uuid, now: i32) -> bool {
        if self.exempts(agent, parcel.owner) {
            return false;
        }
        if parcel.flags & USE_BAN_LIST == 0 {
            return false;
        }
        parcel
            .access
            .iter()
            .find(|entry| entry.agent == agent && entry.flags == ACCESS_BAN)
            .is_some_and(|entry| entry.expires == 0 || entry.expires > now)
    }
}

/// `MathF.Round` is midpoint-to-even, then the saturating `(int)` cast.
/// NaN becomes 0 and an infinity becomes an `i32` limit, as .NET 10 does.
fn rounded_bounds(request: PropertiesRequest) -> [i32; 4] {
    [
        round_half_to_even(request.west),
        round_half_to_even(request.south),
        round_half_to_even(request.east),
        round_half_to_even(request.north),
    ]
}

fn round_half_to_even(value: f32) -> i32 {
    value.round_ties_even() as i32
}

fn reply_cap(metres_x: i32, metres_y: i32) -> usize {
    let tiles_x = metres_x / 64;
    let tiles_y = metres_y / 64;
    if tiles_x <= 0 || tiles_y <= 0 {
        return 0;
    }
    (tiles_x as usize)
        .saturating_mul(tiles_y as usize)
        .min(REPLY_CAP)
}

fn sequence_for(
    last_sequence: &mut HashMap<i32, i32>,
    local_id: i32,
    request: PropertiesRequest,
) -> i32 {
    if request.snap_selection && request.sequence_id == 0 {
        last_sequence.get(&local_id).copied().unwrap_or(0)
    } else {
        last_sequence.insert(local_id, request.sequence_id);
        request.sequence_id
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

        let mut last = std::collections::HashMap::new();
        let cell = map.properties_request(
            ESTATE,
            PropertiesRequest {
                west: 5.0,
                south: 1.0,
                east: 6.0,
                north: 2.0,
                sequence_id: 7,
                snap_selection: false,
            },
            0,
            &mut last,
        );
        assert_eq!(
            cell,
            vec![std::fs::read(root.join("request-cell.bin")).unwrap()]
        );
        let span = map.properties_request(
            ESTATE,
            PropertiesRequest {
                west: 4.0,
                south: 0.0,
                east: 12.0,
                north: 4.0,
                sequence_id: 8,
                snap_selection: false,
            },
            0,
            &mut last,
        );
        assert_eq!(
            span,
            vec![
                std::fs::read(root.join("request-span-west.bin")).unwrap(),
                std::fs::read(root.join("request-span-east.bin")).unwrap(),
            ]
        );
        let west_id = map
            .parcels()
            .iter()
            .find(|parcel| parcel.name == "West")
            .unwrap()
            .local_id;
        assert_eq!(last.get(&west_id), Some(&8));
        let snap = map.properties_request(
            ESTATE,
            PropertiesRequest {
                west: 5.0,
                south: 1.0,
                east: 6.0,
                north: 2.0,
                sequence_id: 0,
                snap_selection: true,
            },
            0,
            &mut last,
        );
        let xml = String::from_utf8(snap.into_iter().next().unwrap()).unwrap();
        assert!(
            xml.contains("<key>SequenceID</key><integer>8</integer>"),
            "{xml}"
        );
        assert!(
            xml.contains("<key>SnapSelection</key><boolean>1</boolean>"),
            "{xml}"
        );
        assert_eq!(last.get(&west_id), Some(&8));
        for (west, south, east, north) in [
            (-1.0, 0.0, 4.0, 4.0),
            (0.0, -1.0, 4.0, 4.0),
            (256.0, 0.0, 260.0, 4.0),
            (0.0, 0.0, 257.0, 4.0),
            (0.0, 256.0, 4.0, 260.0),
        ] {
            assert!(
                map.properties_request(
                    ESTATE,
                    PropertiesRequest {
                        west,
                        south,
                        east,
                        north,
                        sequence_id: 1,
                        snap_selection: false,
                    },
                    0,
                    &mut last,
                )
                .is_empty(),
                "{west},{south},{east},{north}"
            );
        }
    }

    #[test]
    fn half_metres_round_to_even() {
        assert_eq!(round_half_to_even(0.5), 0);
        assert_eq!(round_half_to_even(1.5), 2);
        assert_eq!(round_half_to_even(2.5), 2);
        assert_eq!(round_half_to_even(4.5), 4);
        assert_eq!(round_half_to_even(-0.5), 0);
        assert_eq!(round_half_to_even(-1.5), -2);
        assert_eq!(round_half_to_even(5.0), 5);
        assert_eq!(round_half_to_even(f32::NAN), 0);
        assert_eq!(round_half_to_even(f32::INFINITY), i32::MAX);
        assert_eq!(round_half_to_even(f32::NEG_INFINITY), i32::MIN);
        assert_eq!(round_half_to_even(3.0e9), i32::MAX);
        assert_ne!(round_half_to_even(0.5), 1);
    }

    /// NaN rounds to 0, so the request is the parcel at the origin.
    #[test]
    fn a_non_finite_bound_selects_the_origin_parcel() {
        let map = two_parcels(256);
        let mut last = std::collections::HashMap::new();
        let got = names(&map.properties_request(
            ESTATE,
            PropertiesRequest {
                west: f32::NAN,
                south: f32::NAN,
                east: f32::NAN,
                north: f32::NAN,
                sequence_id: 1,
                snap_selection: false,
            },
            0,
            &mut last,
        ));
        assert_eq!(got, vec!["West".to_string()]);
    }

    #[test]
    fn a_banned_parcel_is_skipped_on_a_span_and_sent_for_one_cell() {
        let mut map = two_parcels(256);
        let owner = map.parcels[0].owner;
        let banned = uuid::uuid!("00000000-0000-0000-0000-000000000012");
        map.parcels[0].access.push(super::super::LandAccess {
            agent: banned,
            flags: ACCESS_BAN,
            expires: 0,
        });
        map.parcels[0].access.push(super::super::LandAccess {
            agent: owner,
            flags: ACCESS_BAN,
            expires: 0,
        });
        let mut last = std::collections::HashMap::new();
        let span = PropertiesRequest {
            west: 0.0,
            south: 0.0,
            east: 256.0,
            north: 4.0,
            sequence_id: 8,
            snap_selection: false,
        };
        let skipped = names(&map.properties_request(banned, span, 1_700_000_000, &mut last));
        assert_eq!(skipped, vec!["East".to_string()]);
        let single = b"<key>RequestResult</key><integer />";
        assert!(
            map.properties_request(banned, span, 1_700_000_000, &mut last)[0]
                .windows(single.len())
                .any(|window| window == single)
        );
        let both = names(&map.properties_request(owner, span, 1_700_000_000, &mut last));
        assert_eq!(both, vec!["West".to_string(), "East".to_string()]);
        let estate = names(&map.properties_request(ESTATE, span, 1_700_000_000, &mut last));
        assert_eq!(estate, vec!["West".to_string(), "East".to_string()]);
        map.set_tax_free(true);
        let free = names(&map.properties_request(banned, span, 1_700_000_000, &mut last));
        assert_eq!(free, vec!["West".to_string(), "East".to_string()]);
        map.set_tax_free(false);
        map.parcels[0].access[0].expires = 1_700_000_000;
        let expired = names(&map.properties_request(banned, span, 1_700_000_000, &mut last));
        assert_eq!(expired, vec!["West".to_string(), "East".to_string()]);
        map.parcels[0].access[0].expires = 1_700_000_001;
        let still = names(&map.properties_request(banned, span, 1_700_000_000, &mut last));
        assert_eq!(still, vec!["East".to_string()]);
        map.parcels[0].access[0].flags = 3;
        let both_flag = names(&map.properties_request(banned, span, 1_700_000_000, &mut last));
        assert_eq!(both_flag, vec!["West".to_string(), "East".to_string()]);
        map.parcels[0].access[0].flags = ACCESS_BAN;
        map.parcels[0].access[0].expires = 0;
        let cell = PropertiesRequest {
            west: 0.0,
            south: 0.0,
            east: 4.0,
            north: 4.0,
            sequence_id: 7,
            snap_selection: false,
        };
        let one = names(&map.properties_request(banned, cell, 1_700_000_000, &mut last));
        assert_eq!(one, vec!["West".to_string()]);
    }

    /// The estate row's owner and a manager stay in a span when West bans them.
    /// The INI owner that `open_parcels` stored first does not. An administrator
    /// uses the installed god rule and stays as well.
    #[test]
    fn a_stored_owner_and_a_banned_manager_stay_in_the_span() {
        let mut map = two_parcels(256);
        let stored = uuid::uuid!("00000000-0000-0000-0000-0000000000aa");
        let manager = uuid::uuid!("00000000-0000-0000-0000-0000000000bb");
        let ini = ESTATE;
        map.set_estate(Some(stored), vec![manager], false);
        for agent in [stored, manager, ini] {
            map.parcels[0].access.push(super::super::LandAccess {
                agent,
                flags: ACCESS_BAN,
                expires: 0,
            });
        }
        let mut last = std::collections::HashMap::new();
        let span = PropertiesRequest {
            west: 0.0,
            south: 0.0,
            east: 256.0,
            north: 4.0,
            sequence_id: 8,
            snap_selection: false,
        };
        assert_eq!(
            names(&map.properties_request(stored, span, 0, &mut last)),
            vec!["West".to_string(), "East".to_string()]
        );
        assert_eq!(
            names(&map.properties_request(manager, span, 0, &mut last)),
            vec!["West".to_string(), "East".to_string()]
        );
        assert_eq!(
            names(&map.properties_request(ini, span, 0, &mut last)),
            vec!["East".to_string()]
        );
        let god = uuid::uuid!("00000000-0000-0000-0000-0000000000cc");
        map.parcels[0].access.push(super::super::LandAccess {
            agent: god,
            flags: ACCESS_BAN,
            expires: 0,
        });
        map.set_administrator(std::sync::Arc::new(move |agent| agent == god));
        assert_eq!(
            names(&map.properties_request(god, span, 0, &mut last)),
            vec!["West".to_string(), "East".to_string()]
        );
    }

    /// `IsBannedFromLand_inner` returns on the first Ban entry for the agent.
    #[test]
    fn the_first_ban_entry_decides_whether_it_has_expired() {
        let mut map = two_parcels(256);
        let agent = uuid::uuid!("00000000-0000-0000-0000-000000000012");
        map.parcels[0].access.push(super::super::LandAccess {
            agent,
            flags: ACCESS_BAN,
            expires: 10,
        });
        map.parcels[0].access.push(super::super::LandAccess {
            agent,
            flags: ACCESS_BAN,
            expires: 0,
        });
        let mut last = std::collections::HashMap::new();
        let span = PropertiesRequest {
            west: 0.0,
            south: 0.0,
            east: 256.0,
            north: 4.0,
            sequence_id: 4,
            snap_selection: false,
        };
        assert_eq!(
            names(&map.properties_request(agent, span, 11, &mut last)),
            vec!["West".to_string(), "East".to_string()]
        );
        map.parcels[0].access[0].expires = 0;
        map.parcels[0].access[1].expires = 10;
        assert_eq!(
            names(&map.properties_request(agent, span, 11, &mut last)),
            vec!["East".to_string()]
        );
    }

    #[test]
    fn a_multi_parcel_reply_keeps_the_prefix_and_a_long_text_is_refused() {
        let _guard = limits_lock();
        let mut narrow = two_parcels(32);
        let mut last = std::collections::HashMap::new();
        let span = |edge: f32| PropertiesRequest {
            west: 0.0,
            south: 0.0,
            east: edge,
            north: 4.0,
            sequence_id: 3,
            snap_selection: false,
        };
        let replies = hits("parcel.reply", "drop");
        let none = narrow.properties_request(ESTATE, span(32.0), 0, &mut last);
        assert!(none.is_empty());
        assert_eq!(hits("parcel.reply", "drop"), replies + 1);
        // A scan that selects one parcel stays under the multi-parcel cap, even at zero.
        let only = narrow.parcels[0].local_id as u32;
        narrow.parcels.pop();
        narrow.cells.fill(only);
        let one = narrow.properties_request(ESTATE, span(32.0), 0, &mut last);
        assert_eq!(one.len(), 1);
        assert_eq!(hits("parcel.reply", "drop"), replies + 1);

        let wide = two_parcels(64);
        let prefix = names(&wide.properties_request(ESTATE, span(64.0), 0, &mut last));
        assert_eq!(prefix, vec!["West".to_string()]);
        assert_eq!(hits("parcel.reply", "drop"), replies + 2);
        let single = b"<key>RequestResult</key><integer />";
        assert!(
            wide.properties_request(ESTATE, span(64.0), 0, &mut last)[0]
                .windows(single.len())
                .any(|window| window == single)
        );

        let mut text = two_parcels(64);
        text.parcels[0].description = "x".repeat(9_000);
        let texts = hits("parcel.text", "refuse");
        let cell = PropertiesRequest {
            west: 0.0,
            south: 0.0,
            east: 4.0,
            north: 4.0,
            sequence_id: 7,
            snap_selection: false,
        };
        assert!(
            text.properties_request(ESTATE, cell, 0, &mut last)
                .is_empty()
        );
        assert_eq!(hits("parcel.text", "refuse"), texts + 1);
        let limit = text.text_limit();
        assert!(limit > TEXT_BYTES);
        if texts == 0 {
            let rendered = ferroan_diag::render();
            assert!(
                rendered.contains(&format!("configured={limit}")),
                "{rendered}"
            );
        }
    }

    fn two_parcels(edge: u32) -> ParcelMap {
        let mut map = ParcelMap::empty([edge, edge]).unwrap();
        let cells = (edge / 4) as usize;
        let mut left = vec![0u8; cells * cells / 8];
        let mut right = vec![0u8; cells * cells / 8];
        for x in 0..cells / 2 {
            set_cell(&mut left, cells, x, 0);
            set_cell(&mut right, cells, cells / 2 + x, 0);
        }
        let mut west = super::super::Parcel::estate_land(
            uuid::uuid!("00000000-0000-0000-0000-000000000001"),
            1_700_000_000,
        );
        west.name = "West".into();
        west.flags = USE_BAN_LIST;
        let mut east = super::super::Parcel::estate_land(
            uuid::uuid!("00000000-0000-0000-0000-000000000003"),
            1_700_000_000,
        );
        east.name = "East".into();
        map.insert(super::super::StoredParcel {
            data: west,
            bitmap: left,
        });
        map.insert(super::super::StoredParcel {
            data: east,
            bitmap: right,
        });
        map.set_estate(Some(ESTATE), Vec::new(), false);
        map.update_geometry(None);
        map
    }

    fn set_cell(bitmap: &mut [u8], stride: usize, x: usize, y: usize) {
        let n = y * stride + x;
        bitmap[n / 8] |= 1 << (n % 8);
    }

    fn names(elements: &[Vec<u8>]) -> Vec<String> {
        elements
            .iter()
            .map(|element| {
                let xml = String::from_utf8(element.clone()).unwrap();
                let key = "<key>Name</key><string>";
                let start = xml.find(key).unwrap() + key.len();
                let end = xml[start..].find("</string>").unwrap();
                xml[start..start + end].to_string()
            })
            .collect()
    }

    fn hits(id: &str, action: &str) -> u64 {
        let needle = format!("ferroan_limit_hits_total{{limit=\"{id}\",action=\"{action}\"}} ");
        ferroan_diag::metrics()
            .lines()
            .find_map(|line| line.strip_prefix(&needle)?.parse().ok())
            .unwrap_or(0)
    }

    fn limits_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
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
