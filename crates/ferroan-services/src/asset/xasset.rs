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
//   Source/OpenSim.Data.PGSQL/PGSQLXAssetData.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `IXAssetDataPlugin` for MySQL and PostgreSQL.
//!
//! There is no SQLite plugin at the pin. Compression stays off:
//! `m_enableCompression` defaults to false, and the gzip branch is not the
//! path a default server runs. This store does not chain to another asset
//! service and it does not serve HTTP. `XAssetService` does both.
//!
//! Both plugins hash the stored bytes with SHA-256 and keep one `XAssetsData`
//! row per hash. `StoreAsset` is void: a failed metadata or data write is
//! logged, the transaction is rolled back, and the method returns. That is
//! not the ordinary asset store's TB20 choice. Both `Delete` methods remove
//! the meta row and leave the data row. PostgreSQL binds the id as `uuid`
//! (TB30, #480). Rows are shared by hash and there is no reference count, so
//! an unused data row is not deleted. MySQL `AssetsExist` reads `XAssetsMeta`
//! (TB28, #476). PostgreSQL `FetchAssetMetadataSet` uses `ORDER BY id LIMIT
//! count OFFSET start` and returns the page (TB29, #479).
//!
//! Clocks are `Utils.DateTimeToUnixTime(DateTime.UtcNow)` on both plugins.
//! That is UTC unix seconds, not the ordinary PostgreSQL asset store's local
//! clock. `GetAsset` and `FetchAssetMetadataSet` call `UpdateAccessTime`,
//! which writes only when the stored access time is at least 30 days old.
//! MySQL `GetAsset` skips that call when the stored access time is 0.
//! PostgreSQL metadata uses `ORDER BY id LIMIT count OFFSET start`. The
//! ordinary asset store uses the same limit shape (TB19, #477).

use ferroan_db::{Backend, Pool};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::model::{Asset, AssetMetadata};
use super::store::{AssetClock, StoreError};

/// Seconds in `DaysBetweenAccessTimeUpdates` (30).
const ACCESS_INTERVAL_SECONDS: i64 = 30 * 24 * 60 * 60;

pub struct XAssetStore {
    pool: Pool,
}

#[cfg(test)]
pub(crate) static XASSET_MYSQL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(test)]
pub(crate) static XASSET_POSTGRES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl XAssetStore {
    /// Accept a MySQL or PostgreSQL pool. SQLite has no XAsset plugin.
    pub fn new(pool: Pool) -> Result<Self, StoreError> {
        match pool.backend() {
            Backend::Mysql | Backend::Postgres => Ok(Self { pool }),
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    pub const fn backend(&self) -> Backend {
        self.pool.backend()
    }

    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.pool.occupancy()
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Asset>, StoreError> {
        match self.backend() {
            Backend::Mysql => super::xmysql::get(self.mysql()?, id).await,
            Backend::Postgres => super::xpostgres::get(self.postgres()?, id).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// Plugin `StoreAsset`. A failed metadata or data statement rolls back and
    /// returns `Ok`, which is the void method returning.
    pub async fn store(&self, asset: &Asset) -> Result<(), StoreError> {
        match self.backend() {
            Backend::Mysql => super::xmysql::store(self.mysql()?, asset).await,
            Backend::Postgres => super::xpostgres::store(self.postgres()?, asset).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// Plugin `Delete(string)`. MySQL returns true whenever the statement
    /// runs. PostgreSQL's statement fails when the id is bound as text.
    pub async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        match self.backend() {
            Backend::Mysql => super::xmysql::delete(self.mysql()?, id).await,
            Backend::Postgres => super::xpostgres::delete(self.postgres()?, id).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    pub async fn exists(&self, ids: &[Uuid]) -> Result<Vec<bool>, StoreError> {
        match self.backend() {
            Backend::Mysql => super::xmysql::exists(self.mysql()?, ids).await,
            Backend::Postgres => super::xpostgres::exists(self.postgres()?, ids).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// `FetchAssetMetadataSet`. PostgreSQL sends `LIMIT start, count`.
    pub async fn metadata_set(
        &self,
        start: i32,
        count: i32,
    ) -> Result<Vec<AssetMetadata>, StoreError> {
        match self.backend() {
            Backend::Mysql => super::xmysql::metadata_set(self.mysql()?, start, count).await,
            Backend::Postgres => {
                super::xpostgres::metadata_set(self.postgres()?, start, count).await
            }
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    pub async fn clock(&self, id: Uuid) -> Result<Option<AssetClock>, StoreError> {
        match self.backend() {
            Backend::Mysql => super::xmysql::clock(self.mysql()?, id).await,
            Backend::Postgres => super::xpostgres::clock(self.postgres()?, id).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// How many `XAssetsData` rows exist. Delete does not remove them.
    pub async fn data_rows(&self) -> Result<i64, StoreError> {
        match self.backend() {
            Backend::Mysql => super::xmysql::data_rows(self.mysql()?).await,
            Backend::Postgres => super::xpostgres::data_rows(self.postgres()?).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    fn mysql(&self) -> Result<&sqlx::MySqlPool, StoreError> {
        self.pool.mysql().ok_or(StoreError::Backend("mysql"))
    }

    fn postgres(&self) -> Result<&sqlx::PgPool, StoreError> {
        self.pool.postgres().ok_or(StoreError::Backend("postgres"))
    }

    #[cfg(test)]
    pub(super) fn pool(&self) -> &Pool {
        &self.pool
    }

    #[cfg(test)]
    pub(crate) async fn wipe(&self) -> Result<(), StoreError> {
        match self.backend() {
            Backend::Mysql => {
                sqlx::query("DELETE FROM XAssetsMeta")
                    .execute(self.mysql()?)
                    .await?;
                sqlx::query("DELETE FROM XAssetsData")
                    .execute(self.mysql()?)
                    .await?;
            }
            Backend::Postgres => {
                sqlx::query("DELETE FROM xassetsmeta")
                    .execute(self.postgres()?)
                    .await?;
                sqlx::query("DELETE FROM xassetsdata")
                    .execute(self.postgres()?)
                    .await?;
            }
            Backend::Sqlite => return Err(StoreError::Backend("sqlite")),
        }
        Ok(())
    }
}

pub(super) fn content_hash(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&digest);
    hash
}

/// `UpdateAccessTime` writes when `(UtcNow - unix(stored)).TotalDays >= 30`.
pub(super) fn access_time_is_due(now: i32, stored: i32) -> bool {
    i64::from(now).saturating_sub(i64::from(stored)) >= ACCESS_INTERVAL_SECONDS
}

#[cfg(test)]
mod unit {
    use super::{access_time_is_due, content_hash};

    #[test]
    fn sha256_of_empty_is_the_standard_digest() {
        assert_eq!(
            content_hash(&[]),
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55,
            ]
        );
    }

    #[test]
    fn access_time_updates_after_thirty_days() {
        let now = 1_700_000_000;
        let day = 24 * 60 * 60;
        assert!(!access_time_is_due(now, now));
        assert!(!access_time_is_due(now, now - 29 * day));
        assert!(access_time_is_due(now, now - 30 * day));
        assert!(access_time_is_due(now, 0));
        assert!(!access_time_is_due(now, now + 10));
    }
}

#[cfg(test)]
#[path = "xasset_tests.rs"]
mod tests;
