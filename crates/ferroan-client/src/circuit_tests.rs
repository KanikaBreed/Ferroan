// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Circuit send-path regressions against the pinned C# socket capture.

use super::*;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use ferroan_protocol::{build_packet, parse_packet};

const IMMEDIATE_ACK: &[u8] = include_bytes!("../../../fixtures/circuit/packet-ack.bin");
const ESTABLISHED_ACK: &[u8] =
    include_bytes!("../../../fixtures/circuit/packet-ack-established.bin");
const HANDSHAKE: &[u8] = include_bytes!("../../../fixtures/circuit/region-handshake.bin");
const LOGOUT: &[u8] = include_bytes!("../../../fixtures/circuit/logout-reply.bin");
const ARRIVAL: &[u8] = include_bytes!("../../../fixtures/arrival/agent-movement-complete.bin");

fn movement(expect: CircuitExpect, sequence: u32, acks: &[u32]) -> Vec<u8> {
    let mut message = CompleteAgentMovement::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    message.agent_data.circuit_code = expect.circuit_code;
    wire(&message, sequence, acks)
}

#[tokio::test]
async fn land_packets_drain_while_the_reliable_window_has_room() {
    let config = ArrivalConfig {
        land: vec![
            crate::LandLayer {
                kind: 0x4c,
                data: b"patch-a".to_vec(),
            },
            crate::LandLayer {
                kind: 0x4c,
                data: b"patch-b".to_vec(),
            },
        ],
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    let io = FakeIo::default();
    state.maintain(&io).await.unwrap();
    assert!(
        state.pending_land.is_empty(),
        "terrain waits for CompleteAgentMovement"
    );
    let expect = identity(Uuid::from_u128(1));
    let request = movement(expect, 1, &[]);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&request).unwrap(),
            request.len(),
        )
        .await
        .unwrap();
    release_throttled(&mut state, &io).await;
    let sent = io.sent.lock().unwrap();
    let bodies: Vec<(u8, Vec<u8>)> = sent
        .iter()
        .filter_map(|bytes| {
            let packet = parse_packet(bytes).ok()?;
            if packet.id != ferroan_protocol::msgs::LayerData::ID {
                return None;
            }
            let message = packet.decode::<ferroan_protocol::msgs::LayerData>().ok()?;
            Some((message.layer_id.type_, message.layer_data.data))
        })
        .collect();
    assert_eq!(
        bodies,
        vec![(0x4c, b"patch-a".to_vec()), (0x4c, b"patch-b".to_vec()),]
    );
    assert!(state.pending_land.is_empty());
}

fn overlay_chunk(sequence_id: i32, byte: u8) -> crate::ParcelOverlayChunk {
    crate::ParcelOverlayChunk {
        sequence_id,
        data: vec![byte; 8],
    }
}

fn overlay_message(sequence_id: i32, byte: u8) -> ferroan_protocol::msgs::ParcelOverlay {
    let mut message = ferroan_protocol::msgs::ParcelOverlay::default();
    message.parcel_data.sequence_id = sequence_id;
    message.parcel_data.data = vec![byte; 8];
    message
}

fn sample_land() -> ArrivalConfig {
    ArrivalConfig {
        overlay: vec![overlay_chunk(0, 0x11), overlay_chunk(1, 0x22)],
        land: vec![crate::LandLayer {
            kind: 0x4c,
            data: b"patch".to_vec(),
        }],
        ..ArrivalConfig::default()
    }
}

fn land_ids(bytes: &[Vec<u8>]) -> Vec<ferroan_protocol::MsgId> {
    bytes
        .iter()
        .filter_map(|packet| {
            let parsed = parse_packet(packet).ok()?;
            matches!(
                parsed.id,
                ferroan_protocol::msgs::ParcelOverlay::ID | ferroan_protocol::msgs::LayerData::ID
            )
            .then_some(parsed.id)
        })
        .collect()
}

async fn complete(state: &mut State, io: &FakeIo, expect: &CircuitExpect, sequence: u32) {
    let request = movement(*expect, sequence, &[]);
    state
        .handle(
            io,
            peer(),
            expect,
            &parse_packet(&request).unwrap(),
            request.len(),
        )
        .await
        .unwrap();
}

/// Eligible completion queues the overlay ahead of staged terrain and drains
/// both on Land while the reliable window has room.
#[tokio::test]
async fn overlay_precedes_land_and_drains_through_the_reliable_window() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), sample_land());
    state.peer = Some(peer());
    let io = FakeIo::default();
    state.maintain(&io).await.unwrap();
    assert!(land_ids(&io.sent.lock().unwrap()).is_empty());
    assert!(state.pending_land.is_empty());
    complete(&mut state, &io, &expect, 1).await;
    let mut layer = ferroan_protocol::msgs::LayerData::default();
    layer.layer_id.type_ = 0x4c;
    layer.layer_data.data = b"patch".to_vec();
    let queued = [
        encode_payload(&overlay_message(0, 0x11)).unwrap().bytes,
        encode_payload(&overlay_message(1, 0x22)).unwrap().bytes,
        encode_payload(&layer).unwrap().bytes,
    ];
    let pending: Vec<_> = state
        .pending_land
        .iter()
        .map(|payload| payload.bytes.clone())
        .collect();
    assert_eq!(pending, queued);
    release_throttled(&mut state, &io).await;
    assert_eq!(
        land_ids(&io.sent.lock().unwrap()),
        vec![
            ferroan_protocol::msgs::ParcelOverlay::ID,
            ferroan_protocol::msgs::ParcelOverlay::ID,
            ferroan_protocol::msgs::LayerData::ID,
        ]
    );
    let sent = io.sent.lock().unwrap();
    let sequences: Vec<i32> = sent
        .iter()
        .filter_map(|bytes| {
            let packet = parse_packet(bytes).ok()?;
            if packet.id != ferroan_protocol::msgs::ParcelOverlay::ID {
                return None;
            }
            Some(
                packet
                    .decode::<ferroan_protocol::msgs::ParcelOverlay>()
                    .unwrap()
                    .parcel_data
                    .sequence_id,
            )
        })
        .collect();
    assert_eq!(sequences, vec![0, 1]);
}

/// A crossing update that teleport flags did not clear suppresses terrain and
/// the overlay. AgentMovementComplete still goes out.
#[tokio::test]
async fn a_crossing_update_suppresses_the_overlay() {
    let expect = identity(Uuid::from_u128(1));
    let mut config = sample_land();
    config.teleport_flags = 0;
    config.crossing_flags = 4;
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    let io = FakeIo::default();
    complete(&mut state, &io, &expect, 1).await;
    assert!(state.pending_land.is_empty());
    assert!(land_ids(&io.sent.lock().unwrap()).is_empty());
    assert!(message_ids(&io).contains(&ferroan_protocol::msgs::AgentMovementComplete::ID));
    state.maintain(&io).await.unwrap();
    assert!(state.pending_land.is_empty());
}

/// Positive teleport flags clear the crossing update, so the pass is sent.
#[tokio::test]
async fn positive_teleport_flags_clear_a_crossing_update() {
    let expect = identity(Uuid::from_u128(1));
    let mut config = sample_land();
    config.crossing_flags = 4;
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    let io = FakeIo::default();
    complete(&mut state, &io, &expect, 1).await;
    assert_eq!(state.pending_land.len(), 3);
}

/// A second CompleteAgentMovement does not send another arrival or overlay.
#[tokio::test]
async fn a_second_complete_movement_does_not_repeat_the_overlay() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), sample_land());
    state.peer = Some(peer());
    let io = FakeIo::default();
    complete(&mut state, &io, &expect, 1).await;
    let queued = state.pending_land.len();
    complete(&mut state, &io, &expect, 2).await;
    assert_eq!(state.pending_land.len(), queued);
    let arrivals = message_ids(&io)
        .iter()
        .filter(|id| **id == ferroan_protocol::msgs::AgentMovementComplete::ID)
        .count();
    assert_eq!(arrivals, 1);
}

#[derive(Default)]
struct LandPass {
    flag: std::sync::atomic::AtomicBool,
}

impl crate::EntityUpdates for LandPass {
    fn has_updates(&self) -> bool {
        false
    }

    fn take_updates(&self, _: &mut crate::TaskBatch) {}

    fn take_avatar_packets(&self, _: usize) -> Vec<AvatarPacket> {
        Vec::new()
    }

    fn take_land_pass(&self) -> bool {
        self.flag.swap(false, Ordering::AcqRel)
    }
}

fn with_pass(flags: u32, pass: Arc<LandPass>) -> ArrivalConfig {
    ArrivalConfig {
        teleport_flags: flags,
        updates: Some(crate::EntityFeed {
            updates: pass,
            wake: Arc::new(Notify::new()),
        }),
        ..sample_land()
    }
}

/// Zero teleport flags append another copy when SendInitialData asks.
/// ViaLogin ignores that signal.
#[tokio::test]
async fn the_initial_data_gate_repeats_land_only_for_zero_teleport_flags() {
    let expect = identity(Uuid::from_u128(1));
    let pass = Arc::new(LandPass::default());
    let mut state = State::new(captured_config(), with_pass(0, Arc::clone(&pass)));
    state.peer = Some(peer());
    let io = FakeIo::default();
    complete(&mut state, &io, &expect, 1).await;
    release_throttled(&mut state, &io).await;
    assert_eq!(land_ids(&io.sent.lock().unwrap()).len(), 3);
    pass.flag.store(true, Ordering::Release);
    state.advance_throttle(std::time::Duration::from_secs(1));
    state.maintain(&io).await.unwrap();
    release_throttled(&mut state, &io).await;
    assert_eq!(land_ids(&io.sent.lock().unwrap()).len(), 6);

    let login = Arc::new(LandPass::default());
    let mut state = State::new(
        captured_config(),
        with_pass(crate::VIA_LOGIN, Arc::clone(&login)),
    );
    state.peer = Some(peer());
    let io = FakeIo::default();
    complete(&mut state, &io, &expect, 1).await;
    release_throttled(&mut state, &io).await;
    login.flag.store(true, Ordering::Release);
    state.advance_throttle(std::time::Duration::from_secs(1));
    state.maintain(&io).await.unwrap();
    assert!(state.pending_land.is_empty());
    assert_eq!(land_ids(&io.sent.lock().unwrap()).len(), 3);
}

/// The overlay waits on Land with the same reliable-window stall as terrain.
#[tokio::test(start_paused = true)]
async fn a_full_reliable_window_holds_the_overlay() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), sample_land());
    state.peer = Some(peer());
    let io = FakeIo::default();
    for _ in 0..=ACK_STALL {
        state
            .send_payload(
                &io,
                peer(),
                Payload {
                    bytes: vec![0xff, 0xff, 0x00, 0x01, 1],
                    coding: Coding::Plain,
                    reliable: true,
                },
                true,
            )
            .await
            .unwrap();
    }
    io.sent.lock().unwrap().clear();
    complete(&mut state, &io, &expect, 1).await;
    assert_eq!(state.pending_land.len(), 3);
    state.advance_throttle(std::time::Duration::from_millis(200));
    state.maintain(&io).await.unwrap();
    assert_eq!(state.pending_land.len(), 3);
    assert!(land_ids(&io.sent.lock().unwrap()).is_empty());
    state.unacked.acknowledge(1..=10);
    release_throttled(&mut state, &io).await;
    assert!(state.pending_land.is_empty());
    assert_eq!(land_ids(&io.sent.lock().unwrap()).len(), 3);
}

fn teleport_request(
    expect: CircuitExpect,
    handle: u64,
    position: [f32; 3],
    sequence: u32,
) -> Vec<u8> {
    let mut message = ferroan_protocol::msgs::TeleportLocationRequest::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    message.info.region_handle = handle;
    message.info.position = position.into();
    message.info.look_at = [1.0, 0.0, 0.0].into();
    wire(&message, sequence, &[])
}

