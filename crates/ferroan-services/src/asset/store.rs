// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Asset rows on SQLite, MySQL and PostgreSQL.
//!
//! `AssetService.Store` checks existence and skips the plugin when the id is
//! already present. [`AssetStore::store_new`] is that path, which is what the
//! HTTP POST handler calls. [`AssetStore::store_data`] is each plugin's
//! `StoreAsset` when the statement succeeds: SQLite updates, MySQL replaces
//! the row, and PostgreSQL updates the mutable columns.
//!
//! Existence checks are parameterised. The C# methods concatenate UUID text
//! after `UUID.Parse` has accepted it. The rows returned are the same.
//!
//! `FetchAssetMetadataSet` is not capped in C#. A start past one million, or a
//! count outside 0..=10_000, is refused here.

use ferroan_db::{Backend, Pool};
use ferroan_diag::{self, ASSET_METADATA_ROWS, Kind};
use uuid::Uuid;

use super::model::{Asset, AssetMetadata};

/// Largest `count` accepted by [`AssetStore::metadata_set`].
pub const METADATA_ROW_CAP: i32 = 10_000;
/// Largest `start` accepted by [`AssetStore::metadata_set`].
pub const METADATA_START_CAP: i32 = 1_000_000;

/// `create_time`, `access_time` and the flags column before the known-bit mask.
///
/// SQLite has no time columns, so both times are `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetClock {
    pub create_time: Option<i32>,
    pub access_time: Option<i32>,
    pub raw_flags: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("asset store backend {0} has no pool")]
    Backend(&'static str),
    #[error("asset database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("asset row {0} is not readable")]
    Row(String),
    #[error("asset metadata window is outside the bound")]
    Bound,
    #[error("asset file: {0}")]
    Io(#[from] std::io::Error),
}

pub struct AssetStore {
    pool: Pool,
}

impl AssetStore {
    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.pool.occupancy()
    }

    pub const fn backend(&self) -> Backend {
        self.pool.backend()
    }

    /// Accept a pool whose backend is SQLite, MySQL or PostgreSQL.
    pub fn new(pool: Pool) -> Result<Self, StoreError> {
        match pool.backend() {
            Backend::Sqlite | Backend::Mysql | Backend::Postgres => Ok(Self { pool }),
        }
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Asset>, StoreError> {
        match self.backend() {
            Backend::Sqlite => super::sqlite::get(self.sqlite()?, id).await,
            Backend::Mysql => super::mysql::get(self.mysql()?, id).await,
            Backend::Postgres => super::postgres::get(self.postgres()?, id).await,
        }
    }

    /// Insert when the id is absent. An existing id is not rewritten.
    ///
    /// This follows `AssetService.Store`. The plugin methods are
    /// [`Self::store_data`].
    pub async fn store_new(&self, asset: &Asset) -> Result<(), StoreError> {
        if self.exists_one(asset.id).await? {
            return Ok(());
        }
        match self.backend() {
            Backend::Sqlite => super::sqlite::insert_new(self.sqlite()?, asset).await,
            Backend::Mysql => super::mysql::store_data(self.mysql()?, asset).await,
            Backend::Postgres => super::postgres::store_data(self.postgres()?, asset).await,
        }
    }

    /// Plugin `StoreAsset` for a statement that succeeds.
    pub async fn store_data(&self, asset: &Asset) -> Result<(), StoreError> {
        match self.backend() {
            Backend::Sqlite => super::sqlite::store_data(self.sqlite()?, asset).await,
            Backend::Mysql => super::mysql::store_data(self.mysql()?, asset).await,
            Backend::Postgres => super::postgres::store_data(self.postgres()?, asset).await,
        }
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), StoreError> {
        match self.backend() {
            Backend::Sqlite => super::sqlite::delete(self.sqlite()?, id).await,
            Backend::Mysql => super::mysql::delete(self.mysql()?, id).await,
            Backend::Postgres => super::postgres::delete(self.postgres()?, id).await,
        }
    }

    /// Plugin `Delete(string)`. The bool is that method's return value.
    pub async fn delete_data(&self, id: &str) -> Result<bool, StoreError> {
        match self.backend() {
            Backend::Sqlite => super::sqlite::delete_data(self.sqlite()?, id).await,
            Backend::Mysql => super::mysql::delete_data(self.mysql()?, id).await,
            Backend::Postgres => super::postgres::delete_data(self.postgres()?, id).await,
        }
    }

    pub async fn exists(&self, ids: &[Uuid]) -> Result<Vec<bool>, StoreError> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.exists_one(*id).await?);
        }
        Ok(out)
    }

    /// `FetchAssetMetadataSet`. PostgreSQL uses `LIMIT count OFFSET start`.
    pub async fn metadata_set(
        &self,
        start: i32,
        count: i32,
    ) -> Result<Vec<AssetMetadata>, StoreError> {
        metadata_window(start, count)?;
        match self.backend() {
            Backend::Sqlite => super::sqlite::metadata_set(self.sqlite()?, start, count).await,
            Backend::Mysql => super::mysql::metadata_set(self.mysql()?, start, count).await,
            Backend::Postgres => {
                super::postgres::metadata_set(self.postgres()?, start, count).await
            }
        }
    }

    pub async fn clock(&self, id: Uuid) -> Result<Option<AssetClock>, StoreError> {
        match self.backend() {
            Backend::Sqlite => super::sqlite::clock(self.sqlite()?, id).await,
            Backend::Mysql => super::mysql::clock(self.mysql()?, id).await,
            Backend::Postgres => super::postgres::clock(self.postgres()?, id).await,
        }
    }

    async fn exists_one(&self, id: Uuid) -> Result<bool, StoreError> {
        match self.backend() {
            Backend::Sqlite => super::sqlite::exists_one(self.sqlite()?, id).await,
            Backend::Mysql => super::mysql::exists_one(self.mysql()?, id).await,
            Backend::Postgres => super::postgres::exists_one(self.postgres()?, id).await,
        }
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

    /// Close the pool. Tests do this before a C# process opens the same SQLite file.
    pub async fn close(self) {
        self.pool.close().await;
    }
}

/// Tests that write the shared `assets` table take these. `DELETE FROM assets`
/// in the plugin tests removes every row, including a fallback row another
/// test still expects to find.
#[cfg(test)]
pub(crate) static ASSET_MYSQL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(test)]
pub(crate) static ASSET_POSTGRES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

fn metadata_window(start: i32, count: i32) -> Result<(), StoreError> {
    if (0..=METADATA_START_CAP).contains(&start) && (0..=METADATA_ROW_CAP).contains(&count) {
        return Ok(());
    }
    let observed = if !(0..=METADATA_ROW_CAP).contains(&count) {
        count
    } else {
        start
    };
    ferroan_diag::refuse(
        ASSET_METADATA_ROWS,
        Kind::Hard,
        u64::from(METADATA_ROW_CAP.cast_unsigned()),
        u64::from(observed.cast_unsigned()),
        "asset",
    );
    Err(StoreError::Bound)
}
