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
use ferroan_region::grid::{GridClient, GridError};
use ferroan_region::simulation::SimulationState;
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

async fn serve(config: RegionConfig) -> Result<(), RunError> {
    // Install handlers before registration. A termination received while the
    // bounded grid request is in flight is handled as soon as it completes,
    // so a successful registration is still followed by deregistration.
    let mut shutdown = tokio::spawn(shutdown_signal()?);
    // Bind both sockets before advertising the region. Early viewer datagrams
    // remain in the socket until Robust supplies the matching circuit.
    let circuit = CircuitServer::bind_at(config.udp_bind)
        .await?
        .with_handshake(config.handshake.clone());
    let listener = TcpListener::bind(config.http_bind).await?;
    let caps = CapsRegistry::new(&config.server_uri)?;
    let (accepted, mut agents) = mpsc::channel(1);
    let simulation = SimulationState::new(config.id, caps.clone(), accepted);
    let app = simulation.router().merge(caps.router());
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
    let mut circuits = tokio::spawn(async move {
        while let Some(agent) = agents.recv().await {
            match circuit
                .serve_one(agent.expect, Duration::from_secs(300))
                .await
            {
                Ok(()) => println!("ferroan-region: LogoutReply sent"),
                Err(error) => eprintln!("ferroan-region: circuit ended: {error}"),
            }
            // The accepted agent owns the seed and the one-circuit reservation.
            drop(agent);
        }
    });
    let outcome: Result<(), RunError> = tokio::select! {
        signal = &mut shutdown => signal.map_err(RunError::Task).and_then(|result| result.map_err(RunError::Io)),
        result = &mut http => match result {
            Ok(result) => result.map_err(RunError::Io),
            Err(error) => Err(RunError::Task(error)),
        },
        result = &mut circuits => match result {
            Ok(()) => Err(RunError::DispatcherStopped),
            Err(error) => Err(RunError::Task(error)),
        },
    };
    shutdown.abort();
    if !circuits.is_finished() {
        circuits.abort();
        // Abort drops an active agent lease and its capability routes.
        let _ = circuits.await;
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
