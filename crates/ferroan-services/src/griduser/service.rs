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
//   Source/OpenSim.Services.UserAccountService/GridUserService.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grid-user service.
//!
//! The C# cache is static. This process holds one service, and the cache
//! lives on that service. An entry, including a miss, lasts 300 seconds. The
//! next cache use drops every expired entry. Past `griduser.cache-entries`
//! a new key drops the soonest entry, and the next read of that key comes
//! from the store. A lookup uses the first 36 characters when the id is
//! longer. A new row keeps the id the caller sent. `loggedin` returns the
//! store error, and still answers the row when the store declines it without
//! an error. The other writes answer false when the store does not, and
//! `loggedoutregion` still updates the other rows.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use ferroan_diag::{GRIDUSER_CACHE_ENTRIES, Kind, drop_bound};

use super::model::{GridUser, ONLINE_TRUE};
use super::store::{GridUserStore, StoreError};
use super::wire::{self, GridUserInfo, Vec3};

/// Lifetime of one cache entry, in milliseconds.
pub const CACHE_MS: u64 = 300_000;

/// Entries kept in this process. There is no entry cap on the other side.
pub const CACHE_CAP: usize = 4096;

const ID_CHARS: usize = 36;

pub trait GridClock: Send + Sync {
    fn unix_time(&self) -> i32;
    fn mono_millis(&self) -> u64;
}

#[derive(Clone, Copy, Default)]
pub struct SystemClock;

impl GridClock for SystemClock {
    fn unix_time(&self) -> i32 {
        let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return 0;
        };
        i32::try_from(elapsed.as_secs()).unwrap_or(i32::MAX)
    }

    fn mono_millis(&self) -> u64 {
        let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return 0;
        };
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
    }
}

pub trait GridData: Send + Sync {
    fn get_all(
        &self,
        prefix: &str,
    ) -> impl std::future::Future<Output = Result<Vec<GridUser>, StoreError>> + Send;
    fn get_online_in_region(
        &self,
        region_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<GridUser>, StoreError>> + Send;
    fn store(
        &self,
        user: &GridUser,
    ) -> impl std::future::Future<Output = Result<bool, StoreError>> + Send;
}

impl GridData for GridUserStore {
    async fn get_all(&self, prefix: &str) -> Result<Vec<GridUser>, StoreError> {
        GridUserStore::get_all(self, prefix).await
    }

    async fn get_online_in_region(&self, region_id: &str) -> Result<Vec<GridUser>, StoreError> {
        GridUserStore::get_online_in_region(self, region_id).await
    }

