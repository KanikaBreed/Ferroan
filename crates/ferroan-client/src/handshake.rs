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
//     (SendRegionHandshake, GetRegionFlags)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPZeroEncoder.cs (AddShortString)
//   Source/OpenSim.Framework/Util.cs (osUTF8Getbytes, ConvertMaturityToAccessLevel)
//   Source/OpenSim.Framework/RegionSettings.cs
//   Source/OpenSim.Framework/EstateSettings.cs
//   Source/OpenSim.Region.Framework/Scenes/Scene.cs (terrain texture defaults)
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
//   OpenMetaverse/Simulator.cs (RegionFlags)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Region settings announced by LLClientView.SendRegionHandshake. The caller
//! supplies the region's current settings and per-agent permission/capability
//! results; this builder does not implement estate storage or terrain services.

use ferroan_protocol::msgs::{RegionHandshake, region_handshake};
use uuid::Uuid;

/// Current region/estate settings, separate from the circuit's agent identity.
#[derive(Debug, Clone)]
pub struct HandshakeConfig {
    pub region_id: Uuid,
    pub sim_name: String,
    pub product_name: String,
    pub sim_access: u8,
    /// LLClientView.GetRegionFlags, also sent unchanged in RegionInfo4.
    pub region_flags: u32,
    pub estate_owner: Uuid,
    pub water_height: f32,
    pub billable_factor: f32,
    /// Created once for a fresh region, not once per handshake or login.
    pub cache_id: Uuid,
    pub terrain_textures: [Uuid; 4],
    pub terrain_pbr: [Uuid; 4],
    /// C#'s wire order is SW, NW, SE, NE.
    pub terrain_start_heights: [f32; 4],
    /// C# sends Elevation2 directly, without subtracting Elevation1.
    pub terrain_height_ranges: [f32; 4],
    /// True only when the region's server-side baking module is enabled.
    pub server_side_baking: bool,
}

impl HandshakeConfig {
    /// Defaults of fresh RegionSettings/EstateSettings after scene creation.
    /// Persisted settings and negotiated viewer features are supplied by callers.
    pub fn for_region(region_id: Uuid) -> Self {
        let terrain = [
            Uuid::from_u128(0xb8d3965aad78bf43699bbff8eca6c975),
            Uuid::from_u128(0xabb783e63e9326c0248a247666855da3),
            Uuid::from_u128(0x179cdabd398a9b6b13914dc333ba321f),
            Uuid::from_u128(0xbeb169c711eafff2efe50f24dc881df2),
        ];
        Self {
            region_id,
            sim_name: String::new(),
            product_name: String::new(),
            sim_access: 13,
            // AllowLandmark, AllowSetHome, AllowParcelAccessOverride,
            // ExternallyVisible, AllowDirectTeleport, AllowParcelChanges, AllowVoice.
            region_flags: (1 << 1)
                | (1 << 2)
                | (1 << 5)
                | (1 << 15)
                | (1 << 20)
                | (1 << 26)
                | (1 << 28),
            estate_owner: Uuid::nil(),
            water_height: 20.0,
            billable_factor: 0.0,
            cache_id: Uuid::new_v4(),
            terrain_textures: terrain,
            terrain_pbr: terrain,
            terrain_start_heights: [10.0; 4],
            terrain_height_ranges: [60.0; 4],
            server_side_baking: false,
        }
    }

