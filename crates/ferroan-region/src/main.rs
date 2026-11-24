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
// Root-arrival and logout behavior ported from
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/GridUser/ActivityDetector.cs
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Presence/PresenceDetector.cs
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs (Lookat)
//   Source/OpenSim.Region.Framework/Scenes/Scene.LoginArrivals.cs
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs (ping and outgoing workers)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs (async movement dispatch)
//   Source/OpenSim.Region.CoreModules/Framework/EntityTransfer/EntityTransferModule.cs
//     (TeleportAgentWithinRegion)
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: async lifecycle, generation-safe cleanup and process orchestration.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Register a single region with C# Robust and accept its local login messages.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use ferroan_client::{CapsRegistry, CircuitServer, CircuitStop, LandLayer};
use ferroan_config::{Config, RegionLoad};
use ferroan_region::config::{RegionConfig, RegionConfigError};
use ferroan_region::fetch_caps::FetchCaps;
use ferroan_region::grid::{GridClient, GridError, GridUserClient, PresenceClient};
use ferroan_region::simulation::{
    ADMISSION_QUEUE, AcceptedAgent, SimulationState, release_callback,
};
use ferroan_region::source_hop::{HopEnv, HopLogin, HopPresence};
use ferroan_region::status::{
    RegionMeters, StatusState, metrics_router, region_router, with_request_counts,
};
use ferroan_scene::TerrainFrame;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("{0}")]
    Arguments(&'static str),
    #[error(transparent)]
    Config(#[from] RegionConfigError),
    #[error(transparent)]
    Log(#[from] ferroan_log::LogError),
    #[error(transparent)]
    Grid(#[from] GridError),
    #[error(transparent)]
    Caps(#[from] ferroan_client::CapsError),
    #[error(transparent)]
    Circuit(#[from] ferroan_client::CircuitError),
    #[error("region I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("region task: {0}")]
    Task(#[from] tokio::task::JoinError),
    #[error("the circuit dispatcher stopped")]
    DispatcherStopped,
    #[error("timed out draining region logout notifications")]
    LogoutDrainTimeout,
    #[error("terrain: {0}")]
    Terrain(String),
    #[error("objects: {0}")]
    Objects(String),
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ferroan-region: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), RunError> {
    let Some(load) = arguments()? else {
        return Ok(());
    };
    let config = RegionConfig::load(&load)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(serve(config))
}

fn arguments() -> Result<Option<RegionLoad>, RunError> {
    let mut argv = Config::default();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "usage: ferroan-region [--inimaster FILE] [--inifile FILE] [--inidirectory DIR] [--background]"
                );
                return Ok(None);
            }
            "--inimaster" | "--inifile" | "--inidirectory" => {
                let value = args
                    .next()
                    .ok_or(RunError::Arguments("configuration option needs a value"))?;
                argv.set("Startup", &arg[2..], &value);
            }
            "--background" => {}
            _ => {
                return Err(RunError::Arguments(
                    "unsupported command-line argument; use --help",
                ));
            }
        }
    }
    Ok(Some(RegionLoad {
        config_dir: env::current_dir()?,
        argv,
        environment: env::vars().collect(),
    }))
}

async fn serve(mut config: RegionConfig) -> Result<(), RunError> {
    let log_guard = ferroan_log::install(&config.logs)?;
    let terrain = load_terrain(&config).await?;
    let objects = load_stored_objects(&config).await?;
    config.resolve_control_plane().await?;
    // Install handlers before registration. A termination received while the
    // bounded grid request is in flight is handled as soon as it completes,
    // so a successful registration is still followed by deregistration.
    let mut shutdown = tokio::spawn(shutdown_signal()?);
    // Bind both sockets before advertising the region. Early viewer datagrams
    // remain in the socket until Robust supplies the matching circuit.
    let meters = Arc::new(RegionMeters::new());
    ferroan_diag::set_context("ferroan-region", &config.name);
    ferroan_diag::set_configuration(&format!(
        "server=ferroan-region name={} id={} http_bind={} udp_bind={}",
        config.name, config.id, config.http_bind, config.udp_bind
    ));
    ferroan_diag::set_snapshot({
        let meters = Arc::clone(&meters);
        Arc::new(move || meters.diag_lines())
    });
    let circuit = CircuitServer::bind_at(config.udp_bind)
        .await?
        .with_handshake(config.handshake.clone())
        .with_timing(config.circuit_timing)?
        .with_udp_counters(meters.udp());
    if let Some(path) = &config.udp_capture
        && let Err(error) = circuit.open_datagram_log(path)
    {
        tracing::warn!("ferroan-region: datagram capture did not open: {error}");
    }
    let listener = TcpListener::bind(config.http_bind).await?;
    let metrics_listener = match config.monitoring.metrics_listen {
        Some(listen) => Some(TcpListener::bind(listen).await?),
        None => None,
    };
    // PresenceDetector.AddRegion and ActivityDetector.AddRegion clear users
    // stranded by a previous process. Finish before exposing admission: stale
    // Robust registration may still address this listener. A refusal is
    // nonfatal, as in C#.
    notify_region_presence(&PresenceClient::new(&config.presence_server_uri), config.id).await;
    notify_startup(
        &GridUserClient::new(&config.grid_user_server_uri),
        config.id,
    )
    .await;
    let fetch = Arc::new(FetchCaps::new(&config));
    let caps = CapsRegistry::with_grants(
        &config.server_uri,
        config.cap_grants.clone(),
        Some(fetch.service()),
    )?;
    fetch.bind_uploader_routes(caps.uploader_routes());
    // One queued accept per admitted root, including the staff and god bypass.
    // The dispatcher spawns the circuit, so a same-agent replacement still fits
    // while the old task is running.
    let (accepted, agents) = mpsc::channel(ADMISSION_QUEUE);
    let mut simulation =
        SimulationState::with_meters(&config, caps.clone(), accepted, meters.clone());
    simulation.keep_objects(objects);
    if let Some(connection) = config.estate_store.as_deref() {
        match ferroan_scene::open_estate(connection, config.id).await {
            Ok(Some(access)) => simulation.apply_estate(access),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!("ferroan-region: estate tables were not read: {error}");
            }
        }
    }
    let status = StatusState::new(
        config.id,
        config.name.clone(),
        &config.monitoring,
        meters.clone(),
    );
    let mut app = simulation
        .clone()
        .router()
        .merge(caps.router())
        .merge(region_router(status));
    if let Some(console) = log_guard.console_router() {
        app = app.merge(console);
    }
    let app = with_request_counts(app, meters.clone());
    let (stop_http, stopped) = oneshot::channel();
    let mut http = tokio::spawn(async move {
        axum::serve(
            ferroan_http::accepted_nodelay(listener),
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
    });
    let (stop_metrics, mut metrics_task) = if let Some(listener) = metrics_listener {
        let metrics = metrics_router(meters.clone());
        let (stop_metrics, stopped_metrics) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(ferroan_http::accepted_nodelay(listener), metrics)
                .with_graceful_shutdown(async {
                    let _ = stopped_metrics.await;
                })
                .await
        });
        (Some(stop_metrics), Some(task))
    } else {
        (None, None)
    };
    let grid = GridClient::new(&config);
    if let Err(error) = grid.register(&config).await {
        let _ = stop_http.send(());
        http.abort();
        let _ = http.await;
        if let Some(stop) = stop_metrics {
            let _ = stop.send(());
        }
        if let Some(task) = metrics_task.as_mut() {
            task.abort();
            let _ = task.await;
        }
        return Err(error.into());
    }
    meters.set_registered(true);
    let region_name = &config.name;
    tracing::info!("INITIALIZATION COMPLETE FOR {region_name} - LOGINS ENABLED");
    let (stop_circuits, stopped_circuits) = watch::channel(false);
    let hop = HopEnv::from_region(&config, caps.clone(), HopPresence::live(simulation.clone()));
    let mut circuits = tokio::spawn(dispatch_circuits(
        circuit,
        config.clone(),
        terrain,
        agents,
        stopped_circuits,
        Arc::new(AtomicUsize::new(0)),
        hop,
    ));
    let mut circuits_consumed = false;
    let mut outcome: Result<(), RunError> = tokio::select! {
        signal = &mut shutdown => signal.map_err(RunError::Task).and_then(|result| result.map_err(RunError::Io)),
        result = &mut http => match result {
            Ok(result) => result.map_err(RunError::Io),
            Err(error) => Err(RunError::Task(error)),
        },
        result = &mut circuits => {
            circuits_consumed = true;
            match result {
                Ok(Ok(())) => Err(RunError::DispatcherStopped),
                Ok(Err(error)) => Err(RunError::Grid(error)),
                Err(error) => Err(RunError::Task(error)),
            }
        },
    };
    shutdown.abort();
    simulation.stop_accepting();
    let _ = stop_circuits.send(true);
    if !circuits_consumed {
        // One active generation and one queued replacement: two arrival writes,
        // up to six failed-login cleanup requests and one admission, each
        // bounded at 30s. Concurrent roots take the same budget. HTTP cleanup
        // holds one lifecycle lock, so a region at the ordinary root limit plus
        // the staff bypass can still outrun it. Shutdown never cancels a write
        // ahead of its cleanup.
        let drained = tokio::time::timeout(Duration::from_secs(275), &mut circuits).await;
        let error = match drained {
            Ok(Ok(Ok(()))) => None,
            Ok(Ok(Err(error))) => Some(RunError::Grid(error)),
            Ok(Err(error)) => Some(RunError::Task(error)),
            Err(_) => {
                circuits.abort();
                let _ = circuits.await;
                Some(RunError::LogoutDrainTimeout)
            }
        };
        if let Some(error) = error {
            tracing::error!("ferroan-region: logout cleanup failed: {error}");
            outcome = Err(error);
        }
    }
    // PresenceDetector.RemoveRegion. A failure is logged and shutdown continues.
    notify_region_presence(&PresenceClient::new(&config.presence_server_uri), config.id).await;
    meters.set_registered(false);
    let deregistered = grid.deregister(config.id).await;
    if let Some(stop) = stop_metrics {
        let _ = stop.send(());
    }
    if let Some(task) = metrics_task.as_mut()
        && !task.is_finished()
        && tokio::time::timeout(Duration::from_secs(5), &mut *task)
            .await
            .is_err()
    {
        task.abort();
        let _ = task.await;
    }
    let _ = stop_http.send(());
    if !http.is_finished()
        && tokio::time::timeout(Duration::from_secs(5), &mut http)
            .await
            .is_err()
    {
        http.abort();
        let _ = http.await;
    }
    deregistered?;
    tracing::info!("ferroan-region: deregistered");
    outcome
}

