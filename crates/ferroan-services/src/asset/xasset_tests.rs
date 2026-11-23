// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Plugin behaviour of `MySQLXAssetData` and `PGSQLXAssetData`.
//!
//! MySQL and PostgreSQL run when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL`
//! is set, and skip with a printed reason otherwise. The cross-read runs when
//! `cargo xtask asset xasset-cross` sets `FERROAN_XASSET_CROSS`.

use ferroan_db::{Backend, Pool};
use uuid::Uuid;

use super::super::model::{Asset, KNOWN_FLAGS};
use super::super::time::mysql_unix_now;
use super::XAssetStore;

use super::{XASSET_MYSQL, XASSET_POSTGRES};

#[tokio::test]
async fn xasset_plugins_match_the_pin() {
    if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
        let _guard = XASSET_MYSQL.lock().await;
        check_plugin(&store).await;
        store.close().await;
    }
    if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
        let _guard = XASSET_POSTGRES.lock().await;
        check_plugin(&store).await;
        store.close().await;
    }
}

#[tokio::test]
async fn xasset_plugins_cross_read() {
    let Ok(oracle) = std::env::var("FERROAN_ASSET_ORACLE") else {
        eprintln!("FERROAN_ASSET_ORACLE is unset; XAsset cross-read was not run");
        return;
    };
    if oracle.is_empty() || std::env::var("FERROAN_XASSET_CROSS").ok().as_deref() != Some("1") {
        eprintln!("FERROAN_XASSET_CROSS is unset; XAsset cross-read was not run");
        return;
    }
    if let Ok(ado) = std::env::var("FERROAN_ASSET_ADO_MYSQL") {
        if !ado.is_empty() {
            let _guard = XASSET_MYSQL.lock().await;
            if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
                cross_one(&store, &oracle, "mysql", &ado).await;
                store.close().await;
            }
        }
    } else {
        eprintln!("FERROAN_ASSET_ADO_MYSQL is unset; MySQL XAsset cross-read was not run");
    }
    if let Ok(ado) = std::env::var("FERROAN_ASSET_ADO_POSTGRES") {
        if !ado.is_empty() {
            let _guard = XASSET_POSTGRES.lock().await;
            if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
                cross_one(&store, &oracle, "postgres", &ado).await;
                store.close().await;
            }
        }
    } else {
        eprintln!("FERROAN_ASSET_ADO_POSTGRES is unset; PostgreSQL XAsset cross-read was not run");
    }
}

