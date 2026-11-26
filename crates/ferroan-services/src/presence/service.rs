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
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    MAX_RESULTS, MAX_ROW_BYTES, PresenceData, PresenceStore, PresenceStoreError, check, lock,
};

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

#[derive(Default)]
struct Cache<K> {
    entries: HashMap<K, (Instant, Row)>,
    next_sweep: Option<Instant>,
}

impl<K: Hash + Eq> Cache<K> {
    fn sweep(&mut self, now: Instant) {
        if let Some(next) = self.next_sweep
            && now >= next
        {
            let elapsed = now.duration_since(next);
            let tick = next + Duration::from_secs((elapsed.as_secs() / 60) * 60);
            self.entries.retain(|_, (expires, _)| *expires >= tick);
            self.next_sweep = (!self.entries.is_empty()).then_some(tick + SWEEP);
        }
    }

    fn insert(&mut self, key: K, row: Row, now: Instant)
    where
        K: Clone,
    {
        self.sweep(now);
        if self.entries.len() == CACHE_CAP
            && !self.entries.contains_key(&key)
            && let Some((oldest, _)) = self.entries.iter().min_by_key(|(_, (expires, _))| *expires)
        {
            let oldest = oldest.clone();
            self.entries.remove(&oldest);
            ferroan_diag::drop_bound(
                ferroan_diag::PRESENCE_CACHE,
                ferroan_diag::Kind::Hard,
                CACHE_CAP as u64,
                CACHE_CAP as u64 + 1,
                "presence",
            );
        }
        self.next_sweep.get_or_insert(now + SWEEP);
        self.entries.insert(key, (now + TTL, row));
    }

    fn get<Q: Hash + Eq + ?Sized>(&mut self, key: &Q) -> Option<Row>
    where
        K: std::borrow::Borrow<Q>,
    {
        self.sweep(Instant::now());
        self.entries.get(key).map(|(_, row)| Arc::clone(row))
    }

    fn remove(&mut self, key: &K) {
        self.entries.remove(key);
        if self.entries.is_empty() {
            self.next_sweep = None;
        }
    }
}

#[derive(Default)]
struct Cached {
    sessions: Cache<Uuid>,
    users: Cache<String>,
}
impl Cached {
    fn remember(&mut self, row: Row) {
        let now = Instant::now();
        self.sessions.insert(row.session_id, Arc::clone(&row), now);
        self.users.insert(row.user_id.clone(), row, now);
    }
    fn forget(&mut self, row: &PresenceData) {
        self.sessions.remove(&row.session_id);
        self.users.remove(&row.user_id);
    }
}

pub struct PresenceService {
    store: Arc<dyn PresenceStore>,
    allow_duplicates: bool,
    cached: Mutex<Cached>,
    // A delayed read must not repopulate the cache after a logout. Reads
    // proceed together; changes to the store and its two indexes are atomic
    // with respect to other calls on this configured service.
    lifecycle: RwLock<()>,
}

