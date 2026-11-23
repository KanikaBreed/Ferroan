// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Plugin behaviour of `MySQLFSAssetData` and `PGSQLFSAssetData`.
//!
//! MySQL and PostgreSQL run when `FERROAN_MYSQL_URL` or `FERROAN_POSTGRES_URL`
//! is set, and skip with a printed reason otherwise. The cross-read runs when
//! `cargo xtask asset fsassets-cross` sets `FERROAN_FSASSET_CROSS`.

use ferroan_db::{Backend, Pool};
use uuid::Uuid;

use super::super::fscommands::import_done_lines;
use super::super::fservice::FsAssetService;
use super::super::fsfiles::{FsFiles, sha256_hex};
use super::super::model::KNOWN_FLAGS;
use super::FsAssetStore;
use super::FsMetadata;

use super::{FSASSET_MYSQL, FSASSET_POSTGRES};

const HASH_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const HASH_B: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const IMPORT_TABLE: &str = "ferroan_fs_import";

#[tokio::test]
async fn fsassets_plugins_match_the_pin() {
    if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
        let _guard = FSASSET_MYSQL.lock().await;
        check_plugin(&store).await;
        store.close().await;
    }
    if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
        let _guard = FSASSET_POSTGRES.lock().await;
        check_plugin(&store).await;
        store.close().await;
    }
}

#[tokio::test]
async fn fsassets_plugins_cross_read() {
    let Ok(oracle) = std::env::var("FERROAN_ASSET_ORACLE") else {
        eprintln!("FERROAN_ASSET_ORACLE is unset; FSAssets cross-read was not run");
        return;
    };
    if oracle.is_empty() || std::env::var("FERROAN_FSASSET_CROSS").ok().as_deref() != Some("1") {
        eprintln!("FERROAN_FSASSET_CROSS is unset; FSAssets cross-read was not run");
        return;
    }
    if let Ok(ado) = std::env::var("FERROAN_ASSET_ADO_MYSQL") {
        if !ado.is_empty() {
            let _guard = FSASSET_MYSQL.lock().await;
            if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
                cross_one(&store, &oracle, "mysql", &ado).await;
                store.close().await;
            }
        }
    } else {
        eprintln!("FERROAN_ASSET_ADO_MYSQL is unset; MySQL FSAssets cross-read was not run");
    }
    if let Ok(ado) = std::env::var("FERROAN_ASSET_ADO_POSTGRES") {
        if !ado.is_empty() {
            let _guard = FSASSET_POSTGRES.lock().await;
            if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
                cross_one(&store, &oracle, "postgres", &ado).await;
                store.close().await;
            }
        }
    } else {
        eprintln!(
            "FERROAN_ASSET_ADO_POSTGRES is unset; PostgreSQL FSAssets cross-read was not run"
        );
    }
}

#[tokio::test]
async fn fs_console_commands_import_show_and_delete() {
    if let Ok(url) = std::env::var("FERROAN_MYSQL_URL")
        && !url.is_empty()
    {
        let _guard = FSASSET_MYSQL.lock().await;
        if let Some(store) = open_url("FERROAN_MYSQL_URL", "MySQL").await {
            command_round_trip(store, &url).await;
        }
    }
    if let Ok(url) = std::env::var("FERROAN_POSTGRES_URL")
        && !url.is_empty()
    {
        let _guard = FSASSET_POSTGRES.lock().await;
        if let Some(store) = open_url("FERROAN_POSTGRES_URL", "PostgreSQL").await {
            command_round_trip(store, &url).await;
        }
    }
}

