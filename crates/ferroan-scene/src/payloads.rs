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
// Copyright (c) 2015, InWorldz Halcyon Developers
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
//   * Redistributions of source code must retain the above copyright notice, this
//     list of conditions and the following disclaimer.
//
//   * Redistributions in binary form must reproduce the above copyright notice,
//     this list of conditions and the following disclaimer in the documentation
//     and/or other materials provided with the distribution.
//
//   * Neither the name of halcyon nor the names of its
//     contributors may be used to endorse or promote products derived from
//     this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
// FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
// DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
// CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
// OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
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
// Ported to Rust for Ferroan from DAMap.cs, KeyframeMotion.cs, SOPVehicle.cs,
// VehicleConstants.cs, PhysicsInertia.cs, SceneObjectPart.DeSerializeAnimations,
// PrimitiveBaseShape.MediaList and RenderMaterialsOvrFromRawBin, and
// LinksetData.cs, at OpenSim-Tranquillity `dev-build`
// @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Media entry fields follow libopenmetaverse ObjectMedia.cs
// @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.
// LinksetData.cs has no licence header at that pin.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Structured prim payloads from a region-store load.
//!
//! A dynattrs value that is not a map of maps fails the row. A short, oversized
//! or non-KFM1 keyframe blob is absent. A vehicle or inertia record that is
//! not the expected element fails the row; a record whose fields do not read
//! is absent. A vehicle record that omits one of the six camera and motor-limit
//! numbers receives that vehicle type's number. Its other numbers stay zero
//! unless the record contains them. An animation sequence number is a clock and
//! is not kept. Linkset text that is blank or neither record form is absent.
//! Media whose type is not `sl`, or whose list is empty, is absent. A materials
//! override with no entries is present and empty; a null cell, a count above
//! 128, a zero-length entry, or an old asset prefix leaves it absent.

use ferroan_llsd::xml::Element;
use ferroan_llsd::{Llsd, Map, from_xml, from_xml_element};
use serde_json::Value;
use uuid::Uuid;

const KFM_MAX_FRAMES: i32 = 1 << 20;
const KFM_MAX_BYTES: usize = 16 * 1024 * 1024;
const LINKSET_MAX: i32 = 131_072;
const SEEN_ALL: u8 = 63;