async fn check_plugin(store: &XAssetStore) {
    store.wipe().await.expect("wipe");
    let backend = store.backend();
    let id = xid(1);
    let other = xid(2);
    let bytes = vec![9, 8, 7];
    let mut created = sample(id, "first", 8);
    created.local = true;
    created.temporary = true;
    created.data.clone_from(&bytes);
    store.store(&created).await.expect("store");

    let loaded = store.get(id).await.expect("get").expect("row");
    assert_eq!(loaded.name, "first");
    assert_eq!(loaded.description, "d");
    assert_eq!(loaded.type_code, 1);
    assert!(loaded.local);
    assert!(loaded.temporary);
    assert_eq!(loaded.creator_id, "creator");
    assert_eq!(loaded.flags, 0, "known-bit mask drops flag 8");
    assert_eq!(loaded.data, bytes);
    assert_eq!(loaded.upload_attempts, 0);
    let clock = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(clock.raw_flags, 8);
    let now = mysql_unix_now();
    let create = clock.create_time.expect("create");
    let access = clock.access_time.expect("access");
    near(create, now);
    near(access, now);
    assert_eq!(store.data_rows().await.expect("data"), 1);

    let again = store.clock(id).await.expect("clock").expect("row");
    let _ = store.get(id).await.expect("get");
    let after_get = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(after_get.access_time, again.access_time);

    let mut twin = sample(other, "twin", 1);
    twin.data.clone_from(&bytes);
    store.store(&twin).await.expect("twin");
    assert_eq!(store.data_rows().await.expect("data"), 1, "one hash");
    let twin_row = store.get(other).await.expect("get").expect("twin");
    assert_eq!(twin_row.data, bytes);
    assert_eq!(twin_row.flags, 1);

    let mut rewritten = sample(id, "rewritten", 9);
    rewritten.data = vec![4, 5];
    store.store(&rewritten).await.expect("rewrite");
    assert_eq!(store.data_rows().await.expect("data"), 2, "old hash stays");
    let row = store.get(id).await.expect("get").expect("row");
    assert_eq!(row.name, "rewritten");
    assert_eq!(row.data, vec![4, 5]);
    assert_eq!(row.flags, 9 & KNOWN_FLAGS);
    let raw = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(raw.raw_flags, 9);
    near(raw.create_time.expect("create"), mysql_unix_now());
    near(raw.access_time.expect("access"), mysql_unix_now());

    set_times(store, id, 1, 1).await;
    store.store(&rewritten).await.expect("clock rewrite");
    let clocks = store.clock(id).await.expect("clock").expect("row");
    near(clocks.create_time.expect("create"), mysql_unix_now());
    near(clocks.access_time.expect("access"), mysql_unix_now());
    assert_eq!(store.data_rows().await.expect("data"), 2);

    let stale = mysql_unix_now() - 31 * 24 * 60 * 60;
    set_access(store, id, stale).await;
    let _ = store.get(id).await.expect("get");
    let bumped = store.clock(id).await.expect("clock").expect("row");
    near(bumped.access_time.expect("access"), mysql_unix_now());
    assert_eq!(
        bumped.create_time.expect("create"),
        clocks.create_time.expect("create")
    );

    set_access(store, id, 0).await;
    let _ = store.get(id).await.expect("get");
    let zero = store.clock(id).await.expect("clock").expect("row");
    match backend {
        Backend::Mysql => assert_eq!(zero.access_time, Some(0), "get skips a stored 0"),
        Backend::Postgres => near(zero.access_time.expect("access"), mysql_unix_now()),
        Backend::Sqlite => panic!("sqlite"),
    }

    set_access(store, other, stale).await;
    let page = store.metadata_set(0, 10).await.expect("metadata");
    let other_clock = store.clock(other).await.expect("clock").expect("twin");
    match backend {
        Backend::Mysql | Backend::Postgres => {
            assert!(page.iter().any(|row| row.id == other && row.flags == 1));
            near(other_clock.access_time.expect("access"), mysql_unix_now());
        }
        Backend::Sqlite => panic!("sqlite"),
    }

    let mut long = sample(xid(3), &"n".repeat(80), 1);
    long.description = "d".repeat(80);
    long.data = vec![1];
    store.store(&long).await.expect("truncate");
    let truncated = store.get(xid(3)).await.expect("get").expect("row");
    assert_eq!(truncated.name, "n".repeat(64));
    assert_eq!(truncated.description, "d".repeat(64));

    let before = store.data_rows().await.expect("data");
    let mut bad = sample(xid(4), "bad", 1);
    bad.creator_id = "c".repeat(129);
    bad.data = vec![2, 2];
    store.store(&bad).await.expect("swallowed write");
    assert!(store.get(xid(4)).await.expect("get").is_none());
    assert_eq!(store.data_rows().await.expect("data"), before);

    let exists = store.exists(&[id, xid(9)]).await.expect("exists");
    match backend {
        Backend::Mysql | Backend::Postgres => assert_eq!(exists, vec![true, false]),
        Backend::Sqlite => panic!("sqlite"),
    }
    assert!(store.exists(&[]).await.expect("empty").is_empty());

    match backend {
        Backend::Mysql => {
            assert!(store.delete(&id.to_string()).await.expect("delete"));
            assert!(store.delete("nope").await.expect("bad id"));
            assert!(store.get(id).await.expect("get").is_none());
            assert!(store.get(other).await.expect("get").is_some());
            assert_eq!(store.data_rows().await.expect("data"), before);
        }
        Backend::Postgres => {
            assert!(store.delete(&id.to_string()).await.expect("delete"));
            assert!(!store.delete("nope").await.expect("bad id"));
            assert!(
                store.get(id).await.expect("get").is_none(),
                "the meta row goes"
            );
            assert!(store.get(other).await.expect("get").is_some());
            assert_eq!(
                store.data_rows().await.expect("data"),
                before,
                "the data row stays"
            );
            assert!(store.delete(&other.to_string()).await.expect("other"));
            assert!(
                store.get(other).await.expect("get").is_none(),
                "the twin meta row goes"
            );
            assert_eq!(
                store.data_rows().await.expect("data"),
                before,
                "a shared hash stays after its last meta row goes"
            );
            assert!(store.get(xid(3)).await.expect("truncated").is_some());
            assert!(store.delete(&xid(3).to_string()).await.expect("truncated"));
            assert!(
                store.get(xid(3)).await.expect("get").is_none(),
                "the meta row goes"
            );
            assert_eq!(store.data_rows().await.expect("data"), before);
        }
        Backend::Sqlite => panic!("sqlite"),
    }
    assert!(store.get(xid(8)).await.expect("missing").is_none());
}

