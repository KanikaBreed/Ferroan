// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite `assets` rows, matching `SQLiteAssetData` at Tranquillity
//! `713857f10b8e91a639a967d4354a4682b080793c`.
//!
//! Existence checks are parameterised. The C# method concatenates UUID text
//! into the statement after `UUID.Parse` has accepted it. The rows returned
//! are the same.
//!
//! `AssetService.Store` does not call the database when the id is already
//! present, so an existing row is left unchanged. `UpdateContent` in that
//! service returns false and is not implemented here.

use ferroan_db::Pool;
use sqlx::Row;
use uuid::Uuid;

use super::model::Asset;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("this asset service only opens a SQLite store")]
    NotSqlite,
    #[error("asset database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("asset row {0} is not readable")]
    Row(String),
}

pub struct AssetStore {
    pool: Pool,
}

impl AssetStore {
    pub fn new(pool: Pool) -> Result<Self, StoreError> {
        if pool.sqlite().is_none() {
            return Err(StoreError::NotSqlite);
        }
        Ok(Self { pool })
    }

    fn pool(&self) -> &sqlx::sqlite::SqlitePool {
        // INFALLIBLE: new() returns StoreError::NotSqlite when sqlite() is None.
        #[allow(clippy::expect_used)]
        self.pool.sqlite().expect("checked in new")
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Asset>, StoreError> {
        let row = sqlx::query(
            "SELECT UUID, Name, Description, Type, Local, Temporary, asset_flags, CreatorID, Data \
             FROM assets WHERE UUID = ?",
        )
        .bind(id.to_string())
        .fetch_optional(self.pool())
        .await?;
        row.map(asset_from_row).transpose()
    }

    /// Insert when the id is absent. An existing id is not rewritten.
    ///
    /// `AssetService.Store` checks existence and skips `StoreAsset`. The
    /// SQLite plugin would update a row that is already there; this method
    /// follows the service, so a second store leaves the row unchanged.
    /// The check and the insert share one transaction.
    pub async fn store_new(&self, asset: &Asset) -> Result<(), StoreError> {
        let mut tx = self.pool().begin().await?;
        let exists = sqlx::query("SELECT 1 FROM assets WHERE UUID = ?")
            .bind(asset.id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_some() {
            tx.rollback().await?;
            return Ok(());
        }
        let inserted = sqlx::query(
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
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => {
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

    pub async fn delete(&self, id: Uuid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM assets WHERE UUID = ?")
            .bind(id.to_string())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn exists(&self, ids: &[Uuid]) -> Result<Vec<bool>, StoreError> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.exists_one(*id).await?);
        }
        Ok(out)
    }

    async fn exists_one(&self, id: Uuid) -> Result<bool, StoreError> {
        let row = sqlx::query("SELECT 1 FROM assets WHERE UUID = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool())
            .await?;
        Ok(row.is_some())
    }
}

fn asset_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Asset, StoreError> {
    let id_text: String = row.try_get("UUID")?;
    let id = Uuid::parse_str(&id_text).map_err(|_| StoreError::Row(id_text))?;
    let name: String = row.try_get("Name")?;
    let description: String = row.try_get("Description")?;
    let type_code = i64_to_i8(row.try_get("Type")?)?;
    let local = i64_to_bool(row.try_get("Local")?)?;
    let temporary = i64_to_bool(row.try_get("Temporary")?)?;
    let flags = i64_to_i32(row.try_get("asset_flags")?)?;
    let creator_id: String = row.try_get("CreatorID")?;
    let data: Vec<u8> = row.try_get("Data")?;
    Ok(Asset {
        id,
        name,
        description,
        type_code,
        local,
        temporary,
        creator_id,
        flags,
        data,
        upload_attempts: 0,
    })
}

fn i64_to_i8(value: i64) -> Result<i8, StoreError> {
    i8::try_from(value).map_err(|_| StoreError::Row(format!("type {value}")))
}

fn i64_to_i32(value: i64) -> Result<i32, StoreError> {
    i32::try_from(value).map_err(|_| StoreError::Row(format!("flags {value}")))
}

fn i64_to_bool(value: i64) -> Result<bool, StoreError> {
    Ok(value != 0)
}

fn is_unique(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|db| db.is_unique_violation())
}
