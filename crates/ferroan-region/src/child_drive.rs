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
//! One task receives each [`ferroan_scene::ChildNotice`] and diffs
//! [`crate::grid::VisibleNeighbours`] on an enable. Closes and new claims are
//! decided before the 2 s gate is settled, which is when Pin8 clears
//! `m_childUpdatesBusy` and stamps the timer (`ScenePresence.cs` 4772–4778).
//! The closes go out first. Each neighbour then posts, waits 500 ms and
//! announces, or puts a child that stayed, in that neighbour's order, on a
//! task the drive does not await. A position pass sends one `AgentPosition`
//! per simulator. A position update goes only to an open child, so a PUT
//! cannot land before the child exists.

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;

use ferroan_scene::{ChildNotice, ChildPass, TerrainFrame};
use uuid::Uuid;

use crate::children::{Admission, NeighbourView, PostReport, admission_events};
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
    loop {
        let Some(notice) = notices.recv().await else {
            break;
        };
        // The lookup is the part this task waits for. Admission HTTP is a
        // separate task, so the next position pass is not held behind a POST.
        // Aborting this task does not abort that HTTP. The caller awaits
        // those handles, then sends any close a guard staged.
        let visible = if notice.pass == ChildPass::Enable {
            neighbours
                .in_view(notice.position, notice.draw_distance)
                .await
        } else {
            Vec::new()
        };
        apply_notice(&simulation, &config, &frame, &client, &notice, &visible).await;
    }
}

/// Send a close that an aborted admission staged after logout. The drive
/// task has stopped, so its guard has already dropped.
pub async fn close_staged_after_stop(simulation: &SimulationState) {
    let staged = simulation.child_book().take_every_staged_close();
    if staged.is_empty() {
        return;
    }
    let client = match SimulationClient::new() {
        Ok(client) => client,
        Err(error) => {
            tracing::error!("ferroan-region: simulation client unavailable: {error}");
            return;
        }
    };
    for (agent, session, end) in staged {
        if let Err(error) = client.close_agent(&end, agent, &session.to_string()).await {
            tracing::debug!("dropped admission close: {error}");
        }
    }
}

