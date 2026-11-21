// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Services.AssetService/XAssetService.cs
//   Source/OpenSim.Services.AssetService/XAssetServiceBase.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `XAssetService` on the asset plugins.
//!
//! `Store` skips `StoreAsset` when `AssetsExist` is already true and still
//! returns the id. `UpdateContent` returns false. `Get` of a local miss, when
//! a chained `AssetService` is set, loads that asset, stores it, and deletes
//! it from the chain. `Delete` deletes on the chain first. An id that does
//! not parse is false and does not call either plugin.
//!
//! `XAssetServiceBase` creates `DefaultAssetLoader` only when a chain is set.
//! The constructor calls `ForEachDefaultXmlAsset` only when that loader is
//! set and no chain is set. Both cannot be true, so the call does not run.
//! This service does not seed the library. A chain plus a filesystem loader
//! does not read `AssetLoaderArgs`. An unknown loader name fails only when a
//! chain is set.
//!
//! MySQL `AssetsExist` reads `XAssetsMeta` (TB28), so a chained migrate
//! stores the asset and then deletes the chain row (TB33). PostgreSQL
//! metadata returns the page (TB29). PostgreSQL `Delete` binds `uuid` and
//! removes only the meta row (TB30). The data row stays. This is not TB19
//! or TB20.

use uuid::Uuid;

use super::model::{Asset, parse_id};
use super::store::{AssetStore, StoreError};
use super::xasset::XAssetStore;

pub struct XAssetService {
    store: XAssetStore,
    chain: Option<AssetStore>,
}

impl XAssetService {
    pub fn new(store: XAssetStore, chain: Option<AssetStore>) -> Self {
        Self { store, chain }
    }

    pub fn pool_occupancy(&self) -> (u32, u32) {
        if let Some(chain) = &self.chain {
            let _ = chain.pool_occupancy();
        }
        self.store.pool_occupancy()
    }

    /// `Get`. A local error is a miss and does not read the chain.
    pub async fn get(&self, id: &str) -> Option<Asset> {
        let uuid = parse_id(id)?;
        match self.store.get(uuid).await {
            Ok(Some(asset)) => Some(asset),
            Ok(None) => self.migrate(id).await,
            Err(_) => None,
        }
    }

    /// Plugin `AssetsExist`. Both backends read the XAsset metadata table.
    pub async fn exists(&self, ids: &[Uuid]) -> Result<Vec<bool>, StoreError> {
        self.store.exists(ids).await
    }

    /// Skip `StoreAsset` when `AssetsExist` is true. Always returns the id
    /// when that check and the plugin return.
    pub async fn store(&self, asset: &Asset) -> Result<String, StoreError> {
        let exists = self.store.exists(&[asset.id]).await?;
        if !exists.first().copied().unwrap_or(false) {
            self.store.store(asset).await?;
        }
        Ok(asset.id.to_string())
    }

