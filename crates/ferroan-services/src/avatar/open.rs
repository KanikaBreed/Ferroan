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
// Ported from Source/OpenSim.Services.AvatarService/AvatarServiceBase.cs and
// Source/OpenSim.Server.Handlers/Avatar/AvatarServerConnector.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Open Pin8's configured AvatarService over a migrated Avatar database.
use ferroan_config::Config;
use ferroan_db::{Backend, Pool};

use super::http::AvatarHttp;
use super::model::DEFAULT_REALM;
use super::service::AvatarService;
use super::store::AvatarStore;

#[derive(Debug, thiserror::Error)]
pub enum AvatarError {
    #[error("{0}")]
    Config(&'static str),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Db(#[from] ferroan_db::DbError),
    #[error(transparent)]
    Store(#[from] super::store::StoreError),
}

const SECTION: &str = "AvatarService";

impl AvatarHttp {
    pub async fn open(config: &Config) -> Result<Self, AvatarError> {
        let module = nonempty(config, SECTION, "LocalServiceModule")
            .ok_or(AvatarError::Config("No LocalServiceModule in config file"))?;
        if !matches!(
            module,
            "AvatarService" | "OpenSim.Services.AvatarService.dll:AvatarService"
        ) {
            return Err(AvatarError::Config(
                "only OpenSim.Services.AvatarService.dll:AvatarService is served",
            ));
        }
        refuse_transport(config)?;
        let provider = config
            .get(SECTION, "StorageProvider")
            .or_else(|| config.get("DatabaseService", "StorageProvider"))
            .filter(|value| !value.is_empty())
            .ok_or(AvatarError::Config("No StorageProvider configured"))?;
        let backend = avatar_backend(provider)?;
        let connection = config
            .get(SECTION, "ConnectionString")
            .or_else(|| config.get("DatabaseService", "ConnectionString"))
            .filter(|value| !value.is_empty())
            .ok_or(AvatarError::Config("No ConnectionString configured"))?;
        let realm = config.get(SECTION, "Realm").unwrap_or(DEFAULT_REALM);
        let pool = Pool::open(backend, connection, &["Avatar"]).await?;
        let store = AvatarStore::new(pool, realm)?;
        let service = AvatarService::new(store);
        let reject_shard_header = !config_bool_either(config, "AllowllHTTPRequestIn", false)?;
        Ok(Self::new(service, reject_shard_header))
    }
}

fn refuse_transport(config: &Config) -> Result<(), AvatarError> {
    if config_bool(config, "Network", "https_main", false)? {
        return Err(AvatarError::Config(
            "https_main is set and this build serves HTTP only",
        ));
    }
    let auth = auth_type(config);
    if auth != "None" && auth != "TrustedGridAuthentication" {
        return Err(AvatarError::Message(format!(
            "AuthType {auth} is not implemented"
        )));
    }
    Ok(())
}

fn avatar_backend(provider: &str) -> Result<Backend, AvatarError> {
    if provider.contains("SQLite") {
        Ok(Backend::Sqlite)
    } else if provider.contains("MySQL") {
        Ok(Backend::Mysql)
    } else if provider.contains("PGSQL") || provider.contains("Postgres") {
        Ok(Backend::Postgres)
    } else {
        Err(AvatarError::Config(
            "this avatar service opens SQLite, MySQL and PostgreSQL stores",
        ))
    }
}

fn auth_type(config: &Config) -> &str {
    let mut found = "None";
    for section in ["Network", SECTION] {
        if let Some(value) = nonempty(config, section, "AuthType") {
            found = value;
        }
    }
    found
}

fn config_bool_either(config: &Config, key: &str, default: bool) -> Result<bool, AvatarError> {
    let mut value = default;
    for section in ["Network", SECTION] {
        if config.get(section, key).is_some() {
            value = config_bool(config, section, key, value)?;
        }
    }
    Ok(value)
}

fn config_bool(
    config: &Config,
    section: &str,
    key: &str,
    default: bool,
) -> Result<bool, AvatarError> {
    let Some(text) = config.get(section, key) else {
        return Ok(default);
    };
    if text.is_empty() {
        return Ok(default);
    }
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(AvatarError::Message(format!(
            "{section} {key} is not a boolean"
        ))),
    }
}

fn nonempty<'a>(config: &'a Config, section: &str, key: &str) -> Option<&'a str> {
    config.get(section, key).filter(|value| !value.is_empty())
}
