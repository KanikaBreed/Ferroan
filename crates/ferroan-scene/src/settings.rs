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
// Ported to Rust for Ferroan from SQLiteSimulationData.LoadRegionSettings,
// StoreRegionSettings, buildRegionSettings, LoadSpawnPoints, RegionSettings
// and Source/OpenSim.Region.Framework/Scenes/Scene.cs (terrain PBR defaults) at
// OpenSim-Tranquillity `dev-build` @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Region settings and spawn points from a SQLite region store.
//!
//! A missing file leaves the caller on fresh handshake defaults. In an existing
//! store, first open saves a settings row and its cache id. A zero classic
//! texture takes the shipped default. An entirely empty PBR set inherits the
//! four resolved classic textures and is saved for subsequent opens.
//! Elevations clamp to −100..4000, water to 0..8000, the raise limit to 0..200
//! and the lower limit to −200..0. Spawn points stay in insertion order, as
//! pitch, yaw and distance relative to the telehub. MySQL and PostgreSQL are
//! left unread. A null or unreadable cell keeps the fresh default and the
//! region still loads (TB70, D53). A terrain save persists a missing cache id.
//! Otherwise a null cache id leaves the handshake on its existing id.

use ferroan_db::{Backend, Pool};
use sqlx::Row;
use uuid::Uuid;

use crate::ObjectError;
use crate::objects::ObjectStoreKind;

const MIN_TERRAIN: f64 = -100.0;
const MAX_TERRAIN: f64 = 4000.0;
const MIN_WATER: f64 = 0.0;
const MAX_WATER: f64 = 8000.0;
const DEFAULT_TEXTURES: [u128; 4] = [
    0xb8d3_965a_ad78_bf43_699b_bff8_eca6_c975,
    0xabb7_83e6_3e93_26c0_248a_2476_6685_5da3,
    0x179c_dabd_398a_9b6b_1391_4dc3_33ba_321f,
    0xbeb1_69c7_11ea_fff2_efe5_0f24_dc88_1df2,
];

/// One spawn offset. Pitch, yaw and distance are relative to the telehub.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnPoint {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

/// One region's stored settings after the same clamps and texture substitution
/// a successful load applies. FixedSun is inert. The legacy sun position and
/// vector columns are retained as data and have no handshake fields.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionSettings {
    pub region_id: Uuid,
    pub block_terraform: bool,
    pub block_fly: bool,
    pub allow_damage: bool,
    pub restrict_pushing: bool,
    pub allow_land_resell: bool,
    pub allow_land_join_divide: bool,
    pub block_show_in_search: bool,
    pub agent_limit: i32,
    pub object_bonus: f64,
    pub maturity: i32,
    pub disable_scripts: bool,
    pub disable_collisions: bool,
    pub disable_physics: bool,
    pub terrain_textures: [Uuid; 4],
    pub terrain_pbr: [Uuid; 4],
    /// Elevation1 in wire order: SW, NW, SE, NE.
    pub terrain_start_heights: [f64; 4],
    /// Elevation2 in the same order. It is not Elevation2 minus Elevation1.
    pub terrain_height_ranges: [f64; 4],
    pub water_height: f64,
    pub terrain_raise_limit: f64,
    pub terrain_lower_limit: f64,
    pub use_estate_sun: bool,
    pub sandbox: bool,
    pub sun_vector: [f64; 3],
    pub fixed_sun: bool,
    pub sun_position: f64,
    pub covenant: Uuid,
    pub covenant_datetime: i32,
    pub map_tile_id: Uuid,
    pub telehub_object: Uuid,
    pub parcel_tile_id: Uuid,
    pub god_block_search: bool,
    pub casino: bool,
    /// A terrain save initializes a null or unreadable cell. Otherwise the
    /// caller keeps its id when this is `None`.
    pub cache_id: Option<Uuid>,
    pub spawn_points: Vec<SpawnPoint>,
}

/// Where the settings came from. `OtherDatabase` means the connection is MySQL
/// or PostgreSQL and the caller keeps fresh defaults.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsSource {
    Sqlite(Option<Box<RegionSettings>>),
    OtherDatabase,
}

/// Open the region store and load one region's settings. A missing SQLite file
/// gives `Sqlite(None)`. A missing row is initialized. A MySQL or PostgreSQL provider is
/// `OtherDatabase` and is not opened. A SQLite store that cannot be read is an error.
pub async fn open_region_settings(
    connection: &str,
    region: Uuid,
    kind: ObjectStoreKind,
) -> Result<SettingsSource, ObjectError> {
    if kind != ObjectStoreKind::Sqlite {
        return Ok(SettingsSource::OtherDatabase);
    }
    let path = ferroan_db::sqlite_filename(connection)
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    if !path.exists() {
        return Ok(SettingsSource::Sqlite(None));
    }
    let pool = Pool::open(Backend::Sqlite, connection, &["RegionStore"])
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    let result = read_region_settings(&pool, region).await;
    pool.close().await;
    result
}

