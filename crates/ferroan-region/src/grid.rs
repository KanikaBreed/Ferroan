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
//   Source/OpenSim.Services.Connectors/UserAccounts/UserAccountServicesConnector.cs (GetUserAccount)
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Region registration, presence verification and logout through C# Robust.
//! Transport has a 30 second deadline and ferroan-http's bounded XML reader.
//! No retries: startup fails when registration is refused or unconfirmed.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use ferroan_http::{Field, FieldMap};
use uuid::Uuid;

use crate::config::RegionConfig;

const MAX_SERVICE_REPLY: usize = 1024 * 1024;
/// Account level lookup during admission. A miss or a timeout is not a god.
const ACCOUNT_LEVEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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

/// Presence and GridUser connectors call
/// `SynchronousRestFormsRequester.MakeRequest`, which calls
/// `EnsureSuccessStatusCode`. A non-2xx body must not admit an agent or
/// confirm a logout. Grid register and deregister do not use this function.
pub(crate) async fn service_form(endpoint: &str, fields: &FieldMap) -> Result<FieldMap, GridError> {
    service_form_within(endpoint, fields, std::time::Duration::from_secs(30)).await
}

async fn service_form_within(
    endpoint: &str,
    fields: &FieldMap,
    timeout: std::time::Duration,
) -> Result<FieldMap, GridError> {
    Ok(ferroan_http::post_form_with(
        endpoint,
        fields,
        ferroan_http::FormOptions::checked(timeout, MAX_SERVICE_REPLY),
    )
    .await?)
}

/// `UserAccountServicesConnector.GetUserAccount` posts `METHOD=getaccount` to
/// `{UserAccountServerURI}/accounts`. A missing service, a bad reply, or no
/// `UserLevel` is `None` (not a grid god). The reply body is not logged.
pub(crate) async fn account_user_level(uri: &str, scope: Uuid, agent: Uuid) -> Option<i32> {
    if uri.is_empty() || agent.is_nil() {
        return None;
    }
    let endpoint = format!("{}/accounts", uri.trim_end_matches('/'));
    let mut fields = service_fields("getaccount");
    fields.insert("ScopeID".into(), Field::Str(scope.to_string()));
    fields.insert("UserID".into(), Field::Str(agent.to_string()));
    let reply = service_form_within(&endpoint, &fields, ACCOUNT_LEVEL_TIMEOUT)
        .await
        .ok()?;
    let Field::Map(result) = reply.get("result")? else {
        return None;
    };
    match result.get("UserLevel") {
        Some(Field::Str(text)) => text.trim().parse().ok(),
        Some(Field::Int(value)) => Some(*value),
        _ => None,
    }
}

pub(crate) fn service_fields(method: &str) -> FieldMap {
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
    let vector = |value: [f32; 3]| {
        format!(
            "<{}, {}, {}>",
            ferroan_services::griduser::format_f32(value[0]),
            ferroan_services::griduser::format_f32(value[1]),
            ferroan_services::griduser::format_f32(value[2])
        )
    };
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

/// One grid row a hop can address. The region secret is not stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedRegion {
    pub region_id: Uuid,
    pub name: String,
    pub loc_x: u32,
    pub loc_y: u32,
    pub size_x: u32,
    pub size_y: u32,
    pub server_uri: String,
    pub endpoint: SocketAddr,
}

#[derive(Clone)]
pub struct GridClient {
    endpoint: String,
}

impl GridClient {
    pub fn new(config: &RegionConfig) -> Self {
        // The reference appends to serviceURI, not its trimmed m_ServerURI.
        Self::from_base(&config.grid_server_uri)
    }

    pub fn from_base(grid_server_uri: &str) -> Self {
        Self {
            endpoint: format!("{grid_server_uri}/grid"),
        }
    }

    pub async fn register(&self, config: &RegionConfig) -> Result<(), GridError> {
        // GridServicesConnector.RegisterRegion calls MakePostRequest. At Pin5
        // that helper checks the HTTP status (TB10, #438) and the connector
        // returns an error string when the call fails.
        check_result(&grid_form(&self.endpoint, &registration_fields(config)).await?)
    }

    pub async fn deregister(&self, region_id: Uuid) -> Result<(), GridError> {
        // GridServicesConnector.DeregisterRegion calls MakePostRequest (TB10).
        // A failed status is false.
        check_result(&grid_form(&self.endpoint, &deregistration_fields(region_id)).await?)
    }

