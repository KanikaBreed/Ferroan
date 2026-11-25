// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Circuit send-path regressions against the pinned C# socket capture.

use super::*;
use std::collections::HashMap;
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

async fn handle_teleport(state: &mut State, io: &FakeIo, expect: &CircuitExpect, request: &[u8]) {
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
            SceneCall::Pose(pose) => {
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
            SceneCall::AgentUpdate(_) | SceneCall::AlwaysRun(_) | SceneCall::HandshakeReply => {
                Ok(SceneReply::Applied)
            }
            SceneCall::Departure => Ok(SceneReply::Departure(crate::circuit::SceneDeparture {
                flying: false,
                pose: crate::source_hop::HopPose::default(),
            })),
            SceneCall::Arrive(pose) => {
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
            zerocoded: false,
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
            zerocoded: false,
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
        zerocoded: false,
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
        zerocoded: false,
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

#[tokio::test]
async fn local_teleport_stays_behind_a_queued_terse_update() {
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let (mut state, expect, io) = seated_teleport(region);
    let places = Arc::new(Mutex::new(HashMap::<Uuid, [f32; 3]>::new()));
    state.login_scene = Some(recording_scene(places));
    state
        .enqueue_task(&ferroan_protocol::msgs::ImprovedTerseObjectUpdate::default())
        .unwrap();
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0], 2);
    handle_teleport(&mut state, &io, &expect, &request).await;
    // Start is immediate. The terse was already queued, so it leaves before TeleportLocal.
    release_throttled(&mut state, &io).await;
    assert_eq!(
        message_ids(&io),
        vec![
            ferroan_protocol::msgs::TeleportStart::ID,
            ferroan_protocol::msgs::ImprovedTerseObjectUpdate::ID,
            ferroan_protocol::msgs::TeleportLocal::ID,
        ]
    );
    assert!(state.pending_task.is_empty());
}

#[tokio::test]
async fn local_teleport_lands_and_moves_the_chat_seat() {
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let shared = region.position.clone();
    let (mut state, expect, io) = seated_teleport(region);
    let chat = RegionChat::new();
    let mut seat = chat.seat(expect.agent_id, "Test User".into());
    state.chat = Some(chat);
    let places = Arc::new(Mutex::new(HashMap::<Uuid, [f32; 3]>::new()));
    state.login_scene = Some(recording_scene(Arc::clone(&places)));
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0], 2);
    handle_teleport(&mut state, &io, &expect, &request).await;
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
    state
        .chat
        .as_ref()
        .unwrap()
        .publish(expect.agent_id, 1, 0, "after", &heard);
    let crate::chat::ViewerBound::Chat(said) = seat.rx.try_recv().unwrap() else {
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
    handle_teleport(&mut state, &io, &expect, &request).await;
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
    handle_teleport(&mut state, &io, &expect, &request).await;
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
                    center: [1.0, 2.0, 3.0],
                    head_rotation: [0.0, 0.0, 0.6, 0.8],
                    body_rotation: [0.0, 0.0, 0.8, 0.6],
                },
            }))
            .into(),
            _ => Err(crate::circuit::SceneRefused).into(),
        }));
        let io = FakeIo::default();
        let foreign = teleport_request(expect, 256, [10.0, 10.0, 30.0], 2);
        handle_teleport(&mut state, &io, &expect, &foreign).await;
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
        let (sent, pose) = flags.lock().unwrap().unwrap();
        assert_eq!(sent, expected);
        // The frame's kept values, not the last raw AgentUpdate.
        assert_eq!(pose.center, [1.0, 2.0, 3.0]);
        assert_eq!(pose.head_rotation, [0.0, 0.0, 0.6, 0.8]);
        assert_eq!(pose.body_rotation, [0.0, 0.0, 0.8, 0.6]);
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
    };
    state.finish_return(expect.agent_id, pose).await.unwrap();
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
    handle_teleport(&mut state, &io, &expect, &foreign).await;
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
    handle_teleport(&mut state, &io, &expect, &again).await;
    let local = teleport_request(expect, 0, [40.0, 200.0, 30.0], 4);
    handle_teleport(&mut state, &io, &expect, &local).await;
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
    handle_teleport(&mut state, &io, &expect, &after).await;
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
    handle_teleport(&mut state, &io, &expect, &foreign).await;
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
        avatar: Some(crate::AvatarWire::standing(expect.agent_id, "Test", "User")),
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

#[tokio::test]
async fn a_local_teleport_without_a_scene_cannot_be_processed() {
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let (mut state, expect, io) = seated_teleport(region);
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0], 2);
    handle_teleport(&mut state, &io, &expect, &request).await;
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
    handle_teleport(&mut state, &io, &expect, &request).await;
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
    let _seat = chat.seat(expect.agent_id, "Test User".into());
    state.chat = Some(chat);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    state.login_scene = Some(Arc::new(move |_agent, _call| {
        counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(SceneReply::Places(HashMap::new())).into()
    }));
    let io = FakeIo::default();
    for (channel, sequence) in [(5, 4), (0, 5)] {
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
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
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
        avatar: Some(crate::AvatarWire::standing(expect.agent_id, "Test", "User")),
        ..ArrivalConfig::default()
    };
    let mut state = State::new(captured_config(), config);
    state.peer = Some(peer());
    state.login_scene = Some(Arc::new(move |agent, call| {
        // The fresh login is one arrival call: no separate pose and release.
        let SceneCall::Arrive(pose) = call else {
            return Err(crate::circuit::SceneRefused).into();
        };
        assert_eq!(agent, expect.agent_id);
        assert_eq!(pose.position, [10.0, 20.0, 30.0]);
        assert_eq!(pose.rotation, LOGIN_ROTATION);
        assert_eq!(pose.velocity, [0.0, 0.0, 0.0]);
        assert_eq!(pose.control_flags, 0);
        assert_eq!(pose.draw_distance, LOGIN_DRAW_DISTANCE);
        let mut stored = pose;
        stored.position[2] = 41.5;
        Ok(SceneReply::Pose(stored)).into()
    }));
    let io = FakeIo::default();
    state.send_arrival(&io, peer(), &expect).await.unwrap();
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

/// A feed the test fills with ready blocks. `take_terse` records the budget.
#[derive(Default)]
struct BlockFeed {
    blocks: Mutex<Vec<[u8; crate::AVATAR_TERSE_LEN]>>,
    budgets: Mutex<Vec<i32>>,
}

impl crate::EntityUpdates for BlockFeed {
    fn has_updates(&self) -> bool {
        !self.blocks.lock().unwrap().is_empty()
    }

    fn take_terse(&self, max_bytes: i32) -> Vec<[u8; crate::AVATAR_TERSE_LEN]> {
        self.budgets.lock().unwrap().push(max_bytes);
        std::mem::take(&mut *self.blocks.lock().unwrap())
    }
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
