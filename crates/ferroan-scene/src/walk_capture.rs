// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replays `fixtures/walk/walk.cap`, a C# region on BasicPhysics, through the
//! walking stand-in and the terse-update thresholds of `ScenePresence.Update`.
//!
//! The capture is one `cargo xtask live run walk grid --physics basicphysics`
//! at pin `c401bceecd`. The region sends the walker three
//! `ImprovedTerseObjectUpdate` blocks: when it starts, after five metres on
//! X, and when the delayed stop lands. Each block carries the live position
//! and velocity of the tick that sent it.
//!
//! The C# frame clock is not in the capture. The replay puts tick `k` at
//! `t0 + (k - k0) * FrameTime`, where `t0` is the arrival of the first terse
//! block and `k0` is the stand-in tick whose position equals that block.
//! Every input and every other send must then fall on that clock.
//!
//! `ScenePresence.Update` returns early until `SendInitialData` has run
//! (`NeedInitialData`, lines 4067–4071 and 4266–4290). That frame returns
//! too. On a login `SendInitialData` does not send terrain (line 4338 needs
//! `TeleportFlags` 0): the `LayerData` burst comes from the terrain module's
//! every-fourth-frame check (`Scene.cs` 1768–1771) after `CompleteMovement`
//! queued it (line 2472). On this recording that check ran in the
//! `SendInitialData` frame (region log 23:26:36,005, first `LayerData`
//! 36,017), so the first `LayerData` stands in for that frame. A new
//! recording may not line up.
//!
//! Positions match bit for bit: BasicPhysics adds `velocity * timeStep` in
//! single precision, and the stand-in does the same sums in the same order.
//! Velocity and rotation are compared after C#'s 16-bit packing, which is all
//! the block carries. Times match within half a frame: the capture stores
//! arrival times, not frame starts, and this heartbeat ran about 90.4 ms
//! frames while each step is still `FrameTime` 0.0909 s.
//!
//! The thresholds and the block are the shipped ones in [`crate::terse`]:
//! each sent block is built with [`avatar_block`] from that tick's values
//! and must equal the captured 60 bytes, the local id included.

use ferroan_capture::{Kind, Record, check_udp, read_capture};
use ferroan_protocol::msgs::{
    AgentMovementComplete, AgentUpdate, AvatarAnimation, ImprovedTerseObjectUpdate, LayerData,
    ObjectUpdate,
};
use ferroan_protocol::{Msg, parse_packet};
use uuid::Uuid;

use crate::animation::{Movement, STAND, default_animation};
use crate::frame::{FRAME_TIME, Presence};
use crate::height::Heightmap;
use crate::motion::{Body, CONTROL_AT_POS, MotionUpdate};
use crate::terse::{AvatarTerse, LastSent, UNIT_W, avatar_block};
use crate::walk;

const PCODE_AVATAR: u8 = 47;
const FRAME_MS: f64 = FRAME_TIME as f64 * 1000.0;
/// The terrain blob in the fixture is `TerrainData` revision 23 (gzip).
const TERRAIN_REVISION: i32 = 23;
const REGION_EDGE: i32 = 256;

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/walk")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// One decoded avatar terse block (`CreateAvatartImprovedTerseBlock`, 60 bytes).
#[derive(Clone, Copy, Debug)]
struct Terse {
    time_ms: u64,
    bytes: [u8; 60],
    local_id: u32,
    state: u8,
    plane: [f32; 4],
    position: [f32; 3],
    acceleration: [u16; 3],
}

fn read_f32(data: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

fn read_u16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(data[at..at + 2].try_into().unwrap())
}

fn terse(time_ms: u64, data: &[u8]) -> Terse {
    assert_eq!(data.len(), 60, "an avatar terse block is 60 bytes");
    assert_eq!(data[5], 1, "the walker's block is an avatar block");
    let vec = |at: usize| {
        [
            read_f32(data, at),
            read_f32(data, at + 4),
            read_f32(data, at + 8),
        ]
    };
    let u3 = |at: usize| {
        [
            read_u16(data, at),
            read_u16(data, at + 2),
            read_u16(data, at + 4),
        ]
    };
    Terse {
        time_ms,
        bytes: data.try_into().unwrap(),
        local_id: u32::from_le_bytes(data[0..4].try_into().unwrap()),
        state: data[4],
        plane: [
            read_f32(data, 6),
            read_f32(data, 10),
            read_f32(data, 14),
            read_f32(data, 18),
        ],
        position: vec(22),
        acceleration: u3(40),
    }
}

