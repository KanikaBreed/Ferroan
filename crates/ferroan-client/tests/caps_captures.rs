// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP replies compared with the pinned C# server's captured bytes.

// Integration tests are a test crate; the production lints stay on the library.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::Method;
use ferroan_client::CapsRegistry;

struct RegisteredCaps {
    registry: CapsRegistry,
    seed: String,
    events: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for RegisteredCaps {
    fn drop(&mut self) {
        self.registry.clear();
        self.task.abort();
    }
}

impl RegisteredCaps {
    async fn bind() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let registry = CapsRegistry::new(&base).unwrap();
        let seed_path = capture("seed")["path"].as_str().unwrap().to_owned();
        registry.register(&seed_path).unwrap();
        let app = registry.router();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut caps = Self {
            registry,
            seed: format!("{base}{seed_path}"),
            events: String::new(),
            task,
        };
        let response = ferroan_bot::login::http_client()
            .unwrap()
            .post(caps.seed_url())
            .header("content-type", "application/llsd+xml")
            .body(include_bytes!("../../../fixtures/caps/seed.request.xml").as_slice())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = response.bytes().await.unwrap();
        let seed = ferroan_llsd::from_xml(&bytes).unwrap();
        caps.events = seed.get("EventQueueGet").as_str().to_owned();
        let shape = &manifest()["event_queue_url_shape"];
        let path = caps.events.strip_prefix(&base).unwrap();
        let id = path
            .strip_prefix(shape["path_prefix"].as_str().unwrap())
            .unwrap();
        let uuid = uuid::Uuid::parse_str(id).unwrap();
        assert_eq!(id, uuid.to_string());
        assert_eq!(shape["uuid_format"], "hyphenated-lowercase");
        assert_eq!(shape["uuid_version"], uuid.get_version_num());
        assert_eq!(shape["uuid_variant"], format!("{:?}", uuid.get_variant()));
        assert_eq!(shape["trailing_slash"], path.ends_with('/'));
        caps
    }

    fn seed_url(&self) -> &str {
        &self.seed
    }
    fn event_queue_url(&self) -> &str {
        &self.events
    }
}

fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../fixtures/caps/manifest.json")).unwrap()
}

fn capture(name: &str) -> serde_json::Value {
    manifest()["exchanges"][name].clone()
}

#[tokio::test]
async fn delivered_event_batch_matches_captured_csharp_bytes() {
    let expected = include_str!("../../../fixtures/caps/events.response.xml");
    let captured = ferroan_llsd::from_xml(expected.as_bytes()).unwrap();
    let ferroan_llsd::Llsd::Array(events) = captured.get("events") else {
        panic!("capture must contain an events array");
    };
    let caps = RegisteredCaps::bind().await;
    for event in events {
        caps.registry.enqueue_event(event.clone()).unwrap();
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
    let caps = RegisteredCaps::bind().await;
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
            actual.replace(
                caps.event_queue_url(),
                manifest()["event_queue_url"].as_str().unwrap()
            ),
            expected,
            "{name} must preserve the C# XML bytes"
        );
    }
}

#[tokio::test]
async fn seed_errors_and_options_match_captured_csharp_replies() {
    let caps = RegisteredCaps::bind().await;
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
            for (name, value) in fixture["cors"].as_object().unwrap() {
                assert_eq!(reply.headers()[name], value.as_str().unwrap());
            }
        } else {
            assert_eq!(
                reply.headers()["connection"],
                fixture["response_connection"].as_str().unwrap()
            );
        }
        assert_eq!(reply.bytes().await.unwrap().as_ref(), expected, "{name}");
    }
}

#[tokio::test]
async fn binary_seed_requests_match_captured_csharp_replies() {
    let caps = RegisteredCaps::bind().await;
    let http = ferroan_bot::login::http_client().unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/caps");
    for name in [
        "seed-binary-invalid",
        "seed-binary-unpadded",
        "seed-binary-base16",
        "seed-binary-empty-base16",
        "seed-binary-selfclosing-base16",
    ] {
        let fixture = capture(name);
        let request =
            std::fs::read(directory.join(fixture["request_file"].as_str().unwrap())).unwrap();
        let expected =
            std::fs::read(directory.join(fixture["response_file"].as_str().unwrap())).unwrap();
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
            fixture["status"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            reply.headers()["content-type"],
            fixture["response_content_type"].as_str().unwrap(),
            "{name}"
        );
        if !reply.status().is_success() {
            assert_eq!(
                reply.headers()["connection"],
                fixture["response_connection"].as_str().unwrap(),
                "{name}"
            );
        }
        assert_eq!(reply.bytes().await.unwrap().as_ref(), expected, "{name}");
    }
}
