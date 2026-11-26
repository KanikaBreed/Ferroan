// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::entity_queue::{FULL_UPDATE, FULL_UPDATE_WITH_ANIM, MATERIAL_OVR};
use crate::prim_update::{LinksetUpdateFlags, StoredPrimUpdate};
use crate::terse::ViewerUpdate;
use crate::test_store::CopiedStore;
use crate::{ObjectGroup, ObjectStoreKind, PrimScaleLimits, open_objects};
use ferroan_capture::{Kind, read_capture};
use ferroan_db::{Backend, Pool};
use ferroan_protocol::{Msg, Writer, msgs::ObjectUpdate, parse_packet};
use std::collections::{BTreeMap, HashSet};

const REGION: Uuid = Uuid::from_u128(0x6d1a7f4e_0c3b_4f53_9a1e_5f0e3c2a9b01);
const MAP: &str = include_str!("../../../../../fixtures/prim-updates/map.json");

async fn fixture_groups() -> Vec<ObjectGroup> {
    let copy = CopiedStore::new("prim-updates/scene.db");
    let connection = copy.connection();
    let pool = Pool::connect(Backend::Sqlite, &connection).await.unwrap();
    sqlx::raw_sql("CREATE TEMP TABLE foreign_prim AS SELECT * FROM prims WHERE UUID=SceneGroupID LIMIT 1;
        UPDATE foreign_prim SET UUID='91000000-0000-4000-8000-000000000009',
        SceneGroupID='91000000-0000-4000-8000-000000000009', RegionUUID='91000000-0000-4000-8000-000000000099';
        INSERT INTO prims SELECT * FROM foreign_prim;")
        .execute(pool.sqlite().unwrap()).await.unwrap();
    pool.close().await;
    let foreign = Uuid::from_u128(0x91000000_0000_4000_8000_000000000099);
    assert_eq!(
        open_objects(&connection, foreign, ObjectStoreKind::Sqlite)
            .await
            .unwrap()
            .len(),
        1
    );
    open_objects(&connection, REGION, ObjectStoreKind::Sqlite)
        .await
        .unwrap()
}

#[derive(Default)]
struct Delivery {
    // Capture and frame IDs come from different counters. Resolve parent IDs
    // on each circuit through roots already delivered on that same circuit.
    local: BTreeMap<u32, Uuid>,
    parents: BTreeMap<Uuid, Option<Uuid>>,
}

impl Delivery {
    fn offsets_by_alias(&self) -> BTreeMap<Uuid, u32> {
        let (&first, _) = self.local.first_key_value().unwrap();
        self.local
            .iter()
            .map(|(&id, &alias)| (alias, id - first))
            .collect()
    }

    fn record(&mut self, alias: Uuid, id: u32, parent: u32) {
        assert_ne!(id, 0);
        assert!(self.local.insert(id, alias).is_none(), "local id reused");
        let parent = (parent != 0).then(|| self.local[&parent]);
        assert_ne!(parent, Some(alias));
        assert!(self.parents.insert(alias, parent).is_none());
    }
}

// First-delivery packet sizes and identities only. Pin8 can interleave
// unrelated parts differently between runs and repeat previously sent parts.
fn captured(bytes: &[u8]) -> (Delivery, Vec<usize>) {
    let mut scene = Delivery::default();
    let mut batches = Vec::new();
    for record in read_capture(bytes).unwrap() {
        let packet = parse_packet(&record.bytes).unwrap();
        if record.kind != Kind::UdpOut || packet.id != ObjectUpdate::ID {
            continue;
        }
        let mut count = 0;
        for part in packet.decode::<ObjectUpdate>().unwrap().object_data {
            if part.p_code == 47 || scene.parents.contains_key(&part.full_id) {
                continue;
            }
            scene.record(part.full_id, part.id, part.parent_id);
            count += 1;
        }
        if count != 0 {
            batches.push(count);
        }
    }
    (scene, batches)
}

fn batch(feed: &ViewerFeed, count: usize, aliases: &BTreeMap<Uuid, Uuid>, seen: &mut Delivery) {
    let mut delivered = 0;
    while delivered < count {
        let next = feed.take_update().expect("missing queued prim");
        let ViewerUpdate::Full {
            part,
            update,
            queued,
            ..
        } = next
        else {
            continue;
        };
        delivered += 1;
        let block = update.unwrap().block_for(Uuid::nil());
        assert_eq!(
            (block.full_id, block.id, block.parent_id),
            (part.uuid, part.local_id, part.parent_id)
        );
        assert_eq!(queued.local_id, block.id);
        seen.record(aliases[&part.uuid], block.id, block.parent_id);
    }
}

#[test]
fn two_arriving_feeds_match_the_pin8_part_and_parent_identities() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let groups = rt.block_on(fixture_groups());
    let map: serde_json::Value = serde_json::from_str(MAP).unwrap();
    let aliases = map["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["store_uuid"].as_str().unwrap().parse().unwrap(),
                p["capture_uuid"].as_str().unwrap().parse().unwrap(),
            )
        })
        .collect();
    let (control, control_batches) = captured(include_bytes!(
        "../../../../../fixtures/prim-arrival/control.cap"
    ));
    let (limited, limited_batches) = captured(include_bytes!(
        "../../../../../fixtures/prim-arrival/limited.cap"
    ));
    assert_eq!(control_batches, [6, 2]);
    assert_eq!(limited_batches, [1; 8]);
    let frame = TerrainFrame::with_objects(None, Arc::new(groups), None).unwrap();
    let walker = Uuid::from_u128(100);
    let watcher = Uuid::from_u128(101);
    let (walker_id, left, left_seeds) = attach(&frame, walker);
    let (watcher_id, right, right_seeds) = attach(&frame, watcher);
    for agent in [walker, watcher] {
        login(&frame, agent, pose_at([128.0, 128.0, 25.0]));
        reuse(&frame, agent);
        frame.handshake_reply(agent).unwrap();
    }
    wait_frames(&frame, 5);
    assert!(!left.has_updates() && !right.has_updates());
    left_seeds.store(true, Ordering::Release);
    wait_frames(&frame, 7);
    assert_eq!(local_ids(&right.take_terse(0)), [walker_id]);
    assert!(
        !right.has_updates(),
        "prim delivery before this viewer's seeds"
    );
    right_seeds.store(true, Ordering::Release);
    wait_frames(&frame, 7);
    left.take_terse(0);
    right.take_terse(0);
    assert!(left.has_updates() && !left.has_avatar_terse());

    // Two independently paced consumers pause with full updates still queued.
    let mut left_seen = Delivery::default();
    let mut right_seen = Delivery::default();
    batch(&left, control_batches[0], &aliases, &mut left_seen);
    batch(&right, limited_batches[0], &aliases, &mut right_seen);
    frame
        .agent_update(
            walker,
            walking_update(clock_ms(Instant::now()), 2, CONTROL_AT_POS),
        )
        .unwrap();
    wait_frames(&frame, 2);
    for feed in [&left, &right] {
        let Some(ViewerUpdate::Terse(block)) = feed.take_update() else {
            panic!("missing walker")
        };
        assert_eq!(local_ids(&[block]), [walker_id]);
        assert_ne!(&block[34..36], &[0xff, 0x7f]);
    }
    batch(&left, control_batches[1], &aliases, &mut left_seen);
    for count in &limited_batches[1..] {
        batch(&right, *count, &aliases, &mut right_seen);
    }
    assert_eq!(left_seen.parents, control.parents);
    assert_eq!(right_seen.parents, limited.parents);
    assert_eq!(left_seen.offsets_by_alias(), control.offsets_by_alias());
    assert_eq!(right_seen.offsets_by_alias(), limited.offsets_by_alias());
    assert_eq!(left_seen.local, right_seen.local);
    assert!(
        !left_seen.local.contains_key(&walker_id) && !left_seen.local.contains_key(&watcher_id)
    );
    assert_eq!(walker_id, left_seen.local.last_key_value().unwrap().0 + 1);
    assert_eq!(watcher_id, walker_id + 1);
    // Further walker movement can produce terses but never repeats initial admission.
    for feed in [&left, &right] {
        while let Some(update) = feed.take_update() {
            assert!(matches!(update, ViewerUpdate::Terse(_)));
        }
    }
}

