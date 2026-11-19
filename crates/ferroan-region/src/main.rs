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
// Logout behavior ported from
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/GridUser/ActivityDetector.cs
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Presence/PresenceDetector.cs
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs (Lookat)
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
// Ferroan changes: async lifecycle, generation-safe cleanup and process orchestration.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Register a single region with C# Robust and accept its local login messages.

use std::env;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use ferroan_client::{CapsRegistry, CircuitServer};
use ferroan_config::{Config, RegionLoad};
use ferroan_region::config::{RegionConfig, RegionConfigError};
use ferroan_region::grid::{GridClient, GridError, GridUserClient, PresenceClient};
use ferroan_region::simulation::{AcceptedAgent, SimulationState};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("{0}")]
    Arguments(&'static str),
    #[error(transparent)]
    Config(#[from] RegionConfigError),
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
    config.resolve_control_plane().await?;
    // Install handlers before registration. A termination received while the
    // bounded grid request is in flight is handled as soon as it completes,
    // so a successful registration is still followed by deregistration.
    let mut shutdown = tokio::spawn(shutdown_signal()?);
    // Bind both sockets before advertising the region. Early viewer datagrams
    // remain in the socket until Robust supplies the matching circuit.
    let circuit = CircuitServer::bind_at(config.udp_bind)
        .await?
        .with_handshake(config.handshake.clone())
        .with_timing(config.circuit_timing)?;
    let listener = TcpListener::bind(config.http_bind).await?;
    let caps = CapsRegistry::new(&config.server_uri)?;
    let (accepted, agents) = mpsc::channel(1);
    let simulation = SimulationState::new(&config, caps.clone(), accepted);
    let app = simulation.clone().router().merge(caps.router());
    let (stop_http, stopped) = oneshot::channel();
    let mut http = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
    });
    let grid = GridClient::new(&config);
    if let Err(error) = grid.register(&config).await {
        let _ = stop_http.send(());
        http.abort();
        let _ = http.await;
        return Err(error.into());
    }
    println!(
        "INITIALIZATION COMPLETE FOR {} - LOGINS ENABLED",
        config.name
    );
    let (stop_circuits, stopped_circuits) = oneshot::channel();
    let mut circuits = tokio::spawn(dispatch_circuits(
        circuit,
        config.clone(),
        agents,
        stopped_circuits,
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
    let _ = stop_circuits.send(());
    if !circuits_consumed {
        // One active + one queued generation; at most one GridUser and two
        // Presence notifications, each bounded at 30s, plus an admission request.
        let drained = tokio::time::timeout(Duration::from_secs(125), &mut circuits).await;
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
            eprintln!("ferroan-region: logout cleanup failed: {error}");
            outcome = Err(error);
        }
    }
    let deregistered = grid.deregister(config.id).await;
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
    println!("ferroan-region: deregistered");
    outcome
}

