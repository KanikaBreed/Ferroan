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
//   Source/OpenSim.Data.SQLite/SQLiteGridUserData.cs
//   Source/OpenSim.Data.MySQL/MySQLGridUserData.cs
//   Source/OpenSim.Data.PGSQL/PGSQLGridUserData.cs
//   Source/OpenSim.Data.SQLite/SQLiteGenericTableHandler.cs
//   Source/OpenSim.Data.MySQL/MySQLGenericTableHandler.cs
//   Source/OpenSim.Data.PGSQL/PGSQLGenericTableHandler.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `IGridUserData` for SQLite, MySQL and PostgreSQL.
//!
//! The store does not run `GridUserStore` migrations. SQLite and MySQL
//! `Store` are `REPLACE`. PostgreSQL `Store` updates the named columns and
//! inserts when that update matches nothing. `GetAll` matches `UserID` as a
//! literal prefix: the pattern is a parameter, `%`, `_` and `!` are escaped,
//! and the statement has `ESCAPE '!'` (TB43). `GetMany` asks once per 500
//! distinct ids and leaves out a user with no row. `GetOnlineInRegion`
//! requires both `LastRegionID` and `Online`, and `Online` is `True`.
//! PostgreSQL `character(n)` reads are padded to the column width, matching
//! Npgsql. MySQL reads drop trailing spaces, matching MySqlConnector.
//! SQLite does not pad.

use std::collections::{BTreeMap, HashSet};

use ferroan_db::{Backend, Pool};
use sqlx::Row;

use super::model::{
    BATCH_SIZE, DATA_COLUMNS, DEFAULT_REALM, GridUser, ONLINE_TRUE, like_prefix, postgres_text,
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("grid user table name is not an identifier")]
    Realm,
    #[error("grid user column is not in the schema: {0}")]
    Column(String),
    #[error("this grid user store is not {0}")]
    Backend(&'static str),
    #[error("grid user database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("grid user row is not readable: {0}")]
    Row(String),
}

pub struct GridUserStore {
    pool: Pool,
    realm: String,
}

impl GridUserStore {
    /// Open realm `GridUser`. This does not migrate the database.
    pub fn open(pool: Pool) -> Result<Self, StoreError> {
        Self::new(pool, DEFAULT_REALM)
    }

