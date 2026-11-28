// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
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
//   Source/OpenSim.Services.AvatarService/AppearanceService.cs
//   Source/OpenSim.Server.Handlers/Appearance/AppearanceServerConnector.cs
//   Source/OpenSim.Framework.Servers/MainServer.cs (GetHttpServer)
//   Source/OpenSim.Server.Base/ServerUtils.cs (GetServer, LoadPlugin)
//   Source/OpenSim.Services.AssetService/AssetServiceBase.cs
//   Source/OpenSim.Services.AvatarService/AvatarService.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4.
// This port remains available under MPL-2.0 and BSD-3-Clause and forms part
// of Ferroan's GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! When the viewer bake read is mounted, and which stores it reads.
//!
//! A section-only ini enables `[AppearanceService]` directly. An ini that has
//! `[Startup] ServiceConnectors` or `[ServiceList]` enables the route only when
//! that list names `OpenSim.Server.Handlers.dll:AppearanceServerConnector`.
//! The example grid file contains the section and leaves the connector
//! commented out, so the section alone does not publish the route.
//!
//! A connector port of 0, or the same port as `[Network] port`, uses the
//! network listener. Any other port is a second listener, and `GET /texture`
//! is not mounted on the network listener. That is `MainServer.GetHttpServer`.
//!
//! `AvatarService` and `AssetService` in the appearance section name the
//! modules. An empty value falls back to that service's `LocalServiceModule`.
//! A stock asset module may name another ini section as
//! `Section@OpenSim.Services.AssetService.dll:AssetService`, which is the
//! extra constructor argument `ServerUtils.LoadPlugin` passes. Any other
//! module refuses to start.

use ferroan_config::Config;

use crate::asset::AssetHttp;
use crate::avatar::AvatarHttp;

use super::http::AppearanceHttp;

const SECTION: &str = "AppearanceService";
const CONNECTOR: &str = "OpenSim.Server.Handlers.dll:AppearanceServerConnector";
const STOCK_AVATAR: &str = "OpenSim.Services.AvatarService.dll:AvatarService";
const STOCK_ASSET: &str = "OpenSim.Services.AssetService.dll:AssetService";

