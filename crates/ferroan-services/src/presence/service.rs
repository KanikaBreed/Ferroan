/*
 * Copyright (c) Contributors, http://opensimulator.org/
 * See CONTRIBUTORS.TXT for a full list of copyright holders.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *     * Redistributions of source code must retain the above copyright
 *       notice, this list of conditions and the following disclaimer.
 *     * Redistributions in binary form must reproduce the above copyright
 *       notice, this list of conditions and the following disclaimer in the
 *       documentation and/or other materials provided with the distribution.
 *     * Neither the name of the OpenSimulator Project nor the
 *       names of its contributors may be used to endorse or promote products
 *       derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
 * EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

// Ported from Tranquillity Source/OpenSim.Services.PresenceService/PresenceService.cs and OpenSim.Framework/ExpiringCacheOS.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncGuard, RwLock};
use tokio::time::Instant;
use uuid::Uuid;

use super::{MAX_ROW_BYTES, PresenceData, PresenceStore, PresenceStoreError, check, lock};

#[derive(Debug, PartialEq, Eq)]
pub struct PresenceInfo {
    pub user_id: String,
    pub region_id: Uuid,
}

impl From<&PresenceData> for PresenceInfo {
    fn from(row: &PresenceData) -> Self {
        Self {
            user_id: row.user_id.clone(),
            region_id: row.region_id(),
        }
    }
}

type Row = Arc<PresenceData>;
const TTL: Duration = Duration::from_secs(300);
const SWEEP: Duration = Duration::from_secs(60);
const CACHE_CAP: usize = 4096;

// Even an empty value needs one "&uuids[]" term. Every list that fits a
// getagents HTTP body can pass this count check, including compact UUIDs.
pub(super) const MAX_AGENT_IDS: usize =
    (ferroan_http::MAX_QUERY_BYTES - "METHOD=getagents".len()) / "&uuids[]".len();

#[derive(Default)]
struct Cached {
    sessions: HashMap<Uuid, (Instant, Row)>,
    users: HashMap<String, Uuid>,
    next_sweep: Option<Instant>,
    // Cold lookups may return a snapshot taken before a concurrent mutation.
    // Only publish it to the shared indexes if no mutation has completed since.
    generation: u64,
}

impl Cached {
    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn sweep(&mut self, now: Instant) {
        if let Some(next) = self.next_sweep
            && now >= next
        {
            let elapsed = now.duration_since(next);
            let tick = next + Duration::from_secs((elapsed.as_secs() / 60) * 60);
            self.sessions.retain(|id, (expires, row)| {
                if *expires >= tick {
                    return true;
                }
                if self.users.get(&row.user_id) == Some(id) {
                    self.users.remove(&row.user_id);
                }
                false
            });
            self.next_sweep = (!self.sessions.is_empty()).then_some(tick + SWEEP);
        }
    }

    fn session(&mut self, id: Uuid) -> Option<Row> {
        self.sweep(Instant::now());
        self.sessions.get(&id).map(|(_, row)| row.clone())
    }

    fn user(&mut self, user: &str) -> Option<Row> {
        let session = *self.users.get(user)?;
        self.session(session)
    }

    fn remove_session(&mut self, session: Uuid) {
        if let Some((_, row)) = self.sessions.remove(&session)
            && self.users.get(&row.user_id) == Some(&session)
        {
            self.users.remove(&row.user_id);
        }
        if self.sessions.is_empty() {
            self.next_sweep = None;
        }
    }

    fn remember(&mut self, row: Row) {
        self.remember_at(row, Instant::now());
    }

    fn remember_at(&mut self, row: Row, now: Instant) {
        self.sweep(now);
        if let Some((_, old)) = self.sessions.get(&row.session_id) {
            if old.user_id != row.user_id && self.users.get(&old.user_id) == Some(&row.session_id) {
                self.users.remove(&old.user_id);
            }
        } else if self.sessions.len() == CACHE_CAP
            && let Some((&id, _)) = self
                .sessions
                .iter()
                .min_by_key(|(_, (expires, _))| *expires)
        {
            self.remove_session(id);
            ferroan_diag::drop_bound(
                ferroan_diag::PRESENCE_CACHE,
                ferroan_diag::Kind::Hard,
                CACHE_CAP as u64,
                CACHE_CAP as u64 + 1,
                "presence",
            );
        }
        self.next_sweep.get_or_insert(now + SWEEP);
        self.users.insert(row.user_id.clone(), row.session_id);
        self.sessions.insert(row.session_id, (now + TTL, row));
    }

    fn forget(&mut self, row: &PresenceData) {
        self.remove_session(row.session_id);
        self.users.remove(&row.user_id);
    }

    fn forget_user(&mut self, user: &str) {
        self.sessions.retain(|_, (_, row)| row.user_id != user);
        self.users.remove(user);
        if self.sessions.is_empty() {
            self.next_sweep = None;
        }
    }
}

// Fixed stripes bound lock storage even when every request names a new ID.
struct KeyLocks<L> {
    hash: RandomState,
    locks: [L; 128],
}
impl<L: Default> Default for KeyLocks<L> {
    fn default() -> Self {
        Self {
            hash: RandomState::new(),
            locks: std::array::from_fn(|_| L::default()),
        }
    }
}
impl<L> KeyLocks<L> {
    fn for_key(&self, key: impl Hash) -> &L {
        let slot = self.hash.hash_one(key) as usize % self.locks.len();
        &self.locks[slot]
    }
}

pub struct PresenceService {
    store: Arc<dyn PresenceStore>,
    allow_duplicates: bool,
    cached: Mutex<Cached>,
    // A region delete excludes reports entering that region (and logins into region zero).
    // Mutators acquire region, session and (for duplicate suppression) user
    // guards in that order. Reads take none of these across provider calls.
    region_logout: KeyLocks<RwLock<()>>,
    session_writes: KeyLocks<AsyncMutex<()>>,
    user_writes: KeyLocks<AsyncMutex<()>>,
}

impl PresenceService {
    pub fn new(store: Arc<dyn PresenceStore>, allow_duplicates: bool) -> Self {
        Self {
            store,
            allow_duplicates,
            cached: Mutex::default(),
            region_logout: KeyLocks::default(),
            session_writes: KeyLocks::default(),
            user_writes: KeyLocks::default(),
        }
    }

    /// In-process login only. A fresh row lets each SQL provider supply LastSeen (D84).
    pub async fn login_agent(
        &self,
        user: &str,
        session: Uuid,
        secure: Uuid,
    ) -> Result<bool, PresenceStoreError> {
        check(ferroan_diag::PRESENCE_ROW_BYTES, MAX_ROW_BYTES, user.len())?;
        let row = Arc::new(PresenceData::new(
            user.to_owned(),
            session,
            Uuid::nil(),
            BTreeMap::from([("SecureSessionID".into(), secure.to_string())]),
        ));
        row.validate()?;
        let _region = self.region_logout.for_key(Uuid::nil()).read().await;
        let _session = self.session_writes.for_key(session).lock().await;
        let _user = self.duplicate_guard(user).await;
        if !self.allow_duplicates {
            let cached = lock(&self.cached).user(user).is_some();
            if cached || !self.store.get_by_field("UserID", user).await?.is_empty() {
                self.store.delete("UserID", user).await?;
                let mut cache = lock(&self.cached);
                cache.changed();
                cache.forget_user(user);
            }
        }
        if !self.store.store(Arc::clone(&row)).await? {
            return Ok(false);
        }
        let mut cache = lock(&self.cached);
        cache.changed();
        cache.remember(row);
        Ok(true)
    }

    async fn duplicate_guard(&self, user: &str) -> Option<AsyncGuard<'_, ()>> {
        if self.allow_duplicates {
            None
        } else {
            Some(self.user_writes.for_key(user).lock().await)
        }
    }

    pub async fn logout_agent(&self, session: Uuid) -> Result<bool, PresenceStoreError> {
        let _session = self.session_writes.for_key(session).lock().await;
        let deleted = self.store.delete("SessionID", &session.to_string()).await?;
        let mut cache = lock(&self.cached);
        cache.changed();
        if let Some(row) = cache.session(session) {
            cache.forget(&row);
        }
        Ok(deleted)
    }

    pub async fn logout_region_agents(&self, region: Uuid) -> Result<bool, PresenceStoreError> {
        let _region = self.region_logout.for_key(region).write().await;
        let members = match self
            .store
            .get_by_field("RegionID", &region.to_string())
            .await
        {
            Ok(rows) if rows.is_empty() => return Ok(true),
            Ok(rows) => Some(rows),
            Err(PresenceStoreError::Limit(limit)) if limit.id == ferroan_diag::PRESENCE_RESULTS => {
                None
            }
            Err(error) => return Err(error),
        };
        let result = self.store.logout_region_agents(region).await;
        let mut cache = lock(&self.cached);
        cache.changed();
        if let Some(rows) = members {
            for row in rows {
                cache.forget(&row);
            }
        } else {
            cache.sessions.clear();
            cache.users.clear();
            cache.next_sweep = None;
        }
        result.map(|()| true)
    }

    pub async fn report_agent(
        &self,
        session: Uuid,
        region: Uuid,
    ) -> Result<bool, PresenceStoreError> {
        match self.report(session, region).await {
            Err(PresenceStoreError::Database(_)) => {
                tracing::warn!("Presence region report failed in the store");
                Ok(false)
            }
            result => result,
        }
    }

    async fn report(&self, session: Uuid, region: Uuid) -> Result<bool, PresenceStoreError> {
        let _region = self.region_logout.for_key(region).read().await;
        let _session = self.session_writes.for_key(session).lock().await;
        let hit = lock(&self.cached).session(session);
        let cached = hit.is_some();
        let Some(row) = (match hit {
            Some(row) => Some(row),
            None => self.store.get(session).await?,
        }) else {
            return Ok(false);
        };
        let _user = self.duplicate_guard(&row.user_id).await;
        let success = self.store.report_agent(session, region).await?;
        let mut cache = lock(&self.cached);
        cache.changed();
        if success {
            row.set_region_id(region);
            cache.remember(row);
        } else if cached {
            cache.forget(&row);
        }
        Ok(success)
    }

    pub async fn get_agent(
        &self,
        session: Uuid,
    ) -> Result<Option<PresenceInfo>, PresenceStoreError> {
        self.lookup(Lookup::Session(session)).await
    }

    pub async fn get_agents(
        &self,
        users: &[String],
    ) -> Result<Vec<PresenceInfo>, PresenceStoreError> {
        check(ferroan_diag::PRESENCE_AGENT_IDS, MAX_AGENT_IDS, users.len())?;
        let mut result = Vec::new();
        for user in users {
            check(ferroan_diag::PRESENCE_ROW_BYTES, MAX_ROW_BYTES, user.len())?;
            if let Some(info) = self.lookup(Lookup::User(user)).await? {
                result.push(info);
            }
        }
        Ok(result)
    }

    async fn lookup(&self, key: Lookup<'_>) -> Result<Option<PresenceInfo>, PresenceStoreError> {
        let generation = {
            let mut cache = lock(&self.cached);
            let hit = match key {
                Lookup::Session(id) => cache.session(id),
                Lookup::User(user) => cache.user(user),
            };
            if let Some(row) = hit {
                let info = PresenceInfo::from(row.as_ref());
                cache.remember(row);
                return Ok(Some(info));
            }
            cache.generation
        };
        let row = match key {
            Lookup::Session(id) => self.store.get(id).await?,
            Lookup::User(user) => self
                .store
                .get_by_field("UserID", user)
                .await?
                .into_iter()
                .next(),
        };
        Ok(row.map(|row| {
            let info = PresenceInfo::from(row.as_ref());
            let mut cache = lock(&self.cached);
            if cache.generation == generation {
                cache.remember(row);
            }
            info
        }))
    }
}

enum Lookup<'a> {
    Session(Uuid),
    User(&'a str),
}

#[cfg(test)]
mod tests;
