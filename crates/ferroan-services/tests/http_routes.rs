// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The asset routes against a copy of Tranquillity's AssetStore SQLite file.
//! The file is schema only; the rows are inserted through the HTTP store.

// Integration tests are a test crate; the production lints stay on the library.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ferroan_config::Config;
use ferroan_db::open_sqlite_file;
use ferroan_services::{
    Asset, AssetHttp, AssetOptions, AssetStore, DeleteMode, decode_asset, encode_bool,
    encode_bools, encode_strings,
};
use tower::ServiceExt;
use uuid::Uuid;

static NEXT: AtomicU64 = AtomicU64::new(0);

#[tokio::test]
async fn store_get_exists_and_refuses_to_overwrite() {
    let (app, _dir) = router(AssetOptions::default()).await;
    let texture = fixture("asset/texture.xml");
    let stored = call(&app, "POST", "/assets/", &texture, None).await;
    assert_eq!(stored.status, StatusCode::OK);
    assert_eq!(stored.content_type, "application/xml");
    assert_eq!(stored.body, fixture("scalar/string-id.xml"));

    let got = call(
        &app,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff",
        &[],
        None,
    )
    .await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.content_type, "text/xml");
    assert_eq!(got.body, texture);

    let data = call(
        &app,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff/data",
        &[],
        None,
    )
    .await;
    assert_eq!(data.status, StatusCode::OK);
    assert_eq!(data.content_type, "application/octet-stream");
    assert_eq!(data.body, b"hello");

    let meta = call(
        &app,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff/metadata",
        &[],
        None,
    )
    .await;
    assert_eq!(meta.status, StatusCode::OK);
    assert_eq!(meta.content_type, "image/x-j2c");
    assert_eq!(meta.body, fixture("metadata/texture.xml"));

    let mut changed = decode_asset(&texture).unwrap();
    changed.asset.name = "replaced".to_string();
    let again = call(
        &app,
        "POST",
        "/assets/",
        &ferroan_services::encode_asset(&changed.asset),
        None,
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    let still = call(
        &app,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff",
        &[],
        None,
    )
    .await;
    assert_eq!(still.body, texture);

    let update = call(
        &app,
        "POST",
        "/assets/00112233-4455-6677-8899-aabbccddeeff",
        &texture,
        None,
    )
    .await;
    assert_eq!(update.status, StatusCode::OK);
    assert_eq!(update.content_type, "application/xml");
    assert_eq!(update.body, fixture("scalar/bool-false.xml"));
    assert_eq!(encode_bool(false), fixture("scalar/bool-false.xml"));

    let ids = encode_strings(&[
        "00112233-4455-6677-8899-aabbccddeeff",
        "00000000-0000-0000-0000-000000000000",
    ]);
    let exists = call(&app, "POST", "/get_assets_exist", &ids, None).await;
    assert_eq!(exists.body, encode_bools(&[true, false]));

    let bad = encode_strings(&["00112233-4455-6677-8899-aabbccddeeff", "not-a-uuid"]);
    let none = call(&app, "POST", "/get_assets_exist", &bad, None).await;
    assert_eq!(none.body, encode_bools(&[false, false]));

    let removed = call(
        &app,
        "DELETE",
        "/assets/00112233-4455-6677-8899-aabbccddeeff",
        &[],
        None,
    )
    .await;
    assert_eq!(removed.body, encode_bool(false));
    let kept = call(
        &app,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff",
        &[],
        None,
    )
    .await;
    assert_eq!(kept.status, StatusCode::OK);
}

