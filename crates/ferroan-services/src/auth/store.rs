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
//   Source/OpenSim.Data.SQLite/SQLiteAuthenticationData.cs
//   Source/OpenSim.Data.MySQL/MySQLAuthenticationData.cs
//   Source/OpenSim.Data.PGSQL/PGSQLAuthenticationData.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `auth` and `tokens` rows.
//!
//! The column list is fixed here. SQLite and PostgreSQL publish a shared name
//! list and then a query on another thread can read it (TB8, `835d6cfe77`).
//! sqlx has no equivalent list. A MySQL `UPDATE` that changes nothing reports
//! zero rows, and the pin's update-then-insert then hits the primary key.
//! This store updates when the row is already present.
//!
//! Token times use the database clock on SQLite and MySQL, and the process
//! clock on PostgreSQL, which is what each plugin binds. Expired tokens are
//! deleted on the first token call and again after 30 seconds. The pin's
//! `Environment.TickCount` is already past that gap at startup.

use std::time::{Duration, Instant};

use chrono::{Local, TimeDelta};
use ferroan_db::Pool;
use sqlx::Row;
use uuid::Uuid;

use super::model::AuthRow;

#[derive(Debug, thiserror::Error)]
pub(crate) enum StoreError {
    #[error("authentication database is not sqlite, mysql or postgres")]
    Backend,
    #[error("authentication database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("authentication row is not readable: {0}")]
    Row(String),
    #[error("authentication table name is not an identifier")]
    Realm,
}

pub(crate) struct AuthStore {
    pool: Pool,
    realm: String,
    last_expire: std::sync::Mutex<Option<Instant>>,
}

impl AuthStore {
    pub(crate) fn pool_occupancy(&self) -> (u32, u32) {
        self.pool.occupancy()
    }

    pub(crate) fn new(pool: Pool, realm: &str) -> Result<Self, StoreError> {
        if !is_identifier(realm) {
            return Err(StoreError::Realm);
        }
        if pool.sqlite().is_none() && pool.mysql().is_none() && pool.postgres().is_none() {
            return Err(StoreError::Backend);
        }
        Ok(Self {
            pool,
            realm: realm.to_string(),
            last_expire: std::sync::Mutex::new(None),
        })
    }

    pub(crate) async fn get(&self, principal: Uuid) -> Result<Option<AuthRow>, StoreError> {
        if self.pool.sqlite().is_some() {
            self.get_sqlite(principal).await
        } else if self.pool.mysql().is_some() {
            self.get_mysql(principal).await
        } else if self.pool.postgres().is_some() {
            self.get_postgres(principal).await
        } else {
            Err(StoreError::Backend)
        }
    }

    pub(crate) async fn store(&self, row: &AuthRow) -> Result<bool, StoreError> {
        if self.get(row.principal_id).await?.is_some() {
            self.update(row).await
        } else {
            self.insert(row).await
        }
    }

    /// `SetDataItem`. The column name is part of the statement, so it has to
    /// be one of the four auth columns. SQLite also asks `pragma_table_info`.
    /// The HTTP handler stores a whole row. This is the data-plugin method.
    #[allow(dead_code)]
    pub(crate) async fn set_data_item(
        &self,
        principal: Uuid,
        item: &str,
        value: &str,
    ) -> Result<bool, StoreError> {
        let Some(column) = canonical_column(item) else {
            return Ok(false);
        };
        if self.pool.sqlite().is_some() && !self.sqlite_has_column(column).await? {
            return Ok(false);
        }
        let id = principal.to_string();
        let changed = if let Some(pool) = self.pool.sqlite() {
            let sql = format!("UPDATE `{}` SET `{column}` = ? WHERE UUID = ?", self.realm);
            sqlx::query(&sql)
                .bind(value)
                .bind(&id)
                .execute(pool)
                .await?
                .rows_affected()
        } else if let Some(pool) = self.pool.mysql() {
            let sql = format!("UPDATE `{}` SET `{column}` = ? WHERE UUID = ?", self.realm);
            sqlx::query(&sql)
                .bind(value)
                .bind(&id)
                .execute(pool)
                .await?
                .rows_affected()
        } else if let Some(pool) = self.pool.postgres() {
            let sql = format!(
                "UPDATE {} SET \"{column}\" = $1 WHERE uuid = $2",
                self.realm
            );
            sqlx::query(&sql)
                .bind(value)
                .bind(principal)
                .execute(pool)
                .await?
                .rows_affected()
        } else {
            return Err(StoreError::Backend);
        };
        Ok(changed > 0)
    }