async fn notify_startup(grid_user: &GridUserClient, region: uuid::Uuid) {
    if let Err(error) = grid_user.logged_out_region(region).await {
        tracing::error!("ferroan-region: startup GridUser cleanup failed: {error}");
    }
}

async fn load_stored_objects(
    config: &RegionConfig,
) -> Result<Arc<Vec<ferroan_scene::ObjectGroup>>, RunError> {
    let stored =
        ferroan_region::objects::load_stored_objects(config.object_store.as_deref(), config.id)
            .await
            .map_err(RunError::Objects)?;
    let (objects, prims) = {
        let prims: usize = stored.groups.iter().map(|group| group.parts.len()).sum();
        (stored.groups.len(), prims)
    };
    tracing::info!(
        "ferroan-region: loaded {objects} objects, {prims} prims in {ms} ms",
        ms = stored.elapsed_ms
    );
    Ok(stored.groups)
}

async fn load_terrain(config: &RegionConfig) -> Result<Option<Arc<TerrainFrame>>, RunError> {
    let map = match config.terrain_store.as_deref() {
        Some(connection) => {
            let width = i32::try_from(config.size[0]).unwrap_or(i32::MAX);
            let height = i32::try_from(config.size[1]).unwrap_or(i32::MAX);
            ferroan_scene::open_terrain(connection, config.id, width, height)
                .await
                .map_err(|error| RunError::Terrain(error.to_string()))?
        }
        None => None,
    };
    let frame = match map {
        Some(map) => {
            TerrainFrame::start(map).map_err(|error| RunError::Terrain(error.to_string()))?
        }
        None => TerrainFrame::start_without_map()
            .map_err(|error| RunError::Terrain(error.to_string()))?,
    };
    Ok(Some(Arc::new(frame)))
}

/// `MakeRootAgent` without physics: lift Z, and take the precomputed land packets.
fn terrain_login(frame: Option<&TerrainFrame>, start: [f32; 3]) -> ([f32; 3], Vec<LandLayer>) {
    let Some(frame) = frame else {
        return (start, Vec::new());
    };
    if !frame.has_map() {
        return (start, Vec::new());
    }
    let mut position = start;
    match frame.ground(position[0], position[1]) {
        Ok(height) => position[2] = ferroan_scene::movement_z(position[2], height),
        Err(error) => tracing::error!("ferroan-region: terrain height: {error}"),
    }
    let land = match frame.layers() {
        Ok(layers) => layers
            .into_iter()
            .map(|layer| LandLayer {
                kind: layer.kind,
                data: layer.data,
            })
            .collect(),
        Err(error) => {
            tracing::error!("ferroan-region: terrain layers: {error}");
            Vec::new()
        }
    };
    (position, land)
}

async fn notify_region_presence(presence: &PresenceClient, region: uuid::Uuid) {
    if let Err(error) = presence.logout_region(region).await {
        tracing::error!("ferroan-region: Presence logoutregion failed: {error}");
    }
}

/// Aborts the circuit task when the dispatcher future is dropped. `JoinHandle`
/// on drop detaches, and the region tests abort the dispatcher.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl AbortOnDrop {
    fn join(mut self) -> impl Future<Output = ()> + Send {
        std::future::poll_fn(
            move |cx: &mut Context<'_>| match Pin::new(&mut self.0).poll(cx) {
                Poll::Ready(_) => Poll::Ready(()),
                Poll::Pending => Poll::Pending,
            },
        )
    }
}

#[derive(Clone)]
struct LifecycleClients {
    grid_user: GridUserClient,
    presence: PresenceClient,
}

struct DispatchCtx {
    circuit: CircuitServer,
    config: RegionConfig,
    terrain: Option<Arc<TerrainFrame>>,
    clients: LifecycleClients,
    stop: watch::Receiver<bool>,
    /// Tests wait until a dequeued agent has passed same-agent cleanup.
    started: Arc<AtomicUsize>,
    running: HashMap<Uuid, AbortOnDrop>,
    hop: HopEnv,
}

impl DispatchCtx {
    /// Join a same-agent predecessor before its replacement circuit starts.
    async fn launch(&mut self, agent: AcceptedAgent) {
        let agent_id = agent.expect.agent_id;
        if let Some(previous) = self.running.remove(&agent_id) {
            previous.join().await;
        }
        if *self.stop.borrow() {
            self.note_started();
            let position = agent.start_pos;
            settle(
                &self.clients.grid_user,
                &self.clients.presence,
                &self.config,
                agent,
                false,
                position,
            )
            .await;
            return;
        }
        let started = Arc::clone(&self.started);
        let circuit = self.circuit.clone();
        let config = self.config.clone();
        let terrain = self.terrain.clone();
        let clients = self.clients.clone();
        let stop = self.stop.clone();
        let hop = self.hop.clone();
        let handle = tokio::spawn(async move {
            started.fetch_add(1, Ordering::Release);
            serve_agent(circuit, config, terrain, agent, stop, clients, hop).await;
        });
        self.running.insert(agent_id, AbortOnDrop(handle));
    }

    fn note_started(&self) {
        self.started.fetch_add(1, Ordering::Release);
    }

    async fn join_running(&mut self) {
        let pending: Vec<AbortOnDrop> = self.running.drain().map(|(_, task)| task).collect();
        for task in pending {
            task.join().await;
        }
    }
}

