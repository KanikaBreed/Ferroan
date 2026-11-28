// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use axum::body::{Body, to_bytes};
use axum::http::header;
use base64::{Engine, engine::general_purpose::STANDARD};
use ferroan_config::Config;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use crate::asset::{Asset, AssetHttp, AssetOptions, AssetStore};
use crate::avatar::{AvatarHttp, AvatarService, AvatarStore};
use crate::{AppearanceHttp, AppearanceRoute};

#[test]
fn section_only_ini_enables_the_bake_read() {
    let mut config = Config::default();
    config.set(
        "AppearanceService",
        "LocalServiceModule",
        "OpenSim.Services.AvatarService.dll:AppearanceService",
    );
    assert_eq!(
        AppearanceHttp::selected(&config, 8003).unwrap(),
        Some(AppearanceRoute {
            section: "AppearanceService".into(),
            public_port: None,
        })
    );
}

#[test]
fn a_connector_list_ignores_the_section_until_the_connector_is_named() {
    let mut config = Config::default();
    config.set(
        "AppearanceService",
        "LocalServiceModule",
        "OpenSim.Services.AvatarService.dll:AppearanceService",
    );
    config.set(
        "ServiceList",
        "AssetServiceConnector",
        "8003/OpenSim.Server.Handlers.dll:AssetServiceConnector",
    );
    assert!(AppearanceHttp::selected(&config, 8003).unwrap().is_none());
    config.set(
        "ServiceList",
        "AppearanceServiceConnector",
        "8002/OpenSim.Server.Handlers.dll:AppearanceServerConnector",
    );
    assert_eq!(
        AppearanceHttp::selected(&config, 8003).unwrap(),
        Some(AppearanceRoute {
            section: "AppearanceService".into(),
            public_port: Some(8002),
        })
    );
    config.set(
        "ServiceList",
        "AppearanceServiceConnector",
        "0/OpenSim.Server.Handlers.dll:AppearanceServerConnector",
    );
    assert_eq!(
        AppearanceHttp::selected(&config, 8003)
            .unwrap()
            .unwrap()
            .public_port,
        None
    );
    config.set(
        "ServiceList",
        "AppearanceServiceConnector",
        "8003/OpenSim.Server.Handlers.dll:AppearanceServerConnector",
    );
    assert_eq!(
        AppearanceHttp::selected(&config, 8003)
            .unwrap()
            .unwrap()
            .public_port,
        None
    );
    config.set(
        "ServiceList",
        "AppearanceServiceConnector",
        "AppearanceService@8004/OpenSim.Server.Handlers.dll:AppearanceServerConnector",
    );
    assert_eq!(
        AppearanceHttp::selected(&config, 8003).unwrap(),
        Some(AppearanceRoute {
            section: "AppearanceService".into(),
            public_port: Some(8004),
        })
    );
    config.set(
        "ServiceList",
        "AppearanceServiceConnector",
        "nope/OpenSim.Server.Handlers.dll:AppearanceServerConnector",
    );
    assert!(AppearanceHttp::selected(&config, 8003).is_err());
}

#[test]
fn a_named_connector_without_its_section_is_refused() {
    let mut config = Config::default();
    config.set(
        "ServiceList",
        "AppearanceServiceConnector",
        "8003/OpenSim.Server.Handlers.dll:AppearanceServerConnector",
    );
    assert!(AppearanceHttp::selected(&config, 8003).is_err());
}

