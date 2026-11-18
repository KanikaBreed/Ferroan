// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP replies compared with the pinned C# server's captured bytes.

use axum::http::Method;
use ferroan_client::CapsServer;

const CAPTURED_EVENTS_URL: &str = "http://192.0.2.1:9000/CE/00000000-0000-4000-8000-000000000001";

fn capture(name: &str) -> serde_json::Value {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/caps/manifest.json")).unwrap();
    manifest["exchanges"][name].clone()
}

#[tokio::test]
async fn delivered_event_batch_matches_captured_csharp_bytes() {
    let expected = include_str!("../../../fixtures/caps/events.response.xml");
    let captured = ferroan_llsd::from_xml(expected.as_bytes()).unwrap();
    let ferroan_llsd::Llsd::Array(events) = captured.get("events") else {
        panic!("capture must contain an events array");
    };
    let caps = CapsServer::bind().await.unwrap();
    for event in events {
        caps.enqueue_event(event.clone()).unwrap();
    }
    let http = ferroan_bot::login::http_client().unwrap();
    let fixture = capture("events");
    let reply = http
        .post(caps.event_queue_url())
        .header(
            "content-type",
            fixture["request_content_type"].as_str().unwrap(),
        )
        .body(include_bytes!("../../../fixtures/caps/events.request.xml").as_slice())
        .send()
        .await
        .unwrap();
    assert_eq!(
        u64::from(reply.status().as_u16()),
        fixture["status"].as_u64().unwrap()
    );
    assert_eq!(
        reply.headers()["content-type"].to_str().unwrap(),
        fixture["response_content_type"].as_str().unwrap()
    );
    let actual = reply.text().await.unwrap();
    let delivered = ferroan_llsd::from_xml(actual.as_bytes()).unwrap();
    let id = delivered.get("id").as_i32();
    assert!((0..30_000_000).contains(&id));
    // Only the random batch id changes. Events and their XML encoding retain
    // every byte of the scrubbed C# capture.
    let suffix = format!("<key>id</key><integer>{id}</integer></map></llsd>");
    let captured_suffix = format!(
        "<key>id</key><integer>{}</integer></map></llsd>",
        captured.get("id").as_i32()
    );
    assert_eq!(
        actual.strip_suffix(&suffix),
        expected.strip_suffix(&captured_suffix)
    );
    assert!(actual.ends_with(&suffix));
    assert!(expected.ends_with(&captured_suffix));
}

#[tokio::test]
async fn seed_string_and_uri_names_match_captured_csharp_replies() {
    let caps = CapsServer::bind().await.unwrap();
    let http = ferroan_bot::login::http_client().unwrap();
    for (name, request, expected) in [
        (
            "seed",
            include_bytes!("../../../fixtures/caps/seed.request.xml").as_slice(),
            include_str!("../../../fixtures/caps/seed.response.xml"),
        ),
        (
            "seed-uri",
            include_bytes!("../../../fixtures/caps/seed-uri.request.xml").as_slice(),
            include_str!("../../../fixtures/caps/seed-uri.response.xml"),
        ),
    ] {
        let fixture = capture(name);
        let reply = http
            .post(caps.seed_url())
            .header(
                "content-type",
                fixture["request_content_type"].as_str().unwrap(),
            )
            .body(request)
            .send()
            .await
            .unwrap();
        assert_eq!(
            u64::from(reply.status().as_u16()),
            fixture["status"].as_u64().unwrap()
        );
        assert_eq!(
            reply.headers()["content-type"].to_str().unwrap(),
            fixture["response_content_type"].as_str().unwrap()
        );
        let actual = reply.text().await.unwrap();
        assert_eq!(
            actual.replace(caps.event_queue_url(), CAPTURED_EVENTS_URL),
            expected,
            "{name} must preserve the C# XML bytes"
        );
    }
}

#[tokio::test]
async fn seed_errors_and_options_match_captured_csharp_replies() {
    let caps = CapsServer::bind().await.unwrap();
    let http = ferroan_bot::login::http_client().unwrap();
    for (name, method, request, expected) in [
        (
            "seed-get",
            Method::GET,
            b"".as_slice(),
            include_bytes!("../../../fixtures/caps/seed-get.response.body").as_slice(),
        ),
        (
            "seed-malformed",
            Method::POST,
            include_bytes!("../../../fixtures/caps/seed-malformed.request.body").as_slice(),
            include_bytes!("../../../fixtures/caps/seed-malformed.response.body").as_slice(),
        ),
        (
            "seed-options",
            Method::OPTIONS,
            b"".as_slice(),
            include_bytes!("../../../fixtures/caps/seed-options.response.body").as_slice(),
        ),
    ] {
        let fixture = capture(name);
        let mut request = http.request(method, caps.seed_url()).body(request);
        if let Some(content_type) = fixture["request_content_type"].as_str() {
            request = request.header("content-type", content_type);
        }
        let reply = request.send().await.unwrap();
        assert_eq!(
            u64::from(reply.status().as_u16()),
            fixture["status"].as_u64().unwrap()
        );
        assert_eq!(
            reply.headers()["content-type"].to_str().unwrap(),
            fixture["response_content_type"].as_str().unwrap()
        );
        if name == "seed-options" {
            assert_eq!(reply.headers()["access-control-allow-origin"], "*");
            assert_eq!(
                reply.headers()["access-control-allow-methods"],
                "GET, POST, DELETE, PUT, OPTIONS"
            );
            assert_eq!(
                reply.headers()["access-control-allow-headers"],
                "Content-Type"
            );
        } else {
            assert_eq!(reply.headers()["connection"], "close");
        }
        assert_eq!(reply.bytes().await.unwrap().as_ref(), expected, "{name}");
    }
}
