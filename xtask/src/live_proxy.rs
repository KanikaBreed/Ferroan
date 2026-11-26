// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Owned capture proxies: drain exchanges and reject any failed exchange.
#![cfg(target_os = "linux")]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use ferroan_capture::{DEFAULT_REPLY_TIMEOUT, ProxyStatus};

pub(crate) struct Proxies {
    runtime: tokio::runtime::Runtime,
    /// One per bind, in the order `start` was given them, with its capture.
    statuses: Vec<(ProxyStatus, PathBuf)>,
}

impl Proxies {
    /// Start and join the grid's UDP recorder on the same owned runtime.
    pub(crate) fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }

    pub(crate) fn start(
        binds: &[(SocketAddr, SocketAddr, PathBuf)],
    ) -> Result<(Vec<SocketAddr>, Self), String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("could not start proxy runtime: {error}"))?;
        let (addresses, statuses) = runtime.block_on(async {
            let mut addresses = Vec::new();
            let mut statuses = Vec::new();
            for (listen, upstream, path) in binds {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| format!("could not create proxy directory: {error}"))?;
                }
                let listener = tokio::net::TcpListener::bind(listen)
                    .await
                    .map_err(|error| format!("could not bind proxy on {listen}: {error}"))?;
                addresses.push(listener.local_addr().map_err(|error| error.to_string())?);
                let status = ProxyStatus::default();
                statuses.push((status.clone(), path.clone()));
                let upstream = *upstream;
                let path = path.clone();
                tokio::spawn(async move {
                    // The shared status preserves errors for finish(), even if
                    // a scenario succeeds after an exchange failed.
                    if let Err(error) = ferroan_capture::serve_on_with_status(
                        listener,
                        upstream,
                        &path,
                        DEFAULT_REPLY_TIMEOUT,
                        status,
                    )
                    .await
                    {
                        eprintln!("live: proxy {} stopped: {error}", path.display());
                    }
                });
            }
            Ok::<_, String>((addresses, statuses))
        })?;
        Ok((addresses, Self { runtime, statuses }))
    }

    /// The harness is about to stop the upstream of bind `index` on purpose.
    ///
    /// A request that reaches that proxy afterwards and finds the upstream gone
    /// (refused, or reset, closed or broken before any response byte) is kept
    /// in its capture and is not a failure. In the grid, the C#
    /// neighbour closes its child agent on the home region after the bot logs
    /// out of the neighbour, on its own thread; that DELETE can come after the
    /// home region has been stopped (PR #249's run 45546454682). A refusal
    /// before this call still fails the run.
    pub(crate) fn upstream_stopping(&self, index: usize) {
        if let Some((status, _)) = self.statuses.get(index) {
            status.upstream_stopping();
        }
    }

    /// The status `stop_in_order` marks for bind `index`.
    #[cfg(test)]
    pub(crate) fn status(&self, index: usize) -> Option<ProxyStatus> {
        self.statuses.get(index).map(|(status, _)| status.clone())
    }

    /// Serve on listeners that are already bound, one closed upstream each.
    ///
    /// Does not bind port 0. [`listeners_and_closed`] has already chosen the
    /// upstreams, and a new bind of port 0 could be given one of them.
    #[cfg(test)]
    pub(crate) fn start_bound(
        listeners: Vec<std::net::TcpListener>,
        upstreams: &[SocketAddr],
        paths: &[PathBuf],
    ) -> Result<(Vec<SocketAddr>, Self), String> {
        if listeners.len() != upstreams.len() || listeners.len() != paths.len() {
            return Err("proxy listeners, upstreams and paths differ in length".to_string());
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("could not start proxy runtime: {error}"))?;
        let upstreams = upstreams.to_vec();
        let paths = paths.to_vec();
        let (addresses, statuses) = runtime.block_on(async move {
            let mut addresses = Vec::new();
            let mut statuses = Vec::new();
            for ((listener, upstream), path) in listeners.into_iter().zip(upstreams).zip(paths) {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| format!("could not create proxy directory: {error}"))?;
                }
                listener
                    .set_nonblocking(true)
                    .map_err(|error| format!("could not set proxy nonblocking: {error}"))?;
                let listener = tokio::net::TcpListener::from_std(listener)
                    .map_err(|error| format!("could not use proxy listener: {error}"))?;
                addresses.push(listener.local_addr().map_err(|error| error.to_string())?);
                let status = ProxyStatus::default();
                statuses.push((status.clone(), path.clone()));
                tokio::spawn(async move {
                    if let Err(error) = ferroan_capture::serve_on_with_status(
                        listener,
                        upstream,
                        &path,
                        DEFAULT_REPLY_TIMEOUT,
                        status,
                    )
                    .await
                    {
                        eprintln!("live: proxy {} stopped: {error}", path.display());
                    }
                });
            }
            Ok::<_, String>((addresses, statuses))
        })?;
        Ok((addresses, Self { runtime, statuses }))
    }

    pub(crate) fn finish(self) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut failure = None;
            for (status, path) in self.statuses {
                let result = status
                    .finish(DEFAULT_REPLY_TIMEOUT + Duration::from_secs(20))
                    .await;
                let late = status.refused_after_stop();
                if late > 0 {
                    println!(
                        "live: proxy {} kept {late} request(s) that came after its upstream was stopped",
                        path.display()
                    );
                }
                if let Err(error) = result {
                    failure.get_or_insert_with(|| format!("capture proxy failed: {error}"));
                }
            }
            failure.map_or(Ok(()), Err)
        })
    }
}

