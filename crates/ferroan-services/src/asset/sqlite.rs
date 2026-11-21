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
//   Source/OpenSim.Data.SQLite/SQLiteAssetData.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Existence checks are parameterised. The C# method concatenates UUID text
// after UUID.Parse has accepted it. The rows returned are the same.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `SQLiteAssetData` at the pin. The table has no `create_time` or `access_time`.

use sqlx::SqlitePool;
use uuid::Uuid;

use super::decode::{column_bool, column_bytes, column_i8, column_i32, column_string, is_unique};
use super::model::{Asset, AssetMetadata, KNOWN_FLAGS};
use super::store::{AssetClock, StoreError};

pub(super) async fn get(pool: &SqlitePool, id: Uuid) -> Result<Option<Asset>, StoreError> {
    let row = sqlx::query(
        "SELECT UUID, Name, Description, Type, Local, Temporary, asset_flags, CreatorID, Data \
         FROM assets WHERE UUID = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(asset_from_row).transpose()
}

pub(super) async fn exists_one(pool: &SqlitePool, id: Uuid) -> Result<bool, StoreError> {
    let row = sqlx::query("SELECT 1 FROM assets WHERE UUID = ?")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

/// Insert when the id is absent. A concurrent insert leaves the first row.
///
/// `AssetService.Store` checks existence and skips `StoreAsset`. This is that
/// path, not `SQLiteAssetData.StoreAsset`, which would update.
pub(super) async fn insert_new(pool: &SqlitePool, asset: &Asset) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    let exists = sqlx::query("SELECT 1 FROM assets WHERE UUID = ?")
        .bind(asset.id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_some() {
        tx.rollback().await?;
        return Ok(());
    }
    let inserted = insert(&mut *tx, asset).await;
    match inserted {
        Ok(()) => {
            tx.commit().await?;
            Ok(())
        }
        Err(error) if is_unique(&error) => {
            let _ = tx.rollback().await;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// `SQLiteAssetData.StoreAsset`: update when the id exists, otherwise insert.
pub(super) async fn store_data(pool: &SqlitePool, asset: &Asset) -> Result<(), StoreError> {
    if exists_one(pool, asset.id).await? {
        return update(pool, asset).await;
    }
    match insert(pool, asset).await {
        Ok(()) => Ok(()),
        Err(error) if is_unique(&error) => update(pool, asset).await,
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM assets WHERE UUID = ?")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// `SQLiteAssetData.Delete`: an id that does not parse is false. A parsed id
/// is true after the statement, including when no row was removed.
pub(super) async fn delete_data(pool: &SqlitePool, id: &str) -> Result<bool, StoreError> {
    let Ok(id) = Uuid::parse_str(id.trim()) else {
        return Ok(false);
    };
    delete(pool, id).await?;
    Ok(true)
}

pub(super) async fn metadata_set(
    pool: &SqlitePool,
    start: i32,
    count: i32,
) -> Result<Vec<AssetMetadata>, StoreError> {
    let rows = sqlx::query(
        "SELECT Name, Description, Type, Temporary, asset_flags, UUID, CreatorID \
         FROM assets LIMIT ?, ?",
    )
    .bind(start)
    .bind(count)
    .fetch_all(pool)
    .await?;
    rows.iter().map(metadata_from_row).collect()
}

pub(super) async fn clock(pool: &SqlitePool, id: Uuid) -> Result<Option<AssetClock>, StoreError> {
    let row = sqlx::query("SELECT asset_flags FROM assets WHERE UUID = ?")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(AssetClock {
        create_time: None,
        access_time: None,
        raw_flags: column_i32(&row, "asset_flags")?,
    }))
}

async fn insert<'a, E>(executor: E, asset: &Asset) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'a, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO assets(UUID, Name, Description, Type, Local, Temporary, asset_flags, CreatorID, Data) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(asset.id.to_string())
    .bind(asset.stored_name())
    .bind(asset.stored_description())
    .bind(i32::from(asset.type_code))
    .bind(i32::from(asset.local))
    .bind(i32::from(asset.temporary))
    .bind(asset.flags)
    .bind(&asset.creator_id)
    .bind(&asset.data)
    .execute(executor)
    .await?;
    Ok(())
}

async fn update(pool: &SqlitePool, asset: &Asset) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE assets SET Name = ?, Description = ?, Type = ?, Local = ?, Temporary = ?, \
         asset_flags = ?, CreatorID = ?, Data = ? WHERE UUID = ?",
    )
    .bind(asset.stored_name())
    .bind(asset.stored_description())
    .bind(i32::from(asset.type_code))
    .bind(i32::from(asset.local))
    .bind(i32::from(asset.temporary))
    .bind(asset.flags)
    .bind(&asset.creator_id)
    .bind(&asset.data)
    .bind(asset.id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

fn asset_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Asset, StoreError> {
    let id = Uuid::parse_str(column_string(&row, "UUID")?.trim())
        .map_err(|error| StoreError::Row(error.to_string()))?;
    Ok(Asset {
        id,
        name: column_string(&row, "Name")?,
        description: column_string(&row, "Description")?,
        type_code: column_i8(&row, "Type")?,
        local: column_bool(&row, "Local")?,
        temporary: column_bool(&row, "Temporary")?,
        creator_id: column_string(&row, "CreatorID")?,
        flags: column_i32(&row, "asset_flags")? & KNOWN_FLAGS,
        data: column_bytes(&row, "Data")?,
        upload_attempts: 0,
    })
}

fn metadata_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AssetMetadata, StoreError> {
    Ok(AssetMetadata {
        id: Uuid::parse_str(column_string(row, "UUID")?.trim())
            .map_err(|error| StoreError::Row(error.to_string()))?,
        name: column_string(row, "Name")?,
        description: column_string(row, "Description")?,
        type_code: column_i8(row, "Type")?,
        temporary: column_bool(row, "Temporary")?,
        creator_id: column_string(row, "CreatorID")?,
        flags: column_i32(row, "asset_flags")? & KNOWN_FLAGS,
    })
}
