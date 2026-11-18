// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-net/src/caps.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// The POST is the viewer's seed request: an LLSD array of names, answered
// with a map of URLs. The viewer's full list and its retry loop are not
// included.
//
// Event queue reply and crossing-event interpretation follows
//   Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/EventQueueGetModule.cs
//   Source/OpenSim.Region.ClientStack.LindenCaps/EventQueue/EventQueueGetHandlers.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
// Original source notice:
// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! The seed capability.

use std::collections::HashMap;

use ferroan_llsd::{Llsd, from_xml, to_xml};

use crate::body::read_limited;

/// Capabilities the later scenarios call. The seed returns whichever of
/// these the region actually has.
pub const SEED_CAPS: &[&str] = &[
    "EventQueueGet",
    "GetTexture",
    "FetchInventoryDescendents2",
    "FetchInventory2",
    "NewFileAgentInventory",
];

/// A seed reply larger than this is refused. The answer is a short map of URLs.
pub const MAX_SEED_BODY: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CapsError {
    #[error("seed url is not http")]
    Url,
    #[error("http: {0}")]
    Http(String),
    #[error("seed status {0}")]
    Status(u16),
    #[error("seed reply is {got} bytes, over the {limit} byte limit")]
    TooLarge { got: usize, limit: usize },
    #[error("seed reply is not an LLSD map")]
    Malformed,
    #[error("llsd: {0}")]
    Llsd(String),
}

/// POST [`SEED_CAPS`] to the seed URL and return the URLs the region gave back.
pub async fn request_seed(
    http: &reqwest::Client,
    seed: &str,
) -> Result<HashMap<String, String>, CapsError> {
    let url = reqwest::Url::parse(seed).map_err(|_| CapsError::Url)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(CapsError::Url);
    }
    let names = Llsd::Array(SEED_CAPS.iter().map(|name| Llsd::from(*name)).collect());
    let body = to_xml(&names);
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/llsd+xml")
        .header(reqwest::header::ACCEPT, "application/llsd+xml")
        .body(body.clone())
        .send()
        .await
        .map_err(|error| CapsError::Http(error.without_url().to_string()))?;
    let status = response.status();
    let response_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if !status.is_success() {
        return Err(CapsError::Status(status.as_u16()));
    }
    let bytes = match read_limited(response, MAX_SEED_BODY).await {
        Ok(bytes) => bytes,
        Err(crate::body::BodyError::Transport) => {
            return Err(CapsError::Http("reading the seed reply failed".into()));
        }
        Err(crate::body::BodyError::TooLarge(got)) => {
            return Err(CapsError::TooLarge {
                got,
                limit: MAX_SEED_BODY,
            });
        }
    };
    crate::trace::note_exchange(crate::trace::Exchange {
        method: "POST",
        url: seed,
        request_type: Some("application/llsd+xml"),
        accept: Some("application/llsd+xml"),
        request_body: &body,
        status: status.as_u16(),
        response_type: response_type.as_deref(),
        response_body: &bytes,
    });
    let value = from_xml(&bytes).map_err(|error| CapsError::Llsd(error.to_string()))?;
    let Some(map) = value.as_map() else {
        return Err(CapsError::Malformed);
    };
    let mut caps = HashMap::new();
    for (name, item) in map {
        let text = item.to_string_value();
        if !text.is_empty() {
            caps.insert(name.clone(), text);
        }
    }
    Ok(caps)
}

/// One event-queue response: the `id` to ack, then the events in order.
pub struct EqBatch {
    pub id: i32,
    pub events: Vec<EqMessage>,
}

pub struct EqMessage {
    pub message: String,
    pub body: Llsd,
}

/// What a crossing event says about the destination simulator.
///
/// `EnableSimulator` and `TeleportFinish` carry a region handle.
/// `EstablishAgentCommunication` carries the seed and `sim-ip-and-port`
/// and no handle. `finish` is set only for `TeleportFinish`.
pub struct CrossingHint {
    pub ip: [u8; 4],
    pub port: u16,
    pub handle: Option<u64>,
    pub seed: Option<String>,
    pub finish: bool,
}

#[derive(Debug)]
pub enum EqPollError {
    /// The poll was held and then dropped, or the region had nothing yet.
    Retry,
    /// The queue is gone. The caller can still finish from UDP.
    Stop,
    Fatal(String),
}

