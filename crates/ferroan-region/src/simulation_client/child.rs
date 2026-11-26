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
// Ported from Source/OpenSim.Region.CoreModules/Framework/EntityTransfer/EntityTransferModule.cs,
// Source/OpenSim.Framework/AvatarAppearance.cs, AvatarWearable.cs, ChildAgentDataUpdate.cs
// and Source/OpenSim.Services.Connectors/Simulation/SimulationServiceConnector.cs.
// KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::simulation::AcceptedAgent;
use std::collections::HashMap;
use tokio::time::Instant;

/// A root presence in its own scene coordinates. Viewer throttles are seven
/// little-endian floats; their calculation belongs to the viewer throttle path.
#[derive(Clone, Debug)]
pub struct AgentPosition {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub region_handle: u64,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub center: [f32; 3],
    pub size: [f32; 3],
    pub far: f32,
    pub viewer_ui_is_god: Option<bool>,
    pub throttles: Option<[u8; 28]>,
}

impl AcceptedAgent {
    /// Build a circuit for a visible neighbour while the viewer remains rooted
    /// here. `caps_path` is the seed the child book stored for this destination.
    /// Only the current height carries into the new appearance.
    pub fn visible_child_circuit(
        &self,
        home: &RegionEnd,
        destination: &RegionEnd,
        position: [f32; 3],
        draw_distance: f32,
        height: f32,
        caps_path: &str,
    ) -> Circuit {
        let offset = |source: u32, dest: u32| (i64::from(source) - i64::from(dest)) as f32;
        Circuit {
            agent_id: self.expect.agent_id,
            base_folder: Uuid::nil(),
            inventory_folder: Uuid::nil(),
            caps_path: caps_path.to_owned(),
            children_seeds: Vec::new(),
            child: true,
            circuit_code: self.expect.circuit_code,
            first_name: self.first_name.clone(),
            last_name: self.last_name.clone(),
            session_id: self.expect.session_id,
            secure_session_id: self.secure_session_id,
            service_session_id: self.service_session_id.clone(),
            start_pos: [
                position[0] + offset(home.loc_x, destination.loc_x),
                position[1] + offset(home.loc_y, destination.loc_y),
                position[2],
            ],
            client_ip: self.client_ip.clone(),
            viewer: self.viewer.clone(),
            channel: self.channel.clone(),
            mac: self.mac.clone(),
            id0: self.id0.clone(),
            far: draw_distance,
            appearance: Some(child_appearance(height)),
            service_urls: self.service_urls.clone(),
        }
    }
}

fn child_appearance(height: f32) -> Appearance {
    let mut wearables = vec![Vec::new(); LEGACY_WEARABLES as usize];
    let defaults = [
        (
            uuid::uuid!("66c41e39-38f9-f75a-024e-585989bfaba9"),
            uuid::uuid!("66c41e39-38f9-f75a-024e-585989bfab73"),
        ), // body
        (
            uuid::uuid!("77c41e39-38f9-f75a-024e-585989bfabc9"),
            uuid::uuid!("77c41e39-38f9-f75a-024e-585989bbabbb"),
        ), // skin
        (
            uuid::uuid!("d342e6c1-b9d2-11dc-95ff-0800200c9a66"),
            uuid::uuid!("d342e6c0-b9d2-11dc-95ff-0800200c9a66"),
        ), // hair
        (
            uuid::uuid!("cdc31054-eed8-4021-994f-4e0c6e861b50"),
            uuid::uuid!("4bb6fa4d-1cd2-498a-a84c-95c1a0e745a7"),
        ), // eyes
        (
            uuid::uuid!("77c41e39-38f9-f75a-0000-585989bf0000"),
            uuid::uuid!("00000000-38f9-1111-024e-222222111110"),
        ), // shirt
        (
            uuid::uuid!("77c41e39-38f9-f75a-0000-5859892f1111"),
            uuid::uuid!("00000000-38f9-1111-024e-222222111120"),
        ), // pants
    ];
    for (slot, (item, asset)) in wearables.iter_mut().zip(defaults) {
        slot.push(WearableItem { item, asset });
    }
    Appearance {
        serial: 0,
        height,
        aphz: 0.,
        has_texture: true,
        wearables,
        te8: Vec::new(),
        textures: vec![Uuid::from_bytes(ferroan_client::DEFAULT_AVATAR_TEXTURE); 29],
        baked: Vec::new(),
        visual_params: ferroan_types::DEFAULT_VISUAL_PARAMS.to_vec(),
        attachments: Vec::new(),
    }
}

