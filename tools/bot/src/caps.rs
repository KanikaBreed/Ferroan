// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-net/src/caps.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// The POST is the viewer's seed request: an LLSD array of names, answered
// with a map of URLs. The viewer's full list and its retry loop are not
// included.

//! The seed capability.

use std::collections::HashMap;

use ferroan_llsd::{Llsd, from_xml, to_xml};

use crate::body::read_limited;

/// Capabilities the later scenarios call. The seed returns whichever of
/// these the region actually has.
pub const SEED_CAPS: &[&str] = &[
    "EventQueueGet",
    "GetTexture",
    "FetchInventoryDescendents2",
    "FetchInventory2",
    "NewFileAgentInventory",
];

/// A seed reply larger than this is refused. The answer is a short map of URLs.
pub const MAX_SEED_BODY: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CapsError {
    #[error("seed url is not http")]
    Url,
    #[error("http: {0}")]
    Http(String),
    #[error("seed status {0}")]
    Status(u16),
    #[error("seed reply is {got} bytes, over the {limit} byte limit")]
    TooLarge { got: usize, limit: usize },
    #[error("seed reply is not an LLSD map")]
    Malformed,
    #[error("llsd: {0}")]
    Llsd(String),
}

/// POST [`SEED_CAPS`] to the seed URL and return the URLs the region gave back.
pub async fn request_seed(
    http: &reqwest::Client,
    seed: &str,
) -> Result<HashMap<String, String>, CapsError> {
    let url = reqwest::Url::parse(seed).map_err(|_| CapsError::Url)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(CapsError::Url);
    }
    let names = Llsd::Array(SEED_CAPS.iter().map(|name| Llsd::from(*name)).collect());
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/llsd+xml")
        .header(reqwest::header::ACCEPT, "application/llsd+xml")
        .body(to_xml(&names))
        .send()
        .await
        .map_err(|error| CapsError::Http(error.without_url().to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(CapsError::Status(status.as_u16()));
    }
    let bytes = match read_limited(response, MAX_SEED_BODY).await {
        Ok(bytes) => bytes,
        Err(crate::body::BodyError::Transport) => {
            return Err(CapsError::Http("reading the seed reply failed".into()));
        }
        Err(crate::body::BodyError::TooLarge(got)) => {
            return Err(CapsError::TooLarge {
                got,
                limit: MAX_SEED_BODY,
            });
        }
    };
    let value = from_xml(&bytes).map_err(|error| CapsError::Llsd(error.to_string()))?;
    let Some(map) = value.as_map() else {
        return Err(CapsError::Malformed);
    };
    let mut caps = HashMap::new();
    for (name, item) in map {
        let text = item.to_string_value();
        if !text.is_empty() {
            caps.insert(name.clone(), text);
        }
    }
    Ok(caps)
}