/// POST one event-queue poll. The first poll sends `done` and no `ack`.
///
/// A failed transport and a 502 from `EventQueueGetModule.NoEvents` are
/// [`EqPollError::Retry`]. The per-request limit outlasts Tranquillity's
/// 58-second held poll and overrides the HTTP client's shorter default.
pub async fn poll_event_queue(
    http: &reqwest::Client,
    url: &str,
    ack: Option<i32>,
) -> Result<EqBatch, EqPollError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| EqPollError::Fatal("event queue url is not http".to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(EqPollError::Fatal(
            "event queue url is not http".to_string(),
        ));
    }
    let mut request = Llsd::new_map();
    if let Some(ack) = ack {
        request.insert("ack", Llsd::Integer(ack));
    }
    request.insert("done", Llsd::Boolean(false));
    let xml = to_xml(&request);
    let response = match http
        .post(parsed)
        .timeout(std::time::Duration::from_secs(65))
        .header(reqwest::header::CONTENT_TYPE, "application/llsd+xml")
        .header(reqwest::header::ACCEPT, "application/llsd+xml")
        .body(xml.clone())
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return Err(EqPollError::Retry),
    };
    let status = response.status().as_u16();
    let response_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = match read_limited(response, MAX_SEED_BODY).await {
        Ok(bytes) => bytes,
        Err(crate::body::BodyError::TooLarge(got)) => {
            return Err(EqPollError::Fatal(format!(
                "event queue reply is {got} bytes, over the {MAX_SEED_BODY} byte limit"
            )));
        }
        Err(crate::body::BodyError::Transport) => return Err(EqPollError::Retry),
    };
    crate::trace::note_exchange(crate::trace::Exchange {
        method: "POST",
        url,
        request_type: Some("application/llsd+xml"),
        accept: Some("application/llsd+xml"),
        request_body: &xml,
        status,
        response_type: response_type.as_deref(),
        response_body: &bytes,
    });
    // NoEvents is 502 once the region gives up the long poll.
    if status == 502 || status == 408 || status == 503 {
        return Err(EqPollError::Retry);
    }
    if status == 404 {
        return Err(EqPollError::Stop);
    }
    if !(200..300).contains(&status) {
        return Err(EqPollError::Fatal(format!("event queue status {status}")));
    }
    parse_event_queue(&bytes).map_err(EqPollError::Fatal)
}

/// Parse an event-queue body. The shape is `EventQueueGetModule.GetEvents`.
pub fn parse_event_queue(bytes: &[u8]) -> Result<EqBatch, String> {
    let value = from_xml(bytes).map_err(|error| error.to_string())?;
    if !value.has("events") || !value.has("id") {
        return Err("event queue reply is not an LLSD map of events".to_string());
    }
    let mut events = Vec::new();
    for event in value.get("events").as_array() {
        let message = event.get("message").as_str();
        if message.is_empty() {
            continue;
        }
        events.push(EqMessage {
            message: message.to_string(),
            body: event.get("body").clone(),
        });
    }
    Ok(EqBatch {
        id: value.get("id").as_i32(),
        events,
    })
}

/// `EnableSimulator`, `EstablishAgentCommunication` or `TeleportFinish`.
/// Other messages, and a destination with no port, are `None`.
pub fn crossing_hint(message: &str, body: &Llsd) -> Option<CrossingHint> {
    match message {
        "EnableSimulator" => {
            let info = body.get("SimulatorInfo").at(0);
            Some(CrossingHint {
                ip: ipv4(info.get("IP"))?,
                port: port_of(info.get("Port"))?,
                handle: u64_binary(info.get("Handle")),
                seed: None,
                finish: false,
            })
        }
        "EstablishAgentCommunication" => {
            let (ip, port) = split_host_port(body.get("sim-ip-and-port").as_str())?;
            Some(CrossingHint {
                ip,
                port,
                handle: None,
                seed: nonempty(body.get("seed-capability")),
                finish: false,
            })
        }
        "TeleportFinish" => {
            let info = body.get("Info").at(0);
            Some(CrossingHint {
                ip: ipv4(info.get("SimIP"))?,
                port: port_of(info.get("SimPort"))?,
                handle: u64_binary(info.get("RegionHandle")),
                seed: nonempty(info.get("SeedCapability")),
                finish: true,
            })
        }
        _ => None,
    }
}

fn ipv4(value: &Llsd) -> Option<[u8; 4]> {
    match value {
        Llsd::Binary(bytes) if bytes.len() == 4 => Some([bytes[0], bytes[1], bytes[2], bytes[3]]),
        _ => None,
    }
}

fn port_of(value: &Llsd) -> Option<u16> {
    let port = value.as_i32();
    u16::try_from(port).ok().filter(|port| *port != 0)
}

fn u64_binary(value: &Llsd) -> Option<u64> {
    match value {
        Llsd::Binary(bytes) if bytes.len() >= 8 => Some(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])),
        _ => None,
    }
}

fn nonempty(value: &Llsd) -> Option<String> {
    let text = value.as_str();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn split_host_port(text: &str) -> Option<([u8; 4], u16)> {
    let (host, port) = text.rsplit_once(':')?;
    let ip: std::net::Ipv4Addr = host.parse().ok()?;
    let port: u16 = port.parse().ok()?;
    if port == 0 {
        return None;
    }
    Some((ip.octets(), port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn held_event_poll_outlasts_the_old_eight_second_limit() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 8192];
            let _ = stream.read(&mut request).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(9)).await;
            let body = b"<llsd><map><key>events</key><array><map><key>message</key><string>TeleportFinish</string><key>body</key><map /></map></array><key>id</key><integer>7</integer></map></llsd>";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
        });
        let http = crate::login::http_client().unwrap();
        let batch = poll_event_queue(&http, &format!("http://{address}/events"), None)
            .await
            .unwrap();
        assert_eq!(batch.id, 7);
        assert_eq!(batch.events[0].message, "TeleportFinish");
        server.await.unwrap();
    }
}
