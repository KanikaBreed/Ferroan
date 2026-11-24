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
//   Source/OpenSim.Data.MySQL/MySQLXAssetData.cs
//   Source/OpenSim.Data.MySQL/Resources/XAssetStore.migrations
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `MySQLXAssetData` at the pin.
//!
//! `StoreAsset` is `REPLACE INTO XAssetsMeta`, so a second store of an id
//! rewrites `CreateTime` and `AccessTime`, then inserts `XAssetsData` with
//! `INSERT IGNORE` so a hash another store just wrote does not roll this one
//! back (TB58, D43). The metadata replace, the data insert and the exists-hash
//! read enlist the connection's transaction (TB27, #475). A failed metadata or
//! data statement rolls back and the method returns. A hash-lookup error is
//! returned. `GetAsset` joins the tables and masks flags. It calls
//! `UpdateAccessTime` only when the stored access
//! time is greater than 0. `AssetsExist` selects `XAssetsMeta` (TB28, #476).
//! `Delete` removes the meta row, leaves the data row, and returns true
//! whenever the statement runs.

use sqlx::MySqlPool;
use uuid::Uuid;

use super::decode::{
    column_bool, column_bytes, column_i8, column_i32, column_string, column_uuid, mysql_local,
    optional_i32,
};
use super::model::{Asset, AssetMetadata, KNOWN_FLAGS};
use super::store::{AssetClock, StoreError};
use super::time::mysql_unix_now;
use super::xasset::{access_time_is_due, content_hash};

pub(super) async fn get(pool: &MySqlPool, id: Uuid) -> Result<Option<Asset>, StoreError> {
    let row = sqlx::query(
        "SELECT Name, Description, AccessTime, AssetType, Local, Temporary, AssetFlags, \
         CreatorID, Data FROM XAssetsMeta \
         JOIN XAssetsData ON XAssetsMeta.Hash = XAssetsData.Hash WHERE ID = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let access_time = column_i32(&row, "AccessTime")?;
    let asset = Asset {
        id,
        name: column_string(&row, "Name")?,
        description: column_string(&row, "Description")?,
        type_code: column_i8(&row, "AssetType")?,
        local: mysql_local(&row, "Local")?,
        temporary: column_bool(&row, "Temporary")?,
        creator_id: column_string(&row, "CreatorID")?,
        flags: column_i32(&row, "AssetFlags")? & KNOWN_FLAGS,
        data: column_bytes(&row, "Data")?,
        upload_attempts: 0,
    };
    if access_time > 0 {
        let now = mysql_unix_now();
        if access_time_is_due(now, access_time) {
            let _ = update_access(pool, id, now).await;
        }
    }
    Ok(Some(asset))
}

pub(super) async fn store(pool: &MySqlPool, asset: &Asset) -> Result<(), StoreError> {
    let hash = content_hash(&asset.data);
    let now = mysql_unix_now();
    let name = asset.stored_name();
    let description = asset.stored_description();
    let mut tx = pool.begin().await?;
    let meta = sqlx::query(
        "REPLACE INTO XAssetsMeta(ID, Hash, Name, Description, AssetType, Local, Temporary, \
         CreateTime, AccessTime, AssetFlags, CreatorID) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(asset.id.to_string())
    .bind(hash.as_slice())
    .bind(&name)
    .bind(&description)
    .bind(asset.type_code)
    .bind(i8::from(asset.local))
    .bind(i8::from(asset.temporary))
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
            "xasset mysql metadata write rolled back"
        );
        tx.rollback().await?;
        return Ok(());
    }
    let present = match sqlx::query("SELECT Hash FROM XAssetsData WHERE Hash = ?")
        .bind(hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(row) => row.is_some(),
        Err(error) => {
            tracing::error!(
                asset_id = %asset.id,
                error = %error,
                "xasset mysql hash lookup failed"
            );
            tx.rollback().await?;
            return Err(error.into());
        }
    };
    if !present {
        let inserted = sqlx::query("INSERT IGNORE INTO XAssetsData(Hash, Data) VALUES (?, ?)")
            .bind(hash.as_slice())
            .bind(&asset.data)
            .execute(&mut *tx)
            .await;
        if let Err(error) = inserted {
            tracing::error!(
                asset_id = %asset.id,
                error = %error,
                "xasset mysql data write rolled back"
            );
            tx.rollback().await?;
            return Ok(());
        }
    }
    tx.commit().await?;
    Ok(())
}

pub(super) async fn delete(pool: &MySqlPool, id: &str) -> Result<bool, StoreError> {
    sqlx::query("DELETE FROM XAssetsMeta WHERE ID = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(true)
}

/// Selects `XAssetsMeta` (TB28, #476). An empty input returns an empty list.
pub(super) async fn exists(pool: &MySqlPool, ids: &[Uuid]) -> Result<Vec<bool>, StoreError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row = sqlx::query("SELECT 1 FROM XAssetsMeta WHERE ID = ?")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;
        out.push(row.is_some());
    }
    Ok(out)
}

pub(super) async fn metadata_set(
    pool: &MySqlPool,
    start: i32,
    count: i32,
) -> Result<Vec<AssetMetadata>, StoreError> {
    let rows = sqlx::query(
        "SELECT Name, Description, AccessTime, AssetType, Temporary, ID, AssetFlags, CreatorID \
         FROM XAssetsMeta LIMIT ?, ?",
    )
    .bind(start)
    .bind(count)
    .fetch_all(pool)
    .await?;
    let mut pending = Vec::with_capacity(rows.len());
    for row in &rows {
        pending.push((
            AssetMetadata {
                id: column_uuid(row, "ID")?,
                name: column_string(row, "Name")?,
                description: column_string(row, "Description")?,
                type_code: column_i8(row, "AssetType")?,
                temporary: column_bool(row, "Temporary")?,
                creator_id: column_string(row, "CreatorID")?,
                flags: column_i32(row, "AssetFlags")? & KNOWN_FLAGS,
            },
            column_i32(row, "AccessTime")?,
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

pub(super) async fn clock(pool: &MySqlPool, id: Uuid) -> Result<Option<AssetClock>, StoreError> {
    let row =
        sqlx::query("SELECT CreateTime, AccessTime, AssetFlags FROM XAssetsMeta WHERE ID = ?")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(AssetClock {
        create_time: optional_i32(&row, "CreateTime")?,
        access_time: optional_i32(&row, "AccessTime")?,
        raw_flags: column_i32(&row, "AssetFlags")?,
    }))
}

pub(super) async fn data_rows(pool: &MySqlPool) -> Result<i64, StoreError> {
    let rows = sqlx::query("SELECT 1 AS present FROM XAssetsData")
        .fetch_all(pool)
        .await?;
    i64::try_from(rows.len()).map_err(|_| StoreError::Row("xasset data count".to_string()))
}

async fn update_access(pool: &MySqlPool, id: Uuid, now: i32) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE XAssetsMeta SET AccessTime = ? WHERE ID = ?")
        .bind(now)
        .bind(id.to_string())
        .execute(pool)
        .await
        .map(|_| ())
}
