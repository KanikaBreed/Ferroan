// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `[UserAccountService]` keys `ferroan-robust` reads.
//!
//! Storage provider and connection string follow `UserAccountServiceBase`:
//! the account section wins, and an empty value falls through to
//! `[DatabaseService]`. Only SQLite is opened, against a file already at
//! UserAccount version 4. `AuthType = BasicHttpAuthentication` and
//! `https_main = true` refuse to start. A set plugin DLL is refused rather
//! than skipped, because those plugins belong to later rows.

use std::sync::Arc;

use ferroan_config::Config;
use ferroan_db::{Backend, Pool};

use super::http::AccountHttp;
use super::service::{AccountService, SystemClock};
use super::store::AccountStore;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("{0}")]
    Config(&'static str),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Db(#[from] ferroan_db::DbError),
    #[error(transparent)]
    Store(#[from] super::store::StoreError),
}

const SECTION: &str = "UserAccountService";
const PLUGINS: &[&str] = &[
    "GridService",
    "AuthenticationService",
    "GridUserService",
    "InventoryService",
    "AvatarService",
];

impl AccountHttp {
    pub async fn open(config: &Config) -> Result<Self, AccountError> {
        let module = nonempty(config, SECTION, "LocalServiceModule")
            .ok_or(AccountError::Config("No LocalServiceModule in config file"))?;
        if module != "UserAccountService" && !module.ends_with(":UserAccountService") {
            return Err(AccountError::Config(
                "only OpenSim.Services.UserAccountService.dll:UserAccountService is served",
            ));
        }
        if https_main(config)? {
            return Err(AccountError::Config(
                "https_main is set and this build serves HTTP only",
            ));
        }
        let auth = auth_type(config);
        if auth != "None" && auth != "TrustedGridAuthentication" {
            return Err(AccountError::Message(format!(
                "AuthType {auth} is not implemented"
            )));
        }
        for key in PLUGINS {
            if nonempty(config, SECTION, key).is_some() {
                return Err(AccountError::Message(format!(
                    "{key} is set; this row does not load that plugin"
                )));
            }
        }
        let provider = nonempty(config, SECTION, "StorageProvider")
            .or_else(|| nonempty(config, "DatabaseService", "StorageProvider"))
            .ok_or(AccountError::Config("No StorageProvider configured"))?;
        if !provider.contains("SQLite") {
            return Err(AccountError::Config(
                "this account service only opens a SQLite store; MySQL and PostgreSQL wait for P1.A1 (#59)",
            ));
        }
        let connection = nonempty(config, SECTION, "ConnectionString")
            .or_else(|| nonempty(config, "DatabaseService", "ConnectionString"))
            .ok_or(AccountError::Config("No ConnectionString configured"))?;
        let realm = nonempty(config, SECTION, "Realm").unwrap_or("UserAccounts");
        let pool = Pool::open(Backend::Sqlite, connection, &["UserAccount"]).await?;
        let store = AccountStore::new(pool, realm)?;
        let service = AccountService::open(store, Arc::new(SystemClock)).await?;
        let allow_create_user = config_bool(config, "AllowCreateUser", false)?;
        let allow_set_account = config_bool(config, "AllowSetAccount", false)?;
        let reject_shard_header = !config_bool_either(config, "AllowllHTTPRequestIn", false)?;
        Ok(Self::new(
            service,
            allow_create_user,
            allow_set_account,
            reject_shard_header,
        ))
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

fn https_main(config: &Config) -> Result<bool, AccountError> {
    config_bool_in(config, "Network", "https_main", false)
}

fn config_bool(config: &Config, key: &str, default: bool) -> Result<bool, AccountError> {
    config_bool_in(config, SECTION, key, default)
}

fn config_bool_either(config: &Config, key: &str, default: bool) -> Result<bool, AccountError> {
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
) -> Result<bool, AccountError> {
    let Some(text) = config.get(section, key) else {
        return Ok(default);
    };
    if text.is_empty() {
        return Ok(default);
    }
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(AccountError::Message(format!(
            "{section} {key} is not a boolean"
        ))),
    }
}

fn nonempty<'a>(config: &'a Config, section: &str, key: &str) -> Option<&'a str> {
    config.get(section, key).filter(|value| !value.is_empty())
}
