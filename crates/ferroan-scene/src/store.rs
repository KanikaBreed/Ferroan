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
//! MySQL and PostgreSQL follow `MySQLSimulationData.cs` (`StoreTerrain` 504,
//! `StoreBakedTerrain` 552, `LoadTerrain` 611, `LoadBakedTerrain` 653) and
//! `PGSQLSimulationData.cs` (`LoadTerrain` 540, `LoadBakedTerrain` 579,
//! `StoreTerrain` 623, `StoreBakedTerrain` 667) at `c401bceecd`: the same
//! revision 23 rows, keyed by the id string on MySQL and by a `uuid` column on
//! PostgreSQL. Both load the highest-revision `terrain` row. PostgreSQL takes
//! the first `bakedterrain` row; MySQL reads them all and keeps the last
//! row's revision with the last cell that is not NULL. Both treat a NULL
//! cell as no map, where SQLite throws on it. C# stores on MySQL from a
//! fire-and-forget callback and on PostgreSQL as two statements on two
//! connections; Ferroan stores each row in one transaction before the frame
//! starts.
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
    fn name(self) -> &'static str {
        match self {
            Self::Terrain => "terrain",
            Self::Baked => "bakedterrain",
        }
    }

    /// SQLite: `LoadTerrain` orders by revision; `LoadBakedTerrain` takes the
    /// first row. `?2` is the blob cap.
    fn sqlite_select(self) -> &'static str {
        match self {
            Self::Terrain => {
                "SELECT Revision, length(Heightfield), CASE WHEN length(Heightfield) <= ?2 THEN Heightfield END FROM terrain WHERE RegionUUID = ?1 ORDER BY Revision DESC LIMIT 1"
            }
            Self::Baked => {
                "SELECT Revision, length(Heightfield), CASE WHEN length(Heightfield) <= ?2 THEN Heightfield END FROM bakedterrain WHERE RegionUUID = ?1 LIMIT 1"
            }
        }
    }

    /// PostgreSQL, the same order as SQLite. `$2` is the blob cap.
    fn postgres_select(self) -> &'static str {
        match self {
            Self::Terrain => {
                r#"SELECT "Revision"::int8, octet_length("Heightfield")::int8, CASE WHEN octet_length("Heightfield") <= $2 THEN "Heightfield" END FROM terrain WHERE "RegionUUID" = $1 ORDER BY "Revision" DESC LIMIT 1"#
            }
            Self::Baked => {
                r#"SELECT "Revision"::int8, octet_length("Heightfield")::int8, CASE WHEN octet_length("Heightfield") <= $2 THEN "Heightfield" END FROM bakedterrain WHERE "RegionUUID" = $1 LIMIT 1"#
            }
        }
    }

    fn sqlite_delete(self) -> &'static str {
        match self {
            Self::Terrain => "DELETE FROM terrain WHERE RegionUUID = ?1",
            Self::Baked => "DELETE FROM bakedterrain WHERE RegionUUID = ?1",
        }
    }

    fn sqlite_insert(self) -> &'static str {
        match self {
            Self::Terrain => {
                "INSERT INTO terrain (RegionUUID, Revision, Heightfield) VALUES (?1, ?2, ?3)"
            }
            Self::Baked => {
                "INSERT INTO bakedterrain (RegionUUID, Revision, Heightfield) VALUES (?1, ?2, ?3)"
            }
        }
    }

    fn mysql_delete(self) -> &'static str {
        match self {
            Self::Terrain => "DELETE FROM terrain WHERE RegionUUID = ?",
            Self::Baked => "DELETE FROM bakedterrain WHERE RegionUUID = ?",
        }
    }

    fn mysql_insert(self) -> &'static str {
        match self {
            Self::Terrain => {
                "INSERT INTO terrain (RegionUUID, Revision, Heightfield) VALUES (?, ?, ?)"
            }
            Self::Baked => {
                "INSERT INTO bakedterrain (RegionUUID, Revision, Heightfield) VALUES (?, ?, ?)"
            }
        }
    }

    fn postgres_delete(self) -> &'static str {
        match self {
            Self::Terrain => r#"DELETE FROM terrain WHERE "RegionUUID" = $1"#,
            Self::Baked => r#"DELETE FROM bakedterrain WHERE "RegionUUID" = $1"#,
        }
    }

    fn postgres_insert(self) -> &'static str {
        match self {
            Self::Terrain => {
                r#"INSERT INTO terrain ("RegionUUID", "Revision", "Heightfield") VALUES ($1, $2, $3)"#
            }
            Self::Baked => {
                r#"INSERT INTO bakedterrain ("RegionUUID", "Revision", "Heightfield") VALUES ($1, $2, $3)"#
            }
        }
    }
}

