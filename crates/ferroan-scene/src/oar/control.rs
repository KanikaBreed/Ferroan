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
//   Source/OpenSim.Region.CoreModules/World/Archiver/ArchiveReadRequest.cs
//     (LoadControlFile)
//   Source/OpenSim.Region.CoreModules/World/Archiver/DearchiveScenesGroup.cs
//     (DearchiveScenesInfo)
//   OpenMetaverse.Types/Vector3.cs (TryParse)
//     libopenmetaverse a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Control file and the region map.
//!
//! `LoadControlFile` reads each element, including the one that follows a
//! content value. Pin8's extra `Read` skips that element in compact XML
//! (TB195). Ferroan loads those regions (D132). A second `<dir>` files the
//! current region under the new name as well. `SetRegionDirectory` never
//! clears the current region.

use uuid::Uuid;

use super::SimulatorRegion;
use super::xml_text::{StartEl, XmlText};
use crate::parcels::vector_text::parse_f32;

const MAX_MAJOR_VERSION: i32 = 1;
const REGIONS_PATH: &str = "regions/";

struct RegionObj {
    original_id: String,
    loc_x: i32,
    loc_y: i32,
    size_bits: [i32; 3],
    scene_id: Option<Uuid>,
}

struct DirKey {
    directory: String,
    index: usize,
}

pub(super) struct RegionRow {
    pub directory: String,
    pub original_id: String,
    pub loc_x: i32,
    pub loc_y: i32,
    pub size_bits: [i32; 3],
    pub scene_id: Option<Uuid>,
}

pub(super) struct LoadedControl {
    pub error: Option<String>,
    pub creation_time: i32,
    pub default_original_id: String,
    pub multi_region: bool,
    pub control_loaded: bool,
    pub incoming_size_bits: [i32; 3],
    pub regions: Vec<RegionRow>,
    pub loaded: Vec<Uuid>,
}

struct Draft {
    original_id: String,
    loc_x: i32,
    loc_y: i32,
    size_bits: [i32; 3],
}

enum Current {
    None,
    Draft(Draft),
    Stored(usize),
}

pub(super) struct Scenes {
    multi: bool,
    creation: i32,
    default_original_id: String,
    regions: Vec<RegionObj>,
    keys: Vec<DirKey>,
    loaded: Vec<Uuid>,
    cur_y: Option<i32>,
    cur_x: Option<i32>,
    current: Current,
}

pub(super) fn default_size_bits() -> [i32; 3] {
    [net_bits(256.0), net_bits(256.0), net_bits(f32::MAX)]
}

pub(super) fn fresh_scenes() -> Scenes {
    Scenes {
        multi: false,
        creation: 0,
        default_original_id: String::new(),
        regions: Vec::new(),
        keys: Vec::new(),
        loaded: Vec::new(),
        cur_y: None,
        cur_x: None,
        current: Current::None,
    }
}

pub(super) fn load_control(
    data: &[u8],
    scenes: &mut Scenes,
    incoming: &mut [i32; 3],
) -> Result<(), String> {
    scenes.creation = 0;
    scenes.default_original_id.clear();
    let text = ascii_bytes(data);
    let mut reader = XmlText::new(text);
    let mut multi = false;
    while let Some(element) = reader.next_element()? {
        on_element(&element, &mut reader, scenes, incoming, &mut multi)?;
    }
    scenes.multi = multi;
    if !multi {
        scenes.start_row();
        scenes.start_region();
        let id = scenes.default_original_id.clone();
        scenes.set_original_id(&id);
        scenes.set_directory("");
        scenes.set_size(*incoming);
    }
    Ok(())
}

