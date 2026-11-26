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
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs
//     (SendTeleportStart, SendTeleportFailed, SendMapBlock)
//   Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/EventQueueGetHandlers.cs
//     (TeleportFinishEvent)
//   Source/OpenSim.Framework/CapsUtil.cs (GetCapsSeedPath)
//   Source/OpenSim.Framework/Util.cs (RegionHandleToRegionLoc)
//   Source/OpenSim.Framework/MapBlockData.cs
//   Source/OpenSim.Region.CoreModules/Framework/EntityTransfer/EntityTransferModule.cs
//     (Teleport, the unknown-region map block)
//   OpenSim-NGC/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
//   OpenMetaverse SimAccess.Down (254) and the PG access byte (13)
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Viewer packets for one source-region hop.
//!
//! The circuit spawns the hop so a second `TeleportLocationRequest` can be
//! read while the neighbour calls are in flight. This module builds those
//! packets and the event. It does not call the neighbour and it does not
//! change the agent's presence.

use std::future::Future;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::Arc;

use ferroan_llsd::Llsd;
use ferroan_protocol::msgs::{MapBlockReply, map_block_reply};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::agent_animations::AgentAnimations;
use crate::teleport::{IS_FLYING, VIA_LOCATION};

/// `TeleportFinishEvent` stores this access byte. The caller passes 13.
pub const SIM_ACCESS: i32 = 13;
/// `SimAccess.Down`: the map tile is offline.
pub const MAP_DOWN: u8 = 254;
/// `EventQueueGetHandlers.TeleportFinishEvent` stores this location id.
/// The caller passes 0. The viewer sees 4.
pub const FINISH_LOCATION: i32 = 4;
pub const NOT_FOUND: &str = "The region you tried to teleport to was not found";
pub const RESOLVE_FAILED: &str = "Could not resolve destination Address";
pub const INTERNAL: &str = "Internal error";
/// The reason `Fail` passes into the sentence the viewer displays.
pub const CONNECT_INNER: &str =
    "Connection between viewer and destination region could not be established.";

/// `Fail` formats the viewer packet as this sentence. The inner reason is
/// [`CONNECT_INNER`].
pub fn connect_reason(region_name: &str) -> String {
    format!("Problems connecting to destination {region_name}, reason: {CONNECT_INNER}")
}

/// `GetCapsSeedPath`: `{server}/CAPS/{capsObjectPath}0000/`.
pub fn caps_seed(server_uri: &str, caps_path: &str) -> String {
    format!("{}/CAPS/{caps_path}0000/", server_uri.trim_end_matches('/'))
}

/// `RegionHandleToRegionLoc`, then the `ushort` stored in the map block.
pub fn region_loc(handle: u64) -> (u16, u16) {
    let x = ((handle >> 40) & 0x00ff_ffff) as u16;
    let y = ((handle >> 8) & 0x00ff_ffff) as u16;
    (x, y)
}

/// One offline tile. Unset `MapBlockData` fields stay zero, and the size
/// block is present with a count of zero because every size is within 256.
pub fn offline_map_block(agent: Uuid, handle: u64) -> MapBlockReply {
    let (x, y) = region_loc(handle);
    let mut message = MapBlockReply::default();
    message.agent_data.agent_id = agent;
    message.agent_data.flags = 0;
    message.data.push(map_block_reply::Data {
        x,
        y,
        name: Vec::new(),
        access: MAP_DOWN,
        region_flags: 0,
        water_height: 0,
        agents: 0,
        map_image_id: Uuid::nil(),
    });
    message.omitted_trailing_blocks = 0;
    message
}

/// Flags stored in the event. Other bits, including godlike, are dropped.
/// Flying keeps `ViaLocation` as well.
pub fn finish_flags(flags: u32) -> u32 {
    if flags & IS_FLYING != 0 {
        VIA_LOCATION | IS_FLYING
    } else {
        VIA_LOCATION
    }
}

/// One unsigned value the viewer reads from a 4-byte big-endian binary.
fn u32_binary(value: u32) -> Llsd {
    Llsd::Binary(value.to_be_bytes().to_vec())
}

/// Camera and body from the last viewer update, and the frame's animator
/// for `CopyTo`. Rotations start as identity, the animator standing.
#[derive(Clone, Debug, PartialEq)]
pub struct HopPose {
    pub always_run: bool,
    pub center: [f32; 3],
    pub head_rotation: [f32; 4],
    pub body_rotation: [f32; 4],
    pub animations: AgentAnimations,
}

impl Default for HopPose {
    fn default() -> Self {
        Self {
            always_run: false,
            center: [0.0; 3],
            head_rotation: [0.0, 0.0, 0.0, 1.0],
            body_rotation: [0.0, 0.0, 0.0, 1.0],
            animations: AgentAnimations::standing(),
        }
    }
}

/// Fields the viewer reads from one `TeleportFinish` event.
#[derive(Clone, Debug)]
pub struct FinishEvent {
    pub agent: Uuid,
    pub handle: u64,
    pub endpoint: Ipv4Addr,
    pub port: u16,
    pub flags: u32,
    pub seed: String,
    pub size_x: u32,
    pub size_y: u32,
}