    pub(crate) async fn set_token(
        &self,
        principal: Uuid,
        token: &str,
        lifetime: i32,
    ) -> Result<bool, StoreError> {
        self.expire_if_due().await?;
        let changed = if let Some(pool) = self.pool.sqlite() {
            sqlx::query(
                "INSERT INTO tokens (UUID, token, validity) VALUES (?, ?, datetime('now', 'localtime', ?))",
            )
            .bind(principal.to_string())
            .bind(token)
            .bind(lifetime_modifier(lifetime))
            .execute(pool)
            .await?
            .rows_affected()
        } else if let Some(pool) = self.pool.mysql() {
            sqlx::query(
                "INSERT INTO tokens (UUID, token, validity) VALUES (?, ?, DATE_ADD(NOW(), INTERVAL ? MINUTE))",
            )
            .bind(principal.to_string())
            .bind(token)
            .bind(lifetime)
            .execute(pool)
            .await?
            .rows_affected()
        } else if let Some(pool) = self.pool.postgres() {
            sqlx::query("INSERT INTO tokens (uuid, token, validity) VALUES ($1, $2, $3::timestamp)")
                .bind(principal)
                .bind(token)
                .bind(pg_when(lifetime)?)
                .execute(pool)
                .await?
                .rows_affected()
        } else {
            return Err(StoreError::Backend);
        };
        Ok(changed > 0)
    }

    pub(crate) async fn check_token(
        &self,
        principal: Uuid,
        token: &str,
        lifetime: i32,
    ) -> Result<bool, StoreError> {
        self.expire_if_due().await?;
        let changed = if let Some(pool) = self.pool.sqlite() {
            sqlx::query(
                "UPDATE tokens SET validity = datetime('now', 'localtime', ?) WHERE UUID = ? AND token = ? AND validity > datetime('now', 'localtime')",
            )
            .bind(lifetime_modifier(lifetime))
            .bind(principal.to_string())
            .bind(token)
            .execute(pool)
            .await?
            .rows_affected()
        } else if let Some(pool) = self.pool.mysql() {
            sqlx::query(
                "UPDATE tokens SET validity = DATE_ADD(NOW(), INTERVAL ? MINUTE) WHERE UUID = ? AND token = ? AND validity > NOW()",
            )
            .bind(lifetime)
            .bind(principal.to_string())
            .bind(token)
            .execute(pool)
            .await?
            .rows_affected()
        } else if let Some(pool) = self.pool.postgres() {
            sqlx::query(
                "UPDATE tokens SET validity = $1::timestamp WHERE uuid = $2 AND token = $3 AND validity > (CURRENT_DATE + CURRENT_TIME)",
            )
            .bind(pg_when(lifetime)?)
            .bind(principal)
            .bind(token)
            .execute(pool)
            .await?
            .rows_affected()
        } else {
            return Err(StoreError::Backend);
        };
        Ok(changed > 0)
    }

    async fn expire_if_due(&self) -> Result<(), StoreError> {
        let due = {
            let last = self
                .last_expire
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            match *last {
                Some(at) => at.elapsed() > Duration::from_secs(30),
                None => true,
            }
        };
        if !due {
            return Ok(());
        }
        self.expire().await?;
        let mut last = self
            .last_expire
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *last = Some(Instant::now());
        Ok(())
    }

