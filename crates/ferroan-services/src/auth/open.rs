// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `[AuthenticationService]` and `[AuthorizationService]` keys `ferroan-robust` reads.
//!
//! Storage provider and connection string follow `AuthenticationServiceBase`:
//! the authentication section wins, and an empty value falls through to
//! `[DatabaseService]`. SQLite, MySQL and PostgreSQL are opened against a
//! database already at AuthStore version 2, 4 or 3. The realm defaults to
//! `auth`. `AuthType = BasicHttpAuthentication` and `https_main = true`
//! refuse to start. `AllowllHTTPRequestIn` is read from `[Network]` and then
//! this section (later wins, an empty value keeps the running value). It
//! defaults to false, and then `POST /auth` refuses `X-SecondLife-Shard`.
//!
//! `AuthorizationServiceBase` loads an `IAssetDataPlugin` and refuses a
//! provider that is not one. The handler never reads that plugin (TB42).
//! This process checks `LocalServiceModule` and does not open an asset database.

use ferroan_config::Config;
use ferroan_db::{Backend, Pool};

use super::http::AuthHttp;
use super::model::AuthKind;
use super::service::AuthService;
use super::store::{self, AuthStore};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    Config(&'static str),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Db(#[from] ferroan_db::DbError),
}

const SECTION: &str = "AuthenticationService";

impl AuthHttp {
    pub async fn open(config: &Config) -> Result<Self, AuthError> {
        let module = nonempty(config, SECTION, "LocalServiceModule")
            .ok_or(AuthError::Config("No LocalServiceModule in config file"))?;
        let Some(kind) = AuthKind::from_module(module) else {
            return Err(AuthError::Config(
                "only PasswordAuthenticationService, WebkeyAuthenticationService and WebkeyOrPasswordAuthenticationService are served",
            ));
        };
        refuse_transport(config, SECTION)?;
        let provider = nonempty(config, SECTION, "StorageProvider")
            .or_else(|| nonempty(config, "DatabaseService", "StorageProvider"))
            .ok_or(AuthError::Config("No StorageProvider configured"))?;
        let backend = auth_backend(provider)?;
        let connection = nonempty(config, SECTION, "ConnectionString")
            .or_else(|| nonempty(config, "DatabaseService", "ConnectionString"))
            .ok_or(AuthError::Config("No ConnectionString configured"))?;
        let realm = match config.get(SECTION, "Realm") {
            Some("") => {
                return Err(AuthError::Config("No StorageProvider configured"));
            }
            Some(realm) => realm,
            None => "auth",
        };
        if !store::is_identifier(realm) {
            return Err(AuthError::Message(
                "authentication realm is not an identifier".to_string(),
            ));
        }
        let pool = Pool::open(backend, connection, &["AuthStore"]).await?;
        let store =
            AuthStore::new(pool, realm).map_err(|error| AuthError::Message(error.to_string()))?;
        let service = AuthService::new(store, kind);
        Ok(Self::new(
            service,
            config_bool(config, SECTION, "AllowGetAuthInfo", false)?,
            config_bool(config, SECTION, "AllowSetAuthInfo", false)?,
            config_bool(config, SECTION, "AllowSetPassword", false)?,
            !config_bool_either(config, SECTION, "AllowllHTTPRequestIn", false)?,
        ))
    }
}

pub fn open_authorization(config: &Config) -> Result<(), AuthError> {
    let module = nonempty(config, "AuthorizationService", "LocalServiceModule")
        .ok_or(AuthError::Config("No AuthorizationService in config file"))?;
    if module != "AuthorizationService" && !module.ends_with(":AuthorizationService") {
        return Err(AuthError::Config(
            "only OpenSim.Services.AuthorizationService.dll:AuthorizationService is served",
        ));
    }
    refuse_transport(config, "AuthorizationService")
}

pub(crate) fn refuse_transport(config: &Config, section: &str) -> Result<(), AuthError> {
    if https_main(config)? {
        return Err(AuthError::Config(
            "https_main is set and this build serves HTTP only",
        ));
    }
    let auth = auth_type(config, section);
    if auth != "None" && auth != "TrustedGridAuthentication" {
        return Err(AuthError::Message(format!(
            "AuthType {auth} is not implemented"
        )));
    }
    Ok(())
}

fn auth_backend(provider: &str) -> Result<Backend, AuthError> {
    if provider.contains("SQLite") {
        Ok(Backend::Sqlite)
    } else if provider.contains("MySQL") {
        Ok(Backend::Mysql)
    } else if provider.contains("PGSQL") || provider.contains("Postgres") {
        Ok(Backend::Postgres)
    } else {
        Err(AuthError::Config(
            "this authentication service opens SQLite, MySQL and PostgreSQL stores",
        ))
    }
}

fn auth_type<'a>(config: &'a Config, section: &str) -> &'a str {
    let mut found = "None";
    for name in ["Network", section] {
        if let Some(value) = nonempty(config, name, "AuthType") {
            found = value;
        }
    }
    found
}

fn https_main(config: &Config) -> Result<bool, AuthError> {
    config_bool(config, "Network", "https_main", false)
}

pub(crate) fn config_bool_either(
    config: &Config,
    service: &str,
    key: &str,
    default: bool,
) -> Result<bool, AuthError> {
    let mut value = default;
    for section in ["Network", service] {
        if config.get(section, key).is_some() {
            value = config_bool(config, section, key, value)?;
        }
    }
    Ok(value)
}

fn config_bool(
    config: &Config,
    section: &str,
    key: &str,
    default: bool,
) -> Result<bool, AuthError> {
    let Some(text) = config.get(section, key) else {
        return Ok(default);
    };
    if text.is_empty() {
        return Ok(default);
    }
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(AuthError::Message(format!(
            "{section} {key} is not a boolean"
        ))),
    }
}

fn nonempty<'a>(config: &'a Config, section: &str, key: &str) -> Option<&'a str> {
    config.get(section, key).filter(|value| !value.is_empty())
}