/// What one tick leaves for `ScenePresence.Update` to compare.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    tick: u64,
    position: [f32; 3],
    velocity: [f32; 3],
    rotation: [f32; 4],
}

impl Sample {
    /// The walker as the frame reads it: state 0, no collision plane, not
    /// flying, no angular velocity on this walk.
    fn terse(&self, local_id: u32) -> AvatarTerse {
        AvatarTerse {
            local_id,
            state: 0,
            collision_plane: UNIT_W,
            position: self.position,
            velocity: self.velocity,
            rotation: self.rotation,
            angular_velocity: [0.0; 3],
            full_rotation: false,
        }
    }
}

struct Walk {
    local_id: u32,
    start: [f32; 3],
    start_rotation: [f32; 4],
    updates: Vec<MotionUpdate>,
    terse: Vec<Terse>,
    animations: Vec<Animations>,
    first_layer_ms: u64,
    last_ms: u64,
}

/// One `AvatarAnimation` to the walker.
#[derive(Clone, Debug)]
struct Animations {
    time_ms: u64,
    sender: Uuid,
    list: Vec<(Uuid, i32)>,
    sources: usize,
}

fn motion_update(record: &Record, sequence: u32, message: &AgentUpdate) -> MotionUpdate {
    let data = &message.agent_data;
    MotionUpdate {
        now_ms: record.time_ms,
        sequence,
        child: false,
        in_transit: false,
        body_rotation: data.body_rotation.to_array(),
        head_rotation: data.head_rotation.to_array(),
        state: data.state,
        camera_center: data.camera_center.to_array(),
        camera_at: data.camera_at_axis.to_array(),
        camera_left: data.camera_left_axis.to_array(),
        camera_up: data.camera_up_axis.to_array(),
        far: data.far,
        control_flags: data.control_flags,
        flags: data.flags,
    }
}

fn read_walk() -> Walk {
    let records = read_capture(&fixture("walk.cap")).unwrap();
    // Every committed datagram decodes and encodes again.
    check_udp(&records).unwrap();
    let mut local_id = None;
    let mut start = None;
    let mut start_rotation = None;
    let mut arrival = None;
    let mut updates = Vec::new();
    let mut terse_blocks = Vec::new();
    let mut animations = Vec::new();
    let mut first_layer_ms = None;
    for record in &records {
        let Ok(packet) = parse_packet(&record.bytes) else {
            continue;
        };
        match record.kind {
            Kind::UdpIn if packet.id == AgentUpdate::ID => {
                let message = packet.decode::<AgentUpdate>().unwrap();
                updates.push(motion_update(record, packet.sequence, &message));
            }
            Kind::UdpOut if packet.id == AgentMovementComplete::ID => {
                let message = packet.decode::<AgentMovementComplete>().unwrap();
                arrival = Some(message.data.position.to_array());
            }
            Kind::UdpOut if packet.id == ObjectUpdate::ID => {
                let message = packet.decode::<ObjectUpdate>().unwrap();
                for block in &message.object_data {
                    if block.p_code == PCODE_AVATAR {
                        let data = &block.object_data;
                        assert_eq!(data.len(), 76, "the full avatar block");
                        local_id = Some(block.id);
                        start = Some([read_f32(data, 16), read_f32(data, 20), read_f32(data, 24)]);
                        // The full block packs X, Y and Z; W is the positive root.
                        let (x, y, z) =
                            (read_f32(data, 52), read_f32(data, 56), read_f32(data, 60));
                        start_rotation = Some([x, y, z, (1.0 - x * x - y * y - z * z).sqrt()]);
                    }
                }
            }
            Kind::UdpOut if packet.id == ImprovedTerseObjectUpdate::ID => {
                let message = packet.decode::<ImprovedTerseObjectUpdate>().unwrap();
                for block in &message.object_data {
                    terse_blocks.push(terse(record.time_ms, &block.data));
                }
            }
            Kind::UdpOut if packet.id == LayerData::ID => {
                first_layer_ms.get_or_insert(record.time_ms);
            }
            Kind::UdpOut if packet.id == AvatarAnimation::ID => {
                let message = packet.decode::<AvatarAnimation>().unwrap();
                animations.push(Animations {
                    time_ms: record.time_ms,
                    sender: message.sender.id,
                    list: message
                        .animation_list
                        .iter()
                        .map(|anim| (anim.anim_id, anim.anim_sequence_id))
                        .collect(),
                    sources: message.animation_source_list.len(),
                });
            }
            _ => {}
        }
    }
    let start = start.expect("the walker's full ObjectUpdate");
    assert_eq!(
        arrival,
        Some(start),
        "AgentMovementComplete is the same pose"
    );
    Walk {
        local_id: local_id.unwrap(),
        start,
        start_rotation: start_rotation.unwrap(),
        updates,
        terse: terse_blocks,
        animations,
        first_layer_ms: first_layer_ms.expect("the LayerData burst"),
        last_ms: records.last().unwrap().time_ms,
    }
}

