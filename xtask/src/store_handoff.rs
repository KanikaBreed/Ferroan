// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A viewer on each region implementation reads the same committed SQLite scene.
//! Pin8 c401bceecd: Scene.HandleEditCommand, SceneObjectPart.Resize and Copy,
//! SceneObjectGroup.ProcessBackup and SQLiteSimulationData.StoreObject.

use super::{Proc, RegionStart, RegionStore, login_capture, prim_capture};
use ferroan_bot::scenario::{HANDOFF_TASK_BPS, PRIM_CAPTURE_COUNT, PRIM_LOAD_COUNT};
use ferroan_capture::{Kind, Record};
use ferroan_protocol::{Msg, msgs::AgentThrottle, parse_packet};
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

fn roots(db: &Path, count: usize) -> Result<BTreeMap<Uuid, Uuid>, String> {
    let sql = format!(
        "SELECT substr(UUID,1,37),substr(SceneGroupID,1,37) FROM prims WHERE RegionUUID='{}' LIMIT {};",
        super::live::REGION_UUID,
        count + 1
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
    if roots.len() != count || !roots.iter().any(|(id, root)| id != root && *root == BOX) {
        return Err("handoff scene is missing its linked box or stored parts".into());
    }
    Ok(roots)
}

struct Receipt {
    first_ms: u64,
    first_resent: bool,
    first_seq: u32,
    first_fresh_repeat_seq: Option<u32>,
    fresh_repeats: usize,
    resent_repeats: usize,
}

struct Viewer {
    parts: Blocks,
    receipts: BTreeMap<Uuid, Receipt>,
}

impl Viewer {
    fn read(records: &[Record], count: usize) -> Result<Self, String> {
        if records.len() > ferroan_bot::session::LOAD_RECEIPT_LIMIT
            || records.windows(2).any(|p| p[1].time_ms < p[0].time_ms)
        {
            return Err("invalid viewer receipt window".into());
        }
        // Read the request that actually left this circuit. A different Task
        // rate anywhere in the window would make the timings incomparable.
        let mut throttle_ms = None;
        for record in records.iter().filter(|r| r.kind == Kind::UdpIn) {
            let packet = parse_packet(&record.bytes).map_err(|e| e.to_string())?;
            let request = packet
                .decode::<AgentThrottle>()
                .map_err(|e| e.to_string())?;
            if packet.id != AgentThrottle::ID
                || request.throttle.throttles.get(16..20) != Some(&HANDOFF_TASK_BPS.to_le_bytes())
            {
                return Err("unexpected viewer Task rate".into());
            }
            throttle_ms.get_or_insert(record.time_ms);
        }
        let mut receipts = BTreeMap::new();
        let parts = prim_capture::read_blocks(records, count, |id, at, packet| {
            receipts
                .entry(id)
                .and_modify(|r: &mut Receipt| {
                    if packet.resent() {
                        r.resent_repeats += 1;
                    } else {
                        r.fresh_repeats += 1;
                        r.first_fresh_repeat_seq.get_or_insert(packet.sequence);
                    }
                })
                .or_insert(Receipt {
                    first_ms: at,
                    first_resent: packet.resent(),
                    first_seq: packet.sequence,
                    first_fresh_repeat_seq: None,
                    fresh_repeats: 0,
                    resent_repeats: 0,
                });
        })?;
        if receipts.len() != count {
            return Err(format!(
                "expected {count} viewer receipts, found {}",
                receipts.len()
            ));
        }
        let first = receipts
            .values()
            .map(|r| r.first_ms)
            .min()
            .ok_or("no prim receipts")?;
        if throttle_ms.is_none_or(|at| at > first) {
            return Err("missing Task request before the first prim".into());
        }
        Ok(Self { parts, receipts })
    }

    fn report(&self, name: &str, root: &Path) -> Result<String, String> {
        use std::fmt::Write;
        let mut first = u64::MAX;
        let mut last = 0;
        let mut fresh_repeats = 0;
        let mut resent_repeats = 0;
        let mut rows = String::from(
            "store_uuid\tfirst_ms\tfirst_seq\tfirst_resent\tfresh_repeats\tresent_repeats\tfirst_fresh_repeat_seq\n",
        );
        for (id, receipt) in &self.receipts {
            first = first.min(receipt.first_ms);
            last = last.max(receipt.first_ms);
            fresh_repeats += receipt.fresh_repeats;
            resent_repeats += receipt.resent_repeats;
            writeln!(
                rows,
                "{id}\t{}\t{}\t{}\t{}\t{}\t{}",
                receipt.first_ms,
                receipt.first_seq,
                u8::from(receipt.first_resent),
                receipt.fresh_repeats,
                receipt.resent_repeats,
                receipt
                    .first_fresh_repeat_seq
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".into())
            )
            .map_err(|e| e.to_string())?;
        }
        fs::write(root.join(format!("prim-load-{name}.tsv")), rows).map_err(|e| e.to_string())?;
        let elapsed = last.checked_sub(first).ok_or("missing first delivery")?;
        Ok(format!(
            "{name}: {} first deliveries, {fresh_repeats} fresh repeats, {resent_repeats} RESENT repeats, {elapsed} ms first to last at {HANDOFF_TASK_BPS} bit/s",
            self.receipts.len()
        ))
    }
}

fn viewer(log: &Path, count: usize) -> Result<Viewer, String> {
    let bytes = login_capture::bounded(&log.with_extension("viewer.cap"), 9 * 1024 * 1024)?;
    let records =
        ferroan_capture::read_capture_limited(&bytes, ferroan_bot::session::LOAD_RECEIPT_LIMIT)
            .map_err(|e| e.to_string())?;
    Viewer::read(&records, count)
}

fn loss_counts(text: &str) -> Result<[u64; 2], String> {
    if !text
        .lines()
        .any(|line| line == "# TYPE ferroan_limit_hits_total counter")
    {
        return Err("missing limit counters".into());
    }
    // An untouched action has no sample. The family declaration above
    // distinguishes that case from a scrape of the wrong endpoint.
    let mut counts = [None; 2];
    for row in text.lines().filter_map(|line| {
        line.strip_prefix(r#"ferroan_limit_hits_total{limit="region.entity-updates",action=""#)
    }) {
        let (action, value) = row
            .split_once("\"} ")
            .ok_or("malformed entity queue counter")?;
        let slot = match action {
            "refuse" => &mut counts[0],
            "drop" => &mut counts[1],
            _ => continue,
        };
        let value = value.parse().map_err(|_| "invalid entity queue counter")?;
        if slot.replace(value).is_some() {
            return Err("duplicate entity queue counter".into());
        }
    }
    Ok(counts.map(Option::unwrap_or_default))
}

fn verify_losses(before: [u64; 2], after: [u64; 2]) -> Result<(), String> {
    if before != after {
        return Err(format!(
            "entity queue loss counters changed: {before:?} -> {after:?}"
        ));
    }
    Ok(())
}

fn losses(port: u16) -> Result<[u64; 2], String> {
    let (status, body) = super::http_get_url(&format!("http://127.0.0.1:{port}"), "/metrics")?;
    if status != 200 {
        return Err(format!("region metrics returned {status}"));
    }
    loss_counts(&body)
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
    let load = spec.store == RegionStore::LoadFixture;
    let (scenario, count) = if load {
        ("prim-load", PRIM_LOAD_COUNT)
    } else {
        ("store-handoff", PRIM_CAPTURE_COUNT)
    };
    let metrics = spec.metrics_port.take();
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
    let parents = roots(&db, count)?;
    let login = format!("http://{}:{robust_public}/", super::ROBUST_BIND);
    let private = format!("http://{}:{}", super::ADVERTISED, spec.private_proxy);
    let observe = |proc: &Proc, ferroan| {
        super::run_bot(
            scenario,
            &login,
            estate,
            chat,
            &proc.log,
            true,
            (&private, root, ferroan),
            None,
            None,
        )?;
        viewer(&proc.log, count)
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
    spec.metrics_port = metrics;
    let rust = start(&spec)?;
    let before = metrics.map(losses).transpose()?;
    let actual = observe(&rust, true)?;
    check_store()?;
    compare(&reference.parts, &actual.parts, &parents)?;
    if load {
        let after = losses(metrics.ok_or("prim-load requires region metrics")?)?;
        verify_losses(before.ok_or("missing pre-login loss counters")?, after)?;
        return Ok(format!(
            "prim-load: {count} mapped viewer full blocks match on the same committed SQLite store\n{}\n{}\nregion.entity-updates: 0 new refusals, 0 new drops (before and after: {}, {})",
            reference.report("Pin8", root)?,
            actual.report("Ferroan", root)?,
            after[0],
            after[1]
        ));
    }
    prim_capture::write_fixture(&root.join("store-handoff"), reference.parts, &parents)?;
    Ok(format!(
        "store-handoff: C# committed box scale {x}, {y}, {z}; C# home exited; same SQLite file\nstore-handoff: {} viewer full blocks match with mapped local and parent IDs",
        actual.parts.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_protocol::Reader;

    #[test]
    fn first_receipt_times_and_repeats_match_the_pin8_control_circuit() {
        let control = include_bytes!("../../fixtures/prim-arrival/control.cap");
        let records = ferroan_capture::read_capture(control).unwrap();
        let view = Viewer::read(&records, 8).unwrap();
        let receipt = &view.receipts[&Uuid::from_u128(1)];
        assert_eq!(
            (
                receipt.first_ms,
                receipt.fresh_repeats,
                receipt.resent_repeats
            ),
            (426, 1, 0)
        );
        assert_eq!(view.receipts[&Uuid::from_u128(2)].fresh_repeats, 0);
        let dir = std::env::temp_dir().join(format!("prim-receipts-{}", Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let line = view.report("Pin8", &dir).unwrap();
        let rows = fs::read_to_string(dir.join("prim-load-Pin8.tsv")).unwrap();
        let _ = fs::remove_dir_all(dir);
        assert_eq!(
            line,
            "Pin8: 8 first deliveries, 7 fresh repeats, 0 RESENT repeats, 15 ms first to last at 512000 bit/s"
        );
        assert_eq!(rows.lines().count(), 9);
        assert!(
            rows.lines()
                .any(|line| line == "00000000-0000-0000-0000-000000000001\t426\t32\t0\t1\t0\t35")
        );
        assert!(Viewer::read(&records, 7).is_err());
        assert!(Viewer::read(&records[1..], 8).is_err());
        let limited = include_bytes!("../../fixtures/prim-arrival/limited.cap");
        assert!(Viewer::read(&ferroan_capture::read_capture(limited).unwrap(), 8).is_err());
        assert!(ferroan_capture::read_capture_limited(control, records.len() - 1).is_err());
    }

    #[test]
    fn prim_load_receipts_distinguish_resent_delivery_from_a_fresh_repeat() {
        use ferroan_protocol::PacketFlags;

        let control = include_bytes!("../../fixtures/prim-arrival/control.cap");
        let mut records = ferroan_capture::read_capture(control).unwrap();
        // Drop the original seq 32. Its retransmission is the first delivery
        // of the linked box and child. Repeating that datagram is a RESENT
        // repeat for both. The later fresh seq 35 repeats only the box.
        let i = records
            .iter()
            .position(|r| r.kind == Kind::UdpOut && parse_packet(&r.bytes).unwrap().sequence == 32)
            .unwrap();
        records[i].bytes[0] |= PacketFlags::RESENT;
        records.insert(i + 1, records[i].clone());
        let view = Viewer::read(&records, 8).unwrap();
        let root = &view.receipts[&Uuid::from_u128(1)];
        assert!(root.first_resent);
        assert_eq!((root.fresh_repeats, root.resent_repeats), (1, 1));
        let child = &view.receipts[&Uuid::from_u128(2)];
        assert!(child.first_resent);
        assert_eq!((child.fresh_repeats, child.resent_repeats), (0, 1));
        assert!(!view.receipts[&Uuid::from_u128(7)].first_resent);
    }

    #[test]
    fn metrics_require_the_limit_family_and_detect_either_kind_of_queue_loss() {
        let family = "# TYPE ferroan_limit_hits_total counter\n";
        assert!(loss_counts("ferroan_region_registered 1").is_err());
        assert_eq!(loss_counts(family).unwrap(), [0, 0]);
        let sample = format!(
            "{family}ferroan_limit_hits_total{{limit=\"region.entity-updates\",action=\"refuse\"}} 4\nferroan_limit_hits_total{{limit=\"region.entity-updates\",action=\"drop\"}} 7\n"
        );
        let before = loss_counts(&sample).unwrap();
        assert_eq!(before, [4, 7]);
        assert!(verify_losses(before, before).is_ok());
        assert!(verify_losses(before, [5, 7]).is_err());
        assert!(verify_losses(before, [4, 8]).is_err());
        assert!(verify_losses(before, [0, 0]).is_err());
    }

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
