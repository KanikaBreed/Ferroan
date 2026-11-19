// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Checked form calls fail on the final HTTP status before reading the body.
//! Unchecked calls parse that body. The policy is chosen per call.

use axum::{Router, http::StatusCode, routing::any};
use ferroan_http::{
    Field, FieldMap, FormOptions, FormStatusPolicy, HttpError, post_form, post_form_with,
    post_form_with_client,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const XML: &str = "<ServerResponse><result>Success</result></ServerResponse>";

fn checked(timeout: Duration, max_response_bytes: usize) -> FormOptions {
    FormOptions {
        timeout,
        max_response_bytes,
        status: FormStatusPolicy::Checked,
    }
}

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, task)
}

#[tokio::test]
async fn checked_non_success_status_rejects_plausible_xml_without_exposing_private_data() {
    for status in [299, 300, 302, 400, 401, 403, 404, 500, 503] {
        let app = Router::new().route(
            "/",
            any(move || async move { (StatusCode::from_u16(status).unwrap(), XML) }),
        );
        let (url, server) = serve(app).await;
        let url = url.replacen("http://", "http://fixture-user:fixture-password@", 1);
        let result = post_form_with(
            &format!("{url}/?private=fixture-secret"),
            &FieldMap::new(),
            checked(Duration::from_secs(5), 1024),
        )
        .await;
        server.abort();
        if status == 299 {
            assert_eq!(result.unwrap()["result"], Field::Str("Success".into()));
        } else {
            let error = result.unwrap_err();
            assert!(matches!(error, HttpError::Status { status: actual } if actual == status));
            for display in [error.to_string(), format!("{error:?}")] {
                assert!(!display.contains("fixture-"));
                assert!(!display.contains(XML));
            }
        }
    }
}

#[tokio::test]
async fn unchecked_status_still_parses_success_xml() {
    let app = Router::new().route(
        "/",
        any(|| async { (StatusCode::INTERNAL_SERVER_ERROR, XML) }),
    );
    let (url, server) = serve(app).await;
    let parsed = post_form(&url, &FieldMap::new()).await.unwrap();
    server.abort();
    assert_eq!(parsed["result"], Field::Str("Success".into()));
}

#[tokio::test]
async fn supplied_client_preserves_redirect_policy() {
    let app = Router::new()
        .route(
            "/",
            any(|| async { (StatusCode::FOUND, [("location", "/success")], XML) }),
        )
        .route("/success", any(|| async { XML }));
    let (url, server) = serve(app).await;
    let no_redirects = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    assert!(matches!(
        post_form_with_client(
            &no_redirects,
            &url,
            &FieldMap::new(),
            checked(Duration::from_secs(5), 1024)
        )
        .await,
        Err(HttpError::Status { status: 302 })
    ));
    assert_eq!(
        post_form(&url, &FieldMap::new()).await.unwrap()["result"],
        Field::Str("Success".into())
    );
    server.abort();
}

// Send a response prefix then hold the connection open. Rejection of a failed
// status happens before the absent remainder arrives.
async fn held_response(prefix: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            assert!(request.len() < 4096);
            request.push(socket.read_u8().await.unwrap());
        }
        socket.write_all(&prefix).await.unwrap();
        std::future::pending::<()>().await;
    });
    (url, task)
}

#[tokio::test]
async fn failed_status_wins_over_an_unread_stalled_oversized_body() {
    let (url, server) = held_response(
        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 999999999\r\n\r\n".to_vec(),
    )
    .await;
    let client = reqwest::Client::new();
    let fields = FieldMap::new();
    let result = post_form_with_client(&client, &url, &fields, checked(Duration::from_secs(2), 1));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), result)
            .await
            .unwrap(),
        Err(HttpError::Status { status: 500 })
    ));
    server.abort();
}

#[tokio::test]
async fn successful_chunked_body_is_bounded_before_the_stream_finishes() {
    let (url, server) = held_response(
        format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{XML}\r\n",
            XML.len()
        )
        .into_bytes(),
    )
    .await;
    let client = reqwest::Client::new();
    let fields = FieldMap::new();
    let result = post_form_with_client(&client, &url, &fields, checked(Duration::from_secs(2), 16));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), result)
            .await
            .unwrap(),
        Err(HttpError::Limit { limit: 16, .. })
    ));
    server.abort();
}

#[tokio::test]
async fn per_request_deadline_covers_a_stalled_success_body() {
    let (url, server) =
        held_response(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n".to_vec()).await;
    let client = reqwest::Client::new();
    let fields = FieldMap::new();
    let result = post_form_with_client(
        &client,
        &url,
        &fields,
        checked(Duration::from_millis(50), 100),
    );
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), result)
            .await
            .unwrap(),
        Err(HttpError::Transport(_))
    ));
    server.abort();
}
