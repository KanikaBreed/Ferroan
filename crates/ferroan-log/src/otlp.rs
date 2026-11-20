// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! OTLP/HTTP JSON export. Nothing is sent unless `[Monitoring] OtlpEndpoint`
//! is set. Records are dropped when the queue is full so the server never
//! waits on the collector.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::LogError;
use crate::timefmt::stamp;

const QUEUE: usize = 256;
const BATCH: usize = 20;
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(crate) struct Exporter {
    tx: SyncSender<Message>,
    records: Arc<AtomicU64>,
}

pub(crate) struct ExporterTask {
    tx: Option<SyncSender<Message>>,
    thread: Option<JoinHandle<()>>,
}

enum Message {
    Log(LogRecord),
    Span(SpanRecord),
    Flush(mpsc::Sender<()>),
    Stop,
}

#[derive(Clone)]
pub(crate) struct LogRecord {
    pub unix_nano: u64,
    pub severity_number: i32,
    pub severity_text: String,
    pub body: String,
    pub target: String,
    pub agent_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

#[derive(Clone)]
pub(crate) struct SpanRecord {
    pub name: String,
    pub trace_id: String,
    pub span_id: String,
    pub start_nano: u64,
    pub end_nano: u64,
    pub agent_id: Option<String>,
}

pub(crate) struct Endpoint {
    host: String,
    addr: SocketAddr,
    prefix: String,
}

impl Exporter {
    pub(crate) fn start(endpoint: &str, server: &str) -> Result<(Self, ExporterTask), LogError> {
        let endpoint = Endpoint::parse(endpoint)?;
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let records = Arc::new(AtomicU64::new(0));
        let records_thread = Arc::clone(&records);
        let server = server.to_owned();
        let thread = std::thread::Builder::new()
            .name("ferroan-otlp".to_owned())
            .spawn(move || run(rx, endpoint, server, records_thread))
            .map_err(|error| LogError::Export(error.to_string()))?;
        Ok((
            Self {
                tx: tx.clone(),
                records,
            },
            ExporterTask {
                tx: Some(tx),
                thread: Some(thread),
            },
        ))
    }

    pub(crate) fn log(&self, record: LogRecord) {
        self.records.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.try_send(Message::Log(record));
    }

    pub(crate) fn span(&self, record: SpanRecord) {
        match self.tx.try_send(Message::Span(record)) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {}
        }
    }
}

