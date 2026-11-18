// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP exchanges the bot records for the live harness.
//!
//! The bytes are HTTP/1.1, the same shape `ferroan-capture http-proxy` writes.
//! Recording is off unless [`HttpTrace::scope`] is active on this task.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ferroan_capture::{Kind, Record};

tokio::task_local! {
    static ACTIVE: Arc<HttpTrace>;
}

/// HTTP records collected while a scenario runs.
pub struct HttpTrace {
    records: Mutex<Vec<Record>>,
}

impl HttpTrace {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(Vec::new()),
        })
    }

    /// Run `future` on this task with recording switched on.
    pub async fn scope<F, T>(self: &Arc<Self>, future: F) -> T
    where
        F: Future<Output = T>,
    {
        ACTIVE.scope(Arc::clone(self), future).await
    }

    pub fn records(&self) -> Vec<Record> {
        self.records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    fn push(&self, kind: Kind, bytes: Vec<u8>) {
        let Ok(time_ms) = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
        else {
            return;
        };
        self.records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(Record {
                kind,
                time_ms,
                bytes,
            });
    }
}

/// One HTTP exchange to record.
pub struct Exchange<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub request_type: Option<&'a str>,
    pub accept: Option<&'a str>,
    pub request_body: &'a [u8],
    pub status: u16,
    pub response_type: Option<&'a str>,
    pub response_body: &'a [u8],
}

/// Record one request and its response when a trace is in scope.
/// A message over 1 MiB is left out. The call does nothing with no trace.
pub fn note_exchange(exchange: Exchange<'_>) {
    let Ok(trace) = ACTIVE.try_with(Arc::clone) else {
        return;
    };
    let Some((host, path)) = target(exchange.url) else {
        return;
    };
    let mut request_headers = vec![("Host", host.as_str())];
    if let Some(kind) = exchange.request_type {
        request_headers.push(("Content-Type", kind));
    }
    if let Some(kind) = exchange.accept {
        request_headers.push(("Accept", kind));
    }
    let request = message(
        &format!("{} {path} HTTP/1.1", exchange.method),
        &request_headers,
        exchange.request_body,
    );
    let mut response_headers = Vec::new();
    if let Some(kind) = exchange.response_type {
        response_headers.push(("Content-Type", kind));
    }
    let response = message(
        &format!("HTTP/1.1 {} {}", exchange.status, reason(exchange.status)),
        &response_headers,
        exchange.response_body,
    );
    if request.len() > 1_048_576 || response.len() > 1_048_576 {
        return;
    }
    trace.push(Kind::HttpRequest, request);
    trace.push(Kind::HttpResponse, response);
}

fn target(url: &str) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    let mut path = parsed.path().to_string();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    Some((format!("{host}:{port}"), path))
}

fn message(start: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(start.as_bytes());
    out.extend_from_slice(b"\r\n");
    for (name, value) in headers {
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if !body.is_empty() {
        let len = body.len().to_string();
        out.extend_from_slice(b"Content-Length: ");
        out.extend_from_slice(len.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    out
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_exchange_is_recorded_only_inside_the_scope() {
        note_exchange(Exchange {
            method: "GET",
            url: "http://127.0.0.1:9/missing",
            request_type: None,
            accept: None,
            request_body: b"",
            status: 200,
            response_type: None,
            response_body: b"",
        });
        let trace = HttpTrace::new();
        assert!(trace.records().is_empty());
        trace
            .scope(async {
                note_exchange(Exchange {
                    method: "POST",
                    url: "http://127.0.0.1:9100/login",
                    request_type: Some("text/xml"),
                    accept: None,
                    request_body: b"<xml/>",
                    status: 200,
                    response_type: Some("text/xml"),
                    response_body: b"<ok/>",
                });
            })
            .await;
        let records = trace.records();
        assert_eq!(records.len(), 2);
        let request = String::from_utf8(records[0].bytes.clone()).unwrap();
        assert!(request.starts_with("POST /login HTTP/1.1\r\n"));
        assert!(request.contains("Host: 127.0.0.1:9100\r\n"));
        assert!(request.contains("Content-Type: text/xml\r\n"));
        assert!(request.contains("\r\n\r\n<xml/>"));
        let response = String::from_utf8(records[1].bytes.clone()).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("\r\n\r\n<ok/>"));
    }
}
