// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! reqwest calls that post a Robust form or an LLSD XML body and read the reply.

use ferroan_llsd::{Llsd, from_xml, to_xml};
use reqwest::header::CONTENT_TYPE;
use std::time::Duration;

use crate::MAX_XML_CHARS;
use crate::error::{HttpError, scrub};
use crate::query::build_query_string;
use crate::value::FieldMap;
use crate::xml::parse_xml_response;

fn client() -> Result<reqwest::Client, HttpError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| HttpError::Transport(scrub(&error.to_string())))
}

/// POST `application/x-www-form-urlencoded` and parse the `text/xml` reply.
pub async fn post_form(url: &str, fields: &FieldMap) -> Result<FieldMap, HttpError> {
    let body = build_query_string(fields)?;
    let response = client()?
        .post(url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|error| HttpError::Transport(scrub(&error.to_string())))?;
    let bytes = response_bytes(response).await?;
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
    let mut buf = Vec::new();
    let mut response = response;
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| HttpError::Transport(scrub(&error.to_string())))?;
        let Some(chunk) = chunk else {
            break;
        };
        let next = buf.len().saturating_add(chunk.len());
        if next > MAX_XML_CHARS {
            return Err(HttpError::Limit {
                what: "reply",
                limit: MAX_XML_CHARS,
                got: next,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}
