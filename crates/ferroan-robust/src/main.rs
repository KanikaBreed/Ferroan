// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The grid-services process. This build listens for the asset, user-account,
//! user-alias, authentication, authorization, avatar and grid-user routes whose ini
//! sections are present. It also serves the configured XBakes connector.
//!
//! Configuration is loaded the way `GridServerConfigSource` loads it.
//! `--inimaster` defaults to `OpenSim.Server.GridServer.ini` in the working
//! directory. Repeat `--inifile` for extra files. `--inidirectory` defaults
//! to `config` and is skipped when that directory is absent.
//!
//! An asset database must already be a Tranquillity AssetStore: SQLite version
//! 6, MySQL version 10, or PostgreSQL version 9. A user-account database must
//! already be at UserAccount version 4. An authentication database must
//! already be an AuthStore: SQLite version 2, MySQL version 4, or PostgreSQL
//! version 3. A grid-user database must already be a GridUserStore: SQLite
//! version 1, MySQL version 2, or PostgreSQL version 2. This program does not
//! create or migrate those files. User aliases need a table the pin does not
//! ship for SQLite; the process refuses to start that section until the table
//! exists. Authorization does not open a database.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::env;
use std::process::ExitCode;

use axum::Router;
use ferroan_config::GridLoad;
use ferroan_services::bakes::Bakes;
use ferroan_services::{
    AccountHttp, AliasHttp, AssetHttp, AuthHttp, AvatarHttp, GridUserHttp, authorization_router,
    listen_address, network_port, open_authorization,
};

mod status;
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
    let settings =
        ferroan_log::Settings::from_config("ferroan-robust", &resolved.config, None, None)
            .map_err(|error| error.to_string())?;
    let log_guard = ferroan_log::install(&settings).map_err(|error| error.to_string())?;
    for warning in &resolved.warnings {
        let missing = &warning.value;
        tracing::warn!("ferroan-robust: include not found: {missing}");
    }
    let port = network_port(&resolved.config).map_err(|error| error.to_string())?;
    let listen = listen_address(&resolved.config).map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let config = &resolved.config;
        let assets = config.section_names().any(|name| name == "AssetService");
        let accounts = config
            .section_names()
            .any(|name| name == "UserAccountService");
        let aliases = config
            .section_names()
            .any(|name| name == "UserAliasService");
        let auth = config
            .section_names()
            .any(|name| name == "AuthenticationService");
        let authorization = config
            .section_names()
            .any(|name| name == "AuthorizationService");
        let avatars = config.section_names().any(|name| name == "AvatarService");
        let bakes = Bakes::configured(config, port).map_err(|e| e.to_string())?;
        let griduser = config.section_names().any(|name| name == "GridUserService");
        if !assets && !accounts && !aliases && !auth && !authorization && !griduser && !avatars && bakes.is_none() {
            return Err(
                "no AssetService, UserAccountService, UserAliasService, AuthenticationService, AuthorizationService, AvatarService, BakedTextureService or GridUserService section"
                    .to_string(),
            );
        }
        let mut router = Router::new();
        let mut pools = status::Pools::default();
        if assets {
            let http = AssetHttp::open(config)
                .await
                .map_err(|error| error.to_string())?;
            pools = pools.asset(http.clone());
            router = router.merge(http.into_router());
            tracing::info!("ferroan-robust: asset service on {listen}:{port}");
        }
        if accounts {
            let http = AccountHttp::open(config)
                .await
                .map_err(|error| error.to_string())?;
            pools = pools.account(http.clone());
            router = router.merge(http.into_router());
            tracing::info!("ferroan-robust: user account service on {listen}:{port}");
        }
        if aliases {
            let http = AliasHttp::open(config)
                .await
                .map_err(|error| error.to_string())?;
            pools = pools.alias(http.clone());
            router = router.merge(http.into_router());
            tracing::info!("ferroan-robust: user alias service on {listen}:{port}");
        }
        if auth {
            let http = AuthHttp::open(config)
                .await
                .map_err(|error| error.to_string())?;
            pools = pools.auth(http.clone());
            router = router.merge(http.into_router());
            tracing::info!("ferroan-robust: authentication service on {listen}:{port}");
        }
        if authorization {
            open_authorization(config).map_err(|error| error.to_string())?;
            router = router.merge(authorization_router());
            tracing::info!("ferroan-robust: authorization service on {listen}:{port}");
        }
        if avatars {
            let http = AvatarHttp::open(config).await.map_err(|error| error.to_string())?;
            pools = pools.avatar(http.clone());
            router = router.merge(http.into_router());
            tracing::info!("ferroan-robust: avatar service on {listen}:{port}");
        }
        if let Some(cache) = bakes {
            router = router.merge(cache.clone().into_router());
            log_guard.register_console_command(&["delete", "bakes"], Bakes::DELETE_HELP,
                move |args| cache.delete(args).to_owned());
            tracing::info!("ferroan-robust: baked-texture cache on {listen}:{port}");
        }
        if griduser {
            let http = GridUserHttp::open(config)
                .await
                .map_err(|error| error.to_string())?;
            pools = pools.griduser(http.clone());
            router = router.merge(http.into_router());
            tracing::info!("ferroan-robust: grid user service on {listen}:{port}");
        }
        if let Some(console) = log_guard.console_router() {
            router = router.merge(console);
        }
        router = router.merge(status::status_router(config));
        let meters = std::sync::Arc::new(status::RobustMeters::new(pools));
        ferroan_diag::set_context("ferroan-robust", "");
        ferroan_diag::set_configuration(&format!("server=ferroan-robust listen={listen}:{port}"));
        ferroan_diag::set_snapshot({
            let meters = std::sync::Arc::clone(&meters);
            std::sync::Arc::new(move || meters.diag_lines())
        });
        router = status::with_request_counts(router, meters.clone());
        let metrics = status::metrics_listen(config)?;
        let listener = TcpListener::bind((listen, port))
            .await
            .map_err(|error| error.to_string())?;
        if let Some(address) = metrics {
            let metrics_listener = TcpListener::bind(address)
                .await
                .map_err(|error| error.to_string())?;
            let metrics_router = status::metrics_router(meters);
            let service = axum_serve(listener, router);
            let scrape = axum_serve(metrics_listener, metrics_router);
            tokio::try_join!(service, scrape).map(|_| ())
        } else {
            axum_serve(listener, router).await
        }
    })
}

async fn axum_serve(listener: TcpListener, router: axum::Router) -> Result<(), String> {
    axum::serve(ferroan_http::accepted_nodelay(listener), router)
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
