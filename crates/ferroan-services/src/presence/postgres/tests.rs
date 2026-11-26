// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::process::Command;

use super::*;
use ferroan_db::{Backend, Pool};

async fn timestamp(pool: &PgPool, realm: &str, session: Uuid) -> Option<String> {
    sqlx::query_scalar(&format!(
        r#"SELECT "LastSeen"::text FROM {realm} WHERE "SessionID" = $1"#
    ))
    .bind(session)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn db_clock(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT localtimestamp::text")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn postgres_presence_plugins_cross_read() {
    if std::env::var_os("FERROAN_PRESENCE_PG_CROSS").is_none() {
        eprintln!(
            "FERROAN_PRESENCE_PG_CROSS is unset; run cargo xtask presence cross postgres for the Pin8 comparison"
        );
        return;
    }
    let dll = std::env::var_os("FERROAN_PRESENCE_ORACLE").unwrap();
    let connection = std::env::var("FERROAN_PRESENCE_ADO_POSTGRES")
        .expect("set FERROAN_PRESENCE_ADO_POSTGRES for a disposable PostgreSQL database");
    let db = Pool::connect(Backend::Postgres, &connection).await.unwrap();
    cross_realm(&db, &dll, false).await;
    cross_realm(&db, &dll, true).await;
    db.close().await;
}

async fn cross_realm(db: &Pool, dll: &std::ffi::OsStr, custom: bool) {
    let pool = db.postgres().unwrap();
    let base = Uuid::new_v4().as_u128() & !0xff;
    let id = |n| Uuid::from_u128(base | n);
    let (user, first, second, third, secure, home, away, staging, third_secure) = (
        id(1),
        id(2),
        id(3),
        id(4),
        id(5),
        id(6),
        id(7),
        id(8),
        id(9),
    );
    let ids = id(0).to_string();
    let mode = if custom { "alternate" } else { "default" };
    let realm = if custom {
        format!("PresenceGridB{}", id(0).simple())
    } else {
        "Presence".to_owned()
    };
    if custom {
        sqlx::query(&format!(
            "CREATE TABLE {realm} (LIKE presence INCLUDING ALL)"
        ))
        .execute(pool)
        .await
        .unwrap();
    }
    let oracle = |phase, stamp: &str| {
        let output = Command::new("dotnet")
            .arg(dll)
            .args(["postgres", phase, &ids[..34], stamp, mode])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let step = stderr
            .lines()
            .find(|line| line.starts_with("Presence check failed:"))
            .unwrap_or(phase);
        assert!(
            output.status.success(),
            "Pin8 PostgreSQL {step}: {}",
            output.status
        );
    };

    oracle("seed", "");
    db.require_stores(&["Presence"]).await.unwrap();
    let default_store = PgPresenceData::open(pool.clone()).unwrap();
    let store = if custom {
        assert!(PgPresenceData::new(pool.clone(), "invalid realm").is_err());
        assert!(default_store.get(first).await.unwrap().is_none());
        PgPresenceData::new(pool.clone(), &realm).unwrap()
    } else {
        default_store
    };
    let partial = |session, region, data| {
        Arc::new(PresenceData::new(user.to_string(), session, region, data))
    };

    let fresh = store.get(first).await.unwrap().unwrap();
    assert_eq!(fresh.region_id(), staging);
    assert_eq!(fresh.data["LastSeen"], "");
    assert_eq!(fresh.data["SecureSessionID"], secure.to_string());
    assert_eq!(timestamp(pool, &realm, first).await, None);
    assert_eq!(
        store
            .get_by_field("UserID", &user.to_string())
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        store
            .get_by_field("SecureSessionID", "invalid")
            .await
            .unwrap()
            .iter()
            .any(|r| r.session_id == second)
    );
    assert!(store.get_by_field("LastSeen", "").await.is_err());
    assert!(store.delete("LastSeen", "").await.is_err());
    assert!(store.verify_agent(user, secure).await.unwrap());
    assert!(!store.verify_agent(away, secure).await.unwrap());
    assert!(!store.verify_agent(user, third_secure).await.unwrap());
    assert!(
        store
            .store(partial(
                third,
                staging,
                BTreeMap::from([("SecureSessionID".into(), id(10).to_string())])
            ))
            .await
            .unwrap()
    );
    assert!(
        store
            .store(partial(
                third,
                home,
                BTreeMap::from([("SecureSessionID".into(), third_secure.to_string())])
            ))
            .await
            .unwrap()
    );

    let before = db_clock(pool).await;
    oracle("report", "");
    let reported = store.get(first).await.unwrap().unwrap();
    let reported_time = timestamp(pool, &realm, first).await.unwrap();
    assert!((before..=db_clock(pool).await).contains(&reported_time));
    let exact = reported_time;
    assert_eq!(reported.region_id(), away);
    assert_eq!(reported.data["LastSeen"], exact);

    // Omitted, explicitly NULL, and supplied timestamp all produce different
    // updates. The two held rows come from different reads of one session.
    assert!(
        store
            .store(partial(first, staging, BTreeMap::new()))
            .await
            .unwrap()
    );
    oracle("partial", &exact);
    assert!(store.store(fresh).await.unwrap());
    oracle("restored-null", "");
    assert!(store.store(Arc::clone(&reported)).await.unwrap());
    oracle("restored-in-place", &exact);
    assert!(store.delete("SessionID", &first.to_string()).await.unwrap());
    assert!(store.store(reported).await.unwrap());
    assert!(!store.report_agent(second, Uuid::nil()).await.unwrap());
    assert!(!store.report_agent(id(10), away).await.unwrap());
    assert_eq!(timestamp(pool, &realm, second).await, None);
    let before = db_clock(pool).await;
    assert!(store.report_agent(second, away).await.unwrap());
    assert!(
        (before..=db_clock(pool).await).contains(&timestamp(pool, &realm, second).await.unwrap())
    );
    oracle("restored-report", &exact);
    assert!(store.get(third).await.unwrap().is_none());
    assert_eq!(
        store
            .get_by_field("RegionID", &away.to_string())
            .await
            .unwrap()
            .len(),
        2
    );
    let matching = store.get_by_field("LastSeen", &exact).await.unwrap();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].session_id, first);
    assert!(store.delete("LastSeen", &exact).await.unwrap());
    assert!(!store.delete("SessionID", &first.to_string()).await.unwrap());
    assert!(store.get(second).await.unwrap().is_some());
    store.logout_region_agents(away).await.unwrap();
    oracle("after-logout", "");
    if custom {
        sqlx::query(&format!("DROP TABLE {realm}"))
            .execute(pool)
            .await
            .unwrap();
    }
    println!(
        "presence postgres: {mode} realm: both directions match; TB105: Pin8 InvalidCastException, Ferroan restores SQL NULL and the reported microseconds (proposed D84)"
    );
    println!(
        "TB114: Pin8 InvalidCastException on LastSeen get/delete, Ferroan matches by typed timestamp (proposed D89)"
    );
}

