// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Plugin behaviour of `SQLiteAvatarData`, `MySQLAvatarData` and
//! `PGSQLAvatarData`.
//!
//! SQLite always runs, on a copy of `fixtures/db/sqlite/Avatar.db`.
//! MySQL and PostgreSQL run when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL`
//! is set, and skip with a printed reason otherwise. The cross-read runs when
//! `cargo xtask avatar cross` sets `FERROAN_AVATAR_CROSS`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ferroan_db::{Backend, Pool};

use super::store::{AvatarStore, sql_ident};

static NEXT: AtomicU64 = AtomicU64::new(0);
static MYSQL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static POSTGRES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const ALICE: &str = "11111111-1111-4111-8111-111111111111";
const BOB: &str = "22222222-2222-4222-8222-222222222222";
const CASED: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const CASED_UPPER: &str = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";

#[test]
fn realm_is_one_identifier() {
    assert!(sql_ident("Avatars").is_ok());
    assert!(sql_ident("avatar_rows").is_ok());
    assert!(sql_ident("").is_err());
    assert!(sql_ident("1avatar").is_err());
    assert!(sql_ident("Avatars;drop").is_err());
    assert!(sql_ident("Avatar Name").is_err());
    assert!(sql_ident(&"A".repeat(65)).is_err());
}

#[tokio::test]
async fn avatar_plugins_match_the_pin() {
    let (store, _path) = open_sqlite().await;
    check_plugin(&store).await;
    store.close().await;
    if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
        let _guard = MYSQL.lock().await;
        check_plugin(&store).await;
        store.close().await;
    }
    if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
        let _guard = POSTGRES.lock().await;
        check_plugin(&store).await;
        store.close().await;
    }
}

#[tokio::test]
async fn avatar_plugins_cross_read() {
    let Ok(oracle) = std::env::var("FERROAN_AVATAR_ORACLE") else {
        eprintln!("FERROAN_AVATAR_ORACLE is unset; avatar cross-read was not run");
        return;
    };
    if oracle.is_empty() || std::env::var("FERROAN_AVATAR_CROSS").ok().as_deref() != Some("1") {
        eprintln!("FERROAN_AVATAR_CROSS is unset; avatar cross-read was not run");
        return;
    }
    let (store, path) = open_sqlite().await;
    let ado = format!("Data Source={};Version=3;Pooling=False;", path.display());
    cross_sqlite(store, &oracle, &ado, &path).await;

    if let Ok(ado) = std::env::var("FERROAN_AVATAR_ADO_MYSQL") {
        if ado.is_empty() {
            eprintln!("FERROAN_AVATAR_ADO_MYSQL is empty; MySQL avatar cross-read was not run");
        } else if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
            let _guard = MYSQL.lock().await;
            cross_server(&store, &oracle, "mysql", &ado).await;
            store.close().await;
        }
    } else {
        eprintln!("FERROAN_AVATAR_ADO_MYSQL is unset; MySQL avatar cross-read was not run");
    }
    if let Ok(ado) = std::env::var("FERROAN_AVATAR_ADO_POSTGRES") {
        if ado.is_empty() {
            eprintln!(
                "FERROAN_AVATAR_ADO_POSTGRES is empty; PostgreSQL avatar cross-read was not run"
            );
        } else if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
            let _guard = POSTGRES.lock().await;
            cross_server(&store, &oracle, "postgres", &ado).await;
            store.close().await;
        }
    } else {
        eprintln!("FERROAN_AVATAR_ADO_POSTGRES is unset; PostgreSQL avatar cross-read was not run");
    }
}