async fn apply_notice(
    simulation: &SimulationState,
    config: &RegionConfig,
    frame: &TerrainFrame,
    client: &SimulationClient,
    notice: &ChildNotice,
    visible: &[VisibleNeighbour],
) {
    let Some(identity) = simulation.child_identity(notice.agent) else {
        // Nothing to post. Still release a busy gate.
        finish(frame, notice, true);
        return;
    };
    match notice.pass {
        ChildPass::Enable => {
            // The gate settles inside `enable`, before the HTTP task starts.
            enable(
                simulation, config, frame, client, notice, &identity, visible,
            );
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

/// One neighbour after the closes, in the order the lookup returned them.
enum EnableStep {
    /// A neighbour that was not already claimed. The guard is held until the
    /// POST is reported or the task is cancelled.
    Admit {
        neighbour: VisibleNeighbour,
        work: crate::children::ClaimWork,
    },
    /// An open child that stayed in view. One PUT, not folded by simulator.
    Stay(RegionEnd),
}

/// `EnableChildAgents` after the neighbour lookup. Decisions and the gate
/// settle here. HTTP runs on the returned task.
fn enable(
    simulation: &SimulationState,
    config: &RegionConfig,
    frame: &TerrainFrame,
    client: &SimulationClient,
    notice: &ChildNotice,
    identity: &ChildIdentity,
    visible: &[VisibleNeighbour],
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
    let mut closes = Vec::new();
    for child in &before {
        if visible_ids.contains(&child.end.region_id) {
            continue;
        }
        if let Admission::Close { end, session } =
            book.leave_view(notice.agent, child.end.region_id)
        {
            closes.push((end, session));
        }
    }
    // Sizes are local. Refresh before the cap return so a refused enable
    // still matches the neighbours this pass can see.
    book.refresh_sizes(
        notice.agent,
        &visible
            .iter()
            .map(|neighbour| (neighbour.region.region_id, neighbour.size))
            .collect::<Vec<_>>(),
    );
    if simulation.admission_full(notice.agent) {
        // Pin8 has no admission cap and would send each close now. These
        // closes are not claims, so a task the cap does not track sends them.
        spawn_detached_closes(Arc::clone(&book), client.clone(), notice.agent, closes);
        finish(frame, notice, true);
        return;
    }
    let mut steps = Vec::new();
    for neighbour in visible {
        let known = before
            .iter()
            .find(|child| child.end.region_id == neighbour.region.region_id);
        if let Some(child) = known {
            if child.open && !child.promoted {
                steps.push(EnableStep::Stay(child.end.clone()));
            }
            continue;
        }
        if let Ok(work) = book.claim_work_sized(
            notice.agent,
            identity.generation,
            neighbour.region.clone(),
            neighbour.size,
        ) {
            steps.push(EnableStep::Admit {
                neighbour: neighbour.clone(),
                work,
            });
        }
    }
    // Pin8 stamps and clears busy when `EnableChildAgents` returns, which is
    // after the lookup and the close and claim decisions, and before the
    // fire-and-forget HTTP. An arrival notice does not settle, so this leaves
    // the arrival stamp in place.
    finish(frame, notice, true);
    // Stage before the task starts. The guard lives in the future before
    // the first poll, so aborting an unpolled task releases every close.
    for (end, session) in &closes {
        book.stage_decided_close(notice.agent, *session, end.clone());
    }
    let pass = EnableHttp {
        simulation: simulation.clone(),
        config: config.clone(),
        client: client.clone(),
        notice: notice.clone(),
        identity: identity.clone(),
        closes: OwnedCloses {
            book: Arc::clone(&book),
            agent: notice.agent,
            pending: closes.into(),
        },
        steps,
    };
    simulation.track_admission(notice.agent, tokio::spawn(pass.run()));
}

struct EnableHttp {
    simulation: SimulationState,
    config: RegionConfig,
    client: SimulationClient,
    notice: ChildNotice,
    identity: ChildIdentity,
    closes: OwnedCloses,
    steps: Vec<EnableStep>,
}

impl EnableHttp {
    async fn run(mut self) {
        let idle = self.closes.idle();
        let closes = std::mem::replace(&mut self.closes, idle);
        send_decided_closes(&self.client, closes).await;
        // Take the steps so each admission can still borrow this pass.
        for step in std::mem::take(&mut self.steps) {
            match step {
                EnableStep::Admit { neighbour, work } => {
                    admit_one(&self, neighbour, work).await;
                }
                EnableStep::Stay(end) => {
                    let _ = put_position(
                        &self.client,
                        &self.config,
                        &self.notice,
                        &self.identity,
                        &end,
                    )
                    .await;
                }
            }
        }
    }
}

async fn admit_one(
    pass: &EnableHttp,
    neighbour: VisibleNeighbour,
    work: crate::children::ClaimWork,
) {
    let seed = work.seed().to_owned();
    let Some(addr) = neighbour.viewer_endpoint().await else {
        drop(work);
        return;
    };
    let IpAddr::V4(ip) = addr.ip() else {
        drop(work);
        return;
    };
    let height = avatar_height(&pass.notice, &pass.identity);
    let mut circuit = child_circuit(
        &pass.identity,
        &home_end(&pass.config),
        &neighbour.region,
        pass.notice.position,
        pass.notice.draw_distance,
        height,
        &seed,
    );
    let report = match pass
        .client
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
    let book = pass.simulation.child_book();
    let admission = book.report(
        pass.notice.agent,
        pass.identity.generation,
        neighbour.region.region_id,
        report,
    );
    match &admission {
        Admission::Announce { .. } => {
            let view = NeighbourView {
                handle: region_handle(neighbour.region.loc_x, neighbour.region.loc_y),
                address: ip,
                port: addr.port(),
                size_x: neighbour.size[0],
                size_y: neighbour.size[1],
                agent: pass.notice.agent,
                seed,
            };
            for event in admission_events(&admission, &view) {
                if let Err(error) = pass.simulation.caps().enqueue_for(pass.notice.agent, event) {
                    tracing::debug!("child event was not queued: {error}");
                }
            }
        }
        Admission::Close { end, session } => {
            let session = *session;
            book.stage_decided_close(pass.notice.agent, session, end.clone());
            let mut handed =
                HandedClose::begin(Arc::clone(&book), pass.notice.agent, session, end.clone());
            close_child(&pass.client, end, pass.notice.agent, session).await;
            handed.returned();
            handed.ack();
        }
        Admission::Quiet => {}
    }
    work.settle();
}

/// `Some(only)` is the enable pass: one PUT per neighbour that stayed.
/// `None` is the position pass: one PUT per distinct `ServerURI`, which is
/// what `LocalSimulationConnector.UpdateAgent` applies to every scene there.
async fn send_open_positions(
    simulation: &SimulationState,
    config: &RegionConfig,
    client: &SimulationClient,
    notice: &ChildNotice,
    identity: &ChildIdentity,
    only: Option<&HashSet<Uuid>>,
) -> bool {
    let mut updated = false;
    let mut sent_servers = HashSet::new();
    for child in simulation.child_book().tracked(notice.agent) {
        if !child.open || child.promoted {
            continue;
        }
        if only.is_some_and(|regions| !regions.contains(&child.end.region_id)) {
            continue;
        }
        if only.is_none() && !sent_servers.insert(child.end.server_uri.clone()) {
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

/// Closes the cap refused to admit. The task is not tracked, so a full
/// admission list does not hold the DELETEs until serve end. A far hop still
/// skips them while they are in flight.
fn spawn_detached_closes(
    book: Arc<crate::children::ChildBook>,
    client: SimulationClient,
    agent: Uuid,
    closes: Vec<(RegionEnd, Uuid)>,
) {
    if closes.is_empty() {
        return;
    }
    for (end, session) in &closes {
        book.stage_decided_close(agent, *session, end.clone());
    }
    let pending = OwnedCloses {
        book,
        agent,
        pending: closes.into(),
    };
    tokio::spawn(async move {
        send_decided_closes(&client, pending).await;
    });
}

/// Closes staged before this task started. A far hop's take leaves an
/// in-flight one alone. Aborting the await acks a DELETE already handed to
/// HTTP and releases one that had not started, so serve end sends that one.
async fn send_decided_closes(client: &SimulationClient, mut pending: OwnedCloses) {
    let agent = pending.agent;
    let book = Arc::clone(&pending.book);
    while let Some((end, session)) = pending.next_close() {
        let mut handed = HandedClose::begin(Arc::clone(&book), agent, session, end.clone());
        close_child(client, &end, agent, session).await;
        // The ack is what drops an answered close. Without it the close stays
        // staged and a later take sends the DELETE again.
        handed.returned();
        handed.ack();
    }
}

/// Closes this task has not started. Drop releases them for a later take.
struct OwnedCloses {
    book: Arc<crate::children::ChildBook>,
    agent: Uuid,
    pending: VecDeque<(RegionEnd, Uuid)>,
}

impl OwnedCloses {
    fn idle(&self) -> Self {
        Self {
            book: Arc::clone(&self.book),
            agent: self.agent,
            pending: VecDeque::new(),
        }
    }

    fn next_close(&mut self) -> Option<(RegionEnd, Uuid)> {
        self.pending.pop_front()
    }
}

impl Drop for OwnedCloses {
    fn drop(&mut self) {
        for (end, session) in self.pending.drain(..) {
            self.book.release_unsent_close(self.agent, session, &end);
        }
    }
}

/// One decided close this task has handed to HTTP.
///
/// Dropped during the send: the neighbour may already have the DELETE, so it
/// is acked and not sent again. Dropped after [`Self::returned`] without
/// [`Self::ack`]: the DELETE came back and a later take may send it.
#[must_use = "dropping a handed close before ack treats the DELETE as finished"]
struct HandedClose {
    book: Arc<crate::children::ChildBook>,
    agent: Uuid,
    session: Uuid,
    end: RegionEnd,
    returned: bool,
    acked: bool,
}

impl HandedClose {
    fn begin(
        book: Arc<crate::children::ChildBook>,
        agent: Uuid,
        session: Uuid,
        end: RegionEnd,
    ) -> Self {
        book.begin_decided_close(agent, session, &end);
        Self {
            book,
            agent,
            session,
            end,
            returned: false,
            acked: false,
        }
    }

    /// `close_child` returned, including a transport error.
    fn returned(&mut self) {
        self.returned = true;
    }

    fn ack(&mut self) {
        self.book
            .ack_decided_close(self.agent, self.session, &self.end);
        self.acked = true;
    }
}

impl Drop for HandedClose {
    fn drop(&mut self) {
        if self.acked {
            return;
        }
        if self.returned {
            self.book
                .release_answered_close(self.agent, self.session, &self.end);
            return;
        }
        self.book
            .ack_decided_close(self.agent, self.session, &self.end);
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::{Notify, mpsc};

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
        )
        .await;
        state.finish_admissions(agent).await;
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
        )
        .await;
        state.finish_admissions(agent).await;
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
        apply_notice(&state, &config, &frame, &client, &position, &[]).await;
        let (method, path, body) = recorded_rx.recv().await.unwrap();
        assert_eq!(method, "PUT");
        assert_eq!(path, format!("/agent/{agent}/"));
        let posted: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(posted["message_type"], "AgentPosition");
        assert_eq!(posted["session_uuid"], json!(session.to_string()));
        assert_eq!(posted["size"], "<0.45, 0.6, 1.9>");
        assert!(recorded_rx.try_recv().is_err());

        apply_notice(&state, &config, &frame, &client, &enable, &[]).await;
        state.finish_admissions(agent).await;
        let (method, path, _) = recorded_rx.recv().await.unwrap();
        assert_eq!(method, "DELETE");
        assert!(path.contains(&format!("/agent/{agent}/")));
        assert!(path.contains(&format!("auth={session}")));
        assert!(state.child_book().seed(agent, Uuid::from_u128(9)).is_none());

        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }

    fn enable_notice(agent: Uuid) -> ChildNotice {
        ChildNotice {
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
        }
    }

    fn neighbour_at(uri: &str, id: u128, port: u16, size: [u32; 2]) -> VisibleNeighbour {
        VisibleNeighbour::at(
            RegionEnd {
                server_uri: uri.to_owned(),
                region_id: Uuid::from_u128(id),
                name: "Neighbour".into(),
                loc_x: 256_256,
                loc_y: 256_000,
            },
            size,
            "127.0.0.1",
            port,
        )
    }

    /// A teleport's `CreateAgent` has `child = true`. The drive still posts
    /// once that generation is the root.
    #[tokio::test]
    async fn a_teleport_root_posts_children_after_it_is_marked_root() {
        let mut body: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        body["child"] = json!(true);
        body["teleport_flags"] = json!("16");
        let agent = Uuid::parse_str(body["agent_id"].as_str().unwrap()).unwrap();
        let session = Uuid::parse_str(body["session_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(body["destination_uuid"].as_str().unwrap()).unwrap();
        let home_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let home_base = format!("http://{}", home_listener.local_addr().unwrap());
        let caps = CapsRegistry::new(&home_base).unwrap();
        let (accepted_tx, mut accepted_rx) = mpsc::channel(1);
        let config = region_config(&home_base, region);
        let state = SimulationState::new(&config, caps.clone(), accepted_tx);
        let app = state.clone().router().merge(caps.router()).route(
            "/presence",
            axum::routing::post(move || {
                let reply = presence_reply(agent);
                async move { reply }
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
        let (recorded_tx, mut recorded_rx) = mpsc::channel(4);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = tokio::spawn(async move {
            let app = axum::Router::new().fallback(move |request: Request| {
                let recorded_tx = recorded_tx.clone();
                async move {
                    let method = request.method().to_string();
                    let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                    recorded_tx.send(method).await.unwrap();
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
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), reqwest::StatusCode::OK);
        let accepted = accepted_rx.recv().await.unwrap();
        let stored = state.child_identity(agent).unwrap();
        assert_eq!(stored.session_id, session);
        accepted.mark_root();
        assert_eq!(state.child_identity(agent).unwrap().session_id, session);

        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let neighbour = neighbour_at(&neighbour_uri, 9, neighbour_addr.port(), [256, 256]);
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            std::slice::from_ref(&neighbour),
        )
        .await;
        state.finish_admissions(agent).await;
        assert_eq!(recorded_rx.recv().await.unwrap(), "POST");
        drop(accepted);
        home_server.abort();
        neighbour_server.abort();
    }

    /// A later visibility child must not replace the root identity still held.
    #[tokio::test]
    async fn a_visibility_child_does_not_replace_a_held_root_identity() {
        let root_body: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        let agent = Uuid::parse_str(root_body["agent_id"].as_str().unwrap()).unwrap();
        let root_session = Uuid::parse_str(root_body["session_id"].as_str().unwrap()).unwrap();
        let region = Uuid::parse_str(root_body["destination_uuid"].as_str().unwrap()).unwrap();
        let home_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let home_base = format!("http://{}", home_listener.local_addr().unwrap());
        let caps = CapsRegistry::new(&home_base).unwrap();
        let (accepted_tx, mut accepted_rx) = mpsc::channel(1);
        let config = region_config(&home_base, region);
        let state = SimulationState::new(&config, caps.clone(), accepted_tx);
        let app = state.clone().router().merge(caps.router()).route(
            "/presence",
            axum::routing::post(move || {
                let reply = presence_reply(agent);
                async move { reply }
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
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let post = |body: Value| {
            let http = http.clone();
            let home_base = home_base.clone();
            async move {
                http.post(format!("{home_base}/agent/{agent}/"))
                    .header("content-type", "application/json")
                    .body(body.to_string())
                    .send()
                    .await
                    .unwrap()
            }
        };
        assert_eq!(post(root_body).await.status(), reqwest::StatusCode::OK);
        let root = accepted_rx.recv().await.unwrap();
        assert_eq!(
            state.child_identity(agent).unwrap().session_id,
            root_session
        );
        let child_session = Uuid::from_u128(70);
        let mut child_body: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        child_body["child"] = json!(true);
        child_body["teleport_flags"] = json!("0");
        child_body["session_id"] = json!(child_session.to_string());
        child_body["secure_session_id"] = json!(Uuid::from_u128(71).to_string());
        child_body["caps_path"] = json!("00000000000040008000000000000099");
        child_body["circuit_code"] = json!("2");
        assert_eq!(post(child_body).await.status(), reqwest::StatusCode::OK);
        assert_eq!(
            state.child_identity(agent).unwrap().session_id,
            root_session,
            "a visibility child replaced the root identity"
        );
        let (recorded_tx, mut recorded_rx) = mpsc::channel(4);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = tokio::spawn(async move {
            let app = axum::Router::new().fallback(move |request: Request| {
                let recorded_tx = recorded_tx.clone();
                async move {
                    let method = request.method().to_string();
                    let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                    recorded_tx.send(method).await.unwrap();
                    (
                        axum::http::StatusCode::OK,
                        r#"{"success":true,"reason":""}"#,
                    )
                }
            });
            axum::serve(neighbour_listener, app).await.unwrap();
        });
        drop(root);
        assert_eq!(
            state.child_identity(agent).unwrap().session_id,
            child_session,
            "the successor identity was not published when the old lease dropped"
        );
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let neighbour = neighbour_at(&neighbour_uri, 9, neighbour_addr.port(), [256, 256]);
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            std::slice::from_ref(&neighbour),
        )
        .await;
        let method = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            state.finish_admissions(agent).await;
            recorded_rx.recv().await
        })
        .await
        .expect("enable between the lease drop and mark_root did not post");
        assert_eq!(method.unwrap(), "POST");
        let child = accepted_rx.recv().await.unwrap();
        child.mark_root();
        assert_eq!(
            state.child_identity(agent).unwrap().session_id,
            child_session
        );
        home_server.abort();
        neighbour_server.abort();
    }

    /// The position pass folds regions that share a simulator. The enable
    /// pass still puts once per neighbour.
    #[tokio::test]
    async fn position_pass_sends_one_update_per_server_uri() {
        let args: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
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
                let reply = presence_reply(agent);
                async move { reply }
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
                    let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                    recorded_tx.send(method).await.unwrap();
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
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let port = neighbour_addr.port();
        let neighbours = [
            neighbour_at(&neighbour_uri, 9, port, [512, 512]),
            VisibleNeighbour::at(
                RegionEnd {
                    server_uri: neighbour_uri.clone(),
                    region_id: Uuid::from_u128(10),
                    name: "Neighbour Two".into(),
                    loc_x: 256_512,
                    loc_y: 256_000,
                },
                [256, 256],
                "127.0.0.1",
                port,
            ),
        ];
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &neighbours,
        )
        .await;
        state.finish_admissions(agent).await;
        assert_eq!(recorded_rx.recv().await.unwrap(), "POST");
        assert_eq!(recorded_rx.recv().await.unwrap(), "POST");
        assert!(recorded_rx.try_recv().is_err());

        let mut position = enable_notice(agent);
        position.pass = ChildPass::Position;
        position.position = [220.0, 128.0, 30.0];
        apply_notice(&state, &config, &frame, &client, &position, &[]).await;
        assert_eq!(recorded_rx.recv().await.unwrap(), "PUT");
        assert!(
            recorded_rx.try_recv().is_err(),
            "one simulator received two position updates"
        );

        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &neighbours,
        )
        .await;
        state.finish_admissions(agent).await;
        assert_eq!(recorded_rx.recv().await.unwrap(), "PUT");
        assert_eq!(recorded_rx.recv().await.unwrap(), "PUT");
        assert!(recorded_rx.try_recv().is_err());

        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }

    /// Serve end aborts a blocked admission after logout and takes the close
    /// the guard staged. The enable itself returns before that HTTP finishes.
    #[tokio::test]
    async fn a_cancelled_admission_is_taken_at_serve_end() {
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
                let reply = presence_reply(agent);
                async move { reply }
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
        let hold = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hold_addr = hold.local_addr().unwrap();
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let neighbour = neighbour_at(
            &format!("http://{hold_addr}/"),
            11,
            hold_addr.port(),
            [256, 256],
        );
        let started = std::time::Instant::now();
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            std::slice::from_ref(&neighbour),
        )
        .await;
        assert!(
            started.elapsed() < std::time::Duration::from_millis(400),
            "the enable waited for admission HTTP"
        );
        let (mut sock, _) = tokio::time::timeout(std::time::Duration::from_secs(2), hold.accept())
            .await
            .unwrap()
            .unwrap();
        let mut buf = [0u8; 64];
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::io::AsyncReadExt::read(&mut sock, &mut buf),
        )
        .await;
        assert!(state.take_open_children(agent).is_empty());
        state.abort_admissions(agent).await;
        let staged = state.take_staged_closes(agent);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].0, session);
        assert_eq!(staged[0].1.region_id, Uuid::from_u128(11));
        assert!(state.take_staged_closes(agent).is_empty());
        drop(sock);
        drop(accepted_rx);
        home_server.abort();
    }

    /// Two neighbours share one listener. The first DELETE is held. Aborting
    /// the admission task must still stage the close that was never sent.
    #[tokio::test]
    async fn an_aborted_enable_stages_a_close_that_was_not_sent() {
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
                let reply = presence_reply(agent);
                async move { reply }
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
        let posts = Arc::new(AtomicUsize::new(0));
        let deletes = Arc::new(AtomicUsize::new(0));
        let held_path = Arc::new(std::sync::Mutex::new(String::new()));
        let first_delete = Arc::new(Notify::new());
        let hold = Arc::new(Notify::new());
        let first = Uuid::from_u128(21);
        let second = Uuid::from_u128(22);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = {
            let posts = Arc::clone(&posts);
            let deletes = Arc::clone(&deletes);
            let held_path = Arc::clone(&held_path);
            let first_delete = Arc::clone(&first_delete);
            let hold = Arc::clone(&hold);
            tokio::spawn(async move {
                let app = axum::Router::new().fallback(move |request: Request| {
                    let posts = Arc::clone(&posts);
                    let deletes = Arc::clone(&deletes);
                    let held_path = Arc::clone(&held_path);
                    let first_delete = Arc::clone(&first_delete);
                    let hold = Arc::clone(&hold);
                    async move {
                        let method = request.method().clone();
                        let path = request.uri().path().to_owned();
                        let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                        if method == axum::http::Method::DELETE {
                            if deletes.fetch_add(1, Ordering::SeqCst) == 0 {
                                *held_path.lock().unwrap() = path;
                                first_delete.notify_one();
                                hold.notified().await;
                            }
                        } else {
                            posts.fetch_add(1, Ordering::SeqCst);
                        }
                        (
                            axum::http::StatusCode::OK,
                            r#"{"success":true,"reason":""}"#,
                        )
                    }
                });
                axum::serve(neighbour_listener, app).await.unwrap();
            })
        };
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let port = neighbour_addr.port();
        let neighbours = [
            neighbour_at(&neighbour_uri, 21, port, [256, 256]),
            VisibleNeighbour::at(
                RegionEnd {
                    server_uri: neighbour_uri,
                    region_id: second,
                    name: "Neighbour Two".into(),
                    loc_x: 256_512,
                    loc_y: 256_000,
                },
                [256, 256],
                "127.0.0.1",
                port,
            ),
        ];
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &neighbours,
        )
        .await;
        state.finish_admissions(agent).await;
        assert_eq!(posts.load(Ordering::SeqCst), 2);
        assert!(state.child_book().seed(agent, first).is_some());
        assert!(state.child_book().seed(agent, second).is_some());

        apply_notice(&state, &config, &frame, &client, &enable_notice(agent), &[]).await;
        tokio::time::timeout(std::time::Duration::from_secs(2), first_delete.notified())
            .await
            .expect("the first close was not sent");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            deletes.load(Ordering::SeqCst),
            1,
            "the closes were not sent one at a time"
        );
        assert!(state.take_open_children(agent).is_empty());
        state.abort_admissions(agent).await;
        let staged = state.take_staged_closes(agent);
        assert_eq!(
            staged.len(),
            1,
            "a DELETE already on the wire was staged again"
        );
        assert_eq!(staged[0].0, session);
        let held = held_path.lock().unwrap().clone();
        assert!(
            !held.contains(&staged[0].1.region_id.to_string()),
            "the close that was already sent was returned"
        );
        assert!(held.contains(&first.to_string()) || held.contains(&second.to_string()));
        hold.notify_waiters();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            deletes.load(Ordering::SeqCst),
            1,
            "the held DELETE was sent a second time"
        );
        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }

    /// A take that does not abort the admission task must not return a close
    /// that task is still sending. Each neighbour then receives one DELETE.
    #[tokio::test]
    async fn a_take_while_the_close_is_in_flight_does_not_return_it() {
        let args: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
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
                let reply = presence_reply(agent);
                async move { reply }
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
        let posts = Arc::new(AtomicUsize::new(0));
        let deletes = Arc::new(AtomicUsize::new(0));
        let first_delete = Arc::new(Notify::new());
        let hold = Arc::new(Notify::new());
        let second = Uuid::from_u128(42);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = {
            let posts = Arc::clone(&posts);
            let deletes = Arc::clone(&deletes);
            let first_delete = Arc::clone(&first_delete);
            let hold = Arc::clone(&hold);
            tokio::spawn(async move {
                let app = axum::Router::new().fallback(move |request: Request| {
                    let posts = Arc::clone(&posts);
                    let deletes = Arc::clone(&deletes);
                    let first_delete = Arc::clone(&first_delete);
                    let hold = Arc::clone(&hold);
                    async move {
                        let method = request.method().clone();
                        let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                        if method == axum::http::Method::DELETE {
                            if deletes.fetch_add(1, Ordering::SeqCst) == 0 {
                                first_delete.notify_one();
                                hold.notified().await;
                            }
                        } else {
                            posts.fetch_add(1, Ordering::SeqCst);
                        }
                        (
                            axum::http::StatusCode::OK,
                            r#"{"success":true,"reason":""}"#,
                        )
                    }
                });
                axum::serve(neighbour_listener, app).await.unwrap();
            })
        };
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let port = neighbour_addr.port();
        let neighbours = [
            neighbour_at(&neighbour_uri, 41, port, [256, 256]),
            VisibleNeighbour::at(
                RegionEnd {
                    server_uri: neighbour_uri,
                    region_id: second,
                    name: "Neighbour Two".into(),
                    loc_x: 256_512,
                    loc_y: 256_000,
                },
                [256, 256],
                "127.0.0.1",
                port,
            ),
        ];
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &neighbours,
        )
        .await;
        state.finish_admissions(agent).await;
        assert_eq!(posts.load(Ordering::SeqCst), 2);

        apply_notice(&state, &config, &frame, &client, &enable_notice(agent), &[]).await;
        tokio::time::timeout(std::time::Duration::from_secs(2), first_delete.notified())
            .await
            .expect("the first close was not sent");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            deletes.load(Ordering::SeqCst),
            1,
            "the closes were not sent one at a time"
        );
        assert_eq!(state.child_book().staged_outstanding(agent), 2);
        assert!(
            state.take_staged_closes(agent).is_empty(),
            "a take without an abort returned a close the task is still sending"
        );
        assert_eq!(state.child_book().staged_outstanding(agent), 2);
        hold.notify_waiters();
        state.finish_admissions(agent).await;
        assert_eq!(deletes.load(Ordering::SeqCst), 2);
        assert_eq!(state.child_book().staged_outstanding(agent), 0);
        assert!(state.take_staged_closes(agent).is_empty());
        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }

    /// Aborting the enable before its first poll must release both closes.
    /// On the current-thread runtime the spawned task is not polled until
    /// this test yields, so the abort drops it unpolled.
    #[tokio::test(flavor = "current_thread")]
    async fn an_abort_before_the_enable_is_polled_returns_both_closes() {
        let args: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
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
                let reply = presence_reply(agent);
                async move { reply }
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
        let deletes = Arc::new(AtomicUsize::new(0));
        let second = Uuid::from_u128(42);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = {
            let deletes = Arc::clone(&deletes);
            tokio::spawn(async move {
                let app = axum::Router::new().fallback(move |request: Request| {
                    let deletes = Arc::clone(&deletes);
                    async move {
                        let method = request.method().clone();
                        let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                        if method == axum::http::Method::DELETE {
                            deletes.fetch_add(1, Ordering::SeqCst);
                        }
                        (
                            axum::http::StatusCode::OK,
                            r#"{"success":true,"reason":""}"#,
                        )
                    }
                });
                axum::serve(neighbour_listener, app).await.unwrap();
            })
        };
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let port = neighbour_addr.port();
        let neighbours = [
            neighbour_at(&neighbour_uri, 41, port, [256, 256]),
            VisibleNeighbour::at(
                RegionEnd {
                    server_uri: neighbour_uri,
                    region_id: second,
                    name: "Neighbour Two".into(),
                    loc_x: 256_512,
                    loc_y: 256_000,
                },
                [256, 256],
                "127.0.0.1",
                port,
            ),
        ];
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &neighbours,
        )
        .await;
        state.finish_admissions(agent).await;
        apply_notice(&state, &config, &frame, &client, &enable_notice(agent), &[]).await;
        state.abort_admissions(agent).await;
        assert_eq!(deletes.load(Ordering::SeqCst), 0);
        assert_eq!(state.child_book().staged_outstanding(agent), 2);
        assert_eq!(state.take_staged_closes(agent).len(), 2);
        assert_eq!(state.child_book().staged_outstanding(agent), 0);
        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }

    /// A pass whose DELETEs are answered leaves no close staged. Without the
    /// ack after each DELETE, both stay staged for a later take.
    #[tokio::test]
    async fn an_answered_close_is_not_staged() {
        let args: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
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
                let reply = presence_reply(agent);
                async move { reply }
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
        let (recorded_tx, mut recorded_rx) = mpsc::channel(8);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = tokio::spawn(async move {
            let app = axum::Router::new().fallback(move |request: Request| {
                let recorded_tx = recorded_tx.clone();
                async move {
                    let method = request.method().to_string();
                    let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                    recorded_tx.send(method).await.unwrap();
                    (
                        axum::http::StatusCode::OK,
                        r#"{"success":true,"reason":""}"#,
                    )
                }
            });
            axum::serve(neighbour_listener, app).await.unwrap();
        });
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let port = neighbour_addr.port();
        let second = Uuid::from_u128(52);
        let neighbours = [
            neighbour_at(&neighbour_uri, 51, port, [256, 256]),
            VisibleNeighbour::at(
                RegionEnd {
                    server_uri: neighbour_uri,
                    region_id: second,
                    name: "Neighbour Two".into(),
                    loc_x: 256_512,
                    loc_y: 256_000,
                },
                [256, 256],
                "127.0.0.1",
                port,
            ),
        ];
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &neighbours,
        )
        .await;
        state.finish_admissions(agent).await;
        assert_eq!(recorded_rx.recv().await.unwrap(), "POST");
        assert_eq!(recorded_rx.recv().await.unwrap(), "POST");

        apply_notice(&state, &config, &frame, &client, &enable_notice(agent), &[]).await;
        state.finish_admissions(agent).await;
        assert_eq!(recorded_rx.recv().await.unwrap(), "DELETE");
        assert_eq!(recorded_rx.recv().await.unwrap(), "DELETE");
        assert_eq!(state.child_book().staged_outstanding(agent), 0);
        assert!(
            state.take_staged_closes(agent).is_empty(),
            "an answered close was left staged"
        );
        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }

    /// The enable stores the size this pass reported for a child it already
    /// knows. Removing the size refresh leaves the size from the claim.
    #[tokio::test]
    async fn an_enable_stores_a_new_size_for_a_known_child() {
        let args: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
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
                let reply = presence_reply(agent);
                async move { reply }
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
        let (recorded_tx, mut recorded_rx) = mpsc::channel(4);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = tokio::spawn(async move {
            let app = axum::Router::new().fallback(move |request: Request| {
                let recorded_tx = recorded_tx.clone();
                async move {
                    let method = request.method().to_string();
                    let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                    recorded_tx.send(method).await.unwrap();
                    (
                        axum::http::StatusCode::OK,
                        r#"{"success":true,"reason":""}"#,
                    )
                }
            });
            axum::serve(neighbour_listener, app).await.unwrap();
        });
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let neighbour_id = Uuid::from_u128(61);
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &[neighbour_at(
                &neighbour_uri,
                61,
                neighbour_addr.port(),
                [256, 256],
            )],
        )
        .await;
        state.finish_admissions(agent).await;
        assert_eq!(recorded_rx.recv().await.unwrap(), "POST");
        assert_eq!(
            state.child_book().destination_size(agent, neighbour_id),
            Some([256, 256])
        );

        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &[neighbour_at(
                &neighbour_uri,
                61,
                neighbour_addr.port(),
                [512, 400],
            )],
        )
        .await;
        assert_eq!(
            state.child_book().destination_size(agent, neighbour_id),
            Some([512, 400])
        );
        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }

    /// Eight admission tasks already running. The next enable sends the close
    /// it decided and does not claim the new neighbour.
    #[tokio::test]
    async fn an_enable_past_the_admission_cap_sends_the_close_and_does_not_claim() {
        let args: Value = serde_json::from_str(CREATE_REQUEST).unwrap();
        let agent = Uuid::parse_str(args["agent_id"].as_str().unwrap()).unwrap();
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
                let reply = presence_reply(agent);
                async move { reply }
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
        let (recorded_tx, mut recorded_rx) = mpsc::channel(8);
        let neighbour_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let neighbour_addr = neighbour_listener.local_addr().unwrap();
        let neighbour_uri = format!("http://{neighbour_addr}/");
        let neighbour_server = tokio::spawn(async move {
            let app = axum::Router::new().fallback(move |request: Request| {
                let recorded_tx = recorded_tx.clone();
                async move {
                    let method = request.method().to_string();
                    let _ = axum::body::to_bytes(request.into_body(), 1024 * 1024).await;
                    recorded_tx.send(method).await.unwrap();
                    (
                        axum::http::StatusCode::OK,
                        r#"{"success":true,"reason":""}"#,
                    )
                }
            });
            axum::serve(neighbour_listener, app).await.unwrap();
        });
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let client = SimulationClient::new().unwrap();
        let port = neighbour_addr.port();
        let first = Uuid::from_u128(31);
        let second = Uuid::from_u128(32);
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &[neighbour_at(&neighbour_uri, 31, port, [256, 256])],
        )
        .await;
        state.finish_admissions(agent).await;
        assert_eq!(recorded_rx.recv().await.unwrap(), "POST");
        assert!(state.child_book().seed(agent, first).is_some());
        for _ in 0..SimulationState::CHILD_ADMISSION_TASKS {
            state.track_admission(agent, tokio::spawn(std::future::pending::<()>()));
        }
        apply_notice(
            &state,
            &config,
            &frame,
            &client,
            &enable_notice(agent),
            &[neighbour_at(&neighbour_uri, 32, port, [256, 256])],
        )
        .await;
        let method = tokio::time::timeout(std::time::Duration::from_secs(2), recorded_rx.recv())
            .await
            .expect("the capped close was not sent")
            .unwrap();
        assert_eq!(method, "DELETE");
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while state.child_book().staged_outstanding(agent) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the capped close stayed staged");
        assert!(state.take_staged_closes(agent).is_empty());
        assert!(
            recorded_rx.try_recv().is_err(),
            "a full admission list still posted a child"
        );
        assert!(state.child_book().seed(agent, second).is_none());
        state.abort_admissions(agent).await;
        drop(accepted_rx);
        home_server.abort();
        neighbour_server.abort();
    }
}
