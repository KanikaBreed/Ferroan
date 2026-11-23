// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Plugin behaviour of the three asset stores.
//!
//! SQLite always runs, on a copy of the committed AssetStore fixture.
//! MySQL and PostgreSQL run when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL`
//! is set, and skip with a printed reason otherwise. Those URLs point at a
//! database Tranquillity's `Migration.Update` already migrated.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ferroan_db::{Backend, Pool};
use uuid::Uuid;

use super::super::model::Asset;
use super::super::time::{mysql_unix_now, postgres_unix_now};
use super::AssetStore;

static NEXT: AtomicU64 = AtomicU64::new(1);

#[tokio::test]
async fn plugin_store_fetch_metadata_exists_and_delete() {
    let (store, path) = open_sqlite().await;
    check_plugin(&store).await;
    store.close().await;
    let _ = std::fs::remove_dir_all(path.parent().expect("temp dir"));

    if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
        let _guard = super::ASSET_MYSQL.lock().await;
        check_plugin(&store).await;
        store.close().await;
    }
    if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
        let _guard = super::ASSET_POSTGRES.lock().await;
        check_plugin(&store).await;
        store.close().await;
    }
}

async fn check_plugin(store: &AssetStore) {
    wipe(store).await;
    let backend = store.backend();
    let id = asset_id(1);
    let mut created = sample(id, "first", 8);
    created.local = true;
    created.temporary = true;
    created.data = vec![9, 8, 7];
    store.store_data(&created).await.expect("store");

    let loaded = store.get(id).await.expect("get").expect("row");
    assert_eq!(loaded.name, "first");
    assert_eq!(loaded.description, "d");
    assert_eq!(loaded.type_code, 1);
    assert!(loaded.local);
    assert!(loaded.temporary);
    assert_eq!(loaded.creator_id, "creator");
    assert_eq!(loaded.flags, 0, "known-bit mask drops flag 8");
    assert_eq!(loaded.data, vec![9, 8, 7]);
    assert_eq!(loaded.upload_attempts, 0);
    let clock = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(clock.raw_flags, 8);

    let mut again = sample(id, "second", 1);
    store.store_new(&again).await.expect("service store");
    let kept = store.get(id).await.expect("get").expect("row");
    assert_eq!(kept.name, "first", "AssetService.Store does not rewrite");

    again.name = "rewritten".to_string();
    again.flags = 2;
    again.data = vec![4];
    set_clock(store, id).await;
    let before = store.clock(id).await.expect("clock").expect("row");
    if backend != Backend::Sqlite {
        assert_eq!(before.create_time, Some(1));
        assert_eq!(before.access_time, Some(1));
    }
    let _ = store.get(id).await.expect("get");
    let after_get = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(after_get.access_time, before.access_time);
    assert_eq!(after_get.raw_flags, 8);

    store.store_data(&again).await.expect("plugin store");
    let rewritten = store.get(id).await.expect("get").expect("row");
    assert_eq!(rewritten.name, "rewritten");
    assert_eq!(rewritten.data, vec![4]);
    let after = store.clock(id).await.expect("clock").expect("row");
    match backend {
        Backend::Sqlite => {
            assert_eq!(rewritten.flags, 2);
            assert_eq!(after.create_time, None);
            assert_eq!(after.raw_flags, 2);
        }
        Backend::Mysql => {
            assert_eq!(rewritten.flags, 2, "REPLACE rewrites flags");
            let now = mysql_unix_now();
            let create = after.create_time.expect("create");
            let access = after.access_time.expect("access");
            assert!(
                (create - now).abs() < 120,
                "mysql create {create} now {now}"
            );
            assert!(
                (access - now).abs() < 120,
                "mysql access {access} now {now}"
            );
            assert_eq!(after.raw_flags, 2, "REPLACE rewrites flags");
        }
        Backend::Postgres => {
            assert_eq!(
                rewritten.flags, 0,
                "UPDATE leaves flags 8, which read back as 0"
            );
            assert_eq!(after.create_time, Some(1));
            assert_eq!(after.access_time, Some(1));
            assert_eq!(after.raw_flags, 8, "UPDATE does not touch flags");
            let now = postgres_unix_now();
            assert!((now - 1).abs() > 120);
        }
    }

    let long = sample(asset_id(2), &"n".repeat(80), 1);
    store.store_data(&long).await.expect("long name");
    let stored = store.get(asset_id(2)).await.expect("get").expect("row");
    assert_eq!(stored.name.chars().count(), 64);

    assert!(store.exists(&[id, asset_id(9)]).await.expect("exists") == vec![true, false]);

    assert!(store.delete_data(&id.to_string()).await.expect("delete"));
    assert!(store.get(id).await.expect("get").is_none());
    let missing = "00000000-0000-0000-0000-000000000099";
    match backend {
        Backend::Sqlite => {
            assert!(store.delete_data(missing).await.expect("sqlite missing"));
            assert!(!store.delete_data("nope").await.expect("sqlite bad"));
        }
        Backend::Mysql => {
            assert!(store.delete_data(missing).await.expect("mysql missing"));
            assert!(store.delete_data("nope").await.expect("mysql bad"));
        }
        Backend::Postgres => {
            assert!(!store.delete_data(missing).await.expect("postgres missing"));
            assert!(!store.delete_data("nope").await.expect("postgres bad"));
        }
    }

    wipe(store).await;
    for n in 1..=4 {
        store
            .store_data(&sample(
                asset_id(n),
                &format!("m{n}"),
                if n == 4 { 8 } else { 1 },
            ))
            .await
            .expect("metadata row");
    }
    let page = store.metadata_set(0, 3).await.expect("metadata");
    match backend {
        Backend::Postgres => {
            assert_eq!(page.len(), 3);
            assert_eq!(page[0].id, asset_id(1));
            assert_eq!(page[1].id, asset_id(2));
            assert_eq!(page[2].id, asset_id(3));
            assert_eq!(page[0].flags, 0);
            let next = store.metadata_set(1, 3).await.expect("offset");
            assert_eq!(next.len(), 3);
            assert_eq!(next[0].id, asset_id(2));
            assert_eq!(store.metadata_set(0, 1).await.expect("one").len(), 1);
            assert!(store.metadata_set(0, 0).await.expect("zero").is_empty());
        }
        Backend::Sqlite | Backend::Mysql => {
            assert_eq!(page.len(), 3);
            let ids: std::collections::BTreeSet<_> = page.iter().map(|row| row.id).collect();
            assert_eq!(ids.len(), 3);
            assert!(ids.iter().all(|id| (1..=4).any(|n| *id == asset_id(n))));
            let flags_eight = page.iter().find(|row| row.id == asset_id(4));
            if let Some(row) = flags_eight {
                assert_eq!(row.flags, 0);
            }
            let flag_one = page.iter().find(|row| row.id == asset_id(1));
            if let Some(row) = flag_one {
                assert_eq!(row.flags, 1);
            }
            assert_eq!(store.metadata_set(1, 3).await.expect("offset").len(), 3);
            assert_eq!(store.metadata_set(0, 1).await.expect("one").len(), 1);
            assert!(store.metadata_set(0, 0).await.expect("zero").is_empty());
        }
    }
    assert!(store.metadata_set(-1, 1).await.is_err());
    assert!(store.metadata_set(0, 10_001).await.is_err());
    assert!(store.metadata_set(1_000_001, 1).await.is_err());
}

