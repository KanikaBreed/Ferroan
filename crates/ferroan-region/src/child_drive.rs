// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Ported from
//   Source/OpenSim.Region.CoreModules/Framework/EntityTransfer/EntityTransferModule.cs
//     (EnableChildAgents, InformClientOfNeighbourAsync, CloseChildAgents)
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs
//     (CheckForSignificantMovement child half, CloseChildAgents)
//   Source/OpenSim.Region.Framework/Scenes/SceneCommunicationService.cs
//     (SendChildAgentDataUpdate)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs the frame's child notices off the scene thread.
//!
//! One task receives each [`ferroan_scene::ChildNotice`], diffs
//! [`crate::grid::VisibleNeighbours`] on an enable, and admits, updates or
//! closes. The scene thread has already returned. A new neighbour waits 500 ms
//! after `CreateAgent` before the viewer events. A position that became due
//! during that wait is sent once the child is open and before those events:
//! the viewer then requests the seed, and the neighbour proxy would otherwise
//! see the PUT and the seed at the same time. A position update goes only
//! to an open child, so a PUT cannot land before the child exists.

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;

use ferroan_scene::{ChildNotice, ChildPass, TerrainFrame};
use uuid::Uuid;

use crate::children::{Admission, Claim, ClaimWork, NeighbourView, PostReport, admission_events};
use crate::config::RegionConfig;
use crate::grid::{GridClient, VisibleNeighbour, VisibleNeighbours};
use crate::simulation::SimulationState;
use crate::simulation_client::{
    AgentPosition, ChildIdentity, RegionEnd, SimulationClient, child_circuit,
};

/// `AvatarAppearance` constructor width and depth at Pin8. The appearance
/// module keeps the same defaults private.
const CHILD_SIZE_X: f32 = 0.45;
const CHILD_SIZE_Y: f32 = 0.6;
const CHILD_HEIGHT: f32 = 1.9;

/// Diff, admit and close until the frame stops sending notices.
pub async fn run(
    mut notices: tokio::sync::mpsc::Receiver<ChildNotice>,
    simulation: SimulationState,
    config: RegionConfig,
    frame: Arc<TerrainFrame>,
    client: SimulationClient,
) {
    let mut neighbours = VisibleNeighbours::new(GridClient::new(&config), &config);
    let mut pending = VecDeque::new();
    loop {
        let notice = match pending.pop_front() {
            Some(notice) => notice,
            None => match notices.recv().await {
                Some(notice) => notice,
                None => break,
            },
        };
        let visible = if notice.pass == ChildPass::Enable {
            neighbours
                .in_view(notice.position, notice.draw_distance)
                .await
        } else {
            Vec::new()
        };
        let mut inbox = Inbox {
            notices: &mut notices,
            pending: &mut pending,
        };
        apply_notice(
            &simulation,
            &config,
            &frame,
            &client,
            &notice,
            &visible,
            Some(&mut inbox),
        )
        .await;
    }
}

/// Notices already queued while this task is inside one pass.
struct Inbox<'a> {
    notices: &'a mut tokio::sync::mpsc::Receiver<ChildNotice>,
    pending: &'a mut VecDeque<ChildNotice>,
}

impl Inbox<'_> {
    /// Position passes for `agent` that arrived before the viewer events.
    /// Anything else stays queued for the outer loop.
    fn take_positions(&mut self, agent: Uuid) -> Vec<ChildNotice> {
        let mut positions = Vec::new();
        let mut rest = VecDeque::new();
        while let Some(notice) = self.pending.pop_front() {
            if notice.pass == ChildPass::Position && notice.agent == agent {
                positions.push(notice);
            } else {
                rest.push_back(notice);
            }
        }
        *self.pending = rest;
        while let Ok(notice) = self.notices.try_recv() {
            if notice.pass == ChildPass::Position && notice.agent == agent {
                positions.push(notice);
            } else {
                self.pending.push_back(notice);
            }
        }
        positions
    }
}

