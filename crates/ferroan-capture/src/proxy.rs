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
//! The request is queued as soon as it is in hand, and a side thread writes
//! the file. A held poll is therefore in the capture before its response,
//! and a stalled disk does not sit on the path that owes the client a reply.
//! The response is queued before that reply is written, so the next poll
//! cannot land between them. This task waits for the disk only after the
//! client has the bytes. A reset does not fail the proxy. A client that
//! shuts down only its write side still receives the response.
//!
//! The owner calls `ProxyStatus::upstream_stopping` when it begins to stop
//! the upstream server on purpose. A request that arrives after that and finds
//! the upstream gone (its connect refused, or the connection reset, closed or
//! broken before any response byte) stays in the capture with no response, is
//! counted by `ProxyStatus::refused_after_stop`, and is not a failure: another
//! server still running sent it to one the owner is shutting down. The same
//! errors before that call still fail the proxy.
//!
//! Timeout reference (no C# code copied): EventQueueGetModule.cs, KEEPALIVE,
//! VIEWERKEEPALIVE and OnRegisterCaps, under
//! Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/ at Tranquillity
//! 858321883f5487a5d192d36ea9d7b90fa468310c: polls can be held for 58 seconds.

use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::format::{Kind, Record, write_capture};

const HEADER_CAP: usize = 65_536;
const BODY_CAP: usize = 1_048_576;
const IO_WAIT: Duration = Duration::from_secs(5);
pub const DEFAULT_REPLY_TIMEOUT: Duration = Duration::from_secs(70);
const MAX_REPLY_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_EXCHANGES: usize = 256;
const MAX_INTERIM_RESPONSES: usize = 16;
const MAX_FINISH_TIMEOUT: Duration = Duration::from_secs(320);

/// `TCP_NODELAY`, as `HttpListener.cs` does after accept and
/// `OutboundUrlFilter.cs` does before connect. The same call as
/// `ferroan_http::set_tcp_nodelay`; this crate does not depend on that one.
fn set_tcp_nodelay(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)
}

/// Retained evidence for one proxy, even if its serving task has returned.
/// Use a fresh status per server and call `finish` before dropping its runtime.
#[derive(Clone, Default)]
pub struct ProxyStatus(Arc<StatusInner>);

#[derive(Default)]
struct StatusInner {
    failures: AtomicU64,
    /// Set by the owner when it begins stopping the upstream server.
    upstream_stopping: AtomicBool,
    /// Requests that found the upstream gone after `upstream_stopping` was set.
    refused_after_stop: AtomicU64,
    stopping: AtomicBool,
    finished: AtomicBool,
    stop: Notify,
}

impl ProxyStatus {
    pub fn failures(&self) -> u64 {
        self.0.failures.load(Ordering::Acquire)
    }

    /// Mark the upstream as being stopped on purpose. From now on a request
    /// that finds the upstream gone before any response byte is not a failure. Call it just before
    /// the upstream server is told to stop, never while it is meant to be up.
    pub fn upstream_stopping(&self) {
        self.0.upstream_stopping.store(true, Ordering::Release);
    }

    /// Requests that arrived after `upstream_stopping` and found the upstream gone.
    pub fn refused_after_stop(&self) -> u64 {
        self.0.refused_after_stop.load(Ordering::Acquire)
    }

    /// The upstream was gone before any response byte.
    ///
    /// Call this when the request is already in hand. A stop flagged by then
    /// counts a late refusal and is not a failure. A forwarded exchange uses
    /// the same helper with the flag it read when its request arrived.
    pub fn upstream_was_gone(&self) {
        let stopping = self.0.upstream_stopping.load(Ordering::Acquire);
        if !self.note_upstream_gone(stopping) {
            self.failed();
        }
    }

    /// Count a late refusal when `stopping` was set as the request arrived.
    /// Returns whether that refusal is expected.
    fn note_upstream_gone(&self, stopping: bool) -> bool {
        if stopping {
            self.0.refused_after_stop.fetch_add(1, Ordering::AcqRel);
        }
        stopping
    }

    pub fn ensure_success(&self) -> io::Result<()> {
        let failures = self.failures();
        if failures == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "HTTP proxy recorded {failures} failure(s)"
            )))
        }
    }

    fn failed(&self) {
        let mut count = self.0.failures.load(Ordering::Acquire);
        while count < u64::MAX {
            match self.0.failures.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => count = current,
            }
        }
    }

    /// Stop accepting, finish accepted exchanges, and reject any lost evidence.
    /// A bounded timeout is also a failure; callers must then stop the runtime.
    pub async fn finish(&self, wait: Duration) -> io::Result<()> {
        if wait.is_zero() || wait > MAX_FINISH_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "proxy finish timeout must be greater than zero and at most 320 seconds",
            ));
        }
        self.0.stopping.store(true, Ordering::Release);
        self.0.stop.notify_one();
        if timeout(wait, async {
            while !self.0.finished.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_err()
        {
            self.failed();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "HTTP proxy did not finish accepted exchanges",
            ));
        }
        self.ensure_success()
    }
}

struct RunGuard {
    status: ProxyStatus,
    completed: bool,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.status.failed();
        }
        self.status.0.finished.store(true, Ordering::Release);
    }
}

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
    serve_on_with_status(
        listener,
        upstream,
        path,
        reply_timeout,
        ProxyStatus::default(),
    )
    .await
}

/// Serve with retained failure evidence and a graceful, bounded finish handle.
pub async fn serve_on_with_status(
    listener: TcpListener,
    upstream: SocketAddr,
    path: &Path,
    reply_timeout: Duration,
    status: ProxyStatus,
) -> io::Result<()> {
    let mut guard = RunGuard {
        status: status.clone(),
        completed: false,
    };
    let result = serve_inner(listener, upstream, path, reply_timeout, &status).await;
    if result.is_err() && status.failures() == 0 {
        status.failed();
    }
    guard.completed = true;
    result
}