async fn cross_one(store: &XAssetStore, oracle: &str, backend: &str, ado: &str) {
    store.wipe().await.expect("wipe");
    let id = xid(1);
    let other = xid(2);
    let bytes = vec![9, 8, 7];
    let mut created = sample(id, &"n".repeat(70), 8);
    created.local = true;
    created.temporary = true;
    created.data.clone_from(&bytes);
    store.store(&created).await.expect("rust store");
    let mut twin = sample(other, "twin", 1);
    twin.data.clone_from(&bytes);
    store.store(&twin).await.expect("twin");
    assert_eq!(store.data_rows().await.expect("data"), 1);

    let got = csharp(
        oracle,
        backend,
        ado,
        "get",
        serde_json::json!({"id": id.to_string()}),
    );
    assert_eq!(got["found"], true);
    assert_eq!(got["name"], "n".repeat(64));
    assert_eq!(got["local"], true);
    assert_eq!(got["temporary"], true);
    assert_eq!(got["flags"], 0);
    assert_eq!(got["data"], serde_json::json!([9, 8, 7]));

    let exists = csharp(
        oracle,
        backend,
        ado,
        "exists",
        serde_json::json!({"ids": [id.to_string(), xid(9).to_string()]}),
    );
    // TB28 fixed (#476): MySQL reads `XAssetsMeta`, as PostgreSQL does.
    assert_eq!(exists["exists"], serde_json::json!([true, false]));

    let page = csharp(
        oracle,
        backend,
        ado,
        "metadata",
        serde_json::json!({"start": 0, "count": 10}),
    );
    let rows = page["rows"].as_array().expect("rows");
    // TB29 fixed (#479): PostgreSQL returns the page, as MySQL does.
    assert!(rows.iter().any(|row| row["id"] == id.to_string()));
    assert!(rows.iter().any(|row| row["flags"] == 0));

    let stored = csharp(
        oracle,
        backend,
        ado,
        "put",
        serde_json::json!({
            "id": xid(3).to_string(),
            "name": "from-csharp",
            "description": "d".repeat(70),
            "type": 1,
            "local": false,
            "temporary": true,
            "flags": 9,
            "creator": "creator",
            "data": [9, 8, 7]
        }),
    );
    assert_eq!(stored["stored"], true);

    let bad = csharp(
        oracle,
        backend,
        ado,
        "put",
        serde_json::json!({
            "id": xid(4).to_string(),
            "name": "bad",
            "description": "d",
            "type": 1,
            "local": false,
            "temporary": false,
            "flags": 1,
            "creator": "c".repeat(129),
            "data": [3]
        }),
    );
    assert_eq!(bad["stored"], true);

    let reopened = reopen(store.backend()).await;
    assert_eq!(reopened.data_rows().await.expect("data"), 1);
    // TB27 fixed (#475): MySQL enlists the transaction, so both engines
    // write the row.
    let written = reopened
        .get(xid(3))
        .await
        .expect("get")
        .expect("csharp row");
    assert_eq!(written.name, "from-csharp");
    assert_eq!(written.description.len(), 64);
    assert!(!written.local);
    assert!(written.temporary);
    assert_eq!(written.flags, 1);
    assert_eq!(written.data, bytes);
    assert!(reopened.get(xid(4)).await.expect("get").is_none());

    let deleted = csharp_status(
        oracle,
        backend,
        ado,
        "delete",
        serde_json::json!({"id": id.to_string()}),
    );
    let after = reopen(store.backend()).await;
    // TB30 fixed (#480): PostgreSQL binds the id as uuid and deletes the meta
    // row, as MySQL does. The shared data row stays.
    assert!(deleted.status.success());
    let body = json_line(&deleted);
    assert_eq!(body["deleted"], true);
    assert!(after.get(id).await.expect("get").is_none());
    assert!(after.get(other).await.expect("get").is_some());
    assert_eq!(after.data_rows().await.expect("data"), 1);
    after.close().await;
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

fn xid(n: u32) -> Uuid {
    Uuid::parse_str(&format!("a22a0000-0000-4000-8000-{n:012x}")).expect("id")
}

fn near(value: i32, now: i32) {
    assert!((value - now).abs() < 120, "{value} vs {now}");
}

async fn set_times(store: &XAssetStore, id: Uuid, create: i32, access: i32) {
    match store.backend() {
        Backend::Mysql => {
            sqlx::query("UPDATE XAssetsMeta SET CreateTime = ?, AccessTime = ? WHERE ID = ?")
                .bind(create)
                .bind(access)
                .bind(id.to_string())
                .execute(store.pool().mysql().expect("mysql"))
                .await
                .expect("times");
        }
        Backend::Postgres => {
            sqlx::query(
                "UPDATE xassetsmeta SET create_time = $2, access_time = $3 WHERE id = $1::uuid",
            )
            .bind(id.to_string())
            .bind(create)
            .bind(access)
            .execute(store.pool().postgres().expect("postgres"))
            .await
            .expect("times");
        }
        Backend::Sqlite => panic!("sqlite"),
    }
}

async fn set_access(store: &XAssetStore, id: Uuid, access: i32) {
    match store.backend() {
        Backend::Mysql => {
            sqlx::query("UPDATE XAssetsMeta SET AccessTime = ? WHERE ID = ?")
                .bind(access)
                .bind(id.to_string())
                .execute(store.pool().mysql().expect("mysql"))
                .await
                .expect("access");
        }
        Backend::Postgres => {
            sqlx::query("UPDATE xassetsmeta SET access_time = $2 WHERE id = $1::uuid")
                .bind(id.to_string())
                .bind(access)
                .execute(store.pool().postgres().expect("postgres"))
                .await
                .expect("access");
        }
        Backend::Sqlite => panic!("sqlite"),
    }
}

async fn open_url(var: &str, label: &str) -> Option<XAssetStore> {
    let Ok(url) = std::env::var(var) else {
        eprintln!("{var} is unset; {label} xasset store was not run");
        return None;
    };
    if url.is_empty() {
        eprintln!("{var} is empty; {label} xasset store was not run");
        return None;
    }
    let pool = Pool::connect_sqlx_url(&url)
        .await
        .unwrap_or_else(|error| panic!("{label} connect: {error}"));
    pool.require_stores(&["XAssetStore"])
        .await
        .unwrap_or_else(|error| panic!("{label} XAssetStore schema: {error}"));
    Some(XAssetStore::new(pool).expect(label))
}

async fn reopen(backend: Backend) -> XAssetStore {
    let (var, label) = match backend {
        Backend::Mysql => ("FERROAN_MYSQL_URL", "MySQL"),
        Backend::Postgres => ("FERROAN_POSTGRES_URL", "PostgreSQL"),
        Backend::Sqlite => panic!("sqlite"),
    };
    open_url(var, label).await.expect(label)
}

fn csharp(
    oracle: &str,
    backend: &str,
    ado: &str,
    op: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let output = csharp_status(oracle, backend, ado, op, body);
    let stderr = scrub(&String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "csharp {backend} {op} failed: {stderr}"
    );
    json_line(&output)
}

fn csharp_status(
    oracle: &str,
    backend: &str,
    ado: &str,
    op: &str,
    body: serde_json::Value,
) -> std::process::Output {
    let mut child = std::process::Command::new("dotnet")
        .arg(oracle)
        .arg("xdb")
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
    child.wait_with_output().expect("wait")
}

fn json_line(output: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|error| {
        panic!("xasset json {error}: {}", scrub(&stdout));
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
