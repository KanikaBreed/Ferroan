// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Robust HTTP forms, XML replies and LLSD bodies.
//!
//! Query strings and `<ServerResponse>` documents follow Tranquillity
//! `Source/OpenSim.Server.Base/ServerUtils.cs` at
//! [`REFERENCE_TRANQUILLITY_REVISION`]. The comparisons are the files under
//! `fixtures/http`, produced by `cargo xtask http`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod client;
mod error;
mod llsd_http;
mod nodelay;
mod query;
mod service;
mod value;
mod xml;

pub use client::{
    FormOptions, FormStatusPolicy, post_form, post_form_with, post_form_with_client, post_llsd_xml,
};
pub use error::HttpError;
pub use llsd_http::{LlsdBody, llsd_from_http_body, llsd_response, llsd_response_status};
pub use nodelay::{accepted_nodelay, set_tcp_nodelay};
pub use query::{build_query_string, parse_query_string, parse_query_string_limited};
pub use service::{BodyLimit, FormBody, XmlResponse, form_router, read_body, xml_response};
pub use value::{Field, FieldMap};
pub use xml::{build_xml_response, parse_xml_response, parse_xml_response_limited};

/// Tranquillity revision of the committed `fixtures/http` documents.
/// `query.rs` and `xml.rs` were ported from `713857f`; regenerating at this
/// pin left those case bytes unchanged.
pub const REFERENCE_TRANQUILLITY_REVISION: &str = "c70fee205f64ee8672a8d607cef6a733931da951";

/// `ParseXmlResponse` sets `XmlReaderSettings.MaxCharactersInDocument` to this.
pub const MAX_XML_CHARS: usize = 50_000_000;

/// Nesting Ferroan refuses. Tranquillity's reader has no depth cap and would
/// recurse until the stack overflows. Robust replies are a few maps deep.
pub const MAX_XML_DEPTH: usize = 64;

/// Query strings and HTTP bodies larger than this are refused. Tranquillity
/// does not cap a query string. The XML character cap above is the body cap
/// for a reply.
pub const MAX_QUERY_BYTES: usize = 1024 * 1024;

/// Keys in one map, including list items added one at a time.
pub const MAX_FIELDS: usize = 4096;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_revision_is_a_full_commit_hash() {
        assert_eq!(REFERENCE_TRANQUILLITY_REVISION.len(), 40);
        assert!(
            REFERENCE_TRANQUILLITY_REVISION
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
    }
}