async fn serve_inner(
    listener: TcpListener,
    upstream: SocketAddr,
    path: &Path,
    reply_timeout: Duration,
    status: &ProxyStatus,
) -> io::Result<()> {
    validate_timeout(reply_timeout)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    file.write_all(crate::format::MAGIC)?;
    let capture = Arc::new(Capture::start(file));
    let mut exchanges = JoinSet::new();
    loop {
        if status.0.stopping.load(Ordering::Acquire) {
            break;
        }
        tokio::select! {
            biased;
            result = exchanges.join_next(), if !exchanges.is_empty() => {
                check_exchange(result, status)?;
            }
            _ = status.0.stop.notified() => {}
            accepted = listener.accept(), if exchanges.len() < MAX_EXCHANGES => {
                let (client, _) = accepted?;
                // HttpListener.cs disposes the socket and keeps accepting when
                // setting NoDelay throws. No exchange has started.
                if set_tcp_nodelay(&client).is_err() {
                    continue;
                }
                let capture = Arc::clone(&capture);
                let status = status.clone();
                exchanges.spawn(async move {
                    let result = forward(client, upstream, capture, reply_timeout, &status).await;
                    if result.is_err() { status.failed(); }
                    result
                });
            }
        }
    }
    drop(listener);
    while let Some(result) = exchanges.join_next().await {
        check_exchange(Some(result), status)?;
    }
    status.ensure_success()
}

fn check_exchange(
    result: Option<Result<io::Result<()>, tokio::task::JoinError>>,
    status: &ProxyStatus,
) -> io::Result<()> {
    match result {
        Some(Ok(result)) => result,
        Some(Err(_)) => {
            status.failed();
            Err(io::Error::other(
                "HTTP proxy exchange task did not complete",
            ))
        }
        None => Ok(()),
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

/// Record bytes queued for the writer thread and not yet written. A stalled
/// disk with every exchange in flight would otherwise hold up to
/// `MAX_EXCHANGES` requests and responses of up to `HEADER_CAP` plus
/// `BODY_CAP` each, about 544 MiB. Limit `capture.writer-queue`.
pub const WRITER_QUEUE_BYTES: usize = 64 * 1024 * 1024;

/// Bytes one record takes in the queue: its body plus the capture framing.
fn queued_size(records: &[Record]) -> usize {
    records
        .iter()
        .map(|record| record.bytes.len().saturating_add(16))
        .fold(0usize, usize::saturating_add)
}

enum Job {
    Write(Vec<Record>, usize),
    Flush(tokio::sync::oneshot::Sender<io::Result<()>>),
}

/// One writer thread, in submission order. Callers enqueue without waiting,
/// so a full pipe cannot hold the next POST, up to [`WRITER_QUEUE_BYTES`]
/// queued. Past that the record is refused and the exchange fails. `flushed`
/// waits, without a blocking thread, until the writer has applied every
/// record submitted so far.
struct Capture {
    tx: Mutex<Option<std::sync::mpsc::Sender<Job>>>,
    writer: Mutex<Option<std::thread::JoinHandle<()>>>,
    queued: Arc<AtomicUsize>,
    budget: usize,
}

impl Capture {
    fn start(file: std::fs::File) -> Self {
        Self::start_with(file, WRITER_QUEUE_BYTES)
    }

    fn start_with(file: std::fs::File, budget: usize) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let written = Arc::clone(&queued);
        let writer = std::thread::spawn(move || writer_loop(file, rx, &written));
        Self {
            tx: Mutex::new(Some(tx)),
            writer: Mutex::new(Some(writer)),
            queued,
            budget,
        }
    }

    fn submit(&self, records: Vec<Record>) -> io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let size = queued_size(&records);
        let reserved = self
            .queued
            .try_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                queued
                    .checked_add(size)
                    .filter(|total| *total <= self.budget)
            });
        if let Err(queued) = reserved {
            let observed = queued.saturating_add(size);
            ferroan_diag::refuse(
                ferroan_diag::CAPTURE_WRITER_QUEUE,
                ferroan_diag::Kind::Hard,
                u64::try_from(self.budget).unwrap_or(u64::MAX),
                u64::try_from(observed).unwrap_or(u64::MAX),
                "capture proxy",
            );
            return Err(io::Error::other(format!(
                "capture writer queue is full ({observed} bytes past {}, limit {})",
                self.budget,
                ferroan_diag::CAPTURE_WRITER_QUEUE
            )));
        }
        let guard = self.tx.lock().unwrap_or_else(|poison| poison.into_inner());
        let sent = guard
            .as_ref()
            .map(|tx| tx.send(Job::Write(records, size)).is_ok())
            .unwrap_or(false);
        if !sent {
            self.queued.fetch_sub(size, Ordering::AcqRel);
            return Err(io::Error::other("capture writer stopped"));
        }
        Ok(())
    }

    async fn flushed(&self) -> io::Result<()> {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        {
            let guard = self.tx.lock().unwrap_or_else(|poison| poison.into_inner());
            let tx = guard
                .as_ref()
                .ok_or_else(|| io::Error::other("capture writer stopped"))?;
            tx.send(Job::Flush(ack_tx))
                .map_err(|_| io::Error::other("capture writer stopped"))?;
        }
        ack_rx
            .await
            .unwrap_or_else(|_| Err(io::Error::other("capture writer stopped")))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.tx
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        if let Some(writer) = self
            .writer
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            let _ = writer.join();
        }
    }
}

