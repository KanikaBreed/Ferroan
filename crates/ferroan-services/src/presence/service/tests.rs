// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::presence::{NullPresenceData, PresenceFuture, Sessions};

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
    for i in 0..super::super::MAX_RESULTS {
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

#[tokio::test]
async fn getagents_accepts_the_largest_list_that_fits_the_form() {
    let service = PresenceService::new(
        Arc::new(NullPresenceData {
            sessions: Arc::new(Mutex::new(Sessions::default())),
        }),
        true,
    );
    let mut users = vec![String::new(); MAX_AGENT_IDS];
    assert!(service.get_agents(&users).await.unwrap().is_empty());
    users.push(String::new());
    let Err(PresenceStoreError::Limit(limit)) = service.get_agents(&users).await else {
        panic!("expected the GetAgents input limit")
    };
    assert_eq!(limit.id, ferroan_diag::PRESENCE_AGENT_IDS);
    assert_eq!(
        (limit.maximum, limit.observed),
        (MAX_AGENT_IDS, MAX_AGENT_IDS + 1)
    );
}

#[tokio::test]
async fn session_eviction_keeps_the_user_index_consistent_with_duplicate_presences() {
    let store = Arc::new(NullPresenceData {
        sessions: Arc::new(Mutex::new(Sessions::default())),
    });
    let service = PresenceService::new(store.clone(), true);
    let make = |user: &str| row(user.to_owned(), Uuid::new_v4(), Uuid::new_v4());
    let old = make("duplicate");
    let current = make("duplicate");
    let evicted = make("another");
    let remaining = make("another");
    for row in [&old, &current, &evicted, &remaining] {
        assert!(store.store(row.clone()).await.unwrap());
    }
    {
        let mut cache = lock(&service.cached);
        let start = Instant::now();
        // The first two sessions will be evicted. Only one still owns its user entry.
        cache.remember_at(old.clone(), start);
        cache.remember_at(evicted.clone(), start);
        let later = start + Duration::from_millis(1);
        cache.remember_at(current.clone(), later);
        for i in 0..CACHE_CAP - 1 {
            cache.remember_at(make(&i.to_string()), later);
        }
        assert_eq!(cache.sessions.len(), CACHE_CAP);
        assert!(!cache.sessions.contains_key(&old.session_id));
        assert!(!cache.sessions.contains_key(&evicted.session_id));
        assert!(!cache.users.contains_key(&evicted.user_id));
        assert_eq!(cache.users.get(&old.user_id), Some(&current.session_id));
    }
    assert!(service.logout_agent(evicted.session_id).await.unwrap());
    assert_eq!(
        service
            .get_agents(std::slice::from_ref(&evicted.user_id))
            .await
            .unwrap(),
        [PresenceInfo::from(remaining.as_ref())]
    );
}

/// Hold a detached user lookup after its snapshot was read, as a delayed SQL response.
struct PausedLookup {
    inner: NullPresenceData,
    captured: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

impl PresenceStore for PausedLookup {
    fn store(&self, row: Row) -> PresenceFuture<'_, bool> {
        self.inner.store(row)
    }
    fn get(&self, session: Uuid) -> PresenceFuture<'_, Option<Row>> {
        self.inner.get(session)
    }
    fn get_by_field<'a>(&'a self, field: &'a str, value: &'a str) -> PresenceFuture<'a, Vec<Row>> {
        Box::pin(async move {
            let rows = self.inner.get_by_field(field, value).await?;
            if field != "UserID" || value != "slow" {
                return Ok(rows);
            }
            let snapshot = rows
                .iter()
                .map(|r| {
                    Arc::new(PresenceData::new(
                        r.user_id.clone(),
                        r.session_id,
                        r.region_id(),
                        r.data.clone(),
                    ))
                })
                .collect();
            self.captured.notify_one();
            self.resume.notified().await;
            Ok(snapshot)
        })
    }
    fn delete<'a>(&'a self, field: &'a str, value: &'a str) -> PresenceFuture<'a, bool> {
        self.inner.delete(field, value)
    }
    fn report_agent(&self, session: Uuid, region: Uuid) -> PresenceFuture<'_, bool> {
        self.inner.report_agent(session, region)
    }
    fn logout_region_agents(&self, region: Uuid) -> PresenceFuture<'_, ()> {
        self.inner.logout_region_agents(region)
    }
    fn verify_agent(&self, agent: Uuid, secure: Uuid) -> PresenceFuture<'_, bool> {
        self.inner.verify_agent(agent, secure)
    }
}

#[tokio::test]
async fn a_report_completes_during_getagents_and_the_stale_read_is_not_cached() {
    let store = Arc::new(PausedLookup {
        inner: NullPresenceData {
            sessions: Arc::new(Mutex::new(Sessions::default())),
        },
        captured: Default::default(),
        resume: Default::default(),
    });
    let session = Uuid::new_v4();
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    assert!(store.store(row("slow".into(), session, old)).await.unwrap());
    let service = Arc::new(PresenceService::new(store.clone(), false));
    let reader = service.clone();
    let lookup =
        tokio::spawn(async move { reader.get_agents(&["missing".into(), "slow".into()]).await });
    let guard = Duration::from_secs(2);
    tokio::time::timeout(guard, store.captured.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(guard, service.report_agent(session, new))
            .await
            .expect("a region report must progress during a blocked GetAgents")
            .unwrap()
    );
    store.resume.notify_one();
    assert_eq!(lookup.await.unwrap().unwrap()[0].region_id, old);
    assert_eq!(
        service.get_agent(session).await.unwrap().unwrap().region_id,
        new
    );
}

#[test]
fn cache_sweeps_both_indexes_after_expiry() {
    let session = Uuid::new_v4();
    let r = row("user".into(), session, Uuid::new_v4());
    let mut cache = Cached::default();
    let start = Instant::now();
    cache.remember_at(r.clone(), start);
    assert_eq!(cache.sessions[&session].0, start + Duration::from_secs(300));
    cache.sweep(start + Duration::from_secs(359));
    assert!(cache.sessions.contains_key(&session));
    cache.sweep(start + Duration::from_secs(360));
    assert!(cache.sessions.is_empty());
    assert!(cache.users.is_empty());
}