/// One `TeleportFinish` event. `SimIP` is four address bytes.
pub fn teleport_finish_event(event: &FinishEvent) -> Llsd {
    let mut info = Llsd::new_map();
    info.insert("AgentID", Llsd::Uuid(event.agent));
    info.insert(
        "LocationID",
        u32_binary(u32::try_from(FINISH_LOCATION).unwrap_or(0)),
    );
    info.insert("SimIP", Llsd::Binary(event.endpoint.octets().to_vec()));
    info.insert("SimPort", Llsd::Integer(i32::from(event.port)));
    info.insert(
        "RegionHandle",
        Llsd::Binary(event.handle.to_be_bytes().to_vec()),
    );
    info.insert("SeedCapability", Llsd::String(event.seed.clone()));
    info.insert("SimAccess", Llsd::Integer(SIM_ACCESS));
    info.insert("TeleportFlags", u32_binary(finish_flags(event.flags)));
    info.insert("RegionSizeX", u32_binary(event.size_x));
    info.insert("RegionSizeY", u32_binary(event.size_y));
    let mut body = Llsd::new_map();
    body.insert("Info", Llsd::Array(vec![info]));
    let mut event = Llsd::new_map();
    event.insert("message", Llsd::String("TeleportFinish".to_owned()));
    event.insert("body", body);
    event
}

/// What the viewer asked for. Flags already include `ViaLocation`.
#[derive(Clone, Debug)]
pub struct HopRequest {
    pub agent_id: Uuid,
    pub session_id: Uuid,
    pub region_handle: u64,
    pub position: [f32; 3],
    pub look_at: [f32; 3],
    pub flags: u32,
    pub client_ip: String,
    /// Last camera center and body rotations. Axes are computed later.
    pub pose: HopPose,
}

pub(crate) enum HopCommand {
    Start {
        flags: u32,
        done: oneshot::Sender<bool>,
    },
    Failed {
        reason: String,
        done: oneshot::Sender<bool>,
    },
    Map {
        message: MapBlockReply,
        done: oneshot::Sender<bool>,
    },
    /// Far teleport: leave the circuit without `KickUser` or `DisableSimulator`.
    Leave {
        done: oneshot::Sender<bool>,
    },
    Settle,
}

/// Sends viewer packets through the circuit that owns the reliable window.
#[derive(Clone)]
pub struct HopOut {
    tx: mpsc::Sender<HopCommand>,
}

impl HopOut {
    pub(crate) fn new(tx: mpsc::Sender<HopCommand>) -> Self {
        Self { tx }
    }

    pub async fn teleport_start(&self, flags: u32) -> bool {
        self.round_trip(|done| HopCommand::Start { flags, done })
            .await
    }

    pub async fn teleport_failed(&self, reason: &str) -> bool {
        let reason = reason.to_owned();
        self.round_trip(|done| HopCommand::Failed { reason, done })
            .await
    }

    pub async fn map_block(&self, message: MapBlockReply) -> bool {
        self.round_trip(|done| HopCommand::Map { message, done })
            .await
    }

    /// Far teleport. The circuit stops without a kick or `DisableSimulator`.
    pub async fn leave(&self) -> bool {
        self.round_trip(|done| HopCommand::Leave { done }).await
    }

    /// Queue the in-transit clear. The circuit applies it on its own task.
    pub(crate) fn try_settle(&self) {
        let _ = self.tx.try_send(HopCommand::Settle);
    }

    async fn round_trip(&self, command: impl FnOnce(oneshot::Sender<bool>) -> HopCommand) -> bool {
        let (done, waiting) = oneshot::channel();
        if self.tx.send(command(done)).await.is_err() {
            return false;
        }
        waiting.await.unwrap_or(false)
    }
}

type HopFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type HopCall = dyn Fn(HopRequest, HopOut) -> HopFuture + Send + Sync;

/// Region code installed on one circuit. The circuit clears in-transit after
/// this future returns, including when it is dropped.
#[derive(Clone)]
pub struct SourceHop {
    call: Arc<HopCall>,
}

impl std::fmt::Debug for SourceHop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SourceHop")
    }
}

impl SourceHop {
    pub fn new(call: impl Fn(HopRequest, HopOut) -> HopFuture + Send + Sync + 'static) -> Self {
        Self {
            call: Arc::new(call),
        }
    }

