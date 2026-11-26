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
//     (FindAndLoadControlFile, the region-path gate in DearchiveRegion)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! OAR control file and the region a path belongs to.
//!
//! `load_oar` is `FindAndLoadControlFile` plus the `GetRegionFromPath` test
//! `DearchiveRegion` applies to every later member. Terrain, objects, parcels,
//! settings and assets stay with their own rows.

mod control;
mod tar;

use std::io::Read;

use flate2::read::GzDecoder;
use uuid::Uuid;

use control::{LoadedControl, RegionRow};
use tar::{TYPE_DIRECTORY, TarEntry};

const CONTROL_FILE: &str = "archive.xml";
const SEEK_ERROR: &str = "[ARCHIVER]: Error reading archive: control file wasn't the first file, and the input stream doesn't allow seeking";
const MISSING_CONTROL: &str = "[ARCHIVER]: Control file not found";

/// One simulator region the load can match. `loc_x` and `loc_y` are grid cells.
#[derive(Clone, Copy, Debug)]
pub struct SimulatorRegion {
    pub id: Uuid,
    pub loc_x: u32,
    pub loc_y: u32,
}

/// Where the archive bytes came from. A late control file seeks or reopens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OarSource {
    /// Gzip member. A late control file decompresses the same bytes again.
    GzipPath,
    /// Uncompressed tar that can be read from the start again.
    TarSeek,
    /// Uncompressed tar that cannot be read from the start again.
    TarNoSeek,
}

/// One archived region after the control file and the simulator match.
#[derive(Clone, Debug)]
pub struct ArchivedRegion {
    pub directory: String,
    pub original_id: String,
    pub loc_x: i32,
    pub loc_y: i32,
    pub size_bits: [i32; 3],
    pub scene_id: Option<Uuid>,
}

/// One tar member after the control file has been loaded.
#[derive(Clone, Debug)]
pub struct OarEntry {
    pub path: String,
    pub kind: &'static str,
    /// `GetRegionFromPath`'s return. Directories are skipped before that call.
    pub use_entry: bool,
    pub scene_id: Option<Uuid>,
    pub relative: String,
    pub body: Vec<u8>,
}

/// The control file, the matched regions, and the path decision for each member.
#[derive(Clone, Debug)]
pub struct OarReport {
    pub error: Option<String>,
    pub creation_time: i32,
    pub default_original_id: String,
    pub multi_region: bool,
    pub control_loaded: bool,
    pub incoming_size_bits: [i32; 3],
    pub scene_count: i32,
    pub loaded_scenes: Vec<Uuid>,
    pub regions: Vec<ArchivedRegion>,
    pub entries: Vec<OarEntry>,
}

/// Load the control file and map every later member the way Pin8 does.
pub fn load_oar(
    root: &SimulatorRegion,
    scenes: &[SimulatorRegion],
    bytes: &[u8],
    source: OarSource,
) -> OarReport {
    let tar = match tar_bytes(bytes, source) {
        Ok(tar) => tar,
        Err(error) => return empty_report(Some(error)),
    };
    let members = tar::entries(&tar);
    let Some(control_at) = find_control(&members) else {
        return empty_report(Some(MISSING_CONTROL.to_string()));
    };
    let mut state = control::fresh_scenes();
    let mut incoming = control::default_size_bits();
    if let Err(error) = control::load_control(&members[control_at].body, &mut state, &mut incoming)
    {
        return report_of(state.finish(Some(error), false, incoming), Vec::new());
    }
    state.match_simulator(root, scenes);
    if !control_is_first(&members, control_at) && source == OarSource::TarNoSeek {
        return report_of(
            state.finish(Some(SEEK_ERROR.to_string()), true, incoming),
            Vec::new(),
        );
    }
    let walk = if control_is_first(&members, control_at) {
        &members[control_at + 1..]
    } else {
        members.as_slice()
    };
    let entries = walk.iter().map(|entry| map_entry(entry, &state)).collect();
    report_of(state.finish(None, true, incoming), entries)
}

fn tar_bytes(bytes: &[u8], source: OarSource) -> Result<Vec<u8>, String> {
    match source {
        OarSource::GzipPath => {
            let mut decoder = GzDecoder::new(bytes);
            let mut tar = Vec::new();
            decoder
                .read_to_end(&mut tar)
                .map_err(|error| error.to_string())?;
            Ok(tar)
        }
        OarSource::TarSeek | OarSource::TarNoSeek => Ok(bytes.to_vec()),
    }
}

fn find_control(members: &[TarEntry]) -> Option<usize> {
    members
        .iter()
        .position(|entry| entry.kind != TYPE_DIRECTORY && entry.path == CONTROL_FILE)
}

/// A directory before the control file does not make the control file late.
fn control_is_first(members: &[TarEntry], control_at: usize) -> bool {
    members
        .iter()
        .take(control_at)
        .all(|entry| entry.kind == TYPE_DIRECTORY)
}

fn map_entry(entry: &TarEntry, state: &control::Scenes) -> OarEntry {
    if entry.kind == TYPE_DIRECTORY {
        return OarEntry {
            path: entry.path.clone(),
            kind: entry.kind,
            use_entry: false,
            scene_id: None,
            relative: entry.path.clone(),
            body: entry.body.clone(),
        };
    }
    let (use_entry, scene_id, relative) = state.region_from_path(&entry.path);
    OarEntry {
        path: entry.path.clone(),
        kind: entry.kind,
        use_entry,
        scene_id,
        relative,
        body: entry.body.clone(),
    }
}

