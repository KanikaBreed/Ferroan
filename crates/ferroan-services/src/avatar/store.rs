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
//   Source/OpenSim.Data.SQLite/SQLiteAvatarData.cs
//   Source/OpenSim.Data.MySQL/MySQLAvatarData.cs
//   Source/OpenSim.Data.PGSQL/PGSQLAvatarData.cs
//   Source/OpenSim.Data.SQLite/SQLiteGenericTableHandler.cs
//   Source/OpenSim.Data.MySQL/MySQLGenericTableHandler.cs
//   Source/OpenSim.Data.PGSQL/PGSQLGenericTableHandler.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// MySQL write errors follow Source/OpenSim.Data.MySQL/MySQLFramework.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `IAvatarData` for SQLite, MySQL and PostgreSQL.
//!
//! The store does not run `Avatar` migrations. SQLite and MySQL upserts are
//! `REPLACE` of `(PrincipalID, Name, Value)`. PostgreSQL updates that pair
//! and inserts when the update matches nothing. `Get` has no `ORDER BY`.
//! PostgreSQL binds `PrincipalID` as `uuid`. MySQL reads of `PrincipalID`
//! drop trailing spaces, matching a `char(36)` column. `Name` and `Value`
//! are returned as stored.

use ferroan_db::{Backend, Pool};
use sqlx::Row;

use super::model::{AvatarRow, DEFAULT_REALM};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("avatar table name is not an identifier")]
    Realm,
    #[error("avatar rows exceed the service read limit")]
    Limit,
    #[error("this avatar store is not {0}")]
    Backend(&'static str),
    #[error("avatar database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("avatar row is not readable: {0}")]
    Row(String),
}

pub struct AvatarStore {
    pool: Pool,
    realm: String,
}

impl AvatarStore {
    /// Open realm `Avatars`. This does not migrate the database.
    pub fn open(pool: Pool) -> Result<Self, StoreError> {
        Self::new(pool, DEFAULT_REALM)
    }

    /// `Realm` from `[AvatarService]`. Anything that is not one identifier
    /// is refused, because the name is interpolated.
    pub fn new(pool: Pool, realm: &str) -> Result<Self, StoreError> {
        sql_ident(realm)?;
        Ok(Self {
            pool,
            realm: realm.to_string(),
        })
    }

