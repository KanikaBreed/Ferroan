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
// Ported from LandManagementModule.SendParcelOverlay and LandChannel.cs at Tranquillity
// c401bceecd1684832ae8ba86a31b5ee01a4ded7b. ParcelFlags follow OpenMetaverse/ParcelManager.cs
// at libopenmetaverse a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.

// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{Parcel, ParcelMap};
use uuid::Uuid;

/// The viewer's ParcelData block. The last packet is padded to the same size.
pub struct OverlayPacket {
    pub sequence_id: i32,
    pub data: [u8; 1024],
}

impl ParcelMap {
    /// `sendClientInitialLandInfo`: a root with no parcel under them returns
    /// before the overlay. A child skips that lookup and still receives it.
    pub fn arrival_overlay(
        &self,
        requester: Uuid,
        position: [f32; 3],
        child: bool,
        is_member: impl Fn(Uuid) -> bool,
    ) -> Vec<OverlayPacket> {
        if !child && self.at_metres(position[0], position[1]).is_none() {
            return Vec::new();
        }
        self.overlay(requester, is_member).collect()
    }

    /// Encode the bounded region map from southwest to northeast in 4 m cells.
    /// Membership is tested in each parcel's group independently of the active group.
    pub fn overlay(
        &self,
        requester: Uuid,
        is_member: impl Fn(Uuid) -> bool,
    ) -> impl Iterator<Item = OverlayPacket> + '_ {
        // The cell map indexes parcels from one. Zero marks an unassigned cell.
        let types: Vec<_> = self
            .parcels
            .iter()
            .map(|p| parcel_byte(p, requester, &is_member))
            .collect();
        self.cells
            .chunks(1024)
            .enumerate()
            .map(move |(sequence, cells)| {
                let mut data = [0; 1024];
                for (offset, &id) in cells.iter().enumerate() {
                    if id == 0 {
                        continue;
                    }
                    let index = sequence * 1024 + offset;
                    let mut byte = types[id as usize - 1];
                    if index.is_multiple_of(self.width) || self.cells[index - 1] != id {
                        byte |= 0x40;
                    }
                    if index < self.width || self.cells[index - self.width] != id {
                        byte |= 0x80;
                    }
                    data[offset] = byte;
                }
                // ParcelMap's checked region dimensions bound the packet count to 1024.
                OverlayPacket {
                    sequence_id: sequence as i32,
                    data,
                }
            })
    }
}

const FOR_SALE: u32 = 1 << 2;
const SOUND_LOCAL: u32 = 1 << 15;