fn writer_loop(mut file: std::fs::File, rx: std::sync::mpsc::Receiver<Job>, queued: &AtomicUsize) {
    let mut error: Option<String> = None;
    while let Ok(job) = rx.recv() {
        match job {
            Job::Write(records, size) => {
                if error.is_none()
                    && let Err(err) = append_records(&mut file, &records)
                {
                    error = Some(err.to_string());
                }
                drop(records);
                queued.fetch_sub(size, Ordering::AcqRel);
            }
            Job::Flush(ack) => {
                let result = match &error {
                    Some(message) => Err(io::Error::other(message.clone())),
                    None => Ok(()),
                };
                let _ = ack.send(result);
            }
        }
    }
}

fn append_records(file: &mut std::fs::File, records: &[Record]) -> io::Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    let encoded = write_capture(records).map_err(io::Error::other)?;
    file.write_all(&encoded[crate::format::MAGIC.len()..])?;
    file.flush()
}

async fn forward(
    mut client: TcpStream,
    upstream: SocketAddr,
    capture: Arc<Capture>,
    reply_timeout: Duration,
    status: &ProxyStatus,
) -> io::Result<()> {
    let Some(request) = read_message_or_unused(&mut client, true, IO_WAIT).await? else {
        // A connection pool may preconnect and close without sending HTTP.
        // No exchange began, so neither a capture nor a failure is warranted.
        return Ok(());
    };
    // Read when the request is in hand: only a request that arrives after the
    // owner began stopping the upstream may meet a refusal without failing.
    let upstream_stopping = status.0.upstream_stopping.load(Ordering::Acquire);
    // Record the bytes the upstream server reads. C# copies the request's
    // Connection onto the response, so the fixture has to show this rewrite.
    // The stamp is this moment, before the upstream wait, so a held poll still
    // shows its wait in the capture times. The write itself is the side
    // thread: waiting for it here is what made getgriduserinfo miss its second.
    let request_ms = unix_ms();
    let forwarded = close_connection(&request)?;
    capture.submit(vec![Record {
        kind: Kind::HttpRequest,
        time_ms: request_ms,
        bytes: forwarded.clone(),
    }])?;
    let response = match exchange(&mut client, upstream, &forwarded, reply_timeout).await {
        Ok(Reply::Response(response)) => response,
        Ok(Reply::Gone(error)) => {
            if status.note_upstream_gone(upstream_stopping) {
                // The upstream was shut down on purpose before this request came.
                // Keep the request as evidence; the client sees a closed socket.
                capture.flushed().await?;
                return Ok(());
            }
            // The upstream is meant to be up: a failed exchange.
            let _ = capture.flushed().await;
            return Err(error);
        }
        Ok(Reply::ClientLeft) => {
            // The client reset after the request was queued. Leave that
            // request in the capture and keep the listener up.
            capture.flushed().await?;
            return Ok(());
        }
        Err(error) => {
            // The client was still waiting. The request is already queued,
            // with no response, as the failed-exchange tests require.
            let _ = capture.flushed().await;
            return Err(error);
        }
    };
    let response_ms = unix_ms();
    let to_client = close_connection(&response)?;
    // Queue the response before the client can send the next poll. Ferrobot
    // polls again as soon as it has this reply; a later append let that poll
    // land between this request and this response.
    capture.submit(vec![Record {
        kind: Kind::HttpResponse,
        time_ms: response_ms,
        bytes: response,
    }])?;
    if let Err(error) = write_message(&mut client, &to_client).await {
        capture.flushed().await?;
        if client_went_away(&error) {
            return Ok(());
        }
        return Err(error);
    }
    // The client has the response. Waiting for the disk is after that.
    capture.flushed().await?;
    Ok(())
}

/// How one forwarded exchange ended, short of an error.
enum Reply {
    Response(Vec<u8>),
    /// The client went away while the upstream held the reply.
    ClientLeft,
    /// The upstream is gone before any response byte: the connect was refused,
    /// or the connection was reset, closed or broken.
    Gone(io::Error),
}

/// An upstream error that means the server went away, not a bad message.
fn upstream_went_away(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe
    )
}

async fn exchange(
    client: &mut TcpStream,
    upstream: SocketAddr,
    forwarded: &[u8],
    reply_timeout: Duration,
) -> io::Result<Reply> {
    let connected = timeout(IO_WAIT, TcpStream::connect(upstream))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "upstream connect timed out"))?;
    let mut server = match connected {
        Ok(server) => server,
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
            return Ok(Reply::Gone(error));
        }
        Err(error) => return Err(error),
    };
    // The socket exists only after connect. C# sets NoDelay before ConnectAsync;
    // the following writes see the same option.
    set_tcp_nodelay(&server)?;
    if let Err(error) = write_message(&mut server, forwarded).await {
        return if upstream_went_away(&error) {
            Ok(Reply::Gone(error))
        } else {
            Err(error)
        };
    }
    let read = async {
        let mut progress = ReadProgress::new();
        let result = read_with_progress(&mut server, false, reply_timeout, &mut progress).await;
        // Nothing of the response has arrived, not even an interim one.
        let nothing = progress.stage == "headers" && progress.received == 0;
        (result, nothing)
    };
    tokio::select! {
        biased;
        (result, nothing) = read => match result {
            Ok(Some(response)) => Ok(Reply::Response(response)),
            Ok(None) => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no HTTP response was sent")),
            Err(error) if nothing && upstream_went_away(&error) => Ok(Reply::Gone(error)),
            Err(error) => Err(error),
        },
        _ = client_left(client) => Ok(Reply::ClientLeft),
    }
}

