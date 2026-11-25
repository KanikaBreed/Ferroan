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
//   Source/OpenSim.Data.MySQL/MySQLFSAssetData.cs
//   Source/OpenSim.Data.MySQL/Resources/FSAssetStore.migrations
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
//   Delete and ExecuteNonQuery checked at dev-build @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `MySQLFSAssetData` at the pin.
//!
//! The table name is `Realm`. `Get` selects the row and then
//! `UpdateAccessTime`. `Store` inserts when the id is absent. The name and
//! description keep only what a `utf8` column holds, and a failed insert is
//! returned (TB22, #481). The pin drops those characters and returns the
//! insert's result. An existing id is not rewritten unless `force` is set,
//! which updates `hash` and `access_time` only (TB45). `Delete` returns true
//! when the statement runs, and false for a database error. `AssetsExist`
//! and `Count` read this table. The hash query reads `id`, `hash` and `type`
//! without updating `access_time`. When the connection cannot be acquired, including access
//! denied and an unknown database, that query answers a null hash for each
//! id. A statement error after the connection is open is returned. `Import`
//! reads one source row at a time by the last id.

use std::collections::HashMap;

use sqlx::MySqlPool;
use sqlx::Row;
use uuid::Uuid;

use super::decode::{ReadColumn, column_bytes, column_i32, column_string, column_uuid};
use super::fsasset::{
    FsAsset, FsClock, FsHash, FsImportRow, FsMetadata, access_is_due, import_keyset_sql,
    import_row_within_bound, import_seek_sql, utf8mb3,
};
use super::model::KNOWN_FLAGS;
use super::store::StoreError;
use super::time::mysql_unix_now;

pub(super) async fn get(
    pool: &MySqlPool,
    table: &str,
    days: i32,
    id: Uuid,
) -> Result<Option<FsAsset>, StoreError> {
    let sql = format!(
        "SELECT id, name, description, type, hash, create_time, asset_flags, access_time \
         FROM {table} WHERE id = ?"
    );
    let row = sqlx::query(&sql)
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let asset = row_asset(&row, id)?;
    let access_time = column_i32(&row, "access_time")?;
    if access_is_due(mysql_unix_now(), access_time, days) {
        let sql = format!("UPDATE {table} SET `access_time` = UNIX_TIMESTAMP() WHERE `id` = ?");
        if let Err(error) = sqlx::query(&sql).bind(id.to_string()).execute(pool).await {
            tracing::error!(
                asset_id = %id,
                error = %error,
                "fsassets mysql access-time update failed"
            );
        }
    }
    Ok(Some(asset))
}

pub(super) async fn store(
    pool: &MySqlPool,
    table: &str,
    days: i32,
    meta: &FsMetadata,
    hash: &str,
    force: bool,
) -> Result<bool, StoreError> {
    if get(pool, table, days, meta.id).await?.is_some() {
        if force {
            let sql =
                format!("UPDATE {table} SET hash = ?, access_time = UNIX_TIMESTAMP() WHERE id = ?");
            if let Err(error) = sqlx::query(&sql)
                .bind(hash)
                .bind(meta.id.to_string())
                .execute(pool)
                .await
            {
                tracing::error!(
                    asset_id = %meta.id,
                    error = %error,
                    "fsassets mysql hash update failed"
                );
                return Err(error.into());
            }
        }
        return Ok(true);
    }
    let name = utf8mb3(&meta.name);
    let description = utf8mb3(&meta.description);
    let sql = format!(
        "INSERT INTO {table} (id, name, description, type, hash, asset_flags, create_time, \
         access_time) VALUES (?, ?, ?, ?, ?, ?, UNIX_TIMESTAMP(), UNIX_TIMESTAMP())"
    );
    if let Err(error) = sqlx::query(&sql)
        .bind(meta.id.to_string())
        .bind(name)
        .bind(description)
        .bind(i32::from(meta.type_code))
        .bind(hash)
        .bind(meta.flags)
        .execute(pool)
        .await
    {
        tracing::error!(
            asset_id = %meta.id,
            error = %error,
            "fsassets mysql insert failed"
        );
        return Err(error.into());
    }
    Ok(true)
}

