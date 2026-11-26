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
//   Source/OpenSim.Region.CoreModules/World/Archiver/ArchiveReadRequest.cs
//     (LoadControlFile)
//   Source/OpenSim.Region.CoreModules/World/Archiver/DearchiveScenesGroup.cs
//     (DearchiveScenesInfo)
//   OpenMetaverse.Types/Vector3.cs (TryParse)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Control file and the region map. `ReadElementContentAsString` leaves the
//! reader on the node after the end tag, and the caller's next `Read` skips
//! that node.

use ferroan_llsd::xml::{self, Element, Node};
use uuid::Uuid;

use super::SimulatorRegion;

const MAX_MAJOR_VERSION: i32 = 1;
const REGIONS_PATH: &str = "regions/";

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
    regions: Vec<RegionRow>,
    loaded: Vec<Uuid>,
    cur_y: Option<i32>,
    cur_x: Option<i32>,
    current: Current,
}

pub(super) fn default_size_bits() -> [i32; 3] {
    [
        256.0f32.to_bits() as i32,
        256.0f32.to_bits() as i32,
        f32::MAX.to_bits() as i32,
    ]
}

pub(super) fn fresh_scenes() -> Scenes {
    Scenes {
        multi: false,
        creation: 0,
        default_original_id: String::new(),
        regions: Vec::new(),
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
    let root = xml::parse(&text).map_err(|error| error.to_string())?;
    let mut cursor = Cursor::new(&root);
    let mut multi = false;
    while cursor.read() {
        let (name, attrs) = match cursor.current() {
            Some(Tok::Start { name, attrs, .. }) => (name.clone(), attrs.clone()),
            _ => continue,
        };
        on_element(&name, &attrs, &mut cursor, scenes, incoming, &mut multi)?;
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
    name: &str,
    attrs: &[(String, String)],
    cursor: &mut Cursor,
    scenes: &mut Scenes,
    incoming: &mut [i32; 3],
    multi: &mut bool,
) -> Result<(), String> {
    match name {
        "archive" => {
            let major = require_int(attrs, "major_version")?;
            let _minor = require_int(attrs, "minor_version")?;
            if major > MAX_MAJOR_VERSION {
                return Err(version_error(major));
            }
        }
        "datetime" => {
            let text = cursor.read_element_content();
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
            let id = cursor.read_element_content();
            scenes.default_original_id = id.clone();
            if *multi {
                scenes.set_original_id(&id);
            }
        }
        "dir" => {
            let directory = cursor.read_element_content();
            scenes.set_directory(&directory);
        }
        "size_in_meters" => {
            let text = cursor.read_element_content();
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
        self.cur_y = Some(self.cur_y.map_or(0, |y| y + 1));
        self.cur_x = None;
    }

    fn start_region(&mut self) {
        self.cur_x = Some(self.cur_x.map_or(0, |x| x + 1));
    }

    fn set_original_id(&mut self, id: &str) {
        self.current = Current::Draft(Draft {
            original_id: id.to_string(),
            loc_x: self.cur_x.unwrap_or(0),
            loc_y: self.cur_y.unwrap_or(0),
            size_bits: default_size_bits(),
        });
    }

    fn set_directory(&mut self, directory: &str) {
        let draft = match std::mem::replace(&mut self.current, Current::None) {
            Current::Draft(draft) => draft,
            Current::Stored(index) => {
                self.current = Current::Stored(index);
                return;
            }
            Current::None => return,
        };
        if let Some(index) = self
            .regions
            .iter()
            .position(|region| region.directory == directory)
        {
            let scene_id = self.regions[index].scene_id;
            self.regions[index] = row(directory, &draft, scene_id);
            self.current = Current::Stored(index);
            return;
        }
        self.regions.push(row(directory, &draft, None));
        self.current = Current::Stored(self.regions.len() - 1);
    }

    fn set_size(&mut self, size: [i32; 3]) {
        let stored = match &mut self.current {
            Current::Draft(draft) => {
                draft.size_bits = size;
                return;
            }
            Current::Stored(index) => *index,
            Current::None => return,
        };
        if let Some(region) = self.regions.get_mut(stored) {
            region.size_bits = size;
        }
    }

    pub(super) fn match_simulator(&mut self, root: &SimulatorRegion, scenes: &[SimulatorRegion]) {
        self.loaded.clear();
        for region in &mut self.regions {
            region.scene_id = None;
            let x = root.loc_x as i32 + region.loc_x;
            let y = root.loc_y as i32 + region.loc_y;
            if x < 0 || y < 0 {
                continue;
            }
            let Some(scene) = scenes
                .iter()
                .rev()
                .find(|scene| scene.loc_x == x as u32 && scene.loc_y == y as u32)
            else {
                continue;
            };
            region.scene_id = Some(scene.id);
            if !self.loaded.contains(&scene.id) {
                self.loaded.push(scene.id);
            }
        }
    }

    /// `(use this file, scene, relative path)`.
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
        let Some(region) = self
            .regions
            .iter()
            .find(|region| region.directory == directory)
        else {
            return (false, None, relative.to_string());
        };
        (
            region.scene_id.is_some(),
            region.scene_id,
            relative.to_string(),
        )
    }

    pub(super) fn finish(
        self,
        error: Option<String>,
        control_loaded: bool,
        incoming: [i32; 3],
    ) -> LoadedControl {
        LoadedControl {
            error,
            creation_time: self.creation,
            default_original_id: self.default_original_id,
            multi_region: self.multi,
            control_loaded,
            incoming_size_bits: incoming,
            regions: self.regions,
            loaded: self.loaded,
        }
    }
}

fn row(directory: &str, draft: &Draft, scene_id: Option<Uuid>) -> RegionRow {
    RegionRow {
        directory: directory.to_string(),
        original_id: draft.original_id.clone(),
        loc_x: draft.loc_x,
        loc_y: draft.loc_y,
        size_bits: draft.size_bits,
        scene_id,
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
    let x = parse_float(text.get(start..comma)?)?;
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
    let y = parse_float(text.get(start..comma)?)?;
    comma += 1;
    start = comma;
    loop {
        comma += 1;
        if comma >= bytes.len() || bytes[comma] == b' ' || bytes[comma] == b'>' {
            break;
        }
    }
    let z = parse_float(text.get(start..comma.min(bytes.len()))?)?;
    Some([x.to_bits() as i32, y.to_bits() as i32, z.to_bits() as i32])
}

fn parse_float(text: &str) -> Option<f32> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let body = trimmed
        .strip_prefix('+')
        .or_else(|| trimmed.strip_prefix('-'))
        .unwrap_or(trimmed);
    if body.is_empty()
        || !body
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return None;
    }
    trimmed.parse().ok()
}

enum Tok {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        empty: bool,
    },
    End,
    Text(String),
}

