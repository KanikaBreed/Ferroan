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
// Ported to Rust for Ferroan from:
//   Source/OpenSim.Data.SQLite/SQLiteSimulationData.cs
//   Source/OpenSim.Framework/{LandData,ViewerEnvironment}.cs
//   Source/OpenSim.Region.CoreModules/World/Land/{LandObject,LandManagementModule}.cs
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite land rows and the immutable region parcel map, in four-metre cells.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use ferroan_db::{Backend, Pool};
use ferroan_diag::{Kind, REGION_PARCEL_ACCESS, REGION_PARCEL_BYTES, REGION_PARCELS};
use futures_util::TryStreamExt;
use sqlx::{Row, SqliteConnection, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

const MAX_PARCELS: i64 = 65_536;
const MAX_ACCESS: i64 = 65_536;
const MAX_BYTES: i64 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ParcelError {
    #[error("parcel store: {0}")]
    Store(#[from] ferroan_db::DbError),
    #[error("parcel query: {0}")]
    Query(#[from] sqlx::Error),
    #[error("invalid parcel column {0}")]
    Column(&'static str),
    #[error("parcel store exceeds {0}")]
    Limit(&'static str),
    #[error(transparent)]
    Size(#[from] crate::TerrainError),
}

#[derive(Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub struct LandAccess {
    pub agent: Uuid,
    pub flags: u32,
    pub expires: i32,
}

#[derive(Debug, PartialEq)]
#[cfg_attr(test, derive(serde::Deserialize))]
pub struct Parcel {
    pub id: Uuid,
    pub local_id: i32,
    pub owner: Uuid,
    pub group: Uuid,
    pub group_owned: bool,
    pub name: String,
    pub description: String,
    pub area: i32,
    #[cfg_attr(test, serde(default))]
    pub simwide_area: i32,
    pub auction: u32,
    pub category: i32,
    pub claim_date: i32,
    pub claim_price: i32,
    pub sale_price: i32,
    pub status: i32,
    pub flags: u32,
    pub landing_type: u8,
    pub media_auto_scale: u8,
    pub media_id: Uuid,
    pub media_url: String,
    pub music_url: String,
    pub pass_hours: f32,
    pub pass_price: i32,
    pub snapshot: Uuid,
    pub dwell: f32,
    pub media_type: String,
    pub media_description: String,
    pub media_size: [i32; 2],
    pub media_loop: bool,
    pub obscure_media: bool,
    pub obscure_music: bool,
    pub see_avs: bool,
    pub any_av_sounds: bool,
    pub group_av_sounds: bool,
    pub location: [f32; 3],
    pub look_at: [f32; 3],
    pub auth_buyer: Uuid,
    pub other_clean_time: i32,
    #[cfg_attr(test, serde(skip))]
    /// The persisted LLSD text of the loaded viewer environment.
    pub environment: Option<String>,
    pub environment_version: i32,
    pub access: Vec<LandAccess>,
}

impl Parcel {
    /// LandData constructor, then CreateDefaultParcel's estate owner and claim time.
    fn estate_land(owner: Uuid, claim_date: i32) -> Self {
        Self {
            id: Uuid::new_v4(),
            owner,
            claim_date,
            name: "Your Parcel".into(),
            landing_type: 2,
            flags: 1 | 2 | 8 | 64 | (1 << 13) | (1 << 27) | (1 << 29),
            media_type: "none/none".into(),
            see_avs: true,
            any_av_sounds: true,
            group_av_sounds: true,
            environment_version: -1,
            local_id: 0,
            group: Uuid::nil(),
            group_owned: false,
            description: String::new(),
            area: 0,
            simwide_area: 0,
            auction: 0,
            category: 0,
            claim_price: 0,
            sale_price: 0,
            status: 0,
            media_auto_scale: 0,
            media_id: Uuid::nil(),
            media_url: String::new(),
            music_url: String::new(),
            pass_hours: 0.0,
            pass_price: 0,
            snapshot: Uuid::nil(),
            dwell: 0.0,
            media_description: String::new(),
            media_size: [0; 2],
            media_loop: false,
            obscure_media: false,
            obscure_music: false,
            location: [0.0; 3],
            look_at: [0.0; 3],
            auth_buyer: Uuid::nil(),
            other_clean_time: 0,
            environment: None,
            access: Vec::new(),
        }
    }
}

struct StoredParcel {
    data: Parcel,
    bitmap: Vec<u8>,
}

pub struct ParcelMap {
    width: usize,
    cells: Vec<u32>,
    parcels: Vec<Parcel>,
}

impl ParcelMap {
    pub fn parcels(&self) -> &[Parcel] {
        &self.parcels
    }

    /// Parcel at a four-metre cell, with (0, 0) in the southwest.
    pub fn at_cell(&self, x: usize, y: usize) -> Option<&Parcel> {
        if x >= self.width || y >= self.cells.len() / self.width {
            return None;
        }
        let id = self.cells[y * self.width + x];
        self.parcels.get(id.checked_sub(1)? as usize)
    }

    fn assemble(
        size: [u32; 2],
        estate: Uuid,
        now: i32,
        rows: Vec<StoredParcel>,
    ) -> Result<Self, ParcelError> {
        for side in size {
            crate::height::check_edge(i32::try_from(side).unwrap_or(i32::MAX))?;
        }
        let width = size[0] as usize / 4;
        let count = width * (size[1] as usize / 4);
        let mut cells = vec![0; count];
        let mut parcels = Vec::new();
        for StoredParcel { mut data, bitmap } in rows {
            let indices = bitmap_cells(&bitmap, width, count);
            if indices.clone().any(|i| cells[i] != 0) {
                log::warn!("ignoring overlapping parcel {}", data.id);
                continue;
            }
            data.local_id = parcels.len() as i32 + 1;
            if data.group_owned {
                data.group = data.owner;
            }
            data.area = 0;
            for i in indices {
                cells[i] = data.local_id as u32;
                data.area += 16;
            }
            parcels.push(data);
        }

        let unclaimed = cells.iter().filter(|id| **id == 0).count();
        if unclaimed > 0 {
            let id = if parcels.len() == 1 {
                // D80: the area and the final live cell map agree.
                parcels[0].area = (count * 16) as i32;
                1
            } else {
                let mut parcel = Parcel::estate_land(estate, now);
                parcel.local_id = parcels.len() as i32 + 1;
                parcel.area = (16 * unclaimed) as i32;
                let id = parcel.local_id as u32;
                parcels.push(parcel);
                id
            };
            for cell in &mut cells {
                if *cell == 0 {
                    *cell = id;
                }
            }
        }
        let mut owned_area = HashMap::new();
        for parcel in &parcels {
            *owned_area.entry(parcel.owner).or_insert(0) += parcel.area;
        }
        for parcel in &mut parcels {
            parcel.simwide_area = owned_area[&parcel.owner];
        }
        Ok(Self {
            width,
            cells,
            parcels,
        })
    }
}

/// ConvertBytesToLandBitmap clips the input first. A resulting 512 bytes
/// has the legacy 64-cell stride even in a larger or rectangular region.
fn bitmap_cells(
    bytes: &[u8],
    width: usize,
    count: usize,
) -> impl Iterator<Item = usize> + Clone + '_ {
    let len = bytes.len().min(count / 8);
    let stride = if len == 512 { 64 } else { width };
    bytes[..len]
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte != 0)
        .flat_map(move |(i, byte)| {
            (0..8).filter_map(move |bit| {
                let n = i * 8 + bit;
                let x = n % stride;
                let cell = n / stride * width + x;
                (byte & (1 << bit) != 0 && x < width && cell < count).then_some(cell)
            })
        })
}

/// Load just this SQLite region. A missing file or absent store has no parcels.
/// The generated default exists only in memory (the database write is P2.9a).
pub async fn open_parcels(
    connection: Option<&str>,
    region: Uuid,
    size: [u32; 2],
    estate: Uuid,
) -> Result<ParcelMap, ParcelError> {
    let mut rows = Vec::new();
    if let Some(connection) = connection
        && ferroan_db::sqlite_filename(connection)?
            .try_exists()
            .map_err(sqlx::Error::Io)?
    {
        let pool = Pool::open(Backend::Sqlite, connection, &["RegionStore"]).await?;
        let result = match pool.sqlite() {
            Some(sqlite) => load_rows(sqlite, region).await,
            None => Err(ParcelError::Column("SQLite provider")),
        };
        pool.close().await;
        rows = result?;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i32;
    ParcelMap::assemble(size, estate, now, rows)
}

const LAND_COLUMNS: &str = "UUID,LocalLandID,Bitmap,Name,Desc,OwnerUUID,IsGroupOwned,Area,AuctionID,Category,ClaimDate,ClaimPrice,GroupUUID,SalePrice,LandStatus,LandFlags,LandingType,MediaAutoScale,MediaTextureUUID,MediaURL,MusicURL,PassHours,PassPrice,SnapshotUUID,Dwell,MediaType,MediaDescription,MediaSize,MediaLoop,ObscureMedia,ObscureMusic,SeeAVs,AnyAVSounds,GroupAVSounds,UserLocationX,UserLocationY,UserLocationZ,UserLookAtX,UserLookAtY,UserLookAtZ,AuthbuyerID,OtherCleanTime,environment";
const REGION_LAND: &str = "FROM land WHERE RegionUUID = ? COLLATE NOCASE";
const REGION_ACCESS: &str = "FROM landaccesslist AS a JOIN land AS l ON a.LandUUID = l.UUID COLLATE NOCASE WHERE l.RegionUUID = ? COLLATE NOCASE";

fn check_limit(id: &'static str, max: i64, observed: i64) -> Result<(), ParcelError> {
    if observed > max {
        ferroan_diag::refuse(id, Kind::Hard, max as u64, observed as u64, "land");
        return Err(ParcelError::Limit(id));
    }
    Ok(())
}

/// Measure only the selected cells, before sqlx materialises their contents.
/// Both preflights and reads see the same SQLite snapshot.
async fn row_budget(
    conn: &mut SqliteConnection,
    columns: &str,
    from: &str,
    region: &str,
    id: &'static str,
    max: i64,
) -> Result<i64, ParcelError> {
    let lengths = columns
        .split(',')
        .map(|column| format!("coalesce(length(CAST({column} AS BLOB)),0)"))
        .collect::<Vec<_>>()
        .join("+");
    let query = format!(
        "SELECT count(*), coalesce(sum(bytes),0) FROM (SELECT {lengths} AS bytes {from} LIMIT {})",
        max + 1
    );
    let (count, bytes): (i64, i64) = sqlx::query_as(&query).bind(region).fetch_one(conn).await?;
    check_limit(id, max, count)?;
    Ok(bytes)
}

async fn load_rows(pool: &SqlitePool, region: Uuid) -> Result<Vec<StoredParcel>, ParcelError> {
    let region = region.to_string();
    let mut tx = pool.begin().await?;
    let land_bytes = row_budget(
        &mut tx,
        LAND_COLUMNS,
        REGION_LAND,
        &region,
        REGION_PARCELS,
        MAX_PARCELS,
    )
    .await?;
    let access_bytes = row_budget(
        &mut tx,
        "a.LandUUID,a.AccessUUID,a.Flags",
        REGION_ACCESS,
        &region,
        REGION_PARCEL_ACCESS,
        MAX_ACCESS,
    )
    .await?;
    check_limit(REGION_PARCEL_BYTES, MAX_BYTES, land_bytes + access_bytes)?;

    let mut rows = Vec::new();
    let mut indices = HashMap::new();
    {
        let query = format!("SELECT {LAND_COLUMNS} {REGION_LAND} ORDER BY rowid");
        let mut stream = sqlx::query(&query).bind(&region).fetch(&mut *tx);
        while let Some(row) = stream.try_next().await? {
            let data = build_land_data(&row)?;
            indices.insert(data.id, rows.len());
            rows.push(StoredParcel {
                data,
                bitmap: required(&row, "Bitmap")?,
            });
        }
    }
    {
        let query =
            format!("SELECT a.LandUUID,a.AccessUUID,a.Flags {REGION_ACCESS} ORDER BY a.rowid");
        let mut stream = sqlx::query(&query).bind(&region).fetch(&mut *tx);
        while let Some(row) = stream.try_next().await? {
            let parcel = uuid(&row, "LandUUID")?;
            if let Some(index) = indices.get(&parcel) {
                rows[*index].data.access.push(LandAccess {
                    agent: uuid(&row, "AccessUUID")?,
                    flags: integer(&row, "Flags")?,
                    expires: 0,
                });
            }
        }
    }
    tx.commit().await?;
    Ok(rows)
}

fn required<'r, T: sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>>(
    row: &'r SqliteRow,
    key: &'static str,
) -> Result<T, ParcelError> {
    row.try_get::<Option<T>, _>(key)?
        .ok_or(ParcelError::Column(key))
}

fn uuid(row: &SqliteRow, key: &'static str) -> Result<Uuid, ParcelError> {
    Uuid::parse_str(required::<&str>(row, key)?.trim()).map_err(|_| ParcelError::Column(key))
}

fn integer<T: TryFrom<i64>>(row: &SqliteRow, key: &'static str) -> Result<T, ParcelError> {
    T::try_from(required::<i64>(row, key)?).map_err(|_| ParcelError::Column(key))
}

fn boolean(row: &SqliteRow, key: &'static str) -> Result<bool, ParcelError> {
    match required::<i64>(row, key) {
        Ok(i) => Ok(i != 0),
        Err(ParcelError::Query(sqlx::Error::ColumnDecode { .. })) => {
            let word: &str = required(row, key)?;
            if word.eq_ignore_ascii_case("true") {
                Ok(true)
            } else if word.eq_ignore_ascii_case("false") {
                Ok(false)
            } else {
                Err(ParcelError::Column(key))
            }
        }
        Err(e) => Err(e),
    }
}

fn media_size(text: &str) -> Result<[i32; 2], ParcelError> {
    let mut parts = text.split(',');
    let (Some(w), Some(h)) = (parts.next(), parts.next()) else {
        return Ok([0, 0]);
    };
    let parse = |s: &str| {
        s.trim()
            .parse()
            .map_err(|_| ParcelError::Column("MediaSize"))
    };
    Ok([parse(w)?, parse(h)?])
}

fn environment_version(text: &str) -> Option<i32> {
    if text.is_empty() {
        return None;
    }
    let prefix = text.as_bytes();
    let starts = |s: &[u8]| {
        prefix
            .get(..s.len())
            .is_some_and(|v| v.eq_ignore_ascii_case(s))
    };
    let osd = if starts(b"<? llsd/notation") {
        ferroan_llsd::from_notation(prefix).ok()?
    } else if starts(b"<? llsd/binary") {
        ferroan_llsd::from_binary(prefix).ok()?.0
    } else if starts(b"<llsd") || starts(b"<?xml") || starts(b"<? llsd/xml") {
        ferroan_llsd::from_xml(prefix).ok()?
    } else {
        let json: serde_json::Value = serde_json::from_str(text).ok()?;
        use serde_json::Value;
        let map = match json {
            Value::Object(map) => map,
            Value::Array(_) | Value::Number(_) => return Some(0),
            _ => return None,
        };
        return Some(match map.get("env_version") {
            None => 1,
            Some(Value::Number(n)) => n
                .as_i64()
                .map(|i| i as i32)
                .unwrap_or_else(|| n.as_f64().unwrap_or_default().round_ties_even() as i32),
            Some(Value::Bool(b)) => i32::from(*b),
            Some(Value::String(s)) => version_from_string(s),
            _ => 0,
        });
    };
    Some(if !osd.is_map() {
        0
    } else if osd.has("env_version") {
        match osd.get("env_version") {
            ferroan_llsd::Llsd::Real(r) => r.round_ties_even() as i32,
            ferroan_llsd::Llsd::String(s) => version_from_string(s),
            value => value.as_i32(),
        }
    } else {
        1
    })
}

fn version_from_string(text: &str) -> i32 {
    text.trim()
        .parse::<f64>()
        .map(|n| n.floor() as i32)
        .unwrap_or(0)
}

fn build_land_data(row: &SqliteRow) -> Result<Parcel, ParcelError> {
    let vector = |keys: [&str; 3]| -> Result<Option<[f32; 3]>, sqlx::Error> {
        let mut values = [0.0; 3];
        for (out, key) in values.iter_mut().zip(keys) {
            let Some(v) = row.try_get::<Option<f64>, _>(key)? else {
                return Ok(None);
            };
            *out = v as f32;
        }
        Ok(Some(values))
    };
    let (location, look_at) = vector(["UserLocationX", "UserLocationY", "UserLocationZ"])?
        .zip(vector(["UserLookAtX", "UserLookAtY", "UserLookAtZ"])?)
        .unwrap_or_default();
    let env: Option<String> = row.try_get("environment")?;
    let version = env.as_deref().and_then(environment_version);
    Ok(Parcel {
        id: uuid(row, "UUID")?,
        local_id: integer(row, "LocalLandID")?,
        name: required(row, "Name")?,
        description: required(row, "Desc")?,
        owner: uuid(row, "OwnerUUID")?,
        group_owned: boolean(row, "IsGroupOwned")?,
        area: integer(row, "Area")?,
        auction: integer(row, "AuctionID")?,
        category: i32::from(integer::<i32>(row, "Category")? as i8),
        claim_date: integer(row, "ClaimDate")?,
        claim_price: integer(row, "ClaimPrice")?,
        group: uuid(row, "GroupUUID")?,
        sale_price: integer(row, "SalePrice")?,
        status: i32::from(integer::<i32>(row, "LandStatus")? as i8),
        flags: integer(row, "LandFlags")?,
        landing_type: integer(row, "LandingType")?,
        media_auto_scale: integer(row, "MediaAutoScale")?,
        media_id: uuid(row, "MediaTextureUUID")?,
        media_url: required(row, "MediaURL")?,
        music_url: required(row, "MusicURL")?,
        pass_hours: required::<f64>(row, "PassHours")? as f32,
        pass_price: integer(row, "PassPrice")?,
        snapshot: uuid(row, "SnapshotUUID")?,
        dwell: integer::<i32>(row, "Dwell")? as f32,
        media_type: required(row, "MediaType")?,
        media_description: required(row, "MediaDescription")?,
        media_size: media_size(required(row, "MediaSize")?)?,
        media_loop: boolean(row, "MediaLoop")?,
        obscure_media: boolean(row, "ObscureMedia")?,
        obscure_music: boolean(row, "ObscureMusic")?,
        see_avs: boolean(row, "SeeAVs")?,
        any_av_sounds: boolean(row, "AnyAVSounds")?,
        group_av_sounds: boolean(row, "GroupAVSounds")?,
        location,
        look_at,
        auth_buyer: Uuid::parse_str(required::<&str>(row, "AuthbuyerID")?.trim())
            .unwrap_or_default(),
        other_clean_time: integer(row, "OtherCleanTime")?,
        environment: env.filter(|_| version.is_some()),
        environment_version: version.unwrap_or(-1),
        simwide_area: 0,
        access: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_store::CopiedStore;

    const SEED: &str = include_str!("../../../fixtures/regionstore/parcels.sql");
    const DUMP: &str = include_str!("../../../fixtures/regionstore/parcels.json");

    #[tokio::test]
    async fn regionstore_parcels_match_pin8_load_and_access_entries() {
        let oracle_db = std::env::var_os("FERROAN_PARCEL_ORACLE_DB");
        let copy = oracle_db
            .is_none()
            .then(|| CopiedStore::new("regionstore/objects.db"));
        let connection = match &copy {
            Some(copy) => copy.connection(),
            None => format!(
                "Data Source={};Version=3",
                std::path::PathBuf::from(oracle_db.unwrap()).display()
            ),
        };
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        if copy.is_some() {
            sqlx::raw_sql(SEED).execute(sqlite).await.unwrap();
        }
        let dump: serde_json::Value = serde_json::from_str(DUMP).unwrap();
        for probe in dump["environments"].as_array().unwrap() {
            let text = probe["text"].as_str().unwrap();
            let version: Option<i32> = serde_json::from_value(probe["version"].clone()).unwrap();
            assert_eq!(environment_version(text), version, "{text}");
        }
        let region = dump["region"].as_str().unwrap().parse().unwrap();
        let rows = load_rows(sqlite, region).await.unwrap();
        let expected = dump["parcels"].as_array().unwrap();
        assert_eq!(rows.len(), expected.len());
        for (i, (row, json)) in rows.into_iter().zip(expected).enumerate() {
            let reference: Parcel = serde_json::from_value(json.clone()).unwrap();
            let mut parcel = row.data;
            let env = parcel.environment.take();
            assert_eq!(
                env.as_deref(),
                (i == 0).then_some("<? llsd/notation ?>{\"env_version\":i7}")
            );
            assert_eq!(parcel, reference);
            let hex: String = row
                .bitmap
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(hex, json["bitmap"].as_str().unwrap());
        }

        let map = open_parcels(Some(&connection), region, [32, 16], Uuid::nil())
            .await
            .unwrap();
        assert_eq!(map.parcels().len(), 3);
        assert_eq!(map.at_cell(1, 0).unwrap().name, "West");
        assert_eq!(map.at_cell(7, 0).unwrap().name, "West");
        assert_eq!(map.at_cell(0, 1).unwrap().name, "West");
        assert_eq!(map.at_cell(2, 0).unwrap().name, "East");
        assert_eq!(map.at_cell(1, 1).unwrap().name, "Your Parcel");
        assert_eq!(map.at_cell(2, 0).unwrap().local_id, 2);
        assert_eq!(map.at_cell(0, 1).unwrap().area, 48);
        assert!(map.at_cell(8, 0).is_none());
        assert!(map.at_cell(0, 4).is_none());

        let owner = Uuid::from_u128(0xab);
        let empty = open_parcels(Some(&connection), Uuid::nil(), [32, 16], owner)
            .await
            .unwrap();
        assert_eq!(empty.parcels().len(), 1);
        let mut default = empty.parcels.into_iter().next().unwrap();
        assert_eq!(
            (
                default.local_id,
                default.owner,
                default.area,
                default.simwide_area
            ),
            (1, owner, 512, 512)
        );
        assert!(!default.id.is_nil());
        assert!(default.claim_date > 0);
        // Compare every other constructor field with an actual Pin8 LandData.
        default.id = Uuid::nil();
        default.local_id = 0;
        default.owner = Uuid::nil();
        default.area = 0;
        default.simwide_area = 0;
        default.claim_date = 0;
        assert_eq!(
            default,
            serde_json::from_value(dump["defaults"].clone()).unwrap()
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM land")
            .fetch_one(sqlite)
            .await
            .unwrap();
        assert_eq!(count, 3);
        pool.close().await;
    }

    fn stored(name: &str, bitmap: Vec<u8>) -> StoredParcel {
        let data = Parcel {
            name: name.into(),
            ..Parcel::estate_land(Uuid::nil(), 1)
        };
        StoredParcel { data, bitmap }
    }

    #[tokio::test]
    async fn a_malformed_owner_stops_the_load() {
        let copy = CopiedStore::new("regionstore/objects.db");
        let pool = Pool::open(Backend::Sqlite, &copy.connection(), &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        sqlx::raw_sql(SEED).execute(sqlite).await.unwrap();
        sqlx::query("UPDATE land SET OwnerUUID = 'broken' WHERE Name = 'West'")
            .execute(sqlite)
            .await
            .unwrap();
        let region = "11111111-1111-1111-1111-111111111111".parse().unwrap();
        assert!(matches!(
            load_rows(sqlite, region).await,
            Err(ParcelError::Column("OwnerUUID"))
        ));
        pool.close().await;
    }

    #[tokio::test]
    async fn parcel_budgets_count_only_matching_rows_and_measure_bytes_past_nul() {
        let copy = CopiedStore::new("regionstore/objects.db");
        let pool = Pool::open(Backend::Sqlite, &copy.connection(), &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        sqlx::raw_sql(SEED).execute(sqlite).await.unwrap();
        let mut tx = sqlite.begin().await.unwrap();
        let region = "11111111-1111-1111-1111-111111111111";
        for (columns, from, limit, id) in [
            ("UUID", REGION_LAND, 1, REGION_PARCELS),
            ("a.AccessUUID", REGION_ACCESS, 2, REGION_PARCEL_ACCESS),
        ] {
            assert!(
                matches!(row_budget(&mut tx, columns, from, region, id, limit).await,
                Err(ParcelError::Limit(actual)) if actual == id)
            );
        }
        sqlx::query("UPDATE land SET Name = CAST(x'61006263' AS TEXT) WHERE Name = 'West'")
            .execute(&mut *tx)
            .await
            .unwrap();
        // Four bytes for West (including the embedded NUL), four for East.
        let bytes = row_budget(&mut tx, "Name", REGION_LAND, region, REGION_PARCELS, 2)
            .await
            .unwrap();
        assert_eq!(bytes, 8);
        assert!(matches!(
            check_limit(REGION_PARCEL_BYTES, 7, bytes),
            Err(ParcelError::Limit(REGION_PARCEL_BYTES))
        ));
        tx.rollback().await.unwrap();
        pool.close().await;
    }

    #[test]
    fn clipped_512_byte_bitmap_keeps_its_legacy_stride() {
        let mut bitmap = vec![0; 512];
        bitmap[8] = 1;
        bitmap[511] = 0x80;
        let map = ParcelMap::assemble(
            [512, 256],
            Uuid::nil(),
            42,
            vec![stored("legacy", bitmap.clone()), stored("other", vec![1])],
        )
        .unwrap();
        assert_eq!(map.at_cell(0, 1).unwrap().name, "legacy");
        assert_eq!(map.at_cell(63, 63).unwrap().name, "legacy");
        assert_eq!(map.at_cell(64, 0).unwrap().name, "Your Parcel");
        // A longer stored bitmap clipped to 512 bytes also uses the legacy width.
        bitmap.push(0xff);
        let cells: Vec<_> = bitmap_cells(&bitmap, 128, 4096).collect();
        assert_eq!(cells, [128]);
    }

    #[test]
    fn one_parcel_expands_and_multiple_parcels_leave_the_remainder_to_the_estate() {
        let owner = Uuid::from_u128(17);
        let one = ParcelMap::assemble([16, 16], owner, 42, vec![stored("one", vec![1])]).unwrap();
        assert_eq!(one.parcels().len(), 1);
        assert_eq!(one.at_cell(3, 3).unwrap().name, "one");
        assert_eq!(
            (one.parcels()[0].area, one.parcels()[0].simwide_area),
            (256, 256)
        );
        let multi = ParcelMap::assemble(
            [16, 16],
            owner,
            42,
            vec![
                stored("one", vec![1]),
                stored("overlaps", vec![3]),
                stored("two", vec![2]),
            ],
        )
        .unwrap();
        assert_eq!(multi.parcels().len(), 3);
        assert_eq!(multi.at_cell(1, 0).unwrap().local_id, 2);
        let estate = multi.at_cell(3, 3).unwrap();
        assert_eq!(
            (estate.owner, estate.area, estate.claim_date),
            (owner, 14 * 16, 42)
        );
    }
}
