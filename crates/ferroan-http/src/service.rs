// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A small axum surface for one Robust POST: a form body in, XML out.
//!
//! Handlers that need LLSD use [`crate::LlsdBody`] instead. The content type
//! of an XML reply is `text/xml`, which is what the Robust handlers set.

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::FromRequest;
use axum::http::{Request, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;

use crate::MAX_XML_CHARS;
use crate::error::HttpError;
use crate::query::parse_query_string;
use crate::value::FieldMap;
use crate::xml::build_xml_response;

/// Replaces the default body cap for the extractors on this request.
#[derive(Clone, Copy)]
pub struct BodyLimit(pub usize);

pub struct FormBody(pub FieldMap);

impl<S> FromRequest<S> for FormBody
where
    S: Send + Sync,
{
    type Rejection = HttpError;

    async fn from_request(req: Request<Body>, _state: &S) -> Result<Self, Self::Rejection> {
        let limit = req
            .extensions()
            .get::<BodyLimit>()
            .map(|limit| limit.0)
            .unwrap_or(MAX_XML_CHARS);
        let bytes = read_body(req.into_body(), limit).await?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| HttpError::Query("body is not utf-8"))?;
        Ok(FormBody(parse_query_string(text)?))
    }
}

pub struct XmlResponse(pub FieldMap);

impl IntoResponse for XmlResponse {
    fn into_response(self) -> Response {
        match build_xml_response(&self.0) {
            Ok(xml) => ([(header::CONTENT_TYPE, "text/xml")], xml).into_response(),
            Err(error) => error.into_response(),
        }
    }
}

pub fn xml_response(fields: &FieldMap) -> Result<Response, HttpError> {
    let xml = build_xml_response(fields)?;
    Ok(([(header::CONTENT_TYPE, "text/xml")], xml).into_response())
}

/// POST `path` with a form body. The handler's map is the XML reply.
pub fn form_router<F, Fut>(path: &'static str, handler: F) -> Router
where
    F: Fn(FieldMap) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<FieldMap, HttpError>> + Send + 'static,
{
    Router::new().route(
        path,
        post(move |FormBody(fields): FormBody| {
            let handler = handler.clone();
            async move {
                match handler(fields).await {
                    Ok(map) => XmlResponse(map).into_response(),
                    Err(error) => error.into_response(),
                }
            }
        }),
    )
}

pub async fn read_body(body: Body, max: usize) -> Result<Bytes, HttpError> {
    match to_bytes(body, max).await {
        Ok(bytes) => Ok(bytes),
        Err(error) => {
            let text = error.to_string();
            if text.contains("length limit") {
                Err(HttpError::Limit {
                    what: "body",
                    limit: max,
                    got: max.saturating_add(1),
                })
            } else {
                Err(HttpError::Transport(crate::error::scrub(&text)))
            }
        }
    }
}