struct Cursor {
    toks: Vec<Tok>,
    index: usize,
    started: bool,
}

impl Cursor {
    fn new(root: &Element) -> Self {
        let mut toks = Vec::new();
        flatten(root, &mut toks);
        Self {
            toks,
            index: 0,
            started: false,
        }
    }

    fn read(&mut self) -> bool {
        if !self.started {
            self.started = true;
            self.index = 0;
        } else if self.index < self.toks.len() {
            self.index += 1;
        }
        self.index < self.toks.len()
    }

    fn current(&self) -> Option<&Tok> {
        self.toks.get(self.index)
    }

    fn read_element_content(&mut self) -> String {
        if matches!(self.current(), Some(Tok::Start { empty: true, .. })) {
            let _ = self.read();
            return String::new();
        }
        let mut depth = 1i32;
        let mut text = String::new();
        loop {
            if !self.read() {
                break;
            }
            let step = match self.current() {
                Some(Tok::Start { empty: false, .. }) => Step::Deeper,
                Some(Tok::End) => Step::Shallower,
                Some(Tok::Text(value)) => Step::Text(value.clone()),
                _ => Step::Skip,
            };
            match step {
                Step::Deeper => depth += 1,
                Step::Shallower => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Step::Text(value) => text.push_str(&value),
                Step::Skip => {}
            }
        }
        let _ = self.read();
        text
    }
}

enum Step {
    Deeper,
    Shallower,
    Text(String),
    Skip,
}

fn flatten(element: &Element, out: &mut Vec<Tok>) {
    out.push(Tok::Start {
        name: element.name.clone(),
        attrs: element.attrs.clone(),
        empty: element.self_closing,
    });
    if element.self_closing {
        return;
    }
    for child in &element.children {
        match child {
            Node::Text(text) => out.push(Tok::Text(text.clone())),
            Node::Element(child) => flatten(child, out),
        }
    }
    out.push(Tok::End);
}