/// A localhost port with nothing bound.
///
/// Chosen after the proxy listeners. Binding port 0 and dropping the socket
/// leaves a port a connect refuses on Linux, macOS and Windows. `taken` are
/// the listen ports and any closed ports already chosen; the result is none
/// of them. Nothing after this may bind port 0: that bind can be given the
/// port, which is the #297 race. A socket that is held and never listens is
/// not a closed port, because macOS does not refuse that connect.
#[cfg(test)]
pub(crate) struct ClosedUpstream {
    address: SocketAddr,
}

#[cfg(test)]
impl ClosedUpstream {
    pub(crate) fn reserve(taken: &[SocketAddr]) -> Self {
        for _ in 0..32 {
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let address = socket.local_addr().unwrap();
            drop(socket);
            if !taken.contains(&address) {
                return Self { address };
            }
        }
        panic!("closed upstream collided with a proxy listen port");
    }

    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }
}

/// Bind `count` proxy listeners, then one closed upstream per listener.
///
/// Listeners stay bound. Each closed port is chosen only after that, so it
/// is not a listen port, and the caller must not bind port 0 afterwards.
#[cfg(test)]
pub(crate) fn listeners_and_closed(
    count: usize,
) -> (Vec<std::net::TcpListener>, Vec<ClosedUpstream>) {
    let listeners: Vec<_> = (0..count)
        .map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    let mut taken: Vec<SocketAddr> = listeners
        .iter()
        .map(|listener| listener.local_addr().unwrap())
        .collect();
    let mut closed = Vec::with_capacity(count);
    for _ in 0..count {
        let upstream = ClosedUpstream::reserve(&taken);
        taken.push(upstream.address());
        closed.push(upstream);
    }
    (listeners, closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn a_refusal_after_the_harness_stops_that_upstream_is_not_a_failure() {
        let paths: Vec<PathBuf> = ["stopped", "up"]
            .iter()
            .map(|name| {
                crate::repo_root()
                    .join(".scratch")
                    .join(format!("proxy-{name}-{}.cap", uuid::Uuid::new_v4()))
            })
            .collect();
        // Two upstreams with nothing listening: one the harness is stopping,
        // one that is meant to be up. Listeners are bound first. Each closed
        // port is a bind of port 0 that is dropped, and it is not a listen port.
        let (listeners, closed) = listeners_and_closed(2);
        let stopped = closed[0].address();
        let up = closed[1].address();
        let refused = std::net::TcpStream::connect(stopped).unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::ConnectionRefused);
        let (addresses, proxies) = Proxies::start_bound(listeners, &[stopped, up], &paths).unwrap();
        assert!(
            addresses
                .iter()
                .all(|address| *address != stopped && *address != up)
        );
        proxies.upstream_stopping(0);
        for address in &addresses {
            let mut client = std::net::TcpStream::connect(address).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            client
                .write_all(b"DELETE /agent/x/ HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut response = Vec::new();
            let _ = client.read_to_end(&mut response);
            assert!(response.is_empty());
        }
        // Only the refusal while the upstream was meant to be up counts.
        let error = proxies.finish().unwrap_err();
        assert!(error.contains("recorded 1 failure(s)"), "{error}");
        for path in paths {
            let records = ferroan_capture::read_capture(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(records.len(), 1, "{}", path.display());
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn a_forced_upstream_error_fails_the_harness_proxy_run() {
        let path = crate::repo_root()
            .join(".scratch")
            .join(format!("proxy-error-{}.cap", uuid::Uuid::new_v4()));
        let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let (addresses, proxies) = Proxies::start(&[(
            "127.0.0.1:0".parse().unwrap(),
            upstream.local_addr().unwrap(),
            path.clone(),
        )])
        .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 1024];
            assert!(stream.read(&mut request).unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort")
                .unwrap();
        });
        let mut client = std::net::TcpStream::connect(addresses[0]).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        server.join().unwrap();
        assert!(
            proxies
                .finish()
                .unwrap_err()
                .contains("capture proxy failed")
        );
        std::fs::remove_file(path).unwrap();
    }
}