fn seated_teleport(region: crate::teleport::TeleportRegion) -> (State, CircuitExpect, FakeIo) {
    let expect = identity(Uuid::from_u128(1));
    let config = ArrivalConfig {
        teleport: Some(region),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    (state, expect, FakeIo::default())
}

async fn handle_packet(state: &mut State, io: &FakeIo, expect: &CircuitExpect, request: &[u8]) {
    state
        .handle(
            io,
            peer(),
            expect,
            &parse_packet(request).unwrap(),
            request.len(),
        )
        .await
        .unwrap();
}

fn stored_pose(position: [f32; 3]) -> LoginPose {
    LoginPose {
        position,
        rotation: LOGIN_ROTATION,
        velocity: [0.0, 0.0, 0.0],
        control_flags: 0,
        draw_distance: LOGIN_DRAW_DISTANCE,
    }
}

fn recording_scene(store: Arc<Mutex<HashMap<Uuid, [f32; 3]>>>) -> LoginScene {
    Arc::new(move |agent, call| {
        let mut poses = store.lock().unwrap_or_else(|poison| poison.into_inner());
        let result: crate::circuit::SceneResult = match call {
            SceneCall::Pose(pose, _) => {
                poses.insert(agent, pose.position);
                Ok(SceneReply::Pose(pose))
            }
            SceneCall::Complete(crate::return_hop::ReturnPose { position, .. }) => {
                poses.insert(agent, position);
                Ok(SceneReply::Pose(stored_pose(position)))
            }
            SceneCall::Teleport(request) => {
                poses.insert(agent, request.position);
                Ok(SceneReply::Moved(SceneMove {
                    position: request.position,
                    look_at: request.look_at,
                    flags: request.flags,
                }))
            }
            SceneCall::AgentUpdate(_)
            | SceneCall::AlwaysRun(_)
            | SceneCall::AgentAnimation(_)
            | SceneCall::HandshakeReply
            | SceneCall::InitialLand
            | SceneCall::ParcelRequest(_) => Ok(SceneReply::Applied),
            SceneCall::Departure => Ok(SceneReply::Departure(crate::circuit::SceneDeparture {
                flying: false,
                pose: crate::source_hop::HopPose::default(),
            })),
            SceneCall::Arrive(pose, _) => {
                let position = *poses.entry(agent).or_insert(pose.position);
                Ok(SceneReply::Pose(stored_pose(position)))
            }
            SceneCall::Read => Ok(poses
                .get(&agent)
                .copied()
                .map_or(SceneReply::Missing, |position| {
                    SceneReply::Pose(stored_pose(position))
                })),
            SceneCall::Places(ids) => {
                let places = ids
                    .into_iter()
                    .filter_map(|id| poses.get(&id).copied().map(|position| (id, position)))
                    .collect();
                Ok(SceneReply::Places(places))
            }
        };
        result.into()
    })
}

async fn release_throttled(state: &mut State, io: &FakeIo) {
    for _ in 0..40 {
        if state.pending_land.is_empty() && state.pending_task.is_empty() {
            return;
        }
        state.advance_throttle(std::time::Duration::from_millis(200));
        state.maintain(io).await.unwrap();
    }
    panic!("queued land or task did not drain");
}

#[tokio::test(start_paused = true)]
async fn a_task_flood_stays_within_the_viewer_task_rate() {
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let blob = ferroan_bot::throttle::initial_throttle_bytes();
    state.throttle.set_throttles(&blob, 1.0);
    let rate = f64::from(state.throttle.view().categories[TASK].drip);
    assert!(rate > 100_000.0, "viewer task rate {rate}");
    let body = vec![0_u8; 1_100];
    for _ in 0..200 {
        state.pending_task.push_back(Payload {
            bytes: body.clone(),
            coding: Coding::Plain,
            reliable: true,
        });
    }
    let io = FakeIo::default();
    let step = std::time::Duration::from_millis(5);
    let steps = 100_u32;
    for _ in 0..steps {
        state.advance_throttle(step);
        state.maintain(&io).await.unwrap();
    }
    let sent = io.sent.lock().unwrap();
    assert!(sent.len() > 1);
    assert!(!state.pending_task.is_empty());
    let mut bytes = 0_usize;
    let mut largest = 0_usize;
    for packet in sent.iter() {
        let parsed = parse_packet(packet).unwrap();
        assert!(!parsed.resent());
        assert_ne!(parsed.id, ferroan_protocol::msgs::StartPingCheck::ID);
        bytes += packet.len();
        largest = largest.max(packet.len());
    }
    let window = step.as_secs_f64() * f64::from(steps);
    let budget = rate * window + largest as f64 + 32.0;
    assert!(
        (bytes as f64) <= budget,
        "sent {bytes} bytes, budget {budget}, rate {rate}"
    );
    assert!(
        bytes > 50_000,
        "sent {bytes} bytes, which is inside the default task rate"
    );
}

#[tokio::test(start_paused = true)]
async fn an_ack_backlog_holds_land_and_task_but_not_resends() {
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let io = FakeIo::default();
    // `LLUDPClient.DequeueOutgoing` (lines 605-609): resends only past 50.
    for _ in 0..=ACK_STALL {
        let payload = Payload {
            bytes: vec![0xff, 0xff, 0x00, 0x01, 1, 2, 3],
            coding: Coding::Plain,
            reliable: true,
        };
        state
            .send_payload(&io, peer(), payload, true)
            .await
            .unwrap();
    }
    assert_eq!(state.unacked.len(), ACK_STALL + 1);
    io.sent.lock().unwrap().clear();
    state.pending_task.push_back(Payload {
        bytes: vec![0xff, 0xff, 0x00, 0x02, 4],
        coding: Coding::Plain,
        reliable: true,
    });
    state
        .pending_land
        .push_back(encode_payload(&LayerData::default()).unwrap());
    state.advance_throttle(Duration::from_millis(200));
    state.maintain(&io).await.unwrap();
    assert_eq!(state.pending_task.len(), 1);
    assert_eq!(state.pending_land.len(), 1);
    assert!(io.sent.lock().unwrap().is_empty());

    tokio::time::advance(RESEND_AFTER + RESEND_INTERVAL).await;
    state.advance_throttle(Duration::from_millis(200));
    state.maintain(&io).await.unwrap();
    let resent = io
        .sent
        .lock()
        .unwrap()
        .iter()
        .filter(|bytes| parse_packet(bytes).unwrap().resent())
        .count();
    assert!(resent > 0, "resends are not gated by the backlog");
    assert_eq!(state.pending_task.len(), 1);
    assert_eq!(state.pending_land.len(), 1);

    // At 50 waiting the queues move again. Each send adds one more.
    state.unacked.acknowledge(1..=10);
    release_throttled(&mut state, &io).await;
    assert!(state.pending_task.is_empty());
    assert!(state.pending_land.is_empty());
}

#[tokio::test]
async fn a_map_block_reply_is_queued_on_land() {
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let io = FakeIo::default();
    let (done, answer) = oneshot::channel();
    let message = crate::source_hop::offline_map_block(Uuid::from_u128(3), 1_000);
    state
        .apply_hop(&io, HopCommand::Map { message, done })
        .await
        .unwrap();
    // `SendMapBlock` only queues (`LLClientView.cs` 2209 and 2238).
    assert!(answer.await.unwrap());
    assert!(io.sent.lock().unwrap().is_empty());
    assert_eq!(state.pending_land.len(), 1);
    release_throttled(&mut state, &io).await;
    assert!(
        message_ids(&io).contains(&ferroan_protocol::msgs::MapBlockReply::ID),
        "the land queue sends the map block"
    );
}

#[tokio::test]
async fn a_nan_throttle_blob_keeps_the_circuit_sending() {
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let before = state.throttle.view();
    let mut blob = ferroan_bot::throttle::initial_throttle_bytes();
    blob[16..20].copy_from_slice(&f32::NAN.to_le_bytes());
    let expect = identity(Uuid::from_u128(1));
    let mut message = AgentThrottle::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    message.agent_data.circuit_code = expect.circuit_code;
    message.throttle.throttles = blob;
    let bytes = wire(&message, 1, &[]);
    let io = FakeIo::default();
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    assert_eq!(state.throttle.view(), before);
    state.pending_task.push_back(Payload {
        bytes: vec![0xff, 0xff, 0x00, 0x02, 4],
        coding: Coding::Plain,
        reliable: true,
    });
    release_throttled(&mut state, &io).await;
}

fn message_ids(io: &FakeIo) -> Vec<ferroan_protocol::MsgId> {
    io.sent
        .lock()
        .unwrap()
        .iter()
        .map(|bytes| parse_packet(bytes).unwrap().id)
        .collect()
}

impl crate::EntityUpdates for ferroan_scene::terse::ViewerFeed {
    fn has_updates(&self) -> bool {
        self.has_avatar_terse()
    }

    fn take_updates(&self, batch: &mut crate::TaskBatch) {
        for block in self.take_terse(batch.budget()) {
            batch.push_terse(block);
        }
    }

    fn take_avatar_packets(&self, max: usize) -> Vec<AvatarPacket> {
        // This frame has only the sender, with coarse locations disabled:
        // the only avatar packets are its own animations, which this test
        // does not send.
        assert!(
            self.take_avatar_packets(max)
                .iter()
                .all(|packet| matches!(packet, ferroan_scene::AvatarPacket::Animations { .. }))
        );
        Vec::new()
    }

    fn take_land_pass(&self) -> bool {
        ferroan_scene::terse::ViewerFeed::take_land_pass(self)
    }
}

#[tokio::test]
async fn local_teleport_stays_behind_the_frames_preteleport_terse() {
    use ferroan_protocol::msgs::{ImprovedTerseObjectUpdate, TeleportLocal, TeleportStart};
    use ferroan_scene::{AvatarLook, Presence, TerrainFrame, ViewerLink};

    let start = [128.0, 128.0, ferroan_scene::AVATAR_SIZE_Z];
    let (mut state, expect, io) = seated_teleport(crate::teleport::TeleportRegion::flat(start));
    let frame = TerrainFrame::start_without_map().unwrap();
    let feed = frame.new_feed();
    let wake = feed.wake();
    let link = ViewerLink {
        feed: Arc::clone(&feed),
        seeds_sent: Arc::new(true.into()),
        circuit_open: Arc::new(false.into()),
        look: Arc::new(AvatarLook {
            first_name: "Test".into(),
            last_name: "User".into(),
            title: String::new(),
            visual_params: vec![],
            texture: vec![],
            height: 1.9,
            hover: 0.0,
        }),
        teleport_flags: 1 << 7,
    };
    let local_id = frame
        .attach(expect.agent_id, link)
        .unwrap()
        .wait()
        .await
        .unwrap();
    let pose = Presence {
        position: start,
        rotation: [0.0, 0.0, 0.0, 1.0],
        velocity: [0.0; 3],
        control_flags: 0,
        draw_distance: 128.0,
    };
    frame
        .arrive(expect.agent_id, pose, [1.0, 0.0, 0.0])
        .unwrap()
        .wait()
        .await
        .unwrap();
    frame.handshake_reply(expect.agent_id).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !feed.has_updates() {
            wake.notified().await;
        }
    })
    .await
    .unwrap();
    state.feed = Some(EntityFeed {
        updates: feed,
        wake,
    });
    state.login_scene = Some(Arc::new(move |agent, call| {
        let SceneCall::Teleport(request) = call else {
            panic!("expected a local teleport")
        };
        let moved = frame
            .teleport_local(
                agent,
                ferroan_scene::LocalTeleport {
                    position: request.position,
                    look_at: request.look_at,
                    flags: request.flags,
                    half_height: request.half_height,
                    default_landing: request.default_landing,
                    size_x: request.size_x,
                    size_y: request.size_y,
                },
            )
            .unwrap();
        SceneAnswer::wait(async move {
            let moved = moved.wait().await.unwrap();
            Ok(SceneReply::Moved(SceneMove {
                position: moved.pose.position,
                look_at: moved.look_at,
                flags: moved.flags,
            }))
        })
    }));
    // Hold Task while the normal send loop picks up and encodes the frame's pose.
    state.advance_throttle(Duration::from_secs(1));
    assert!(
        state
            .throttle
            .try_spend(TASK, 1_000_000, state.throttle_clock.now_secs())
    );
    state.maintain(&io).await.unwrap();
    assert_eq!(state.pending_task.len(), 1);
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0], 2);
    handle_packet(&mut state, &io, &expect, &request).await;
    let order = [
        TeleportStart::ID,
        ImprovedTerseObjectUpdate::ID,
        TeleportLocal::ID,
    ];
    let sent = || {
        io.sent
            .lock()
            .unwrap()
            .iter()
            .map(|bytes| parse_packet(bytes).unwrap())
            .filter(|packet| order.contains(&packet.id))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        sent().iter().map(|p| p.id).collect::<Vec<_>>(),
        [TeleportStart::ID]
    );
    state.advance_throttle(Duration::from_secs(600));
    release_throttled(&mut state, &io).await;
    let packets = sent();
    assert_eq!(
        packets.iter().take(3).map(|p| p.id).collect::<Vec<_>>(),
        order
    );
    let terse = packets[1].decode::<ImprovedTerseObjectUpdate>().unwrap();
    let block = &terse.object_data[0].data;
    assert_eq!(block[..4], local_id.to_le_bytes());
    assert_eq!(block[22..26], start[0].to_le_bytes());
    assert_eq!(block[26..30], start[1].to_le_bytes());
    assert_eq!(block[30..34], start[2].to_le_bytes());
    assert_eq!(
        packets[2]
            .decode::<TeleportLocal>()
            .unwrap()
            .info
            .position
            .to_array(),
        [40.0, 200.0, 30.0]
    );
    // A later frame may already have offered another terse at the landing.
    for packet in &packets[3..] {
        let update = packet.decode::<ImprovedTerseObjectUpdate>().unwrap();
        for block in &update.object_data {
            assert_eq!(block.data[22..26], 40.0_f32.to_le_bytes());
            assert_eq!(block.data[26..30], 200.0_f32.to_le_bytes());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn local_teleport_after_arrival_with_land_and_task_backlogs_matches_csharp() {
    use ferroan_protocol::msgs::{
        AgentMovementComplete, TeleportFailed, TeleportLocal, TeleportStart,
    };

    let capture =
        ferroan_capture::read_capture(include_bytes!("../../../fixtures/captures/session.cap"))
            .unwrap();
    let pair: Vec<_> = capture
        .iter()
        .filter(|r| r.kind == ferroan_capture::Kind::UdpOut)
        .filter_map(|r| parse_packet(&r.bytes).ok())
        .filter(|p| !p.resent() && [TeleportStart::ID, TeleportLocal::ID].contains(&p.id))
        .take(2)
        .collect();
    assert_eq!(pair[0].id, TeleportStart::ID);
    let local = pair[1].decode::<TeleportLocal>().unwrap();
    let expect = identity(local.info.agent_id);
    for name_count in [0, MAX_UNACKED] {
        let arrival = ArrivalConfig {
            land: vec![
                crate::LandLayer {
                    kind: 0x4c,
                    data: vec![0x77; 128]
                };
                256
            ],
            teleport: Some(crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0])),
            ..ArrivalConfig::default()
        };
        let mut state = State::new(captured_config(), arrival);
        state.peer = Some(peer());
        let io = FakeIo::default();
        state.login_scene = Some(recording_scene(Arc::default()));
        handle_packet(&mut state, &io, &expect, &movement(expect, 1, &[])).await;
        assert!(message_ids(&io).contains(&AgentMovementComplete::ID));
        for _ in 0..256 {
            state.advance_throttle(Duration::from_millis(200));
            state.maintain(&io).await.unwrap();
            if state.unacked.len() > ACK_STALL {
                break;
            }
        }
        assert_eq!(state.unacked.len(), ACK_STALL + 1);
        assert!(!state.pending_land.is_empty());
        for n in 0..name_count {
            state
                .enqueue_name(
                    Uuid::from_u128(n as u128 + 1),
                    &UserName::new("Test", "User"),
                )
                .unwrap();
        }
        let request = teleport_request(expect, 0, local.info.position.to_array(), 2);
        handle_packet(&mut state, &io, &expect, &request).await;
        let ids = message_ids(&io);
        assert!(ids.contains(&TeleportStart::ID));
        assert!(!ids.contains(&TeleportFailed::ID));
        assert!(!ids.contains(&TeleportLocal::ID));
        // Acknowledge only datagrams delivered to this viewer.
        let mut acknowledged = 0;
        for tick in 0..256 {
            let acks = {
                let sent = io.sent.lock().unwrap();
                let acks = sent[acknowledged..]
                    .iter()
                    .map(|b| parse_packet(b).unwrap())
                    .filter(|p| p.reliable())
                    .map(|p| p.sequence)
                    .collect::<Vec<_>>();
                acknowledged = sent.len();
                acks
            };
            let ack = PacketAck {
                packets: acks
                    .into_iter()
                    .map(|id| packet_ack::Packets { id })
                    .collect(),
                omitted_trailing_blocks: 0,
            };
            let mut bytes = Vec::new();
            build_packet(&ack, tick + 3, false, false, &[], &mut bytes);
            handle_packet(&mut state, &io, &expect, &bytes).await;
            state.advance_throttle(Duration::from_millis(200));
            state.maintain(&io).await.unwrap();
            if state.pending_task.is_empty() {
                break;
            }
        }
        let sent = io.sent.lock().unwrap();
        let actual: Vec<_> = sent
            .iter()
            .map(|b| parse_packet(b).unwrap())
            .filter(|p| [TeleportStart::ID, TeleportLocal::ID].contains(&p.id))
            .collect();
        assert_eq!(actual.len(), 2);
        for (packet, reference) in actual.iter().zip(&pair) {
            assert_eq!(packet.id, reference.id);
            assert_eq!(packet.body, reference.body);
            let flags =
                ferroan_protocol::PacketFlags::RELIABLE | ferroan_protocol::PacketFlags::ZEROCODED;
            assert_eq!(packet.flags & flags, reference.flags & flags);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn only_unsent_local_teleports_consume_the_teleport_bound() {
    use ferroan_protocol::msgs::{TeleportFailed, TeleportStart};
    let (mut state, expect, io) =
        seated_teleport(crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]));
    let poses = Arc::new(Mutex::new(HashMap::new()));
    state.login_scene = Some(recording_scene(Arc::clone(&poses)));
    let first = [40.0, 200.0, 30.0];
    for n in 0..MAX_PENDING_TELEPORTS {
        let request = teleport_request(expect, 0, first, n as u32 + 1);
        handle_packet(&mut state, &io, &expect, &request).await;
        let start = parse_packet(io.sent.lock().unwrap().last().unwrap()).unwrap();
        assert_eq!(start.id, TeleportStart::ID);
        state.unacked.acknowledge([start.sequence]);
    }
    let next = [41.0, 201.0, 30.0];
    let last = MAX_PENDING_TELEPORTS as u32 + 1;
    handle_packet(
        &mut state,
        &io,
        &expect,
        &teleport_request(expect, 0, next, last),
    )
    .await;
    assert_eq!(message_ids(&io).last(), Some(&TeleportFailed::ID));
    assert_eq!(poses.lock().unwrap()[&expect.agent_id], first);

    state.advance_throttle(Duration::from_secs(1));
    state.maintain(&io).await.unwrap();
    handle_packet(
        &mut state,
        &io,
        &expect,
        &teleport_request(expect, 0, next, last + 1),
    )
    .await;
    assert_eq!(message_ids(&io).last(), Some(&TeleportStart::ID));
    assert_eq!(poses.lock().unwrap()[&expect.agent_id], next);
}

#[tokio::test]
async fn local_teleport_lands_and_moves_the_chat_seat() {
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let shared = region.position.clone();
    let (mut state, expect, io) = seated_teleport(region);
    let chat = RegionChat::new();
    state.chat = Some(chat.seat(expect.agent_id, "Test User".into()));
    let places = Arc::new(Mutex::new(HashMap::<Uuid, [f32; 3]>::new()));
    state.login_scene = Some(recording_scene(Arc::clone(&places)));
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0], 2);
    handle_packet(&mut state, &io, &expect, &request).await;
    release_throttled(&mut state, &io).await;
    assert_eq!(
        message_ids(&io),
        vec![
            ferroan_protocol::msgs::TeleportStart::ID,
            ferroan_protocol::msgs::TeleportLocal::ID,
        ]
    );
    let bytes = io.sent.lock().unwrap().last().unwrap().clone();
    let packet = parse_packet(&bytes).unwrap();
    let local = packet
        .decode::<ferroan_protocol::msgs::TeleportLocal>()
        .unwrap();
    assert!(packet.reliable());
    assert_eq!(packet.flags & ferroan_protocol::PacketFlags::ZEROCODED, 0);
    assert_eq!(local.info.agent_id, expect.agent_id);
    assert_eq!(local.info.location_id, crate::teleport::LOCATION_ID);
    assert_eq!(local.info.position.to_array(), [40.0, 200.0, 30.0]);
    assert_eq!(local.info.look_at.to_array(), [1.0, 0.0, 0.0]);
    assert_eq!(local.info.teleport_flags, crate::teleport::VIA_LOCATION);
    assert_eq!(state.arrival.position, [40.0, 200.0, 30.0]);
    assert_eq!(*shared.lock().unwrap(), [40.0, 200.0, 30.0]);
    let heard = places
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    assert_eq!(
        heard.get(&expect.agent_id).copied(),
        Some([40.0, 200.0, 30.0])
    );
    chat.audience()
        .publish(expect.agent_id, 1, 0, "after", &heard);
    let crate::chat::ViewerBound::Chat(said) = state.chat.as_mut().unwrap().rx.try_recv().unwrap()
    else {
        panic!("expected chat from the moved seat");
    };
    assert_eq!(said.chat_data.position.to_array(), [40.0, 200.0, 30.0]);
}

