// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The packed movement inside ObjectUpdate and ImprovedTerseObjectUpdate
// follows libopenmetaverse
//   OpenMetaverse/ObjectManager.cs
//   libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// The default box uses the same path quanta as Primitive.PackEndCut and
// Primitive.PackPathScale in that tree. This file is new Rust.

//! Packets the scenarios send, and the positions the region sends back.

use ferroan_protocol::msgs::{
    AgentMovementComplete, AgentUpdate, ChatFromSimulator, ChatFromViewer, DeRezObject,
    ImprovedTerseObjectUpdate, KillObject, MultipleObjectUpdate, ObjectAdd, ObjectName,
    ObjectProperties, ObjectPropertiesFamily, ObjectSelect, ObjectUpdate,
    RequestObjectPropertiesFamily, TeleportFailed, TeleportFinish, TeleportLocal,
    TeleportLocationRequest,
};
use ferroan_protocol::{IncomingPacket, Msg, field_str, str_field};
use glam::{Quat, Vec3};
use uuid::Uuid;

use crate::session::Agent;

/// Walk forward, from libopenmetaverse `AgentManager.ControlFlags`.
pub const CONTROL_AT_POS: u32 = 1;
/// Say, as opposed to whisper (0) or shout (2).
pub const CHAT_SAY: u8 = 1;
/// A boxed prim.
pub const PCODE_PRIM: u8 = 9;
/// An avatar.
pub const PCODE_AVATAR: u8 = 47;
/// Straight path. libopenmetaverse `PathCurve.Line`.
const PATH_LINE: u8 = 16;
/// Square profile.
const PROFILE_SQUARE: u8 = 1;
/// Wood. The value viewers send for an ordinary box.
const MATERIAL_WOOD: u8 = 3;
/// `PrimFlags.CreateSelected`.
const CREATE_SELECTED: u32 = 2;
/// `MultipleObjectUpdate` position.
pub const UPDATE_POSITION: u8 = 1;
/// `MultipleObjectUpdate` scale.
pub const UPDATE_SCALE: u8 = 4;
/// `DeRezAction.Delete`. The region does not handle `ObjectDelete`.
const DEREZ_DELETE: u8 = 6;
const CHAT_BYTES: usize = 1023;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeenObject {
    pub local_id: u32,
    pub full_id: Uuid,
    pub position: [f32; 3],
    pub scale: [f32; 3],
    pub pcode: u8,
}

pub fn agent_update(agent: Agent, position: Vec3, facing: Quat, controls: u32) -> AgentUpdate {
    let mut message = AgentUpdate::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message.agent_data.body_rotation = facing;
    message.agent_data.head_rotation = facing;
    message.agent_data.camera_center = position;
    let forward = horizontal_forward(facing);
    message.agent_data.camera_at_axis = forward;
    message.agent_data.camera_left_axis = Vec3::Z.cross(forward);
    message.agent_data.camera_up_axis = Vec3::Z;
    message.agent_data.far = 128.0;
    message.agent_data.control_flags = controls;
    message
}

pub fn say(agent: Agent, text: &str) -> ChatFromViewer {
    let mut message = ChatFromViewer::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message.chat_data.message = str_field(&truncate_chat(text));
    message.chat_data.type_ = CHAT_SAY;
    message.chat_data.channel = 0;
    message
}

pub fn box_at(agent: Agent, position: Vec3, scale: Vec3) -> ObjectAdd {
    let mut message = ObjectAdd::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message.object_data.p_code = PCODE_PRIM;
    message.object_data.material = MATERIAL_WOOD;
    message.object_data.add_flags = CREATE_SELECTED;
    message.object_data.path_curve = PATH_LINE;
    message.object_data.profile_curve = PROFILE_SQUARE;
    message.object_data.path_end = pack_end(1.0);
    message.object_data.path_scale_x = pack_path_scale(1.0);
    message.object_data.path_scale_y = pack_path_scale(1.0);
    message.object_data.profile_end = pack_end(1.0);
    message.object_data.bypass_raycast = 1;
    message.object_data.ray_start = position;
    message.object_data.ray_end = position;
    message.object_data.scale = scale;
    message.object_data.rotation = Quat::IDENTITY;
    message
}

