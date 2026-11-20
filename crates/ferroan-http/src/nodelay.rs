// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `TCP_NODELAY` on sockets Ferroan accepts or opens.
//!
//! At the pin, `Source/OpenSim.Framework.Servers.HttpServer/OSHttpServer/HttpListener.cs`
//! sets `socket.NoDelay = true` after accept (lines 170–172) and, on
//! `SocketException`, disposes that socket and continues the accept loop.
//! `Source/OpenSim.Framework/OutboundUrlFilter.cs` sets `NoDelay = true` on
//! the socket before `ConnectAsync` (`ConnectProxy` line 480, `ConnectSocket`
//! line 513). This file does not copy that C#.
//!
//! Axum's [`tap_io`](axum::serve::ListenerExt::tap_io) cannot drop the
//! connection. A failed [`set_tcp_nodelay`] is ignored and the connection is
//! still served. That differs from C# only when `setsockopt` fails.
//!
//! reqwest 0.12 `ClientBuilder::tcp_nodelay` defaults to `true`. No Ferroan
//! builder passes `false`, so outbound reqwest sockets stay on C#'s choice.

use std::io;

use tokio::net::TcpStream;

/// Enable `TCP_NODELAY` on one connected TCP socket.
pub fn set_tcp_nodelay(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)
}

/// Run [`set_tcp_nodelay`] on every socket this listener accepts.
pub fn accepted_nodelay<L>(
    listener: L,
) -> axum::serve::TapIo<L, impl FnMut(&mut TcpStream) + Send + 'static>
where
    L: axum::serve::Listener<Io = TcpStream>,
{
    use axum::serve::ListenerExt;
    listener.tap_io(|stream| {
        let _ = set_tcp_nodelay(stream);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::serve::ListenerExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn accepted_socket_reads_tcp_nodelay() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        let listener = accepted_nodelay(listener).tap_io(move |stream| {
            let mut slot = slot.lock().unwrap_or_else(|poison| poison.into_inner());
            if slot.is_none() {
                *slot = Some(stream.nodelay().ok());
            }
        });
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/", axum::routing::get(|| async { "ok" })),
            )
            .await
            .unwrap();
        });
        let _client = TcpStream::connect(addr).await.unwrap();
        let read = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(value) = *seen.lock().unwrap_or_else(|poison| poison.into_inner()) {
                    return value;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(read, Some(true));
        server.abort();
    }
}
