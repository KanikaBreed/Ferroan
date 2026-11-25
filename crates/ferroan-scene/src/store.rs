// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `terrain` and `bakedterrain` rows for one region, and the startup
//! that writes them when the store has no heightmap.
//!
//! `SQLiteSimulationData.LoadTerrain` (`Source/OpenSim.Data.SQLite/SQLiteSimulationData.cs`)
//! selects `RegionUUID, Revision, Heightfield` where the id matches, ordered by
//! `Revision` descending, and keeps the first row. `LoadBakedTerrain` keeps the
//! first `bakedterrain` row with no order. The id string is `UUID.ToString()`,
//! the lowercase hyphenated form.
//!
//! [`load_world_map`] is `Scene.LoadWorldMap` (`Scene.cs` 2313–2385 at
//! `c401bceecd`). With no `terrain` row it copies the `bakedterrain` row, or
//! builds `[Terrain] InitialTerrain`, and stores it as `terrain`
//! (`StoreTerrain`, delete then insert). When no baked row was loaded it then
//! stores the heightmap as `bakedterrain` (`StoreBakedTerrain`), also when a
//! `terrain` row was there. These two writes are the only ones the scene makes
//! to the region store. Each is one transaction here; C# runs the delete and
//! the insert as two statements.
//!
//! A damaged `terrain` or `bakedterrain` row stops startup and nothing is
//! written (D72, TB88). C# loads a damaged blob as a zeroed or partial map
//! (`TerrainData.cs` 496, 759–765, 822–828, 586–589), keeps the baked row,
//! and copies it into `terrain` when that row is missing; only a NULL cell, an
//! out-of-range revision or a truncated Compressed2D blob (671–713) throws, is
//! logged and is overwritten (`Scene.cs` 2324–2328, 2380–2384). A failed
//! write stops startup here. C# swallows a failed `StoreTerrain` (inside the
//! `try`, `Scene.cs` 2340 and 2353, caught at 2375–2379), but
//! `StoreBakedTerrain` (2383) is outside it and fails region start, so only
//! the case that copies the baked row into `terrain` differs.

use ferroan_db::{Backend, Pool};
use uuid::Uuid;

use crate::error::TerrainError;
use crate::height::{BLOB_CAP, Heightmap, refuse_blob};
use crate::initial::InitialTerrain;
use crate::objects::ObjectStoreKind;

/// The two tables `Scene.LoadWorldMap` reads and writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Table {
    Terrain,
    Baked,
}

impl Table {
    /// `LoadTerrain` orders by revision; `LoadBakedTerrain` takes the first row.
    fn select(self) -> &'static str {
        match self {
            Self::Terrain => {
                "SELECT Revision, length(Heightfield), CASE WHEN length(Heightfield) <= ?2 THEN Heightfield END FROM terrain WHERE RegionUUID = ?1 ORDER BY Revision DESC LIMIT 1"
            }
            Self::Baked => {
                "SELECT Revision, length(Heightfield), CASE WHEN length(Heightfield) <= ?2 THEN Heightfield END FROM bakedterrain WHERE RegionUUID = ?1 LIMIT 1"
            }
        }
    }

    fn delete(self) -> &'static str {
        match self {
            Self::Terrain => "DELETE FROM terrain WHERE RegionUUID = ?1",
            Self::Baked => "DELETE FROM bakedterrain WHERE RegionUUID = ?1",
        }
    }

    fn insert(self) -> &'static str {
        match self {
            Self::Terrain => {
                "INSERT INTO terrain (RegionUUID, Revision, Heightfield) VALUES (?1, ?2, ?3)"
            }
            Self::Baked => {
                "INSERT INTO bakedterrain (RegionUUID, Revision, Heightfield) VALUES (?1, ?2, ?3)"
            }
        }
    }
}

/// Where the region's heightmap came from at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainSource {
    /// The stored `terrain` row.
    Stored,
    /// No `terrain` row: the `bakedterrain` row, now also stored as `terrain`.
    Baked,
    /// No row of either kind: the initial terrain, now stored as both rows.
    Generated,
    /// No store file: the initial terrain, not stored. Ferroan does not create
    /// a region store (it runs no migrations).
    GeneratedUnstored,
}

/// The heightmap the region runs with, and what [`load_world_map`] did.
#[derive(Clone, Debug)]
pub struct WorldMap {
    pub map: Heightmap,
    pub source: TerrainSource,
    /// `true` when this start wrote the `bakedterrain` row.
    pub stored_baked: bool,
}