impl Drop for ExporterTask {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let (ack_tx, ack_rx) = mpsc::channel();
            let _ = tx.send(Message::Flush(ack_tx));
            let _ = ack_rx.recv_timeout(Duration::from_secs(3));
            let _ = tx.send(Message::Stop);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(rx: Receiver<Message>, endpoint: Endpoint, server: String, records: Arc<AtomicU64>) {
    let mut logs = Vec::new();
    let mut spans = Vec::new();
    let mut last_error = None;
    loop {
        let message = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(message) => Some(message),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let mut flush = None;
        let mut stop = false;
        match message {
            Some(Message::Log(record)) => logs.push(record),
            Some(Message::Span(record)) => spans.push(record),
            Some(Message::Flush(ack)) => flush = Some(ack),
            Some(Message::Stop) => stop = true,
            None => {}
        }
        while logs.len() < BATCH && spans.len() < BATCH {
            match rx.try_recv() {
                Ok(Message::Log(record)) => logs.push(record),
                Ok(Message::Span(record)) => spans.push(record),
                Ok(Message::Flush(ack)) => flush = Some(ack),
                Ok(Message::Stop) => stop = true,
                Err(_) => break,
            }
        }
        if !logs.is_empty() || !spans.is_empty() || flush.is_some() || stop {
            if let Err(error) = send_batch(
                &endpoint,
                &server,
                &logs,
                &spans,
                records.load(Ordering::Relaxed),
            ) {
                note_error(&mut last_error, &error);
            }
            logs.clear();
            spans.clear();
        }
        if let Some(ack) = flush {
            let _ = ack.send(());
        }
        if stop {
            break;
        }
    }
}

fn note_error(last: &mut Option<Instant>, error: &str) {
    let now = Instant::now();
    if last.is_some_and(|then| now.duration_since(then) < Duration::from_secs(30)) {
        return;
    }
    *last = Some(now);
    eprintln!("ferroan-log: OTLP export failed: {error}");
}

fn send_batch(
    endpoint: &Endpoint,
    server: &str,
    logs: &[LogRecord],
    spans: &[SpanRecord],
    records: u64,
) -> Result<(), String> {
    if !logs.is_empty() {
        post(endpoint, "/v1/logs", &logs_body(server, logs))?;
    }
    if !spans.is_empty() {
        post(endpoint, "/v1/traces", &traces_body(server, spans))?;
    }
    let now = stamp(std::time::SystemTime::now()).unix_nano.to_string();
    post(
        endpoint,
        "/v1/metrics",
        &metrics_body(server, &now, records),
    )
}

fn logs_body(server: &str, logs: &[LogRecord]) -> Value {
    json!({
        "resourceLogs": [{
            "resource": {"attributes": [string_attr("service.name", server)]},
            "scopeLogs": [{
                "scope": {"name": "ferroan-log"},
                "logRecords": logs.iter().map(log_record).collect::<Vec<_>>()
            }]
        }]
    })
}

fn log_record(record: &LogRecord) -> Value {
    let mut value = json!({
        "timeUnixNano": record.unix_nano.to_string(),
        "severityNumber": record.severity_number,
        "severityText": record.severity_text,
        "body": {"stringValue": record.body},
        "attributes": [string_attr("target", &record.target)],
    });
    if let Some(agent) = &record.agent_id {
        push_attr(&mut value, "agent.id", agent);
    }
    if let Some(trace_id) = &record.trace_id {
        value["traceId"] = json!(trace_id);
    }
    if let Some(span_id) = &record.span_id {
        value["spanId"] = json!(span_id);
    }
    value
}

fn traces_body(server: &str, spans: &[SpanRecord]) -> Value {
    json!({
        "resourceSpans": [{
            "resource": {"attributes": [string_attr("service.name", server)]},
            "scopeSpans": [{
                "scope": {"name": "ferroan-log"},
                "spans": spans.iter().map(span_record).collect::<Vec<_>>()
            }]
        }]
    })
}

fn span_record(record: &SpanRecord) -> Value {
    let mut value = json!({
        "traceId": record.trace_id,
        "spanId": record.span_id,
        "name": record.name,
        "kind": 1,
        "startTimeUnixNano": record.start_nano.to_string(),
        "endTimeUnixNano": record.end_nano.to_string(),
    });
    if let Some(agent) = &record.agent_id {
        value["attributes"] = json!([string_attr("agent.id", agent)]);
    }
    value
}

fn metrics_body(server: &str, now: &str, records: u64) -> Value {
    json!({
        "resourceMetrics": [{
            "resource": {"attributes": [string_attr("service.name", server)]},
            "scopeMetrics": [{
                "scope": {"name": "ferroan-log"},
                "metrics": [
                    {
                        "name": "ferroan_process_up",
                        "gauge": {"dataPoints": [{"asInt": "1", "timeUnixNano": now}]}
                    },
                    {
                        "name": "ferroan_log_records_total",
                        "sum": {
                            "aggregationTemporality": 2,
                            "isMonotonic": true,
                            "dataPoints": [{"asInt": records.to_string(), "timeUnixNano": now}]
                        }
                    }
                ]
            }]
        }]
    })
}

fn string_attr(key: &str, value: &str) -> Value {
    json!({"key": key, "value": {"stringValue": value}})
}

fn push_attr(value: &mut Value, key: &str, attr: &str) {
    if let Some(list) = value.get_mut("attributes").and_then(Value::as_array_mut) {
        list.push(string_attr(key, attr));
    }
}

pub(crate) fn post(endpoint: &Endpoint, path: &str, body: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(body).map_err(|error| error.to_string())?;
    let mut stream = TcpStream::connect_timeout(&endpoint.addr, TIMEOUT)
        .map_err(|error| format!("connect: {error}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|error| error.to_string())?;
    let target = format!("{}{path}", endpoint.prefix);
    let header = format!(
        "POST {target} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n",
        host = endpoint.host,
        len = bytes.len(),
    );
    stream
        .write_all(header.as_bytes())
        .map_err(|error| error.to_string())?;
    stream
        .write_all(&bytes)
        .map_err(|error| error.to_string())?;
    let mut buf = [0u8; 80];
    let read = stream.read(&mut buf).unwrap_or(0);
    let text = String::from_utf8_lossy(&buf[..read]);
    let status = text.split_whitespace().nth(1).unwrap_or("");
    match status {
        "200" | "202" | "204" => Ok(()),
        _ => Err(format!("HTTP {status}")),
    }
}

impl Endpoint {
    pub(crate) fn parse(raw: &str) -> Result<Self, LogError> {
        let text = raw.trim().trim_end_matches('/');
        let Some(rest) = text.strip_prefix("http://") else {
            return Err(LogError::Endpoint);
        };
        if rest.contains('@') || rest.is_empty() {
            return Err(LogError::Endpoint);
        }
        let (authority, prefix) = rest
            .split_once('/')
            .map_or((rest, ""), |(host, path)| (host, path));
        if authority.is_empty() {
            return Err(LogError::Endpoint);
        }
        let (host, port) = if let Some(host) = authority.strip_prefix('[') {
            let (host, port) = host.split_once("]:").ok_or(LogError::Endpoint)?;
            (host, port)
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            (host, port)
        } else {
            (authority, "80")
        };
        if host.is_empty() {
            return Err(LogError::Endpoint);
        }
        let port: u16 = port.parse().map_err(|_| LogError::Endpoint)?;
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|_| LogError::Endpoint)?
            .next()
            .ok_or(LogError::Endpoint)?;
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("/{prefix}")
        };
        Ok(Self {
            host: authority.to_owned(),
            addr,
            prefix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_log_and_a_span_are_posted_as_otlp_json() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("bind");
        let addr = listener.local_addr().expect("addr");
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let saved = Arc::clone(&bodies);
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let _ = stream.set_read_timeout(Some(TIMEOUT));
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            buf.extend_from_slice(&chunk[..count]);
                            if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length = head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    });
                    let mut body = body.to_owned();
                    if let Some(length) = length {
                        while body.len() < length {
                            match stream.read(&mut chunk) {
                                Ok(0) | Err(_) => break,
                                Ok(count) => {
                                    body.push_str(&String::from_utf8_lossy(&chunk[..count]))
                                }
                            }
                        }
                        body.truncate(length);
                    }
                    saved.lock().expect("bodies").push(body);
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let (exporter, task) =
            Exporter::start(&format!("http://{addr}"), "ferroan-region").expect("start");
        let trace_id = "0123456789abcdef0123456789abcdef";
        let span_id = "0123456789abcdef";
        exporter.log(LogRecord {
            unix_nano: 1,
            severity_number: 9,
            severity_text: "INFO".to_owned(),
            body: "login accepted".to_owned(),
            target: "ferroan_region".to_owned(),
            agent_id: Some("agent".to_owned()),
            trace_id: Some(trace_id.to_owned()),
            span_id: Some(span_id.to_owned()),
        });
        exporter.span(SpanRecord {
            name: "login".to_owned(),
            trace_id: trace_id.to_owned(),
            span_id: span_id.to_owned(),
            start_nano: 1,
            end_nano: 2,
            agent_id: None,
        });
        drop(exporter);
        drop(task);
        let _ = server.join();
        let joined = bodies.lock().expect("bodies").join("\n");
        assert!(joined.contains("login accepted"), "{joined}");
        assert!(joined.contains(trace_id), "{joined}");
        assert!(joined.contains("ferroan_process_up"), "{joined}");
    }
}