pub fn request_properties(agent: Agent, object_id: Uuid) -> RequestObjectPropertiesFamily {
    let mut message = RequestObjectPropertiesFamily::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message.object_data.object_id = object_id;
    message
}

pub fn rename(agent: Agent, local_id: u32, name: &str) -> ObjectName {
    let mut message = ObjectName::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message
        .object_data
        .push(ferroan_protocol::msgs::object_name::ObjectData {
            local_id,
            name: str_field(name),
        });
    message
}

pub fn transform(agent: Agent, local_id: u32, kind: u8, value: Vec3) -> MultipleObjectUpdate {
    let mut message = MultipleObjectUpdate::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message
        .object_data
        .push(ferroan_protocol::msgs::multiple_object_update::ObjectData {
            object_local_id: local_id,
            type_: kind,
            data: vec3_bytes(value),
        });
    message
}

pub fn select_objects(agent: Agent, local_ids: &[u32]) -> ObjectSelect {
    let mut message = ObjectSelect::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message.object_data = local_ids
        .iter()
        .map(
            |local_id| ferroan_protocol::msgs::object_select::ObjectData {
                object_local_id: *local_id,
            },
        )
        .collect();
    message
}

pub fn delete_object(agent: Agent, local_id: u32) -> DeRezObject {
    let mut message = DeRezObject::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message.agent_block.destination = DEREZ_DELETE;
    message.agent_block.packet_count = 1;
    message
        .object_data
        .push(ferroan_protocol::msgs::de_rez_object::ObjectData {
            object_local_id: local_id,
        });
    message
}

pub fn teleport_to(agent: Agent, region_handle: u64, position: Vec3) -> TeleportLocationRequest {
    let mut message = TeleportLocationRequest::default();
    message.agent_data.agent_id = agent.agent_id;
    message.agent_data.session_id = agent.session_id;
    message.info.region_handle = region_handle;
    message.info.position = position;
    message.info.look_at = Vec3::X;
    message
}

/// A rotation whose horizontal forward axis is `look_at`, or +X when that
/// vector has no horizontal length. Identity faces +X.
pub fn facing_look(look_at: Vec3) -> Quat {
    let flat = Vec3::new(look_at.x, look_at.y, 0.0);
    if flat.length_squared() < 1.0e-6 {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_arc(Vec3::X, flat.normalize())
    }
}

pub fn horizontal_forward(facing: Quat) -> Vec3 {
    let forward = facing * Vec3::X;
    let flat = Vec3::new(forward.x, forward.y, 0.0);
    if flat.length_squared() < 1.0e-6 {
        Vec3::X
    } else {
        flat.normalize()
    }
}

pub fn movement_position(packet: &IncomingPacket) -> Option<Vec3> {
    if packet.id != AgentMovementComplete::ID {
        return None;
    }
    let decoded = packet.decode::<AgentMovementComplete>().ok()?;
    Some(decoded.data.position)
}

pub fn movement_look(packet: &IncomingPacket) -> Option<(Vec3, u64)> {
    if packet.id != AgentMovementComplete::ID {
        return None;
    }
    let decoded = packet.decode::<AgentMovementComplete>().ok()?;
    Some((decoded.data.look_at, decoded.data.region_handle))
}

pub fn chat_heard(packet: &IncomingPacket) -> Option<(String, Uuid, u8, String)> {
    if packet.id != ChatFromSimulator::ID {
        return None;
    }
    let decoded = packet.decode::<ChatFromSimulator>().ok()?;
    Some((
        field_str(&decoded.chat_data.from_name),
        decoded.chat_data.source_id,
        decoded.chat_data.chat_type,
        field_str(&decoded.chat_data.message),
    ))
}

pub fn objects_in(packet: &IncomingPacket) -> Vec<SeenObject> {
    if packet.id != ObjectUpdate::ID {
        return Vec::new();
    }
    let Ok(update) = packet.decode::<ObjectUpdate>() else {
        return Vec::new();
    };
    update
        .object_data
        .iter()
        .filter_map(|block| {
            let position = position_in_object_data(&block.object_data)?;
            Some(SeenObject {
                local_id: block.id,
                full_id: block.full_id,
                position: [position.x, position.y, position.z],
                scale: [block.scale.x, block.scale.y, block.scale.z],
                pcode: block.p_code,
            })
        })
        .collect()
}