#[tokio::test]
async fn a_return_landing_writes_the_scene_pose() {
    let expect = identity(Uuid::from_u128(4));
    let places = Arc::new(Mutex::new(HashMap::<Uuid, [f32; 3]>::new()));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.login_scene = Some(recording_scene(Arc::clone(&places)));
    let pose = crate::return_hop::ReturnPose {
        position: [128.0, 128.0, 30.0],
        look_at: [1.0, 0.0, 0.0],
        origin: Uuid::from_u128(8),
        ..crate::return_hop::ReturnPose::default()
    };
    state.finish_return(expect.agent_id, pose).await.unwrap();
    assert_eq!(state.arrival.position, [128.0, 128.0, 30.0]);
    assert_eq!(state.arrival.look_at, [1.0, 0.0, 0.0]);
    assert_eq!(
        places
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&expect.agent_id)
            .copied(),
        Some([128.0, 128.0, 30.0])
    );
}

#[tokio::test]
async fn a_teleport_outside_the_region_or_while_frozen_fails() {
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let (mut state, expect, io) = seated_teleport(region);
    let request = teleport_request(expect, 256, [10.0, 10.0, 30.0], 2);
    handle_packet(&mut state, &io, &expect, &request).await;
    let failed = parse_packet(&io.sent.lock().unwrap()[0])
        .unwrap()
        .decode::<ferroan_protocol::msgs::TeleportFailed>()
        .unwrap();
    assert_eq!(
        ferroan_protocol::field_str(&failed.info.reason),
        crate::teleport::FAIL_PROCESS
    );
    assert!(
        message_ids(&io)
            .iter()
            .all(|id| *id != ferroan_protocol::msgs::TeleportLocal::ID)
    );

    let mut frozen = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    frozen.allow_movement = false;
    let (mut state, expect, io) = seated_teleport(frozen);
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0], 2);
    handle_packet(&mut state, &io, &expect, &request).await;
    let failed = parse_packet(&io.sent.lock().unwrap()[0])
        .unwrap()
        .decode::<ferroan_protocol::msgs::TeleportFailed>()
        .unwrap();
    assert_eq!(
        ferroan_protocol::field_str(&failed.info.reason),
        crate::teleport::FAIL_FROZEN
    );
}

#[tokio::test]
async fn a_source_hop_takes_flight_and_pose_from_the_scene_departure() {
    for flying in [false, true] {
        let flags = Arc::new(Mutex::new(None));
        let seen = Arc::clone(&flags);
        let hop = crate::source_hop::SourceHop::new(move |request, _out| {
            *seen.lock().unwrap() = Some((request.flags, request.pose));
            Box::pin(async {})
        });
        let expect = identity(Uuid::from_u128(1));
        let config = ArrivalConfig {
            teleport: Some(crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0])),
            source_hop: Some(hop),
            ..ArrivalConfig::default()
        };
        let mut state = State::new(captured_config(), config);
        state.peer = Some(peer());
        state.login_scene = Some(Arc::new(move |_agent, call| match call {
            SceneCall::Departure => Ok(SceneReply::Departure(crate::circuit::SceneDeparture {
                flying,
                pose: crate::source_hop::HopPose {
                    always_run: true,
                    center: [1.0, 2.0, 3.0],
                    head_rotation: [0.0, 0.0, 0.6, 0.8],
                    body_rotation: [0.0, 0.0, 0.8, 0.6],
                    animations: crate::AgentAnimations {
                        motion_state: 1,
                        ..crate::AgentAnimations::standing()
                    },
                },
            }))
            .into(),
            _ => Err(crate::circuit::SceneRefused).into(),
        }));
        let io = FakeIo::default();
        let foreign = teleport_request(expect, 256, [10.0, 10.0, 30.0], 2);
        handle_packet(&mut state, &io, &expect, &foreign).await;
        for _ in 0..50 {
            if flags.lock().unwrap().is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let mut expected = crate::teleport::VIA_LOCATION;
        if flying {
            expected |= crate::teleport::IS_FLYING;
        }
        let (sent, pose) = flags.lock().unwrap().clone().unwrap();
        assert_eq!(sent, expected);
        // The frame's kept values, not the last raw AgentUpdate.
        assert!(pose.always_run);
        assert_eq!(pose.center, [1.0, 2.0, 3.0]);
        assert_eq!(pose.head_rotation, [0.0, 0.0, 0.6, 0.8]);
        assert_eq!(pose.body_rotation, [0.0, 0.0, 0.8, 0.6]);
        // And its animator, for `CopyTo`.
        assert_eq!(pose.animations.motion_state, 1);
    }
}

#[tokio::test]
async fn a_return_forwards_the_whole_agent_data_to_the_scene() {
    let expect = identity(Uuid::from_u128(4));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    let seen = Arc::new(Mutex::new(None));
    let store = Arc::clone(&seen);
    state.login_scene = Some(Arc::new(move |_agent, call| {
        let SceneCall::Complete(pose) = call else {
            return Err(crate::circuit::SceneRefused).into();
        };
        *store.lock().unwrap() = Some(pose);
        Ok(SceneReply::Pose(stored_pose([1.0, 2.0, 3.0]))).into()
    }));
    let pose = crate::return_hop::ReturnPose {
        position: [128.0, 128.0, 30.0],
        look_at: [0.0, 1.0, 0.0],
        origin: Uuid::from_u128(8),
        velocity: [1.0, 0.0, 0.0],
        far: 96.0,
        body_rotation: [0.0, 0.0, 1.0, 0.0],
        head_rotation: [0.0, 0.0, 0.0, 1.0],
        control_flags: 1 << 13,
        always_run: true,
        // The animator fields reach the frame with the rest.
        animations: crate::AgentAnimations {
            motion_state: 1,
            ..crate::AgentAnimations::standing()
        },
    };
    state
        .finish_return(expect.agent_id, pose.clone())
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), Some(pose));
    // The arrival uses the position the frame stored.
    assert_eq!(state.arrival.position, [1.0, 2.0, 3.0]);
    assert_eq!(state.arrival.look_at, [0.0, 1.0, 0.0]);
}

#[tokio::test]
async fn a_second_teleport_during_a_source_hop_is_refused() {
    let calls = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Notify::new());
    let seen = Arc::clone(&calls);
    let wait = Arc::clone(&gate);
    let hop = crate::source_hop::SourceHop::new(move |_request, _out| {
        let seen = Arc::clone(&seen);
        let wait = Arc::clone(&wait);
        Box::pin(async move {
            seen.fetch_add(1, Ordering::SeqCst);
            wait.notified().await;
        })
    });
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let position = Arc::clone(&region.position);
    let expect = identity(Uuid::from_u128(1));
    let config = ArrivalConfig {
        teleport: Some(region),
        source_hop: Some(hop),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    let io = FakeIo::default();
    let foreign = teleport_request(expect, 256, [10.0, 10.0, 30.0], 2);
    handle_packet(&mut state, &io, &expect, &foreign).await;
    for _ in 0..50 {
        if calls.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(state.in_transit);
    io.sent.lock().unwrap().clear();
    let again = teleport_request(expect, 256, [10.0, 10.0, 30.0], 3);
    handle_packet(&mut state, &io, &expect, &again).await;
    let local = teleport_request(expect, 0, [40.0, 200.0, 30.0], 4);
    handle_packet(&mut state, &io, &expect, &local).await;
    {
        let sent = io.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        for bytes in sent.iter() {
            let packet = parse_packet(bytes).unwrap();
            let failed = packet
                .decode::<ferroan_protocol::msgs::TeleportFailed>()
                .unwrap();
            assert_eq!(
                ferroan_protocol::field_str(&failed.info.reason),
                crate::teleport::FAIL_INCOMPLETE
            );
            assert!(packet.reliable());
            assert_eq!(packet.flags & ferroan_protocol::PacketFlags::ZEROCODED, 0);
        }
    }
    assert!(
        message_ids(&io)
            .iter()
            .all(|id| *id != ferroan_protocol::msgs::TeleportLocal::ID)
    );
    assert_eq!(*position.lock().unwrap(), [128.0, 128.0, 25.0]);
    assert_eq!(state.arrival.position, [128.0, 128.0, 25.0]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    gate.notify_one();
    for _ in 0..50 {
        tokio::task::yield_now().await;
        state.maintain(&io).await.unwrap();
        if !state.in_transit {
            break;
        }
    }
    assert!(!state.in_transit);
    let after = teleport_request(expect, 256, [10.0, 10.0, 30.0], 5);
    handle_packet(&mut state, &io, &expect, &after).await;
    for _ in 0..50 {
        if calls.load(Ordering::SeqCst) == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    gate.notify_one();
}

#[tokio::test]
async fn a_source_hop_does_not_write_the_scene_pose() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let hop = crate::source_hop::SourceHop::new(move |_request, _out| {
        let seen = Arc::clone(&seen);
        Box::pin(async move {
            seen.fetch_add(1, Ordering::SeqCst);
        })
    });
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let position = Arc::clone(&region.position);
    let places = Arc::new(Mutex::new(HashMap::<Uuid, [f32; 3]>::new()));
    let expect = identity(Uuid::from_u128(1));
    let config = ArrivalConfig {
        teleport: Some(region),
        source_hop: Some(hop),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    state.login_scene = Some(recording_scene(Arc::clone(&places)));
    let io = FakeIo::default();
    let foreign = teleport_request(expect, 256, [10.0, 10.0, 30.0], 2);
    handle_packet(&mut state, &io, &expect, &foreign).await;
    for _ in 0..50 {
        if calls.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        places
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .is_empty()
    );
    assert_eq!(*position.lock().unwrap(), [128.0, 128.0, 25.0]);
    assert_eq!(state.arrival.position, [128.0, 128.0, 25.0]);
}

#[tokio::test]
async fn complete_movement_send_path_matches_captured_csharp_arrival() {
    let config = ArrivalConfig {
        // Preserve C#'s exact terrain-adjusted Z (one ULP above literal25.96).
        position: [128.0, 128.0, f32::from_bits(0x41cf_ae15)],
        look_at: [0.99, 0.042, 0.0],
        region_handle: 1_099_511_628_032_000,
        channel_version:
            "OpenSim-NGC Tranquillity Release 1.2.192-alpha+f67db54399 (X64/Unix/DotNet)".into(),
        land: Vec::new(),
        overlay: Vec::new(),
        teleport_flags: 1 << 7,
        crossing_flags: 0,
        avatar: None,
        teleport: None,
        source_hop: None,
        return_slot: None,
        updates: None,
    };
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), config);
    let io = FakeIo::default();
    let captured =
        ferroan_capture::read_capture(include_bytes!("../../../fixtures/arrival/sequence.cap"))
            .unwrap();
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    for record in captured
        .iter()
        .filter(|record| record.kind == ferroan_capture::Kind::UdpIn)
    {
        let packet = parse_packet(&record.bytes).unwrap();
        state
            .handle(&io, peer(), &expect, &packet, record.bytes.len())
            .await
            .unwrap();
    }
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    let sent = io.sent.lock().unwrap();
    let mut arrival = sent
        .iter()
        .find(|bytes| {
            parse_packet(bytes).unwrap().id == ferroan_protocol::msgs::AgentMovementComplete::ID
        })
        .unwrap()
        .clone();
    let packet = parse_packet(&arrival).unwrap();
    let message = packet
        .decode::<ferroan_protocol::msgs::AgentMovementComplete>()
        .unwrap();
    assert!(packet.reliable());
    assert_eq!(packet.flags, ferroan_protocol::PacketFlags::RELIABLE);
    assert!((before..=after).contains(&message.data.timestamp));
    assert!(state.unacked.contains(packet.sequence));
    // The capture and expected agent use P0.10d identity folding. Only the
    // run's Unix timestamp changes here; preserve every other wire byte.
    arrival[74..78].fill(0);
    assert_eq!(arrival, ARRIVAL);
}

#[tokio::test]
async fn arrival_sends_the_avatar_update_and_unbaked_appearance() {
    let expect = identity(Uuid::from_u128(7));
    let config = ArrivalConfig {
        avatar: Some(crate::AvatarWire::standing(
            expect.agent_id,
            1,
            "Test",
            "User",
        )),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    let io = FakeIo::default();
    let request = movement(expect, 1, &[]);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&request).unwrap(),
            request.len(),
        )
        .await
        .unwrap();
    // Both ride the task category (`LLClientView.cs` 4656 and 4788).
    assert_eq!(state.pending_task.len(), 2);
    release_throttled(&mut state, &io).await;
    let sent = io.sent.lock().unwrap();
    let mut saw_object = false;
    let mut saw_appearance = false;
    for bytes in sent.iter() {
        let packet = parse_packet(bytes).unwrap();
        if packet.id == ferroan_protocol::msgs::ObjectUpdate::ID {
            let message = packet
                .decode::<ferroan_protocol::msgs::ObjectUpdate>()
                .unwrap();
            saw_object = message.object_data.iter().any(|object| {
                object.p_code == 47
                    && object.full_id == expect.agent_id
                    && object.object_data.len() == 76
            });
        }
        if packet.id == ferroan_protocol::msgs::AvatarAppearance::ID {
            let message = packet
                .decode::<ferroan_protocol::msgs::AvatarAppearance>()
                .unwrap();
            saw_appearance = message.appearance_data.is_empty()
                && message.appearance_hover.len() == 1
                && message.sender.id == expect.agent_id
                && !message.visual_param.is_empty();
        }
    }
    assert!(
        saw_object && saw_appearance,
        "object {saw_object} appearance {saw_appearance}"
    );
}

