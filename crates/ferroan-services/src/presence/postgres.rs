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

// Ported from Source/OpenSim.Data.PGSQL/PGSQLPresenceData.cs,
// PGSQLGenericTableHandler.cs, PGSQLManager.cs and Resources/Presence.migrations
// at KanikaBreed/OpenSim-Tranquillity c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The PostgreSQL Presence v4 table. SQL rows are detached from the stored session.

use std::{collections::BTreeMap, sync::Arc};

use ferroan_diag::{PRESENCE_RESULTS, PRESENCE_ROW_BYTES};
use sqlx::{PgPool, Postgres, QueryBuilder, Row, postgres::PgRow};
use uuid::Uuid;

use super::{
    MAX_RESULTS, MAX_ROW_BYTES, PresenceData, PresenceFuture, PresenceStore, PresenceStoreError,
    check, parse_id,
};

#[derive(Clone)]
pub struct PgPresenceData {
    pool: PgPool,
    realm: String,
}

impl PgPresenceData {
    /// Use the pool for the database with the Presence v4 migration.
    pub fn open(pool: PgPool) -> Result<Self, PresenceStoreError> {
        Self::new(pool, "Presence")
    }

    /// Use the table named by `[PresenceService] Realm`. Pin8 leaves its
    /// identifier unquoted, so PostgreSQL folds ASCII capitals to lowercase.
    pub fn new(pool: PgPool, realm: &str) -> Result<Self, PresenceStoreError> {
        if !crate::sql_ident(realm) {
            return Err(PresenceStoreError::Realm);
        }
        Ok(Self {
            pool,
            realm: realm.to_owned(),
        })
    }
}

// sqlx sets DateStyle to ISO on connection. PostgreSQL can represent and
// re-read every timestamp it stores, including infinity, at full precision.
const SELECT: &str = r#"SELECT "UserID", "SessionID", "RegionID", "SecureSessionID", "LastSeen"::text AS "LastSeen""#;

fn read_row(row: &PgRow) -> Result<Arc<PresenceData>, sqlx::Error> {
    let secure: Uuid = row.try_get("SecureSessionID")?;
    let stamp: Option<String> = row.try_get("LastSeen")?;
    Ok(Arc::new(PresenceData::new(
        row.try_get("UserID")?,
        row.try_get("SessionID")?,
        row.try_get("RegionID")?,
        BTreeMap::from([
            ("SecureSessionID".into(), secure.to_string()),
            ("LastSeen".into(), stamp.unwrap_or_default()),
        ]),
    )))
}