impl SimulationClient {
    /// InformClientOfNeighbourAsync passes the destination twice and a null
    /// appearance context. CreateAgent adds its default context after packing.
    pub async fn create_visible_child(
        &self,
        destination: &RegionEnd,
        circuit: &mut Circuit,
    ) -> Result<CreateOutcome, ClientError> {
        let body = pack_create(Some(destination), destination, circuit, 0, None);
        self.send_create(destination, circuit, body).await
    }

    async fn put_position(
        &self,
        destination: &RegionEnd,
        data: &AgentPosition,
    ) -> Result<bool, ClientError> {
        let body = encode_json(&pack_position(destination, data))?;
        let response = self
            .execute(Outbound {
                method: Method::PUT,
                url: format!(
                    "{}/agent/{}/",
                    trim_uri(&destination.server_uri),
                    data.agent_id
                ),
                body: Some(body),
                compress: true,
                keepalive: false,
                timeout: Duration::from_secs(10),
            })
            .await?;
        Ok(http_success(&response))
    }
}

fn pack_position(destination: &RegionEnd, data: &AgentPosition) -> Value {
    let mut entries = vec![
        ("message_type", Osd::Str("AgentPosition".into())),
        ("region_handle", Osd::Str(data.region_handle.to_string())),
        ("circuit_code", Osd::Str("0".into())),
        ("agent_uuid", Osd::Uuid(data.agent_id)),
        ("session_uuid", Osd::Uuid(data.session_id)),
        ("position", Osd::Str(format_vec3(data.position))),
        ("velocity", Osd::Str(format_vec3(data.velocity))),
        ("center", Osd::Str(format_vec3(data.center))),
        ("size", Osd::Str(format_vec3(data.size))),
        ("at_axis", Osd::Str(format_vec3([0.; 3]))),
        ("left_axis", Osd::Str(format_vec3([0.; 3]))),
        ("up_axis", Osd::Str(format_vec3([0.; 3]))),
        ("far", Osd::Real(data.far)),
    ];
    if let Some(god) = data.viewer_ui_is_god {
        entries.push((
            "god_data",
            Osd::Map(vec![("ViewerUiIsGod".into(), Osd::Bool(god))]),
        ));
        entries.push(("god_level", Osd::Int(if god { 200 } else { 0 })));
    }
    if let Some(throttles) = data.throttles {
        entries.push(("throttles", Osd::Bytes(throttles.to_vec())));
    }
    entries.extend(pack_ends(None, destination, None));
    entries.push(("context", pack_context(&TransferContext::negotiated())));
    object(entries)
}

#[derive(Debug, Default)]
pub(super) struct PositionQueue {
    pending: HashMap<(String, Uuid), Option<AgentPosition>>,
    failed_until: HashMap<String, Instant>,
}

// Each entry is either an active destination/agent or a failed simulator.
// Completing a failed PUT releases one active entry before caching the failure.
const MAX_POSITION_ENDPOINTS: usize = 4096;

