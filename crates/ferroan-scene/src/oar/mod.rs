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
//! `DearchiveRegion` applies to every later member. The walk decides from the
//! path and the declared size, then reads or skips. `load_oar` keeps no member
//! body after the control file. Terrain, objects, parcels, settings and assets
//! stay with their own rows. A throw reports an empty scene list. The regions
//! parsed before a throw that happens after the control file stay on the report.

mod control;
mod tar;
mod xml_text;

use std::io::{Cursor, Read};

use flate2::read::MultiGzDecoder;
use uuid::Uuid;

use control::LoadedControl;
use tar::TYPE_DIRECTORY;

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
    /// Concatenated gzip members are one archive, as `GZipStream` reads them.
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
    /// Bytes the caller asked to keep. A skipped member stores nothing.
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
    match source {
        OarSource::GzipPath => load_from(
            root,
            scenes,
            tar::Session::new(MultiGzDecoder::new(bytes)),
            true,
            || tar::Session::new(MultiGzDecoder::new(bytes)),
            |_, _, _| false,
        ),
        OarSource::TarSeek => load_from(
            root,
            scenes,
            tar::Session::new(Cursor::new(bytes)),
            true,
            || tar::Session::new(Cursor::new(bytes)),
            |_, _, _| false,
        ),
        OarSource::TarNoSeek => load_from(
            root,
            scenes,
            tar::Session::new(Cursor::new(bytes)),
            false,
            || tar::Session::new(Cursor::new(bytes)),
            |_, _, _| false,
        ),
    }
}

fn load_from<R, F, K>(
    root: &SimulatorRegion,
    scenes: &[SimulatorRegion],
    mut session: tar::Session<R>,
    can_reopen: bool,
    reopen: F,
    mut keep: K,
) -> OarReport
where
    R: Read,
    F: FnOnce() -> tar::Session<R>,
    K: FnMut(&str, &str, i32) -> bool,
{
    let mut incoming = control::default_size_bits();
    let found = match find_control(&mut session) {
        Ok(found) => found,
        Err(error) => return empty_report(Some(error)),
    };
    let Some(control_body) = found.body else {
        return empty_report(Some(MISSING_CONTROL.to_string()));
    };
    let mut state = control::fresh_scenes();
    if let Err(error) = control::load_control(&control_body, &mut state, &mut incoming) {
        return control_failure(error, incoming);
    }
    state.match_simulator(root, scenes);
    if found.late {
        if !can_reopen {
            return report_of(
                state.finish(Some(SEEK_ERROR.to_string()), true, incoming),
                Vec::new(),
            );
        }
        session = reopen();
    }
    let (entries, error) = walk(&mut session, &state, &mut keep);
    report_of(state.finish(error, true, incoming), entries)
}

struct Found {
    body: Option<Vec<u8>>,
    late: bool,
}

fn find_control<R: Read>(session: &mut tar::Session<R>) -> Result<Found, String> {
    let mut saw_file = false;
    loop {
        let Some(member) = session.next_member()? else {
            return Ok(Found {
                body: None,
                late: false,
            });
        };
        if member.kind == TYPE_DIRECTORY {
            session.skip_body()?;
            continue;
        }
        if member.path == CONTROL_FILE {
            tar::member_limit(member.size)?;
            tar::kept_limit(CONTROL_FILE, member.size)?;
            let body = session.read_body()?;
            return Ok(Found {
                body: Some(body),
                late: saw_file,
            });
        }
        session.skip_body()?;
        saw_file = true;
    }
}