fn parcel_byte(parcel: &Parcel, requester: Uuid, is_member: &impl Fn(Uuid) -> bool) -> u8 {
    let mut byte = if parcel.owner == requester {
        3
    } else if parcel.group_owned && is_member(parcel.group) {
        2
    } else if parcel.flags & FOR_SALE != 0
        && (parcel.auth_buyer.is_nil() || parcel.auth_buyer == requester)
    {
        4
    } else if parcel.owner.is_nil() {
        0
    } else {
        1
    };
    if parcel.flags & SOUND_LOCAL != 0 {
        byte |= 0x20;
    }
    if !parcel.see_avs {
        byte |= 0x10;
    }
    byte
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parcels::open_parcels, test_store::CopiedStore};
    use ferroan_db::{Backend, Pool};
    use std::path::Path;

    const OWNER: Uuid = uuid::uuid!("4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55");
    const MEMBER: Uuid = uuid::uuid!("8c4e1a70-2b59-4d6f-9e13-5a7c0d8b2f44");
    const HOME: Uuid = uuid::uuid!("6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01");
    const SEED_REGION: Uuid = uuid::uuid!("11111111-1111-1111-1111-111111111111");

    #[tokio::test]
    async fn stored_parcel_overlays_match_every_pin8_sequence_and_payload_byte() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/parcel-overlay");
        for seed in ["empty", "multi", "lone", "legacy", "original"] {
            let store = CopiedStore::new("regionstore/objects.db");
            let pool = Pool::open(Backend::Sqlite, &store.connection(), &["RegionStore"])
                .await
                .unwrap();
            let sqlite = pool.sqlite().unwrap();
            if seed != "empty" {
                sqlx::raw_sql(include_str!("../../../../fixtures/regionstore/parcels.sql"))
                    .execute(sqlite)
                    .await
                    .unwrap();
                if matches!(seed, "lone" | "legacy") {
                    let sql =
                        std::fs::read_to_string(fixtures.join(format!("{seed}.sql"))).unwrap();
                    sqlx::raw_sql(&sql).execute(sqlite).await.unwrap();
                }
                if seed != "original" {
                    sqlx::raw_sql(include_str!(
                        "../../../../fixtures/parcel-overlay/home-region.sql"
                    ))
                    .execute(sqlite)
                    .await
                    .unwrap();
                }
            }
            pool.close().await;
            let region = if seed == "original" {
                SEED_REGION
            } else {
                HOME
            };
            let width = if seed == "legacy" { 512 } else { 256 };
            let map = open_parcels(Some(&store.connection()), region, [width, width], OWNER)
                .await
                .unwrap();
            let capture = if seed == "original" { "multi" } else { seed };
            for (viewer, agent) in [("owner", OWNER), ("member", MEMBER)] {
                let raw =
                    std::fs::read(fixtures.join(format!("parcel-{capture}-{viewer}.bin"))).unwrap();
                let count = if seed == "legacy" { 16 } else { 4 };
                assert_eq!(raw.len(), count * 1030);
                let mut packets = map.overlay(agent, |group| {
                    agent == MEMBER && group == Uuid::from_u128(3)
                });
                for (index, body) in raw.as_chunks::<1030>().0.iter().enumerate() {
                    let packet = packets.next().unwrap();
                    assert_eq!(packet.sequence_id as usize, index);
                    assert_eq!(body[..4], packet.sequence_id.to_le_bytes());
                    assert_eq!(body[4..6], 1024u16.to_le_bytes());
                    if let Some(cell) = body[6..].iter().zip(packet.data).position(|(&a, b)| a != b)
                    {
                        panic!(
                            "{seed}/{viewer} sequence {index} cell {cell}: Pin8 {:#04x}, Rust {:#04x}",
                            body[6 + cell],
                            packet.data[cell]
                        );
                    }
                }
                assert!(packets.next().is_none());
            }
        }
    }

    #[test]
    fn a_root_with_no_parcel_gets_no_overlay_and_a_child_still_does() {
        let map = ParcelMap::empty([16, 16]).unwrap();
        let root = map.arrival_overlay(OWNER, [8.0, 8.0, 25.0], false, |_| false);
        assert!(root.is_empty());
        let child = map.arrival_overlay(OWNER, [8.0, 8.0, 25.0], true, |_| false);
        assert_eq!(child.len(), 1);
        assert_eq!(child[0].sequence_id, 0);
        assert_eq!(child[0].data, [0; 1024]);
        assert!(map.at_metres(-1.0, 8.0).is_none());
        assert!(map.at_metres(16.0, 8.0).is_none());
    }

    #[test]
    fn a_cleared_cell_has_no_type_or_borders() {
        let map = ParcelMap::empty([16, 16]).unwrap();
        let packet = map.overlay(OWNER, |_| false).next().unwrap();
        assert_eq!(packet.sequence_id, 0);
        assert_eq!(packet.data, [0; 1024]);
    }

    #[tokio::test]
    async fn final_partial_overlay_preserves_the_row_border_and_pads_with_zeroes() {
        let map = open_parcels(None, HOME, [256, 80], OWNER).await.unwrap();
        let mut packets = map.overlay(OWNER, |_| false);
        assert_eq!(packets.next().unwrap().sequence_id, 0);
        let last = packets.next().unwrap();
        assert_eq!(last.sequence_id, 1);
        for (cell, &value) in last.data[..256].iter().enumerate() {
            assert_eq!(value, if cell.is_multiple_of(64) { 0x43 } else { 3 });
        }
        assert_eq!(last.data[256..], [0; 768]);
        assert!(packets.next().is_none());
    }
}
