// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Real region circuit and bot sessions across a scripted UDP loss burst.
//! The relay only drops or forwards original socket datagrams. Retry behavior
//! follows LLUDPServer.HandleUnacked/ResendUnacked at
//! Tranquillity 40432e0f83d3802434f898a4e28687bd977d5cf5; no C# code is copied.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use ferroan_bot::{Agent, Session};
use ferroan_client::{ArrivalConfig, CapsRegistry, CircuitExpect, CircuitServer};
use ferroan_protocol::msgs::{
    AgentMovementComplete, LogoutReply, PacketAck, RegionHandshake, UseCircuitCode,
};
use ferroan_protocol::{Msg, MsgId, PacketFlags, parse_packet};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::oneshot;
use uuid::Uuid;

const BURST: Duration = Duration::from_secs(1);
const BUDGET: Duration = Duration::from_secs(10);
const MAX_PACKETS: usize = 256;

struct AbortTasks(Vec<tokio::task::AbortHandle>);

impl Drop for AbortTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

struct Observed {
    bytes: Vec<u8>,
    elapsed: Duration,
    dropped: bool,
}

async fn relay(
    front: UdpSocket,
    back: UdpSocket,
    target: MsgId,
    mut stop: oneshot::Receiver<()>,
) -> Vec<Observed> {
    let mut viewer: Option<SocketAddr> = None;
    let mut first_target = None;
    let mut evidence = Vec::new();
    let mut incoming = [0; 4096];
    let mut outgoing = [0; 4096];
    let mut forwarded = 0;
    loop {
        tokio::select! {
            _ = &mut stop => return evidence,
            received = front.recv_from(&mut incoming) => {
                let (size, from) = received.unwrap();
                assert_eq!(*viewer.get_or_insert(from), from);
                forwarded += 1;
                assert!(forwarded <= MAX_PACKETS, "unexpected client packet flood");
                // Client packets, including ACKs, pass unchanged. Recovery
                // must come from the server's own reliable send window.
                back.send(&incoming[..size]).await.unwrap();
            }
            received = back.recv(&mut outgoing) => {
                let size = received.unwrap();
                let bytes = &outgoing[..size];
                let packet = parse_packet(bytes).unwrap();
                let elapsed = if packet.id == target {
                    first_target.get_or_insert_with(Instant::now).elapsed()
                } else {
                    Duration::ZERO
                };
                let dropped = packet.id == target && elapsed < BURST;
                assert!(evidence.len() < MAX_PACKETS, "unexpected server packet flood");
                evidence.push(Observed { bytes: bytes.to_vec(), elapsed, dropped });
                if !dropped {
                    front.send_to(bytes, viewer.expect("server replied before circuit open")).await.unwrap();
                }
            }
        }
    }
}