#[tokio::test]
async fn arrival_requires_current_identity_and_circuit_then_sends_only_once() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let io = FakeIo::default();
    for (index, wrong) in [
        CircuitExpect {
            agent_id: Uuid::from_u128(99),
            ..expect
        },
        CircuitExpect {
            session_id: Uuid::from_u128(99),
            ..expect
        },
        CircuitExpect {
            circuit_code: 99,
            ..expect
        },
    ]
    .into_iter()
    .enumerate()
    {
        let request = movement(wrong, index as u32 + 1, &[]);
        state
            .handle(
                &io,
                peer(),
                &expect,
                &parse_packet(&request).unwrap(),
                request.len(),
            )
            .await
            .unwrap();
        assert!(!state.arrived);
        assert!(io.sent.lock().unwrap().is_empty());
    }
    // Only the wrong-code packet passes the C# agent/session validation and
    // gets ACKed; the callback declines its movement request.
    assert_eq!(
        state.pending_acks.iter().copied().collect::<Vec<_>>(),
        vec![3]
    );
    for sequence in [4, 4, 5] {
        let request = movement(expect, sequence, &[]);
        state
            .handle(
                &io,
                peer(),
                &expect,
                &parse_packet(&request).unwrap(),
                request.len(),
            )
            .await
            .unwrap();
    }
    assert!(state.arrived);
    assert_eq!(io.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn unacknowledged_arrival_retries_the_original_timestamp_and_stops_on_ack() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let io = FakeIo::default();
    let request = movement(expect, 1, &[]);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&request).unwrap(),
            request.len(),
        )
        .await
        .unwrap();
    let initial = io.sent.lock().unwrap()[0].clone();
    tokio::time::sleep(RESEND_AFTER + Duration::from_millis(10)).await;
    state.maintain(&io).await.unwrap();
    let mut expected_retry = initial;
    expected_retry[0] |= ferroan_protocol::PacketFlags::RESENT;
    assert_eq!(io.sent.lock().unwrap()[1], expected_retry);
    let ack = PacketAck {
        omitted_trailing_blocks: 0,
        packets: vec![packet_ack::Packets { id: 1 }],
    };
    let request = wire(&ack, 2, &[]);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&request).unwrap(),
            request.len(),
        )
        .await
        .unwrap();
    assert!(state.unacked.is_empty());
    tokio::time::sleep(RESEND_AFTER + Duration::from_millis(10)).await;
    state.maintain(&io).await.unwrap();
    assert_eq!(
        io.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|bytes| parse_packet(bytes).unwrap().id
                == ferroan_protocol::msgs::AgentMovementComplete::ID)
            .count(),
        2
    );
}

#[derive(Default)]
struct FakeIo {
    incoming: Mutex<VecDeque<Result<Vec<u8>, io::ErrorKind>>>,
    send_errors: Mutex<VecDeque<Option<io::ErrorKind>>>,
    sent: Mutex<Vec<Vec<u8>>>,
}

impl DatagramIo for FakeIo {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let item = self.incoming.lock().unwrap().pop_front();
        match item {
            Some(Ok(bytes)) => {
                assert!(bytes.len() <= buf.len());
                buf[..bytes.len()].copy_from_slice(&bytes);
                Ok((bytes.len(), peer()))
            }
            Some(Err(kind)) => Err(io::Error::from(kind)),
            None => std::future::pending().await,
        }
    }

    async fn send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<usize> {
        assert_eq!(to, peer());
        match self.send_errors.lock().unwrap().pop_front().flatten() {
            Some(kind) => Err(io::Error::from(kind)),
            None => {
                self.sent.lock().unwrap().push(buf.to_vec());
                Ok(buf.len())
            }
        }
    }
}

fn peer() -> SocketAddr {
    "127.0.0.1:9000".parse().unwrap()
}

fn captured_config() -> HandshakeConfig {
    // Settings of the captured public harness region. CacheID is the recorded
    // randomly generated region setting; it is not normalized out of the test.
    let mut config =
        HandshakeConfig::for_region(Uuid::from_u128(0x6d1a7f4e0c3b4f539a1e5f0e3c2a9b01));
    config.sim_name = "Ferroan Test".to_string();
    config.estate_owner = Uuid::from_u128(0x4f0c8d2e7a514b8e9c3d2e6f1a0b7c55);
    config.cache_id = Uuid::from_u128(0xc44d3638a934459eaa982a71df4ccd6b);
    config
}

fn identity(agent_id: Uuid) -> CircuitExpect {
    CircuitExpect {
        agent_id,
        session_id: Uuid::from_u128(2),
        circuit_code: 1,
        region_id: captured_config().region_id,
    }
}

fn wire<M: Msg>(message: &M, sequence: u32, acks: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    build_packet(message, sequence, true, false, acks, &mut bytes);
    bytes
}

fn use_circuit(expect: CircuitExpect) -> Vec<u8> {
    let mut message = UseCircuitCode::default();
    message.circuit_code.id = expect.agent_id;
    message.circuit_code.session_id = expect.session_id;
    message.circuit_code.code = expect.circuit_code;
    wire(&message, 1, &[])
}

fn limit_hits(id: &str, action: &str) -> u64 {
    let prefix = format!("ferroan_limit_hits_total{{limit=\"{id}\",action=\"{action}\"}} ");
    ferroan_diag::metrics()
        .lines()
        .find_map(|line| line.strip_prefix(&prefix)?.parse().ok())
        .unwrap_or(0)
}

#[test]
fn a_full_circuit_inbox_reports_and_keeps_the_first_128_datagrams() {
    let inbox = Inbox::new();
    let before = limit_hits(UDP_CIRCUIT_INBOX, "refuse");
    for n in 0..=128 {
        inbox.push(Datagram {
            from: peer(),
            bytes: vec![n],
        });
    }
    assert_eq!(limit_hits(UDP_CIRCUIT_INBOX, "refuse"), before + 1);
    for n in 0..128 {
        assert_eq!(inbox.pop().unwrap().bytes, [n]);
    }
    assert!(inbox.pop().is_none());
}

#[tokio::test]
async fn pending_logins_keep_all_61_and_report_only_an_eviction() {
    let server = CircuitServer::bind().await.unwrap();
    let shared = &server.inner.shared;
    let before = limit_hits(UDP_PENDING_LOGINS, "drop");
    let submit = |port: u16| {
        let from = SocketAddr::from(([127, 0, 0, 1], port));
        assert_eq!(
            route(
                shared,
                from,
                use_circuit(identity(Uuid::from_u128(u128::from(port))))
            ),
            Some((from, 1))
        );
    };
    for port in 1..=61 {
        submit(port);
    }
    submit(1); // An existing endpoint moves to the back without displacing another.
    assert_eq!(limit_hits(UDP_PENDING_LOGINS, "drop"), before);
    submit(62);
    assert_eq!(limit_hits(UDP_PENDING_LOGINS, "drop"), before + 1);
    let pending = &shared.fan.lock().unwrap().pending;
    let ports: Vec<_> = pending.iter().map(|d| d.from.port()).collect();
    assert_eq!(ports, (3..=61).chain([1, 62]).collect::<Vec<_>>());
}

#[tokio::test]
async fn a_pending_login_keeps_the_movement_burst_until_its_serve() {
    let server = CircuitServer::bind().await.unwrap();
    let expect = identity(Uuid::from_u128(7));
    let from = SocketAddr::from(([127, 0, 0, 1], 4400));
    let opened = use_circuit(expect);
    let first = movement(expect, 2, &[]);
    let second = movement(expect, 3, &[]);
    {
        let shared = &server.inner.shared;
        assert_eq!(route(shared, from, opened.clone()), Some((from, 1)));
        assert_eq!(route(shared, from, first.clone()), None);
        assert_eq!(route(shared, from, second.clone()), None);
        // A resent UseCircuitCode is acknowledged and does not join the burst.
        assert_eq!(route(shared, from, opened.clone()), Some((from, 1)));
        let other = SocketAddr::from(([127, 0, 0, 1], 4401));
        assert_eq!(route(shared, other, first.clone()), None);
        let pending = &shared.fan.lock().unwrap().pending;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].packets.len(), 3);
    }
    let session = server.open_session(expect);
    let mut buf = [0u8; 2048];
    let mut taken = Vec::new();
    while let Some((len, source)) = session.try_recv_from(&mut buf).unwrap() {
        assert_eq!(source, from);
        taken.push(buf[..len].to_vec());
    }
    assert_eq!(taken, vec![opened, first, second]);
}

fn relogin(agent: u128, session: u128, code: u32) -> CircuitExpect {
    CircuitExpect {
        agent_id: Uuid::from_u128(agent),
        session_id: Uuid::from_u128(session),
        circuit_code: code,
        region_id: captured_config().region_id,
    }
}

/// A UseCircuitCode nobody claimed must not ack and drop the next login
/// from the same socket.
#[tokio::test]
async fn a_new_use_circuit_code_reaches_a_waiter_while_the_old_login_is_pending() {
    let server = CircuitServer::bind().await.unwrap();
    let from = SocketAddr::from(([127, 0, 0, 1], 4600));
    let stale = relogin(21, 210, 21);
    let fresh = relogin(21, 211, 22);
    let shared = &server.inner.shared;
    assert_eq!(route(shared, from, use_circuit(stale)), Some((from, 1)));
    let session = server.open_session(fresh);
    assert!(route(shared, from, use_circuit(fresh)).is_none());
    assert!(shared.fan.lock().unwrap().bound.contains_key(&from));
    let mut buf = [0u8; 2048];
    let (len, source) = session.try_recv_from(&mut buf).unwrap().unwrap();
    assert_eq!(source, from);
    assert_eq!(&buf[..len], &use_circuit(fresh));
    assert!(shared.fan.lock().unwrap().pending.is_empty());
}

/// The new code can arrive before its serve is registered. It replaces the
/// unclaimed burst instead of being acknowledged and dropped.
#[tokio::test]
async fn a_new_use_circuit_code_replaces_a_pending_burst_before_its_serve() {
    let server = CircuitServer::bind().await.unwrap();
    let from = SocketAddr::from(([127, 0, 0, 1], 4601));
    let stale = relogin(23, 230, 23);
    let fresh = relogin(23, 231, 24);
    let shared = &server.inner.shared;
    assert_eq!(route(shared, from, use_circuit(stale)), Some((from, 1)));
    assert_eq!(route(shared, from, movement(stale, 2, &[])), None);
    let fresh_bytes = use_circuit(fresh);
    assert_eq!(route(shared, from, fresh_bytes.clone()), Some((from, 1)));
    {
        let pending = &shared.fan.lock().unwrap().pending;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].packets.len(), 1);
    }
    // A resend of the replacement, with a new sequence, is still only an ack.
    let mut resent = UseCircuitCode::default();
    resent.circuit_code.id = fresh.agent_id;
    resent.circuit_code.session_id = fresh.session_id;
    resent.circuit_code.code = fresh.circuit_code;
    assert_eq!(route(shared, from, wire(&resent, 9, &[])), Some((from, 9)));
    assert_eq!(shared.fan.lock().unwrap().pending[0].packets.len(), 1);
    let session = server.open_session(fresh);
    assert!(shared.fan.lock().unwrap().bound.contains_key(&from));
    let mut buf = [0u8; 2048];
    let (len, source) = session.try_recv_from(&mut buf).unwrap().unwrap();
    assert_eq!(source, from);
    assert_eq!(&buf[..len], &fresh_bytes);
    assert!(session.try_recv_from(&mut buf).unwrap().is_none());
}