fn filter<'a>(
    query: &mut QueryBuilder<'a, Postgres>,
    field: &str,
    value: &'a str,
) -> Result<(), PresenceStoreError> {
    check(
        PRESENCE_ROW_BYTES,
        MAX_ROW_BYTES,
        field.len().max(value.len()),
    )?;
    // Field names are quoted (case-sensitive). PGSQLManager binds a UUID
    // even when UUID.TryParse failed, in which case the value is zero.
    match field {
        "UserID" => {
            query.push(r#" WHERE "UserID" = "#).push_bind(value);
        }
        "RegionID" | "SessionID" | "SecureSessionID" => {
            query
                .push(format!(r#" WHERE "{field}" = "#))
                .push_bind(parse_id(value).unwrap_or_default());
        }
        "LastSeen" => {
            query
                .push(r#" WHERE "LastSeen" = "#)
                .push_bind(value)
                .push("::text::timestamp");
        }
        _ => return Err(sqlx::Error::ColumnNotFound(field.into()).into()),
    }
    Ok(())
}

impl PresenceStore for PgPresenceData {
    fn store(&self, row: Arc<PresenceData>) -> PresenceFuture<'_, bool> {
        Box::pin(async move {
            row.validate()?;
            if let Some(field) = row
                .data
                .keys()
                .find(|k| !matches!(k.as_str(), "SecureSessionID" | "LastSeen"))
            {
                return Err(sqlx::Error::ColumnNotFound(field.clone()).into());
            }
            let secure = row
                .data
                .get("SecureSessionID")
                .map(|v| parse_id(v).unwrap_or_default());
            // No key omits the column. An empty field explicitly writes SQL NULL.
            let stamp = row
                .data
                .get("LastSeen")
                .map(|v| (!v.is_empty()).then_some(v.as_str()));
            let region = row.region_id();
            let mut update = QueryBuilder::new(format!(r#"UPDATE {} SET "UserID" = "#, self.realm));
            update
                .push_bind(&row.user_id)
                .push(r#", "RegionID" = "#)
                .push_bind(region);
            if let Some(secure) = secure {
                update.push(r#", "SecureSessionID" = "#).push_bind(secure);
            }
            if let Some(stamp) = stamp {
                update
                    .push(r#", "LastSeen" = "#)
                    .push_bind(stamp)
                    .push("::text::timestamp");
            }
            update
                .push(r#" WHERE "SessionID" = "#)
                .push_bind(row.session_id);
            let mut conn = self.pool.acquire().await?;
            if update.build().execute(&mut *conn).await?.rows_affected() != 0 {
                return Ok(true);
            }
            let mut insert = QueryBuilder::new(format!(
                r#"INSERT INTO {} ("UserID","RegionID","SessionID","SecureSessionID","LastSeen") VALUES ("#,
                self.realm
            ));
            let mut values = insert.separated(", ");
            values
                .push_bind(&row.user_id)
                .push_bind(region)
                .push_bind(row.session_id);
            if let Some(secure) = secure {
                values.push_bind(secure);
            } else {
                values.push("DEFAULT");
            }
            if let Some(stamp) = stamp {
                values
                    .push_bind(stamp)
                    .push_unseparated("::text::timestamp");
            } else {
                values.push("DEFAULT");
            }
            insert.push(")");
            Ok(insert.build().execute(&mut *conn).await?.rows_affected() != 0)
        })
    }

    fn get(&self, session: Uuid) -> PresenceFuture<'_, Option<Arc<PresenceData>>> {
        Box::pin(async move {
            let sql = format!(r#"{SELECT} FROM {} WHERE "SessionID" = $1"#, self.realm);
            let row = sqlx::query(&sql)
                .bind(session)
                .fetch_optional(&self.pool)
                .await?;
            Ok(row.as_ref().map(read_row).transpose()?)
        })
    }

    fn get_by_field<'a>(
        &'a self,
        field: &'a str,
        value: &'a str,
    ) -> PresenceFuture<'a, Vec<Arc<PresenceData>>> {
        Box::pin(async move {
            let mut query = QueryBuilder::new(format!("{SELECT} FROM {}", self.realm));
            filter(&mut query, field, value)?;
            query.push(" LIMIT ").push_bind((MAX_RESULTS + 1) as i64);
            let rows = query.build().fetch_all(&self.pool).await?;
            check(PRESENCE_RESULTS, MAX_RESULTS, rows.len())?;
            Ok(rows.iter().map(read_row).collect::<Result<_, _>>()?)
        })
    }

    fn delete<'a>(&'a self, field: &'a str, value: &'a str) -> PresenceFuture<'a, bool> {
        Box::pin(async move {
            let mut query = QueryBuilder::new(format!("DELETE FROM {}", self.realm));
            filter(&mut query, field, value)?;
            Ok(query.build().execute(&self.pool).await?.rows_affected() != 0)
        })
    }

    fn report_agent(&self, session: Uuid, region: Uuid) -> PresenceFuture<'_, bool> {
        Box::pin(async move {
            if self.get(session).await?.is_none() || region.is_nil() {
                return Ok(false);
            }
            Ok(sqlx::query(&format!(
                r#"UPDATE {} SET "RegionID"=$1, "LastSeen"=now() WHERE "SessionID"=$2"#,
                self.realm
            ))
            .bind(region)
            .bind(session)
            .execute(&self.pool)
            .await?
            .rows_affected()
                != 0)
        })
    }

    fn logout_region_agents(&self, region: Uuid) -> PresenceFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query(&format!(
                r#"DELETE FROM {} WHERE "RegionID"=$1"#,
                self.realm
            ))
            .bind(region)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn verify_agent(&self, agent: Uuid, secure_session: Uuid) -> PresenceFuture<'_, bool> {
        Box::pin(async move {
            let user: Option<String> = sqlx::query_scalar(&format!(
                r#"SELECT "UserID" FROM {} WHERE "SecureSessionID"=$1 LIMIT 1"#,
                self.realm
            ))
            .bind(secure_session)
            .fetch_optional(&self.pool)
            .await?;
            Ok(user.as_deref() == Some(&agent.to_string()))
        })
    }
}

#[cfg(test)]
mod tests;