async fn command_round_trip(store: FsAssetStore, url: &str) {
    store.wipe().await.expect("wipe");
    let root = std::env::temp_dir().join(format!(
        "ferroan-fs-cmd-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let base = root.join("base");
    let spool = root.join("spool");
    std::fs::create_dir_all(&base).expect("base");
    std::fs::create_dir_all(&spool).expect("spool");
    let files = FsFiles::new(&base, &spool, false);
    let service = FsAssetService::new(store, files.clone(), None, true);
    assert_eq!(service.console_commands().len(), 5);
    assert_eq!(
        service.console_show_assets().await.expect("count"),
        vec!["Total asset count: 0".to_string()]
    );
    assert_eq!(
        service
            .console_show_digest(&["show", "digest"])
            .await
            .expect("syntax"),
        vec!["Syntax: show digest <ID>".to_string()]
    );
    assert_eq!(
        service
            .console_delete_asset(&["delete"])
            .await
            .expect("syntax"),
        vec!["Syntax: delete asset <ID>".to_string()]
    );
    assert_eq!(
        service
            .console_import(&["import", "conn", "assets", "nope"])
            .await
            .expect("bad int"),
        vec!["ERROR: start and count must be integers".to_string()]
    );
    let failed = service
        .console_import(&["import", "", "assets"])
        .await
        .expect("bad conn");
    assert!(
        failed.iter().all(|line| !line.contains("Import done")),
        "a failed connection does not finish the import"
    );
    assert!(failed.is_empty());

    reset_import(service_store(&service)).await;
    insert_import(service_store(&service), 1, "one", vec![1, 2, 3]).await;
    insert_import(service_store(&service), 2, "two", vec![4]).await;
    let imported = service
        .console_import(&["import", url, IMPORT_TABLE])
        .await
        .expect("import");
    assert_eq!(imported, import_done_lines(2));
    assert_eq!(
        service.console_show_assets().await.expect("count"),
        vec!["Total asset count: 2".to_string()]
    );
    assert!(files.writer_pass());
    let first = iid(1);
    let digest = service
        .console_show_digest(&["show", "digest", &first.to_string()])
        .await
        .expect("digest");
    let mysql = service_store(&service).backend() == Backend::Mysql;
    if mysql {
        assert_eq!(digest[0], "Name: one");
        assert_eq!(digest[1], "Description: desc");
    } else {
        assert_eq!(digest[0], "Name: ");
        assert_eq!(digest[1], "Description: ");
    }
    assert_eq!(digest[2], "Type: 1");
    assert!(digest.iter().any(|line| line == "0000: 01-02-03"));
    let old_hash = sha256_hex(&[4]);
    assert!(digest_of(&service, iid(2)).await.contains(&old_hash));

    let gzip = files.gzip_path(&sha256_hex(&[1, 2, 3]));
    std::fs::remove_file(&gzip).expect("remove file");
    assert_eq!(
        service
            .console_show_digest(&["show", "digest", &first.to_string()])
            .await
            .expect("missing file"),
        vec!["Asset not found".to_string()]
    );
    assert_eq!(
        service
            .console_delete_asset(&["delete", "asset", &first.to_string()])
            .await
            .expect("delete"),
        vec!["Asset deleted".to_string()]
    );
    assert_eq!(
        service
            .console_delete_asset(&["delete", "asset", "not-a-uuid"])
            .await
            .expect("non-uuid"),
        vec!["Asset not found".to_string()]
    );
    assert_eq!(
        service.console_show_assets().await.expect("count"),
        vec!["Total asset count: 1".to_string()]
    );

    replace_import_data(service_store(&service), 2, vec![9, 9]).await;
    let again = service
        .console_import(&["import", url, IMPORT_TABLE, "1", "1"])
        .await
        .expect("page");
    assert_eq!(again, import_done_lines(1));
    assert!(files.writer_pass());
    assert!(
        digest_of(&service, iid(2)).await.contains(&old_hash),
        "an ordinary import leaves the stored hash"
    );
    let forced = service
        .console_import(&["force", "import", url, IMPORT_TABLE, "1", "1"])
        .await
        .expect("force");
    assert_eq!(forced, import_done_lines(1));
    assert!(files.writer_pass());
    let new_hash = sha256_hex(&[9, 9]);
    let after = digest_of(&service, iid(2)).await;
    assert!(after.contains(&new_hash));
    assert!(!after.contains(&old_hash));

    drop_import(service_store(&service)).await;
    service_store(&service).wipe().await.expect("wipe");
    let _ = std::fs::remove_dir_all(root);
}

fn service_store(service: &FsAssetService) -> &FsAssetStore {
    // The commands hold the store. Tests reach the same pool through the
    // service only for setup rows, via this crate-visible helper.
    service.store_for_test()
}

async fn digest_of(service: &FsAssetService, id: Uuid) -> String {
    service
        .console_show_digest(&["show", "digest", &id.to_string()])
        .await
        .expect("digest")
        .join("\n")
}

async fn replace_import_data(store: &FsAssetStore, n: u32, data: Vec<u8>) {
    let id = iid(n);
    match store.backend() {
        Backend::Mysql => {
            sqlx::query("UPDATE ferroan_fs_import SET data = ? WHERE id = ?")
                .bind(data)
                .bind(id.to_string())
                .execute(store.pool().mysql().expect("mysql"))
                .await
                .expect("update import");
        }
        Backend::Postgres => {
            sqlx::query("UPDATE ferroan_fs_import SET data = $1 WHERE id = $2::uuid")
                .bind(data)
                .bind(id.to_string())
                .execute(store.pool().postgres().expect("postgres"))
                .await
                .expect("update import");
        }
        Backend::Sqlite => panic!("sqlite"),
    }
}

async fn check_plugin(store: &FsAssetStore) {
    store.wipe().await.expect("wipe");
    let backend = store.backend();
    let id = fid(1);
    let kept = meta(id, "sea😀weed", "€ok😀", 9);
    assert!(store.store(&kept, HASH_A).await.expect("store"));
    let loaded = store.get(id).await.expect("get").expect("row");
    match backend {
        Backend::Mysql => {
            assert_eq!(loaded.metadata.name, "seaweed");
            assert_eq!(loaded.metadata.description, "€ok");
        }
        Backend::Postgres => {
            assert_eq!(loaded.metadata.name, "");
            assert_eq!(loaded.metadata.description, "");
        }
        Backend::Sqlite => panic!("sqlite"),
    }
    assert_eq!(loaded.metadata.type_code, 7);
    assert_eq!(loaded.metadata.flags, 9 & KNOWN_FLAGS);
    same_hash(HASH_A, &loaded.hash);
    let clock = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(clock.raw_flags, 9);
    same_hash(HASH_A, &clock.hash);
    let now = db_now(store).await;
    near(clock.create_time, now);
    near(clock.access_time, now);

    set_access(store, id, 50).await;
    let _ = store.get(id).await.expect("get");
    let bumped = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(bumped.create_time, clock.create_time);
    near(bumped.access_time, db_now(store).await);

    let again = meta(id, "other", "other", 1);
    assert!(store.store(&again, HASH_B).await.expect("second"));
    let stayed = store.clock(id).await.expect("clock").expect("row");
    same_hash(HASH_A, &stayed.hash);
    assert_eq!(stayed.raw_flags, 9);
    assert_eq!(stayed.create_time, clock.create_time);
    match backend {
        Backend::Mysql => assert_eq!(stayed.name, "seaweed"),
        Backend::Postgres => assert_eq!(stayed.name, ""),
        Backend::Sqlite => panic!("sqlite"),
    }

    if backend == Backend::Mysql {
        let edge = meta(fid(2), &format!("{}😀", "n".repeat(64)), "d", 1);
        assert!(store.store(&edge, HASH_B).await.expect("edge"));
        let trimmed = store.get(fid(2)).await.expect("get").expect("edge");
        assert_eq!(trimmed.metadata.name, "n".repeat(64));
        let long = meta(fid(3), &"n".repeat(65), "d", 1);
        assert!(store.store(&long, HASH_B).await.is_err(), "TB22 reports");
        assert!(store.get(fid(3)).await.expect("get").is_none());
    }

    let other = fid(4);
    assert!(
        store
            .store(&meta(other, "twin", "d", 1), HASH_B)
            .await
            .expect("twin")
    );
    let exists = store.exists(&[id, fid(9), other]).await.expect("exists");
    assert_eq!(exists, vec![true, false, true]);
    assert!(store.exists(&[]).await.expect("empty").is_empty());
    assert_eq!(
        store.count().await.expect("count"),
        if backend == Backend::Mysql { 3 } else { 2 }
    );

    let forced_id = fid(6);
    assert!(
        store
            .store(&meta(forced_id, "keep", "keep-desc", 2), HASH_A)
            .await
            .expect("force seed")
    );
    let before_force = store.clock(forced_id).await.expect("clock").expect("row");
    assert!(
        store
            .store_forced(&meta(forced_id, "other", "other", 4), HASH_B)
            .await
            .expect("force")
    );
    let forced = store.clock(forced_id).await.expect("clock").expect("row");
    same_hash(HASH_B, &forced.hash);
    assert_eq!(forced.raw_flags, 2);
    assert_eq!(forced.create_time, before_force.create_time);
    match backend {
        Backend::Mysql => {
            assert_eq!(forced.name, "keep");
            assert_eq!(forced.description, "keep-desc");
        }
        Backend::Postgres => assert_eq!(forced.name, ""),
        Backend::Sqlite => panic!("sqlite"),
    }

    check_import(store).await;
    check_interval(store, fid(5)).await;

    assert!(store.delete(&id.to_string()).await.expect("delete"));
    assert!(store.get(id).await.expect("get").is_none());
    assert!(store.get(other).await.expect("get").is_some());
    match backend {
        Backend::Mysql => assert!(store.delete("nope").await.expect("text")),
        Backend::Postgres => assert!(!store.delete("nope").await.expect("text")),
        Backend::Sqlite => panic!("sqlite"),
    }
    assert!(store.get(other).await.expect("twin").is_some());
    assert!(store.delete(&other.to_string()).await.expect("other"));
    assert!(store.get(fid(8)).await.expect("missing").is_none());
}

async fn check_interval(store: &FsAssetStore, id: Uuid) {
    store.set_access_days(30);
    assert!(
        store
            .store(&meta(id, "aged", "d", 1), HASH_A)
            .await
            .expect("aged")
    );
    let written = store.clock(id).await.expect("clock").expect("row");
    let _ = store.get(id).await.expect("get");
    let fresh = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(fresh.access_time, written.access_time);
    let now = db_now(store).await;
    set_access(store, id, now.saturating_sub(10 * 86_400)).await;
    let _ = store.get(id).await.expect("get");
    let young = store.clock(id).await.expect("clock").expect("row");
    assert_eq!(young.access_time, now.saturating_sub(10 * 86_400));
    set_access(store, id, now.saturating_sub(31 * 86_400)).await;
    let _ = store.get(id).await.expect("get");
    let old = store.clock(id).await.expect("clock").expect("row");
    near(old.access_time, db_now(store).await);
    assert_eq!(old.create_time, written.create_time);
    store.set_access_days(0);
}

async fn check_import(store: &FsAssetStore) {
    reset_import(store).await;
    insert_import(store, 1, "one", vec![1]).await;
    insert_import(store, 2, "two", vec![2, 2]).await;
    insert_import(store, 3, "three", vec![3, 3, 3]).await;
    let all = store.import_reader(IMPORT_TABLE, 0, -1).await.expect("all");
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].id, iid(1));
    assert_eq!(all[1].id, iid(2));
    assert_eq!(all[2].id, iid(3));
    match store.backend() {
        Backend::Mysql => {
            assert!(
                all.iter()
                    .any(|row| row.name == "two" && row.data == vec![2, 2])
            );
        }
        Backend::Postgres => {
            assert!(
                all.iter()
                    .all(|row| row.name.is_empty() && row.description.is_empty())
            );
            assert!(all.iter().any(|row| row.data == vec![2, 2]));
        }
        Backend::Sqlite => panic!("sqlite"),
    }
    let page = store.import_reader(IMPORT_TABLE, 1, 1).await.expect("page");
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, iid(2));
    let empty = store
        .import_reader(IMPORT_TABLE, 1, 0)
        .await
        .expect("empty page");
    assert!(empty.is_empty(), "LIMIT 0");
    let tail = store.import_reader(IMPORT_TABLE, 2, 1).await.expect("tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].id, iid(3));
    drop_import(store).await;
}

