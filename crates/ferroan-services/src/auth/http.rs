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
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Server.Handlers/Authentication/AuthenticationServerPostHandler.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `POST /auth`, `POST /auth/plain` and `POST /auth/crypt`.
//!
//! Success and failure are `XmlTextWriter` documents: LF, no BOM, no encoding
//! attribute, two-space indent. `getauthinfo` is `ServerUtils.BuildXmlResponse`.
//! `/auth/crypt` reads at most 16384 bytes and returns an empty body. A body
//! over [`ferroan_http::MAX_QUERY_BYTES`] is 413, which Tranquillity does not
//! do. `X-SecondLife-Shard` is not checked on this handler. A bad `LIFETIME`
//! is the uncaught `Convert.ToInt32` failure: HTTP 500, `application/xml`,
//! empty body. `Convert.ToInt32` accepts surrounding spaces and one leading `+`.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ferroan_http::{Field, FieldMap, MAX_QUERY_BYTES, build_xml_response, parse_query_string};
use uuid::Uuid;

use super::model::AuthRow;
use super::service::AuthService;

const FAILURE: &str =
    "<?xml version=\"1.0\"?>\n<ServerResponse>\n  <Result>Failure</Result>\n</ServerResponse>";
const SUCCESS: &str =
    "<?xml version=\"1.0\"?>\n<ServerResponse>\n  <Result>Success</Result>\n</ServerResponse>";

#[derive(Clone)]
pub struct AuthHttp {
    inner: Arc<Inner>,
}

struct Inner {
    service: AuthService,
    allow_get_auth_info: bool,
    allow_set_auth_info: bool,
    allow_set_password: bool,
}

impl AuthHttp {
    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.inner.service.pool_occupancy()
    }

    pub(crate) fn new(
        service: AuthService,
        allow_get_auth_info: bool,
        allow_set_auth_info: bool,
        allow_set_password: bool,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                service,
                allow_get_auth_info,
                allow_set_auth_info,
                allow_set_password,
            }),
        }
    }

    pub fn into_router(self) -> Router {
        Router::new()
            .route("/auth", post(post_auth))
            .route("/auth/", post(post_auth))
            .route("/auth/{*rest}", post(post_auth_rest))
            .with_state(self.inner)
    }
}

async fn post_auth(State(inner): State<Arc<Inner>>, body: Body) -> Response {
    dispatch(inner, "", body).await
}

async fn post_auth_rest(
    State(inner): State<Arc<Inner>>,
    Path(rest): Path<String>,
    body: Body,
) -> Response {
    dispatch(inner, &rest, body).await
}