/// The request is already consumed. A reset drops the upstream socket with
/// this task. A clean end of the request (the client shut down its write
/// side and is still reading) must not: that is how an event-queue poll ends
/// its request, and treating it as a departure recorded no reply.
async fn client_left(client: &mut TcpStream) {
    let mut buf = [0u8; 1];
    loop {
        match client.read(&mut buf).await {
            Err(_) => return,
            Ok(0) => std::future::pending::<()>().await,
            Ok(_) => continue,
        }
    }
}

fn client_went_away(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::NotConnected
    )
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

#[cfg(test)]
async fn read_message(
    stream: &mut TcpStream,
    request: bool,
    wait: Duration,
) -> io::Result<Vec<u8>> {
    read_message_or_unused(stream, request, wait)
        .await?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no HTTP request was sent"))
}

struct ReadProgress {
    stage: &'static str,
    received: usize,
    expected: Option<usize>,
}

impl ReadProgress {
    fn new() -> Self {
        Self {
            stage: "headers",
            received: 0,
            expected: None,
        }
    }
}

async fn read_message_or_unused(
    stream: &mut TcpStream,
    request: bool,
    wait: Duration,
) -> io::Result<Option<Vec<u8>>> {
    read_with_progress(stream, request, wait, &mut ReadProgress::new()).await
}

/// Read one message; `progress` keeps how far it got, including on error.
/// `received` counts every byte read, interim responses included.
async fn read_with_progress(
    stream: &mut TcpStream,
    request: bool,
    wait: Duration,
    progress: &mut ReadProgress,
) -> io::Result<Option<Vec<u8>>> {
    let result = timeout(wait, read_message_inner(stream, request, progress))
        .await
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "HTTP message timed out",
            ))
        });
    result.map_err(|error| {
        let direction = if request { "request" } else { "response" };
        let expected = progress
            .expected
            .map(|count| format!(", expected {count}"))
            .unwrap_or_default();
        io::Error::new(
            error.kind(),
            format!(
                "HTTP {direction} {}: {error} (received {} bytes{expected})",
                progress.stage, progress.received
            ),
        )
    })
}

async fn read_message_inner(
    stream: &mut TcpStream,
    request: bool,
    progress: &mut ReadProgress,
) -> io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut interim = 0;
    let head_end = loop {
        if let Some(at) = find_bytes(&buf, b"\r\n\r\n") {
            if at + 4 > HEADER_CAP {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "headers are too large",
                ));
            }
            if !request && (100..200).contains(&response_status(&buf[..at])?) {
                interim += 1;
                if interim > MAX_INTERIM_RESPONSES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "too many interim HTTP responses",
                    ));
                }
                // Interim responses have no body. Keep any bytes already read
                // from the final response, including when both arrived at once.
                buf.drain(..at + 4);
                continue;
            }
            break at + 4;
        }
        if buf.len() > HEADER_CAP {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "headers are too large",
            ));
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            if request && buf.is_empty() {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
        progress.received += n;
    };
    if header_has(&buf[..head_end], "transfer-encoding:") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunked bodies are not captured",
        ));
    }
    let body_len = content_length(&buf[..head_end]);
    progress.stage = "body";
    progress.received = buf.len() - head_end;
    progress.expected = body_len;
    match body_len {
        Some(len) if len > BODY_CAP => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "body is too large",
        )),
        Some(len) => {
            while buf.len() < head_end + len {
                let n = read_some(stream, &mut tmp).await?;
                buf.extend_from_slice(&tmp[..n]);
                progress.received += n;
                if buf.len() > head_end + BODY_CAP {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "body is too large",
                    ));
                }
            }
            buf.truncate(head_end + len);
            Ok(Some(buf))
        }
        None if request => Ok(Some(buf[..head_end].to_vec())),
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
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        progress.received += n;
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(Some(buf))
        }
    }
}

