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
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Avatar/RemoteAvatarServiceConnector.cs
//   Source/OpenSim.Services.Connectors/Avatar/AvatarServicesConnector.cs (GetAvatar)
//   Source/OpenSim.Services.Interfaces/IAvatarService.cs (ToAvatarAppearance)
//   Source/OpenSim.Server.Base/ProtocolVersions.cs
//   KanikaBreed/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `getavatar` the way `AvatarServicesConnector` calls the C# avatar service.
//! An empty store is the constructor appearance. A failed call keeps those
//! defaults and does not refuse the login: C# would throw from `GetAppearance`
//! on a null row, and this region still has to seat the avatar.

use ferroan_client::{AvatarWire, StoredAppearance};
use ferroan_http::{Field, FieldMap};
use uuid::Uuid;

use crate::grid::{GridError, service_fields, service_form};

#[derive(Debug, thiserror::Error)]
pub enum AvatarError {
    #[error(transparent)]
    Http(#[from] ferroan_http::HttpError),
    #[error("avatar service reply has no appearance")]
    InvalidReply,
    #[error("avatar service appearance could not be read")]
    InvalidAppearance,
}

/// Fetch when `AvatarServerURI` is set. An empty URI is the constructor appearance.
/// The caller assigns the region local ID after attaching to the frame and before sending.
pub async fn load(uri: &str, agent_id: Uuid, first_name: &str, last_name: &str) -> AvatarWire {
    let mut wire = AvatarWire::standing(agent_id, 0, first_name, last_name);
    if uri.is_empty() {
        return wire;
    }
    match fetch(uri, agent_id).await {
        Ok(stored) => wire.apply_stored(&stored),
        Err(error) => tracing::error!("ferroan-region: avatar getavatar: {error}"),
    }
    wire
}

async fn fetch(uri: &str, agent_id: Uuid) -> Result<StoredAppearance, AvatarError> {
    let endpoint = format!("{}/avatar", uri.trim_end_matches('/'));
    let mut fields = service_fields("getavatar");
    fields.insert("UserID".into(), Field::Str(agent_id.to_string()));
    let reply = service_form(&endpoint, &fields)
        .await
        .map_err(avatar_error)?;
    parse_appearance(&reply)
}

fn avatar_error(error: GridError) -> AvatarError {
    match error {
        GridError::Http(http) => AvatarError::Http(http),
        GridError::Rejected { .. } | GridError::InvalidReply => AvatarError::InvalidReply,
    }
}

pub(crate) fn parse_appearance(reply: &FieldMap) -> Result<StoredAppearance, AvatarError> {
    let Some(result) = reply.get("result") else {
        return Err(AvatarError::InvalidReply);
    };
    match result {
        Field::Null => Ok(StoredAppearance::default()),
        Field::Str(text) if text.eq_ignore_ascii_case("null") => Ok(StoredAppearance::default()),
        Field::Map(data) => stored_from_map(data),
        _ => Err(AvatarError::InvalidReply),
    }
}

fn stored_from_map(data: &FieldMap) -> Result<StoredAppearance, AvatarError> {
    // AvatarType is outside Data. An empty Data map is the constructor appearance.
    let visual_params = match data.get("VisualParams") {
        None => None,
        Some(field) => Some(parse_params(&field_text(field)?)?),
    };
    let height = match data.get("AvatarHeight") {
        None => None,
        Some(field) => Some(parse_height(&field_text(field)?)?),
    };
    Ok(StoredAppearance {
        visual_params,
        height,
    })
}

fn field_text(field: &Field) -> Result<String, AvatarError> {
    match field {
        Field::Str(text) => Ok(text.clone()),
        Field::Int(number) => Ok(number.to_string()),
        _ => Err(AvatarError::InvalidAppearance),
    }
}

fn parse_params(text: &str) -> Result<Vec<u8>, AvatarError> {
    let mut params = Vec::new();
    for part in text.split(',') {
        let value = part
            .trim()
            .parse::<i32>()
            .map_err(|_| AvatarError::InvalidAppearance)?;
        params.push(value as u8);
    }
    if params.is_empty() {
        return Err(AvatarError::InvalidAppearance);
    }
    Ok(params)
}

fn parse_height(text: &str) -> Result<f32, AvatarError> {
    // `AvatarData.TryParseHeight` (Tranquillity #520, TB44): a C# peer writes
    // the height in its own culture, so a value with no dot and at most one
    // comma reads the comma as the decimal mark.
    let trimmed = text.trim();
    let dotted;
    let text = if !trimmed.contains('.') && trimmed.matches(',').count() <= 1 {
        dotted = trimmed.replace(',', ".");
        dotted.as_str()
    } else {
        trimmed
    };
    let height = text
        .parse::<f32>()
        .map_err(|_| AvatarError::InvalidAppearance)?;
    if !height.is_finite() {
        return Err(AvatarError::InvalidAppearance);
    }
    // ToAvatarAppearance replaces a stored 0 with the constructor height.
    if height == 0.0 { Ok(1.9) } else { Ok(height) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comma_height_reads_as_a_decimal() {
        assert_eq!(parse_height("1,8125").unwrap(), 1.8125);
        assert_eq!(parse_height("1.8125").unwrap(), 1.8125);
        assert_eq!(parse_height("0").unwrap(), 1.9);
        assert!(parse_height("1,8,1").is_err());
        assert!(parse_height("1.8,1").is_err());
    }

    #[test]
    fn empty_store_keeps_constructor_fields_absent() {
        let xml = "<ServerResponse><result type=\"List\"><AvatarType>1</AvatarType></result></ServerResponse>";
        let reply = ferroan_http::parse_xml_response(xml).unwrap();
        let stored = parse_appearance(&reply).unwrap();
        assert!(stored.visual_params.is_none());
        assert!(stored.height.is_none());
    }

    #[test]
    fn stored_params_and_zero_height_follow_to_avatar_appearance() {
        let xml = "<ServerResponse><result type=\"List\"><AvatarType>1</AvatarType><AvatarHeight>0</AvatarHeight><VisualParams>9, 300</VisualParams></result></ServerResponse>";
        let reply = ferroan_http::parse_xml_response(xml).unwrap();
        let stored = parse_appearance(&reply).unwrap();
        assert_eq!(stored.visual_params.unwrap(), vec![9, 44]);
        assert_eq!(stored.height.unwrap(), 1.9);
    }

    #[test]
    fn a_bad_param_list_is_rejected() {
        let xml = "<ServerResponse><result type=\"List\"><VisualParams>nope</VisualParams></result></ServerResponse>";
        let reply = ferroan_http::parse_xml_response(xml).unwrap();
        assert!(parse_appearance(&reply).is_err());
    }
}
