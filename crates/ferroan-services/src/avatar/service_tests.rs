// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{AvatarData, AvatarHttp, AvatarService, AvatarStore, StoreError};
use axum::body::{Body, to_bytes};
use axum::http::{Request, header};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use uuid::Uuid;

pub(super) struct CopiedStore(PathBuf);

impl CopiedStore {
    pub(super) fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("ferroan-avatar-http-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        std::fs::copy(fixtures.join("db/sqlite/Avatar.db"), dir.join("Avatar.db")).unwrap();
        Self(dir)
    }

    pub(super) fn path(&self) -> PathBuf {
        self.0.join("Avatar.db")
    }
}

impl Drop for CopiedStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn avatar_http_matches_pin8_server_replies() {
    let cases = observations("service.json", 47);
    let copy = CopiedStore::new();
    let pool = ferroan_db::open_sqlite_file(&copy.path(), &["Avatar"])
        .await
        .unwrap();
    let db = pool.sqlite().unwrap();
    sqlx::query("CREATE TABLE RefusedStore (Name TEXT)")
        .execute(db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER refuse_store BEFORE INSERT ON Avatars WHEN NEW.Name IN (SELECT Name FROM RefusedStore) BEGIN SELECT RAISE(IGNORE); END").execute(db).await.unwrap();
    let mut config = ferroan_config::Config::default();
    config.set(
        "AvatarService",
        "LocalServiceModule",
        "OpenSim.Services.AvatarService.dll:AvatarService",
    );
    config.set(
        "DatabaseService",
        "StorageProvider",
        "OpenSim.Data.SQLite.dll",
    );
    config.set(
        "AvatarService",
        "ConnectionString",
        &format!("Data Source={}", copy.path().display()),
    );
    let router = AvatarHttp::open(&config).await.unwrap().into_router();
    for case in cases {
        sqlx::query("DELETE FROM RefusedStore")
            .execute(db)
            .await
            .unwrap();
        if let Some(name) = case["Refuse"].as_str() {
            sqlx::query("INSERT INTO RefusedStore VALUES (?)")
                .bind(name)
                .execute(db)
                .await
                .unwrap();
        }
        compare_reply(&router, &case).await;
    }
    drop(router);
    pool.close().await;
}

fn observations(file: &str, count: usize) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/avatar")
        .join(file);
    let mut fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        fixture["pin"],
        ferroan_http::REFERENCE_TRANQUILLITY_REVISION
    );
    let Value::Array(cases) = fixture["observations"].take() else {
        panic!("expected observations")
    };
    assert_eq!(cases.len(), count);
    cases
}

async fn compare_reply(router: &axum::Router, case: &Value) {
    let label = case["Label"].as_str().unwrap();
    let mut post =
        Request::post("/avatar").header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if case["Shard"].as_bool().unwrap() {
        post = post.header("X-SecondLife-Shard", "fixture");
    }
    let response = router
        .clone()
        .oneshot(
            post.body(Body::from(case["Body"].as_str().unwrap().to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        u64::from(response.status().as_u16()),
        case["Status"].as_u64().unwrap(),
        "{label}"
    );
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap()),
        case["ContentType"].as_str(),
        "{label}"
    );
    let body = to_bytes(response.into_body(), ferroan_http::MAX_QUERY_BYTES)
        .await
        .unwrap();
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        case["Reply"].as_str().unwrap(),
        "{label}"
    );
}

#[tokio::test]
async fn mysql_failed_appearance_and_delete_match_pin8_replies() {
    let Ok(url) = std::env::var("FERROAN_MYSQL_URL") else {
        eprintln!("FERROAN_MYSQL_URL unset; MySQL avatar HTTP replay skipped");
        return;
    };
    let pool = ferroan_db::Pool::connect_sqlx_url(&url).await.unwrap();
    pool.require_stores(&["Avatar"]).await.unwrap();
    let db = pool.mysql().unwrap().clone();
    let realm = format!("AvatarHttp_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE TABLE {realm} LIKE Avatars"))
        .execute(&db)
        .await
        .unwrap();
    let store = AvatarStore::new(pool, &realm).unwrap();
    let router = AvatarHttp::new(AvatarService::new(store), true).into_router();
    let cases = observations("service-mysql.json", 5);
    let (failed_delete, appearance_cases) = cases.split_last().unwrap();
    assert_eq!(failed_delete["Label"], "failed-delete");
    for case in appearance_cases {
        compare_reply(&router, case).await;
    }
    sqlx::query(&format!("DROP TABLE {realm}"))
        .execute(&db)
        .await
        .unwrap();
    compare_reply(&router, failed_delete).await;

    // The same request fails if the store cannot acquire a connection.
    db.close().await;
    let mut closed = failed_delete.clone();
    closed["Label"] = "closed-pool-delete".into();
    closed["Reply"] = std::str::from_utf8(crate::griduser::FAILURE_XML)
        .unwrap()
        .into();
    compare_reply(&router, &closed).await;
}

#[tokio::test]
async fn oversized_stored_value_can_be_replaced_without_reading_it() {
    let copy = CopiedStore::new();
    let pool = ferroan_db::open_sqlite_file(&copy.path(), &["Avatar"])
        .await
        .unwrap();
    let db = pool.sqlite().unwrap().clone();
    let store = AvatarStore::open(pool).unwrap();
    let id = Uuid::new_v4();
    store
        .upsert(
            &id.to_string(),
            "obsolete",
            &"x".repeat(ferroan_http::MAX_QUERY_BYTES),
        )
        .await
        .unwrap();
    let service = AvatarService::new(store);
    assert!(matches!(
        service.get_avatar(id).await,
        Err(StoreError::Limit)
    ));
    let replacement = AvatarData {
        avatar_type: 3,
        fields: vec![("Serial".into(), "5".into())],
    };
    assert!(service.set_avatar(id, &replacement).await.unwrap());
    assert_eq!(service.get_avatar(id).await.unwrap(), replacement);
    db.close().await;
}

#[tokio::test]
async fn avatar_reset_recovers_a_principal_above_the_stored_row_limit() {
    let copy = CopiedStore::new();
    let pool = ferroan_db::open_sqlite_file(&copy.path(), &["Avatar"])
        .await
        .unwrap();
    let db = pool.sqlite().unwrap().clone();
    let id = Uuid::new_v4();
    sqlx::query(
        "WITH RECURSIVE seq(n) AS (
            SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?
        ) INSERT INTO Avatars (PrincipalID, Name, Value) SELECT ?, 'key' || n, 'v' FROM seq",
    )
    .bind((ferroan_http::MAX_FIELDS + 1) as i64)
    .bind(id.to_string())
    .execute(&db)
    .await
    .unwrap();
    let service = AvatarService::new(AvatarStore::open(pool).unwrap());
    assert!(matches!(
        service.get_avatar(id).await,
        Err(StoreError::Limit)
    ));
    assert!(service.reset_avatar(id).await.unwrap());
    assert_eq!(service.get_avatar(id).await.unwrap().avatar_type, 1);
    db.close().await;
}