fn report_of(loaded: LoadedControl, entries: Vec<OarEntry>) -> OarReport {
    OarReport {
        error: loaded.error,
        creation_time: loaded.creation_time,
        default_original_id: loaded.default_original_id,
        multi_region: loaded.multi_region,
        control_loaded: loaded.control_loaded,
        incoming_size_bits: loaded.incoming_size_bits,
        scene_count: i32::try_from(loaded.regions.len()).unwrap_or(i32::MAX),
        loaded_scenes: loaded.loaded,
        regions: loaded.regions.into_iter().map(archived).collect(),
        entries,
    }
}

fn archived(region: RegionRow) -> ArchivedRegion {
    ArchivedRegion {
        directory: region.directory,
        original_id: region.original_id,
        loc_x: region.loc_x,
        loc_y: region.loc_y,
        size_bits: region.size_bits,
        scene_id: region.scene_id,
    }
}

fn empty_report(error: Option<String>) -> OarReport {
    OarReport {
        error,
        creation_time: 0,
        default_original_id: String::new(),
        multi_region: false,
        control_loaded: false,
        incoming_size_bits: control::default_size_bits(),
        scene_count: 0,
        loaded_scenes: Vec::new(),
        regions: Vec::new(),
        entries: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;

    #[test]
    fn pin8_load_oar_control_matches_the_recorded_regions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oar-control");
        let document: Value =
            serde_json::from_slice(&fs::read(root.join("load.json")).unwrap()).unwrap();
        let references = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/REFERENCES.md"),
        )
        .unwrap();
        let pin = document["pin"].as_str().unwrap();
        assert!(references.contains(&format!("`{pin}`")));
        let cases = document["cases"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let bytes = fs::read(root.join(case["archive"].as_str().unwrap())).unwrap();
            let report = load_oar(
                &region_of(&case["root"]),
                &case["scenes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(region_of)
                    .collect::<Vec<_>>(),
                &bytes,
                source_of(case["source"].as_str().unwrap()),
            );
            assert_eq!(report_value(&report), control_value(case), "{name}");
            if case["source"] == "gzip-path" {
                assert_path_event(case, name);
            }
        }
    }

    fn region_of(value: &Value) -> SimulatorRegion {
        SimulatorRegion {
            id: Uuid::parse_str(value["id"].as_str().unwrap()).unwrap(),
            loc_x: value["loc_x"].as_u64().unwrap() as u32,
            loc_y: value["loc_y"].as_u64().unwrap() as u32,
        }
    }

    fn source_of(name: &str) -> OarSource {
        match name {
            "gzip-path" => OarSource::GzipPath,
            "tar-seek" => OarSource::TarSeek,
            "tar-noseek" => OarSource::TarNoSeek,
            _ => panic!("unknown oar source {name}"),
        }
    }

    fn report_value(report: &OarReport) -> Value {
        json!({
            "error": report.error,
            "creation_time": report.creation_time,
            "default_original_id": report.default_original_id,
            "multi_region": report.multi_region,
            "control_loaded": report.control_loaded,
            "incoming_size_bits": report.incoming_size_bits,
            "scene_count": report.scene_count,
            "loaded_scenes": report.loaded_scenes.iter().map(Uuid::to_string).collect::<Vec<_>>(),
            "regions": report.regions.iter().map(region_value).collect::<Vec<_>>(),
            "entries": report.entries.iter().map(entry_value).collect::<Vec<_>>(),
        })
    }

    fn region_value(region: &ArchivedRegion) -> Value {
        json!({
            "directory": region.directory,
            "original_id": region.original_id,
            "loc_x": region.loc_x,
            "loc_y": region.loc_y,
            "size_bits": region.size_bits,
            "scene_id": region.scene_id.map(|id| id.to_string()),
        })
    }

    fn entry_value(entry: &OarEntry) -> Value {
        json!({
            "path": entry.path,
            "type": entry.kind,
            "use": entry.use_entry,
            "scene_id": entry.scene_id.map(|id| id.to_string()),
            "relative": entry.relative,
        })
    }

    fn control_value(case: &Value) -> Value {
        let mut value = case.clone();
        let object = value.as_object_mut().unwrap();
        for key in [
            "name",
            "archive",
            "source",
            "root",
            "scenes",
            "dearchive_fired",
            "dearchive_scenes",
            "dearchive_error",
            "dearchive_thrown",
        ] {
            object.remove(key);
        }
        value
    }

    fn assert_path_event(case: &Value, name: &str) {
        assert!(case["dearchive_fired"].as_bool().unwrap(), "{name}");
        assert!(case["dearchive_thrown"].is_null(), "{name}");
        if case["error"].is_null() {
            assert_eq!(case["dearchive_error"].as_str(), Some(""), "{name}");
            assert_eq!(case["dearchive_scenes"], case["loaded_scenes"], "{name}");
        } else {
            assert!(
                case["dearchive_scenes"].as_array().unwrap().is_empty(),
                "{name}"
            );
            assert_eq!(case["dearchive_error"], case["error"], "{name}");
        }
    }
}
