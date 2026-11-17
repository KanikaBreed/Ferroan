// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP/1.1 reverse proxy that appends each request and response to a capture.
//!
//! It speaks plain HTTP on a local address. Chunked bodies are refused.
//! A body without `Content-Length` is empty on a request and is read until
//! the server closes on a response, up to 1 MiB.

use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

use crate::format::{Kind, Record, write_capture};

const HEADER_CAP: usize = 65_536;
const BODY_CAP: usize = 1_048_576;
const IO_WAIT: Duration = Duration::from_secs(5);

pub async fn serve(listen: SocketAddr, upstream: SocketAddr, path: &Path) -> io::Result<()> {
    let listener = TcpListener::bind(listen).await?;
    serve_on(listener, upstream, path).await
}

pub async fn serve_on(listener: TcpListener, upstream: SocketAddr, path: &Path) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    file.write_all(crate::format::MAGIC)?;
    let capture = Arc::new(Capture {
        file: Mutex::new(file),
    });
    loop {
        let (client, _) = listener.accept().await?;
        let capture = Arc::clone(&capture);
        tokio::spawn(async move {
            if let Err(error) = forward(client, upstream, capture).await {
                eprintln!("ferroan-capture: {error}");
            }
        });
    }
}

struct Capture {
    file: Mutex<std::fs::File>,
}

impl Capture {
    fn push(&self, kind: Kind, bytes: &[u8]) -> io::Result<()> {
        let record = Record {
            kind,
            time_ms: unix_ms(),
            bytes: bytes.to_vec(),
        };
        let encoded = write_capture(&[record]).map_err(io::Error::other)?;
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        file.write_all(&encoded[crate::format::MAGIC.len()..])?;
        file.flush()
    }
}

async fn forward(
    mut client: TcpStream,
    upstream: SocketAddr,
    capture: Arc<Capture>,
) -> io::Result<()> {
    let request = read_message(&mut client, true).await?;
    capture.push(Kind::HttpRequest, &request)?;
    let mut server = timeout(IO_WAIT, TcpStream::connect(upstream))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "upstream connect timed out"))??;
    server.write_all(&request).await?;
    let response = read_message(&mut server, false).await?;
    capture.push(Kind::HttpResponse, &response)?;
    client.write_all(&response).await?;
    Ok(())
}

async fn read_message(stream: &mut TcpStream, request: bool) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        if buf.len() > HEADER_CAP {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "headers are too large",
            ));
        }
        let n = read_some(stream, &mut tmp).await?;
        buf.extend_from_slice(&tmp[..n]);
        if let Some(at) = find_bytes(&buf, b"\r\n\r\n") {
            break at + 4;
        }
    };
    if header_has(&buf[..head_end], "transfer-encoding:") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunked bodies are not captured",
        ));
    }
    let body_len = content_length(&buf[..head_end]);
    match body_len {
        Some(len) if len > BODY_CAP => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "body is too large",
        )),
        Some(len) => {
            while buf.len() < head_end + len {
                let n = read_some(stream, &mut tmp).await?;
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > head_end + BODY_CAP {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "body is too large",
                    ));
                }
            }
            buf.truncate(head_end + len);
            Ok(buf)
        }
        None if request => Ok(buf[..head_end].to_vec()),
        None => {
            loop {
                if buf.len() > head_end + BODY_CAP {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "body is too large",
                    ));
                }
                match timeout(IO_WAIT, stream.read(&mut tmp)).await {
                    Ok(Ok(0)) | Err(_) => break,
                    Ok(Ok(n)) => buf.extend_from_slice(&tmp[..n]),
                    Ok(Err(error)) => return Err(error),
                }
            }
            Ok(buf)
        }
    }
}

async fn read_some(stream: &mut TcpStream, tmp: &mut [u8]) -> io::Result<usize> {
    let n = timeout(IO_WAIT, stream.read(tmp))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "read timed out"))??;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "connection closed",
        ));
    }
    Ok(n)
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len())
        .position(|window| window == needle)
}

fn header_has(head: &[u8], name: &str) -> bool {
    String::from_utf8_lossy(head)
        .to_ascii_lowercase()
        .contains(name)
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn content_length(head: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(head);
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::read_capture;

    #[tokio::test]
    async fn a_post_is_recorded_on_both_sides() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = upstream.accept().await.unwrap();
            let mut buf = vec![0u8; 2048];
            let mut got = Vec::new();
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                if got.windows(4).any(|window| window == b"\r\n\r\n") {
                    let head = got.split(|b| *b == b'\n').find_map(|line| {
                        let line = String::from_utf8_lossy(line);
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                    });
                    let split = got
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .unwrap()
                        + 4;
                    if got.len() >= split + head.unwrap_or(0) {
                        break;
                    }
                }
            }
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\npong")
                .await
                .unwrap();
        });

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_addr = listener.local_addr().unwrap();
        let path = std::env::temp_dir().join(format!("ferroan-capture-{}.cap", std::process::id()));
        let path2 = path.clone();
        tokio::spawn(async move {
            let _ = serve_on(listener, upstream_addr, &path2).await;
        });

        let mut client = TcpStream::connect(listen_addr).await.unwrap();
        client
            .write_all(b"POST /assets HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 4\r\n\r\nping")
            .await
            .unwrap();
        let mut response = vec![0u8; 128];
        let n = client.read(&mut response).await.unwrap();
        assert!(response[..n].windows(4).any(|window| window == b"pong"));

        let stored = std::fs::read(&path).unwrap();
        let records = read_capture(&stored).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].kind, Kind::HttpRequest);
        assert!(records[0].bytes.windows(4).any(|window| window == b"ping"));
        assert_eq!(records[1].kind, Kind::HttpResponse);
        assert!(records[1].bytes.windows(4).any(|window| window == b"pong"));
        assert!(records[0].time_ms > 0);
        assert!(records[1].time_ms > 0);
        let _ = std::fs::remove_file(path);
    }
}