async fn dispatch_circuits(
    circuit: CircuitServer,
    config: RegionConfig,
    mut agents: mpsc::Receiver<AcceptedAgent>,
    mut stop: oneshot::Receiver<()>,
) -> Result<(), GridError> {
    let grid_user = GridUserClient::new(&config.grid_user_server_uri);
    let presence = PresenceClient::new(&config.presence_server_uri);
    let mut stopping = false;
    let mut failure = None;
    loop {
        let next = if stopping {
            agents.recv().await
        } else {
            tokio::select! {
                biased;
                _ = &mut stop => {
                    stopping = true;
                    agents.close();
                    agents.recv().await
                }
                next = agents.recv() => next,
            }
        };
        let Some(mut agent) = next else { break };
        if !stopping {
            let expect = agent.expect;
            let arrival = config.arrival(agent.start_pos);
            tokio::select! {
                biased;
                _ = &mut stop => { stopping = true; agents.close(); }
                _ = agent.replaced() => println!("ferroan-region: circuit replaced by a new login"),
                result = circuit.serve_one_with_arrival(expect, arrival) => match result {
                    Ok(()) => println!("ferroan-region: LogoutReply sent"),
                    Err(error) => eprintln!("ferroan-region: circuit ended: {error}"),
                },
            }
        }
        // Keep the generation lease and admission gate throughout both HTTP
        // calls. A stale generation must never mark its replacement offline.
        let guard = agent.lock_lifecycle().await;
        let result = notify_logout(&grid_user, &presence, &config, &agent).await;
        drop(agent);
        drop(guard);
        if let Err(error) = result {
            eprintln!("ferroan-region: logout notification failed: {error}");
            failure.get_or_insert(error);
            stopping = true;
            agents.close();
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn notify_logout(
    grid_user: &GridUserClient,
    presence: &PresenceClient,
    config: &RegionConfig,
    agent: &AcceptedAgent,
) -> Result<(), GridError> {
    let grid_result = if agent.is_current() {
        // ActivityDetector reads ScenePresence.Lookat (zero before camera
        // updates), not CompleteMovement's nonzero arrival-packet fallback.
        // Position remains the accepted start until the scene/terrain tasks.
        grid_user
            .logged_out(agent.expect.agent_id, config.id, agent.start_pos, [0.0; 3])
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
    use ferroan_http::{Field, FieldMap};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    struct Harness {
        state: SimulationState,
        sender: mpsc::Sender<AcceptedAgent>,
        base: String,
        requests: Arc<Mutex<Vec<FieldMap>>>,
        stop: Option<oneshot::Sender<()>>,
        running: Option<tokio::task::JoinHandle<Result<(), GridError>>>,
        tasks: Vec<tokio::task::AbortHandle>,
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
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = requests.clone();
            let app = axum::Router::new().fallback(move |body: String| {
                let fields = ferroan_http::parse_query_string(&body).unwrap();
                let method = match &fields["METHOD"] { Field::Str(s) => s.clone(), _ => panic!() };
                seen.lock().unwrap().push(fields);
                async move {
                    if method == "getagent" {
                        "<ServerResponse><result type=\"List\"><UserID>00000000-0000-4000-8000-000000000001</UserID><RegionID>00000000-0000-0000-0000-000000000000</RegionID></result></ServerResponse>"
                    } else if fail_grid_user && method == "loggedout" {
                        "<ServerResponse><result>Failure</result></ServerResponse>"
                    } else {
                        "<ServerResponse><result>Success</result></ServerResponse>"
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
            config.resolve_control_plane().await.unwrap();
            if short_timeout {
                config.circuit_timing.ack_timeout = Duration::from_millis(20);
            }
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let caps = CapsRegistry::new(&base).unwrap();
            let (sender, agents) = mpsc::channel(1);
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
            let (stop, stopped) = oneshot::channel();
            let running = tokio::spawn(dispatch_circuits(circuit, config, agents, stopped));
            let tasks = vec![
                mock.abort_handle(),
                http.abort_handle(),
                running.abort_handle(),
            ];
            Self {
                state,
                sender,
                base,
                requests,
                stop: Some(stop),
                running: Some(running),
                tasks,
            }
        }

        async fn admit(&self, replacement: bool, same_session: bool) {
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
                while self.sender.capacity() == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }

        fn count(&self, method: &str) -> usize {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|fields| fields.get("METHOD") == Some(&Field::Str(method.into())))
                .count()
        }

        async fn finish(&mut self) -> Result<(), GridError> {
            self.state.stop_accepting();
            let _ = self.stop.take().unwrap().send(());
            tokio::time::timeout(Duration::from_secs(3), self.running.take().unwrap())
                .await
                .unwrap()
                .unwrap()
        }
    }

    #[tokio::test]
    async fn shutdown_and_inactivity_both_report_griduser_and_presence_logout() {
        for timeout in [false, true] {
            let mut harness = Harness::new(timeout, false).await;
            harness.admit(false, false).await;
            if timeout {
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
    async fn griduser_failure_still_removes_presence_and_fails_dispatcher() {
        let mut harness = Harness::new(false, true).await;
        harness.admit(false, false).await;
        assert!(harness.finish().await.is_err());
        assert_eq!(harness.count("loggedout"), 1);
        assert_eq!(harness.count("logout"), 1);
    }
}
