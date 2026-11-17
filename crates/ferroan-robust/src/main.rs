// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The grid-services process. This build listens for the asset routes and
//! nothing else.
//!
//! Configuration is loaded the way `GridServerConfigSource` loads it.
//! `--inimaster` defaults to `OpenSim.Server.GridServer.ini` in the working
//! directory. Repeat `--inifile` for extra files. `--inidirectory` defaults
//! to `config` and is skipped when that directory is absent.
//!
//! The asset database must already be a Tranquillity SQLite file at
//! AssetStore version 6. This program does not create or migrate it.

use std::env;
use std::process::ExitCode;

use ferroan_config::GridLoad;
use ferroan_services::{AssetHttp, listen_address, network_port};
use tokio::net::TcpListener;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ferroan-robust: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let load = arguments()?;
    let resolved = load.load().map_err(|error| error.to_string())?;
    for warning in &resolved.warnings {
        eprintln!("ferroan-robust: include not found: {}", warning.value);
    }
    let port = network_port(&resolved.config).map_err(|error| error.to_string())?;
    let listen = listen_address(&resolved.config).map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let http = AssetHttp::open(&resolved.config)
            .await
            .map_err(|error| error.to_string())?;
        let router = http.into_router();
        let listener = TcpListener::bind((listen, port))
            .await
            .map_err(|error| error.to_string())?;
        eprintln!("ferroan-robust: asset service on {listen}:{port}");
        axum_serve(listener, router).await
    })
}

async fn axum_serve(listener: TcpListener, router: axum::Router) -> Result<(), String> {
    axum::serve(listener, router)
        .await
        .map_err(|error| error.to_string())
}

fn arguments() -> Result<GridLoad, String> {
    let mut ini_master = None;
    let mut ini_files = Vec::new();
    let mut ini_directory = Some("config".to_string());
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "usage: ferroan-robust [--inimaster FILE] [--inifile FILE] [--inidirectory DIR]"
                );
                std::process::exit(0);
            }
            "--inimaster" => ini_master = Some(next_value(&mut args, "--inimaster")?),
            "--inifile" => ini_files.push(next_value(&mut args, "--inifile")?),
            "--inidirectory" => {
                ini_directory = Some(next_value(&mut args, "--inidirectory")?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(GridLoad {
        config_dir: env::current_dir().map_err(|error| error.to_string())?,
        ini_master,
        ini_files,
        ini_directory,
        environment: env::vars().collect(),
    })
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{flag} needs a value"))
}