async fn cross_one(store: &FsAssetStore, oracle: &str, backend: &str, ado: &str) {
    store.wipe().await.expect("wipe");
    let mut differences = Vec::new();
    let id = fid(1);
    let rusted = meta(id, "sea😀weed", "€ok😀", 9);
    assert!(store.store(&rusted, HASH_A).await.expect("rust store"));
    let got = csharp(
        oracle,
        backend,
        ado,
        "get",
        serde_json::json!({"id": id.to_string()}),
    );
    assert_eq!(got["found"], true);
    assert_eq!(got["type"], 7);
    assert_eq!(got["flags"], 1);
    same_hash(HASH_A, got["hash"].as_str().expect("hash"));
    note_hash_padding(got["hash"].as_str().expect("hash"));
    match backend {
        "mysql" => {
            assert_eq!(got["name"], "seaweed");
            assert_eq!(got["description"], "€ok");
        }
        "postgres" => {
            assert_eq!(got["name"], "");
            assert_eq!(got["description"], "");
        }
        _ => panic!("backend"),
    }

    let exists = csharp(
        oracle,
        backend,
        ado,
        "exists",
        serde_json::json!({"ids": [id.to_string(), fid(9).to_string()]}),
    );
    assert_eq!(exists["exists"], serde_json::json!([true, false]));
    let counted = csharp(oracle, backend, ado, "count", serde_json::json!({}));
    assert_eq!(counted["count"], 1);

    let written = csharp(
        oracle,
        backend,
        ado,
        "put",
        serde_json::json!({
            "id": fid(2).to_string(),
            "name": "from-csharp",
            "description": "plain",
            "type": 3,
            "flags": 8,
            "hash": HASH_B
        }),
    );
    assert_eq!(written["stored"], true);
    let from_csharp = store.get(fid(2)).await.expect("get").expect("csharp row");
    assert_eq!(from_csharp.metadata.type_code, 3);
    assert_eq!(from_csharp.metadata.flags, 0);
    same_hash(HASH_B, &from_csharp.hash);
    if backend == "mysql" {
        assert_eq!(from_csharp.metadata.name, "from-csharp");
        assert_eq!(from_csharp.metadata.description, "plain");
        let lost = csharp(
            oracle,
            backend,
            ado,
            "put",
            serde_json::json!({
                "id": fid(3).to_string(),
                "name": "bad😀",
                "description": "d",
                "type": 1,
                "flags": 1,
                "hash": HASH_A
            }),
        );
        assert_eq!(lost["stored"], true);
        let kept = store
            .get(fid(3))
            .await
            .expect("get")
            .expect("emoji dropped");
        assert_eq!(kept.metadata.name, "bad");
        assert_eq!(kept.metadata.description, "d");
    }

    let second = csharp(
        oracle,
        backend,
        ado,
        "put",
        serde_json::json!({
            "id": fid(2).to_string(),
            "name": "again",
            "description": "again",
            "type": 4,
            "flags": 1,
            "hash": HASH_A
        }),
    );
    let after = store.clock(fid(2)).await.expect("clock").expect("row");
    same_hash(HASH_B, &after.hash);
    assert_eq!(second["stored"], true);
    if backend == "postgres" {
        assert!(
            store
                .store(&meta(fid(2), "", "", 1), HASH_A)
                .await
                .expect("rust second")
        );
        let still = store.clock(fid(2)).await.expect("clock").expect("row");
        same_hash(HASH_B, &still.hash);
    }

    let deleted = csharp(
        oracle,
        backend,
        ado,
        "delete",
        serde_json::json!({"id": id.to_string()}),
    );
    assert_eq!(deleted["deleted"], true);
    assert!(store.get(id).await.expect("get").is_none());
    if backend == "postgres" {
        // TB39 fixed (#490): a non-uuid deletes nothing and answers false.
        let bad = csharp(
            oracle,
            backend,
            ado,
            "delete",
            serde_json::json!({"id": "nope"}),
        );
        assert_eq!(bad["deleted"], false);
        assert!(!store.delete("nope").await.expect("rust nope"));
        assert!(store.get(fid(2)).await.expect("kept").is_some());
    } else {
        let bad = csharp(
            oracle,
            backend,
            ado,
            "delete",
            serde_json::json!({"id": "nope"}),
        );
        assert_eq!(bad["deleted"], true);
    }

    let force_id = fid(8);
    assert!(
        store
            .store(&meta(force_id, "kept-name", "kept-desc", 2), HASH_A)
            .await
            .expect("force seed")
    );
    let pin_force = csharp(
        oracle,
        backend,
        ado,
        "put",
        serde_json::json!({
            "id": force_id.to_string(),
            "name": "other",
            "description": "other",
            "type": 4,
            "flags": 1,
            "hash": HASH_B
        }),
    );
    assert_eq!(pin_force["stored"], true);
    let pin_row = store.clock(force_id).await.expect("clock").expect("row");
    same_hash(HASH_A, &pin_row.hash);
    assert!(
        store
            .store_forced(&meta(force_id, "other", "other", 4), HASH_B)
            .await
            .expect("rust force")
    );
    let forced = store.clock(force_id).await.expect("clock").expect("row");
    same_hash(HASH_B, &forced.hash);
    assert_eq!(forced.raw_flags, 2);
    assert_eq!(forced.create_time, pin_row.create_time);
    if backend == "mysql" {
        assert_eq!(forced.name, "kept-name");
        assert_eq!(forced.description, "kept-desc");
    }
    differences.push("TB45");

    cross_import(store, oracle, backend, ado).await;
    if differences.is_empty() {
        println!("fsassets: rows match");
    } else {
        println!("known differences: {}", differences.join(", "));
    }
}

