// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! reqwest calls that post a Robust form or an LLSD XML body and read the reply.
//!
//! Form status follows `Source/OpenSim.Framework/WebUtil.cs`
//! `SynchronousRestFormsRequester` at `858321883f5487a5d192d36ea9d7b90fa468310c`.
//! `MakeRequest` calls `EnsureSuccessStatusCode`. `MakePostRequest` does not
//! (TB10). The caller picks the helper. This file does not copy that C#.

use ferroan_llsd::{Llsd, from_xml, to_xml};
use reqwest::header::CONTENT_TYPE;
use std::time::Duration;

use crate::MAX_XML_CHARS;
use crate::error::{HttpError, scrub};
use crate::query::build_query_string;
use crate::value::FieldMap;
use crate::xml::parse_xml_response;

fn client() -> Result<reqwest::Client, HttpError> {
    // Leave `tcp_nodelay` at reqwest's default `true`. That matches
    // `OutboundUrlFilter` setting `NoDelay = true` before connect.
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| HttpError::Transport(scrub(&error.to_string())))
}

/// Which C# form helper a call follows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormStatusPolicy {
    /// `MakeRequest`: a final non-2xx status is [`HttpError::Status`] and the
    /// body is not read.
    Checked,
    /// `MakePostRequest`: the body is parsed whatever the HTTP status is.
    Unchecked,
}

/// Per-request bounds and the C# helper's status policy.
#[derive(Clone, Copy, Debug)]
pub struct FormOptions {
    /// Total request and response-body deadline. Overrides the client default.
    pub timeout: Duration,
    /// Maximum response bytes. The XML character limit remains an upper bound.
    pub max_response_bytes: usize,
    /// `MakeRequest` or `MakePostRequest`.
    pub status: FormStatusPolicy,
}

impl Default for FormOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_response_bytes: MAX_XML_CHARS,
            status: FormStatusPolicy::Unchecked,
        }
    }
}

impl FormOptions {
    /// Presence and GridUser connectors: `MakeRequest`.
    pub fn checked(timeout: Duration, max_response_bytes: usize) -> Self {
        Self {
            timeout,
            max_response_bytes,
            status: FormStatusPolicy::Checked,
        }
    }

    /// Grid register and deregister: `MakePostRequest`.
    pub fn unchecked(timeout: Duration, max_response_bytes: usize) -> Self {
        Self {
            timeout,
            max_response_bytes,
            status: FormStatusPolicy::Unchecked,
        }
    }
}

/// POST `application/x-www-form-urlencoded` and parse the `text/xml` reply.
///
/// This is `MakePostRequest`: a non-2xx status is still parsed. Callers that
/// follow `MakeRequest` pass [`FormOptions::checked`].
pub async fn post_form(url: &str, fields: &FieldMap) -> Result<FieldMap, HttpError> {
    post_form_with(url, fields, FormOptions::default()).await
}

/// [`post_form`] with an explicit helper policy and bounds.
pub async fn post_form_with(
    url: &str,
    fields: &FieldMap,
    options: FormOptions,
) -> Result<FieldMap, HttpError> {
    post_form_with_client(&client()?, url, fields, options).await
}

/// A form call on a caller-supplied client. The client's redirect policy is
/// kept. A checked call fails on the final non-2xx status before the body is
/// read. Errors omit request URLs and response bodies.
pub async fn post_form_with_client(
    client: &reqwest::Client,
    url: &str,
    fields: &FieldMap,
    options: FormOptions,
) -> Result<FieldMap, HttpError> {
    let body = build_query_string(fields)?;
    let response = client
        .post(url)
        .timeout(options.timeout)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(form_transport)?;
    if options.status == FormStatusPolicy::Checked && !response.status().is_success() {
        return Err(HttpError::Status {
            status: response.status().as_u16(),
        });
    }
    let bytes = bounded_response_bytes(
        response,
        options.max_response_bytes.min(MAX_XML_CHARS),
        form_transport,
    )
    .await?;
    let text = std::str::from_utf8(&bytes).map_err(|_| HttpError::Xml("reply is not utf-8"))?;
    parse_xml_response(text)
}

/// POST informal LLSD XML and parse an informal LLSD XML reply.
pub async fn post_llsd_xml(url: &str, value: &Llsd) -> Result<Llsd, HttpError> {
    let response = client()?
        .post(url)
        .header(CONTENT_TYPE, "application/llsd+xml")
        .header(reqwest::header::ACCEPT, "application/llsd+xml")
        .body(to_xml(value))
        .send()
        .await
        .map_err(|error| HttpError::Transport(scrub(&error.to_string())))?;
    let bytes = response_bytes(response).await?;
    Ok(from_xml(&bytes)?)
}

async fn response_bytes(response: reqwest::Response) -> Result<Vec<u8>, HttpError> {
    bounded_response_bytes(response, MAX_XML_CHARS, |error| {
        HttpError::Transport(scrub(&error.to_string()))
    })
    .await
}

fn form_transport(error: reqwest::Error) -> HttpError {
    HttpError::Transport(scrub(&error.without_url().to_string()))
}

async fn bounded_response_bytes(
    response: reqwest::Response,
    limit: usize,
    transport: impl Fn(reqwest::Error) -> HttpError,
) -> Result<Vec<u8>, HttpError> {
    let mut buf = Vec::new();
    let mut response = response;
    loop {
        let chunk = response.chunk().await.map_err(&transport)?;
        let Some(chunk) = chunk else {
            break;
        };
        let next = buf.len().saturating_add(chunk.len());
        if next > limit {
            return Err(HttpError::Limit {
                what: "reply",
                limit,
                got: next,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}