#[tokio::test]
async fn viewer_bake_replies_and_rows_match_pin9() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../fixtures/appearance/service.json")).unwrap();
    assert_eq!(fixture["pin"], "41f18f983bb2bfc82e033fed410d361ba57d5aa4");
    assert_eq!(fixture["backend"], "sqlite");
    assert_eq!(
        fixture["seed"]["rows"], fixture["indexAfter"],
        "the pinned GET wrote the avatar index"
    );

    let dir = TempDir::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite");
    std::fs::copy(root.join("Avatar.db"), dir.path().join("Avatar.db")).unwrap();
    std::fs::copy(root.join("AssetStore.db"), dir.path().join("AssetStore.db")).unwrap();

    let avatar_pool = ferroan_db::open_sqlite_file(&dir.path().join("Avatar.db"), &["Avatar"])
        .await
        .unwrap();
    let avatar_store = AvatarStore::new(avatar_pool, "Avatars").unwrap();
    let agent = fixture["seed"]["agent"].as_str().unwrap();
    for row in fixture["seed"]["rows"].as_array().unwrap() {
        assert!(
            avatar_store
                .upsert(
                    agent,
                    row["name"].as_str().unwrap(),
                    row["value"].as_str().unwrap()
                )
                .await
                .unwrap()
        );
    }
    let avatars = AvatarHttp::new(AvatarService::new(avatar_store), true).service();

    let asset_pool =
        ferroan_db::open_sqlite_file(&dir.path().join("AssetStore.db"), &["AssetStore"])
            .await
            .unwrap();
    let asset_store = AssetStore::new(asset_pool).unwrap();
    for row in fixture["seed"]["assets"].as_array().unwrap() {
        let data = STANDARD.decode(row["data"].as_str().unwrap()).unwrap();
        asset_store
            .store_new(&Asset {
                id: Uuid::parse_str(row["id"].as_str().unwrap()).unwrap(),
                name: "Fixture bake".into(),
                description: String::new(),
                type_code: 0,
                local: false,
                temporary: false,
                creator_id: "00000000-0000-0000-0000-000000000000".into(),
                flags: 0,
                data,
                upload_attempts: 0,
            })
            .await
            .unwrap();
    }
    let assets = AssetHttp::new(asset_store, AssetOptions::default());
    let http = AppearanceHttp::from_services(avatars.clone(), assets.clone(), true);
    let router = http.into_router();

    for case in fixture["observations"].as_array().unwrap() {
        if let Some(id) = case["dropAsset"].as_str() {
            assert!(assets.remove_stored(id).await, "{}", case["label"]);
        }
        let label = case["label"].as_str().unwrap();
        let mut request = axum::http::Request::builder()
            .method(case["method"].as_str().unwrap())
            .uri(case["target"].as_str().unwrap());
        if case["shard"].as_bool() == Some(true) {
            request = request.header("X-SecondLife-Shard", "fixture");
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            u64::from(response.status().as_u16()),
            case["status"].as_u64().unwrap(),
            "{label}"
        );
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .map(|value| value.to_str().unwrap()),
            case["contentType"].as_str(),
            "{label}"
        );
        let reply = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            STANDARD.encode(reply),
            case["reply"].as_str().unwrap(),
            "{label}"
        );
    }

    let agent_id = Uuid::parse_str(agent).unwrap();
    let mut got: Vec<_> = avatars.get_avatar(agent_id).await.unwrap().fields;
    got.sort();
    let mut want: Vec<(String, String)> = fixture["indexAfter"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["name"].as_str().unwrap().to_string(),
                row["value"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    want.sort();
    assert_eq!(got, want);

    for row in fixture["assetsAfter"].as_array().unwrap() {
        let stored = assets.stored_bytes(row["id"].as_str().unwrap()).await;
        let encoded = stored
            .filter(|data| !data.is_empty())
            .map(|data| STANDARD.encode(data))
            .unwrap_or_default();
        assert_eq!(encoded, row["data"].as_str().unwrap(), "{}", row["id"]);
    }
}

#[tokio::test]
async fn appearance_modules_read_the_named_asset_store() {
    let dir = TempDir::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite");
    let grid_path = dir.path().join("grid.db");
    let bake_path = dir.path().join("bakes.db");
    std::fs::copy(root.join("Avatar.db"), dir.path().join("Avatar.db")).unwrap();
    std::fs::copy(root.join("AssetStore.db"), &grid_path).unwrap();
    std::fs::copy(root.join("AssetStore.db"), &bake_path).unwrap();

    let agent = "a7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    let grid_id = "b7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    let bake_id = "c7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    let grid_bytes = b"grid-bake".to_vec();
    let bake_bytes = b"other-bake".to_vec();

    let avatar_pool = ferroan_db::open_sqlite_file(&dir.path().join("Avatar.db"), &["Avatar"])
        .await
        .unwrap();
    let avatar_store = AvatarStore::new(avatar_pool, "Avatars").unwrap();
    avatar_store
        .upsert(agent, "Bake:Head", grid_id)
        .await
        .unwrap();
    avatar_store
        .upsert(agent, "Bake:Upper", bake_id)
        .await
        .unwrap();
    let avatars = AvatarHttp::new(AvatarService::new(avatar_store), true);

    let grid_pool = ferroan_db::open_sqlite_file(&grid_path, &["AssetStore"])
        .await
        .unwrap();
    let grid_store = AssetStore::new(grid_pool).unwrap();
    grid_store
        .store_new(&stored_asset(grid_id, grid_bytes.clone()))
        .await
        .unwrap();
    let grid_assets = AssetHttp::new(grid_store, AssetOptions::default());

    let bake_pool = ferroan_db::open_sqlite_file(&bake_path, &["AssetStore"])
        .await
        .unwrap();
    let bake_store = AssetStore::new(bake_pool).unwrap();
    bake_store
        .store_new(&stored_asset(bake_id, bake_bytes.clone()))
        .await
        .unwrap();
    drop(bake_store);

    let stock_avatar = "OpenSim.Services.AvatarService.dll:AvatarService";
    let stock_asset = "OpenSim.Services.AssetService.dll:AssetService";
    let mut config = Config::default();
    config.set(
        "AppearanceService",
        "LocalServiceModule",
        "OpenSim.Services.AvatarService.dll:AppearanceService",
    );
    config.set("AppearanceService", "AvatarService", stock_avatar);
    config.set("AppearanceService", "AssetService", stock_asset);
    config.set(
        "AppearanceService",
        "ConnectionString",
        &format!(
            "Data Source={};Version=3;",
            dir.path().join("missing.db").display()
        ),
    );
    config.set("AvatarService", "LocalServiceModule", stock_avatar);
    config.set("AssetService", "LocalServiceModule", stock_asset);
    config.set("AssetService", "StorageProvider", "OpenSim.Data.SQLite.dll");
    config.set(
        "AssetService",
        "ConnectionString",
        &format!("Data Source={};Version=3;", grid_path.display()),
    );
    config.set("BakeAssets", "StorageProvider", "OpenSim.Data.SQLite.dll");
    config.set(
        "BakeAssets",
        "ConnectionString",
        &format!("Data Source={};Version=3;", bake_path.display()),
    );

    let same = AppearanceHttp::open(&config, "AppearanceService", &avatars, grid_assets.clone())
        .await
        .unwrap()
        .into_router();
    let (status, body) = bake_reply(&same, &texture_path(agent, "head", grid_id)).await;
    assert_eq!(status, 200);
    assert_eq!(body, grid_bytes);
    let (status, body) = bake_reply(&same, &texture_path(agent, "upper", bake_id)).await;
    assert_eq!(status, 404);
    assert_ne!(body, bake_bytes);

    config.set("AppearanceService", "AssetService", "");
    let unset = AppearanceHttp::open(&config, "AppearanceService", &avatars, grid_assets.clone())
        .await
        .unwrap()
        .into_router();
    let (status, body) = bake_reply(&unset, &texture_path(agent, "head", grid_id)).await;
    assert_eq!((status, body), (200, grid_bytes.clone()));

    config.set(
        "AppearanceService",
        "AssetService",
        &format!("BakeAssets@{stock_asset}"),
    );
    let other = AppearanceHttp::open(&config, "AppearanceService", &avatars, grid_assets.clone())
        .await
        .unwrap()
        .into_router();
    let (status, body) = bake_reply(&other, &texture_path(agent, "head", grid_id)).await;
    assert_eq!(status, 404);
    assert_ne!(body, grid_bytes);
    let (status, body) = bake_reply(&other, &texture_path(agent, "upper", bake_id)).await;
    assert_eq!((status, body), (200, bake_bytes));

    config.set(
        "AppearanceService",
        "AssetService",
        "OpenSim.Services.FSAssetService.dll:FSAssetService",
    );
    let refused =
        AppearanceHttp::open(&config, "AppearanceService", &avatars, grid_assets.clone()).await;
    assert!(refused.is_err());

    config.set("AppearanceService", "AssetService", stock_asset);
    config.set(
        "AppearanceService",
        "AvatarService",
        "BakeAvatars@OpenSim.Services.AvatarService.dll:AvatarService",
    );
    let prefixed =
        AppearanceHttp::open(&config, "AppearanceService", &avatars, grid_assets.clone()).await;
    assert!(prefixed.is_err());

    let shared = "OpenSim.Services.FSAssetService.dll:FSAssetService";
    config.set("AppearanceService", "AvatarService", stock_avatar);
    config.set("AppearanceService", "AssetService", shared);
    config.set("AssetService", "LocalServiceModule", shared);
    let reused = AppearanceHttp::open(&config, "AppearanceService", &avatars, grid_assets)
        .await
        .unwrap()
        .into_router();
    let (status, body) = bake_reply(&reused, &texture_path(agent, "head", grid_id)).await;
    assert_eq!((status, body), (200, grid_bytes));
}

#[tokio::test]
async fn the_advertised_port_serves_the_bake_and_the_network_port_does_not() {
    let dir = TempDir::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite");
    std::fs::copy(root.join("Avatar.db"), dir.path().join("Avatar.db")).unwrap();
    std::fs::copy(root.join("AssetStore.db"), dir.path().join("AssetStore.db")).unwrap();
    let agent = "a7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    let bake_id = "b7d2ff2e-dc32-44d8-aa61-3d22070a4964";
    let bake_bytes = b"public-bake".to_vec();

    let avatar_pool = ferroan_db::open_sqlite_file(&dir.path().join("Avatar.db"), &["Avatar"])
        .await
        .unwrap();
    let avatar_store = AvatarStore::new(avatar_pool, "Avatars").unwrap();
    avatar_store
        .upsert(agent, "Bake:Head", bake_id)
        .await
        .unwrap();
    let avatars = AvatarHttp::new(AvatarService::new(avatar_store), true);
    let asset_pool =
        ferroan_db::open_sqlite_file(&dir.path().join("AssetStore.db"), &["AssetStore"])
            .await
            .unwrap();
    let asset_store = AssetStore::new(asset_pool).unwrap();
    asset_store
        .store_new(&stored_asset(bake_id, bake_bytes.clone()))
        .await
        .unwrap();
    let assets = AssetHttp::new(asset_store, AssetOptions::default());

    let public_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let network_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let public_port = public_listener.local_addr().unwrap().port();
    let network_port = network_listener.local_addr().unwrap().port();
    let stock_avatar = "OpenSim.Services.AvatarService.dll:AvatarService";
    let stock_asset = "OpenSim.Services.AssetService.dll:AssetService";
    let mut config = Config::default();
    config.set("Network", "port", &network_port.to_string());
    config.set(
        "ServiceList",
        "AppearanceServiceConnector",
        &format!("{public_port}/OpenSim.Server.Handlers.dll:AppearanceServerConnector"),
    );
    config.set(
        "AppearanceService",
        "LocalServiceModule",
        "OpenSim.Services.AvatarService.dll:AppearanceService",
    );
    config.set("AppearanceService", "AvatarService", stock_avatar);
    config.set("AppearanceService", "AssetService", stock_asset);
    config.set("AvatarService", "LocalServiceModule", stock_avatar);
    config.set("AssetService", "LocalServiceModule", stock_asset);

    let route = AppearanceHttp::selected(&config, network_port)
        .unwrap()
        .unwrap();
    assert_eq!(
        route,
        AppearanceRoute {
            section: "AppearanceService".into(),
            public_port: Some(public_port),
        }
    );
    let texture = AppearanceHttp::open(&config, &route.section, &avatars, assets)
        .await
        .unwrap()
        .into_router();
    let public_task = tokio::spawn(async move {
        axum::serve(ferroan_http::accepted_nodelay(public_listener), texture)
            .await
            .unwrap();
    });
    let network_task = tokio::spawn(async move {
        axum::serve(
            ferroan_http::accepted_nodelay(network_listener),
            axum::Router::new(),
        )
        .await
        .unwrap();
    });

    let path = texture_path(agent, "head", bake_id);
    let (status, body) = http_get(public_port, &path).await;
    assert_eq!(status, 200);
    assert_eq!(body, bake_bytes);
    let (status, body) = http_get(network_port, &path).await;
    assert_eq!(status, 404);
    assert_ne!(body, bake_bytes);
    public_task.abort();
    network_task.abort();
}

fn stored_asset(id: &str, data: Vec<u8>) -> Asset {
    Asset {
        id: Uuid::parse_str(id).unwrap(),
        name: "Fixture bake".into(),
        description: String::new(),
        type_code: 0,
        local: false,
        temporary: false,
        creator_id: "00000000-0000-0000-0000-000000000000".into(),
        flags: 0,
        data,
        upload_attempts: 0,
    }
}

fn texture_path(agent: &str, channel: &str, id: &str) -> String {
    format!("/texture/{agent}/{channel}/{id}")
}

async fn bake_reply(router: &axum::Router, path: &str) -> (u16, Vec<u8>) {
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = to_bytes(response.into_body(), 4096).await.unwrap().to_vec();
    (status, body)
}

async fn http_get(port: u16, path: &str) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut last = String::new();
    for _ in 0..40 {
        match tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await {
            Ok(mut stream) => {
                let request =
                    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut raw = Vec::new();
                stream.read_to_end(&mut raw).await.unwrap();
                let end = raw
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .unwrap();
                let head = std::str::from_utf8(&raw[..end]).unwrap();
                let status = head
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .parse()
                    .unwrap();
                return (status, raw[end + 4..].to_vec());
            }
            Err(error) => {
                last = error.to_string();
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
    }
    panic!("connect {port}: {last}");
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("appearance-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
