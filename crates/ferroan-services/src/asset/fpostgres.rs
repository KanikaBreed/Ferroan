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
//   Source/OpenSim.Data.PGSQL/PGSQLFSAssetData.cs
//   Source/OpenSim.Data.PGSQL/Resources/FSAssetStore.migrations
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `PGSQLFSAssetData` at the pin.
//!
//! The table is `fsassets`. `Initialise` ignores `Realm`. There are no name
//! or description columns; `Get` returns empty strings. The stored clock is
//! `(DateTime.Now.Ticks - epoch) / 10000000`. `Store` inserts when the id is
//! absent. An existing id updates `access_time` only and answers success
//! (TB35, #482). The pin does the same. `Delete` binds `uuid` and returns
//! true when the statement runs.
//! An id that is not a uuid deletes nothing and returns false (TB39). The pin
//! throws from `new UUID(id)` before the query. `Import` is
//! `ORDER BY id LIMIT count OFFSET start` (TB40). `force` updates `hash` and
//! `access_time` on an existing row (TB45). An ordinary store does not.

use sqlx::PgPool;
use uuid::Uuid;

use super::decode::{column_bytes, column_i32, column_string, column_uuid};
use super::fsasset::{
    FsAsset, FsClock, FsImportRow, FsMetadata, access_is_due, import_suffix, sql_ident,
};
use super::model::KNOWN_FLAGS;
use super::store::StoreError;
use super::time::{mysql_unix_now, postgres_unix_now};

pub(super) async fn get(pool: &PgPool, days: i32, id: Uuid) -> Result<Option<FsAsset>, StoreError> {
    let row = sqlx::query(
        "SELECT \"type\", \"hash\", \"create_time\", \"access_time\", \"asset_flags\" \
         FROM fsassets WHERE \"id\" = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let asset = FsAsset {
        metadata: FsMetadata {
            id,
            name: String::new(),
            description: String::new(),
            type_code: column_i32(&row, "type")? as i8,
            flags: column_i32(&row, "asset_flags")? & KNOWN_FLAGS,
        },
        hash: column_string(&row, "hash")?,
        create_time: column_i32(&row, "create_time")?,
    };
    let access_time = column_i32(&row, "access_time")?;
    if access_is_due(mysql_unix_now(), access_time, days)
        && let Err(error) =
            sqlx::query("UPDATE fsassets SET \"access_time\" = $2 WHERE \"id\" = $1::uuid")
                .bind(id.to_string())
                .bind(postgres_unix_now())
                .execute(pool)
                .await
    {
        tracing::error!(
            asset_id = %id,
            error = %error,
            "fsassets postgres access-time update failed"
        );
    }
    Ok(Some(asset))
}

pub(super) async fn store(
    pool: &PgPool,
    days: i32,
    meta: &FsMetadata,
    hash: &str,
    force: bool,
) -> Result<bool, StoreError> {
    let now = postgres_unix_now();
    if get(pool, days, meta.id).await?.is_some() {
        if force {
            sqlx::query(
                "UPDATE fsassets SET \"hash\" = $2, \"access_time\" = $3 WHERE \"id\" = $1::uuid",
            )
            .bind(meta.id.to_string())
            .bind(hash)
            .bind(now)
            .execute(pool)
            .await?;
        } else {
            sqlx::query("UPDATE fsassets SET \"access_time\" = $2 WHERE \"id\" = $1::uuid")
                .bind(meta.id.to_string())
                .bind(now)
                .execute(pool)
                .await?;
        }
        return Ok(true);
    }
    sqlx::query(
        "INSERT INTO fsassets (\"id\", \"type\", \"hash\", \"asset_flags\", \"create_time\", \
         \"access_time\") VALUES ($1::uuid, $2, $3, $4, $5, $6)",
    )
    .bind(meta.id.to_string())
    .bind(i32::from(meta.type_code))
    .bind(hash)
    .bind(meta.flags)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(true)
}

pub(super) async fn exists(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<bool>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut marks = String::new();
    for index in 0..ids.len() {
        if index > 0 {
            marks.push(',');
        }
        marks.push('$');
        marks.push_str(&index.saturating_add(1).to_string());
        marks.push_str("::uuid");
    }
    let sql = format!("SELECT \"id\" FROM fsassets WHERE id IN ({marks})");
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

pub(super) async fn count(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM fsassets")
        .fetch_one(pool)
        .await?;
    super::decode::ReadColumn::i64_col(&row, "n")
}

pub(super) async fn delete(pool: &PgPool, id: &str) -> Result<bool, StoreError> {
    let Ok(uuid) = Uuid::parse_str(id.trim()) else {
        return Ok(false);
    };
    sqlx::query("DELETE FROM fsassets WHERE \"id\" = $1::uuid")
        .bind(uuid.to_string())
        .execute(pool)
        .await?;
    Ok(true)
}

pub(super) async fn import_reader(
    pool: &PgPool,
    table: &str,
    start: i32,
    count: i32,
) -> Result<Vec<FsImportRow>, StoreError> {
    sql_ident(table)?;
    let suffix = import_suffix(start, count, true)?;
    let sql = format!("SELECT * FROM {table}{suffix}");
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(FsImportRow {
            id: column_uuid(row, "id")?,
            name: String::new(),
            description: String::new(),
            type_code: column_i32(row, "assetType")? as i8,
            create_time: column_i32(row, "create_time")?,
            data: column_bytes(row, "data")?,
        });
    }
    Ok(out)
}

pub(super) async fn clock(pool: &PgPool, id: Uuid) -> Result<Option<FsClock>, StoreError> {
    let row = sqlx::query(
        "SELECT \"hash\", \"create_time\", \"access_time\", \"asset_flags\" \
         FROM fsassets WHERE \"id\" = $1::uuid",
    )
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
        name: String::new(),
        description: String::new(),
    }))
}