async fn apply_notice(
    simulation: &SimulationState,
    config: &RegionConfig,
    frame: &TerrainFrame,
    client: &SimulationClient,
    notice: &ChildNotice,
    visible: &[VisibleNeighbour],
    inbox: Option<&mut Inbox<'_>>,
) {
    let Some(identity) = simulation.child_identity(notice.agent) else {
        // Nothing to post. Still release a busy gate.
        finish(frame, notice, true);
        return;
    };
    match notice.pass {
        ChildPass::Enable => {
            enable(
                simulation, config, frame, client, notice, &identity, visible, inbox,
            )
            .await;
            finish(frame, notice, true);
        }
        ChildPass::Position => {
            let updated =
                send_open_positions(simulation, config, client, notice, &identity, None).await;
            finish(frame, notice, updated);
        }
    }
}

fn finish(frame: &TerrainFrame, notice: &ChildNotice, committed: bool) {
    if !notice.settle {
        return;
    }
    if let Err(error) = frame.settle_child(notice.agent, notice.epoch, committed) {
        tracing::debug!("child pass was not settled: {error}");
    }
}

async fn enable(
    simulation: &SimulationState,
    config: &RegionConfig,
    frame: &TerrainFrame,
    client: &SimulationClient,
    notice: &ChildNotice,
    identity: &ChildIdentity,
    visible: &[VisibleNeighbour],
    mut inbox: Option<&mut Inbox<'_>>,
) {
    if let Err(error) = frame.note_child_neighbours(notice.agent, !visible.is_empty()) {
        tracing::debug!("child neighbour flag was not stored: {error}");
    }
    let book = simulation.child_book();
    let before = book.tracked(notice.agent);
    let visible_ids: HashSet<Uuid> = visible
        .iter()
        .map(|neighbour| neighbour.region.region_id)
        .collect();
    close_absent(client, &book, notice.agent, &before, &visible_ids).await;
    for neighbour in visible {
        if before
            .iter()
            .any(|child| child.end.region_id == neighbour.region.region_id)
        {
            continue;
        }
        admit_one(
            simulation,
            config,
            frame,
            client,
            notice,
            identity,
            neighbour,
            inbox.as_deref_mut(),
        )
        .await;
    }
    let staying: HashSet<Uuid> = before
        .iter()
        .filter(|child| child.open && !child.promoted && visible_ids.contains(&child.end.region_id))
        .map(|child| child.end.region_id)
        .collect();
    send_open_positions(simulation, config, client, notice, identity, Some(&staying)).await;
}

async fn close_absent(
    client: &SimulationClient,
    book: &crate::children::ChildBook,
    agent: Uuid,
    before: &[crate::children::TrackedChild],
    visible_ids: &HashSet<Uuid>,
) {
    for child in before {
        if visible_ids.contains(&child.end.region_id) {
            continue;
        }
        if let Admission::Close { end, session } = book.leave_view(agent, child.end.region_id) {
            close_child(client, &end, agent, session).await;
        }
    }
}

