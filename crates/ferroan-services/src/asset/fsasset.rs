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
//   Source/OpenSim.Data.PGSQL/PGSQLFSAssetData.cs
//   Source/OpenSim.Data.MySQL/Resources/FSAssetStore.migrations
//   Source/OpenSim.Data.PGSQL/Resources/FSAssetStore.migrations
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `IFSAssetDataPlugin` for MySQL and PostgreSQL.
//!
//! There is no SQLite plugin at the pin. This store does not run
//! `FSAssetStore` migrations, write asset files, or serve HTTP.
//! `Realm` is the MySQL table name and defaults to `fsassets`. PostgreSQL
//! ignores `Realm` and always uses `fsassets`, as `PGSQLFSAssetData` does.
//!
//! `Get` reads the row and updates `access_time` when the age is at least
//! `DaysBetweenAccessTimeUpdates`. The default is 0, so every get updates.
//! MySQL writes `UNIX_TIMESTAMP()`. PostgreSQL writes the local clock read
//! as UTC, the same formula as `DateTime.Now` in the plugin.
//!
//! MySQL `Store` drops code points above U+FFFF from the name and description
//! before insert, and a failed insert is returned (TB22). An existing id is
//! left unchanged and answers success. PostgreSQL `Store` of an id that
//! already has a row updates `access_time` only and answers success (TB35).
//! PostgreSQL `Delete` of a non-uuid deletes nothing and returns false (TB39).
//! Plugin `AssetsExist` reads the table only. The service existence check is
//! a separate hash query plus a file check, and it does not update
//! `access_time`. Import reads one source row, then the caller stores it,
//! before the next read. Both engines order by `id`. The walk seeks the
//! start row once, then reads `WHERE id > ? ORDER BY id LIMIT 1`, so a long
//! table is not rescanned for every row. MySQL binds the stored id text.
//! A row that does not sort after that text is an error. A whole-table read
//! still skips `start` rows. The pin's own page is still MySQL
//! `LIMIT start, count` and PostgreSQL `LIMIT count OFFSET start` (TB40).
//! `store_forced` updates the hash of an existing row
//! (TB45). An ordinary `store` does not. Files are `FsFiles`, not this store.
//! AssetService's metadata page and refused write are that store.

use std::sync::atomic::{AtomicI32, Ordering};

use ferroan_db::{Backend, Pool};
use ferroan_diag::{self, ASSET_FSASSETS_IMPORT_ROW, Kind};
use uuid::Uuid;

use super::store::StoreError;

/// MySQL `Realm` default, and the only PostgreSQL table.
pub(super) const DEFAULT_TABLE: &str = "fsassets";

/// One `fsassets` row, without the file bytes. The hash is the SHA-256 hex
/// the service stored. Name and description are empty on PostgreSQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsMetadata {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub type_code: i8,
    pub flags: i32,
}

/// `Get`'s metadata and hash. `create_time` is the stored unix seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsAsset {
    pub metadata: FsMetadata,
    pub hash: String,
    pub create_time: i32,
}

/// Hash and type from one existence query. A missing id is `None`, not this
/// struct. The query does not update `access_time`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsHash {
    pub hash: String,
    pub type_code: i8,
}

/// A row `Import` would hand to its store delegate. PostgreSQL leaves the
/// name and description empty. The bytes are the source table's `data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsImportRow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub type_code: i8,
    pub create_time: i32,
    pub data: Vec<u8>,
}

/// `create_time`, `access_time`, the hash and the flags column before the mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsClock {
    pub hash: String,
    pub create_time: i32,
    pub access_time: i32,
    pub raw_flags: i32,
    pub name: String,
    pub description: String,
}

pub struct FsAssetStore {
    pool: Pool,
    table: String,
    access_days: AtomicI32,
}

