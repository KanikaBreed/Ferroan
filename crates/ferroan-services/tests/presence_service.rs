// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Replay the HTTP sequence in its own process. Null shares sessions across instances.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::{Body, to_bytes};
use axum::http::{Request, header};
use ferroan_config::Config;
use ferroan_db::{Backend, Pool};
use ferroan_services::PresenceHttp;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

fn config(provider: &str) -> Config {
    let mut config = Config::default();
    config.set(
        "PresenceService",
        "LocalServiceModule",
        "OpenSim.Services.PresenceService.dll:PresenceService",
    );
    config.set("DatabaseService", "StorageProvider", provider);
    config
}

async fn replay(config: &Config, data: &str) {
    let fixture: Value = serde_json::from_str(data).unwrap();
    let http = PresenceHttp::open(config).await.unwrap();
    let router = http.clone().into_router();
    for step in fixture["steps"].as_array().unwrap() {
        if let Some(user) = step["login"].as_str() {
            let id = |key: &str| step[key].as_str().unwrap().parse().unwrap();
            assert_eq!(
                http.service()
                    .login_agent(user, id("session"), id("secure"))
                    .await
                    .unwrap(),
                step["result"].as_bool().unwrap()
            );
            continue;
        }
        let mut request = Request::post("/presence")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if step["shard"].as_bool().unwrap() {
            request = request.header("X-SecondLife-Shard", "fixture");
        }
        let response = router
            .clone()
            .oneshot(
                request
                    .body(Body::from(step["body"].as_str().unwrap().to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let label = &step["label"];
        assert_eq!(
            response.status().as_u16() as u64,
            step["status"].as_u64().unwrap(),
            "{label}"
        );
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            step["contentType"].as_str().unwrap(),
            "{label}"
        );
        let body = to_bytes(response.into_body(), ferroan_http::MAX_QUERY_BYTES)
            .await
            .unwrap();
        let hex: String = body.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, step["reply"].as_str().unwrap(), "{label}");
    }
}

#[tokio::test]
async fn null_presence_http_matches_pin8() {
    let mut cfg = config("OpenSim.Data.Null.dll");
    cfg.set("PresenceService", "AllowDuplicatePresences", "true");
    replay(
        &cfg,
        include_str!("../../../fixtures/presence/service.json"),
    )
    .await;
}

#[tokio::test]
async fn postgres_presence_http_matches_pin8() {
    let Ok(connection) = std::env::var("FERROAN_PRESENCE_ADO_POSTGRES") else {
        eprintln!("FERROAN_PRESENCE_ADO_POSTGRES unset; PostgreSQL HTTP comparison skipped");
        return;
    };
    let db = Pool::open(Backend::Postgres, &connection, &["Presence"])
        .await
        .unwrap();
    let pg = db.postgres().unwrap();
    let realm = format!("presencehttp{}", Uuid::new_v4().simple());
    sqlx::query(&format!(
        "CREATE TABLE {realm} (LIKE presence INCLUDING ALL)"
    ))
    .execute(pg)
    .await
    .unwrap();
    let mut cfg = config("OpenSim.Data.Null.dll");
    cfg.set(
        "PresenceService",
        "StorageProvider",
        "OpenSim.Data.PGSQL.dll",
    );
    cfg.set("PresenceService", "ConnectionString", &connection);
    cfg.set("PresenceService", "Realm", &realm);
    // A fresh SQL login takes the table defaults (including LastSeen). The
    // report after a zero-region refusal distinguishes this store from Null.
    replay(
        &cfg,
        include_str!("../../../fixtures/presence/service-postgres.json"),
    )
    .await;
    // Distinguish an omitted LastSeen from one explicitly supplied as NULL.
    sqlx::query(&format!(
        r#"ALTER TABLE {realm} ALTER COLUMN "LastSeen" SET DEFAULT '2001-02-03 04:05:06'"#
    ))
    .execute(pg)
    .await
    .unwrap();
    let http = PresenceHttp::open(&cfg).await.unwrap();
    let session = Uuid::new_v4();
    let secure = Uuid::new_v4();
    let user = Uuid::new_v4().to_string();
    assert!(
        http.service()
            .login_agent(&user, session, secure)
            .await
            .unwrap()
    );
    let saved: (bool, Uuid) = sqlx::query_as(&format!(
        r#"SELECT "LastSeen"=timestamp '2001-02-03 04:05:06',"SecureSessionID" FROM {realm} WHERE "SessionID"=$1"#
    ))
    .bind(session)
    .fetch_one(pg)
    .await
    .unwrap();
    assert_eq!(saved, (true, secure));
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/presence/service-postgres.json"
    ))
    .unwrap();
    sqlx::query(&format!(
        "CREATE RULE refuse_insert AS ON INSERT TO {realm} DO INSTEAD NOTHING"
    ))
    .execute(pg)
    .await
    .unwrap();
    let refused = Uuid::new_v4();
    assert_eq!(
        fixture["refusedStoreLogin"], true,
        "TB126: Pin8 local result on a refused insert"
    );
    assert!(
        !http
            .service()
            .login_agent(&Uuid::new_v4().to_string(), refused, secure)
            .await
            .unwrap()
    );
    assert!(http.service().get_agent(refused).await.unwrap().is_none());
    sqlx::query(&format!("DROP TABLE {realm}"))
        .execute(pg)
        .await
        .unwrap();
    assert!(
        !http
            .service()
            .report_agent(session, Uuid::new_v4())
            .await
            .unwrap()
    );
    assert!(http.service().get_agent(session).await.unwrap().is_some());
    drop(http);
    db.close().await;
}

