// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `POST /useralias`.
//!
//! Same failure bytes and `application/xml` content type as `/accounts`.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ferroan_http::{Field, FieldMap, MAX_QUERY_BYTES, build_xml_response, parse_query_string};
use uuid::Uuid;

use super::service::AliasService;

const FAILURE: &[u8] =
    b"<?xml version =\"1.0\"?><ServerResponse><result>Failure</result></ServerResponse>";

#[derive(Clone)]
pub struct AliasHttp {
    inner: Arc<Inner>,
}

struct Inner {
    service: AliasService,
    reject_shard_header: bool,
}

impl AliasHttp {
    pub fn new(service: AliasService, reject_shard_header: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                service,
                reject_shard_header,
            }),
        }
    }

    pub fn into_router(self) -> Router {
        Router::new()
            .route("/useralias", post(post_alias))
            .with_state(self.inner)
    }
}

async fn post_alias(State(inner): State<Arc<Inner>>, headers: HeaderMap, body: Body) -> Response {
    if inner.reject_shard_header && headers.get("X-SecondLife-Shard").is_some() {
        return plain(StatusCode::FORBIDDEN, Vec::new());
    }
    let bytes = match axum::body::to_bytes(body, MAX_QUERY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return plain(StatusCode::PAYLOAD_TOO_LARGE, Vec::new()),
    };
    let text = String::from_utf8_lossy(&bytes);
    let fields = match parse_query_string(text.trim()) {
        Ok(fields) => fields,
        Err(_) => return failure(),
    };
    match dispatch(&inner.service, &fields).await {
        Ok(body) => xml(body),
        Err(()) => failure(),
    }
}

async fn dispatch(service: &AliasService, fields: &FieldMap) -> Result<Vec<u8>, ()> {
    let method = text_field(fields, "METHOD").ok_or(())?;
    match method.as_str() {
        "getuserforalias" => get_user_for_alias(service, fields).await,
        "getuseraliases" => get_user_aliases(service, fields).await,
        _ => Err(()),
    }
}

async fn get_user_for_alias(service: &AliasService, fields: &FieldMap) -> Result<Vec<u8>, ()> {
    if let Some(text) = text_field(fields, "AliasID")
        && let Ok(alias_id) = Uuid::parse_str(&text)
        && let Some(alias) = service.get_user_for_alias(alias_id).await.map_err(|_| ())?
    {
        let mut result = FieldMap::new();
        result.insert("result".to_string(), Field::Map(alias.to_fields()));
        return xml_of(&result);
    }
    Ok(result_null())
}

async fn get_user_aliases(service: &AliasService, fields: &FieldMap) -> Result<Vec<u8>, ()> {
    if let Some(text) = text_field(fields, "UserID")
        && let Ok(user_id) = Uuid::parse_str(&text)
        && let Some(aliases) = service.get_user_aliases(user_id).await.map_err(|_| ())?
    {
        let mut result = FieldMap::new();
        for (index, alias) in aliases.iter().enumerate() {
            result.insert(format!("alias{index}"), Field::Map(alias.to_fields()));
        }
        return xml_of(&result);
    }
    Ok(result_null())
}

fn result_null() -> Vec<u8> {
    let mut result = FieldMap::new();
    result.insert("result".to_string(), Field::Str("null".to_string()));
    xml_of(&result).unwrap_or_else(|_| FAILURE.to_vec())
}

fn xml_of(fields: &FieldMap) -> Result<Vec<u8>, ()> {
    build_xml_response(fields)
        .map(|text| text.into_bytes())
        .map_err(|_| ())
}

fn text_field(fields: &FieldMap, key: &str) -> Option<String> {
    match fields.get(key) {
        Some(Field::Str(text) | Field::Uuid(text)) => Some(text.clone()),
        _ => None,
    }
}

fn failure() -> Response {
    xml(FAILURE.to_vec())
}

fn xml(body: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        body,
    )
        .into_response()
}

fn plain(status: StatusCode, body: Vec<u8>) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain")], body).into_response()
}