#[cfg(test)]
pub(crate) static FSASSET_MYSQL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(test)]
pub(crate) static FSASSET_POSTGRES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl FsAssetStore {
    /// MySQL and PostgreSQL only. `Realm` is `fsassets` and the access
    /// interval is 0, which updates `access_time` on every get.
    pub fn new(pool: Pool) -> Result<Self, StoreError> {
        Self::with_settings(pool, DEFAULT_TABLE, 0)
    }

    /// `Initialise(connect, realm, SkipAccessTimeDays)`. PostgreSQL keeps
    /// `fsassets` whatever `realm` says. A MySQL realm that is not one
    /// identifier is refused. This does not migrate the database.
    pub fn with_settings(pool: Pool, realm: &str, access_days: i32) -> Result<Self, StoreError> {
        let table = match pool.backend() {
            Backend::Mysql => {
                sql_ident(realm)?;
                realm.to_string()
            }
            Backend::Postgres => DEFAULT_TABLE.to_string(),
            Backend::Sqlite => return Err(StoreError::Backend("sqlite")),
        };
        Ok(Self {
            pool,
            table,
            access_days: AtomicI32::new(access_days),
        })
    }

    pub const fn backend(&self) -> Backend {
        self.pool.backend()
    }

    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.pool.occupancy()
    }

    fn days(&self) -> i32 {
        self.access_days.load(Ordering::Relaxed)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<FsAsset>, StoreError> {
        match self.backend() {
            Backend::Mysql => super::fmysql::get(self.mysql()?, &self.table, self.days(), id).await,
            Backend::Postgres => super::fpostgres::get(self.postgres()?, self.days(), id).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// Plugin `Store`. `Ok(true)` means the row is there. A failed insert or
    /// update is `Err`, which is the failure TB22 reports. An existing id is
    /// not rewritten.
    pub async fn store(&self, meta: &FsMetadata, hash: &str) -> Result<bool, StoreError> {
        self.store_row(meta, hash, false).await
    }

    /// `force import`'s store. An existing row's hash and `access_time` are
    /// updated. Name, description, type, flags and `create_time` stay (TB45).
    pub async fn store_forced(&self, meta: &FsMetadata, hash: &str) -> Result<bool, StoreError> {
        self.store_row(meta, hash, true).await
    }

    async fn store_row(
        &self,
        meta: &FsMetadata,
        hash: &str,
        force: bool,
    ) -> Result<bool, StoreError> {
        match self.backend() {
            Backend::Mysql => {
                super::fmysql::store(self.mysql()?, &self.table, self.days(), meta, hash, force)
                    .await
            }
            Backend::Postgres => {
                super::fpostgres::store(self.postgres()?, self.days(), meta, hash, force).await
            }
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// Plugin `AssetsExist`. An empty input returns an empty list. The check
    /// is the table only.
    pub async fn exists(&self, ids: &[Uuid]) -> Result<Vec<bool>, StoreError> {
        match self.backend() {
            Backend::Mysql => super::fmysql::exists(self.mysql()?, &self.table, ids).await,
            Backend::Postgres => super::fpostgres::exists(self.postgres()?, ids).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// Hash and type for each id, in request order. `None` is a missing id
    /// or a null hash. This does not update `access_time`. MySQL answers
    /// `None` for every id when the connection cannot be acquired, including
    /// access denied and an unknown database. A statement error after the
    /// connection is open is returned. PostgreSQL returns either failure.
    pub async fn hashes(&self, ids: &[Uuid]) -> Result<Vec<Option<FsHash>>, StoreError> {
        match self.backend() {
            Backend::Mysql => super::fmysql::hashes(self.mysql()?, &self.table, ids).await,
            Backend::Postgres => super::fpostgres::hashes(self.postgres()?, ids).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    pub async fn count(&self) -> Result<i64, StoreError> {
        match self.backend() {
            Backend::Mysql => super::fmysql::count(self.mysql()?, &self.table).await,
            Backend::Postgres => super::fpostgres::count(self.postgres()?).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// Plugin `Delete`. A statement that runs returns true, including when no
    /// row matched. MySQL returns false for a database error. PostgreSQL
    /// returns false for an id that is not a uuid and returns a statement error.
    pub async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        match self.backend() {
            Backend::Mysql => super::fmysql::delete(self.mysql()?, &self.table, id).await,
            Backend::Postgres => super::fpostgres::delete(self.postgres()?, id).await,
            Backend::Sqlite => Err(StoreError::Backend("sqlite")),
        }
    }

    /// Plugin `Import`'s reader. `count == -1` reads until the table ends,
    /// after skipping `start` rows, one row at a time. The caller stores the
    /// rows. This method does not write `fsassets`.
    pub async fn import_reader(
        &self,
        table: &str,
        start: i32,
        count: i32,
    ) -> Result<Vec<FsImportRow>, StoreError> {
        self.import_from(self.pool(), table, start, count).await
    }

    /// The same reader against a connection the operator named. The pool's
    /// backend must be this store's backend. The rows are collected after
    /// each keyset read of one so a caller that only needs the list still
    /// does not ask the driver for the whole table at once.
    pub async fn import_from(
        &self,
        pool: &Pool,
        table: &str,
        start: i32,
        count: i32,
    ) -> Result<Vec<FsImportRow>, StoreError> {
        if pool.backend() != self.backend() {
            return Err(StoreError::Backend("import"));
        }
        let mut out = Vec::new();
        import_walk(pool, table, start, count, async |row| {
            out.push(row);
            Ok::<(), StoreError>(())
        })
        .await?;
        Ok(out)
    }

    pub async fn clock(&self, id: Uuid) -> Result<Option<FsClock>, StoreError> {
        match self.backend() {
            Backend::Mysql => super::fmysql::clock(self.mysql()?, &self.table, id).await,
            Backend::Postgres => super::fpostgres::clock(self.postgres()?, id).await,
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

    pub(super) fn pool(&self) -> &Pool {
        &self.pool
    }

    #[cfg(test)]
    pub(crate) fn set_access_days(&self, days: i32) {
        self.access_days.store(days, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) async fn wipe(&self) -> Result<(), StoreError> {
        match self.backend() {
            Backend::Mysql => {
                let sql = format!("DELETE FROM {}", self.table);
                sqlx::query(&sql).execute(self.mysql()?).await?;
            }
            Backend::Postgres => {
                sqlx::query("DELETE FROM fsassets")
                    .execute(self.postgres()?)
                    .await?;
            }
            Backend::Sqlite => return Err(StoreError::Backend("sqlite")),
        }
        Ok(())
    }
}

/// One SQL identifier: a letter or underscore, then letters, digits or
/// underscores. Table names are interpolated, so anything else is refused.
pub(super) fn sql_ident(name: &str) -> Result<(), StoreError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(StoreError::Backend("realm"));
    };
    if name.len() > 64 || (!first.is_ascii_alphabetic() && first != '_') {
        return Err(StoreError::Backend("realm"));
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(StoreError::Backend("realm"));
    }
    Ok(())
}

/// What a MySQL `utf8` (`utf8mb3`) column can hold: the basic multilingual
/// plane. A four-byte character is dropped. The 64-character limit is the
/// column, and an insert that is still too long fails.
pub(super) fn utf8mb3(text: &str) -> String {
    text.chars().filter(|ch| u32::from(*ch) <= 0xFFFF).collect()
}

/// `DaysBetweenAccessTimeUpdates <= 0` updates on every read. Otherwise the
/// pin skips the write while `(UtcNow - unix(stored)).TotalDays < days`.
pub(super) fn access_is_due(now: i32, stored: i32, days: i32) -> bool {
    if days <= 0 {
        return true;
    }
    i64::from(now).saturating_sub(i64::from(stored)) >= i64::from(days).saturating_mul(86_400)
}

/// Largest `data` value one legacy import row may carry.
pub(super) const IMPORT_ROW_BYTES: usize = 50_000_000;

/// Refuse a legacy `data` value past [`IMPORT_ROW_BYTES`]. Import reads one
/// row at a time and does not cap that column.
pub(super) fn import_row_within_bound(len: usize) -> Result<(), StoreError> {
    if len <= IMPORT_ROW_BYTES {
        return Ok(());
    }
    ferroan_diag::refuse(
        ASSET_FSASSETS_IMPORT_ROW,
        Kind::Hard,
        u64::try_from(IMPORT_ROW_BYTES).unwrap_or(u64::MAX),
        u64::try_from(len).unwrap_or(u64::MAX),
        "fsassets import",
    );
    Err(StoreError::Bound)
}

/// The value the next keyset predicate binds. MySQL keeps the stored id
/// text. PostgreSQL keeps the uuid.
enum ImportCursor {
    Mysql(String),
    Postgres(Uuid),
}

fn cursor_advances(previous: &ImportCursor, next: &ImportCursor) -> bool {
    match (previous, next) {
        (ImportCursor::Mysql(previous), ImportCursor::Mysql(next)) => next > previous,
        (ImportCursor::Postgres(previous), ImportCursor::Postgres(next)) => next > previous,
        _ => false,
    }
}

/// Read the legacy table one row at a time and hand each row to `on_row`
/// before the next read. `count == -1` reads until the table ends, after
/// skipping `start` rows. `count == 0` reads nothing. `start` skips that many
/// rows in `id` order. The skip is one id-only seek. Each later read is
/// `WHERE id > ? ORDER BY id LIMIT 1`, so the prefix is not scanned again.
/// MySQL binds the stored id text. A row that does not sort after the
/// previous cursor is an error. The callback's error stops the read.
pub(super) async fn import_walk<F, E>(
    pool: &Pool,
    table: &str,
    start: i32,
    count: i32,
    mut on_row: F,
) -> Result<(), E>
where
    F: AsyncFnMut(FsImportRow) -> Result<(), E>,
    E: From<StoreError>,
{
    if count == 0 {
        return Ok(());
    }
    if start < 0 || count < -1 {
        return Err(E::from(StoreError::Bound));
    }
    let mut after = if start == 0 {
        None
    } else {
        match fetch_import_seek(pool, table, start - 1)
            .await
            .map_err(E::from)?
        {
            Some(id) => Some(id),
            None => return Ok(()),
        }
    };
    let mut produced = 0i32;
    loop {
        if count >= 0 && produced >= count {
            break;
        }
        let rows = fetch_import_keyset(pool, table, after.as_ref(), 1)
            .await
            .map_err(E::from)?;
        let Some((row, cursor)) = rows.into_iter().next() else {
            break;
        };
        if let Some(previous) = &after
            && !cursor_advances(previous, &cursor)
        {
            return Err(E::from(StoreError::Row(
                "fsassets import id did not advance".into(),
            )));
        }
        after = Some(cursor);
        on_row(row).await?;
        produced = produced.saturating_add(1);
    }
    Ok(())
}

async fn fetch_import_seek(
    pool: &Pool,
    table: &str,
    offset: i32,
) -> Result<Option<ImportCursor>, StoreError> {
    match pool.backend() {
        Backend::Mysql => {
            let Some(mysql) = pool.mysql() else {
                return Err(StoreError::Backend("mysql"));
            };
            Ok(super::fmysql::import_seek(mysql, table, offset)
                .await?
                .map(ImportCursor::Mysql))
        }
        Backend::Postgres => {
            let Some(postgres) = pool.postgres() else {
                return Err(StoreError::Backend("postgres"));
            };
            Ok(super::fpostgres::import_seek(postgres, table, offset)
                .await?
                .map(ImportCursor::Postgres))
        }
        Backend::Sqlite => Err(StoreError::Backend("sqlite")),
    }
}

async fn fetch_import_keyset(
    pool: &Pool,
    table: &str,
    after: Option<&ImportCursor>,
    limit: i32,
) -> Result<Vec<(FsImportRow, ImportCursor)>, StoreError> {
    match pool.backend() {
        Backend::Mysql => {
            let Some(mysql) = pool.mysql() else {
                return Err(StoreError::Backend("mysql"));
            };
            let text = match after {
                None => None,
                Some(ImportCursor::Mysql(text)) => Some(text.as_str()),
                Some(ImportCursor::Postgres(_)) => return Err(StoreError::Backend("mysql")),
            };
            let rows = super::fmysql::import_keyset(mysql, table, text, limit).await?;
            Ok(rows
                .into_iter()
                .map(|(row, text)| (row, ImportCursor::Mysql(text)))
                .collect())
        }
        Backend::Postgres => {
            let Some(postgres) = pool.postgres() else {
                return Err(StoreError::Backend("postgres"));
            };
            let id = match after {
                None => None,
                Some(ImportCursor::Postgres(id)) => Some(*id),
                Some(ImportCursor::Mysql(_)) => return Err(StoreError::Backend("postgres")),
            };
            let rows = super::fpostgres::import_keyset(postgres, table, id, limit).await?;
            Ok(rows
                .into_iter()
                .map(|row| {
                    let cursor = ImportCursor::Postgres(row.id);
                    (row, cursor)
                })
                .collect())
        }
        Backend::Sqlite => Err(StoreError::Backend("sqlite")),
    }
}

/// One keyset page. `after` is the last id already produced. The limit is a
/// count this process chooses, never a caller string.
pub(super) fn import_keyset_sql(
    table: &str,
    postgres: bool,
    after: bool,
    limit: i32,
) -> Result<String, StoreError> {
    sql_ident(table)?;
    if !(1..=64).contains(&limit) {
        return Err(StoreError::Bound);
    }
    if postgres {
        let predicate = if after {
            " WHERE \"id\" > $1::uuid"
        } else {
            ""
        };
        Ok(format!(
            "SELECT * FROM {table}{predicate} ORDER BY \"id\" LIMIT {limit}"
        ))
    } else {
        let predicate = if after { " WHERE id > ?" } else { "" };
        Ok(format!(
            "SELECT * FROM {table}{predicate} ORDER BY id LIMIT {limit}"
        ))
    }
}

/// The id at `offset` in id order, selected alone. One seek for `start`.
pub(super) fn import_seek_sql(
    table: &str,
    postgres: bool,
    offset: i32,
) -> Result<String, StoreError> {
    sql_ident(table)?;
    if offset < 0 {
        return Err(StoreError::Bound);
    }
    if postgres {
        Ok(format!(
            "SELECT \"id\" FROM {table} ORDER BY \"id\" LIMIT 1 OFFSET {offset}"
        ))
    } else {
        Ok(format!(
            "SELECT id FROM {table} ORDER BY id LIMIT {offset},1"
        ))
    }
}

/// The page the pin's `Import` appends. `count == -1` is the whole table.
/// PostgreSQL is `ORDER BY id LIMIT count OFFSET start` (TB40). MySQL is
/// `ORDER BY id LIMIT start, count`. The walk does not send this once per row.
#[cfg(test)]
pub(super) fn import_suffix(start: i32, count: i32, postgres: bool) -> Result<String, StoreError> {
    if start < 0 || count < -1 {
        return Err(StoreError::Bound);
    }
    if count == -1 {
        return Ok(" ORDER BY id".to_string());
    }
    if postgres {
        Ok(format!(" ORDER BY id LIMIT {count} OFFSET {start}"))
    } else {
        Ok(format!(" ORDER BY id LIMIT {start},{count}"))
    }
}

#[cfg(test)]
mod unit {
    use super::{
        IMPORT_ROW_BYTES, access_is_due, import_keyset_sql, import_row_within_bound,
        import_seek_sql, import_suffix, sql_ident, utf8mb3,
    };

    #[test]
    fn utf8mb3_drops_four_byte_characters_and_keeps_the_rest() {
        assert_eq!(utf8mb3("seaweed"), "seaweed");
        assert_eq!(utf8mb3("€"), "€");
        assert_eq!(utf8mb3("a😀b"), "ab");
        assert_eq!(utf8mb3("😀"), "");
        assert_eq!(utf8mb3(&format!("{}😀", "n".repeat(64))), "n".repeat(64));
    }

    #[test]
    fn access_interval_matches_the_pin() {
        let now = 1_700_000_000;
        let day = 86_400;
        assert!(access_is_due(now, now, 0));
        assert!(access_is_due(now, 0, 0));
        assert!(access_is_due(now, now, -1));
        assert!(!access_is_due(now, now, 30));
        assert!(!access_is_due(now, now - 29 * day, 30));
        assert!(access_is_due(now, now - 30 * day, 30));
    }

    #[test]
    fn import_row_bound_refuses_past_the_cap() {
        assert!(import_row_within_bound(0).is_ok());
        assert!(import_row_within_bound(IMPORT_ROW_BYTES).is_ok());
        assert!(import_row_within_bound(IMPORT_ROW_BYTES.saturating_add(1)).is_err());
    }

    #[test]
    fn import_limit_is_count_then_start_in_id_order() {
        assert_eq!(import_suffix(0, -1, false).expect("all"), " ORDER BY id");
        assert_eq!(
            import_suffix(1, 2, false).expect("mysql"),
            " ORDER BY id LIMIT 1,2"
        );
        assert_eq!(
            import_suffix(2, 1, true).expect("postgres"),
            " ORDER BY id LIMIT 1 OFFSET 2"
        );
        assert!(import_suffix(-1, 1, false).is_err());
        assert!(import_suffix(0, -2, true).is_err());
    }

    #[test]
    fn import_walk_reads_the_next_id_instead_of_a_growing_offset() {
        assert_eq!(
            import_keyset_sql("assets", false, false, 1).expect("first"),
            "SELECT * FROM assets ORDER BY id LIMIT 1"
        );
        assert_eq!(
            import_keyset_sql("assets", false, true, 1).expect("mysql"),
            "SELECT * FROM assets WHERE id > ? ORDER BY id LIMIT 1"
        );
        assert_eq!(
            import_keyset_sql("assets", true, true, 1).expect("postgres"),
            "SELECT * FROM assets WHERE \"id\" > $1::uuid ORDER BY \"id\" LIMIT 1"
        );
        assert_eq!(
            import_seek_sql("assets", false, 10).expect("mysql seek"),
            "SELECT id FROM assets ORDER BY id LIMIT 10,1"
        );
        assert_eq!(
            import_seek_sql("assets", true, 3).expect("postgres seek"),
            "SELECT \"id\" FROM assets ORDER BY \"id\" LIMIT 1 OFFSET 3"
        );
        assert!(import_keyset_sql("assets", false, true, 0).is_err());
        assert!(import_keyset_sql("assets;drop", false, false, 1).is_err());
        assert!(import_seek_sql("assets", false, -1).is_err());
    }

    #[test]
    fn realm_is_one_identifier() {
        assert!(sql_ident("fsassets").is_ok());
        assert!(sql_ident("_t").is_ok());
        assert!(sql_ident("").is_err());
        assert!(sql_ident("1bad").is_err());
        assert!(sql_ident("fsassets;drop").is_err());
        assert!(sql_ident(&"a".repeat(65)).is_err());
    }
}

#[cfg(test)]
#[path = "fsasset_tests.rs"]
mod tests;
