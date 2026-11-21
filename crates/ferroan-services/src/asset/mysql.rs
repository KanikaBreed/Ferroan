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
//   Source/OpenSim.Data.MySQL/MySQLAssetData.cs
//   Source/OpenSim.Data.MySQL/Resources/AssetStore.migrations
//   OpenSim-NGC/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `MySQLAssetData` at the pin.
//!
//! `StoreAsset` is `REPLACE INTO`, so an existing id is deleted and inserted
//! again and both timestamps become the current UTC unix second. Get does not
//! call `UpdateAccessTime`. `Delete` returns true whenever the statement
//! runs, including when the id is not a UUID and when no row was removed.
//! A failed statement here is an error. The C# method catches that and
//! returns false (TB20).

use sqlx::MySqlPool;
use uuid::Uuid;

use super::decode::{
    column_bool, column_bytes, column_i8, column_i32, column_string, column_uuid, mysql_local,
    optional_i32,
};
use super::model::{Asset, AssetMetadata, KNOWN_FLAGS};
use super::store::{AssetClock, StoreError};
use super::time::mysql_unix_now;

pub(super) async fn get(pool: &MySqlPool, id: Uuid) -> Result<Option<Asset>, StoreError> {
    let row = sqlx::query(
        "SELECT name, description, assetType, local, temporary, asset_flags, CreatorID, data \
         FROM assets WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(Asset {
        id,
        name: column_string(&row, "name")?,
        description: column_string(&row, "description")?,
        type_code: column_i8(&row, "assetType")?,
        local: mysql_local(&row, "local")?,
        temporary: column_bool(&row, "temporary")?,
        creator_id: column_string(&row, "CreatorID")?,
        flags: column_i32(&row, "asset_flags")? & KNOWN_FLAGS,
        data: column_bytes(&row, "data")?,
        upload_attempts: 0,
    }))
}

pub(super) async fn exists_one(pool: &MySqlPool, id: Uuid) -> Result<bool, StoreError> {
    let row = sqlx::query("SELECT 1 FROM assets WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub(super) async fn store_data(pool: &MySqlPool, asset: &Asset) -> Result<(), StoreError> {
    let now = mysql_unix_now();
    sqlx::query(
        "REPLACE INTO assets(id, name, description, assetType, local, temporary, create_time, \
         access_time, asset_flags, CreatorID, data) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(asset.id.to_string())
    .bind(asset.stored_name())
    .bind(asset.stored_description())
    .bind(asset.type_code)
    .bind(i8::from(asset.local))
    .bind(i8::from(asset.temporary))
    .bind(now)
    .bind(now)
    .bind(asset.flags)
    .bind(&asset.creator_id)
    .bind(&asset.data)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn delete(pool: &MySqlPool, id: Uuid) -> Result<(), StoreError> {
    delete_data(pool, &id.to_string()).await?;
    Ok(())
}

pub(super) async fn delete_data(pool: &MySqlPool, id: &str) -> Result<bool, StoreError> {
    sqlx::query("DELETE FROM assets WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(true)
}

pub(super) async fn metadata_set(
    pool: &MySqlPool,
    start: i32,
    count: i32,
) -> Result<Vec<AssetMetadata>, StoreError> {
    let rows = sqlx::query(
        "SELECT name, description, assetType, temporary, id, asset_flags, CreatorID \
         FROM assets LIMIT ?, ?",
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
            creator_id: column_string(row, "CreatorID")?,
            flags: column_i32(row, "asset_flags")? & KNOWN_FLAGS,
        });
    }
    Ok(out)
}

pub(super) async fn clock(pool: &MySqlPool, id: Uuid) -> Result<Option<AssetClock>, StoreError> {
    let row = sqlx::query("SELECT create_time, access_time, asset_flags FROM assets WHERE id = ?")
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
