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
                Some(notify),
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