/// `None` when the store has no terrain row. A damaged blob is an error.
pub async fn read_terrain(
    pool: &Pool,
    region: Uuid,
    width: i32,
    height: i32,
) -> Result<Option<Heightmap>, TerrainError> {
    read_row(pool, Table::Terrain, region, width, height).await
}

/// `LoadBakedTerrain`: the first `bakedterrain` row, or `None`.
pub async fn read_baked_terrain(
    pool: &Pool,
    region: Uuid,
    width: i32,
    height: i32,
) -> Result<Option<Heightmap>, TerrainError> {
    read_row(pool, Table::Baked, region, width, height).await
}

/// One row of `table`. The cell is fetched only when `length(Heightfield)`
/// is within [`BLOB_CAP`]; a larger one is refused through `terrain.blob`
/// before sqlx loads it. A NULL cell is damaged.
async fn read_row(
    pool: &Pool,
    table: Table,
    region: Uuid,
    width: i32,
    height: i32,
) -> Result<Option<Heightmap>, TerrainError> {
    let Some(sqlite) = pool.sqlite() else {
        return Err(TerrainError::Store("connection is not SQLite".to_owned()));
    };
    let row: Option<(i64, Option<i64>, Option<Vec<u8>>)> = sqlx::query_as(table.select())
        .bind(region.as_hyphenated().to_string())
        .bind(i64::try_from(BLOB_CAP).unwrap_or(i64::MAX))
        .fetch_optional(sqlite)
        .await
        .map_err(|error| TerrainError::Store(error.to_string()))?;
    let Some((revision, length, blob)) = row else {
        return Ok(None);
    };
    let Some(length) = length else {
        return Err(TerrainError::Damaged);
    };
    let Some(blob) = blob else {
        refuse_blob(usize::try_from(length).unwrap_or(usize::MAX));
        return Err(TerrainError::Blob);
    };
    let revision = i32::try_from(revision).map_err(|_| TerrainError::Revision(i32::MIN))?;
    Ok(Some(Heightmap::from_blob(revision, &blob, width, height)?))
}

/// `StoreTerrain`: replace this region's `terrain` rows with one revision 23 row.
pub async fn store_terrain(pool: &Pool, region: Uuid, map: &Heightmap) -> Result<(), TerrainError> {
    store(pool, Table::Terrain, region, map).await
}

/// `StoreBakedTerrain`: replace this region's `bakedterrain` rows.
pub async fn store_baked_terrain(
    pool: &Pool,
    region: Uuid,
    map: &Heightmap,
) -> Result<(), TerrainError> {
    store(pool, Table::Baked, region, map).await
}

async fn store(
    pool: &Pool,
    table: Table,
    region: Uuid,
    map: &Heightmap,
) -> Result<(), TerrainError> {
    let Some(sqlite) = pool.sqlite() else {
        return Err(TerrainError::Store("connection is not SQLite".to_owned()));
    };
    let (revision, blob) = map.to_blob()?;
    let id = region.as_hyphenated().to_string();
    let failed = |error: sqlx::Error| TerrainError::Store(error.to_string());
    let mut tx = sqlite.begin().await.map_err(failed)?;
    sqlx::query(table.delete())
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    sqlx::query(table.insert())
        .bind(&id)
        .bind(i64::from(revision))
        .bind(blob)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)
}

/// `Scene.LoadWorldMap` on an open store: read both rows, fill in the
/// missing ones, and return the heightmap the region runs with.
pub async fn world_map(
    pool: &Pool,
    region: Uuid,
    width: i32,
    height: i32,
    initial: InitialTerrain,
) -> Result<WorldMap, TerrainError> {
    let baked = read_baked_terrain(pool, region, width, height).await?;
    let (map, source) = match read_terrain(pool, region, width, height).await? {
        Some(map) => (map, TerrainSource::Stored),
        None => {
            let (map, source) = match &baked {
                Some(baked) => (baked.clone(), TerrainSource::Baked),
                None => (initial.generate(width, height)?, TerrainSource::Generated),
            };
            store_terrain(pool, region, &map).await?;
            (map, source)
        }
    };
    let stored_baked = baked.is_none();
    if stored_baked {
        store_baked_terrain(pool, region, &map).await?;
    }
    Ok(WorldMap {
        map,
        source,
        stored_baked,
    })
}

