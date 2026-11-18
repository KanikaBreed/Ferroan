// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP/1.1 reverse proxy that appends each request and response to a capture.
//!
//! It speaks plain HTTP on a local address. Chunked bodies are refused.
//! A body without `Content-Length` is empty on a request and is read until
//! the server closes on a response, up to 1 MiB.
//! Each connection carries one exchange. Forwarded messages explicitly say
//! `Connection: close`; captures retain the original received headers and body.
//! The configurable reply deadline includes held polls and their complete body.
//!
//! Timeout reference (no C# code copied): EventQueueGetModule.cs, KEEPALIVE,
//! VIEWERKEEPALIVE and OnRegisterCaps, under
//! Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/ at Tranquillity
//! 858321883f5487a5d192d36ea9d7b90fa468310c: polls can be held for 58 seconds.

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
pub const DEFAULT_REPLY_TIMEOUT: Duration = Duration::from_secs(70);
const MAX_REPLY_TIMEOUT: Duration = Duration::from_secs(300);

pub async fn serve(listen: SocketAddr, upstream: SocketAddr, path: &Path) -> io::Result<()> {
    serve_with_timeout(listen, upstream, path, DEFAULT_REPLY_TIMEOUT).await
}

pub async fn serve_on(listener: TcpListener, upstream: SocketAddr, path: &Path) -> io::Result<()> {
    serve_on_with_timeout(listener, upstream, path, DEFAULT_REPLY_TIMEOUT).await
}

pub async fn serve_with_timeout(
    listen: SocketAddr,
    upstream: SocketAddr,
    path: &Path,
    reply_timeout: Duration,
) -> io::Result<()> {
    validate_timeout(reply_timeout)?;
    let listener = TcpListener::bind(listen).await?;
    serve_on_with_timeout(listener, upstream, path, reply_timeout).await
}

/// Run one proxy with a total upstream reply deadline, greater than zero and
/// at most five minutes. Requests, connects and writes retain five-second bounds.
pub async fn serve_on_with_timeout(
    listener: TcpListener,
    upstream: SocketAddr,
    path: &Path,
    reply_timeout: Duration,
) -> io::Result<()> {
    validate_timeout(reply_timeout)?;
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
            if let Err(error) = forward(client, upstream, capture, reply_timeout).await {
                eprintln!("ferroan-capture: {error}");
            }
        });
    }
}

fn validate_timeout(wait: Duration) -> io::Result<()> {
    if wait.is_zero() || wait > MAX_REPLY_TIMEOUT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "reply timeout must be greater than zero and at most 300 seconds",
        ));
    }
    Ok(())
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
    reply_timeout: Duration,
) -> io::Result<()> {
    let request = read_message(&mut client, true, IO_WAIT).await?;
    capture.push(Kind::HttpRequest, &request)?;
    let mut server = timeout(IO_WAIT, TcpStream::connect(upstream))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "upstream connect timed out"))??;
    write_message(&mut server, &close_connection(&request)?).await?;
    let response = read_message(&mut server, false, reply_timeout).await?;
    capture.push(Kind::HttpResponse, &response)?;
    write_message(&mut client, &close_connection(&response)?).await?;
    Ok(())
}

async fn write_message(stream: &mut TcpStream, bytes: &[u8]) -> io::Result<()> {
    timeout(IO_WAIT, stream.write_all(bytes))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "write timed out"))?
}

/// Preserve the payload while making single-exchange connection ownership
/// explicit to both HTTP pools. Do not advertise a reusable socket then drop it.
fn close_connection(message: &[u8]) -> io::Result<Vec<u8>> {
    let end = find_bytes(message, b"\r\n\r\n")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP headers are incomplete"))?;
    let mut output = Vec::with_capacity(message.len() + 21);
    for line in message[..end].split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(colon) = line.iter().position(|byte| *byte == b':') {
            let name = &line[..colon];
            if name.eq_ignore_ascii_case(b"connection")
                || name.eq_ignore_ascii_case(b"keep-alive")
                || name.eq_ignore_ascii_case(b"proxy-connection")
            {
                continue;
            }
        }
        output.extend_from_slice(line);
        output.extend_from_slice(b"\r\n");
    }
    output.extend_from_slice(b"Connection: close\r\n\r\n");
    output.extend_from_slice(&message[end + 4..]);
    Ok(output)
}

async fn read_message(
    stream: &mut TcpStream,
    request: bool,
    wait: Duration,
) -> io::Result<Vec<u8>> {
    timeout(wait, read_message_inner(stream, request))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "HTTP message timed out"))?
}

async fn read_message_inner(stream: &mut TcpStream, request: bool) -> io::Result<Vec<u8>> {
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
            if at + 4 > HEADER_CAP {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "headers are too large",
                ));
            }
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
                match stream.read(&mut tmp).await {
                    Ok(0) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    Err(error) => return Err(error),
                }
            }
            Ok(buf)
        }
    }
}

