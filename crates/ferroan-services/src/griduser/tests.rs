// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Plugin behaviour of `SQLiteGridUserData`, `MySQLGridUserData` and
//! `PGSQLGridUserData`.
//!
//! SQLite always runs, on a copy of `fixtures/db/sqlite/GridUserStore.db`.
//! MySQL and PostgreSQL run when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL`
//! is set, and skip with a printed reason otherwise. The cross-read runs when
//! `cargo xtask griduser cross` sets `FERROAN_GRIDUSER_CROSS`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ferroan_db::{Backend, Pool};

use super::model::{BATCH_SIZE, GridUser, ONLINE_TRUE, like_prefix, postgres_text};
use super::store::{GridUserStore, sql_ident};

static NEXT: AtomicU64 = AtomicU64::new(0);
static MYSQL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static POSTGRES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const REGION: &str = "11111111-1111-1111-1111-111111111111";
const OTHER_REGION: &str = "22222222-2222-2222-2222-222222222222";
const ZERO_REGION: &str = "00000000-0000-0000-0000-000000000000";
/// A quote, a `%`, a `_` and the escape character.
const SPECIAL: &str = "a'b%c_d!e";

#[test]
fn like_prefix_escapes_wildcards_and_keeps_a_quote() {
    assert_eq!(like_prefix(SPECIAL), "a'b!%c!_d!!e%");
    assert_eq!(like_prefix(""), "%");
    assert_eq!(like_prefix("agent"), "agent%");
    assert!(sql_ident("GridUser").is_ok());
    assert!(sql_ident("grid_user").is_ok());
    assert!(sql_ident("").is_err());
    assert!(sql_ident("1grid").is_err());
    assert!(sql_ident("Grid User").is_err());
    assert!(sql_ident("GridUser;drop").is_err());
    assert!(sql_ident(&"G".repeat(65)).is_err());
}

#[tokio::test]
async fn griduser_plugins_match_the_pin() {
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
async fn griduser_plugins_cross_read() {
    let Ok(oracle) = std::env::var("FERROAN_GRIDUSER_ORACLE") else {
        eprintln!("FERROAN_GRIDUSER_ORACLE is unset; grid user cross-read was not run");
        return;
    };
    if oracle.is_empty() || std::env::var("FERROAN_GRIDUSER_CROSS").ok().as_deref() != Some("1") {
        eprintln!("FERROAN_GRIDUSER_CROSS is unset; grid user cross-read was not run");
        return;
    }
    let (store, path) = open_sqlite().await;
    let ado = format!("Data Source={};Version=3;Pooling=False;", path.display());
    cross_sqlite(store, &oracle, &ado, &path).await;

    if let Ok(ado) = std::env::var("FERROAN_GRIDUSER_ADO_MYSQL") {
        if ado.is_empty() {
            eprintln!(
                "FERROAN_GRIDUSER_ADO_MYSQL is empty; MySQL grid user cross-read was not run"
            );
        } else if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
            let _guard = MYSQL.lock().await;
            cross_server(&store, &oracle, "mysql", &ado).await;
            store.close().await;
        }
    } else {
        eprintln!("FERROAN_GRIDUSER_ADO_MYSQL is unset; MySQL grid user cross-read was not run");
    }
    if let Ok(ado) = std::env::var("FERROAN_GRIDUSER_ADO_POSTGRES") {
        if ado.is_empty() {
            eprintln!(
                "FERROAN_GRIDUSER_ADO_POSTGRES is empty; PostgreSQL grid user cross-read was not run"
            );
        } else if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
            let _guard = POSTGRES.lock().await;
            cross_server(&store, &oracle, "postgres", &ado).await;
            store.close().await;
        }
    } else {
        eprintln!(
            "FERROAN_GRIDUSER_ADO_POSTGRES is unset; PostgreSQL grid user cross-read was not run"
        );
    }
}