async fn read_region_settings(pool: &Pool, region: Uuid) -> Result<SettingsSource, ObjectError> {
    let Some(sqlite) = pool.sqlite() else {
        return Err(ObjectError::Store("connection is not SQLite".to_owned()));
    };
    let wanted = region.as_hyphenated().to_string();
    let stored = sqlx::query(SETTINGS_SQL)
        .bind(&wanted)
        .fetch_optional(sqlite)
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    let row = if let Some(row) = stored {
        row
    } else {
        insert_default_settings(sqlite, &wanted).await?
    };
    let settings = resolve_terrain_settings(sqlite, read_settings_row(&row, region)?).await?;
    let spawns = sqlx::query(SPAWN_SQL)
        .bind(&wanted)
        .fetch_all(sqlite)
        .await
        .map_err(|error| ObjectError::Store(error.to_string()))?;
    let mut spawn_points = Vec::with_capacity(spawns.len());
    for spawn in &spawns {
        spawn_points.push(read_spawn(spawn)?);
    }
    Ok(SettingsSource::Sqlite(Some(Box::new(RegionSettings {
        spawn_points,
        ..settings
    }))))
}

fn needs_terrain_save(settings: &RegionSettings) -> bool {
    settings.terrain_textures.iter().any(Uuid::is_nil)
        || settings.terrain_pbr.iter().all(Uuid::is_nil)
}

async fn resolve_terrain_settings(
    sqlite: &sqlx::SqlitePool,
    stored: RegionSettings,
) -> Result<RegionSettings, ObjectError> {
    if !needs_terrain_save(&stored) {
        return Ok(stored);
    }

    // Another opener or editor may have repaired the row since the first read.
    let region = stored.region_id;
    let wanted = region.to_string();
    let mut tx = sqlite
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(store_error)?;
    let row = sqlx::query(SETTINGS_SQL)
        .bind(&wanted)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
    let mut current = read_settings_row(&row, region)?;
    if needs_terrain_save(&current) {
        for (id, default) in current.terrain_textures.iter_mut().zip(DEFAULT_TEXTURES) {
            if id.is_nil() {
                *id = Uuid::from_u128(default);
            }
        }
        let pbr_columns = if current.terrain_pbr.iter().all(Uuid::is_nil) {
            current.terrain_pbr = current.terrain_textures;
            ", TerrainPBR1=?1, TerrainPBR2=?2, TerrainPBR3=?3, TerrainPBR4=?4"
        } else {
            ""
        };
        let update = format!("UPDATE regionsettings SET terrain_texture_1=?1, terrain_texture_2=?2,
            terrain_texture_3=?3, terrain_texture_4=?4, cacheID=?5{pbr_columns} WHERE regionUUID=?6");
        let ids = current.terrain_textures.map(|id| id.to_string());
        let cache_id = current.cache_id.unwrap_or_else(Uuid::new_v4);
        sqlx::query(&update)
            .bind(&ids[0])
            .bind(&ids[1])
            .bind(&ids[2])
            .bind(&ids[3])
            .bind(cache_id.to_string())
            .bind(&wanted)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        current.cache_id = Some(cache_id);
    }
    tx.commit().await.map_err(store_error)?;
    Ok(current)
}

async fn insert_default_settings(
    sqlite: &sqlx::SqlitePool,
    wanted: &str,
) -> Result<sqlx::sqlite::SqliteRow, ObjectError> {
    // Concurrent first opens must agree on the id chosen by the winner.
    let mut tx = sqlite.begin().await.map_err(store_error)?;
    let textures = DEFAULT_TEXTURES.map(|id| Uuid::from_u128(id).to_string());
    let inserted = sqlx::query(INSERT_DEFAULTS)
        .bind(wanted)
        .bind(Uuid::new_v4().to_string())
        .bind(&textures[0])
        .bind(&textures[1])
        .bind(&textures[2])
        .bind(&textures[3])
        .bind(Uuid::nil().to_string())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
    if inserted.rows_affected() == 1 {
        sqlx::query("DELETE FROM spawn_points WHERE RegionID = ?")
            .bind(wanted)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
    }
    let row = sqlx::query(SETTINGS_SQL)
        .bind(wanted)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
    tx.commit().await.map_err(store_error)?;
    Ok(row)
}

fn store_error(error: sqlx::Error) -> ObjectError {
    ObjectError::Store(error.to_string())
}

// RegionSettings defaults after scene startup, including resolved terrain textures.
const INSERT_DEFAULTS: &str = "INSERT INTO regionsettings (
regionUUID, cacheID, block_terraform, block_fly, allow_damage, restrict_pushing,
allow_land_resell, allow_land_join_divide, block_show_in_search, agent_limit, object_bonus,
maturity, disable_scripts, disable_collisions, disable_physics,
terrain_texture_1, terrain_texture_2, terrain_texture_3, terrain_texture_4,
TerrainPBR1, TerrainPBR2, TerrainPBR3, TerrainPBR4,
elevation_1_nw, elevation_2_nw, elevation_1_ne, elevation_2_ne,
elevation_1_se, elevation_2_se, elevation_1_sw, elevation_2_sw,
water_height, terrain_raise_limit, terrain_lower_limit, use_estate_sun,
fixed_sun, sun_position, sandbox, covenant)
VALUES (?1, ?2, 0,0,0,0, 1,1,0,40,1, 0,0,0,0, ?3,?4,?5,?6, ?3,?4,?5,?6,
10,60,10,60,10,60,10,60,20,100,-100,1,0,0,0,?7)
ON CONFLICT(regionUUID) DO NOTHING";

