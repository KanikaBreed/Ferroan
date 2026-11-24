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
//     (MoveAgentIntoRegion, constructor)
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs (CompleteMovement)
//   Source/OpenSim.Framework/Util.cs (StringToBytes1024)
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
//   OpenMetaverse.Types/UtilsConversions.cs (StringToBytes, osUTF8GetBytesCount)
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The arrival notification for a root login. The region supplies the position
//! `MakeRootAgent` would use: the login start, lifted to terrain height when a
//! heightmap is loaded. ScenePresence's initial camera and velocity are zero,
//! so a fresh root login without an AgentUpdate uses its exact (not unit
//! length) fallback look vector. Land packets follow `AgentMovementComplete`.
//! Crossing, parcel and animation messages are later work. The avatar
//! `ObjectUpdate` and `AvatarAppearance` ride on `avatar` when the region has one.

use ferroan_protocol::msgs::AgentMovementComplete;

use crate::CircuitExpect;

/// Region and accepted-login data needed by LLClientView.MoveAgentIntoRegion.
#[derive(Clone, Debug)]
pub struct ArrivalConfig {
    pub position: [f32; 3],
    /// Resolved root-login heading, after ScenePresence.CompleteMovement.
    pub look_at: [f32; 3],
    pub region_handle: u64,
    /// Scene.GetSimulatorVersion, encoded as a bounded NUL-terminated string.
    pub channel_version: String,
    /// Land `LayerData` bodies, one patch per packet, sent after arrival.
    pub land: Vec<LandLayer>,
    /// Avatar full update and appearance. Absent on tests that only check arrival.
    pub avatar: Option<crate::AvatarWire>,
    /// Local teleport. Absent until the region installs its rectangle and ground.
    pub teleport: Option<crate::teleport::TeleportRegion>,
    /// Source hop for a destination outside this region. Absent in tests that
    /// only check a local landing, and on a circuit the region has not armed.
    pub source_hop: Option<crate::source_hop::SourceHop>,
    /// Child update and the promotion back to root. Absent on a circuit that
    /// cannot be a teleport destination.
    pub return_slot: Option<crate::return_hop::ReturnSlot>,
}

/// One reliable land packet. `kind` is `TerrainPatch.LayerType` (`0x4C` or `0x4D`).
/// `data` is the bitpack, without the UDP header or the variable-length prefix.
#[derive(Clone, Debug)]
pub struct LandLayer {
    pub kind: u8,
    pub data: Vec<u8>,
}

#[cfg(test)]
impl Default for ArrivalConfig {
    /// Defaults for the standalone circuit test listener. A region must supply
    /// its actual configuration and accepted login data.
    fn default() -> Self {
        Self {
            position: [128.0, 128.0, 25.0],
            look_at: [0.99, 0.042, 0.0],
            region_handle: 0,
            channel_version: "Ferroan Server".into(),
            land: Vec::new(),
            avatar: None,
            teleport: None,
            source_hop: None,
            return_slot: None,
        }
    }
}

impl ArrivalConfig {
    pub(crate) fn valid(&self) -> bool {
        self.position
            .iter()
            .chain(&self.look_at)
            .all(|number| number.is_finite())
    }

    pub(crate) fn build(&self, expect: &CircuitExpect, timestamp: u32) -> AgentMovementComplete {
        let mut reply = AgentMovementComplete::default();
        reply.agent_data.agent_id = expect.agent_id;
        reply.agent_data.session_id = expect.session_id;
        // MoveAgentIntoRegion falls back to m_startpos when its pos argument
        // is zero. For this login path position IS that accepted start position.
        reply.data.position = self.position.into();
        reply.data.look_at = self.look_at.into();
        reply.data.region_handle = self.region_handle;
        reply.data.timestamp = timestamp;
        reply.sim_data.channel_version = channel_version(&self.channel_version);
        reply
    }
}

fn channel_version(text: &str) -> Vec<u8> {
    // Utils.StringToBytes stops at the first NUL, keeps whole UTF-8 characters,
    // then appends a terminator. The 1024-byte limit includes that terminator.
    let text = text.split('\0').next().unwrap_or_default();
    let mut end = text.len().min(1023);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return Vec::new();
    }
    let mut bytes = text.as_bytes()[..end].to_vec();
    bytes.push(0);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_bytes_follow_csharp_nul_and_utf8_limits() {
        assert_eq!(channel_version(""), b"");
        assert_eq!(channel_version("\0ignored"), b"");
        assert_eq!(channel_version("Ferroan\0ignored"), b"Ferroan\0");
        assert_eq!(
            channel_version(&format!("{}é", "a".repeat(1022))),
            [vec![b'a'; 1022], vec![0]].concat()
        );
        assert_eq!(channel_version(&"😀".repeat(300)).len(), 1021);
    }

    #[test]
    fn arrival_rejects_nonfinite_external_vectors() {
        let mut config = ArrivalConfig::default();
        assert!(config.valid());
        config.position[1] = f32::INFINITY;
        assert!(!config.valid());
        config.position[1] = 0.0;
        config.look_at[2] = f32::NAN;
        assert!(!config.valid());
    }
}
