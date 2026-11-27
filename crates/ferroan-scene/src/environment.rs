// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One region's environment settings row.
//!
//! `LoadRegionEnvironmentSettings`, `StoreRegionEnvironmentSettings` and
//! `RemoveRegionEnvironmentSettings` at Pin8 `c401bceecd`:
//! `SQLiteSimulationData.cs` 425–476, `MySQLSimulationData.cs` 818–882 and
//! `PGSQLSimulationData.cs` 829–889. The cell is the string the caller passes.
//! `StoreOnRegion` (`EnvironmentModule.cs` 222) passes
//! `SerializeLLSDNotationFull` of the environment, or removes the row when
//! the environment is null. This module does not parse that document.
//!
//! The table is `regionenvironment`, created by each backend's `RegionStore`
//! migrations. SQLite version 31 stores `region_id` as varchar and
//! `llsd_settings` as text. MySQL version 31 does the same and version 62
//! widens the settings column to `MEDIUMTEXT`. PostgreSQL version 37 stores
//! `region_id` as uuid and `llsd_settings` as varchar. SQLite and PostgreSQL
//! declare `llsd_settings NOT NULL`, and `region_id` is the primary key on
//! each backend. MySQL version 62's `MODIFY llsd_settings MEDIUMTEXT` drops
//! `NOT NULL`, so only MySQL can hold a NULL cell. Ferroan does not run
//! those migrations. A store it opens is already at the pinned `RegionStore`
//! version, which includes the table.
//!
//! SQLite and MySQL key the row by `UUID.ToString()`, the lowercase
//! hyphenated form. PostgreSQL keys it by the uuid column. A missing row
//! loads as an empty string. MySQL `REPLACE`s the row. SQLite updates the
//! existing row or inserts one; Ferroan deletes and inserts in one
//! transaction, which leaves the same cells. PostgreSQL deletes and then
//! inserts on two connections (TB205). Ferroan runs those two statements in
//! one transaction (D140), and the terrain writer already does the same, so
//! a failed insert does not leave the row missing. A cell longer than
//! [`ENVIRONMENT_BYTES`] is refused (`region.environment-bytes`).

use ferroan_db::Pool;
use ferroan_diag::{Kind, REGION_ENVIRONMENT_BYTES};
use thiserror::Error;
use uuid::Uuid;

/// MySQL `MEDIUMTEXT` holds at most this many bytes. Pin8 has no matching cap.
pub const ENVIRONMENT_BYTES: usize = 16_777_215;

/// A region-environment read or write failed.
#[derive(Debug, Error)]
pub enum EnvironmentError {
    /// The database rejected the statement, or the connection is not one of
    /// the three region stores.
    #[error("region environment store: {0}")]
    Store(String),
    /// `llsd_settings` is longer than [`ENVIRONMENT_BYTES`].
    #[error("region environment settings exceed {ENVIRONMENT_BYTES} bytes")]
    TooLarge,
}

fn refuse_settings(observed: u64) {
    ferroan_diag::refuse(
        REGION_ENVIRONMENT_BYTES,
        Kind::Hard,
        ENVIRONMENT_BYTES as u64,
        observed,
        "region",
    );
}

/// Byte length, and the cell when that length is within the cap. NULL is `None`.
type EnvironmentRow = (Option<i64>, Option<String>);