async fn check_plugin(store: &GridUserStore) {
    store.wipe().await.expect("wipe");
    assert!(store.get("missing").await.expect("get").is_none());
    assert!(store.get_all("").await.expect("all").is_empty());
    assert!(store.get_many(&[]).await.expect("none").is_empty());

    store_user(store, &sample("one")).await;
    store_user(store, &sample("two")).await;
    let all = ids_of(&store.get_all("").await.expect("all"));
    assert_eq!(all, set(&["one", "two"]));

    store.wipe().await.expect("wipe");
    let online = sample("agent-online");
    store_user(store, &online).await;
    let loaded = store.get("agent-online").await.expect("get").expect("row");
    assert_eq!(loaded, presented_store(store, &online));
    assert!(store.get("agent-offline").await.expect("miss").is_none());

    let bare = GridUser::new("defaults-user");
    store_user(store, &bare).await;
    let defaults = store.get("defaults-user").await.expect("get").expect("row");
    assert_field(store, &defaults, "HomeRegionID", ZERO_REGION);
    assert_field(store, &defaults, "HomePosition", "<0,0,0>");
    assert_field(store, &defaults, "HomeLookAt", "<0,0,0>");
    assert_field(store, &defaults, "LastRegionID", ZERO_REGION);
    assert_field(store, &defaults, "LastPosition", "<0,0,0>");
    assert_field(store, &defaults, "LastLookAt", "<0,0,0>");
    assert_field(store, &defaults, "Online", "false");
    assert_field(store, &defaults, "Login", "0");
    assert_field(store, &defaults, "Logout", "0");

    check_partial(store).await;
    check_many(store).await;
    check_online(store).await;
    check_literal_prefix(store).await;
    check_case(store).await;

    let mut bad = GridUser::new("bad-column");
    bad.data.insert("Nope".to_string(), "1".to_string());
    assert!(store.store(&bad).await.is_err());
    let mut key = GridUser::new("bad-key");
    key.data.insert("UserID".to_string(), "other".to_string());
    assert!(store.store(&key).await.is_err());
    store.wipe().await.expect("wipe");
}

async fn check_partial(store: &GridUserStore) {
    let mut full = sample("partial");
    full.data
        .insert("HomePosition".to_string(), "<1,2,3>".to_string());
    full.data
        .insert("Login".to_string(), "1700000000".to_string());
    store_user(store, &full).await;
    let mut only = GridUser::new("partial");
    only.data.insert("Online".to_string(), "False".to_string());
    store_user(store, &only).await;
    let loaded = store.get("partial").await.expect("get").expect("row");
    assert_field(store, &loaded, "Online", "False");
    match store.backend() {
        Backend::Postgres => {
            assert_field(store, &loaded, "HomePosition", "<1,2,3>");
            assert_field(store, &loaded, "Login", "1700000000");
            assert_field(store, &loaded, "LastRegionID", REGION);
        }
        Backend::Sqlite | Backend::Mysql => {
            assert_field(store, &loaded, "HomePosition", "<0,0,0>");
            assert_field(store, &loaded, "Login", "0");
            assert_field(store, &loaded, "LastRegionID", ZERO_REGION);
        }
    }
}

async fn check_many(store: &GridUserStore) {
    store_user(store, &sample("many-a")).await;
    store_user(store, &sample("many-b")).await;
    // `many-a` twice, then 499 missing ids, then `many-b`: 502 entries and
    // 501 distinct ids, so the second statement carries only `many-b`.
    let mut ids = vec!["many-a".to_string(), "many-a".to_string()];
    for index in 0..BATCH_SIZE.saturating_sub(1) {
        ids.push(format!("missing-{index}"));
    }
    ids.push("many-b".to_string());
    assert_eq!(ids.len(), BATCH_SIZE.saturating_add(2));
    let found = store.get_many(&ids).await.expect("many");
    assert_eq!(ids_of(&found), set(&["many-a", "many-b"]));
    let none = store
        .get_many(&["not-stored".to_string()])
        .await
        .expect("omit");
    assert!(none.is_empty());
}

