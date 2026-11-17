// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `[AssetService]` and `[Network]` keys `ferroan-robust` reads.
//!
//! Storage provider and connection string follow `AssetServiceBase`: the
//! asset section wins, and an empty value falls through to
//! `[DatabaseService]`. Only a SQLite provider is opened. The filesystem
//! asset loader is not run. `AuthType = BasicHttpAuthentication` and
//! `https_main = true` refuse to start, because this process would otherwise
//! serve the assets without the protection the file asked for.
//! `TrustedGridAuthentication` does not reject callers when no trust context
//! is set, which is this process, so it is accepted and does not add a check.

use ferroan_config::Config;
use ferroan_db::{Backend, Pool};

use super::http::{AssetHttp, AssetOptions, DeleteMode};
use super::store::AssetStore;

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
        if !is_stock_asset_service(module) {
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
        if !provider.contains("SQLite") {
            return Err(AssetError::Config(
                "this asset service only opens a SQLite store",
            ));
        }
        let connection = nonempty(config, SECTION, "ConnectionString")
            .or_else(|| nonempty(config, "DatabaseService", "ConnectionString"))
            .ok_or(AssetError::Config("No ConnectionString configured"))?;
        let pool = Pool::open(Backend::Sqlite, connection, &["AssetStore"]).await?;
        let store = AssetStore::new(pool)?;
        let delete = delete_mode(config)?;
        let redirect_url = nonempty(config, SECTION, "RedirectURL").map(str::to_string);
        if redirect_url
            .as_deref()
            .is_some_and(|url| url.contains(['\r', '\n']))
        {
            return Err(AssetError::Config("RedirectURL contains a line break"));
        }
        let reject_shard_header = !config_bool(config, "AllowllHTTPRequestIn", false)?;
        Ok(Self::new(
            store,
            AssetOptions {
                delete,
                redirect_url,
                reject_shard_header,
            },
        ))
    }
}

pub fn network_port(config: &Config) -> Result<u16, AssetError> {
    let text =
        nonempty(config, "Network", "port").ok_or(AssetError::Config("Network port is not set"))?;
    text.parse::<u16>()
        .map_err(|_| AssetError::Config("Network port is not a port number"))
}

fn is_stock_asset_service(module: &str) -> bool {
    let module = module.trim();
    if module.contains("FSAsset") || module.contains("XAsset") {
        return false;
    }
    module == "AssetService" || module.ends_with(":AssetService")
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