/// `LoadRegionEnvironmentSettings`. A missing row is an empty string.
/// The cell is selected only when its byte length is within
/// [`ENVIRONMENT_BYTES`]; a longer one is refused before sqlx reads it.
pub async fn load_region_environment(
    pool: &Pool,
    region: Uuid,
) -> Result<String, EnvironmentError> {
    let id = region.as_hyphenated().to_string();
    let failed = |error: sqlx::Error| EnvironmentError::Store(error.to_string());
    let cap = i64::try_from(ENVIRONMENT_BYTES).unwrap_or(i64::MAX);
    // A NULL cell is `Convert.ToString` on MySQL and PostgreSQL, which is
    // empty. SQLite casts the cell to string and throws. Only MySQL can
    // store NULL: version 62 drops `NOT NULL`. SQLite and PostgreSQL declare
    // it, so that error cannot occur on a pinned store of those backends.
    let (cell, null_errors) = if let Some(sqlite) = pool.sqlite() {
        // `length()` counts characters for TEXT. The cast measures bytes.
        let cell: Option<EnvironmentRow> = sqlx::query_as(
            "SELECT length(CAST(llsd_settings AS BLOB)), CASE WHEN length(CAST(llsd_settings AS BLOB)) <= ?2 THEN llsd_settings END FROM regionenvironment WHERE region_id = ?1",
        )
        .bind(&id)
        .bind(cap)
        .fetch_optional(sqlite)
        .await
        .map_err(failed)?;
        (cell, true)
    } else if let Some(mysql) = pool.mysql() {
        let cell: Option<EnvironmentRow> = sqlx::query_as(
            "SELECT CAST(LENGTH(llsd_settings) AS SIGNED), CASE WHEN LENGTH(llsd_settings) <= ? THEN llsd_settings END FROM regionenvironment WHERE region_id = ?",
        )
        .bind(cap)
        .bind(&id)
        .fetch_optional(mysql)
        .await
        .map_err(failed)?;
        (cell, false)
    } else if let Some(postgres) = pool.postgres() {
        let cell: Option<EnvironmentRow> = sqlx::query_as(
            "SELECT octet_length(llsd_settings)::int8, CASE WHEN octet_length(llsd_settings) <= $2 THEN llsd_settings END FROM regionenvironment WHERE region_id = $1",
        )
        .bind(region)
        .bind(cap)
        .fetch_optional(postgres)
        .await
        .map_err(failed)?;
        (cell, false)
    } else {
        return Err(EnvironmentError::Store(
            "connection is not SQLite, MySQL or PostgreSQL".to_owned(),
        ));
    };
    match cell {
        // The CASE withheld the cell, so this length is past the cap.
        Some((Some(length), None)) => {
            refuse_settings(u64::try_from(length).unwrap_or(u64::MAX));
            Err(EnvironmentError::TooLarge)
        }
        Some((Some(_), Some(text))) => Ok(text),
        Some((None, _)) if null_errors => {
            Err(EnvironmentError::Store("llsd_settings is null".to_owned()))
        }
        None | Some((None, _)) => Ok(String::new()),
    }
}

/// `StoreRegionEnvironmentSettings`. MySQL uses `REPLACE`. SQLite and
/// PostgreSQL delete the region's row and insert the new one, in one
/// transaction (D140). A value longer than [`ENVIRONMENT_BYTES`] is refused
/// before any write.
pub async fn store_region_environment(
    pool: &Pool,
    region: Uuid,
    settings: &str,
) -> Result<(), EnvironmentError> {
    if settings.len() > ENVIRONMENT_BYTES {
        refuse_settings(u64::try_from(settings.len()).unwrap_or(u64::MAX));
        return Err(EnvironmentError::TooLarge);
    }
    let id = region.as_hyphenated().to_string();
    let failed = |error: sqlx::Error| EnvironmentError::Store(error.to_string());
    if let Some(sqlite) = pool.sqlite() {
        let mut tx = sqlite.begin().await.map_err(failed)?;
        sqlx::query("DELETE FROM regionenvironment WHERE region_id = ?1")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        sqlx::query("INSERT INTO regionenvironment (region_id, llsd_settings) VALUES (?1, ?2)")
            .bind(&id)
            .bind(settings)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        return tx.commit().await.map_err(failed);
    }
    if let Some(mysql) = pool.mysql() {
        sqlx::query("REPLACE INTO regionenvironment (region_id, llsd_settings) VALUES (?, ?)")
            .bind(&id)
            .bind(settings)
            .execute(mysql)
            .await
            .map_err(failed)?;
        return Ok(());
    }
    if let Some(postgres) = pool.postgres() {
        let mut tx = postgres.begin().await.map_err(failed)?;
        sqlx::query("DELETE FROM regionenvironment WHERE region_id = $1")
            .bind(region)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        sqlx::query("INSERT INTO regionenvironment (region_id, llsd_settings) VALUES ($1, $2)")
            .bind(region)
            .bind(settings)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        return tx.commit().await.map_err(failed);
    }
    Err(EnvironmentError::Store(
        "connection is not SQLite, MySQL or PostgreSQL".to_owned(),
    ))
}