const SETTINGS_SQL: &str = "\
SELECT regionUUID AS region_uuid, block_terraform, block_fly, allow_damage, \
restrict_pushing, allow_land_resell, allow_land_join_divide, block_show_in_search, \
agent_limit, object_bonus, maturity, disable_scripts, disable_collisions, \
disable_physics, terrain_texture_1, terrain_texture_2, terrain_texture_3, \
terrain_texture_4, TerrainPBR1 AS terrain_pbr_1, TerrainPBR2 AS terrain_pbr_2, \
TerrainPBR3 AS terrain_pbr_3, TerrainPBR4 AS terrain_pbr_4, \
elevation_1_sw, elevation_1_nw, elevation_1_se, elevation_1_ne, \
elevation_2_sw, elevation_2_nw, elevation_2_se, elevation_2_ne, \
water_height, terrain_raise_limit, terrain_lower_limit, use_estate_sun, sandbox, \
sunvectorx, sunvectory, sunvectorz, fixed_sun, sun_position, covenant, \
covenant_datetime, map_tile_ID AS map_tile_id, TelehubObject AS telehub_object, \
parcel_tile_ID AS parcel_tile_id, block_search, casino, cacheID AS cache_id \
FROM regionsettings WHERE regionUUID = ?1 LIMIT 1";

const SPAWN_SQL: &str = "\
SELECT Yaw AS yaw, Pitch AS pitch, Distance AS distance \
FROM spawn_points WHERE RegionID = ?1 ORDER BY rowid";

fn read_settings_row(
    row: &sqlx::sqlite::SqliteRow,
    region: Uuid,
) -> Result<RegionSettings, ObjectError> {
    let textures = [
        uuid_cell(row, "terrain_texture_1")?,
        uuid_cell(row, "terrain_texture_2")?,
        uuid_cell(row, "terrain_texture_3")?,
        uuid_cell(row, "terrain_texture_4")?,
    ];
    let pbr = [
        uuid_cell(row, "terrain_pbr_1")?,
        uuid_cell(row, "terrain_pbr_2")?,
        uuid_cell(row, "terrain_pbr_3")?,
        uuid_cell(row, "terrain_pbr_4")?,
    ];
    Ok(RegionSettings {
        region_id: region,
        block_terraform: flag(row, "block_terraform", false)?,
        block_fly: flag(row, "block_fly", false)?,
        allow_damage: flag(row, "allow_damage", false)?,
        restrict_pushing: flag(row, "restrict_pushing", false)?,
        allow_land_resell: flag(row, "allow_land_resell", true)?,
        allow_land_join_divide: flag(row, "allow_land_join_divide", true)?,
        block_show_in_search: flag(row, "block_show_in_search", false)?,
        agent_limit: integer(row, "agent_limit", 40)?,
        object_bonus: number(row, "object_bonus", 1.0)?,
        maturity: integer(row, "maturity", 0)?,
        disable_scripts: flag(row, "disable_scripts", false)?,
        disable_collisions: flag(row, "disable_collisions", false)?,
        disable_physics: flag(row, "disable_physics", false)?,
        terrain_textures: textures,
        terrain_pbr: pbr,
        terrain_start_heights: [
            clamped(row, "elevation_1_sw", 10.0, MIN_TERRAIN, MAX_TERRAIN)?,
            clamped(row, "elevation_1_nw", 10.0, MIN_TERRAIN, MAX_TERRAIN)?,
            clamped(row, "elevation_1_se", 10.0, MIN_TERRAIN, MAX_TERRAIN)?,
            clamped(row, "elevation_1_ne", 10.0, MIN_TERRAIN, MAX_TERRAIN)?,
        ],
        terrain_height_ranges: [
            clamped(row, "elevation_2_sw", 60.0, MIN_TERRAIN, MAX_TERRAIN)?,
            clamped(row, "elevation_2_nw", 60.0, MIN_TERRAIN, MAX_TERRAIN)?,
            clamped(row, "elevation_2_se", 60.0, MIN_TERRAIN, MAX_TERRAIN)?,
            clamped(row, "elevation_2_ne", 60.0, MIN_TERRAIN, MAX_TERRAIN)?,
        ],
        water_height: clamped(row, "water_height", 20.0, MIN_WATER, MAX_WATER)?,
        terrain_raise_limit: clamped(row, "terrain_raise_limit", 100.0, 0.0, 200.0)?,
        terrain_lower_limit: clamped(row, "terrain_lower_limit", -100.0, -200.0, 0.0)?,
        use_estate_sun: flag(row, "use_estate_sun", true)?,
        sandbox: flag(row, "sandbox", false)?,
        sun_vector: [
            number(row, "sunvectorx", 0.0)?,
            number(row, "sunvectory", 0.0)?,
            number(row, "sunvectorz", 0.0)?,
        ],
        fixed_sun: false,
        sun_position: number(row, "sun_position", 0.0)?,
        covenant: uuid_cell(row, "covenant")?,
        covenant_datetime: integer(row, "covenant_datetime", 0)?,
        map_tile_id: uuid_cell(row, "map_tile_id")?,
        telehub_object: uuid_cell(row, "telehub_object")?,
        parcel_tile_id: uuid_cell(row, "parcel_tile_id")?,
        god_block_search: flag(row, "block_search", false)?,
        casino: flag(row, "casino", false)?,
        cache_id: cache_id(row)?,
        spawn_points: Vec::new(),
    })
}