/// What a select returned for one row: the revision, the cell's length in
/// bytes, and the cell when it is within the cap. NULL is `None`.
type RawRow = (Option<i64>, Option<i64>, Option<Vec<u8>>);

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

/// One row of `table`, by the backend's `LoadTerrain` or `LoadBakedTerrain`.
/// The cell is fetched only when its length is within [`BLOB_CAP`]; a larger
/// one is refused through `terrain.blob` before sqlx loads it.
async fn read_row(
    pool: &Pool,
    table: Table,
    region: Uuid,
    width: i32,
    height: i32,
) -> Result<Option<Heightmap>, TerrainError> {
    read_row_capped(pool, table, region, width, height, BLOB_CAP).await
}

async fn read_row_capped(
    pool: &Pool,
    table: Table,
    region: Uuid,
    width: i32,
    height: i32,
    cap: usize,
) -> Result<Option<Heightmap>, TerrainError> {
    let cap_bind = i64::try_from(cap).unwrap_or(i64::MAX);
    let failed = |error: sqlx::Error| TerrainError::Store(error.to_string());
    // SQLite's reader casts the cell to `byte[]`, so a NULL cell throws in
    // C# (TB88): it is damaged here. MySQL and PostgreSQL test the cell for
    // `DBNull` and load no map, so a NULL cell is no row there.
    let (row, null_is_absent) = if let Some(sqlite) = pool.sqlite() {
        let row: Option<RawRow> = sqlx::query_as(table.sqlite_select())
            .bind(region.as_hyphenated().to_string())
            .bind(cap_bind)
            .fetch_optional(sqlite)
            .await
            .map_err(failed)?;
        (row, false)
    } else if let Some(postgres) = pool.postgres() {
        let row: Option<RawRow> = sqlx::query_as(table.postgres_select())
            .bind(region)
            .bind(cap_bind)
            .fetch_optional(postgres)
            .await
            .map_err(failed)?;
        (row, true)
    } else if pool.mysql().is_some() {
        (read_mysql_row(pool, table, region, cap_bind).await?, true)
    } else {
        return Err(TerrainError::Store(
            "connection is not SQLite, MySQL or PostgreSQL".to_owned(),
        ));
    };
    let Some((revision, length, blob)) = row else {
        return Ok(None);
    };
    // `Convert.ToInt32` of a NULL revision throws on every backend.
    let Some(revision) = revision else {
        return Err(TerrainError::Damaged);
    };
    let Some(length) = length else {
        return if null_is_absent {
            Ok(None)
        } else {
            Err(TerrainError::Damaged)
        };
    };
    let Some(blob) = blob else {
        refuse_blob(usize::try_from(length).unwrap_or(usize::MAX));
        return Err(TerrainError::Blob);
    };
    let revision = i32::try_from(revision).map_err(|_| TerrainError::Revision(i32::MIN))?;
    Ok(Some(Heightmap::from_blob(revision, &blob, width, height)?))
}

