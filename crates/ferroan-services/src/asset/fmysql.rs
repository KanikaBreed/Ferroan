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
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `MySQLFSAssetData` at the pin.
//!
//! The table name is `Realm`. `Get` selects the row and then
//! `UpdateAccessTime`. `Store` inserts when the id is absent. The name and
//! description keep only what a `utf8` column holds, and a failed insert is
//! returned (TB22, #481). The pin drops those characters and returns the
//! insert's result. An existing id is not rewritten. `Delete` returns true
//! when the statement runs. `AssetsExist` and `Count` read this table. `Import` is
//! `SELECT * FROM {table} LIMIT start, count`.

use sqlx::MySqlPool;
use uuid::Uuid;

use super::decode::{ReadColumn, column_bytes, column_i32, column_string, column_uuid};
use super::fsasset::{
    FsAsset, FsClock, FsImportRow, FsMetadata, access_is_due, import_suffix, sql_ident, utf8mb3,
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
) -> Result<bool, StoreError> {
    if get(pool, table, days, meta.id).await?.is_some() {
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

pub(super) async fn count(pool: &MySqlPool, table: &str) -> Result<i64, StoreError> {
    let sql = format!("SELECT COUNT(*) AS n FROM {table}");
    let row = sqlx::query(&sql).fetch_one(pool).await?;
    row.i64_col("n")
}

pub(super) async fn delete(pool: &MySqlPool, table: &str, id: &str) -> Result<bool, StoreError> {
    let sql = format!("DELETE FROM {table} WHERE id = ?");
    sqlx::query(&sql).bind(id).execute(pool).await?;
    Ok(true)
}

pub(super) async fn import_reader(
    pool: &MySqlPool,
    table: &str,
    start: i32,
    count: i32,
) -> Result<Vec<FsImportRow>, StoreError> {
    sql_ident(table)?;
    let suffix = import_suffix(start, count, false)?;
    let sql = format!("SELECT * FROM {table}{suffix}");
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(FsImportRow {
            id: column_uuid(row, "id")?,
            name: column_string(row, "name")?,
            description: column_string(row, "description")?,
            type_code: column_i32(row, "assetType")? as i8,
            create_time: column_i32(row, "create_time")?,
            data: column_bytes(row, "data")?,
        });
    }
    Ok(out)
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
