// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One localhost HTTP request with the accepted socket left at the OS default,
//! then one with [`ferroan_http::set_tcp_nodelay`]. `live run` prints both.
//! The client sets `TCP_NODELAY` on each, which is reqwest's default. The
//! numbers are a record, not a threshold.

use std::io::{self, ErrorKind};
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(2);

const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

/// Print `live: tcp nodelay request latency before <us> after <us>`.
pub(crate) fn record() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("tcp nodelay latency runtime: {error}"))?;
    let (before, after) = runtime
        .block_on(samples())
        .map_err(|error| format!("tcp nodelay request latency: {error}"))?;
    println!(
        "live: tcp nodelay request latency before {}us after {}us",
        before.as_micros(),
        after.as_micros()
    );
    Ok(())
}

async fn samples() -> io::Result<(Duration, Duration)> {
    let before = timeout(LIMIT, one(false))
        .await
        .map_err(|_| io::Error::new(ErrorKind::TimedOut, "before sample timed out"))??;
    let after = timeout(LIMIT, one(true))
        .await
        .map_err(|_| io::Error::new(ErrorKind::TimedOut, "after sample timed out"))??;
    Ok((before, after))
}

async fn one(nodelay: bool) -> io::Result<Duration> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        if nodelay {
            ferroan_http::set_tcp_nodelay(&stream)?;
        }
        let enabled = stream.nodelay()?;
        if enabled != nodelay {
            return Err(io::Error::other(format!(
                "accepted TCP_NODELAY is {enabled}, want {nodelay}"
            )));
        }
        read_headers(&mut stream).await?;
        stream.write_all(RESPONSE).await?;
        Ok(())
    });

    let mut client = TcpStream::connect(addr).await?;
    ferroan_http::set_tcp_nodelay(&client)?;
    if !client.nodelay()? {
        return Err(io::Error::other("outbound TCP_NODELAY is off"));
    }
    let started = Instant::now();
    client.write_all(REQUEST).await?;
    let response = read_headers(&mut client).await?;
    let elapsed = started.elapsed();
    match timeout(LIMIT, server).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => return Err(error),
        Ok(Err(error)) => return Err(io::Error::other(error.to_string())),
        Err(_) => {
            return Err(io::Error::new(
                ErrorKind::TimedOut,
                "latency server timed out",
            ));
        }
    }
    if !response.starts_with(b"HTTP/1.1 200 ") {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "latency response was not 200",
        ));
    }
    Ok(elapsed)
}

async fn read_headers(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "connection closed before the HTTP headers",
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 1024 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "headers are too large",
            ));
        }
        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_latency_records_before_and_after() {
        let (before, after) = samples().await.unwrap();
        assert!(!before.is_zero(), "before sample was zero");
        assert!(!after.is_zero(), "after sample was zero");
    }
}