    /// `Delete`. The chain runs first. Its bool is ignored. Its error aborts
    /// before the local plugin, which is the exception leaving this method.
    pub async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        if parse_id(id).is_none() {
            return Ok(false);
        }
        if let Some(chain) = &self.chain {
            let _ignored = chain.delete_data(id).await?;
        }
        self.store.delete(id).await
    }

    async fn migrate(&self, id: &str) -> Option<Asset> {
        let chain = self.chain.as_ref()?;
        let uuid = parse_id(id)?;
        let asset = match chain.get(uuid).await {
            Ok(asset) => asset?,
            Err(_) => return None,
        };
        // Store then chain.Delete. A store error is the outer Get catch:
        // the chain row stays. Exists reads XAssetsMeta (TB28), so the write
        // lands and the chain row is then removed.
        if self.store(&asset).await.is_err() {
            return None;
        }
        if chain.delete_data(&asset.id.to_string()).await.is_err() {
            return None;
        }
        Some(asset)
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ferroan_config::Config;
    use ferroan_db::{Backend, Pool};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::super::http::{AssetHttp, AssetOptions, DeleteMode};
    use super::super::model::{Asset, MAPTILE};
    use super::super::store::AssetStore;
    use super::super::xasset::{XASSET_MYSQL, XASSET_POSTGRES, XAssetStore};
    use super::super::xml;
    use super::XAssetService;

    #[tokio::test]
    async fn xasset_service_matches_the_pin() {
        if let Some(pair) = connect_both("FERROAN_MYSQL_URL", "MySQL").await {
            let _guard = XASSET_MYSQL.lock().await;
            check_service(pair, "FERROAN_MYSQL_URL", "MySQL", Backend::Mysql).await;
        }
        if let Some(pair) = connect_both("FERROAN_POSTGRES_URL", "PostgreSQL").await {
            let _guard = XASSET_POSTGRES.lock().await;
            check_service(
                pair,
                "FERROAN_POSTGRES_URL",
                "PostgreSQL",
                Backend::Postgres,
            )
            .await;
        }
    }

    async fn check_service(
        (store, chain): (XAssetStore, AssetStore),
        var: &str,
        label: &str,
        backend: Backend,
    ) {
        store.wipe().await.expect("wipe");
        for n in 1..=12 {
            chain.delete(xid(n)).await.expect("forget");
        }
        let (look_x, look_a) = connect_both(var, label).await.expect(label);
        let service = XAssetService::new(store, Some(chain));
        let app = AssetHttp::from_xasset(
            service,
            AssetOptions {
                delete: DeleteMode::All,
                ..AssetOptions::default()
            },
        )
        .into_router();

        let id = xid(1);
        let stored = call(
            &app,
            "POST",
            "/assets/",
            &xml::encode_asset(&sample(id, "first", 0)),
            None,
        )
        .await;
        assert_eq!(stored.0, StatusCode::OK);
        assert_eq!(stored.1, xml::encode_string(&id.to_string()));
        let got = call(&app, "GET", &asset_path(id), &[], None).await;
        assert_eq!(got.0, StatusCode::OK);
        let decoded = xml::decode_asset(&got.1).expect("asset");
        assert_eq!(decoded.asset.name, "first");
        assert_eq!(decoded.asset.data, b"q22b");
        let data = call(&app, "GET", &format!("/assets/{id}/data"), &[], None).await;
        assert_eq!(data.0, StatusCode::OK);
        assert_eq!(data.1, b"q22b");

        let again = call(
            &app,
            "POST",
            "/assets/",
            &xml::encode_asset(&sample(id, "rewritten", 0)),
            None,
        )
        .await;
        assert_eq!(again.0, StatusCode::OK);
        let after =
            xml::decode_asset(&call(&app, "GET", &asset_path(id), &[], None).await.1).expect("row");
        assert_eq!(after.asset.name, "first", "exists skips the second store");

        let update = call(
            &app,
            "POST",
            &asset_path(id),
            &xml::encode_asset(&sample(id, "updated", 0)),
            None,
        )
        .await;
        assert_eq!(update.0, StatusCode::OK);
        assert_eq!(update.1, xml::encode_bool(false));
        let still =
            xml::decode_asset(&call(&app, "GET", &asset_path(id), &[], None).await.1).expect("row");
        assert_eq!(still.asset.name, after.asset.name);

        let exists = call(
            &app,
            "POST",
            "/get_assets_exist",
            &xml::encode_strings(&[&id.to_string()]),
            None,
        )
        .await;
        assert_eq!(exists.0, StatusCode::OK);
        assert_eq!(exists.1, xml::encode_bools(&[true]));

        let shard = call(&app, "GET", &asset_path(id), &[], Some("region")).await;
        assert_eq!(shard.0, StatusCode::FORBIDDEN);

        let skip_id = xid(2);
        look_a
            .store_data(&sample(skip_id, "ordinary", 0))
            .await
            .expect("chain");
        let absent = call(
            &app,
            "POST",
            "/get_assets_exist",
            &xml::encode_strings(&[&skip_id.to_string()]),
            None,
        )
        .await;
        assert_eq!(absent.0, StatusCode::OK);
        assert_eq!(
            absent.1,
            xml::encode_bools(&[false]),
            "an id only in assets is absent from XAssetsMeta"
        );
        let written = call(
            &app,
            "POST",
            "/assets/",
            &xml::encode_asset(&sample(skip_id, "written", 0)),
            None,
        )
        .await;
        assert_eq!(written.0, StatusCode::OK);
        assert_eq!(written.1, xml::encode_string(&skip_id.to_string()));
        let row = look_x.get(skip_id).await.expect("get").expect("stored");
        assert_eq!(row.name, "written");
        let seen = call(
            &app,
            "POST",
            "/get_assets_exist",
            &xml::encode_strings(&[&skip_id.to_string()]),
            None,
        )
        .await;
        assert_eq!(seen.1, xml::encode_bools(&[true]));
        look_a.delete(skip_id).await.expect("clear");
        let _ = look_x.delete(&skip_id.to_string()).await;

        let migrate_id = xid(3);
        look_a
            .store_data(&sample(migrate_id, "chained", MAPTILE))
            .await
            .expect("seed");
        let migrated = call(&app, "GET", &asset_path(migrate_id), &[], None).await;
        assert_eq!(migrated.0, StatusCode::OK);
        let migrated_asset = xml::decode_asset(&migrated.1).expect("migrated");
        assert_eq!(migrated_asset.asset.name, "chained");
        assert_eq!(migrated_asset.asset.data, b"q22b");
        assert!(look_a.get(migrate_id).await.expect("chain").is_none());
        let second = call(&app, "GET", &asset_path(migrate_id), &[], None).await;
        assert!(look_x.get(migrate_id).await.expect("x").is_some());
        assert_eq!(second.0, StatusCode::OK);

        let both = xid(4);
        look_a
            .store_data(&sample(both, "chain-name", 0))
            .await
            .expect("chain");
        look_x
            .store(&sample(both, "local-name", 0))
            .await
            .expect("local");
        let fetched = xml::decode_asset(&call(&app, "GET", &asset_path(both), &[], None).await.1)
            .expect("local");
        assert_eq!(fetched.asset.name, "local-name");
        assert!(look_a.get(both).await.expect("chain").is_some());
        let removed = call(&app, "DELETE", &asset_path(both), &[], None).await;
        assert!(look_a.get(both).await.expect("chain").is_none());
        assert_eq!(removed.0, StatusCode::OK);
        assert_eq!(removed.1, xml::encode_bool(true));
        assert!(look_x.get(both).await.expect("x").is_none());

        let missing = call(&app, "DELETE", &asset_path(xid(5)), &[], None).await;
        assert_eq!(missing.0, StatusCode::OK);
        assert_eq!(missing.1, xml::encode_bool(false));

        let local_only = xid(6);
        look_x
            .store(&sample(local_only, "only-local", 0))
            .await
            .expect("local");
        let deleted = call(&app, "DELETE", &asset_path(local_only), &[], None).await;
        assert_eq!(deleted.0, StatusCode::OK);
        assert_eq!(deleted.1, xml::encode_bool(true));
        assert!(look_x.get(local_only).await.expect("x").is_none());

        drop(app);
        look_x.wipe().await.expect("wipe");
        look_x.close().await;
        look_a.close().await;

        let (plain, plain_chain) = connect_both(var, label).await.expect(label);
        plain.wipe().await.expect("wipe");
        let plain_service = XAssetService::new(plain, Some(plain_chain));
        assert!(!plain_service.delete("nope").await.expect("parse"));
        let kept = xid(7);
        plain_service
            .store
            .store(&sample(kept, "kept", 0))
            .await
            .expect("store");
        assert!(plain_service.store.get(kept).await.expect("get").is_some());
        plain_service.store.wipe().await.expect("wipe");
        plain_service.store.close().await;
        if let Some(chain) = plain_service.chain {
            chain.close().await;
        }

        check_maptile(var, label).await;
        check_open(var, label, backend).await;
    }

    async fn check_maptile(var: &str, label: &str) {
        let (store, chain) = connect_both(var, label).await.expect(label);
        store.wipe().await.expect("wipe");
        let normal = xid(8);
        let tile = xid(9);
        store
            .store(&sample(normal, "normal", 0))
            .await
            .expect("normal");
        store
            .store(&sample(tile, "tile", MAPTILE))
            .await
            .expect("tile");
        let (look_x, look_a) = connect_both(var, label).await.expect(label);
        let app = AssetHttp::from_xasset(
            XAssetService::new(store, Some(chain)),
            AssetOptions {
                delete: DeleteMode::Maptile,
                ..AssetOptions::default()
            },
        )
        .into_router();
        let refused = call(&app, "DELETE", &asset_path(normal), &[], None).await;
        assert_eq!(refused.0, StatusCode::OK);
        assert_eq!(refused.1, xml::encode_bool(false));
        assert!(look_x.get(normal).await.expect("kept").is_some());
        let allowed = call(&app, "DELETE", &asset_path(tile), &[], None).await;
        assert_eq!(allowed.0, StatusCode::OK);
        assert_eq!(allowed.1, xml::encode_bool(true));
        assert!(look_x.get(tile).await.expect("gone").is_none());
        drop(app);
        look_x.wipe().await.expect("wipe");
        look_x.close().await;
        look_a.close().await;
    }

    async fn check_open(var: &str, label: &str, backend: Backend) {
        let ado = ado_from_sqlx_url(&env::var(var).expect(label))
            .unwrap_or_else(|| panic!("{label} URL could not be turned into a connection string"));
        let provider = match backend {
            Backend::Mysql => "OpenSim.Data.MySQL.dll",
            Backend::Postgres => "OpenSim.Data.PGSQL.dll",
            Backend::Sqlite => panic!("sqlite"),
        };
        let mut config = base_config(provider, &ado);
        config.set("AssetService", "DefaultAssetLoader", "Nope.dll");
        let http = AssetHttp::open(&config).await.unwrap_or_else(|error| {
            panic!("{label} ignored a loader only when a chain is set: {error}")
        });
        drop(http);

        config.set(
            "AssetService",
            "DefaultAssetLoader",
            "OpenSim.Framework.AssetLoader.Filesystem.dll",
        );
        config.set(
            "AssetService",
            "AssetLoaderArgs",
            "/tmp/ferroan-q22b-missing-library.xml",
        );
        config.set(
            "AssetService",
            "ChainedServiceModule",
            "OpenSim.Services.AssetService.dll:AssetService",
        );
        let http = AssetHttp::open(&config).await.unwrap_or_else(|error| {
            panic!("{label} chained loader must not read the missing library: {error}")
        });
        drop(http);
        let (check, chain) = connect_both(var, label).await.expect(label);
        assert_eq!(check.data_rows().await.expect("rows"), 0);
        check.close().await;
        chain.close().await;
    }

    fn base_config(provider: &str, ado: &str) -> Config {
        let mut config = Config::default();
        config.set(
            "AssetService",
            "LocalServiceModule",
            "OpenSim.Services.AssetService.dll:XAssetService",
        );
        config.set("AssetService", "StorageProvider", provider);
        config.set("AssetService", "ConnectionString", ado);
        config.set("Network", "port", "8003");
        config
    }

    fn ado_from_sqlx_url(url: &str) -> Option<String> {
        let mysql = url.starts_with("mysql://");
        let rest = url
            .strip_prefix("mysql://")
            .or_else(|| url.strip_prefix("postgres://"))
            .or_else(|| url.strip_prefix("postgresql://"))?;
        let (userinfo, hostpart) = rest.rsplit_once('@')?;
        let (user, pass) = userinfo.split_once(':')?;
        if user.is_empty()
            || pass.is_empty()
            || user.contains([';', '\n', '\r'])
            || pass.contains([';', '\n', '\r'])
        {
            return None;
        }
        let (hostport, dbquery) = hostpart.split_once('/')?;
        let db = dbquery.split(['?', '#']).next().unwrap_or("");
        if db.is_empty() || db.contains([';', '\n', '\r']) {
            return None;
        }
        let (host, port) = match hostport.split_once(':') {
            Some((host, port)) => (host, port),
            None => (hostport, if mysql { "3306" } else { "5432" }),
        };
        if host.is_empty() || port.parse::<u16>().is_err() {
            return None;
        }
        if mysql {
            Some(format!(
                "Server={host};Port={port};Database={db};User ID={user};Password={pass};SslMode=None"
            ))
        } else {
            Some(format!(
                "Server={host};Port={port};Database={db};User Id={user};Password={pass};SSL Mode=Disable"
            ))
        }
    }

    async fn connect_both(var: &str, label: &str) -> Option<(XAssetStore, AssetStore)> {
        let Ok(url) = env::var(var) else {
            eprintln!("{var} is unset; {label} xasset service was not run");
            return None;
        };
        if url.is_empty() {
            eprintln!("{var} is empty; {label} xasset service was not run");
            return None;
        }
        Some((
            open_xasset(&url, label).await,
            open_asset(&url, label).await,
        ))
    }

    async fn open_xasset(url: &str, label: &str) -> XAssetStore {
        let pool = Pool::connect_sqlx_url(url)
            .await
            .unwrap_or_else(|error| panic!("{label} xasset connect: {error}"));
        pool.require_stores(&["XAssetStore"])
            .await
            .unwrap_or_else(|error| panic!("{label} XAssetStore schema: {error}"));
        XAssetStore::new(pool).expect(label)
    }

    async fn open_asset(url: &str, label: &str) -> AssetStore {
        let pool = Pool::connect_sqlx_url(url)
            .await
            .unwrap_or_else(|error| panic!("{label} asset connect: {error}"));
        pool.require_stores(&["AssetStore"])
            .await
            .unwrap_or_else(|error| panic!("{label} AssetStore schema: {error}"));
        AssetStore::new(pool).expect(label)
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: &[u8],
        shard: Option<&str>,
    ) -> (StatusCode, Vec<u8>) {
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
        let bytes = axum::body::to_bytes(response.into_body(), 2_000_000)
            .await
            .unwrap()
            .to_vec();
        (status, bytes)
    }

    fn sample(id: Uuid, name: &str, flags: i32) -> Asset {
        Asset {
            id,
            name: name.to_string(),
            description: "d".to_string(),
            type_code: 0,
            local: false,
            temporary: false,
            creator_id: "creator".to_string(),
            flags,
            data: b"q22b".to_vec(),
            upload_attempts: 0,
        }
    }

    fn xid(n: u32) -> Uuid {
        Uuid::parse_str(&format!("a22b0000-0000-4000-8000-{n:012x}")).expect("id")
    }

    fn asset_path(id: Uuid) -> String {
        format!("/assets/{id}")
    }
}