async fn check_plugin(store: &AvatarStore) {
    store.wipe().await.expect("wipe");
    assert!(store.get(ALICE).await.expect("get").is_empty());
    assert!(!store.delete_principal(ALICE).await.expect("delete"));
    assert!(!store.delete(ALICE, "AvatarType").await.expect("delete"));

    let rows = sample_rows();
    for (name, value) in &rows {
        assert!(store.upsert(ALICE, name, value).await.expect("upsert"));
    }
    assert!(store.upsert(BOB, "AvatarType", "1").await.expect("bob"));
    assert_eq!(names_of(store, ALICE).await, map_of(&rows));
    assert_eq!(
        names_of(store, BOB).await,
        BTreeMap::from([("AvatarType".to_string(), "1".to_string())])
    );

    assert!(
        store
            .upsert(ALICE, "AvatarType", "2")
            .await
            .expect("replace")
    );
    let mut replaced = map_of(&rows);
    replaced.insert("AvatarType".to_string(), "2".to_string());
    assert_eq!(names_of(store, ALICE).await, replaced);
    assert_eq!(
        names_of(store, BOB)
            .await
            .get("AvatarType")
            .map(String::as_str),
        Some("1")
    );

    assert!(store.delete(ALICE, "Empty").await.expect("delete name"));
    assert!(!store.delete(ALICE, "Empty").await.expect("second"));
    replaced.remove("Empty");
    assert_eq!(names_of(store, ALICE).await, replaced);

    assert!(
        store
            .delete_principal(ALICE)
            .await
            .expect("delete principal")
    );
    assert!(names_of(store, ALICE).await.is_empty());
    assert!(
        !store
            .delete_principal(ALICE)
            .await
            .expect("second principal")
    );
    assert_eq!(
        names_of(store, BOB)
            .await
            .get("AvatarType")
            .map(String::as_str),
        Some("1")
    );

    assert!(store.upsert(ALICE, "Bake", "one").await.expect("bake"));
    assert!(store.upsert(ALICE, "bake", "two").await.expect("bake case"));
    let found = names_of(store, ALICE).await;
    match store.backend() {
        Backend::Mysql => {
            assert_eq!(found.get("bake").map(String::as_str), Some("two"));
            assert_eq!(found.len(), 1);
        }
        Backend::Sqlite | Backend::Postgres => {
            assert_eq!(found.get("Bake").map(String::as_str), Some("one"));
            assert_eq!(found.get("bake").map(String::as_str), Some("two"));
        }
    }

    assert!(store.upsert(CASED, "AvatarType", "1").await.expect("cased"));
    let upper = store.get(CASED_UPPER).await.expect("upper");
    match store.backend() {
        Backend::Sqlite => assert!(upper.is_empty()),
        Backend::Mysql | Backend::Postgres => {
            assert_eq!(upper.len(), 1);
            assert!(upper[0].principal_id.eq_ignore_ascii_case(CASED));
            assert_eq!(upper[0].value, "1");
        }
    }

    if store.backend() == Backend::Postgres {
        assert!(store.get("not-a-uuid").await.is_err());
    } else {
        assert!(store.get("not-a-uuid").await.expect("text id").is_empty());
    }

    store.wipe().await.expect("wipe");
}

async fn cross_sqlite(store: AvatarStore, oracle: &str, ado: &str, path: &std::path::Path) {
    store.wipe().await.expect("wipe");
    write_sample(&store, ALICE).await;
    store.close().await;
    expect_csharp_reads(oracle, "sqlite", ado, ALICE);
    csharp_upsert(oracle, "sqlite", ado, BOB, "AvatarType", "9");
    let store = open_sqlite_at(path).await;
    assert_eq!(
        names_of(&store, BOB)
            .await
            .get("AvatarType")
            .map(String::as_str),
        Some("9")
    );
    csharp_delete(oracle, "sqlite", ado, ALICE, "Note");
    assert!(!names_of(&store, ALICE).await.contains_key("Note"));
    assert!(names_of(&store, ALICE).await.contains_key("Wearable 0:0"));
    let removed = csharp_delete_principal(oracle, "sqlite", ado, BOB);
    assert_eq!(removed["deleted"], true);
    assert!(names_of(&store, BOB).await.is_empty());
    let again = csharp_delete_principal(oracle, "sqlite", ado, BOB);
    assert_eq!(again["deleted"], false);
    store.wipe().await.expect("wipe");
    store.close().await;
}

async fn cross_server(store: &AvatarStore, oracle: &str, backend: &str, ado: &str) {
    store.wipe().await.expect("wipe");
    write_sample(store, ALICE).await;
    expect_csharp_reads(oracle, backend, ado, ALICE);
    csharp_upsert(oracle, backend, ado, BOB, "AvatarType", "9");
    assert_eq!(
        names_of(store, BOB)
            .await
            .get("AvatarType")
            .map(String::as_str),
        Some("9")
    );
    csharp_delete(oracle, backend, ado, ALICE, "Note");
    assert!(!names_of(store, ALICE).await.contains_key("Note"));
    assert!(names_of(store, ALICE).await.contains_key("Wearable 0:0"));
    let removed = csharp_delete_principal(oracle, backend, ado, BOB);
    assert_eq!(removed["deleted"], true);
    assert!(names_of(store, BOB).await.is_empty());
    store.wipe().await.expect("wipe");
}

fn expect_csharp_reads(oracle: &str, backend: &str, ado: &str, principal: &str) {
    let got = csharp_get(oracle, backend, ado, principal);
    assert_eq!(json_map(&got), map_of(&sample_rows()));
    let missing = csharp_get(oracle, backend, ado, BOB);
    assert!(json_map(&missing).is_empty());
}

async fn write_sample(store: &AvatarStore, principal: &str) {
    for (name, value) in sample_rows() {
        assert!(
            store
                .upsert(principal, &name, &value)
                .await
                .expect("upsert")
        );
    }
}

