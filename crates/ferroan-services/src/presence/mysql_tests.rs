// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;

use ferroan_db::{Backend, Pool};
use sqlx::MySqlPool;
use uuid::Uuid;

use super::{MAX_RESULTS, MySqlPresenceData, PresenceData, PresenceStore, PresenceStoreError};

const OLD: &str = "2001-02-03 04:05:06";

async fn set_old(pool: &MySqlPool, realm: &str, session: Uuid) {
    assert_eq!(
        sqlx::query(&format!(
            "UPDATE {realm} SET LastSeen = ? WHERE SessionID = ?"
        ))
        .bind(OLD)
        .bind(session.to_string())
        .execute(pool)
        .await
        .unwrap()
        .rows_affected(),
        1
    );
}

async fn now(pool: &MySqlPool) -> String {
    sqlx::query_scalar("SELECT CAST(NOW() AS CHAR)")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn has_current_stamp(
    pool: &MySqlPool,
    store: &dyn PresenceStore,
    session: Uuid,
    since: &str,
) {
    let row = store.get(session).await.unwrap().unwrap();
    let stamp = &row.data["LastSeen"];
    assert!(
        since <= stamp.as_str() && stamp <= &now(pool).await,
        "expected database time, got {stamp}"
    );
}

#[tokio::test]
async fn mysql_presence_plugins_cross_read() {
    if std::env::var_os("FERROAN_PRESENCE_CROSS").is_none() {
        eprintln!(
            "FERROAN_PRESENCE_CROSS is unset; run cargo xtask presence cross mysql for the Pin8 comparison"
        );
        return;
    }
    let dll = std::env::var_os("FERROAN_PRESENCE_ORACLE").unwrap();
    let connection = std::env::var("FERROAN_PRESENCE_ADO_MYSQL")
        .expect("set FERROAN_PRESENCE_ADO_MYSQL for a disposable MySQL database");
    // Read the server clock before the C# constructor migrates the DB and writes.
    let db = Pool::connect(Backend::Mysql, &connection).await.unwrap();
    let pool = db.mysql().unwrap();
    cross_realm(&db, pool, &dll, false).await;
    cross_realm(&db, pool, &dll, true).await;
    db.close().await;
}

async fn cross_realm(db: &Pool, pool: &MySqlPool, dll: &std::ffi::OsStr, custom: bool) {
    // Include at least one hex letter for the oracle's uppercase secure ID.
    let base = (Uuid::new_v4().as_u128() & !0xfff) | 0xa00;
    let id = |suffix| Uuid::from_u128(base | suffix);
    let (user, first, second, secure, home, away, new_secure) =
        (id(1), id(2), id(3), id(4), id(5), id(6), id(7));
    let mode = if custom { "alternate" } else { "default" };
    let realm = if custom {
        format!("PresenceGridB{}", id(0).simple())
    } else {
        "Presence".to_owned()
    };
    if custom {
        sqlx::query(&format!("CREATE TABLE {realm} LIKE Presence"))
            .execute(pool)
            .await
            .unwrap();
    }
    let oracle = |phase| {
        let prefix = id(0).to_string();
        let output = Command::new("dotnet")
            .arg(dll)
            .args(["mysql", phase, &prefix[..34], mode])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let check = stderr
            .lines()
            .find(|line| line.starts_with("Presence check failed:"))
            .unwrap_or(phase);
        assert!(
            output.status.success(),
            "Pin8 MySQL Presence {check}: {}",
            output.status
        );
    };

    let before = now(pool).await;
    oracle("seed");
    db.require_stores(&["Presence"]).await.unwrap();
    let default_store = MySqlPresenceData::open(pool.clone()).unwrap();
    let store = if custom {
        assert!(MySqlPresenceData::new(pool.clone(), "invalid realm").is_err());
        assert!(default_store.get(first).await.unwrap().is_none());
        MySqlPresenceData::new(pool.clone(), &realm).unwrap()
    } else {
        default_store
    };
    assert!(matches!(
        store.get(id(9)).await,
        Err(PresenceStoreError::Database(
            sqlx::Error::ColumnDecode { .. }
        ))
    ));
    assert!(matches!(
        store.report_agent(id(9), away).await,
        Err(PresenceStoreError::Database(
            sqlx::Error::ColumnDecode { .. }
        ))
    ));
    assert!(store.delete("SessionID", &id(9).to_string()).await.unwrap());
    let row = store.get(first).await.unwrap().unwrap();
    assert_eq!(row.user_id, user.to_string());
    assert_eq!(row.region_id(), home);
    assert_eq!(row.data["SecureSessionID"], secure.to_string());
    has_current_stamp(pool, &store, first, &before).await;
    let sessions = store
        .get_by_field("UserID", &user.to_string())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().any(
        |row| row.session_id == second && row.data["SecureSessionID"] == new_secure.to_string()
    ));
    assert!(store.verify_agent(user, secure).await.unwrap());
    assert!(!store.verify_agent(away, secure).await.unwrap());
    assert!(!store.verify_agent(user, id(8)).await.unwrap());

    set_old(pool, &realm, first).await;
    let before = now(pool).await;
    oracle("report");
    has_current_stamp(pool, &store, first, &before).await;
    assert_eq!(store.get(first).await.unwrap().unwrap().region_id(), away);
    oracle("restore");
    // A distinct historical second makes replacing LastSeen with NOW() observable.
    set_old(pool, &realm, first).await;
    let fetched = store.get(first).await.unwrap().unwrap();
    assert_eq!(fetched.data["LastSeen"], OLD);
    fetched.set_region_id(home);
    assert!(store.store(fetched).await.unwrap());

    assert!(
        store
            .delete("SecureSessionID", &new_secure.to_string())
            .await
            .unwrap()
    );
    assert!(
        !store
            .delete("SessionID", &second.to_string())
            .await
            .unwrap()
    );
    assert!(!store.report_agent(second, home).await.unwrap());
    let fresh = |region, data| Arc::new(PresenceData::new(user.to_string(), second, region, data));
    let before = now(pool).await;
    assert!(
        store
            .store(fresh(
                home,
                BTreeMap::from([("SecureSessionID".into(), new_secure.to_string())])
            ))
            .await
            .unwrap()
    );
    has_current_stamp(pool, &store, second, &before).await;
    assert!(store.store(fresh(home, BTreeMap::new())).await.unwrap());
    let invalid = fresh(
        away,
        BTreeMap::from([("LastSeen".into(), "invalid".into())]),
    );
    assert!(!store.store(invalid).await.unwrap());
    let held = store.get(second).await.unwrap().unwrap();
    assert_eq!(held.data["SecureSessionID"], Uuid::nil().to_string());
    assert!(!store.report_agent(second, Uuid::nil()).await.unwrap());
    assert_eq!(store.get(second).await.unwrap().unwrap().region_id(), home);
    assert!(store.report_agent(second, away).await.unwrap());
    assert_eq!(held.region_id(), home);
    assert_eq!(store.get(second).await.unwrap().unwrap().region_id(), away);
    // Even a report to the same region must explicitly stamp LastSeen.
    set_old(pool, &realm, second).await;
    let before = now(pool).await;
    assert!(store.report_agent(second, away).await.unwrap());
    has_current_stamp(pool, &store, second, &before).await;
    oracle("read-rust");
    assert!(store.get(first).await.unwrap().is_none());
    assert!(store.get(second).await.unwrap().is_some());
    store.logout_region_agents(away).await.unwrap();
    oracle("after-logout");
    if custom {
        sqlx::query(&format!("DROP TABLE {realm}"))
            .execute(pool)
            .await
            .unwrap();
    }
    println!(
        "presence mysql: {mode} realm: both directions match; TB104 fetched re-store: Pin8 false, Ferroan true with LastSeen preserved (D84)"
    );
}