    async fn expire(&self) -> Result<(), StoreError> {
        if let Some(pool) = self.pool.sqlite() {
            sqlx::query("DELETE FROM tokens WHERE validity < datetime('now', 'localtime')")
                .execute(pool)
                .await?;
        } else if let Some(pool) = self.pool.mysql() {
            sqlx::query("DELETE FROM tokens WHERE validity < NOW()")
                .execute(pool)
                .await?;
        } else if let Some(pool) = self.pool.postgres() {
            let now = Local::now()
                .naive_local()
                .format("%Y-%m-%d %H:%M:%S%.6f")
                .to_string();
            sqlx::query("DELETE FROM tokens WHERE validity < $1::timestamp")
                .bind(now)
                .execute(pool)
                .await?;
        } else {
            return Err(StoreError::Backend);
        }
        Ok(())
    }

    async fn get_sqlite(&self, principal: Uuid) -> Result<Option<AuthRow>, StoreError> {
        let Some(pool) = self.pool.sqlite() else {
            return Err(StoreError::Backend);
        };
        let sql = format!(
            "SELECT `passwordHash`, `passwordSalt`, `webLoginKey`, `accountType` FROM `{}` WHERE UUID = ?",
            self.realm
        );
        let row = sqlx::query(&sql)
            .bind(principal.to_string())
            .fetch_optional(pool)
            .await?;
        match row {
            Some(row) => Ok(Some(row_of(principal, &row)?)),
            None => Ok(None),
        }
    }

    async fn get_mysql(&self, principal: Uuid) -> Result<Option<AuthRow>, StoreError> {
        let Some(pool) = self.pool.mysql() else {
            return Err(StoreError::Backend);
        };
        let sql = format!(
            "SELECT `passwordHash`, `passwordSalt`, `webLoginKey`, `accountType` FROM `{}` WHERE UUID = ?",
            self.realm
        );
        let row = sqlx::query(&sql)
            .bind(principal.to_string())
            .fetch_optional(pool)
            .await?;
        match row {
            Some(row) => Ok(Some(row_of(principal, &row)?)),
            None => Ok(None),
        }
    }

    async fn get_postgres(&self, principal: Uuid) -> Result<Option<AuthRow>, StoreError> {
        let Some(pool) = self.pool.postgres() else {
            return Err(StoreError::Backend);
        };
        let sql = format!(
            "SELECT \"passwordHash\", \"passwordSalt\", \"webLoginKey\", \"accountType\" FROM {} WHERE uuid = $1",
            self.realm
        );
        let row = sqlx::query(&sql)
            .bind(principal)
            .fetch_optional(pool)
            .await?;
        match row {
            Some(row) => Ok(Some(row_of(principal, &row)?)),
            None => Ok(None),
        }
    }

    async fn update(&self, row: &AuthRow) -> Result<bool, StoreError> {
        let id = row.principal_id.to_string();
        let changed = if let Some(pool) = self.pool.sqlite() {
            let sql = format!(
                "UPDATE `{}` SET `passwordHash` = ?, `passwordSalt` = ?, `webLoginKey` = ?, `accountType` = ? WHERE UUID = ?",
                self.realm
            );
            sqlx::query(&sql)
                .bind(&row.password_hash)
                .bind(&row.password_salt)
                .bind(&row.web_login_key)
                .bind(&row.account_type)
                .bind(&id)
                .execute(pool)
                .await?
                .rows_affected()
        } else if let Some(pool) = self.pool.mysql() {
            let sql = format!(
                "UPDATE `{}` SET `passwordHash` = ?, `passwordSalt` = ?, `webLoginKey` = ?, `accountType` = ? WHERE UUID = ?",
                self.realm
            );
            sqlx::query(&sql)
                .bind(&row.password_hash)
                .bind(&row.password_salt)
                .bind(&row.web_login_key)
                .bind(&row.account_type)
                .bind(&id)
                .execute(pool)
                .await?
                .rows_affected()
        } else if let Some(pool) = self.pool.postgres() {
            let sql = format!(
                "UPDATE {} SET \"passwordHash\" = $1, \"passwordSalt\" = $2, \"webLoginKey\" = $3, \"accountType\" = $4 WHERE uuid = $5",
                self.realm
            );
            sqlx::query(&sql)
                .bind(&row.password_hash)
                .bind(&row.password_salt)
                .bind(&row.web_login_key)
                .bind(&row.account_type)
                .bind(row.principal_id)
                .execute(pool)
                .await?
                .rows_affected()
        } else {
            return Err(StoreError::Backend);
        };
        Ok(changed > 0)
    }