/// Accept reserves the code before the lookups. The stored burst is still
/// claimed after the 60 second cache, and an unreserved entry is not.
#[tokio::test]
async fn a_reserved_login_keeps_its_burst_past_the_pending_ttl() {
    let server = CircuitServer::bind().await.unwrap();
    let from = SocketAddr::from(([127, 0, 0, 1], 4610));
    let expect = relogin(27, 270, 27);
    let opened = use_circuit(expect);
    let burst = movement(expect, 2, &[]);
    let hold = server.reserve_login(
        expect,
        Some(tokio::time::Instant::now() + std::time::Duration::from_secs(120)),
    );
    let shared = &server.inner.shared;
    assert_eq!(route(shared, from, opened.clone()), Some((from, 1)));
    assert_eq!(route(shared, from, burst.clone()), None);

    let other_from = SocketAddr::from(([127, 0, 0, 1], 4611));
    let other = relogin(29, 290, 29);
    assert_eq!(
        route(shared, other_from, use_circuit(other)),
        Some((other_from, 1))
    );
    {
        let mut fan = shared.fan.lock().unwrap();
        let stale = tokio::time::Instant::now()
            .checked_sub(PENDING_LOGIN_TTL + std::time::Duration::from_secs(1))
            .unwrap();
        for login in &mut fan.pending {
            login.seen = stale;
        }
    }

    let session = server.open_session(expect);
    // The hold is still alive. The serve itself releases the reservation.
    assert_eq!(server.reserved_logins(), 0);
    assert!(shared.fan.lock().unwrap().bound.contains_key(&from));
    assert!(shared.fan.lock().unwrap().pending.is_empty());
    let mut buf = [0u8; 2048];
    let mut taken = Vec::new();
    while let Some((len, source)) = session.try_recv_from(&mut buf).unwrap() {
        assert_eq!(source, from);
        taken.push(buf[..len].to_vec());
    }
    assert_eq!(taken, vec![opened, burst]);
    drop(hold);

    let late = relogin(28, 280, 28);
    let late_from = SocketAddr::from(([127, 0, 0, 1], 4612));
    let late_open = use_circuit(late);
    let late_burst = movement(late, 2, &[]);
    let late_hold = server.reserve_login(
        late,
        Some(
            tokio::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(1))
                .unwrap(),
        ),
    );
    assert_eq!(
        route(shared, late_from, late_open.clone()),
        Some((late_from, 1))
    );
    assert_eq!(route(shared, late_from, late_burst.clone()), None);
    {
        let mut fan = shared.fan.lock().unwrap();
        fan.pending[0].seen = tokio::time::Instant::now()
            .checked_sub(PENDING_LOGIN_TTL + std::time::Duration::from_secs(1))
            .unwrap();
    }
    let late_session = server.open_session(late);
    assert!(!shared.fan.lock().unwrap().bound.contains_key(&late_from));
    assert!(shared.fan.lock().unwrap().pending.is_empty());
    assert!(late_session.try_recv_from(&mut buf).unwrap().is_none());
    drop(late_hold);

    let dropped = relogin(31, 310, 31);
    let dropped_from = SocketAddr::from(([127, 0, 0, 1], 4613));
    let dropped_hold = server.reserve_login(dropped, None);
    assert_eq!(
        route(shared, dropped_from, use_circuit(dropped)),
        Some((dropped_from, 1))
    );
    {
        let mut fan = shared.fan.lock().unwrap();
        fan.pending[0].seen = tokio::time::Instant::now()
            .checked_sub(PENDING_LOGIN_TTL + std::time::Duration::from_secs(1))
            .unwrap();
    }
    drop(dropped_hold);
    let dropped_session = server.open_session(dropped);
    assert!(!shared.fan.lock().unwrap().bound.contains_key(&dropped_from));
    assert!(shared.fan.lock().unwrap().pending.is_empty());
    assert!(dropped_session.try_recv_from(&mut buf).unwrap().is_none());
}

#[tokio::test]
async fn an_expired_pending_login_is_not_claimed() {
    let server = CircuitServer::bind().await.unwrap();
    let from = SocketAddr::from(([127, 0, 0, 1], 4602));
    let stale = relogin(25, 250, 25);
    let shared = &server.inner.shared;
    assert_eq!(route(shared, from, use_circuit(stale)), Some((from, 1)));
    route(shared, from, movement(stale, 2, &[]));
    {
        let mut fan = shared.fan.lock().unwrap();
        fan.pending[0].seen = tokio::time::Instant::now()
            .checked_sub(PENDING_LOGIN_TTL + std::time::Duration::from_secs(1))
            .unwrap();
    }
    let session = server.open_session(stale);
    assert!(!shared.fan.lock().unwrap().bound.contains_key(&from));
    assert!(shared.fan.lock().unwrap().pending.is_empty());
    let mut buf = [0u8; 64];
    assert!(session.try_recv_from(&mut buf).unwrap().is_none());
}

#[tokio::test]
async fn a_pending_burst_refuses_past_32_datagrams() {
    let server = CircuitServer::bind().await.unwrap();
    let from = SocketAddr::from(([127, 0, 0, 1], 4603));
    let expect = relogin(26, 260, 26);
    let shared = &server.inner.shared;
    let before = limit_hits(UDP_PENDING_LOGIN_BURST, "refuse");
    assert_eq!(route(shared, from, use_circuit(expect)), Some((from, 1)));
    for sequence in 2..=PENDING_BURST_CAP as u32 {
        assert_eq!(route(shared, from, movement(expect, sequence, &[])), None);
    }
    assert_eq!(route(shared, from, movement(expect, 100, &[])), None);
    assert_eq!(limit_hits(UDP_PENDING_LOGIN_BURST, "refuse"), before + 1);
    let pending = &shared.fan.lock().unwrap().pending;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].packets.len(), PENDING_BURST_CAP);
    assert_eq!(
        parse_packet(pending[0].packets.back().unwrap())
            .unwrap()
            .sequence,
        PENDING_BURST_CAP as u32
    );
}

/// Login datagrams queued before the serve reads. `recv_from` waits until
/// `release`, then both it and `try_recv_from` take from the same queue.
struct HeldBurst {
    released: std::sync::atomic::AtomicBool,
    waiting: std::sync::atomic::AtomicBool,
    packets: Mutex<VecDeque<Vec<u8>>>,
    sent: Mutex<Vec<Vec<u8>>>,
    ready: Notify,
}

impl HeldBurst {
    fn new(packets: Vec<Vec<u8>>) -> Self {
        Self {
            released: std::sync::atomic::AtomicBool::new(false),
            waiting: std::sync::atomic::AtomicBool::new(false),
            packets: Mutex::new(packets.into()),
            sent: Mutex::new(Vec::new()),
            ready: Notify::new(),
        }
    }

    fn release(&self) {
        self.released
            .store(true, std::sync::atomic::Ordering::Release);
        self.ready.notify_one();
    }

    fn overlay_count(&self) -> usize {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|bytes| {
                parse_packet(bytes)
                    .is_ok_and(|packet| packet.id == ferroan_protocol::msgs::ParcelOverlay::ID)
            })
            .count()
    }
}

impl DatagramIo for HeldBurst {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        loop {
            if self.released.load(std::sync::atomic::Ordering::Acquire)
                && let Some(bytes) = self.packets.lock().unwrap().pop_front()
            {
                assert!(bytes.len() <= buf.len());
                buf[..bytes.len()].copy_from_slice(&bytes);
                return Ok((bytes.len(), peer()));
            }
            self.waiting
                .store(true, std::sync::atomic::Ordering::Release);
            self.ready.notified().await;
        }
    }

    fn try_recv_from(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
        if !self.released.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(None);
        }
        let Some(bytes) = self.packets.lock().unwrap().pop_front() else {
            return Ok(None);
        };
        assert!(bytes.len() <= buf.len());
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok(Some((bytes.len(), peer())))
    }

    async fn send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<usize> {
        assert_eq!(to, peer());
        self.sent.lock().unwrap().push(buf.to_vec());
        Ok(buf.len())
    }
}

fn land_throttle(expect: CircuitExpect, land_bits: f32, sequence: u32) -> Vec<u8> {
    // Resend, Land, Wind, Cloud, Task, Texture, Asset. The side channels sit
    // on their floors so the 0.125 scale does not shrink Land.
    let rates = [8_000.0, land_bits, 400.0, 400.0, 8_000.0, 8_000.0, 8_000.0];
    let mut blob = Vec::with_capacity(28);
    for rate in rates {
        blob.extend(rate.to_le_bytes());
    }
    let mut message = AgentThrottle::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    message.agent_data.circuit_code = expect.circuit_code;
    message.throttle.throttles = blob;
    wire(&message, sequence, &[])
}

fn overlay_wire_len(len: usize) -> usize {
    let mut message = ferroan_protocol::msgs::ParcelOverlay::default();
    message.parcel_data.sequence_id = 1;
    message.parcel_data.data = vec![0x11; len];
    let payload = encode_payload(&message).unwrap();
    ferroan_protocol::packet::raw_len(&payload.bytes, payload.coding == Coding::Zero)
}

/// The three login datagrams are queued before the serve reads them. The
/// throttle clock has already moved, so a maintain between movement and the
/// throttle would empty the default Land burst. Only the first overlay leaves
/// until Land refills.
#[tokio::test(start_paused = true)]
async fn the_login_burst_applies_land_before_the_first_overlay_pass() {
    let expect = identity(Uuid::from_u128(1));
    let wire_len = overlay_wire_len(24);
    assert!(
        wire_len > 10 && wire_len * 2 <= 91 && wire_len * 3 > 91,
        "overlay datagram is {wire_len} bytes; the slow burst is 10 and the default burst is 91"
    );
    let overlay = (1..=3)
        .map(|sequence| crate::ParcelOverlayChunk {
            sequence_id: sequence,
            data: vec![0x11; 24],
        })
        .collect();
    let arrival = ArrivalConfig {
        overlay,
        ..ArrivalConfig::default()
    };
    let packets = vec![
        use_circuit(expect),
        movement(expect, 2, &[]),
        land_throttle(expect, 8_000.0, 3),
    ];
    let io = Arc::new(HeldBurst::new(packets));
    let serving_io = Arc::clone(&io);
    let serving = tokio::spawn(async move {
        serve_socket(
            serving_io.as_ref(),
            expect,
            captured_config(),
            arrival,
            CircuitTiming::default(),
        )
        .await
    });
    for _ in 0..16 {
        if io.waiting.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        io.waiting.load(std::sync::atomic::Ordering::Acquire),
        "the serve did not reach its first read"
    );
    tokio::time::advance(std::time::Duration::from_millis(50)).await;
    io.release();
    let mut first = 0;
    for _ in 0..32 {
        tokio::task::yield_now().await;
        let count = io.overlay_count();
        if count == first {
            continue;
        }
        first = count;
        if count >= 1 {
            // One more quiet poll, so a second chunk in the same pass is visible.
            tokio::task::yield_now().await;
            first = io.overlay_count();
            break;
        }
    }
    assert_eq!(first, 1, "the first pass let {} overlay chunks out", first);
    tokio::time::advance(std::time::Duration::from_millis(500)).await;
    let mut later = first;
    for _ in 0..32 {
        tokio::task::yield_now().await;
        later = io.overlay_count();
    }
    assert_eq!(
        later, 2,
        "Land should release the next chunk and hold the last"
    );
    serving.abort();
}

/// `try_recv_from` never goes empty. Without a drain cap the serve never
/// returns to the stop check.
struct FloodIo {
    opened: Mutex<Option<Vec<u8>>>,
    flood: Vec<u8>,
    sent: Mutex<Vec<Vec<u8>>>,
}

impl DatagramIo for FloodIo {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let bytes = self
            .opened
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| self.flood.clone());
        assert!(bytes.len() <= buf.len());
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok((bytes.len(), peer()))
    }

    fn try_recv_from(&self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
        assert!(self.flood.len() <= buf.len());
        buf[..self.flood.len()].copy_from_slice(&self.flood);
        Ok(Some((self.flood.len(), peer())))
    }

    async fn send_to(&self, buf: &[u8], to: SocketAddr) -> io::Result<usize> {
        assert_eq!(to, peer());
        self.sent.lock().unwrap().push(buf.to_vec());
        tokio::task::yield_now().await;
        Ok(buf.len())
    }
}

#[tokio::test]
async fn a_full_inbox_does_not_starve_a_stop() {
    let expect = identity(Uuid::from_u128(1));
    let mut ping = ferroan_protocol::msgs::StartPingCheck::default();
    ping.ping_id.ping_id = 3;
    let io = Arc::new(FloodIo {
        opened: Mutex::new(Some(use_circuit(expect))),
        flood: wire(&ping, 4, &[]),
        sent: Mutex::new(Vec::new()),
    });
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving_io = Arc::clone(&io);
    let serving = tokio::spawn(async move {
        serve_socket_reporting(
            serving_io.as_ref(),
            expect,
            captured_config(),
            ArrivalConfig::default(),
            CircuitTiming::default(),
            None,
            SceneThrottle::stock(),
            ServeExtra {
                root_arrival: None,
                connected: None,
                pending: None,
                stop: Some(stop_rx),
                chat: None,
                login_scene: None,
                names: None,
            },
        )
        .await
    });
    for _ in 0..1_000 {
        let seen = io.sent.lock().unwrap().iter().any(|bytes| {
            parse_packet(bytes)
                .is_ok_and(|packet| packet.id == ferroan_protocol::msgs::RegionHandshake::ID)
        });
        if seen {
            break;
        }
        tokio::task::yield_now().await;
    }
    stop_tx.send(CircuitStop::Shutdown).unwrap();
    let finished = tokio::time::timeout(std::time::Duration::from_secs(5), serving)
        .await
        .expect("a full inbox held the serve past the stop");
    assert!(matches!(finished.unwrap(), Err(CircuitError::Closed)));
}

#[tokio::test]
async fn an_agent_update_and_always_run_are_forwarded() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::clone(&seen);
    state.login_scene = Some(Arc::new(move |_agent, call| {
        store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(call);
        Ok(SceneReply::Applied).into()
    }));
    let mut message = ferroan_protocol::msgs::AgentUpdate::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    message.agent_data.body_rotation = glam::Quat::from_xyzw(0.0, 0.0, 0.0, 1.0);
    message.agent_data.head_rotation = glam::Quat::from_xyzw(0.0, 0.0, 0.0, 1.0);
    message.agent_data.control_flags = 1;
    message.agent_data.far = 64.0;
    message.agent_data.flags = 0x80;
    let bytes = wire(&message, 4, &[]);
    let io = FakeIo::default();
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    let mut run = ferroan_protocol::msgs::SetAlwaysRun::default();
    run.agent_data.agent_id = expect.agent_id;
    run.agent_data.session_id = expect.session_id;
    run.agent_data.always_run = true;
    let bytes = wire(&run, 5, &[]);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    let seen = seen.lock().unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(seen.len(), 2);
    match &seen[0] {
        SceneCall::AgentUpdate(update) => {
            assert_eq!(update.sequence, 4);
            assert!(!update.child && !update.in_transit);
            assert_eq!(update.control_flags, 1);
            assert_eq!(update.far, 64.0);
            assert_eq!(update.flags, 0x80);
            assert_eq!(update.body_rotation, [0.0, 0.0, 0.0, 1.0]);
            assert!(update.received <= std::time::Instant::now());
        }
        other => panic!("expected an agent update, got {other:?}"),
    }
    assert!(matches!(seen[1], SceneCall::AlwaysRun(true)));
}