pub(super) async fn exists(
    pool: &MySqlPool,
    table: &str,
    ids: &[Uuid],
) -> Result<Vec<bool>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let marks = "?,".repeat(ids.len());
    let marks = marks.trim_end_matches(',');
    let sql = format!("SELECT id FROM {table} WHERE id IN ({marks})");
    let mut query = sqlx::query(&sql);
    for id in ids {
        query = query.bind(id.to_string());
    }
    let rows = query.fetch_all(pool).await?;
    let mut found = Vec::with_capacity(rows.len());
    for row in &rows {
        found.push(column_uuid(row, "id")?);
    }
    Ok(ids.iter().map(|id| found.contains(id)).collect())
}

/// One hash query. A missing id, or a null hash, is `None`. This does not
/// update `access_time`. When the connection cannot be acquired, every id is
/// `None`. That is the open, including access denied and an unknown database.
/// A statement error after the connection is open is returned.
pub(super) async fn hashes(
    pool: &MySqlPool,
    table: &str,
    ids: &[Uuid],
) -> Result<Vec<Option<FsHash>>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let marks = "?,".repeat(ids.len());
    let marks = marks.trim_end_matches(',');
    let sql = format!("SELECT id, hash, type FROM {table} WHERE id IN ({marks})");
    let mut query = sqlx::query(&sql);
    for id in ids {
        query = query.bind(id.to_string());
    }
    let mut connection = match pool.acquire().await {
        Ok(connection) => connection,
        Err(error) if mysql_driver_or_pool_error(&error) => {
            tracing::error!("fsassets mysql hashes: the database did not open");
            return Ok(vec![None; ids.len()]);
        }
        Err(error) => return Err(error.into()),
    };
    let rows = match query.fetch_all(&mut *connection).await {
        Ok(rows) => rows,
        Err(error) => return Err(error.into()),
    };
    let mut by_id = HashMap::with_capacity(rows.len());
    for row in &rows {
        let id = column_uuid(row, "id")?;
        match optional_text(row, "hash")? {
            Some(hash) => {
                by_id.insert(
                    id,
                    FsHash {
                        hash,
                        type_code: column_i32(row, "type")? as i8,
                    },
                );
            }
            None => {
                by_id.remove(&id);
            }
        }
    }
    Ok(ids.iter().map(|id| by_id.get(id).cloned()).collect())
}

pub(super) async fn count(pool: &MySqlPool, table: &str) -> Result<i64, StoreError> {
    let sql = format!("SELECT COUNT(*) AS n FROM {table}");
    let row = sqlx::query(&sql).fetch_one(pool).await?;
    row.i64_col("n")
}