/// The section to read and, when set, the dedicated public port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearanceRoute {
    pub section: String,
    pub public_port: Option<u16>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppearanceError {
    #[error("{0}")]
    Config(&'static str),
    #[error("{0}")]
    Message(String),
}

impl AppearanceHttp {
    /// The config section and public port, when this process should serve the route.
    pub fn selected(
        config: &Config,
        listen_port: u16,
    ) -> Result<Option<AppearanceRoute>, AppearanceError> {
        let startup = config.get("Startup", "ServiceConnectors");
        let has_list = config.section_names().any(|name| name == "ServiceList");
        if startup.is_none() && !has_list {
            return Ok(config.section_names().any(|name| name == SECTION).then(|| {
                AppearanceRoute {
                    section: SECTION.to_string(),
                    public_port: None,
                }
            }));
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
            if selected.is_none() {
                selected = Some(AppearanceRoute {
                    section: section.to_string(),
                    public_port: dedicated_port(port, listen_port)?,
                });
            }
        }
        if let Some(route) = &selected
            && !config.section_names().any(|name| name == route.section)
        {
            return Err(AppearanceError::Config(
                "No section for AppearanceServerConnector in config file",
            ));
        }
        Ok(selected)
    }

    /// Open the bake read on the modules named by the appearance section.
    ///
    /// An empty `AvatarService` or `AssetService` uses the grid section's
    /// `LocalServiceModule`. A different asset module is logged. The same
    /// module string reuses the service this process already opened, unless it
    /// is a stock asset class that names another section. That section is
    /// opened even when appearance inherits the grid module unchanged. A stock
    /// asset module may name another section as
    /// `Section@OpenSim.Services.AssetService.dll:AssetService`. Any other
    /// module, and an `@` avatar module, is refused.
    pub async fn open(
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
        let (avatar_module, asset_module) = service_modules(config, section);
        if avatar_module.is_empty() {
            return Err(AppearanceError::Message(format!(
                "[{section}] AvatarService not set, nor [AvatarService] LocalServiceModule; the appearance service has no bake index to read"
            )));
        }
        if asset_module.is_empty() {
            return Err(AppearanceError::Message(format!(
                "[{section}] AssetService not set, nor [AssetService] LocalServiceModule; the appearance service has no assets to serve"
            )));
        }
        let grid_avatar = module_value(config, "AvatarService", "LocalServiceModule");
        let avatars = avatar_for_appearance(&avatar_module, &grid_avatar, avatars.service())?;
        let assets = asset_for_appearance(config, section, &asset_module, assets).await?;
        let reject_shard =
            !crate::auth::config_bool_either(config, section, "AllowllHTTPRequestIn", false)
                .map_err(|error| AppearanceError::Message(error.to_string()))?;
        Ok(Self::from_services(avatars, assets, reject_shard))
    }
}

fn dedicated_port(port: &str, listen_port: u16) -> Result<Option<u16>, AppearanceError> {
    if port.is_empty() {
        return Ok(None);
    }
    let parsed = port.parse::<u16>().map_err(|_| {
        AppearanceError::Config("AppearanceServerConnector port is not a port number")
    })?;
    if parsed == 0 || parsed == listen_port {
        Ok(None)
    } else {
        Ok(Some(parsed))
    }
}

fn module_value(config: &Config, section: &str, key: &str) -> String {
    config
        .get(section, key)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("")
        .to_string()
}

fn service_modules(config: &Config, section: &str) -> (String, String) {
    let mut avatar = module_value(config, section, "AvatarService");
    let mut asset = module_value(config, section, "AssetService");
    if avatar.is_empty() {
        avatar = module_value(config, "AvatarService", "LocalServiceModule");
    }
    if asset.is_empty() {
        asset = module_value(config, "AssetService", "LocalServiceModule");
    }
    (avatar, asset)
}

fn avatar_for_appearance(
    module: &str,
    grid: &str,
    opened: std::sync::Arc<crate::avatar::AvatarService>,
) -> Result<std::sync::Arc<crate::avatar::AvatarService>, AppearanceError> {
    // `AvatarService` has only an `IConfigSource` constructor. An `@` prefix
    // is an extra argument and does not construct.
    if !module.contains('@') && ((!grid.is_empty() && module == grid) || module == STOCK_AVATAR) {
        return Ok(opened);
    }
    Err(AppearanceError::Message(format!(
        "Could not load AvatarService '{module}' for the appearance service"
    )))
}

async fn asset_for_appearance(
    config: &Config,
    section: &str,
    module: &str,
    opened: AssetHttp,
) -> Result<AssetHttp, AppearanceError> {
    let grid = module_value(config, "AssetService", "LocalServiceModule");
    if !grid.is_empty() && module != grid {
        // `{Section}` is the opened appearance section, twice, as the pin logs it.
        let message = format!(
            "[APPEARANCE SERVICE]: [{section}] AssetService is {module}, but the grid stores assets through {grid}. Bakes stored there cannot be served from here; remove AssetService from [{section}] to use the grid's."
        );
        tracing::warn!("{message}");
    }
    if !grid.is_empty() && module == grid {
        // The grid opener always reads `[AssetService]`. A stock class that
        // names another section still has to open that section, including
        // when appearance inherits the grid module unchanged.
        if let Some((store_section, class)) = named_plugin(module)
            && !store_section.is_empty()
            && store_section != "AssetService"
            && stock_asset(class)
        {
            return AssetHttp::open_stock_from(config, store_section)
                .await
                .map_err(|error| AppearanceError::Message(error.to_string()));
        }
        return Ok(opened);
    }
    let (store_section, class) = named_plugin(module).ok_or_else(|| {
        AppearanceError::Message(format!(
            "Could not load AssetService '{module}' for the appearance service"
        ))
    })?;
    if !stock_asset(class) {
        return Err(AppearanceError::Message(format!(
            "Could not load AssetService '{module}' for the appearance service"
        )));
    }
    let store_section = if store_section.is_empty() {
        "AssetService"
    } else {
        store_section
    };
    if store_section == "AssetService" && stock_asset(&grid) {
        return Ok(opened);
    }
    AssetHttp::open_stock_from(config, store_section)
        .await
        .map_err(|error| AppearanceError::Message(error.to_string()))
}

/// `ServerUtils.LoadPlugin`: one `@` prefix is the config-section constructor
/// argument, and the remainder is `assembly:class`. More than one `@` does
/// not match `AssetService(IConfigSource, string)`.
fn named_plugin(module: &str) -> Option<(&str, &str)> {
    match module.split_once('@') {
        None => Some(("", module)),
        Some((store_section, class))
            if !store_section.is_empty() && !class.is_empty() && !class.contains('@') =>
        {
            Some((store_section, class))
        }
        Some(_) => None,
    }
}

fn stock_asset(module: &str) -> bool {
    module == "AssetService" || module == STOCK_ASSET
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
