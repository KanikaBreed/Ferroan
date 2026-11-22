// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Circuit send-path regressions against the pinned C# socket capture.

use super::*;
use std::sync::Mutex;

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

fn teleport_request(expect: CircuitExpect, handle: u64, position: [f32; 3]) -> Vec<u8> {
    let mut message = ferroan_protocol::msgs::TeleportLocationRequest::default();
    message.agent_data.agent_id = expect.agent_id;
    message.agent_data.session_id = expect.session_id;
    message.info.region_handle = handle;
    message.info.position = position.into();
    message.info.look_at = [1.0, 0.0, 0.0].into();
    wire(&message, 2, &[])
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
    state
        .enqueue_task(&ferroan_protocol::msgs::ImprovedTerseObjectUpdate::default())
        .unwrap();
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0]);
    handle_teleport(&mut state, &io, &expect, &request).await;
    // Start is immediate. The terse was already queued, so it leaves before TeleportLocal.
    state.maintain(&io).await.unwrap();
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
    let mut seat = chat.seat(expect.agent_id, "Test User".into(), [128.0, 128.0, 25.0]);
    state.chat = Some(chat);
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0]);
    handle_teleport(&mut state, &io, &expect, &request).await;
    state.maintain(&io).await.unwrap();
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
    state
        .chat
        .as_ref()
        .unwrap()
        .publish(expect.agent_id, 1, 0, "after");
    let crate::chat::ViewerBound::Chat(said) = seat.rx.try_recv().unwrap() else {
        panic!("expected chat from the moved seat");
    };
    assert_eq!(said.chat_data.position.to_array(), [40.0, 200.0, 30.0]);
}

#[tokio::test]
async fn a_teleport_outside_the_region_or_while_frozen_fails() {
    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let (mut state, expect, io) = seated_teleport(region);
    let request = teleport_request(expect, 256, [10.0, 10.0, 30.0]);
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
    let request = teleport_request(expect, 0, [40.0, 200.0, 30.0]);
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