fn read_spawn(row: &sqlx::sqlite::SqliteRow) -> Result<SpawnPoint, ObjectError> {
    Ok(SpawnPoint {
        yaw: finite_f32(number(row, "yaw", 0.0)?),
        pitch: finite_f32(number(row, "pitch", 0.0)?),
        distance: finite_f32(number(row, "distance", 0.0)?),
    })
}

fn finite_f32(value: f64) -> f32 {
    let narrow = value as f32;
    if narrow.is_finite() { narrow } else { 0.0 }
}

fn flag(row: &sqlx::sqlite::SqliteRow, name: &str, default: bool) -> Result<bool, ObjectError> {
    Ok(match cell(row, name)? {
        Cell::Missing => return Err(missing(name)),
        Cell::Null => {
            warn_default(name);
            default
        }
        Cell::Int(value) => value != 0,
        Cell::Float(value) => value != 0.0,
        Cell::Bool(value) => value,
        Cell::Text(text) if truthy(&text) => true,
        Cell::Text(text) if falsy(&text) => false,
        Cell::Text(_) => {
            warn_default(name);
            default
        }
    })
}

fn number(row: &sqlx::sqlite::SqliteRow, name: &str, default: f64) -> Result<f64, ObjectError> {
    Ok(match cell(row, name)? {
        Cell::Missing => return Err(missing(name)),
        Cell::Null => {
            warn_default(name);
            default
        }
        Cell::Int(value) => value as f64,
        Cell::Float(value) => value,
        Cell::Bool(value) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        Cell::Text(text) => text.trim().parse::<f64>().unwrap_or_else(|_| {
            warn_default(name);
            default
        }),
    })
}

fn clamped(
    row: &sqlx::sqlite::SqliteRow,
    name: &str,
    default: f64,
    min: f64,
    max: f64,
) -> Result<f64, ObjectError> {
    Ok(number(row, name, default)?.clamp(min, max))
}

fn integer(row: &sqlx::sqlite::SqliteRow, name: &str, default: i32) -> Result<i32, ObjectError> {
    let value = number(row, name, f64::from(default))?;
    if value.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value) {
        Ok(value as i32)
    } else {
        warn_default(name);
        Ok(default)
    }
}