/// TB1. A stored row whose flags are 8, or 9 (Maptile mixed with that unnamed
/// bit), is what `AssetServerGetHandler` serializes after
/// `StoredAssetFlags.FromStored`. The bodies are the oracle documents.
#[tokio::test]
async fn stored_unnamed_and_mixed_flags_match_the_csharp_get() {
    let path = scratch_db();
    let pool = open_sqlite_file(&path, &["AssetStore"])
        .await
        .expect("open asset store");
    let store = AssetStore::new(pool).expect("sqlite");
    let cases = [
        ("unnamed-flags", "33333333-4444-5555-6666-777777777777", 8),
        ("mixed-flags", "33333333-4444-5555-6666-777777777778", 9),
    ];
    for (name, id, flags) in cases {
        store
            .store_data(&Asset {
                id: Uuid::parse_str(id).expect("id"),
                name: name.to_string(),
                description: String::new(),
                type_code: 0,
                local: false,
                temporary: false,
                creator_id: "creator".to_string(),
                flags,
                data: Vec::new(),
                upload_attempts: 0,
            })
            .await
            .expect("store raw flags");
    }
    let app = AssetHttp::new(store, AssetOptions::default()).into_router();
    for (name, id, _) in cases {
        let got = call(&app, "GET", &format!("/assets/{id}"), &[], None).await;
        assert_eq!(got.status, StatusCode::OK, "{name}");
        assert_eq!(got.content_type, "text/xml", "{name}");
        assert_eq!(got.body, fixture(&format!("asset/{name}.xml")), "{name}");
    }
    let _ = std::fs::remove_dir_all(path.parent().expect("temp dir"));
}

#[tokio::test]
async fn long_names_are_truncated_to_64_utf16_units() {
    let (app, _dir) = router(AssetOptions::default()).await;
    let posted = call(
        &app,
        "POST",
        "/assets/",
        &fixture("asset/long-data.xml"),
        None,
    )
    .await;
    assert_eq!(posted.status, StatusCode::OK);
    let got = call(
        &app,
        "GET",
        "/assets/11111111-2222-3333-4444-555555555555",
        &[],
        None,
    )
    .await;
    let decoded = decode_asset(&got.body).unwrap();
    assert_eq!(decoded.asset.name, "n".repeat(64));
    assert_eq!(decoded.asset.description, "d".repeat(64));
    assert_eq!(decoded.asset.data.len(), 200);
}

#[tokio::test]
async fn maptile_delete_follows_the_flags() {
    let (app, _dir) = router(AssetOptions {
        delete: DeleteMode::Maptile,
        ..AssetOptions::default()
    })
    .await;
    let texture = "00112233-4455-6677-8899-aabbccddeeff";
    let maptile = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    call(
        &app,
        "POST",
        "/assets/",
        &fixture("asset/texture.xml"),
        None,
    )
    .await;
    call(
        &app,
        "POST",
        "/assets/",
        &fixture("asset/escaped.xml"),
        None,
    )
    .await;
    let blocked = call(&app, "DELETE", &format!("/assets/{texture}"), &[], None).await;
    assert_eq!(blocked.body, encode_bool(false));
    assert_eq!(
        call(&app, "GET", &format!("/assets/{texture}"), &[], None)
            .await
            .status,
        StatusCode::OK
    );
    let allowed = call(&app, "DELETE", &format!("/assets/{maptile}"), &[], None).await;
    assert_eq!(allowed.body, encode_bool(true));
    let gone = call(&app, "GET", &format!("/assets/{maptile}"), &[], None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.content_type, "text/plain");
    assert!(gone.body.is_empty());
}

#[tokio::test]
async fn missing_asset_redirects_and_unknown_commands_are_rejected() {
    let (app, _dir) = router(AssetOptions {
        redirect_url: Some("http://example.test".to_string()),
        ..AssetOptions::default()
    })
    .await;
    let missing = call(
        &app,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff/data",
        &[],
        None,
    )
    .await;
    assert_eq!(missing.status, StatusCode::FOUND);
    assert_eq!(
        missing.location.as_deref(),
        Some("http://example.test/assets/00112233-4455-6677-8899-aabbccddeeff/data")
    );
    let (slash, _dir) = router(AssetOptions {
        redirect_url: Some("http://example.test/".to_string()),
        ..AssetOptions::default()
    })
    .await;
    let doubled = call(
        &slash,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff",
        &[],
        None,
    )
    .await;
    assert_eq!(
        doubled.location.as_deref(),
        Some("http://example.test//assets/00112233-4455-6677-8899-aabbccddeeff")
    );
    let unknown = call(
        &app,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff/nope",
        &[],
        None,
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert_eq!(unknown.content_type, "text/plain");
    let bare = call(&app, "GET", "/assets", &[], None).await;
    assert_eq!(bare.status, StatusCode::OK);
    assert_eq!(bare.content_type, "application/xml");
    assert!(bare.body.is_empty());
    let bad = call(&app, "POST", "/assets/", b"<AssetBase>", None).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.content_type, "application/xml");
    assert!(bad.body.is_empty());
}

