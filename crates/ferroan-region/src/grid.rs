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
//   Source/OpenSim.Services.Connectors/Presence/PresenceServicesConnector.cs
//   Source/OpenSim.Services.Connectors/GridUser/GridUserServicesConnector.cs
//   Source/OpenSim.Framework/GridInfo.cs (OSHHTPHost URI normalization)
//   Source/OpenSim.Framework/WebUtil.cs (SynchronousRestFormsRequester)
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/Presence/PresenceDetector.cs
//   Source/OpenSim.Region.CoreModules/ServiceConnectorsOut/GridUser/ActivityDetector.cs
//   Source/OpenSim.Region.Framework/Scenes/Scene.cs (VerifyUserPresence)
//   Source/OpenSim.Services.Interfaces/IGridUserService.cs (GridUserInfo)
//   Source/OpenSim.Region.Framework/Scenes/Scene.LoginArrivals.cs
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Region registration, presence verification and logout through C# Robust.
//! Transport has a 30 second deadline and ferroan-http's bounded XML reader.
//! No retries: startup fails when registration is refused or unconfirmed.

use ferroan_http::{Field, FieldMap};
use uuid::Uuid;

use crate::config::RegionConfig;

const MAX_SERVICE_REPLY: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GridError {
    #[error(transparent)]
    Http(#[from] ferroan_http::HttpError),
    #[error("grid service refused the request: {message}")]
    Rejected { message: String },
    #[error("grid service reply has no recognized Result")]
    InvalidReply,
    #[error("reported position or look direction is not finite")]
    InvalidPosition,
    #[error("region service HTTP request failed")]
    Transport(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct PresenceClient {
    endpoint: String,
}

impl PresenceClient {
    pub fn new(base_uri: &str) -> Self {
        Self {
            endpoint: format!("{base_uri}/presence"),
        }
    }

    /// Scene.VerifyUserPresence checks the returned UserID, not RegionID.
    /// Missing/malformed/failed replies must never authorize admission.
    pub async fn verify(&self, agent_id: Uuid, session_id: Uuid) -> Result<bool, GridError> {
        self.verify_within(agent_id, session_id, std::time::Duration::from_secs(30))
            .await
    }

    /// `timeout` is the caller's remaining CreateAgent budget. Zero does not
    /// mean "no deadline": reqwest treats that as unlimited.
    pub async fn verify_within(
        &self,
        agent_id: Uuid,
        session_id: Uuid,
        timeout: std::time::Duration,
    ) -> Result<bool, GridError> {
        if timeout.is_zero() {
            return Err(GridError::Rejected {
                message: "presence verify budget exhausted".into(),
            });
        }
        let reply = service_form_within(
            &self.endpoint,
            &presence_fields("getagent", session_id),
            timeout,
        )
        .await?;
        match reply.get("result") {
            Some(Field::Map(presence)) => Ok(
                matches!(presence.get("UserID"), Some(Field::Str(user)) if user == &agent_id.to_string()),
            ),
            Some(Field::Str(result)) if result == "null" => Ok(false),
            _ => Err(GridError::InvalidReply),
        }
    }

    pub async fn logout(&self, session_id: Uuid) -> Result<(), GridError> {
        service_result(&service_form(&self.endpoint, &presence_fields("logout", session_id)).await?)
    }

    /// PresenceDetector.AddRegion and RemoveRegion. The connector sends no SessionID.
    pub async fn logout_region(&self, region_id: Uuid) -> Result<(), GridError> {
        let mut fields = service_fields("logoutregion");
        fields.insert("RegionID".into(), Field::Str(region_id.to_string()));
        service_result(&service_form(&self.endpoint, &fields).await?)
    }

    pub async fn report_agent(&self, session_id: Uuid, region_id: Uuid) -> Result<(), GridError> {
        let mut fields = presence_fields("report", session_id);
        fields.insert("RegionID".into(), Field::Str(region_id.to_string()));
        service_result(&service_form(&self.endpoint, &fields).await?)
    }
}

#[derive(Clone)]
pub struct GridUserClient {
    endpoint: String,
}

pub struct GridUserLocation {
    pub online: bool,
    pub region_id: Uuid,
    pub position: [f32; 3],
    pub look_at: [f32; 3],
}

impl GridUserClient {
    pub fn new(base_uri: &str) -> Self {
        Self {
            // GridUserServicesConnector uses OSHHTPHost.URI, unlike Presence's
            // raw configured URI. Configuration rejects nonroot URL paths.
            endpoint: format!("{}/griduser", base_uri.trim_end_matches('/')),
        }
    }

    pub async fn logged_out_region(&self, region_id: Uuid) -> Result<(), GridError> {
        let mut fields =
            position_fields("loggedoutregion", Uuid::nil(), region_id, [0.; 3], [0.; 3])?;
        fields.insert("UserID".into(), Field::Str(String::new()));
        service_result(&service_form(&self.endpoint, &fields).await?)
    }

    pub async fn location(&self, agent_id: Uuid) -> Result<Option<GridUserLocation>, GridError> {
        let mut fields = service_fields("getgriduserinfo");
        fields.insert("UserID".into(), Field::Str(agent_id.to_string()));
        let reply = service_form(&self.endpoint, &fields).await?;
        let Some(Field::Map(info)) = reply.get("result") else {
            return Ok(None);
        };
        let text = |key: &str| match info.get(key) {
            Some(Field::Str(value)) => value.as_str(),
            _ => "",
        };
        Ok(Some(GridUserLocation {
            online: text("Online").eq_ignore_ascii_case("true"),
            region_id: text("LastRegionID").parse().unwrap_or(Uuid::nil()),
            position: parse_vector(text("LastPosition")).unwrap_or([0.; 3]),
            look_at: parse_vector(text("LastLookAt")).unwrap_or([0.; 3]),
        }))
    }

    /// The C# connector does not transmit SessionID. Callers must serialize
    /// admission/cleanup and skip this call for a superseded generation.
    pub async fn logged_out(
        &self,
        agent_id: Uuid,
        region_id: Uuid,
        position: [f32; 3],
        look_at: [f32; 3],
    ) -> Result<(), GridError> {
        let fields = position_fields("loggedout", agent_id, region_id, position, look_at)?;
        service_result(&service_form(&self.endpoint, &fields).await?)
    }

    /// ActivityDetector passes UUID.Zero as session; the remote connector
    /// omits that parameter and writes the known local user's last location.
    pub async fn set_last_position(
        &self,
        agent_id: Uuid,
        region_id: Uuid,
        position: [f32; 3],
        look_at: [f32; 3],
    ) -> Result<(), GridError> {
        let fields = position_fields("setposition", agent_id, region_id, position, look_at)?;
        service_result(&service_form(&self.endpoint, &fields).await?)
    }
}

fn parse_vector(text: &str) -> Option<[f32; 3]> {
    let text = text.trim().strip_prefix('<')?.strip_suffix('>')?;
    let mut parts = text.split(',');
    let mut values = [0.; 3];
    for value in &mut values {
        *value = parts.next()?.trim().parse::<f32>().ok()?;
        if !value.is_finite() {
            return None;
        }
    }
    parts.next().is_none().then_some(values)
}

// EnsureSuccessStatusCode in SynchronousRestFormsRequester is essential here:
// an error response containing an otherwise valid UserID must not admit an agent.
async fn service_form(endpoint: &str, fields: &FieldMap) -> Result<FieldMap, GridError> {
    service_form_within(endpoint, fields, std::time::Duration::from_secs(30)).await
}

async fn service_form_within(
    endpoint: &str,
    fields: &FieldMap,
    timeout: std::time::Duration,
) -> Result<FieldMap, GridError> {
    let body = ferroan_http::build_query_string(fields)?;
    let mut response = reqwest::Client::builder()
        .timeout(timeout)
        .build()?
        .post(endpoint)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let got = bytes.len().saturating_add(chunk.len());
        if got > MAX_SERVICE_REPLY {
            return Err(ferroan_http::HttpError::Limit {
                what: "service reply",
                limit: MAX_SERVICE_REPLY,
                got,
            }
            .into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| ferroan_http::HttpError::Xml("reply is not utf-8"))?;
    Ok(ferroan_http::parse_xml_response(text)?)
}

fn service_fields(method: &str) -> FieldMap {
    FieldMap::from_iter([
        ("VERSIONMIN".into(), Field::Str("1".into())),
        ("VERSIONMAX".into(), Field::Str("1".into())),
        ("METHOD".into(), Field::Str(method.into())),
    ])
}

fn presence_fields(method: &str, session: Uuid) -> FieldMap {
    let mut fields = service_fields(method);
    fields.insert("SessionID".into(), Field::Str(session.to_string()));
    fields
}

fn position_fields(
    method: &str,
    agent: Uuid,
    region: Uuid,
    position: [f32; 3],
    look: [f32; 3],
) -> Result<FieldMap, GridError> {
    if !position
        .iter()
        .chain(&look)
        .all(|coordinate| coordinate.is_finite())
    {
        return Err(GridError::InvalidPosition);
    }
    let vector = |v: [f32; 3]| format!("<{}, {}, {}>", v[0], v[1], v[2]);
    let mut fields = service_fields(method);
    for (key, value) in [
        ("UserID", agent.to_string()),
        ("RegionID", region.to_string()),
        ("Position", vector(position)),
        ("LookAt", vector(look)),
    ] {
        fields.insert(key.into(), Field::Str(value));
    }
    Ok(fields)
}

fn service_result(reply: &FieldMap) -> Result<(), GridError> {
    match reply.get("result") {
        Some(Field::Str(result)) if result.eq_ignore_ascii_case("success") => Ok(()),
        Some(Field::Str(result)) if result.eq_ignore_ascii_case("failure") => {
            Err(GridError::Rejected {
                message: "presence/grid-user operation failed".into(),
            })
        }
        _ => Err(GridError::InvalidReply),
    }
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

    #[tokio::test]
    async fn root_arrival_forms_match_captured_csharp_exchanges() {
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/root-arrival/exchanges.json"
        ))
        .unwrap();
        let queue = Arc::new(Mutex::new(VecDeque::from([
            fixture["services"]["presence-report"].clone(),
            fixture["services"]["griduser-setposition"].clone(),
        ])));
        let remaining = queue.clone();
        let app = axum::Router::new().fallback(
            move |method: axum::http::Method,
                  uri: axum::http::Uri,
                  headers: axum::http::HeaderMap,
                  body: String| {
                let captured = queue.lock().unwrap().pop_front().unwrap();
                assert_eq!(method.as_str(), captured["method"].as_str().unwrap());
                assert_eq!(uri.path(), captured["path"].as_str().unwrap());
                assert_eq!(
                    headers["content-type"],
                    captured["request_content_type"].as_str().unwrap()
                );
                assert_eq!(body, captured["request"].as_str().unwrap());
                assert!(!headers.contains_key("content-encoding"));
                assert!(!headers.contains_key("x-content-encoding"));
                async move {
                    (
                        axum::http::StatusCode::from_u16(
                            captured["status"].as_u64().unwrap() as u16
                        )
                        .unwrap(),
                        [(
                            axum::http::header::CONTENT_TYPE,
                            captured["response_content_type"]
                                .as_str()
                                .unwrap()
                                .to_owned(),
                        )],
                        captured["response"].as_str().unwrap().to_owned(),
                    )
                }
            },
        );
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let agent = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let session = "00000000-0000-4000-8000-000000000004".parse().unwrap();
        let region = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        PresenceClient::new(&base)
            .report_agent(session, region)
            .await
            .unwrap();
        GridUserClient::new(&base)
            .set_last_position(
                agent,
                region,
                [128., 128., f32::from_bits(0x41cf_ae15)],
                [0.; 3],
            )
            .await
            .unwrap();
        assert!(remaining.lock().unwrap().is_empty());
        serving.abort();
    }

    #[tokio::test]
    async fn presence_and_logout_forms_match_captured_csharp_exchanges() {
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/region-parity/exchanges.json"
        ))
        .unwrap();
        let selected: VecDeque<_> = [
            "presence-get",
            "griduser-loggedout",
            "presence-logout",
            "presence-missing",
        ]
        .into_iter()
        .map(|name| fixture["services"][name].clone())
        .collect();
        let queue = Arc::new(Mutex::new(selected));
        let remaining = queue.clone();
        let app = axum::Router::new().fallback(
            move |method: axum::http::Method,
                  uri: axum::http::Uri,
                  headers: axum::http::HeaderMap,
                  body: String| {
                let captured = queue.lock().unwrap().pop_front().unwrap();
                assert_eq!(method.as_str(), captured["method"].as_str().unwrap());
                assert_eq!(uri.path(), captured["path"].as_str().unwrap());
                assert_eq!(
                    headers["content-type"],
                    captured["request_content_type"].as_str().unwrap()
                );
                assert_eq!(body, captured["request"].as_str().unwrap());
                async move {
                    (
                        axum::http::StatusCode::from_u16(
                            captured["status"].as_u64().unwrap() as u16
                        )
                        .unwrap(),
                        [(
                            axum::http::header::CONTENT_TYPE,
                            captured["response_content_type"]
                                .as_str()
                                .unwrap()
                                .to_owned(),
                        )],
                        captured["response"].as_str().unwrap().to_owned(),
                    )
                }
            },
        );
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let agent = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let session = "00000000-0000-4000-8000-000000000004".parse().unwrap();
        let region = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        let presence = PresenceClient::new(&base);
        assert!(presence.verify(agent, session).await.unwrap());
        GridUserClient::new(&base)
            .logged_out(agent, region, [128., 128., 26.034952], [0.; 3])
            .await
            .unwrap();
        presence.logout(session).await.unwrap();
        assert!(!presence.verify(agent, session).await.unwrap());
        assert!(remaining.lock().unwrap().is_empty());
        serving.abort();
    }

    #[tokio::test]
    async fn presence_verification_checks_user_and_never_accepts_missing_or_malformed_replies() {
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};
        let owner = Uuid::from_u128(1);
        let session = Uuid::from_u128(2);
        let replies = Arc::new(Mutex::new(VecDeque::from([
            format!(
                "<ServerResponse><result type=\"List\"><UserID>{owner}</UserID><RegionID>{}</RegionID></result></ServerResponse>",
                Uuid::nil()
            ),
            format!(
                "<ServerResponse><result type=\"List\"><UserID>{}</UserID></result></ServerResponse>",
                Uuid::from_u128(3)
            ),
            "<ServerResponse><result>null</result></ServerResponse>".into(),
            "<ServerResponse><Result>Success</Result></ServerResponse>".into(),
            "not XML".into(),
        ])));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let app = axum::Router::new().route(
            "/presence",
            axum::routing::post(move |body: String| {
                seen.lock().unwrap().push(body);
                let reply = replies.lock().unwrap().pop_front().unwrap();
                async move { reply }
            }),
        );
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = PresenceClient::new(&format!("http://{}", socket.local_addr().unwrap()));
        let serving = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        assert!(client.verify(owner, session).await.unwrap());
        assert!(!client.verify(owner, session).await.unwrap());
        assert!(!client.verify(owner, session).await.unwrap());
        assert!(client.verify(owner, session).await.is_err());
        assert!(client.verify(owner, session).await.is_err());
        serving.abort();
        for request in requests.lock().unwrap().iter() {
            assert_eq!(
                request,
                &format!("VERSIONMIN=1&VERSIONMAX=1&METHOD=getagent&SessionID={session}")
            );
        }
    }

    #[tokio::test]
    async fn logout_connectors_send_the_source_fields_and_require_success() {
        use std::sync::{Arc, Mutex};
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri, body: String| {
            seen.lock().unwrap().push((uri.path().to_owned(), body));
            async { "<ServerResponse><result>Success</result></ServerResponse>" }
        });
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let agent = Uuid::from_u128(1);
        let session = Uuid::from_u128(2);
        let region = Uuid::from_u128(3);
        GridUserClient::new(&format!("{base}/"))
            .logged_out(agent, region, [128., 128., 25.], [0.99, 0.042, 0.])
            .await
            .unwrap();
        PresenceClient::new(&base).logout(session).await.unwrap();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/pin3/login-robustness.json"))
                .unwrap();
        let region_logout = fixture["logout_region"]["region"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        PresenceClient::new(&base)
            .logout_region(region_logout)
            .await
            .unwrap();
        serving.abort();
        let requests = requests.lock().unwrap();
        assert_eq!(requests[0].0, "/griduser");
        assert_eq!(
            requests[0].1,
            format!(
                "VERSIONMIN=1&VERSIONMAX=1&METHOD=loggedout&UserID={agent}&RegionID={region}&Position=%3c128%2c+128%2c+25%3e&LookAt=%3c0.99%2c+0.042%2c+0%3e"
            )
        );
        assert!(!requests[0].1.contains("SessionID"));
        assert_eq!(
            requests[1],
            (
                "/presence".into(),
                format!("VERSIONMIN=1&VERSIONMAX=1&METHOD=logout&SessionID={session}")
            )
        );
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/pin3/login-robustness.json"))
                .unwrap();
        assert_eq!(requests[2].0, "/presence");
        assert_eq!(
            requests[2].1,
            fixture["logout_region"]["calls"][0]["request"]
                .as_str()
                .unwrap()
        );
        for xml in [
            "<ServerResponse><result>Failure</result></ServerResponse>",
            "<ServerResponse><result type=\"List\"><UserID>1</UserID></result></ServerResponse>",
            "<ServerResponse/>",
        ] {
            assert!(service_result(&ferroan_http::parse_xml_response(xml).unwrap()).is_err());
        }
        assert!(position_fields("loggedout", agent, region, [f32::NAN, 0., 0.], [0.; 3]).is_err());
    }

    #[tokio::test]
    async fn service_errors_cannot_authorize_or_confirm_logout() {
        let agent = Uuid::from_u128(1);
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri| async move {
            let body = if uri.path() == "/presence" {
                format!("<ServerResponse><result type=\"List\"><UserID>{agent}</UserID></result></ServerResponse>")
            } else {
                "<ServerResponse><result>Success</result></ServerResponse>".into()
            };
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, body)
        });
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        assert!(matches!(
            PresenceClient::new(&base)
                .verify(agent, Uuid::from_u128(2))
                .await,
            Err(GridError::Transport(_))
        ));
        assert!(matches!(
            GridUserClient::new(&base)
                .logged_out(agent, Uuid::from_u128(3), [0.; 3], [0.; 3])
                .await,
            Err(GridError::Transport(_))
        ));
        serving.abort();
    }

    #[test]
    fn registration_and_deregistration_match_the_csharp_grid_capture() {
        let server = ferroan_config::Config::parse(
            "[Network]\nhttp_listener_port=9000\nbind_ip_address=127.0.0.2\n\
             [GridService]\nGridServerURI=http://192.0.2.1:8003\n\
             [PresenceService]\nPresenceServerURI=http://192.0.2.1:8003\n\
             [GridUserService]\nGridUserServerURI=http://192.0.2.1:8003\n\
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