pub fn terse_in(packet: &IncomingPacket) -> Vec<(u32, bool, Vec3)> {
    if packet.id != ImprovedTerseObjectUpdate::ID {
        return Vec::new();
    }
    let Ok(update) = packet.decode::<ImprovedTerseObjectUpdate>() else {
        return Vec::new();
    };
    update
        .object_data
        .iter()
        .filter_map(|block| terse_position(&block.data))
        .collect()
}

pub fn property_name(packet: &IncomingPacket, object_id: Uuid) -> Option<String> {
    if packet.id == ObjectProperties::ID
        && let Ok(decoded) = packet.decode::<ObjectProperties>()
    {
        for block in decoded.object_data {
            if block.object_id == object_id {
                return Some(field_str(&block.name));
            }
        }
    }
    // A properties-family request is answered with ObjectPropertiesFamily,
    // not ObjectProperties.
    if packet.id == ObjectPropertiesFamily::ID
        && let Ok(decoded) = packet.decode::<ObjectPropertiesFamily>()
        && decoded.object_data.object_id == object_id
    {
        return Some(field_str(&decoded.object_data.name));
    }
    None
}

pub fn killed(packet: &IncomingPacket) -> Vec<u32> {
    if packet.id != KillObject::ID {
        return Vec::new();
    }
    let Ok(kill) = packet.decode::<KillObject>() else {
        return Vec::new();
    };
    kill.object_data.iter().map(|block| block.id).collect()
}

pub fn teleport_failure(packet: &IncomingPacket) -> Option<String> {
    if packet.id != TeleportFailed::ID {
        return None;
    }
    packet
        .decode::<TeleportFailed>()
        .ok()
        .map(|reply| field_str(&reply.info.reason))
}

pub fn teleport_landed(packet: &IncomingPacket) -> Option<Vec3> {
    if packet.id != TeleportLocal::ID {
        return None;
    }
    packet
        .decode::<TeleportLocal>()
        .ok()
        .map(|reply| reply.info.position)
}

/// Where `TeleportFinish` tells the viewer to open the next circuit.
///
/// `seed_capability` is the variable block, decoded as a UTF-8 string.
pub struct TeleportArrival {
    pub sim_ip: [u8; 4],
    pub sim_port: u16,
    pub region_handle: u64,
    pub seed_capability: String,
}

pub fn teleport_arrival(packet: &IncomingPacket) -> Option<TeleportArrival> {
    if packet.id != TeleportFinish::ID {
        return None;
    }
    let reply = packet.decode::<TeleportFinish>().ok()?;
    Some(TeleportArrival {
        sim_ip: reply.info.sim_ip,
        sim_port: reply.info.sim_port,
        region_handle: reply.info.region_handle,
        seed_capability: field_str(&reply.info.seed_capability),
    })
}

pub fn position_in_object_data(data: &[u8]) -> Option<Vec3> {
    let offset = match data.len() {
        76 | 48 => 16,
        60 | 32 | 16 => 0,
        _ => return None,
    };
    match data.len() {
        76 | 60 => read_vec3(data, offset),
        32 | 48 => Some(Vec3::new(
            unpack_u16(data, offset, -128.0, 384.0)?,
            unpack_u16(data, offset + 2, -128.0, 384.0)?,
            unpack_u16(data, offset + 4, -256.0, 768.0)?,
        )),
        16 => Some(Vec3::new(
            unpack_u8(data, offset, -256.0, 256.0)?,
            unpack_u8(data, offset + 1, -256.0, 256.0)?,
            unpack_u8(data, offset + 2, -256.0, 256.0)?,
        )),
        _ => None,
    }
}

pub fn terse_position(data: &[u8]) -> Option<(u32, bool, Vec3)> {
    if data.len() < 6 {
        return None;
    }
    let local_id = u32::from_le_bytes(data[0..4].try_into().ok()?);
    let avatar = data[5] != 0;
    let mut at = 6;
    if avatar {
        at += 16;
    }
    let position = read_vec3(data, at)?;
    Some((local_id, avatar, position))
}

pub fn is_j2c(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] == 0x4f
}

fn pack_end(end: f32) -> u16 {
    50_000u16.saturating_sub((end * 50_000.0).round() as u16)
}