impl SimulationClient {
    /// One worker per destination/agent URL sends the latest queued position.
    /// A failed position PUT suppresses new updates to that simulator for 120s.
    pub async fn update_child_position(
        &self,
        destination: &RegionEnd,
        data: AgentPosition,
    ) -> Result<bool, ClientError> {
        let key = (destination.server_uri.clone(), data.agent_id);
        {
            let mut state = self.positions.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            state.failed_until.retain(|_, until| *until > now);
            if state.failed_until.contains_key(&key.0) {
                return Ok(false);
            }
            if let Some(pending) = state.pending.get_mut(&key) {
                *pending = Some(data);
                return Ok(true);
            }
            let count = state.pending.len() + state.failed_until.len();
            if count >= MAX_POSITION_ENDPOINTS {
                ferroan_diag::refuse(
                    ferroan_diag::CHILD_POSITION_ENDPOINTS,
                    Kind::Hard,
                    MAX_POSITION_ENDPOINTS as u64,
                    count as u64 + 1,
                    "AgentPosition",
                );
                return Ok(false);
            }
            state.pending.insert(key.clone(), None);
        }
        // Once queued the worker belongs to the connector, even if this
        // caller stops waiting for its result.
        let client = self.clone();
        let destination = destination.clone();
        tokio::spawn(async move {
            let mut current = data;
            loop {
                let result = client.put_position(&destination, &current).await;
                let mut state = client.positions.lock().unwrap_or_else(|e| e.into_inner());
                if matches!(result, Ok(true)) {
                    if let Some(next) = state.pending.get_mut(&key).and_then(Option::take) {
                        current = next;
                        continue;
                    }
                } else {
                    state
                        .failed_until
                        .insert(key.0.clone(), Instant::now() + Duration::from_secs(120));
                }
                state.pending.remove(&key);
                return result;
            }
        })
        .await
        .map_err(|_| ClientError::Unavailable)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_child_wearable_items_and_assets_match_pin8() {
        // AvatarWearable.cs at c401bceecd, DEFAULT_{BODY,SKIN,HAIR,EYES,SHIRT,PANTS}_{ITEM,ASSET}.
        let expected = [
            (
                "body",
                uuid::uuid!("66c41e39-38f9-f75a-024e-585989bfaba9"),
                uuid::uuid!("66c41e39-38f9-f75a-024e-585989bfab73"),
            ),
            (
                "skin",
                uuid::uuid!("77c41e39-38f9-f75a-024e-585989bfabc9"),
                uuid::uuid!("77c41e39-38f9-f75a-024e-585989bbabbb"),
            ),
            (
                "hair",
                uuid::uuid!("d342e6c1-b9d2-11dc-95ff-0800200c9a66"),
                uuid::uuid!("d342e6c0-b9d2-11dc-95ff-0800200c9a66"),
            ),
            (
                "eyes",
                uuid::uuid!("cdc31054-eed8-4021-994f-4e0c6e861b50"),
                uuid::uuid!("4bb6fa4d-1cd2-498a-a84c-95c1a0e745a7"),
            ),
            (
                "shirt",
                uuid::uuid!("77c41e39-38f9-f75a-0000-585989bf0000"),
                uuid::uuid!("00000000-38f9-1111-024e-222222111110"),
            ),
            (
                "pants",
                uuid::uuid!("77c41e39-38f9-f75a-0000-5859892f1111"),
                uuid::uuid!("00000000-38f9-1111-024e-222222111120"),
            ),
        ];
        let wearables = child_appearance(1.8).wearables;
        assert_eq!(wearables.len(), 15);
        for (slot, (name, item, asset)) in expected.into_iter().enumerate() {
            let actual = &wearables[slot];
            assert_eq!(actual.len(), 1, "{name} slot");
            assert_eq!(actual[0].item, item, "{name} item");
            assert_eq!(actual[0].asset, asset, "{name} asset");
        }
        assert!(wearables[expected.len()..].iter().all(Vec::is_empty));
    }

    #[tokio::test]
    async fn position_endpoint_bound_still_accepts_an_update_for_an_active_agent() {
        let client = SimulationClient::new().unwrap();
        let dest = RegionEnd {
            server_uri: "unused".into(),
            region_id: Uuid::from_u128(2),
            name: "Neighbour".into(),
            loc_x: 0,
            loc_y: 0,
        };
        let position = AgentPosition {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            region_handle: 0,
            position: [1., 2., 3.],
            velocity: [0.; 3],
            center: [0.; 3],
            size: [0.; 3],
            far: 0.,
            viewer_ui_is_god: None,
            throttles: None,
        };
        let active = (dest.server_uri.clone(), position.agent_id);
        {
            let mut state = client.positions.lock().unwrap();
            let until = Instant::now() + Duration::from_secs(120);
            for i in 1..MAX_POSITION_ENDPOINTS {
                state.failed_until.insert(format!("down-{i}"), until);
            }
            state.pending.insert(active.clone(), None);
        }
        assert!(
            client
                .update_child_position(&dest, position.clone())
                .await
                .unwrap()
        );
        let next = AgentPosition {
            agent_id: Uuid::from_u128(3),
            ..position
        };
        assert!(!client.update_child_position(&dest, next).await.unwrap());
        let state = client.positions.lock().unwrap();
        assert!(state.pending[&active].is_some());
        assert!(!state.failed_until.contains_key(&dest.server_uri));
    }
}