/// Namespaces, then stores, then each store's own entries.
#[derive(Clone, Debug, PartialEq)]
pub struct DynAttrs {
    pub namespaces: Map,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyframeMotion {
    pub serialized_position: [f32; 3],
    pub base_position: [f32; 3],
    pub base_rotation: [f32; 4],
    pub current_frame: Keyframe,
    pub frames: Vec<Keyframe>,
    pub keyframes: Option<Vec<Keyframe>>,
    pub mode: i32,
    pub data: i32,
    pub running: bool,
    pub iterations: i32,
    pub skip_loops: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub position: Option<[f32; 3]>,
    pub rotation: Option<[f32; 4]>,
    pub start_rotation: [f32; 4],
    pub time_ms: i32,
    pub time_total: i32,
    pub angular_velocity: [f32; 3],
    pub start_position: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Vehicle {
    pub kind: i32,
    pub flags: i32,
    pub linear_motor_direction: [f32; 3],
    pub linear_friction_timescale: [f32; 3],
    pub linear_motor_decay_timescale: f32,
    pub linear_motor_timescale: f32,
    pub linear_motor_offset: [f32; 3],
    pub angular_motor_direction: [f32; 3],
    pub angular_motor_timescale: f32,
    pub angular_motor_decay_timescale: f32,
    pub angular_friction_timescale: [f32; 3],
    pub angular_deflection_efficiency: f32,
    pub angular_deflection_timescale: f32,
    pub linear_deflection_efficiency: f32,
    pub linear_deflection_timescale: f32,
    pub banking_efficiency: f32,
    pub banking_mix: f32,
    pub banking_timescale: f32,
    pub hover_height: f32,
    pub hover_efficiency: f32,
    pub hover_timescale: f32,
    pub buoyancy: f32,
    pub vertical_attraction_efficiency: f32,
    pub vertical_attraction_timescale: f32,
    pub reference_frame: [f32; 4],
    pub linear_motor_timescale_axes: Option<[f32; 3]>,
    pub linear_motor_decay_timescale_axes: Option<[f32; 3]>,
    pub angular_motor_timescale_axes: Option<[f32; 3]>,
    pub angular_motor_decay_timescale_axes: Option<[f32; 3]>,
    pub mouselook_azimuth: f32,
    pub mouselook_altitude: f32,
    pub banking_azimuth: f32,
    pub disable_motors_above: f32,
    pub disable_motors_after: f32,
    pub inverted_banking_modifier: f32,
    pub linear_wind_efficiency: Option<[f32; 3]>,
    pub angular_wind_efficiency: Option<[f32; 3]>,
    pub linear_motor_run_age: Option<f32>,
    pub linear_motor_run_direction: Option<[f32; 3]>,
    pub linear_motor_run_target: Option<[f32; 3]>,
    pub angular_motor_run_age: Option<f32>,
    pub angular_motor_run_direction: Option<[f32; 3]>,
    pub angular_motor_run_target: Option<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsInertia {
    pub mass: f32,
    pub center: [f32; 3],
    pub inertia: [f32; 3],
    pub rotation: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectAnimation {
    pub id: Uuid,
    pub name: String,
}

/// `entries` is `None` when the record's list itself is null.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinksetData {
    pub entries: Option<Vec<LinksetEntry>>,
    pub bytes_used: i32,
    pub bytes_free: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinksetEntry {
    pub key: String,
    pub value: Option<String>,
    pub password: Option<String>,
}

/// One face. Strings are `None` on a default face and `Some` when the map supplied them.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaFace {
    pub alt_image_enable: bool,
    pub auto_loop: bool,
    pub auto_play: bool,
    pub auto_scale: bool,
    pub auto_zoom: bool,
    pub controls: i32,
    pub current_url: Option<String>,
    pub first_click_interact: bool,
    pub height_pixels: i32,
    pub home_url: Option<String>,
    pub perms_control: i32,
    pub perms_interact: i32,
    pub whitelist: Option<Vec<String>>,
    pub whitelist_enable: bool,
    pub width_pixels: i32,
}

/// `overrides` is `None` when the materials object is present and its list is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderMaterials {
    pub overrides: Option<Vec<MaterialOverride>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialOverride {
    pub te_index: u8,
    pub data: String,
}

pub(crate) fn parse_dyn_attrs(text: &str) -> Result<DynAttrs, ()> {
    let parsed = from_xml(text.as_bytes()).map_err(|_| ())?;
    let Llsd::Map(mut namespaces) = parsed else {
        return Err(());
    };
    namespaces.retain(|_, value| value.is_map());
    for value in namespaces.values_mut() {
        if let Llsd::Map(stores) = value {
            stores.retain(|_, store| store.is_map());
        }
    }
    Ok(DynAttrs { namespaces })
}

pub(crate) fn parse_keyframe(data: &[u8]) -> Option<KeyframeMotion> {
    if data.len() < 8 || data.len() > KFM_MAX_BYTES {
        return None;
    }
    let mut cursor = Cursor { data, pos: 0 };
    if cursor.take(4) != Some(b"KFM1") {
        return None;
    }
    if cursor.u32() != Some(1) {
        return None;
    }
    let serialized_position = cursor.vec3()?;
    let base_position = cursor.vec3()?;
    let base_rotation = cursor.quat()?;
    let current_frame = cursor.keyframe()?;
    let frame_count = cursor.i32()?;
    if !(0..=KFM_MAX_FRAMES).contains(&frame_count) {
        return None;
    }
    let frame_count = usize::try_from(frame_count).ok()?;
    let mut frames = Vec::new();
    for _ in 0..frame_count {
        frames.push(cursor.keyframe()?);
    }
    let kf_count = cursor.i32()?;
    if !(-1..=KFM_MAX_FRAMES).contains(&kf_count) {
        return None;
    }
    let keyframes = if kf_count < 0 {
        None
    } else {
        let kf_count = usize::try_from(kf_count).ok()?;
        let mut stored = Vec::new();
        for _ in 0..kf_count {
            stored.push(cursor.keyframe()?);
        }
        Some(stored)
    };
    Some(KeyframeMotion {
        serialized_position,
        base_position,
        base_rotation,
        current_frame,
        frames,
        keyframes,
        mode: cursor.i32()?,
        data: cursor.i32()?,
        running: cursor.boolean()?,
        iterations: cursor.i32()?,
        skip_loops: cursor.i32()?,
    })
}

pub(crate) fn parse_vehicle(text: &str) -> Result<Option<Vehicle>, ()> {
    if text.is_empty() {
        return Ok(None);
    }
    let root = ferroan_llsd::xml::parse(text.as_bytes()).map_err(|_| ())?;
    if root.name != "Vehicle" {
        return Err(());
    }
    let mut vehicle = Vehicle::blank();
    let mut seen = 0u8;
    let mut failed = false;
    for child in root.elements() {
        match apply_vehicle(&mut vehicle, &mut seen, child) {
            VehicleRead::Skip => {}
            VehicleRead::Ok => {}
            VehicleRead::Failed => failed = true,
        }
    }
    if failed {
        return Ok(None);
    }
    if seen != SEEN_ALL {
        let preset = inworldz_defaults(vehicle.kind);
        if seen & 1 == 0 {
            vehicle.mouselook_azimuth = preset[0];
        }
        if seen & 2 == 0 {
            vehicle.mouselook_altitude = preset[1];
        }
        if seen & 4 == 0 {
            vehicle.banking_azimuth = preset[2];
        }
        if seen & 8 == 0 {
            vehicle.disable_motors_above = preset[3];
        }
        if seen & 16 == 0 {
            vehicle.disable_motors_after = preset[4];
        }
        if seen & 32 == 0 {
            vehicle.inverted_banking_modifier = preset[5];
        }
    }
    Ok(Some(vehicle))
}

pub(crate) fn parse_inertia(text: &str) -> Result<Option<PhysicsInertia>, ()> {
    if text.is_empty() {
        return Ok(None);
    }
    let root = ferroan_llsd::xml::parse(text.as_bytes()).map_err(|_| ())?;
    if root.name != "PhysicsInertia" {
        return Err(());
    }
    let mut inertia = PhysicsInertia {
        mass: 0.0,
        center: [0.0; 3],
        inertia: [0.0; 3],
        rotation: [0.0; 4],
    };
    let mut failed = false;
    for child in root.elements() {
        let read = match child.name.as_str() {
            "MASS" => xml_f32(child).map(|value| inertia.mass = value),
            "CM" => xml_vec3(child).map(|value| inertia.center = value),
            "INERTIA" => xml_vec3(child).map(|value| inertia.inertia = value),
            "IROT" => xml_vec4(child).map(|value| inertia.rotation = value),
            _ => continue,
        };
        if read.is_none() {
            failed = true;
        }
    }
    if failed { Ok(None) } else { Ok(Some(inertia)) }
}

pub(crate) fn parse_animations(data: &[u8]) -> Option<Vec<ObjectAnimation>> {
    if data.len() < 2 {
        return Some(Vec::new());
    }
    let count = u16::from_le_bytes([data[0], data[1]]) as usize;
    if count == 0 {
        return None;
    }
    let mut animations = Vec::new();
    let mut pos = 2usize;
    for _ in 0..count {
        let raw = data.get(pos..pos.checked_add(16)?)?;
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(raw);
        let id = Uuid::from_bytes(bytes);
        if id.is_nil() {
            break;
        }
        pos = pos.checked_add(16)?;
        let len = *data.get(pos)? as usize;
        pos = pos.checked_add(1)?;
        let end = pos.checked_add(len)?;
        let name = String::from_utf8_lossy(data.get(pos..end)?).into_owned();
        if name.is_empty() {
            break;
        }
        pos = end;
        animations.push(ObjectAnimation { id, name });
    }
    Some(animations)
}

pub(crate) fn parse_linkset(text: &str) -> Option<LinksetData> {
    if text.chars().all(char::is_whitespace) {
        return None;
    }
    let value: Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    if let Some(record) = linkset_record(obj) {
        return Some(record);
    }
    linkset_fallback(obj)
}

pub(crate) fn parse_media(text: &str) -> Option<Vec<MediaFace>> {
    let root = ferroan_llsd::xml::parse(text.as_bytes()).ok()?;
    if root.attr("type") != Some("sl") || root.name != "OSMedia" {
        return None;
    }
    let wrapper = root.elements().next()?;
    let llsd = wrapper.elements().next()?;
    let value = from_xml_element(llsd).ok()?;
    let Llsd::Array(items) = value else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    Some(items.iter().map(media_face).collect())
}

pub(crate) fn parse_materials(data: &[u8]) -> Option<RenderMaterials> {
    if data.is_empty() {
        return None;
    }
    let count = usize::from(data[0]);
    if count > 128 {
        return None;
    }
    if count == 0 {
        return Some(RenderMaterials { overrides: None });
    }
    let mut index = 1usize;
    let mut overrides = Vec::with_capacity(count);
    for _ in 0..count {
        let te_index = *data.get(index)?;
        index = index.checked_add(1)?;
        let lo = usize::from(*data.get(index)?);
        index = index.checked_add(1)?;
        let hi = usize::from(*data.get(index)?);
        index = index.checked_add(1)?;
        let len = lo + (hi << 8);
        if len == 0 {
            return None;
        }
        let end = index.checked_add(len)?;
        let text = bytes_to_string(data.get(index..end)?);
        if text.starts_with("{\"asset") {
            return None;
        }
        overrides.push(MaterialOverride {
            te_index,
            data: text,
        });
        index = end;
    }
    Some(RenderMaterials {
        overrides: Some(overrides),
    })
}

impl Vehicle {
    fn blank() -> Self {
        Self {
            kind: 0,
            flags: 0,
            linear_motor_direction: [0.0; 3],
            linear_friction_timescale: [0.0; 3],
            linear_motor_decay_timescale: 0.0,
            linear_motor_timescale: 0.0,
            linear_motor_offset: [0.0; 3],
            angular_motor_direction: [0.0; 3],
            angular_motor_timescale: 0.0,
            angular_motor_decay_timescale: 0.0,
            angular_friction_timescale: [0.0; 3],
            angular_deflection_efficiency: 0.0,
            angular_deflection_timescale: 0.0,
            linear_deflection_efficiency: 0.0,
            linear_deflection_timescale: 0.0,
            banking_efficiency: 0.0,
            banking_mix: 0.0,
            banking_timescale: 0.0,
            hover_height: 0.0,
            hover_efficiency: 0.0,
            hover_timescale: 0.0,
            buoyancy: 0.0,
            vertical_attraction_efficiency: 0.0,
            vertical_attraction_timescale: 0.0,
            reference_frame: [0.0; 4],
            linear_motor_timescale_axes: None,
            linear_motor_decay_timescale_axes: None,
            angular_motor_timescale_axes: None,
            angular_motor_decay_timescale_axes: None,
            mouselook_azimuth: 0.0,
            mouselook_altitude: 0.0,
            banking_azimuth: 0.0,
            disable_motors_above: 0.0,
            disable_motors_after: 0.0,
            inverted_banking_modifier: 0.0,
            linear_wind_efficiency: None,
            angular_wind_efficiency: None,
            linear_motor_run_age: None,
            linear_motor_run_direction: None,
            linear_motor_run_target: None,
            angular_motor_run_age: None,
            angular_motor_run_direction: None,
            angular_motor_run_target: None,
        }
    }
}

impl LinksetEntry {
    fn cost(&self) -> i32 {
        if self.key.trim().is_empty() {
            return 0;
        }
        let mut cost = utf8_len(&self.key);
        if let Some(value) = self.value.as_deref().filter(|text| !text.trim().is_empty()) {
            cost = cost.wrapping_add(utf8_len(value));
        }
        if let Some(password) = self.password.as_deref().filter(|text| !text.is_empty()) {
            cost = cost.wrapping_add(utf8_len(password).max(32));
        }
        cost
    }
}

enum VehicleRead {
    Skip,
    Ok,
    Failed,
}

fn apply_vehicle(vehicle: &mut Vehicle, seen: &mut u8, child: &Element) -> VehicleRead {
    let applied = match child.name.as_str() {
        "TYPE" => xml_i32(child).map(|value| vehicle.kind = value),
        "FLAGS" => xml_i32(child).map(|value| vehicle.flags = value),
        "LMDIR" => xml_vec3(child).map(|value| vehicle.linear_motor_direction = value),
        "LMFTIME" => xml_vec3(child).map(|value| vehicle.linear_friction_timescale = value),
        "LMDTIME" => xml_f32(child).map(|value| vehicle.linear_motor_decay_timescale = value),
        "LMTIME" => xml_f32(child).map(|value| vehicle.linear_motor_timescale = value),
        "LMOFF" => xml_vec3(child).map(|value| vehicle.linear_motor_offset = value),
        "LMTIMEV" => xml_vec3(child).map(|value| vehicle.linear_motor_timescale_axes = Some(value)),
        "LMDTIMEV" => {
            xml_vec3(child).map(|value| vehicle.linear_motor_decay_timescale_axes = Some(value))
        }
        "AMDIR" => xml_vec3(child).map(|value| vehicle.angular_motor_direction = value),
        "AMTIME" => xml_f32(child).map(|value| vehicle.angular_motor_timescale = value),
        "AMDTIME" => xml_f32(child).map(|value| vehicle.angular_motor_decay_timescale = value),
        "AMFTIME" => xml_vec3(child).map(|value| vehicle.angular_friction_timescale = value),
        "AMTIMEV" => {
            xml_vec3(child).map(|value| vehicle.angular_motor_timescale_axes = Some(value))
        }
        "AMDTIMEV" => {
            xml_vec3(child).map(|value| vehicle.angular_motor_decay_timescale_axes = Some(value))
        }
        "ADEFF" => xml_f32(child).map(|value| vehicle.angular_deflection_efficiency = value),
        "ADTIME" => xml_f32(child).map(|value| vehicle.angular_deflection_timescale = value),
        "LDEFF" => xml_f32(child).map(|value| vehicle.linear_deflection_efficiency = value),
        "LDTIME" => xml_f32(child).map(|value| vehicle.linear_deflection_timescale = value),
        "BEFF" => xml_f32(child).map(|value| vehicle.banking_efficiency = value),
        "BMIX" => xml_f32(child).map(|value| vehicle.banking_mix = value),
        "BTIME" => xml_f32(child).map(|value| vehicle.banking_timescale = value),
        "HHEI" => xml_f32(child).map(|value| vehicle.hover_height = value),
        "HEFF" => xml_f32(child).map(|value| vehicle.hover_efficiency = value),
        "HTIME" => xml_f32(child).map(|value| vehicle.hover_timescale = value),
        "VBUO" => xml_f32(child).map(|value| vehicle.buoyancy = value),
        "VAEFF" => xml_f32(child).map(|value| vehicle.vertical_attraction_efficiency = value),
        "VATIME" => xml_f32(child).map(|value| vehicle.vertical_attraction_timescale = value),
        "REF_FRAME" => xml_vec4(child).map(|value| vehicle.reference_frame = value),
        "MLAZ" => xml_f32(child).map(|value| {
            vehicle.mouselook_azimuth = value;
            *seen |= 1;
        }),
        "MLALT" => xml_f32(child).map(|value| {
            vehicle.mouselook_altitude = value;
            *seen |= 2;
        }),
        "BAZ" => xml_f32(child).map(|value| {
            vehicle.banking_azimuth = value;
            *seen |= 4;
        }),
        "DMABOVE" => xml_f32(child).map(|value| {
            vehicle.disable_motors_above = value;
            *seen |= 8;
        }),
        "DMAFTER" => xml_f32(child).map(|value| {
            vehicle.disable_motors_after = value;
            *seen |= 16;
        }),
        "IBMOD" => xml_f32(child).map(|value| {
            vehicle.inverted_banking_modifier = value;
            *seen |= 32;
        }),
        "LWEFF" => xml_vec3(child).map(|value| vehicle.linear_wind_efficiency = Some(value)),
        "AWEFF" => xml_vec3(child).map(|value| vehicle.angular_wind_efficiency = Some(value)),
        "LMRAGE" => xml_f32(child).map(|value| vehicle.linear_motor_run_age = Some(value)),
        "LMRDIR" => xml_vec3(child).map(|value| vehicle.linear_motor_run_direction = Some(value)),
        "LMRTGT" => xml_vec3(child).map(|value| vehicle.linear_motor_run_target = Some(value)),
        "AMRAGE" => xml_f32(child).map(|value| vehicle.angular_motor_run_age = Some(value)),
        "AMRDIR" => xml_vec3(child).map(|value| vehicle.angular_motor_run_direction = Some(value)),
        "AMRTGT" => xml_vec3(child).map(|value| vehicle.angular_motor_run_target = Some(value)),
        _ => return VehicleRead::Skip,
    };
    if applied.is_some() {
        VehicleRead::Ok
    } else {
        VehicleRead::Failed
    }
}

fn inworldz_defaults(kind: i32) -> [f32; 6] {
    let pi = std::f32::consts::PI;
    let mut values = [pi / 4.0, pi / 4.0, pi / 2.0, 0.0, 0.0, 1.0];
    match kind {
        2 => {
            values[3] = 0.75;
            values[4] = 2.5;
        }
        10002 => {
            values[3] = 1.5;
            values[4] = 2.5;
            values[5] = -0.5;
        }
        10001 => values[5] = -0.2,
        _ => {}
    }
    values
}

fn linkset_record(obj: &serde_json::Map<String, Value>) -> Option<LinksetData> {
    let bytes_used = json_i32(obj.get("BytesUsed")?)?;
    let bytes_free = match obj.get("BytesFree") {
        None => LINKSET_MAX,
        Some(value) => json_i32(value)?,
    };
    let entries = match obj.get("Data") {
        None => Some(Vec::new()),
        Some(Value::Null) => None,
        Some(value) => Some(entry_list(value.as_object()?)?),
    };
    Some(LinksetData {
        entries,
        bytes_used,
        bytes_free,
    })
}

fn linkset_fallback(obj: &serde_json::Map<String, Value>) -> Option<LinksetData> {
    let entries = entry_list(obj)?;
    let cost = entries
        .iter()
        .fold(0i32, |total, entry| total.wrapping_add(entry.cost()));
    Some(LinksetData {
        entries: Some(entries),
        bytes_used: cost,
        bytes_free: LINKSET_MAX.wrapping_sub(cost),
    })
}

fn entry_list(obj: &serde_json::Map<String, Value>) -> Option<Vec<LinksetEntry>> {
    let mut entries = Vec::new();
    for (key, child) in obj {
        let child = child.as_object()?;
        entries.push(LinksetEntry {
            key: key.clone(),
            value: json_string(child.get("Value"))?,
            password: json_string(child.get("Password"))?,
        });
    }
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    Some(entries)
}

fn json_i32(value: &Value) -> Option<i32> {
    i32::try_from(value.as_i64()?).ok()
}

fn json_string(value: Option<&Value>) -> Option<Option<String>> {
    match value {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(text)) => Some(Some(text.clone())),
        Some(_) => None,
    }
}

fn utf8_len(text: &str) -> i32 {
    i32::try_from(text.len()).unwrap_or(i32::MAX)
}

fn media_face(value: &Llsd) -> MediaFace {
    if !value.is_map() {
        return MediaFace {
            alt_image_enable: false,
            auto_loop: false,
            auto_play: false,
            auto_scale: false,
            auto_zoom: false,
            controls: 0,
            current_url: None,
            first_click_interact: false,
            height_pixels: 0,
            home_url: None,
            perms_control: 0,
            perms_interact: 0,
            whitelist: None,
            whitelist_enable: false,
            width_pixels: 0,
        };
    }
    MediaFace {
        alt_image_enable: value.get("alt_image_enable").as_bool(),
        auto_loop: value.get("auto_loop").as_bool(),
        auto_play: value.get("auto_play").as_bool(),
        auto_scale: value.get("auto_scale").as_bool(),
        auto_zoom: value.get("auto_zoom").as_bool(),
        controls: value.get("controls").as_i32(),
        current_url: Some(value.get("current_url").to_osd_string()),
        first_click_interact: value.get("first_click_interact").as_bool(),
        height_pixels: value.get("height_pixels").as_i32(),
        home_url: Some(value.get("home_url").to_osd_string()),
        perms_control: value.get("perms_control").as_i32(),
        perms_interact: value.get("perms_interact").as_i32(),
        whitelist: media_whitelist(value.get("whitelist")),
        whitelist_enable: value.get("whitelist_enable").as_bool(),
        width_pixels: value.get("width_pixels").as_i32(),
    }
}

fn media_whitelist(value: &Llsd) -> Option<Vec<String>> {
    let Llsd::Array(items) = value else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    Some(items.iter().map(Llsd::to_osd_string).collect())
}

fn bytes_to_string(bytes: &[u8]) -> String {
    let bytes = match bytes.last() {
        Some(0) => &bytes[..bytes.len() - 1],
        _ => bytes,
    };
    String::from_utf8_lossy(bytes).into_owned()
}

fn xml_f32(element: &Element) -> Option<f32> {
    element.text().trim().parse().ok()
}

fn xml_i32(element: &Element) -> Option<i32> {
    element.text().trim().parse().ok()
}

fn xml_vec3(element: &Element) -> Option<[f32; 3]> {
    let children: Vec<&Element> = element.elements().collect();
    if children.len() != 3 {
        return None;
    }
    Some([
        xml_f32(children[0])?,
        xml_f32(children[1])?,
        xml_f32(children[2])?,
    ])
}

fn xml_vec4(element: &Element) -> Option<[f32; 4]> {
    let children: Vec<&Element> = element.elements().collect();
    if children.len() != 4 {
        return None;
    }
    Some([
        xml_f32(children[0])?,
        xml_f32(children[1])?,
        xml_f32(children[2])?,
        xml_f32(children[3])?,
    ])
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take(&mut self, len: usize) -> Option<&[u8]> {
        let end = self.pos.checked_add(len)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn boolean(&mut self) -> Option<bool> {
        self.u8().map(|byte| byte != 0)
    }

    fn i32(&mut self) -> Option<i32> {
        let bytes = self.take(4)?;
        Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes = self.take(4)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn f32(&mut self) -> Option<f32> {
        let bytes = self.take(4)?;
        Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn vec3(&mut self) -> Option<[f32; 3]> {
        Some([self.f32()?, self.f32()?, self.f32()?])
    }

    fn quat(&mut self) -> Option<[f32; 4]> {
        Some([self.f32()?, self.f32()?, self.f32()?, self.f32()?])
    }

    fn keyframe(&mut self) -> Option<Keyframe> {
        let position = if self.boolean()? {
            Some(self.vec3()?)
        } else {
            None
        };
        let rotation = if self.boolean()? {
            Some(self.quat()?)
        } else {
            None
        };
        Some(Keyframe {
            position,
            rotation,
            start_rotation: self.quat()?,
            time_ms: self.i32()?,
            time_total: self.i32()?,
            angular_velocity: self.vec3()?,
            start_position: self.vec3()?,
        })
    }
}
