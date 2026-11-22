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
//   Source/OpenSim.Server.Handlers/Authorization/AuthorizationServerPostHandler.cs
//   Source/OpenSim.Services.Interfaces/IAuthorizationService.cs
//   Source/OpenSim.Server.Base/ServerUtils.cs SerializeResult
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `POST /authorization`.
//!
//! `IsAuthorizedForRegion` always returns true. The handler ignores that
//! message and writes `{ID} has been authorized`. `SerializeResult` uses
//! `XmlTextWriter` with `Util.UTF8`, which is UTF-8 with a BOM, LF and a
//! two-space indent. A document this parser cannot read is the same 500 the
//! uncaught `XmlSerializer` exception becomes. A missing `ID` is null, and
//! C# string concatenation treats that as empty.

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ferroan_http::MAX_QUERY_BYTES;

pub fn router() -> Router {
    Router::new()
        .route("/authorization", post(post_authorization))
        .route("/authorization/", post(post_authorization))
}

async fn post_authorization(request: Request) -> Response {
    let bytes = match axum::body::to_bytes(request.into_body(), MAX_QUERY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return plain(StatusCode::PAYLOAD_TOO_LARGE),
    };
    match authorization_body(&bytes) {
        Ok(body) => xml(StatusCode::OK, body),
        Err(()) => xml(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()),
    }
}

fn authorization_body(bytes: &[u8]) -> Result<Vec<u8>, ()> {
    let id = read_id(bytes)?;
    Ok(response_bytes(&id))
}

fn response_bytes(id: &str) -> Vec<u8> {
    let message = xml_escape(&format!("{id} has been authorized"));
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<AuthorizationResponse xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n  <IsAuthorized>true</IsAuthorized>\n  <Message>{message}</Message>\n</AuthorizationResponse>"
    );
    let mut body = Vec::with_capacity(3 + text.len());
    body.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    body.extend(text.into_bytes());
    body
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// The `ID` text, or empty when the element is absent.
fn read_id(bytes: &[u8]) -> Result<String, ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut scan = Scan { text, index: 0 };
    scan.skip_ws();
    scan.skip_declaration()?;
    scan.skip_ws();
    let (name, empty) = scan.start_tag()?;
    if name != "AuthorizationRequest" {
        return Err(());
    }
    if empty {
        return Ok(String::new());
    }
    let mut id = None;
    loop {
        scan.skip_ws();
        if scan.starts_with("</") {
            break;
        }
        if !scan.starts_with("<") {
            return Err(());
        }
        let (child, child_empty) = scan.start_tag()?;
        if !is_request_field(&child) {
            return Err(());
        }
        let value = if child_empty {
            String::new()
        } else {
            let raw = scan.text_until_markup()?;
            let end = format!("</{child}>");
            if !scan.consume(&end) {
                return Err(());
            }
            decode_text(&raw)?
        };
        if child == "ID" {
            if id.is_some() {
                return Err(());
            }
            id = Some(value);
        }
    }
    if !scan.consume("</AuthorizationRequest>") {
        return Err(());
    }
    scan.skip_ws();
    if scan.index != scan.text.len() {
        return Err(());
    }
    Ok(id.unwrap_or_default())
}

fn is_request_field(name: &str) -> bool {
    matches!(
        name,
        "ID" | "FirstName" | "SurName" | "Email" | "RegionName" | "RegionID"
    )
}

struct Scan<'a> {
    text: &'a str,
    index: usize,
}

