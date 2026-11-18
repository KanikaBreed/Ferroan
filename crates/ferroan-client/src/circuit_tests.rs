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
    config.cache_id = Uuid::from_u128(0x00795643ab6b490fa9dc3a0ccf4b9ca1);
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
    let mut state = State::new(config);
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
    let captured = parse_packet(ESTABLISHED_ACK).unwrap();
    let ack = captured.decode::<PacketAck>().unwrap();
    let mut state = State::new(captured_config());
    state.sequence = SequenceCounter::new(captured.sequence);
    state
        .pending_acks
        .extend(ack.packets.iter().map(|block| block.id));
    let io = FakeIo::default();
    state.flush_acks(&io, peer()).await.unwrap();
    assert_eq!(io.sent.lock().unwrap().as_slice(), &[ESTABLISHED_ACK]);
    assert_eq!(state.sequence.peek(), captured.sequence + 1);
    assert!(state.pending_acks.is_empty());
    assert!(state.unacked.is_empty());
}

#[tokio::test]
async fn logout_send_path_matches_csharp_bytes_and_finishes_without_ack() {
    let captured = parse_packet(LOGOUT).unwrap();
    let expect = identity(Uuid::from_u128(1));
    let mut state = State::new(captured_config());
    state.peer = Some(peer());
    state.sequence = SequenceCounter::new(captured.sequence);
    let io = FakeIo::default();
    let request = logout(expect, &[]);
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
    assert_eq!(io.sent.lock().unwrap().as_slice(), &[LOGOUT]);
    assert!(state.unacked.contains(captured.sequence));

    // With no incoming ACK at all, the real serving loop must still complete.
    let io = FakeIo::default();
    io.incoming
        .lock()
        .unwrap()
        .extend([Ok(use_circuit(expect)), Ok(logout(expect, &[]))]);
    tokio::time::timeout(
        Duration::from_secs(1),
        serve_socket(&io, expect, captured_config(), Duration::from_secs(5)),
    )
    .await
    .expect("logout must not wait for an ACK")
    .unwrap();
    assert!(io.incoming.lock().unwrap().is_empty());
    assert!(
        io.sent
            .lock()
            .unwrap()
            .iter()
            .any(|bytes| parse_packet(bytes).unwrap().id == LogoutReply::ID)
    );
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
    serve_socket(&io, expect, captured_config(), Duration::from_secs(1))
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
    let mut state = State::new(config);
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
    let mut state = State::new(captured_config());
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
