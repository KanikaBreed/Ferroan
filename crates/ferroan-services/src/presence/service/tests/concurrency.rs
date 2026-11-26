// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use futures_util::FutureExt;
use std::{future::Future, pin::pin};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Store,
    Report,
    UserLookup,
    RegionLookup,
}

/// A provider with SQL-style detached rows. Only the selected call pauses,
/// after the provider has finished the operation and before the service sees its result.
struct PausedStore {
    inner: NullPresenceData,
    pause: Mutex<Option<After>>,
    resume: tokio::sync::Notify,
}

fn fixture(allow_duplicates: bool) -> (Arc<PausedStore>, PresenceService) {
    let store = Arc::new(PausedStore {
        inner: NullPresenceData {
            sessions: Arc::new(Mutex::new(Sessions::default())),
        },
        pause: Mutex::default(),
        resume: Default::default(),
    });
    let service = PresenceService::new(store.clone(), allow_duplicates);
    (store, service)
}

fn copy_row(r: &Row) -> Row {
    Arc::new(PresenceData::new(
        r.user_id.clone(),
        r.session_id,
        r.region_id(),
        r.data.clone(),
    ))
}

impl PausedStore {
    async fn after(&self, point: After) {
        if lock(&self.pause).take_if(|p| *p == point).is_some() {
            self.resume.notified().await;
        }
    }

    /// Poll each call to its first suspension. Without a guard the second
    /// call completes before the first returns to its cache update.
    async fn interleave<A, B>(
        &self,
        point: After,
        first: impl Future<Output = A>,
        second: impl Future<Output = B>,
    ) -> (A, B, bool) {
        *lock(&self.pause) = Some(point);
        let mut first = pin!(first);
        let mut second = pin!(second);
        assert!(first.as_mut().now_or_never().is_none());
        assert_eq!(*lock(&self.pause), None, "first call must reach {point:?}");
        let early = second.as_mut().now_or_never();
        let second_completed = early.is_some();
        self.resume.notify_one();
        let a = first.await;
        let b = match early {
            Some(b) => b,
            None => second.await,
        };
        (a, b, second_completed)
    }
}

impl PresenceStore for PausedStore {
    fn store(&self, row: Row) -> PresenceFuture<'_, bool> {
        Box::pin(async move {
            let stored = self.inner.store(copy_row(&row)).await?;
            self.after(After::Store).await;
            Ok(stored)
        })
    }
    fn get(&self, session: Uuid) -> PresenceFuture<'_, Option<Row>> {
        Box::pin(async move { Ok(self.inner.get(session).await?.as_ref().map(copy_row)) })
    }
    fn get_by_field<'a>(&'a self, field: &'a str, value: &'a str) -> PresenceFuture<'a, Vec<Row>> {
        Box::pin(async move {
            let rows = self
                .inner
                .get_by_field(field, value)
                .await?
                .iter()
                .map(copy_row)
                .collect();
            match field {
                "UserID" => self.after(After::UserLookup).await,
                "RegionID" => self.after(After::RegionLookup).await,
                _ => (),
            }
            Ok(rows)
        })
    }
    fn delete<'a>(&'a self, field: &'a str, value: &'a str) -> PresenceFuture<'a, bool> {
        self.inner.delete(field, value)
    }
    fn report_agent(&self, session: Uuid, region: Uuid) -> PresenceFuture<'_, bool> {
        Box::pin(async move {
            let moved = self.inner.report_agent(session, region).await?;
            self.after(After::Report).await;
            Ok(moved)
        })
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
    let (store, service) = fixture(false);
    let session = Uuid::new_v4();
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    assert!(store.store(row("slow".into(), session, old)).await.unwrap());
    let users = ["slow".into(), "missing".into()];
    let (lookup, report, early) = store
        .interleave(
            After::UserLookup,
            service.get_agents(&users),
            service.report_agent(session, new),
        )
        .await;
    assert!(early && report.unwrap());
    assert_eq!(lookup.unwrap()[0].region_id, old);
    assert_eq!(
        service.get_agent(session).await.unwrap().unwrap().region_id,
        new
    );
}

#[tokio::test]
async fn session_logout_stays_final_after_a_report() {
    let (store, service) = fixture(true);
    let session = Uuid::new_v4();
    assert!(
        service
            .login_agent("user", session, Uuid::new_v4())
            .await
            .unwrap()
    );
    let (report, logout, _) = store
        .interleave(
            After::Report,
            service.report_agent(session, Uuid::new_v4()),
            service.logout_agent(session),
        )
        .await;
    assert!(report.unwrap() && logout.unwrap());
    assert!(store.get(session).await.unwrap().is_none());
    assert!(service.get_agent(session).await.unwrap().is_none());
}