#[tokio::test]
async fn postgres_clock_matches_the_local_unix_reading() {
    let now = postgres_unix_now();
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let offset = (i64::from(now) - i64::try_from(unix).unwrap_or(0)).unsigned_abs();
    assert!(offset < 18 * 3600, "local offset {offset} seconds");
}

async fn wipe(store: &AssetStore) {
    let sql = "DELETE FROM assets";
    match store.backend() {
        Backend::Sqlite => {
            sqlx::query(sql)
                .execute(store.sqlite().expect("sqlite"))
                .await
                .expect("wipe");
        }
        Backend::Mysql => {
            sqlx::query(sql)
                .execute(store.mysql().expect("mysql"))
                .await
                .expect("wipe");
        }
        Backend::Postgres => {
            sqlx::query(sql)
                .execute(store.postgres().expect("postgres"))
                .await
                .expect("wipe");
        }
    }
}

async fn set_clock(store: &AssetStore, id: Uuid) {
    match store.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE assets SET asset_flags = 8 WHERE UUID = ?")
                .bind(id.to_string())
                .execute(store.sqlite().expect("sqlite"))
                .await
                .expect("flags");
        }
        Backend::Mysql => {
            sqlx::query(
                "UPDATE assets SET create_time = 1, access_time = 1, asset_flags = 8 WHERE id = ?",
            )
            .bind(id.to_string())
            .execute(store.mysql().expect("mysql"))
            .await
            .expect("clock");
        }
        Backend::Postgres => {
            sqlx::query(
                "UPDATE assets SET create_time = 1, access_time = 1, asset_flags = 8 WHERE id = $1::uuid",
            )
            .bind(id.to_string())
            .execute(store.postgres().expect("postgres"))
            .await
            .expect("clock");
        }
    }
}