async fn cross_import(store: &FsAssetStore, oracle: &str, backend: &str, ado: &str) {
    reset_import(store).await;
    insert_import(store, 1, "one", vec![1]).await;
    insert_import(store, 2, "two", vec![2, 2]).await;
    insert_import(store, 3, "three", vec![3, 3, 3]).await;
    let rust_rows = store
        .import_reader(IMPORT_TABLE, 0, -1)
        .await
        .expect("rust import");
    let csharp_rows = csharp(
        oracle,
        backend,
        ado,
        "import",
        serde_json::json!({"table": IMPORT_TABLE, "start": 0, "count": -1}),
    );
    let rows = csharp_rows["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), rust_rows.len());
    for row in &rust_rows {
        let data = serde_json::Value::from(row.data.clone());
        assert!(
            rows.iter()
                .any(|item| item["id"] == row.id.to_string() && item["data"] == data)
        );
    }
    if backend == "postgres" {
        let rust_page = store
            .import_reader(IMPORT_TABLE, 1, 0)
            .await
            .expect("rust page");
        assert!(
            rust_page.is_empty(),
            "LIMIT count OFFSET start with count 0"
        );
        let page = csharp(
            oracle,
            backend,
            ado,
            "import",
            serde_json::json!({"table": IMPORT_TABLE, "start": 1, "count": 0}),
        );
        // TB40 fixed (#491): the pin reads `limit count offset start` too.
        assert!(page["rows"].as_array().expect("page").is_empty());
    }
    drop_import(store).await;
}

fn meta(id: Uuid, name: &str, description: &str, flags: i32) -> FsMetadata {
    FsMetadata {
        id,
        name: name.to_string(),
        description: description.to_string(),
        type_code: 7,
        flags,
    }
}

fn fid(n: u32) -> Uuid {
    Uuid::parse_str(&format!("f23a0000-0000-4000-8000-{n:012x}")).expect("id")
}

fn iid(n: u32) -> Uuid {
    Uuid::parse_str(&format!("f23b0000-0000-4000-8000-{n:012x}")).expect("import id")
}

fn same_hash(stored: &str, loaded: &str) {
    assert_eq!(loaded.trim_end(), stored, "hash {loaded:?}");
}

fn note_hash_padding(loaded: &str) {
    if loaded.len() != loaded.trim_end().len() {
        eprintln!("hash padding: csharp length {}", loaded.len());
    }
}

fn near(value: i32, now: i32) {
    assert!(
        (i64::from(value) - i64::from(now)).abs() < 120,
        "{value} vs {now}"
    );
}

async fn db_now(store: &FsAssetStore) -> i32 {
    match store.backend() {
        Backend::Mysql => {
            let row = sqlx::query("SELECT CAST(UNIX_TIMESTAMP() AS SIGNED) AS n")
                .fetch_one(store.pool().mysql().expect("mysql"))
                .await
                .expect("now");
            super::super::decode::column_i32(&row, "n").expect("now")
        }
        Backend::Postgres => super::super::time::postgres_unix_now(),
        Backend::Sqlite => panic!("sqlite"),
    }
}

async fn set_access(store: &FsAssetStore, id: Uuid, access: i32) {
    match store.backend() {
        Backend::Mysql => {
            sqlx::query("UPDATE fsassets SET access_time = ? WHERE id = ?")
                .bind(access)
                .bind(id.to_string())
                .execute(store.pool().mysql().expect("mysql"))
                .await
                .expect("access");
        }
        Backend::Postgres => {
            sqlx::query("UPDATE fsassets SET access_time = $2 WHERE id = $1::uuid")
                .bind(id.to_string())
                .bind(access)
                .execute(store.pool().postgres().expect("postgres"))
                .await
                .expect("access");
        }
        Backend::Sqlite => panic!("sqlite"),
    }
}

async fn reset_import(store: &FsAssetStore) {
    drop_import(store).await;
    let sql = match store.backend() {
        Backend::Mysql => {
            "CREATE TABLE ferroan_fs_import (
                id char(36) NOT NULL,
                name varchar(64) NOT NULL DEFAULT '',
                description varchar(64) NOT NULL DEFAULT '',
                assetType tinyint NOT NULL,
                create_time int NOT NULL DEFAULT 0,
                data longblob NOT NULL,
                PRIMARY KEY (id)
            ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3"
        }
        Backend::Postgres => {
            "CREATE TABLE ferroan_fs_import (
                id uuid NOT NULL PRIMARY KEY,
                name varchar(64) NOT NULL DEFAULT '',
                description varchar(64) NOT NULL DEFAULT '',
                \"assetType\" smallint NOT NULL,
                create_time integer NOT NULL DEFAULT 0,
                data bytea NOT NULL
            )"
        }
        Backend::Sqlite => panic!("sqlite"),
    };
    match store.backend() {
        Backend::Mysql => {
            sqlx::query(sql)
                .execute(store.pool().mysql().expect("mysql"))
                .await
                .expect("import table");
        }
        Backend::Postgres => {
            sqlx::query(sql)
                .execute(store.pool().postgres().expect("postgres"))
                .await
                .expect("import table");
        }
        Backend::Sqlite => panic!("sqlite"),
    }
}