    /// `GetRegionByPosition`. A missing result map is an unknown region.
    /// A present value that cannot be read is an unknown region too.
    pub async fn region_by_position(
        &self,
        scope: Uuid,
        x: i32,
        y: i32,
    ) -> Result<Option<LocatedRegion>, GridError> {
        let mut fields = FieldMap::new();
        for (key, value) in [
            ("SCOPEID", scope.to_string()),
            ("X", x.to_string()),
            ("Y", y.to_string()),
            ("METHOD", "get_region_by_position".to_string()),
        ] {
            fields.insert(key.to_owned(), Field::Str(value));
        }
        let reply = grid_form(&self.endpoint, &fields).await?;
        let Some(Field::Map(region)) = reply.get("result") else {
            return Ok(None);
        };
        let Some(mut located) = parse_located(region) else {
            return Ok(None);
        };
        let Some(host) = located_host(region) else {
            return Ok(None);
        };
        let port = located.endpoint.port();
        located.endpoint = endpoint_for(&host, port).await;
        Ok(Some(located))
    }
}

/// A literal address is used as written. A name is looked up for at most two
/// seconds, and an IPv4 answer wins. An empty name is not looked up.
async fn endpoint_for(host: &str, port: u16) -> SocketAddr {
    if host.is_empty() {
        return SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return SocketAddr::from((ip, port));
    }
    if let Ok(ip) = host.parse::<Ipv6Addr>() {
        return SocketAddr::from((ip, port));
    }
    let lookup = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::lookup_host((host, port)),
    )
    .await;
    let Ok(Ok(addrs)) = lookup else {
        return SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    };
    let addrs: Vec<SocketAddr> = addrs.collect();
    if let Some(address) = addrs.iter().find(|address| address.is_ipv4()) {
        return *address;
    }
    addrs
        .into_iter()
        .next()
        .unwrap_or_else(|| SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)))
}

fn located_host(map: &FieldMap) -> Option<String> {
    match map.get("serverIP") {
        None => Some("127.0.0.1".to_string()),
        Some(Field::Str(text)) => Some(text.clone()),
        _ => None,
    }
}

fn parse_located(map: &FieldMap) -> Option<LocatedRegion> {
    let region_id = match map.get("uuid") {
        None => Uuid::nil(),
        Some(Field::Str(text)) => Uuid::parse_str(text).ok()?,
        _ => return None,
    };
    let loc_x = taken_u32(map, "locX", 0)?;
    let loc_y = taken_u32(map, "locY", 0)?;
    let size_x = taken_u32(map, "sizeX", 256)?;
    let size_y = taken_u32(map, "sizeY", 256)?;
    let name = match map.get("regionName") {
        None => String::new(),
        Some(Field::Str(text)) => text.clone(),
        _ => return None,
    };
    let server_uri = match map.get("serverURI") {
        None => String::new(),
        Some(Field::Str(text)) if text.is_empty() => String::new(),
        Some(Field::Str(text)) if text.ends_with('/') => text.clone(),
        Some(Field::Str(text)) => format!("{text}/"),
        _ => return None,
    };
    let host = match map.get("serverIP") {
        None => "127.0.0.1",
        Some(Field::Str(text)) if text.is_empty() => "",
        Some(Field::Str(text)) => text.as_str(),
        _ => return None,
    };
    let port = endpoint_port(map)?;
    // The caller resolves a name. A non-address stays unspecified until then.
    let endpoint = match host.parse::<Ipv4Addr>() {
        Ok(ip) => SocketAddr::from((ip, port)),
        Err(_) => host
            .parse::<Ipv6Addr>()
            .map(|ip| SocketAddr::from((ip, port)))
            .unwrap_or_else(|_| SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))),
    };
    Some(LocatedRegion {
        region_id,
        name,
        loc_x,
        loc_y,
        size_x,
        size_y,
        server_uri,
        endpoint,
    })
}

/// Missing uses `default`. A non-integer or a negative value is unreadable.
fn taken_u32(map: &FieldMap, key: &str, default: u32) -> Option<u32> {
    let number = match map.get(key) {
        None => return Some(default),
        Some(Field::Str(text)) => text.parse::<i32>().ok()?,
        Some(Field::Int(number)) => *number,
        _ => return None,
    };
    u32::try_from(number).ok()
}