fn sample(id: Uuid, name: &str, flags: i32) -> Asset {
    Asset {
        id,
        name: name.to_string(),
        description: "d".to_string(),
        type_code: 1,
        local: false,
        temporary: false,
        creator_id: "creator".to_string(),
        flags,
        data: vec![1],
        upload_attempts: 3,
    }
}

fn asset_id(n: u32) -> Uuid {
    Uuid::from_u128(u128::from(n))
}

#[tokio::test]
async fn csharp_and_rust_read_each_others_rows() {
    let Ok(oracle) = std::env::var("FERROAN_ASSET_ORACLE") else {
        eprintln!("FERROAN_ASSET_ORACLE is unset; C# cross-read was not run");
        return;
    };
    if oracle.is_empty() || std::env::var("FERROAN_ASSET_CROSS").ok().as_deref() != Some("1") {
        eprintln!("FERROAN_ASSET_CROSS is unset; C# cross-read was not run");
        return;
    }

    let (store, path) = open_sqlite().await;
    let ado = format!("Data Source={};Version=3;Pooling=False;", path.display());
    let backend = store.backend();
    let page = prepare_cross(&store).await;
    store.close().await;
    drive_csharp(&oracle, "sqlite", &ado, &page);
    let store = open_sqlite_at(&path).await;
    expect_csharp_writes(&store, backend).await;
    store.close().await;
    let _ = std::fs::remove_dir_all(path.parent().expect("temp dir"));

    if let Ok(ado) = std::env::var("FERROAN_ASSET_ADO_MYSQL") {
        if ado.is_empty() {
            eprintln!("FERROAN_ASSET_ADO_MYSQL is empty; MySQL cross-read was not run");
        } else {
            let _guard = super::ASSET_MYSQL.lock().await;
            if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
                let backend = store.backend();
                let page = prepare_cross(&store).await;
                store.close().await;
                drive_csharp(&oracle, "mysql", &ado, &page);
                let store = open_url("FERROAN_MYSQL_URL", "MySQL").await.expect("mysql");
                expect_csharp_writes(&store, backend).await;
                store.close().await;
            }
        }
    } else {
        eprintln!("FERROAN_ASSET_ADO_MYSQL is unset; MySQL cross-read was not run");
    }
    if let Ok(ado) = std::env::var("FERROAN_ASSET_ADO_POSTGRES") {
        if ado.is_empty() {
            eprintln!("FERROAN_ASSET_ADO_POSTGRES is empty; PostgreSQL cross-read was not run");
        } else {
            let _guard = super::ASSET_POSTGRES.lock().await;
            if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
                let backend = store.backend();
                let page = prepare_cross(&store).await;
                store.close().await;
                drive_csharp(&oracle, "postgres", &ado, &page);
                let store = open_url("FERROAN_POSTGRES_URL", "PostgreSQL")
                    .await
                    .expect("postgres");
                expect_csharp_writes(&store, backend).await;
                store.close().await;
            }
        }
    } else {
        eprintln!("FERROAN_ASSET_ADO_POSTGRES is unset; PostgreSQL cross-read was not run");
    }
}