fn terrain() -> Heightmap {
    Heightmap::from_blob(
        TERRAIN_REVISION,
        &fixture("terrain.bin"),
        REGION_EDGE,
        REGION_EDGE,
    )
    .unwrap()
}

fn login_pose(walk: &Walk) -> Presence {
    Presence {
        position: walk.start,
        rotation: walk.start_rotation,
        velocity: [0.0; 3],
        control_flags: 0,
        draw_distance: 128.0,
    }
}

/// One frame of the stand-in, as `step_avatars` runs it.
fn tick(body: &mut Body, pose: &mut Presence, map: &Heightmap, now_ms: u64) {
    let edge = REGION_EDGE as f32;
    let terrain = walk::terrain_height(Some(map), pose.position, edge, edge);
    body.step_physics(pose, terrain, edge, edge, FRAME_TIME, now_ms);
}

/// Without a clock: the first forward update is applied before tick 1 and
/// held. Returns the tick whose X and Y equal the first terse block.
fn first_send_tick(walk: &Walk, map: &Heightmap) -> u64 {
    let mut pose = login_pose(walk);
    let mut body = Body::new(&pose);
    let forward = walk.updates[0];
    assert_eq!(forward.control_flags, CONTROL_AT_POS);
    body.apply(&forward, &mut pose);
    let target = walk.terse[0].position;
    let mut found = None;
    for k in 1..=64u64 {
        tick(&mut body, &mut pose, map, 0);
        if pose.position[0] == target[0] && pose.position[1] == target[1] {
            assert!(found.is_none(), "one tick matches");
            found = Some(k);
        }
    }
    found.expect("a tick at the first terse position")
}

#[test]
fn the_csharp_walk_matches_the_stand_in_and_the_terse_thresholds() {
    let walk = read_walk();
    let map = terrain();
    assert_eq!(walk.terse.len(), 3, "start, five metres, stop");
    // 24 held-forward updates, one every 100 ms, then the release.
    assert_eq!(walk.updates.len(), 25);
    assert!(
        walk.updates[..24]
            .iter()
            .all(|update| update.control_flags == CONTROL_AT_POS)
    );
    assert_eq!(walk.updates[24].control_flags, 0);
    assert!(
        walk.terse
            .iter()
            .all(|block| block.local_id == walk.local_id)
    );

    let k0 = first_send_tick(&walk, &map);
    assert_eq!(k0, 4);
    let t0 = walk.terse[0].time_ms as f64;
    let clock = |k: u64| t0 + (k as f64 - k0 as f64) * FRAME_MS;

    // The first forward update lands between ticks 0 and 1.
    let first_forward = walk.updates[0].now_ms as f64;
    assert!(clock(0) < first_forward && first_forward < clock(1));

    // Replay every input on that clock, up to the logout. An update that
    // arrived before a frame's time is applied before that frame's step.
    let end = walk.last_ms as f64;
    let mut pose = login_pose(&walk);
    let mut body = Body::new(&pose);
    let mut pending = walk.updates.iter().peekable();
    let mut samples = Vec::new();
    let mut k = 1u64;
    while clock(k) <= end {
        let now = clock(k);
        while let Some(update) = pending.next_if(|update| (update.now_ms as f64) < now) {
            body.apply(update, &mut pose);
        }
        tick(&mut body, &mut pose, &map, now as u64);
        samples.push(Sample {
            tick: k,
            position: pose.position,
            velocity: pose.velocity,
            rotation: pose.rotation,
        });
        k += 1;
    }

    // `NeedInitialData`: the `SendInitialData` frame returns, and the
    // threshold test runs from the next frame on. The first LayerData is
    // only a marker: on this recording the terrain check shared that frame.
    let gate = (1..k)
        .find(|&k| clock(k - 1) >= walk.first_layer_ms as f64)
        .unwrap();
    assert_eq!(gate, k0, "the first frame past SendInitialData sends");

    let mut last = LastSent::default();
    let mut sends = Vec::new();
    for sample in samples.iter().filter(|sample| sample.tick >= gate) {
        let now = sample.terse(walk.local_id);
        if last.due(&now) {
            last.store(&now);
            sends.push(*sample);
        }
    }

    // Start, the first X more than 5 m past the start send, and the stop.
    let ticks: Vec<u64> = sends.iter().map(|sample| sample.tick).collect();
    assert_eq!(ticks, [k0, k0 + 23, k0 + 27]);
    assert_eq!(sends.len(), walk.terse.len());
    for (sent, block) in sends.iter().zip(&walk.terse) {
        // Each block arrives within half a frame of its tick on the clock.
        let offset = block.time_ms as f64 - clock(sent.tick);
        assert!(
            offset.abs() < FRAME_MS / 2.0,
            "tick {}: {offset} ms",
            sent.tick
        );
        for axis in 0..3 {
            assert_eq!(
                sent.position[axis].to_bits(),
                block.position[axis].to_bits(),
                "tick {} axis {axis}: {:?} {:?}",
                sent.tick,
                sent.position,
                block.position
            );
        }
        // The shipped block is the captured one, byte for byte.
        assert_eq!(
            avatar_block(&sent.terse(walk.local_id)),
            block.bytes,
            "tick {}",
            sent.tick
        );
        assert_eq!(block.acceleration, [0x7fff; 3]);
        assert_eq!(block.state, 0);
        assert_eq!(block.plane, [0.0, 0.0, 0.0, 1.0]);
    }
    // The delayed stop lands on the last send's tick. The avatar then rests
    // where that block put it, and nothing else is due before the logout.
    let stop = sends[2];
    assert_eq!(stop.velocity, [0.0; 3]);
    assert!(
        samples
            .iter()
            .filter(|sample| sample.tick >= stop.tick)
            .all(|sample| sample.position == stop.position && sample.velocity == [0.0; 3])
    );
    let release = walk
        .updates
        .iter()
        .find(|update| update.control_flags == 0)
        .unwrap();
    let deadline = (release.now_ms + 250) as f64;
    assert!(clock(stop.tick - 1) <= deadline && deadline < clock(stop.tick));
}

