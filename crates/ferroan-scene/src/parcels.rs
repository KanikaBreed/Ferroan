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
//   Source/OpenSim.Framework/{LandData,ViewerDaycycle,ViewerEnvironment,ViewerSky,ViewerWater}.cs
//   Source/OpenSim.Region.CoreModules/World/Land/{LandObject,LandManagementModule}.cs
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite land rows and the immutable region parcel map, in four-metre cells.

pub mod oar;
mod overlay;
mod properties;
mod vector_text;

pub use oar::{OarParcel, OarParcelError, ParcelLoad, ParcelOptions, load_parcels};
pub use overlay::OverlayPacket;
pub(crate) use properties::prim_counts;
pub use properties::{ParcelPropertiesSend, ParcelRegion, PrimCounts, PropertiesRequest};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ferroan_db::{Backend, Pool};
use ferroan_diag::{Kind, REGION_PARCEL_ACCESS, REGION_PARCEL_BYTES, REGION_PARCELS};
use ferroan_llsd::Llsd;
use futures_util::TryStreamExt;
use serde_json::Value;
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
    /// Bounding box from `LandObject.UpdateGeometryValues`, stored when the
    /// land is loaded. A later encode reads these samples.
    #[cfg_attr(test, serde(default))]
    pub aabb_min: [f32; 3],
    #[cfg_attr(test, serde(default))]
    pub aabb_max: [f32; 3],
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
            aabb_min: [0.0; 3],
            aabb_max: [0.0; 3],
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

/// `PermissionsModule.IsAdministrator`, supplied by the region. The default
/// answers false. The region installs `StaffGate`'s god rule.
pub type Administrator = Arc<dyn Fn(Uuid) -> bool + Send + Sync>;

struct EstateBan {
    /// `EstateSettings.EstateOwner`. Absent from the row keeps the INI owner.
    owner: Uuid,
    /// `EstateSettings.l_EstateManagers`.
    managers: Vec<Uuid>,
    /// `EstateSettings.TaxFree`. Region access control is the only check then.
    tax_free: bool,
    administrator: Administrator,
}

impl EstateBan {
    fn fresh(owner: Uuid) -> Self {
        Self {
            owner,
            managers: Vec::new(),
            tax_free: false,
            administrator: Arc::new(|_| false),
        }
    }
}

pub struct ParcelMap {
    width: usize,
    cells: Vec<u32>,
    parcels: Vec<Parcel>,
    /// Shared so a later estate read updates every holder of this map.
    ban: Arc<Mutex<EstateBan>>,
    /// Empty element for this region plus 8192 bytes. Fixed with the map size.
    text_limit: u64,
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

    /// `GetLandObject(float, float)`: truncate to an integer metre, return
    /// nothing outside the region, then index the 4 m cell. An empty land
    /// list is nothing, including inside the region.
    pub fn at_metres(&self, x: f32, y: f32) -> Option<&Parcel> {
        if self.width == 0 || !x.is_finite() || !y.is_finite() {
            return None;
        }
        let x = x as i32;
        let y = y as i32;
        let height = self.cells.len() / self.width;
        let size_x = (self.width * 4) as i32;
        let size_y = (height * 4) as i32;
        if x < 0 || y < 0 || x >= size_x || y >= size_y {
            return None;
        }
        self.at_cell((x / 4) as usize, (y / 4) as usize)
    }