/// `RemoveRegionEnvironmentSettings`. A missing row stays missing.
pub async fn remove_region_environment(pool: &Pool, region: Uuid) -> Result<(), EnvironmentError> {
    let id = region.as_hyphenated().to_string();
    let failed = |error: sqlx::Error| EnvironmentError::Store(error.to_string());
    if let Some(sqlite) = pool.sqlite() {
        sqlx::query("DELETE FROM regionenvironment WHERE region_id = ?1")
            .bind(&id)
            .execute(sqlite)
            .await
            .map_err(failed)?;
        return Ok(());
    }
    if let Some(mysql) = pool.mysql() {
        sqlx::query("DELETE FROM regionenvironment WHERE region_id = ?")
            .bind(&id)
            .execute(mysql)
            .await
            .map_err(failed)?;
        return Ok(());
    }
    if let Some(postgres) = pool.postgres() {
        sqlx::query("DELETE FROM regionenvironment WHERE region_id = $1")
            .bind(region)
            .execute(postgres)
            .await
            .map_err(failed)?;
        return Ok(());
    }
    Err(EnvironmentError::Store(
        "connection is not SQLite, MySQL or PostgreSQL".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        ENVIRONMENT_BYTES, EnvironmentError, load_region_environment, remove_region_environment,
        store_region_environment,
    };
    use ferroan_db::{Backend, Pool};
    use ferroan_diag::REGION_ENVIRONMENT_BYTES;
    use uuid::Uuid;

    use crate::objects::ObjectStoreKind;

    const REGION: Uuid = Uuid::from_u128(0x7e45_0000_0000_4000_8000_0000_0000_0001);
    const OTHER: Uuid = Uuid::from_u128(0x7e45_0000_0000_4000_8000_0000_0000_0002);

    fn scratch_store(name: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-environment-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("RegionStore.db");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/db/sqlite/RegionStore.db");
        std::fs::copy(&fixture, &path).unwrap();
        (
            dir,
            format!("URI=file:{};UseUTF16Encoding=True", path.display()),
        )
    }

    async fn open(kind: ObjectStoreKind, connection: &str) -> Pool {
        let backend = match kind {
            ObjectStoreKind::Mysql => Backend::Mysql,
            ObjectStoreKind::Postgres => Backend::Postgres,
            ObjectStoreKind::Sqlite => Backend::Sqlite,
        };
        Pool::open(backend, connection, &["RegionStore"])
            .await
            .unwrap()
    }

    async fn count(pool: &Pool, region: Uuid) -> i64 {
        let id = region.as_hyphenated().to_string();
        if let Some(sqlite) = pool.sqlite() {
            sqlx::query_scalar("SELECT COUNT(*) FROM regionenvironment WHERE region_id = ?1")
                .bind(id)
                .fetch_one(sqlite)
                .await
                .unwrap()
        } else if let Some(mysql) = pool.mysql() {
            sqlx::query_scalar("SELECT COUNT(*) FROM regionenvironment WHERE region_id = ?")
                .bind(id)
                .fetch_one(mysql)
                .await
                .unwrap()
        } else {
            sqlx::query_scalar("SELECT COUNT(*) FROM regionenvironment WHERE region_id = $1")
                .bind(region)
                .fetch_one(pool.postgres().unwrap())
                .await
                .unwrap()
        }
    }

    async fn round_trip(pool: &Pool) {
        assert_eq!(load_region_environment(pool, REGION).await.unwrap(), "");
        assert_eq!(count(pool, REGION).await, 0);
        store_region_environment(pool, OTHER, "kept").await.unwrap();
        store_region_environment(pool, REGION, "one").await.unwrap();
        assert_eq!(load_region_environment(pool, REGION).await.unwrap(), "one");
        assert_eq!(count(pool, REGION).await, 1);
        let replaced = "second\ncafé \"quote\"";
        store_region_environment(pool, REGION, replaced)
            .await
            .unwrap();
        assert_eq!(
            load_region_environment(pool, REGION).await.unwrap(),
            replaced
        );
        assert_eq!(count(pool, REGION).await, 1);
        assert_eq!(load_region_environment(pool, OTHER).await.unwrap(), "kept");
        store_region_environment(pool, REGION, "").await.unwrap();
        assert_eq!(load_region_environment(pool, REGION).await.unwrap(), "");
        assert_eq!(count(pool, REGION).await, 1);
        remove_region_environment(pool, REGION).await.unwrap();
        assert_eq!(load_region_environment(pool, REGION).await.unwrap(), "");
        assert_eq!(count(pool, REGION).await, 0);
        remove_region_environment(pool, REGION).await.unwrap();
        assert_eq!(load_region_environment(pool, OTHER).await.unwrap(), "kept");
        remove_region_environment(pool, OTHER).await.unwrap();
        assert_eq!(count(pool, OTHER).await, 0);
    }

    #[tokio::test]
    async fn sqlite_environment_replaces_and_removes_one_region() {
        let (dir, connection) = scratch_store("unit");
        let pool = open(ObjectStoreKind::Sqlite, &connection).await;
        round_trip(&pool).await;
        pool.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_environment_past_the_byte_cap_is_refused() {
        let (dir, connection) = scratch_store("cap");
        let pool = open(ObjectStoreKind::Sqlite, &connection).await;
        let before = crate::parcels::limit_hits(REGION_ENVIRONMENT_BYTES, "refuse");
        let at_cap = "a".repeat(ENVIRONMENT_BYTES);
        store_region_environment(&pool, REGION, &at_cap)
            .await
            .unwrap();
        assert_eq!(
            load_region_environment(&pool, REGION).await.unwrap(),
            at_cap
        );
        drop(at_cap);
        remove_region_environment(&pool, REGION).await.unwrap();
        assert_eq!(count(&pool, REGION).await, 0);

        match store_region_environment(&pool, REGION, &"b".repeat(ENVIRONMENT_BYTES + 1)).await {
            Err(EnvironmentError::TooLarge) => {}
            Err(error) => panic!("store: {error}"),
            Ok(()) => panic!("store wrote past the cap"),
        }
        assert_eq!(count(&pool, REGION).await, 0);
        assert_eq!(
            crate::parcels::limit_hits(REGION_ENVIRONMENT_BYTES, "refuse"),
            before + 1
        );

        // One past the cap in bytes, and half that in characters: `length()`
        // on the text would accept it. Each U+00E9 is two UTF-8 bytes.
        let characters = i64::try_from(ENVIRONMENT_BYTES.div_ceil(2)).unwrap();
        assert_eq!(
            characters * 2,
            i64::try_from(ENVIRONMENT_BYTES).unwrap() + 1
        );
        sqlx::query(
            "INSERT INTO regionenvironment (region_id, llsd_settings) VALUES (?1, replace(hex(zeroblob(?2)), '00', char(233)))",
        )
        .bind(REGION.as_hyphenated().to_string())
        .bind(characters)
        .execute(pool.sqlite().unwrap())
        .await
        .unwrap();
        assert_eq!(count(&pool, REGION).await, 1);
        match load_region_environment(&pool, REGION).await {
            Err(EnvironmentError::TooLarge) => {}
            Err(error) => panic!("load: {error}"),
            Ok(text) => panic!("load returned {} bytes", text.len()),
        }
        assert_eq!(count(&pool, REGION).await, 1);
        assert_eq!(
            crate::parcels::limit_hits(REGION_ENVIRONMENT_BYTES, "refuse"),
            before + 2
        );
        pool.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    struct Cross {
        kind: ObjectStoreKind,
        connection: String,
        ado: String,
        prefix: &'static str,
    }

    fn cross(kind: ObjectStoreKind) -> Option<Cross> {
        if std::env::var("FERROAN_REGIONSTORE_CROSS").ok().as_deref() != Some("1") {
            return None;
        }
        let (url, ado, prefix) = match kind {
            ObjectStoreKind::Mysql => (
                "FERROAN_REGIONSTORE_MYSQL",
                "FERROAN_REGIONSTORE_MYSQL_ADO",
                "mysql-",
            ),
            ObjectStoreKind::Postgres => (
                "FERROAN_REGIONSTORE_POSTGRES",
                "FERROAN_REGIONSTORE_POSTGRES_ADO",
                "postgres-",
            ),
            ObjectStoreKind::Sqlite => return None,
        };
        Some(Cross {
            kind,
            connection: std::env::var(url).ok()?,
            ado: std::env::var(ado).ok()?,
            prefix,
        })
    }

    fn oracle(dll: &str, args: &[&str]) -> serde_json::Value {
        let output = std::process::Command::new("dotnet")
            .arg(dll)
            .args(args)
            .env("MSBUILDDISABLENODEREUSE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn settings_of(value: &serde_json::Value, pin: &str) -> String {
        assert_eq!(value["pin"].as_str(), Some(pin));
        value["settings"].as_str().unwrap().to_owned()
    }

    fn row_of(value: &serde_json::Value, pin: &str, region: &str) -> String {
        assert_eq!(value["region"].as_str(), Some(region));
        settings_of(value, pin)
    }

    /// Pin8 reopens the store after Ferroan writes and removes, and Ferroan
    /// reads the row Pin8 stored and the empty load after Pin8 removes it.
    async fn pin_swap(pool: &Pool, target: &str, prefix: &str, dll: &str) {
        let pin = ferroan_db::TRANQUILLITY_REVISION;
        let region = REGION.as_hyphenated().to_string();
        let sample = settings_of(&oracle(dll, &["environment-text", pin]), pin);
        assert!(!sample.is_empty());
        store_region_environment(pool, REGION, &sample)
            .await
            .unwrap();
        let loaded = oracle(
            dll,
            &[&format!("{prefix}environment-load"), target, &region, pin],
        );
        assert_eq!(row_of(&loaded, pin, &region), sample);
        let replaced = "second\ncafé \"quote\"";
        store_region_environment(pool, REGION, replaced)
            .await
            .unwrap();
        let loaded = oracle(
            dll,
            &[&format!("{prefix}environment-load"), target, &region, pin],
        );
        assert_eq!(row_of(&loaded, pin, &region), replaced);
        remove_region_environment(pool, REGION).await.unwrap();
        let loaded = oracle(
            dll,
            &[&format!("{prefix}environment-load"), target, &region, pin],
        );
        assert_eq!(row_of(&loaded, pin, &region), "");
        let stored = oracle(
            dll,
            &[&format!("{prefix}environment-store"), target, &region, pin],
        );
        assert_eq!(row_of(&stored, pin, &region), sample);
        assert_eq!(load_region_environment(pool, REGION).await.unwrap(), sample);
        let removed = oracle(
            dll,
            &[&format!("{prefix}environment-remove"), target, &region, pin],
        );
        assert_eq!(row_of(&removed, pin, &region), "");
        assert_eq!(load_region_environment(pool, REGION).await.unwrap(), "");
        oracle(
            dll,
            &[&format!("{prefix}environment-store"), target, &region, pin],
        );
        remove_region_environment(pool, REGION).await.unwrap();
        let loaded = oracle(
            dll,
            &[&format!("{prefix}environment-load"), target, &region, pin],
        );
        assert_eq!(row_of(&loaded, pin, &region), "");
        assert_eq!(count(pool, REGION).await, 0);
    }

    #[tokio::test]
    async fn regionstore_environment_round_trip_matches_pin8() {
        if std::env::var("FERROAN_REGIONSTORE").ok().as_deref() != Some("1") {
            eprintln!("regionstore environment oracle load skipped");
            return;
        }
        let dll = std::env::var("FERROAN_REGIONSTORE_ORACLE").unwrap();
        let (dir, connection) = scratch_store("oracle");
        let pool = open(ObjectStoreKind::Sqlite, &connection).await;
        let path = ferroan_db::sqlite_filename(&connection).unwrap();
        pin_swap(&pool, &path.display().to_string(), "", &dll).await;
        pool.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    async fn server_swap(cross: Cross) {
        let dll = std::env::var("FERROAN_REGIONSTORE_ORACLE").unwrap();
        let pool = open(cross.kind, &cross.connection).await;
        remove_region_environment(&pool, REGION).await.unwrap();
        remove_region_environment(&pool, OTHER).await.unwrap();
        round_trip(&pool).await;
        pin_swap(&pool, &cross.ado, cross.prefix, &dll).await;
        // Migration 62 drops NOT NULL. Pin8's Convert.ToString of that cell
        // is empty, and so is Ferroan's load. SQLite and PostgreSQL declare
        // NOT NULL, so this step is MySQL only.
        if cross.kind == ObjectStoreKind::Mysql {
            let region = REGION.as_hyphenated().to_string();
            let pin = ferroan_db::TRANQUILLITY_REVISION;
            sqlx::query(
                "INSERT INTO regionenvironment (region_id, llsd_settings) VALUES (?, NULL)",
            )
            .bind(&region)
            .execute(pool.mysql().unwrap())
            .await
            .unwrap();
            assert_eq!(load_region_environment(&pool, REGION).await.unwrap(), "");
            let loaded = oracle(
                &dll,
                &[
                    &format!("{}environment-load", cross.prefix),
                    &cross.ado,
                    &region,
                    pin,
                ],
            );
            assert_eq!(row_of(&loaded, pin, &region), "");
            remove_region_environment(&pool, REGION).await.unwrap();
            assert_eq!(count(&pool, REGION).await, 0);
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn regionstore_mysql_environment_matches_the_pin() {
        let Some(cross) = cross(ObjectStoreKind::Mysql) else {
            eprintln!("regionstore mysql environment skipped");
            return;
        };
        server_swap(cross).await;
    }

    #[tokio::test]
    async fn regionstore_postgres_environment_matches_the_pin() {
        let Some(cross) = cross(ObjectStoreKind::Postgres) else {
            eprintln!("regionstore postgres environment skipped");
            return;
        };
        server_swap(cross).await;
    }
}
