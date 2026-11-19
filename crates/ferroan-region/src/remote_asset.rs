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
//   Source/OpenSim.Services.Connectors/Asset/AssetServicesConnector.cs
//   Source/OpenSim.Server.Handlers/Asset/AssetServerPostHandler.cs
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! HTTP asset GET and POST against Robust. The region keeps no asset store.
//! Bodies are capped at 16 MiB. Failures do not include the reply.

use std::time::Duration;

use ferroan_services::{Asset, decode_asset, encode_asset};
use uuid::Uuid;

pub const MAX_ASSET_BODY: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct AssetClient {
    base: String,
    http: reqwest::Client,
}

impl AssetClient {
    pub fn new(service_uri: &str, http: reqwest::Client) -> Option<Self> {
        let base = service_uri.trim().trim_end_matches('/').to_owned();
        if base.is_empty() {
            None
        } else {
            Some(Self { base, http })
        }
    }

    /// Missing, empty, or undecodable assets are `Ok(None)`.
    pub async fn get(&self, id: Uuid) -> Result<Option<Asset>, ()> {
        let response = self
            .http
            .get(format!("{}/assets/{id}", self.base))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let bytes = read_limited(response).await?;
        let decoded = decode_asset(&bytes).map_err(|_| ())?;
        if decoded.data_nil || decoded.asset.data.is_empty() {
            return Ok(None);
        }
        Ok(Some(decoded.asset))
    }

    /// POST an `AssetBase` document. Success requires the service to echo the id.
    pub async fn store(&self, asset: &Asset) -> Result<bool, ()> {
        let response = self
            .http
            .post(format!("{}/assets/", self.base))
            .timeout(Duration::from_secs(30))
            .header(reqwest::header::CONTENT_TYPE, "text/xml")
            .body(encode_asset(asset))
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            return Ok(false);
        }
        let bytes = read_limited(response).await?;
        Ok(xml_string(&bytes).is_some_and(|text| {
            Uuid::parse_str(text.trim())
                .ok()
                .is_some_and(|id| id == asset.id)
        }))
    }
}

async fn read_limited(mut response: reqwest::Response) -> Result<Vec<u8>, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ASSET_BODY as u64)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_ASSET_BODY {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn xml_string(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    let start = text.find("<string")?;
    let open = text[start..].find('>')? + start + 1;
    let end = text[open..].find("</string>")? + open;
    Some(text[open..end].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_serializer_string_document_yields_the_id() {
        let id = "00000000-0000-4000-8000-0000000000aa";
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<string xmlns=\"http://schemas.microsoft.com/2003/10/Serialization/\">{id}</string>"
        );
        assert_eq!(xml_string(document.as_bytes()), Some(id));
    }
}