/// `TryParse` failure is port 0. A port outside 0..=65535 cannot be an endpoint.
fn endpoint_port(map: &FieldMap) -> Option<u16> {
    match map.get("serverPort") {
        None => Some(0),
        Some(Field::Str(text)) => match text.parse::<i32>() {
            Ok(port) => u16::try_from(port).ok(),
            Err(_) => Some(0),
        },
        Some(Field::Int(port)) => u16::try_from(*port).ok(),
        _ => None,
    }
}

/// `MakePostRequest` at Pin5: a non-2xx status is an error.
async fn grid_form(endpoint: &str, fields: &FieldMap) -> Result<FieldMap, ferroan_http::HttpError> {
    ferroan_http::post_form_with(
        endpoint,
        fields,
        ferroan_http::FormOptions::checked(
            std::time::Duration::from_secs(30),
            ferroan_http::MAX_XML_CHARS,
        ),
    )
    .await
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
            .logged_out(agent, region, [128., 128., 26.033272], [0.; 3])
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
        GridUserClient::new(&format!("{base}/"))
            .logged_out(agent, region, [f32::NAN, 128., 25.], [0.99, 0.042, 0.])
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
        let nan = position_fields("loggedout", agent, region, [f32::NAN, 0., 0.], [0.; 3])
            .expect("nan logout");
        assert_eq!(
            nan.get("Position"),
            Some(&Field::Str("<NaN, 0, 0>".to_string()))
        );
        let infinite = position_fields(
            "loggedout",
            agent,
            region,
            [f32::INFINITY, f32::NEG_INFINITY, 128.3],
            [0.; 3],
        )
        .expect("infinite logout");
        assert_eq!(
            infinite.get("Position"),
            Some(&Field::Str("<∞, -∞, 128.3>".to_string()))
        );
        assert!(requests[3].1.contains("Position=%3cNaN%2c+128%2c+25%3e"));
        assert_eq!(requests[3].0, "/griduser");
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
            Err(GridError::Http(ferroan_http::HttpError::Status {
                status: 500
            }))
        ));
        assert!(matches!(
            GridUserClient::new(&base)
                .logged_out(agent, Uuid::from_u128(3), [0.; 3], [0.; 3])
                .await,
            Err(GridError::Http(ferroan_http::HttpError::Status {
                status: 500
            }))
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

    fn status_fixture(caller: &str, status: u16) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../fixtures/http-status/{caller}-{status}.json"));
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    /// Serves one injected status and records the request body.
    async fn serve_injected(
        fixture: &serde_json::Value,
    ) -> (
        String,
        std::sync::Arc<std::sync::Mutex<String>>,
        tokio::task::JoinHandle<()>,
    ) {
        use std::sync::{Arc, Mutex};
        let status = fixture["injected_response"]["status"].as_u64().unwrap() as u16;
        let body = fixture["injected_response"]["body"]
            .as_str()
            .unwrap()
            .to_owned();
        let mime = fixture["injected_response"]["content_type"]
            .as_str()
            .unwrap()
            .to_owned();
        let seen = Arc::new(Mutex::new(String::new()));
        let record = seen.clone();
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri, request: String| {
            let record = record.clone();
            let body = body.clone();
            let mime = mime.clone();
            async move {
                *record.lock().unwrap() = format!("{} {request}", uri.path());
                (
                    axum::http::StatusCode::from_u16(status).unwrap(),
                    [(axum::http::header::CONTENT_TYPE, mime)],
                    body,
                )
            }
        });
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        (base, seen, serving)
    }

    fn assert_status_result(
        caller: &str,
        status: u16,
        accepted: bool,
        result: Result<(), GridError>,
    ) {
        match result {
            Ok(()) => assert!(accepted, "{caller} {status} accepted a result C# rejected"),
            Err(GridError::Http(ferroan_http::HttpError::Status { status: observed })) => {
                assert!(!accepted, "{caller} {status} rejected a result C# accepted");
                assert_eq!(observed, status, "{caller}");
            }
            other => panic!("{caller} {status}: {other:?}"),
        }
    }

    async fn each_status(caller: &str, mut call: impl AsyncFnMut(&str, u16, &serde_json::Value)) {
        for status in [200, 201, 302, 500] {
            let fixture = status_fixture(caller, status);
            assert_eq!(fixture["caller"], caller);
            assert_eq!(fixture["injected_response"]["status"], status);
            assert!(fixture["injected_response"]["location"].is_null());
            call(caller, status, &fixture).await;
        }
    }

    #[tokio::test]
    async fn grid_register_matches_csharp_checked_status() {
        each_status("grid-register", async |caller, status, fixture| {
            let (base, seen, serving) = serve_injected(fixture).await;
            let agent = "00000000-0000-4000-8000-000000000001";
            let region = "00000000-0000-4000-8000-000000000002";
            let server = ferroan_config::Config::parse(&format!(
                "[Network]\nhttp_listener_port=9000\nbind_ip_address=127.0.0.2\n\
                 [GridService]\nGridServerURI={base}\n\
                 [PresenceService]\nPresenceServerURI={base}\n\
                 [GridUserService]\nGridUserServerURI={base}\n\
                 [Estates]\nDefaultEstateOwnerUUID={agent}\n\
                 [Map]\nGenerateMaptiles=false\n"
            ))
            .unwrap();
            let regions = ferroan_config::Config::parse(&format!(
                "[Fixture Region]\nRegionUUID={region}\nLocation=1000,1000\n\
                 InternalAddress=127.0.0.1\nInternalPort=9000\nExternalHostName=192.0.2.1\n"
            ))
            .unwrap();
            let result = GridClient::new(&RegionConfig::from_configs(&server, &regions).unwrap())
                .register(&RegionConfig::from_configs(&server, &regions).unwrap())
                .await;
            serving.abort();
            let recorded = seen.lock().unwrap().clone();
            assert!(recorded.starts_with("/grid "), "{recorded}");
            assert!(recorded.contains("METHOD=register"), "{recorded}");
            let accepted = fixture["outcome"]["accepted"].as_bool().unwrap();
            if accepted {
                assert_eq!(fixture["outcome"]["value"], "");
            } else {
                let value = fixture["outcome"]["value"].as_str().unwrap();
                assert!(
                    value.starts_with("Error communicating with the grid service at "),
                    "{value}"
                );
            }
            assert_status_result(caller, status, accepted, result);
        })
        .await;
    }

    #[tokio::test]
    async fn grid_deregister_matches_csharp_checked_status() {
        each_status("grid-deregister", async |caller, status, fixture| {
            let (base, seen, serving) = serve_injected(fixture).await;
            let region = "00000000-0000-4000-8000-000000000002".parse().unwrap();
            let server = ferroan_config::Config::parse(&format!(
                "[Network]\nhttp_listener_port=9000\nbind_ip_address=127.0.0.2\n\
                 [GridService]\nGridServerURI={base}\n\
                 [PresenceService]\nPresenceServerURI={base}\n\
                 [GridUserService]\nGridUserServerURI={base}\n\
                 [Estates]\nDefaultEstateOwnerUUID=00000000-0000-4000-8000-000000000001\n\
                 [Map]\nGenerateMaptiles=false\n"
            ))
            .unwrap();
            let regions = ferroan_config::Config::parse(
                "[Fixture Region]\nRegionUUID=00000000-0000-4000-8000-000000000002\n\
                 Location=1000,1000\nInternalAddress=127.0.0.1\nInternalPort=9000\n\
                 ExternalHostName=192.0.2.1\n",
            )
            .unwrap();
            let result = GridClient::new(&RegionConfig::from_configs(&server, &regions).unwrap())
                .deregister(region)
                .await;
            serving.abort();
            assert_eq!(
                seen.lock().unwrap().as_str(),
                format!("/grid {}", fixture["request"]["body"].as_str().unwrap())
            );
            let accepted = fixture["outcome"]["accepted"].as_bool().unwrap();
            if accepted {
                assert_eq!(fixture["outcome"]["value"], true);
            } else {
                assert_eq!(fixture["outcome"]["value"], false);
            }
            assert_status_result(caller, status, accepted, result);
        })
        .await;
    }

    /// TB10. Grid register and deregister fixtures reject HTTP 302 and 500.
    /// Register's value is the connector's error string. Deregister's is false.
    #[test]
    fn csharp_make_post_request_rejects_grid_failure_status() {
        for caller in ["grid-register", "grid-deregister"] {
            for status in [302u16, 500] {
                let fixture = status_fixture(caller, status);
                let path = format!("fixtures/http-status/{caller}-{status}.json");
                assert_eq!(
                    fixture["outcome"]["accepted"], false,
                    "TB10: {path} still accepts a non-2xx status. Change crates/ferroan-region/src/grid.rs grid_form and FormStatusPolicy::Checked."
                );
                assert_eq!(fixture["injected_response"]["status"], status);
                assert!(fixture["injected_response"]["location"].is_null());
                if caller == "grid-register" {
                    let value = fixture["outcome"]["value"].as_str().unwrap();
                    assert!(
                        value.starts_with("Error communicating with the grid service at "),
                        "{path}: {value}"
                    );
                } else {
                    assert_eq!(fixture["outcome"]["value"], false, "{path}");
                }
            }
        }
    }

    #[tokio::test]
    async fn presence_getagent_matches_csharp_checked_status() {
        each_status("presence-getagent", async |caller, status, fixture| {
            let (base, seen, serving) = serve_injected(fixture).await;
            let agent = "00000000-0000-4000-8000-000000000001".parse().unwrap();
            let session = "00000000-0000-4000-8000-000000000004".parse().unwrap();
            let result = PresenceClient::new(&base).verify(agent, session).await;
            serving.abort();
            assert_eq!(
                seen.lock().unwrap().as_str(),
                format!("/presence {}", fixture["request"]["body"].as_str().unwrap())
            );
            let accepted = fixture["outcome"]["accepted"].as_bool().unwrap();
            match result {
                Ok(true) => assert!(accepted),
                Err(GridError::Http(ferroan_http::HttpError::Status { status: observed })) => {
                    assert!(!accepted);
                    assert!(fixture["outcome"]["value"].is_null());
                    assert_eq!(observed, status, "{caller}");
                }
                other => panic!("{caller} {status}: {other:?}"),
            }
        })
        .await;
    }

    #[tokio::test]
    async fn presence_getagents_matches_csharp_checked_status() {
        // Ferroan has no GetAgents caller. The shared client is the checked
        // helper that method uses. C# catches the status failure and returns
        // an empty array; a status error here is that failure, not success.
        each_status("presence-getagents", async |caller, status, fixture| {
            let (base, _seen, serving) = serve_injected(fixture).await;
            let fields =
                ferroan_http::parse_query_string(fixture["request"]["body"].as_str().unwrap())
                    .unwrap();
            let result = ferroan_http::post_form_with(
                &format!("{base}/presence"),
                &fields,
                ferroan_http::FormOptions::checked(
                    std::time::Duration::from_secs(5),
                    MAX_SERVICE_REPLY,
                ),
            )
            .await;
            serving.abort();
            let accepted = fixture["outcome"]["accepted"].as_bool().unwrap();
            match result {
                Ok(reply) => {
                    assert!(accepted, "{caller} {status}");
                    let Field::Map(presence) = &reply["presence0"] else {
                        panic!("{caller} {status}: {reply:?}");
                    };
                    assert_eq!(
                        presence.get("UserID"),
                        Some(&Field::Str("00000000-0000-4000-8000-000000000001".into()))
                    );
                }
                Err(ferroan_http::HttpError::Status { status: observed }) => {
                    assert!(!accepted);
                    assert_eq!(fixture["outcome"]["value"].as_array().unwrap().len(), 0);
                    assert_eq!(observed, status);
                }
                other => panic!("{caller} {status}: {other:?}"),
            }
        })
        .await;
    }

    #[tokio::test]
    async fn presence_logout_matches_csharp_checked_status() {
        each_status("presence-logout", async |caller, status, fixture| {
            let (base, seen, serving) = serve_injected(fixture).await;
            let session = "00000000-0000-4000-8000-000000000004".parse().unwrap();
            let result = PresenceClient::new(&base).logout(session).await;
            serving.abort();
            assert_eq!(
                seen.lock().unwrap().as_str(),
                format!("/presence {}", fixture["request"]["body"].as_str().unwrap())
            );
            assert_eq!(fixture["outcome"]["value"], fixture["outcome"]["accepted"]);
            assert_status_result(
                caller,
                status,
                fixture["outcome"]["accepted"].as_bool().unwrap(),
                result,
            );
        })
        .await;
    }

    #[tokio::test]
    async fn presence_report_matches_csharp_checked_status() {
        each_status("presence-report", async |caller, status, fixture| {
            let (base, seen, serving) = serve_injected(fixture).await;
            let session = "00000000-0000-4000-8000-000000000004".parse().unwrap();
            let region = "00000000-0000-4000-8000-000000000002".parse().unwrap();
            let result = PresenceClient::new(&base)
                .report_agent(session, region)
                .await;
            serving.abort();
            assert_eq!(
                seen.lock().unwrap().as_str(),
                format!("/presence {}", fixture["request"]["body"].as_str().unwrap())
            );
            assert_status_result(
                caller,
                status,
                fixture["outcome"]["value"].as_bool().unwrap(),
                result,
            );
        })
        .await;
    }

    #[tokio::test]
    async fn griduser_loggedout_matches_csharp_checked_status() {
        each_status("griduser-loggedout", async |caller, status, fixture| {
            let (base, seen, serving) = serve_injected(fixture).await;
            let agent = "00000000-0000-4000-8000-000000000001".parse().unwrap();
            let region = "00000000-0000-4000-8000-000000000002".parse().unwrap();
            let result = GridUserClient::new(&base)
                .logged_out(agent, region, [128., 128., 25.], [0.; 3])
                .await;
            serving.abort();
            assert_eq!(
                seen.lock().unwrap().as_str(),
                format!("/griduser {}", fixture["request"]["body"].as_str().unwrap())
            );
            assert_status_result(
                caller,
                status,
                fixture["outcome"]["value"].as_bool().unwrap(),
                result,
            );
        })
        .await;
    }

    #[tokio::test]
    async fn griduser_setposition_matches_csharp_checked_status() {
        each_status("griduser-setposition", async |caller, status, fixture| {
            let (base, seen, serving) = serve_injected(fixture).await;
            let agent = "00000000-0000-4000-8000-000000000001".parse().unwrap();
            let region = "00000000-0000-4000-8000-000000000002".parse().unwrap();
            let result = GridUserClient::new(&base)
                .set_last_position(agent, region, [128., 128., 25.], [0.; 3])
                .await;
            serving.abort();
            assert_eq!(
                seen.lock().unwrap().as_str(),
                format!("/griduser {}", fixture["request"]["body"].as_str().unwrap())
            );
            assert_status_result(
                caller,
                status,
                fixture["outcome"]["value"].as_bool().unwrap(),
                result,
            );
        })
        .await;
    }

    #[tokio::test]
    async fn griduser_getgriduserinfo_matches_csharp_checked_status() {
        each_status(
            "griduser-getgriduserinfo",
            async |caller, status, fixture| {
                let (base, seen, serving) = serve_injected(fixture).await;
                let agent = "00000000-0000-4000-8000-000000000001".parse().unwrap();
                let result = GridUserClient::new(&base).location(agent).await;
                serving.abort();
                assert_eq!(
                    seen.lock().unwrap().as_str(),
                    format!("/griduser {}", fixture["request"]["body"].as_str().unwrap())
                );
                let accepted = fixture["outcome"]["accepted"].as_bool().unwrap();
                match result {
                    Ok(Some(location)) => {
                        assert!(accepted, "{caller}");
                        assert!(location.online);
                        assert_eq!(
                            location.region_id,
                            "00000000-0000-4000-8000-000000000002"
                                .parse::<Uuid>()
                                .unwrap()
                        );
                        assert_eq!(location.position, [128., 128., 25.]);
                    }
                    Err(GridError::Http(ferroan_http::HttpError::Status { status: observed })) => {
                        assert!(!accepted);
                        assert!(fixture["outcome"]["value"].is_null());
                        assert_eq!(observed, status, "{caller}");
                    }
                    Ok(None) => {
                        panic!("{caller} {status}: C# returned a user and Rust returned none")
                    }
                    Err(error) => panic!("{caller} {status}: {error}"),
                }
            },
        )
        .await;
    }

    #[tokio::test]
    async fn region_by_position_reads_a_result_map_and_ignores_a_miss() {
        use std::sync::{Arc, Mutex};
        let seen = Arc::new(Mutex::new(String::new()));
        let recorded = Arc::clone(&seen);
        let region_id = Uuid::from_u128(7);
        let mut region = FieldMap::new();
        for (key, value) in [
            ("uuid", region_id.to_string()),
            ("locX", "256".to_string()),
            ("locY", "512".to_string()),
            ("sizeX", "256".to_string()),
            ("regionName", "Neighbour".to_string()),
            ("serverIP", "127.0.0.1".to_string()),
            ("serverPort", "9100".to_string()),
            ("serverURI", "http://127.0.0.1:9200".to_string()),
            ("regionSecret", "do-not-keep".to_string()),
        ] {
            region.insert(key.to_owned(), Field::Str(value));
        }
        let mut ok = FieldMap::new();
        ok.insert("result".to_owned(), Field::Map(region));
        let ok_xml = ferroan_http::build_xml_response(&ok).unwrap();
        let mut missing = FieldMap::new();
        missing.insert("result".to_owned(), Field::Str("null".to_owned()));
        let missing_xml = ferroan_http::build_xml_response(&missing).unwrap();
        let mut bad = FieldMap::new();
        let mut bad_region = FieldMap::new();
        bad_region.insert("uuid".to_owned(), Field::Str(region_id.to_string()));
        bad_region.insert("sizeX".to_owned(), Field::Str("nope".to_owned()));
        bad.insert("result".to_owned(), Field::Map(bad_region));
        let bad_xml = ferroan_http::build_xml_response(&bad).unwrap();
        let bodies = Arc::new(Mutex::new(vec![ok_xml, missing_xml, bad_xml]));
        let queued = Arc::clone(&bodies);
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri, body: String| {
            *recorded.lock().unwrap() = format!("{} {body}", uri.path());
            let next = queued.lock().unwrap().remove(0);
            async move {
                (
                    axum::http::StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "application/xml")],
                    next,
                )
            }
        });
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = GridClient {
            endpoint: format!("{base}/grid"),
        };
        let scope = Uuid::from_u128(3);
        let found = client.region_by_position(scope, 256, 512).await.unwrap();
        let request = seen.lock().unwrap().clone();
        assert!(request.starts_with("/grid "), "{request}");
        assert!(request.contains(&format!("SCOPEID={scope}")), "{request}");
        assert!(request.contains("X=256"), "{request}");
        assert!(request.contains("Y=512"), "{request}");
        assert!(
            request.contains("METHOD=get_region_by_position"),
            "{request}"
        );
        let scope_at = request.find("SCOPEID=").unwrap();
        let x_at = request.find("X=").unwrap();
        let y_at = request.find("Y=").unwrap();
        let method_at = request.find("METHOD=").unwrap();
        assert!(scope_at < x_at && x_at < y_at && y_at < method_at);
        assert_eq!(
            found,
            Some(LocatedRegion {
                region_id,
                name: "Neighbour".to_string(),
                loc_x: 256,
                loc_y: 512,
                size_x: 256,
                size_y: 256,
                server_uri: "http://127.0.0.1:9200/".to_string(),
                endpoint: "127.0.0.1:9100".parse().unwrap(),
            })
        );
        assert!(
            client
                .region_by_position(scope, 1, 2)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            client
                .region_by_position(scope, 1, 2)
                .await
                .unwrap()
                .is_none()
        );
        serving.abort();
    }

    #[tokio::test]
    async fn a_dns_name_resolves_to_ipv4_and_an_empty_host_does_not() {
        assert_eq!(
            endpoint_for("localhost", 9100).await,
            "127.0.0.1:9100".parse().unwrap()
        );
        assert_eq!(
            endpoint_for("", 9100).await,
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, 9100))
        );
        assert_eq!(
            endpoint_for("ferroan.invalid", 9100).await,
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, 9100))
        );
    }

    #[tokio::test]
    async fn region_by_position_resolves_a_dns_name() {
        let mut region = FieldMap::new();
        for (key, value) in [
            ("uuid", Uuid::from_u128(7).to_string()),
            ("locX", "256".to_string()),
            ("locY", "256".to_string()),
            ("serverIP", "localhost".to_string()),
            ("serverPort", "9100".to_string()),
            ("serverURI", "http://localhost:9200".to_string()),
        ] {
            region.insert(key.to_owned(), Field::Str(value));
        }
        let mut ok = FieldMap::new();
        ok.insert("result".to_owned(), Field::Map(region));
        let xml = ferroan_http::build_xml_response(&ok).unwrap();
        let app = axum::Router::new().fallback(move || {
            let xml = xml.clone();
            async move {
                (
                    axum::http::StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "application/xml")],
                    xml,
                )
            }
        });
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let serving = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = GridClient {
            endpoint: format!("{base}/grid"),
        };
        let found = client
            .region_by_position(Uuid::from_u128(3), 256, 256)
            .await
            .unwrap()
            .unwrap();
        serving.abort();
        assert_eq!(found.endpoint, "127.0.0.1:9100".parse().unwrap());
        assert_eq!(found.server_uri, "http://localhost:9200/");
    }
}