    pub(crate) fn start(&self, request: HopRequest, out: HopOut) -> HopFuture {
        (self.call)(request, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_loc_divides_the_handle_by_256() {
        let handle = (256_000_u64 << 32) | 256_000;
        assert_eq!(region_loc(handle), (1000, 1000));
    }

    #[test]
    fn finish_event_matches_the_viewer_fields() {
        let agent = Uuid::from_u128(7);
        let handle = (256_000_u64 << 32) | 256_000;
        let event = teleport_finish_event(&FinishEvent {
            agent,
            handle,
            endpoint: Ipv4Addr::LOCALHOST,
            port: 9000,
            flags: VIA_LOCATION | (1 << 9),
            seed: "http://127.0.0.1:9/CAPS/path0000/".to_owned(),
            size_x: 256,
            size_y: 256,
        });
        let info = event.get("body").get("Info").at(0);
        assert_eq!(event.get("message").as_str(), "TeleportFinish");
        assert_u32_binary(info.get("LocationID"), 4);
        assert!(matches!(info.get("SimAccess"), Llsd::Integer(13)));
        assert_u32_binary(info.get("TeleportFlags"), VIA_LOCATION);
        assert_u32_binary(info.get("RegionSizeX"), 256);
        assert_u32_binary(info.get("RegionSizeY"), 256);
        let xml = ferroan_llsd::to_xml_string(&event);
        assert!(xml.contains("<key>LocationID</key><binary>"));
        assert!(xml.contains("<key>TeleportFlags</key><binary>"));
        assert!(xml.contains("<key>RegionSizeX</key><binary>"));
        assert!(xml.contains("<key>RegionSizeY</key><binary>"));
        assert!(xml.contains("<key>SimPort</key><integer>"));
        assert!(xml.contains("<key>SimAccess</key><integer>"));
        assert!(!xml.contains("<key>LocationID</key><integer>"));
        assert_eq!(info.get("RegionHandle").as_u64(), handle);
        assert_eq!(info.get("SimIP").as_binary(), [127, 0, 0, 1]);
        assert_eq!(info.get("SimPort").as_i32(), 9000);
        assert_eq!(info.get("AgentID").as_uuid(), agent);
        assert!(info.get("SeedCapability").as_str().contains("/CAPS/"));
    }

    #[test]
    fn teleport_finish_event_matches_the_captured_event() {
        let bytes = include_bytes!("../../../fixtures/neighbour-teleport/proxy-home.cap");
        let records = ferroan_capture::read_complete_prefix(bytes).unwrap();
        let mut captured = None;
        for record in records {
            if record.kind != ferroan_capture::Kind::HttpResponse {
                continue;
            }
            let Some(body) = http_body(&record.bytes) else {
                continue;
            };
            if !body.windows(14).any(|part| part == b"TeleportFinish") {
                continue;
            }
            let value = ferroan_llsd::from_xml(body).unwrap();
            for event in value.get("events").as_array() {
                if event.get("message").as_str() == "TeleportFinish" {
                    captured = Some(event.clone());
                }
            }
        }
        let captured = captured.expect("proxy-home.cap TeleportFinish");
        let info = captured.get("body").get("Info").at(0);
        let octets: [u8; 4] = info.get("SimIP").as_binary().try_into().unwrap();
        let built = teleport_finish_event(&FinishEvent {
            agent: info.get("AgentID").as_uuid(),
            handle: info.get("RegionHandle").as_u64(),
            endpoint: Ipv4Addr::from(octets),
            port: u16::try_from(info.get("SimPort").as_i32()).unwrap(),
            flags: info.get("TeleportFlags").as_u32(),
            seed: info.get("SeedCapability").as_str().to_owned(),
            size_x: info.get("RegionSizeX").as_u32(),
            size_y: info.get("RegionSizeY").as_u32(),
        });
        assert_eq!(built, captured);
        let built_info = built.get("body").get("Info").at(0);
        let keys = |value: &Llsd| value.as_map().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&built), keys(&captured));
        assert_eq!(keys(built_info), keys(info));
    }

    fn http_body(bytes: &[u8]) -> Option<&[u8]> {
        let split = bytes.windows(4).position(|part| part == b"\r\n\r\n")?;
        let head = &bytes[..split];
        let body = &bytes[split + 4..];
        let length = head.split(|&byte| byte == b'\n').find_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let rest = line.strip_prefix(b"Content-Length:")?;
            std::str::from_utf8(rest).ok()?.trim().parse::<usize>().ok()
        });
        match length {
            Some(length) if length <= body.len() => Some(&body[..length]),
            _ => Some(body),
        }
    }

    #[test]
    fn an_offline_map_block_has_one_down_tile_and_a_zero_size_count() {
        let agent = Uuid::from_u128(3);
        let handle = (256_u64 << 32) | 512;
        let message = offline_map_block(agent, handle);
        assert_eq!(message.agent_data.agent_id, agent);
        assert_eq!(message.agent_data.flags, 0);
        assert_eq!(message.data.len(), 1);
        assert_eq!(message.data[0].access, MAP_DOWN);
        assert_eq!(message.data[0].x, region_loc(handle).0);
        assert_eq!(message.data[0].y, region_loc(handle).1);
        assert!(message.data[0].name.is_empty());
        assert!(message.size.is_empty());
        assert_eq!(message.omitted_trailing_blocks, 0);
        assert_eq!(
            caps_seed("http://127.0.0.1:9/", "abc"),
            "http://127.0.0.1:9/CAPS/abc0000/"
        );
    }

    fn assert_u32_binary(value: &Llsd, expected: u32) {
        match value {
            Llsd::Binary(bytes) if bytes.len() == 4 => {
                assert_eq!(
                    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                    expected
                );
            }
            _ => panic!("expected a 4-byte binary, value was not that type"),
        }
    }
}