// The Linux check loads the Pin8 schema before running the workspace tests.
#[tokio::test]
async fn mysql_presence_lookup_bound_allows_bulk_removal() {
    let Ok(url) = std::env::var("FERROAN_MYSQL_URL") else {
        eprintln!("FERROAN_MYSQL_URL is unset; MySQL Presence lookup bound was not tested");
        return;
    };
    let db = Pool::connect_sqlx_url(&url).await.unwrap();
    db.require_stores(&["Presence"]).await.unwrap();
    let pool = db.mysql().unwrap();
    let store = MySqlPresenceData::open(pool.clone()).unwrap();
    let user = Uuid::new_v4().to_string();
    let region = Uuid::new_v4();
    let mut insert = sqlx::QueryBuilder::new("INSERT INTO Presence (UserID, SessionID, RegionID) ");
    insert.push_values(0..MAX_RESULTS, |mut values, _| {
        values
            .push_bind(&user)
            .push_bind(Uuid::new_v4().to_string())
            .push_bind(region.to_string());
    });
    insert.build().execute(pool).await.unwrap();
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
    assert!(store.store(last).await.unwrap());
    assert!(
        matches!(store.get_by_field("UserID", &user).await, Err(PresenceStoreError::Limit(limit)) if limit.observed == MAX_RESULTS + 1)
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
