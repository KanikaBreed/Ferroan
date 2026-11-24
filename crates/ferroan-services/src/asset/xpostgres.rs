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
//   Source/OpenSim.Data.PGSQL/PGSQLXAssetData.cs
//   Source/OpenSim.Data.PGSQL/Resources/XAssetStore.migrations
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `PGSQLXAssetData` at the pin.
//!
//! `StoreAsset` inserts the meta row when the id is absent, then updates that
//! id, including `create_time` and `access_time`, to now. Data is inserted
//! with `ON CONFLICT (hash) DO NOTHING`, so a hash another store just wrote
//! does not roll this one back (TB58, D43). A failed metadata or data
//! statement rolls back and returns. A hash-lookup error is returned. The
//! clock is UTC unix time, the same helper MySQL uses, not the ordinary
//! PostgreSQL asset store's local clock.
//!
//! `FetchAssetMetadataSet` is `ORDER BY id LIMIT count OFFSET start` (TB29,
//! #479) and returns that page. `Delete` parses the id as a uuid and deletes
//! only the meta row (TB30, #480). The data row stays: rows are shared by
//! hash and there is no reference count.

use sqlx::PgPool;
use uuid::Uuid;

use super::decode::{
    column_bool, column_bytes, column_i8, column_i32, column_string, column_uuid, optional_i32,
};
use super::model::{Asset, AssetMetadata, KNOWN_FLAGS};
use super::store::{AssetClock, StoreError};
use super::time::mysql_unix_now;
use super::xasset::{access_time_is_due, content_hash};