/// Open the region store read and write, require `RegionStore` at the
/// pinned migration, and run [`world_map`]. A missing file gets the initial
/// terrain in memory and nothing is written.
pub async fn load_world_map(
    connection: &str,
    region: Uuid,
    width: i32,
    height: i32,
    initial: InitialTerrain,
) -> Result<WorldMap, TerrainError> {
    let path = ferroan_db::sqlite_filename(connection)
        .map_err(|error| TerrainError::Store(error.to_string()))?;
    if !path.exists() {
        return Ok(WorldMap {
            map: initial.generate(width, height)?,
            source: TerrainSource::GeneratedUnstored,
            stored_baked: false,
        });
    }
    let pool = Pool::open(Backend::Sqlite, connection, &["RegionStore"])
        .await
        .map_err(|error| TerrainError::Store(error.to_string()))?;
    let loaded = world_map(&pool, region, width, height, initial).await;
    pool.close().await;
    loaded
}

/// Open the region store, require `RegionStore` at the pinned migration, and
/// read its terrain. `None` means the file is absent, the table has no row
/// for this region, or the provider is MySQL or PostgreSQL. Those stores have
/// no saved heightmap yet and the connection string is not read as SQLite.
/// `Standalone.ini` names `URI=file:OpenSim.db`, which C# creates; this row
/// does not, so a missing file is no saved heightmap.
pub async fn open_terrain(
    connection: &str,
    region: Uuid,
    width: i32,
    height: i32,
    kind: ObjectStoreKind,
) -> Result<Option<Heightmap>, TerrainError> {
    if kind != ObjectStoreKind::Sqlite {
        return Ok(None);
    }
    let path = ferroan_db::sqlite_filename(connection)
        .map_err(|error| TerrainError::Store(error.to_string()))?;
    if !path.exists() {
        return Ok(None);
    }
    let pool = Pool::open(Backend::Sqlite, connection, &["RegionStore"])
        .await
        .map_err(|error| TerrainError::Store(error.to_string()))?;
    read_terrain(&pool, region, width, height).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use crate::objects::ObjectStoreKind;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    fn gzip_blob() -> Vec<u8> {
        let mut plain = Vec::new();
        plain.extend_from_slice(&16i32.to_le_bytes());
        plain.extend_from_slice(&16i32.to_le_bytes());
        for y in 0..16 {
            for x in 0..16 {
                let h = if x == 0 && y == 15 { 40.0f32 } else { 21.0 };
                plain.extend_from_slice(&h.to_le_bytes());
            }
        }
        let mut gzip = Vec::new();
        let mut enc = GzEncoder::new(&mut gzip, Compression::default());
        enc.write_all(&plain).unwrap();
        enc.finish().unwrap();
        gzip
    }

    #[tokio::test]
    async fn a_missing_region_file_has_no_terrain() {
        let path = std::env::temp_dir().join(format!(
            "ferroan-terrain-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let connection = format!("URI=file:{};UseUTF16Encoding=True", path.display());
        let map = open_terrain(&connection, Uuid::nil(), 256, 256, ObjectStoreKind::Sqlite)
            .await
            .unwrap();
        assert!(map.is_none());
    }

    #[tokio::test]
    async fn region_store_revision_is_checked_before_the_blob() {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-terrain-{}-{}",
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
        let connection = format!("Data Source={};Version=3", path.display());
        let region = Uuid::from_u128(0x6d1a_7f4e_0c3b_4f53_9a1e_5f0e_3c2a_9b01);
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        assert!(read_terrain(&pool, region, 16, 16).await.unwrap().is_none());
        let blob = gzip_blob();
        sqlx::query("INSERT INTO terrain (RegionUUID, Revision, Heightfield) VALUES (?1, ?2, ?3)")
            .bind(region.as_hyphenated().to_string())
            .bind(23_i64)
            .bind(&blob)
            .execute(pool.sqlite().unwrap())
            .await
            .unwrap();
        let map = read_terrain(&pool, region, 16, 16).await.unwrap().unwrap();
        assert_eq!(map.cells()[15 * 16], 40.0);
        assert_eq!(map.height_at(0.0, 15.0), 40.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scratch_store(name: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-worldmap-{name}-{}-{}",
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

    async fn rows(pool: &Pool, table: &str, region: Uuid) -> Vec<(i64, Vec<u8>)> {
        sqlx::query_as(&format!(
            "SELECT Revision, Heightfield FROM {table} WHERE RegionUUID = ?1"
        ))
        .bind(region.as_hyphenated().to_string())
        .fetch_all(pool.sqlite().unwrap())
        .await
        .unwrap()
    }

    fn same_cells(a: &Heightmap, b: &Heightmap) -> bool {
        a.cells().len() == b.cells().len()
            && a.cells()
                .iter()
                .zip(b.cells())
                .all(|(x, y)| x.to_bits() == y.to_bits())
    }

    const REGION: Uuid = Uuid::from_u128(0x6d1a_7f4e_0c3b_4f53_9a1e_5f0e_3c2a_9b01);

    #[tokio::test]
    async fn an_empty_store_gets_the_island_as_both_rows() {
        let (dir, connection) = scratch_store("empty");
        let first = load_world_map(&connection, REGION, 256, 256, InitialTerrain::PinheadIsland)
            .await
            .unwrap();
        assert_eq!(first.source, TerrainSource::Generated);
        assert!(first.stored_baked);
        // The region runs with the unrounded island, as C# does on this start.
        let island = InitialTerrain::PinheadIsland.generate(256, 256).unwrap();
        assert!(same_cells(&first.map, &island));

        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let terrain = rows(&pool, "terrain", REGION).await;
        let baked = rows(&pool, "bakedterrain", REGION).await;
        assert_eq!(terrain.len(), 1);
        assert_eq!(baked.len(), 1);
        assert_eq!(terrain[0].0, 23);
        assert_eq!(baked[0].0, 23);
        let csharp = Heightmap::from_blob(
            23,
            &std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/walk/terrain.bin"),
            )
            .unwrap(),
            256,
            256,
        )
        .unwrap();
        for (_, blob) in terrain.iter().chain(&baked) {
            let map = Heightmap::from_blob(23, blob, 256, 256).unwrap();
            assert!(same_cells(&map, &csharp), "a stored row is C#'s row");
        }
        pool.close().await;

        // The next start reads the row and writes nothing.
        let second = load_world_map(&connection, REGION, 256, 256, InitialTerrain::Flat)
            .await
            .unwrap();
        assert_eq!(second.source, TerrainSource::Stored);
        assert!(!second.stored_baked);
        assert!(same_cells(&second.map, &csharp));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_terrain_row_without_a_baked_row_gets_the_baked_row() {
        let (dir, connection) = scratch_store("terrain-only");
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let flat = InitialTerrain::Flat.generate(256, 256).unwrap();
        store_terrain(&pool, REGION, &flat).await.unwrap();
        let loaded = world_map(&pool, REGION, 256, 256, InitialTerrain::PinheadIsland)
            .await
            .unwrap();
        assert_eq!(loaded.source, TerrainSource::Stored);
        assert!(loaded.stored_baked);
        assert!(same_cells(&loaded.map, &flat));
        let baked = read_baked_terrain(&pool, REGION, 256, 256)
            .await
            .unwrap()
            .unwrap();
        assert!(same_cells(&baked, &flat));
        assert_eq!(rows(&pool, "terrain", REGION).await.len(), 1);
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_baked_row_without_a_terrain_row_is_copied() {
        let (dir, connection) = scratch_store("baked-only");
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let flat = InitialTerrain::Flat.generate(256, 256).unwrap();
        store_baked_terrain(&pool, REGION, &flat).await.unwrap();
        let before = rows(&pool, "bakedterrain", REGION).await;
        let loaded = world_map(&pool, REGION, 256, 256, InitialTerrain::PinheadIsland)
            .await
            .unwrap();
        assert_eq!(loaded.source, TerrainSource::Baked);
        assert!(!loaded.stored_baked);
        assert!(same_cells(&loaded.map, &flat));
        let terrain = read_terrain(&pool, REGION, 256, 256)
            .await
            .unwrap()
            .unwrap();
        assert!(same_cells(&terrain, &flat));
        // The baked row is left as it was.
        assert_eq!(rows(&pool, "bakedterrain", REGION).await, before);
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_oversized_or_null_cell_is_refused_before_it_is_read() {
        let (dir, connection) = scratch_store("oversized");
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        sqlx::query("INSERT INTO bakedterrain (RegionUUID, Revision, Heightfield) VALUES (?1, 23, zeroblob(?2))")
            .bind(REGION.as_hyphenated().to_string())
            .bind(i64::try_from(BLOB_CAP).unwrap() + 1)
            .execute(pool.sqlite().unwrap())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO terrain (RegionUUID, Revision, Heightfield) VALUES (?1, 23, NULL)",
        )
        .bind(REGION.as_hyphenated().to_string())
        .execute(pool.sqlite().unwrap())
        .await
        .unwrap();
        assert!(matches!(
            read_baked_terrain(&pool, REGION, 256, 256).await,
            Err(TerrainError::Blob)
        ));
        assert!(matches!(
            read_terrain(&pool, REGION, 256, 256).await,
            Err(TerrainError::Damaged)
        ));
        // Startup stops and writes nothing.
        assert!(
            world_map(&pool, REGION, 256, 256, InitialTerrain::PinheadIsland)
                .await
                .is_err()
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM terrain) + (SELECT COUNT(*) FROM bakedterrain)",
        )
        .fetch_one(pool.sqlite().unwrap())
        .await
        .unwrap();
        assert_eq!(count, 2);
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_missing_store_runs_the_island_without_writing() {
        let path = std::env::temp_dir().join(format!(
            "ferroan-worldmap-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let connection = format!("URI=file:{};UseUTF16Encoding=True", path.display());
        let loaded = load_world_map(&connection, REGION, 256, 256, InitialTerrain::PinheadIsland)
            .await
            .unwrap();
        assert_eq!(loaded.source, TerrainSource::GeneratedUnstored);
        assert!(!loaded.stored_baked);
        assert_eq!(loaded.map.height_at(128.0, 128.0), 25.0);
        assert!(!path.exists());
    }

    /// `cargo xtask regionstore verify` (and `cross`): the rows Ferroan
    /// writes for an empty store load through the pin's `LoadTerrain` and
    /// `LoadBakedTerrain` to the cells of C#'s own row,
    /// `fixtures/walk/terrain.bin`.
    #[tokio::test]
    async fn regionstore_terrain_rows_load_in_csharp() {
        if std::env::var("FERROAN_REGIONSTORE").ok().as_deref() != Some("1") {
            eprintln!("regionstore terrain oracle load skipped");
            return;
        }
        let dll = std::env::var("FERROAN_REGIONSTORE_ORACLE").unwrap();
        let (dir, connection) = scratch_store("oracle");
        let loaded = load_world_map(&connection, REGION, 256, 256, InitialTerrain::PinheadIsland)
            .await
            .unwrap();
        assert_eq!(loaded.source, TerrainSource::Generated);
        let path = ferroan_db::sqlite_filename(&connection).unwrap();
        let output = std::process::Command::new("dotnet")
            .arg(&dll)
            .args([
                "terrain-load",
                &path.display().to_string(),
                &REGION.as_hyphenated().to_string(),
                ferroan_db::TRANQUILLITY_REVISION,
            ])
            .env("MSBUILDDISABLENODEREUSE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let dump: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let csharp = Heightmap::from_blob(
            23,
            &std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/walk/terrain.bin"),
            )
            .unwrap(),
            256,
            256,
        )
        .unwrap();
        let expected: Vec<u64> = csharp
            .cells()
            .iter()
            .map(|h| u64::from(h.to_bits()))
            .collect();
        for table in ["terrain", "bakedterrain"] {
            let row = &dump[table];
            assert_eq!(row["size_x"], 256, "{table}");
            assert_eq!(row["size_y"], 256, "{table}");
            assert_eq!(row["stored_revision"], 23, "{table}");
            let cells: Vec<u64> = row["cells"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| cell.as_u64().unwrap())
                .collect();
            assert!(
                cells == expected,
                "{table}: C# loads the island Ferroan stored"
            );
            eprintln!(
                "{table}: C# loads {} cells; stored {} gzip bytes, the pin writes {} for the same map; same bytes: {}",
                cells.len(),
                row["stored_bytes"],
                row["csharp_bytes"],
                row["csharp_blob_is_stored_blob"]
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn mysql_terrain_does_not_open_the_connection_string() {
        let map = open_terrain(
            "Data Source=127.0.0.1;Database=ferroan;User ID=ferroan;Password=unused",
            Uuid::nil(),
            256,
            256,
            ObjectStoreKind::Mysql,
        )
        .await
        .unwrap();
        assert!(map.is_none());
    }
}