fn uuid_cell(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<Uuid, ObjectError> {
    Ok(match cell(row, name)? {
        Cell::Missing => return Err(missing(name)),
        Cell::Text(text) => match text.trim().parse::<Uuid>() {
            Ok(id) => id,
            Err(_) => {
                warn_default(name);
                Uuid::nil()
            }
        },
        Cell::Null | Cell::Int(_) | Cell::Float(_) | Cell::Bool(_) => {
            warn_default(name);
            Uuid::nil()
        }
    })
}

fn cache_id(row: &sqlx::sqlite::SqliteRow) -> Result<Option<Uuid>, ObjectError> {
    Ok(match cell(row, "cache_id")? {
        Cell::Missing => return Err(missing("cacheID")),
        Cell::Null => None,
        Cell::Text(text) => match text.trim().parse::<Uuid>() {
            Ok(id) => Some(id),
            Err(_) => {
                log::warn!("region settings cacheID could not be read");
                None
            }
        },
        Cell::Int(_) | Cell::Float(_) | Cell::Bool(_) => {
            log::warn!("region settings cacheID could not be read");
            None
        }
    })
}

enum Cell {
    Missing,
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

fn cell(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<Cell, ObjectError> {
    match row.try_get::<Option<i64>, _>(name) {
        Ok(Some(value)) => return Ok(Cell::Int(value)),
        Ok(None) => return Ok(Cell::Null),
        Err(error) if missing_column(&error) => return Ok(Cell::Missing),
        Err(_) => {}
    }
    match row.try_get::<Option<f64>, _>(name) {
        Ok(Some(value)) => return Ok(Cell::Float(value)),
        Ok(None) => return Ok(Cell::Null),
        Err(error) if missing_column(&error) => return Ok(Cell::Missing),
        Err(_) => {}
    }
    match row.try_get::<Option<bool>, _>(name) {
        Ok(Some(value)) => return Ok(Cell::Bool(value)),
        Ok(None) => return Ok(Cell::Null),
        Err(error) if missing_column(&error) => return Ok(Cell::Missing),
        Err(_) => {}
    }
    match row.try_get::<Option<String>, _>(name) {
        Ok(Some(value)) => Ok(Cell::Text(value)),
        Ok(None) => Ok(Cell::Null),
        Err(error) if missing_column(&error) => Ok(Cell::Missing),
        Err(error) => Err(ObjectError::Store(format!("{name}: {error}"))),
    }
}

fn missing_column(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::ColumnNotFound(_))
}

fn missing(name: &str) -> ObjectError {
    ObjectError::Store(format!("region settings column {name} is missing"))
}

fn warn_default(name: &str) {
    log::warn!("region settings column {name} could not be read; the load keeps the fresh default");
}

fn truthy(text: &str) -> bool {
    text.eq_ignore_ascii_case("true") || text == "1"
}

fn falsy(text: &str) -> bool {
    text.eq_ignore_ascii_case("false") || text == "0"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::ObjectStoreKind;
    use crate::test_store::CopiedStore;

    const REGION: &str = "11111111-1111-1111-1111-111111111111";

    fn dump() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/regionstore/settings.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn loaded(source: SettingsSource) -> RegionSettings {
        match source {
            SettingsSource::Sqlite(Some(settings)) => *settings,
            other => panic!("expected a settings row, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn regionstore_settings_match_the_committed_dump() {
        let dump = dump();
        assert_eq!(dump["pin"], "c401bceecd1684832ae8ba86a31b5ee01a4ded7b");
        assert_eq!(dump["region"], REGION);
        let store = CopiedStore::new("regionstore/settings.db");
        let connection = store.connection();
        let region = Uuid::parse_str(REGION).unwrap();
        let settings = loaded(
            open_region_settings(&connection, region, ObjectStoreKind::Sqlite)
                .await
                .unwrap(),
        );
        assert_eq!(settings.region_id, region);
        assert_eq!(
            settings.allow_damage,
            dump["allow_damage"].as_bool().unwrap()
        );
        assert_eq!(
            settings.block_terraform,
            dump["block_terraform"].as_bool().unwrap()
        );
        assert_eq!(settings.block_fly, dump["block_fly"].as_bool().unwrap());
        assert_eq!(
            settings.restrict_pushing,
            dump["restrict_pushing"].as_bool().unwrap()
        );
        assert_eq!(
            settings.allow_land_resell,
            dump["allow_land_resell"].as_bool().unwrap()
        );
        assert_eq!(
            settings.allow_land_join_divide,
            dump["allow_land_join_divide"].as_bool().unwrap()
        );
        assert_eq!(
            settings.block_show_in_search,
            dump["block_show_in_search"].as_bool().unwrap()
        );
        assert_eq!(settings.sandbox, dump["sandbox"].as_bool().unwrap());
        assert_eq!(settings.casino, dump["casino"].as_bool().unwrap());
        assert_eq!(
            settings.god_block_search,
            dump["god_block_search"].as_bool().unwrap()
        );
        assert_eq!(
            settings.disable_scripts,
            dump["disable_scripts"].as_bool().unwrap()
        );
        assert_eq!(
            settings.disable_collisions,
            dump["disable_collisions"].as_bool().unwrap()
        );
        assert_eq!(
            settings.disable_physics,
            dump["disable_physics"].as_bool().unwrap()
        );
        assert!(!settings.fixed_sun);
        assert!(!dump["fixed_sun"].as_bool().unwrap());
        assert!(dump["fixed_sun_column"].as_bool().unwrap());
        assert_eq!(
            settings.agent_limit,
            dump["agent_limit"].as_i64().unwrap() as i32
        );
        assert_eq!(
            settings.object_bonus,
            dump["object_bonus"].as_f64().unwrap()
        );
        assert_eq!(settings.maturity, dump["maturity"].as_i64().unwrap() as i32);
        assert_eq!(settings.water_height, 42.5);
        assert_eq!(
            settings.water_height,
            f64::from(dump["water_height"].as_f64().unwrap() as f32)
        );
        assert_eq!(settings.terrain_start_heights[0], -100.0);
        assert_eq!(settings.terrain_height_ranges[3], 4000.0);
        assert_eq!(
            settings.terrain_raise_limit,
            dump["terrain_raise_limit"].as_f64().unwrap()
        );
        assert_eq!(
            settings.terrain_lower_limit,
            dump["terrain_lower_limit"].as_f64().unwrap()
        );
        assert_eq!(
            settings.use_estate_sun,
            dump["use_estate_sun"].as_bool().unwrap()
        );
        assert_eq!(
            settings.covenant,
            Uuid::parse_str(dump["covenant"].as_str().unwrap()).unwrap()
        );
        assert_eq!(
            settings.covenant_datetime,
            dump["covenant_datetime"].as_i64().unwrap() as i32
        );
        assert_eq!(
            settings.map_tile_id,
            Uuid::parse_str(dump["map_tile_id"].as_str().unwrap()).unwrap()
        );
        assert_eq!(
            settings.telehub_object,
            Uuid::parse_str(dump["telehub_object"].as_str().unwrap()).unwrap()
        );
        assert_eq!(
            settings.parcel_tile_id,
            Uuid::parse_str(dump["parcel_tile_id"].as_str().unwrap()).unwrap()
        );
        assert_eq!(
            settings.cache_id.unwrap().as_hyphenated().to_string(),
            dump["cache_id"].as_str().unwrap()
        );
        let textures = dump["terrain_textures"].as_array().unwrap();
        let pbr = dump["terrain_pbr"].as_array().unwrap();
        for index in 0..4 {
            assert_eq!(
                settings.terrain_textures[index],
                Uuid::parse_str(textures[index].as_str().unwrap()).unwrap()
            );
            assert_eq!(
                settings.terrain_pbr[index],
                Uuid::parse_str(pbr[index].as_str().unwrap()).unwrap()
            );
            assert_eq!(
                settings.terrain_start_heights[index],
                f64::from(dump["terrain_start_heights"][index].as_f64().unwrap() as f32)
            );
            assert_eq!(
                settings.terrain_height_ranges[index],
                f64::from(dump["terrain_height_ranges"][index].as_f64().unwrap() as f32)
            );
        }
        assert!(settings.terrain_pbr[0].is_nil());
        assert_eq!(
            settings.terrain_textures[0],
            Uuid::from_u128(DEFAULT_TEXTURES[0])
        );
        let column = dump["sun_vector_column"].as_array().unwrap();
        assert_eq!(settings.sun_vector[0], column[0].as_f64().unwrap());
        assert_eq!(settings.sun_vector[1], column[1].as_f64().unwrap());
        assert_eq!(settings.sun_vector[2], column[2].as_f64().unwrap());
        assert_eq!(
            settings.sun_position,
            dump["sun_position_column"].as_f64().unwrap()
        );
        assert_eq!(dump["sun_vector"][0].as_f64().unwrap(), 0.0);
        assert_eq!(dump["sun_position"].as_f64().unwrap(), 0.0);
        let points = dump["spawn_points"].as_array().unwrap();
        assert_eq!(settings.spawn_points.len(), points.len());
        assert_eq!(settings.spawn_points.len(), 2);
        for (point, dumped) in settings.spawn_points.iter().zip(points) {
            assert_eq!(point.yaw, dumped["yaw"].as_f64().unwrap() as f32);
            assert_eq!(point.pitch, dumped["pitch"].as_f64().unwrap() as f32);
            assert_eq!(point.distance, dumped["distance"].as_f64().unwrap() as f32);
        }
    }

    #[tokio::test]
    async fn regionstore_missing_settings_share_one_persistent_cache_id() {
        assert_eq!(dump()["missing_cache_persists"], true);
        let store = CopiedStore::new("regionstore/settings.db");
        let connection = store.connection();
        let load = || open_region_settings(&connection, Uuid::nil(), ObjectStoreKind::Sqlite);
        let (a, b) = tokio::join!(load(), load());
        let a = loaded(a.unwrap());
        assert!(a.cache_id.is_some_and(|id| !id.is_nil()));
        assert_eq!(a.cache_id, loaded(b.unwrap()).cache_id);
        assert_eq!(a.cache_id, loaded(load().await.unwrap()).cache_id);
    }

    #[tokio::test]
    async fn regionstore_empty_pbr_matches_the_csharp_scene_save() {
        let store = CopiedStore::new("regionstore/settings.db");
        let connection = store.connection();
        let region = Uuid::parse_str(REGION).unwrap();
        let path = ferroan_db::sqlite_filename(&connection).unwrap();
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        sqlx::query("UPDATE regionsettings SET cacheID=NULL, TerrainPBR1=?1, TerrainPBR2=?1, TerrainPBR3=?1, TerrainPBR4=?1 WHERE regionUUID=?2")
            .bind(Uuid::nil().to_string()).bind(REGION).execute(&pool).await.unwrap();
        let load = || open_region_settings(&connection, region, ObjectStoreKind::Sqlite);
        let original = loaded(load().await.unwrap());
        assert!(original.cache_id.is_some_and(|id| !id.is_nil()));
        assert_eq!(original.terrain_pbr, original.terrain_textures);
        assert_eq!(
            serde_json::json!(original.terrain_pbr.map(|id| id.to_string())),
            dump()["saved_pbr_from_empty"]
        );
        let classic: String =
            sqlx::query_scalar("SELECT terrain_texture_1 FROM regionsettings WHERE regionUUID=?")
                .bind(REGION)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(classic, original.terrain_textures[0].to_string());
        sqlx::query("UPDATE regionsettings SET terrain_texture_2=? WHERE regionUUID=?")
            .bind(Uuid::new_v4().to_string())
            .bind(REGION)
            .execute(&pool)
            .await
            .unwrap();
        let reopened = loaded(load().await.unwrap());
        assert_eq!(reopened.cache_id, original.cache_id);
        assert_ne!(reopened.terrain_textures, original.terrain_textures);
        assert_eq!(reopened.terrain_pbr, original.terrain_textures);
        pool.close().await;
    }

    #[tokio::test]
    async fn regionstore_zero_classic_texture_is_saved_without_writing_partial_pbr() {
        let store = CopiedStore::new("regionstore/settings.db");
        let connection = store.connection();
        let region = Uuid::parse_str(REGION).unwrap();
        let pool = settings_test_pool(&connection).await;
        sqlx::query(
            "UPDATE regionsettings SET terrain_texture_2=?, cacheID=NULL WHERE regionUUID=?",
        )
        .bind(Uuid::nil().to_string())
        .bind(REGION)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE TRIGGER keep_partial_pbr BEFORE UPDATE OF TerrainPBR1, TerrainPBR2, TerrainPBR3, TerrainPBR4 ON regionsettings BEGIN SELECT RAISE(ABORT, 'PBR set is populated'); END")
            .execute(&pool).await.unwrap();
        let load = || open_region_settings(&connection, region, ObjectStoreKind::Sqlite);
        let first = loaded(load().await.unwrap());
        assert_eq!(
            serde_json::json!(first.terrain_pbr.map(|id| id.to_string())),
            dump()["terrain_pbr"]
        );
        let stored: String =
            sqlx::query_scalar("SELECT terrain_texture_2 FROM regionsettings WHERE regionUUID=?")
                .bind(REGION)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, Uuid::from_u128(DEFAULT_TEXTURES[1]).to_string());
        assert!(first.cache_id.is_some_and(|id| !id.is_nil()));
        assert_eq!(loaded(load().await.unwrap()).cache_id, first.cache_id);
        pool.close().await;
    }

    #[tokio::test]
    async fn regionstore_first_insert_clears_only_its_orphan_spawn_points() {
        let store = CopiedStore::new("regionstore/settings.db");
        let connection = store.connection();
        let pool = settings_test_pool(&connection).await;
        let new_region = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO spawn_points (RegionID, Yaw, Pitch, Distance) VALUES (?, 1, 2, 3)",
        )
        .bind(new_region.to_string())
        .execute(&pool)
        .await
        .unwrap();
        let settings = loaded(
            open_region_settings(&connection, new_region, ObjectStoreKind::Sqlite)
                .await
                .unwrap(),
        );
        assert!(settings.spawn_points.is_empty());
        // A caller that lost the insert race must keep the existing region's spawns.
        insert_default_settings(&pool, REGION).await.unwrap();
        let counts: Vec<(String, i64)> =
            sqlx::query_as("SELECT RegionID, COUNT(*) FROM spawn_points GROUP BY RegionID")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(counts, [(REGION.to_owned(), 2)]);
        pool.close().await;
    }

    #[tokio::test]
    async fn regionstore_terrain_save_rechecks_the_row_under_the_write_lock() {
        let store = CopiedStore::new("regionstore/settings.db");
        let connection = store.connection();
        let pool = settings_test_pool(&connection).await;
        let region = Uuid::parse_str(REGION).unwrap();
        let texture = Uuid::new_v4();
        let cache = Uuid::new_v4();
        let mut editor = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("UPDATE regionsettings SET terrain_texture_1=?, TerrainPBR1=?, cacheID=? WHERE regionUUID=?")
            .bind(texture.to_string()).bind(texture.to_string()).bind(cache.to_string()).bind(REGION)
            .execute(&mut *editor).await.unwrap();
        // The loader sees the old row while the editor's repair is uncommitted.
        let row = sqlx::query(SETTINGS_SQL)
            .bind(REGION)
            .fetch_one(&pool)
            .await
            .unwrap();
        let stale = read_settings_row(&row, region).unwrap();
        assert!(stale.terrain_textures[0].is_nil());
        sqlx::query("CREATE TRIGGER already_repaired BEFORE UPDATE ON regionsettings BEGIN SELECT RAISE(ABORT, 'redundant settings save'); END")
            .execute(&mut *editor).await.unwrap();
        editor.commit().await.unwrap();

        // Even a now-clean row must be re-read under the write lock. With a
        // zero busy timeout, contention is observable without scheduling sleeps.
        let writer = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let busy = resolve_terrain_settings(&pool, stale.clone())
            .await
            .unwrap_err();
        assert!(busy.to_string().contains("database is locked"), "{busy}");
        writer.commit().await.unwrap();

        let fresh = resolve_terrain_settings(&pool, stale).await.unwrap();
        assert_eq!(fresh.terrain_textures[0], texture);
        assert_eq!(fresh.terrain_pbr[0], texture);
        assert_eq!(fresh.cache_id, Some(cache));
        pool.close().await;
    }

    async fn settings_test_pool(connection: &str) -> sqlx::SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(ferroan_db::sqlite_filename(connection).unwrap())
            .busy_timeout(std::time::Duration::ZERO);
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_null_covenant_keeps_the_region() {
        let store = CopiedStore::new("regionstore/settings.db");
        let connection = store.connection();
        let path = ferroan_db::sqlite_filename(&connection).unwrap();
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("UPDATE regionsettings SET covenant = NULL WHERE regionUUID = ?1")
            .bind(REGION)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let settings = loaded(
            open_region_settings(
                &connection,
                Uuid::parse_str(REGION).unwrap(),
                ObjectStoreKind::Sqlite,
            )
            .await
            .unwrap(),
        );
        assert!(settings.covenant.is_nil());
        assert_eq!(settings.water_height, 42.5);
        assert_eq!(settings.spawn_points.len(), 2);
    }

    #[tokio::test]
    async fn a_missing_file_and_another_database_leave_no_row() {
        let path = std::env::temp_dir().join(format!(
            "ferroan-settings-missing-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let connection = format!("Data Source={};Version=3", path.display());
        assert!(matches!(
            open_region_settings(&connection, Uuid::from_u128(1), ObjectStoreKind::Sqlite)
                .await
                .unwrap(),
            SettingsSource::Sqlite(None)
        ));
        assert!(matches!(
            open_region_settings(
                "Data Source=127.0.0.1;Database=ferroan;User ID=ferroan;Password=unused",
                Uuid::nil(),
                ObjectStoreKind::Mysql,
            )
            .await
            .unwrap(),
            SettingsSource::OtherDatabase
        ));
        assert!(matches!(
            open_region_settings(
                "Host=127.0.0.1;Database=ferroan;",
                Uuid::nil(),
                ObjectStoreKind::Postgres,
            )
            .await
            .unwrap(),
            SettingsSource::OtherDatabase
        ));
    }

    #[tokio::test]
    async fn regionstore_settings_oracle_reload_matches_the_committed_dump() {
        if std::env::var("FERROAN_REGIONSTORE").ok().as_deref() != Some("1") {
            eprintln!("regionstore settings oracle reload skipped");
            return;
        }
        let dll = std::env::var("FERROAN_REGIONSTORE_ORACLE").unwrap();
        let dump = dump();
        let pin = dump["pin"].as_str().unwrap();
        let store = CopiedStore::new("regionstore/settings.db");
        let connection = store.connection();
        let path = ferroan_db::sqlite_filename(&connection).unwrap();
        let output = std::process::Command::new("dotnet")
            .arg(&dll)
            .args(["settings-load", &path.display().to_string(), REGION, pin])
            .env("MSBUILDDISABLENODEREUSE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let fresh: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(fresh, dump);
    }
}