/// The animator on the same walk. The region sent the walker three
/// `AvatarAnimation` packets: `STAND` sequence 1 from `CompleteMovement`,
/// `WALK` about 10 ms after the first forward `AgentUpdate` (well before the
/// next frame: `AddNewMovement` runs the animator on the packet), and `STAND`
/// on the next sequence number after the release, on a frame. The release
/// takes the 250 ms path (the avatar is on the surface), so only
/// `RegionHeartbeatEnd` can stand it, and only on every second heartbeat.
/// `CompleteMovement` subscribed before tick 0 (its `STAND` packet came
/// first), so the animator runs at the end of ticks 1, 3, 5 and so on: the
/// release lands between ticks 27 and 28, tick 28 skips, and tick 29 stands.
/// The actor stepped at tick 0, so it was on the ground for the `WALK`.
///
/// The sequence numbers start from a clock in C#, so only the step between
/// them is compared. The packet itself is P2.3b2.
#[test]
fn the_csharp_walk_animates_walk_then_stand_on_the_heartbeat() {
    let walk = read_walk();
    let map = terrain();
    let walk_id = default_animation("WALK").unwrap();

    // What the capture holds.
    assert_eq!(walk.animations.len(), 3, "arrival, walk, stand");
    let agent = walk.animations[0].sender;
    assert!(walk.animations.iter().all(|anims| anims.sender == agent));
    assert!(walk.animations.iter().all(|anims| anims.sources == 0));
    assert_eq!(walk.animations[0].list, [(STAND, 1)]);
    let [(walked, walk_seq)] = walk.animations[1].list[..] else {
        panic!("one animation: {:?}", walk.animations[1].list)
    };
    assert_eq!(walked, walk_id);
    assert_eq!(walk.animations[2].list, [(STAND, walk_seq + 1)]);

    let t0 = walk.terse[0].time_ms as f64;
    let clock = |k: u64| t0 + (k as f64 - 4.0) * FRAME_MS;
    let forward = walk.updates[0];
    let release = walk.updates[24];
    // CompleteMovement's pack came before tick 0, the forward update after it.
    assert!((walk.animations[0].time_ms as f64) < clock(0));
    assert!(clock(0) < forward.now_ms as f64 && (forward.now_ms as f64) < clock(1));

    // The replay: the arrival's actor before tick 0, every input on the
    // clock, each tick's step, then the heartbeat's end.
    let mut pose = login_pose(&walk);
    let mut body = Body::new(&pose);
    body.set_agent(agent);
    body.new_actor();
    let initial = body.animator().animations().arrays();
    assert_eq!(initial.len(), 1);
    assert_eq!((initial[0].animation, initial[0].sequence), (STAND, 1));

    let edge = REGION_EDGE as f32;
    let mut pending = walk.updates.iter().peekable();
    let mut changes = Vec::new();
    let mut k = 0u64;
    while clock(k) <= walk.last_ms as f64 {
        let now = clock(k);
        while let Some(update) = pending.next_if(|update| (update.now_ms as f64) < now) {
            body.apply(update, &mut pose);
            if body.take_animation_pack() {
                changes.push((update.now_ms as f64, None, body.animator().current()));
            }
        }
        let terrain = walk::terrain_height(Some(&map), pose.position, edge, edge);
        body.step_physics(&mut pose, terrain, edge, edge, FRAME_TIME, now as u64);
        body.heartbeat(now as u64);
        if body.take_animation_pack() {
            changes.push((now, Some(k), body.animator().current()));
        }
        k += 1;
    }

    assert_eq!(changes.len(), 2, "{changes:?}");
    // WALK on the first forward update, not on a frame.
    assert_eq!(changes[0], (forward.now_ms as f64, None, Movement::Walk));
    let walk_sent = walk.animations[1].time_ms as f64;
    assert!(walk_sent >= forward.now_ms as f64 && walk_sent < forward.now_ms as f64 + 20.0);
    // STAND on tick 29, the second heartbeat after the release.
    assert_eq!(changes[1].1, Some(29), "{changes:?}");
    assert_eq!(changes[1].2, Movement::Stand);
    assert!(clock(27) < release.now_ms as f64 && (release.now_ms as f64) < clock(28));
    let stand_offset = walk.animations[2].time_ms as f64 - clock(29);
    assert!(stand_offset.abs() < FRAME_MS / 2.0, "{stand_offset} ms");

    // The set the pack would carry: the walk, then the stand, one number on,
    // with the agent as the object.
    let set = body.animator().animations();
    assert_eq!(set.default_animation().animation, STAND);
    assert_eq!(set.default_animation().object_id, agent);
    assert!(set.others().is_empty());
}

