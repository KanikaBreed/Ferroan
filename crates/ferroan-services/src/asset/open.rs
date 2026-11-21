// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `[AssetService]` and `[Network]` keys `ferroan-robust` reads.
//!
//! Storage provider and connection string follow `AssetServiceBase`: the
//! asset section wins, and an empty value falls through to
//! `[DatabaseService]`. SQLite, MySQL and PostgreSQL providers are opened for
//! `AssetService`. `XAssetService` opens MySQL and PostgreSQL only. FSAssets
//! is not served. `DefaultAssetLoader` names the filesystem loader, and
//! `AssetLoaderArgs` is the asset-set control file. An empty loader name
//! loads nothing. `AssetLoaderEnabled = false` skips the load. `XAssetService`
//! loads a loader name only when `ChainedServiceModule` is set, and it does
//! not read the library file: `ForEachDefaultXmlAsset` is unreachable at the
//! pin. `AuthType = BasicHttpAuthentication` and `https_main = true` refuse to
//! start, because this process would otherwise serve the assets without the
//! protection the file asked for. `TrustedGridAuthentication` does not reject
//! callers when no trust context is set, which is this process, so it is
//! accepted and does not add a check.

use ferroan_config::Config;
use ferroan_db::{Backend, Pool};

use super::http::{AssetHttp, AssetOptions, DeleteMode};
use super::loader::{filesystem_loader, load_asset_sets, seed_assets};
use super::store::AssetStore;
use super::xasset::XAssetStore;
use super::xservice::XAssetService;

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("{0}")]
    Config(&'static str),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Db(#[from] ferroan_db::DbError),
    #[error(transparent)]
    Store(#[from] super::store::StoreError),
}

const SECTION: &str = "AssetService";

impl AssetHttp {
    pub async fn open(config: &Config) -> Result<Self, AssetError> {
        let module = nonempty(config, SECTION, "LocalServiceModule")
            .ok_or(AssetError::Config("No LocalServiceModule in config file"))?;
        if !is_xasset_service(module) && !is_stock_asset_service(module) {
            return Err(AssetError::Config(
                "only OpenSim.Services.AssetService.dll:AssetService is served; FSAssets is not ported",
            ));
        }
        if https_main(config)? {
            return Err(AssetError::Config(
                "https_main is set and this build serves HTTP only",
            ));
        }
        let auth = section_value(config, "AuthType").unwrap_or("None");
        if auth != "None" && auth != "TrustedGridAuthentication" {
            return Err(AssetError::Message(format!(
                "AuthType {auth} is not implemented"
            )));
        }
        let provider = nonempty(config, SECTION, "StorageProvider")
            .or_else(|| nonempty(config, "DatabaseService", "StorageProvider"))
            .ok_or(AssetError::Config("No StorageProvider configured"))?;
        let backend = asset_backend(provider)?;
        let connection = nonempty(config, SECTION, "ConnectionString")
            .or_else(|| nonempty(config, "DatabaseService", "ConnectionString"))
            .ok_or(AssetError::Config("No ConnectionString configured"))?;
        let delete = delete_mode(config)?;
        let redirect_url = nonempty(config, SECTION, "RedirectURL").map(str::to_string);
        if redirect_url
            .as_deref()
            .is_some_and(|url| url.contains(['\r', '\n']))
        {
            return Err(AssetError::Config("RedirectURL contains a line break"));
        }
        let reject_shard_header = !config_bool(config, "AllowllHTTPRequestIn", false)?;
        let options = AssetOptions {
            delete,
            redirect_url,
            reject_shard_header,
        };
        if is_xasset_service(module) {
            return open_xasset(config, backend, connection, options).await;
        }
        let pool = Pool::open(backend, connection, &["AssetStore"]).await?;
        let store = AssetStore::new(pool)?;
        load_configured_assets(&store, config).await?;
        Ok(Self::new(store, options))
    }
}

async fn open_xasset(
    config: &Config,
    backend: Backend,
    connection: &str,
    options: AssetOptions,
) -> Result<AssetHttp, AssetError> {
    if backend == Backend::Sqlite {
        return Err(AssetError::Config("XAssetService has no SQLite plugin"));
    }
    let chained = match nonempty(config, SECTION, "ChainedServiceModule") {
        Some(name) if is_stock_asset_service(name) => true,
        Some(name) => {
            return Err(AssetError::Message(format!(
                "Failed to load ChainedAssetService from {name}"
            )));
        }
        None => false,
    };
    if chained && let Some(loader) = nonempty(config, SECTION, "DefaultAssetLoader") {
        // The loader object is created only when a chain is set. The
        // constructor then calls ForEachDefaultXmlAsset only when no chain
        // is set, so the library file is never read.
        if !filesystem_loader(loader) {
            return Err(AssetError::Message(format!(
                "Asset loader could not be loaded from {loader}"
            )));
        }
    }
    let pool = Pool::open(backend, connection, &["XAssetStore"]).await?;
    let store = XAssetStore::new(pool)?;
    let chain = if chained {
        let pool = Pool::open(backend, connection, &["AssetStore"]).await?;
        Some(AssetStore::new(pool)?)
    } else {
        None
    };
    Ok(AssetHttp::from_xasset(
        XAssetService::new(store, chain),
        options,
    ))
}

pub fn network_port(config: &Config) -> Result<u16, AssetError> {
    let text =
        nonempty(config, "Network", "port").ok_or(AssetError::Config("Network port is not set"))?;
    text.parse::<u16>()
        .map_err(|_| AssetError::Config("Network port is not a port number"))
}

/// Address of `[Network] bind_ip_address`.
///
/// Empty or absent means every IPv4 interface (`0.0.0.0`), which is what
/// Tranquillity does when the key is unset. A value must be an IP address.
/// The harness sets `127.0.0.1`. The name is the one Tranquillity PR #412 adds.
pub fn listen_address(config: &Config) -> Result<std::net::IpAddr, AssetError> {
    let Some(text) = config.get("Network", "bind_ip_address") else {
        return Ok(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    }
    if text.len() > 64 {
        return Err(AssetError::Config(
            "[Network] bind_ip_address is not an IP address",
        ));
    }
    text.parse()
        .map_err(|_| AssetError::Config("[Network] bind_ip_address is not an IP address"))
}

async fn load_configured_assets(store: &AssetStore, config: &Config) -> Result<(), AssetError> {
    let Some(name) = nonempty(config, SECTION, "DefaultAssetLoader") else {
        return Ok(());
    };
    if !filesystem_loader(name) {
        return Err(AssetError::Message(format!(
            "Asset loader could not be loaded from {name}"
        )));
    }
    if !config_bool_in(config, SECTION, "AssetLoaderEnabled", true)? {
        return Ok(());
    }
    let args = config.get(SECTION, "AssetLoaderArgs").unwrap_or("");
    let loaded = load_asset_sets(std::path::Path::new(args))
        .map_err(|error| AssetError::Message(error.to_string()))?;
    seed_assets(store, &loaded)
        .await
        .map_err(|error| AssetError::Message(error.to_string()))?;
    Ok(())
}

fn asset_backend(provider: &str) -> Result<Backend, AssetError> {
    if provider.contains("SQLite") {
        Ok(Backend::Sqlite)
    } else if provider.contains("MySQL") {
        Ok(Backend::Mysql)
    } else if provider.contains("PGSQL") || provider.contains("Postgres") {
        Ok(Backend::Postgres)
    } else {
        Err(AssetError::Config(
            "this asset service opens SQLite, MySQL and PostgreSQL stores",
        ))
    }
}

fn is_stock_asset_service(module: &str) -> bool {
    let module = module.trim();
    if module.contains("FSAsset") || module.contains("XAsset") {
        return false;
    }
    module == "AssetService" || module.ends_with(":AssetService")
}

fn is_xasset_service(module: &str) -> bool {
    let module = module.trim();
    !module.contains("FSAsset") && module.contains("XAsset")
}

fn delete_mode(config: &Config) -> Result<DeleteMode, AssetError> {
    let allow = config_bool_in(config, SECTION, "AllowRemoteDelete", false)?;
    let all = config_bool_in(config, SECTION, "AllowRemoteDeleteAllTypes", false)?;
    Ok(match (allow, all) {
        (false, _) => DeleteMode::None,
        (true, false) => DeleteMode::Maptile,
        (true, true) => DeleteMode::All,
    })
}

fn https_main(config: &Config) -> Result<bool, AssetError> {
    config_bool_in(config, "Network", "https_main", false)
}

/// Later section wins, matching `Util.GetConfigVarFromSections` for
/// `{ "Network", "AssetService" }`.
fn section_value<'a>(config: &'a Config, key: &str) -> Option<&'a str> {
    let mut found = None;
    for section in ["Network", SECTION] {
        if let Some(value) = nonempty(config, section, key) {
            found = Some(value);
        }
    }
    found
}