async fn check_online(store: &GridUserStore) {
    store.wipe().await.expect("wipe");
    store_user(store, &sample("agent-online")).await;
    let mut offline = sample("agent-offline");
    offline
        .data
        .insert("Online".to_string(), "false".to_string());
    store_user(store, &offline).await;
    let mut elsewhere = sample("agent-elsewhere");
    elsewhere
        .data
        .insert("LastRegionID".to_string(), OTHER_REGION.to_string());
    store_user(store, &elsewhere).await;
    let mut lower = sample("agent-lower");
    lower.data.insert("Online".to_string(), "true".to_string());
    store_user(store, &lower).await;
    let rows = store.get_online_in_region(REGION).await.expect("online");
    // MySQL `latin1_swedish_ci` compares `Online` without case, so `true`
    // matches `True`. SQLite and PostgreSQL equality stays case-sensitive.
    let online = match store.backend() {
        Backend::Mysql => set(&["agent-online", "agent-lower"]),
        Backend::Sqlite | Backend::Postgres => set(&["agent-online"]),
    };
    assert_eq!(ids_of(&rows), online);
    let other = store
        .get_online_in_region(OTHER_REGION)
        .await
        .expect("other");
    assert_eq!(ids_of(&other), set(&["agent-elsewhere"]));
}

async fn check_literal_prefix(store: &GridUserStore) {
    store.wipe().await.expect("wipe");
    store_user(store, &GridUser::new(SPECIAL)).await;
    store_user(store, &GridUser::new(format!("{SPECIAL}.more"))).await;
    store_user(store, &GridUser::new("a'bXc_d!e")).await;
    store_user(store, &GridUser::new("a'b%cXd!e")).await;
    store_user(store, &GridUser::new("a'b%c_de")).await;
    store_user(store, &GridUser::new("z'b%c_d!e")).await;
    let rows = store.get_all(SPECIAL).await.expect("prefix");
    assert_eq!(
        ids_of(&rows),
        set(&[SPECIAL, &format!("{SPECIAL}.more")]),
        "a quote, %, _ and ! stay literal"
    );
}

async fn check_case(store: &GridUserStore) {
    store.wipe().await.expect("wipe");
    store_user(store, &GridUser::new("AbC")).await;
    store_user(store, &GridUser::new("abc")).await;
    let found = ids_of(&store.get_all("ab").await.expect("like"));
    match store.backend() {
        Backend::Mysql => {
            assert_eq!(found, set(&["abc"]));
            let row = store.get("AbC").await.expect("get").expect("ci");
            assert_eq!(row.user_id, "abc");
        }
        Backend::Sqlite => {
            assert_eq!(found, set(&["AbC", "abc"]));
            assert_eq!(
                store.get("AbC").await.expect("get").expect("row").user_id,
                "AbC"
            );
        }
        Backend::Postgres => {
            assert_eq!(found, set(&["abc"]));
            assert_eq!(
                store.get("AbC").await.expect("get").expect("row").user_id,
                "AbC"
            );
        }
    }
}

async fn cross_sqlite(store: GridUserStore, oracle: &str, ado: &str, path: &std::path::Path) {
    store.wipe().await.expect("wipe");
    let rusted = sample("rust-user");
    store_user(&store, &rusted).await;
    store.close().await;
    expect_csharp_reads(oracle, "sqlite", ado, &rusted);
    let written = csharp(oracle, "sqlite", ado, "store", &sample("csharp-user"));
    assert_eq!(written["stored"], true);
    let store = open_sqlite_at(path).await;
    let from_csharp = store.get("csharp-user").await.expect("get").expect("row");
    assert_eq!(from_csharp, presented("sqlite", &sample("csharp-user")));
    store_special(&store).await;
    store.close().await;
    expect_special(oracle, "sqlite", ado);
    let store = open_sqlite_at(path).await;
    store.wipe().await.expect("wipe");
    store.close().await;
}

