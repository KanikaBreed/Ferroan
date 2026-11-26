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
// Local/GroupsServiceLocalConnectorModule.cs, Service/{GroupsService,GroupsServiceBase,GroupRules}.cs,
// Hypergrid/GroupsServiceHGConnectorModule.cs,
// Source/OpenSim.Data.SQLite/SQLiteGroupsData.cs, Source/OpenSim.Data/IGroupsData.cs,
// Source/OpenSim.Region.OptionalModules/Avatar/XmlRpcGroups/{GroupsModule,XmlRpcGroupsServicesConnectorModule}.cs
// and Source/OpenSim.Region.ClientStack.LindenUDP/LLClientView.cs (RefreshGroupMembership)
// at Tranquillity c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Group memberships for the region's parcel overlay.
//!
//! The Remote connector reads Robust. The Local connector reads the SQLite
//! `os_groups` tables the way `GroupsService.GetAgentGroupMemberships` does.
use crate::{
    config::{RegionConfigError, boolean, check_uri, value},
    grid::GridError,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ferroan_config::Config;
use ferroan_db::{Backend, Pool};
use ferroan_http::{Field, FieldMap, FormOptions, post_form_with_client};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};
use uuid::Uuid;

/// Pin8 reads every row. A membership, group, role or role-membership read past
/// this count is refused (`groups.membership-rows`).
const MAX_MEMBERSHIP_ROWS: usize = 4096;
const ROW_FETCH: usize = MAX_MEMBERSHIP_ROWS + 1;
/// Longest selected GroupID, OwnerRoleID, RoleID, Powers or PrincipalID value
/// (`groups.column-bytes`). Measured with `length()` before the value is fetched.
const MAX_COLUMN_BYTES: i64 = 1024;
/// `GroupsBatch.Size`. The IN list is split here. It is not a membership cap.
const GROUP_BATCH: usize = 500;

/// No Debug: the dedicated HTTP client can carry the configured service key.
#[derive(Clone)]
pub struct GroupsClient(Option<Source>);

#[derive(Clone)]
enum Source {
    Remote(Remote),
    Local(LocalStore),
}

#[derive(Clone)]
struct Remote {
    endpoint: String,
    http: reqwest::Client,
}

#[derive(Clone)]
struct LocalStore {
    connection: String,
}