async fn admit_one(
    simulation: &SimulationState,
    config: &RegionConfig,
    frame: &TerrainFrame,
    client: &SimulationClient,
    notice: &ChildNotice,
    identity: &ChildIdentity,
    neighbour: &VisibleNeighbour,
    mut inbox: Option<&mut Inbox<'_>>,
) {
    let book = simulation.child_book();
    let Claim::New { seed } =
        book.claim(notice.agent, identity.generation, neighbour.region.clone())
    else {
        return;
    };
    let work = ClaimWork::watch(
        Arc::clone(&book),
        notice.agent,
        identity.generation,
        neighbour.region.region_id,
    );
    let Some(addr) = neighbour.viewer_endpoint().await else {
        drop(work);
        return;
    };
    let IpAddr::V4(ip) = addr.ip() else {
        drop(work);
        return;
    };
    let height = avatar_height(notice, identity);
    let mut circuit = child_circuit(
        identity,
        &home_end(config),
        &neighbour.region,
        notice.position,
        notice.draw_distance,
        height,
        &seed,
    );
    let report = match client
        .create_visible_child(&neighbour.region, &mut circuit)
        .await
    {
        Ok(outcome) => PostReport::from_create(&outcome),
        Err(error) => {
            tracing::debug!("child create did not finish: {error}");
            PostReport::Transport
        }
    };
    if report == PostReport::Accepted {
        crate::children::pause_before_announce().await;
    }
    let admission = book.report(
        notice.agent,
        identity.generation,
        neighbour.region.region_id,
        report,
    );
    match &admission {
        Admission::Announce { .. } => {
            // The child is open. Deliver a position that arrived during the
            // announce wait before the viewer learns the seed URL. The
            // neighbour then sees the PUT finish, and only afterwards the
            // viewer's seed request.
            if let Some(inbox) = inbox.as_deref_mut() {
                for next in inbox.take_positions(notice.agent) {
                    let updated =
                        send_open_positions(simulation, config, client, &next, identity, None)
                            .await;
                    finish(frame, &next, updated);
                }
            }
            let view = NeighbourView {
                handle: region_handle(neighbour.region.loc_x, neighbour.region.loc_y),
                address: ip,
                port: addr.port(),
                size_x: neighbour.size[0],
                size_y: neighbour.size[1],
                agent: notice.agent,
                seed,
            };
            for event in admission_events(&admission, &view) {
                if let Err(error) = simulation.caps().enqueue_for(notice.agent, event) {
                    tracing::debug!("child event was not queued: {error}");
                }
            }
        }
        Admission::Close { end, session } => close_child(client, end, notice.agent, *session).await,
        Admission::Quiet => {}
    }
    work.settle();
}

/// `Some(only)` limits the PUT to those regions. `None` updates every open
/// child that is not the remote root.
async fn send_open_positions(
    simulation: &SimulationState,
    config: &RegionConfig,
    client: &SimulationClient,
    notice: &ChildNotice,
    identity: &ChildIdentity,
    only: Option<&HashSet<Uuid>>,
) -> bool {
    let mut updated = false;
    for child in simulation.child_book().tracked(notice.agent) {
        if !child.open || child.promoted {
            continue;
        }
        if only.is_some_and(|regions| !regions.contains(&child.end.region_id)) {
            continue;
        }
        if put_position(client, config, notice, identity, &child.end).await {
            updated = true;
        }
    }
    updated
}

async fn put_position(
    client: &SimulationClient,
    config: &RegionConfig,
    notice: &ChildNotice,
    identity: &ChildIdentity,
    end: &RegionEnd,
) -> bool {
    let height = avatar_height(notice, identity);
    let data = AgentPosition {
        agent_id: identity.agent_id,
        session_id: identity.session_id,
        region_handle: config.handle(),
        position: notice.position,
        velocity: notice.velocity,
        center: notice.camera,
        // Pin8's default avatar size, with the live height.
        size: [CHILD_SIZE_X, CHILD_SIZE_Y, height],
        far: notice.draw_distance,
        // Ferroan does not store a changing viewer-UI god. The hop sends false.
        viewer_ui_is_god: Some(false),
        // Q85 owns the packed viewer throttles.
        throttles: None,
    };
    match client.update_child_position(end, data).await {
        Ok(sent) => sent,
        Err(error) => {
            tracing::debug!("child position was not sent: {error}");
            false
        }
    }
}

async fn close_child(client: &SimulationClient, end: &RegionEnd, agent: Uuid, session: Uuid) {
    if let Err(error) = client.close_agent(end, agent, &session.to_string()).await {
        tracing::debug!("child close was not sent: {error}");
    }
}

