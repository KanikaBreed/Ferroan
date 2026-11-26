// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A viewer on each region implementation reads the same committed SQLite scene.
//! Pin8 c401bceecd: Scene.HandleEditCommand, SceneObjectPart.Resize and Copy,
//! SceneObjectGroup.ProcessBackup and SQLiteSimulationData.StoreObject.

use super::{Proc, RegionStart, RegionStore, login_capture, prim_capture};
use ferroan_bot::scenario::PRIM_CAPTURE_COUNT;
use ferroan_protocol::{Writer, msgs::object_update::ObjectData};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

const BOX: Uuid = Uuid::from_u128(0x65000000_0000_4000_8000_000000000001);
const SCALE: [f32; 3] = [1.25, 1.5, 0.875];
type Blocks = BTreeMap<Uuid, ObjectData>;

fn command(proc: &mut Proc, command: &str) -> Result<(), String> {
    let input = proc.stdin.as_mut().ok_or("home console missing")?;
    writeln!(input, "{command}")
        .and_then(|()| input.flush())
        .map_err(|e| e.to_string())
}

fn shutdown(mut proc: Proc) -> Result<(), String> {
    command(&mut proc, "quit")?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = proc.child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("C# home exited with {status}"))
            };
        }
        if Instant::now() >= deadline {
            return Err("C# home did not exit after quit".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn query(db: &Path, sql: &str) -> Result<String, String> {
    let output = Command::new("sqlite3")
        .args(["-batch", "-readonly", "-cmd", ".timeout 2000"])
        .arg(db)
        .arg(sql)
        .output()
        .map_err(|e| format!("read committed scene: {e}"))?;
    if !output.status.success() {
        return Err("could not read the committed region store".into());
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

fn edited(db: &Path) -> Result<bool, String> {
    let [x, y, z] = SCALE;
    let sql = format!(
        "SELECT EXISTS(SELECT 1 FROM primshapes WHERE UUID='{BOX}' AND ScaleX={x} AND ScaleY={y} AND ScaleZ={z});"
    );
    match query(db, &sql)?.trim() {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err("unexpected scale query result".into()),
    }
}

fn roots(db: &Path) -> Result<BTreeMap<Uuid, Uuid>, String> {
    let sql = format!(
        "SELECT substr(UUID,1,37),substr(SceneGroupID,1,37) FROM prims WHERE RegionUUID='{}' LIMIT {};",
        super::live::REGION_UUID,
        PRIM_CAPTURE_COUNT + 1
    );
    let mut roots = BTreeMap::new();
    for line in query(db, &sql)?.lines() {
        let (id, root) = line.split_once('|').ok_or("invalid stored linkset")?;
        if roots
            .insert(
                id.parse().map_err(|_| "invalid part UUID")?,
                root.parse().map_err(|_| "invalid root UUID")?,
            )
            .is_some()
        {
            return Err("duplicate stored UUID".into());
        }
    }
    if roots.len() != PRIM_CAPTURE_COUNT
        || !roots.iter().any(|(id, root)| id != root && *root == BOX)
    {
        return Err("handoff scene is missing its linked box or stored parts".into());
    }
    Ok(roots)
}

fn viewer(log: &Path) -> Result<Blocks, String> {
    let bytes = login_capture::bounded(&log.with_extension("viewer.cap"), 2 * 1024 * 1024)?;
    let records = ferroan_capture::read_capture(&bytes).map_err(|e| e.to_string())?;
    prim_capture::captured_blocks(&records)
}

fn compare(
    reference: &Blocks,
    actual: &Blocks,
    roots: &BTreeMap<Uuid, Uuid>,
) -> Result<(), String> {
    prim_capture::validate_ids(reference, roots)?;
    prim_capture::validate_ids(actual, roots)?;
    if reference
        .get(&BOX)
        .is_none_or(|part| part.scale.to_array() != SCALE)
    {
        return Err("C# viewer did not receive the committed box scale".into());
    }
    for (uuid, expected) in reference {
        let mut got = actual[uuid].clone();
        got.id = expected.id;
        got.parent_id = expected.parent_id;
        let mut left = Writer::new();
        let mut right = Writer::new();
        expected.encode(&mut left);
        got.encode(&mut right);
        if left.buf != right.buf {
            let at = left
                .buf
                .iter()
                .zip(&right.buf)
                .position(|(a, b)| a != b)
                .unwrap_or(left.buf.len().min(right.buf.len()));
            return Err(format!(
                "part {uuid} differs at block byte {at} (C# {} bytes, Ferroan {} bytes)",
                left.buf.len(),
                right.buf.len()
            ));
        }
    }
    Ok(())
}

pub(super) fn run(
    mut spec: RegionStart,
    root: &Path,
    robust_public: u16,
    estate: &str,
    chat: &str,
    passwords: &[String],
) -> Result<String, String> {
    let home = root.join("home");
    let db = home.join("OpenSim.db");
    let start = |spec: &RegionStart| {
        super::start_region(
            &home,
            &super::live::publish_dir(),
            &super::live::source_dir(),
            estate,
            spec,
            passwords,
            false,
        )
    };
    let mut editor = start(&spec)?;
    // Holding the original inode open also prevents its identity being reused after unlink.
    let original = fs::File::open(&db).map_err(|e| e.to_string())?;
    let identity = original.metadata().map_err(|e| e.to_string())?;
    let check_store = || {
        let now = fs::metadata(&db).map_err(|e| e.to_string())?;
        if (identity.dev(), identity.ino()) != (now.dev(), now.ino()) || !edited(&db)? {
            return Err("handed-off database was replaced or lost the committed edit".to_string());
        }
        Ok(())
    };
    if edited(&db)? {
        return Err("seed already contains the handoff edit".into());
    }
    let [x, y, z] = SCALE;
    command(&mut editor, &format!("edit scale box {x} {y} {z}"))?;
    command(&mut editor, "backup")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !edited(&db)? {
        if Instant::now() >= deadline {
            return Err("C# backup did not commit the edited box scale".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    shutdown(editor)?;
    check_store()?;
    let parents = roots(&db)?;
    let login = format!("http://{}:{robust_public}/", super::ROBUST_BIND);
    let private = format!("http://{}:{}", super::ADVERTISED, spec.private_proxy);
    let observe = |proc: &Proc, ferroan| {
        super::run_bot(
            "store-handoff",
            &login,
            estate,
            chat,
            &proc.log,
            true,
            (&private, root, ferroan),
            None,
        )?;
        viewer(&proc.log)
    };
    // Copy(false) in ProcessBackup assigns new cache CRCs. Observe the
    // committed CRCs on a cold C# load before handing the file to Ferroan.
    spec.store = RegionStore::Existing;
    spec.capture = Some(root.join("capture-reference.cap"));
    let csharp = start(&spec)?;
    let reference = observe(&csharp, false)?;
    shutdown(csharp)?;
    check_store()?;
    spec.ferroan = true;
    spec.basic_physics = false;
    spec.login_only = true;
    spec.capture = Some(root.join("capture-ferroan.cap"));
    let rust = start(&spec)?;
    let actual = observe(&rust, true)?;
    check_store()?;
    compare(&reference, &actual, &parents)?;
    prim_capture::write_fixture(&root.join("store-handoff"), reference, &parents)?;
    Ok(format!(
        "store-handoff: C# committed box scale {x}, {y}, {z}; C# home exited; same SQLite file\nstore-handoff: {} viewer full blocks match with mapped local and parent IDs",
        actual.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_protocol::Reader;

    // The checked-in blocks are the raw C# block fields with Q65a's aliases.
    fn captured_linkset() -> (Blocks, BTreeMap<Uuid, Uuid>) {
        let base = crate::repo_root().join("fixtures/store-handoff");
        let child = Uuid::from_u128(BOX.as_u128() + 1);
        let mut blocks = Blocks::new();
        for (id, alias) in [(BOX, 1), (child, 2)] {
            let bytes = fs::read(base.join(format!("part-{alias}.bin"))).unwrap();
            let mut reader = Reader::new(&bytes);
            let mut block = ObjectData::decode(&mut reader).unwrap();
            assert_eq!(reader.remaining(), 0);
            assert_eq!(block.full_id, Uuid::from_u128(alias));
            block.full_id = id;
            blocks.insert(id, block);
        }
        assert_ne!(blocks[&BOX].texture_entry, blocks[&child].texture_entry);
        (blocks, BTreeMap::from([(BOX, BOX), (child, BOX)]))
    }

    #[test]
    fn captured_linkset_changes_only_the_committed_scale_and_cache_crcs() {
        let fixtures = crate::repo_root().join("fixtures");
        // Independent read of prims.pseudocrc after the handoff. Attaching
        // the restored box increments its CRC once; the mesh keeps its value.
        for (alias, stored_crc) in [(1, 1_645_670_632), (2, 1_645_675_865)] {
            let file = format!("part-{alias}.bin");
            let before = fs::read(fixtures.join("prim-updates").join(&file)).unwrap();
            let mut expected = ObjectData::decode(&mut Reader::new(&before)).unwrap();
            expected.crc = stored_crc;
            if alias == 1 {
                expected.scale = SCALE.into();
                expected.crc += 1;
            }
            let mut writer = Writer::new();
            expected.encode(&mut writer);
            assert_eq!(
                writer.buf,
                fs::read(fixtures.join("store-handoff").join(file)).unwrap()
            );
        }
    }

    #[test]
    fn committed_linkset_matches_with_independent_runtime_ids() {
        let (reference, parents) = captured_linkset();
        let mut actual = reference.clone();
        for block in actual.values_mut() {
            block.id += 400;
            if block.parent_id != 0 {
                block.parent_id += 400;
            }
        }
        compare(&reference, &actual, &parents).unwrap();
    }

    #[test]
    fn a_missing_child_wrong_parent_changed_texture_or_stale_scale_fails_the_handoff() {
        let (reference, parents) = captured_linkset();
        let child = Uuid::from_u128(BOX.as_u128() + 1);
        let mut actual = reference.clone();
        actual.remove(&child);
        assert!(compare(&reference, &actual, &parents).is_err());
        let mut actual = reference.clone();
        actual.get_mut(&child).unwrap().parent_id = 0;
        assert!(compare(&reference, &actual, &parents).is_err());
        let mut actual = reference.clone();
        actual.get_mut(&child).unwrap().texture_entry[0] ^= 1;
        assert!(compare(&reference, &actual, &parents).is_err());
        let mut stale = reference.clone();
        stale.get_mut(&BOX).unwrap().scale.x = 0.5;
        assert!(compare(&reference, &stale, &parents).is_err());
        assert!(compare(&stale, &stale, &parents).is_err());
    }
}
