// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Estate owner and managers for one region.
//!
//! `EstateDataService` (`Source/OpenSim.Services.EstateService/EstateDataService.cs`)
//! reads `[DatabaseService] ConnectionString`, then `EstateConnectionString`,
//! then `[EstateDataStore] ConnectionString`. SQLite stores the region on
//! `estate_map.RegionID` as `UUID.ToString()` and the managers on
//! `estate_managers.uuid` (`SQLiteEstateData.LoadUUIDList`).
//! `Constants.MaxEstateManagers` is 20. This read stops there. It does not
//! open MySQL or PostgreSQL and it does not create an estate.

use ferroan_db::{Backend, Pool};
use uuid::Uuid;

/// `Constants.MaxEstateManagers` at the pin.
pub const MAX_ESTATE_MANAGERS: usize = 20;

/// Owner and managers loaded for one region. `owner` is `None` when the row
/// has no usable id; the caller keeps its INI owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EstateAccess {
    pub owner: Option<Uuid>,
    pub managers: Vec<Uuid>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EstateError(String);

/// `None` when the file is absent or this region has no `estate_map` row.
/// A missing `EstateStore`, a non-SQLite string, or a version mismatch is an error.
pub async fn open_estate(
    connection: &str,
    region: Uuid,
) -> Result<Option<EstateAccess>, EstateError> {
    let path =
        ferroan_db::sqlite_filename(connection).map_err(|error| EstateError(error.to_string()))?;
    if !path.exists() {
        return Ok(None);
    }
    let pool = Pool::open(Backend::Sqlite, connection, &["EstateStore"])
        .await
        .map_err(|error| EstateError(error.to_string()))?;
    read_estate(&pool, region).await
}

async fn read_estate(pool: &Pool, region: Uuid) -> Result<Option<EstateAccess>, EstateError> {
    let Some(sqlite) = pool.sqlite() else {
        return Err(EstateError("connection is not SQLite".to_owned()));
    };
    let estate: Option<(i64,)> =
        sqlx::query_as("SELECT EstateID FROM estate_map WHERE lower(RegionID) = ?1 LIMIT 1")
            .bind(region.as_hyphenated().to_string())
            .fetch_optional(sqlite)
            .await
            .map_err(|error| EstateError(error.to_string()))?;
    let Some((estate,)) = estate else {
        return Ok(None);
    };
    let owner: Option<(String,)> =
        sqlx::query_as("SELECT EstateOwner FROM estate_settings WHERE EstateID = ?1 LIMIT 1")
            .bind(estate)
            .fetch_optional(sqlite)
            .await
            .map_err(|error| EstateError(error.to_string()))?;
    let owner = owner.and_then(|(text,)| parse_agent(&text));
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT uuid FROM estate_managers WHERE EstateID = ?1 ORDER BY uuid LIMIT ?2",
    )
    .bind(estate)
    .bind(i64::try_from(MAX_ESTATE_MANAGERS).unwrap_or(i64::MAX))
    .fetch_all(sqlite)
    .await
    .map_err(|error| EstateError(error.to_string()))?;
    let mut managers = Vec::new();
    for (text,) in rows {
        let Some(id) = parse_agent(&text) else {
            continue;
        };
        if !managers.contains(&id) {
            managers.push(id);
        }
    }
    Ok(Some(EstateAccess { owner, managers }))
}

fn parse_agent(text: &str) -> Option<Uuid> {
    let id = text.trim().parse::<Uuid>().ok()?;
    (!id.is_nil()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn empty_store(path: &std::path::Path) {
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        for statement in [
            "CREATE TABLE migrations (name TEXT, version INTEGER)",
            "INSERT INTO migrations (name, version) VALUES ('migrations', 1)",
            "INSERT INTO migrations (name, version) VALUES ('EstateStore', 13)",
            "CREATE TABLE estate_map (RegionID TEXT, EstateID INTEGER)",
            "CREATE TABLE estate_settings (EstateID INTEGER PRIMARY KEY, EstateOwner TEXT)",
            "CREATE TABLE estate_managers (EstateID INTEGER, uuid TEXT)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn a_missing_file_has_no_estate() {
        let path = std::env::temp_dir().join(format!(
            "ferroan-estate-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let connection = format!("Data Source={};Version=3", path.display());
        let access = open_estate(&connection, Uuid::from_u128(1)).await.unwrap();
        assert!(access.is_none());
    }

    #[tokio::test]
    async fn owner_and_managers_come_from_the_region_row() {
        let path = std::env::temp_dir().join(format!(
            "ferroan-estate-row-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        empty_store(&path).await;
        let region = Uuid::from_u128(0x0e57_a7e0_0000_4000_8000_0000_0000_0001);
        let owner = Uuid::from_u128(0x0e57_a7e0_0000_4000_8000_0000_0000_0002);
        let manager = Uuid::from_u128(0x0e57_a7e0_0000_4000_8000_0000_0000_0003);
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("INSERT INTO estate_map (RegionID, EstateID) VALUES (?1, 7)")
            .bind(region.as_hyphenated().to_string().to_ascii_uppercase())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO estate_settings (EstateID, EstateOwner) VALUES (7, ?1)")
            .bind(owner.as_hyphenated().to_string())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO estate_managers (EstateID, uuid) VALUES (7, ?1)")
            .bind(manager.as_hyphenated().to_string())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO estate_managers (EstateID, uuid) VALUES (7, ?1)")
            .bind(manager.as_hyphenated().to_string())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO estate_managers (EstateID, uuid) VALUES (7, 'not-a-uuid')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let connection = format!("Data Source={};Version=3", path.display());
        let access = open_estate(&connection, region).await.unwrap().unwrap();
        assert_eq!(access.owner, Some(owner));
        assert_eq!(access.managers, vec![manager]);
        assert!(
            open_estate(&connection, Uuid::from_u128(9))
                .await
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_file(&path);
    }
}
