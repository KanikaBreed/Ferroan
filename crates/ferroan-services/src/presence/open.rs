/*
 * Copyright (c) Contributors, http://opensimulator.org/
 * See CONTRIBUTORS.TXT for a full list of copyright holders.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *     * Redistributions of source code must retain the above copyright
 *       notice, this list of conditions and the following disclaimer.
 *     * Redistributions in binary form must reproduce the above copyright
 *       notice, this list of conditions and the following disclaimer in the
 *       documentation and/or other materials provided with the distribution.
 *     * Neither the name of the OpenSimulator Project nor the
 *       names of its contributors may be used to endorse or promote products
 *       derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
 * EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

// Ported from Tranquillity Source/OpenSim.Services.PresenceService/PresenceServiceBase.cs and OpenSim.Server.Handlers/Presence/PresenceServerConnector.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    MySqlPresenceData, NullPresenceData, PgPresenceData, PresenceHttp, PresenceService,
    PresenceStore, PresenceStoreError,
};
use ferroan_config::Config;
use ferroan_db::{Backend, Pool};
use std::sync::Arc;

const SECTION: &str = "PresenceService";

#[derive(Debug, thiserror::Error)]
pub enum PresenceError {
    #[error("Presence configuration: {0}")]
    Config(&'static str),
    #[error(transparent)]
    Database(#[from] ferroan_db::DbError),
    #[error(transparent)]
    Store(#[from] PresenceStoreError),
}

impl PresenceHttp {
    pub async fn open(config: &Config) -> Result<Self, PresenceError> {
        if !matches!(
            config.get(SECTION, "LocalServiceModule"),
            Some(
                "PresenceService"
                    | "OpenSim.Services.PresenceService.dll"
                    | "OpenSim.Services.PresenceService.dll:PresenceService"
            )
        ) {
            return Err(PresenceError::Config(
                "expected LocalServiceModule PresenceService",
            ));
        }
        if option(config, "Network", "https_main", false)? {
            return Err(PresenceError::Config("HTTPS is unsupported"));
        }
        let auth = [SECTION, "Network"]
            .into_iter()
            .find_map(|s| config.get(s, "AuthType").filter(|v| !v.is_empty()))
            .unwrap_or("None");
        if !matches!(auth, "None" | "TrustedGridAuthentication") {
            return Err(PresenceError::Config("unsupported AuthType"));
        }
        let allow_shard = option(
            config,
            SECTION,
            "AllowllHTTPRequestIn",
            option(config, "Network", "AllowllHTTPRequestIn", false)?,
        )?;
        let duplicates = option(config, SECTION, "AllowDuplicatePresences", false)?;
        let setting = |key| {
            config
                .get(SECTION, key)
                .or_else(|| config.get("DatabaseService", key))
        };
        let provider = setting("StorageProvider").unwrap_or("");
        let mut pool = None;
        let store: Arc<dyn PresenceStore> = match provider {
            "OpenSim.Data.Null.dll" | "OpenSim.Data.Null.dll:NullPresenceData" => {
                Arc::new(NullPresenceData::default())
            }
            "OpenSim.Data.MySQL.dll"
            | "OpenSim.Data.MySQL.dll:MySQLPresenceData"
            | "OpenSim.Data.PGSQL.dll"
            | "OpenSim.Data.PGSQL.dll:PGSQLPresenceData" => {
                let backend = if provider.starts_with("OpenSim.Data.MySQL.") {
                    Backend::Mysql
                } else {
                    Backend::Postgres
                };
                let realm = config.get(SECTION, "Realm").unwrap_or("Presence");
                let conn = setting("ConnectionString")
                    .filter(|s| !s.is_empty())
                    .ok_or(PresenceError::Config("missing ConnectionString"))?;
                let db = Pool::open(backend, conn, &["Presence"]).await?;
                let store: Arc<dyn PresenceStore> = if let Some(mysql) = db.mysql() {
                    Arc::new(MySqlPresenceData::new(mysql.clone(), realm)?)
                } else {
                    let pg = db
                        .postgres()
                        .ok_or(PresenceError::Config("expected a PostgreSQL pool"))?;
                    Arc::new(PgPresenceData::new(pg.clone(), realm)?)
                };
                pool = Some(Arc::new(db));
                store
            }
            _ => {
                return Err(PresenceError::Config(
                    "expected a Null, MySQL or PostgreSQL StorageProvider",
                ));
            }
        };
        Ok(Self {
            service: Arc::new(PresenceService::new(store, duplicates)),
            reject_shard: !allow_shard,
            pool,
        })
    }
}

fn option(config: &Config, section: &str, key: &str, default: bool) -> Result<bool, PresenceError> {
    match config
        .get(section, key)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "" => Ok(default),
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(PresenceError::Config("invalid boolean option")),
    }
}