fn config_bool(config: &Config, key: &str, default: bool) -> Result<bool, AssetError> {
    let mut value = default;
    for section in ["Network", SECTION] {
        if config.get(section, key).is_some() {
            value = config_bool_in(config, section, key, value)?;
        }
    }
    Ok(value)
}

fn config_bool_in(
    config: &Config,
    section: &str,
    key: &str,
    default: bool,
) -> Result<bool, AssetError> {
    let Some(text) = config.get(section, key) else {
        return Ok(default);
    };
    if text.is_empty() {
        return Ok(default);
    }
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(AssetError::Message(format!(
            "{section} {key} is not a boolean"
        ))),
    }
}

fn nonempty<'a>(config: &'a Config, section: &str, key: &str) -> Option<&'a str> {
    config.get(section, key).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    fn parsed(text: &str) -> Config {
        Config::parse(text).expect("ini")
    }

    #[test]
    fn listen_address_defaults_to_every_ipv4_interface() {
        let missing = listen_address(&parsed("[Network]\nport = 9000\n")).expect("default");
        assert_eq!(missing, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        let empty = listen_address(&parsed("[Network]\nbind_ip_address =\n")).expect("empty");
        assert_eq!(empty, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        let blank = listen_address(&parsed("[Network]\nbind_ip_address =   \n")).expect("blank");
        assert_eq!(blank, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }

    #[test]
    fn listen_address_accepts_an_ip_and_rejects_a_name() {
        let v4 = listen_address(&parsed("[Network]\nbind_ip_address = 127.0.0.1\n")).expect("v4");
        assert_eq!(v4, IpAddr::V4(Ipv4Addr::LOCALHOST));
        let v6 = listen_address(&parsed("[Network]\nbind_ip_address = ::1\n")).expect("v6");
        assert_eq!(v6, IpAddr::V6(Ipv6Addr::LOCALHOST));
        let explicit =
            listen_address(&parsed("[Network]\nbind_ip_address = 0.0.0.0\n")).expect("any");
        assert_eq!(explicit, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert!(listen_address(&parsed("[Network]\nbind_ip_address = localhost\n")).is_err());
    }

    #[tokio::test]
    async fn xasset_refuses_sqlite_a_bad_chain_and_an_unknown_loader() {
        let sqlite = parsed(
            "[AssetService]\n\
             LocalServiceModule = OpenSim.Services.AssetService.dll:XAssetService\n\
             StorageProvider = OpenSim.Data.SQLite.dll\n\
             ConnectionString = Data Source=/tmp/ferroan-q22b-unused.db;Version=3\n",
        );
        let Err(error) = AssetHttp::open(&sqlite).await else {
            panic!("sqlite xasset was accepted");
        };
        assert!(error.to_string().contains("SQLite"), "{error}");

        let chained = parsed(
            "[AssetService]\n\
             LocalServiceModule = OpenSim.Services.AssetService.dll:XAssetService\n\
             StorageProvider = OpenSim.Data.MySQL.dll\n\
             ConnectionString = Server=127.0.0.1;Port=1;Database=ferroan;User ID=ferroan;Password=ferroan;SslMode=None\n\
             ChainedServiceModule = OpenSim.Services.AssetService.dll:XAssetService\n",
        );
        let Err(error) = AssetHttp::open(&chained).await else {
            panic!("bad chain was accepted");
        };
        assert!(
            error
                .to_string()
                .contains("Failed to load ChainedAssetService"),
            "{error}"
        );

        let loader = parsed(
            "[AssetService]\n\
             LocalServiceModule = OpenSim.Services.AssetService.dll:XAssetService\n\
             StorageProvider = OpenSim.Data.MySQL.dll\n\
             ConnectionString = Server=127.0.0.1;Port=1;Database=ferroan;User ID=ferroan;Password=ferroan;SslMode=None\n\
             ChainedServiceModule = OpenSim.Services.AssetService.dll:AssetService\n\
             DefaultAssetLoader = Nope.dll\n",
        );
        let Err(error) = AssetHttp::open(&loader).await else {
            panic!("unknown loader was accepted");
        };
        assert!(
            error
                .to_string()
                .contains("Asset loader could not be loaded"),
            "{error}"
        );
    }
}
