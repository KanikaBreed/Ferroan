// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `ferroan-bot login` posts `login_to_simulator` and prints the result.
//!
//! `--circuit` also completes the region handshake and logs out. The password
//! is never printed. Pass it with `--password` or `FERROAN_BOT_PASSWORD`.

use std::env;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use ferroan_bot::login::{self, LoginRequest};
use ferroan_bot::session::{Agent, Session};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ferroan-bot: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("login") => login_command(args),
        Some("--help") | Some("-h") | None => {
            println!(
                "usage: ferroan-bot login --uri URL --user \"First Last\" [--password PASS] [--start last] [--circuit]\n\n\
                 The password may also be FERROAN_BOT_PASSWORD. It is not printed.\n\
                 --circuit sends UseCircuitCode, completes the handshake, requests the seed and logs out."
            );
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other}")),
    }
}

fn login_command(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut uri = None;
    let mut user = None;
    let mut password = None;
    let mut start = "last".to_owned();
    let mut circuit = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--uri" => uri = Some(need(&mut args, "--uri")?),
            "--user" => user = Some(need(&mut args, "--user")?),
            "--password" => password = Some(need(&mut args, "--password")?),
            "--start" => start = need(&mut args, "--start")?,
            "--circuit" => circuit = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let uri = uri.ok_or("login needs --uri")?;
    let user = user.ok_or("login needs --user")?;
    let password = password
        .or_else(|| env::var("FERROAN_BOT_PASSWORD").ok())
        .ok_or("login needs --password or FERROAN_BOT_PASSWORD")?;
    let request = LoginRequest {
        login_uri: uri,
        username: user,
        password,
        start,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let http = login::http_client().map_err(|error| error.to_string())?;
        let response = login::login(&http, &request)
            .await
            .map_err(|error| error.to_string())?;
        println!("login: true");
        println!("agent_id: {}", response.agent_id);
        println!("sim: {}:{}", response.sim_ip, response.sim_port);
        println!(
            "seed: {}",
            if response.seed_capability.is_empty() {
                "no"
            } else {
                "yes"
            }
        );
        if circuit {
            let sim = SocketAddr::from((response.sim_ip, response.sim_port));
            let mut session = Session::connect(
                sim,
                Agent {
                    agent_id: response.agent_id,
                    session_id: response.session_id,
                    circuit_code: response.circuit_code,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            session
                .run_until_ready(&http, &response.seed_capability, Duration::from_secs(20))
                .await
                .map_err(|error| error.to_string())?;
            println!("circuit: ready");
            session
                .logout(Duration::from_secs(10))
                .await
                .map_err(|error| error.to_string())?;
            println!("logout: true");
        }
        Ok(())
    })
}

fn need(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}
