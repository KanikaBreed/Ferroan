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
//! Test transcriptions, not shipped code: the thresholds of
//! `ScenePresence.Update` (`Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs`
//! lines 4056–4133) and the avatar block of `CreateAvatartImprovedTerseBlock`
//! (`Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs` lines
//! 7352–7411, the path `ProcessEntityUpdates` takes at line 5634) at
//! KanikaBreed/OpenSim-Tranquillity `c401bceecd`, with `Utils.FloatToUInt16`
//! and `Utils.FloatToUnitUInt16` from OpenSim-NGC/libopenmetaverse
//! `a3596616`. Their notices are in `lib.rs`.

use ferroan_capture::{Kind, Record, check_udp, read_capture};
use ferroan_protocol::msgs::{
    AgentMovementComplete, AgentUpdate, ImprovedTerseObjectUpdate, LayerData, ObjectUpdate,
};
use ferroan_protocol::{Msg, parse_packet};

use crate::frame::{FRAME_TIME, Presence};
use crate::height::Heightmap;
use crate::motion::{Body, CONTROL_AT_POS, MotionUpdate};
use crate::walk;

/// `ScenePresence.cs` lines 4056–4060 at pin `c401bceecd`.
const ROTATION_TOLERANCE: f32 = 0.01;
const VELOCITY_TOLERANCE: f32 = 0.1;
const LOW_VELOCITY_SQ: f32 = 0.1;
const POSITION_LARGE_TOLERANCE: f32 = 5.0;
const POSITION_SMALL_TOLERANCE: f32 = 0.05;

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
    local_id: u32,
    state: u8,
    plane: [f32; 4],
    position: [f32; 3],
    velocity: [u16; 3],
    acceleration: [u16; 3],
    rotation: [u16; 4],
    angular: [u16; 3],
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
        local_id: u32::from_le_bytes(data[0..4].try_into().unwrap()),
        state: data[4],
        plane: [
            read_f32(data, 6),
            read_f32(data, 10),
            read_f32(data, 14),
            read_f32(data, 18),
        ],
        position: vec(22),
        velocity: u3(34),
        acceleration: u3(40),
        rotation: [
            read_u16(data, 46),
            read_u16(data, 48),
            read_u16(data, 50),
            read_u16(data, 52),
        ],
        angular: u3(54),
    }
}

/// `Utils.FloatToUInt16(value, range)` in OpenMetaverse `a3596616`, as
/// `Vector3.ClampedToShortsBytes` calls it for a vector inside the range: the
/// product is a double, then truncated.
fn pack_u16(value: f32, range: f32) -> u16 {
    let value = value + range;
    if value <= 0.0 {
        return 0;
    }
    let value = value / range;
    if value >= 2.0 {
        return u16::MAX;
    }
    (f64::from(value) * 32767.5) as u16
}

/// `Utils.FloatToUnitUInt16`: a different rounding from `pack_u16`.
fn pack_unit(value: f32) -> u16 {
    if value <= -1.0 {
        return 0;
    }
    if value >= 1.0 {
        return u16::MAX;
    }
    (f64::from(value) * 32767.5 + 32767.0) as u16
}

/// The body rotation as `CreateAvatartImprovedTerseBlock` writes it for an
/// avatar that is not flying or sitting: X and Y are `0x7fff`, and Z and W
/// are scaled by `1 / sqrt(z * z + w * w)`.
fn pack_rotation(rotation: [f32; 4]) -> [u16; 4] {
    let (z, w) = (rotation[2], rotation[3]);
    let a = 1.0 / (z * z + w * w).sqrt();
    [0x7fff, 0x7fff, pack_unit(z * a), pack_unit(w * a)]
}

/// What one tick leaves for `ScenePresence.Update` to compare.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    tick: u64,
    position: [f32; 3],
    velocity: [f32; 3],
    rotation: [f32; 4],
}

/// The last values `SendTerseUpdateToAllClients` stored. State and the
/// collision plane do not change on this walk: BasicPhysics reports no
/// collision plane, and the block carries unit W.
#[derive(Clone, Copy, Debug)]
struct LastSent {
    position: [f32; 3],
    velocity: [f32; 3],
    rotation: [f32; 4],
}

impl LastSent {
    /// The fresh presence: every stored value is zero.
    fn new() -> Self {
        Self {
            position: [0.0; 3],
            velocity: [0.0; 3],
            rotation: [0.0; 4],
        }
    }

    /// `ScenePresence.Update` lines 4100–4130. Vector tolerances are per axis
    /// (`Vector3.ApproxEquals` and `ApproxZero`).
    fn due(&self, now: &Sample) -> bool {
        let turned = (0..3)
            .any(|axis| (now.rotation[axis] - self.rotation[axis]).abs() > ROTATION_TOLERANCE);
        if turned {
            return true;
        }
        let velocity_moved = (0..3)
            .any(|axis| (now.velocity[axis] - self.velocity[axis]).abs() > VELOCITY_TOLERANCE);
        let stopped = now.velocity == [0.0; 3] && self.velocity != [0.0; 3];
        if velocity_moved || stopped {
            return true;
        }
        let moved = |tolerance: f32| {
            (0..3).any(|axis| (now.position[axis] - self.position[axis]).abs() > tolerance)
        };
        let speed_sq = now.velocity.iter().map(|axis| axis * axis).sum::<f32>();
        moved(POSITION_LARGE_TOLERANCE)
            || (moved(POSITION_SMALL_TOLERANCE) && speed_sq < LOW_VELOCITY_SQ)
    }

    fn store(&mut self, now: &Sample) {
        self.position = now.position;
        self.velocity = now.velocity;
        self.rotation = now.rotation;
    }
}

struct Walk {
    local_id: u32,
    start: [f32; 3],
    start_rotation: [f32; 4],
    updates: Vec<MotionUpdate>,
    terse: Vec<Terse>,
    first_layer_ms: u64,
    last_ms: u64,
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

    let mut last = LastSent::new();
    let mut sends = Vec::new();
    for sample in samples.iter().filter(|sample| sample.tick >= gate) {
        if last.due(sample) {
            last.store(sample);
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
        assert_eq!(
            sent.velocity.map(|axis| pack_u16(axis, 128.0)),
            block.velocity
        );
        assert_eq!(pack_rotation(sent.rotation), block.rotation);
        assert_eq!(block.acceleration, [0x7fff; 3]);
        assert_eq!(block.angular, [pack_u16(0.0, 64.0); 3]);
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