#[tokio::test]
async fn shard_header_is_forbidden_except_on_exists() {
    let (app, _dir) = router(AssetOptions::default()).await;
    let blocked = call(
        &app,
        "GET",
        "/assets/00112233-4455-6677-8899-aabbccddeeff",
        &[],
        Some("region"),
    )
    .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    let exists = call(
        &app,
        "POST",
        "/get_assets_exist",
        &encode_strings(&["00112233-4455-6677-8899-aabbccddeeff"]),
        Some("region"),
    )
    .await;
    assert_eq!(exists.status, StatusCode::OK);
    assert_eq!(exists.body, encode_bools(&[false]));
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused() {
    let path = scratch_db();
    let pool = open_sqlite_file(&path, &["AssetStore"]).await.unwrap();
    let app = AssetHttp::new(AssetStore::new(pool).unwrap(), AssetOptions::default())
        .into_router_limited(32);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/assets/")
                .body(Body::from(vec![b'a'; 64]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn open_reads_the_asset_section_and_rejects_other_backends() {
    let path = scratch_db();
    let mut config = Config::default();
    config.set(
        "AssetService",
        "LocalServiceModule",
        "OpenSim.Services.AssetService.dll:AssetService",
    );
    config.set("AssetService", "StorageProvider", "OpenSim.Data.SQLite.dll");
    config.set(
        "AssetService",
        "ConnectionString",
        &format!("Data Source={};Version=3", path.display()),
    );
    config.set("Network", "port", "8003");
    config.set("AssetService", "AllowRemoteDelete", "true");
    let http = match AssetHttp::open(&config).await {
        Ok(http) => http,
        Err(error) => panic!("sqlite asset service did not open: {error}"),
    };
    let _ = http.into_router();
    config.set(
        "AssetService",
        "LocalServiceModule",
        "OpenSim.Services.FSAssetService.dll:FSAssetConnector",
    );
    let Err(error) = AssetHttp::open(&config).await else {
        panic!("FSAssets was accepted");
    };
    assert!(error.to_string().contains("storage interface"), "{error}");
    config.set(
        "AssetService",
        "LocalServiceModule",
        "OpenSim.Services.AssetService.dll:XAssetService",
    );
    config.set("AssetService", "StorageProvider", "OpenSim.Data.SQLite.dll");
    let Err(error) = AssetHttp::open(&config).await else {
        panic!("SQLite XAsset was accepted");
    };
    assert!(error.to_string().contains("SQLite"), "{error}");
    config.set(
        "AssetService",
        "LocalServiceModule",
        "OpenSim.Services.AssetService.dll:AssetService",
    );
    config.set("AssetService", "StorageProvider", "OpenSim.Data.MySQL.dll");
    let Err(error) = AssetHttp::open(&config).await else {
        panic!("MySQL was accepted");
    };
    assert!(
        error.to_string().contains("version"),
        "a MySQL provider must fail on the SQLite connection string, not open it: {error}"
    );
    config.set("AssetService", "AuthType", "BasicHttpAuthentication");
    config.set("AssetService", "StorageProvider", "OpenSim.Data.SQLite.dll");
    let Err(error) = AssetHttp::open(&config).await else {
        panic!("basic auth was accepted");
    };
    assert!(error.to_string().contains("AuthType"), "{error}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

struct Answer {
    status: StatusCode,
    content_type: String,
    location: Option<String>,
    body: Vec<u8>,
}

async fn router(options: AssetOptions) -> (Router, PathBuf) {
    let path = scratch_db();
    let pool = open_sqlite_file(&path, &["AssetStore"])
        .await
        .expect("open asset store");
    let http = AssetHttp::new(AssetStore::new(pool).expect("sqlite"), options);
    (http.into_router(), path)
}

async fn call(app: &Router, method: &str, uri: &str, body: &[u8], shard: Option<&str>) -> Answer {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(shard) = shard {
        builder = builder.header("X-SecondLife-Shard", shard);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_vec())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .unwrap()
        .to_vec();
    Answer {
        status,
        content_type,
        location,
        body,
    }
}

fn fixture(relative: &str) -> Vec<u8> {
    std::fs::read(fixture_root().join(relative)).unwrap_or_else(|_| panic!("missing {relative}"))
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/asset")
}

fn scratch_db() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ferroan-asset-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("AssetStore.db");
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite/AssetStore.db");
    std::fs::copy(&source, &path).unwrap();
    path
}