#[tokio::test]
async fn admission_clamps_each_part_and_offers_children_in_link_order() {
    let mut group = fixture_groups()
        .await
        .into_iter()
        .find(|g| g.parts.len() == 2)
        .unwrap();
    let mut third = group.parts[1].clone();
    third.uuid = Uuid::from_u128(3);
    third.link_num = 3;
    group.parts.push(third);
    for part in &mut group.parts {
        part.shape.scale = [0.0, 1.0, 100.0];
        part.shape.materials = Some(crate::RenderMaterials {
            overrides: Some(vec![crate::MaterialOverride {
                te_index: 0,
                data: "{}".into(),
            }]),
        });
    }
    // Only the first child has an animation collection. Even an empty
    // collection survives SendUpdateToClient's null check.
    group.parts[1].animations = Some(Vec::new());
    // The root requests mesh animations. Only the second child has a reflection probe.
    group.parts[0].shape.extra_params = vec![1, 0x70, 0, 4, 0, 0, 0, 1, 0, 0, 0];
    group.parts[2].shape.extra_params = vec![1, 0x90, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let viewer = PriorityViewer {
        agent_id: Uuid::nil(),
        position: Some(group.parts[0].group_position),
        sitting_on: None,
    };
    for limits in [
        None,
        Some(PrimScaleLimits {
            min: 0.001,
            max: 32.0,
        }),
    ] {
        let frame =
            TerrainFrame::with_objects(None, Arc::new(vec![group.clone()]), limits).unwrap();
        let feed = frame.new_feed();
        feed.offer_stored_parts(&viewer);
        let mut ids = HashSet::new();
        let mut root = 0;
        for link in 1..=3 {
            let Some(ViewerUpdate::Full {
                queued,
                part,
                update,
                ..
            }) = feed.take_update()
            else {
                panic!("missing part")
            };
            let block = update.unwrap().block_for(viewer.agent_id);
            assert_eq!(part.link_num, link);
            assert_eq!(block.parent_id, root);
            assert!(ids.insert(block.id));
            if link == 1 {
                root = block.id;
            } else {
                assert_eq!(block.id, root + (link as u32 - 1));
            }
            assert_eq!(
                part.shape.scale,
                if limits.is_some() {
                    [0.001, 1.0, 32.0]
                } else {
                    // ClampScale still applies the default nonphysical minimum.
                    [0.001, 1.0, 100.0]
                }
            );
            assert_eq!(block.scale.to_array(), part.shape.scale);
            assert_eq!(queued.priority, 4);
            let full = if link == 2 {
                FULL_UPDATE_WITH_ANIM
            } else {
                FULL_UPDATE
            };
            assert_eq!(
                queued.flags,
                full | if link == 3 { 0 } else { MATERIAL_OVR }
            );
        }
        assert!(!feed.has_updates());
        let far = frame.new_feed();
        let mut far_viewer = viewer.clone();
        let mut position = group.parts[0].group_position;
        position[0] += 100.0;
        far_viewer.position = Some(position);
        far.offer_stored_parts(&far_viewer);
        for _ in 0..3 {
            let Some(ViewerUpdate::Full { queued, .. }) = far.take_update() else {
                panic!("missing distant prim")
            };
            assert_eq!(queued.priority, 7);
        }
    }
}

#[tokio::test]
async fn stable_link_order_assigns_ids_and_queues_parts_without_reordering_the_store() {
    let mut group = fixture_groups()
        .await
        .into_iter()
        .find(|g| g.parts.len() == 2)
        .unwrap();
    let mut third = group.parts[1].clone();
    third.uuid = Uuid::from_u128(3);
    third.link_num = 3;
    let mut fourth = group.parts[1].clone();
    fourth.uuid = Uuid::from_u128(4);
    group.parts.extend([third, fourth]);
    let expected = [0, 1, 3, 2].map(|i| group.parts[i].uuid);
    for root_link in [1, 7] {
        group.parts[0].link_num = root_link;
        let frame = TerrainFrame::with_objects(None, Arc::new(vec![group.clone()]), None).unwrap();
        let stored = frame.stored_groups();
        assert!(
            stored[0]
                .parts
                .iter()
                .map(|p| p.uuid)
                .eq(group.parts.iter().map(|p| p.uuid))
        );
        let root_id = stored[0].parts[0].local_id;
        let feed = frame.new_feed();
        feed.offer_stored_parts(&PriorityViewer {
            agent_id: Uuid::nil(),
            position: Some(group.parts[0].group_position),
            sitting_on: None,
        });
        for (offset, uuid) in expected.iter().enumerate() {
            let Some(ViewerUpdate::Full { part, .. }) = feed.take_update() else {
                panic!("missing linked part")
            };
            assert_eq!((part.uuid, part.local_id), (*uuid, root_id + offset as u32));
        }
        assert!(feed.take_update().is_none());
    }
}

#[tokio::test]
async fn priority_follows_the_stored_roots_physics_actor() {
    let template = fixture_groups()
        .await
        .into_iter()
        .find(|g| g.parts.len() == 2)
        .unwrap();
    let viewer = PriorityViewer {
        agent_id: Uuid::nil(),
        position: Some(template.parts[0].group_position),
        sitting_on: None,
    };
    let mut groups = Vec::new();
    for (i, (physical, path_curve, state, pcode)) in [
        (true, 16, 0, 9),
        (false, 16, 0, 9),
        (true, 128, 0, 9),
        (true, 16, 1, 9),
        (true, 16, 1, 95),
    ]
    .into_iter()
    .enumerate()
    {
        let mut group = template.clone();
        group.parts[0].uuid = Uuid::from_u128(2 * i as u128 + 1);
        group.parts[1].uuid = Uuid::from_u128(2 * i as u128 + 2);
        group.parts[0].flags = u32::from(physical);
        group.parts[1].flags = u32::from(!physical);
        group.parts[0].shape.path_curve = path_curve;
        group.parts[0].shape.state = state;
        group.parts[0].shape.pcode = pcode;
        groups.push(group);
    }
    for collidable in [true, false] {
        let frame = TerrainFrame::with_object_settings(
            None,
            None,
            Arc::new(groups.clone()),
            None,
            PhysicsScaleLimits::default(),
            collidable,
        )
        .unwrap();
        let feed = frame.new_feed();
        feed.offer_stored_parts(&viewer);
        let mut priorities = BTreeMap::new();
        while let Some(ViewerUpdate::Full { part, queued, .. }) = feed.take_update() {
            priorities.insert(part.uuid, queued.priority);
        }
        assert_eq!(priorities.len(), 10);
        for id in 1..=10 {
            let physical = collidable && matches!(id, 1 | 2 | 9 | 10);
            assert_eq!(
                priorities[&Uuid::from_u128(id)],
                if physical { 3 } else { 4 }
            );
        }
    }
}

#[tokio::test]
async fn a_stored_zero_scale_is_sent_as_pin8_bytes() {
    let template = fixture_groups()
        .await
        .into_iter()
        .find(|group| group.parts[0].name == "box")
        .unwrap()
        .parts
        .remove(0);
    let rows = pin8_scale_rows();
    assert!(rows.iter().any(|row| row.name == "zero-nonphys"));
    let mut groups = Vec::new();
    for row in &rows {
        if groups
            .last()
            .is_some_and(|(name, _): &(_, Vec<_>)| name == &row.name)
        {
            groups.last_mut().unwrap().1.push(row);
        } else {
            groups.push((row.name.clone(), vec![row]));
        }
    }
    for (name, rows) in groups {
        let physics = rows[0].physics;
        let clamp = rows[0].clamp.then_some(PrimScaleLimits {
            min: physics.nonphys_min,
            max: physics.nonphys_max,
        });
        let mut group = ObjectGroup {
            from_item_id: Uuid::nil(),
            rez_parameter: None,
            parts: Vec::new(),
        };
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.index, index, "{name}");
            assert_eq!(row.physics, physics, "{name}");
            assert_eq!(row.clamp, clamp.is_some(), "{name}");
            assert_eq!(row.collidable, rows[0].collidable, "{name}");
            let mut part = template.clone();
            part.uuid = Uuid::from_u128((index + 1) as u128);
            part.link_num = index as i32;
            part.flags = row.flags;
            part.volume_detect = row.volume;
            part.physics_shape_type = row.shape;
            part.shape.path_curve = row.path;
            part.shape.state = row.state;
            part.shape.pcode = row.pcode;
            part.shape.scale = row.scale;
            group.parts.push(part);
        }
        let frame = TerrainFrame::with_object_settings(
            None,
            None,
            Arc::new(vec![group]),
            clamp,
            physics,
            rows[0].collidable,
        )
        .unwrap();
        let stored = frame.stored_groups();
        let (root, children) = stored[0].parts.split_first().unwrap();
        let linkset = LinksetUpdateFlags::from_parts(root, children);
        for (row, part) in rows.iter().zip(&stored[0].parts) {
            let mut stored_scale = [0; 12];
            for (slot, axis) in stored_scale
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(part.shape.scale)
            {
                slot.copy_from_slice(&axis.to_le_bytes());
            }
            assert_eq!(stored_scale, row.out, "{name} part {}", row.index);
            let block = StoredPrimUpdate::from_store(part, &linkset, part.local_id, root.local_id)
                .unwrap()
                .block_for(part.owner_id);
            let mut writer = Writer::new();
            block.encode(&mut writer);
            assert_eq!(
                &writer.buf[28..40],
                &row.out[..],
                "{name} part {} wire scale",
                row.index
            );
        }
    }
}

