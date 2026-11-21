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
//   Source/OpenSim.Data.PGSQL/PGSQLAssetData.cs
//   Source/OpenSim.Data.PGSQL/Resources/AssetStore.migrations
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `PGSQLAssetData` at the pin.
//!
//! `StoreAsset` updates the mutable columns of an existing id and does not
//! touch `asset_flags`, `create_time` or `access_time`. A missing id is
//! inserted with both times set from the local clock. Get does not update
//! `access_time`. `FetchAssetMetadataSet` binds `LIMIT count OFFSET start`
//! (TB19, #477) and does not read flags. `count` 0 is an empty page. `Delete`
//! is false when the id does not parse or when no row was removed. A failed
//! statement here is an error. The C# method logs that failure and returns
//! false (TB20, #478). The success-path SQL is unchanged.

use sqlx::PgPool;
use uuid::Uuid;

use super::decode::{
    column_bool, column_bytes, column_i8, column_i32, column_string, column_uuid, optional_i32,
};
use super::model::{Asset, AssetMetadata};
use super::store::{AssetClock, StoreError};
use super::time::postgres_unix_now;

pub(super) async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Asset>, StoreError> {
    let row = sqlx::query(
        "SELECT id, name, description, \"assetType\", local, temporary, asset_flags, creatorid, data \
         FROM assets WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(Asset {
        id: column_uuid(&row, "id")?,
        name: column_string(&row, "name")?,
        description: column_string(&row, "description")?,
        type_code: column_i8(&row, "assetType")?,
        local: column_bool(&row, "local")?,
        temporary: column_bool(&row, "temporary")?,
        creator_id: column_string(&row, "creatorid")?,
        flags: column_i32(&row, "asset_flags")? & super::model::KNOWN_FLAGS,
        data: column_bytes(&row, "data")?,
        upload_attempts: 0,
    }))
}

pub(super) async fn exists_one(pool: &PgPool, id: Uuid) -> Result<bool, StoreError> {
    let row = sqlx::query("SELECT 1 FROM assets WHERE id = $1::uuid")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub(super) async fn store_data(pool: &PgPool, asset: &Asset) -> Result<(), StoreError> {
    let now = postgres_unix_now();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE assets SET name = $2, description = $3, \"assetType\" = $4, local = $5, \
         temporary = $6, creatorid = $7, data = $8 WHERE id = $1::uuid",
    )
    .bind(asset.id.to_string())
    .bind(asset.stored_name())
    .bind(asset.stored_description())
    .bind(i16::from(asset.type_code))
    .bind(asset.local)
    .bind(asset.temporary)
    .bind(&asset.creator_id)
    .bind(&asset.data)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO assets (id, name, description, \"assetType\", local, temporary, \
         create_time, access_time, creatorid, asset_flags, data) \
         SELECT $1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11 \
         WHERE NOT EXISTS (SELECT 1 FROM assets WHERE id = $1::uuid)",
    )
    .bind(asset.id.to_string())
    .bind(asset.stored_name())
    .bind(asset.stored_description())
    .bind(i16::from(asset.type_code))
    .bind(asset.local)
    .bind(asset.temporary)
    .bind(now)
    .bind(now)
    .bind(&asset.creator_id)
    .bind(asset.flags)
    .bind(&asset.data)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn delete(pool: &PgPool, id: Uuid) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM assets WHERE id = $1::uuid")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn delete_data(pool: &PgPool, id: &str) -> Result<bool, StoreError> {
    let Ok(id) = Uuid::parse_str(id.trim()) else {
        return Ok(false);
    };
    let result = sqlx::query("DELETE FROM assets WHERE id = $1::uuid")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub(super) async fn metadata_set(
    pool: &PgPool,
    start: i32,
    count: i32,
) -> Result<Vec<AssetMetadata>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, name, description, \"assetType\", temporary, creatorid \
         FROM assets ORDER BY id LIMIT $2 OFFSET $1",
    )
    .bind(start)
    .bind(count)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(AssetMetadata {
            id: column_uuid(row, "id")?,
            name: column_string(row, "name")?,
            description: column_string(row, "description")?,
            type_code: column_i8(row, "assetType")?,
            temporary: column_bool(row, "temporary")?,
            creator_id: column_string(row, "creatorid")?,
            flags: 0,
        });
    }
    Ok(out)
}

pub(super) async fn clock(pool: &PgPool, id: Uuid) -> Result<Option<AssetClock>, StoreError> {
    let row =
        sqlx::query("SELECT create_time, access_time, asset_flags FROM assets WHERE id = $1::uuid")
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