async fn drop_import(store: &FsAssetStore) {
    match store.backend() {
        Backend::Mysql => {
            sqlx::query("DROP TABLE IF EXISTS ferroan_fs_import")
                .execute(store.pool().mysql().expect("mysql"))
                .await
                .expect("drop import");
        }
        Backend::Postgres => {
            sqlx::query("DROP TABLE IF EXISTS ferroan_fs_import")
                .execute(store.pool().postgres().expect("postgres"))
                .await
                .expect("drop import");
        }
        Backend::Sqlite => panic!("sqlite"),
    }
}

async fn insert_import(store: &FsAssetStore, n: u32, name: &str, data: Vec<u8>) {
    let id = iid(n);
    match store.backend() {
        Backend::Mysql => {
            sqlx::query(
                "INSERT INTO ferroan_fs_import (id, name, description, assetType, create_time, data) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(name)
            .bind("desc")
            .bind(n as i32)
            .bind(1_700_000_000)
            .bind(data)
            .execute(store.pool().mysql().expect("mysql"))
            .await
            .expect("import row");
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO ferroan_fs_import (id, name, description, \"assetType\", create_time, data) \
                 VALUES ($1::uuid, $2, $3, $4, $5, $6)",
            )
            .bind(id.to_string())
            .bind(name)
            .bind("desc")
            .bind(n as i32)
            .bind(1_700_000_000)
            .bind(data)
            .execute(store.pool().postgres().expect("postgres"))
            .await
            .expect("import row");
        }
        Backend::Sqlite => panic!("sqlite"),
    }
}

async fn open_url(var: &str, label: &str) -> Option<FsAssetStore> {
    let Ok(url) = std::env::var(var) else {
        eprintln!("{var} is unset; {label} fsassets store was not run");
        return None;
    };
    if url.is_empty() {
        eprintln!("{var} is empty; {label} fsassets store was not run");
        return None;
    }
    let pool = Pool::connect_sqlx_url(&url)
        .await
        .unwrap_or_else(|error| panic!("{label} connect: {error}"));
    pool.require_stores(&["FSAssetStore"])
        .await
        .unwrap_or_else(|error| panic!("{label} FSAssetStore schema: {error}"));
    let realm = match pool.backend() {
        Backend::Postgres => "not_used",
        Backend::Mysql | Backend::Sqlite => "fsassets",
    };
    Some(FsAssetStore::with_settings(pool, realm, 0).expect(label))
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
        .arg("fdb")
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
    serde_json::from_str(line)
        .unwrap_or_else(|error| panic!("fsassets json {error}: {}", scrub(&stdout)))
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
