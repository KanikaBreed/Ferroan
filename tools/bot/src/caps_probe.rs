// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local-grid HTTP evidence probe. Expectations are checked against the pinned
//! C# handlers; the capture proxy records actual HTTP bytes and poll timing.

use std::time::{Duration, Instant};

use ferroan_llsd::{Llsd, from_xml, to_xml};

use crate::Session;

const SEED: &[u8] = b"<llsd><array><string>EventQueueGet</string></array></llsd>";
const URI_SEED: &[u8] = b"<llsd><array><uri>EventQueueGet</uri></array></llsd>";

struct Reply {
    status: u16,
    body: Vec<u8>,
    content_type: String,
    elapsed: Duration,
}

pub(crate) async fn run(
    http: &reqwest::Client,
    session: &mut Session,
    seed: &str,
) -> Result<String, String> {
    let reply = exchange(http, session, "POST", seed, SEED).await?;
    require_status(&reply, 200, "seed")?;
    let map = from_xml(&reply.body).map_err(|error| error.to_string())?;
    let event_url = map.get("EventQueueGet").as_str().to_string();
    if event_url.is_empty() {
        return Err("caps: seed supplied no EventQueueGet".to_string());
    }
    let uri = exchange(http, session, "POST", seed, URI_SEED).await?;
    require_status(&uri, 200, "URI seed")?;
    if uri.body != reply.body {
        return Err("caps: URI seed entry differs from the string entry".to_string());
    }
    require_status(
        &exchange(http, session, "GET", seed, &[]).await?,
        404,
        "GET seed",
    )?;
    require_status(
        &exchange(http, session, "POST", seed, b"not LLSD XML").await?,
        400,
        "malformed seed",
    )?;
    require_status(
        &exchange(http, session, "OPTIONS", seed, &[]).await?,
        200,
        "OPTIONS seed",
    )?;

    let mut ack = None;
    let mut delivered = 0;
    for _ in 0..8 {
        let mut request = Llsd::new_map();
        if let Some(id) = ack {
            request.insert("ack", Llsd::Integer(id));
        }
        request.insert("done", Llsd::Boolean(false));
        let poll = exchange(http, session, "POST", &event_url, &to_xml(&request)).await?;
        if poll.status == 502 && poll.elapsed >= Duration::from_secs(55) {
            if poll.content_type != "text/html" {
                return Err(format!(
                    "caps: empty poll content type {}",
                    poll.content_type
                ));
            }
            if !poll.body.is_empty() {
                return Err("caps: empty poll returned a nonempty body".to_string());
            }
            return Ok(format!(
                "caps seed: pass (string, URI, GET, malformed, OPTIONS)\ncaps empty poll: pass (502, {}ms, {})\ncaps event batches: {delivered}",
                poll.elapsed.as_millis(),
                poll.content_type
            ));
        }
        // A marker already in C#'s new queue can produce an immediate 502.
        if poll.status == 502 {
            continue;
        }
        require_status(&poll, 200, "event poll")?;
        if poll.content_type != "application/xml" {
            return Err(format!(
                "caps: event reply content type {}",
                poll.content_type
            ));
        }
        let batch = crate::caps::parse_event_queue(&poll.body)?;
        if batch.events.is_empty() {
            return Err("caps: empty event poll answered 200 immediately".to_string());
        }
        ack = Some(batch.id);
        delivered += 1;
    }
    Err("caps: no held empty poll after eight event batches".to_string())
}

fn require_status(reply: &Reply, expected: u16, label: &str) -> Result<(), String> {
    if reply.status != expected {
        return Err(format!(
            "caps: {label} status {}, expected {expected}",
            reply.status
        ));
    }
    if label.contains("seed") && reply.content_type != "text/html;charset=UTF-8" {
        return Err(format!("caps: {label} content type {}", reply.content_type));
    }
    Ok(())
}

async fn exchange(
    http: &reqwest::Client,
    session: &mut Session,
    method: &str,
    url: &str,
    body: &[u8],
) -> Result<Reply, String> {
    let method_value =
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "invalid probe method")?;
    let started = Instant::now();
    let pending = async {
        let response = http
            .request(method_value, url)
            .timeout(Duration::from_secs(65))
            .header(reqwest::header::CONTENT_TYPE, "application/llsd+xml")
            .body(body.to_vec())
            .send()
            .await
            .map_err(|error| format!("caps request: {}", error.without_url()))?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let response_body = crate::body::read_limited(response, crate::caps::MAX_SEED_BODY)
            .await
            .map_err(|_| "caps response body failed or exceeded its bound".to_string())?;
        Ok::<_, String>(Reply {
            status,
            body: response_body,
            content_type,
            elapsed: started.elapsed(),
        })
    };
    tokio::pin!(pending);
    loop {
        tokio::select! {
            reply = &mut pending => {
                let reply = reply?;
                crate::trace::note_exchange(crate::trace::Exchange {
                    method, url, request_type: Some("application/llsd+xml"), accept: None,
                    request_body: body, status: reply.status, response_type: Some(&reply.content_type),
                    response_body: &reply.body,
                });
                return Ok(reply);
            }
            result = session.drive(Duration::from_millis(100)) => {
                result.map_err(|error| format!("caps circuit: {error}"))?;
            }
        }
    }
}