struct ScaleRow {
    name: String,
    index: usize,
    flags: u32,
    path: u8,
    state: u8,
    pcode: u8,
    shape: u8,
    volume: bool,
    clamp: bool,
    collidable: bool,
    physics: PhysicsScaleLimits,
    scale: [f32; 3],
    out: [u8; 12],
}

fn pin8_scale_rows() -> Vec<ScaleRow> {
    let text = include_str!("../../../../../fixtures/prim-scale/pin8-scales.txt");
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ScaleRow::parse)
        .collect()
}

impl ScaleRow {
    fn parse(line: &str) -> Self {
        let field: Vec<_> = line.split_whitespace().collect();
        assert_eq!(field.len(), 18, "{line}");
        let float = |index| hex_f32(field[index]);
        Self {
            name: field[0].to_owned(),
            index: field[1].parse().unwrap(),
            flags: field[2].parse().unwrap(),
            path: field[3].parse().unwrap(),
            state: field[4].parse().unwrap(),
            pcode: field[5].parse().unwrap(),
            shape: field[6].parse().unwrap(),
            volume: field[7] == "1",
            clamp: field[8] == "1",
            collidable: field[9] == "1",
            physics: PhysicsScaleLimits {
                nonphys_min: float(10),
                nonphys_max: float(11),
                phys_min: float(12),
                phys_max: float(13),
            },
            scale: [float(14), float(15), float(16)],
            out: hex_bytes(field[17]).try_into().unwrap(),
        }
    }
}

fn hex_f32(text: &str) -> f32 {
    let bytes: [u8; 4] = hex_bytes(text).try_into().unwrap();
    f32::from_le_bytes(bytes)
}

fn hex_bytes(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}