async fn login_through_loss(target: MsgId) {
    let mut cleanup = AbortTasks(Vec::new());
    let expect = CircuitExpect {
        agent_id: Uuid::from_u128(1),
        session_id: Uuid::from_u128(2),
        circuit_code: 42,
        region_id: Uuid::from_u128(9),
    };
    let arrival = ArrivalConfig {
        position: [128.0, 128.0, 25.0],
        look_at: [0.99, 0.042, 0.0],
        region_handle: (256_000_u64 << 32) | 256_000,
        channel_version: "Ferroan loss-test region".into(),
    };
    let server = CircuitServer::bind().await.unwrap();
    let server_address = server.local_addr().unwrap();
    let running = tokio::spawn(async move { server.serve_one_with_arrival(expect, arrival).await });
    cleanup.0.push(running.abort_handle());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let caps = CapsRegistry::new(&base).unwrap();
    let seed_path = format!("/CAPS/{}0000/", Uuid::from_u128(3));
    caps.register(&seed_path).unwrap();
    let app = caps.router();
    let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    cleanup.0.push(serving.abort_handle());

    let front = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let relay_address = front.local_addr().unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    back.connect(server_address).await.unwrap();
    let (stop, stopped) = oneshot::channel();
    let relaying = tokio::spawn(relay(front, back, target, stopped));
    cleanup.0.push(relaying.abort_handle());

    let mut bot = Session::connect(
        relay_address,
        Agent {
            agent_id: expect.agent_id,
            session_id: expect.session_id,
            circuit_code: expect.circuit_code,
        },
    )
    .await
    .unwrap();
    let http = ferroan_bot::login::http_client().unwrap();
    tokio::time::timeout(
        BUDGET,
        bot.run_until_ready(&http, &format!("{base}{seed_path}"), BUDGET),
    )
    .await
    .expect("login exceeded test budget")
    .unwrap();
    assert_eq!(bot.region_id(), Some(expect.region_id));
    assert!(bot.caps().contains_key("EventQueueGet"));
    bot.logout(Duration::from_secs(3)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    stop.send(()).unwrap();
    let evidence = relaying.await.unwrap();

    let affected: Vec<_> = evidence
        .iter()
        .filter(|record| parse_packet(&record.bytes).unwrap().id == target)
        .collect();
    assert!(
        affected.len() >= 2,
        "must observe both original and actual server retry"
    );
    let original = parse_packet(&affected[0].bytes).unwrap();
    assert!(original.reliable());
    assert!(!original.resent());
    assert!(affected[0].dropped);
    let delivered = affected
        .iter()
        .find(|record| !record.dropped)
        .expect("target was never delivered after the loss burst");
    assert!(delivered.elapsed >= BURST);
    for retried in affected.iter().skip(1) {
        let retry = parse_packet(&retried.bytes).unwrap();
        assert!(retry.reliable() && retry.resent());
        assert_eq!(retry.sequence, original.sequence);
        let mut expected = affected[0].bytes.clone();
        expected[0] |= PacketFlags::RESENT;
        assert_eq!(
            retried.bytes, expected,
            "retry must retain the original wire body and sequence"
        );
    }
    // Handshake loss can deliver arrival early. The bot leaves that arrival
    // unacked, so readiness also exercises the real server's arrival retry.
    if target == RegionHandshake::ID {
        assert!(evidence.iter().any(|record| {
            let packet = parse_packet(&record.bytes).unwrap();
            packet.id == AgentMovementComplete::ID && packet.resent()
        }));
    }
    let logout = evidence
        .iter()
        .find(|record| parse_packet(&record.bytes).unwrap().id == LogoutReply::ID)
        .expect("logout must be observed on the server socket");
    let logout = parse_packet(&logout.bytes)
        .unwrap()
        .decode::<LogoutReply>()
        .unwrap();
    assert_eq!(logout.agent_data.agent_id, expect.agent_id);
    assert_eq!(logout.agent_data.session_id, expect.session_id);
}

#[tokio::test]
async fn bot_login_survives_one_second_region_handshake_loss() {
    login_through_loss(RegionHandshake::ID).await;
}

#[tokio::test]
async fn bot_login_survives_one_second_agent_movement_complete_loss() {
    login_through_loss(AgentMovementComplete::ID).await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Burst {
    ViewerAcks,
    Both,
}

async fn relay_burst(
    front: UdpSocket,
    back: UdpSocket,
    mode: Burst,
    mut stop: oneshot::Receiver<()>,
) -> Vec<Observed> {
    let mut viewer: Option<SocketAddr> = None;
    let mut started: Option<Instant> = None;
    let mut evidence = Vec::new();
    let mut incoming = [0; 4096];
    let mut outgoing = [0; 4096];
    let mut forwarded = 0usize;
    loop {
        tokio::select! {
            _ = &mut stop => return evidence,
            received = front.recv_from(&mut incoming) => {
                let (size, from) = received.unwrap();
                assert_eq!(*viewer.get_or_insert(from), from);
                forwarded += 1;
                assert!(forwarded <= MAX_PACKETS, "unexpected client packet flood");
                let bytes = incoming[..size].to_vec();
                let packet = parse_packet(&bytes).unwrap();
                let opens = match mode {
                    Burst::ViewerAcks => packet.id == PacketAck::ID || !packet.acks.is_empty(),
                    Burst::Both => true,
                };
                if started.is_none() && opens {
                    started = Some(Instant::now());
                }
                let elapsed = started.map(|start| start.elapsed()).unwrap_or(Duration::ZERO);
                let dropped = started.is_some_and(|start| start.elapsed() < BURST)
                    && match mode {
                        Burst::ViewerAcks => packet.id == PacketAck::ID || !packet.acks.is_empty(),
                        Burst::Both => true,
                    };
                assert!(evidence.len() < MAX_PACKETS, "unexpected packet flood");
                evidence.push(Observed { bytes, elapsed, dropped });
                if !dropped {
                    back.send(&incoming[..size]).await.unwrap();
                }
            }
            received = back.recv(&mut outgoing) => {
                let size = received.unwrap();
                let bytes = outgoing[..size].to_vec();
                if started.is_none() && mode == Burst::Both {
                    started = Some(Instant::now());
                }
                let elapsed = started.map(|start| start.elapsed()).unwrap_or(Duration::ZERO);
                let dropped = mode == Burst::Both && started.is_some_and(|start| start.elapsed() < BURST);
                assert!(evidence.len() < MAX_PACKETS, "unexpected server packet flood");
                evidence.push(Observed {
                    bytes: bytes.clone(),
                    elapsed,
                    dropped,
                });
                if !dropped {
                    front
                        .send_to(&bytes, viewer.expect("server replied before circuit open"))
                        .await
                        .unwrap();
                }
            }
        }
    }
}

async fn login_through_burst(mode: Burst, budget: Duration) -> Vec<Observed> {
    let mut cleanup = AbortTasks(Vec::new());
    let expect = CircuitExpect {
        agent_id: Uuid::from_u128(1),
        session_id: Uuid::from_u128(2),
        circuit_code: 42,
        region_id: Uuid::from_u128(9),
    };
    let arrival = ArrivalConfig {
        position: [128.0, 128.0, 25.0],
        look_at: [0.99, 0.042, 0.0],
        region_handle: (256_000_u64 << 32) | 256_000,
        channel_version: "Ferroan loss-test region".into(),
    };
    let server = CircuitServer::bind().await.unwrap();
    let server_address = server.local_addr().unwrap();
    let running = tokio::spawn(async move { server.serve_one_with_arrival(expect, arrival).await });
    cleanup.0.push(running.abort_handle());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let caps = CapsRegistry::new(&base).unwrap();
    let seed_path = format!("/CAPS/{}0000/", Uuid::from_u128(3));
    caps.register(&seed_path).unwrap();
    let app = caps.router();
    let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    cleanup.0.push(serving.abort_handle());

    let front = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let relay_address = front.local_addr().unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    back.connect(server_address).await.unwrap();
    let (stop, stopped) = oneshot::channel();
    let relaying = tokio::spawn(relay_burst(front, back, mode, stopped));
    cleanup.0.push(relaying.abort_handle());

    let mut bot = Session::connect(
        relay_address,
        Agent {
            agent_id: expect.agent_id,
            session_id: expect.session_id,
            circuit_code: expect.circuit_code,
        },
    )
    .await
    .unwrap();
    let http = ferroan_bot::login::http_client().unwrap();
    tokio::time::timeout(
        budget,
        bot.run_until_ready(&http, &format!("{base}{seed_path}"), budget),
    )
    .await
    .expect("login exceeded test budget")
    .unwrap();
    assert_eq!(bot.region_id(), Some(expect.region_id));
    // Viewer-ack loss does not stall the bot: it already has the handshake.
    // The server resend is due one second after the send, so hold the circuit
    // until that retry is observable. A both-ways blackout stalls login itself.
    if mode == Burst::ViewerAcks {
        tokio::time::sleep(BURST + Duration::from_millis(400)).await;
    }
    bot.logout(Duration::from_secs(3)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    stop.send(()).unwrap();
    relaying.await.unwrap()
}

#[tokio::test]
async fn bot_login_survives_one_second_of_viewer_ack_loss() {
    let evidence = login_through_burst(Burst::ViewerAcks, BUDGET).await;
    let dropped: Vec<_> = evidence.iter().filter(|record| record.dropped).collect();
    assert!(!dropped.is_empty(), "no viewer ack was dropped");
    assert!(dropped.iter().all(|record| {
        let packet = parse_packet(&record.bytes).unwrap();
        packet.id == PacketAck::ID || !packet.acks.is_empty()
    }));
    assert!(evidence.iter().any(|record| {
        let packet = parse_packet(&record.bytes).unwrap();
        !record.dropped && packet.reliable() && packet.resent()
    }));
    assert!(
        evidence
            .iter()
            .any(|record| { parse_packet(&record.bytes).unwrap().id == LogoutReply::ID })
    );
}

#[tokio::test]
async fn bot_login_survives_one_second_blackout_both_ways() {
    let budget = Duration::from_secs(15);
    let evidence = login_through_burst(Burst::Both, budget).await;
    assert!(evidence.iter().any(|record| record.dropped));
    assert!(evidence.iter().any(|record| {
        let packet = parse_packet(&record.bytes).unwrap();
        packet.id == UseCircuitCode::ID && packet.resent() || packet.reliable() && packet.resent()
    }));
    assert!(
        evidence
            .iter()
            .any(|record| record.dropped && record.elapsed < BURST)
    );
    assert!(
        evidence
            .iter()
            .any(|record| { parse_packet(&record.bytes).unwrap().id == LogoutReply::ID })
    );
}