/// `HandleAgentAnimation`: every block's id and `StartAnim`, in packet
/// order, as one post; a nil id goes too, as C# invokes it. The ids must be
/// the circuit's (`NeedValidateIDs`).
#[tokio::test]
async fn an_agent_animation_is_forwarded_block_by_block() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::clone(&seen);
    state.login_scene = Some(Arc::new(move |_agent, call| {
        store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(call);
        Ok(SceneReply::Applied).into()
    }));
    let mut message = ferroan_protocol::msgs::AgentAnimation::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    for (id, start) in [(7u128, true), (0, true), (8, false)] {
        message
            .animation_list
            .push(ferroan_protocol::msgs::agent_animation::AnimationList {
                anim_id: Uuid::from_u128(id),
                start_anim: start,
            });
    }
    let bytes = wire(&message, 4, &[]);
    let packet = parse_packet(&bytes).unwrap();
    assert!(valid_packet_ids(&packet, &expect));
    let io = FakeIo::default();
    state
        .handle(&io, peer(), &expect, &packet, bytes.len())
        .await
        .unwrap();
    let seen = seen.lock().unwrap_or_else(|poison| poison.into_inner());
    match seen.as_slice() {
        [SceneCall::AgentAnimation(changes)] => assert_eq!(
            changes.as_slice(),
            [
                (Uuid::from_u128(7), true),
                (Uuid::nil(), true),
                (Uuid::from_u128(8), false)
            ]
        ),
        other => panic!("expected one animation post, got {other:?}"),
    }
    let mut forged = message.clone();
    forged.agent_data.session_id = Uuid::from_u128(99);
    let bytes = wire(&forged, 5, &[]);
    assert!(!valid_packet_ids(&parse_packet(&bytes).unwrap(), &expect));
}

#[tokio::test]
async fn a_local_teleport_without_a_scene_cannot_be_processed() {
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let (mut state, expect, io) = seated_teleport(region);
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0], 2);
    handle_packet(&mut state, &io, &expect, &request).await;
    state.maintain(&io).await.unwrap();
    assert_eq!(
        message_ids(&io),
        vec![ferroan_protocol::msgs::TeleportFailed::ID]
    );
}

#[tokio::test]
async fn the_scene_flags_go_on_the_local_teleport_packets() {
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let (mut state, expect, io) = seated_teleport(region);
    state.login_scene = Some(Arc::new(|_agent, call| match call {
        SceneCall::Teleport(request) => {
            assert_eq!(request.flags, crate::teleport::VIA_LOCATION);
            Ok(SceneReply::Moved(SceneMove {
                position: [41.0, 201.0, 31.0],
                look_at: [1.0, 0.0, 0.0],
                flags: request.flags | crate::teleport::IS_FLYING,
            }))
            .into()
        }
        _ => Err(crate::circuit::SceneRefused).into(),
    }));
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0], 2);
    handle_packet(&mut state, &io, &expect, &request).await;
    state.maintain(&io).await.unwrap();
    let sent = io.sent.lock().unwrap();
    let flags: Vec<u32> = sent
        .iter()
        .filter_map(|bytes| {
            let packet = parse_packet(bytes).unwrap();
            if packet.id == ferroan_protocol::msgs::TeleportStart::ID {
                Some(
                    packet
                        .decode::<ferroan_protocol::msgs::TeleportStart>()
                        .unwrap()
                        .info
                        .teleport_flags,
                )
            } else if packet.id == ferroan_protocol::msgs::TeleportLocal::ID {
                let local = packet
                    .decode::<ferroan_protocol::msgs::TeleportLocal>()
                    .unwrap();
                assert_eq!(local.info.position.to_array(), [41.0, 201.0, 31.0]);
                Some(local.info.teleport_flags)
            } else {
                None
            }
        })
        .collect();
    let expected = crate::teleport::VIA_LOCATION | crate::teleport::IS_FLYING;
    assert_eq!(flags, vec![expected, expected]);
}

#[tokio::test]
async fn a_refused_always_run_is_kept_and_sent_again_before_the_next_update() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let full = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (store, refuse) = (Arc::clone(&seen), Arc::clone(&full));
    state.login_scene = Some(Arc::new(move |_agent, call| {
        if refuse.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(crate::circuit::SceneRefused).into();
        }
        store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(call);
        Ok(SceneReply::Applied).into()
    }));
    let io = FakeIo::default();
    for (always_run, sequence) in [(true, 4), (false, 5), (true, 6)] {
        let mut run = ferroan_protocol::msgs::SetAlwaysRun::default();
        run.agent_data.agent_id = expect.agent_id;
        run.agent_data.session_id = expect.session_id;
        run.agent_data.always_run = always_run;
        let bytes = wire(&run, sequence, &[]);
        state
            .handle(
                &io,
                peer(),
                &expect,
                &parse_packet(&bytes).unwrap(),
                bytes.len(),
            )
            .await
            .unwrap();
    }
    // Only the latest refused value waits.
    assert_eq!(state.always_run, Some((expect.agent_id, true)));
    full.store(false, std::sync::atomic::Ordering::Relaxed);
    let mut message = ferroan_protocol::msgs::AgentUpdate::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    let bytes = wire(&message, 7, &[]);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    assert_eq!(state.always_run, None);
    let seen = seen.lock().unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(seen.len(), 2);
    assert!(matches!(seen[0], SceneCall::AlwaysRun(true)));
    assert!(matches!(seen[1], SceneCall::AgentUpdate(_)));
}

#[tokio::test]
async fn an_update_waits_while_the_always_run_before_it_is_refused() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let refuse_run = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (store, refuse) = (Arc::clone(&seen), Arc::clone(&refuse_run));
    state.login_scene = Some(Arc::new(move |_agent, call| {
        if matches!(call, SceneCall::AlwaysRun(_))
            && refuse.load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(crate::circuit::SceneRefused).into();
        }
        store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(call);
        Ok(SceneReply::Applied).into()
    }));
    let io = FakeIo::default();
    let mut run = ferroan_protocol::msgs::SetAlwaysRun::default();
    run.agent_data.agent_id = expect.agent_id;
    run.agent_data.session_id = expect.session_id;
    run.agent_data.always_run = true;
    let bytes = wire(&run, 4, &[]);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    for sequence in [5, 6] {
        let mut message = ferroan_protocol::msgs::AgentUpdate::default();
        message.agent_data.agent_id = expect.agent_id;
        message.agent_data.session_id = expect.session_id;
        let bytes = wire(&message, sequence, &[]);
        state
            .handle(
                &io,
                peer(),
                &expect,
                &parse_packet(&bytes).unwrap(),
                bytes.len(),
            )
            .await
            .unwrap();
    }
    // The frame would take the update, but not ahead of the always-run.
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(
        state.pending_update.map(|(_, update)| update.sequence),
        Some(6)
    );
    refuse_run.store(false, std::sync::atomic::Ordering::Relaxed);
    state.maintain(&io).await.unwrap();
    assert!(state.always_run.is_none() && state.pending_update.is_none());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(matches!(seen[0], SceneCall::AlwaysRun(true)));
    assert!(matches!(&seen[1], SceneCall::AgentUpdate(update) if update.sequence == 6));
}

#[tokio::test]
async fn a_refused_update_is_held_and_posted_by_maintenance() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    state.login_scene = Some(Arc::new(move |_agent, call| {
        assert!(matches!(call, SceneCall::AgentUpdate(_)));
        if counted.fetch_add(1, Ordering::Relaxed) == 0 {
            Err(crate::circuit::SceneRefused).into()
        } else {
            Ok(SceneReply::Applied).into()
        }
    }));
    let mut message = ferroan_protocol::msgs::AgentUpdate::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    let bytes = wire(&message, 4, &[]);
    let io = FakeIo::default();
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    assert!(state.pending_update.is_some());
    state.maintain(&io).await.unwrap();
    assert!(state.pending_update.is_none());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn a_refused_always_run_is_sent_again_by_maintenance() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    state.login_scene = Some(Arc::new(move |_agent, call| {
        assert!(matches!(call, SceneCall::AlwaysRun(true)));
        // The first post is refused, the second is taken.
        if counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
            Err(crate::circuit::SceneRefused).into()
        } else {
            Ok(SceneReply::Applied).into()
        }
    }));
    let mut run = ferroan_protocol::msgs::SetAlwaysRun::default();
    run.agent_data.agent_id = expect.agent_id;
    run.agent_data.session_id = expect.session_id;
    run.agent_data.always_run = true;
    let bytes = wire(&run, 4, &[]);
    let io = FakeIo::default();
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    assert!(state.always_run.is_some());
    state.maintain(&io).await.unwrap();
    assert!(state.always_run.is_none());
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
}

#[tokio::test]
async fn chat_on_another_channel_does_not_ask_the_scene() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let chat = RegionChat::new();
    state.chat = Some(chat.seat(expect.agent_id, "Test User".into()));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    state.login_scene = Some(Arc::new(move |_agent, _call| {
        counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(SceneReply::Places(HashMap::new())).into()
    }));
    let io = FakeIo::default();
    for (channel, sequence, expected_calls) in [(5, 4, 0), (0, 5, 1), (i32::MAX, 6, 2)] {
        let mut message = ChatFromViewer::default();
        message.agent_data.agent_id = expect.agent_id;
        message.agent_data.session_id = expect.session_id;
        message.chat_data.channel = channel;
        message.chat_data.type_ = 1;
        message.chat_data.message = ferroan_protocol::str_field("hello");
        let bytes = wire(&message, sequence, &[]);
        state
            .handle(
                &io,
                peer(),
                &expect,
                &parse_packet(&bytes).unwrap(),
                bytes.len(),
            )
            .await
            .unwrap();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            expected_calls
        );
    }
}

