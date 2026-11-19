// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `[UserAliasService]` keys `ferroan-robust` reads.
//!
//! The connection string falls through to `[DatabaseService]`. Only SQLite is
//! opened. The table must already exist; this process does not create it and
//! does not write a migrations version the pin does not have.

use ferroan_config::Config;
use ferroan_db::{Backend, Pool};

use super::http::AliasHttp;
use super::service::AliasService;
use super::store::AliasStore;

#[derive(Debug, thiserror::Error)]
pub enum AliasError {
    #[error("{0}")]
    Config(&'static str),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Db(#[from] ferroan_db::DbError),
    #[error(transparent)]
    Store(#[from] super::store::StoreError),
}

const SECTION: &str = "UserAliasService";

impl AliasHttp {
    pub async fn open(config: &Config) -> Result<Self, AliasError> {
        let module = nonempty(config, SECTION, "LocalServiceModule")
            .ok_or(AliasError::Config("No LocalServiceModule in config file"))?;
        if module != "UserAliasService" && !module.ends_with(":UserAliasService") {
            return Err(AliasError::Config(
                "only OpenSim.Services.UserAccountService.dll:UserAliasService is served",
            ));
        }
        if config_bool_in(config, "Network", "https_main", false)? {
            return Err(AliasError::Config(
                "https_main is set and this build serves HTTP only",
            ));
        }
        let auth = auth_type(config);
        if auth != "None" && auth != "TrustedGridAuthentication" {
            return Err(AliasError::Message(format!(
                "AuthType {auth} is not implemented"
            )));
        }
        let provider = nonempty(config, SECTION, "StorageProvider")
            .or_else(|| nonempty(config, "DatabaseService", "StorageProvider"))
            .ok_or(AliasError::Config("No StorageProvider configured"))?;
        if !provider.contains("SQLite") {
            return Err(AliasError::Config(
                "this alias service only opens a SQLite store; MySQL and PostgreSQL wait for P1.A1 (#59)",
            ));
        }
        let connection = nonempty(config, SECTION, "ConnectionString")
            .or_else(|| nonempty(config, "DatabaseService", "ConnectionString"))
            .ok_or(AliasError::Config("No ConnectionString configured"))?;
        let realm = nonempty(config, SECTION, "Realm").unwrap_or("UserAlias");
        let pool = Pool::connect(Backend::Sqlite, connection).await?;
        let store = AliasStore::new(pool, realm).await?;
        let reject_shard_header = !config_bool_either(config, "AllowllHTTPRequestIn", false)?;
        Ok(Self::new(AliasService::new(store), reject_shard_header))
    }
}

fn auth_type(config: &Config) -> &str {
    let mut found = "None";
    for section in ["Network", SECTION] {
        if let Some(value) = nonempty(config, section, "AuthType") {
            found = value;
        }
    }
    found
}

fn config_bool_either(config: &Config, key: &str, default: bool) -> Result<bool, AliasError> {
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
) -> Result<bool, AliasError> {
    let Some(text) = config.get(section, key) else {
        return Ok(default);
    };
    if text.is_empty() {
        return Ok(default);
    }
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(AliasError::Message(format!(
            "{section} {key} is not a boolean"
        ))),
    }
}

fn nonempty<'a>(config: &'a Config, section: &str, key: &str) -> Option<&'a str> {
    config.get(section, key).filter(|value| !value.is_empty())
}
