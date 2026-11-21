// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The newest `terrain` row for one region.
//!
//! `SQLiteSimulationData.LoadTerrain` (`Source/OpenSim.Data.SQLite/SQLiteSimulationData.cs`)
//! selects `RegionUUID, Revision, Heightfield` where the id matches, ordered by
//! `Revision` descending, and keeps the first row. The id string is
//! `UUID.ToString()`, the lowercase hyphenated form.

use ferroan_db::{Backend, Pool};
use uuid::Uuid;

use crate::error::TerrainError;
use crate::height::Heightmap;

/// `None` when the store has no terrain row. A damaged blob is an error.
pub async fn read_terrain(
    pool: &Pool,
    region: Uuid,
    width: i32,
    height: i32,
) -> Result<Option<Heightmap>, TerrainError> {
    let Some(sqlite) = pool.sqlite() else {
        return Err(TerrainError::Store("connection is not SQLite".to_owned()));
    };
    let row: Option<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT Revision, Heightfield FROM terrain WHERE RegionUUID = ?1 ORDER BY Revision DESC LIMIT 1",
    )
    .bind(region.as_hyphenated().to_string())
    .fetch_optional(sqlite)
    .await
    .map_err(|error| TerrainError::Store(error.to_string()))?;
    let Some((revision, blob)) = row else {
        return Ok(None);
    };
    let revision = i32::try_from(revision).map_err(|_| TerrainError::Revision(i32::MIN))?;
    Ok(Some(Heightmap::from_blob(revision, &blob, width, height)?))
}

/// Open the region store, require `RegionStore` at the pinned migration, and
/// read its terrain. `None` means the file is absent or the table has no row
/// for this region. `Standalone.ini` names `URI=file:OpenSim.db`, which C#
/// creates; this row does not, so a missing file is no saved heightmap.
pub async fn open_terrain(
    connection: &str,
    region: Uuid,
    width: i32,
    height: i32,
) -> Result<Option<Heightmap>, TerrainError> {
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
        let map = open_terrain(&connection, Uuid::nil(), 256, 256)
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
}
