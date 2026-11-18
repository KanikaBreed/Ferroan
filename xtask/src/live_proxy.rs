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
    statuses: Vec<ProxyStatus>,
}

impl Proxies {
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
                statuses.push(status.clone());
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

    pub(crate) fn finish(self) -> Result<(), String> {
        self.runtime.block_on(async {
            let mut failure = None;
            for status in self.statuses {
                if let Err(error) = status
                    .finish(DEFAULT_REPLY_TIMEOUT + Duration::from_secs(20))
                    .await
                {
                    failure.get_or_insert_with(|| format!("capture proxy failed: {error}"));
                }
            }
            failure.map_or(Ok(()), Err)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

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
