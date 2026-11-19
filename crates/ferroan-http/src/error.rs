// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ferroan_llsd::LlsdError;

/// A form, XML document, LLSD body or HTTP call that Ferroan will not use.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("{what} of {got} exceeds the limit {limit}")]
    Limit {
        what: &'static str,
        limit: usize,
        got: usize,
    },
    #[error("query: {0}")]
    Query(&'static str),
    #[error("xml: {0}")]
    Xml(&'static str),
    #[error(transparent)]
    Llsd(#[from] LlsdError),
    /// Tranquillity would answer `application/llsd+json` here. `ferroan-llsd`
    /// has no JSON codec, so the call fails instead of sending a body the
    /// codec cannot produce.
    #[error("unsupported media type {0}")]
    Unsupported(&'static str),
    #[error("http: {0}")]
    Transport(String),
    /// Final response status only. Never attach the request URL or reply body.
    #[error("http: unsuccessful response status {status}")]
    Status { status: u16 },
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Limit { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Unsupported(_) => StatusCode::NOT_ACCEPTABLE,
            _ => StatusCode::BAD_REQUEST,
        };
        (status, self.to_string()).into_response()
    }
}

/// Drop URL userinfo and cap the length. Driver text can contain a password.
/// A URL with no `@` before the path is left alone.
pub(crate) fn scrub(message: &str) -> String {
    let mut cleaned = String::with_capacity(message.len().min(500));
    let bytes = message.as_bytes();
    let mut index = 0;
    while index < bytes.len() && cleaned.len() < 500 {
        if bytes[index..].starts_with(b"://") {
            let start = index + 3;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'@' && bytes[end] != b'/' {
                end += 1;
            }
            if !push_capped(&mut cleaned, "://") {
                break;
            }
            index = if end < bytes.len() && bytes[end] == b'@' {
                end + 1
            } else {
                start
            };
            continue;
        }
        let ch = message[index..].chars().next().unwrap();
        if !push_capped(&mut cleaned, &message[index..index + ch.len_utf8()]) {
            break;
        }
        index += ch.len_utf8();
    }
    cleaned
}

fn push_capped(out: &mut String, text: &str) -> bool {
    let room = 500usize.saturating_sub(out.len());
    if text.len() <= room {
        out.push_str(text);
        return true;
    }
    let mut cut = room;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    out.push_str(&text[..cut]);
    false
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn scrub_drops_userinfo_and_caps_the_length() {
        assert_eq!(
            scrub("connect to https://user:secret@grid.example/path failed"),
            "connect to https://grid.example/path failed"
        );
        assert_eq!(scrub("http://grid.example/a"), "http://grid.example/a");
        let long = "x".repeat(800);
        assert_eq!(scrub(&long).len(), 500);
    }
}
