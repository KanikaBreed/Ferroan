// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Full ObjectData blocks from Pin8 LLClientView.CreatePrimUpdateBlock.
//! Keep the stored fields and verify runtime identities before redacting them.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use ferroan_bot::scenario::PRIM_CAPTURE_COUNT;
use ferroan_capture::{Kind, Record, read_capture};
use ferroan_protocol::msgs::{
    ObjectUpdate,
    object_update::{ObjectData, RegionData},
};
use ferroan_protocol::{Msg, PacketFlags, Reader, Writer, parse_packet};
use ferroan_scene::{ObjectStoreKind, open_objects};
use serde_json::json;
use uuid::Uuid;

fn captured_blocks(records: &[Record]) -> Result<BTreeMap<Uuid, ObjectData>, String> {
    let mut parts = BTreeMap::new();
    for record in records.iter().filter(|r| r.kind == Kind::UdpOut) {
        let packet = parse_packet(&record.bytes).map_err(|e| e.to_string())?;
        if packet.id != ObjectUpdate::ID {
            continue;
        }
        let mut reader = Reader::new(&packet.body);
        RegionData::decode(&mut reader).map_err(|e| e.to_string())?;
        let count = reader.u8().map_err(|e| e.to_string())?;
        for _ in 0..count {
            let start = packet.body.len() - reader.remaining();
            let block = ObjectData::decode(&mut reader).map_err(|e| e.to_string())?;
            if block.p_code == 47 {
                continue;
            }
            if !packet.reliable() || packet.flags & PacketFlags::ZEROCODED == 0 {
                return Err("full prim update must be reliable and zero-coded".into());
            }
            let mut writer = Writer::new();
            block.encode(&mut writer);
            if writer.buf != packet.body[start..packet.body.len() - reader.remaining()] {
                return Err("decoded prim does not reproduce the C# block bytes".into());
            }
            let id = block.full_id;
            if let Some(previous) = parts.get(&id) {
                if previous != &block {
                    return Err(format!("different initial blocks for {id}"));
                }
            } else {
                if parts.len() == PRIM_CAPTURE_COUNT {
                    return Err("capture contains unexpected prims".into());
                }
                parts.insert(id, block);
            }
        }
        if reader.remaining() != 0 {
            return Err("trailing ObjectUpdate bytes".into());
        }
    }
    Ok(parts)
}