    async fn insert(&self, row: &AuthRow) -> Result<bool, StoreError> {
        let id = row.principal_id.to_string();
        let changed = if let Some(pool) = self.pool.sqlite() {
            let sql = format!(
                "INSERT INTO `{}` (`UUID`, `passwordHash`, `passwordSalt`, `webLoginKey`, `accountType`) VALUES (?, ?, ?, ?, ?)",
                self.realm
            );
            sqlx::query(&sql)
                .bind(&id)
                .bind(&row.password_hash)
                .bind(&row.password_salt)
                .bind(&row.web_login_key)
                .bind(&row.account_type)
                .execute(pool)
                .await?
                .rows_affected()
        } else if let Some(pool) = self.pool.mysql() {
            let sql = format!(
                "INSERT INTO `{}` (`UUID`, `passwordHash`, `passwordSalt`, `webLoginKey`, `accountType`) VALUES (?, ?, ?, ?, ?)",
                self.realm
            );
            sqlx::query(&sql)
                .bind(&id)
                .bind(&row.password_hash)
                .bind(&row.password_salt)
                .bind(&row.web_login_key)
                .bind(&row.account_type)
                .execute(pool)
                .await?
                .rows_affected()
        } else if let Some(pool) = self.pool.postgres() {
            let sql = format!(
                "INSERT INTO {} (uuid, \"passwordHash\", \"passwordSalt\", \"webLoginKey\", \"accountType\") VALUES ($1, $2, $3, $4, $5)",
                self.realm
            );
            sqlx::query(&sql)
                .bind(row.principal_id)
                .bind(&row.password_hash)
                .bind(&row.password_salt)
                .bind(&row.web_login_key)
                .bind(&row.account_type)
                .execute(pool)
                .await?
                .rows_affected()
        } else {
            return Err(StoreError::Backend);
        };
        Ok(changed > 0)
    }

    #[allow(dead_code)]
    async fn sqlite_has_column(&self, column: &str) -> Result<bool, StoreError> {
        let Some(pool) = self.pool.sqlite() else {
            return Err(StoreError::Backend);
        };
        let found = sqlx::query("SELECT 1 FROM pragma_table_info(?) WHERE name = ? COLLATE NOCASE")
            .bind(&self.realm)
            .bind(column)
            .fetch_optional(pool)
            .await?;
        Ok(found.is_some())
    }
}

fn row_of<R>(principal: Uuid, row: &R) -> Result<AuthRow, StoreError>
where
    R: Row,
    usize: sqlx::ColumnIndex<R>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    let text = |index: usize| -> Result<String, StoreError> {
        row.try_get(index)
            .map_err(|error| StoreError::Row(error.to_string()))
    };
    Ok(AuthRow {
        principal_id: principal,
        password_hash: text(0)?,
        password_salt: text(1)?,
        web_login_key: text(2)?,
        account_type: text(3)?,
    })
}

fn lifetime_modifier(lifetime: i32) -> String {
    format!("+{lifetime} minutes")
}

fn pg_when(lifetime: i32) -> Result<String, StoreError> {
    let delta = TimeDelta::try_minutes(i64::from(lifetime))
        .ok_or_else(|| StoreError::Row("token lifetime is outside a timestamp".to_string()))?;
    let when = Local::now().naive_local() + delta;
    Ok(when.format("%Y-%m-%d %H:%M:%S%.6f").to_string())
}

#[allow(dead_code)]
fn canonical_column(name: &str) -> Option<&'static str> {
    if !is_identifier(name) {
        return None;
    }
    match name.to_ascii_lowercase().as_str() {
        "passwordhash" => Some("passwordHash"),
        "passwordsalt" => Some("passwordSalt"),
        "webloginkey" => Some("webLoginKey"),
        "accounttype" => Some("accountType"),
        _ => None,
    }
}

pub(crate) fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.is_empty()
}