    pub fn build(&self, is_estate_manager: bool, supports_terrain_pbr: bool) -> RegionHandshake {
        let textures = if supports_terrain_pbr {
            self.terrain_pbr
        } else {
            self.terrain_textures
        };
        RegionHandshake {
            omitted_trailing_blocks: 0,
            region_info: region_handshake::RegionInfo {
                region_flags: self.region_flags,
                sim_access: self.sim_access,
                sim_name: short_string(&self.sim_name),
                sim_owner: self.estate_owner,
                is_estate_manager,
                water_height: self.water_height,
                billable_factor: self.billable_factor,
                cache_id: self.cache_id,
                terrain_base0: Uuid::nil(),
                terrain_base1: Uuid::nil(),
                terrain_base2: Uuid::nil(),
                terrain_base3: Uuid::nil(),
                terrain_detail0: textures[0],
                terrain_detail1: textures[1],
                terrain_detail2: textures[2],
                terrain_detail3: textures[3],
                terrain_start_height00: self.terrain_start_heights[0],
                terrain_start_height01: self.terrain_start_heights[1],
                terrain_start_height10: self.terrain_start_heights[2],
                terrain_start_height11: self.terrain_start_heights[3],
                terrain_height_range00: self.terrain_height_ranges[0],
                terrain_height_range01: self.terrain_height_ranges[1],
                terrain_height_range10: self.terrain_height_ranges[2],
                terrain_height_range11: self.terrain_height_ranges[3],
            },
            region_info2: region_handshake::RegionInfo2 {
                region_id: self.region_id,
            },
            region_info3: region_handshake::RegionInfo3 {
                cpu_class_id: 9,
                cpu_ratio: 1,
                colo_name: Vec::new(),
                product_sku: Vec::new(),
                product_name: short_string(&self.product_name),
            },
            region_info4: vec![region_handshake::RegionInfo4 {
                region_flags_extended: u64::from(self.region_flags),
                region_protocols: (1u64 << 63) | u64::from(self.server_side_baking),
            }],
        }
    }
}

/// AddShortString's 255 byte limit includes its optional trailing NUL.
/// Rust strings cannot contain the unpaired UTF-16 surrogates C# skips.
fn short_string(text: &str) -> Vec<u8> {
    let mut end = text.len().min(254);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut bytes = text.as_bytes()[..end].to_vec();
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        bytes.push(0);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_handshake_matches_the_csharp_capture_byte_for_byte() {
        use ferroan_protocol::{Msg, Writer, packet};

        // Public test-grid configuration from fixtures/circuit/manifest.json.
        // CacheID is random in C#; supply its captured value, not an encoder
        // round-trip or a normalizer that could hide a missing field.
        let mut config =
            HandshakeConfig::for_region(Uuid::from_u128(0x6d1a7f4e0c3b4f539a1e5f0e3c2a9b01));
        config.sim_name = "Ferroan Test".to_owned();
        config.estate_owner = Uuid::from_u128(0x4f0c8d2e7a514b8e9c3d2e6f1a0b7c55);
        config.cache_id = Uuid::from_u128(0xa99d0f3fee1a464cb6e6c9585885a0e5);
        let handshake = config.build(true, false);
        let mut writer = Writer::new();
        RegionHandshake::ID.encode(&mut writer.buf);
        handshake.encode_body(&mut writer);
        let mut datagram = Vec::new();
        packet::build_raw(&writer.buf, true, 1, true, false, &[], &mut datagram);
        assert_eq!(
            datagram,
            include_bytes!("../../../fixtures/circuit/region-handshake.bin")
        );
    }

    #[test]
    fn short_strings_keep_whole_utf8_and_existing_terminators() {
        assert!(short_string("").is_empty());
        assert_eq!(short_string("name"), b"name\0");
        assert_eq!(short_string("name\0"), b"name\0");
        assert_eq!(
            short_string(&format!("{}é", "a".repeat(253))),
            ["a".repeat(253).as_bytes(), &[0]].concat()
        );
        assert_eq!(short_string(&"😀".repeat(100)).len(), 253);
    }

    #[test]
    fn configured_fields_and_negotiated_terrain_keep_the_wire_corner_order() {
        let mut config = HandshakeConfig::for_region(Uuid::from_u128(1));
        config.terrain_start_heights = [1.0, 2.0, 3.0, 4.0];
        config.terrain_height_ranges = [5.0, 6.0, 7.0, 8.0];
        config.terrain_pbr = [Uuid::from_u128(2); 4];
        config.server_side_baking = true;
        let packet = config.build(true, true);
        assert_eq!(packet.region_info.terrain_start_height01, 2.0);
        assert_eq!(packet.region_info.terrain_start_height10, 3.0);
        assert_eq!(packet.region_info.terrain_height_range01, 6.0);
        assert_eq!(packet.region_info.terrain_height_range10, 7.0);
        assert_eq!(packet.region_info.terrain_detail0, Uuid::from_u128(2));
        assert_eq!(packet.region_info4[0].region_protocols, (1 << 63) | 1);
        assert_eq!(
            packet.region_info.cache_id,
            config.build(false, false).region_info.cache_id
        );
    }
}
