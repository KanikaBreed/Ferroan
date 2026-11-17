// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! LLSD bodies on an HTTP request.
//!
//! Tranquillity's `OSDParser.Deserialize(Stream)` looks at the first bytes.
//! The prefixes are case-sensitive, which is the stream overload
//! (`StructuredData.cs` at libopenmetaverse `a3596616`). A body that matches
//! none of them is JSON there. Ferroan has no JSON codec and returns
//! [`HttpError::Unsupported`](crate::HttpError::Unsupported).
//!
//! The binary serializer writes `<?llsd/binary?>` with no space. The stream
//! check looks for `<? llsd/binary`. Ferroan accepts both, because the
//! serializer's own header would otherwise be rejected.

use axum::body::Body;
use axum::extract::FromRequest;
use axum::http::{HeaderMap, Request, header};
use axum::response::{IntoResponse, Response};
use ferroan_llsd::{Llsd, from_binary, from_notation, from_xml, to_xml};

use crate::BodyLimit;
use crate::MAX_XML_CHARS;
use crate::error::HttpError;
use crate::service::read_body;

const XML: &[&str] = &["application/llsd+xml", "application/xml", "text/xml"];
const JSON: &[&str] = &["application/llsd+json", "application/json"];

pub struct LlsdBody(pub Llsd);

impl<S> FromRequest<S> for LlsdBody
where
    S: Send + Sync,
{
    type Rejection = HttpError;

    async fn from_request(req: Request<Body>, _state: &S) -> Result<Self, Self::Rejection> {
        let limit = req
            .extensions()
            .get::<BodyLimit>()
            .copied()
            .map(|limit| limit.0)
            .unwrap_or(MAX_XML_CHARS);
        let bytes = read_body(req.into_body(), limit).await?;
        Ok(LlsdBody(llsd_from_http_body(&bytes)?))
    }
}

pub fn llsd_from_http_body(bytes: &[u8]) -> Result<Llsd, HttpError> {
    let head = header_text(bytes);
    if starts_with_any(&head, &["<llsd>", "<?xml", "<? llsd/xml"]) {
        return Ok(from_xml(bytes)?);
    }
    if head.starts_with("<? llsd/notatio") {
        return Ok(from_notation(bytes)?);
    }
    if head.starts_with("<? llsd/binary") || head.starts_with("<?llsd/binary?>") {
        return Ok(from_binary(bytes)?.0);
    }
    Err(HttpError::Unsupported("application/llsd+json"))
}

/// Pick the response bytes the way `BaseHttpServer.BuildLLSDResponse` does.
/// JSON is refused. Anything else is informal LLSD XML, which is also the
/// fallback Tranquillity uses.
pub fn llsd_response(
    value: &Llsd,
    accept: Option<&str>,
    content_type: Option<&str>,
) -> Result<Response, HttpError> {
    if wants_json(accept, content_type) {
        return Err(HttpError::Unsupported("application/llsd+json"));
    }
    let body = to_xml(value);
    Ok(([(header::CONTENT_TYPE, "application/llsd+xml")], body).into_response())
}

fn wants_json(accept: Option<&str>, content_type: Option<&str>) -> bool {
    if let Some(accept) = accept {
        for part in accept.split(',') {
            let token = media_token(part);
            if XML.contains(&token) {
                return false;
            }
            if JSON.contains(&token) {
                return true;
            }
        }
    }
    if let Some(content_type) = content_type {
        let token = media_token(content_type);
        return JSON.contains(&token);
    }
    false
}

fn media_token(value: &str) -> &str {
    value.split(';').next().unwrap_or(value).trim()
}

fn header_text(bytes: &[u8]) -> String {
    let n = bytes.len().min(17);
    bytes[..n]
        .iter()
        .map(|byte| if byte.is_ascii() { *byte as char } else { '?' })
        .collect()
}

fn starts_with_any(head: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| head.starts_with(prefix))
}

/// `Accept` values the LLSD response helper understands. Exposed so a handler
/// can pass the header through without parsing it twice.
pub fn accept_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
}

pub fn content_type_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
}

/// Status used when a handler wants the negotiated body as a response.
pub fn llsd_response_status(value: &Llsd, headers: &HeaderMap) -> Result<Response, HttpError> {
    llsd_response(value, accept_header(headers), content_type_header(headers))
}