async fn dispatch(inner: Arc<Inner>, rest: &str, body: Body) -> Response {
    let bytes = match axum::body::to_bytes(body, MAX_QUERY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return plain(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let mode = first_segment(rest);
    match mode {
        "plain" => plain_methods(&inner, &bytes).await,
        "crypt" => {
            let _capped = bytes.len().min(16384);
            xml(StatusCode::OK, Vec::new())
        }
        _ => xml(StatusCode::OK, Vec::new()),
    }
}

fn first_segment(rest: &str) -> &str {
    rest.split(['/', '?', '&'])
        .find(|part| !part.is_empty())
        .unwrap_or("")
}

async fn plain_methods(inner: &Inner, body: &[u8]) -> Response {
    let text = String::from_utf8_lossy(body);
    let fields = match parse_query_string(text.trim()) {
        Ok(fields) => fields,
        Err(_) => return failure(),
    };
    let lifetime = match lifetime_of(&fields) {
        Ok(lifetime) => lifetime,
        Err(()) => return xml(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()),
    };
    let Some(method) = text_field(&fields, "METHOD") else {
        return failure();
    };
    let Some(principal_text) = text_field(&fields, "PRINCIPAL") else {
        return failure();
    };
    let Ok(principal) = Uuid::parse_str(&principal_text) else {
        return failure();
    };
    match method.as_str() {
        "authenticate" => {
            let Some(password) = text_field(&fields, "PASSWORD") else {
                return failure();
            };
            match inner
                .service
                .authenticate(principal, &password, lifetime)
                .await
            {
                Ok(token) if !token.is_empty() => xml(StatusCode::OK, success_token(&token)),
                Ok(_) => failure(),
                Err(error) => server_error(&error),
            }
        }
        "setpassword" => {
            if !inner.allow_set_password {
                return failure();
            }
            let Some(password) = text_field(&fields, "PASSWORD") else {
                return failure();
            };
            match inner.service.set_password(principal, &password).await {
                Ok(true) => xml(StatusCode::OK, SUCCESS.as_bytes().to_vec()),
                Ok(false) => failure(),
                Err(error) => server_error(&error),
            }
        }
        "verify" => {
            let Some(token) = token_field(&fields) else {
                return failure();
            };
            match inner.service.verify(principal, &token, lifetime).await {
                Ok(true) => xml(StatusCode::OK, SUCCESS.as_bytes().to_vec()),
                Ok(false) => failure(),
                Err(error) => server_error(&error),
            }
        }
        "release" => {
            let Some(token) = token_field(&fields) else {
                return failure();
            };
            match inner.service.release(principal, &token).await {
                Ok(true) => xml(StatusCode::OK, SUCCESS.as_bytes().to_vec()),
                Ok(false) => failure(),
                Err(error) => server_error(&error),
            }
        }
        "getauthinfo" => {
            if !inner.allow_get_auth_info {
                return failure();
            }
            match inner.service.get(principal).await {
                Ok(Some(row)) => match auth_info_xml(&row) {
                    Ok(body) => xml(StatusCode::OK, body),
                    Err(error) => server_error(&error),
                },
                Ok(None) => failure(),
                Err(error) => server_error(&error),
            }
        }
        "setauthinfo" => {
            if !inner.allow_set_auth_info {
                return failure();
            }
            let existing = match inner.service.get(principal).await {
                Ok(Some(row)) => row,
                Ok(None) => return failure(),
                Err(error) => return server_error(&error),
            };
            let row = apply_info(existing, &fields);
            match inner.service.set_info(&row).await {
                Ok(true) => xml(StatusCode::OK, SUCCESS.as_bytes().to_vec()),
                Ok(false) => failure(),
                Err(error) => server_error(&error),
            }
        }
        _ => failure(),
    }
}

fn apply_info(mut row: AuthRow, fields: &FieldMap) -> AuthRow {
    if let Some(value) = text_field(fields, "AccountType") {
        row.account_type = value;
    }
    if let Some(value) = text_field(fields, "PasswordHash") {
        row.password_hash = value;
    }
    if let Some(value) = text_field(fields, "PasswordSalt") {
        row.password_salt = value;
    }
    if let Some(value) = text_field(fields, "WebLoginKey") {
        row.web_login_key = value;
    }
    row
}

fn auth_info_xml(row: &AuthRow) -> Result<Vec<u8>, ferroan_http::HttpError> {
    let mut nested = FieldMap::new();
    nested.insert(
        "PrincipalID".to_string(),
        Field::Str(row.principal_id.to_string()),
    );
    nested.insert(
        "AccountType".to_string(),
        Field::Str(row.account_type.clone()),
    );
    nested.insert(
        "PasswordHash".to_string(),
        Field::Str(row.password_hash.clone()),
    );
    nested.insert(
        "PasswordSalt".to_string(),
        Field::Str(row.password_salt.clone()),
    );
    nested.insert(
        "WebLoginKey".to_string(),
        Field::Str(row.web_login_key.clone()),
    );
    let mut outer = FieldMap::new();
    outer.insert("result".to_string(), Field::Map(nested));
    Ok(build_xml_response(&outer)?.into_bytes())
}

fn token_field(fields: &FieldMap) -> Option<String> {
    let text = text_field(fields, "TOKEN")?;
    let id = Uuid::parse_str(&text).ok()?;
    Some(id.to_string())
}

fn success_token(token: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\"?>\n<ServerResponse>\n  <Result>Success</Result>\n  <Token>{token}</Token>\n</ServerResponse>"
    )
    .into_bytes()
}

/// `Convert.ToInt32`, then clamp a value above 30 down to 30.
fn lifetime_of(fields: &FieldMap) -> Result<i32, ()> {
    let Some(field) = fields.get("LIFETIME") else {
        return Ok(30);
    };
    let Some(text) = field_text(field) else {
        return Err(());
    };
    let text = text.trim();
    let text = text.strip_prefix('+').unwrap_or(text);
    let value: i32 = text.parse().map_err(|_| ())?;
    Ok(if value > 30 { 30 } else { value })
}

fn text_field(fields: &FieldMap, key: &str) -> Option<String> {
    fields.get(key).and_then(field_text)
}

fn field_text(field: &Field) -> Option<String> {
    match field {
        Field::Str(text) | Field::Uuid(text) => Some(text.clone()),
        Field::Bool(true) => Some("True".to_string()),
        Field::Bool(false) => Some("False".to_string()),
        Field::Int(number) => Some(number.to_string()),
        Field::List(_) | Field::Map(_) | Field::Null => None,
    }
}

fn failure() -> Response {
    xml(StatusCode::OK, FAILURE.as_bytes().to_vec())
}

fn server_error(error: &impl std::fmt::Display) -> Response {
    tracing::error!("authentication: {error}");
    xml(StatusCode::INTERNAL_SERVER_ERROR, Vec::new())
}

fn xml(status: StatusCode, body: Vec<u8>) -> Response {
    (status, [(header::CONTENT_TYPE, "application/xml")], body).into_response()
}

fn plain(status: StatusCode) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain")],
        Body::empty(),
    )
        .into_response()
}