/// `keep` sees the archive path, the tar type and the declared size. It does
/// not see the body. A directory is skipped and is not offered to `keep`.
fn walk<R, K>(
    session: &mut tar::Session<R>,
    state: &control::Scenes,
    keep: &mut K,
) -> (Vec<OarEntry>, Option<String>)
where
    R: Read,
    K: FnMut(&str, &str, i32) -> bool,
{
    let mut entries = Vec::new();
    loop {
        let member = match session.next_member() {
            Ok(Some(member)) => member,
            Ok(None) => return (entries, None),
            Err(error) => return (entries, Some(error)),
        };
        if let Err(error) = tar::member_limit(member.size) {
            return (entries, Some(error));
        }
        if member.kind == TYPE_DIRECTORY {
            if let Err(error) = session.skip_body() {
                return (entries, Some(error));
            }
            entries.push(OarEntry {
                path: member.path.clone(),
                kind: member.kind,
                use_entry: false,
                scene_id: None,
                relative: member.path,
                body: Vec::new(),
            });
            continue;
        }
        let (use_entry, scene_id, relative) = state.region_from_path(&member.path);
        let body = if keep(&member.path, member.kind, member.size) {
            if let Err(error) = tar::kept_limit(&relative, member.size) {
                return (entries, Some(error));
            }
            match session.read_body() {
                Ok(body) => body,
                Err(error) => return (entries, Some(error)),
            }
        } else {
            match session.skip_body() {
                Ok(()) => Vec::new(),
                Err(error) => return (entries, Some(error)),
            }
        };
        entries.push(OarEntry {
            path: member.path,
            kind: member.kind,
            use_entry,
            scene_id,
            relative,
            body,
        });
    }
}

fn report_of(loaded: LoadedControl, entries: Vec<OarEntry>) -> OarReport {
    let failed = loaded.error.is_some();
    OarReport {
        error: loaded.error,
        creation_time: loaded.creation_time,
        default_original_id: loaded.default_original_id,
        multi_region: loaded.multi_region,
        control_loaded: loaded.control_loaded,
        incoming_size_bits: loaded.incoming_size_bits,
        scene_count: i32::try_from(loaded.regions.len()).unwrap_or(i32::MAX),
        loaded_scenes: if failed { Vec::new() } else { loaded.loaded },
        regions: loaded.regions.into_iter().map(archived).collect(),
        entries,
    }
}

/// A control-file throw leaves the out arguments unset. The size already
/// written to the request is the one field that survives.
fn control_failure(error: String, incoming: [i32; 3]) -> OarReport {
    OarReport {
        error: Some(error),
        creation_time: 0,
        default_original_id: String::new(),
        multi_region: false,
        control_loaded: false,
        incoming_size_bits: incoming,
        scene_count: 0,
        loaded_scenes: Vec::new(),
        regions: Vec::new(),
        entries: Vec::new(),
    }
}

