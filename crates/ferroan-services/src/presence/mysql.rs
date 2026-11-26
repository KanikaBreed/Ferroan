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

// Ported from Source/OpenSim.Data.MySQL/{MySQLPresenceData.cs,
// MySQLGenericTableHandler.cs, MySQLFramework.cs, Resources/Presence.migrations}
// at KanikaBreed/OpenSim-Tranquillity c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;
use std::sync::Arc;

use sqlx::{
    MySqlPool, QueryBuilder, Row,
    mysql::{MySqlQueryResult, MySqlRow},
};
use uuid::Uuid;

use super::{
    MAX_RESULTS, MAX_ROW_BYTES, PresenceData, PresenceFuture, PresenceStore, PresenceStoreError,
    check,
};
use ferroan_diag::{PRESENCE_RESULTS, PRESENCE_ROW_BYTES};

/// The Presence v4 table created by the Tranquillity MySQL migration.
#[derive(Clone)]
pub struct MySqlPresenceData {
    pool: MySqlPool,
    realm: String,
}

impl MySqlPresenceData {
    pub fn open(pool: MySqlPool) -> Result<Self, PresenceStoreError> {
        Self::new(pool, "Presence")
    }

    /// Use the table named by `[PresenceService] Realm`.
    pub fn new(pool: MySqlPool, realm: &str) -> Result<Self, PresenceStoreError> {
        if !crate::sql_ident(realm) {
            return Err(PresenceStoreError::Realm);
        }
        Ok(Self {
            pool,
            realm: realm.to_owned(),
        })
    }
}

const COLUMNS: [&str; 5] = [
    "UserID",
    "RegionID",
    "SessionID",
    "SecureSessionID",
    "LastSeen",
];

fn column(field: &str, value: &str) -> Result<Option<&'static str>, PresenceStoreError> {
    check(
        PRESENCE_ROW_BYTES,
        MAX_ROW_BYTES,
        field.len().max(value.len()),
    )?;
    Ok(COLUMNS
        .into_iter()
        .find(|name| name.eq_ignore_ascii_case(field)))
}

// MySQLFramework returns zero on a refused statement. Opening the connection
// is outside that handler, so acquisition failures still reach the caller.
fn wrote(result: Result<MySqlQueryResult, sqlx::Error>) -> bool {
    match result {
        Ok(result) => result.rows_affected() != 0,
        Err(error) => {
            // Server messages can contain the rejected field value, including a secure session ID.
            if let Some(db) = error
                .as_database_error()
                .and_then(|e| e.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>())
            {
                tracing::warn!(
                    number = db.number(),
                    sqlstate = db.code(),
                    "MySQL Presence statement failed"
                );
            } else {
                tracing::warn!(error = %error, "MySQL Presence statement failed");
            }
            false
        }
    }
}

// On the v4 schema MySqlConnector reads each CHAR(36) as a Guid, including
// SecureSessionID in the additional fields dictionary.
fn guid(row: &MySqlRow, column: &str) -> Result<Uuid, sqlx::Error> {
    Uuid::parse_str(row.try_get(column)?).map_err(|err| sqlx::Error::ColumnDecode {
        index: column.into(),
        source: Box::new(err),
    })
}

impl PresenceStore for MySqlPresenceData {
    fn store(&self, row: Arc<PresenceData>) -> PresenceFuture<'_, bool> {
        Box::pin(async move {
            row.validate()?;
            let mut query = QueryBuilder::new(format!("REPLACE INTO {} SET UserID = ", self.realm));
            query
                .push_bind(&row.user_id)
                .push(", RegionID = ")
                .push_bind(row.region_id().to_string())
                .push(", SessionID = ")
                .push_bind(row.session_id.to_string());
            // Omitting either additional column requests its migration default.
            let mut added = Vec::new();
            for (field, value) in &row.data {
                let Some(name @ ("SecureSessionID" | "LastSeen")) = column(field, value)? else {
                    return Ok(false);
                };
                if added.contains(&name) {
                    return Ok(false);
                }
                added.push(name);
                query.push(", ").push(name).push(" = ").push_bind(value);
            }
            let mut conn = self.pool.acquire().await?;
            Ok(wrote(query.build().execute(&mut *conn).await))
        })
    }

    fn get(&self, session: Uuid) -> PresenceFuture<'_, Option<Arc<PresenceData>>> {
        Box::pin(async move {
            Ok(self
                .get_by_field("SessionID", &session.to_string())
                .await?
                .pop())
        })
    }

    fn get_by_field<'a>(
        &'a self,
        field: &'a str,
        value: &'a str,
    ) -> PresenceFuture<'a, Vec<Arc<PresenceData>>> {
        Box::pin(async move {
            let field = column(field, value)?
                .ok_or_else(|| sqlx::Error::ColumnNotFound(field.to_owned()))?;
            // The SQL timestamp spelling is suitable for binding on re-store (D84).
            let query = format!(
                "SELECT UserID, SessionID, RegionID, SecureSessionID, CAST(LastSeen AS CHAR) AS LastSeen FROM {} WHERE {field} = ? LIMIT ?",
                self.realm
            );
            let rows = sqlx::query(&query)
                .bind(value)
                .bind((MAX_RESULTS + 1) as u32)
                .fetch_all(&self.pool)
                .await?;
            check(PRESENCE_RESULTS, MAX_RESULTS, rows.len())?;
            rows.into_iter()
                .map(|r| {
                    Ok(Arc::new(PresenceData::new(
                        r.try_get("UserID")?,
                        guid(&r, "SessionID")?,
                        guid(&r, "RegionID")?,
                        BTreeMap::from([
                            (
                                "SecureSessionID".into(),
                                guid(&r, "SecureSessionID")?.to_string(),
                            ),
                            ("LastSeen".into(), r.try_get("LastSeen")?),
                        ]),
                    )))
                })
                .collect()
        })
    }

    fn delete<'a>(&'a self, field: &'a str, value: &'a str) -> PresenceFuture<'a, bool> {
        Box::pin(async move {
            let Some(field) = column(field, value)? else {
                return Ok(false);
            };
            let query = format!("DELETE FROM {} WHERE {field} = ?", self.realm);
            let mut conn = self.pool.acquire().await?;
            Ok(wrote(
                sqlx::query(&query).bind(value).execute(&mut *conn).await,
            ))
        })
    }

    fn report_agent(&self, session: Uuid, region: Uuid) -> PresenceFuture<'_, bool> {
        Box::pin(async move {
            if self.get(session).await?.is_none() || region.is_nil() {
                return Ok(false);
            }
            let mut conn = self.pool.acquire().await?;
            Ok(wrote(
                sqlx::query(&format!(
                    "UPDATE {} SET RegionID = ?, LastSeen = NOW() WHERE SessionID = ?",
                    self.realm
                ))
                .bind(region.to_string())
                .bind(session.to_string())
                .execute(&mut *conn)
                .await,
            ))
        })
    }

    fn logout_region_agents(&self, region: Uuid) -> PresenceFuture<'_, ()> {
        Box::pin(async move {
            self.delete("RegionID", &region.to_string()).await?;
            Ok(())
        })
    }

    fn verify_agent(&self, agent: Uuid, secure_session: Uuid) -> PresenceFuture<'_, bool> {
        Box::pin(async move {
            let user: Option<String> = sqlx::query_scalar(&format!(
                "SELECT UserID FROM {} WHERE SecureSessionID = ? LIMIT 1",
                self.realm
            ))
            .bind(secure_session.to_string())
            .fetch_optional(&self.pool)
            .await?;
            Ok(user.is_some_and(|user| user == agent.to_string()))
        })
    }
}
