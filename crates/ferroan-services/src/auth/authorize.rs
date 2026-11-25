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
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `POST /authorization`.
//!
//! The connector constructs this handler with no service authenticator, so
//! `X-SecondLife-Shard` is not refused. `IsAuthorizedForRegion` always returns
//! true. The handler ignores that message and writes `{ID} has been authorized`. `SerializeResult` uses
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

fn valid_root_attribute(attribute: &::xml::attribute::OwnedAttribute) -> bool {
    if attribute.name.namespace.as_deref() != Some("http://www.w3.org/2001/XMLSchema-instance") {
        return true;
    }
    match attribute.name.local_name.as_str() {
        "type" => attribute.value == "AuthorizationRequest",
        // A true nil deserializes to null, which the handler cannot use.
        "nil" => matches!(
            attribute.value.trim_matches([' ', '\t', '\r', '\n']),
            "false" | "0"
        ),
        _ => true,
    }
}

/// Pin8's XmlSerializer reads the first unqualified occurrence of each string
/// property. Unknown and duplicate subtrees are skipped as complete XML.
fn read_id(bytes: &[u8]) -> Result<String, ()> {
    use ::xml::common::XmlVersion;
    use ::xml::reader::{ParserConfig, XmlEvent};
    use ferroan_http::MAX_XML_DEPTH;

    const FIELDS: [&str; 6] = [
        "ID",
        "FirstName",
        "SurName",
        "Email",
        "RegionName",
        "RegionID",
    ];
    let reader = ParserConfig::new()
        .allow_multiple_root_elements(false)
        .ignore_comments(false)
        .coalesce_characters(false)
        .create_reader(bytes);
    let mut seen = [false; FIELDS.len()];
    let mut field = None;
    let mut depth = 0;
    let mut keep_root_space = false;
    let mut keep_field_space = false;
    let mut id = String::new();
    for event in reader {
        match event.map_err(|_| ())? {
            XmlEvent::StartDocument { version, .. } if version != XmlVersion::Version10 => {
                return Err(());
            }
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    ferroan_diag::refuse(
                        ferroan_diag::HTTP_XML_DEPTH,
                        ferroan_diag::Kind::Hard,
                        MAX_XML_DEPTH as u64,
                        depth as u64,
                        "authorization",
                    );
                    return Err(());
                }
                let space = attributes
                    .iter()
                    .find(|attribute| {
                        attribute.name.namespace.as_deref() == Some(::xml::namespace::NS_XML_URI)
                            && attribute.name.local_name == "space"
                    })
                    .map(|attribute| attribute.value.as_str());
                let preserve = match space {
                    Some("preserve") => true,
                    Some("default") => false,
                    Some(_) => return Err(()),
                    None => keep_root_space,
                };
                match depth {
                    1 => {
                        if name.namespace.is_some()
                            || name.local_name != "AuthorizationRequest"
                            || !attributes.iter().all(valid_root_attribute)
                        {
                            return Err(());
                        }
                        keep_root_space = preserve;
                    }
                    2 if name.namespace.is_none() => {
                        if let Some(index) = FIELDS.iter().position(|&key| key == name.local_name)
                            && !seen[index]
                        {
                            seen[index] = true;
                            field = Some(index);
                            keep_field_space = preserve;
                        }
                    }
                    3 if field.is_some() => return Err(()),
                    _ => {}
                }
            }
            XmlEvent::EndElement { .. } => {
                if depth == 2 {
                    field = None;
                }
                depth -= 1;
            }
            XmlEvent::CData(value) if field == Some(0) => id.push_str(&value),
            XmlEvent::Characters(value) | XmlEvent::Whitespace(value)
                if field == Some(0)
                    && (keep_field_space
                        || !value.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))) =>
            {
                id.push_str(&value);
            }
            XmlEvent::Doctype { .. } => return Err(()),
            _ => {}
        }
    }
    Ok(id)
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
    fn depth_limit_includes_ignored_extension_children() {
        for (depth, allowed) in [
            (ferroan_http::MAX_XML_DEPTH, true),
            (ferroan_http::MAX_XML_DEPTH + 1, false),
        ] {
            let inner = depth - 1;
            let document = format!(
                "<AuthorizationRequest>{}{}</AuthorizationRequest>",
                "<X>".repeat(inner),
                "</X>".repeat(inner)
            );
            assert_eq!(authorization_body(document.as_bytes()).is_ok(), allowed);
        }
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
            authorization_body(b"<AuthorizationRequest><Other></Broken></AuthorizationRequest>")
                .is_err()
        );
        let escaped =
            authorization_body(b"<AuthorizationRequest><ID>a&amp;b</ID></AuthorizationRequest>")
                .expect("escaped");
        let text = String::from_utf8(escaped).expect("utf-8");
        assert!(text.contains("<Message>a&amp;b has been authorized</Message>"));
    }
}