/// The `LayerData` burst C# sent from its generated `pinhead-island` holds
/// the same 256 patches, bit for bit, as Ferroan's land layers for the
/// island it generates for a store with no `terrain` row. C# packs several
/// patches into one packet in view-distance order (`SendTerrainUpdatesByViewDistance`),
/// and Ferroan sends one patch per packet in the spiral, so each captured
/// body is split into the patches it carries: the same layer header, then
/// patches that each equal one of Ferroan's, then the end marker.
#[test]
fn the_csharp_layer_burst_is_the_generated_island() {
    let records = read_capture(&fixture("walk.cap")).unwrap();
    let island = crate::initial::InitialTerrain::PinheadIsland
        .generate(REGION_EDGE, REGION_EDGE)
        .unwrap();
    let ours = crate::compress::land_patch_bits(&island);
    let layers = crate::compress::land_layers(&island);
    let header = bits_of(&layers[0].data[..4]);
    let end: Vec<bool> = (0..8).map(|bit| 97u8 & (0x80 >> bit) != 0).collect();
    let mut seen = std::collections::HashSet::new();
    let mut packets = 0;
    for record in records.iter().filter(|record| record.kind == Kind::UdpOut) {
        let Ok(packet) = parse_packet(&record.bytes) else {
            continue;
        };
        if packet.id != LayerData::ID {
            continue;
        }
        packets += 1;
        let message = packet.decode::<LayerData>().unwrap();
        assert_eq!(message.layer_id.type_, layers[0].kind);
        let bits = bits_of(&message.layer_data.data);
        assert_eq!(bits[..32], header[..], "LayerData {packets} header");
        let mut at = 32;
        loop {
            let found = ours.iter().find(|(patch, patch_bits)| {
                !seen.contains(patch) && bits.get(at..at + patch_bits.len()) == Some(patch_bits)
            });
            match found {
                Some((patch, patch_bits)) => {
                    seen.insert(*patch);
                    at += patch_bits.len();
                }
                None => {
                    assert_eq!(
                        bits.get(at..at + 8),
                        Some(&end[..]),
                        "LayerData {packets}: bit {at} is neither one of Ferroan's patches nor the end"
                    );
                    break;
                }
            }
        }
        // What is left is the byte's padding.
        assert!(
            bits.len() - (at + 8) < 8,
            "LayerData {packets} trailing bytes"
        );
    }
    assert_eq!(packets, 12);
    assert_eq!(seen.len(), 256, "every patch of the island, once");
}

fn bits_of(bytes: &[u8]) -> Vec<bool> {
    bytes
        .iter()
        .flat_map(|byte| (0..8).map(move |bit| byte & (0x80 >> bit) != 0))
        .collect()
}
