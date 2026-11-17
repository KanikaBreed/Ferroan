// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! axum and reqwest on localhost, plus the limits the fixtures do not hit.

use std::future::Future;

use axum::Router;
use axum::extract::Request;
use axum::http::HeaderMap;
use axum::middleware::{Next, from_fn};
use axum::response::Response;
use axum::routing::post;
use ferroan_http::{
    BodyLimit, Field, FieldMap, FormBody, LlsdBody, MAX_FIELDS, MAX_XML_CHARS, MAX_XML_DEPTH,
    XmlResponse, form_router, llsd_from_http_body, llsd_response, llsd_response_status,
    parse_query_string_limited, parse_xml_response_limited, post_form, post_llsd_xml,
};
use ferroan_llsd::{Llsd, to_binary_framed};

fn text_map() -> FieldMap {
    let mut map = FieldMap::new();
    map.insert("Success".to_string(), Field::Str("true".to_string()));
    map.insert("Message".to_string(), Field::Str("hello".to_string()));
    map
}

async fn serve<F>(app: Router, check: impl FnOnce(String) -> F)
where
    F: Future<Output = ()>,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    check(format!("http://{addr}")).await;
}

#[tokio::test]
async fn form_post_matches_the_xml_reply() {
    let app = form_router("/friends", |fields| async move { Ok(fields) });
    serve(app, |base| async move {
        let parsed = post_form(&format!("{base}/friends"), &text_map())
            .await
            .unwrap();
        assert_eq!(parsed, text_map());
    })
    .await;
}

#[tokio::test]
async fn llsd_post_matches_informal_xml() {
    let app = Router::new().route(
        "/caps",
        post(|headers: HeaderMap, LlsdBody(value): LlsdBody| async move {
            llsd_response_status(&value, &headers).unwrap()
        }),
    );
    let value = Llsd::Map({
        let mut map = ferroan_llsd::Map::new();
        map.insert("ok".to_string(), Llsd::Boolean(true));
        map
    });
    serve(app, |base| async move {
        let parsed = post_llsd_xml(&format!("{base}/caps"), &value)
            .await
            .unwrap();
        assert_eq!(parsed, value);
    })
    .await;
}

async fn small_limit(mut request: Request, next: Next) -> Response {
    request.extensions_mut().insert(BodyLimit(8));
    next.run(request).await
}

#[tokio::test]
async fn a_body_over_the_limit_is_rejected() {
    let app = Router::new()
        .route(
            "/echo",
            post(|FormBody(fields): FormBody| async move { XmlResponse(fields) }),
        )
        .layer(from_fn(small_limit));
    serve(app, |base| async move {
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{base}/echo"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body("name=abcdefghij")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    })
    .await;
}

#[test]
fn query_and_xml_limits_return_errors() {
    let query = parse_query_string_limited("a=1&b=2", 3, MAX_FIELDS);
    assert!(query.is_err());
    let fields = parse_query_string_limited("a=1&b=2", 100, 1);
    assert!(fields.is_err());
    let xml = "<ServerResponse><a>hello</a></ServerResponse>";
    assert!(parse_xml_response_limited(xml, 10, MAX_XML_DEPTH).is_err());
    let mut deep = String::from("<ServerResponse>");
    for index in 0..MAX_XML_DEPTH {
        deep.push_str(&format!("<m{index} type=\"List\">"));
    }
    deep.push_str("<z>1</z>");
    for index in (0..MAX_XML_DEPTH).rev() {
        deep.push_str(&format!("</m{index}>"));
    }
    deep.push_str("</ServerResponse>");
    assert!(deep.chars().count() < MAX_XML_CHARS);
    assert!(parse_xml_response_limited(&deep, MAX_XML_CHARS, MAX_XML_DEPTH).is_err());
}

#[test]
fn llsd_headers_follow_the_stream_detector() {
    let xml = b"<llsd><boolean>1</boolean></llsd>";
    assert!(matches!(
        llsd_from_http_body(xml).unwrap(),
        Llsd::Boolean(true)
    ));
    let framed = to_binary_framed(&Llsd::Boolean(true));
    assert!(llsd_from_http_body(&framed).is_ok());
    let mut spaced = b"<? llsd/binary ?>\n".to_vec();
    spaced.extend_from_slice(&ferroan_llsd::to_binary(&Llsd::Boolean(true)));
    assert!(llsd_from_http_body(&spaced).is_ok());
    assert!(llsd_from_http_body(b"{\"ok\":true}").is_err());
    assert!(llsd_response(&Llsd::Boolean(true), Some("application/llsd+json"), None).is_err());
    assert!(
        llsd_response(
            &Llsd::Boolean(true),
            Some("application/llsd+xml"),
            Some("application/llsd+json")
        )
        .is_ok()
    );
}