fn response_status(head: &[u8]) -> io::Result<u16> {
    let line = head.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let mut fields = line.split(|byte| *byte == b' ');
    let version = fields.next().unwrap_or_default();
    let code = fields.next().unwrap_or_default();
    if !matches!(version, b"HTTP/1.0" | b"HTTP/1.1")
        || code.len() != 3
        || !code.iter().all(u8::is_ascii_digit)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP response status",
        ));
    }
    Ok(
        u16::from(code[0] - b'0') * 100
            + u16::from(code[1] - b'0') * 10
            + u16::from(code[2] - b'0'),
    )
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
        (Arc::new(Capture::start(file)), path)
    }

    #[tokio::test]
    async fn production_default_allows_a_held_reply() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/caps/manifest.json")).unwrap();
        let captured_hold = Duration::from_millis(
            fixture["exchanges"]["empty-poll"]["elapsed_ms"]
                .as_u64()
                .unwrap(),
        );
        assert!(
            DEFAULT_REPLY_TIMEOUT > captured_hold,
            "production proxy deadline must exceed the captured C# hold ({captured_hold:?})"
        );
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
        let (_, path) = capture_file();
        let capture_path = path.clone();
        let forwarding =
            tokio::spawn(async move { serve_on(listener, upstream_addr, &capture_path).await });
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
        server.await.unwrap();
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[1].bytes.starts_with(b"HTTP/1.1 502 Bad Gateway"));
        assert!(!forwarding.is_finished());
        forwarding.abort();
        assert!(forwarding.await.unwrap_err().is_cancelled());
        std::fs::remove_file(path).unwrap();
    }

    async fn tracked_proxy(
        upstream: SocketAddr,
        wait: Duration,
    ) -> (
        SocketAddr,
        ProxyStatus,
        tokio::task::JoinHandle<io::Result<()>>,
        std::path::PathBuf,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (_, path) = capture_file();
        let status = ProxyStatus::default();
        let child_status = status.clone();
        let child_path = path.clone();
        let task = tokio::spawn(async move {
            serve_on_with_status(listener, upstream, &child_path, wait, child_status).await
        });
        (address, status, task, path)
    }

    async fn request(address: SocketAddr) -> Vec<u8> {
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"GET /test HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        timeout(IO_WAIT, client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        response
    }

    #[tokio::test]
    async fn an_unused_connection_is_not_an_exchange_and_later_requests_work() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let request = read_message(&mut socket, true, IO_WAIT).await.unwrap();
            assert!(request.starts_with(b"GET /test "));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let (address, status, task, path) =
            tracked_proxy(upstream_addr, DEFAULT_REPLY_TIMEOUT).await;
        let mut unused = TcpStream::connect(address).await.unwrap();
        unused.shutdown().await.unwrap();
        let mut bytes = Vec::new();
        timeout(IO_WAIT, unused.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert!(bytes.is_empty());
        assert!(request(address).await.starts_with(b"HTTP/1.1 200 OK"));
        status.finish(IO_WAIT).await.unwrap();
        task.await.unwrap().unwrap();
        server.await.unwrap();
        assert_eq!(status.failures(), 0);
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            records.len(),
            2,
            "unused socket must not create capture records"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn partial_request_eof_is_a_failure_with_stage_and_size_only() {
        for (bytes, stage) in [
            (
                b"GET /private".as_slice(),
                "headers: connection closed (received 12 bytes)",
            ),
            (
                b"POST /private HTTP/1.1\r\nContent-Length: 8\r\n\r\npart".as_slice(),
                "body: connection closed (received 4 bytes, expected 8)",
            ),
        ] {
            let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let (address, status, task, path) =
                tracked_proxy(upstream.local_addr().unwrap(), DEFAULT_REPLY_TIMEOUT).await;
            let mut client = TcpStream::connect(address).await.unwrap();
            client.write_all(bytes).await.unwrap();
            client.shutdown().await.unwrap();
            let mut response = Vec::new();
            timeout(IO_WAIT, client.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
            let error = task.await.unwrap().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
            assert_eq!(error.to_string(), format!("HTTP request {stage}"));
            assert!(!error.to_string().contains("private"));
            assert_eq!(status.failures(), 1);
            assert!(status.finish(IO_WAIT).await.is_err());
            assert!(
                read_capture(&std::fs::read(&path).unwrap())
                    .unwrap()
                    .is_empty()
            );
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test]
    async fn an_idle_socket_without_eof_still_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        let error = read_message_or_unused(&mut socket, true, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(
            error.to_string(),
            "HTTP request headers: HTTP message timed out (received 0 bytes)"
        );
    }

    #[tokio::test]
    async fn interim_responses_keep_the_final_response_in_the_same_or_later_read() {
        for split in [false, true] {
            let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream_addr = upstream.local_addr().unwrap();
            let final_response = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\npong";
            let server = tokio::spawn(async move {
                let (mut socket, _) = upstream.accept().await.unwrap();
                read_message(&mut socket, true, IO_WAIT).await.unwrap();
                let interim = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 103 Early Hints\r\nLink: </style.css>\r\n\r\n";
                if split {
                    socket.write_all(interim).await.unwrap();
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    socket.write_all(final_response).await.unwrap();
                } else {
                    socket
                        .write_all(&[interim.as_slice(), final_response.as_slice()].concat())
                        .await
                        .unwrap();
                }
            });
            let (address, status, task, path) =
                tracked_proxy(upstream_addr, DEFAULT_REPLY_TIMEOUT).await;
            let response = request(address).await;
            assert!(response.starts_with(b"HTTP/1.1 200 OK"));
            assert!(response.ends_with(b"pong"));
            status.finish(IO_WAIT).await.unwrap();
            task.await.unwrap().unwrap();
            server.await.unwrap();
            assert_eq!(status.failures(), 0);
            let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(records.len(), 2);
            assert_eq!(records[1].bytes, final_response);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test]
    async fn failed_exchanges_are_counted_and_fail_the_serving_task_and_finish() {
        for (reply, stall, kind) in [
            (Vec::new(), true, io::ErrorKind::TimedOut),
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".to_vec(),
                false,
                io::ErrorKind::InvalidData,
            ),
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    BODY_CAP + 1
                )
                .into_bytes(),
                false,
                io::ErrorKind::InvalidData,
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\npart".to_vec(),
                false,
                io::ErrorKind::UnexpectedEof,
            ),
            (
                b"HTTP/1.1 100 Continue\r\n\r\n".to_vec(),
                false,
                io::ErrorKind::UnexpectedEof,
            ),
            (
                b"HTTP/1.1 103 Early Hints\r\n\r\n".repeat(MAX_INTERIM_RESPONSES + 1),
                false,
                io::ErrorKind::InvalidData,
            ),
        ] {
            let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream_addr = upstream.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = upstream.accept().await.unwrap();
                read_message(&mut socket, true, IO_WAIT).await.unwrap();
                socket.write_all(&reply).await.unwrap();
                if stall {
                    let _ = socket.read(&mut [0]).await;
                }
            });
            let (address, status, task, path) =
                tracked_proxy(upstream_addr, Duration::from_millis(100)).await;
            assert!(request(address).await.is_empty());
            assert_eq!(task.await.unwrap().unwrap_err().kind(), kind);
            assert_eq!(status.failures(), 1);
            assert!(status.ensure_success().is_err());
            assert!(status.finish(IO_WAIT).await.is_err());
            server.await.unwrap();
            let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].kind, Kind::HttpRequest);
            std::fs::remove_file(path).unwrap();
        }
    }

    async fn closed_port() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        address
    }

    #[tokio::test]
    async fn a_refused_upstream_that_is_meant_to_be_up_fails_the_proxy() {
        let (address, status, task, path) =
            tracked_proxy(closed_port().await, Duration::from_secs(1)).await;
        assert!(request(address).await.is_empty());
        assert_eq!(
            timeout(IO_WAIT, task)
                .await
                .expect("proxy did not stop")
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionRefused
        );
        assert_eq!(status.failures(), 1);
        assert_eq!(status.refused_after_stop(), 0);
        assert!(status.finish(IO_WAIT).await.is_err());
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, Kind::HttpRequest);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn a_request_after_the_upstream_was_stopped_is_not_a_failure() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let (address, status, task, path) =
            tracked_proxy(upstream_addr, Duration::from_secs(1)).await;
        // The owner begins stopping the upstream; it still answers until it is gone.
        status.upstream_stopping();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            read_message(&mut socket, true, IO_WAIT).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();
        });
        assert!(request(address).await.starts_with(b"HTTP/1.1 200 OK"));
        server.await.unwrap();
        // Now nothing listens there: the late request (C#'s CloseAgent from a
        // neighbour still running) is refused and kept, and the proxy goes on.
        assert!(request(address).await.is_empty());
        assert!(request(address).await.is_empty());
        assert!(!task.is_finished());
        assert_eq!(status.failures(), 0);
        assert_eq!(status.refused_after_stop(), 2);
        status.finish(IO_WAIT).await.unwrap();
        task.await.unwrap().unwrap();
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        let kinds: Vec<Kind> = records.iter().map(|record| record.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::HttpRequest,
                Kind::HttpResponse,
                Kind::HttpRequest,
                Kind::HttpRequest
            ]
        );
        std::fs::remove_file(path).unwrap();
    }

    /// An upstream that accepts, reads the request and closes without a byte.
    async fn closing_upstream() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = upstream.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            read_message(&mut socket, true, IO_WAIT).await.unwrap();
        });
        (address, server)
    }

    #[tokio::test]
    async fn an_upstream_closing_before_a_response_fails_only_while_it_is_meant_to_be_up() {
        for stopping in [false, true] {
            let (upstream, server) = closing_upstream().await;
            let (address, status, task, path) =
                tracked_proxy(upstream, Duration::from_secs(1)).await;
            if stopping {
                status.upstream_stopping();
            }
            assert!(request(address).await.is_empty());
            server.await.unwrap();
            if stopping {
                assert_eq!(status.failures(), 0);
                assert_eq!(status.refused_after_stop(), 1);
                status.finish(IO_WAIT).await.unwrap();
                timeout(IO_WAIT, task)
                    .await
                    .expect("proxy did not stop")
                    .unwrap()
                    .unwrap();
            } else {
                assert_eq!(
                    timeout(IO_WAIT, task)
                        .await
                        .expect("proxy did not stop")
                        .unwrap()
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::UnexpectedEof
                );
                assert_eq!(status.failures(), 1);
                assert_eq!(status.refused_after_stop(), 0);
            }
            let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].kind, Kind::HttpRequest);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test]
    async fn a_partial_response_after_the_mark_still_fails() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            read_message(&mut socket, true, IO_WAIT).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\n").await.unwrap();
        });
        let (address, status, task, path) =
            tracked_proxy(upstream_addr, Duration::from_secs(1)).await;
        status.upstream_stopping();
        assert!(request(address).await.is_empty());
        server.await.unwrap();
        assert_eq!(
            timeout(IO_WAIT, task)
                .await
                .expect("proxy did not stop")
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(status.failures(), 1);
        assert_eq!(status.refused_after_stop(), 0);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn finish_waits_for_in_flight_evidence() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            read_message(&mut socket, true, IO_WAIT).await.unwrap();
            seen_tx.send(()).unwrap();
            release_rx.await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let (address, status, task, path) =
            tracked_proxy(upstream_addr, DEFAULT_REPLY_TIMEOUT).await;
        let client = tokio::spawn(request(address));
        seen_rx.await.unwrap();
        let finishing_status = status.clone();
        let finish = tokio::spawn(async move { finishing_status.finish(IO_WAIT).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!finish.is_finished());
        release_tx.send(()).unwrap();
        assert!(client.await.unwrap().starts_with(b"HTTP/1.1 200 OK"));
        finish.await.unwrap().unwrap();
        task.await.unwrap().unwrap();
        server.await.unwrap();
        assert_eq!(status.failures(), 0);
        assert_eq!(
            read_capture(&std::fs::read(&path).unwrap()).unwrap().len(),
            2
        );
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn fatal_output_errors_and_finish_timeouts_remain_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = listener.local_addr().unwrap();
        let status = ProxyStatus::default();
        assert!(
            serve_on_with_status(
                listener,
                upstream,
                Path::new(env!("CARGO_MANIFEST_DIR")),
                DEFAULT_REPLY_TIMEOUT,
                status.clone()
            )
            .await
            .is_err()
        );
        assert_eq!(status.failures(), 1);
        assert!(status.finish(IO_WAIT).await.is_err());
        let not_running = ProxyStatus::default();
        assert_eq!(
            not_running
                .finish(Duration::from_millis(20))
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(not_running.failures(), 1);
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
                forward(
                    client,
                    upstream_addr,
                    capture,
                    Duration::from_millis(100),
                    &ProxyStatus::default(),
                )
                .await
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
                forward(
                    client,
                    upstream_addr,
                    capture.clone(),
                    IO_WAIT,
                    &ProxyStatus::default(),
                )
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
        // Requests are the forwarded bytes (Connection: close). Responses stay
        // the upstream message, before the client-facing rewrite.
        assert!(records.iter().enumerate().all(|(index, record)| {
            if index % 2 == 0 {
                header_has(&record.bytes, "connection: close")
                    && !header_has(&record.bytes, "keep-alive")
            } else {
                header_has(&record.bytes, "connection: keep-alive")
            }
        }));
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
        let mut response = Vec::new();
        timeout(IO_WAIT, client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.windows(4).any(|window| window == b"pong"));

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

    /// The logout probe names `griduser` when every attempt through this proxy
    /// stays open for a full second. A capture write that blocks in the kernel
    /// (the failing run's disk did not finish a terrain store for about ten
    /// seconds) used to run before the response was sent, so none of those
    /// attempts reached the client.
    #[tokio::test]
    #[cfg(unix)]
    async fn a_blocked_capture_write_does_not_hold_the_next_response() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.scratch/proxy-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("block-{}.fifo", uuid::Uuid::new_v4()));
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let fifo = path.clone();
        let (file_tx, file_rx) = std::sync::mpsc::channel();
        let opener = std::thread::spawn(move || {
            let file = std::fs::File::open(fifo).unwrap();
            file_tx.send(file).unwrap();
        });
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let body = vec![b'x'; 96 * 1024];
        let response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = upstream.accept().await.unwrap();
                read_message(&mut socket, true, IO_WAIT).await.unwrap();
                socket.write_all(&response).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let capture_path = path.clone();
        let forwarding = tokio::spawn(async move {
            let _ = serve_on(listener, upstream_addr, &capture_path).await;
        });
        let reader = tokio::task::spawn_blocking(move || {
            file_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("capture fifo reader")
        })
        .await
        .unwrap();
        let request = b"POST /griduser HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n";
        let started = tokio::time::Instant::now();
        read_bounded_response(address, request).await;
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the first response waited on the capture write ({:?})",
            started.elapsed()
        );
        // The first exchange is now blocked in the capture write. The next
        // POST has to return while that write still holds the pipe.
        let started = tokio::time::Instant::now();
        read_bounded_response(address, request).await;
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the second response waited on the capture write ({:?})",
            started.elapsed()
        );
        drop(reader);
        opener.join().unwrap();
        forwarding.abort();
        let _ = forwarding.await;
        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_file(&path);
    }

    /// A writer stuck on a full pipe holds what was queued, up to the budget.
    /// The next record past it is refused (`capture.writer-queue`), and the
    /// queue takes records again once the pipe drains.
    #[tokio::test]
    #[cfg(unix)]
    async fn a_stalled_writer_queue_refuses_past_its_budget() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.scratch/proxy-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("queue-{}.fifo", uuid::Uuid::new_v4()));
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let fifo = path.clone();
        let opener = std::thread::spawn(move || std::fs::File::open(fifo).unwrap());
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let reader = opener.join().unwrap();
        let capture = Capture::start_with(file, 256 * 1024);
        let record = || {
            vec![Record {
                kind: Kind::HttpRequest,
                time_ms: 1,
                bytes: vec![b'q'; 100 * 1024],
            }]
        };
        // The first record fills the 64 KiB pipe and the writer blocks on it.
        // Results are taken before any assert: the drain thread must start
        // first, or a failed assert would leave the writer (and Drop) stuck.
        let first = capture.submit(record());
        let second = capture.submit(record());
        let third = capture.submit(record());
        let drained = std::thread::spawn(move || {
            let mut reader = reader;
            let mut got = Vec::new();
            std::io::Read::read_to_end(&mut reader, &mut got).unwrap();
            got
        });
        first.unwrap();
        second.unwrap();
        let error = third.unwrap_err();
        assert!(
            error.to_string().contains("capture.writer-queue"),
            "{error}"
        );
        timeout(Duration::from_secs(5), capture.flushed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(capture.queued.load(Ordering::Acquire), 0);
        capture.submit(record()).unwrap();
        timeout(Duration::from_secs(5), capture.flushed())
            .await
            .unwrap()
            .unwrap();
        drop(capture);
        let bytes = drained.join().unwrap();
        let _ = std::fs::remove_file(&path);
        let mut framed = crate::format::MAGIC.to_vec();
        framed.extend_from_slice(&bytes);
        assert_eq!(read_capture(&framed).unwrap().len(), 3);
    }

    /// Ferrobot polls again as soon as it has the reply. Recording the request
    /// and the response as two later appends let the next poll land between
    /// them, and the event-queue check then saw a poll with no reply.
    #[tokio::test]
    #[cfg(unix)]
    async fn an_overlapping_post_cannot_split_a_captured_exchange() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.scratch/proxy-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("pair-{}.fifo", uuid::Uuid::new_v4()));
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let fifo = path.clone();
        let (file_tx, file_rx) = std::sync::mpsc::channel();
        let opener = std::thread::spawn(move || {
            let file = std::fs::File::open(fifo).unwrap();
            file_tx.send(file).unwrap();
        });
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let body = vec![b'y'; 96 * 1024];
        let response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = upstream.accept().await.unwrap();
                read_message(&mut socket, true, IO_WAIT).await.unwrap();
                socket.write_all(&response).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let status = ProxyStatus::default();
        let child_status = status.clone();
        let capture_path = path.clone();
        let forwarding = tokio::spawn(async move {
            serve_on_with_status(
                listener,
                upstream_addr,
                &capture_path,
                DEFAULT_REPLY_TIMEOUT,
                child_status,
            )
            .await
        });
        let reader = tokio::task::spawn_blocking(move || {
            file_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("capture fifo reader")
        })
        .await
        .unwrap();
        read_bounded_response(
            address,
            b"POST /poll-a HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
        )
        .await;
        read_bounded_response(
            address,
            b"POST /poll-b HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
        )
        .await;
        let drained = tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut got = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = std::io::Read::read(&mut reader, &mut buf).unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            got
        });
        status.finish(Duration::from_secs(5)).await.unwrap();
        forwarding.await.unwrap().unwrap();
        server.abort();
        let _ = server.await;
        opener.join().unwrap();
        let bytes = drained.await.unwrap();
        let _ = std::fs::remove_file(&path);
        let records = read_capture(&bytes).unwrap();
        assert_eq!(
            records.len(),
            4,
            "each exchange is one request and one response"
        );
        assert!(
            records.chunks(2).all(|pair| {
                pair[0].kind == Kind::HttpRequest
                    && pair[1].kind == Kind::HttpResponse
                    && pair[1].time_ms >= pair[0].time_ms
            }),
            "a later poll was written between a request and its response"
        );
        let first = std::str::from_utf8(&records[0].bytes).unwrap();
        let second = std::str::from_utf8(&records[2].bytes).unwrap();
        assert!(first.contains("POST /poll-a "), "{first}");
        assert!(second.contains("POST /poll-b "), "{second}");
    }

    #[cfg(unix)]
    async fn read_bounded_response(address: SocketAddr, request: &[u8]) {
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(request).await.unwrap();
        let mut buf = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        let head_end = loop {
            if tokio::time::Instant::now() > deadline {
                panic!("response headers did not arrive within one second");
            }
            if let Some(at) = find_bytes(&buf, b"\r\n\r\n") {
                break at + 4;
            }
            let mut tmp = [0u8; 4096];
            let n = timeout(Duration::from_secs(1), client.read(&mut tmp))
                .await
                .expect("response timed out")
                .unwrap();
            assert!(n > 0, "proxy closed before the response");
            buf.extend_from_slice(&tmp[..n]);
        };
        let len = content_length(&buf[..head_end]).expect("content-length");
        while buf.len() < head_end + len {
            if tokio::time::Instant::now() > deadline {
                panic!(
                    "response body stalled at {} of {len} bytes",
                    buf.len() - head_end
                );
            }
            let mut tmp = [0u8; 8192];
            let n = timeout(Duration::from_secs(1), client.read(&mut tmp))
                .await
                .expect("body timed out")
                .unwrap();
            assert!(n > 0, "proxy closed before the body");
            buf.extend_from_slice(&tmp[..n]);
        }
    }

    /// A quiet event queue holds a poll for about 58 seconds, so a 120-second
    /// session finishes two polls and is still inside the third. The capture
    /// check counts requests. That third request has to be in the file before
    /// its response, or the check reports too few polls.
    #[tokio::test]
    async fn a_held_poll_is_captured_before_its_response() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            read_message(&mut socket, true, IO_WAIT).await.unwrap();
            seen_tx.send(()).unwrap();
            release_rx.await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: 4\r\n\r\npong")
                .await
                .unwrap();
        });
        let (address, status, task, path) =
            tracked_proxy(upstream_addr, DEFAULT_REPLY_TIMEOUT).await;
        let client = tokio::spawn(async move {
            let mut socket = TcpStream::connect(address).await.unwrap();
            socket
                .write_all(
                    b"POST /CE/held HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .unwrap();
            let mut response = Vec::new();
            timeout(Duration::from_secs(2), socket.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
            response
        });
        seen_rx.await.unwrap();
        let path_wait = path.clone();
        let recorded = timeout(Duration::from_secs(2), async move {
            loop {
                if let Ok(bytes) = std::fs::read(&path_wait)
                    && let Ok(records) = read_capture(&bytes)
                    && records.len() == 1
                    && records[0].kind == Kind::HttpRequest
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            recorded.is_ok(),
            "held poll request was not captured before the response"
        );
        release_tx.send(()).unwrap();
        let response = client.await.unwrap();
        assert!(response.windows(4).any(|window| window == b"pong"));
        status.finish(IO_WAIT).await.unwrap();
        task.await.unwrap().unwrap();
        server.await.unwrap();
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].kind, Kind::HttpResponse);
        std::fs::remove_file(path).unwrap();
    }

    /// An event-queue client shuts down its write side after the poll and
    /// keeps reading. Treating that end as a departure dropped the upstream
    /// socket, so the capture kept too few polls.
    #[tokio::test]
    async fn a_half_closed_request_still_receives_the_response() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            read_message(&mut socket, true, IO_WAIT).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: 4\r\n\r\npong")
                .await
                .unwrap();
        });
        let (address, status, task, path) =
            tracked_proxy(upstream_addr, DEFAULT_REPLY_TIMEOUT).await;
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"POST /CE/poll HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        client.shutdown().await.unwrap();
        let mut response = Vec::new();
        timeout(Duration::from_secs(2), client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.windows(4).any(|window| window == b"pong"));
        status.finish(IO_WAIT).await.unwrap();
        task.await.unwrap().unwrap();
        server.await.unwrap();
        assert_eq!(status.failures(), 0);
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].kind, Kind::HttpRequest);
        assert_eq!(records[1].kind, Kind::HttpResponse);
        assert!(records[1].time_ms >= records[0].time_ms);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn accepted_socket_reads_tcp_nodelay() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            set_tcp_nodelay(&stream).unwrap();
            let _ = tx.send(stream.nodelay().unwrap());
        });
        let _client = TcpStream::connect(addr).await.unwrap();
        assert!(rx.await.unwrap());
    }
}