fn avatar_height(notice: &ChildNotice, identity: &ChildIdentity) -> f32 {
    [notice.height, identity.height, CHILD_HEIGHT]
        .into_iter()
        .find(|height| height.is_finite() && *height > 0.0)
        .unwrap_or(CHILD_HEIGHT)
}

fn home_end(config: &RegionConfig) -> RegionEnd {
    RegionEnd {
        server_uri: config.server_uri.clone(),
        region_id: config.id,
        name: config.name.clone(),
        loc_x: config.location[0].wrapping_mul(256),
        loc_y: config.location[1].wrapping_mul(256),
    }
}

fn region_handle(loc_x: u32, loc_y: u32) -> u64 {
    (u64::from(loc_x) << 32) | u64::from(loc_y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Request;
    use ferroan_client::CapsRegistry;
    use serde_json::{Value, json};
    use std::net::SocketAddr;
    use tokio::sync::mpsc;

    const CREATE_REQUEST: &str =
        include_str!("../../../fixtures/region-login/create-agent.request.json");

    fn presence_reply(user: Uuid) -> String {
        format!(
            "<?xml version=\"1.0\"?><ServerResponse><result type=\"List\"><UserID>{user}</UserID></result></ServerResponse>"
        )
    }

    fn region_config(base: &str, region: Uuid) -> RegionConfig {
        let server = ferroan_config::Config::parse(&format!(
            "[Network]\nbind_ip_address=127.0.0.1\nhttp_listener_port=9000\n[GridService]\nGridServerURI={base}\n[PresenceService]\nPresenceServerURI={base}\n[GridUserService]\nGridUserServerURI={base}\n[Map]\nGenerateMaptiles=false\n"
        ))
        .unwrap();
        let regions = ferroan_config::Config::parse(&format!(
            "[Fixture Region]\nRegionUUID={region}\nLocation=1000,1000\nInternalAddress=127.0.0.1\nInternalPort=9000\nExternalHostName=127.0.0.1\n"
        ))
        .unwrap();
        RegionConfig::from_configs(&server, &regions).unwrap()
    }

    #[tokio::test]
    async fn frame_drive_admits_updates_and_closes_a_visible_neighbour() {
        let args: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
        let session = Uuid::parse_str(args["session_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(args["destination_uuid"].as_str().unwrap()).unwrap();
        let home_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let home_base = format!("http://{}", home_listener.local_addr().unwrap());
        let caps = CapsRegistry::new(&home_base).unwrap();
        let (accepted_tx, accepted_rx) = mpsc::channel(1);
        let config = region_config(&home_base, region);
        let state = SimulationState::new(&config, caps.clone(), accepted_tx);
        let app = state.clone().router().merge(caps.router()).route(
            "/presence",
            axum::routing::post(move || {
                let body = presence_reply(agent);
                async move { body }
            }),
        );
        let home_server = tokio::spawn(async move {
            axum::serve(
                home_listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let (recorded_tx, mut recorded_rx) = mpsc::channel(8);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = tokio::spawn(async move {
            let app = axum::Router::new().fallback(move |request: Request| {
                let recorded_tx = recorded_tx.clone();
                async move {
                    let method = request.method().to_string();
                    let path = request.uri().to_string();
                    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                        .await
                        .unwrap();
                    let plain = if bytes.is_empty() {
                        Vec::new()
                    } else {
                        let mut decoder = flate2::read::GzDecoder::new(bytes.as_ref());
                        let mut plain = Vec::new();
                        std::io::Read::read_to_end(&mut decoder, &mut plain).unwrap();
                        plain
                    };
                    recorded_tx.send((method, path, plain)).await.unwrap();
                    (
                        axum::http::StatusCode::OK,
                        r#"{"success":true,"reason":""}"#,
                    )
                }
            });
            axum::serve(neighbour_listener, app).await.unwrap();
        });

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let created = http
            .post(format!("{home_base}/agent/{agent}/"))
            .header("content-type", "application/json")
            .body(CREATE_REQUEST)
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), reqwest::StatusCode::OK);
        let created: Value = serde_json::from_str(&created.text().await.unwrap()).unwrap();
        assert_eq!(created["success"], true);

        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let neighbour = VisibleNeighbour::at(
            RegionEnd {
                server_uri: neighbour_uri,
                region_id: Uuid::from_u128(9),
                name: "Neighbour".into(),
                loc_x: 256_256,
                loc_y: 256_000,
            },
            [256, 256],
            "127.0.0.1",
            neighbour_addr.port(),
        );
        let enable = ChildNotice {
            agent,
            pass: ChildPass::Enable,
            position: [200.0, 128.0, 30.0],
            velocity: [0.0, 0.0, 0.0],
            camera: [200.0, 128.0, 31.0],
            draw_distance: 255.0,
            height: 1.9,
            god: false,
            settle: true,
            epoch: 1,
        };
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable,
            std::slice::from_ref(&neighbour),
            None,
        )
        .await;
        let (method, path, body) = recorded_rx.recv().await.unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, format!("/agent/{agent}/"));
        let posted: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(posted["child"], true);
        assert!(state.child_book().seed(agent, Uuid::from_u128(9)).is_some());
        assert!(recorded_rx.try_recv().is_err());

        let caps_path = args["caps_path"].as_str().unwrap();
        let seed = http
            .post(format!("{home_base}/CAPS/{caps_path}0000/"))
            .header("content-type", "application/llsd+xml")
            .body("<llsd><array><string>EventQueueGet</string></array></llsd>")
            .send()
            .await
            .unwrap();
        assert_eq!(seed.status(), reqwest::StatusCode::OK);
        let seed_xml = seed.text().await.unwrap();
        let events_url = ferroan_llsd::from_xml(seed_xml.as_bytes())
            .unwrap()
            .get("EventQueueGet")
            .as_str()
            .to_owned();
        let events = http.post(events_url).body(Vec::new()).send().await.unwrap();
        assert!(events.status().is_success());
        let events = events.text().await.unwrap();
        let enable_at = events.find("EnableSimulator").unwrap();
        let establish_at = events.find("EstablishAgentCommunication").unwrap();
        assert!(enable_at < establish_at);

        // An already-open neighbour is not created again. EnableChildAgents
        // still sends AgentPosition to the children that stayed in view.
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable,
            std::slice::from_ref(&neighbour),
            None,
        )
        .await;
        let (method, path, body) = recorded_rx.recv().await.unwrap();
        assert_eq!(method, "PUT");
        assert_eq!(path, format!("/agent/{agent}/"));
        let posted: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(posted["message_type"], "AgentPosition");
        assert!(recorded_rx.try_recv().is_err());

        let mut position = enable.clone();
        position.pass = ChildPass::Position;
        position.position = [220.0, 128.0, 30.0];
        position.epoch = 2;
        apply_notice(&state, &config, &frame, &client, &position, &[], None).await;
        let (method, path, body) = recorded_rx.recv().await.unwrap();
        assert_eq!(method, "PUT");
        assert_eq!(path, format!("/agent/{agent}/"));
        let posted: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(posted["message_type"], "AgentPosition");
        assert_eq!(posted["session_uuid"], json!(session.to_string()));
        assert_eq!(posted["size"], "<0.45, 0.6, 1.9>");
        assert!(recorded_rx.try_recv().is_err());

        apply_notice(&state, &config, &frame, &client, &enable, &[], None).await;
        let (method, path, _) = recorded_rx.recv().await.unwrap();
        assert_eq!(method, "DELETE");
        assert!(path.contains(&format!("/agent/{agent}/")));
        assert!(path.contains(&format!("auth={session}")));
        assert!(state.child_book().seed(agent, Uuid::from_u128(9)).is_none());

        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }
}
