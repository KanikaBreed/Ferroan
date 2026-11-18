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
//   Source/OpenSim.Services.Connectors/Grid/GridServicesConnector.cs
//   Source/OpenSim.Services.Interfaces/IGridService.cs (GridRegion.ToKeyValuePairs)
//   Source/OpenSim.Server.Base/ProtocolVersions.cs
//   Read with Source/OpenSim.Server.Handlers/Grid/GridServerPostHandler.cs
//   and Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Grid/RegionGridServiceConnector.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Register and deregister this process's single region through C# Robust.
//! Transport has ferroan-http's 30 second deadline and bounded XML reader.
//! No retries: startup fails when registration is refused or unconfirmed.

use ferroan_http::{Field, FieldMap};
use uuid::Uuid;

use crate::config::RegionConfig;

#[derive(Debug, thiserror::Error)]
pub enum GridError {
    #[error(transparent)]
    Http(#[from] ferroan_http::HttpError),
    #[error("grid service refused the request: {message}")]
    Rejected { message: String },
    #[error("grid service reply has no recognized Result")]
    InvalidReply,
}

pub struct GridClient {
    endpoint: String,
}

impl GridClient {
    pub fn new(config: &RegionConfig) -> Self {
        // The reference appends to serviceURI, not its trimmed m_ServerURI.
        Self {
            endpoint: format!("{}/grid", config.grid_server_uri),
        }
    }

    pub async fn register(&self, config: &RegionConfig) -> Result<(), GridError> {
        check_result(&ferroan_http::post_form(&self.endpoint, &registration_fields(config)).await?)
    }

    pub async fn deregister(&self, region_id: Uuid) -> Result<(), GridError> {
        check_result(
            &ferroan_http::post_form(&self.endpoint, &deregistration_fields(region_id)).await?,
        )
    }
}

/// The insertion order is GridServicesConnector followed by GridRegion.
pub fn registration_fields(config: &RegionConfig) -> FieldMap {
    let mut fields = FieldMap::new();
    for (key, value) in [
        ("SCOPEID", config.scope_id.to_string()),
        ("VERSIONMIN", "1".to_string()),
        ("VERSIONMAX", "1".to_string()),
        ("METHOD", "register".to_string()),
        ("uuid", config.id.to_string()),
        ("locX", (u64::from(config.location[0]) * 256).to_string()),
        ("locY", (u64::from(config.location[1]) * 256).to_string()),
        ("sizeX", config.size[0].to_string()),
        ("sizeY", config.size[1].to_string()),
        ("regionName", config.name.clone()),
        ("serverIP", config.external_host.clone()),
        ("serverHttpPort", config.http_bind.port().to_string()),
        ("serverURI", config.server_uri.clone()),
        ("serverPort", config.udp_bind.port().to_string()),
        ("regionMapTexture", Uuid::nil().to_string()),
        ("parcelMapTexture", Uuid::nil().to_string()),
        ("access", "13".to_string()),
        ("regionSecret", config.region_secret.to_string()),
        ("owner_uuid", config.estate_owner.to_string()),
        ("Token", String::new()),
    ] {
        fields.insert(key.to_owned(), Field::Str(value));
    }
    fields
}

pub fn deregistration_fields(region_id: Uuid) -> FieldMap {
    FieldMap::from_iter([
        ("REGIONID".to_owned(), Field::Str(region_id.to_string())),
        ("METHOD".to_owned(), Field::Str("deregister".to_owned())),
    ])
}

fn check_result(reply: &FieldMap) -> Result<(), GridError> {
    match reply.get("Result") {
        Some(Field::Str(result)) if result.eq_ignore_ascii_case("success") => Ok(()),
        Some(Field::Str(result)) if result.eq_ignore_ascii_case("failure") => {
            let message = match reply.get("Message") {
                Some(Field::Str(message)) => message.chars().take(1024).collect(),
                _ => "no reason supplied".to_owned(),
            };
            Err(GridError::Rejected { message })
        }
        _ => Err(GridError::InvalidReply),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_and_deregistration_match_the_csharp_grid_capture() {
        let server = ferroan_config::Config::parse(
            "[Network]\nhttp_listener_port=9000\nbind_ip_address=127.0.0.2\n\
             [GridService]\nGridServerURI=http://192.0.2.1:8003\n\
             [Estates]\nDefaultEstateOwnerUUID=00000000-0000-4000-8000-000000000001\n\
             [Map]\nGenerateMaptiles=false\n",
        )
        .unwrap();
        let regions = ferroan_config::Config::parse(
            "[Fixture Region]\nRegionUUID=00000000-0000-4000-8000-000000000002\n\
             Location=1000,1000\nInternalAddress=127.0.0.1\nInternalPort=9000\n\
             ExternalHostName=192.0.2.1\n",
        )
        .unwrap();
        let mut config = RegionConfig::from_configs(&server, &regions).unwrap();
        // The captured process's generated secret has this fixture placeholder.
        config.region_secret = Uuid::parse_str("00000000-0000-4000-8000-000000000003").unwrap();
        assert_eq!(
            ferroan_http::build_query_string(&registration_fields(&config)).unwrap(),
            include_str!("../../../fixtures/region-login/register.request.txt"),
        );
        assert_eq!(
            ferroan_http::build_query_string(&deregistration_fields(config.id)).unwrap(),
            include_str!("../../../fixtures/region-login/deregister.request.txt"),
        );
        for reply in [
            include_str!("../../../fixtures/region-login/register.response.txt"),
            include_str!("../../../fixtures/region-login/deregister.response.txt"),
        ] {
            check_result(&ferroan_http::parse_xml_response(reply).unwrap()).unwrap();
        }
    }

    #[test]
    fn failed_or_malformed_grid_replies_never_report_registration_success() {
        for xml in [
            "<ServerResponse/>",
            "<ServerResponse><Result>unexpected</Result></ServerResponse>",
            "<ServerResponse><Result type=\"List\"><nested>Success</nested></Result></ServerResponse>",
        ] {
            assert!(check_result(&ferroan_http::parse_xml_response(xml).unwrap()).is_err());
        }
        let fields = ferroan_http::parse_xml_response("<ServerResponse><Result>Failure</Result><Message>Region overlaps another region</Message></ServerResponse>").unwrap();
        assert!(
            matches!(check_result(&fields), Err(GridError::Rejected { message }) if message == "Region overlaps another region")
        );
        let fields = ferroan_http::parse_xml_response(
            "<ServerResponse><Result>Success</Result></ServerResponse>",
        )
        .unwrap();
        assert!(check_result(&fields).is_ok());
    }
}