    /// `Realm` from `[GridUserService]`. Anything that is not one identifier
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

    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.pool.occupancy()
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    /// The row for this user id, or none when the id is absent.
    pub async fn get(&self, user_id: &str) -> Result<Option<GridUser>, StoreError> {
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = {}",
            select_list(self.backend()),
            qtable(self.backend(), &self.realm),
            qcol(self.backend(), "UserID"),
            marker(self.backend(), 1)
        );
        match self.backend() {
            Backend::Sqlite => {
                let row = sqlx::query(&sql)
                    .bind(user_id)
                    .fetch_optional(self.sqlite()?)
                    .await?;
                row.as_ref()
                    .map(|row| user_from_row(row, Backend::Sqlite))
                    .transpose()
            }
            Backend::Mysql => {
                let row = sqlx::query(&sql)
                    .bind(user_id)
                    .fetch_optional(self.mysql()?)
                    .await?;
                row.as_ref()
                    .map(|row| user_from_row(row, Backend::Mysql))
                    .transpose()
            }
            Backend::Postgres => {
                let row = sqlx::query(&sql)
                    .bind(user_id)
                    .fetch_optional(self.postgres()?)
                    .await?;
                row.as_ref()
                    .map(|row| user_from_row(row, Backend::Postgres))
                    .transpose()
            }
        }
    }

    /// Rows whose user id starts with `prefix`.
    ///
    /// The match is literal. `%`, `_` and `!` in the id match only themselves.
    /// An empty prefix matches every row. There is no `ORDER BY`. SQLite
    /// `LIKE` and MySQL `latin1_swedish_ci` are case-insensitive for ASCII.
    /// PostgreSQL `LIKE` is case-sensitive.
    pub async fn get_all(&self, prefix: &str) -> Result<Vec<GridUser>, StoreError> {
        let sql = format!(
            "SELECT {} FROM {} WHERE {} LIKE {} ESCAPE '{}'",
            select_list(self.backend()),
            qtable(self.backend(), &self.realm),
            qcol(self.backend(), "UserID"),
            marker(self.backend(), 1),
            super::model::LIKE_ESCAPE
        );
        let pattern = like_prefix(prefix);
        self.fetch_bound(&sql, &[&pattern]).await
    }

    /// The rows for these ids, in any order. Duplicates are asked once.
    /// A user with no row is left out. Each statement carries at most 500 ids.
    pub async fn get_many(&self, user_ids: &[String]) -> Result<Vec<GridUser>, StoreError> {
        let unique = distinct_ids(user_ids);
        if unique.is_empty() {
            return Ok(Vec::new());
        }
        let mut found = Vec::new();
        for batch in unique.chunks(BATCH_SIZE) {
            found.extend(self.get_in(batch).await?);
        }
        Ok(found)
    }

    /// Users marked `Online` = `True` whose `LastRegionID` is `region_id`.
    ///
    /// On PostgreSQL the region id is bound as `uuid`. A value that is not a
    /// uuid fails the statement.
    pub async fn get_online_in_region(&self, region_id: &str) -> Result<Vec<GridUser>, StoreError> {
        let region_marker = match self.backend() {
            Backend::Postgres => "$1::uuid".to_string(),
            _ => marker(self.backend(), 1),
        };
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = {} AND {} = {}",
            select_list(self.backend()),
            qtable(self.backend(), &self.realm),
            qcol(self.backend(), "LastRegionID"),
            region_marker,
            qcol(self.backend(), "Online"),
            marker(self.backend(), 2)
        );
        self.fetch_bound(&sql, &[region_id, ONLINE_TRUE]).await
    }

    /// Write the key and the data columns that are present.
    ///
    /// An unknown column name is refused. SQLite and MySQL replace the row, so
    /// an omitted column returns to its default. PostgreSQL updates only the
    /// named columns, and inserts when the user id is new.
    pub async fn store(&self, user: &GridUser) -> Result<bool, StoreError> {
        let present = present_columns(user)?;
        match self.backend() {
            Backend::Sqlite => {
                let (sql, values) = replace_sql(self.backend(), &self.realm, user, &present);
                let refs = value_refs(&values);
                let done = bind_all(sqlx::query(&sql), &refs)
                    .execute(self.sqlite()?)
                    .await?;
                Ok(done.rows_affected() > 0)
            }
            Backend::Mysql => {
                let (sql, values) = replace_sql(self.backend(), &self.realm, user, &present);
                let refs = value_refs(&values);
                let done = bind_all(sqlx::query(&sql), &refs)
                    .execute(self.mysql()?)
                    .await?;
                Ok(done.rows_affected() > 0)
            }
            Backend::Postgres => self.store_postgres(user, &present).await,
        }
    }

    /// Delete every grid-user row. Tests use this so a shared database starts empty.
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

    async fn get_in(&self, ids: &[String]) -> Result<Vec<GridUser>, StoreError> {
        let mut marks = String::new();
        for index in 0..ids.len() {
            if index > 0 {
                marks.push(',');
            }
            marks.push_str(&marker(self.backend(), index.saturating_add(1)));
        }
        let sql = format!(
            "SELECT {} FROM {} WHERE {} IN ({marks})",
            select_list(self.backend()),
            qtable(self.backend(), &self.realm),
            qcol(self.backend(), "UserID")
        );
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        self.fetch_bound(&sql, &refs).await
    }

    async fn fetch_bound(&self, sql: &str, values: &[&str]) -> Result<Vec<GridUser>, StoreError> {
        match self.backend() {
            Backend::Sqlite => {
                let rows = bind_all(sqlx::query(sql), values)
                    .fetch_all(self.sqlite()?)
                    .await?;
                rows.iter()
                    .map(|row| user_from_row(row, Backend::Sqlite))
                    .collect()
            }
            Backend::Mysql => {
                let rows = bind_all(sqlx::query(sql), values)
                    .fetch_all(self.mysql()?)
                    .await?;
                rows.iter()
                    .map(|row| user_from_row(row, Backend::Mysql))
                    .collect()
            }
            Backend::Postgres => {
                let rows = bind_all(sqlx::query(sql), values)
                    .fetch_all(self.postgres()?)
                    .await?;
                rows.iter()
                    .map(|row| user_from_row(row, Backend::Postgres))
                    .collect()
            }
        }
    }

    async fn store_postgres(
        &self,
        user: &GridUser,
        present: &[(String, String)],
    ) -> Result<bool, StoreError> {
        let pool = self.postgres()?;
        let table = qtable(self.backend(), &self.realm);
        let mut sets = vec!["\"UserID\" = $1".to_string()];
        let mut index = 2;
        for (name, _) in present {
            sets.push(pg_assign(name, index));
            index = index.saturating_add(1);
        }
        let update = format!(
            "UPDATE {table} SET {} WHERE \"UserID\" = $1",
            sets.join(", ")
        );
        let values = bound_values(user, present);
        let refs = value_refs(&values);
        let updated = bind_all(sqlx::query(&update), &refs).execute(pool).await?;
        if updated.rows_affected() > 0 {
            return Ok(true);
        }
        let mut cols = vec!["\"UserID\"".to_string()];
        let mut vals = vec!["$1".to_string()];
        let mut index = 2;
        for (name, _) in present {
            cols.push(qcol(Backend::Postgres, name));
            vals.push(pg_value(name, index));
            index = index.saturating_add(1);
        }
        let insert = format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            cols.join(", "),
            vals.join(", ")
        );
        let refs = value_refs(&values);
        let inserted = bind_all(sqlx::query(&insert), &refs).execute(pool).await?;
        Ok(inserted.rows_affected() > 0)
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