    async fn store(&self, user: &GridUser) -> Result<bool, StoreError> {
        GridUserStore::store(self, user).await
    }
}

struct Slot {
    row: Option<GridUser>,
    expires_at: u64,
}

pub struct GridUserService<D, C> {
    data: D,
    clock: C,
    cache: Mutex<HashMap<String, Slot>>,
}

impl<D, C> GridUserService<D, C>
where
    D: GridData,
    C: GridClock,
{
    pub fn new(data: D, clock: C) -> Self {
        Self {
            data,
            clock,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn clock(&self) -> &C {
        &self.clock
    }

    pub async fn logged_in(&self, user_id: &str) -> Result<GridUserInfo, StoreError> {
        let mut row = match self.load(user_id).await? {
            Some(row) => row,
            None => GridUser::new(user_id),
        };
        let now = self.clock.unix_time().to_string();
        row.data
            .insert("Online".to_string(), ONLINE_TRUE.to_string());
        row.data.insert("Login".to_string(), now);
        self.data.store(&row).await?;
        self.remember(user_id, &row);
        Ok(wire::to_info(&row))
    }

    pub async fn logged_out(
        &self,
        user_id: &str,
        region_id: &str,
        position: Vec3,
        look: Vec3,
    ) -> Result<bool, StoreError> {
        let mut row = match self.load(user_id).await? {
            Some(row) => row,
            None => GridUser::new(user_id),
        };
        row.data.insert("Online".to_string(), "False".to_string());
        row.data
            .insert("Logout".to_string(), self.clock.unix_time().to_string());
        row.data
            .insert("LastRegionID".to_string(), region_id.to_string());
        row.data
            .insert("LastPosition".to_string(), wire::format_vector(position));
        row.data
            .insert("LastLookAt".to_string(), wire::format_vector(look));
        self.finish_write(user_id, &row).await
    }

    pub async fn logged_out_region(&self, region_id: &str) -> Result<bool, StoreError> {
        let rows = self.data.get_online_in_region(region_id).await?;
        if rows.is_empty() {
            return Ok(true);
        }
        let now = self.clock.unix_time().to_string();
        let mut ok = true;
        for mut row in rows {
            row.data.insert("Online".to_string(), "False".to_string());
            row.data.insert("Logout".to_string(), now.clone());
            match self.data.store(&row).await {
                Ok(true) => self.remember(&row.user_id, &row),
                _ => ok = false,
            }
        }
        Ok(ok)
    }

    pub async fn set_home(
        &self,
        user_id: &str,
        region_id: &str,
        position: Vec3,
        look: Vec3,
    ) -> Result<bool, StoreError> {
        let mut row = match self.load(user_id).await? {
            Some(row) => row,
            None => GridUser::new(user_id),
        };
        row.data
            .insert("HomeRegionID".to_string(), region_id.to_string());
        row.data
            .insert("HomePosition".to_string(), wire::format_vector(position));
        row.data
            .insert("HomeLookAt".to_string(), wire::format_vector(look));
        self.finish_write(user_id, &row).await
    }

    pub async fn set_last_position(
        &self,
        user_id: &str,
        region_id: &str,
        position: Vec3,
        look: Vec3,
    ) -> Result<bool, StoreError> {
        let mut row = match self.load(user_id).await? {
            Some(row) => row,
            None => GridUser::new(user_id),
        };
        row.data
            .insert("LastRegionID".to_string(), region_id.to_string());
        row.data
            .insert("LastPosition".to_string(), wire::format_vector(position));
        row.data
            .insert("LastLookAt".to_string(), wire::format_vector(look));
        self.finish_write(user_id, &row).await
    }

    pub async fn get_info(&self, user_id: &str) -> Result<Option<GridUserInfo>, StoreError> {
        Ok(self.load(user_id).await?.map(|row| wire::to_info(&row)))
    }

    pub async fn get_infos(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<Option<GridUserInfo>>, StoreError> {
        let mut infos = Vec::with_capacity(user_ids.len());
        for user_id in user_ids {
            infos.push(self.get_info(user_id).await?);
        }
        Ok(infos)
    }

    pub(crate) fn data(&self) -> &D {
        &self.data
    }

    async fn finish_write(&self, user_id: &str, row: &GridUser) -> Result<bool, StoreError> {
        let stored = self.data.store(row).await.unwrap_or(false);
        if stored {
            self.remember(user_id, row);
        }
        Ok(stored)
    }

    async fn load(&self, user_id: &str) -> Result<Option<GridUser>, StoreError> {
        let key = lookup_key(user_id);
        if let Some(cached) = self.cache_get(&key) {
            return Ok(cached);
        }
        let rows = self.data.get_all(&key).await?;
        if rows.is_empty() {
            self.cache_put(&key, None);
            return Ok(None);
        }
        let chosen = pick_recent(rows);
        self.cache_put(&key, Some(chosen.clone()));
        Ok(Some(chosen))
    }

    fn remember(&self, user_id: &str, row: &GridUser) {
        if char_len(user_id) >= ID_CHARS {
            self.cache_put(&first_chars(user_id, ID_CHARS), Some(row.clone()));
        }
    }

    /// `Some(None)` is a cached miss. `None` is not cached or has expired.
    fn cache_get(&self, key: &str) -> Option<Option<GridUser>> {
        let cache = self.swept_cache();
        cache.get(key).map(|slot| slot.row.clone())
    }

    fn cache_put(&self, key: &str, row: Option<GridUser>) {
        let now = self.clock.mono_millis();
        let expires_at = now.saturating_add(CACHE_MS);
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        cache.retain(|_, slot| now < slot.expires_at);
        if !cache.contains_key(key)
            && cache.len() >= CACHE_CAP
            && let Some(victim) = soonest_key(&cache)
        {
            cache.remove(&victim);
            let configured = u64::try_from(CACHE_CAP).unwrap_or(u64::MAX);
            drop_bound(
                GRIDUSER_CACHE_ENTRIES,
                Kind::Hard,
                configured,
                configured.saturating_add(1),
                "griduser",
            );
        }
        cache.insert(key.to_string(), Slot { row, expires_at });
    }

    fn swept_cache(&self) -> MutexGuard<'_, HashMap<String, Slot>> {
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        let now = self.clock.mono_millis();
        cache.retain(|_, slot| now < slot.expires_at);
        cache
    }
}

fn soonest_key(cache: &HashMap<String, Slot>) -> Option<String> {
    cache
        .iter()
        .min_by(|(left_key, left), (right_key, right)| {
            left.expires_at
                .cmp(&right.expires_at)
                .then_with(|| left_key.cmp(right_key))
        })
        .map(|(key, _)| key.clone())
}

impl GridUserService<GridUserStore, SystemClock> {
    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.data.pool_occupancy()
    }
}

fn lookup_key(user_id: &str) -> String {
    if char_len(user_id) > ID_CHARS {
        first_chars(user_id, ID_CHARS)
    } else {
        user_id.to_string()
    }
}

fn first_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// The greatest login or logout wins. A tie keeps the earlier row. A value
/// that does not parse stops the comparison and keeps the best row so far.
fn pick_recent(rows: Vec<GridUser>) -> GridUser {
    let mut rows = rows.into_iter();
    let Some(first) = rows.next() else {
        return GridUser::new("");
    };
    let mut best = first;
    let Some(mut current) = activity(&best) else {
        return best;
    };
    for row in rows {
        let Some(stamp) = activity(&row) else {
            return best;
        };
        if stamp > current {
            current = stamp;
            best = row;
        }
    }
    best
}

fn activity(row: &GridUser) -> Option<i32> {
    let login = wire::parse_i32(row.field("Login")?)?;
    let logout = wire::parse_i32(row.field("Logout")?)?;
    Some(login.max(logout))
}