async fn prepare_cross(store: &AssetStore) -> Vec<super::super::model::AssetMetadata> {
    wipe(store).await;
    let id = asset_id(1);
    let mut created = sample(id, "from-rust", 8);
    created.local = true;
    created.temporary = true;
    created.data = vec![9, 8, 7];
    store.store_data(&created).await.expect("rust store");
    for n in 2..=4 {
        store
            .store_data(&sample(
                asset_id(n),
                &format!("m{n}"),
                if n == 4 { 8 } else { 1 },
            ))
            .await
            .expect("row");
    }
    store.metadata_set(0, 3).await.expect("rust metadata")
}

fn drive_csharp(
    oracle: &str,
    backend: &str,
    ado: &str,
    rust_page: &[super::super::model::AssetMetadata],
) {
    let id = asset_id(1);
    let got = csharp(
        oracle,
        backend,
        ado,
        "get",
        serde_json::json!({"id": id.to_string()}),
    );
    assert_eq!(got["found"], true);
    assert_eq!(got["name"], "from-rust");
    assert_eq!(got["description"], "d");
    assert_eq!(got["type"], 1);
    assert_eq!(got["local"], true);
    assert_eq!(got["temporary"], true);
    assert_eq!(got["flags"], 0);
    assert_eq!(got["creator"], "creator");
    assert_eq!(got["data"], serde_json::json!([9, 8, 7]));

    let page = csharp(
        oracle,
        backend,
        ado,
        "metadata",
        serde_json::json!({"start": 0, "count": 3}),
    );
    let rows = page["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), rust_page.len());
    // TB19 fixed (#477): PostgreSQL returns the full page, as MySQL does.
    assert_eq!(rows.len(), 3);
    let ids: std::collections::BTreeSet<_> = rows
        .iter()
        .map(|row| row["id"].as_str().unwrap_or("").to_string())
        .collect();
    let rust_ids: std::collections::BTreeSet<_> =
        rust_page.iter().map(|row| row.id.to_string()).collect();
    assert_eq!(ids, rust_ids);

    let exists = csharp(
        oracle,
        backend,
        ado,
        "exists",
        serde_json::json!({"ids": [id.to_string(), asset_id(9).to_string()]}),
    );
    assert_eq!(exists["exists"], serde_json::json!([true, false]));

    let stored = csharp(
        oracle,
        backend,
        ado,
        "put",
        serde_json::json!({
            "id": id.to_string(),
            "name": "rewritten",
            "description": "d",
            "type": 1,
            "local": false,
            "temporary": false,
            "flags": 1,
            "creator": "creator",
            "data": [1]
        }),
    );
    assert_eq!(stored["stored"], true);
    let created = csharp(
        oracle,
        backend,
        ado,
        "put",
        serde_json::json!({
            "id": asset_id(5).to_string(),
            "name": "from-csharp",
            "description": "d",
            "type": 1,
            "local": false,
            "temporary": true,
            "flags": 2,
            "creator": "creator",
            "data": [4, 5]
        }),
    );
    assert_eq!(created["stored"], true);

    let missing = "00000000-0000-0000-0000-000000000099";
    let gone = csharp(
        oracle,
        backend,
        ado,
        "delete",
        serde_json::json!({"id": missing}),
    );
    let bad = csharp(
        oracle,
        backend,
        ado,
        "delete",
        serde_json::json!({"id": "nope"}),
    );
    match backend {
        "sqlite" => {
            assert_eq!(gone["deleted"], true);
            assert_eq!(bad["deleted"], false);
        }
        "mysql" => {
            assert_eq!(gone["deleted"], true);
            assert_eq!(bad["deleted"], true);
        }
        "postgres" => {
            assert_eq!(gone["deleted"], false);
            assert_eq!(bad["deleted"], false);
        }
        _ => panic!("backend {backend}"),
    }
}

async fn expect_csharp_writes(store: &AssetStore, backend: Backend) {
    let rewritten = store.get(asset_id(1)).await.expect("get").expect("row");
    assert_eq!(rewritten.name, "rewritten");
    assert_eq!(rewritten.data, vec![1]);
    match backend {
        Backend::Sqlite | Backend::Mysql => assert_eq!(rewritten.flags, 1),
        Backend::Postgres => assert_eq!(rewritten.flags, 0),
    }
    let written = store.get(asset_id(5)).await.expect("get").expect("row");
    assert_eq!(written.name, "from-csharp");
    assert!(written.temporary);
    assert!(!written.local);
    assert_eq!(written.flags, 2);
    assert_eq!(written.data, vec![4, 5]);
}

fn csharp(
    oracle: &str,
    backend: &str,
    ado: &str,
    op: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let mut child = std::process::Command::new("dotnet")
        .arg(oracle)
        .arg("db")
        .arg(op)
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("FERROAN_ASSET_BACKEND", backend)
        .env("FERROAN_ASSET_ADO", ado)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("dotnet");
    serde_json::to_writer(child.stdin.as_mut().expect("stdin"), &body).expect("json");
    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = scrub(&String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "csharp {backend} {op} failed: {stderr}"
    );
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|error| {
        panic!("csharp {backend} {op} json {error}: {}", scrub(&stdout));
    })
}