impl Scan<'_> {
    fn rest(&self) -> &str {
        &self.text[self.index..]
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.rest().starts_with(prefix)
    }

    fn consume(&mut self, prefix: &str) -> bool {
        if self.starts_with(prefix) {
            self.index += prefix.len();
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.rest().chars().next() {
            if c.is_whitespace() {
                self.index += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn skip_declaration(&mut self) -> Result<(), ()> {
        if self.starts_with("<?") {
            let Some(end) = self.rest().find("?>") else {
                return Err(());
            };
            self.index += end + 2;
        }
        Ok(())
    }

    fn start_tag(&mut self) -> Result<(String, bool), ()> {
        if !self.consume("<") {
            return Err(());
        }
        let name = self.read_name()?;
        let mut quote = None;
        while self.index < self.text.len() {
            let c = self.rest().chars().next().ok_or(())?;
            let width = c.len_utf8();
            if let Some(mark) = quote {
                if c == mark {
                    quote = None;
                }
                self.index += width;
                continue;
            }
            match c {
                '"' | '\'' => {
                    quote = Some(c);
                    self.index += width;
                }
                '/' => {
                    if self.consume("/>") {
                        return Ok((name, true));
                    }
                    return Err(());
                }
                '>' => {
                    self.index += 1;
                    return Ok((name, false));
                }
                _ => self.index += width,
            }
        }
        Err(())
    }

    fn read_name(&mut self) -> Result<String, ()> {
        let mut name = String::new();
        while let Some(c) = self.rest().chars().next() {
            if !(c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-') {
                break;
            }
            name.push(c);
            self.index += c.len_utf8();
        }
        if name.is_empty() { Err(()) } else { Ok(name) }
    }

    fn text_until_markup(&mut self) -> Result<String, ()> {
        let Some(at) = self.rest().find('<') else {
            return Err(());
        };
        let text = self.rest()[..at].to_string();
        self.index += at;
        Ok(text)
    }
}

fn decode_text(raw: &str) -> Result<String, ()> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if let Some(entity) = rest.strip_prefix("&amp;") {
            out.push('&');
            rest = entity;
        } else if let Some(entity) = rest.strip_prefix("&lt;") {
            out.push('<');
            rest = entity;
        } else if let Some(entity) = rest.strip_prefix("&gt;") {
            out.push('>');
            rest = entity;
        } else if let Some(entity) = rest.strip_prefix("&quot;") {
            out.push('"');
            rest = entity;
        } else if let Some(entity) = rest.strip_prefix("&apos;") {
            out.push('\'');
            rest = entity;
        } else {
            return Err(());
        }
    }
    out.push_str(rest);
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_captured_request_answers_the_captured_document() {
        let request = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<AuthorizationRequest xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n  <ID>11111111-1111-1111-1111-111111111111</ID>\n  <FirstName>Test</FirstName>\n  <SurName>User</SurName>\n  <Email>a@b.invalid</Email>\n  <RegionName>Region</RegionName>\n  <RegionID>22222222-2222-2222-2222-222222222222</RegionID>\n</AuthorizationRequest>";
        let body = authorization_body(request.as_bytes()).expect("request");
        let text = String::from_utf8(body).expect("utf-8");
        assert!(text.starts_with('\u{feff}'));
        assert!(text.contains("<IsAuthorized>true</IsAuthorized>"));
        assert!(text.contains(
            "<Message>11111111-1111-1111-1111-111111111111 has been authorized</Message>"
        ));
        assert!(text.contains("encoding=\"utf-8\""));
    }

    #[test]
    fn a_missing_id_is_an_empty_prefix_and_junk_is_rejected() {
        let empty = authorization_body(
            b"<?xml version=\"1.0\" encoding=\"utf-8\"?><AuthorizationRequest></AuthorizationRequest>",
        )
        .expect("empty");
        let text = String::from_utf8(empty).expect("utf-8");
        assert!(text.contains("<Message> has been authorized</Message>"));
        assert!(authorization_body(b"<nope/>").is_err());
        assert!(
            authorization_body(b"<AuthorizationRequest><Other></Other></AuthorizationRequest>")
                .is_err()
        );
        let escaped =
            authorization_body(b"<AuthorizationRequest><ID>a&amp;b</ID></AuthorizationRequest>")
                .expect("escaped");
        let text = String::from_utf8(escaped).expect("utf-8");
        assert!(text.contains("<Message>a&amp;b has been authorized</Message>"));
    }
}
