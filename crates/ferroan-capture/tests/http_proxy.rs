// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn failed_http_exchange_makes_the_cli_exit_unsuccessfully() {
    cli_failure(false).await;
}

#[tokio::test]
async fn partial_request_eof_makes_the_cli_exit_unsuccessfully() {
    cli_failure(true).await;
}

async fn cli_failure(partial_request: bool) {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reserved.local_addr().unwrap();
    drop(reserved);
    let server = tokio::spawn(async move {
        if partial_request {
            return;
        }
        let (mut socket, _) = upstream.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut bytes = [0; 256];
            let count = socket.read(&mut bytes).await.unwrap();
            assert!(count > 0 && request.len() + count < 4096);
            request.extend_from_slice(&bytes[..count]);
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n")
            .await
            .unwrap();
    });
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.scratch/proxy-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("cli-{}.cap", uuid::Uuid::new_v4()));
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_ferroan-capture"))
            .args([
                "http-proxy",
                "--listen",
                &address.to_string(),
                "--upstream",
                &format!("http://{upstream_addr}"),
                "--out",
            ])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut client = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "proxy exited before accepting a request"
            );
            if let Ok(client) = TcpStream::connect(address).await {
                break client;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let request = if partial_request {
        b"GET /private".as_slice()
    } else {
        b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n".as_slice()
    };
    client.write_all(request).await.unwrap();
    if partial_request {
        client.shutdown().await.unwrap();
    }
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.is_empty());
    let exit = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(exit) = process.0.try_wait().unwrap() {
                break exit;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(exit.code(), Some(1));
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut process.0.stderr.take().unwrap(), &mut stderr).unwrap();
    if partial_request {
        assert!(stderr.contains("HTTP request headers: connection closed (received 12 bytes)"));
        assert!(!stderr.contains("private"));
    } else {
        assert!(stderr.contains("chunked bodies are not captured"));
    }
    server.await.unwrap();
    let records = ferroan_capture::read_capture(&std::fs::read(&path).unwrap()).unwrap();
    if partial_request {
        assert!(records.is_empty());
    } else {
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, ferroan_capture::Kind::HttpRequest);
    }
    std::fs::remove_file(path).unwrap();
}