#[tokio::test]
async fn postgres_presence_bounds_lookup_but_logs_out_the_whole_region() {
    let Ok(url) = std::env::var("FERROAN_POSTGRES_URL") else {
        eprintln!("FERROAN_POSTGRES_URL is unset; PostgreSQL Presence lookup bound was not tested");
        return;
    };
    let db = Pool::connect_sqlx_url(&url).await.unwrap();
    db.require_stores(&["Presence"]).await.unwrap();
    let pool = db.postgres().unwrap();
    let store = PgPresenceData::open(pool.clone()).unwrap();
    let region = Uuid::new_v4();
    let user = Uuid::new_v4().to_string();
    let mut inserts =
        QueryBuilder::new(r#"INSERT INTO presence ("UserID", "SessionID", "RegionID") "#);
    inserts.push_values(0..MAX_RESULTS, |mut row, _| {
        row.push_bind(&user)
            .push_bind(Uuid::new_v4())
            .push_bind(region);
    });
    inserts.build().execute(pool).await.unwrap();
    assert_eq!(
        store.get_by_field("UserID", &user).await.unwrap().len(),
        MAX_RESULTS
    );
    let last = Arc::new(PresenceData::new(
        user.clone(),
        Uuid::new_v4(),
        region,
        BTreeMap::new(),
    ));
    let session = last.session_id;
    assert!(store.store(last).await.unwrap());
    let default_row = store.get(session).await.unwrap().unwrap();
    assert_eq!(default_row.data["SecureSessionID"], Uuid::nil().to_string());
    assert!(default_row.data["LastSeen"].is_empty());
    // PostgreSQL timestamps include values outside a Rust or .NET datetime.
    sqlx::query(r#"UPDATE presence SET "LastSeen"='infinity' WHERE "SessionID"=$1"#)
        .bind(session)
        .execute(pool)
        .await
        .unwrap();
    let infinite = store.get(session).await.unwrap().unwrap();
    assert_eq!(infinite.data["LastSeen"], "infinity");
    assert!(store.store(infinite).await.unwrap());
    assert_eq!(
        timestamp(pool, "presence", session).await.as_deref(),
        Some("infinity")
    );
    assert!(
        matches!(store.get_by_field("UserID", &user).await, Err(PresenceStoreError::Limit(e)) if e.observed == MAX_RESULTS + 1)
    );
    store.logout_region_agents(region).await.unwrap();
    assert!(
        store
            .get_by_field("UserID", &user)
            .await
            .unwrap()
            .is_empty()
    );
    db.close().await;
}