async fn dispatch_circuits(
    circuit: CircuitServer,
    config: RegionConfig,
    terrain: Option<Arc<TerrainFrame>>,
    mut agents: mpsc::Receiver<AcceptedAgent>,
    stop: watch::Receiver<bool>,
    started: Arc<AtomicUsize>,
    hop: HopEnv,
) -> Result<(), GridError> {
    circuit.enable_chat();
    if let Some(frame) = terrain.as_ref() {
        let frame = Arc::clone(frame);
        circuit.set_login_scene(Arc::new(move |agent, pose| {
            let stored = frame
                .post_login(
                    agent,
                    ferroan_scene::Presence {
                        position: pose.position,
                        rotation: pose.rotation,
                        velocity: pose.velocity,
                        control_flags: pose.control_flags,
                        draw_distance: pose.draw_distance,
                    },
                )
                .map_err(|_| ())?;
            Ok(ferroan_client::LoginPose {
                position: stored.position,
                rotation: stored.rotation,
                velocity: stored.velocity,
                control_flags: stored.control_flags,
                draw_distance: stored.draw_distance,
            })
        }));
    }
    let mut ctx = DispatchCtx {
        circuit,
        clients: LifecycleClients {
            grid_user: GridUserClient::new(&config.grid_user_server_uri),
            presence: PresenceClient::new(&config.presence_server_uri),
        },
        config,
        terrain,
        stop,
        started,
        running: HashMap::new(),
        hop,
    };
    loop {
        let next = tokio::select! {
            biased;
            result = ctx.stop.changed() => {
                if result.is_err() || *ctx.stop.borrow() {
                    agents.close();
                    break;
                }
                continue;
            }
            next = agents.recv() => next,
        };
        let Some(agent) = next else { break };
        ctx.launch(agent).await;
    }
    while let Some(agent) = agents.recv().await {
        let agent_id = agent.expect.agent_id;
        if let Some(previous) = ctx.running.remove(&agent_id) {
            previous.join().await;
        }
        ctx.note_started();
        let position = agent.start_pos;
        settle(
            &ctx.clients.grid_user,
            &ctx.clients.presence,
            &ctx.config,
            agent,
            false,
            position,
        )
        .await;
    }
    ctx.join_running().await;
    Ok(())
}

async fn serve_agent(
    circuit: CircuitServer,
    config: RegionConfig,
    terrain: Option<Arc<TerrainFrame>>,
    mut agent: AcceptedAgent,
    mut stop: watch::Receiver<bool>,
    clients: LifecycleClients,
    hop: HopEnv,
) {
    let LifecycleClients {
        grid_user,
        presence,
    } = clients;
    let mut root_arrived = false;
    let already_stopped = *stop.borrow_and_update();
    let (position, land) = if already_stopped {
        (agent.start_pos, Vec::new())
    } else {
        terrain_login(terrain.as_deref(), agent.start_pos)
    };
    // Logout reports this. It starts as the login position and follows a local landing.
    let landed = Arc::new(Mutex::new(position));
    if !already_stopped {
        let avatar = ferroan_region::avatar::load(
            &config.avatar_server_uri,
            agent.expect.agent_id,
            &agent.first_name,
            &agent.last_name,
        )
        .await;
        let expect = agent.expect;
        let half_height = avatar.height / 2.0;
        let mut arrival = config.arrival(position);
        arrival.land = land;
        arrival.avatar = Some(avatar);
        // TeleportAgentWithinRegion. Godlike needs a cached user level and
        // IsFlying is not tracked, so both stay off. A handle this grid does
        // not know gets the not-found packet and one offline map block.
        // An in-view hop demotes this root and keeps the caps and the circuit.
        // SendLocalTeleport's queue is the circuit's.
        let ground_terrain = terrain.clone();
        arrival.teleport = Some(ferroan_client::TeleportRegion {
            world_x: config.location[0].wrapping_mul(256),
            world_y: config.location[1].wrapping_mul(256),
            size_x: config.size[0] as f32,
            size_y: config.size[1] as f32,
            default_landing: [128.0, 128.0, 30.0],
            half_height,
            allow_movement: true,
            godlike: false,
            flying: false,
            ground: Arc::new(move |x, y| match ground_terrain.as_ref() {
                Some(frame) if frame.has_map() => frame.ground(x, y).unwrap_or(21.0),
                _ => 21.0,
            }),
            position: Arc::clone(&landed),
        });
        arrival.return_slot = Some(agent.return_slot.clone());
        let mut promotions = agent.return_slot.take_promotions();
        arrival.source_hop = Some(hop.install(HopLogin {
            agent_id: expect.agent_id,
            session_id: expect.session_id,
            secure_session_id: agent.secure_session_id,
            circuit_code: expect.circuit_code,
            first_name: agent.first_name.clone(),
            last_name: agent.last_name.clone(),
        }));
        let (report, mut report_ready) = oneshot::channel();
        let (opened, mut opening) = oneshot::channel();
        let (close_tx, close_rx) = oneshot::channel();
        circuit.attach_chat(
            expect.agent_id,
            format!("{} {}", agent.first_name, agent.last_name),
            position,
        );
        let serving = circuit.serve_login(expect, arrival, report, opened, close_rx);
        tokio::pin!(serving);
        let mut close_tx = Some(close_tx);
        let mut reported = false;
        let mut connected = false;
        let deadline = agent.accepted_at + config.login_connect_wait;
        loop {
            tokio::select! {
                biased;
                result = stop.changed() => {
                    if result.is_err() || *stop.borrow() {
                        // Scene.Close kicks a connected viewer. An unopened
                        // login is dropped with no DisableSimulator.
                        if connected && let Some(tx) = close_tx.take() {
                            let _ = tx.send(CircuitStop::Shutdown);
                            match (&mut serving).await {
                                Ok(()) => tracing::info!("ferroan-region: LogoutReply sent"),
                                Err(error) => {
                                    tracing::error!("ferroan-region: circuit ended: {error}")
                                }
                            }
                        }
                        break;
                    }
                }
                _ = agent.replaced() => {
                    tracing::info!("ferroan-region: circuit replaced by a new login");
                    if let Some(tx) = close_tx.take() {
                        let _ = tx.send(CircuitStop::Disable);
                    }
                    match (&mut serving).await {
                        Ok(()) => tracing::info!("ferroan-region: LogoutReply sent"),
                        Err(error) => tracing::error!("ferroan-region: circuit ended: {error}"),
                    }
                    break;
                }
                _ = &mut opening, if !connected => { connected = true; }
                _ = tokio::time::sleep_until(deadline), if !connected && !config.login_connect_wait.is_zero() => {
                    tracing::error!("ferroan-region: login viewer did not connect before its deadline");
                    break;
                }
                report = &mut report_ready, if !reported => {
                    reported = true;
                    if let Ok(report) = report {
                        root_arrived = true;
                        // C# runs movement callbacks separately from ping,
                        // ACK and resend handling. Keep polling UDP during
                        // HTTP, but defer cancellation until writes finish.
                        let reporting = async {
                            let _guard = agent.lock_lifecycle().await;
                            if agent.is_current() {
                                agent.mark_root();
                                notify_root_arrival(&grid_user, &presence, &config, &agent, position).await;
                            }
                        };
                        tokio::pin!(reporting);
                        let ended = tokio::select! {
                            _ = &mut reporting => None,
                            result = &mut serving => {
                                // A logout or timeout cannot cancel a write
                                // which could otherwise race its cleanup.
                                reporting.await;
                                Some(result)
                            }
                        };
                        report.complete();
                        if let Some(result) = ended {
                            match result {
                                Ok(()) => tracing::info!("ferroan-region: LogoutReply sent"),
                                Err(error) => tracing::error!("ferroan-region: circuit ended: {error}"),
                            }
                            break;
                        }
                    }
                }
                promoted = async {
                    match promotions.as_mut() {
                        Some(incoming) => incoming.recv().await,
                        None => std::future::pending().await,
                    }
                }, if promotions.is_some() => {
                    let Some(token) = promoted else {
                        promotions = None;
                        continue;
                    };
                    // The parked circuit already consumed the login signal.
                    // This one makes the child root again, then the circuit
                    // sends AgentMovementComplete.
                    root_arrived = true;
                    let position = agent.return_slot.position();
                    *landed.lock().unwrap_or_else(|poison| poison.into_inner()) = position;
                    let reporting = async {
                        let _guard = agent.lock_lifecycle().await;
                        if agent.is_current() {
                            agent.mark_root();
                            if let Some(callback) = agent.return_slot.take_callback() {
                                tokio::spawn(release_callback(callback));
                            }
                            notify_root_arrival(
                                &grid_user,
                                &presence,
                                &config,
                                &agent,
                                position,
                            )
                            .await;
                        }
                    };
                    tokio::pin!(reporting);
                    let ended = tokio::select! {
                        _ = &mut reporting => None,
                        result = &mut serving => {
                            reporting.await;
                            Some(result)
                        }
                    };
                    token.complete();
                    if let Some(result) = ended {
                        match result {
                            Ok(()) => tracing::info!("ferroan-region: LogoutReply sent"),
                            Err(error) => {
                                tracing::error!("ferroan-region: circuit ended: {error}")
                            }
                        }
                        break;
                    }
                }
                result = &mut serving => {
                    match result {
                        Ok(()) if agent.still_root() => {
                            tracing::info!("ferroan-region: LogoutReply sent")
                        }
                        Ok(()) => tracing::info!("ferroan-region: circuit closed"),
                        Err(error) => tracing::error!("ferroan-region: circuit ended: {error}"),
                    }
                    break;
                }
            }
        }
    }
    // The reply is already out. Close the neighbour child before the harness
    // stops the proxy, the same moment a root logout closes known children.
    if agent.still_root() {
        hop.close_open_child(agent.expect.agent_id, agent.expect.session_id)
            .await;
    }
    let logout_position = *landed.lock().unwrap_or_else(|poison| poison.into_inner());
    // A demoted child and a far-hop close leave the grid presence where it is.
    let tell_grid = root_arrived && agent.still_root();
    settle(
        &grid_user,
        &presence,
        &config,
        agent,
        tell_grid,
        logout_position,
    )
    .await;
}

