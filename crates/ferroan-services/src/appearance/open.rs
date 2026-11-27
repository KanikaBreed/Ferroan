// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! When the viewer bake read is mounted.
//!
//! A section-only ini enables `[AppearanceService]` directly. An ini that has
//! `[Startup] ServiceConnectors` or `[ServiceList]` enables the route only when
//! that list names `OpenSim.Server.Handlers.dll:AppearanceServerConnector`.
//! The example grid file contains the section and leaves the connector
//! commented out, so the section alone does not publish the route.
//!
//! Ferroan has one service listener. A connector line that names another port
//! still serves `GET /texture` on that listener. The pin would open a second
//! listener for a different port.

use ferroan_config::Config;

use crate::asset::AssetHttp;
use crate::avatar::AvatarHttp;

use super::http::AppearanceHttp;

const SECTION: &str = "AppearanceService";
const CONNECTOR: &str = "OpenSim.Server.Handlers.dll:AppearanceServerConnector";

#[derive(Debug, thiserror::Error)]
pub enum AppearanceError {
    #[error("{0}")]
    Config(&'static str),
    #[error("{0}")]
    Message(String),
}

impl AppearanceHttp {
    /// The config section to read, when this process should serve the route.
    pub fn selected(config: &Config, listen_port: u16) -> Result<Option<String>, AppearanceError> {
        let startup = config.get("Startup", "ServiceConnectors");
        let has_list = config.section_names().any(|name| name == "ServiceList");
        if startup.is_none() && !has_list {
            return Ok(config
                .section_names()
                .any(|name| name == SECTION)
                .then(|| SECTION.to_string()));
        }
        let entries = startup
            .into_iter()
            .chain(config.keys("ServiceList").map(|(_, value)| value));
        let mut selected = None;
        for entry in entries.flat_map(|list| list.split([',', ' ', '\n', '\r', '\t'])) {
            if entry.is_empty() {
                continue;
            }
            let (prefix, module) = entry.split_once('/').unwrap_or(("", entry));
            if module != CONNECTOR {
                continue;
            }
            let (section, port) = prefix.split_once('@').unwrap_or((SECTION, prefix));
            let section = if section.is_empty() { SECTION } else { section };
            if !port.is_empty()
                && !matches!(port.parse::<u16>(), Ok(parsed) if parsed == 0 || parsed == listen_port)
            {
                tracing::warn!(
                    "AppearanceServerConnector names port {port}; Ferroan serves GET /texture on {listen_port}"
                );
            }
            if selected.is_none() {
                selected = Some(section.to_string());
            }
        }
        if let Some(section) = &selected
            && !config.section_names().any(|name| name == section)
        {
            return Err(AppearanceError::Config(
                "No section for AppearanceServerConnector in config file",
            ));
        }
        Ok(selected)
    }

    /// Mount the route on services this process already opened.
    ///
    /// The appearance service owns no store. An `AssetService` value in its
    /// section that differs from `[AssetService] LocalServiceModule` is logged
    /// and the in-process asset service is still the one that is read.
    pub fn open(
        config: &Config,
        section: &str,
        avatars: &AvatarHttp,
        assets: AssetHttp,
    ) -> Result<Self, AppearanceError> {
        let module = config
            .get(section, "LocalServiceModule")
            .filter(|value| !value.is_empty())
            .ok_or(AppearanceError::Config(
                "No LocalServiceModule in the appearance section",
            ))?;
        if !matches!(
            module,
            "AppearanceService" | "OpenSim.Services.AvatarService.dll:AppearanceService"
        ) {
            return Err(AppearanceError::Config(
                "only OpenSim.Services.AvatarService.dll:AppearanceService is served",
            ));
        }
        let auth = auth_type(config, section);
        if auth != "None" && auth != "TrustedGridAuthentication" {
            return Err(AppearanceError::Message(format!(
                "AuthType {auth} is not implemented"
            )));
        }
        let named = config.get(section, "AssetService").unwrap_or("");
        let grid = config
            .get("AssetService", "LocalServiceModule")
            .unwrap_or("");
        if !named.is_empty() && !grid.is_empty() && named != grid {
            tracing::warn!(
                "[APPEARANCE SERVICE]: [{section}] AssetService is {named}, but the grid stores assets through {grid}. Bakes stored there cannot be served from a different service; this process reads the asset service it already opened"
            );
        }
        let reject_shard =
            !crate::auth::config_bool_either(config, section, "AllowllHTTPRequestIn", false)
                .map_err(|error| AppearanceError::Message(error.to_string()))?;
        Ok(Self::from_services(avatars.service(), assets, reject_shard))
    }
}

fn auth_type<'a>(config: &'a Config, section: &str) -> &'a str {
    let mut found = "None";
    for name in ["Network", section] {
        if let Some(value) = config.get(name, "AuthType")
            && !value.is_empty()
        {
            found = value;
        }
    }
    found
}