fn sample_rows() -> Vec<(String, String)> {
    let long = "x".repeat(300);
    vec![
        ("AvatarType".to_string(), "1".to_string()),
        (
            "Wearable 0:0".to_string(),
            "33333333-3333-4333-8333-333333333333".to_string(),
        ),
        ("AvatarHeight".to_string(), "1.771488".to_string()),
        ("Empty".to_string(), String::new()),
        ("Spaced".to_string(), " hello ".to_string()),
        ("Quoted".to_string(), "a'b,c".to_string()),
        ("Cafe".to_string(), "café".to_string()),
        ("Note".to_string(), long),
    ]
}

fn map_of(rows: &[(String, String)]) -> BTreeMap<String, String> {
    rows.iter().cloned().collect()
}

async fn names_of(store: &AvatarStore, principal: &str) -> BTreeMap<String, String> {
    let rows = store.get(principal).await.expect("get");
    let mut map = BTreeMap::new();
    for row in rows {
        assert!(
            row.principal_id.eq_ignore_ascii_case(principal),
            "{}",
            row.principal_id
        );
        assert!(map.insert(row.name, row.value).is_none());
    }
    map
}

fn csharp_get(oracle: &str, backend: &str, ado: &str, principal: &str) -> serde_json::Value {
    csharp(
        oracle,
        backend,
        ado,
        "get",
        serde_json::json!({"principal_id": principal}),
    )
}

fn csharp_upsert(oracle: &str, backend: &str, ado: &str, principal: &str, name: &str, value: &str) {
    let written = csharp(
        oracle,
        backend,
        ado,
        "upsert",
        serde_json::json!({"principal_id": principal, "name": name, "value": value}),
    );
    assert_eq!(written["stored"], true);
}

fn csharp_delete(
    oracle: &str,
    backend: &str,
    ado: &str,
    principal: &str,
    name: &str,
) -> serde_json::Value {
    csharp(
        oracle,
        backend,
        ado,
        "delete",
        serde_json::json!({"principal_id": principal, "name": name}),
    )
}

fn csharp_delete_principal(
    oracle: &str,
    backend: &str,
    ado: &str,
    principal: &str,
) -> serde_json::Value {
    csharp(
        oracle,
        backend,
        ado,
        "delete-principal",
        serde_json::json!({"principal_id": principal}),
    )
}

fn json_map(body: &serde_json::Value) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for row in body["rows"].as_array().expect("rows") {
        let name = row["name"].as_str().expect("name").to_string();
        let value = row["value"].as_str().expect("value").to_string();
        assert!(map.insert(name, value).is_none());
    }
    map
}

fn csharp(
    oracle: &str,
    backend: &str,
    ado: &str,
    op: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let output = csharp_status(oracle, backend, ado, op, body);
    assert!(
        output.status.success(),
        "csharp {backend} {op} failed: {}",
        scrub(&output.stderr)
    );
    parse_json(&output.stdout)
}

struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn csharp_status(
    oracle: &str,
    backend: &str,
    ado: &str,
    op: &str,
    body: serde_json::Value,
) -> CommandOutput {
    let mut child = std::process::Command::new("dotnet")
        .arg(oracle)
        .arg(op)
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("MSBUILDDISABLENODEREUSE", "1")
        .env("FERROAN_AVATAR_BACKEND", backend)
        .env("FERROAN_AVATAR_ADO", ado)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("dotnet");
    serde_json::to_writer(child.stdin.as_mut().expect("stdin"), &body).expect("json");
    let output = child.wait_with_output().expect("wait");
    CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn parse_json(stdout: &str) -> serde_json::Value {
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|error| panic!("json {error}: {}", scrub(stdout)))
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

async fn open_sqlite() -> (AvatarStore, PathBuf) {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ferroan-avatar-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp");
    let path = dir.join("Avatar.db");
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite/Avatar.db");
    std::fs::copy(&source, &path).expect("copy fixture");
    let store = open_sqlite_at(&path).await;
    (store, path)
}

async fn open_sqlite_at(path: &std::path::Path) -> AvatarStore {
    let pool = ferroan_db::open_sqlite_file(path, &["Avatar"])
        .await
        .expect("open sqlite");
    AvatarStore::open(pool).expect("store")
}

async fn open_url(var: &str, label: &str) -> Option<AvatarStore> {
    let Ok(url) = std::env::var(var) else {
        eprintln!("{var} is unset; {label} avatar store was not run");
        return None;
    };
    if url.is_empty() {
        eprintln!("{var} is empty; {label} avatar store was not run");
        return None;
    }
    let pool = Pool::connect_sqlx_url(&url)
        .await
        .unwrap_or_else(|error| panic!("{label} connect: {error}"));
    pool.require_stores(&["Avatar"])
        .await
        .unwrap_or_else(|error| panic!("{label} Avatar schema: {error}"));
    Some(AvatarStore::open(pool).expect(label))
}