#[tokio::test]
async fn a_viewer_connecting_during_the_pose_lookup_does_not_hear_the_line() {
    let expect = identity(Uuid::from_u128(1));
    let late_id = Uuid::from_u128(2);
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let chat = RegionChat::new();
    state.chat = Some(chat.seat(expect.agent_id, "Test User".into()));
    let late = Arc::new(Mutex::new(None));
    let (hub, slot) = (chat.clone(), Arc::clone(&late));
    state.login_scene = Some(Arc::new(move |_agent, _call| {
        *slot.lock().unwrap() = Some(hub.seat(late_id, "Chat Bot".into()));
        Ok(SceneReply::Places(HashMap::from([
            (expect.agent_id, [0.0; 3]),
            (late_id, [0.0; 3]),
        ])))
        .into()
    }));
    let mut message = ChatFromViewer::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    message.chat_data.type_ = 1;
    message.chat_data.message = ferroan_protocol::str_field("before connect");
    let bytes = wire(&message, 4, &[]);
    state
        .handle(
            &FakeIo::default(),
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    let mut seat = late.lock().unwrap().take().unwrap();
    assert!(matches!(
        seat.rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

fn logout(expect: CircuitExpect, acks: &[u32]) -> Vec<u8> {
    let mut message = LogoutRequest::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    wire(&message, 2, acks)
}

#[tokio::test]
async fn initial_ack_and_configured_handshake_match_csharp_datagrams() {
    let io = FakeIo::default();
    let config = captured_config();
    let expect = identity(config.estate_owner);
    let mut state = State::new(config, ArrivalConfig::default());
    let request = use_circuit(expect);
    assert!(
        !state
            .handle(
                &io,
                peer(),
                &expect,
                &parse_packet(&request).unwrap(),
                request.len()
            )
            .await
            .unwrap()
    );
    let sent = io.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], IMMEDIATE_ACK);
    assert_eq!(sent[1], HANDSHAKE);
    assert_eq!(state.sequence.peek(), 2);
    assert!(state.unacked.contains(1));
}

#[tokio::test]
async fn queued_ack_matches_csharp_bytes_and_consumes_a_sequence() {
    let (state, io) = drive_captured_inbound(false).await;
    let sent = io.sent.lock().unwrap();
    let actual = sent.last().unwrap();
    captured_wire_except_scene_sequence(actual, ESTABLISHED_ACK);
    assert_eq!(parse_packet(actual).unwrap().sequence, 3);
    assert_eq!(state.sequence.peek(), 4);
    assert!(state.pending_acks.is_empty());
    assert!(state.unacked.is_empty());
}

fn captured_wire_except_scene_sequence(actual: &[u8], captured: &[u8]) {
    // C# emits terrain/appearance/world packets that this login-only region
    // does not yet produce. Compare every captured wire byte except the four
    // outbound sequence bytes, whose allocation is asserted from driven flow.
    assert_eq!(actual.len(), captured.len());
    assert_eq!(actual[..1], captured[..1]);
    assert_eq!(actual[5..], captured[5..]);
}

fn captured_circuit() -> Vec<ferroan_capture::Record> {
    ferroan_capture::read_capture(include_bytes!("../../../fixtures/circuit/sequence.cap")).unwrap()
}

async fn drive_captured_inbound(through_logout: bool) -> (State, FakeIo) {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    let io = FakeIo::default();
    for record in captured_circuit() {
        let packet = parse_packet(&record.bytes).unwrap();
        if record.kind == ferroan_capture::Kind::UdpIn {
            if state
                .handle(&io, peer(), &expect, &packet, record.bytes.len())
                .await
                .unwrap()
            {
                assert!(through_logout);
                return (state, io);
            }
        } else if packet.id == PacketAck::ID && packet.sequence != 0 {
            // C# flushes this batch at the recorded point in the exchange.
            // Every queued ID must come from the captured incoming packets.
            state.flush_acks(&io, peer()).await.unwrap();
            if !through_logout {
                return (state, io);
            }
        }
    }
    panic!("capture lacks the requested established ACK/logout boundary");
}

#[tokio::test]
async fn logout_send_path_matches_csharp_bytes_and_finishes_without_ack() {
    let expect = identity(Uuid::from_u128(1));
    let (state, io) = drive_captured_inbound(true).await;
    {
        let sent = io.sent.lock().unwrap();
        let actual = sent.last().unwrap();
        captured_wire_except_scene_sequence(actual, LOGOUT);
        assert_eq!(parse_packet(actual).unwrap().sequence, 4);
        assert_eq!(state.sequence.peek(), 5);
        assert!(state.unacked.contains(4));
        assert!(sent.iter().all(|bytes| {
            let id = parse_packet(bytes).unwrap().id;
            id != ferroan_protocol::msgs::KickUser::ID
                && id != ferroan_protocol::msgs::DisableSimulator::ID
        }));
    }

    // With no incoming ACK at all, the real serving loop must still complete.
    let io = FakeIo::default();
    for record in captured_circuit()
        .into_iter()
        .filter(|record| record.kind == ferroan_capture::Kind::UdpIn)
    {
        let logout = parse_packet(&record.bytes).unwrap().id == LogoutRequest::ID;
        io.incoming.lock().unwrap().push_back(Ok(record.bytes));
        if logout {
            break;
        }
    }
    tokio::time::timeout(
        Duration::from_secs(1),
        serve_socket(
            &io,
            expect,
            captured_config(),
            ArrivalConfig::default(),
            crate::CircuitTiming::default(),
        ),
    )
    .await
    .expect("logout must not wait for an ACK")
    .unwrap();
    assert!(io.incoming.lock().unwrap().is_empty());
    let sent = io.sent.lock().unwrap();
    assert!(
        sent.iter()
            .any(|bytes| parse_packet(bytes).unwrap().id == LogoutReply::ID)
    );
    assert!(sent.iter().all(|bytes| {
        let id = parse_packet(bytes).unwrap().id;
        id != ferroan_protocol::msgs::KickUser::ID
            && id != ferroan_protocol::msgs::DisableSimulator::ID
    }));
}

#[test]
fn captured_disable_simulator_is_the_csharp_constructor_datagram() {
    let packet = parse_packet(include_bytes!(
        "../../../fixtures/circuit/disable-simulator.bin"
    ))
    .unwrap();
    assert_eq!(packet.id, ferroan_protocol::msgs::DisableSimulator::ID);
    assert_eq!(packet.flags, ferroan_protocol::PacketFlags::RELIABLE);
    assert_eq!(packet.sequence, 0);
    assert!(packet.body.is_empty());
}

#[test]
fn captured_kick_is_a_shutdown_notice_not_the_ordinary_logout_tail() {
    let kick = parse_packet(include_bytes!(
        "../../../fixtures/circuit/shutdown-kick-8583218.bin"
    ))
    .unwrap()
    .decode::<ferroan_protocol::msgs::KickUser>()
    .unwrap();
    assert_eq!(kick.user_info.reason, b"The simulator is going down.\0");
    // Scene.Close emits this exact notice during simulator shutdown. Ordinary
    // Scene.CloseAgent(false) does not call Kick; KillObject needs the avatar
    // LocalId/scene broadcast path planned in P2.0f.
}

#[tokio::test]
async fn receive_and_send_peer_errors_do_not_end_the_circuit() {
    let expect = identity(Uuid::from_u128(1));
    let io = FakeIo::default();
    io.incoming.lock().unwrap().extend([
        Err(io::ErrorKind::ConnectionReset),
        Err(io::ErrorKind::ConnectionRefused),
        Ok(use_circuit(expect)),
        Ok(logout(expect, &[])),
    ]);
    // Both the immediate ACK and reliable handshake fail. The authorized
    // circuit must remain available to receive logout and send its reply.
    io.send_errors.lock().unwrap().extend([
        Some(io::ErrorKind::ConnectionReset),
        Some(io::ErrorKind::ConnectionRefused),
    ]);
    serve_socket(
        &io,
        expect,
        captured_config(),
        ArrivalConfig::default(),
        crate::CircuitTiming::default(),
    )
    .await
    .unwrap();
    let sent = io.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let reply = parse_packet(&sent[0]).unwrap();
    assert_eq!(reply.id, LogoutReply::ID);
    assert!(reply.reliable());
}

#[tokio::test]
async fn failed_reliable_send_is_retained_and_retried() {
    let io = FakeIo::default();
    io.send_errors
        .lock()
        .unwrap()
        .extend([None, Some(io::ErrorKind::ConnectionRefused)]);
    let config = captured_config();
    let expect = identity(config.estate_owner);
    let mut state = State::new(config, ArrivalConfig::default());
    let request = use_circuit(expect);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&request).unwrap(),
            request.len(),
        )
        .await
        .unwrap();
    assert!(state.unacked.contains(1));
    assert_eq!(
        io.sent.lock().unwrap().len(),
        1,
        "only the ACK reached its peer"
    );
    tokio::time::sleep(RESEND_AFTER + Duration::from_millis(10)).await;
    state.maintain(&io).await.unwrap();
    let sent = io.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    let mut expected_resend = HANDSHAKE.to_vec();
    expected_resend[0] |= ferroan_protocol::PacketFlags::RESENT;
    assert_eq!(sent[1], expected_resend);
    let resent = parse_packet(&sent[1]).unwrap();
    assert_eq!(resent.id, ferroan_protocol::msgs::RegionHandshake::ID);
    assert_eq!(resent.sequence, 1);
    assert!(resent.resent());
    assert!(state.unacked.contains(1));
}

#[tokio::test]
async fn wrong_agent_or_session_cannot_acknowledge_the_window() {
    let io = FakeIo::default();
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    let request = use_circuit(expect);
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&request).unwrap(),
            request.len(),
        )
        .await
        .unwrap();
    assert!(state.unacked.contains(1));
    for wrong in [
        CircuitExpect {
            agent_id: Uuid::from_u128(99),
            ..expect
        },
        CircuitExpect {
            session_id: Uuid::from_u128(99),
            ..expect
        },
    ] {
        let request = logout(wrong, &[1]);
        assert!(
            !state
                .handle(
                    &io,
                    peer(),
                    &expect,
                    &parse_packet(&request).unwrap(),
                    request.len()
                )
                .await
                .unwrap()
        );
        assert!(state.unacked.contains(1));
        assert!(state.pending_acks.is_empty());
    }
    assert_eq!(io.sent.lock().unwrap().len(), 2);
    let request = logout(expect, &[1]);
    assert!(
        state
            .handle(
                &io,
                peer(),
                &expect,
                &parse_packet(&request).unwrap(),
                request.len()
            )
            .await
            .unwrap()
    );
    assert!(!state.unacked.contains(1));
}

#[tokio::test]
async fn login_arrival_reads_the_pose_back_from_the_scene() {
    let expect = identity(Uuid::from_u128(7));
    let config = ArrivalConfig {
        position: [10.0, 20.0, 30.0],
        avatar: Some(crate::AvatarWire::standing(
            expect.agent_id,
            1,
            "Test",
            "User",
        )),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    let io = Arc::new(FakeIo::default());
    let during = Arc::clone(&io);
    let order = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&order);
    state.login_scene = Some(Arc::new(move |agent, call| {
        // The fresh login is the arrival, then the parcel element. No
        // separate pose and release. The parcel call runs after
        // AgentMovementComplete is already on the socket.
        let mut steps = seen.lock().unwrap();
        match call {
            SceneCall::Arrive(pose, look_at) => {
                assert!(steps.is_empty(), "{steps:?}");
                assert!(during.sent.lock().unwrap().is_empty());
                steps.push("arrive");
                assert_eq!(agent, expect.agent_id);
                assert_eq!(look_at, [0.99, 0.042, 0.0]);
                assert_eq!(pose.position, [10.0, 20.0, 30.0]);
                assert_eq!(pose.rotation, LOGIN_ROTATION);
                assert_eq!(pose.velocity, [0.0, 0.0, 0.0]);
                assert_eq!(pose.control_flags, 0);
                assert_eq!(pose.draw_distance, LOGIN_DRAW_DISTANCE);
                let mut stored = pose;
                stored.position[2] = 41.5;
                Ok(SceneReply::Pose(stored)).into()
            }
            SceneCall::InitialLand => {
                assert_eq!(steps.as_slice(), ["arrive"]);
                steps.push("land");
                drop(steps);
                let sent = during.sent.lock().unwrap();
                assert!(
                    sent.iter().any(|bytes| {
                        parse_packet(bytes).ok().is_some_and(|packet| {
                            packet.id == ferroan_protocol::msgs::AgentMovementComplete::ID
                        })
                    }),
                    "AgentMovementComplete is already sent"
                );
                Ok(SceneReply::Applied).into()
            }
            _ => Err(crate::circuit::SceneRefused).into(),
        }
    }));
    state
        .send_arrival(io.as_ref(), peer(), &expect)
        .await
        .unwrap();
    assert_eq!(order.lock().unwrap().as_slice(), ["arrive", "land"]);
    release_throttled(&mut state, &io).await;
    assert_eq!(state.arrival.position, [10.0, 20.0, 30.0]);
    let sent = io.sent.lock().unwrap();
    let mut movement_z = None;
    let mut avatar_z = None;
    for bytes in sent.iter() {
        let packet = parse_packet(bytes).unwrap();
        if packet.id == ferroan_protocol::msgs::AgentMovementComplete::ID {
            let message = packet
                .decode::<ferroan_protocol::msgs::AgentMovementComplete>()
                .unwrap();
            movement_z = Some(message.data.position.z);
        }
        if packet.id == ferroan_protocol::msgs::ObjectUpdate::ID {
            let message = packet
                .decode::<ferroan_protocol::msgs::ObjectUpdate>()
                .unwrap();
            if let Some(object) = message
                .object_data
                .iter()
                .find(|object| object.p_code == 47)
            {
                let z = f32::from_le_bytes([
                    object.object_data[24],
                    object.object_data[25],
                    object.object_data[26],
                    object.object_data[27],
                ]);
                avatar_z = Some(z);
            }
        }
    }
    assert_eq!(movement_z, Some(41.5));
    assert_eq!(avatar_z, Some(41.5));
}

/// Ready viewer updates and the budgets supplied by the circuit.
#[derive(Default)]
struct BlockFeed {
    full: Mutex<Option<ferroan_protocol::msgs::object_update::ObjectData>>,
    blocks: Mutex<Vec<[u8; crate::AVATAR_TERSE_LEN]>>,
    materials: Mutex<Vec<Vec<u8>>>,
    budgets: Mutex<Vec<i32>>,
    packets: Mutex<Vec<crate::AvatarPacket>>,
}

impl crate::EntityUpdates for BlockFeed {
    fn has_updates(&self) -> bool {
        !self.blocks.lock().unwrap().is_empty()
            || self.full.lock().unwrap().is_some()
            || !self.materials.lock().unwrap().is_empty()
    }

    fn take_updates(&self, batch: &mut crate::TaskBatch) {
        self.budgets.lock().unwrap().push(batch.budget());
        let mut blocks = self.blocks.lock().unwrap();
        while batch.has_room() && !blocks.is_empty() {
            batch.push_terse(blocks.remove(0));
        }
        if batch.has_room()
            && let Some(full) = self.full.lock().unwrap().take()
        {
            batch.push_full(&full, None);
            for body in self.materials.lock().unwrap().drain(..) {
                batch.push_material(body);
            }
        }
    }

    fn take_avatar_packets(&self, max: usize) -> Vec<crate::AvatarPacket> {
        let mut packets = self.packets.lock().unwrap();
        let count = max.min(packets.len());
        packets.drain(..count).collect()
    }
}

/// The material body is the Pin8 datagram after its 6-byte header.
fn captured_material_payload() -> Vec<u8> {
    let bytes = include_bytes!("../../../fixtures/material-overrides/control.cap");
    let records = ferroan_capture::read_capture(bytes).unwrap();
    assert_eq!(records.len(), 1);
    let datagram = &records[0].bytes;
    let packet = parse_packet(datagram).unwrap();
    assert!(packet.reliable());
    assert!(!packet.resent());
    assert_eq!(packet.flags & ferroan_protocol::PacketFlags::ZEROCODED, 0);
    assert!(packet.acks.is_empty());
    datagram[ferroan_protocol::packet::HEADER_SIZE..].to_vec()
}

fn pin8_material_faces() -> [crate::Face<'static>; 2] {
    [
        crate::Face {
            te_index: 0,
            data: "{'bc':[r1,r0,r0,r1]}",
        },
        crate::Face {
            te_index: 2,
            data: "{'ds':1}",
        },
    ]
}

#[test]
fn the_encoder_matches_the_pin8_material_body() {
    let faces = pin8_material_faces();
    assert_eq!(
        crate::material_packet(1, 9, Some(&faces)).unwrap(),
        captured_material_payload()
    );
}

/// A full update, a terse block and the captured material body stay queued,
/// reliable, in that order. The material body is plain. Flipping its reliable
/// flag or its coding changes this result.
#[tokio::test]
async fn a_queued_material_payload_is_reliable_plain_and_follows_full_and_terse() {
    use ferroan_protocol::msgs::{
        GenericStreamingMessage, ImprovedTerseObjectUpdate, ObjectUpdate,
    };

    let faces = pin8_material_faces();
    let captured = crate::material_packet(1, 9, Some(&faces)).unwrap();
    assert_eq!(captured, captured_material_payload());
    let source = Arc::new(BlockFeed::default());
    *source.full.lock().unwrap() = Some(ferroan_protocol::msgs::object_update::ObjectData {
        id: 1,
        ..Default::default()
    });
    let mut block = [0u8; crate::AVATAR_TERSE_LEN];
    block[0..4].copy_from_slice(&1u32.to_le_bytes());
    source.blocks.lock().unwrap().push(block);
    source.materials.lock().unwrap().push(captured.clone());
    let arrival = ArrivalConfig {
        updates: Some(crate::EntityFeed {
            updates: source,
            wake: Arc::new(Notify::new()),
        }),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), arrival);
    state.peer = Some(peer());
    let io = FakeIo::default();
    let now = state.throttle_clock.now_secs();
    assert!(state.throttle.try_spend(TASK, 10_000_000, now));
    state.maintain(&io).await.unwrap();
    let ids: Vec<_> = state
        .pending_task
        .iter()
        .map(|payload| ferroan_protocol::MsgId::decode(&payload.bytes).unwrap().0)
        .collect();
    let material = ids
        .iter()
        .rposition(|id| *id == GenericStreamingMessage::ID)
        .unwrap();
    let terse = ids
        .iter()
        .rposition(|id| *id == ImprovedTerseObjectUpdate::ID)
        .unwrap();
    assert_eq!(material, ids.len() - 1);
    assert!(terse < material);
    assert!(!ids[..terse].is_empty());
    assert!(ids[..terse].iter().all(|id| *id == ObjectUpdate::ID));
    assert!(
        ids[terse..material]
            .iter()
            .all(|id| *id == ImprovedTerseObjectUpdate::ID)
    );
    for (index, payload) in state.pending_task.iter().enumerate() {
        assert!(payload.reliable);
        let coding = if index < terse {
            Coding::Preencoded
        } else if index < material {
            Coding::Zero
        } else {
            Coding::Plain
        };
        assert!(payload.coding == coding);
    }
    assert_eq!(state.pending_task.back().unwrap().bytes, captured);
}