/// True when the statement runs, including when no row matched. A database
/// error, including a missing table and a refused login, is false. A pool
/// acquisition or execution failure from the MySQL driver is false as well.
pub(super) async fn delete(pool: &MySqlPool, table: &str, id: &str) -> Result<bool, StoreError> {
    let sql = format!("DELETE FROM {table} WHERE id = ?");
    match sqlx::query(&sql).bind(id).execute(pool).await {
        Ok(_) => Ok(true),
        Err(error) if mysql_driver_or_pool_error(&error) => {
            tracing::error!("fsassets mysql delete failed");
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

/// The stored id text at `offset` in id order. The selected column is the id
/// only, and the text is not rewritten into canonical form.
pub(super) async fn import_seek(
    pool: &MySqlPool,
    table: &str,
    offset: i32,
) -> Result<Option<String>, StoreError> {
    let sql = import_seek_sql(table, false, offset)?;
    let row = sqlx::query(&sql).fetch_optional(pool).await?;
    match row {
        Some(row) => Ok(Some(mysql_id_text(&row)?)),
        None => Ok(None),
    }
}

/// The next legacy rows after `after`, in id order. `after` is the stored id
/// text from the previous row. A `data` value past the import bound is
/// refused and is not returned. Each row keeps that same stored text.
pub(super) async fn import_keyset(
    pool: &MySqlPool,
    table: &str,
    after: Option<&str>,
    limit: i32,
) -> Result<Vec<(FsImportRow, String)>, StoreError> {
    let sql = import_keyset_sql(table, false, after.is_some(), limit)?;
    let mut query = sqlx::query(&sql);
    if let Some(id) = after {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let (parsed, stored) = mysql_import_row(row)?;
        import_row_within_bound(parsed.data.len())?;
        out.push((parsed, stored));
    }
    Ok(out)
}

/// The stored id characters. A binary collation comes back as bytes. The
/// text is not rewritten into canonical form.
fn mysql_id_text(row: &sqlx::mysql::MySqlRow) -> Result<String, StoreError> {
    if let Ok(value) = row.try_get::<String, _>("id") {
        return Ok(value);
    }
    if let Ok(value) = row.try_get::<Vec<u8>, _>("id") {
        return String::from_utf8(value).map_err(|_| StoreError::Row("id is not text".into()));
    }
    Err(StoreError::Row("id is not text".into()))
}

fn mysql_import_row(row: &sqlx::mysql::MySqlRow) -> Result<(FsImportRow, String), StoreError> {
    let stored = mysql_id_text(row)?;
    let id = Uuid::parse_str(stored.trim()).map_err(|_| StoreError::Row(stored.clone()))?;
    Ok((
        FsImportRow {
            id,
            name: column_string(row, "name")?,
            description: column_string(row, "description")?,
            type_code: column_i32(row, "assetType")? as i8,
            create_time: column_i32(row, "create_time")?,
            data: column_bytes(row, "data")?,
        },
        stored,
    ))
}

pub(super) async fn clock(
    pool: &MySqlPool,
    table: &str,
    id: Uuid,
) -> Result<Option<FsClock>, StoreError> {
    let sql = format!(
        "SELECT name, description, hash, create_time, access_time, asset_flags \
         FROM {table} WHERE id = ?"
    );
    let row = sqlx::query(&sql)
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(FsClock {
        hash: column_string(&row, "hash")?,
        create_time: column_i32(&row, "create_time")?,
        access_time: column_i32(&row, "access_time")?,
        raw_flags: column_i32(&row, "asset_flags")?,
        name: column_string(&row, "name")?,
        description: column_string(&row, "description")?,
    }))
}

/// MySQL driver and pool failures. The hash lookup checks acquisition errors
/// here. Delete also checks errors returned while executing its statement.
fn mysql_driver_or_pool_error(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::PoolClosed
            | sqlx::Error::PoolTimedOut
            | sqlx::Error::WorkerCrashed
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::Configuration(_)
            | sqlx::Error::Protocol(_)
            | sqlx::Error::Database(_)
    )
}

fn optional_text(row: &sqlx::mysql::MySqlRow, name: &str) -> Result<Option<String>, StoreError> {
    if let Ok(value) = row.try_get::<Option<String>, _>(name) {
        return Ok(value);
    }
    if let Ok(value) = row.try_get::<String, _>(name) {
        return Ok(Some(value));
    }
    Err(StoreError::Row(format!("{name} is not text")))
}

fn row_asset(row: &sqlx::mysql::MySqlRow, id: Uuid) -> Result<FsAsset, StoreError> {
    Ok(FsAsset {
        metadata: FsMetadata {
            id,
            name: column_string(row, "name")?,
            description: column_string(row, "description")?,
            type_code: column_i32(row, "type")? as i8,
            flags: column_i32(row, "asset_flags")? & KNOWN_FLAGS,
        },
        hash: column_string(row, "hash")?,
        create_time: column_i32(row, "create_time")?,
    })
}
