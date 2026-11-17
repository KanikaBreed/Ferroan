// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded reads of an HTTP body.

#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(test)]
use tokio::net::TcpStream;

pub enum BodyError {
    Transport,
    TooLarge(usize),
}

/// Read the response body, stopping once `limit` bytes have arrived.
pub async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BodyError> {
    if let Some(len) = response.content_length()
        && len > limit as u64
    {
        return Err(BodyError::TooLarge(len as usize));
    }
    let mut body = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|_| BodyError::Transport)?;
        let Some(chunk) = chunk else {
            return Ok(body);
        };
        let next = body.len().saturating_add(chunk.len());
        if next > limit {
            return Err(BodyError::TooLarge(next));
        }
        body.extend_from_slice(&chunk);
    }
}

/// Read one HTTP/1.1 request, including a `Content-Length` body.
/// Test helper. Stops at 1 MiB.
#[cfg(test)]
pub async fn read_http_request(sock: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline && buf.len() < 1024 * 1024 {
        let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(500), sock.read(&mut tmp)).await
        else {
            break;
        };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(end) = find_subsequence(&buf, b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&buf[..end]);
            let length = content_length(&header);
            if buf.len() >= end + 4 + length {
                break;
            }
        }
    }
    let _ = sock.flush().await;
    buf
}

#[cfg(test)]
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
fn content_length(header: &str) -> usize {
    header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}