async fn cross_server(store: &GridUserStore, oracle: &str, backend: &str, ado: &str) {
    store.wipe().await.expect("wipe");
    let rusted = sample("rust-user");
    store_user(store, &rusted).await;
    expect_csharp_reads(oracle, backend, ado, &rusted);
    let written = csharp(oracle, backend, ado, "store", &sample("csharp-user"));
    assert_eq!(written["stored"], true);
    let from_csharp = store.get("csharp-user").await.expect("get").expect("row");
    assert_eq!(from_csharp, presented(backend, &sample("csharp-user")));
    let mut only = GridUser::new("csharp-user");
    only.data.insert("Online".to_string(), "False".to_string());
    let partial = csharp(oracle, backend, ado, "store", &only);
    assert_eq!(partial["stored"], true);
    let after = store.get("csharp-user").await.expect("get").expect("row");
    assert_field(store, &after, "Online", "False");
    match store.backend() {
        Backend::Postgres => assert_field(store, &after, "Login", "1700000000"),
        Backend::Mysql | Backend::Sqlite => assert_field(store, &after, "Login", "0"),
    }
    store_special(store).await;
    expect_special(oracle, backend, ado);
    store.wipe().await.expect("wipe");
}

fn expect_csharp_reads(oracle: &str, backend: &str, ado: &str, rusted: &GridUser) {
    let got = csharp(oracle, backend, ado, "get", &GridUser::new("rust-user"));
    assert_eq!(got["found"], true);
    assert_eq!(user_from_json(&got), presented(backend, rusted));
    let all = csharp(oracle, backend, ado, "getall", &GridUser::new("rust-"));
    assert_eq!(json_ids(&all), set(&["rust-user"]));
    let many = csharp_body(
        oracle,
        backend,
        ado,
        "getmany",
        serde_json::json!({"user_ids": ["rust-user", "rust-user", "missing"]}),
    );
    assert_eq!(json_ids(&many), set(&["rust-user"]));
    let online = csharp_body(
        oracle,
        backend,
        ado,
        "online",
        serde_json::json!({"region_id": REGION}),
    );
    assert_eq!(json_ids(&online), set(&["rust-user"]));
}

async fn store_special(store: &GridUserStore) {
    store_user(store, &GridUser::new(SPECIAL)).await;
    store_user(store, &GridUser::new(format!("{SPECIAL}.more"))).await;
    store_user(store, &GridUser::new("a'bXc_d!e")).await;
    store_user(store, &GridUser::new("a'b%cXd!e")).await;
    store_user(store, &GridUser::new("a'b%c_de")).await;
}

fn expect_special(oracle: &str, backend: &str, ado: &str) {
    let exact = csharp(oracle, backend, ado, "get", &GridUser::new(SPECIAL));
    assert_eq!(exact["found"], true);
    assert_eq!(exact["user_id"], SPECIAL);
    // TB43 fixed (#525): every store matches the id as a literal prefix,
    // as the Rust store does (D35).
    let status = csharp_status(
        oracle,
        backend,
        ado,
        "getall",
        serde_json::json!({"user_id": SPECIAL}),
    );
    assert!(status.status.success(), "{}", scrub(&status.stderr));
    let body = parse_json(&status.stdout);
    assert_eq!(
        json_ids(&body),
        set(&[SPECIAL, &format!("{SPECIAL}.more")]),
        "{backend}"
    );
    println!("griduser {backend}: literal prefix matches");
}

async fn store_user(store: &GridUserStore, user: &GridUser) {
    assert!(store.store(user).await.expect("store"));
}

fn presented(backend: &str, user: &GridUser) -> GridUser {
    if backend != "postgres" {
        return user.clone();
    }
    let mut data = BTreeMap::new();
    for (key, value) in &user.data {
        data.insert(key.clone(), postgres_text(key, value));
    }
    GridUser {
        user_id: user.user_id.clone(),
        data,
    }
}

