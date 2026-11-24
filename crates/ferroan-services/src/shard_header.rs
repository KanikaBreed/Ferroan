// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replay of the Pin8 `X-SecondLife-Shard` capture.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ferroan_config::Config;
use ferroan_http::MAX_QUERY_BYTES;
use tower::ServiceExt;

use crate::{AuthHttp, GridUserHttp};

const PIN: &str = "c401bceecd1684832ae8ba86a31b5ee01a4ded7b";

#[tokio::test]
async fn shard_header_matches_the_pin_capture() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/http/shard-header.json");
    let text = std::fs::read_to_string(&path).expect("fixture");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(doc["pin"].as_str(), Some(PIN));
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() > 10);
    let auth_db = copy_fixture("AuthStore.db");
    let grid_db = copy_fixture("GridUserStore.db");
    for case in cases {
        let status = case["status"].as_u64().expect("status");
        let handled = case["requests_handled"].as_u64().expect("handled");
        let bytes_read = case["bytes_read"].as_u64().expect("bytes");
        if status == 403 {
            assert_eq!(handled, 0, "{}", case["id"]);
            assert_eq!(bytes_read, 0, "{}", case["id"]);
        } else {
            assert_eq!(handled, 1, "{}", case["id"]);
        }
        let response = post_case(case, &auth_db, &grid_db).await;
        assert_eq!(response.0, status, "{}", case["id"]);
        assert_eq!(
            response.1,
            case["content_type"].as_str().unwrap(),
            "{}",
            case["id"]
        );
        assert_eq!(
            response.2,
            case["response_body"].as_str().unwrap(),
            "{}",
            case["id"]
        );
    }
}

async fn post_case(
    case: &serde_json::Value,
    auth_db: &std::path::Path,
    grid_db: &std::path::Path,
) -> (u64, String, String) {
    let service = case["service"].as_str().unwrap();
    let ini = service_ini(
        service,
        if service == "auth" { auth_db } else { grid_db },
        case,
    );
    let config = Config::parse(&ini).expect("ini");
    let router = if service == "auth" {
        AuthHttp::open(&config).await.expect("auth").into_router()
    } else {
        GridUserHttp::open(&config)
            .await
            .expect("griduser")
            .into_router()
    };
    let payload = match case["body_len"].as_u64() {
        Some(length) => "x".repeat(length as usize),
        None => case["body"].as_str().unwrap().to_string(),
    };
    let mut builder = Request::builder()
        .method("POST")
        .uri(case["path"].as_str().unwrap());
    if case["header_present"].as_bool().unwrap() {
        builder = builder.header(
            case["header_name"].as_str().unwrap(),
            case["header_value"].as_str().unwrap(),
        );
    }
    let response = router
        .oneshot(builder.body(Body::from(payload)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), MAX_QUERY_BYTES + 8)
        .await
        .unwrap();
    assert_ne!(status, StatusCode::PAYLOAD_TOO_LARGE, "{}", case["id"]);
    (
        status.as_u16() as u64,
        content_type,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

fn service_ini(service: &str, database: &std::path::Path, case: &serde_json::Value) -> String {
    let mut text = String::new();
    let network = case["network"].as_str().unwrap();
    if network != "absent" {
        text.push_str("[Network]\n");
        text.push_str(&allow_line(network));
    }
    if service == "auth" {
        text.push_str("[AuthenticationService]\n");
        text.push_str(
            "LocalServiceModule = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\"\n",
        );
        text.push_str("AllowSetPassword = true\n");
    } else {
        text.push_str("[GridUserService]\n");
        text.push_str(
            "LocalServiceModule = \"OpenSim.Services.UserAccountService.dll:GridUserService\"\n",
        );
    }
    text.push_str("StorageProvider = \"OpenSim.Data.SQLite.dll\"\n");
    text.push_str(&format!(
        "ConnectionString = \"Data Source={};Version=3;\"\n",
        database.display()
    ));
    let section = case["section"].as_str().unwrap();
    if section != "absent" {
        text.push_str(&allow_line(section));
    }
    text
}

fn allow_line(value: &str) -> String {
    match value {
        "true" => "AllowllHTTPRequestIn = true\n".to_string(),
        "false" => "AllowllHTTPRequestIn = false\n".to_string(),
        "empty" => "AllowllHTTPRequestIn =\n".to_string(),
        other => panic!("allow {other}"),
    }
}

fn copy_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ferroan-shard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/db/sqlite")
        .join(name);
    std::fs::copy(source, &path).unwrap();
    path
}