#[tokio::test]
async fn session_logout_stays_final_after_a_login() {
    let (store, service) = fixture(true);
    let session = Uuid::new_v4();
    let (login, logout, _) = store
        .interleave(
            After::Store,
            service.login_agent("user", session, Uuid::new_v4()),
            service.logout_agent(session),
        )
        .await;
    assert!(login.unwrap() && logout.unwrap());
    assert!(store.get(session).await.unwrap().is_none());
    assert!(service.get_agent(session).await.unwrap().is_none());
}

#[tokio::test]
async fn region_logout_finishes_before_a_report_enters_the_region() {
    let (store, service) = fixture(true);
    let region = Uuid::new_v4();
    let resident = Uuid::new_v4();
    let arrival = Uuid::new_v4();
    assert!(
        store
            .store(row("resident".into(), resident, region))
            .await
            .unwrap()
    );
    assert!(
        service
            .login_agent("arrival", arrival, Uuid::new_v4())
            .await
            .unwrap()
    );
    assert!(service.get_agent(resident).await.unwrap().is_some());
    let (logout, report, _) = store
        .interleave(
            After::RegionLookup,
            service.logout_region_agents(region),
            service.report_agent(arrival, region),
        )
        .await;
    assert!(logout.unwrap() && report.unwrap());
    assert!(service.get_agent(resident).await.unwrap().is_none());
    assert_eq!(
        service.get_agent(arrival).await.unwrap().unwrap().region_id,
        region
    );
    assert_eq!(
        store
            .get(arrival)
            .await
            .unwrap()
            .expect("arrival survives bulk delete")
            .region_id(),
        region
    );
}

#[tokio::test]
async fn zero_region_logout_finishes_before_a_new_login() {
    let (store, service) = fixture(true);
    let resident = Uuid::new_v4();
    let arrival = Uuid::new_v4();
    assert!(
        service
            .login_agent("resident", resident, Uuid::new_v4())
            .await
            .unwrap()
    );
    let (logout, login, _) = store
        .interleave(
            After::RegionLookup,
            service.logout_region_agents(Uuid::nil()),
            service.login_agent("arrival", arrival, Uuid::new_v4()),
        )
        .await;
    assert!(logout.unwrap() && login.unwrap());
    assert!(service.get_agent(resident).await.unwrap().is_none());
    assert!(service.get_agent(arrival).await.unwrap().is_some());
    assert!(store.get(arrival).await.unwrap().is_some());
}

// A coincidental session-stripe collision must not serialize a user-lock test.
fn separate_sessions(service: &PresenceService) -> (Uuid, Uuid) {
    let first = Uuid::from_u128(1);
    let first_lock = service.session_writes.for_key(first);
    let second = (2..65536)
        .map(Uuid::from_u128)
        .find(|id| !std::ptr::eq(first_lock, service.session_writes.for_key(*id)))
        .expect("two independent session stripes");
    (first, second)
}

#[tokio::test]
async fn user_replacement_removes_every_old_session_after_a_report() {
    let (store, service) = fixture(false);
    let (old, new) = separate_sessions(&service);
    let other = Uuid::new_v4();
    for session in [other, old] {
        assert!(
            store
                .store(row("user".into(), session, Uuid::nil()))
                .await
                .unwrap()
        );
        assert!(service.get_agent(session).await.unwrap().is_some());
    }
    let (report, login, _) = store
        .interleave(
            After::Report,
            service.report_agent(old, Uuid::new_v4()),
            service.login_agent("user", new, Uuid::new_v4()),
        )
        .await;
    assert!(report.unwrap() && login.unwrap());
    for session in [old, other] {
        assert!(store.get(session).await.unwrap().is_none());
        assert!(service.get_agent(session).await.unwrap().is_none());
    }
    assert!(service.get_agent(new).await.unwrap().is_some());
}

#[tokio::test]
async fn two_logins_of_one_user_store_only_the_replacement() {
    let (store, service) = fixture(false);
    let (old, new) = separate_sessions(&service);
    let (first, second, _) = store
        .interleave(
            After::UserLookup,
            service.login_agent("user", old, Uuid::new_v4()),
            service.login_agent("user", new, Uuid::new_v4()),
        )
        .await;
    assert!(first.unwrap() && second.unwrap());
    let rows = store.get_by_field("UserID", "user").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, new);
    assert!(service.get_agent(old).await.unwrap().is_none());
    assert!(service.get_agent(new).await.unwrap().is_some());
}
