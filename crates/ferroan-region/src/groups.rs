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
// Ported from Addons/OpenSim.Addons.Groups/{GroupsModule,GroupsExtendedData}.cs,
// Remote/{GroupsServiceRemoteConnectorModule,GroupsServiceRemoteConnector,GroupsSecretKey}.cs,
// Local/GroupsServiceLocalConnectorModule.cs, Hypergrid/GroupsServiceHGConnectorModule.cs,
// Source/OpenSim.Region.OptionalModules/Avatar/XmlRpcGroups/{GroupsModule,XmlRpcGroupsServicesConnectorModule}.cs
// and Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs (RefreshGroupMembership)
// at Tranquillity c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The remote Groups V2 membership read used by the region's parcel code.
use crate::{
    config::{RegionConfigError, boolean, check_uri, value},
    grid::GridError,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ferroan_config::Config;
use ferroan_http::{Field, FieldMap, FormOptions, post_form_with_client};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use std::{collections::BTreeMap, time::Duration};
use uuid::Uuid;

/// No Debug: the dedicated HTTP client can carry the configured service key.
#[derive(Clone)]
pub struct GroupsClient(Option<Remote>);

#[derive(Clone)]
struct Remote {
    endpoint: String,
    http: reqwest::Client,
}

impl GroupsClient {
    pub fn from_config(config: &Config) -> Result<Self, RegionConfigError> {
        if config.get("Modules", "Setup_GroupsModule") == Some("disabled")
            || !boolean(config, "Groups", "Enabled")?
        {
            return Ok(Self(None));
        }
        let module = config.get("Groups", "Module");
        let connector = config.get("Groups", "ServicesConnectorModule");
        let setup = match (module, connector) {
            (Some("Groups Module V2"), Some("Groups Remote Service Connector")) => {
                "Setup_GroupsServiceRemoteConnectorModule"
            }
            (Some("Groups Module V2"), Some("Groups Local Service Connector")) => {
                "Setup_GroupsServiceLocalConnectorModule"
            }
            (Some("Groups Module V2"), Some("Groups HG Service Connector")) => {
                "Setup_GroupsServiceHGConnectorModule"
            }
            // The legacy module defaults this key to its XML-RPC connector.
            (Some("GroupsModule"), None | Some("XmlRpcGroupsServicesConnector")) => {
                "Setup_XmlRpcGroupsServicesConnectorModule"
            }
            _ => return Ok(Self(None)),
        };
        if config.get("Modules", setup) == Some("disabled") {
            return Ok(Self(None));
        }
        if connector != Some("Groups Remote Service Connector") {
            return Err(value(
                "Groups",
                "ServicesConnectorModule",
                "unsupported group membership source; supported pair: Groups Module V2 with Groups Remote Service Connector",
            ));
        }
        let uri = config.get("Groups", "GroupsServerURI").unwrap_or_default();
        check_uri(uri).map_err(|why| value("Groups", "GroupsServerURI", why))?;
        let setting = |key| {
            config
                .get("Groups", key)
                .or_else(|| config.get("Network", key))
                .unwrap_or_default()
        };
        let mut headers = HeaderMap::new();
        if setting("AuthType") == "BasicHttpAuthentication" {
            let credentials = format!(
                "{}:{}",
                setting("HttpAuthUsername"),
                setting("HttpAuthPassword")
            );
            let encoded = format!("Basic {}", STANDARD.encode(credentials));
            let mut auth = HeaderValue::from_str(&encoded)
                .map_err(|_| value("Groups", "AuthType", "invalid credentials"))?;
            auth.set_sensitive(true);
            headers.insert(AUTHORIZATION, auth);
        }
        if let Some(key) = config.get("Groups", "SecretKey").filter(|s| !s.is_empty()) {
            let mut header = HeaderValue::from_str(key)
                .map_err(|_| value("Groups", "SecretKey", "invalid HTTP header value"))?;
            header.set_sensitive(true);
            headers.insert("x-opensim-groups-key", header);
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| value("Groups", "GroupsServerURI", "could not open HTTP client"))?;
        Ok(Self(Some(Remote {
            endpoint: format!("{}/groups", uri.trim_end_matches('/')),
            http,
        })))
    }

    /// All group IDs, including inactive memberships with no powers. Pin8's
    /// viewer tests membership by key and uses the unsigned mask for rights.
    pub async fn memberships(&self, agent: Uuid) -> Result<BTreeMap<Uuid, u64>, GridError> {
        let Some(remote) = &self.0 else {
            return Ok(BTreeMap::new());
        };
        let fields = FieldMap::from_iter([
            ("AgentID".into(), Field::Str(agent.to_string())),
            ("ALL".into(), Field::Str("true".into())),
            (
                "RequestingAgentID".into(),
                Field::Str(Uuid::nil().to_string()),
            ),
            ("METHOD".into(), Field::Str("GETMEMBERSHIP".into())),
        ]);
        let response = post_form_with_client(
            &remote.http,
            &remote.endpoint,
            &fields,
            FormOptions::checked(Duration::from_secs(30), 1024 * 1024),
        )
        .await?;
        memberships(&response)
    }
}

fn memberships(response: &FieldMap) -> Result<BTreeMap<Uuid, u64>, GridError> {
    let mut result = BTreeMap::new();
    let rows = match response.get("RESULT") {
        None => return Ok(result),
        Some(Field::Str(s)) if s == "NULL" => return Ok(result),
        Some(Field::Map(rows)) => rows,
        _ => return Err(GridError::InvalidReply),
    };
    for row in rows.values() {
        let Field::Map(row) = row else {
            return Err(GridError::InvalidReply);
        };
        let group = match row.get("GroupID") {
            None => Uuid::nil(),
            Some(Field::Str(s)) => s.parse().map_err(|_| GridError::InvalidReply)?,
            _ => return Err(GridError::InvalidReply),
        };
        let powers = match row.get("GroupPowers") {
            None => 0,
            Some(Field::Str(s)) => s
                .trim_matches(|c| matches!(c, '\t'..='\r' | ' '))
                .parse()
                .map_err(|_| GridError::InvalidReply)?,
            _ => return Err(GridError::InvalidReply),
        };
        result.insert(group, powers);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMBER: &str = include_str!("../../../fixtures/group-memberships/member.xml");
    const OWNER: &str = include_str!("../../../fixtures/group-memberships/owner.xml");
    const NONMEMBER: &str = include_str!("../../../fixtures/group-memberships/nonmember.xml");

    #[test]
    fn all_memberships_and_unsigned_powers_match_pin8() {
        let east = Uuid::from_u128(10);
        for (xml, expected) in [
            (
                MEMBER,
                BTreeMap::from([(east, 33554440), (Uuid::from_u128(11), 0)]),
            ),
            (NONMEMBER, BTreeMap::new()),
            (OWNER, BTreeMap::from([(east, u64::MAX)])),
        ] {
            assert_eq!(
                memberships(&ferroan_http::parse_xml_response(xml).unwrap()).unwrap(),
                expected
            );
        }
        for xml in [
            MEMBER.replace("33554440", "18446744073709551616"),
            MEMBER.replace(&east.to_string(), "broken"),
        ] {
            assert!(memberships(&ferroan_http::parse_xml_response(&xml).unwrap()).is_err());
        }
    }

    fn groups() -> Config {
        Config::parse("[Groups]\nEnabled=true\nModule=Groups Module V2\nServicesConnectorModule=Groups Remote Service Connector\n").unwrap()
    }

    #[tokio::test]
    async fn disabled_groups_and_missing_connectors_supply_no_memberships() {
        assert!(matches!(
            GroupsClient::from_config(&groups()),
            Err(RegionConfigError::Value {
                key: "GroupsServerURI",
                ..
            })
        ));
        let without_connector =
            Config::parse("[Groups]\nEnabled=true\nModule=Groups Module V2").unwrap();
        assert!(
            GroupsClient::from_config(&without_connector)
                .unwrap()
                .memberships(Uuid::nil())
                .await
                .unwrap()
                .is_empty()
        );
        for (section, key, val) in [
            ("Groups", "Enabled", "false"),
            ("Groups", "Module", "Default"),
            ("Groups", "ServicesConnectorModule", "unknown"),
            ("Groups", "ServicesConnectorModule", ""),
            ("Modules", "Setup_GroupsModule", "disabled"),
            (
                "Modules",
                "Setup_GroupsServiceRemoteConnectorModule",
                "disabled",
            ),
        ] {
            let mut config = groups();
            config.set(section, key, val);
            let source = GroupsClient::from_config(&config).unwrap();
            assert!(source.memberships(Uuid::nil()).await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn enabled_unsupported_membership_sources_refuse_region_startup() {
        for (module, connector, setup) in [
            (
                "Groups Module V2",
                Some("Groups Local Service Connector"),
                "Setup_GroupsServiceLocalConnectorModule",
            ),
            (
                "Groups Module V2",
                Some("Groups HG Service Connector"),
                "Setup_GroupsServiceHGConnectorModule",
            ),
            (
                "GroupsModule",
                Some("XmlRpcGroupsServicesConnector"),
                "Setup_XmlRpcGroupsServicesConnectorModule",
            ),
            (
                "GroupsModule",
                None,
                "Setup_XmlRpcGroupsServicesConnectorModule",
            ),
        ] {
            let mut config =
                Config::parse("[Groups]\nEnabled=true\nGroupsServerURI=http://groups.invalid")
                    .unwrap();
            config.set("Groups", "Module", module);
            if let Some(name) = connector {
                config.set("Groups", "ServicesConnectorModule", name);
            }
            // Disabling the Remote connector must not hide the selected Local or HG service.
            config.set(
                "Modules",
                "Setup_GroupsServiceRemoteConnectorModule",
                "disabled",
            );
            assert!(
                matches!(
                    GroupsClient::from_config(&config),
                    Err(RegionConfigError::Value {
                        key: "ServicesConnectorModule",
                        ..
                    })
                ),
                "{module} {connector:?}"
            );
            config.set("Modules", setup, "disabled");
            let source = GroupsClient::from_config(&config).unwrap();
            assert!(source.memberships(Uuid::nil()).await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn membership_redirect_reports_the_original_status() {
        use axum::{Router, response::Redirect, routing::post};
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = groups();
        config.set(
            "Groups",
            "GroupsServerURI",
            &format!("http://{}", socket.local_addr().unwrap()),
        );
        let app = Router::new().route(
            "/groups",
            post(|| async { Redirect::temporary("/elsewhere") }),
        );
        let task = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let result = GroupsClient::from_config(&config)
            .unwrap()
            .memberships(Uuid::nil())
            .await;
        task.abort();
        assert!(matches!(
            result,
            Err(GridError::Http(ferroan_http::HttpError::Status {
                status: 307
            }))
        ));
    }

    #[tokio::test]
    async fn authenticated_request_uses_the_configured_path_and_both_credentials() {
        use axum::{Router, http::StatusCode, routing::post};
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = groups();
        config.set(
            "Groups",
            "GroupsServerURI",
            &format!("http://{}/private/", socket.local_addr().unwrap()),
        );
        let key = Uuid::new_v4().to_string();
        let user = Uuid::new_v4().to_string();
        let password = Uuid::new_v4().to_string();
        config.set("Network", "AuthType", "None");
        config.set("Groups", "AuthType", "BasicHttpAuthentication");
        config.set("Network", "HttpAuthUsername", &user);
        config.set("Groups", "HttpAuthPassword", &password);
        config.set("Groups", "SecretKey", &key);
        let auth = format!("Basic {}", STANDARD.encode(format!("{user}:{password}")));
        let agent = Uuid::new_v4();
        let app = Router::new().route(
            "/private/groups",
            post(move |headers: HeaderMap, body: String| async move {
                if headers
                    .get("x-opensim-groups-key")
                    .and_then(|v| v.to_str().ok())
                    == Some(&key)
                    && headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(&auth)
                    && body
                        == format!(
                            "AgentID={agent}&ALL=true&RequestingAgentID={}&METHOD=GETMEMBERSHIP",
                            Uuid::nil()
                        )
                {
                    (StatusCode::OK, OWNER)
                } else {
                    (StatusCode::FORBIDDEN, "")
                }
            }),
        );
        let task = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = GroupsClient::from_config(&config).unwrap();
        let result = client.memberships(agent).await;
        task.abort();
        assert_eq!(
            result.unwrap(),
            BTreeMap::from([(Uuid::from_u128(10), u64::MAX)])
        );
    }
}
