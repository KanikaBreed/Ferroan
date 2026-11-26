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
// QR9 reference: Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs
// (HandleUseCircuitCode) and Source/OpenSim.Region.Framework/Scenes/{SceneBase,ScenePresence}.cs
// (AllocateLocalId and avatar creation) at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Ferroan changes: async lifecycle, generation-safe cleanup and process orchestration.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Register a single region with C# Robust and accept its local login messages.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::collections::BTreeSet;
use std::env;
use std::future::Future;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferroan_client::{
    CapsRegistry, CircuitServer, CircuitStop, LandLayer, SceneAnswer, SceneCall, SceneRefused,
    SceneReply,
};
use ferroan_config::{Config, RegionLoad};
use ferroan_region::config::{RegionConfig, RegionConfigError};
use ferroan_region::fetch_caps::FetchCaps;
use ferroan_region::grid::{GridClient, GridError, GridUserClient, PresenceClient};
use ferroan_region::simulation::{
    AcceptedAgent, DISPATCH_CAPACITY, SimulationState, release_callback,
};
use ferroan_region::source_hop::{HopEnv, HopLogin, HopPresence};
use ferroan_region::status::{
    RegionMeters, StatusState, metrics_router, region_router, with_request_counts,
};
use ferroan_scene::TerrainFrame;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
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
    #[error("parcels: {0}")]
    Parcels(#[from] ferroan_scene::parcels::ParcelError),
    #[error("region settings: {0}")]
    Settings(String),
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
    if ferroan_region::config::prioritization_scheme_differs(
        config.prioritization_scheme.as_deref(),
    ) {
        let scheme = config.prioritization_scheme.as_deref().unwrap_or("").trim();
        tracing::warn!(
            "ferroan-region: [InterestManagement] UpdatePrioritizationScheme is {scheme}; entity updates use BestAvatarResponsiveness"
        );
    }
    let terrain = load_terrain(&config).await?;
    let _region_settings = load_stored_settings(&mut config).await?;
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
        .with_throttle(config.circuit_throttle)
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
    // A full child share must leave room for queued roots as well as running ones.
    let (accepted, agents) = ferroan_region::simulation::dispatch_queue();
    let mut simulation =
        SimulationState::with_meters(&config, caps.clone(), accepted, meters.clone());
    simulation.bind_circuit(circuit.clone());
    if let Some(frame) = &terrain {
        simulation.keep_objects(frame.stored_groups());
        simulation.keep_scene(Arc::clone(frame));
    }
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
        // Allow time for in-flight detector writes, replacement cleanup and a final
        // admission. Each agent drains independently within the shutdown deadline.
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

async fn load_stored_settings(
    config: &mut RegionConfig,
) -> Result<Option<std::sync::Arc<ferroan_scene::RegionSettings>>, RunError> {
    ferroan_region::settings::apply_stored_settings(
        config.object_store.as_ref(),
        config.id,
        &mut config.handshake,
    )
    .await
    .map_err(RunError::Settings)
}

async fn load_stored_objects(
    config: &RegionConfig,
) -> Result<Arc<Vec<ferroan_scene::ObjectGroup>>, RunError> {
    let stored =
        ferroan_region::objects::load_stored_objects(config.object_store.as_ref(), config.id)
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
    // `SimulationDataService` keeps terrain in the same store as prims and
    // settings: `[SimulationDataStore]` over `[DatabaseService]`.
    let map = match config.object_store.as_ref() {
        Some(store) => {
            let width = i32::try_from(config.size[0]).unwrap_or(i32::MAX);
            let height = i32::try_from(config.size[1]).unwrap_or(i32::MAX);
            let loaded = ferroan_scene::load_world_map(
                &store.connection,
                store.kind,
                config.id,
                width,
                height,
                config.initial_terrain,
            )
            .await
            .map_err(|error| RunError::Terrain(error.to_string()))?;
            log_world_map(&loaded, config.initial_terrain);
            Some(loaded.map)
        }
        None => None,
    };
    let parcels = match &config.object_store {
        Some(store) if store.kind != ferroan_scene::ObjectStoreKind::Sqlite => None,
        store => {
            let connection = store.as_ref().map(|store| store.connection.as_str());
            let parcels = ferroan_scene::parcels::open_parcels(
                connection,
                config.id,
                config.size,
                config.estate_owner,
            )
            .await?;
            tracing::info!("ferroan-region: loaded {} parcels", parcels.parcels().len());
            Some(parcels)
        }
    };
    let objects = load_stored_objects(config).await?;
    let frame = TerrainFrame::with_object_settings(
        map,
        parcels,
        objects,
        config.prim_scale_limits,
        config.physics_scale_limits,
        config.collidable_prims,
        config.server.clone(),
    )
    .map_err(|error| RunError::Terrain(error.to_string()))?;
    Ok(Some(Arc::new(frame)))
}

/// Ferroan's lines for the cases `Scene.LoadWorldMap` logs.
fn log_world_map(loaded: &ferroan_scene::WorldMap, initial: ferroan_scene::InitialTerrain) {
    use ferroan_scene::TerrainSource;
    let name = match initial {
        ferroan_scene::InitialTerrain::Flat => "flat",
        ferroan_scene::InitialTerrain::PinheadIsland => "pinhead-island",
    };
    match loaded.source {
        TerrainSource::Stored => {}
        TerrainSource::Baked => {
            tracing::warn!("ferroan-region: terrain not found. Used stored baked terrain.");
        }
        TerrainSource::Generated => tracing::info!(
            "ferroan-region: no default terrain. Generated {name} and stored it as the terrain row."
        ),
        TerrainSource::GeneratedUnstored => tracing::warn!(
            "ferroan-region: the region store file does not exist. Running {name} without storing it."
        ),
    }
    if loaded.stored_baked {
        tracing::info!("ferroan-region: stored the heightmap as the bakedterrain row.");
    }
}

/// `MakeRootAgent` without physics: lift Z, and take the precomputed land packets.
/// Both answers are awaited, so the frame thread never blocks this worker.
async fn terrain_login(
    frame: Option<&TerrainFrame>,
    start: [f32; 3],
) -> ([f32; 3], Vec<LandLayer>) {
    let Some(frame) = frame else {
        return (start, Vec::new());
    };
    if !frame.has_map() {
        return (start, Vec::new());
    }
    let mut position = start;
    let ground = frame.ground(position[0], position[1]);
    let layers = frame.layers();
    let height = match ground {
        Ok(pending) => pending.wait().await,
        Err(error) => Err(error),
    };
    match height {
        Ok(height) => position[2] = ferroan_scene::movement_z(position[2], height),
        Err(error) => tracing::error!("ferroan-region: terrain height: {error}"),
    }
    let layers = match layers {
        Ok(pending) => pending.wait().await,
        Err(error) => Err(error),
    };
    let land = match layers {
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

/// Overlay chunks for an arrival at `position`. A root with no parcel under
/// them gets none: `sendClientInitialLandInfo` returns before the overlay,
/// after terrain was already queued. A child skips that lookup and still
/// receives the overlay. Group cells use key presence only.
fn overlay_chunks(
    parcels: Option<&ferroan_scene::parcels::ParcelMap>,
    agent: Uuid,
    position: [f32; 3],
    child: bool,
    members: &BTreeSet<Uuid>,
) -> Vec<ferroan_client::ParcelOverlayChunk> {
    let Some(map) = parcels else {
        return Vec::new();
    };
    map.arrival_overlay(agent, position, child, |group| members.contains(&group))
        .into_iter()
        .map(|packet| ferroan_client::ParcelOverlayChunk {
            sequence_id: packet.sequence_id,
            data: packet.data.to_vec(),
        })
        .collect()
}

async fn notify_region_presence(presence: &PresenceClient, region: uuid::Uuid) {
    if let Err(error) = presence.logout_region(region).await {
        tracing::error!("ferroan-region: Presence logoutregion failed: {error}");
    }
}

#[derive(Clone)]
struct LifecycleClients {
    grid_user: GridUserClient,
    presence: PresenceClient,
}

#[derive(Clone)]
struct DispatchCtx {
    circuit: CircuitServer,
    config: RegionConfig,
    terrain: Option<Arc<TerrainFrame>>,
    clients: LifecycleClients,
    stop: watch::Receiver<bool>,
    /// Tests wait until a dequeued agent has passed same-agent cleanup.
    started: Arc<AtomicUsize>,
    hop: HopEnv,
}

fn motion_of(update: ferroan_client::AgentMotionUpdate) -> ferroan_scene::MotionUpdate {
    ferroan_scene::MotionUpdate {
        now_ms: ferroan_scene::clock_ms(update.received),
        sequence: update.sequence,
        child: update.child,
        in_transit: update.in_transit,
        body_rotation: update.body_rotation,
        head_rotation: update.head_rotation,
        state: update.state,
        camera_center: update.camera_center,
        camera_at: update.camera_at,
        camera_left: update.camera_left,
        camera_up: update.camera_up,
        far: update.far,
        control_flags: update.control_flags,
        flags: update.flags,
    }
}

fn pose_reply(stored: ferroan_scene::Presence) -> SceneReply {
    SceneReply::Pose(ferroan_client::LoginPose {
        position: stored.position,
        rotation: stored.rotation,
        velocity: stored.velocity,
        control_flags: stored.control_flags,
        draw_distance: stored.draw_distance,
    })
}

/// A post the frame answers. The circuit awaits the answer on its own task.
fn answered<T: Send + 'static>(
    posted: Result<ferroan_scene::Pending<T>, ferroan_scene::TerrainError>,
    reply: impl FnOnce(T) -> Result<SceneReply, SceneRefused> + Send + 'static,
) -> SceneAnswer {
    match posted {
        Ok(pending) => SceneAnswer::wait(async move {
            match pending.wait().await {
                Ok(value) => reply(value),
                Err(_) => Err(SceneRefused),
            }
        }),
        Err(_) => SceneAnswer::Ready(Err(SceneRefused)),
    }
}

/// A post with no answer. A full queue or a stopped frame is refused.
fn queued(posted: Result<(), ferroan_scene::TerrainError>) -> SceneAnswer {
    SceneAnswer::Ready(
        posted
            .map(|()| SceneReply::Applied)
            .map_err(|_| SceneRefused),
    )
}

/// One circuit call onto the frame. Nothing here waits for the frame thread.
fn scene_call(frame: &TerrainFrame, agent: Uuid, call: SceneCall) -> SceneAnswer {
    match call {
        SceneCall::Pose(pose) => answered(
            frame.post_login(
                agent,
                ferroan_scene::Presence {
                    position: pose.position,
                    rotation: pose.rotation,
                    velocity: pose.velocity,
                    control_flags: pose.control_flags,
                    draw_distance: pose.draw_distance,
                },
            ),
            |stored| Ok(pose_reply(stored)),
        ),
        SceneCall::Teleport(request) => answered(
            frame.teleport_local(
                agent,
                ferroan_scene::LocalTeleport {
                    position: request.position,
                    look_at: request.look_at,
                    half_height: request.half_height,
                    default_landing: request.default_landing,
                    size_x: request.size_x,
                    size_y: request.size_y,
                    flags: request.flags,
                },
            ),
            |stored| {
                Ok(SceneReply::Moved(ferroan_client::SceneMove {
                    position: stored.pose.position,
                    look_at: stored.look_at,
                    flags: stored.flags,
                }))
            },
        ),
        SceneCall::Complete(pose) => answered(
            frame.complete_hop(
                agent,
                ferroan_scene::HopReturn {
                    position: pose.position,
                    look_at: pose.look_at,
                    data: ferroan_scene::ReturnData {
                        velocity: pose.velocity,
                        far: pose.far,
                        body_rotation: pose.body_rotation,
                        head_rotation: pose.head_rotation,
                        control_flags: pose.control_flags,
                        always_run: pose.always_run,
                    },
                },
            ),
            |stored| Ok(pose_reply(stored)),
        ),
        SceneCall::Arrive(pose, look_at) => answered(
            frame.arrive(
                agent,
                ferroan_scene::Presence {
                    position: pose.position,
                    rotation: pose.rotation,
                    velocity: pose.velocity,
                    control_flags: pose.control_flags,
                    draw_distance: pose.draw_distance,
                },
                look_at,
            ),
            |stored| Ok(pose_reply(stored)),
        ),
        SceneCall::Read => answered(frame.presence(agent), |stored| {
            Ok(stored.map_or(SceneReply::Missing, pose_reply))
        }),
        SceneCall::Departure => answered(frame.departure(agent), |departure| {
            let departure = departure.map_or(
                ferroan_client::SceneDeparture {
                    flying: false,
                    pose: ferroan_client::HopPose::default(),
                },
                |departure| ferroan_client::SceneDeparture {
                    flying: departure.flying,
                    pose: ferroan_client::HopPose {
                        always_run: departure.always_run,
                        center: departure.camera_center,
                        head_rotation: departure.head_rotation,
                        body_rotation: departure.body_rotation,
                    },
                },
            );
            Ok(SceneReply::Departure(departure))
        }),
        SceneCall::Places(ids) => answered(frame.chat_positions(&ids), |places| {
            Ok(SceneReply::Places(places))
        }),
        SceneCall::AgentUpdate(update) => queued(frame.agent_update(agent, motion_of(update))),
        SceneCall::AlwaysRun(always_run) => queued(frame.set_always_run(agent, always_run)),
        SceneCall::HandshakeReply => queued(frame.handshake_reply(agent)),
    }
}

async fn dispatch_circuits(
    circuit: CircuitServer,
    config: RegionConfig,
    terrain: Option<Arc<TerrainFrame>>,
    agents: mpsc::Receiver<AcceptedAgent>,
    stop: watch::Receiver<bool>,
    started: Arc<AtomicUsize>,
    hop: HopEnv,
) -> Result<(), GridError> {
    circuit.enable_chat();
    // UserManagementModule: names from the account service, then GridUser.
    circuit.enable_names(ferroan_region::names::GridNames::new(
        &config.user_account_server_uri,
        config.scope_id,
        &config.grid_user_server_uri,
        config.grid_home.clone(),
    ));
    if let Some(frame) = terrain.as_ref() {
        let frame = Arc::clone(frame);
        circuit.set_login_scene(Arc::new(move |agent, call| scene_call(&frame, agent, call)));
    }
    let ctx = DispatchCtx {
        circuit,
        clients: LifecycleClients {
            grid_user: GridUserClient::new(&config.grid_user_server_uri),
            presence: PresenceClient::new(&config.presence_server_uri),
        },
        config,
        terrain,
        stop: stop.clone(),
        started,
        hop,
    };
    dispatch(agents, stop, DISPATCH_CAPACITY, |mut agent, stopping| {
        let ctx = ctx.clone();
        async move {
            agent.wait_for_predecessor().await;
            ctx.started.fetch_add(1, Ordering::Release);
            if stopping
                || *ctx.stop.borrow()
                || ctx.stop.has_changed().is_err()
                || !agent.is_current()
            {
                let position = agent.start_pos;
                settle(
                    &ctx.clients.grid_user,
                    &ctx.clients.presence,
                    &ctx.config,
                    agent,
                    false,
                    position,
                    [0.0; 3],
                )
                .await;
            } else {
                serve_agent(ctx, agent).await;
            }
        }
    })
    .await;
    Ok(())
}

/// Reap completions even when admission is idle. Dropping the set aborts its tasks.
/// Closing admission drains every reserved generation before shutdown returns.
async fn dispatch<T, F, R>(
    mut agents: mpsc::Receiver<T>,
    mut stop: watch::Receiver<bool>,
    capacity: usize,
    serve: impl Fn(T, bool) -> F,
) where
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let mut running = JoinSet::new();
    let mut stopping = *stop.borrow();
    if stopping {
        agents.close();
    }
    loop {
        tokio::select! {
            biased;
            changed = stop.changed(), if !stopping => {
                if changed.is_err() || *stop.borrow() {
                    stopping = true;
                    agents.close();
                }
            }
            Some(result) = running.join_next(), if !running.is_empty() => {
                if let Err(error) = result { tracing::error!("circuit task ended: {error}"); }
            }
            next = agents.recv(), if running.len() < capacity => {
                let Some(agent) = next else { break };
                running.spawn(serve(agent, stopping));
            }
        }
    }
    while let Some(result) = running.join_next().await {
        if let Err(error) = result {
            tracing::error!("circuit task ended: {error}");
        }
    }
}

async fn serve_agent(ctx: DispatchCtx, mut agent: AcceptedAgent) {
    let DispatchCtx {
        circuit,
        config,
        terrain,
        mut stop,
        clients,
        hop,
        ..
    } = ctx;
    let LifecycleClients {
        grid_user,
        presence,
    } = clients;
    // The guard goes first so even an early return or a panic removes the presence.
    let forget = ForgetPresence::new(terrain.clone(), agent.expect.agent_id);
    let mut root_arrived = false;
    let already_stopped = *stop.borrow_and_update();
    let (position, land, members) = if already_stopped {
        (agent.start_pos, Vec::new(), BTreeSet::new())
    } else {
        // RefreshGroupMembership runs in the ScenePresence constructor, before
        // the viewer connects. A transport error leaves the map empty: the
        // overlay still goes out, and every group cell reads as non-member.
        let frame = terrain.as_deref();
        let ((position, land), memberships) = tokio::join!(
            terrain_login(frame, agent.start_pos),
            config.groups.memberships(agent.expect.agent_id),
        );
        let members = match memberships {
            Ok(rows) => rows.into_keys().collect(),
            Err(error) => {
                tracing::error!("ferroan-region: group membership lookup failed: {error}");
                BTreeSet::new()
            }
        };
        (position, land, members)
    };
    // Logout reports this. It starts as the login position and follows a local landing.
    let landed = Arc::new(Mutex::new(position));
    // `LLClientView.IsActive`: the frame sends minimap dots only once the
    // viewer's `UseCircuitCode` has opened the circuit.
    let circuit_open = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if !already_stopped {
        let mut avatar = ferroan_region::avatar::load(
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
        arrival.teleport_flags = agent.teleport_flags;
        arrival.crossing_flags = 0;
        arrival.overlay = overlay_chunks(
            terrain.as_deref().and_then(TerrainFrame::parcels),
            expect.agent_id,
            position,
            agent.return_slot.is_child(),
            &members,
        );
        // The owner and the frame use the same region id space, even if attach fails.
        let attached = if let Some(frame) = terrain.as_deref() {
            let feed = frame.new_feed();
            let link = ferroan_scene::ViewerLink {
                feed: Arc::clone(&feed),
                seeds_sent: Arc::clone(&agent.seeds_sent),
                circuit_open: Arc::clone(&circuit_open),
                teleport_flags: agent.teleport_flags,
                look: Arc::new(ferroan_scene::AvatarLook {
                    first_name: avatar.first_name.clone(),
                    last_name: avatar.last_name.clone(),
                    title: avatar.title.clone(),
                    visual_params: avatar.visual_params.clone(),
                    texture: avatar.texture.clone(),
                    height: avatar.height,
                    hover: avatar.hover,
                }),
            };
            let attached = match frame.attach(expect.agent_id, link) {
                Ok(pending) => pending.wait().await.ok(),
                Err(_) => None,
            };
            if attached.is_some() {
                arrival.updates = Some(ferroan_client::EntityFeed {
                    wake: feed.wake(),
                    updates: Arc::new(SceneFeed {
                        viewer: expect.agent_id,
                        object_animations: Arc::clone(&agent.object_animations),
                        supports_pbr: Arc::clone(&agent.supports_pbr),
                        feed,
                        region_handle: arrival.region_handle,
                    }),
                });
            }
            attached
        } else {
            None
        };
        avatar.local_id = attached.unwrap_or_else(|| {
            // The production region always has a frame. Isolated circuit tests can omit it.
            terrain
                .as_deref()
                .map_or(1, TerrainFrame::allocate_local_id)
        });
        arrival.avatar = Some(avatar);
        // TeleportAgentWithinRegion. Godlike needs a cached user level, so it
        // stays off. The frame owns the landing and IsFlying. A handle this
        // grid does not know gets the not-found packet and one offline map
        // block. An in-view hop demotes this root and keeps the caps and the
        // circuit. SendLocalTeleport's queue is the circuit's.
        arrival.teleport = Some(ferroan_client::TeleportRegion {
            world_x: config.location[0].wrapping_mul(256),
            world_y: config.location[1].wrapping_mul(256),
            size_x: config.size[0] as f32,
            size_y: config.size[1] as f32,
            default_landing: [128.0, 128.0, 30.0],
            half_height,
            allow_movement: true,
            godlike: false,
            position: Arc::clone(&landed),
        });
        arrival.return_slot = Some(agent.return_slot.clone());
        let mut promotions = agent.return_slot.take_promotions();
        // A different session already staged the previous root's children.
        // The closes go out on a separate task with the old session id.
        // This login does not wait on them.
        hop.close_staged(expect.agent_id).await;
        arrival.source_hop = Some(hop.install(HopLogin {
            agent_id: expect.agent_id,
            session_id: expect.session_id,
            secure_session_id: agent.secure_session_id,
            circuit_code: expect.circuit_code,
            first_name: agent.first_name.clone(),
            last_name: agent.last_name.clone(),
            viewer: agent.viewer.clone(),
            channel: agent.channel.clone(),
            mac: agent.mac.clone(),
            id0: agent.id0.clone(),
            service_urls: agent.service_urls.clone(),
            animations: Arc::clone(&agent.animations),
            appearance: agent.appearance.clone(),
        }));
        let (report, mut report_ready) = oneshot::channel();
        let (opened, mut opening) = oneshot::channel();
        let (close_tx, close_rx) = oneshot::channel();
        // Scene.CacheUserName. CreateAgent's service URLs are not kept, so
        // every arrival is a resident of this grid, as an empty HomeURI is.
        circuit.cache_arrival_name(expect.agent_id, &agent.first_name, &agent.last_name);
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
                kind = agent.replaced() => {
                    match kind {
                        CircuitStop::Quiet => {
                            tracing::info!("ferroan-region: CloseAgent closed the circuit")
                        }
                        CircuitStop::Disable | CircuitStop::Shutdown => {
                            tracing::info!("ferroan-region: circuit replaced by a new login")
                        }
                    }
                    if let Some(tx) = close_tx.take() {
                        let _ = tx.send(kind);
                    }
                    match (&mut serving).await {
                        Ok(()) => tracing::info!("ferroan-region: LogoutReply sent"),
                        Err(error) => tracing::error!("ferroan-region: circuit ended: {error}"),
                    }
                    break;
                }
                _ = &mut opening, if !connected => {
                    connected = true;
                    circuit_open.store(true, std::sync::atomic::Ordering::Release);
                }
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
                    let position = token.position;
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
    // The serve is over: the presence leaves the frame, so nothing reads a
    // ghost and a later login of this agent posts a fresh pose and filter.
    // A same-agent replacement waits for this generation to finish cleanup
    // before it posts to the frame.
    let leaving = forget.finish().await;
    // The reply is already out. Close the neighbour child before the harness
    // stops the proxy, the same moment a root logout closes known children.
    if agent.still_root() {
        hop.close_open_child(agent.expect.agent_id, agent.expect.session_id)
            .await;
    }
    // The circuit that was replaced drains the same list when it ends first.
    // The sends do not hold this cleanup. Abort and await this circuit's
    // admission tasks before this take: a cancelled admission stages its
    // close when the guard drops, and an earlier take leaves it in the book.
    hop.close_staged(agent.expect.agent_id).await;
    // ActivityDetector reads AbsolutePosition and Lookat when the connection
    // closes: the walked position and the flattened camera at axis. Without
    // a frame record, the login or landing position and a zero look.
    let (logout_position, logout_look) = match leaving {
        Some(leaving) => (leaving.position, leaving.look_at),
        None => (
            *landed.lock().unwrap_or_else(|poison| poison.into_inner()),
            [0.0; 3],
        ),
    };
    // A far hop, a demoted child and a child that never rooted leave Presence
    // where it is. Failed login is a ViaLogin session that never rooted. A
    // root that was still root when another Presence session replaced it
    // still logs that session out. A demoted or departed generation does not.
    settle(
        &grid_user,
        &presence,
        &config,
        agent,
        root_arrived,
        logout_position,
        logout_look,
    )
    .await;
}

/// One viewer's frame feed, as the circuit drains it.
struct SceneFeed {
    object_animations: Arc<std::sync::atomic::AtomicBool>,
    supports_pbr: Arc<std::sync::atomic::AtomicBool>,
    viewer: Uuid,
    feed: Arc<ferroan_scene::terse::ViewerFeed>,
    region_handle: u64,
}

impl ferroan_client::EntityUpdates for SceneFeed {
    fn has_updates(&self) -> bool {
        self.feed.has_updates()
    }

    fn take_updates(&self, batch: &mut ferroan_client::TaskBatch) {
        use ferroan_scene::terse::ViewerUpdate;
        let supports_animations = self
            .object_animations
            .load(std::sync::atomic::Ordering::Acquire);
        while batch.has_room() {
            match self.feed.take_update_for_viewer(supports_animations) {
                Some(ViewerUpdate::Terse(block)) => batch.push_terse(block),
                Some(ViewerUpdate::Full {
                    update: Ok(update),
                    animations,
                    part,
                    queued,
                }) => {
                    batch.push_full(&update.block_for(self.viewer), animations.map(<[_]>::len));
                    if self.supports_pbr.load(std::sync::atomic::Ordering::Acquire)
                        && queued.flags & ferroan_scene::MATERIAL_OVR != 0
                    {
                        push_part_material(batch, part);
                    }
                }
                // Oversize fields were reported at admission. Continue with the next part.
                Some(ViewerUpdate::Full { update: Err(_), .. }) => continue,
                None => break,
            }
        }
    }

    fn take_land_pass(&self) -> bool {
        self.feed.take_land_pass()
    }

    fn take_avatar_packets(&self, max: usize) -> Vec<ferroan_client::AvatarPacket> {
        let mut out = Vec::new();
        for packet in self.feed.take_avatar_packets(max) {
            match packet {
                ferroan_scene::AvatarPacket::Full(full) => {
                    let wire = avatar_wire(full.agent, full.local_id, &full.look);
                    let live = ferroan_client::LiveAvatar {
                        collision_plane: full.collision_plane,
                        position: full.position,
                        velocity: full.velocity,
                        rotation: full.rotation,
                    };
                    out.push(ferroan_client::AvatarPacket::Full(
                        wire.live_update(self.region_handle, &live),
                    ));
                }
                ferroan_scene::AvatarPacket::Appearance { agent, look } => {
                    out.push(ferroan_client::AvatarPacket::Appearance(
                        avatar_wire(agent, 0, &look).appearance(),
                    ));
                }
                ferroan_scene::AvatarPacket::Kill(ids) => {
                    out.extend(
                        ferroan_client::kill_packets(&ids)
                            .into_iter()
                            .map(ferroan_client::AvatarPacket::Kill),
                    );
                }
                ferroan_scene::AvatarPacket::Coarse(coarse) => {
                    out.push(ferroan_client::AvatarPacket::Coarse(
                        ferroan_client::coarse_update(
                            &coarse.locations,
                            coarse.you,
                            coarse.prey,
                            &coarse.agents,
                        ),
                    ));
                }
            }
        }
        out
    }
}

/// `needMaterials` after the full block. A dropped full datagram was already dequeued.
fn push_part_material(batch: &mut ferroan_client::TaskBatch, part: &ferroan_scene::ObjectPart) {
    let Some(materials) = part.shape.materials.as_ref() else {
        return;
    };
    let faces: Vec<ferroan_client::Face<'_>> = materials
        .overrides
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|entry| ferroan_client::Face {
            te_index: entry.te_index,
            data: entry.data.as_str(),
        })
        .collect();
    if let Some(body) =
        ferroan_client::material_packet(part.local_id, part.shape.pcode, Some(&faces))
    {
        batch.push_material(body);
    }
}

/// The wire form of another avatar, from what its own viewer attached with.
fn avatar_wire(
    agent: Uuid,
    local_id: u32,
    look: &ferroan_scene::AvatarLook,
) -> ferroan_client::AvatarWire {
    let mut wire =
        ferroan_client::AvatarWire::standing(agent, local_id, &look.first_name, &look.last_name);
    wire.title.clone_from(&look.title);
    wire.visual_params.clone_from(&look.visual_params);
    wire.texture.clone_from(&look.texture);
    wire.height = look.height;
    wire.hover = look.hover;
    wire
}

/// Removes one agent's scene presence when its serve ends. `finish` awaits
/// the removal. A serve that panics or is aborted drops this instead, which
/// still queues the removal before a replacement can post.
struct ForgetPresence {
    frame: Option<Arc<TerrainFrame>>,
    agent: Uuid,
}

impl ForgetPresence {
    fn new(frame: Option<Arc<TerrainFrame>>, agent: Uuid) -> Self {
        Self { frame, agent }
    }

    /// The presence's last position and look, when the frame had one.
    async fn finish(mut self) -> Option<ferroan_scene::Leaving> {
        let frame = self.frame.take()?;
        let removed = match frame.remove(self.agent) {
            Ok(pending) => pending.wait().await,
            Err(error) => Err(error),
        };
        match removed {
            Ok(leaving) => leaving,
            Err(error) => {
                tracing::debug!("ferroan-region: scene presence was not removed: {error}");
                None
            }
        }
    }
}

impl Drop for ForgetPresence {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            // The frame applies this call before any later one; nothing waits for it.
            let _ = frame.remove(self.agent);
        }
    }
}