fn on_element(
    element: &StartEl,
    reader: &mut XmlText,
    scenes: &mut Scenes,
    incoming: &mut [i32; 3],
    multi: &mut bool,
) -> Result<(), String> {
    match element.name.as_str() {
        "archive" => {
            let major = require_int(&element.attrs, "major_version")?;
            let _minor = require_int(&element.attrs, "minor_version")?;
            if major > MAX_MAJOR_VERSION {
                return Err(version_error(major));
            }
        }
        "datetime" => {
            let text = reader.read_content(element)?;
            if let Some(value) = try_i32(&text) {
                scenes.creation = value;
            }
        }
        "row" => {
            *multi = true;
            scenes.start_row();
        }
        "region" => scenes.start_region(),
        "id" => {
            let id = reader.read_content(element)?;
            scenes.default_original_id = id.clone();
            if *multi {
                scenes.set_original_id(&id);
            }
        }
        "dir" => {
            let directory = reader.read_content(element)?;
            scenes.set_directory(&directory);
        }
        "size_in_meters" => {
            let text = reader.read_content(element)?;
            if let Some(size) = try_vector3(&format!("<{text},0>")) {
                *incoming = size;
                if *multi {
                    scenes.set_size(*incoming);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn version_error(major: i32) -> String {
    format!(
        "The OAR you are trying to load has major version number of {major} but this version \
         can only load OARs with major version number {MAX_MAJOR_VERSION} and below"
    )
}

impl Scenes {
    fn start_row(&mut self) {
        self.cur_y = Some(match self.cur_y {
            None => 0,
            Some(y) => y.wrapping_add(1),
        });
        self.cur_x = None;
    }

    fn start_region(&mut self) {
        self.cur_x = Some(match self.cur_x {
            None => 0,
            Some(x) => x.wrapping_add(1),
        });
    }

    fn set_original_id(&mut self, id: &str) {
        self.current = Current::Draft(Draft {
            original_id: id.to_string(),
            loc_x: self.cur_x.unwrap_or(0),
            loc_y: self.cur_y.unwrap_or(0),
            size_bits: default_size_bits(),
        });
    }

    /// Files the current region under `directory`. A region already stored
    /// stays current, so a second directory names that same object.
    fn set_directory(&mut self, directory: &str) {
        let index = match std::mem::replace(&mut self.current, Current::None) {
            Current::None => return,
            Current::Stored(index) => {
                self.current = Current::Stored(index);
                index
            }
            Current::Draft(draft) => {
                let index = self.regions.len();
                self.regions.push(RegionObj {
                    original_id: draft.original_id,
                    loc_x: draft.loc_x,
                    loc_y: draft.loc_y,
                    size_bits: draft.size_bits,
                    scene_id: None,
                });
                self.current = Current::Stored(index);
                index
            }
        };
        if let Some(key) = self.keys.iter_mut().find(|key| key.directory == directory) {
            key.index = index;
            return;
        }
        self.keys.push(DirKey {
            directory: directory.to_string(),
            index,
        });
    }

    fn set_size(&mut self, size: [i32; 3]) {
        match &mut self.current {
            Current::Draft(draft) => draft.size_bits = size,
            Current::Stored(index) => {
                if let Some(region) = self.regions.get_mut(*index) {
                    region.size_bits = size;
                }
            }
            Current::None => {}
        }
    }

    pub(super) fn match_simulator(&mut self, root: &SimulatorRegion, scenes: &[SimulatorRegion]) {
        for region in &mut self.regions {
            region.scene_id = None;
        }
        self.loaded.clear();
        let indexes: Vec<usize> = self.keys.iter().map(|key| key.index).collect();
        for index in indexes {
            let region = &mut self.regions[index];
            let x = (root.loc_x as i32).wrapping_add(region.loc_x);
            let y = (root.loc_y as i32).wrapping_add(region.loc_y);
            if x < 0 || y < 0 {
                region.scene_id = None;
                continue;
            }
            let Some(scene) = scenes
                .iter()
                .rev()
                .find(|scene| scene.loc_x == x as u32 && scene.loc_y == y as u32)
            else {
                region.scene_id = None;
                continue;
            };
            region.scene_id = Some(scene.id);
            if !self.loaded.contains(&scene.id) {
                self.loaded.push(scene.id);
            }
        }
    }

    /// `(use this file, scene, relative path)`. The `regions/` check is
    /// ordinal. Pin8's culture-sensitive `StartsWith` is TB196 (D133).
    pub(super) fn region_from_path(&self, full_path: &str) -> (bool, Option<Uuid>, String) {
        if !self.multi {
            return (true, self.loaded.first().copied(), full_path.to_string());
        }
        if !full_path.starts_with(REGIONS_PATH) {
            return (true, None, full_path.to_string());
        }
        let mut parts = full_path.splitn(3, '/');
        let _archive = parts.next();
        let (Some(directory), Some(relative)) = (parts.next(), parts.next()) else {
            return (false, None, full_path.to_string());
        };
        let Some(key) = self.keys.iter().find(|key| key.directory == directory) else {
            return (false, None, relative.to_string());
        };
        let scene_id = self.regions[key.index].scene_id;
        (scene_id.is_some(), scene_id, relative.to_string())
    }

    pub(super) fn finish(
        self,
        error: Option<String>,
        control_loaded: bool,
        incoming: [i32; 3],
    ) -> LoadedControl {
        let regions = self
            .keys
            .iter()
            .map(|key| {
                let region = &self.regions[key.index];
                RegionRow {
                    directory: key.directory.clone(),
                    original_id: region.original_id.clone(),
                    loc_x: region.loc_x,
                    loc_y: region.loc_y,
                    size_bits: region.size_bits,
                    scene_id: region.scene_id,
                }
            })
            .collect();
        LoadedControl {
            error,
            creation_time: self.creation,
            default_original_id: self.default_original_id,
            multi_region: self.multi,
            control_loaded,
            incoming_size_bits: incoming,
            regions,
            loaded: self.loaded,
        }
    }
}

fn ascii_bytes(data: &[u8]) -> Vec<u8> {
    data.iter()
        .map(|byte| if *byte >= 128 { b'?' } else { *byte })
        .collect()
}

fn require_int(attrs: &[(String, String)], name: &str) -> Result<i32, String> {
    match attrs.iter().find(|(key, _)| key == name) {
        Some((_, value)) => parse_i32(value),
        None => Err("Value cannot be null. (Parameter 's')".to_string()),
    }
}

fn try_i32(text: &str) -> Option<i32> {
    parse_i32(text).ok()
}

fn parse_i32(text: &str) -> Result<i32, String> {
    let trimmed = text.trim();
    if !integer_text(trimmed) {
        return Err(format!(
            "The input string '{text}' was not in a correct format."
        ));
    }
    match trimmed.parse::<i32>() {
        Ok(value) => Ok(value),
        Err(_) => Err("Value was either too large or too small for an Int32.".to_string()),
    }
}

fn integer_text(text: &str) -> bool {
    let digits = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// `Vector3.TryParse` for the `<x,y,z>` form `LoadControlFile` builds.
/// Components go through the shared en-US parser. `NaN` keeps Pin8's sign bit.
fn try_vector3(text: &str) -> Option<[i32; 3]> {
    let bytes = text.as_bytes();
    if bytes.len() < 7 {
        return None;
    }
    let mut comma = 0usize;
    let mut mark;
    loop {
        mark = bytes[comma];
        if mark == b',' || mark == b'<' {
            break;
        }
        comma += 1;
        if comma >= bytes.len() {
            break;
        }
    }
    let mut start = 0usize;
    if mark == b'<' {
        comma += 1;
        start = comma;
        loop {
            comma += 1;
            if comma >= bytes.len() || bytes[comma] == b',' {
                break;
            }
        }
        if start > bytes.len().saturating_sub(6) {
            return None;
        }
    }
    let x = parse_f32(text.get(start..comma)?)?;
    comma += 1;
    start = comma;
    loop {
        comma += 1;
        if comma >= bytes.len() || bytes[comma] == b',' {
            break;
        }
    }
    if comma > bytes.len().saturating_sub(3) {
        return None;
    }
    let y = parse_f32(text.get(start..comma)?)?;
    comma += 1;
    start = comma;
    loop {
        comma += 1;
        if comma >= bytes.len() || bytes[comma] == b' ' || bytes[comma] == b'>' {
            break;
        }
    }
    let z = parse_f32(text.get(start..comma.min(bytes.len()))?)?;
    Some([net_bits(x), net_bits(y), net_bits(z)])
}

/// `.NET` `Single.NaN` is the quiet NaN with the sign bit set.
fn net_bits(value: f32) -> i32 {
    if value.is_nan() {
        -4_194_304
    } else {
        value.to_bits() as i32
    }
}