    pub const fn backend(&self) -> Backend {
        self.pool.backend()
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    /// Every row for this principal, in whatever order the engine returns.
    ///
    /// An absent principal is an empty list. On PostgreSQL the id is bound
    /// as `uuid`, so a value that is not a uuid fails the statement.
    pub async fn get(&self, principal_id: &str) -> Result<Vec<AvatarRow>, StoreError> {
        let sql = select_sql(self.backend(), &self.realm);
        self.fetch(&sql, principal_id).await
    }

    pub(super) fn pool_occupancy(&self) -> (u32, u32) {
        self.pool.occupancy()
    }

    /// Read only bounded service data. The SQL projection keeps even one
    /// oversized database cell out of the driver. Cleanup needs names only.
    pub(super) async fn service_rows(
        &self,
        principal: &str,
        names_only: bool,
    ) -> Result<Vec<AvatarRow>, StoreError> {
        use ferroan_http::{MAX_FIELDS, MAX_QUERY_BYTES};
        use futures_util::TryStreamExt;
        let backend = self.backend();
        let name = qcol(backend, "Name");
        let value = if names_only {
            "''".to_owned()
        } else {
            qcol(backend, "Value")
        };
        let bytes = |col: &str| match backend {
            Backend::Sqlite => format!("length(CAST({col} AS BLOB))"),
            _ => format!("octet_length({col})"),
        };
        let length = format!(
            "coalesce({},0) + coalesce({},0)",
            bytes(&name),
            bytes(&value)
        );
        let sql = format!(
            "SELECT CASE WHEN {length} <= {MAX_QUERY_BYTES} THEN {name} END AS {name},
                    CASE WHEN {length} <= {MAX_QUERY_BYTES} THEN {value} END AS {},
                    CASE WHEN {length} > {MAX_QUERY_BYTES} THEN '1' ELSE '0' END AS oversize
             FROM {} WHERE {} LIMIT {}",
            qcol(backend, "Value"),
            qtable(backend, &self.realm),
            principal_eq(backend, 1),
            MAX_FIELDS + 1
        );
        let mut result = Vec::new();
        let mut total = 0usize;
        macro_rules! collect_rows {
            ($pool:expr) => {{
                let mut stream = sqlx::query(&sql).bind(principal).fetch($pool);
                while let Some(row) = stream.try_next().await? {
                    if result.len() == MAX_FIELDS {
                        ferroan_diag::refuse(
                            ferroan_diag::AVATAR_ROWS,
                            ferroan_diag::Kind::Hard,
                            MAX_FIELDS as u64,
                            (MAX_FIELDS + 1) as u64,
                            "avatar",
                        );
                        return Err(StoreError::Limit);
                    }
                    let key = cell(&row, "Name")?;
                    let val = cell(&row, "Value")?;
                    total += key.len() + val.len();
                    if cell(&row, "oversize")? == "1" || total > MAX_QUERY_BYTES {
                        ferroan_diag::refuse(
                            ferroan_diag::AVATAR_ROW_BYTES,
                            ferroan_diag::Kind::Hard,
                            MAX_QUERY_BYTES as u64,
                            total.max(MAX_QUERY_BYTES + 1) as u64,
                            "avatar",
                        );
                        return Err(StoreError::Limit);
                    }
                    result.push(AvatarRow {
                        principal_id: principal.to_owned(),
                        name: key,
                        value: val,
                    });
                }
            }};
        }
        match backend {
            Backend::Sqlite => collect_rows!(self.sqlite()?),
            Backend::Mysql => collect_rows!(self.mysql()?),
            Backend::Postgres => collect_rows!(self.postgres()?),
        }
        Ok(result)
    }

    /// Insert or replace the row for `(principal_id, name)`.
    ///
    /// The written value is the string passed in. This method does not parse
    /// height and does not delete other names.
    pub async fn upsert(
        &self,
        principal_id: &str,
        name: &str,
        value: &str,
    ) -> Result<bool, StoreError> {
        match self.backend() {
            Backend::Sqlite | Backend::Mysql => {
                let sql = format!(
                    "REPLACE INTO {} ({}, {}, {}) VALUES ({}, {}, {})",
                    qtable(self.backend(), &self.realm),
                    qcol(self.backend(), "PrincipalID"),
                    qcol(self.backend(), "Name"),
                    qcol(self.backend(), "Value"),
                    marker(self.backend(), 1),
                    marker(self.backend(), 2),
                    marker(self.backend(), 3)
                );
                self.affect(&sql, &[principal_id, name, value]).await
            }
            Backend::Postgres => self.upsert_postgres(principal_id, name, value).await,
        }
    }

    /// Delete every row for this principal.
    ///
    /// Returns whether the statement removed at least one row.
    pub async fn delete_principal(&self, principal_id: &str) -> Result<bool, StoreError> {
        let sql = format!(
            "DELETE FROM {} WHERE {}",
            qtable(self.backend(), &self.realm),
            principal_eq(self.backend(), 1)
        );
        self.affect(&sql, &[principal_id]).await
    }

    /// Delete the row for this principal and name.
    ///
    /// Returns whether the statement removed a row. Other names stay.
    pub async fn delete(&self, principal_id: &str, name: &str) -> Result<bool, StoreError> {
        let sql = format!(
            "DELETE FROM {} WHERE {} AND {}",
            qtable(self.backend(), &self.realm),
            principal_eq(self.backend(), 1),
            name_eq(self.backend(), 2)
        );
        self.affect(&sql, &[principal_id, name]).await
    }

    /// Delete every avatar row. Tests use this so a shared database starts empty.
    #[cfg(test)]
    pub(crate) async fn wipe(&self) -> Result<(), StoreError> {
        let sql = format!("DELETE FROM {}", qtable(self.backend(), &self.realm));
        match self.backend() {
            Backend::Sqlite => {
                sqlx::query(&sql).execute(self.sqlite()?).await?;
            }
            Backend::Mysql => {
                sqlx::query(&sql).execute(self.mysql()?).await?;
            }
            Backend::Postgres => {
                sqlx::query(&sql).execute(self.postgres()?).await?;
            }
        }
        Ok(())
    }

    async fn upsert_postgres(
        &self,
        principal_id: &str,
        name: &str,
        value: &str,
    ) -> Result<bool, StoreError> {
        let table = qtable(self.backend(), &self.realm);
        let update = format!(
            "UPDATE {table} SET \"PrincipalID\" = $1::uuid, \"Name\" = $2, \"Value\" = $3 WHERE \"PrincipalID\" = $1::uuid AND \"Name\" = $2"
        );
        let values = [principal_id, name, value];
        if self.affect(&update, &values).await? {
            return Ok(true);
        }
        let insert = format!(
            "INSERT INTO {table} (\"PrincipalID\", \"Name\", \"Value\") VALUES ($1::uuid, $2, $3)"
        );
        self.affect(&insert, &values).await
    }

    async fn fetch(&self, sql: &str, principal_id: &str) -> Result<Vec<AvatarRow>, StoreError> {
        let backend = self.backend();
        match backend {
            Backend::Sqlite => {
                let rows = sqlx::query(sql)
                    .bind(principal_id)
                    .fetch_all(self.sqlite()?)
                    .await?;
                rows.iter().map(|row| row_from(row, backend)).collect()
            }
            Backend::Mysql => {
                let rows = sqlx::query(sql)
                    .bind(principal_id)
                    .fetch_all(self.mysql()?)
                    .await?;
                rows.iter().map(|row| row_from(row, backend)).collect()
            }
            Backend::Postgres => {
                let rows = sqlx::query(sql)
                    .bind(principal_id)
                    .fetch_all(self.postgres()?)
                    .await?;
                rows.iter().map(|row| row_from(row, backend)).collect()
            }
        }
    }

    async fn affect(&self, sql: &str, values: &[&str]) -> Result<bool, StoreError> {
        let done = match self.backend() {
            Backend::Sqlite => bind_all(sqlx::query(sql), values)
                .execute(self.sqlite()?)
                .await?
                .rows_affected(),
            Backend::Mysql => {
                // Pin8 opens before ExecuteNonQueryWithConnection: only a
                // failure executing the write counts as zero affected rows.
                let mut connection = self.mysql()?.acquire().await?;
                bind_all(sqlx::query(sql), values)
                    .execute(&mut *connection)
                    .await
                    .map_or(0, |result| result.rows_affected())
            }
            Backend::Postgres => bind_all(sqlx::query(sql), values)
                .execute(self.postgres()?)
                .await?
                .rows_affected(),
        };
        Ok(done > 0)
    }

    fn sqlite(&self) -> Result<&sqlx::SqlitePool, StoreError> {
        self.pool.sqlite().ok_or(StoreError::Backend("sqlite"))
    }

    fn mysql(&self) -> Result<&sqlx::MySqlPool, StoreError> {
        self.pool.mysql().ok_or(StoreError::Backend("mysql"))
    }

    fn postgres(&self) -> Result<&sqlx::PgPool, StoreError> {
        self.pool.postgres().ok_or(StoreError::Backend("postgres"))
    }
}

fn select_sql(backend: Backend, realm: &str) -> String {
    let principal = match backend {
        Backend::Postgres => "\"PrincipalID\"::text AS \"PrincipalID\"".to_string(),
        _ => qcol(backend, "PrincipalID"),
    };
    format!(
        "SELECT {principal}, {}, {} FROM {} WHERE {}",
        qcol(backend, "Name"),
        qcol(backend, "Value"),
        qtable(backend, realm),
        principal_eq(backend, 1)
    )
}

fn principal_eq(backend: Backend, index: usize) -> String {
    match backend {
        Backend::Postgres => format!("\"PrincipalID\" = ${index}::uuid"),
        _ => format!(
            "{} = {}",
            qcol(backend, "PrincipalID"),
            marker(backend, index)
        ),
    }
}

fn name_eq(backend: Backend, index: usize) -> String {
    match backend {
        Backend::Postgres => format!("\"Name\" = ${index}"),
        _ => format!("{} = {}", qcol(backend, "Name"), marker(backend, index)),
    }
}

fn marker(backend: Backend, index: usize) -> String {
    match backend {
        Backend::Postgres => format!("${index}"),
        _ => "?".to_string(),
    }
}

fn qcol(backend: Backend, name: &str) -> String {
    match backend {
        Backend::Postgres => format!("\"{name}\""),
        _ => format!("`{name}`"),
    }
}

fn qtable(backend: Backend, realm: &str) -> String {
    match backend {
        Backend::Postgres => realm.to_string(),
        _ => format!("`{realm}`"),
    }
}

fn bind_all<'a, DB>(
    mut query: sqlx::query::Query<'a, DB, <DB as sqlx::Database>::Arguments<'a>>,
    values: &[&'a str],
) -> sqlx::query::Query<'a, DB, <DB as sqlx::Database>::Arguments<'a>>
where
    DB: sqlx::Database,
    &'a str: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
{
    for value in values {
        query = query.bind(*value);
    }
    query
}

fn row_from<'a, R>(row: &'a R, backend: Backend) -> Result<AvatarRow, StoreError>
where
    R: Row,
    &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Decode<'a, <R as Row>::Database> + sqlx::Type<<R as Row>::Database>,
{
    let mut principal_id = cell(row, "PrincipalID")?;
    if backend == Backend::Mysql {
        let end = principal_id.trim_end_matches(' ').len();
        principal_id.truncate(end);
    }
    Ok(AvatarRow {
        principal_id,
        name: cell(row, "Name")?,
        value: cell(row, "Value")?,
    })
}

fn cell<'a, R>(row: &'a R, name: &'a str) -> Result<String, StoreError>
where
    R: Row,
    &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Decode<'a, <R as Row>::Database> + sqlx::Type<<R as Row>::Database>,
{
    let value: Option<String> = row
        .try_get(name)
        .map_err(|error| StoreError::Row(error.to_string()))?;
    Ok(value.unwrap_or_default())
}

/// One SQL identifier: a letter or underscore, then letters, digits or
/// underscores, at most 64 characters.
pub(crate) fn sql_ident(name: &str) -> Result<(), StoreError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(StoreError::Realm);
    };
    if name.len() > 64 || (!first.is_ascii_alphabetic() && first != '_') {
        return Err(StoreError::Realm);
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(StoreError::Realm);
    }
    Ok(())
}