pub(super) async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Asset>, StoreError> {
    let row = sqlx::query(
        "SELECT name, description, access_time, \"AssetType\", local, temporary, asset_flags, \
         creatorid, data FROM xassetsmeta \
         JOIN xassetsdata ON xassetsmeta.hash = xassetsdata.hash WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let access_time = column_i32(&row, "access_time")?;
    let asset = Asset {
        id,
        name: column_string(&row, "name")?,
        description: column_string(&row, "description")?,
        type_code: column_i8(&row, "AssetType")?,
        local: column_bool(&row, "local")?,
        temporary: column_bool(&row, "temporary")?,
        creator_id: column_string(&row, "creatorid")?,
        flags: column_i32(&row, "asset_flags")? & KNOWN_FLAGS,
        data: column_bytes(&row, "data")?,
        upload_attempts: 0,
    };
    let now = mysql_unix_now();
    if access_time_is_due(now, access_time) {
        let _ = update_access(pool, id, now).await;
    }
    Ok(Some(asset))
}

pub(super) async fn store(pool: &PgPool, asset: &Asset) -> Result<(), StoreError> {
    let hash = content_hash(&asset.data);
    let now = mysql_unix_now();
    let name = asset.stored_name();
    let description = asset.stored_description();
    let mut tx = pool.begin().await?;
    let meta = sqlx::query(
        "INSERT INTO xassetsmeta(id, hash, name, description, \"AssetType\", local, temporary, \
         create_time, access_time, asset_flags, creatorid) \
         SELECT $1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11 \
         WHERE NOT EXISTS (SELECT id FROM xassetsmeta WHERE id = $1::uuid)",
    )
    .bind(asset.id.to_string())
    .bind(hash.as_slice())
    .bind(&name)
    .bind(&description)
    .bind(i16::from(asset.type_code))
    .bind(asset.local)
    .bind(asset.temporary)
    .bind(now)
    .bind(now)
    .bind(asset.flags)
    .bind(&asset.creator_id)
    .execute(&mut *tx)
    .await;
    if let Err(error) = meta {
        tracing::error!(
            asset_id = %asset.id,
            error = %error,
            "xasset postgres metadata insert rolled back"
        );
        tx.rollback().await?;
        return Ok(());
    }
    let updated = sqlx::query(
        "UPDATE xassetsmeta SET hash = $2, name = $3, description = $4, \"AssetType\" = $5, \
         local = $6, temporary = $7, create_time = $8, access_time = $9, asset_flags = $10, \
         creatorid = $11 WHERE id = $1::uuid",
    )
    .bind(asset.id.to_string())
    .bind(hash.as_slice())
    .bind(&name)
    .bind(&description)
    .bind(i16::from(asset.type_code))
    .bind(asset.local)
    .bind(asset.temporary)
    .bind(now)
    .bind(now)
    .bind(asset.flags)
    .bind(&asset.creator_id)
    .execute(&mut *tx)
    .await;
    if let Err(error) = updated {
        tracing::error!(
            asset_id = %asset.id,
            error = %error,
            "xasset postgres metadata update rolled back"
        );
        tx.rollback().await?;
        return Ok(());
    }
    let present = match sqlx::query("SELECT hash FROM xassetsdata WHERE hash = $1")
        .bind(hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(row) => row.is_some(),
        Err(error) => {
            tracing::error!(
                asset_id = %asset.id,
                error = %error,
                "xasset postgres hash lookup failed"
            );
            tx.rollback().await?;
            return Err(error.into());
        }
    };
    if !present {
        let inserted = sqlx::query(
            "INSERT INTO xassetsdata(hash, data) VALUES ($1, $2) ON CONFLICT (hash) DO NOTHING",
        )
        .bind(hash.as_slice())
        .bind(&asset.data)
        .execute(&mut *tx)
        .await;
        if let Err(error) = inserted {
            tracing::error!(
                asset_id = %asset.id,
                error = %error,
                "xasset postgres data write rolled back"
            );
            tx.rollback().await?;
            return Ok(());
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Deletes only the meta row (TB30, #480), as MySQL's `Delete` does. The data
/// row stays: rows are shared by hash and there is no reference count, so
/// deleting an unused data row races a concurrent store that found the hash
/// and skipped its insert. An id that does not parse deletes nothing and
/// returns false. A missing id returns true, as the C# method returns true
/// after `ExecuteNonQuery`.
pub(super) async fn delete(pool: &PgPool, id: &str) -> Result<bool, StoreError> {
    let Ok(uuid) = Uuid::parse_str(id.trim()) else {
        return Ok(false);
    };
    sqlx::query("DELETE FROM xassetsmeta WHERE id = $1::uuid")
        .bind(uuid.to_string())
        .execute(pool)
        .await?;
    Ok(true)
}

pub(super) async fn exists(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<bool>, StoreError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row = sqlx::query("SELECT 1 FROM xassetsmeta WHERE id = $1::uuid")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;
        out.push(row.is_some());
    }
    Ok(out)
}

pub(super) async fn metadata_set(
    pool: &PgPool,
    start: i32,
    count: i32,
) -> Result<Vec<AssetMetadata>, StoreError> {
    let rows = sqlx::query(
        "SELECT name, description, access_time, \"AssetType\", temporary, id, asset_flags, \
         creatorid FROM xassetsmeta ORDER BY id LIMIT $1 OFFSET $2",
    )
    .bind(count)
    .bind(start)
    .fetch_all(pool)
    .await?;
    let mut pending = Vec::with_capacity(rows.len());
    for row in &rows {
        pending.push((
            AssetMetadata {
                id: column_uuid(row, "id")?,
                name: column_string(row, "name")?,
                description: column_string(row, "description")?,
                type_code: column_i8(row, "AssetType")?,
                temporary: column_bool(row, "temporary")?,
                creator_id: column_string(row, "creatorid")?,
                flags: column_i32(row, "asset_flags")? & KNOWN_FLAGS,
            },
            column_i32(row, "access_time")?,
        ));
    }
    let now = mysql_unix_now();
    for (metadata, access_time) in &pending {
        if access_time_is_due(now, *access_time) {
            let _ = update_access(pool, metadata.id, now).await;
        }
    }
    Ok(pending.into_iter().map(|(metadata, _)| metadata).collect())
}

pub(super) async fn clock(pool: &PgPool, id: Uuid) -> Result<Option<AssetClock>, StoreError> {
    let row = sqlx::query(
        "SELECT create_time, access_time, asset_flags FROM xassetsmeta WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(AssetClock {
        create_time: optional_i32(&row, "create_time")?,
        access_time: optional_i32(&row, "access_time")?,
        raw_flags: column_i32(&row, "asset_flags")?,
    }))
}

pub(super) async fn data_rows(pool: &PgPool) -> Result<i64, StoreError> {
    let rows = sqlx::query("SELECT 1 AS present FROM xassetsdata")
        .fetch_all(pool)
        .await?;
    i64::try_from(rows.len()).map_err(|_| StoreError::Row("xasset data count".to_string()))
}

async fn update_access(pool: &PgPool, id: Uuid, now: i32) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE xassetsmeta SET access_time = $2 WHERE id = $1::uuid")
        .bind(id.to_string())
        .bind(now)
        .execute(pool)
        .await
        .map(|_| ())
}