fn pack_path_scale(scale: f32) -> u8 {
    200u8.saturating_sub((scale * 100.0).round() as u8)
}

fn truncate_chat(text: &str) -> String {
    if text.len() <= CHAT_BYTES {
        return text.to_owned();
    }
    let mut end = CHAT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn vec3_bytes(value: Vec3) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(12);
    bytes.extend(value.x.to_le_bytes());
    bytes.extend(value.y.to_le_bytes());
    bytes.extend(value.z.to_le_bytes());
    bytes
}

fn read_vec3(data: &[u8], offset: usize) -> Option<Vec3> {
    Some(Vec3::new(
        read_f32(data, offset)?,
        read_f32(data, offset + 4)?,
        read_f32(data, offset + 8)?,
    ))
}

fn read_f32(data: &[u8], offset: usize) -> Option<f32> {
    let bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    Some(f32::from_le_bytes(bytes))
}

fn unpack_u16(data: &[u8], offset: usize, min: f32, max: f32) -> Option<f32> {
    let bytes: [u8; 2] = data.get(offset..offset + 2)?.try_into().ok()?;
    let value = u16::from_le_bytes(bytes) as f32 / 65535.0;
    Some(min + value * (max - min))
}

fn unpack_u8(data: &[u8], offset: usize, min: f32, max: f32) -> Option<f32> {
    let value = *data.get(offset)? as f32 / 255.0;
    Some(min + value * (max - min))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_object_update_stores_position_after_the_collision_plane() {
        let mut data = vec![0u8; 60];
        data.splice(0..12, vec3_bytes(Vec3::new(10.0, 20.0, 30.0)));
        let position = position_in_object_data(&data).unwrap();
        assert_eq!(position, Vec3::new(10.0, 20.0, 30.0));

        let mut avatar = vec![0u8; 76];
        avatar.splice(16..28, vec3_bytes(Vec3::new(1.5, 2.5, 3.5)));
        let position = position_in_object_data(&avatar).unwrap();
        assert_eq!(position, Vec3::new(1.5, 2.5, 3.5));
    }

    #[test]
    fn a_terse_update_skips_the_avatar_collision_plane() {
        let mut data = vec![0u8; 6 + 16 + 12];
        data[0..4].copy_from_slice(&7u32.to_le_bytes());
        data[5] = 1;
        let at = 6 + 16;
        data[at..at + 12].copy_from_slice(&vec3_bytes(Vec3::new(4.0, 5.0, 6.0)));
        let (local_id, avatar, position) = terse_position(&data).unwrap();
        assert_eq!(local_id, 7);
        assert!(avatar);
        assert_eq!(position, Vec3::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn the_default_box_uses_the_packed_end_and_scale() {
        let agent = Agent {
            agent_id: Uuid::nil(),
            session_id: Uuid::nil(),
            circuit_code: 1,
        };
        let prim = box_at(agent, Vec3::new(128.0, 128.0, 22.0), Vec3::splat(0.5));
        assert_eq!(prim.object_data.path_curve, PATH_LINE);
        assert_eq!(prim.object_data.path_end, 0);
        assert_eq!(prim.object_data.path_scale_x, 100);
        assert_eq!(prim.object_data.profile_curve, PROFILE_SQUARE);
        assert_eq!(prim.object_data.bypass_raycast, 1);
        assert_eq!(prim.object_data.p_code, PCODE_PRIM);
    }

    #[test]
    fn say_is_channel_zero_and_jpeg2000_starts_with_the_codestream_marker() {
        let agent = Agent {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            circuit_code: 3,
        };
        let chat = say(agent, "hello");
        assert_eq!(chat.chat_data.channel, 0);
        assert_eq!(chat.chat_data.type_, CHAT_SAY);
        assert_eq!(field_str(&chat.chat_data.message), "hello");
        assert!(is_j2c(&[0xff, 0x4f, 0xff, 0x51]));
        assert!(!is_j2c(&[0x00, 0x00, 0x00, 0x0c]));
    }

    #[test]
    fn forward_control_faces_the_look_vector() {
        let facing = facing_look(Vec3::Y);
        let forward = horizontal_forward(facing);
        assert!(forward.y > 0.9);
        assert!(forward.x.abs() < 0.1);
        assert_eq!(horizontal_forward(Quat::IDENTITY), Vec3::X);
    }
}