/// `MySQLSimulationData.LoadTerrain` keeps the highest revision.
/// `LoadBakedTerrain` reads every row with no order: the revision is the
/// last row's, and the cell the last one that is not NULL. The rows are
/// streamed without their cells (constant memory), then that one cell is
/// read at its offset in the same unordered scan.
async fn read_mysql_row(
    pool: &Pool,
    table: Table,
    region: Uuid,
    cap: i64,
) -> Result<Option<RawRow>, TerrainError> {
    use futures_util::TryStreamExt;

    let Some(mysql) = pool.mysql() else {
        return Ok(None);
    };
    let failed = |error: sqlx::Error| TerrainError::Store(error.to_string());
    let id = region.as_hyphenated().to_string();
    if table == Table::Terrain {
        return sqlx::query_as(
            "SELECT CAST(Revision AS SIGNED), CAST(LENGTH(Heightfield) AS SIGNED), CASE WHEN LENGTH(Heightfield) <= ? THEN Heightfield END FROM terrain WHERE RegionUUID = ? ORDER BY Revision DESC LIMIT 1",
        )
        .bind(cap)
        .bind(&id)
        .fetch_optional(mysql)
        .await
        .map_err(failed);
    }
    let mut rows = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT CAST(Revision AS SIGNED), CAST(LENGTH(Heightfield) AS SIGNED) FROM bakedterrain WHERE RegionUUID = ?",
    )
    .bind(&id)
    .fetch(mysql);
    let mut last: Option<Option<i64>> = None;
    let mut cell: Option<(u64, i64)> = None;
    let mut index = 0u64;
    while let Some((revision, length)) = rows.try_next().await.map_err(failed)? {
        // `Convert.ToInt32` throws on the first NULL revision.
        if revision.is_none() {
            return Ok(Some((None, length, None)));
        }
        last = Some(revision);
        if let Some(length) = length {
            cell = Some((index, length));
        }
        index += 1;
    }
    drop(rows);
    let Some(revision) = last else {
        return Ok(None);
    };
    let Some((offset, length)) = cell else {
        return Ok(Some((revision, None, None)));
    };
    if length > cap {
        return Ok(Some((revision, Some(length), None)));
    }
    let blob: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
        "SELECT Heightfield FROM bakedterrain WHERE RegionUUID = ? LIMIT 1 OFFSET ?",
    )
    .bind(&id)
    .bind(offset)
    .fetch_optional(mysql)
    .await
    .map_err(failed)?;
    match blob.and_then(|(blob,)| blob) {
        Some(blob) if i64::try_from(blob.len()).unwrap_or(i64::MAX) <= cap => {
            Ok(Some((revision, Some(length), Some(blob))))
        }
        // The rows changed between the two reads.
        _ => Err(TerrainError::Store(
            "bakedterrain rows changed while they were read".to_owned(),
        )),
    }
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

