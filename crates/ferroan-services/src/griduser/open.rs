// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `[GridUserService]` keys `ferroan-robust` reads.
//!
//! Storage provider and connection string follow `GridUserServiceBase`: the
//! grid-user section wins, and an empty value falls through to
//! `[DatabaseService]`. SQLite, MySQL and PostgreSQL are opened against a
//! database already at GridUserStore version 1, 2 or 2. An empty realm uses
//! `GridUser`. `AuthType = BasicHttpAuthentication` and `https_main = true`
//! refuse to start. `TrustedGridAuthentication` does not reject callers when
//! no trust context is set, which is this process, so it is accepted and does
//! not add a check.

use ferroan_config::Config;
use ferroan_db::{Backend, Pool};

use super::http::GridUserHttp;
use super::model::DEFAULT_REALM;
use super::service::{GridUserService, SystemClock};
use super::store::GridUserStore;

#[derive(Debug, thiserror::Error)]
pub enum GridUserError {
    #[error("{0}")]
    Config(&'static str),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Db(#[from] ferroan_db::DbError),
    #[error(transparent)]
    Store(#[from] super::store::StoreError),
}

const SECTION: &str = "GridUserService";

impl GridUserHttp {
    pub async fn open(config: &Config) -> Result<Self, GridUserError> {
        let module = nonempty(config, SECTION, "LocalServiceModule").ok_or(
            GridUserError::Config("No LocalServiceModule in config file"),
        )?;
        if module != "GridUserService" && !module.ends_with(":GridUserService") {
            return Err(GridUserError::Config(
                "only OpenSim.Services.UserAccountService.dll:GridUserService is served",
            ));
        }
        refuse_transport(config)?;
        let provider = nonempty(config, SECTION, "StorageProvider")
            .or_else(|| nonempty(config, "DatabaseService", "StorageProvider"))
            .ok_or(GridUserError::Config("No StorageProvider configured"))?;
        let backend = griduser_backend(provider)?;
        let connection = nonempty(config, SECTION, "ConnectionString")
            .or_else(|| nonempty(config, "DatabaseService", "ConnectionString"))
            .ok_or(GridUserError::Config("No ConnectionString configured"))?;
        let realm = match nonempty(config, SECTION, "Realm") {
            Some(realm) => realm,
            None => DEFAULT_REALM,
        };
        let pool = Pool::open(backend, connection, &["GridUserStore"]).await?;
        let store = GridUserStore::new(pool, realm)?;
        let service = GridUserService::new(store, SystemClock);
        Ok(Self::new(service))
    }
}

fn refuse_transport(config: &Config) -> Result<(), GridUserError> {
    if config_bool(config, "Network", "https_main", false)? {
        return Err(GridUserError::Config(
            "https_main is set and this build serves HTTP only",
        ));
    }
    let auth = auth_type(config);
    if auth != "None" && auth != "TrustedGridAuthentication" {
        return Err(GridUserError::Message(format!(
            "AuthType {auth} is not implemented"
        )));
    }
    Ok(())
}

fn griduser_backend(provider: &str) -> Result<Backend, GridUserError> {
    if provider.contains("SQLite") {
        Ok(Backend::Sqlite)
    } else if provider.contains("MySQL") {
        Ok(Backend::Mysql)
    } else if provider.contains("PGSQL") || provider.contains("Postgres") {
        Ok(Backend::Postgres)
    } else {
        Err(GridUserError::Config(
            "this grid user service opens SQLite, MySQL and PostgreSQL stores",
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

fn config_bool(
    config: &Config,
    section: &str,
    key: &str,
    default: bool,
) -> Result<bool, GridUserError> {
    let Some(text) = config.get(section, key) else {
        return Ok(default);
    };
    if text.is_empty() {
        return Ok(default);
    }
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(GridUserError::Message(format!(
            "{section} {key} is not a boolean"
        ))),
    }
}

fn nonempty<'a>(config: &'a Config, section: &str, key: &str) -> Option<&'a str> {
    config.get(section, key).filter(|value| !value.is_empty())
}