async fn settle(
    grid_user: &GridUserClient,
    presence: &PresenceClient,
    config: &RegionConfig,
    agent: AcceptedAgent,
    root_arrived: bool,
    position: [f32; 3],
) {
    // Keep the generation lease and admission gate throughout both HTTP
    // calls. A stale generation must never mark its replacement offline.
    let guard = agent.lock_lifecycle().await;
    let result = if root_arrived {
        notify_logout(grid_user, presence, config, &agent, position).await
    } else if !config.login_connect_wait.is_zero() {
        notify_failed_login(grid_user, presence, &agent).await
    } else {
        Ok(())
    };
    drop(agent);
    drop(guard);
    if let Err(error) = result {
        tracing::error!("ferroan-region: logout notification failed: {error}");
    }
}

async fn notify_failed_login(
    grid_user: &GridUserClient,
    presence: &PresenceClient,
    agent: &AcceptedAgent,
) -> Result<(), GridError> {
    // Scene.LogOutLogin first checks that this exact Presence session remains;
    // a later login elsewhere removes it and must not be marked offline here.
    if !agent.may_logout_presence()
        || !presence
            .verify(agent.expect.agent_id, agent.expect.session_id)
            .await?
    {
        return Ok(());
    }
    if let Err(error) = presence.logout(agent.expect.session_id).await {
        tracing::error!("ferroan-region: failed-login Presence logout failed: {error}");
    }
    if agent.is_current()
        && let Some(previous) = grid_user.location(agent.expect.agent_id).await?
        && previous.online
    {
        grid_user
            .logged_out(
                agent.expect.agent_id,
                previous.region_id,
                previous.position,
                previous.look_at,
            )
            .await?;
    }
    Ok(())
}

async fn notify_root_arrival(
    grid_user: &GridUserClient,
    presence: &PresenceClient,
    config: &RegionConfig,
    agent: &AcceptedAgent,
    position: [f32; 3],
) {
    // The two OnMakeRootAgent detectors ignore their connector's bool result.
    // Failure is observable in diagnostics, but does not deny viewer arrival
    // or skip the other detector. The accepted local user is already known.
    if let Err(error) = presence
        .report_agent(agent.expect.session_id, config.id)
        .await
    {
        tracing::error!("ferroan-region: root Presence report failed: {error}");
    }
    // ScenePresence.Lookat still has its initial zero camera axis; the nonzero
    // CompleteMovement fallback belongs only to the viewer arrival packet.
    if let Err(error) = grid_user
        .set_last_position(agent.expect.agent_id, config.id, position, [0.0; 3])
        .await
    {
        tracing::error!("ferroan-region: root GridUser position report failed: {error}");
    }
}

async fn notify_logout(
    grid_user: &GridUserClient,
    presence: &PresenceClient,
    config: &RegionConfig,
    agent: &AcceptedAgent,
    position: [f32; 3],
) -> Result<(), GridError> {
    let grid_result = if agent.is_current() {
        // ActivityDetector reads ScenePresence.Lookat (zero before camera
        // updates), not CompleteMovement's nonzero arrival-packet fallback.
        // `position` is the terrain-lifted login position, or the local
        // teleport landing when the avatar moved inside the region.
        grid_user
            .logged_out(agent.expect.agent_id, config.id, position, [0.0; 3])
            .await
    } else {
        Ok(())
    };
    // Attempt both services even if the first refused or timed out. Presence
    // logout is session-specific, unlike GridUserService.LoggedOut.
    let presence_result = if agent.may_logout_presence() {
        presence.logout(agent.expect.session_id).await
    } else {
        Ok(())
    };
    grid_result.and(presence_result)
}