async fn stored_roots(db: &Path) -> Result<BTreeMap<Uuid, Uuid>, String> {
    let region = crate::live::REGION_UUID
        .parse()
        .map_err(|_| "invalid harness region")?;
    let groups = open_objects(
        &format!("Data Source={};Version=3", db.display()),
        region,
        ObjectStoreKind::Sqlite,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut roots = BTreeMap::new();
    for group in groups {
        let root = group.parts.first().ok_or("empty linkset")?.uuid;
        for part in group.parts {
            roots.insert(part.uuid, root);
        }
    }
    Ok(roots)
}

fn validate_ids(
    parts: &BTreeMap<Uuid, ObjectData>,
    roots: &BTreeMap<Uuid, Uuid>,
) -> Result<(), String> {
    if roots.is_empty() || !parts.keys().eq(roots.keys()) {
        return Err("captured prim UUIDs differ from the stored prims".into());
    }
    let mut ids = BTreeSet::new();
    for (uuid, part) in parts {
        if part.id == 0 || !ids.insert(part.id) {
            return Err(format!("zero or repeated local ID for {uuid}"));
        }
        let root = &roots[uuid];
        let parent_id = if root == uuid {
            0
        } else {
            parts.get(root).ok_or("missing root prim")?.id
        };
        if part.parent_id != parent_id {
            return Err(format!("incorrect parent local ID for {uuid}"));
        }
    }
    Ok(())
}

pub(super) fn extract(run: &Path) -> Result<String, String> {
    let bytes = super::login_capture::bounded(&run.join("capture-udp.cap"), 64 * 1024 * 1024)?;
    let records = read_capture(&bytes).map_err(|e| e.to_string())?;
    let parts = captured_blocks(&records)?;
    let roots = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(stored_roots(&run.join("home/OpenSim.db")))?;
    if roots.len() != PRIM_CAPTURE_COUNT {
        return Err(format!(
            "expected {PRIM_CAPTURE_COUNT} stored parts, found {}",
            roots.len()
        ));
    }
    validate_ids(&parts, &roots)?;
    let dir = run.join("prim-updates");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let aliases: BTreeMap<_, _> = parts
        .keys()
        .enumerate()
        .map(|(i, id)| (*id, i as u32 + 1))
        .collect();
    let owner: Uuid = crate::live::ESTATE_OWNER
        .parse()
        .map_err(|_| "invalid harness owner")?;
    let mut map = Vec::new();
    for (uuid, mut part) in parts {
        part.id = aliases[&uuid];
        part.full_id = Uuid::from_u128(u128::from(part.id));
        if part.parent_id != 0 {
            part.parent_id = aliases[&roots[&uuid]];
        }
        if part.owner_id == owner {
            part.owner_id = Uuid::from_u128(100);
        } else if !part.owner_id.is_nil() {
            return Err("unexpected owner in the prim capture".into());
        }
        let file = format!("part-{}.bin", part.id);
        let mut writer = Writer::new();
        part.encode(&mut writer);
        fs::write(dir.join(&file), writer.buf).map_err(|e| e.to_string())?;
        map.push(json!({
            "store_uuid": uuid.to_string(),
            "store_root_uuid": roots[&uuid].to_string(),
            "capture_uuid": part.full_id.to_string(),
            "block": file,
        }));
    }
    let map = json!({
        "owner": {"store_uuid": owner.to_string(), "capture_uuid": Uuid::from_u128(100).to_string()},
        "parts": map,
    });
    let json = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    fs::write(dir.join("map.json"), format!("{json}\n")).map_err(|e| e.to_string())?;
    Ok(format!(
        "prim-capture: {} C# blocks, unique local IDs and matching parent links; fixture in {}",
        roots.len(),
        dir.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StoreDir(std::path::PathBuf);

    impl Drop for StoreDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn pin8_full_blocks_match_the_persisted_prim_identities_and_payloads() {
        let fixture = crate::repo_root().join("fixtures/prim-updates");
        let temp = StoreDir(std::env::temp_dir().join(format!("prim-updates-{}", Uuid::new_v4())));
        fs::create_dir(&temp.0).unwrap();
        let db = temp.0.join("scene.db");
        fs::copy(fixture.join("scene.db"), &db).unwrap();
        let roots = stored_roots(&db).await.unwrap();
        let map: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture.join("map.json")).unwrap()).unwrap();
        assert_eq!(map["owner"]["store_uuid"], crate::live::ESTATE_OWNER);
        assert_eq!(
            map["owner"]["capture_uuid"],
            Uuid::from_u128(100).to_string()
        );
        let map = map["parts"].as_array().unwrap();
        assert_eq!(map.len(), PRIM_CAPTURE_COUNT);
        let mut parts = BTreeMap::new();
        let mut capture_uuids = BTreeSet::new();
        for entry in map {
            let id: Uuid = entry["store_uuid"].as_str().unwrap().parse().unwrap();
            let bytes = fs::read(fixture.join(entry["block"].as_str().unwrap())).unwrap();
            let mut reader = Reader::new(&bytes);
            let block = ObjectData::decode(&mut reader).unwrap();
            assert_eq!(reader.remaining(), 0);
            assert_eq!(entry["store_root_uuid"], roots[&id].to_string());
            assert_eq!(entry["capture_uuid"], block.full_id.to_string());
            assert!(capture_uuids.insert(block.full_id));
            assert_eq!(block.object_data.len(), 60);
            assert!(parts.insert(id, block).is_none());
        }
        validate_ids(&parts, &roots).unwrap();
        let blocks = parts.values().collect::<Vec<_>>();
        let (root, mesh, tree) = (blocks[0], blocks[1], blocks[2]);
        assert_eq!((root.p_code, mesh.p_code, tree.p_code), (9, 9, 255));
        assert_eq!(
            (root.parent_id, mesh.parent_id, tree.parent_id),
            (0, root.id, 0)
        );
        assert_eq!((root.crc, mesh.crc, tree.crc), (6502, 6502, 6504));
        assert_eq!(ferroan_protocol::field_str(&root.text), "Pin8 box");
        assert_eq!(
            ferroan_protocol::field_str(&root.media_url),
            "https://example.invalid/prim"
        );
        assert_eq!(root.texture_anim.len(), 16);
        assert!(!root.ps_block.is_empty());
        assert!(!root.sound.is_nil());
        assert_eq!(root.owner_id, Uuid::from_u128(100));
        assert!(mesh.owner_id.is_nil());
        assert_eq!(
            (mesh.profile_begin, mesh.profile_hollow, mesh.path_scale_y),
            (9375, 27500, 150)
        );
        assert_eq!(tree.data, [4]);
        let (grass, new_tree) = (blocks[3], blocks[4]);
        assert_eq!((grass.p_code, new_tree.p_code), (95, 111));
        assert_eq!(grass.data, [3]);
        assert_eq!(new_tree.data, [3]);
        assert_eq!(grass.object_data[12..], [0; 48]);
        assert_ne!(new_tree.object_data[36..48], [0; 12]);
        // The face's legacy material and the mesh's glTF material survive the store and the wire.
        for (bytes, suffix) in [(&root.texture_entry, 22), (&mesh.extra_params, 26)] {
            let id = format!("65000000-0000-4000-8000-{suffix:012x}")
                .parse::<Uuid>()
                .unwrap();
            assert!(bytes.windows(16).any(|window| window == id.as_bytes()));
        }
    }

    #[test]
    fn repeated_local_ids_and_wrong_link_parents_are_refused_before_masking() {
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let roots = BTreeMap::from([(a, a), (b, a), (c, c)]);
        let block = |id, parent_id| ObjectData {
            id,
            parent_id,
            ..Default::default()
        };
        let mut parts = BTreeMap::from([(a, block(71, 0)), (b, block(72, 71)), (c, block(73, 0))]);
        validate_ids(&parts, &roots).unwrap();
        parts.insert(c, block(72, 0));
        assert!(
            validate_ids(&parts, &roots)
                .unwrap_err()
                .contains("repeated local ID")
        );
        parts.insert(c, block(73, 0));
        parts.insert(b, block(72, 73));
        assert!(
            validate_ids(&parts, &roots)
                .unwrap_err()
                .contains("parent local ID")
        );
    }
}