fn scrub(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(at) = lower.find("password") else {
            result.push_str(rest);
            break;
        };
        result.push_str(&rest[..at + "password".len()]);
        rest = &rest[at + "password".len()..];
        let trimmed = rest.trim_start();
        result.push_str(&rest[..rest.len() - trimmed.len()]);
        rest = trimmed;
        if let Some(stripped) = rest.strip_prefix('=') {
            result.push_str("=redacted");
            let end = stripped
                .find(|ch: char| ch == ';' || ch.is_whitespace())
                .unwrap_or(stripped.len());
            rest = &stripped[end..];
        }
    }
    result
}

async fn open_sqlite_at(path: &std::path::Path) -> AssetStore {
    let pool = ferroan_db::open_sqlite_file(path, &["AssetStore"])
        .await
        .expect("reopen sqlite");
    AssetStore::new(pool).expect("store")
}

async fn open_sqlite() -> (AssetStore, PathBuf) {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ferroan-asset-store-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp");
    let path = dir.join("AssetStore.db");
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite/AssetStore.db");
    std::fs::copy(&source, &path).expect("copy fixture");
    let pool = ferroan_db::open_sqlite_file(&path, &["AssetStore"])
        .await
        .expect("open sqlite");
    let store = AssetStore::new(pool).expect("store");
    (store, path)
}

async fn open_url(var: &str, label: &str) -> Option<AssetStore> {
    let Ok(url) = std::env::var(var) else {
        eprintln!("{var} is unset; {label} asset store was not run");
        return None;
    };
    if url.is_empty() {
        eprintln!("{var} is empty; {label} asset store was not run");
        return None;
    }
    let pool = Pool::connect_sqlx_url(&url)
        .await
        .unwrap_or_else(|error| panic!("{label} connect: {error}"));
    pool.require_stores(&["AssetStore"])
        .await
        .unwrap_or_else(|error| panic!("{label} AssetStore schema: {error}"));
    Some(AssetStore::new(pool).expect(label))
}
