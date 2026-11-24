// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Root-transition ordering and cancellation through the actual circuit handler.
//! C# sources and their licences are recorded in circuit.rs.

use super::*;
use ferroan_protocol::msgs::AgentMovementComplete;
use ferroan_protocol::{build_packet, parse_packet};
use std::sync::Mutex;

#[derive(Default)]
struct RecordedIo(Mutex<Vec<Vec<u8>>>);
impl DatagramIo for RecordedIo {
    async fn recv_from(&self, _: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        std::future::pending().await
    }
    async fn send_to(&self, bytes: &[u8], _: SocketAddr) -> io::Result<usize> {
        self.0.lock().unwrap().push(bytes.to_vec());
        Ok(bytes.len())
    }
}
impl RecordedIo {
    fn arrivals(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|bytes| parse_packet(bytes).unwrap().id == AgentMovementComplete::ID)
            .count()
    }
}
fn peer() -> SocketAddr {
    "127.0.0.1:9000".parse().unwrap()
}
fn identity() -> CircuitExpect {
    CircuitExpect {
        agent_id: Uuid::from_u128(1),
        session_id: Uuid::from_u128(2),
        circuit_code: 42,
        region_id: Uuid::from_u128(3),
    }
}
fn wire<M: Msg>(message: &M, sequence: u32) -> IncomingPacket {
    let mut bytes = Vec::new();
    build_packet(message, sequence, true, false, &[], &mut bytes);
    parse_packet(&bytes).unwrap()
}
fn movement() -> CompleteAgentMovement {
    let mut movement = CompleteAgentMovement::default();
    movement.agent_data.agent_id = identity().agent_id;
    movement.agent_data.session_id = identity().session_id;
    movement.agent_data.circuit_code = identity().circuit_code;
    movement
}
fn state() -> (State, oneshot::Receiver<RootArrival>) {
    let mut state = State::new(
        HandshakeConfig::for_region(identity().region_id),
        ArrivalConfig::default(),
    );
    let (sender, receiver) = oneshot::channel();
    state.root_arrival = Some(sender);
    (state, receiver)
}
async fn connect(state: &mut State, io: &RecordedIo) {
    let mut login = UseCircuitCode::default();
    login.circuit_code.id = identity().agent_id;
    login.circuit_code.session_id = identity().session_id;
    login.circuit_code.code = identity().circuit_code;
    assert!(
        !state
            .handle(io, peer(), &identity(), &wire(&login, 1), 0)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn validated_root_transition_notifies_before_arrival_and_only_once() {
    let (mut state, mut notification) = state();
    let io = RecordedIo::default();
    let expected = identity();
    let packet = wire(&movement(), 2);
    // An unestablished endpoint must not promote itself with movement alone.
    state
        .handle(&io, peer(), &expected, &packet, 0)
        .await
        .unwrap();
    assert!(matches!(
        notification.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    connect(&mut state, &io).await;
    state
        .handle(&io, peer(), &expected, &packet, 0)
        .await
        .unwrap();
    let token = notification.await.unwrap();
    assert_eq!(io.arrivals(), 0);
    token.complete();
    state.arrival_completion.take().unwrap().await.unwrap();
    state.send_arrival(&io, peer(), &expected).await.unwrap();
    assert_eq!(io.arrivals(), 1);
    assert!(state.arrived);
    assert!(state.root_arrival.is_none());
    // Both retransmitted and newly sequenced movement remain a single root visit.
    for sequence in [2, 3] {
        state
            .handle(&io, peer(), &expected, &wire(&movement(), sequence), 0)
            .await
            .unwrap();
    }
    assert_eq!(io.arrivals(), 1);
}

#[tokio::test]
async fn invalid_peer_identity_and_circuit_do_not_notify_root_arrival() {
    let (mut state, mut notification) = state();
    let io = RecordedIo::default();
    connect(&mut state, &io).await;
    let mut sequence = 2;
    let wrong_peer = "127.0.0.1:9001".parse().unwrap();
    state
        .handle(
            &io,
            wrong_peer,
            &identity(),
            &wire(&movement(), sequence),
            0,
        )
        .await
        .unwrap();
    for field in 0..3 {
        let mut message = movement();
        match field {
            0 => message.agent_data.agent_id = Uuid::nil(),
            1 => message.agent_data.session_id = Uuid::nil(),
            _ => message.agent_data.circuit_code += 1,
        }
        sequence += 1;
        state
            .handle(&io, peer(), &identity(), &wire(&message, sequence), 0)
            .await
            .unwrap();
    }
    let mut malformed = wire(&movement(), 10);
    malformed.body.clear();
    state
        .handle(&io, peer(), &identity(), &malformed, 0)
        .await
        .unwrap();
    assert!(matches!(
        notification.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(!state.arrived);
    assert_eq!(io.arrivals(), 0);
}

#[tokio::test]
async fn cancelled_generation_cannot_send_arrival_when_its_token_completes() {
    let (mut old, notification) = state();
    let io = RecordedIo::default();
    connect(&mut old, &io).await;
    let expected = identity();
    let packet = wire(&movement(), 2);
    old.handle(&io, peer(), &expected, &packet, 0)
        .await
        .unwrap();
    let token = notification.await.unwrap();
    drop(old);
    token.complete();
    assert_eq!(io.arrivals(), 0);
    let (new, notification) = state();
    drop(new);
    assert!(
        notification.await.is_err(),
        "a generation without movement must not notify"
    );
}

#[tokio::test]
async fn reporting_receiver_or_token_loss_does_not_refuse_movement() {
    for drop_receiver in [true, false] {
        let (mut state, notification) = state();
        let io = RecordedIo::default();
        connect(&mut state, &io).await;
        let expected = identity();
        let packet = wire(&movement(), 2);
        if drop_receiver {
            drop(notification);
            state
                .handle(&io, peer(), &expected, &packet, 0)
                .await
                .unwrap();
        } else {
            state
                .handle(&io, peer(), &expected, &packet, 0)
                .await
                .unwrap();
            let token = notification.await.unwrap();
            drop(token);
        }
        assert!(state.arrival_completion.take().unwrap().await.is_err());
        state.send_arrival(&io, peer(), &expected).await.unwrap();
        assert_eq!(io.arrivals(), 1);
    }
}

struct QueuedIo {
    incoming: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>,
    outgoing: RecordedIo,
}
impl DatagramIo for QueuedIo {
    async fn recv_from(&self, buffer: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let bytes = self.incoming.lock().await.recv().await.unwrap();
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok((bytes.len(), peer()))
    }
    async fn send_to(&self, bytes: &[u8], target: SocketAddr) -> io::Result<usize> {
        self.outgoing.send_to(bytes, target).await
    }
}
async fn enqueue<M: Msg>(sender: &tokio::sync::mpsc::Sender<Vec<u8>>, message: &M, sequence: u32) {
    let mut bytes = Vec::new();
    build_packet(message, sequence, true, false, &[], &mut bytes);
    sender.send(bytes).await.unwrap();
}
async fn wait_packet(io: &QueuedIo, matches: impl Fn(&IncomingPacket) -> bool) {
    for _ in 0..128 {
        if io
            .outgoing
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|bytes| matches(&parse_packet(bytes).unwrap()))
        {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("transport stopped while root reporting was held");
}

#[tokio::test(start_paused = true)]
async fn held_reporting_keeps_transport_running_and_allows_completion_logout_or_inactivity() {
    use ferroan_protocol::msgs::RegionHandshake;
    use std::sync::Arc;

    for ending in ["completion", "logout", "inactivity"] {
        let (incoming, receiver) = tokio::sync::mpsc::channel(8);
        let io = Arc::new(QueuedIo {
            incoming: tokio::sync::Mutex::new(receiver),
            outgoing: RecordedIo::default(),
        });
        let (notify, notification) = oneshot::channel();
        let serving_io = Arc::clone(&io);
        let serving = tokio::spawn(async move {
            serve_socket_reporting(
                serving_io.as_ref(),
                identity(),
                HandshakeConfig::for_region(identity().region_id),
                ArrivalConfig::default(),
                CircuitTiming {
                    ack_timeout: Duration::from_secs(10),
                    ..CircuitTiming::default()
                },
                None,
                ServeExtra {
                    root_arrival: Some(notify),
                    connected: None,
                    pending: None,
                    stop: None,
                    chat: None,
                    login_scene: None,
                },
            )
            .await
        });
        let mut login = UseCircuitCode::default();
        login.circuit_code.id = identity().agent_id;
        login.circuit_code.session_id = identity().session_id;
        login.circuit_code.code = identity().circuit_code;
        enqueue(&incoming, &login, 1).await;
        enqueue(&incoming, &movement(), 2).await;
        let token = notification.await.unwrap();
        let mut ping = StartPingCheck::default();
        ping.ping_id.ping_id = 71;
        enqueue(&incoming, &ping, 3).await;
        wait_packet(&io, |packet| packet.id == CompletePingCheck::ID).await;
        tokio::time::advance(Duration::from_millis(500)).await;
        wait_packet(&io, |packet| {
            packet.id == PacketAck::ID && packet.sequence != 0
        })
        .await;
        tokio::time::advance(Duration::from_millis(500)).await;
        wait_packet(&io, |packet| {
            packet.id == RegionHandshake::ID && packet.resent()
        })
        .await;
        tokio::time::advance(Duration::from_secs(4)).await;
        wait_packet(&io, |packet| packet.id == StartPingCheck::ID).await;
        assert_eq!(io.outgoing.arrivals(), 0);
        if ending == "inactivity" {
            tokio::time::advance(Duration::from_millis(5100)).await;
            assert!(matches!(serving.await.unwrap(), Err(CircuitError::Timeout)));
            token.complete();
            assert_eq!(io.outgoing.arrivals(), 0);
        } else {
            let held = if ending == "completion" {
                token.complete();
                wait_packet(&io, |packet| packet.id == AgentMovementComplete::ID).await;
                None
            } else {
                Some(token)
            };
            let mut logout = LogoutRequest::default();
            logout.agent_data.agent_id = identity().agent_id;
            logout.agent_data.session_id = identity().session_id;
            enqueue(&incoming, &logout, 4).await;
            serving.await.unwrap().unwrap();
            wait_packet(&io, |packet| packet.id == LogoutReply::ID).await;
            if let Some(token) = held {
                token.complete();
                assert_eq!(io.outgoing.arrivals(), 0);
            } else {
                assert_eq!(io.outgoing.arrivals(), 1);
            }
        }
    }
}

fn handshakes(io: &RecordedIo) -> usize {
    use ferroan_protocol::msgs::RegionHandshake;
    io.0.lock()
        .unwrap()
        .iter()
        .filter(|bytes| parse_packet(bytes).unwrap().id == RegionHandshake::ID)
        .count()
}

fn child_state(
    child: bool,
) -> (
    State,
    crate::return_hop::ReturnSlot,
    oneshot::Receiver<RootArrival>,
) {
    let slot = crate::return_hop::ReturnSlot::new(child);
    let arrival = ArrivalConfig {
        return_slot: Some(slot.clone()),
        position: [10.0, 10.0, 10.0],
        look_at: [0.0, 1.0, 0.0],
        ..ArrivalConfig::default()
    };
    let mut state = State::new(HandshakeConfig::for_region(identity().region_id), arrival);
    let (sender, receiver) = oneshot::channel();
    state.root_arrival = Some(sender);
    (state, slot, receiver)
}

#[tokio::test]
async fn a_new_child_waits_for_the_update_without_a_second_handshake() {
    let (mut state, slot, mut notification) = child_state(true);
    let io = RecordedIo::default();
    connect(&mut state, &io).await;
    assert_eq!(handshakes(&io), 1);
    state
        .handle(&io, peer(), &identity(), &wire(&movement(), 2), 0)
        .await
        .unwrap();
    assert!(state.return_wait);
    assert_eq!(handshakes(&io), 1);
    assert!(matches!(
        notification.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(slot.is_child());
}

#[tokio::test]
async fn a_parked_circuit_returns_with_one_handshake_and_the_update_pose() {
    let (mut state, slot, notification) = child_state(false);
    let io = RecordedIo::default();
    connect(&mut state, &io).await;
    state
        .handle(&io, peer(), &identity(), &wire(&movement(), 2), 0)
        .await
        .unwrap();
    notification.await.unwrap().complete();
    assert_eq!(handshakes(&io), 1);
    slot.arm_child();
    io.0.lock().unwrap().clear();
    state
        .handle(&io, peer(), &identity(), &wire(&movement(), 3), 0)
        .await
        .unwrap();
    assert!(state.return_wait);
    assert_eq!(handshakes(&io), 1);
    state
        .handle(&io, peer(), &identity(), &wire(&movement(), 4), 0)
        .await
        .unwrap();
    assert_eq!(handshakes(&io), 1);
    let origin = Uuid::from_u128(9);
    assert!(slot.store_update(
        crate::return_hop::ReturnPose {
            position: [128.0, 128.0, 30.0],
            look_at: [1.0, 0.0, 0.0],
            origin,
        },
        None,
    ));
    let pose = slot
        .wait_update(std::time::Duration::from_secs(1))
        .await
        .unwrap();
    state.return_wait = false;
    state
        .finish_return(identity().agent_id, pose)
        .await
        .unwrap();
    let mut promotions = slot.take_promotions().unwrap();
    let promoted = promotions.try_recv().unwrap();
    assert_eq!(io.arrivals(), 0);
    slot.note_rooted();
    promoted.complete();
    state.arrival_completion.take().unwrap().await.unwrap();
    state.send_arrival(&io, peer(), &identity()).await.unwrap();
    assert_eq!(io.arrivals(), 1);
    let packet = parse_packet(io.0.lock().unwrap().last().unwrap())
        .unwrap()
        .decode::<AgentMovementComplete>()
        .unwrap();
    assert_eq!(packet.data.position.to_array(), [128.0, 128.0, 30.0]);
    assert_eq!(packet.data.look_at.to_array(), [1.0, 0.0, 0.0]);
    assert!(!slot.is_child());
}

#[tokio::test]
async fn a_child_circuit_moves_to_a_new_endpoint_and_cannot_teleport() {
    use ferroan_protocol::msgs::TeleportFailed;
    let (mut state, slot, _notification) = child_state(true);
    let io = RecordedIo::default();
    connect(&mut state, &io).await;
    let other: SocketAddr = "127.0.0.1:9001".parse().unwrap();
    let mut login = UseCircuitCode::default();
    login.circuit_code.id = identity().agent_id;
    login.circuit_code.session_id = identity().session_id;
    login.circuit_code.code = identity().circuit_code;
    assert!(
        state
            .retarget_child(&io, other, &identity(), &wire(&login, 2))
            .await
            .unwrap()
    );
    assert_eq!(state.peer, Some(other));
    assert_eq!(handshakes(&io), 2);
    assert!(slot.is_child());
    login.circuit_code.code += 1;
    assert!(
        state
            .retarget_child(&io, peer(), &identity(), &wire(&login, 3))
            .await
            .unwrap()
    );
    assert_eq!(state.peer, Some(other));
    let (mut rooted, _slot, _notification) = child_state(false);
    rooted.peer = Some(peer());
    assert!(
        !rooted
            .retarget_child(&io, other, &identity(), &wire(&login, 4))
            .await
            .unwrap()
    );

    let region = crate::teleport::TeleportRegion::flat([128.0, 128.0, 25.0]);
    let (mut state, _slot, _notification) = child_state(true);
    state.arrival.teleport = Some(region);
    state.peer = Some(peer());
    state.arrived = true;
    let request = |sequence| {
        let mut message = TeleportLocationRequest::default();
        message.agent_data.agent_id = identity().agent_id;
        message.agent_data.session_id = identity().session_id;
        message.info.region_handle = 0;
        message.info.position = [40.0, 200.0, 30.0].into();
        message.info.look_at = [1.0, 0.0, 0.0].into();
        wire(&message, sequence)
    };
    io.0.lock().unwrap().clear();
    state
        .handle(&io, peer(), &identity(), &request(5), 0)
        .await
        .unwrap();
    let failed = parse_packet(&io.0.lock().unwrap()[0])
        .unwrap()
        .decode::<TeleportFailed>()
        .unwrap();
    assert_eq!(
        ferroan_protocol::field_str(&failed.info.reason),
        crate::teleport::FAIL_CHILD
    );
    state.in_transit = true;
    io.0.lock().unwrap().clear();
    state
        .handle(&io, peer(), &identity(), &request(6), 0)
        .await
        .unwrap();
    let failed = parse_packet(&io.0.lock().unwrap()[0])
        .unwrap()
        .decode::<TeleportFailed>()
        .unwrap();
    assert_eq!(
        ferroan_protocol::field_str(&failed.info.reason),
        FAIL_INCOMPLETE
    );
}