/// Delete then insert, as every backend's `StoreTerrain` and
/// `StoreBakedTerrain` do, in one transaction. MySQL keys the row by the id
/// string, PostgreSQL by a `uuid` column.
async fn store(
    pool: &Pool,
    table: Table,
    region: Uuid,
    map: &Heightmap,
) -> Result<(), TerrainError> {
    let (revision, blob) = map.to_blob()?;
    let id = region.as_hyphenated().to_string();
    let failed = |error: sqlx::Error| TerrainError::Store(format!("{}: {error}", table.name()));
    if let Some(sqlite) = pool.sqlite() {
        let mut tx = sqlite.begin().await.map_err(failed)?;
        sqlx::query(table.sqlite_delete())
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        sqlx::query(table.sqlite_insert())
            .bind(&id)
            .bind(i64::from(revision))
            .bind(blob)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        return tx.commit().await.map_err(failed);
    }
    if let Some(mysql) = pool.mysql() {
        let mut tx = mysql.begin().await.map_err(failed)?;
        sqlx::query(table.mysql_delete())
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        sqlx::query(table.mysql_insert())
            .bind(&id)
            .bind(revision)
            .bind(blob)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        return tx.commit().await.map_err(failed);
    }
    if let Some(postgres) = pool.postgres() {
        let mut tx = postgres.begin().await.map_err(failed)?;
        sqlx::query(table.postgres_delete())
            .bind(region)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        sqlx::query(table.postgres_insert())
            .bind(region)
            .bind(revision)
            .bind(blob)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        return tx.commit().await.map_err(failed);
    }
    Err(TerrainError::Store(
        "connection is not SQLite, MySQL or PostgreSQL".to_owned(),
    ))
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
/// pinned migration, and run [`world_map`]. `kind` is the configured
/// `StorageProvider`. A SQLite file that does not exist gets the initial
/// terrain in memory and nothing is written; Ferroan does not create a store.
pub async fn load_world_map(
    connection: &str,
    kind: ObjectStoreKind,
    region: Uuid,
    width: i32,
    height: i32,
    initial: InitialTerrain,
) -> Result<WorldMap, TerrainError> {
    let backend = match kind {
        ObjectStoreKind::Sqlite => Backend::Sqlite,
        ObjectStoreKind::Mysql => Backend::Mysql,
        ObjectStoreKind::Postgres => Backend::Postgres,
    };
    if kind == ObjectStoreKind::Sqlite {
        let path = ferroan_db::sqlite_filename(connection)
            .map_err(|error| TerrainError::Store(error.to_string()))?;
        if !path.exists() {
            return Ok(WorldMap {
                map: initial.generate(width, height)?,
                source: TerrainSource::GeneratedUnstored,
                stored_baked: false,
            });
        }
    }
    let pool = Pool::open(backend, connection, &["RegionStore"])
        .await
        .map_err(|error| TerrainError::Store(error.to_string()))?;
    let loaded = world_map(&pool, region, width, height, initial).await;
    pool.close().await;
    loaded
}

/// Read a SQLite store's `terrain` row without writing anything (the
/// harness reads the home store after a run with it). `None` means the file
/// is absent, the table has no row for this region, or `kind` is not SQLite;
/// the region itself loads every backend through [`load_world_map`].
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
    let result = read_terrain(&pool, region, width, height).await;
    pool.close().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_store::CopiedStore;
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
        let store = CopiedStore::new("db/sqlite/RegionStore.db");
        let connection = store.connection();
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
        pool.close().await;
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
        let first = load_world_map(
            &connection,
            ObjectStoreKind::Sqlite,
            REGION,
            256,
            256,
            InitialTerrain::PinheadIsland,
        )
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
        let second = load_world_map(
            &connection,
            ObjectStoreKind::Sqlite,
            REGION,
            256,
            256,
            InitialTerrain::Flat,
        )
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
        let loaded = load_world_map(
            &connection,
            ObjectStoreKind::Sqlite,
            REGION,
            256,
            256,
            InitialTerrain::PinheadIsland,
        )
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
        let loaded = load_world_map(
            &connection,
            ObjectStoreKind::Sqlite,
            REGION,
            256,
            256,
            InitialTerrain::PinheadIsland,
        )
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

    /// The C# row for a new 256 m region (`fixtures/walk/terrain.bin`).
    fn csharp_island() -> Heightmap {
        Heightmap::from_blob(
            23,
            &std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/walk/terrain.bin"),
            )
            .unwrap(),
            256,
            256,
        )
        .unwrap()
    }

    /// One server backend under `cargo xtask regionstore cross`: the
    /// connection, its C# form for the oracle, and the oracle command prefix.
    struct Cross {
        kind: ObjectStoreKind,
        connection: String,
        ado: String,
        oracle: String,
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
                "mysql",
            ),
            ObjectStoreKind::Postgres => (
                "FERROAN_REGIONSTORE_POSTGRES",
                "FERROAN_REGIONSTORE_POSTGRES_ADO",
                "postgres",
            ),
            ObjectStoreKind::Sqlite => return None,
        };
        Some(Cross {
            kind,
            connection: std::env::var(url).ok()?,
            ado: std::env::var(ado).ok()?,
            oracle: std::env::var("FERROAN_REGIONSTORE_ORACLE").ok()?,
            prefix,
        })
    }

    fn oracle(cross: &Cross, command: &str, args: &[&str]) -> Vec<u8> {
        let output = std::process::Command::new("dotnet")
            .arg(&cross.oracle)
            .arg(format!("{}-{command}", cross.prefix))
            .args(args)
            .env("MSBUILDDISABLENODEREUSE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    async fn open(cross: &Cross) -> Pool {
        let backend = match cross.kind {
            ObjectStoreKind::Mysql => Backend::Mysql,
            _ => Backend::Postgres,
        };
        Pool::open(backend, &cross.connection, &["RegionStore"])
            .await
            .unwrap()
    }

    /// A row as a test writes it: any revision, any cell, or NULL.
    async fn raw_insert(
        pool: &Pool,
        table: Table,
        region: Uuid,
        revision: Option<i32>,
        blob: Option<Vec<u8>>,
    ) {
        if let Some(mysql) = pool.mysql() {
            sqlx::query(table.mysql_insert())
                .bind(region.as_hyphenated().to_string())
                .bind(revision)
                .bind(blob)
                .execute(mysql)
                .await
                .unwrap();
        } else {
            sqlx::query(table.postgres_insert())
                .bind(region)
                .bind(revision)
                .bind(blob)
                .execute(pool.postgres().unwrap())
                .await
                .unwrap();
        }
    }

    async fn clear(pool: &Pool, region: Uuid) {
        for table in [Table::Terrain, Table::Baked] {
            if let Some(mysql) = pool.mysql() {
                sqlx::query(table.mysql_delete())
                    .bind(region.as_hyphenated().to_string())
                    .execute(mysql)
                    .await
                    .unwrap();
            } else {
                sqlx::query(table.postgres_delete())
                    .bind(region)
                    .execute(pool.postgres().unwrap())
                    .await
                    .unwrap();
            }
        }
    }

    async fn count(pool: &Pool, table: Table, region: Uuid) -> i64 {
        if let Some(mysql) = pool.mysql() {
            sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {} WHERE RegionUUID = ? AND Heightfield IS NOT NULL",
                table.name()
            ))
            .bind(region.as_hyphenated().to_string())
            .fetch_one(mysql)
            .await
            .unwrap()
        } else {
            sqlx::query_scalar(&format!(
                r#"SELECT COUNT(*) FROM {} WHERE "RegionUUID" = $1 AND "Heightfield" IS NOT NULL"#,
                table.name()
            ))
            .bind(region)
            .fetch_one(pool.postgres().unwrap())
            .await
            .unwrap()
        }
    }

    fn region(n: u128) -> Uuid {
        Uuid::from_u128(0x7e44_0000_0000_4000_8000_0000_0000_0000 | n)
    }

    /// The SQLite cases on a server backend, the oracle reading Ferroan's
    /// rows, and Ferroan reading the rows the pin stores.
    async fn server_terrain_cases(cross: Cross) {
        let pin = ferroan_db::TRANQUILLITY_REVISION;
        let island = csharp_island();
        let flat = InitialTerrain::Flat.generate(256, 256).unwrap();
        let pool = open(&cross).await;

        // An empty store gets the island as both rows, and the pin loads them.
        let empty = region(1);
        clear(&pool, empty).await;
        let first = load_world_map(
            &cross.connection,
            cross.kind,
            empty,
            256,
            256,
            InitialTerrain::PinheadIsland,
        )
        .await
        .unwrap();
        assert_eq!(first.source, TerrainSource::Generated);
        assert!(first.stored_baked);
        let terrain = read_terrain(&pool, empty, 256, 256).await.unwrap().unwrap();
        let baked = read_baked_terrain(&pool, empty, 256, 256)
            .await
            .unwrap()
            .unwrap();
        assert!(same_cells(&terrain, &island));
        assert!(same_cells(&baked, &island));
        let dump: serde_json::Value = serde_json::from_slice(&oracle(
            &cross,
            "terrain-load",
            &[&cross.ado, &empty.as_hyphenated().to_string(), pin],
        ))
        .unwrap();
        let expected: Vec<u64> = island
            .cells()
            .iter()
            .map(|h| u64::from(h.to_bits()))
            .collect();
        for table in ["terrain", "bakedterrain"] {
            let row = &dump[table];
            assert_eq!(row["stored_revision"], 23, "{} {table}", cross.prefix);
            let cells: Vec<u64> = row["cells"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| cell.as_u64().unwrap())
                .collect();
            assert!(
                cells == expected,
                "{} {table}: the pin loads Ferroan's row",
                cross.prefix
            );
            eprintln!(
                "{} {table}: the pin loads {} cells; stored {} gzip bytes, the pin writes {}",
                cross.prefix,
                cells.len(),
                row["stored_bytes"],
                row["csharp_bytes"]
            );
        }
        let second = load_world_map(
            &cross.connection,
            cross.kind,
            empty,
            256,
            256,
            InitialTerrain::Flat,
        )
        .await
        .unwrap();
        assert_eq!(second.source, TerrainSource::Stored);
        assert!(!second.stored_baked);
        assert_eq!(count(&pool, Table::Terrain, empty).await, 1);
        assert_eq!(count(&pool, Table::Baked, empty).await, 1);

        // A terrain row without a baked row gets the baked row.
        let terrain_only = region(2);
        clear(&pool, terrain_only).await;
        store_terrain(&pool, terrain_only, &flat).await.unwrap();
        let loaded = world_map(&pool, terrain_only, 256, 256, InitialTerrain::PinheadIsland)
            .await
            .unwrap();
        assert_eq!(loaded.source, TerrainSource::Stored);
        assert!(loaded.stored_baked);
        let baked = read_baked_terrain(&pool, terrain_only, 256, 256)
            .await
            .unwrap()
            .unwrap();
        assert!(same_cells(&baked, &flat));

        // A baked row without a terrain row is copied; the baked row stays.
        let baked_only = region(3);
        clear(&pool, baked_only).await;
        store_baked_terrain(&pool, baked_only, &flat).await.unwrap();
        let loaded = world_map(&pool, baked_only, 256, 256, InitialTerrain::PinheadIsland)
            .await
            .unwrap();
        assert_eq!(loaded.source, TerrainSource::Baked);
        assert!(!loaded.stored_baked);
        let terrain = read_terrain(&pool, baked_only, 256, 256)
            .await
            .unwrap()
            .unwrap();
        assert!(same_cells(&terrain, &flat));
        assert_eq!(count(&pool, Table::Baked, baked_only).await, 1);

        // The rows the pin stores load to its island, and nothing is written.
        let csharp = region(4);
        clear(&pool, csharp).await;
        oracle(
            &cross,
            "terrain-seed",
            &[&cross.ado, &csharp.as_hyphenated().to_string()],
        );
        let loaded = world_map(&pool, csharp, 256, 256, InitialTerrain::Flat)
            .await
            .unwrap();
        assert_eq!(loaded.source, TerrainSource::Stored);
        assert!(!loaded.stored_baked);
        assert!(same_cells(&loaded.map, &island));

        // A NULL cell is no row on MySQL and PostgreSQL: the island replaces it.
        let null_cell = region(5);
        clear(&pool, null_cell).await;
        raw_insert(&pool, Table::Terrain, null_cell, Some(23), None).await;
        assert!(
            read_terrain(&pool, null_cell, 256, 256)
                .await
                .unwrap()
                .is_none()
        );
        let loaded = world_map(&pool, null_cell, 256, 256, InitialTerrain::PinheadIsland)
            .await
            .unwrap();
        assert_eq!(loaded.source, TerrainSource::Generated);
        assert_eq!(count(&pool, Table::Terrain, null_cell).await, 1);

        // A NULL revision is damaged, and a cell over the cap is refused
        // before it is read.
        let bounded = region(6);
        clear(&pool, bounded).await;
        raw_insert(&pool, Table::Baked, bounded, None, Some(vec![0; 16])).await;
        assert!(matches!(
            read_baked_terrain(&pool, bounded, 256, 256).await,
            Err(TerrainError::Damaged)
        ));
        store_terrain(&pool, bounded, &flat).await.unwrap();
        assert!(matches!(
            read_row_capped(&pool, Table::Terrain, bounded, 256, 256, 100).await,
            Err(TerrainError::Blob)
        ));
        assert!(
            read_row_capped(&pool, Table::Terrain, bounded, 256, 256, BLOB_CAP)
                .await
                .unwrap()
                .is_some()
        );
        for n in 1..=6 {
            clear(&pool, region(n)).await;
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn regionstore_mysql_terrain_matches_the_pin() {
        let Some(cross) = cross(ObjectStoreKind::Mysql) else {
            eprintln!("regionstore mysql terrain skipped");
            return;
        };
        server_terrain_cases(cross).await;
    }

    #[tokio::test]
    async fn regionstore_postgres_terrain_matches_the_pin() {
        let Some(cross) = cross(ObjectStoreKind::Postgres) else {
            eprintln!("regionstore postgres terrain skipped");
            return;
        };
        server_terrain_cases(cross).await;
    }

    /// `MySQLSimulationData.LoadBakedTerrain` keeps the last row's revision
    /// and the last cell that is not NULL.
    #[tokio::test]
    async fn regionstore_mysql_baked_terrain_is_the_last_row() {
        let Some(cross) = cross(ObjectStoreKind::Mysql) else {
            eprintln!("regionstore mysql baked rows skipped");
            return;
        };
        let pool = open(&cross).await;
        let rows = region(7);
        clear(&pool, rows).await;
        let flat = InitialTerrain::Flat.generate(256, 256).unwrap();
        let island = InitialTerrain::PinheadIsland.generate(256, 256).unwrap();
        let (_, flat_blob) = flat.to_blob().unwrap();
        let (_, island_blob) = island.to_blob().unwrap();
        raw_insert(&pool, Table::Baked, rows, Some(23), Some(flat_blob.clone())).await;
        raw_insert(&pool, Table::Baked, rows, Some(23), Some(island_blob)).await;
        let baked = read_baked_terrain(&pool, rows, 256, 256)
            .await
            .unwrap()
            .unwrap();
        assert!(same_cells(&baked, &csharp_island()));
        clear(&pool, rows).await;
        raw_insert(&pool, Table::Baked, rows, Some(23), Some(flat_blob)).await;
        raw_insert(&pool, Table::Baked, rows, Some(23), None).await;
        let baked = read_baked_terrain(&pool, rows, 256, 256)
            .await
            .unwrap()
            .unwrap();
        assert!(same_cells(&baked, &flat));
        clear(&pool, rows).await;
        raw_insert(&pool, Table::Baked, rows, Some(23), None).await;
        assert!(
            read_baked_terrain(&pool, rows, 256, 256)
                .await
                .unwrap()
                .is_none()
        );
        clear(&pool, rows).await;
        pool.close().await;
    }
}