async fn read_some(stream: &mut TcpStream, tmp: &mut [u8]) -> io::Result<usize> {
    let n = stream.read(tmp).await?;
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

    fn capture_file() -> (Arc<Capture>, std::path::PathBuf) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.scratch/proxy-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.cap", uuid::Uuid::new_v4()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(crate::format::MAGIC).unwrap();
        (
            Arc::new(Capture {
                file: Mutex::new(file),
            }),
            path,
        )
    }

    #[tokio::test]
    async fn held_reply_survives_the_previous_five_second_limit() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let request = read_message(&mut socket, true, IO_WAIT).await.unwrap();
            assert!(header_has(&request, "connection: close"));
            tokio::time::sleep(Duration::from_millis(5200)).await;
            socket
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (capture, path) = capture_file();
        let forwarding = tokio::spawn(async move {
            let (client, _) = listener.accept().await.unwrap();
            forward(client, upstream_addr, capture, Duration::from_secs(7)).await
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(
                b"POST /event-queue HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        timeout(Duration::from_secs(9), client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with(b"HTTP/1.1 502 Bad Gateway"));
        forwarding.await.unwrap().unwrap();
        server.await.unwrap();
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[1].bytes.starts_with(b"HTTP/1.1 502 Bad Gateway"));
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn stalled_headers_and_partial_bodies_fail_without_recording_a_reply() {
        for partial in [
            b"".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\npart".as_slice(),
            b"HTTP/1.1 200 OK\r\n\r\npart".as_slice(),
        ] {
            let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream_addr = upstream.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = upstream.accept().await.unwrap();
                read_message(&mut socket, true, IO_WAIT).await.unwrap();
                socket.write_all(partial).await.unwrap();
                // Keep the response incomplete until the proxy deadline expires.
                let mut byte = [0];
                let _ = socket.read(&mut byte).await;
            });
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (capture, path) = capture_file();
            let forwarding = tokio::spawn(async move {
                let (client, _) = listener.accept().await.unwrap();
                forward(client, upstream_addr, capture, Duration::from_millis(100)).await
            });
            let mut client = TcpStream::connect(address).await.unwrap();
            client
                .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await
                .unwrap();
            let mut response = Vec::new();
            timeout(IO_WAIT, client.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert!(response.is_empty());
            assert_eq!(
                forwarding.await.unwrap().unwrap_err().kind(),
                io::ErrorKind::TimedOut
            );
            server.await.unwrap();
            let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].kind, Kind::HttpRequest);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test]
    async fn single_exchange_connections_never_advertise_keep_alive() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = upstream.accept().await.unwrap();
                let request = read_message(&mut socket, true, IO_WAIT).await.unwrap();
                assert!(header_has(&request, "connection: close"));
                assert!(!header_has(&request, "keep-alive"));
                socket.write_all(b"HTTP/1.1 200 OK\r\nConnection: Keep-Alive\r\nKeep-Alive: timeout=30\r\nContent-Length: 4\r\n\r\npong").await.unwrap();
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (capture, path) = capture_file();
        let forwarding = tokio::spawn(async move {
            for _ in 0..2 {
                let (client, _) = listener.accept().await.unwrap();
                forward(client, upstream_addr, capture.clone(), IO_WAIT)
                    .await
                    .unwrap();
            }
        });
        for _ in 0..2 {
            let mut client = TcpStream::connect(address).await.unwrap();
            client.write_all(b"POST /presence HTTP/1.1\r\nHost: localhost\r\nConnection: Keep-Alive\r\nKeep-Alive: timeout=30\r\nContent-Length: 0\r\n\r\n").await.unwrap();
            let mut reply = Vec::new();
            timeout(IO_WAIT, client.read_to_end(&mut reply))
                .await
                .unwrap()
                .unwrap();
            assert!(header_has(&reply, "connection: close"));
            assert!(!header_has(&reply, "keep-alive"));
            assert!(reply.ends_with(b"pong"));
        }
        forwarding.await.unwrap();
        server.await.unwrap();
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records.len(), 4);
        // Captures describe each original received message, before hop headers change.
        assert!(
            records
                .iter()
                .all(|record| header_has(&record.bytes, "connection: keep-alive"))
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reply_deadline_is_bounded() {
        assert!(validate_timeout(Duration::ZERO).is_err());
        assert!(validate_timeout(Duration::from_secs(301)).is_err());
        assert!(validate_timeout(DEFAULT_REPLY_TIMEOUT).is_ok());
    }

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
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.scratch/proxy-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.cap", uuid::Uuid::new_v4()));
        let path2 = path.clone();
        let serving = tokio::spawn(async move {
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
        serving.abort();
        let _ = serving.await;
        std::fs::remove_file(path).unwrap();
    }
}