fn presented_store(store: &GridUserStore, user: &GridUser) -> GridUser {
    let backend = match store.backend() {
        Backend::Postgres => "postgres",
        Backend::Mysql => "mysql",
        Backend::Sqlite => "sqlite",
    };
    presented(backend, user)
}

fn assert_field(store: &GridUserStore, row: &GridUser, name: &str, value: &str) {
    let expected = if store.backend() == Backend::Postgres {
        postgres_text(name, value)
    } else {
        value.to_string()
    };
    assert_eq!(row.field(name), Some(expected.as_str()), "{name}");
}

fn sample(user_id: &str) -> GridUser {
    let mut data = BTreeMap::new();
    data.insert("HomeRegionID".to_string(), ZERO_REGION.to_string());
    data.insert("HomePosition".to_string(), "<1,2,3>".to_string());
    data.insert("HomeLookAt".to_string(), "<0,1,0>".to_string());
    data.insert("LastRegionID".to_string(), REGION.to_string());
    data.insert("LastPosition".to_string(), "<4,5,6>".to_string());
    data.insert("LastLookAt".to_string(), "<0,0,1>".to_string());
    data.insert("Online".to_string(), ONLINE_TRUE.to_string());
    data.insert("Login".to_string(), "1700000000".to_string());
    data.insert("Logout".to_string(), "0".to_string());
    GridUser {
        user_id: user_id.to_string(),
        data,
    }
}

fn ids_of(rows: &[GridUser]) -> BTreeSet<String> {
    rows.iter().map(|row| row.user_id.clone()).collect()
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_string()).collect()
}

fn json_ids(body: &serde_json::Value) -> BTreeSet<String> {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["user_id"].as_str().expect("id").to_string())
        .collect()
}

fn user_from_json(body: &serde_json::Value) -> GridUser {
    let mut data = BTreeMap::new();
    let object = body["data"].as_object().expect("data");
    for (key, value) in object {
        data.insert(key.clone(), value.as_str().expect("text").to_string());
    }
    GridUser {
        user_id: body["user_id"].as_str().expect("id").to_string(),
        data,
    }
}

fn csharp(oracle: &str, backend: &str, ado: &str, op: &str, user: &GridUser) -> serde_json::Value {
    let data = serde_json::Value::Object(
        user.data
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect(),
    );
    csharp_body(
        oracle,
        backend,
        ado,
        op,
        serde_json::json!({"user_id": user.user_id, "data": data}),
    )
}

fn csharp_body(
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
        .env("FERROAN_GRIDUSER_BACKEND", backend)
        .env("FERROAN_GRIDUSER_ADO", ado)
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

async fn open_sqlite() -> (GridUserStore, PathBuf) {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ferroan-griduser-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp");
    let path = dir.join("GridUserStore.db");
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite/GridUserStore.db");
    std::fs::copy(&source, &path).expect("copy fixture");
    let store = open_sqlite_at(&path).await;
    (store, path)
}

async fn open_sqlite_at(path: &std::path::Path) -> GridUserStore {
    let pool = ferroan_db::open_sqlite_file(path, &["GridUserStore"])
        .await
        .expect("open sqlite");
    GridUserStore::open(pool).expect("store")
}

async fn open_url(var: &str, label: &str) -> Option<GridUserStore> {
    let Ok(url) = std::env::var(var) else {
        eprintln!("{var} is unset; {label} grid user store was not run");
        return None;
    };
    if url.is_empty() {
        eprintln!("{var} is empty; {label} grid user store was not run");
        return None;
    }
    let pool = Pool::connect_sqlx_url(&url)
        .await
        .unwrap_or_else(|error| panic!("{label} connect: {error}"));
    pool.require_stores(&["GridUserStore"])
        .await
        .unwrap_or_else(|error| panic!("{label} GridUserStore schema: {error}"));
    Some(GridUserStore::open(pool).expect(label))
}