    fn empty(size: [u32; 2]) -> Result<Self, ParcelError> {
        for side in size {
            crate::height::check_edge(i32::try_from(side).unwrap_or(i32::MAX))?;
        }
        let width = size[0] as usize / 4;
        let count = width * (size[1] as usize / 4);
        let cells_y = count.checked_div(width).unwrap_or(0);
        Ok(Self {
            width,
            cells: vec![0; count],
            parcels: Vec::new(),
            ban: Arc::new(Mutex::new(EstateBan::fresh(Uuid::nil()))),
            text_limit: properties::empty_text_limit(width, cells_y),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_description(&mut self, local_id: i32, description: String) {
        if let Some(parcel) = self
            .parcels
            .iter_mut()
            .find(|parcel| parcel.local_id == local_id)
        {
            parcel.description = description;
        }
    }

    /// The stored estate row. `owner` of `None` keeps the owner already stored,
    /// which is `[Estates] DefaultEstateOwnerUUID` until this runs.
    pub fn set_estate(&self, owner: Option<Uuid>, managers: Vec<Uuid>, tax_free: bool) {
        let mut ban = self.ban.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(owner) = owner {
            ban.owner = owner;
        }
        ban.managers = managers;
        ban.tax_free = tax_free;
    }

    /// `IsAdministrator`. The region passes the permissions god rule.
    pub fn set_administrator(&self, administrator: Administrator) {
        self.ban
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .administrator = administrator;
    }

    #[cfg(test)]
    pub(crate) fn set_tax_free(&self, tax_free: bool) {
        self.ban
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .tax_free = tax_free;
    }

    /// `IsBannedFromLand`'s exemptions (`LandObject.cs` 873–885): `TaxFree`,
    /// `IsAdministrator`, `IsEstateManagerOrOwner`, then the parcel owner.
    pub(crate) fn exempts(&self, agent: Uuid, parcel_owner: Uuid) -> bool {
        let (tax_free, owner, managers, administrator) = {
            let ban = self.ban.lock().unwrap_or_else(|poison| poison.into_inner());
            (
                ban.tax_free,
                ban.owner,
                ban.managers.clone(),
                Arc::clone(&ban.administrator),
            )
        };
        tax_free
            || administrator(agent)
            || agent == owner
            || managers.contains(&agent)
            || agent == parcel_owner
    }

    fn insert(&mut self, StoredParcel { mut data, bitmap }: StoredParcel) {
        let indices = bitmap_cells(&bitmap, self.width, self.cells.len());
        if indices.clone().any(|i| self.cells[i] != 0) {
            log::warn!("ignoring overlapping parcel {}", data.id);
            return;
        }
        data.local_id = self.parcels.len() as i32 + 1;
        if data.group_owned {
            data.group = data.owner;
        }
        data.area = 0;
        for i in indices {
            self.cells[i] = data.local_id as u32;
            data.area += 16;
        }
        self.parcels.push(data);
    }

    fn fill_unclaimed(&mut self, estate: Uuid, now: i32) {
        let unclaimed = self.cells.iter().filter(|id| **id == 0).count();
        if unclaimed > 0 {
            let id = if self.parcels.len() == 1 {
                // D80: the area and the final live cell map agree.
                self.parcels[0].area = (self.cells.len() * 16) as i32;
                1
            } else {
                let mut parcel = Parcel::estate_land(estate, now);
                parcel.local_id = self.parcels.len() as i32 + 1;
                parcel.area = (16 * unclaimed) as i32;
                let id = parcel.local_id as u32;
                self.parcels.push(parcel);
                id
            };
            for cell in &mut self.cells {
                if *cell == 0 {
                    *cell = id;
                }
            }
        }
        let mut owned_area = HashMap::new();
        for parcel in &self.parcels {
            *owned_area.entry(parcel.owner).or_insert(0) += parcel.area;
        }
        for parcel in &mut self.parcels {
            parcel.simwide_area = owned_area[&parcel.owner];
        }
    }
}

#[cfg(test)]
pub(crate) fn limit_hits(id: &str, action: &str) -> u64 {
    let needle = format!("ferroan_limit_hits_total{{limit=\"{id}\",action=\"{action}\"}} ");
    ferroan_diag::metrics()
        .lines()
        .find_map(|line| line.strip_prefix(&needle)?.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn limits_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
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
/// Its claim time is the unix seconds of this call.
pub async fn open_parcels(
    connection: Option<&str>,
    region: Uuid,
    size: [u32; 2],
    estate: Uuid,
) -> Result<ParcelMap, ParcelError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i32;
    open_parcels_at(connection, region, size, estate, now).await
}

/// [`open_parcels`] with the default parcel's claim time supplied by the caller.
pub async fn open_parcels_at(
    connection: Option<&str>,
    region: Uuid,
    size: [u32; 2],
    estate: Uuid,
    now: i32,
) -> Result<ParcelMap, ParcelError> {
    let mut map = ParcelMap::empty(size)?;
    map.set_estate(Some(estate), Vec::new(), false);
    if let Some(connection) = connection
        && ferroan_db::sqlite_filename(connection)?
            .try_exists()
            .map_err(sqlx::Error::Io)?
    {
        let pool = Pool::open(Backend::Sqlite, connection, &["RegionStore"]).await?;
        let result = match pool.sqlite() {
            Some(sqlite) => {
                load_rows(sqlite, region, map.cells.len() / 8, |row| map.insert(row)).await
            }
            None => Err(ParcelError::Column("SQLite provider")),
        };
        pool.close().await;
        result?;
    }
    map.fill_unclaimed(estate, now);
    Ok(map)
}

const LAND_COLUMNS: &str = "UUID,LocalLandID,Name,Desc,OwnerUUID,IsGroupOwned,Area,AuctionID,Category,ClaimDate,ClaimPrice,GroupUUID,SalePrice,LandStatus,LandFlags,LandingType,MediaAutoScale,MediaTextureUUID,MediaURL,MusicURL,PassHours,PassPrice,SnapshotUUID,Dwell,MediaType,MediaDescription,MediaSize,MediaLoop,ObscureMedia,ObscureMusic,SeeAVs,AnyAVSounds,GroupAVSounds,UserLocationX,UserLocationY,UserLocationZ,UserLookAtX,UserLookAtY,UserLookAtZ,AuthbuyerID,OtherCleanTime,environment";
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

async fn load_rows(
    pool: &SqlitePool,
    region: Uuid,
    bitmap_bytes: usize,
    mut visit: impl FnMut(StoredParcel),
) -> Result<(), ParcelError> {
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

    let mut access = HashMap::<Uuid, Vec<LandAccess>>::new();
    {
        let query = format!(
            "SELECT a.LandUUID,a.AccessUUID,a.Flags {REGION_ACCESS} GROUP BY a.LandUUID,a.AccessUUID,a.Flags ORDER BY min(a.rowid)"
        );
        let mut stream = sqlx::query(&query).bind(&region).fetch(&mut *tx);
        while let Some(row) = stream.try_next().await? {
            access
                .entry(uuid(&row, "LandUUID")?)
                .or_default()
                .push(LandAccess {
                    agent: uuid(&row, "AccessUUID")?,
                    flags: integer(&row, "Flags")?,
                    expires: 0,
                });
        }
    }
    {
        // SQLite substr on an empty blob returns NULL. Preserve the empty
        // bitmap and keep a real NULL subject to the required-column check.
        let query = format!(
            "SELECT {LAND_COLUMNS}, CASE WHEN length(Bitmap)=0 THEN Bitmap ELSE substr(Bitmap,1,?) END AS Bitmap {REGION_LAND} ORDER BY rowid"
        );
        let mut stream = sqlx::query(&query)
            .bind(bitmap_bytes as i64)
            .bind(&region)
            .fetch(&mut *tx);
        while let Some(row) = stream.try_next().await? {
            let mut data = build_land_data(&row)?;
            data.access = access.remove(&data.id).unwrap_or_default();
            visit(StoredParcel {
                data,
                bitmap: required(&row, "Bitmap")?,
            });
        }
    }
    tx.commit().await?;
    Ok(())
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

/// Borrow the same day-cycle containers from either stored encoding.
trait EnvironmentNode: Sized {
    fn field(&self, key: &str) -> &Self;
    fn values(&self) -> Option<impl Iterator<Item = &Self>>;
    fn elements(&self) -> &[Self];
    fn string_value(&self) -> Option<&str>;
    fn is_type(&self, name: &str) -> bool {
        self.string_value() == Some(name)
    }
}

impl EnvironmentNode for Llsd {
    fn is_type(&self, name: &str) -> bool {
        self.as_str() == name
    }
    fn field(&self, key: &str) -> &Self {
        self.get(key)
    }
    fn values(&self) -> Option<impl Iterator<Item = &Self>> {
        self.as_map().map(|m| m.values())
    }
    fn elements(&self) -> &[Self] {
        self.as_array()
    }
    fn string_value(&self) -> Option<&str> {
        if let Llsd::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
}

impl EnvironmentNode for Value {
    fn field(&self, key: &str) -> &Self {
        &self[key]
    }
    fn values(&self) -> Option<impl Iterator<Item = &Self>> {
        self.as_object().map(|m| m.values())
    }
    fn elements(&self) -> &[Self] {
        self.as_array().map(Vec::as_slice).unwrap_or_default()
    }
    fn string_value(&self) -> Option<&str> {
        // Empty JSON strings deserialize as an undefined OSD.
        self.as_str().filter(|s| !s.is_empty())
    }
}

fn valid_vectors(node: &impl EnvironmentNode, fields: &[(&str, vector_text::Components)]) -> bool {
    fields.iter().all(|(key, kind)| {
        node.field(key)
            .string_value()
            .is_none_or(|s| vector_text::accepts(s, *kind))
    })
}

/// DayCycle.FromOSD reads every frame and the water track plus four sky tracks.
/// It skips missing or differently typed containers and requires map entries
/// inside the containers it reads. String values for vector and quaternion
/// fields must pass OpenMetaverse's Vector2, Vector3 or Quaternion parser.
/// An empty JSON string is undefined and skips parsing.
fn valid_day_cycle(env: &impl EnvironmentNode) -> bool {
    use vector_text::Components::{Pair, Rotation, Triple};
    let cycle = env.field("day_cycle");
    if let Some(frames) = cycle.field("frames").values() {
        for frame in frames {
            if frame.values().is_none() {
                return false;
            }
            let kind = frame.field("type");
            if kind.is_type("water")
                && !valid_vectors(
                    frame,
                    &[
                        ("normal_scale", Triple),
                        ("water_fog_color", Triple),
                        ("wave1_direction", Pair),
                        ("wave2_direction", Pair),
                    ],
                )
            {
                return false;
            }
            if kind.is_type("sky") {
                if !valid_vectors(
                    frame,
                    &[
                        ("glow", Triple),
                        ("cloud_color", Triple),
                        ("cloud_pos_density1", Triple),
                        ("cloud_pos_density2", Triple),
                        ("cloud_scroll_rate", Pair),
                        ("moon_rotation", Rotation),
                        ("sun_rotation", Rotation),
                    ],
                ) || !valid_vectors(
                    frame.field("legacy_haze"),
                    &[
                        ("ambient", Triple),
                        ("blue_density", Triple),
                        ("blue_horizon", Triple),
                    ],
                ) {
                    return false;
                }
                // SkyData reads the first two absorption layers and one layer
                // each for Mie and Rayleigh scattering, when supplied.
                for (key, count) in [
                    ("absorption_config", 2),
                    ("mie_config", 1),
                    ("rayleigh_config", 1),
                ] {
                    if frame
                        .field(key)
                        .elements()
                        .iter()
                        .take(count)
                        .any(|v| v.values().is_none())
                    {
                        return false;
                    }
                }
            }
        }
    }
    cycle
        .field("tracks")
        .elements()
        .iter()
        .take(5)
        .flat_map(|track| track.elements())
        .all(|entry| entry.values().is_some())
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
        if !valid_day_cycle(&json) {
            return None;
        }
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
    if !valid_day_cycle(&osd) {
        return None;
    }
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
        aabb_min: [0.0; 3],
        aabb_max: [0.0; 3],
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
        // Exercise the same row decoder as a normal load, including clearing
        // the stored text and setting version -1 after a failed parse.
        let update_env =
            format!("UPDATE land SET environment=? WHERE Name='West' RETURNING {LAND_COLUMNS}");
        let mut tx = sqlite.begin().await.unwrap();
        for probe in dump["environments"].as_array().unwrap() {
            let text = probe["text"].as_str().unwrap();
            let version: Option<i32> = serde_json::from_value(probe["version"].clone()).unwrap();
            let row = sqlx::query(&update_env)
                .bind(text)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            let parcel = build_land_data(&row).unwrap();
            assert_eq!(
                parcel.environment.as_deref(),
                version.map(|_| text),
                "{text}"
            );
            assert_eq!(parcel.environment_version, version.unwrap_or(-1), "{text}");
        }
        tx.rollback().await.unwrap();
        let region = dump["region"].as_str().unwrap().parse().unwrap();
        let mut rows = Vec::new();
        load_rows(sqlite, region, 4, |row| rows.push(row))
            .await
            .unwrap();
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

    fn assemble(
        size: [u32; 2],
        estate: Uuid,
        now: i32,
        rows: Vec<StoredParcel>,
    ) -> Result<ParcelMap, ParcelError> {
        let mut map = ParcelMap::empty(size)?;
        for row in rows {
            map.insert(row);
        }
        map.fill_unclaimed(estate, now);
        Ok(map)
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
            load_rows(sqlite, region, 4, |_| {}).await,
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

    #[tokio::test]
    async fn large_region_loads_513_parcels_with_region_sized_bitmaps() {
        let copy = CopiedStore::new("regionstore/objects.db");
        let connection = copy.connection();
        let pool = Pool::open(Backend::Sqlite, &connection, &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        sqlx::raw_sql(SEED).execute(sqlite).await.unwrap();
        let region = Uuid::from_u128(513);
        let bitmap_bytes = 1024 * 1024 / 8;
        // Each parcel owns the first cell of a different byte. Its remaining
        // zero bits are stored too, just as in C#'s full region-sized bitmap.
        let fields = LAND_COLUMNS.strip_prefix("UUID,").unwrap();
        let insert = format!(
            "WITH RECURSIVE parcels(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM parcels WHERE n<512)
             INSERT INTO land ({LAND_COLUMNS},RegionUUID,Bitmap)
             SELECT printf('00000000-0000-4000-8000-%012x',n),{fields},?,
                    CAST(zeroblob(n) || x'01' || zeroblob(?-n-1) AS BLOB)
             FROM parcels CROSS JOIN land WHERE land.Name='West'"
        );
        sqlx::query(&insert)
            .bind(region.to_string())
            .bind(bitmap_bytes)
            .execute(sqlite)
            .await
            .unwrap();
        pool.close().await;
        let map = open_parcels(Some(&connection), region, [4096, 4096], Uuid::nil())
            .await
            .unwrap();
        assert_eq!(map.parcels().len(), 514);
        for n in 0..513 {
            let parcel = map.at_cell(n * 8 % 1024, n * 8 / 1024).unwrap();
            assert_eq!(
                parcel.id,
                Uuid::from_u128(0x00000000_0000_4000_8000_000000000000 + n as u128)
            );
            assert_eq!(parcel.area, 16);
        }
        let rest = map.at_cell(1023, 1023).unwrap();
        assert_eq!(
            (rest.name.as_str(), rest.area),
            ("Your Parcel", 4096 * 4096 - 513 * 16)
        );
    }

    #[tokio::test]
    async fn sqlite_returns_only_the_bitmap_prefix_and_preserves_empty_blobs() {
        let copy = CopiedStore::new("regionstore/objects.db");
        let pool = Pool::open(Backend::Sqlite, &copy.connection(), &["RegionStore"])
            .await
            .unwrap();
        let sqlite = pool.sqlite().unwrap();
        sqlx::raw_sql(SEED).execute(sqlite).await.unwrap();
        sqlx::query(
            "UPDATE land SET Bitmap = CASE Name WHEN 'West' THEN x'820102030405' ELSE x'' END",
        )
        .execute(sqlite)
        .await
        .unwrap();
        let region = "11111111-1111-1111-1111-111111111111".parse().unwrap();
        let mut bitmaps = Vec::new();
        load_rows(sqlite, region, 4, |row| bitmaps.push(row.bitmap))
            .await
            .unwrap();
        assert_eq!(bitmaps, [vec![0x82, 1, 2, 3], vec![]]);
        pool.close().await;
    }

    #[test]
    fn clipped_512_byte_bitmap_keeps_its_legacy_stride() {
        let mut bitmap = vec![0; 512];
        bitmap[8] = 1;
        bitmap[511] = 0x80;
        let map = assemble(
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
        let one = assemble([16, 16], owner, 42, vec![stored("one", vec![1])]).unwrap();
        assert_eq!(one.parcels().len(), 1);
        assert_eq!(one.at_cell(3, 3).unwrap().name, "one");
        assert_eq!(
            (one.parcels()[0].area, one.parcels()[0].simwide_area),
            (256, 256)
        );
        let multi = assemble(
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