#[tokio::test]
async fn presence_open_validates_transport_and_allows_a_null_only_standalone() {
    let cfg = config("OpenSim.Data.Null.dll");
    assert_eq!(
        PresenceHttp::open(&cfg).await.unwrap().pool_occupancy(),
        (0, 0)
    );
    for (section, key, value) in [
        ("PresenceService", "LocalServiceModule", "Unknown"),
        (
            "PresenceService",
            "StorageProvider",
            "OpenSim.Data.SQLite.dll",
        ),
        ("PresenceService", "AllowDuplicatePresences", "invalid"),
        ("Network", "https_main", "true"),
        ("PresenceService", "AuthType", "BasicHttpAuthentication"),
    ] {
        let mut cfg = config("OpenSim.Data.Null.dll");
        cfg.set(section, key, value);
        assert!(PresenceHttp::open(&cfg).await.is_err(), "{section} {key}");
    }
}

#[tokio::test]
async fn mysql_service_honors_the_configured_realm() {
    let Ok(connection) = std::env::var("FERROAN_PRESENCE_ADO_MYSQL") else {
        eprintln!("FERROAN_PRESENCE_ADO_MYSQL unset; MySQL service realm test skipped");
        return;
    };
    let db = Pool::open(Backend::Mysql, &connection, &["Presence"])
        .await
        .unwrap();
    let mysql = db.mysql().unwrap();
    let realm = format!("PresenceHTTP{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE TABLE {realm} LIKE Presence"))
        .execute(mysql)
        .await
        .unwrap();
    let mut cfg = config("OpenSim.Data.MySQL.dll");
    cfg.set("DatabaseService", "ConnectionString", &connection);
    cfg.set("PresenceService", "Realm", &realm);
    let http = PresenceHttp::open(&cfg).await.unwrap();
    let user = Uuid::new_v4().to_string();
    let session = Uuid::new_v4();
    let secure = Uuid::new_v4();
    let region = Uuid::new_v4();
    assert!(
        http.service()
            .login_agent(&user, session, secure)
            .await
            .unwrap()
    );
    assert!(http.service().report_agent(session, region).await.unwrap());
    let stored: (String, String, String) = sqlx::query_as(&format!(
        "SELECT UserID,RegionID,SecureSessionID FROM {realm} WHERE SessionID=?"
    ))
    .bind(session.to_string())
    .fetch_one(mysql)
    .await
    .unwrap();
    assert_eq!(stored, (user, region.to_string(), secure.to_string()));
    assert!(http.service().logout_agent(session).await.unwrap());
    drop(http);
    sqlx::query(&format!("DROP TABLE {realm}"))
        .execute(mysql)
        .await
        .unwrap();
    db.close().await;
}