fn archived(region: control::RegionRow) -> ArchivedRegion {
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
    use std::io::{Cursor, Read};
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
            assert_eq!(report_value(&report), expected(case), "{name}");
            assert_event(case, &report, name);
        }
    }

    #[test]
    fn the_last_scene_registered_at_a_cell_wins() {
        let xml = b"<?xml version=\"1.0\" encoding=\"utf-16\"?>\n\
            <archive major_version=\"0\" minor_version=\"8\">\n\
            <creation_info><datetime>1700000000</datetime>\n\
            <id>11111111-1111-1111-1111-111111111111</id></creation_info>\n\
            <region_info><size_in_meters>256,256</size_in_meters></region_info>\n\
            </archive>";
        let bytes = tar_file("archive.xml", xml);
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let root = SimulatorRegion {
            id: second,
            loc_x: 4,
            loc_y: 5,
        };
        let scenes = [
            SimulatorRegion {
                id: first,
                loc_x: 4,
                loc_y: 5,
            },
            SimulatorRegion {
                id: second,
                loc_x: 4,
                loc_y: 5,
            },
        ];
        let report = load_oar(&root, &scenes, &bytes, OarSource::TarSeek);
        assert!(report.error.is_none());
        assert_eq!(report.loaded_scenes, vec![second]);
        assert_eq!(report.regions[0].scene_id, Some(second));
    }

    #[test]
    fn a_second_directory_loads_the_scene_once() {
        let xml = b"<archive major_version=\"1\" minor_version=\"0\">\n\
            <regions><row><region>\n\
            <id>11111111-1111-1111-1111-111111111111</id>\n\
            <dir>west</dir><dir>alias</dir>\n\
            <size_in_meters>256,256</size_in_meters>\n\
            </region></row></regions></archive>";
        let bytes = tar_file("archive.xml", xml);
        let scene = Uuid::from_u128(0xaaaa);
        let root = SimulatorRegion {
            id: scene,
            loc_x: 10,
            loc_y: 10,
        };
        let report = load_oar(&root, &[root], &bytes, OarSource::TarSeek);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.scene_count, 2);
        assert_eq!(report.regions[0].directory, "west");
        assert_eq!(report.regions[1].directory, "alias");
        assert_eq!(report.regions[0].scene_id, Some(scene));
        assert_eq!(report.regions[1].scene_id, Some(scene));
        assert_eq!(report.regions[0].original_id, report.regions[1].original_id);
        assert_eq!(report.loaded_scenes, vec![scene]);
    }

    #[test]
    fn a_nan_size_keeps_pin8s_sign_and_infinity_does_not_parse() {
        let nan = load_size("NaN,256");
        assert_eq!(nan.incoming_size_bits[0], -4_194_304);
        assert_eq!(nan.incoming_size_bits[1], 256.0f32.to_bits() as i32);
        assert_eq!(nan.incoming_size_bits[2], 0);
        let infinity = load_size("Infinity,256");
        assert_eq!(infinity.incoming_size_bits, control::default_size_bits());
        assert_eq!(infinity.regions[0].size_bits, control::default_size_bits());
    }

    fn load_size(text: &str) -> OarReport {
        let xml = format!(
            "<archive major_version=\"0\" minor_version=\"8\">\n\
             <size_in_meters>{text}</size_in_meters></archive>"
        );
        let bytes = tar_file("archive.xml", xml.as_bytes());
        let root = SimulatorRegion {
            id: Uuid::nil(),
            loc_x: 0,
            loc_y: 0,
        };
        load_oar(&root, &[root], &bytes, OarSource::TarSeek)
    }

    fn limit_hits(id: &str) -> u64 {
        let prefix = format!("ferroan_limit_hits_total{{limit=\"{id}\",action=\"refuse\"}} ");
        ferroan_diag::metrics()
            .lines()
            .find_map(|line| line.strip_prefix(&prefix)?.parse().ok())
            .unwrap_or(0)
    }

    fn header_only(path: &str, size: i32) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..path.len()].copy_from_slice(path.as_bytes());
        let field = format!("{size:011o}");
        header[124..135].copy_from_slice(field.as_bytes());
        header[156] = b'0';
        header
    }

    fn tar_member(path: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = header_only(path, i32::try_from(data.len()).unwrap());
        bytes.extend(data);
        if !data.len().is_multiple_of(512) {
            bytes.extend(vec![0u8; 512 - (data.len() % 512)]);
        }
        bytes
    }

    /// Reads the bytes it was given. The next read panics, so a cap that is
    /// checked after the body starts fails the test.
    struct PanicAfter {
        data: Vec<u8>,
        at: usize,
    }

    impl PanicAfter {
        fn new(data: Vec<u8>) -> Self {
            Self { data, at: 0 }
        }
    }

    impl Read for PanicAfter {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.at >= self.data.len() {
                panic!("read past the provided header");
            }
            let n = (self.data.len() - self.at).min(buf.len());
            buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    fn control_xml() -> &'static [u8] {
        b"<archive major_version=\"0\" minor_version=\"8\">\n\
          <id>11111111-1111-1111-1111-111111111111</id>\n\
          <size_in_meters>256,256</size_in_meters></archive>"
    }

    fn load_bytes<R, K>(bytes: R, keep: K) -> OarReport
    where
        R: Read,
        K: FnMut(&str, &str, i32) -> bool,
    {
        let root = SimulatorRegion {
            id: Uuid::nil(),
            loc_x: 0,
            loc_y: 0,
        };
        load_from(
            &root,
            &[root],
            tar::Session::new(bytes),
            false,
            || panic!("reopen"),
            keep,
        )
    }

    #[test]
    fn a_kept_member_past_its_class_cap_is_refused_before_the_body() {
        let mut bytes = tar_member("archive.xml", control_xml());
        let declared = i32::try_from(tar::MAX_TERRAIN_BYTES).unwrap() + 1;
        bytes.extend(header_only("terrains/r.r32", declared));
        let _guard = tar::LIMIT_TEST.lock().unwrap();
        let before = limit_hits(ferroan_diag::OAR_TERRAIN_BYTES);
        let report = load_bytes(PanicAfter::new(bytes), |path, _, _| {
            path.starts_with("terrains/")
        });
        let error = report.error.expect("class cap");
        assert!(error.contains(ferroan_diag::OAR_TERRAIN_BYTES), "{error}");
        assert!(!error.contains(ferroan_diag::OAR_MEMBER_BYTES), "{error}");
        assert_eq!(limit_hits(ferroan_diag::OAR_TERRAIN_BYTES), before + 1);
    }

    #[test]
    fn a_skipped_member_past_its_class_cap_is_not_a_class_hit() {
        let mut bytes = tar_member("archive.xml", control_xml());
        let declared = i32::try_from(tar::MAX_TERRAIN_BYTES).unwrap() + 1;
        bytes.extend(header_only("terrains/r.r32", declared));
        let _guard = tar::LIMIT_TEST.lock().unwrap();
        let before = limit_hits(ferroan_diag::OAR_TERRAIN_BYTES);
        let report = load_bytes(Cursor::new(bytes), |_, _, _| false);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(limit_hits(ferroan_diag::OAR_TERRAIN_BYTES), before);
        let terrain = report
            .entries
            .iter()
            .find(|entry| entry.path == "terrains/r.r32")
            .unwrap();
        assert!(terrain.body.is_empty());
    }

    #[test]
    fn member_bytes_is_checked_before_the_class_cap() {
        let mut bytes = tar_member("archive.xml", control_xml());
        let declared = i32::try_from(tar::MAX_MEMBER_BYTES).unwrap() + 1;
        bytes.extend(header_only("terrains/r.r32", declared));
        let _guard = tar::LIMIT_TEST.lock().unwrap();
        let before = limit_hits(ferroan_diag::OAR_MEMBER_BYTES);
        let class_before = limit_hits(ferroan_diag::OAR_TERRAIN_BYTES);
        let report = load_bytes(PanicAfter::new(bytes), |_, _, _| true);
        let error = report.error.expect("member cap");
        assert!(error.contains(ferroan_diag::OAR_MEMBER_BYTES), "{error}");
        assert!(!error.contains(ferroan_diag::OAR_TERRAIN_BYTES), "{error}");
        assert_eq!(limit_hits(ferroan_diag::OAR_MEMBER_BYTES), before + 1);
        assert_eq!(limit_hits(ferroan_diag::OAR_TERRAIN_BYTES), class_before);
    }

    #[test]
    fn a_control_file_past_its_cap_is_refused_before_the_body() {
        let declared = i32::try_from(tar::MAX_CONTROL_BYTES).unwrap() + 1;
        let _guard = tar::LIMIT_TEST.lock().unwrap();
        let before = limit_hits(ferroan_diag::OAR_CONTROL_BYTES);
        let report = load_bytes(
            PanicAfter::new(header_only("archive.xml", declared)),
            |_, _, _| true,
        );
        let error = report.error.expect("control cap");
        assert!(error.contains(ferroan_diag::OAR_CONTROL_BYTES), "{error}");
        assert_eq!(limit_hits(ferroan_diag::OAR_CONTROL_BYTES), before + 1);
    }

    #[test]
    fn a_keep_callback_stores_only_the_member_it_names() {
        let mut bytes = tar_member("archive.xml", control_xml());
        bytes.extend(tar_member("notes/a.txt", b"hello"));
        bytes.extend(tar_member("notes/b.txt", b"world"));
        let report = load_bytes(Cursor::new(bytes), |path, _, _| path.ends_with("b.txt"));
        assert!(report.error.is_none(), "{:?}", report.error);
        let body = |path: &str| {
            report
                .entries
                .iter()
                .find(|entry| entry.path == path)
                .unwrap()
                .body
                .clone()
        };
        assert!(body("notes/a.txt").is_empty());
        assert_eq!(body("notes/b.txt"), b"world");
    }

    fn tar_file(path: &str, data: &[u8]) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..path.len()].copy_from_slice(path.as_bytes());
        let size = format!("{:011o}", data.len());
        header[124..135].copy_from_slice(size.as_bytes());
        header[156] = b'0';
        header.extend(data);
        if !data.len().is_multiple_of(512) {
            header.extend(vec![0u8; 512 - (data.len() % 512)]);
        }
        header.extend(vec![0u8; 1024]);
        header
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
            other => panic!("unknown oar source {other}"),
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

    /// Pin8's record, adjusted where Ferroan deliberately differs.
    /// A throw carries the event's empty scene list (`OarFileLoaded`). Compact XML that
    /// skips the next element is loaded (D132). A culture-sensitive region
    /// prefix is compared ordinally (D133, TB196).
    fn expected(case: &Value) -> Value {
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
        if !case["error"].is_null() {
            object.insert("error".to_string(), case["dearchive_error"].clone());
            object.insert("loaded_scenes".to_string(), json!([]));
        }
        match case["name"].as_str() {
            Some("compact-skip") => {
                let id = json!("11111111-1111-1111-1111-111111111111");
                object.insert("default_original_id".to_string(), id.clone());
                object["regions"][0]["original_id"] = id;
            }
            Some("rv-compact-multi") => {
                object.insert("scene_count".to_string(), json!(3));
                object.insert(
                    "loaded_scenes".to_string(),
                    json!([
                        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
                    ]),
                );
                object.insert(
                    "regions".to_string(),
                    json!([
                        region_row(
                            "1_1_West",
                            "11111111-1111-1111-1111-111111111111",
                            0,
                            0,
                            256.0,
                            Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
                        ),
                        region_row(
                            "3_1_East_Isle",
                            "22222222-2222-2222-2222-222222222222",
                            2,
                            0,
                            512.0,
                            Some("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
                        ),
                        region_row(
                            "1_3_North",
                            "33333333-3333-3333-3333-333333333333",
                            0,
                            2,
                            256.0,
                            None
                        ),
                    ]),
                );
                object["entries"][1]["use"] = json!(true);
                object["entries"][1]["scene_id"] = json!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
            }
            Some("rv-two-dirs") => {
                let mut alias = object["regions"][0].clone();
                alias["directory"] = json!("alias");
                object["regions"].as_array_mut().unwrap().insert(1, alias);
                object.insert("scene_count".to_string(), json!(4));
                object["entries"][1]["use"] = json!(true);
                object["entries"][1]["scene_id"] = json!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
            }
            Some("rv-ignorable-path") => {
                let path = object["entries"][1]["path"].clone();
                object["entries"][1]["scene_id"] = Value::Null;
                object["entries"][1]["relative"] = path;
            }
            _ => {}
        }
        value
    }

    fn region_row(
        directory: &str,
        original_id: &str,
        loc_x: i32,
        loc_y: i32,
        metres: f32,
        scene_id: Option<&str>,
    ) -> Value {
        json!({
            "directory": directory,
            "original_id": original_id,
            "loc_x": loc_x,
            "loc_y": loc_y,
            "size_bits": [net_component(metres), net_component(metres), 0],
            "scene_id": scene_id,
        })
    }

    fn net_component(value: f32) -> i32 {
        value.to_bits() as i32
    }

    /// Every source fires the loaded event. A throw's scene list is empty.
    fn assert_event(case: &Value, report: &OarReport, name: &str) {
        assert!(case["dearchive_fired"].as_bool().unwrap(), "{name}");
        assert!(case["dearchive_thrown"].is_null(), "{name}");
        if case["error"].is_null() {
            return;
        }
        assert!(
            case["dearchive_scenes"].as_array().unwrap().is_empty(),
            "{name}"
        );
        assert!(report.loaded_scenes.is_empty(), "{name}");
        assert_eq!(
            report.error.as_deref(),
            case["dearchive_error"].as_str(),
            "{name}"
        );
    }
}
