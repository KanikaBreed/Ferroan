// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Open SQLite databases created by Tranquillity's `Migration.Update`.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ferroan_db::{Backend, Pool, TRANQUILLITY_REVISION, expected_stores, open_sqlite_file};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn committed_sqlite_stores_match_tranquillity() {
    let dir = fixture_dir();
    let manifest = fs::read_to_string(dir.join("manifest.txt"))
        .expect("fixtures/db/sqlite/manifest.txt is missing; run cargo xtask schema generate");
    let manifest = manifest.replace("\r\n", "\n");
    let mut lines = manifest.lines();
    assert_eq!(lines.next(), Some("format 1"));
    let revision_line = format!("revision {TRANQUILLITY_REVISION}");
    assert_eq!(lines.next(), Some(revision_line.as_str()));
    let mut seen = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split(' ');
        let name = parts.next().unwrap();
        let version: i32 = parts.next().unwrap().parse().unwrap();
        let file = parts.next().unwrap();
        assert!(parts.next().is_none());
        assert_eq!(file, format!("{name}.db"));
        let expected = expected_stores(Backend::Sqlite)
            .iter()
            .find(|store| store.name == name)
            .unwrap_or_else(|| panic!("manifest store {name} is not in the catalog"));
        assert_eq!(version, expected.version, "{name}");
        open_sqlite_file(&dir.join(file), &[name])
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        seen.push(name.to_string());
    }
    let catalog: Vec<&str> = expected_stores(Backend::Sqlite)
        .iter()
        .map(|store| store.name)
        .collect();
    let seen_ref: Vec<&str> = seen.iter().map(String::as_str).collect();
    assert_eq!(seen_ref, catalog);
}

#[tokio::test]
async fn a_changed_version_is_refused() {
    let source = fixture_dir().join("AssetStore.db");
    let dir = scratch_dir();
    let path = dir.join("AssetStore.db");
    fs::copy(&source, &path).unwrap();
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::query("UPDATE migrations SET version = version + 1 WHERE name = 'AssetStore'")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let error = open_sqlite_file(&path, &["AssetStore"])
        .await
        .expect_err("a database Tranquillity did not migrate to this version must be refused");
    let text = error.to_string();
    assert!(text.contains("AssetStore"), "{text}");
    assert!(text.contains("expects 6"), "{text}");
}

#[tokio::test]
async fn a_database_without_the_migrations_table_is_refused() {
    let dir = scratch_dir();
    let path = dir.join("empty.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    pool.close().await;
    let error = Pool::open(
        Backend::Sqlite,
        &format!("Data Source={};Version=3", path.display()),
        &["AssetStore"],
    )
    .await
    .expect_err("an empty file is not a Tranquillity database");
    let text = error.to_string();
    assert!(text.contains("migrations") || text.contains("-1"), "{text}");
}

#[tokio::test]
async fn mysql_url_is_checked_when_a_server_is_configured() {
    let Ok(url) = env::var("FERROAN_MYSQL_URL") else {
        eprintln!("FERROAN_MYSQL_URL is unset; MySQL schema check was not run");
        return;
    };
    if url.is_empty() {
        eprintln!("FERROAN_MYSQL_URL is empty; MySQL schema check was not run");
        return;
    }
    let pool = Pool::connect_sqlx_url(&url)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let names: Vec<&str> = expected_stores(Backend::Mysql)
        .iter()
        .map(|store| store.name)
        .collect();
    pool.require_stores(&names)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let _ = pool;
}

#[tokio::test]
async fn postgres_url_is_checked_when_a_server_is_configured() {
    let Ok(url) = env::var("FERROAN_POSTGRES_URL") else {
        eprintln!("FERROAN_POSTGRES_URL is unset; PostgreSQL schema check was not run");
        return;
    };
    if url.is_empty() {
        eprintln!("FERROAN_POSTGRES_URL is empty; PostgreSQL schema check was not run");
        return;
    }
    let pool = Pool::connect_sqlx_url(&url)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let names: Vec<&str> = expected_stores(Backend::Postgres)
        .iter()
        .map(|store| store.name)
        .collect();
    pool.require_stores(&names)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite")
}

fn scratch_dir() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let base = env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let dir = base.join(format!("ferroan-db-{n}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
