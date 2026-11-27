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

use crate::AppearanceHttp;
use crate::asset::{Asset, AssetHttp, AssetOptions, AssetStore};
use crate::avatar::{AvatarHttp, AvatarService, AvatarStore};

#[test]
fn section_only_ini_enables_the_bake_read() {
    let mut config = Config::default();
    config.set(
        "AppearanceService",
        "LocalServiceModule",
        "OpenSim.Services.AvatarService.dll:AppearanceService",
    );
    assert_eq!(
        AppearanceHttp::selected(&config, 8003).unwrap().as_deref(),
        Some("AppearanceService")
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
        AppearanceHttp::selected(&config, 8003).unwrap().as_deref(),
        Some("AppearanceService")
    );
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
