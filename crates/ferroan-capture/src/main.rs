// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `ferroan-capture http-proxy`, `check`, `redact` and `replay`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use ferroan_capture::{
    DEFAULT_REPLY_TIMEOUT, check_udp, read_capture, redact_records, serve_with_timeout,
    write_capture,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ferroan-capture: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("http-proxy") => proxy(args),
        Some("check") | Some("replay") => {
            let path = PathBuf::from(need(&mut args, "file")?);
            let records = load(&path)?;
            check_udp(&records).map_err(|error| error.to_string())?;
            println!("replay: {} records, udp round-trip matched", records.len());
            Ok(())
        }
        Some("redact") => redact(args),
        Some("--help") | Some("-h") | None => {
            println!(
                "usage: ferroan-capture http-proxy --listen ADDR --upstream URL --out FILE [--reply-timeout-secs 70]\n       \
                 ferroan-capture check FILE\n       \
                 ferroan-capture redact --in FILE --out FILE\n       \
                 ferroan-capture replay FILE"
            );
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other}")),
    }
}

fn proxy(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut listen = None;
    let mut upstream = None;
    let mut out = None;
    let mut reply_timeout = DEFAULT_REPLY_TIMEOUT;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => listen = Some(parse_addr(&need(&mut args, "--listen")?)?),
            "--upstream" => upstream = Some(parse_upstream(&need(&mut args, "--upstream")?)?),
            "--out" => out = Some(PathBuf::from(need(&mut args, "--out")?)),
            "--reply-timeout-secs" => {
                reply_timeout = parse_reply_timeout(&need(&mut args, "--reply-timeout-secs")?)?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let listen = listen.ok_or("http-proxy needs --listen")?;
    let upstream = upstream.ok_or("http-proxy needs --upstream")?;
    let out = out.ok_or("http-proxy needs --out")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime
        .block_on(serve_with_timeout(listen, upstream, &out, reply_timeout))
        .map_err(|error| error.to_string())
}

fn parse_reply_timeout(text: &str) -> Result<Duration, String> {
    let seconds: u64 = text
        .parse()
        .map_err(|_| "reply timeout must be from 1 to 300 seconds")?;
    if !(1..=300).contains(&seconds) {
        return Err("reply timeout must be from 1 to 300 seconds".to_string());
    }
    Ok(Duration::from_secs(seconds))
}

fn redact(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut input = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--in" => input = Some(PathBuf::from(need(&mut args, "--in")?)),
            "--out" => output = Some(PathBuf::from(need(&mut args, "--out")?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let input = input.ok_or("redact needs --in")?;
    let output = output.ok_or("redact needs --out")?;
    let records = load(&input)?;
    let redacted = redact_records(&records).map_err(|error| error.to_string())?;
    let bytes = write_capture(&redacted).map_err(|error| error.to_string())?;
    std::fs::write(&output, bytes).map_err(|error| error.to_string())?;
    println!("redact: wrote {} records", redacted.len());
    Ok(())
}

fn load(path: &std::path::Path) -> Result<Vec<ferroan_capture::Record>, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    read_capture(&bytes).map_err(|error| error.to_string())
}

fn need(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_addr(text: &str) -> Result<SocketAddr, String> {
    text.parse()
        .map_err(|_| format!("{text} is not an address"))
}

fn parse_upstream(text: &str) -> Result<SocketAddr, String> {
    let rest = text
        .strip_prefix("http://")
        .ok_or("upstream must be http://host:port")?;
    let host = rest.split('/').next().unwrap_or(rest);
    host.parse()
        .map_err(|_| format!("{text} is not http://host:port"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_timeout_option_rejects_invalid_or_unbounded_values() {
        for value in ["0", "301", "-1", "NaN", "18446744073709551616"] {
            assert!(parse_reply_timeout(value).is_err());
        }
        assert_eq!(parse_reply_timeout("70").unwrap(), Duration::from_secs(70));
    }
}