struct StoredGroup {
    owner: Uuid,
    roles: HashMap<Uuid, Option<u64>>,
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
        if connector == Some("Groups Local Service Connector") {
            return Ok(Self(Some(Source::Local(local_store(config)?))));
        }
        if connector != Some("Groups Remote Service Connector") {
            return Err(value(
                "Groups",
                "ServicesConnectorModule",
                "unsupported group membership source; supported pairs: Groups Module V2 with Groups Remote Service Connector, or Groups Module V2 with Groups Local Service Connector on SQLite",
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
        Ok(Self(Some(Source::Remote(Remote {
            endpoint: format!("{}/groups", uri.trim_end_matches('/')),
            http,
        }))))
    }

    /// All group IDs, including inactive memberships with no powers. Pin8's
    /// viewer tests membership by key and uses the unsigned mask for rights.
    pub async fn memberships(&self, agent: Uuid) -> Result<BTreeMap<Uuid, u64>, GridError> {
        match &self.0 {
            None => Ok(BTreeMap::new()),
            Some(Source::Remote(remote)) => remote_memberships(remote, agent).await,
            Some(Source::Local(store)) => local_memberships(store, agent).await,
        }
    }
}

async fn remote_memberships(
    remote: &Remote,
    agent: Uuid,
) -> Result<BTreeMap<Uuid, u64>, GridError> {
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

/// `[Groups]` overrides `[DatabaseService]` for a key that is present, including
/// an empty value. `GroupsServiceBase` does not read `[SimulationDataStore]`.
fn groups_setting<'a>(
    config: &'a Config,
    key: &'static str,
) -> Result<Option<&'a str>, RegionConfigError> {
    if let Some(text) = config.get("Groups", key) {
        if text.is_empty() {
            let reason = match key {
                "StorageProvider" => "no storage provider configured",
                _ => "no connection string configured",
            };
            return Err(value("Groups", key, reason));
        }
        return Ok(Some(text));
    }
    Ok(config
        .get("DatabaseService", key)
        .filter(|text| !text.is_empty()))
}

fn local_store(config: &Config) -> Result<LocalStore, RegionConfigError> {
    match config.get("Groups", "Realm") {
        None | Some("os_groups") => {}
        Some(_) => {
            return Err(value(
                "Groups",
                "Realm",
                "local memberships read the os_groups tables",
            ));
        }
    }
    let Some(provider) = groups_setting(config, "StorageProvider")? else {
        return Err(value(
            "Groups",
            "StorageProvider",
            "no storage provider configured",
        ));
    };
    if !provider.to_ascii_lowercase().contains("sqlite") {
        return Err(value(
            "Groups",
            "StorageProvider",
            "local memberships read a SQLite os_groups store",
        ));
    }
    let Some(connection) = groups_setting(config, "ConnectionString")? else {
        return Err(value(
            "Groups",
            "ConnectionString",
            "no connection string configured",
        ));
    };
    if ferroan_db::sqlite_filename(connection).is_err() {
        return Err(value(
            "Groups",
            "ConnectionString",
            "not a SQLite connection string",
        ));
    }
    Ok(LocalStore {
        connection: connection.to_owned(),
    })
}

async fn local_memberships(
    store: &LocalStore,
    agent: Uuid,
) -> Result<BTreeMap<Uuid, u64>, GridError> {
    let pool = Pool::open(Backend::Sqlite, &store.connection, &["os_groups_Store"])
        .await
        .map_err(store_error)?;
    let result = match pool.sqlite() {
        Some(sqlite) => read_memberships(sqlite, agent).await,
        None => Err(GridError::Store(
            "the groups connection is not SQLite".to_owned(),
        )),
    };
    pool.close().await;
    result
}

async fn read_memberships(
    sqlite: &sqlx::SqlitePool,
    agent: Uuid,
) -> Result<BTreeMap<Uuid, u64>, GridError> {
    let agent_id = agent.to_string();
    let memberships = membership_ids(sqlite, &agent_id).await?;
    if memberships.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut seen = HashSet::new();
    let mut group_ids = Vec::new();
    for text in &memberships {
        let id = parse_uuid(text);
        if seen.insert(id) {
            // C# looks the group up by UUID.ToString(), which is lowercase.
            group_ids.push(id.to_string());
        }
    }
    let groups = group_rows(sqlite, &group_ids).await?;
    let roles = role_rows(sqlite, &group_ids).await?;
    let held = held_rows(sqlite, &agent_id).await?;
    assemble(&memberships, groups, roles, held, &agent_id)
}

async fn membership_ids(sqlite: &sqlx::SqlitePool, agent: &str) -> Result<Vec<String>, GridError> {
    within_column_bytes(
        sqlite,
        &["GroupID"],
        "FROM os_groups_membership WHERE PrincipalID = ?",
        &[agent],
    )
    .await?;
    let sql =
        format!("SELECT GroupID FROM os_groups_membership WHERE PrincipalID = ? LIMIT {ROW_FETCH}");
    let rows: Vec<(String,)> = sqlx::query_as(&sql)
        .bind(agent)
        .fetch_all(sqlite)
        .await
        .map_err(store_error)?;
    if rows.len() > MAX_MEMBERSHIP_ROWS {
        return Err(too_many_rows());
    }
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

async fn group_rows(
    sqlite: &sqlx::SqlitePool,
    ids: &[String],
) -> Result<Vec<(String, String)>, GridError> {
    let mut rows = Vec::new();
    for chunk in ids.chunks(GROUP_BATCH) {
        let mut filter = String::from("FROM os_groups_groups WHERE GroupID IN (");
        push_placeholders(&mut filter, chunk.len());
        filter.push(')');
        let binds: Vec<&str> = chunk.iter().map(String::as_str).collect();
        within_column_bytes(
            sqlite,
            &["GroupID", "CAST(OwnerRoleID AS TEXT)"],
            &filter,
            &binds,
        )
        .await?;
        let room = ROW_FETCH.saturating_sub(rows.len());
        let mut sql = String::from(
            "SELECT GroupID, CAST(OwnerRoleID AS TEXT) FROM os_groups_groups WHERE GroupID IN (",
        );
        push_placeholders(&mut sql, chunk.len());
        sql.push_str(&format!(") LIMIT {room}"));
        let mut query = sqlx::query_as::<_, (String, String)>(&sql);
        for id in chunk {
            query = query.bind(id);
        }
        let batch = query.fetch_all(sqlite).await.map_err(store_error)?;
        if rows.len() + batch.len() > MAX_MEMBERSHIP_ROWS {
            return Err(too_many_rows());
        }
        rows.extend(batch);
    }
    Ok(rows)
}

async fn role_rows(
    sqlite: &sqlx::SqlitePool,
    ids: &[String],
) -> Result<Vec<(String, String, String)>, GridError> {
    let mut rows = Vec::new();
    for chunk in ids.chunks(GROUP_BATCH) {
        let mut filter = String::from("FROM os_groups_roles WHERE GroupID IN (");
        push_placeholders(&mut filter, chunk.len());
        filter.push(')');
        let binds: Vec<&str> = chunk.iter().map(String::as_str).collect();
        within_column_bytes(
            sqlite,
            &["GroupID", "RoleID", "CAST(Powers AS TEXT)"],
            &filter,
            &binds,
        )
        .await?;
        let room = ROW_FETCH.saturating_sub(rows.len());
        let mut sql = String::from(
            "SELECT GroupID, RoleID, CAST(Powers AS TEXT) FROM os_groups_roles WHERE GroupID IN (",
        );
        push_placeholders(&mut sql, chunk.len());
        sql.push_str(&format!(") LIMIT {room}"));
        let mut query = sqlx::query_as::<_, (String, String, String)>(&sql);
        for id in chunk {
            query = query.bind(id);
        }
        let batch = query.fetch_all(sqlite).await.map_err(store_error)?;
        if rows.len() + batch.len() > MAX_MEMBERSHIP_ROWS {
            return Err(too_many_rows());
        }
        rows.extend(batch);
    }
    Ok(rows)
}

async fn held_rows(
    sqlite: &sqlx::SqlitePool,
    agent: &str,
) -> Result<Vec<(String, String, String)>, GridError> {
    within_column_bytes(
        sqlite,
        &["GroupID", "RoleID", "PrincipalID"],
        "FROM os_groups_rolemembership WHERE PrincipalID = ?",
        &[agent],
    )
    .await?;
    let sql = format!(
        "SELECT GroupID, RoleID, PrincipalID FROM os_groups_rolemembership WHERE PrincipalID = ? LIMIT {ROW_FETCH}"
    );
    let rows: Vec<(String, String, String)> = sqlx::query_as(&sql)
        .bind(agent)
        .fetch_all(sqlite)
        .await
        .map_err(store_error)?;
    if rows.len() > MAX_MEMBERSHIP_ROWS {
        return Err(too_many_rows());
    }
    Ok(rows)
}

fn push_placeholders(sql: &mut String, count: usize) {
    for index in 0..count {
        if index > 0 {
            sql.push(',');
        }
        sql.push('?');
    }
}

fn assemble(
    memberships: &[String],
    groups: Vec<(String, String)>,
    roles: Vec<(String, String, String)>,
    held: Vec<(String, String, String)>,
    agent: &str,
) -> Result<BTreeMap<Uuid, u64>, GridError> {
    let mut by_group = HashMap::new();
    for (group_id, owner) in groups {
        let id = parse_uuid(&group_id);
        by_group.entry(id).or_insert_with(|| StoredGroup {
            owner: parse_uuid(&owner),
            roles: HashMap::new(),
        });
    }
    for (group_id, role_id, powers) in roles {
        let id = parse_uuid(&group_id);
        let Some(group) = by_group.get_mut(&id) else {
            continue;
        };
        group
            .roles
            .entry(parse_uuid(&role_id))
            .or_insert(parse_powers(&powers));
    }
    let mut held_by_group: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (group_id, role_id, principal) in held {
        if !same_principal(&principal, agent) {
            continue;
        }
        held_by_group
            .entry(parse_uuid(&group_id))
            .or_default()
            .push(parse_uuid(&role_id));
    }
    let mut result = BTreeMap::new();
    for group_text in memberships {
        let group = parse_uuid(group_text);
        let Some(stored) = by_group.get(&group) else {
            continue;
        };
        let held_roles = held_by_group.get(&group).map(Vec::as_slice).unwrap_or(&[]);
        result.insert(group, powers_of(held_roles, &stored.roles, stored.owner));
    }
    Ok(result)
}

/// `GroupRules.PowersOf`: the owners role is every bit, even when its stored mask is zero.
fn powers_of(held: &[Uuid], roles: &HashMap<Uuid, Option<u64>>, owner: Uuid) -> u64 {
    if held.is_empty() {
        return 0;
    }
    let mut powers = 0u64;
    for role in held {
        if !owner.is_nil() && *role == owner {
            return u64::MAX;
        }
        if let Some(Some(mask)) = roles.get(role) {
            powers |= *mask;
        }
    }
    powers
}

/// OpenMetaverse `UUID.TryParse`: dashed or compact hex, with ASCII space
/// padding. A failure is nil, which drops that membership when no group row
/// has that id. A tab, braces and `urn:uuid:` fail.
fn parse_uuid(text: &str) -> Uuid {
    let text = text.trim_matches(' ');
    if matches!(text.len(), 32 | 36)
        && let Ok(id) = Uuid::parse_str(text)
    {
        return id;
    }
    Uuid::nil()
}

/// `UInt64.TryParse` with `NumberStyles.Integer`: surrounding spaces, no sign.
fn parse_powers(text: &str) -> Option<u64> {
    let trimmed = text.trim_matches(|c: char| matches!(c, '\t'..='\r' | ' '));
    if trimmed.is_empty() || trimmed.starts_with('+') || trimmed.starts_with('-') {
        return None;
    }
    trimmed.parse().ok()
}

fn same_principal(stored: &str, agent: &str) -> bool {
    stored.eq_ignore_ascii_case(agent)
}

fn byte_select(columns: &[&str]) -> String {
    let parts: Vec<String> = columns
        .iter()
        .map(|column| format!("coalesce(length(CAST(({column}) AS BLOB)),0)"))
        .collect();
    if parts.len() == 1 {
        format!("coalesce(max({}),0)", parts[0])
    } else {
        format!("coalesce(max(max({})),0)", parts.join(", "))
    }
}

async fn within_column_bytes(
    sqlite: &sqlx::SqlitePool,
    columns: &[&str],
    from: &str,
    binds: &[&str],
) -> Result<(), GridError> {
    let sql = format!("SELECT {} {from}", byte_select(columns));
    let mut query = sqlx::query_as::<_, (i64,)>(&sql);
    for value in binds {
        query = query.bind(*value);
    }
    let (longest,) = query.fetch_one(sqlite).await.map_err(store_error)?;
    if longest > MAX_COLUMN_BYTES {
        ferroan_diag::refuse(
            ferroan_diag::GROUPS_COLUMN_BYTES,
            ferroan_diag::Kind::Hard,
            MAX_COLUMN_BYTES as u64,
            longest as u64,
            "groups",
        );
        return Err(GridError::Store(format!(
            "a groups value is longer than {MAX_COLUMN_BYTES} bytes"
        )));
    }
    Ok(())
}

fn too_many_rows() -> GridError {
    ferroan_diag::refuse(
        ferroan_diag::GROUPS_MEMBERSHIP_ROWS,
        ferroan_diag::Kind::Hard,
        MAX_MEMBERSHIP_ROWS as u64,
        ROW_FETCH as u64,
        "groups",
    );
    GridError::Store(format!(
        "the groups read returned more than {MAX_MEMBERSHIP_ROWS} rows"
    ))
}

fn store_error(error: impl std::fmt::Display) -> GridError {
    GridError::Store(error.to_string())
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
            // Disabling the Remote connector must not hide the selected HG or XML-RPC service.
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

    #[test]
    fn power_text_follows_uint64_try_parse() {
        assert_eq!(parse_powers(" 8 "), Some(8));
        assert_eq!(parse_powers("8\t"), Some(8));
        assert_eq!(parse_powers("+8"), None);
        assert_eq!(parse_powers("-1"), None);
        assert_eq!(parse_powers("nope"), None);
        assert_eq!(parse_powers(""), None);
    }

    struct Scratch {
        dir: std::path::PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("ferroan-q86-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn groups(&self, name: &str) -> std::path::PathBuf {
            let dest = self.dir.join(name);
            std::fs::copy(fixture("db/sqlite/os_groups_Store.db"), &dest).unwrap();
            dest
        }

        fn land(&self) -> std::path::PathBuf {
            let dest = self.dir.join("OpenSim.db");
            std::fs::copy(fixture("regionstore/objects.db"), &dest).unwrap();
            dest
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture(path: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(path)
    }

    fn connection(path: &std::path::Path) -> String {
        format!("Data Source={};Version=3", path.display())
    }

    /// Provider from DatabaseService, connection and realm from Groups, as SQLiteStandalone.ini does.
    fn local_ini(connection: &str) -> Config {
        let mut config = Config::parse(
            "[Groups]\nEnabled=true\nModule=Groups Module V2\nServicesConnectorModule=Groups Local Service Connector\n",
        )
        .unwrap();
        config.set(
            "DatabaseService",
            "StorageProvider",
            "OpenSim.Data.SQLite.dll",
        );
        config.set("Groups", "ConnectionString", connection);
        config.set("Groups", "Realm", "os_groups");
        config
    }

    async fn apply_sql(path: &std::path::Path, sql: &str) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(false),
            )
            .await
            .unwrap();
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        pool.close().await;
    }

    fn captured_map(xml: &str) -> BTreeMap<Uuid, u64> {
        memberships(&ferroan_http::parse_xml_response(xml).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn local_memberships_match_the_pin8_store() {
        let scratch = Scratch::new();
        let db = scratch.groups("groups.db");
        apply_sql(
            &db,
            include_str!("../../../fixtures/group-memberships/seed.sql"),
        )
        .await;
        let client = GroupsClient::from_config(&local_ini(&connection(&db))).unwrap();
        assert_eq!(
            client.memberships(Uuid::from_u128(1)).await.unwrap(),
            captured_map(MEMBER)
        );
        assert_eq!(
            client.memberships(Uuid::from_u128(2)).await.unwrap(),
            captured_map(OWNER)
        );
        assert_eq!(
            client.memberships(Uuid::from_u128(3)).await.unwrap(),
            captured_map(NONMEMBER)
        );
    }

    #[tokio::test]
    async fn local_memberships_paint_the_captured_overlay() {
        let scratch = Scratch::new();
        let groups = scratch.groups("groups.db");
        apply_sql(
            &groups,
            include_str!("../../../fixtures/parcel-overlay/membership.sql"),
        )
        .await;
        let land = scratch.land();
        apply_sql(
            &land,
            include_str!("../../../fixtures/regionstore/parcels.sql"),
        )
        .await;
        apply_sql(
            &land,
            include_str!("../../../fixtures/parcel-overlay/home-region.sql"),
        )
        .await;
        let owner = uuid::uuid!("4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55");
        let member = uuid::uuid!("8c4e1a70-2b59-4d6f-9e13-5a7c0d8b2f44");
        let home = uuid::uuid!("6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01");
        let map =
            ferroan_scene::parcels::open_parcels(Some(&connection(&land)), home, [256, 256], owner)
                .await
                .unwrap();
        let client = GroupsClient::from_config(&local_ini(&connection(&groups))).unwrap();
        let member_groups = client.memberships(member).await.unwrap();
        let owner_groups = client.memberships(owner).await.unwrap();
        let other = client.memberships(Uuid::from_u128(99)).await.unwrap();
        assert!(member_groups.contains_key(&Uuid::from_u128(3)));
        assert!(owner_groups.is_empty());
        assert!(other.is_empty());
        assert_overlay(
            &map,
            member,
            |group| member_groups.contains_key(&group),
            &std::fs::read(fixture("parcel-overlay/parcel-multi-member.bin")).unwrap(),
        );
        assert_overlay(
            &map,
            owner,
            |group| owner_groups.contains_key(&group),
            &std::fs::read(fixture("parcel-overlay/parcel-multi-owner.bin")).unwrap(),
        );
    }

    fn assert_overlay(
        map: &ferroan_scene::parcels::ParcelMap,
        agent: Uuid,
        is_member: impl Fn(Uuid) -> bool,
        raw: &[u8],
    ) {
        assert_eq!(raw.len(), 4 * 1030);
        let mut packets = map.overlay(agent, is_member);
        for (index, body) in raw.as_chunks::<1030>().0.iter().enumerate() {
            let packet = packets.next().unwrap();
            assert_eq!(packet.sequence_id as usize, index);
            assert_eq!(&body[..4], &packet.sequence_id.to_le_bytes());
            assert_eq!(&body[4..6], &1024u16.to_le_bytes());
            assert_eq!(&body[6..], &packet.data);
        }
        assert!(packets.next().is_none());
    }

    #[tokio::test]
    async fn groups_connection_overrides_the_database_service() {
        let scratch = Scratch::new();
        let seeded = scratch.groups("seeded.db");
        let empty = scratch.groups("empty.db");
        apply_sql(
            &seeded,
            include_str!("../../../fixtures/group-memberships/seed.sql"),
        )
        .await;
        let mut config = Config::parse(
            "[Groups]\nEnabled=true\nModule=Groups Module V2\nServicesConnectorModule=Groups Local Service Connector\n",
        )
        .unwrap();
        config.set(
            "DatabaseService",
            "StorageProvider",
            "OpenSim.Data.MySQL.dll",
        );
        config.set("DatabaseService", "ConnectionString", &connection(&empty));
        config.set("Groups", "StorageProvider", "OpenSim.Data.SQLite.dll");
        config.set("Groups", "ConnectionString", &connection(&seeded));
        let client = GroupsClient::from_config(&config).unwrap();
        assert_eq!(
            client.memberships(Uuid::from_u128(1)).await.unwrap(),
            captured_map(MEMBER)
        );
    }

    #[tokio::test]
    async fn local_connector_refuses_a_missing_or_non_sqlite_store() {
        let mut config = Config::parse(
            "[Groups]\nEnabled=true\nModule=Groups Module V2\nServicesConnectorModule=Groups Local Service Connector\n",
        )
        .unwrap();
        assert!(matches!(
            GroupsClient::from_config(&config),
            Err(RegionConfigError::Value {
                key: "StorageProvider",
                ..
            })
        ));
        config.set("Groups", "StorageProvider", "OpenSim.Data.MySQL.dll");
        config.set(
            "Groups",
            "ConnectionString",
            "Server=db.invalid;Database=opensim;",
        );
        assert!(matches!(
            GroupsClient::from_config(&config),
            Err(RegionConfigError::Value {
                key: "StorageProvider",
                ..
            })
        ));
        config.set("Groups", "StorageProvider", "OpenSim.Data.SQLite.dll");
        config.set("Groups", "ConnectionString", "");
        assert!(matches!(
            GroupsClient::from_config(&config),
            Err(RegionConfigError::Value {
                key: "ConnectionString",
                ..
            })
        ));
        config.set(
            "Groups",
            "ConnectionString",
            "Server=db.invalid;Database=opensim;",
        );
        assert!(matches!(
            GroupsClient::from_config(&config),
            Err(RegionConfigError::Value {
                key: "ConnectionString",
                ..
            })
        ));
        config.set(
            "Groups",
            "ConnectionString",
            "Data Source=groups.db;Version=3",
        );
        config.set("Groups", "Realm", "custom");
        assert!(matches!(
            GroupsClient::from_config(&config),
            Err(RegionConfigError::Value { key: "Realm", .. })
        ));
        config.set(
            "Modules",
            "Setup_GroupsServiceLocalConnectorModule",
            "disabled",
        );
        let source = GroupsClient::from_config(&config).unwrap();
        assert!(source.memberships(Uuid::nil()).await.unwrap().is_empty());

        let missing =
            std::env::temp_dir().join(format!("ferroan-q86-missing-{}.db", Uuid::new_v4()));
        let client = GroupsClient::from_config(&local_ini(&connection(&missing))).unwrap();
        assert!(matches!(
            client.memberships(Uuid::nil()).await,
            Err(GridError::Store(_))
        ));
    }

    #[tokio::test]
    async fn local_store_skips_a_missing_group_and_an_unparsable_mask() {
        let scratch = Scratch::new();
        let db = scratch.groups("groups.db");
        let agent = Uuid::from_u128(0xbb);
        let masks = Uuid::from_u128(0x100);
        let gone = Uuid::from_u128(0x101);
        let titled = Uuid::from_u128(0x102);
        let negative = Uuid::from_u128(0x103);
        let eight = Uuid::from_u128(0x104);
        let junk = Uuid::from_u128(0x105);
        let four = Uuid::from_u128(0x106);
        let sql = format!(
            "INSERT INTO os_groups_groups(GroupID, Name, OwnerRoleID) VALUES
                ('{masks}', 'masks', '00000000-0000-0000-0000-000000000000'),
                ('{titled}', 'titled', 'not-a-uuid');
             INSERT INTO os_groups_roles(GroupID, RoleID, Name, Powers) VALUES
                ('{masks}', '{negative}', 'negative', -1),
                ('{masks}', '{eight}', 'eight', 8),
                ('{masks}', '{junk}', 'junk', 'nope'),
                ('{titled}', '{four}', 'four', 4);
             INSERT INTO os_groups_membership(GroupID, PrincipalID, SelectedRoleID) VALUES
                ('{masks}', '{agent}', '{eight}'),
                ('{gone}', '{agent}', '00000000-0000-0000-0000-000000000000'),
                ('{titled}', '{agent}', '{four}');
             INSERT INTO os_groups_rolemembership(GroupID, RoleID, PrincipalID) VALUES
                ('{masks}', '{negative}', '{agent}'),
                ('{masks}', '{eight}', '{agent}'),
                ('{masks}', '{junk}', '{agent}'),
                ('{titled}', '{four}', '{agent}');"
        );
        apply_sql(&db, &sql).await;
        let client = GroupsClient::from_config(&local_ini(&connection(&db))).unwrap();
        assert_eq!(
            client.memberships(agent).await.unwrap(),
            BTreeMap::from([(masks, 8), (titled, 4)])
        );
    }

    #[tokio::test]
    async fn an_uppercase_principal_id_is_not_a_membership() {
        let scratch = Scratch::new();
        let db = scratch.groups("groups.db");
        apply_sql(
            &db,
            include_str!("../../../fixtures/group-memberships/seed.sql"),
        )
        .await;
        let agent = Uuid::from_u128(0xcc);
        let stored = agent.to_string().to_ascii_uppercase();
        apply_sql(
            &db,
            &format!(
                "INSERT INTO os_groups_membership(GroupID, PrincipalID, SelectedRoleID)
                 VALUES ('00000000-0000-0000-0000-00000000000a', '{stored}', '00000000-0000-0000-0000-000000000000')"
            ),
        )
        .await;
        let client = GroupsClient::from_config(&local_ini(&connection(&db))).unwrap();
        assert!(client.memberships(agent).await.unwrap().is_empty());
    }

    #[test]
    fn uuid_text_follows_try_parse() {
        let east = Uuid::from_u128(0xa);
        assert_eq!(parse_uuid(" 00000000-0000-0000-0000-00000000000a"), east);
        assert_eq!(parse_uuid("00000000-0000-0000-0000-00000000000a "), east);
        assert_eq!(parse_uuid("0000000000000000000000000000000a"), east);
        assert_eq!(parse_uuid("00000000-0000-0000-0000-00000000000A"), east);
        assert!(parse_uuid("junk").is_nil());
        assert!(parse_uuid("{00000000-0000-0000-0000-00000000000a}").is_nil());
        assert!(parse_uuid("urn:uuid:00000000-0000-0000-0000-00000000000a").is_nil());
        assert!(parse_uuid("\t00000000-0000-0000-0000-00000000000a").is_nil());
    }

    async fn seeded_member(extra: &str) -> BTreeMap<Uuid, u64> {
        let scratch = Scratch::new();
        let db = scratch.groups("groups.db");
        apply_sql(
            &db,
            include_str!("../../../fixtures/group-memberships/seed.sql"),
        )
        .await;
        if !extra.is_empty() {
            apply_sql(&db, extra).await;
        }
        let client = GroupsClient::from_config(&local_ini(&connection(&db))).unwrap();
        client.memberships(Uuid::from_u128(1)).await.unwrap()
    }

    fn assert_pin8_member(map: &BTreeMap<Uuid, u64>) {
        assert_eq!(map, &captured_map(MEMBER));
        assert_eq!(map[&Uuid::from_u128(0xa)], 33_554_440);
        assert_eq!(map[&Uuid::from_u128(0xb)], 0);
    }

    #[tokio::test]
    async fn a_junk_group_id_row_is_skipped() {
        let member = "00000000-0000-0000-0000-000000000001";
        let junk_membership = format!(
            "INSERT INTO os_groups_membership(GroupID, PrincipalID, SelectedRoleID)
             VALUES ('junk', '{member}', '00000000-0000-0000-0000-000000000000')"
        );
        let junk_held = format!(
            "INSERT INTO os_groups_rolemembership(GroupID, RoleID, PrincipalID)
             VALUES ('junk', '00000000-0000-0000-0000-000000000000', '{member}')"
        );
        assert_pin8_member(&seeded_member(&junk_membership).await);
        assert_pin8_member(&seeded_member(&junk_held).await);
        assert_pin8_member(&seeded_member(&format!("{junk_membership}; {junk_held}")).await);

        let padded = format!(
            "UPDATE os_groups_membership SET GroupID = ' 00000000-0000-0000-0000-00000000000a'
             WHERE GroupID = '00000000-0000-0000-0000-00000000000a' AND PrincipalID = '{member}'"
        );
        assert_pin8_member(&seeded_member(&padded).await);

        let braced = format!(
            "UPDATE os_groups_membership SET GroupID = '{{00000000-0000-0000-0000-00000000000a}}'
             WHERE GroupID = '00000000-0000-0000-0000-00000000000a' AND PrincipalID = '{member}'"
        );
        let map = seeded_member(&braced).await;
        assert_eq!(map, BTreeMap::from([(Uuid::from_u128(0xb), 0)]));
    }

    #[tokio::test]
    async fn a_second_group_batch_is_read() {
        let scratch = Scratch::new();
        let db = scratch.groups("groups.db");
        let agent = Uuid::from_u128(0xaa);
        let count = 501;
        let mut groups_sql = String::from("INSERT INTO os_groups_groups(GroupID, Name) VALUES ");
        let mut members_sql = String::from(
            "INSERT INTO os_groups_membership(GroupID, PrincipalID, SelectedRoleID) VALUES ",
        );
        for n in 1..=count {
            let id = format!("00000000-0000-4000-8000-{n:012x}");
            if n > 1 {
                groups_sql.push(',');
                members_sql.push(',');
            }
            groups_sql.push_str(&format!("('{id}','batch-{n}')"));
            members_sql.push_str(&format!(
                "('{id}','{agent}','00000000-0000-0000-0000-000000000000')"
            ));
        }
        let last = format!("00000000-0000-4000-8000-{count:012x}");
        let roles_sql = format!(
            "INSERT INTO os_groups_roles(GroupID, RoleID, Name, Powers) VALUES
                ('{last}', '00000000-0000-0000-0000-000000000001', 'Land', 8);
             INSERT INTO os_groups_rolemembership(GroupID, RoleID, PrincipalID) VALUES
                ('{last}', '00000000-0000-0000-0000-000000000001', '{agent}')"
        );
        apply_sql(&db, &groups_sql).await;
        apply_sql(&db, &members_sql).await;
        apply_sql(&db, &roles_sql).await;
        let client = GroupsClient::from_config(&local_ini(&connection(&db))).unwrap();
        let map = client.memberships(agent).await.unwrap();
        let last_id = last.parse::<Uuid>().unwrap();
        assert_eq!(map.len(), count);
        assert_eq!(map[&last_id], 8);
        assert!(
            map.iter()
                .all(|(id, powers)| *id == last_id || *powers == 0)
        );
    }

    #[tokio::test]
    async fn more_than_4096_membership_rows_are_refused() {
        let scratch = Scratch::new();
        let db = scratch.groups("groups.db");
        let agent = Uuid::from_u128(0xdd);
        let mut sql = String::from(
            "INSERT INTO os_groups_membership(GroupID, PrincipalID, SelectedRoleID) VALUES ",
        );
        for n in 1..=4097 {
            if n > 1 {
                sql.push(',');
            }
            sql.push_str(&format!(
                "('00000000-0000-4000-8000-{n:012x}','{agent}','00000000-0000-0000-0000-000000000000')"
            ));
        }
        apply_sql(&db, &sql).await;
        let client = GroupsClient::from_config(&local_ini(&connection(&db))).unwrap();
        assert!(matches!(
            client.memberships(agent).await,
            Err(GridError::Store(_))
        ));
        assert!(ferroan_diag::render().contains("groups.membership-rows"));
    }

    #[tokio::test]
    async fn more_than_4096_group_rows_are_refused() {
        let scratch = Scratch::new();
        let db = scratch.groups("groups.db");
        apply_sql(
            &db,
            include_str!("../../../fixtures/group-memberships/seed.sql"),
        )
        .await;
        apply_sql(
            &db,
            "CREATE TABLE os_groups_groups_open AS SELECT * FROM os_groups_groups;
             DROP TABLE os_groups_groups;
             ALTER TABLE os_groups_groups_open RENAME TO os_groups_groups;
             INSERT INTO os_groups_groups (GroupID, Name, OwnerRoleID)
             SELECT '00000000-0000-0000-0000-00000000000a', 'dup-' || n, '00000000-0000-0000-0000-000000000000'
             FROM (WITH RECURSIVE seq(n) AS (
                 SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 4096
             ) SELECT n FROM seq);",
        )
        .await;
        let client = GroupsClient::from_config(&local_ini(&connection(&db))).unwrap();
        assert!(matches!(
            client.memberships(Uuid::from_u128(1)).await,
            Err(GridError::Store(_))
        ));
        assert!(ferroan_diag::render().contains("groups.membership-rows"));
    }

    #[tokio::test]
    async fn a_ten_megabyte_role_id_is_refused() {
        let scratch = Scratch::new();
        let db = scratch.groups("groups.db");
        apply_sql(
            &db,
            include_str!("../../../fixtures/group-memberships/seed.sql"),
        )
        .await;
        let mut role_id = String::from("00000000-0000-0000-0000-000000000010");
        role_id.extend(std::iter::repeat_n('a', 10 * 1024 * 1024));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&db)
                    .create_if_missing(false),
            )
            .await
            .unwrap();
        sqlx::query(
            "UPDATE os_groups_roles SET RoleID = ? WHERE GroupID = '00000000-0000-0000-0000-00000000000a' AND Name = 'Land'",
        )
        .bind(&role_id)
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        drop(role_id);
        let client = GroupsClient::from_config(&local_ini(&connection(&db))).unwrap();
        assert!(matches!(
            client.memberships(Uuid::from_u128(1)).await,
            Err(GridError::Store(_))
        ));
        assert!(ferroan_diag::render().contains("groups.column-bytes"));
    }
}