impl PresenceService {
    pub fn new(store: Arc<dyn PresenceStore>, allow_duplicates: bool) -> Self {
        Self {
            store,
            allow_duplicates,
            cached: Mutex::default(),
            lifecycle: RwLock::default(),
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
        let _guard = self.lifecycle.write().await;
        if !self.allow_duplicates {
            let previous = lock(&self.cached).users.get(user);
            let exists =
                previous.is_some() || !self.store.get_by_field("UserID", user).await?.is_empty();
            if exists {
                self.store.delete("UserID", user).await?;
                if let Some(previous) = previous {
                    lock(&self.cached).forget(&previous);
                }
            }
        }
        if !self.store.store(Arc::clone(&row)).await? {
            return Ok(false);
        }
        lock(&self.cached).remember(row);
        Ok(true)
    }

    pub async fn logout_agent(&self, session: Uuid) -> Result<bool, PresenceStoreError> {
        let _guard = self.lifecycle.write().await;
        let hit = lock(&self.cached).sessions.get(&session);
        let deleted = self.store.delete("SessionID", &session.to_string()).await?;
        if let Some(row) = hit {
            lock(&self.cached).forget(&row);
        }
        Ok(deleted)
    }

    pub async fn logout_region_agents(&self, region: Uuid) -> Result<bool, PresenceStoreError> {
        let _guard = self.lifecycle.write().await;
        let members = self
            .store
            .get_by_field("RegionID", &region.to_string())
            .await;
        match members {
            Ok(rows) if rows.is_empty() => return Ok(true),
            Ok(rows) => {
                let mut cached = lock(&self.cached);
                for row in rows {
                    cached.forget(&row);
                }
            }
            Err(PresenceStoreError::Limit(limit)) if limit.id == ferroan_diag::PRESENCE_RESULTS => {
                // The bulk delete is unbounded by the field lookup. A cache miss
                // will reload any unaffected row on the next call.
                *lock(&self.cached) = Cached::default();
            }
            Err(error) => return Err(error),
        }
        self.store.logout_region_agents(region).await?;
        Ok(true)
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
        let _guard = self.lifecycle.write().await;
        let hit = lock(&self.cached).sessions.get(&session);
        let cached = hit.is_some();
        let Some(row) = (match hit {
            Some(row) => Some(row),
            None => self.store.get(session).await?,
        }) else {
            return Ok(false);
        };
        let success = self.store.report_agent(session, region).await?;
        if success {
            row.set_region_id(region);
            lock(&self.cached).remember(row);
        } else if cached {
            lock(&self.cached).forget(&row);
        }
        Ok(success)
    }

    pub async fn get_agent(
        &self,
        session: Uuid,
    ) -> Result<Option<PresenceInfo>, PresenceStoreError> {
        let _guard = self.lifecycle.read().await;
        let hit = lock(&self.cached).sessions.get(&session);
        let row = match hit {
            Some(row) => Some(row),
            None => self.store.get(session).await?,
        };
        Ok(row.map(|row| {
            let info = PresenceInfo::from(row.as_ref());
            lock(&self.cached).remember(row);
            info
        }))
    }

    pub async fn get_agents(
        &self,
        users: &[String],
    ) -> Result<Vec<PresenceInfo>, PresenceStoreError> {
        check(ferroan_diag::PRESENCE_RESULTS, MAX_RESULTS, users.len())?;
        let _guard = self.lifecycle.read().await;
        let mut result = Vec::new();
        for user in users {
            check(ferroan_diag::PRESENCE_ROW_BYTES, MAX_ROW_BYTES, user.len())?;
            let hit = lock(&self.cached).users.get(user.as_str());
            let row = match hit {
                Some(row) => Some(row),
                None => self
                    .store
                    .get_by_field("UserID", user)
                    .await?
                    .into_iter()
                    .next(),
            };
            if let Some(row) = row {
                result.push(PresenceInfo::from(row.as_ref()));
                lock(&self.cached).remember(row);
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presence::{NullPresenceData, Sessions};

    fn row(user: String, session: Uuid, region: Uuid) -> Row {
        Arc::new(PresenceData::new(user, session, region, BTreeMap::new()))
    }

    #[tokio::test]
    async fn cold_user_lookup_chooses_the_first_session_and_reads_refresh_both_indexes() {
        let store = Arc::new(NullPresenceData {
            sessions: Arc::new(Mutex::new(Sessions::default())),
        });
        let service = PresenceService::new(store.clone(), true);
        let user = Uuid::new_v4().to_string();
        let first = row(user.clone(), Uuid::new_v4(), Uuid::new_v4());
        let second = row(user.clone(), Uuid::new_v4(), Uuid::new_v4());
        for r in [&first, &second] {
            assert!(store.store(Arc::clone(r)).await.unwrap());
        }
        assert_eq!(
            service
                .get_agents(std::slice::from_ref(&user))
                .await
                .unwrap(),
            [PresenceInfo::from(first.as_ref())]
        );
        assert_eq!(
            service.get_agent(second.session_id).await.unwrap(),
            Some(PresenceInfo::from(second.as_ref()))
        );
        assert_eq!(
            service.get_agents(&[user]).await.unwrap(),
            [PresenceInfo::from(second.as_ref())]
        );
    }

    #[tokio::test]
    async fn region_logout_survives_the_store_lookup_limit() {
        let store = Arc::new(NullPresenceData {
            sessions: Arc::new(Mutex::new(Sessions::default())),
        });
        let service = PresenceService::new(store.clone(), true);
        let region = Uuid::new_v4();
        let first = row("first".into(), Uuid::new_v4(), region);
        assert!(store.store(Arc::clone(&first)).await.unwrap());
        assert!(service.get_agent(first.session_id).await.unwrap().is_some());
        for i in 0..MAX_RESULTS {
            assert!(
                store
                    .store(row(i.to_string(), Uuid::new_v4(), region))
                    .await
                    .unwrap()
            );
        }
        assert!(service.logout_region_agents(region).await.unwrap());
        assert!(service.get_agent(first.session_id).await.unwrap().is_none());
        assert!(
            store
                .get_by_field("RegionID", &region.to_string())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cache_sweeps_after_expiry_and_stays_bounded() {
        let session = Uuid::new_v4();
        let r = row("user".into(), session, Uuid::new_v4());
        let mut cache = Cache::<usize>::default();
        let start = Instant::now();
        cache.insert(0, Arc::clone(&r), start);
        assert_eq!(cache.entries[&0].0, start + Duration::from_secs(300));
        cache.sweep(start + Duration::from_secs(359));
        assert!(cache.entries.contains_key(&0));
        cache.sweep(start + Duration::from_secs(360));
        assert!(cache.entries.is_empty());
        for i in 0..=CACHE_CAP {
            cache.insert(i, Arc::clone(&r), start + Duration::from_millis(i as u64));
        }
        assert_eq!(cache.entries.len(), CACHE_CAP);
        assert!(cache.get(&CACHE_CAP).is_some());
        assert!(cache.get(&0).is_none());
    }
}
