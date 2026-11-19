// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Deterministic clock regressions for pinned LLUDPServer/LLUDPClient timing.
//! Source paths, revision and original licence are recorded in circuit.rs.

use super::*;
use ferroan_protocol::msgs::RegionHandshake;
use ferroan_protocol::{build_packet, parse_packet};
use std::sync::Mutex;

#[derive(Default)]
struct RecordedIo(Mutex<Vec<Vec<u8>>>, Mutex<Vec<SocketAddr>>);
impl DatagramIo for RecordedIo {
    async fn recv_from(&self, _: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        std::future::pending().await
    }
    async fn send_to(&self, bytes: &[u8], to: SocketAddr) -> io::Result<usize> {
        self.0.lock().unwrap().push(bytes.to_vec());
        self.1.lock().unwrap().push(to);
        Ok(bytes.len())
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
async fn deliver<M: Msg>(state: &mut State, io: &RecordedIo, from: SocketAddr, message: &M) {
    let mut bytes = Vec::new();
    build_packet(message, 100, false, false, &[], &mut bytes);
    assert!(
        !state
            .handle(
                io,
                from,
                &identity(),
                &parse_packet(&bytes).unwrap(),
                bytes.len()
            )
            .await
            .unwrap()
    );
}
async fn connected() -> (State, RecordedIo) {
    connected_with(CircuitTiming::default()).await
}
async fn connected_with(timing: CircuitTiming) -> (State, RecordedIo) {
    let mut state = State::with_timing(
        HandshakeConfig::for_region(identity().region_id),
        ArrivalConfig::default(),
        timing,
    );
    let io = RecordedIo::default();
    let mut login = UseCircuitCode::default();
    login.circuit_code.id = identity().agent_id;
    login.circuit_code.session_id = identity().session_id;
    login.circuit_code.code = identity().circuit_code;
    deliver(&mut state, &io, peer(), &login).await;
    (state, io)
}
async fn tick(state: &mut State, io: &RecordedIo, count: usize) {
    for _ in 0..count {
        tokio::time::advance(RESEND_INTERVAL).await;
        state.maintain(io).await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn initial_resend_schedule_retains_unacked_until_strict_inactivity_deadline() {
    let (mut state, io) = connected().await;
    tick(&mut state, &io, 9).await;
    assert_eq!(state.unacked.retry_count(1), Some(0));
    tick(&mut state, &io, 1).await;
    assert_eq!(state.unacked.retry_count(1), Some(1));
    tick(&mut state, &io, 590).await;
    assert_eq!(state.unacked.retry_count(1), Some(60));
    assert_eq!(state.unacked.len(), 1);
    let packets: Vec<_> =
        io.0.lock()
            .unwrap()
            .iter()
            .map(|bytes| parse_packet(bytes).unwrap())
            .collect();
    let handshakes: Vec<_> = packets
        .iter()
        .filter(|packet| packet.id == RegionHandshake::ID)
        .collect();
    assert_eq!(handshakes.len(), 61);
    assert!(
        handshakes
            .iter()
            .all(|packet| packet.sequence == 1 && packet.reliable())
    );
    assert!(!handshakes[0].resent());
    assert!(handshakes[1..].iter().all(|packet| packet.resent()));
    let pings: Vec<_> = packets
        .iter()
        .filter(|packet| packet.id == StartPingCheck::ID)
        .map(|packet| packet.decode::<StartPingCheck>().unwrap())
        .collect();
    assert_eq!(pings.len(), 12);
    for (index, ping) in pings.iter().enumerate() {
        assert_eq!(ping.ping_id.ping_id, index as u8);
        assert_eq!(ping.ping_id.oldest_unacked, 1);
    }
    tokio::time::advance(RESEND_INTERVAL).await;
    assert!(matches!(
        state.maintain(&io).await,
        Err(CircuitError::Timeout)
    ));
    assert_eq!(state.unacked.retry_count(1), Some(60));
    let sent = io.0.lock().unwrap();
    let disable = sent.iter().rev().find_map(|bytes| {
        let packet = parse_packet(bytes).unwrap();
        (packet.id == ferroan_protocol::msgs::DisableSimulator::ID).then_some(packet)
    });
    let disable = disable.expect("timeout sends DisableSimulator");
    assert!(disable.reliable());
    assert!(disable.body.is_empty());
    assert!(
        !sent.iter().any(|bytes| {
            parse_packet(bytes).unwrap().id == ferroan_protocol::msgs::KickUser::ID
        })
    );
}

#[tokio::test(start_paused = true)]
async fn unsolicited_ping_completion_without_a_sample_keeps_initial_rto() {
    let (mut state, io) = connected().await;
    assert!(state.last_ping.is_none());
    tokio::time::advance(Duration::from_millis(100)).await;
    deliver(&mut state, &io, peer(), &CompletePingCheck::default()).await;
    // Without a sent ping there is no elapsed RTT to measure. This explicit
    // boundary differs from C# using its zero-initialized ping timestamp.
    assert_eq!(state.rto, CircuitTiming::default().default_rto);
    assert_eq!(state.ping_ms, 20);
    assert!(state.last_ping.is_none());
}

#[tokio::test(start_paused = true)]
async fn ping_filter_controls_rto_with_csharp_limits_and_no_ack_rtt_or_backoff() {
    let (mut state, io) = connected().await;
    assert_eq!(state.rto, Duration::from_secs(1));
    for (elapsed, filtered, rto) in [(100, 36, 250), (1000, 228, 1140), (10000, 2182, 3000)] {
        state.send_ping(&io, peer()).await.unwrap();
        tokio::time::advance(Duration::from_millis(elapsed)).await;
        // Source does not match PingID against the outstanding ping.
        let mut pong = CompletePingCheck::default();
        pong.ping_id.ping_id = 255;
        deliver(&mut state, &io, peer(), &pong).await;
        assert_eq!(state.ping_ms, filtered);
        assert_eq!(state.rto, Duration::from_millis(rto));
    }
    let ack = PacketAck {
        omitted_trailing_blocks: 0,
        packets: vec![packet_ack::Packets { id: 1 }],
    };
    deliver(&mut state, &io, peer(), &ack).await;
    assert_eq!(state.rto, Duration::from_secs(3));
    assert!(state.unacked.is_empty());
    state
        .send_message(&io, peer(), &CompletePingCheck::default(), true)
        .await
        .unwrap();
    tick(&mut state, &io, 300).await;
    assert_eq!(state.rto, Duration::from_secs(3));
    assert_eq!(state.unacked.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn mapped_traffic_keeps_circuit_alive_beyond_five_minutes() {
    let (mut state, io) = connected().await;
    for _ in 0..360 {
        tick(&mut state, &io, 10).await;
        deliver(&mut state, &io, peer(), &StartPingCheck::default()).await;
    }
    assert_eq!(state.unacked.len(), 1);
    assert_eq!(state.unacked.retry_count(1), Some(u8::MAX));
    tick(&mut state, &io, 600).await;
    tokio::time::advance(RESEND_INTERVAL).await;
    assert!(matches!(
        state.maintain(&io).await,
        Err(CircuitError::Timeout)
    ));
}

#[tokio::test(start_paused = true)]
async fn ping_adjustment_changes_the_pending_packets_resend_deadline() {
    let (mut state, io) = connected().await;
    tick(&mut state, &io, 50).await;
    assert_eq!(state.unacked.retry_count(1), Some(5));
    tick(&mut state, &io, 1).await;
    deliver(&mut state, &io, peer(), &CompletePingCheck::default()).await;
    assert_eq!(state.rto, Duration::from_millis(250));
    tick(&mut state, &io, 1).await;
    assert_eq!(state.unacked.retry_count(1), Some(5));
    // 250ms expires at the next 100ms maintenance check, not at the old1s RTO.
    tick(&mut state, &io, 1).await;
    assert_eq!(state.unacked.retry_count(1), Some(6));
}

#[tokio::test(start_paused = true)]
async fn pause_extends_inactivity_and_resume_sends_ping() {
    let (mut state, io) = connected().await;
    let mut pause = AgentPause::default();
    pause.agent_data.agent_id = identity().agent_id;
    pause.agent_data.session_id = identity().session_id;
    deliver(&mut state, &io, peer(), &pause).await;
    tick(&mut state, &io, 3000).await;
    assert!(state.paused);
    let mut resume = AgentResume::default();
    resume.agent_data.agent_id = identity().agent_id;
    resume.agent_data.session_id = identity().session_id;
    let before = state.ping_sequence;
    deliver(&mut state, &io, peer(), &resume).await;
    assert!(!state.paused);
    assert_eq!(state.ping_sequence, before.wrapping_add(1));
    tick(&mut state, &io, 600).await;
    tokio::time::advance(RESEND_INTERVAL).await;
    assert!(matches!(
        state.maintain(&io).await,
        Err(CircuitError::Timeout)
    ));
}

#[tokio::test(start_paused = true)]
async fn only_mapped_endpoint_refreshes_inactivity_even_before_ids_are_checked() {
    let (mut state, io) = connected().await;
    tick(&mut state, &io, 590).await;
    let previous = state.last_received;
    deliver(
        &mut state,
        &io,
        "127.0.0.1:9001".parse().unwrap(),
        &CompletePingCheck::default(),
    )
    .await;
    assert_eq!(state.last_received, previous);
    // This follows C# PacketReceived's ordering; it must not acknowledge the
    // reliable handshake or pause the circuit when identity validation fails.
    deliver(&mut state, &io, peer(), &AgentPause::default()).await;
    assert_eq!(state.last_received, Instant::now());
    assert!(!state.paused);
    assert_eq!(state.unacked.len(), 1);
    tick(&mut state, &io, 20).await;
}

#[test]
fn timing_accepts_csharp_overrides_without_clamping_initial_rto() {
    let custom = CircuitTiming {
        default_rto: Duration::from_secs(4),
        max_rto: Duration::from_secs(2),
        ack_timeout: Duration::from_secs(8),
        paused_ack_timeout: Duration::from_secs(16),
    };
    assert!(custom.valid());
    assert!(
        !CircuitTiming {
            ack_timeout: Duration::ZERO,
            ..custom
        }
        .valid()
    );
    assert!(
        !CircuitTiming {
            max_rto: Duration::MAX,
            ..custom
        }
        .valid()
    );
}

#[tokio::test(start_paused = true)]
async fn configured_initial_rto_and_inactivity_are_used_by_the_send_path() {
    let timing = CircuitTiming {
        default_rto: Duration::from_millis(1500),
        max_rto: Duration::from_millis(800),
        ack_timeout: Duration::from_secs(2),
        paused_ack_timeout: Duration::from_secs(4),
    };
    let (mut state, io) = connected_with(timing).await;
    tick(&mut state, &io, 14).await;
    assert_eq!(state.unacked.retry_count(1), Some(0));
    tick(&mut state, &io, 1).await;
    assert_eq!(state.unacked.retry_count(1), Some(1));
    tick(&mut state, &io, 5).await;
    tokio::time::advance(RESEND_INTERVAL).await;
    assert!(matches!(
        state.maintain(&io).await,
        Err(CircuitError::Timeout)
    ));
}

#[tokio::test(start_paused = true)]
async fn inactivity_starts_at_use_circuit_code_not_at_serve_start() {
    let mut state = State::with_timing(
        HandshakeConfig::for_region(identity().region_id),
        ArrivalConfig::default(),
        CircuitTiming::default(),
    );
    let io = RecordedIo::default();
    for _ in 0..700 {
        tokio::time::advance(RESEND_INTERVAL).await;
        state.maintain(&io).await.unwrap();
    }
    assert!(state.peer.is_none());
    assert!(io.0.lock().unwrap().is_empty());
    let mut login = UseCircuitCode::default();
    login.circuit_code.id = identity().agent_id;
    login.circuit_code.session_id = identity().session_id;
    login.circuit_code.code = identity().circuit_code;
    deliver(&mut state, &io, peer(), &login).await;
    tick(&mut state, &io, 600).await;
    tokio::time::advance(RESEND_INTERVAL).await;
    assert!(matches!(
        state.maintain(&io).await,
        Err(CircuitError::Timeout)
    ));
    assert!(io.0.lock().unwrap().iter().any(|bytes| {
        let packet = parse_packet(bytes).unwrap();
        packet.id == ferroan_protocol::msgs::DisableSimulator::ID && packet.reliable()
    }));
}

#[tokio::test(start_paused = true)]
async fn ping_keeps_the_last_oldest_after_the_window_empties() {
    let (mut state, io) = connected().await;
    tick(&mut state, &io, 1).await;
    assert_eq!(state.last_oldest, 1);
    let ack = PacketAck {
        omitted_trailing_blocks: 0,
        packets: vec![packet_ack::Packets { id: 1 }],
    };
    deliver(&mut state, &io, peer(), &ack).await;
    assert!(state.unacked.is_empty());
    tick(&mut state, &io, 49).await;
    let pings: Vec<_> = io
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|bytes| {
            let packet = parse_packet(bytes).unwrap();
            (packet.id == StartPingCheck::ID).then(|| packet.decode::<StartPingCheck>().unwrap())
        })
        .collect();
    assert!(!pings.is_empty());
    assert!(pings.iter().all(|ping| ping.ping_id.oldest_unacked == 1));
}

#[tokio::test]
async fn shutdown_sends_kick_then_disable_and_an_unmapped_close_sends_nothing() {
    let (mut state, io) = connected().await;
    let error = state
        .finish_close(&io, &identity(), CircuitStop::Shutdown)
        .await
        .unwrap_err();
    assert!(matches!(error, CircuitError::Closed));
    {
        let sent = io.0.lock().unwrap();
        let closing: Vec<_> = sent
            .iter()
            .filter_map(|bytes| {
                let packet = parse_packet(bytes).unwrap();
                matches!(
                    packet.id,
                    ferroan_protocol::msgs::KickUser::ID
                        | ferroan_protocol::msgs::DisableSimulator::ID
                )
                .then_some(packet)
            })
            .collect();
        assert_eq!(closing.len(), 2);
        assert_eq!(closing[0].id, ferroan_protocol::msgs::KickUser::ID);
        assert!(closing[0].reliable());
        let kick = closing[0]
            .decode::<ferroan_protocol::msgs::KickUser>()
            .unwrap();
        assert_eq!(kick.target_block.target_ip, [0, 0, 0, 0]);
        assert_eq!(kick.target_block.target_port, 0);
        assert_eq!(kick.user_info.agent_id, identity().agent_id);
        assert_eq!(kick.user_info.session_id, identity().session_id);
        assert_eq!(kick.user_info.reason, b"The simulator is going down.\0");
        assert_eq!(closing[1].id, ferroan_protocol::msgs::DisableSimulator::ID);
        assert!(closing[1].reliable());
        assert!(closing[1].body.is_empty());
    }

    let mut idle = State::with_timing(
        HandshakeConfig::for_region(identity().region_id),
        ArrivalConfig::default(),
        CircuitTiming::default(),
    );
    let quiet = RecordedIo::default();
    let error = idle
        .finish_close(&quiet, &identity(), CircuitStop::Shutdown)
        .await
        .unwrap_err();
    assert!(matches!(error, CircuitError::Closed));
    assert!(quiet.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn other_endpoint_use_circuit_code_is_acked_without_moving_the_peer() {
    let (mut state, io) = connected().await;
    let before = io.0.lock().unwrap().len();
    let other: SocketAddr = "127.0.0.1:9001".parse().unwrap();
    let mut login = UseCircuitCode::default();
    login.circuit_code.id = identity().agent_id;
    login.circuit_code.session_id = identity().session_id;
    login.circuit_code.code = identity().circuit_code;
    let mut bytes = Vec::new();
    build_packet(&login, 7, true, false, &[], &mut bytes);
    assert!(
        !state
            .handle(
                &io,
                other,
                &identity(),
                &parse_packet(&bytes).unwrap(),
                bytes.len()
            )
            .await
            .unwrap()
    );
    assert_eq!(state.peer, Some(peer()));
    let sent = io.0.lock().unwrap();
    let destinations = io.1.lock().unwrap();
    assert_eq!(destinations[before], other);
    let ack = parse_packet(&sent[before]).unwrap();
    assert_eq!(ack.id, PacketAck::ID);
    assert_eq!(ack.sequence, 0);
    assert!(!ack.reliable());
}

#[tokio::test(start_paused = true)]
async fn id_mismatch_and_a_short_body_are_dropped_before_ack() {
    let (mut state, io) = connected().await;
    let mut always = ferroan_protocol::msgs::SetAlwaysRun::default();
    always.agent_data.agent_id = Uuid::from_u128(99);
    always.agent_data.session_id = identity().session_id;
    let mut bytes = Vec::new();
    build_packet(&always, 50, true, false, &[], &mut bytes);
    assert!(
        !state
            .handle(
                &io,
                peer(),
                &identity(),
                &parse_packet(&bytes).unwrap(),
                bytes.len()
            )
            .await
            .unwrap()
    );
    assert!(!state.pending_acks.contains(&50));
    assert_eq!(state.last_received, Instant::now());

    always.agent_data.agent_id = identity().agent_id;
    let mut bytes = Vec::new();
    build_packet(&always, 51, true, false, &[], &mut bytes);
    assert!(
        !state
            .handle(
                &io,
                peer(),
                &identity(),
                &parse_packet(&bytes).unwrap(),
                bytes.len()
            )
            .await
            .unwrap()
    );
    assert!(state.pending_acks.contains(&51));

    let mut bytes = Vec::new();
    build_packet(&always, 52, true, false, &[], &mut bytes);
    bytes.truncate(16);
    let short = parse_packet(&bytes).unwrap();
    assert_eq!(short.id, ferroan_protocol::msgs::SetAlwaysRun::ID);
    assert!(
        !state
            .handle(&io, peer(), &identity(), &short, bytes.len())
            .await
            .unwrap()
    );
    assert!(!state.pending_acks.contains(&52));

    // LogoutRequest is one of the seven types already checked before this row.
    let logout = LogoutRequest::default();
    let mut bytes = Vec::new();
    build_packet(&logout, 53, true, false, &[], &mut bytes);
    assert!(
        !state
            .handle(
                &io,
                peer(),
                &identity(),
                &parse_packet(&bytes).unwrap(),
                bytes.len()
            )
            .await
            .unwrap()
    );
    assert!(!state.pending_acks.contains(&53));
}