/// A circuit whose reliable window is full takes no avatar packets: they stay
/// in the feed, where its bound applies. Freed room takes only that many.
#[tokio::test]
async fn a_stalled_circuit_leaves_avatar_packets_in_the_feed() {
    let source = Arc::new(BlockFeed::default());
    let arrival = ArrivalConfig {
        updates: Some(crate::EntityFeed {
            updates: source.clone(),
            wake: Arc::new(Notify::new()),
        }),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), arrival);
    state.peer = Some(peer());
    let io = FakeIo::default();
    // Exhaust the task bucket so the waiting packets stay.
    let now = state.throttle_clock.now_secs();
    assert!(state.throttle.try_spend(TASK, 10_000_000, now));
    let mut ping = CompletePingCheck::default();
    ping.ping_id.ping_id = 1;
    for _ in 0..MAX_UNACKED {
        state.enqueue_task(&ping).unwrap();
    }
    {
        let mut packets = source.packets.lock().unwrap();
        for id in 0..5 {
            for kill in crate::kill_packets(&[id]) {
                packets.push(crate::AvatarPacket::Kill(kill));
            }
        }
    }
    state.maintain(&io).await.unwrap();
    assert_eq!(source.packets.lock().unwrap().len(), 5);
    assert_eq!(state.pending_task.len(), MAX_UNACKED);
    // Two slots free up: two packets move, the oldest first.
    state.pending_task.pop_back();
    state.pending_task.pop_back();
    state.maintain(&io).await.unwrap();
    assert_eq!(source.packets.lock().unwrap().len(), 3);
    assert_eq!(state.pending_task.len(), MAX_UNACKED);
    let crate::AvatarPacket::Kill(next) = &source.packets.lock().unwrap()[0] else {
        panic!("a kill");
    };
    assert_eq!(next.object_data[0].id, 2);
}

/// Another avatar's full update, appearance, animations and kill leave in
/// the order the region queued them, ahead of a terse block: the full update
/// reliable and zero-coded, the appearance the same, the animations
/// (`AvatarAnimationHeader`: reliable, High 20) and the kill reliable and
/// unencoded.
#[tokio::test]
async fn avatar_packets_leave_in_order_before_the_terse_blocks() {
    let source = Arc::new(BlockFeed::default());
    let arrival = ArrivalConfig {
        region_handle: 0x0003_e800_0003_e800,
        updates: Some(crate::EntityFeed {
            updates: source.clone(),
            wake: Arc::new(Notify::new()),
        }),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), arrival);
    state.peer = Some(peer());
    let io = FakeIo::default();
    let mut other = crate::AvatarWire::standing(Uuid::from_u128(9), 9, "Chat", "Bot");
    other.local_id = 77;
    let live = crate::LiveAvatar {
        collision_plane: [0.0, 0.0, 0.0, 1.0],
        position: [128.0, 128.0, 25.0],
        velocity: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };
    {
        let mut packets = source.packets.lock().unwrap();
        packets.push(crate::AvatarPacket::Full(
            other.live_update(0x0003_e800_0003_e800, &live),
        ));
        packets.push(crate::AvatarPacket::Appearance(other.appearance()));
        packets.push(crate::AvatarPacket::Animation(crate::avatar_animation(
            Uuid::from_u128(1),
            Uuid::from_u128(9),
            &[crate::Anim {
                animation: Uuid::from_u128(5),
                object_id: Uuid::from_u128(9),
                sequence: 3,
            }],
        )));
        for kill in crate::kill_packets(&[77]) {
            packets.push(crate::AvatarPacket::Kill(kill));
        }
    }
    let mut block = [0u8; crate::AVATAR_TERSE_LEN];
    block[0..4].copy_from_slice(&77u32.to_le_bytes());
    source.blocks.lock().unwrap().push(block);
    state.maintain(&io).await.unwrap();
    // The task bucket holds the rest until it refills.
    for _ in 0..20 {
        if io.sent.lock().unwrap().len() >= 5 {
            break;
        }
        state.advance_throttle(Duration::from_millis(200));
        state.maintain(&io).await.unwrap();
    }
    let sent = io.sent.lock().unwrap().clone();
    let ids: Vec<_> = sent
        .iter()
        .map(|bytes| (bytes[0] & 0xc0, parse_packet(bytes).unwrap().id))
        .collect();
    assert_eq!(
        ids,
        [
            (0xc0, ferroan_protocol::msgs::ObjectUpdate::ID),
            (0xc0, ferroan_protocol::msgs::AvatarAppearance::ID),
            (0x40, ferroan_protocol::msgs::AvatarAnimation::ID),
            (0x40, ferroan_protocol::msgs::KillObject::ID),
            (0xc0, ferroan_protocol::msgs::ImprovedTerseObjectUpdate::ID),
        ]
    );
    // `AvatarAnimationHeader`: the flags, four sequence bytes, no extra
    // header, then message 20 in one byte.
    assert_eq!(sent[2][0], 0x40);
    assert_eq!(&sent[2][5..7], &[0, 20]);
    let kill = parse_packet(&sent[3])
        .unwrap()
        .decode::<ferroan_protocol::msgs::KillObject>()
        .unwrap();
    assert_eq!(kill.object_data[0].id, 77);
}

/// `SendCoarseLocationUpdate` writes its own header: no flags (unreliable,
/// not zero-coded), Medium 6, then the counts, `You` and `Prey` little
/// endian, and the agent ids. It waits behind a kill on the task outbox and
/// never enters the reliable window.
#[tokio::test]
async fn a_coarse_location_update_leaves_unreliable_with_the_csharp_layout() {
    use ferroan_protocol::msgs::coarse_location_update::{AgentData, Index, Location};
    let source = Arc::new(BlockFeed::default());
    let arrival = ArrivalConfig {
        updates: Some(crate::EntityFeed {
            updates: source.clone(),
            wake: Arc::new(Notify::new()),
        }),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), arrival);
    state.peer = Some(peer());
    let io = FakeIo::default();
    let first = Uuid::from_u128(0x0102);
    let second = Uuid::from_u128(0x0304);
    let coarse = ferroan_protocol::msgs::CoarseLocationUpdate {
        location: vec![
            Location {
                x: 128,
                y: 128,
                z: 6,
            },
            Location {
                x: 131,
                y: 40,
                z: 6,
            },
        ],
        index: Index { you: 1, prey: -1 },
        agent_data: vec![
            AgentData { agent_id: first },
            AgentData { agent_id: second },
        ],
        omitted_trailing_blocks: 0,
    };
    {
        let mut packets = source.packets.lock().unwrap();
        for kill in crate::kill_packets(&[77]) {
            packets.push(crate::AvatarPacket::Kill(kill));
        }
        packets.push(crate::AvatarPacket::Coarse(coarse));
    }
    for _ in 0..20 {
        state.maintain(&io).await.unwrap();
        if io.sent.lock().unwrap().len() >= 2 {
            break;
        }
        state.advance_throttle(Duration::from_millis(200));
    }
    let sent = io.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        parse_packet(&sent[0]).unwrap().id,
        ferroan_protocol::msgs::KillObject::ID
    );
    let bytes = &sent[1];
    assert_eq!(bytes[0], 0, "unreliable, not zero-coded, no acks");
    assert_eq!(bytes[5], 0);
    let mut body = vec![0xff, 6, 2, 128, 128, 6, 131, 40, 6, 1, 0, 0xff, 0xff, 2];
    body.extend_from_slice(first.as_bytes());
    body.extend_from_slice(second.as_bytes());
    assert_eq!(&bytes[6..], &body[..]);
    assert_eq!(state.unacked.len(), 1, "only the kill waits for an ack");
}

#[tokio::test(start_paused = true)]
async fn the_circuit_sends_the_captured_full_before_terse_and_resends_the_same_body() {
    use ferroan_protocol::msgs::{ImprovedTerseObjectUpdate, ObjectUpdate};
    let capture = include_bytes!("../../../fixtures/prim-arrival/limited.cap");
    let records = ferroan_capture::read_capture(capture.as_slice()).unwrap();
    let datagram = |seq| {
        &records
            .iter()
            .find(|record| {
                record.kind == ferroan_capture::Kind::UdpOut
                    && parse_packet(&record.bytes).unwrap().sequence == seq
            })
            .unwrap()
            .bytes
    };
    let full = datagram(32);
    let terse = datagram(29);
    let mut update = parse_packet(full)
        .unwrap()
        .decode::<ObjectUpdate>()
        .unwrap();
    assert_eq!(update.object_data.len(), 1);
    let motion = parse_packet(terse)
        .unwrap()
        .decode::<ImprovedTerseObjectUpdate>()
        .unwrap();
    let source = Arc::new(BlockFeed::default());
    *source.full.lock().unwrap() = update.object_data.pop();
    source
        .blocks
        .lock()
        .unwrap()
        .push(motion.object_data[0].data.as_slice().try_into().unwrap());
    let mut state = State::new(
        captured_config(),
        ArrivalConfig {
            region_handle: update.region_data.region_handle,
            updates: Some(crate::EntityFeed {
                updates: source,
                wake: Arc::new(Notify::new()),
            }),
            ..Default::default()
        },
    );
    state.peer = Some(peer());
    state.advance_throttle(Duration::from_secs(1));
    let now = state.throttle_clock.now_secs();
    assert!(state.throttle.try_spend(TASK, 0, now));
    let credit = state.throttle.balance(TASK);
    let io = FakeIo::default();
    state.maintain(&io).await.unwrap();
    let sent = io.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    for (actual, captured) in sent.iter().zip([full, terse]) {
        assert_eq!(actual[0], 0xc0);
        assert_eq!(&actual[5..], &captured[5..]);
    }
    assert_eq!(
        credit - state.throttle.balance(TASK),
        (full.len() + terse.len()) as f32
    );
    tokio::time::advance(RESEND_AFTER + Duration::from_millis(20)).await;
    state.maintain(&io).await.unwrap();
    let all = io.sent.lock().unwrap();
    let resend = all[2..]
        .iter()
        .find(|packet| packet[1..5] == sent[0][1..5])
        .unwrap();
    assert_eq!(resend[0], 0xe0);
    assert_eq!(&resend[5..], &full[5..]);
}

#[tokio::test]
async fn a_waiting_terse_block_leaves_reliable_and_zero_coded_on_the_task_bucket() {
    let source = Arc::new(BlockFeed::default());
    let arrival = ArrivalConfig {
        region_handle: 0x0003_e800_0003_e800,
        updates: Some(crate::EntityFeed {
            updates: source.clone(),
            wake: Arc::new(Notify::new()),
        }),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), arrival);
    state.peer = Some(peer());
    let io = FakeIo::default();
    let mut block = [0u8; crate::AVATAR_TERSE_LEN];
    block[0..4].copy_from_slice(&0x0102_0304u32.to_le_bytes());
    block[5] = 1;
    source.blocks.lock().unwrap().push(block);
    state.maintain(&io).await.unwrap();
    let sent = io.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let bytes = &sent[0];
    // `terseUpdateHeader`: MSG_RELIABLE | MSG_ZEROCODED.
    assert_eq!(bytes[0] & 0xc0, 0xc0);
    let packet = parse_packet(bytes).unwrap();
    assert_eq!(
        packet.id,
        ferroan_protocol::msgs::ImprovedTerseObjectUpdate::ID
    );
    let message = packet
        .decode::<ferroan_protocol::msgs::ImprovedTerseObjectUpdate>()
        .unwrap();
    assert_eq!(message.region_data.region_handle, 0x0003_e800_0003_e800);
    assert_eq!(message.region_data.time_dilation, 0xffff);
    assert_eq!(message.object_data.len(), 1);
    assert_eq!(message.object_data[0].data, block.to_vec());
    assert!(message.object_data[0].texture_entry.is_empty());
    // The budget is 30 ms of the default task rate, 18,500 bytes a second.
    assert_eq!(*source.budgets.lock().unwrap(), [555]);
    // The next ask waits 20 ms.
    source.blocks.lock().unwrap().push(block);
    state.maintain(&io).await.unwrap();
    assert_eq!(io.sent.lock().unwrap().len(), 1);
    assert!(state.time_until_work() <= Duration::from_millis(20));
}

#[tokio::test]
async fn a_region_handshake_reply_is_forwarded_and_a_refusal_waits() {
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config(), ArrivalConfig::default());
    state.peer = Some(peer());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let refuse = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let store = Arc::clone(&seen);
    let refusing = Arc::clone(&refuse);
    state.login_scene = Some(Arc::new(move |_agent, call| {
        if refusing.load(Ordering::Acquire) {
            return Err(crate::circuit::SceneRefused).into();
        }
        store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(call);
        Ok(SceneReply::Applied).into()
    }));
    let mut reply = ferroan_protocol::msgs::RegionHandshakeReply::default();
    reply.agent_data.agent_id = expect.agent_id;
    reply.agent_data.session_id = expect.session_id;
    let bytes = wire(&reply, 3, &[]);
    let io = FakeIo::default();
    state
        .handle(
            &io,
            peer(),
            &expect,
            &parse_packet(&bytes).unwrap(),
            bytes.len(),
        )
        .await
        .unwrap();
    assert!(seen.lock().unwrap().is_empty());
    refuse.store(false, Ordering::Release);
    state.maintain(&io).await.unwrap();
    let seen = seen.lock().unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(seen.len(), 1);
    assert!(matches!(seen[0], SceneCall::HandshakeReply));
}

/// `DequeueOutgoing` reports the task category empty only below five
/// waiting packets, so a fuller outbox does not ask the feed.
#[tokio::test]
async fn the_feed_is_asked_only_below_five_waiting_task_packets() {
    let source = Arc::new(BlockFeed::default());
    let arrival = ArrivalConfig {
        updates: Some(crate::EntityFeed {
            updates: source.clone(),
            wake: Arc::new(Notify::new()),
        }),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), arrival);
    state.peer = Some(peer());
    let io = FakeIo::default();
    // Exhaust the task bucket so the waiting packets stay.
    let now = state.throttle_clock.now_secs();
    assert!(state.throttle.try_spend(TASK, 10_000_000, now));
    let mut ping = CompletePingCheck::default();
    ping.ping_id.ping_id = 1;
    for _ in 0..5 {
        state.enqueue_task(&ping).unwrap();
    }
    source
        .blocks
        .lock()
        .unwrap()
        .push([0u8; crate::AVATAR_TERSE_LEN]);
    state.maintain(&io).await.unwrap();
    assert!(source.budgets.lock().unwrap().is_empty());
    assert_eq!(state.pending_task.len(), 5);
    state.pending_task.pop_back();
    state.maintain(&io).await.unwrap();
    assert_eq!(source.budgets.lock().unwrap().len(), 1);
    assert!(io.sent.lock().unwrap().is_empty());
}