async fn settle(
    grid_user: &GridUserClient,
    presence: &PresenceClient,
    config: &RegionConfig,
    agent: AcceptedAgent,
    became_root: bool,
    position: [f32; 3],
    look_at: [f32; 3],
) {
    // Keep the generation lease and admission gate throughout both HTTP
    // calls. A stale generation must never mark its replacement offline.
    let guard = agent.lock_lifecycle().await;
    let failed_login = !became_root && agent.via_login && !config.login_connect_wait.is_zero();
    // The replacement is the current generation, so this does not mark it offline.
    // `became_root` stays set after a demotion. The session flag is the root
    // bit at the moment this generation was removed.
    let replaced = became_root && agent.replaced_by_other_session();
    let result = if became_root && agent.still_root() {
        notify_logout(grid_user, presence, config, &agent, position, look_at).await
    } else if failed_login || replaced {
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
    look_at: [f32; 3],
) -> Result<(), GridError> {
    let grid_result = if agent.is_current() {
        // ActivityDetector reads AbsolutePosition and ScenePresence.Lookat
        // (zero before camera updates), not CompleteMovement's nonzero
        // arrival-packet fallback.
        grid_user
            .logged_out(agent.expect.agent_id, config.id, position, look_at)
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

    #[tokio::test]
    async fn mysql_storage_provider_startup_does_not_open_sqlite() {
        let server = ferroan_config::Config::parse(
            "[Network]\nhttp_listener_port=9100\nbind_ip_address=127.0.0.2\n\
             [GridService]\nGridServerURI=http://127.0.0.1:8003\n\
             [PresenceService]\nPresenceServerURI=http://127.0.0.1:8003\n\
             [GridUserService]\nGridUserServerURI=http://127.0.0.1:8003\n\
             [Map]\nGenerateMaptiles=false\n\
             [DatabaseService]\nStorageProvider=OpenSim.Data.MySQL.dll\n\
             ConnectionString=\"Data Source=127.0.0.1;Port=1;Database=ferroan;User ID=ferroan;Password=unused\"\n",
        )
        .unwrap();
        let regions = ferroan_config::Config::parse(
            "[Ferroan Test]\nRegionUUID=6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01\n\
             Location=1000,1001\nInternalAddress=127.0.0.1\nInternalPort=9100\n\
             ExternalHostName=127.0.0.1\n",
        )
        .unwrap();
        let mut config = RegionConfig::from_configs(&server, &regions).unwrap();
        assert_eq!(
            config.object_store.as_ref().map(|store| store.kind),
            Some(ferroan_scene::ObjectStoreKind::Mysql)
        );
        let settings = load_stored_settings(&mut config).await.unwrap();
        assert!(settings.is_none());
        // Terrain is read from the MySQL store, which is not running: startup
        // stops on the MySQL connect, not on a SQLite file named by the string.
        match load_terrain(&config).await {
            Err(RunError::Terrain(text)) => {
                assert!(
                    text.starts_with("terrain store: could not connect to mysql"),
                    "{text}"
                );
            }
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("an unreachable MySQL store loaded terrain"),
        }

        let bare = ferroan_config::Config::parse(
            "[Network]\nhttp_listener_port=9100\nbind_ip_address=127.0.0.2\n\
             [GridService]\nGridServerURI=http://127.0.0.1:8003\n\
             [PresenceService]\nPresenceServerURI=http://127.0.0.1:8003\n\
             [GridUserService]\nGridUserServerURI=http://127.0.0.1:8003\n\
             [Map]\nGenerateMaptiles=false\n\
             [DatabaseService]\n\
             ConnectionString=Data Source=127.0.0.1;Database=ferroan;User ID=ferroan;Password=unused\n",
        )
        .unwrap();
        let bare_config = RegionConfig::from_configs(&bare, &regions).unwrap();
        assert!(bare_config.object_store.is_none());
        let bare_terrain = load_terrain(&bare_config).await.unwrap().unwrap();
        assert!(!bare_terrain.has_map());
        assert_eq!(bare_terrain.parcels().unwrap().parcels().len(), 1);
    }
    #[tokio::test]
    async fn terrain_is_stored_in_the_simulation_data_store() {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-region-terrain-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("RegionStore.db");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/prim-updates/scene.db"),
            &path,
        )
        .unwrap();
        let seed = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(&path),
        )
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../../../fixtures/regionstore/parcels.sql"))
            .execute(&seed)
            .await
            .unwrap();
        // Put the parcel fixture's home rows in the prim fixture's region.
        sqlx::query("UPDATE land SET RegionUUID = ? WHERE RegionUUID = ?")
            .bind("6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01")
            .bind("11111111-1111-1111-1111-111111111111")
            .execute(&seed)
            .await
            .unwrap();
        seed.close().await;
        // `[DatabaseService]` names MySQL; `[SimulationDataStore]` replaces it.
        let server = ferroan_config::Config::parse(&format!(
            "[Network]\nhttp_listener_port=9100\nbind_ip_address=127.0.0.2\n\
             [GridService]\nGridServerURI=http://127.0.0.1:8003\n\
             [PresenceService]\nPresenceServerURI=http://127.0.0.1:8003\n\
             [GridUserService]\nGridUserServerURI=http://127.0.0.1:8003\n\
             [Map]\nGenerateMaptiles=false\n\
             [DatabaseService]\nStorageProvider=OpenSim.Data.MySQL.dll\n\
             ConnectionString=Data Source=127.0.0.1;Port=1;Database=ferroan;User ID=ferroan;Password=unused\n\
             [SimulationDataStore]\nStorageProvider=OpenSim.Data.SQLite.dll\n\
             ConnectionString=URI=file:{};UseUTF16Encoding=True\n",
            path.display()
        ))
        .unwrap();
        let mut regions = ferroan_config::Config::parse(
            "[Ferroan Test]\nRegionUUID=6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01\n\
             Location=1000,1001\nInternalAddress=127.0.0.1\nInternalPort=9100\n\
             ExternalHostName=127.0.0.1\n",
        )
        .unwrap();
        regions.set("Ferroan Test", "ClampPrimSize", "true");
        regions.set("Ferroan Test", "NonPhysicalPrimMin", "2");
        let config = RegionConfig::from_configs(&server, &regions).unwrap();
        let terrain = load_terrain(&config).await.unwrap().unwrap();
        assert!(terrain.has_map());
        let parcels = terrain.parcels().unwrap();
        assert_eq!(parcels.parcels().len(), 3);
        assert_eq!(parcels.at_cell(1, 0).unwrap().name, "West");
        assert_eq!(parcels.at_cell(2, 0).unwrap().name, "East");
        let groups = terrain.stored_groups();
        let linked = groups.iter().find(|group| group.parts.len() == 2).unwrap();
        let child = &linked.parts[1]; // Persisted scale (0.5, 0.5, 0.5).
        assert_ne!(linked.parts[0].local_id, 0);
        assert_eq!(child.parent_id, linked.parts[0].local_id);
        assert_eq!(child.shape.scale, [2.0; 3]);
        let stored = ferroan_scene::open_terrain(
            &format!("URI=file:{};UseUTF16Encoding=True", path.display()),
            config.id,
            256,
            256,
            ferroan_scene::ObjectStoreKind::Sqlite,
        )
        .await
        .unwrap();
        assert!(stored.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    use ferroan_client::CircuitExpect;
    use ferroan_client::EntityUpdates;
    use ferroan_http::{Field, FieldMap};
    use ferroan_protocol::msgs::{
        AgentMovementComplete, AgentUpdate, CompleteAgentMovement, CompletePingCheck,
        DisableSimulator, KickUser, LogoutReply, LogoutRequest, RegionHandshake, SetAlwaysRun,
        StartPingCheck, UseCircuitCode,
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
        hold_next_logout: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
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
            Self::with_scene(
                short_timeout,
                fail_grid_user,
                fail_arrival,
                report_gate,
                login_wait,
                None,
            )
            .await
        }

        async fn with_scene(
            short_timeout: bool,
            fail_grid_user: bool,
            fail_arrival: bool,
            report_gate: Option<Arc<tokio::sync::Semaphore>>,
            login_wait: Duration,
            scene: Option<Arc<TerrainFrame>>,
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
            let hold_next_logout = Arc::new(Mutex::new(None::<oneshot::Receiver<()>>));
            let logout_gate = Arc::clone(&hold_next_logout);
            let users = Arc::new(Mutex::new(std::collections::HashMap::new()));
            let service_users = users.clone();
            let app = axum::Router::new().fallback(move |body: String| {
                let fields = ferroan_http::parse_query_string(&body).unwrap();
                let method = match &fields["METHOD"] { Field::Str(s) => s.clone(), _ => panic!() };
                let session = match fields.get("SessionID") { Some(Field::Str(s)) => s.clone(), _ => String::new() };
                seen.lock().unwrap().push(fields);
                let report_gate = report_gate.clone();
                let hold = if method == "logout" { logout_gate.lock().unwrap().take() } else { None };
                let missing = missing.clone();
                let sessions = service_sessions.clone();
                let online = service_online.clone();
                let fail_logout = fail_logout.clone();
                let users = service_users.clone();
                async move {
                    if let Some(release) = hold { let _ = release.await; }
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
            let circuit = CircuitServer::bind()
                .await
                .unwrap()
                .with_timing(config.circuit_timing)
                .unwrap()
                .with_throttle(config.circuit_throttle);
            state.bind_circuit(circuit.clone());
            let app = state.clone().router().merge(caps.router());
            let http = tokio::spawn(async move {
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap()
            });
            let (stop, stopped) = watch::channel(false);
            let udp = circuit.local_addr().unwrap();
            let hop = HopEnv::from_region(&config, caps.clone(), HopPresence::live(state.clone()));
            let running = tokio::spawn(dispatch_circuits(
                circuit,
                config,
                scene,
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
                hold_next_logout,
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
            let started = self.started.load(Ordering::Acquire);
            let expect = self.submit_login(replacement, same_session).await;
            tokio::time::timeout(Duration::from_secs(2), async {
                while self.started.load(Ordering::Acquire) == started {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            expect
        }

        async fn submit_login(&self, replacement: bool, same_session: bool) -> CircuitExpect {
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

        async fn admit_child(&self) -> CircuitExpect {
            let mut body: Value = serde_json::from_str(include_str!(
                "../../../fixtures/region-login/create-agent.request.json"
            ))
            .unwrap();
            body["child"] = json!(true);
            body["teleport_flags"] = json!("16");
            body["caps_path"] = json!(Uuid::from_u128(30).to_string());
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
            assert!(response.status().is_success(), "{}", response.status());
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
    async fn a_replacement_waiting_for_logout_allows_another_root_to_arrive() {
        let mut harness = Harness::new(false, false).await;
        let first = harness.admit(false, false).await;
        let socket = open(&harness, first).await;
        movement(&socket, first, 2).await;
        wait_arrival(&socket).await;

        let (release, hold) = oneshot::channel();
        *harness.hold_next_logout.lock().unwrap() = Some(hold);
        let next = harness.submit_login(true, false).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("logout") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(harness.started.load(Ordering::Acquire), 1);
        let other = harness.admit_other().await;
        let other_socket = open(&harness, other).await;
        movement(&other_socket, other, 2).await;
        wait_arrival(&other_socket).await;
        assert_eq!(harness.started.load(Ordering::Acquire), 2);

        release.send(()).unwrap();
        let next_socket = open(&harness, next).await;
        wait_packet(&next_socket, RegionHandshake::ID).await;
        movement(&next_socket, next, 2).await;
        wait_arrival(&next_socket).await;
        harness.finish().await.unwrap();
    }

    #[tokio::test]
    async fn dispatch_reaps_finished_tasks_while_admission_is_idle() {
        let (submit, queue) = mpsc::channel(1);
        let (_stop, stopping) = watch::channel(false);
        let dispatcher = tokio::spawn(dispatch(
            queue,
            stopping,
            1,
            |result, _| async move { result },
        ));
        for _ in 0..3 {
            let (result, reaped) = oneshot::channel::<()>();
            submit.send(result).await.unwrap();
            // The sender is the task output. It is dropped only when the JoinSet is reaped.
            assert!(
                tokio::time::timeout(Duration::from_secs(1), reaped)
                    .await
                    .unwrap()
                    .is_err()
            );
        }
        drop(submit);
        dispatcher.await.unwrap();
    }

    #[tokio::test]
    async fn two_arrivals_with_the_region_frame_receive_distinct_object_local_ids() {
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let mut harness = Harness::with_scene(
            false,
            false,
            false,
            None,
            Duration::from_secs(120),
            Some(frame.clone()),
        )
        .await;
        let previous_id = frame.allocate_local_id();
        let first = harness.admit(false, false).await;
        let second = harness.admit_other().await;
        let mut ids = Vec::new();
        for expect in [first, second] {
            let socket = open(&harness, expect).await;
            movement(&socket, expect, 2).await;
            let id = tokio::time::timeout(Duration::from_secs(2), async {
                let mut buf = [0; 4096];
                loop {
                    let len = socket.recv(&mut buf).await.unwrap();
                    let packet = parse_packet(&buf[..len]).unwrap();
                    if packet.id == ferroan_protocol::msgs::ObjectUpdate::ID {
                        let update = packet
                            .decode::<ferroan_protocol::msgs::ObjectUpdate>()
                            .unwrap();
                        let block = &update.object_data[0];
                        if block.full_id == expect.agent_id {
                            break block.id;
                        }
                    }
                }
            })
            .await
            .unwrap();
            assert_ne!(id, 0);
            ids.push(id);
        }
        assert_ne!(ids[0], ids[1]);
        // A serve that cannot attach draws its fallback from this same counter.
        let fallback = frame.allocate_local_id();
        assert_eq!(ids, [previous_id + 1, previous_id + 2]);
        assert_eq!(fallback, previous_id + 3);
        harness.finish().await.unwrap();
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

    async fn scene_until<T>(
        frame: &TerrainFrame,
        mut read: impl FnMut(&TerrainFrame) -> ferroan_scene::Pending<T>,
        mut done: impl FnMut(&T) -> bool,
    ) -> T {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let value = read(frame).wait().await.unwrap();
                if done(&value) {
                    return value;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    async fn walk(socket: &tokio::net::UdpSocket, expect: CircuitExpect, sequence: u32) {
        let mut update = AgentUpdate::default();
        update.agent_data.agent_id = expect.agent_id;
        update.agent_data.session_id = expect.session_id;
        update.agent_data.far = 128.0;
        update.agent_data.control_flags = 1;
        send(socket, &update, sequence).await;
    }

    #[tokio::test]
    async fn a_logout_removes_the_scene_presence_and_a_new_login_starts_fresh() {
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let mut harness = Harness::with_scene(
            false,
            false,
            false,
            None,
            Duration::from_secs(120),
            Some(Arc::clone(&frame)),
        )
        .await;
        let first = harness.admit(false, false).await;
        let agent = first.agent_id;
        let socket = open(&harness, first).await;
        wait_packet(&socket, RegionHandshake::ID).await;
        movement(&socket, first, 2).await;
        wait_arrival(&socket).await;
        let login = scene_until(&frame, |f| f.presence(agent).unwrap(), Option::is_some)
            .await
            .unwrap();
        // A high sequence on the first circuit, a walk, and always-run.
        walk(&socket, first, 50).await;
        let mut run = SetAlwaysRun::default();
        run.agent_data.agent_id = agent;
        run.agent_data.session_id = first.session_id;
        run.agent_data.always_run = true;
        send(&socket, &run, 51).await;
        let walking = scene_until(
            &frame,
            |f| f.motion(agent).unwrap(),
            |motion| motion.is_some_and(|motion| motion.always_run),
        )
        .await
        .unwrap();
        assert_eq!(walking.control_flags, 1);
        assert!(walking.target_velocity[0] > 0.0);
        let mut logout = LogoutRequest::default();
        logout.agent_data.agent_id = agent;
        logout.agent_data.session_id = first.session_id;
        send(&socket, &logout, 52).await;
        wait_packet(&socket, LogoutReply::ID).await;
        // The serve ended: no ghost for chat, terse updates or a re-login.
        scene_until(&frame, |f| f.presence(agent).unwrap(), Option::is_none).await;
        assert!(
            frame
                .chat_positions(&[agent])
                .unwrap()
                .wait()
                .await
                .unwrap()
                .is_empty()
        );

        let second = harness.admit(true, false).await;
        let socket = open(&harness, second).await;
        wait_packet(&socket, RegionHandshake::ID).await;
        movement(&socket, second, 2).await;
        wait_arrival(&socket).await;
        let fresh = scene_until(&frame, |f| f.presence(agent).unwrap(), Option::is_some)
            .await
            .unwrap();
        // The login pose again, not where the first circuit walked to. Z may
        // already be on the ground after a tick.
        assert_eq!(fresh.position[..2], login.position[..2]);
        assert_eq!(fresh.control_flags, 0);
        assert_eq!(fresh.draw_distance, 255.0);
        let rested = frame.motion(agent).unwrap().wait().await.unwrap().unwrap();
        assert!(!rested.always_run);
        assert_eq!(rested.target_velocity, [0.0, 0.0, 0.0]);
        // A new client view: sequence 3 is newer than nothing, not older than 50.
        walk(&socket, second, 3).await;
        let walking = scene_until(
            &frame,
            |f| f.motion(agent).unwrap(),
            |motion| motion.is_some_and(|motion| motion.control_flags == 1),
        )
        .await
        .unwrap();
        assert!(walking.target_velocity[0] > 0.0);
        harness.finish().await.unwrap();
        assert!(
            frame
                .presence(agent)
                .unwrap()
                .wait()
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_replaced_login_removes_the_old_presence_before_the_new_one_posts() {
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let mut harness = Harness::with_scene(
            false,
            false,
            false,
            None,
            Duration::from_secs(120),
            Some(Arc::clone(&frame)),
        )
        .await;
        let first = harness.admit(false, false).await;
        let agent = first.agent_id;
        let old = open(&harness, first).await;
        wait_packet(&old, RegionHandshake::ID).await;
        movement(&old, first, 2).await;
        wait_arrival(&old).await;
        walk(&old, first, 50).await;
        let mut run = SetAlwaysRun::default();
        run.agent_data.agent_id = agent;
        run.agent_data.session_id = first.session_id;
        run.agent_data.always_run = true;
        send(&old, &run, 51).await;
        scene_until(
            &frame,
            |f| f.motion(agent).unwrap(),
            |motion| motion.is_some_and(|motion| motion.always_run),
        )
        .await;
        // A second login for the same agent replaces the first circuit. The
        // old serve ends through `replaced()` and removes its presence; the
        // new serve's seat then posts a fresh one.
        let second = harness.admit(true, false).await;
        wait_packet(&old, DisableSimulator::ID).await;
        let seated = scene_until(
            &frame,
            |f| f.motion(agent).unwrap(),
            |motion| motion.is_some_and(|motion| !motion.always_run),
        )
        .await
        .unwrap();
        assert_eq!(seated.target_velocity, [0.0, 0.0, 0.0]);
        let socket = open(&harness, second).await;
        wait_packet(&socket, RegionHandshake::ID).await;
        movement(&socket, second, 2).await;
        wait_arrival(&socket).await;
        walk(&socket, second, 3).await;
        scene_until(
            &frame,
            |f| f.motion(agent).unwrap(),
            |motion| motion.is_some_and(|motion| motion.control_flags == 1),
        )
        .await;
        harness.finish().await.unwrap();
        assert!(
            frame
                .presence(agent)
                .unwrap()
                .wait()
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_dropped_serve_still_removes_its_presence() {
        let frame = Arc::new(TerrainFrame::start_without_map().unwrap());
        let agent = Uuid::from_u128(77);
        frame
            .post_login(
                agent,
                ferroan_scene::Presence {
                    position: [1.0, 2.0, 3.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    velocity: [0.0; 3],
                    control_flags: 0,
                    draw_distance: 255.0,
                },
            )
            .unwrap()
            .wait()
            .await
            .unwrap();
        // A panicking or aborted serve never reaches `finish`.
        drop(ForgetPresence::new(Some(Arc::clone(&frame)), agent));
        assert!(
            frame
                .presence(agent)
                .unwrap()
                .wait()
                .await
                .unwrap()
                .is_none()
        );
        // `finish` takes the frame, so the drop after it does nothing more.
        ForgetPresence::new(Some(Arc::clone(&frame)), agent)
            .finish()
            .await;
        ForgetPresence::new(None, agent).finish().await;
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
    async fn a_far_hop_does_not_log_presence_out() {
        let mut harness = Harness::new(false, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        wait_arrival(&socket).await;
        assert_eq!(harness.count("logout"), 0);
        assert_eq!(harness.count("loggedout"), 0);
        harness.state.close_departed(
            expect.agent_id,
            &ferroan_region::children::FarDestination {
                end: ferroan_region::simulation_client::RegionEnd {
                    server_uri: "http://127.0.0.1:9/".to_string(),
                    region_id: Uuid::from_u128(99),
                    name: "Away".to_string(),
                    loc_x: 768,
                    loc_y: 768,
                },
                size_x: 256,
                size_y: 256,
            },
        );
        harness.finish().await.unwrap();
        assert_eq!(harness.count("logout"), 0);
        assert_eq!(harness.count("loggedout"), 0);
    }

    #[tokio::test]
    async fn a_demoted_child_timeout_does_not_log_presence_out() {
        let mut harness = Harness::new(true, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        wait_arrival(&socket).await;
        harness.state.demote_child(expect.agent_id).unwrap();
        drop(socket);
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.seed_status().await != reqwest::StatusCode::NOT_FOUND {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the demoted child did not time out");
        assert_eq!(harness.count("logout"), 0);
        assert_eq!(harness.count("loggedout"), 0);
        harness.finish().await.unwrap();
        assert_eq!(harness.count("logout"), 0);
        assert_eq!(harness.count("loggedout"), 0);
    }

    #[tokio::test]
    async fn a_fresh_child_that_never_roots_does_not_log_presence_out() {
        let mut harness =
            Harness::with_settings(false, false, false, None, Duration::from_secs(1)).await;
        harness.admit_child().await;
        let seed = format!("{}/CAPS/{}0000/", harness.base, Uuid::from_u128(30));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let before = client
            .post(&seed)
            .header("content-type", "application/llsd+xml")
            .body("<llsd><array/></llsd>")
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(before, reqwest::StatusCode::OK);
        elapse(Duration::from_secs(2)).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = client
                    .post(&seed)
                    .header("content-type", "application/llsd+xml")
                    .body("<llsd><array/></llsd>")
                    .send()
                    .await
                    .unwrap()
                    .status();
                if status == reqwest::StatusCode::NOT_FOUND {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the child seed was not released");
        assert_eq!(harness.count("logout"), 0);
        assert_eq!(harness.count("loggedout"), 0);
        harness.finish().await.unwrap();
        assert_eq!(harness.count("logout"), 0);
        assert_eq!(harness.count("loggedout"), 0);
    }

    #[tokio::test]
    async fn a_demoted_child_replaced_by_another_session_does_not_log_presence_out() {
        let mut harness = Harness::new(false, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        wait_arrival(&socket).await;
        harness.state.demote_child(expect.agent_id).unwrap();
        harness.admit(true, false).await;
        let deadline = std::time::Instant::now() + Duration::from_millis(300);
        while std::time::Instant::now() < deadline {
            assert_eq!(harness.count("logout"), 0);
            assert_eq!(harness.count("loggedout"), 0);
            tokio::task::yield_now().await;
        }
        harness.finish().await.unwrap();
        let old = Field::Str(expect.session_id.to_string());
        let logged_old = harness.requests.lock().unwrap().iter().any(|fields| {
            fields.get("METHOD") == Some(&Field::Str("logout".into()))
                && fields.get("SessionID") == Some(&old)
        });
        assert!(!logged_old);
    }

    #[tokio::test]
    async fn close_agent_of_a_root_still_logs_presence_out() {
        let mut harness = Harness::new(false, false).await;
        let expect = harness.admit(false, false).await;
        let socket = open(&harness, expect).await;
        movement(&socket, expect, 2).await;
        wait_arrival(&socket).await;
        let denied = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
            .delete(format!(
                "{}/agent/{}/{}/?auth=not-the-session",
                harness.base, expect.agent_id, expect.region_id
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), reqwest::StatusCode::OK);
        assert_eq!(harness.count("logout"), 0);
        let closed = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
            .delete(format!(
                "{}/agent/{}/{}/?auth={}",
                harness.base, expect.agent_id, expect.region_id, expect.session_id
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(closed.status(), reqwest::StatusCode::OK);
        tokio::time::timeout(Duration::from_secs(2), async {
            while harness.count("logout") == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("CloseAgent did not log the root out");
        assert_eq!(harness.count("loggedout"), 1);
        let mut saw_disable = false;
        let mut buf = [0u8; 4096];
        let deadline = std::time::Instant::now() + Duration::from_millis(200);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(40), socket.recv(&mut buf)).await {
                Ok(Ok(len)) => {
                    if parse_packet(&buf[..len]).unwrap().id == DisableSimulator::ID {
                        saw_disable = true;
                    }
                }
                _ => break,
            }
        }
        assert!(!saw_disable, "CloseAgent must not send DisableSimulator");
        harness.finish().await.unwrap();
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

    #[tokio::test]
    async fn scene_departure_reads_always_run_from_the_frame() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let agent = Uuid::from_u128(7);
        let pose = ferroan_client::LoginPose {
            position: [128.0, 128.0, 25.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
            control_flags: 0,
            draw_distance: 255.0,
        };
        scene_call(&frame, agent, SceneCall::Arrive(pose, [1.0, 0.0, 0.0]))
            .reply()
            .await
            .unwrap();
        for enabled in [true, false] {
            scene_call(&frame, agent, SceneCall::AlwaysRun(enabled))
                .reply()
                .await
                .unwrap();
            let SceneReply::Departure(departure) = scene_call(&frame, agent, SceneCall::Departure)
                .reply()
                .await
                .unwrap()
            else {
                panic!("expected the frame's departure state");
            };
            assert_eq!(departure.pose.always_run, enabled);
        }
    }

    #[tokio::test]
    async fn the_scene_feed_delivers_the_child_in_the_same_drain_as_a_refused_root() {
        use ferroan_protocol::msgs::ObjectUpdate;

        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/regionstore/objects-xml.json"
        ))
        .unwrap();
        let mut group =
            ferroan_scene::from_xml2(fixture["groups"][1]["xml2"].as_str().unwrap()).unwrap();
        assert_eq!(group.parts.len(), 2);
        group.parts[0]
            .shape
            .texture
            .resize(usize::from(u16::MAX) + 1, 1);
        let child = group.parts[1].uuid;
        let frame = TerrainFrame::with_objects(None, Arc::new(vec![group]), None).unwrap();
        let viewer = Uuid::from_u128(91);
        let feed = frame.new_feed();
        frame
            .attach(
                viewer,
                ferroan_scene::ViewerLink {
                    feed: Arc::clone(&feed),
                    seeds_sent: Arc::new(true.into()),
                    circuit_open: Arc::new(true.into()),
                    look: Arc::new(ferroan_scene::AvatarLook {
                        first_name: "Test".into(),
                        last_name: "User".into(),
                        title: String::new(),
                        texture: Vec::new(),
                        visual_params: Vec::new(),
                        height: 1.8,
                        hover: 0.0,
                    }),
                    teleport_flags: 1 << 7,
                },
            )
            .unwrap()
            .wait()
            .await
            .unwrap();
        let pose = ferroan_scene::Presence {
            position: [128.0, 128.0, 25.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
            control_flags: 0,
            draw_distance: 255.0,
        };
        frame
            .post_login(viewer, pose)
            .unwrap()
            .wait()
            .await
            .unwrap();
        frame
            .arrive(viewer, pose, [1.0, 0.0, 0.0])
            .unwrap()
            .wait()
            .await
            .unwrap();
        frame.handshake_reply(viewer).unwrap();
        let wake = feed.wake();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !feed.has_updates() {
                wake.notified().await;
            }
        })
        .await
        .unwrap();
        // Wait for the frame that offered the root to finish offering its child.
        frame.frames().unwrap().wait().await.unwrap();
        let scene_feed = SceneFeed {
            feed,
            viewer,
            region_handle: 7,
            object_animations: Arc::default(),
            supports_pbr: Arc::default(),
        };
        let ferroan_client::TaskPackets { full, .. } =
            ferroan_client::TaskBatch::drain(&scene_feed, 7, 0, 8);
        assert_eq!(full.len(), 1);
        let mut wire = vec![0xc0, 0, 0, 0, 1, 0];
        wire.extend_from_slice(&full[0]);
        let packet = ferroan_protocol::parse_packet(&wire)
            .unwrap()
            .decode::<ObjectUpdate>()
            .unwrap();
        assert_eq!(packet.object_data.len(), 1);
        assert_eq!(packet.object_data[0].full_id, child);
    }

    /// The feed turns the frame's avatar packets into the wire messages:
    /// another avatar's full update with its local id, name and live values,
    /// its appearance, and the kill.
    #[test]
    fn the_scene_feed_writes_another_avatars_packets() {
        let frame = TerrainFrame::start_without_map().unwrap();
        let feed = frame.new_feed();
        let agent = Uuid::from_u128(90);
        let look = Arc::new(ferroan_scene::AvatarLook {
            first_name: "Chat".to_string(),
            last_name: "Bot".to_string(),
            title: String::new(),
            visual_params: vec![7, 8],
            texture: vec![1; 64],
            height: 1.8,
            hover: 0.0,
        });
        let link = ferroan_scene::ViewerLink {
            feed: frame.new_feed(),
            seeds_sent: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            circuit_open: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            look: Arc::clone(&look),
            teleport_flags: 1 << 7,
        };
        let watcher = Uuid::from_u128(91);
        let watcher_link = ferroan_scene::ViewerLink {
            feed: Arc::clone(&feed),
            seeds_sent: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            circuit_open: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            look: Arc::clone(&look),
            teleport_flags: 1 << 7,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            frame
                .attach(watcher, watcher_link)
                .unwrap()
                .wait()
                .await
                .unwrap();
            let local_id = frame.attach(agent, link).unwrap().wait().await.unwrap();
            let pose = ferroan_scene::Presence {
                position: [100.0, 120.0, 22.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                velocity: [0.0; 3],
                control_flags: 0,
                draw_distance: 255.0,
            };
            frame
                .arrive(agent, pose, [1.0, 0.0, 0.0])
                .unwrap()
                .wait()
                .await
                .unwrap();
            frame.remove(agent).unwrap().wait().await.unwrap();
            let scene_feed = SceneFeed {
                viewer: agent,
                object_animations: Arc::default(),
                supports_pbr: Arc::default(),
                feed,
                region_handle: 77,
            };
            let packets = scene_feed.take_avatar_packets(usize::MAX);
            assert_eq!(packets.len(), 3);
            let ferroan_client::AvatarPacket::Full(update) = &packets[0] else {
                panic!("the full update comes first");
            };
            assert_eq!(update.region_data.region_handle, 77);
            let block = &update.object_data[0];
            assert_eq!(block.id, local_id);
            assert_eq!(block.full_id, agent);
            assert_eq!(block.scale.z, 1.8);
            assert!(
                String::from_utf8_lossy(&block.name_value)
                    .contains("FirstName STRING RW SV Chat\n")
            );
            assert_eq!(&block.object_data[16..20], &100.0f32.to_le_bytes());
            let ferroan_client::AvatarPacket::Appearance(appearance) = &packets[1] else {
                panic!("the appearance follows");
            };
            assert_eq!(appearance.sender.id, agent);
            assert_eq!(appearance.visual_param.len(), 2);
            let ferroan_client::AvatarPacket::Kill(kill) = &packets[2] else {
                panic!("the kill comes last");
            };
            assert_eq!(kill.object_data[0].id, local_id);
        });
    }

    fn override_list(faces: &[(u8, &str)]) -> ferroan_scene::RenderMaterials {
        ferroan_scene::RenderMaterials {
            overrides: Some(
                faces
                    .iter()
                    .map(|(te_index, data)| ferroan_scene::MaterialOverride {
                        te_index: *te_index,
                        data: (*data).to_owned(),
                    })
                    .collect(),
            ),
        }
    }

    fn material_scene() -> (ferroan_scene::ObjectGroup, Uuid, Uuid, Uuid) {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/regionstore/objects-xml.json"
        ))
        .unwrap();
        let mut group =
            ferroan_scene::from_xml2(fixture["groups"][1]["xml2"].as_str().unwrap()).unwrap();
        group.parts.truncate(1);
        group.parts[0].shape.pcode = 9;
        group.parts[0].shape.extra_params = vec![0];
        group.parts[0].animations = None;
        group.parts[0].shape.materials = Some(override_list(&[
            (0, "{'bc':[r1,r0,r0,r1]}"),
            (2, "{'ds':1}"),
        ]));
        let root = group.parts[0].uuid;
        let mut tree = group.parts[0].clone();
        tree.uuid = Uuid::from_u128(0x6500_0000_0000_4000_8000_0000_0000_0003);
        tree.link_num = 2;
        tree.shape.pcode = 255;
        tree.shape.materials = Some(override_list(&[(0, "{'tree':1}")]));
        let tree_id = tree.uuid;
        let mut probe = group.parts[0].clone();
        probe.uuid = Uuid::from_u128(0x6500_0000_0000_4000_8000_0000_0000_0006);
        probe.link_num = 3;
        probe.shape.pcode = 9;
        probe.shape.extra_params = vec![1, 0x90, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        probe.shape.materials = Some(override_list(&[(0, "{'probe':1}")]));
        let probe_id = probe.uuid;
        group.parts.push(tree);
        group.parts.push(probe);
        (group, root, tree_id, probe_id)
    }

    async fn delivered(
        group: ferroan_scene::ObjectGroup,
        pbr: bool,
    ) -> (Vec<(Uuid, u8, u32)>, Vec<String>) {
        use ferroan_protocol::msgs::{GenericStreamingMessage, ObjectUpdate};

        let frame = TerrainFrame::with_objects(None, Arc::new(vec![group]), None).unwrap();
        let viewer = Uuid::from_u128(91);
        let feed = frame.new_feed();
        frame
            .attach(
                viewer,
                ferroan_scene::ViewerLink {
                    feed: Arc::clone(&feed),
                    seeds_sent: Arc::new(true.into()),
                    circuit_open: Arc::new(true.into()),
                    look: Arc::new(ferroan_scene::AvatarLook {
                        first_name: "Test".into(),
                        last_name: "User".into(),
                        title: String::new(),
                        texture: Vec::new(),
                        visual_params: Vec::new(),
                        height: 1.8,
                        hover: 0.0,
                    }),
                    teleport_flags: 1 << 7,
                },
            )
            .unwrap()
            .wait()
            .await
            .unwrap();
        let pose = ferroan_scene::Presence {
            position: [128.0, 128.0, 25.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
            control_flags: 0,
            draw_distance: 255.0,
        };
        frame
            .post_login(viewer, pose)
            .unwrap()
            .wait()
            .await
            .unwrap();
        frame
            .arrive(viewer, pose, [1.0, 0.0, 0.0])
            .unwrap()
            .wait()
            .await
            .unwrap();
        frame.handshake_reply(viewer).unwrap();
        let wake = feed.wake();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !feed.has_updates() {
                wake.notified().await;
            }
        })
        .await
        .unwrap();
        frame.frames().unwrap().wait().await.unwrap();
        let scene_feed = SceneFeed {
            feed,
            viewer,
            region_handle: 7,
            object_animations: Arc::default(),
            supports_pbr: Arc::new(pbr.into()),
        };
        let ferroan_client::TaskPackets {
            full, materials, ..
        } = ferroan_client::TaskBatch::drain(&scene_feed, 7, 0, 8);
        let mut parts = Vec::new();
        for body in full {
            let mut wire = vec![0xc0, 0, 0, 0, 1, 0];
            wire.extend(body);
            let packet = ferroan_protocol::parse_packet(&wire)
                .unwrap()
                .decode::<ObjectUpdate>()
                .unwrap();
            for block in packet.object_data {
                if block.p_code != 47 {
                    parts.push((block.full_id, block.p_code, block.id));
                }
            }
        }
        let mut notes = Vec::new();
        for body in materials {
            assert_eq!(body[0], 31);
            let message = GenericStreamingMessage::decode(&body[1..]).unwrap();
            assert_eq!(message.method_data.method, 16757);
            notes.push(String::from_utf8(message.data_block.data).unwrap());
        }
        (parts, notes)
    }

    #[tokio::test]
    async fn a_pbr_viewer_gets_material_overrides_for_an_ordinary_part_only() {
        let (group, root, tree, probe) = material_scene();
        let (parts, notes) = delivered(group, true).await;
        let local = parts.iter().find(|part| part.0 == root).unwrap().2;
        assert!(parts.iter().any(|part| part.0 == tree && part.1 == 255));
        assert!(parts.iter().any(|part| part.0 == probe && part.1 == 9));
        assert_eq!(
            notes,
            vec![format!(
                "{{'id':i{local},'te':[i0,i2],'od':[{{'bc':[r1,r0,r0,r1]}},{{'ds':1}}]}}"
            )]
        );

        let (mut group, root, _, _) = material_scene();
        group.parts.truncate(1);
        group.parts[0].shape.materials = Some(ferroan_scene::RenderMaterials {
            overrides: Some(vec![ferroan_scene::MaterialOverride {
                te_index: 0,
                data: "a".repeat(5000),
            }]),
        });
        let (parts, notes) = delivered(group, true).await;
        assert!(parts.iter().any(|part| part.0 == root));
        assert!(notes.is_empty());

        let (group, root, _, _) = material_scene();
        let (parts, notes) = delivered(group, false).await;
        assert!(parts.iter().any(|part| part.0 == root));
        assert!(notes.is_empty());
    }
}