fn present_columns(user: &GridUser) -> Result<Vec<(String, String)>, StoreError> {
    for key in user.data.keys() {
        if !DATA_COLUMNS.contains(&key.as_str()) {
            return Err(StoreError::Column(key.clone()));
        }
    }
    let mut present = Vec::new();
    for name in DATA_COLUMNS {
        if let Some(value) = user.data.get(name) {
            present.push((name.to_string(), value.clone()));
        }
    }
    Ok(present)
}

fn bound_values(user: &GridUser, present: &[(String, String)]) -> Vec<String> {
    let mut values = Vec::with_capacity(present.len().saturating_add(1));
    values.push(user.user_id.clone());
    for (_, value) in present {
        values.push(value.clone());
    }
    values
}

fn value_refs(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

fn replace_sql(
    backend: Backend,
    realm: &str,
    user: &GridUser,
    present: &[(String, String)],
) -> (String, Vec<String>) {
    let mut cols = vec![qcol(backend, "UserID")];
    let mut marks = vec![marker(backend, 1)];
    for (index, (name, _)) in present.iter().enumerate() {
        cols.push(qcol(backend, name));
        marks.push(marker(backend, index.saturating_add(2)));
    }
    let sql = format!(
        "REPLACE INTO {} ({}) VALUES ({})",
        qtable(backend, realm),
        cols.join(", "),
        marks.join(", ")
    );
    (sql, bound_values(user, present))
}

fn pg_assign(name: &str, index: usize) -> String {
    format!(
        "{} = {}",
        qcol(Backend::Postgres, name),
        pg_value(name, index)
    )
}

fn pg_value(name: &str, index: usize) -> String {
    if super::model::is_region(name) {
        format!("${index}::uuid")
    } else {
        format!("${index}")
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

fn user_from_row<'a, R>(row: &'a R, backend: Backend) -> Result<GridUser, StoreError>
where
    R: Row,
    &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Decode<'a, <R as Row>::Database> + sqlx::Type<<R as Row>::Database>,
{
    let user_id = cell(row, "UserID", false)?;
    let trim = backend == Backend::Mysql;
    let mut data = BTreeMap::new();
    for name in DATA_COLUMNS {
        let mut text = cell(row, name, trim)?;
        if backend == Backend::Postgres {
            text = postgres_text(name, &text);
        }
        data.insert(name.to_string(), text);
    }
    Ok(GridUser { user_id, data })
}

fn cell<'a, R>(row: &'a R, name: &'a str, trim: bool) -> Result<String, StoreError>
where
    R: Row,
    &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Decode<'a, <R as Row>::Database> + sqlx::Type<<R as Row>::Database>,
{
    let value: Option<String> = row
        .try_get(name)
        .map_err(|error| StoreError::Row(error.to_string()))?;
    let mut text = value.unwrap_or_default();
    if trim {
        let end = text.trim_end_matches(' ').len();
        text.truncate(end);
    }
    Ok(text)
}

fn select_list(backend: Backend) -> String {
    let mut cols = vec![match backend {
        Backend::Postgres => "\"UserID\"".to_string(),
        _ => qcol(backend, "UserID"),
    }];
    for name in DATA_COLUMNS {
        cols.push(match backend {
            Backend::Postgres => format!("{}::text AS \"{name}\"", qcol(backend, name)),
            _ => qcol(backend, name),
        });
    }
    cols.join(", ")
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

fn distinct_ids(ids: &[String]) -> Vec<String> {
    let mut seen = HashSet::<&str>::new();
    let mut unique = Vec::new();
    for id in ids {
        if seen.insert(id.as_str()) {
            unique.push(id.clone());
        }
    }
    unique
}

/// One SQL identifier: a letter or underscore, then letters, digits or
/// underscores, at most 64 characters.
pub(crate) fn sql_ident(name: &str) -> Result<(), StoreError> {
    crate::sql_ident(name)
        .then_some(())
        .ok_or(StoreError::Realm)
}