fn shutdown_signal() -> std::io::Result<impl std::future::Future<Output = std::io::Result<()>>> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        Ok(async move {
            tokio::select! {
                _ = interrupt.recv() => Ok(()),
                _ = terminate.recv() => Ok(()),
            }
        })
    }
    #[cfg(not(unix))]
    {
        Ok(async { tokio::signal::ctrl_c().await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_client::CircuitExpect;
    use ferroan_http::{Field, FieldMap};
    use ferroan_protocol::msgs::{
        AgentMovementComplete, CompleteAgentMovement, CompletePingCheck, DisableSimulator,
        KickUser, LogoutReply, LogoutRequest, RegionHandshake, StartPingCheck, UseCircuitCode,
    };
    use ferroan_protocol::{Msg, MsgId, build_packet, parse_packet};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    fn csharp_lifecycle() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/pin3/lifecycle.json")).unwrap()
    }

    async fn elapse(duration: Duration) {
        tokio::time::pause();
        tokio::time::advance(duration).await;
        tokio::time::resume();
    }

    struct Harness {
        state: SimulationState,
        base: String,
        udp: SocketAddr,
        requests: Arc<Mutex<Vec<FieldMap>>>,
        presence_missing: Arc<std::sync::atomic::AtomicBool>,
        fail_presence_logout: Arc<std::sync::atomic::AtomicBool>,
        sessions: Arc<Mutex<std::collections::HashSet<String>>>,
        online: Arc<std::sync::atomic::AtomicBool>,
        stop: Option<watch::Sender<bool>>,
        running: Option<tokio::task::JoinHandle<Result<(), GridError>>>,
        tasks: Vec<tokio::task::AbortHandle>,
        started: Arc<AtomicUsize>,
        users: Arc<Mutex<std::collections::HashMap<String, String>>>,
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            for task in &self.tasks {
                task.abort();
            }
        }
    }
    impl Harness {
        async fn new(short_timeout: bool, fail_grid_user: bool) -> Self {
            Self::with_arrival_control(short_timeout, fail_grid_user, false, None).await
        }

        async fn with_arrival_control(
            short_timeout: bool,
            fail_grid_user: bool,
            fail_arrival: bool,
            report_gate: Option<Arc<tokio::sync::Semaphore>>,
        ) -> Self {
            Self::with_settings(
                short_timeout,
                fail_grid_user,
                fail_arrival,
                report_gate,
                Duration::from_secs(120),
            )
            .await
        }

        async fn with_settings(
            short_timeout: bool,
            fail_grid_user: bool,
            fail_arrival: bool,
            report_gate: Option<Arc<tokio::sync::Semaphore>>,
            login_wait: Duration,
        ) -> Self {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = requests.clone();
            let presence_missing = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let missing = presence_missing.clone();
            let sessions = Arc::new(Mutex::new(std::collections::HashSet::new()));
            let service_sessions = sessions.clone();
            let online = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let service_online = online.clone();
            let fail_presence_logout = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let fail_logout = fail_presence_logout.clone();
            let users = Arc::new(Mutex::new(std::collections::HashMap::new()));
            let service_users = users.clone();
            let app = axum::Router::new().fallback(move |body: String| {
                let fields = ferroan_http::parse_query_string(&body).unwrap();
                let method = match &fields["METHOD"] { Field::Str(s) => s.clone(), _ => panic!() };
                let session = match fields.get("SessionID") { Some(Field::Str(s)) => s.clone(), _ => String::new() };
                seen.lock().unwrap().push(fields);
                let report_gate = report_gate.clone();
                let missing = missing.clone();
                let sessions = service_sessions.clone();
                let online = service_online.clone();
                let fail_logout = fail_logout.clone();
                let users = service_users.clone();
                async move {
                    if method == "report" && let Some(gate) = report_gate {
                        gate.acquire_owned().await.unwrap().forget();
                    }
                    if method == "logout" { sessions.lock().unwrap().remove(&session); }
                    if method == "loggedout" && !fail_grid_user { online.store(false, std::sync::atomic::Ordering::Relaxed); }
                    if method == "getagent" && (missing.load(std::sync::atomic::Ordering::Relaxed) || !sessions.lock().unwrap().contains(&session)) {
                        "<ServerResponse><result>null</result></ServerResponse>".to_string()
                    } else if method == "getagent" {
                        let user = users.lock().unwrap().get(&session).cloned().unwrap_or_else(|| {
                            "00000000-0000-4000-8000-000000000001".to_string()
                        });
                        format!("<ServerResponse><result type=\"List\"><UserID>{user}</UserID><RegionID>00000000-0000-0000-0000-000000000000</RegionID></result></ServerResponse>")
                    } else if method == "getgriduserinfo" {
                        "<ServerResponse><result type=\"List\"><Online>True</Online><LastRegionID>00000000-0000-4000-8000-000000000900</LastRegionID><LastPosition>&lt;1, 2, 3&gt;</LastPosition><LastLookAt>&lt;0, 1, 0&gt;</LastLookAt></result></ServerResponse>".to_string()
                    } else if (method == "logout" && fail_logout.load(std::sync::atomic::Ordering::Relaxed))
                        || (fail_grid_user && method == "loggedout")
                        || (fail_arrival && matches!(method.as_str(), "report" | "setposition"))
                    {
                        "<ServerResponse><result>Failure</result></ServerResponse>".to_string()
                    } else {
                        "<ServerResponse><result>Success</result></ServerResponse>".to_string()
                    }
                }
            });
            let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let service = format!("http://{}", mock.local_addr().unwrap());
            let mock = tokio::spawn(async move { axum::serve(mock, app).await.unwrap() });
            let config = Config::parse(&format!(
                "[Network]\nhttp_listener_port=9000\nbind_ip_address=127.0.0.1\n[GridService]\nGridServerURI={service}\n[PresenceService]\nPresenceServerURI={service}\n[GridUserService]\nGridUserServerURI={service}\n[Map]\nGenerateMaptiles=false\n[Estates]\nDefaultEstateOwnerUUID=00000000-0000-4000-8000-000000000001\n"
            )).unwrap();
            let regions = Config::parse("[Fixture Region]\nRegionUUID=00000000-0000-4000-8000-000000000002\nLocation=1000,1000\nInternalAddress=127.0.0.1\nInternalPort=9000\nExternalHostName=127.0.0.1\n").unwrap();
            let mut config = RegionConfig::from_configs(&config, &regions).unwrap();
            config.login_connect_wait = login_wait;
            config.resolve_control_plane().await.unwrap();
            if short_timeout {
                config.circuit_timing.ack_timeout = Duration::from_millis(20);
            }
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let caps = CapsRegistry::new(&base).unwrap();
            let (sender, agents) = mpsc::channel(1);
            let started = Arc::new(AtomicUsize::new(0));
            let state = SimulationState::new(&config, caps.clone(), sender.clone());
            let app = state.clone().router().merge(caps.router());
            let http = tokio::spawn(async move {
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap()
            });
            let circuit = CircuitServer::bind()
                .await
                .unwrap()
                .with_timing(config.circuit_timing)
                .unwrap();
            let (stop, stopped) = watch::channel(false);
            let udp = circuit.local_addr().unwrap();
            let hop = HopEnv::from_region(&config, caps.clone(), HopPresence::live(state.clone()));
            let running = tokio::spawn(dispatch_circuits(
                circuit,
                config,
                None,
                agents,
                stopped,
                Arc::clone(&started),
                hop,
            ));
            let tasks = vec![
                mock.abort_handle(),
                http.abort_handle(),
                running.abort_handle(),
            ];
            Self {
                state,
                base,
                udp,
                requests,
                presence_missing,
                fail_presence_logout,
                sessions,
                online,
                stop: Some(stop),
                running: Some(running),
                tasks,
                started,
                users,
            }
        }

        async fn admit(&self, replacement: bool, same_session: bool) -> CircuitExpect {
            let mut body: Value = serde_json::from_str(include_str!(
                "../../../fixtures/region-login/create-agent.request.json"
            ))
            .unwrap();
            if replacement {
                body["caps_path"] = json!(Uuid::from_u128(10).to_string());
                body["circuit_code"] = json!("2");
                if !same_session {
                    body["session_id"] = json!(Uuid::from_u128(11).to_string());
                }
            }
            let session_id = body["session_id"].as_str().unwrap().to_owned();
            let agent_id = body["agent_id"].as_str().unwrap().to_owned();
            self.sessions.lock().unwrap().insert(session_id.clone());
            self.users.lock().unwrap().insert(session_id, agent_id);
            self.online
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let started = self.started.load(Ordering::Acquire);
            let response = reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap()
                .post(format!(
                    "{}/agent/{}/",
                    self.base,
                    body["agent_id"].as_str().unwrap()
                ))
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            let value: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
            assert_eq!(value["success"], true);
            tokio::time::timeout(Duration::from_secs(2), async {
                while self.started.load(Ordering::Acquire) == started {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            CircuitExpect {
                agent_id: body["agent_id"].as_str().unwrap().parse().unwrap(),
                session_id: body["session_id"].as_str().unwrap().parse().unwrap(),
                circuit_code: body["circuit_code"].as_str().unwrap().parse().unwrap(),
                region_id: "00000000-0000-4000-8000-000000000002".parse().unwrap(),
            }
        }

        async fn admit_other(&self) -> CircuitExpect {
            let mut body: Value = serde_json::from_str(include_str!(
                "../../../fixtures/region-login/create-agent.request.json"
            ))
            .unwrap();
            body["agent_id"] = json!(Uuid::from_u128(20).to_string());
            body["session_id"] = json!(Uuid::from_u128(21).to_string());
            body["secure_session_id"] = json!(Uuid::from_u128(22).to_string());
            body["circuit_code"] = json!("7");
            body["caps_path"] = json!(Uuid::from_u128(23).to_string());
            body["first_name"] = json!("Other");
            body["last_name"] = json!("Resident");
            let session_id = body["session_id"].as_str().unwrap().to_owned();
            let agent_id = body["agent_id"].as_str().unwrap().to_owned();
            self.sessions.lock().unwrap().insert(session_id.clone());
            self.users.lock().unwrap().insert(session_id, agent_id);
            self.online
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let started = self.started.load(Ordering::Acquire);
            let response = reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap()
                .post(format!(
                    "{}/agent/{}/",
                    self.base,
                    body["agent_id"].as_str().unwrap()
                ))
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            let value: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
            assert_eq!(value["success"], true, "{value}");
            tokio::time::timeout(Duration::from_secs(2), async {
                while self.started.load(Ordering::Acquire) == started {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            CircuitExpect {
                agent_id: body["agent_id"].as_str().unwrap().parse().unwrap(),
                session_id: body["session_id"].as_str().unwrap().parse().unwrap(),
                circuit_code: body["circuit_code"].as_str().unwrap().parse().unwrap(),
                region_id: "00000000-0000-4000-8000-000000000002".parse().unwrap(),
            }
        }

        fn count(&self, method: &str) -> usize {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|fields| fields.get("METHOD") == Some(&Field::Str(method.into())))
                .count()
        }

        async fn seed_status(&self) -> reqwest::StatusCode {
            let fixture: Value = serde_json::from_str(include_str!(
                "../../../fixtures/region-login/create-agent.request.json"
            ))
            .unwrap();
            reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap()
                .post(format!(
                    "{}/CAPS/{}0000/",
                    self.base,
                    fixture["caps_path"].as_str().unwrap()
                ))
                .header("content-type", "application/llsd+xml")
                .body("<llsd><array/></llsd>")
                .send()
                .await
                .unwrap()
                .status()
        }

        async fn wait_seed_removed(&self) {
            tokio::time::timeout(Duration::from_secs(2), async {
                while self.seed_status().await != reqwest::StatusCode::NOT_FOUND {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }

        async fn matches_csharp_state(&self, expected: &Value) {
            let active = self.seed_status().await == reqwest::StatusCode::OK;
            assert_eq!(json!(active), expected["circuit"]);
            assert_eq!(
                json!(self.online.load(std::sync::atomic::Ordering::Relaxed)),
                expected["online"]
            );
            let initial_session = "00000000-0000-4000-8000-000000000004";
            let present = !self
                .presence_missing
                .load(std::sync::atomic::Ordering::Relaxed)
                && self.sessions.lock().unwrap().contains(initial_session);
            assert_eq!(json!(present), expected["presence_session"]);
        }

        async fn finish(&mut self) -> Result<(), GridError> {
            self.state.stop_accepting();
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(true);
            }
            tokio::time::timeout(Duration::from_secs(3), self.running.take().unwrap())
                .await
                .unwrap()
                .unwrap()
        }
    }

    async fn send<M: Msg>(socket: &tokio::net::UdpSocket, message: &M, sequence: u32) {
        let mut wire = Vec::new();
        build_packet(message, sequence, true, false, &[], &mut wire);
        socket.send(&wire).await.unwrap();
    }

    async fn open(harness: &Harness, expect: CircuitExpect) -> tokio::net::UdpSocket {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket.connect(harness.udp).await.unwrap();
        let mut message = UseCircuitCode::default();
        message.circuit_code.id = expect.agent_id;
        message.circuit_code.session_id = expect.session_id;
        message.circuit_code.code = expect.circuit_code;
        send(&socket, &message, 1).await;
        socket
    }

    async fn movement(socket: &tokio::net::UdpSocket, expect: CircuitExpect, sequence: u32) {
        let mut message = CompleteAgentMovement::default();
        message.agent_data.agent_id = expect.agent_id;
        message.agent_data.session_id = expect.session_id;
        message.agent_data.circuit_code = expect.circuit_code;
        send(socket, &message, sequence).await;
    }

    async fn wait_arrival(socket: &tokio::net::UdpSocket) {
        wait_packet(socket, AgentMovementComplete::ID).await;
    }

    async fn wait_packet(socket: &tokio::net::UdpSocket, id: MsgId) {
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut bytes = [0; 4096];
            loop {
                let len = socket.recv(&mut bytes).await.unwrap();
                if parse_packet(&bytes[..len]).unwrap().id == id {
                    break;
                }
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn validated_arrival_reports_once_and_service_failure_does_not_deny_arrival() {
        for fail_arrival in [false, true] {
            let mut harness = Harness::with_arrival_control(false, false, fail_arrival, None).await;
            let expect = harness.admit(false, false).await;
            assert_eq!(harness.count("report"), 0);
            assert_eq!(harness.count("setposition"), 0);
            let socket = open(&harness, expect).await;
            movement(
                &socket,
                CircuitExpect {
                    circuit_code: expect.circuit_code + 1,
                    ..expect
                },
                2,
            )
            .await;
            movement(&socket, expect, 3).await;
            wait_arrival(&socket).await;
            // A duplicate sequence and a distinct valid movement must not run
            // root detectors again; a subsequent replacement does run them.
            movement(&socket, expect, 3).await;
            movement(&socket, expect, 4).await;
            let next = harness.admit(true, false).await;
            assert_eq!(harness.count("report"), 1);
            assert_eq!(harness.count("setposition"), 1);
            let second = open(&harness, next).await;
            movement(&second, next, 2).await;
            wait_arrival(&second).await;
            assert_eq!(harness.count("report"), 2);
            assert_eq!(harness.count("setposition"), 2);
            harness.finish().await.unwrap();
        }
    }

    #[tokio::test]
    async fn two_roots_keep_their_circuits_when_one_logs_out() {
        let mut harness = Harness::new(false, false).await;
        let first = harness.admit(false, false).await;
        let second = harness.admit_other().await;
        let left = open(&harness, first).await;
        let right = open(&harness, second).await;
        wait_packet(&left, RegionHandshake::ID).await;
        wait_packet(&right, RegionHandshake::ID).await;
        let mut ping = StartPingCheck::default();
        ping.ping_id.ping_id = 11;
        send(&left, &ping, 2).await;
        ping.ping_id.ping_id = 12;
        send(&right, &ping, 2).await;
        wait_packet(&left, CompletePingCheck::ID).await;
        wait_packet(&right, CompletePingCheck::ID).await;
        let mut logout = LogoutRequest::default();
        logout.agent_data.agent_id = first.agent_id;
        logout.agent_data.session_id = first.session_id;
        send(&left, &logout, 3).await;
        wait_packet(&left, LogoutReply::ID).await;
        ping.ping_id.ping_id = 13;
        send(&right, &ping, 3).await;
        wait_packet(&right, CompletePingCheck::ID).await;
        logout.agent_data.agent_id = second.agent_id;
        logout.agent_data.session_id = second.session_id;
        send(&right, &logout, 4).await;
        wait_packet(&right, LogoutReply::ID).await;
        harness.finish().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_finishes_arrival_writes_before_logout() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let mut harness =
            Harness::with_arrival_control(false, false, false, Some(gate.clone())).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("report") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        harness.state.stop_accepting();
        harness.stop.take().unwrap().send(true).unwrap();
        assert_eq!(harness.count("loggedout"), 0);
        gate.add_permits(1);
        harness.finish().await.unwrap();
        let methods: Vec<_> = harness
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|f| f["METHOD"].clone())
            .collect();
        assert_eq!(
            methods,
            ["getagent", "report", "setposition", "loggedout", "logout"]
                .map(|s| Field::Str(s.into()))
        );
    }

    #[tokio::test]
    async fn held_arrival_backend_does_not_block_udp_ping_handling() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let mut harness =
            Harness::with_arrival_control(false, false, false, Some(gate.clone())).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("report") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut ping = StartPingCheck::default();
        ping.ping_id.ping_id = 73;
        send(&socket, &ping, 3).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut bytes = [0; 4096];
            loop {
                let len = socket.recv(&mut bytes).await.unwrap();
                let packet = parse_packet(&bytes[..len]).unwrap();
                assert_ne!(
                    packet.id,
                    AgentMovementComplete::ID,
                    "arrival must await reporting"
                );
                if packet.id == CompletePingCheck::ID {
                    assert_eq!(
                        packet
                            .decode::<CompletePingCheck>()
                            .unwrap()
                            .ping_id
                            .ping_id,
                        73
                    );
                    break;
                }
            }
        })
        .await
        .expect("UDP ping response blocked behind service report");
        assert_eq!(harness.count("setposition"), 0);
        gate.add_permits(1);
        wait_arrival(&socket).await;
        harness.finish().await.unwrap();
    }

    #[tokio::test]
    async fn logout_during_pending_arrival_finishes_writes_before_cleanup() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let mut harness =
            Harness::with_arrival_control(false, false, false, Some(gate.clone())).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("report") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut logout = LogoutRequest::default();
        logout.agent_data.agent_id = expect.agent_id;
        logout.agent_data.session_id = expect.session_id;
        send(&socket, &logout, 3).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut bytes = [0; 4096];
            loop {
                let len = socket.recv(&mut bytes).await.unwrap();
                let packet = parse_packet(&bytes[..len]).unwrap();
                assert_ne!(packet.id, AgentMovementComplete::ID);
                if packet.id == LogoutReply::ID {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(harness.count("loggedout"), 0);
        gate.add_permits(1);
        harness.finish().await.unwrap();
        let methods: Vec<_> = harness
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|f| f["METHOD"].clone())
            .collect();
        assert_eq!(
            methods,
            ["getagent", "report", "setposition", "loggedout", "logout"]
                .map(|s| Field::Str(s.into()))
        );
    }

    #[tokio::test]
    async fn replacement_waits_for_root_writes_and_old_generation_cannot_report_again() {
        for same_session in [false, true] {
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            let mut harness =
                Harness::with_arrival_control(false, false, false, Some(gate.clone())).await;
            let expect = harness.admit(false, false).await;
            let socket = open(&harness, expect).await;
            movement(&socket, expect, 2).await;
            tokio::time::timeout(Duration::from_secs(2), async {
                while harness.count("report") == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            {
                let admission = harness.admit(true, same_session);
                tokio::pin!(admission);
                assert!(
                    tokio::time::timeout(Duration::from_millis(20), &mut admission)
                        .await
                        .is_err()
                );
                assert_eq!(
                    harness.count("getagent"),
                    1,
                    "replacement verification must wait behind reporting"
                );
                gate.add_permits(1);
                admission.await;
            }
            // A late movement from the superseded circuit cannot trigger
            // another old write. Shutdown drains the accepted replacement.
            movement(&socket, expect, 3).await;
            harness.finish().await.unwrap();
            assert_eq!(harness.count("report"), 1);
            assert_eq!(harness.count("setposition"), 1);
            assert_eq!(harness.count("loggedout"), 1);
            assert_eq!(harness.count("logout"), 1 + usize::from(!same_session));
            let methods: Vec<_> = harness
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|f| f["METHOD"].clone())
                .collect();
            assert_eq!(
                &methods[..4],
                ["getagent", "report", "setposition", "getagent"].map(|s| Field::Str(s.into()))
            );
        }
    }

    #[tokio::test]
    async fn shutdown_and_inactivity_both_report_griduser_and_presence_logout() {
        for timeout in [false, true] {
            let mut harness = Harness::new(timeout, false).await;
            let expect = harness.admit(false, false).await;
            if timeout {
                let _socket = open(&harness, expect).await;
                tokio::time::timeout(Duration::from_secs(2), async {
                    while harness.count("logout") == 0 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
            harness.finish().await.unwrap();
            assert_eq!(harness.count("loggedout"), 1);
            assert_eq!(harness.count("logout"), 1);
        }
    }

    #[tokio::test]
    async fn replacement_cleanup_cannot_offline_the_new_generation_or_reused_session() {
        for same_session in [false, true] {
            let mut harness = Harness::new(false, false).await;
            harness.admit(false, false).await;
            harness.admit(true, same_session).await;
            assert_eq!(harness.count("loggedout"), 0);
            assert_eq!(harness.count("logout"), usize::from(!same_session));
            harness.finish().await.unwrap();
            assert_eq!(harness.count("loggedout"), 1);
            assert_eq!(harness.count("logout"), 1 + usize::from(!same_session));
        }
    }

    #[tokio::test]
    async fn griduser_failure_still_removes_presence_and_keeps_dispatcher_available() {
        let mut harness = Harness::new(false, true).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        wait_arrival(&socket).await;
        let mut logout = LogoutRequest::default();
        logout.agent_data.agent_id = expect.agent_id;
        logout.agent_data.session_id = expect.session_id;
        send(&socket, &logout, 3).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("logout") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // A failed handler did not stop admission or the dispatcher.
        harness.wait_seed_removed().await;
        assert_eq!(
            json!(harness.seed_status().await == reqwest::StatusCode::OK),
            csharp_lifecycle()["close_handlers"]["circuit"]
        );
        harness.admit(true, false).await;
        assert_eq!(harness.count("loggedout"), 1);
        assert_eq!(harness.count("logout"), 1);
        let result = harness.finish().await;
        assert_eq!(
            json!(result.is_err()),
            csharp_lifecycle()["close_handlers"]["close_threw"]
        );
        result.unwrap();
    }

    #[tokio::test]
    async fn pending_login_deadline_preserves_last_location_and_releases_caps() {
        let mut harness =
            Harness::with_settings(false, false, false, None, Duration::from_secs(120)).await;
        harness.admit(false, false).await;
        let fixture = csharp_lifecycle();
        harness
            .matches_csharp_state(&fixture["never_connected"]["before"])
            .await;
        elapse(Duration::from_secs(121)).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("loggedout") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        {
            let requests = harness.requests.lock().unwrap();
            let logout = requests
                .iter()
                .find(|f| f["METHOD"] == Field::Str("loggedout".into()))
                .unwrap();
            assert_eq!(
                logout["RegionID"],
                Field::Str("00000000-0000-4000-8000-000000000900".into())
            );
            assert_eq!(logout["Position"], Field::Str("<1, 2, 3>".into()));
            assert_eq!(logout["LookAt"], Field::Str("<0, 1, 0>".into()));
            let methods: Vec<_> = requests.iter().map(|f| f["METHOD"].clone()).collect();
            assert_eq!(
                methods,
                [
                    "getagent",
                    "getagent",
                    "logout",
                    "getgriduserinfo",
                    "loggedout"
                ]
                .map(|s| Field::Str(s.into()))
            );
        }
        harness.wait_seed_removed().await;
        harness
            .matches_csharp_state(&fixture["never_connected"]["after"])
            .await;
        harness.admit(true, false).await;
        harness.finish().await.unwrap();
    }

    #[tokio::test]
    async fn expired_old_session_does_not_log_out_a_new_grid_login() {
        let mut harness =
            Harness::with_settings(false, false, false, None, Duration::from_secs(120)).await;
        harness.admit(false, false).await;
        harness
            .presence_missing
            .store(true, std::sync::atomic::Ordering::Relaxed);
        elapse(Duration::from_secs(121)).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("getagent") < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        harness.finish().await.unwrap();
        harness.wait_seed_removed().await;
        harness
            .matches_csharp_state(&csharp_lifecycle()["later_session"])
            .await;
        assert_eq!(harness.count("logout"), 0);
        assert_eq!(harness.count("getgriduserinfo"), 0);
        assert_eq!(harness.count("loggedout"), 0);
    }

    #[tokio::test]
    async fn zero_wait_keeps_unopened_login_beyond_client_inactivity() {
        let mut harness = Harness::with_settings(true, false, false, None, Duration::ZERO).await;
        harness.admit(false, false).await;
        elapse(Duration::from_secs(121)).await;
        assert_eq!(harness.count("getagent"), 1);
        assert_eq!(harness.count("logout"), 0);
        assert!(!harness.running.as_ref().unwrap().is_finished());
        assert_eq!(harness.seed_status().await, reqwest::StatusCode::OK);
        harness
            .matches_csharp_state(&csharp_lifecycle()["zero_wait"])
            .await;
        harness.finish().await.unwrap();
        assert_eq!(harness.count("loggedout"), 0);
    }

    #[tokio::test]
    async fn connected_and_root_logins_survive_login_deadline_like_csharp() {
        for root in [false, true] {
            let mut harness =
                Harness::with_settings(false, false, false, None, Duration::from_secs(1)).await;
            let expect = harness.admit(false, false).await;
            let socket = open(&harness, expect).await;
            let mut bytes = [0; 4096];
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let len = socket.recv(&mut bytes).await.unwrap();
                    if parse_packet(&bytes[..len]).unwrap().id == RegionHandshake::ID {
                        break;
                    }
                }
            })
            .await
            .unwrap();
            if root {
                movement(&socket, expect, 2).await;
                wait_arrival(&socket).await;
            }
            elapse(Duration::from_secs(2)).await;
            harness
                .matches_csharp_state(
                    &csharp_lifecycle()[if root {
                        "root_arrived"
                    } else {
                        "connected_past_deadline"
                    }],
                )
                .await;
            assert_eq!(harness.count("logout"), 0);
            harness.finish().await.unwrap();
            if !root {
                harness
                    .matches_csharp_state(&csharp_lifecycle()["removed_before_root"])
                    .await;
            }
        }
    }

    #[tokio::test]
    async fn same_endpoint_circuit_open_preserves_pending_login_like_csharp() {
        let mut harness = Harness::new(false, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        let mut again = UseCircuitCode::default();
        again.circuit_code.id = expect.agent_id;
        again.circuit_code.session_id = expect.session_id;
        again.circuit_code.code = expect.circuit_code;
        send(&socket, &again, 2).await;
        let mut ping = StartPingCheck::default();
        ping.ping_id.ping_id = 74;
        send(&socket, &ping, 3).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            let mut bytes = [0; 4096];
            loop {
                let len = socket.recv(&mut bytes).await.unwrap();
                if parse_packet(&bytes[..len]).unwrap().id == CompletePingCheck::ID {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let captured = csharp_lifecycle()["pre_root_endpoint_reconnect"].clone();
        assert_eq!(captured["same_endpoint_preserves_presence"], true);
        harness
            .matches_csharp_state(&captured["same_endpoint"])
            .await;
        assert_eq!(harness.count("logout"), 0);
        movement(&socket, expect, 4).await;
        wait_arrival(&socket).await;
        assert_eq!(harness.count("report"), 1);
        harness.finish().await.unwrap();
    }

    /// TB11. A different UDP endpoint keeps the circuit, scene presence,
    /// watcher, Presence session and online flag (Pin5, #437). Rust still
    /// refuses the other endpoint; this reconnect is not ported.
    #[test]
    fn csharp_pre_root_other_endpoint_keeps_watcher_and_online() {
        let captured = csharp_lifecycle()["pre_root_endpoint_reconnect"].clone();
        let after = &captured["after"];
        let shows_fix = after["circuit"] == true
            && after["scene_presence"] == true
            && after["watched"] == true
            && after["presence_session"] == true
            && after["online"] == true;
        assert!(
            shows_fix,
            "TB11: fixtures/pin3/lifecycle.json pre_root_endpoint_reconnect.after does not keep the circuit, scene presence, watcher, Presence session and GridUser online state. The fixture is the evidence; do not claim the fix if C# still drops them. {after}"
        );
    }

    #[tokio::test]
    async fn startup_cleanup_matches_csharp_and_ignores_old_robust_failure() {
        for case in ["startup_success", "startup_old_robust_failure"] {
            let fixture = csharp_lifecycle()[case].clone();
            let captured = fixture.clone();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let seen = calls.clone();
            let app = axum::Router::new().fallback(
                move |method: axum::http::Method,
                      uri: axum::http::Uri,
                      headers: axum::http::HeaderMap,
                      body: String| {
                    assert_eq!(method.as_str(), captured["method"].as_str().unwrap());
                    assert_eq!(uri.path(), captured["path"].as_str().unwrap());
                    assert_eq!(
                        headers["content-type"],
                        captured["content_type"].as_str().unwrap()
                    );
                    assert_eq!(body, captured["request"].as_str().unwrap());
                    seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let reply = captured["response"].as_str().unwrap().to_owned();
                    async move { reply }
                },
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let region = fixture["detector_region_calls"][0]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            notify_startup(&GridUserClient::new(&base), region).await;
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::Relaxed),
                fixture["detector_region_calls"].as_array().unwrap().len()
            );
            server.abort();
        }
    }

    #[tokio::test]
    async fn presence_logoutregion_matches_csharp_and_ignores_failure() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../fixtures/pin3/login-robustness.json"))
                .unwrap();
        let region = fixture["logout_region"]["region"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        for reply in [
            fixture["logout_region"]["calls"][0]["response"]
                .as_str()
                .unwrap(),
            "<ServerResponse><result>Failure</result></ServerResponse>",
        ] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let recorded = seen.clone();
            let body_reply = reply.to_owned();
            let app = axum::Router::new().fallback(move |uri: axum::http::Uri, body: String| {
                recorded.lock().unwrap().push((uri.path().to_owned(), body));
                let body_reply = body_reply.clone();
                async move { body_reply }
            });
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            notify_region_presence(&PresenceClient::new(&base), region).await;
            server.abort();
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].0, fixture["logout_region"]["calls"][0]["path"]);
            assert_eq!(
                seen[0].1,
                fixture["logout_region"]["calls"][0]["request"]
                    .as_str()
                    .unwrap()
            );
        }
    }

    #[tokio::test]
    async fn presence_logout_failure_keeps_the_dispatcher_and_the_next_login() {
        let mut harness = Harness::new(false, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        wait_arrival(&socket).await;
        harness
            .fail_presence_logout
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut logout = LogoutRequest::default();
        logout.agent_data.agent_id = expect.agent_id;
        logout.agent_data.session_id = expect.session_id;
        send(&socket, &logout, 3).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("logout") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !harness.running.as_ref().unwrap().is_finished(),
            "a Presence logout Failure must not stop the dispatcher"
        );
        harness
            .fail_presence_logout
            .store(false, std::sync::atomic::Ordering::Relaxed);
        harness.admit(true, false).await;
        harness.finish().await.unwrap();
    }

    #[tokio::test]
    async fn failed_presence_on_a_root_replacement_still_logs_the_old_root_out() {
        let mut harness = Harness::new(false, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        wait_arrival(&socket).await;
        harness
            .presence_missing
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut body: Value = serde_json::from_str(include_str!(
            "../../../fixtures/region-login/create-agent.request.json"
        ))
        .unwrap();
        body["caps_path"] = json!(Uuid::from_u128(10).to_string());
        body["circuit_code"] = json!("2");
        body["session_id"] = json!(Uuid::from_u128(11).to_string());
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .post(format!(
                "{}/agent/{}/",
                harness.base,
                body["agent_id"].as_str().unwrap()
            ))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        let value: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_ne!(value.get("success"), Some(&json!(true)));
        assert!(harness.count("loggedout") >= 1);
        assert!(harness.count("logout") >= 1);
        harness
            .presence_missing
            .store(false, std::sync::atomic::Ordering::Relaxed);
        harness.admit(true, false).await;
        harness.finish().await.unwrap();
    }

    async fn wait_id(socket: &tokio::net::UdpSocket, id: ferroan_protocol::MsgId) {
        let mut buf = [0u8; 4096];
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let len = socket.recv(&mut buf).await.unwrap();
                if parse_packet(&buf[..len]).unwrap().id == id {
                    break;
                }
            }
        })
        .await
        .unwrap();
    }

    fn closing_packets(buf: &[u8]) -> Option<ferroan_protocol::IncomingPacket> {
        let packet = parse_packet(buf).unwrap();
        matches!(packet.id, KickUser::ID | DisableSimulator::ID).then_some(packet)
    }

    #[tokio::test]
    async fn shutdown_kicks_then_disables_and_timeout_or_replacement_only_disables() {
        let mut harness = Harness::new(false, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        wait_id(&socket, RegionHandshake::ID).await;
        harness.finish().await.unwrap();
        let mut order = Vec::new();
        let mut buf = [0u8; 4096];
        let deadline = std::time::Instant::now() + Duration::from_millis(400);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(50), socket.recv(&mut buf)).await {
                Ok(Ok(len)) => {
                    if let Some(packet) = closing_packets(&buf[..len]) {
                        if packet.id == KickUser::ID {
                            let kick = packet.decode::<KickUser>().unwrap();
                            assert_eq!(kick.user_info.reason, b"The simulator is going down.\0");
                            assert_eq!(kick.target_block.target_ip, [0, 0, 0, 0]);
                            assert_eq!(kick.target_block.target_port, 0);
                            assert!(packet.reliable());
                        }
                        if packet.id == DisableSimulator::ID {
                            assert!(packet.reliable() && packet.body.is_empty());
                        }
                        order.push(packet.id);
                    }
                }
                _ => break,
            }
        }
        assert_eq!(order, vec![KickUser::ID, DisableSimulator::ID]);

        let mut harness = Harness::new(true, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        let mut order = Vec::new();
        let mut buf = [0u8; 4096];
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let len = socket.recv(&mut buf).await.unwrap();
                if let Some(packet) = closing_packets(&buf[..len]) {
                    order.push(packet.id);
                    if packet.id == DisableSimulator::ID {
                        assert!(packet.reliable() && packet.body.is_empty());
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(order, vec![DisableSimulator::ID]);
        harness.finish().await.unwrap();

        let mut harness = Harness::new(false, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        wait_id(&socket, RegionHandshake::ID).await;
        let next = harness.admit(true, false).await;
        let mut order = Vec::new();
        let mut buf = [0u8; 4096];
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let len = socket.recv(&mut buf).await.unwrap();
                if let Some(packet) = closing_packets(&buf[..len]) {
                    order.push(packet.id);
                    if packet.id == DisableSimulator::ID {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(order, vec![DisableSimulator::ID]);
        let replacement = open(&harness, next).await;
        wait_id(&replacement, RegionHandshake::ID).await;
        harness.finish().await.unwrap();
    }
}
