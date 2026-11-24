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
//   Source/OpenSim.Framework/RegionInfo.cs (ReadNiniConfig)
//   Source/OpenSim.Framework/NetworkServersInfo.cs
//   Source/OpenSim.ApplicationPlugins.LoadRegions/RegionLoaderFileSystem.cs
//   Source/OpenSim.Server.RegionServer/OpenSimBase.cs (CreateRegion)
//   Source/OpenSim.Region.Framework/Scenes/Scene.cs (map configuration)
//   Source/OpenSim.Region.Framework/Scenes/ScenePresence.cs (CompleteMovement)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs (constructor)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPClient.cs (constructor)
//   Source/OpenSim.Framework/Util.cs (GetConfigVarFromSections)
//   Source/OpenSim.Framework/ServiceAuth/ServiceAuth.cs (Create)
//   Source/OpenSim.Framework/GridInfo.cs (OSHHTPHost host/port URI validation)
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The single region used by P2.0d. INI syntax, includes and substitutions
//! belong to ferroan-config. Interactive creation, XML regions, variable
//! region sizes and TLS listeners remain unsupported and are refused.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::control_plane::{ControlPlaneAccess, ControlPlaneError};
use ferroan_client::{CapGrantSet, CircuitTiming, HandshakeConfig};
use ferroan_config::{Config, RegionLoad};
use ferroan_diag::{self, Kind, TIMER_MILLISECONDS, TIMER_SECONDS};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum RegionConfigError {
    #[error(transparent)]
    Ini(#[from] ferroan_config::ConfigError),
    #[error("[{section}] {key}: {reason}")]
    Value {
        section: String,
        key: &'static str,
        reason: &'static str,
    },
    #[error("regionload_regionsdir must define exactly one region across all INI files")]
    RegionCount,
    #[error("XML region files are not supported; use INI region configuration")]
    XmlRegions,
    #[error(transparent)]
    ControlPlane(#[from] ControlPlaneError),
}

/// Resolved public settings. Deliberately no Debug: region_secret is private.
#[derive(Clone)]
pub struct RegionConfig {
    pub id: Uuid,
    pub name: String,
    pub scope_id: Uuid,
    /// Map tiles, as written in Regions.ini. Registration uses metres.
    pub location: [u32; 2],
    pub size: [u32; 2],
    pub udp_bind: SocketAddr,
    pub http_bind: SocketAddr,
    pub external_host: String,
    pub server_uri: String,
    pub grid_server_uri: String,
    pub presence_server_uri: String,
    pub grid_user_server_uri: String,
    pub control_plane: ControlPlaneAccess,
    pub estate_owner: Uuid,
    /// `allow_grid_gods` after `force_grid_gods_only`. Code default is false.
    /// `RegionServerDefaults.ini` sets it true, and a region process loads that
    /// file as inimaster.
    pub allow_grid_gods: bool,
    /// `region_owner_is_god`. Forced off when `force_grid_gods_only` is set.
    pub region_owner_is_god: bool,
    /// `region_manager_is_god`. Forced off when `force_grid_gods_only` is set.
    pub region_manager_is_god: bool,
    /// `[UserAccountService] UserAccountServerURI`. Empty skips the level lookup.
    pub user_account_server_uri: String,
    /// SQLite estate connection, resolved like `EstateDataService`: DatabaseService
    /// `ConnectionString`, then `EstateConnectionString`, then
    /// `[EstateDataStore] ConnectionString`. Absent means no estate tables.
    pub estate_store: Option<String>,
    pub handshake: HandshakeConfig,
    pub circuit_timing: CircuitTiming,
    /// Deadline for ViaLogin circuits which have not opened a viewer connection.
    /// Zero retains the C# legacy behavior of waiting until region shutdown.
    pub login_connect_wait: std::time::Duration,
    /// Server identity sent in AgentMovementComplete, like Scene.GetSimulatorVersion.
    pub channel_version: String,
    pub region_secret: Uuid,
    /// Robust xinventory base. Empty when this region has no inventory service.
    pub inventory_server_uri: String,
    /// Robust asset service base. Empty when this region has no asset service.
    pub asset_server_uri: String,
    /// `[AvatarService] AvatarServerURI`. Empty skips `getavatar` and uses the
    /// constructor appearance. The region does not serve the avatar service.
    pub avatar_server_uri: String,
    /// True when `[LibraryService]` is present. The tree is loaded in-process.
    pub library_enabled: bool,
    pub library_name: String,
    /// Resolved by [`Self::load`] against the region directory. [`Self::from_configs`] leaves this empty.
    pub library_file: Option<PathBuf>,
    pub cap_grants: CapGrantSet,
    pub monitoring: MonitoringSettings,
    /// Stdout, `LOGDIR`, JSON, OTLP and the remote console. Not `Debug`.
    pub logs: ferroan_log::Settings,
    /// `[DatabaseService] ConnectionString` when it is present and not empty.
    /// Absent, or a SQLite file that is not on disk, means no saved heightmap.
    pub terrain_store: Option<String>,
    /// SQLite region store for prims. `[SimulationDataStore]` replaces
    /// `[DatabaseService]` when its key is present. A null provider and a
    /// non-SQLite provider leave this empty. MySQL and PostgreSQL are a later row.
    pub object_store: Option<String>,
    /// `[ClientStack.LindenUDP] CaptureFile`. Empty leaves datagram capture off.
    pub udp_capture: Option<PathBuf>,
}

/// Status and metrics settings. Not `Debug`: the managed-stats password is private.
#[derive(Clone)]
pub struct MonitoringSettings {
    /// `[Monitoring] Enabled`. Absent means true, as in `MonitorModule`.
    pub monitor_enabled: bool,
    /// Loopback metrics listener. `None` unless `[Monitoring] Metrics` is true.
    pub metrics_listen: Option<SocketAddr>,
    /// `Startup.ManagedStatsRemoteFetchURI`. Empty leaves the route unregistered.
    pub managed_stats_uri: String,
    /// `Startup.ManagedStatsRemoteFetchPassword`. Empty means the route is open.
    pub managed_stats_password: String,
    /// `[WebStats] enabled`. Absent means false.
    pub web_stats: bool,
}

impl RegionConfig {
    pub fn load(load: &RegionLoad) -> Result<Self, RegionConfigError> {
        let resolved = load.load()?;
        let directory = resolved
            .config
            .get("Startup", "regionload_regionsdir")
            .unwrap_or("Regions")
            .trim();
        let regions_path = load.config_dir.join(Path::new(directory));
        let regions = load_regions(&regions_path)?;
        let mut region = Self::from_configs(&resolved.config, &regions)?;
        region.library_file = library_file(&load.config_dir, &resolved.config)?;
        Ok(region)
    }

    pub async fn resolve_control_plane(&mut self) -> Result<(), RegionConfigError> {
        self.control_plane.resolve().await?;
        Ok(())
    }

    pub fn from_configs(server: &Config, regions: &Config) -> Result<Self, RegionConfigError> {
        let mut names = regions.section_names();
        let name = names.next().ok_or(RegionConfigError::RegionCount)?;
        if names.next().is_some() {
            return Err(RegionConfigError::RegionCount);
        }
        if name.is_empty() || name.len() > 256 {
            return Err(value(name, "region name", "must contain 1 to 256 bytes"));
        }
        let id = uuid(regions, name, "RegionUUID", None)?;
        if id.is_nil() {
            return Err(value(name, "RegionUUID", "must not be zero"));
        }
        let location = required(regions, name, "Location")?;
        let (x, y) = location.split_once(',').ok_or_else(|| {
            value(
                name,
                "Location",
                "expected two map coordinates separated by a comma",
            )
        })?;
        let coordinate = |text: &str| -> Result<u32, RegionConfigError> {
            let tile: u32 = text
                .trim()
                .parse()
                .map_err(|_| value(name, "Location", "expected unsigned map coordinates"))?;
            if tile > i32::MAX as u32 / 256 {
                return Err(value(
                    name,
                    "Location",
                    "coordinate exceeds the grid's signed metre range",
                ));
            }
            Ok(tile)
        };
        let location = [coordinate(x)?, coordinate(y)?];
        // Util.GetConfigVarFromSections visits Map, then Startup; the latter
        // overrides values already found, including an explicitly empty value.
        let generate_maps = server
            .get("Startup", "GenerateMaptiles")
            .or_else(|| server.get("Map", "GenerateMaptiles"))
            .unwrap_or("true");
        if !generate_maps.eq_ignore_ascii_case("false") {
            return Err(value(
                "Map",
                "GenerateMaptiles",
                "P2.0d requires false; map generation is not implemented",
            ));
        }
        let map_section = if server.get("Startup", "MaptileStaticUUID").is_some() {
            "Startup"
        } else {
            "Map"
        };
        let global_tile = uuid(server, map_section, "MaptileStaticUUID", Some(Uuid::nil()))?;
        let region_tile = uuid(regions, name, "MaptileStaticUUID", Some(Uuid::nil()))?;
        if !global_tile.is_nil() || !region_tile.is_nil() {
            return Err(value(
                name,
                "MaptileStaticUUID",
                "P2.0d does not serve static map assets",
            ));
        }
        let mut size = [256, 256];
        for (index, key) in ["SizeX", "SizeY"].into_iter().enumerate() {
            size[index] = number(regions, name, key, "256")?;
            if size[index] != 256 {
                return Err(value(name, key, "P2.0d supports 256 metre regions"));
            }
        }
        let udp_ip = ip(regions, name, "InternalAddress", "0.0.0.0")?;
        let udp_port = port(regions, name, "InternalPort", "9000")?;
        let http_ip = ip(server, "Network", "bind_ip_address", "0.0.0.0")?;
        let http_port = port(server, "Network", "http_listener_port", "9000")?;
        if boolean(server, "Network", "http_listener_ssl")?
            || boolean(server, "Network", "https_listener")?
        {
            return Err(value(
                "Network",
                "http_listener_ssl",
                "TLS listeners are not implemented",
            ));
        }
        if boolean(regions, name, "ResolveAddress")? {
            return Err(value(
                name,
                "ResolveAddress",
                "startup DNS rewriting is not implemented",
            ));
        }
        let external_host = required(regions, name, "ExternalHostName")?;
        if external_host == "SYSTEMIP"
            || external_host.len() > 253
            || external_host
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '/' | '\\' | '?' | '#' | '@'))
        {
            return Err(value(
                name,
                "ExternalHostName",
                "give an explicit IP address or host name",
            ));
        }
        let server_uri = format!("http://{external_host}:{http_port}/");
        check_uri(&server_uri).map_err(|reason| value(name, "ExternalHostName", reason))?;
        let grid_server_uri = required(server, "GridService", "GridServerURI")?;
        check_uri(grid_server_uri)
            .map_err(|reason| value("GridService", "GridServerURI", reason))?;
        let presence_server_uri = required(server, "PresenceService", "PresenceServerURI")?;
        check_uri(presence_server_uri)
            .map_err(|reason| value("PresenceService", "PresenceServerURI", reason))?;
        let grid_user_server_uri = required(server, "GridUserService", "GridUserServerURI")?;
        check_uri(grid_user_server_uri)
            .map_err(|reason| value("GridUserService", "GridUserServerURI", reason))?;
        // GridUserServicesConnector uses OSHHTPHost.URI, a host/port base;
        // unlike the other connectors it does not accept a service path prefix.
        if reqwest::Url::parse(grid_user_server_uri)
            .map_err(|_| {
                value(
                    "GridUserService",
                    "GridUserServerURI",
                    "expected an HTTP service URI",
                )
            })?
            .path()
            != "/"
        {
            return Err(value(
                "GridUserService",
                "GridUserServerURI",
                "expected a host/port base without a path prefix",
            ));
        }
        reject_unsupported_auth(server)?;
        let inventory_server_uri = optional_uri(server, "InventoryService", "InventoryServerURI")?;
        let asset_server_uri = optional_uri(server, "AssetService", "AssetServerURI")?;
        let avatar_server_uri = optional_uri(server, "AvatarService", "AvatarServerURI")?;
        let (library_enabled, library_name) = library_settings(server)?;
        let estate_owner = uuid(
            server,
            "Estates",
            "DefaultEstateOwnerUUID",
            Some(Uuid::nil()),
        )?;
        let product_name = regions.get(name, "RegionType").unwrap_or("");
        if product_name.len() > 4096 {
            return Err(value(name, "RegionType", "exceeds 4096 bytes"));
        }
        // RegionSettings is not an INI section. The handshake owner stays the
        // INI estate owner. Admission may replace it from the estate tables.
        let (allow_grid_gods, region_owner_is_god, region_manager_is_god) =
            crate::staff::fold_god_flags(
                permission_flag(server, "allow_grid_gods", false)?,
                permission_flag(server, "region_owner_is_god", false)?,
                permission_flag(server, "region_manager_is_god", false)?,
                permission_flag(server, "force_grid_gods_only", false)?,
            );
        let user_account_server_uri =
            optional_uri(server, "UserAccountService", "UserAccountServerURI")?;
        let mut handshake = HandshakeConfig::for_region(id);
        handshake.sim_name = name.to_owned();
        handshake.product_name = product_name.to_owned();
        handshake.estate_owner = estate_owner;
        Ok(Self {
            id,
            name: name.to_owned(),
            scope_id: uuid(regions, name, "ScopeID", Some(Uuid::nil()))?,
            location,
            size,
            udp_bind: SocketAddr::new(udp_ip, udp_port),
            http_bind: SocketAddr::new(http_ip, http_port),
            external_host: external_host.to_owned(),
            server_uri,
            grid_server_uri: grid_server_uri.to_owned(),
            presence_server_uri: presence_server_uri.to_owned(),
            grid_user_server_uri: grid_user_server_uri.to_owned(),
            control_plane: ControlPlaneAccess::from_config(server)?,
            estate_owner,
            allow_grid_gods,
            region_owner_is_god,
            region_manager_is_god,
            user_account_server_uri,
            estate_store: estate_connection(server),
            handshake,
            circuit_timing: circuit_timing(server)?,
            login_connect_wait: login_connect_wait(server)?,
            channel_version: concat!("Ferroan ", env!("CARGO_PKG_VERSION")).to_owned(),
            region_secret: Uuid::new_v4(),
            inventory_server_uri,
            asset_server_uri,
            avatar_server_uri,
            library_enabled,
            library_name,
            library_file: None,
            cap_grants: crate::caps_policy::cap_grants(server),
            monitoring: monitoring_settings(server)?,
            logs: ferroan_log::Settings::from_config(
                "ferroan-region",
                server,
                Some(name),
                Some(id),
            )
            .map_err(log_config_error)?,
            terrain_store: server
                .get("DatabaseService", "ConnectionString")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            object_store: object_store(server),
            udp_capture: server
                .get("ClientStack.LindenUDP", "CaptureFile")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
        })
    }

    pub fn handle(&self) -> u64 {
        (u64::from(self.location[0]) * 256) << 32 | (u64::from(self.location[1]) * 256)
    }

    /// CompleteMovement's initial zero-camera/zero-velocity fallback.
    /// `position` is the login start, or the terrain-lifted start when the
    /// caller has a heightmap. Land packets are attached by the dispatcher.
    pub fn arrival(&self, position: [f32; 3]) -> ferroan_client::ArrivalConfig {
        ferroan_client::ArrivalConfig {
            position,
            look_at: [0.99, 0.042, 0.0],
            region_handle: self.handle(),
            channel_version: self.channel_version.clone(),
            land: Vec::new(),
            avatar: None,
            teleport: None,
            source_hop: None,
            return_slot: None,
        }
    }
}

fn login_connect_wait(server: &Config) -> Result<std::time::Duration, RegionConfigError> {
    let seconds = server
        .get("Startup", "LoginConnectWaitSeconds")
        .unwrap_or("120")
        .trim()
        .parse::<i32>()
        .map_err(|_| {
            value(
                "Startup",
                "LoginConnectWaitSeconds",
                "expected signed 32-bit seconds",
            )
        })?;
    Ok(std::time::Duration::from_secs(seconds.max(0) as u64))
}

fn load_regions(directory: &Path) -> Result<Config, RegionConfigError> {
    const MAX_DIRECTORY_ENTRIES: usize = 4096;
    const MAX_INI_FILES: usize = 256;
    const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
    let failure = |error: std::io::Error| ferroan_config::ConfigError::Io {
        path: directory.display().to_string(),
        message: error.to_string(),
    };
    let entries = std::fs::read_dir(directory).map_err(failure)?;
    let mut paths = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            return Err(ferroan_config::ConfigError::Limit {
                what: "region directory entries",
            }
            .into());
        }
        let entry = entry.map_err(failure)?;
        if entry.file_type().map_err(failure)?.is_dir() {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let matches = |expected: &str| {
            if cfg!(windows) {
                extension.eq_ignore_ascii_case(expected)
            } else {
                extension == expected
            }
        };
        if matches("xml") {
            return Err(RegionConfigError::XmlRegions);
        }
        if matches("ini") {
            if paths.len() >= MAX_INI_FILES {
                return Err(ferroan_config::ConfigError::Limit {
                    what: "region INI file count",
                }
                .into());
            }
            paths.push(path);
        }
    }
    paths.sort();
    let mut selected = None;
    let mut total_bytes = 0u64;
    for path in paths {
        total_bytes = total_bytes.saturating_add(std::fs::metadata(&path).map_err(failure)?.len());
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(ferroan_config::ConfigError::Limit {
                what: "region INI total bytes",
            }
            .into());
        }
        let config = Config::parse_file(&path)?;
        // Each section in each file creates a region in C#. Do not merge files:
        // identical section names in two files still represent two regions.
        let count = config.section_names().count();
        if count > 1 || (count == 1 && selected.is_some()) {
            return Err(RegionConfigError::RegionCount);
        }
        if count == 1 {
            selected = Some(config);
        }
    }
    selected.ok_or(RegionConfigError::RegionCount)
}

fn reject_unsupported_auth(config: &Config) -> Result<(), RegionConfigError> {
    // ServiceAuth.Create uses Network first and the service section last.
    // At the pin, None, none, an empty value and TrustedGridAuthentication add
    // no Authorization header (TrustedGrid's signer is null). This region
    // accepts those four and does not implement signing. BasicHttpAuthentication
    // adds credentials, and any other value is refused: C#'s default branch
    // accepts unknown names, which this row does not ask for.
    for section in [
        "Network",
        "GridService",
        "PresenceService",
        "GridUserService",
        "InventoryService",
        "AssetService",
    ] {
        let setting = config
            .get(section, "AuthType")
            .or_else(|| config.get("Network", "AuthType"))
            .unwrap_or("None");
        if !matches!(setting, "None" | "none" | "" | "TrustedGridAuthentication") {
            return Err(value(
                section,
                "AuthType",
                "only None, none, an empty value, or TrustedGridAuthentication is supported by this region",
            ));
        }
    }
    Ok(())
}

fn log_config_error(error: ferroan_log::LogError) -> RegionConfigError {
    match error {
        ferroan_log::LogError::Format => {
            value("Monitoring", "LogFormat", "must be json, plain or text")
        }
        ferroan_log::LogError::Endpoint => value(
            "Monitoring",
            "OtlpEndpoint",
            "must be an http URL without credentials",
        ),
        ferroan_log::LogError::Origin => value(
            "Network",
            "ConsoleAllowedOrigin",
            "is not a valid header value",
        ),
        ferroan_log::LogError::Directory(_)
        | ferroan_log::LogError::Export(_)
        | ferroan_log::LogError::AlreadyInstalled => {
            value("Monitoring", "OtlpEndpoint", "could not be applied")
        }
    }
}

fn value(section: &str, key: &'static str, reason: &'static str) -> RegionConfigError {
    RegionConfigError::Value {
        section: section.to_owned(),
        key,
        reason,
    }
}

fn required<'a>(
    config: &'a Config,
    section: &str,
    key: &'static str,
) -> Result<&'a str, RegionConfigError> {
    config
        .get(section, key)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| value(section, key, "is required"))
}

fn circuit_timing(config: &Config) -> Result<CircuitTiming, RegionConfigError> {
    const SECTION: &str = "ClientStack.LindenUDP";
    let rto = |key, fallback| -> Result<Duration, RegionConfigError> {
        let milliseconds = number(config, SECTION, key, "0")?;
        if milliseconds > i32::MAX as u32 {
            ferroan_diag::refuse(
                TIMER_MILLISECONDS,
                Kind::Hard,
                i32::MAX as u64,
                u64::from(milliseconds),
                key,
            );
            return Err(value(
                SECTION,
                key,
                "must fit a positive signed millisecond timer",
            ));
        }
        // LLUDPClient treats zero as "use the default", not an immediate retry.
        Ok(Duration::from_millis(u64::from(if milliseconds == 0 {
            fallback
        } else {
            milliseconds
        })))
    };
    let silence = |key, fallback| -> Result<Duration, RegionConfigError> {
        let seconds = number(config, SECTION, key, fallback)?;
        // C# multiplies GetInt by 1000. Refuse nonpositive/overflowing timers
        // rather than accepting a disabled or wrapped inactivity bound.
        if seconds == 0 || seconds > i32::MAX as u32 / 1000 {
            ferroan_diag::refuse(
                TIMER_SECONDS,
                Kind::Hard,
                (i32::MAX as u32 / 1000) as u64,
                u64::from(seconds),
                key,
            );
            return Err(value(
                SECTION,
                key,
                "must be positive seconds fitting a signed millisecond timer",
            ));
        }
        Ok(Duration::from_secs(u64::from(seconds)))
    };
    Ok(CircuitTiming {
        default_rto: rto("DefaultRTO", 1000)?,
        max_rto: rto("MaxRTO", 3000)?,
        ack_timeout: silence("AckTimeout", "60")?,
        paused_ack_timeout: silence("PausedAckTimeout", "300")?,
    })
}

fn number(
    config: &Config,
    section: &str,
    key: &'static str,
    default: &str,
) -> Result<u32, RegionConfigError> {
    config
        .get(section, key)
        .unwrap_or(default)
        .trim()
        .parse()
        .map_err(|_| value(section, key, "expected an unsigned integer"))
}

fn port(
    config: &Config,
    section: &str,
    key: &'static str,
    default: &str,
) -> Result<u16, RegionConfigError> {
    let port = number(config, section, key, default)?;
    u16::try_from(port)
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| value(section, key, "expected a port from 1 to 65535"))
}

fn uuid(
    config: &Config,
    section: &str,
    key: &'static str,
    default: Option<Uuid>,
) -> Result<Uuid, RegionConfigError> {
    match config.get(section, key) {
        Some(text) => {
            Uuid::parse_str(text.trim()).map_err(|_| value(section, key, "expected a UUID"))
        }
        None => default.ok_or_else(|| value(section, key, "is required")),
    }
}

fn ip(
    config: &Config,
    section: &str,
    key: &'static str,
    default: &str,
) -> Result<IpAddr, RegionConfigError> {
    let text = config.get(section, key).unwrap_or(default).trim();
    if text.is_empty() {
        return Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
    text.parse()
        .map_err(|_| value(section, key, "expected an IP address"))
}

fn monitoring_settings(server: &Config) -> Result<MonitoringSettings, RegionConfigError> {
    let monitor_enabled = match server.get("Monitoring", "Enabled") {
        None => true,
        Some(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => {
                return Err(value("Monitoring", "Enabled", "expected true or false"));
            }
        },
    };
    let metrics_listen = if boolean(server, "Monitoring", "Metrics")? {
        Some(metrics_listen_address(server)?)
    } else {
        None
    };
    let managed_stats_uri = server
        .get("Startup", "ManagedStatsRemoteFetchURI")
        .unwrap_or("")
        .trim()
        .to_owned();
    if !managed_stats_uri.is_empty()
        && (managed_stats_uri.len() > 64
            || !managed_stats_uri
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    {
        return Err(value(
            "Startup",
            "ManagedStatsRemoteFetchURI",
            "expected one ASCII path segment",
        ));
    }
    let managed_stats_password = server
        .get("Startup", "ManagedStatsRemoteFetchPassword")
        .unwrap_or("")
        .to_owned();
    if managed_stats_password.len() > 256
        || managed_stats_password
            .bytes()
            .any(|byte| byte == b'\n' || byte == b'\r' || byte == 0)
    {
        return Err(value(
            "Startup",
            "ManagedStatsRemoteFetchPassword",
            "expected a single line of at most 256 bytes",
        ));
    }
    Ok(MonitoringSettings {
        monitor_enabled,
        metrics_listen,
        managed_stats_uri,
        managed_stats_password,
        web_stats: boolean(server, "WebStats", "enabled")?,
    })
}

fn metrics_listen_address(server: &Config) -> Result<SocketAddr, RegionConfigError> {
    match server
        .get("Monitoring", "Listen")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        None => Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, 9109))),
        Some(text) => text
            .parse()
            .map_err(|_| value("Monitoring", "Listen", "expected host:port")),
    }
}

/// `[Permissions]` overwrites `[Startup]`, as `GetConfigVarFromSections` does.
/// A missing key keeps `default`. Only `true` and `false` are accepted.
fn permission_flag(
    config: &Config,
    key: &'static str,
    default: bool,
) -> Result<bool, RegionConfigError> {
    let permissions = nonempty(config, "Permissions", key);
    let startup = nonempty(config, "Startup", key);
    let (section, raw) = if permissions.is_some() {
        ("Permissions", permissions)
    } else {
        ("Startup", startup)
    };
    match raw {
        None => Ok(default),
        Some(text) => match text.to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(value(section, key, "expected true or false")),
        },
    }
}

fn nonempty<'a>(config: &'a Config, section: &str, key: &str) -> Option<&'a str> {
    config
        .get(section, key)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// `[SimulationDataStore]` replaces `[DatabaseService]` for a key that is present.
/// A SQLite provider, or a SQLite connection with no provider, is the object store.
fn object_store(config: &Config) -> Option<String> {
    let mut provider = nonempty(config, "DatabaseService", "StorageProvider").map(str::to_owned);
    let mut connection = nonempty(config, "DatabaseService", "ConnectionString").map(str::to_owned);
    if config
        .get("SimulationDataStore", "StorageProvider")
        .is_some()
    {
        provider = nonempty(config, "SimulationDataStore", "StorageProvider").map(str::to_owned);
    }
    if config
        .get("SimulationDataStore", "ConnectionString")
        .is_some()
    {
        connection = nonempty(config, "SimulationDataStore", "ConnectionString").map(str::to_owned);
    }
    let connection = connection?;
    match provider.as_deref() {
        Some(provider) if provider.to_ascii_lowercase().contains("null") => None,
        Some(provider) if provider.to_ascii_lowercase().contains("sqlite") => Some(connection),
        Some(_) => None,
        None if sqlite_connection(&connection) => Some(connection),
        None => None,
    }
}

fn sqlite_connection(connection: &str) -> bool {
    let lower = connection.to_ascii_lowercase();
    if lower.contains("uri=file:") {
        return !lower.contains("mode=memory");
    }
    lower.contains("data source=") && lower.contains("version=3") && !lower.contains("database=")
}

/// Later keys replace earlier ones. An empty value clears the string.
fn estate_connection(config: &Config) -> Option<String> {
    let mut current = None;
    for (section, key) in [
        ("DatabaseService", "ConnectionString"),
        ("DatabaseService", "EstateConnectionString"),
        ("EstateDataStore", "ConnectionString"),
    ] {
        if config.get(section, key).is_some() {
            current = nonempty(config, section, key).map(str::to_owned);
        }
    }
    current
}

fn boolean(config: &Config, section: &str, key: &'static str) -> Result<bool, RegionConfigError> {
    match config
        .get(section, key)
        .unwrap_or("false")
        .to_ascii_lowercase()
        .as_str()
    {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(value(section, key, "expected true or false")),
    }
}

fn optional_uri(
    config: &Config,
    section: &str,
    key: &'static str,
) -> Result<String, RegionConfigError> {
    match config
        .get(section, key)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        None => Ok(String::new()),
        Some(text) => {
            check_uri(text).map_err(|reason| value(section, key, reason))?;
            Ok(text.to_owned())
        }
    }
}

fn library_settings(config: &Config) -> Result<(bool, String), RegionConfigError> {
    if !config.section_names().any(|name| name == "LibraryService") {
        return Ok((false, String::new()));
    }
    let name = config
        .get("LibraryService", "LibraryName")
        .unwrap_or("OpenSim Library")
        .trim();
    if name.len() > 256 {
        return Err(value("LibraryService", "LibraryName", "exceeds 256 bytes"));
    }
    Ok((true, name.to_owned()))
}

/// `DefaultLibrary` may be absolute. A relative path is joined to the region directory.
fn library_file(config_dir: &Path, config: &Config) -> Result<Option<PathBuf>, RegionConfigError> {
    if !config.section_names().any(|name| name == "LibraryService") {
        return Ok(None);
    }
    let Some(raw) = config
        .get("LibraryService", "DefaultLibrary")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(None);
    };
    if raw.len() > 4096 {
        return Err(value(
            "LibraryService",
            "DefaultLibrary",
            "exceeds 4096 bytes",
        ));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        Ok(Some(path.to_path_buf()))
    } else {
        Ok(Some(config_dir.join(path)))
    }
}

fn check_uri(text: &str) -> Result<(), &'static str> {
    if text.len() > 2048 {
        return Err("URI exceeds 2048 bytes");
    }
    let uri = reqwest::Url::parse(text).map_err(|_| "expected an HTTP service URI")?;
    if !matches!(uri.scheme(), "http" | "https")
        || uri.host_str().is_none()
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return Err("expected an HTTP service URI without credentials, query or fragment");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    fn server() -> Config {
        Config::parse("[Network]\nhttp_listener_port=9100\nbind_ip_address=127.0.0.2\n[GridService]\nGridServerURI=http://127.0.0.1:8003\n[PresenceService]\nPresenceServerURI=http://127.0.0.1:8003\n[GridUserService]\nGridUserServerURI=http://127.0.0.1:8003\n[Map]\nGenerateMaptiles=false\n").unwrap()
    }

    fn regions() -> Config {
        Config::parse("[Ferroan Test]\nRegionUUID=6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01\nLocation=1000,1001\nInternalAddress=127.0.0.1\nInternalPort=9100\nExternalHostName=127.0.0.1\n").unwrap()
    }

    #[test]
    fn login_connect_wait_matches_csharp_default_zero_and_negative_settings() {
        let mut server = server();
        assert_eq!(
            RegionConfig::from_configs(&server, &regions())
                .unwrap()
                .login_connect_wait,
            Duration::from_secs(120)
        );
        for (setting, expected) in [("0", 0), ("-1", 0), ("17", 17)] {
            server.set("Startup", "LoginConnectWaitSeconds", setting);
            assert_eq!(
                RegionConfig::from_configs(&server, &regions())
                    .unwrap()
                    .login_connect_wait,
                Duration::from_secs(expected)
            );
        }
        for invalid in ["not seconds", "2147483648"] {
            server.set("Startup", "LoginConnectWaitSeconds", invalid);
            assert!(RegionConfig::from_configs(&server, &regions()).is_err());
        }
    }

    #[test]
    fn grid_coordinates_and_advertised_address_are_separate_from_bind_address() {
        let region = RegionConfig::from_configs(&server(), &regions()).unwrap();
        assert_eq!(region.location, [1000, 1001]);
        assert_eq!(region.handle(), (256000u64 << 32) | 256256);
        assert_eq!(region.http_bind, "127.0.0.2:9100".parse().unwrap());
        assert_eq!(region.udp_bind, "127.0.0.1:9100".parse().unwrap());
        assert_eq!(region.server_uri, "http://127.0.0.1:9100/");
        assert_eq!(region.size, [256, 256]);
    }

    #[test]
    fn circuit_settings_follow_csharp_units_defaults_and_zero_rto_override() {
        let mut server = server();
        let config = RegionConfig::from_configs(&server, &regions()).unwrap();
        assert_eq!(
            config.circuit_timing.default_rto,
            Duration::from_millis(1000)
        );
        assert_eq!(config.circuit_timing.max_rto, Duration::from_millis(3000));
        assert_eq!(config.circuit_timing.ack_timeout, Duration::from_secs(60));
        assert_eq!(
            config.circuit_timing.paused_ack_timeout,
            Duration::from_secs(300)
        );
        for (key, setting) in [
            ("DefaultRTO", "750"),
            ("MaxRTO", "6000"),
            ("AckTimeout", "90"),
            ("PausedAckTimeout", "420"),
        ] {
            server.set("ClientStack.LindenUDP", key, setting);
        }
        let config = RegionConfig::from_configs(&server, &regions()).unwrap();
        assert_eq!(
            config.circuit_timing.default_rto,
            Duration::from_millis(750)
        );
        assert_eq!(config.circuit_timing.max_rto, Duration::from_millis(6000));
        assert_eq!(config.circuit_timing.ack_timeout, Duration::from_secs(90));
        assert_eq!(
            config.circuit_timing.paused_ack_timeout,
            Duration::from_secs(420)
        );
        server.set("ClientStack.LindenUDP", "DefaultRTO", "0");
        server.set("ClientStack.LindenUDP", "MaxRTO", "0");
        let config = RegionConfig::from_configs(&server, &regions()).unwrap();
        assert_eq!(
            config.circuit_timing.default_rto,
            Duration::from_millis(1000)
        );
        assert_eq!(config.circuit_timing.max_rto, Duration::from_millis(3000));
    }

    #[test]
    fn invalid_or_overflowing_circuit_timer_settings_are_refused() {
        for (key, setting) in [
            ("DefaultRTO", "-1"),
            ("MaxRTO", "2147483648"),
            ("AckTimeout", "0"),
            ("PausedAckTimeout", "2147484"),
            ("AckTimeout", "not-a-number"),
        ] {
            let mut server = server();
            server.set("ClientStack.LindenUDP", key, setting);
            assert!(
                matches!(RegionConfig::from_configs(&server, &regions()), Err(RegionConfigError::Value { key: actual, .. }) if actual == key)
            );
        }
    }

    #[test]
    fn arrival_preserves_login_position_and_uses_region_identity() {
        let mut region = RegionConfig::from_configs(&server(), &regions()).unwrap();
        region.channel_version = "Configured server identity".into();
        let arrival = region.arrival([17.0, 29.0, 0.0]);
        assert_eq!(arrival.position, [17.0, 29.0, 0.0]);
        assert_eq!(arrival.look_at, [0.99, 0.042, 0.0]);
        assert_eq!(arrival.region_handle, (256000u64 << 32) | 256256);
        assert_eq!(arrival.channel_version, "Configured server identity");
        assert!(arrival.teleport.is_none());
    }

    #[test]
    fn permission_flags_follow_startup_then_permissions_and_force_grid_gods() {
        let config = RegionConfig::from_configs(&server(), &regions()).unwrap();
        assert!(!config.allow_grid_gods);
        assert!(!config.region_owner_is_god);
        assert!(config.user_account_server_uri.is_empty());
        assert!(config.estate_store.is_none());

        let mut server = server();
        server.set("Startup", "allow_grid_gods", "false");
        server.set("Permissions", "allow_grid_gods", "true");
        server.set("Startup", "region_owner_is_god", "true");
        server.set(
            "DatabaseService",
            "ConnectionString",
            "Data Source=region.db;Version=3",
        );
        server.set(
            "EstateDataStore",
            "ConnectionString",
            "Data Source=estate.db;Version=3",
        );
        server.set(
            "UserAccountService",
            "UserAccountServerURI",
            "http://127.0.0.1:8003",
        );
        let config = RegionConfig::from_configs(&server, &regions()).unwrap();
        assert!(config.allow_grid_gods);
        assert!(config.region_owner_is_god);
        assert_eq!(config.user_account_server_uri, "http://127.0.0.1:8003");
        assert_eq!(
            config.estate_store.as_deref(),
            Some("Data Source=estate.db;Version=3")
        );

        server.set("Permissions", "force_grid_gods_only", "true");
        server.set("Permissions", "region_manager_is_god", "true");
        let config = RegionConfig::from_configs(&server, &regions()).unwrap();
        assert!(config.allow_grid_gods);
        assert!(!config.region_owner_is_god);
        assert!(!config.region_manager_is_god);

        server.set("Permissions", "allow_grid_gods", "maybe");
        assert!(RegionConfig::from_configs(&server, &regions()).is_err());
    }

    #[test]
    fn object_store_follows_the_simulation_section_and_stays_sqlite() {
        let mut server = server();
        assert!(
            RegionConfig::from_configs(&server, &regions())
                .unwrap()
                .object_store
                .is_none()
        );
        server.set(
            "DatabaseService",
            "ConnectionString",
            "URI=file:OpenSim.db;UseUTF16Encoding=True",
        );
        server.set(
            "DatabaseService",
            "StorageProvider",
            "OpenSim.Data.SQLite.dll",
        );
        let config = RegionConfig::from_configs(&server, &regions()).unwrap();
        assert_eq!(
            config.object_store.as_deref(),
            Some("URI=file:OpenSim.db;UseUTF16Encoding=True")
        );
        server.set(
            "SimulationDataStore",
            "ConnectionString",
            "Data Source=prims.db;Version=3",
        );
        let config = RegionConfig::from_configs(&server, &regions()).unwrap();
        assert_eq!(
            config.object_store.as_deref(),
            Some("Data Source=prims.db;Version=3")
        );
        server.set(
            "SimulationDataStore",
            "StorageProvider",
            "OpenSim.Data.Null.dll",
        );
        assert!(
            RegionConfig::from_configs(&server, &regions())
                .unwrap()
                .object_store
                .is_none()
        );
        server.set(
            "SimulationDataStore",
            "StorageProvider",
            "OpenSim.Data.MySQL.dll",
        );
        assert!(
            RegionConfig::from_configs(&server, &regions())
                .unwrap()
                .object_store
                .is_none()
        );
    }

    #[test]
    fn handshake_uses_region_identity_type_and_configured_estate_owner() {
        let mut server = server();
        server.set(
            "Estates",
            "DefaultEstateOwnerUUID",
            "00000000-0000-0000-0000-000000000003",
        );
        let mut regions = regions();
        regions.set("Ferroan Test", "RegionType", "Configured Region Type");
        let config = RegionConfig::from_configs(&server, &regions).unwrap();
        let packet = config.handshake.build(true, false);
        assert_eq!(packet.region_info.sim_name, b"Ferroan Test\0");
        assert_eq!(
            packet.region_info3.product_name,
            b"Configured Region Type\0"
        );
        assert_eq!(packet.region_info2.region_id, config.id);
        assert_eq!(packet.region_info.sim_owner, config.estate_owner);
        assert!(!packet.region_info.cache_id.is_nil());
        assert_eq!(
            packet.region_info.cache_id,
            config.clone().handshake.cache_id
        );
    }

    #[test]
    fn invalid_region_inputs_return_errors_before_any_network_binding() {
        for (key, setting) in [
            ("RegionUUID", "not-a-uuid"),
            ("Location", "8388608,0"),
            ("Location", "-1,1000"),
            ("Location", "1000,1000,1000"),
            ("InternalPort", "65536"),
            ("InternalPort", "0"),
            ("InternalAddress", "host.example.invalid"),
            ("ExternalHostName", "host.example.invalid/path"),
            ("SizeX", "512"),
            ("MaptileStaticUUID", "00000000-0000-0000-0000-000000000002"),
        ] {
            let mut regions = regions();
            regions.merge(&Config::parse(&format!("[Ferroan Test]\n{key}={setting}\n")).unwrap());
            assert!(
                RegionConfig::from_configs(&server(), &regions).is_err(),
                "{key}"
            );
        }
        let mut multiple = regions();
        multiple.merge(
            &Config::parse("[Second Region]\nRegionUUID=00000000-0000-0000-0000-000000000002\n")
                .unwrap(),
        );
        assert!(matches!(
            RegionConfig::from_configs(&server(), &multiple),
            Err(RegionConfigError::RegionCount)
        ));
    }

    #[test]
    fn service_uri_credentials_are_refused_without_echoing_them() {
        let mut config = server();
        config.merge(
            &Config::parse(
                "[GridService]\nGridServerURI=http://test:private-test-value@localhost:8003\n",
            )
            .unwrap(),
        );
        let error = RegionConfig::from_configs(&config, &regions())
            .err()
            .unwrap();
        assert!(!error.to_string().contains("private-test-value"));
    }

    #[test]
    fn region_server_includes_and_substitution_use_ferroan_config() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.scratch")
            .join(format!("region-config-{}", Uuid::new_v4()));
        fs::create_dir_all(dir.join("Regions")).unwrap();
        fs::write(dir.join("RegionServer.ini"), "[Const]\nServicePort=8003\n[GridService]\nGridServerURI=http://127.0.0.1:${Const|ServicePort}\n[PresenceService]\nPresenceServerURI=http://127.0.0.1:${Const|ServicePort}\n[GridUserService]\nGridUserServerURI=http://127.0.0.1:${Const|ServicePort}\n[Startup]\nInclude-Network=network.ini\n").unwrap();
        fs::write(
            dir.join("network.ini"),
            "[Network]\nhttp_listener_port=9100\nbind_ip_address=127.0.0.2\n[Map]\nGenerateMaptiles=false\n",
        )
        .unwrap();
        fs::write(dir.join("Regions/custom-name.ini"), "[Ferroan Test]\nRegionUUID=6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01\nLocation=1000,1001\nInternalAddress=127.0.0.1\nInternalPort=9100\nExternalHostName=127.0.0.1\n").unwrap();
        let load = RegionLoad {
            config_dir: dir.clone(),
            argv: Config::parse(
                "[Startup]\ninimaster=none\ninifile=RegionServer.ini\ninidirectory=missing\n",
            )
            .unwrap(),
            environment: BTreeMap::new(),
        };
        let config = RegionConfig::load(&load).unwrap();
        assert_eq!(config.grid_server_uri, "http://127.0.0.1:8003");
        assert_eq!(config.http_bind, "127.0.0.2:9100".parse().unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_overrides_map_section_for_generation_and_static_tile() {
        let mut server = server();
        server.set("Map", "GenerateMaptiles", "true");
        server.set("Startup", "GenerateMaptiles", "false");
        server.set(
            "Map",
            "MaptileStaticUUID",
            "00000000-0000-0000-0000-000000000001",
        );
        server.set(
            "Startup",
            "MaptileStaticUUID",
            "00000000-0000-0000-0000-000000000000",
        );
        assert!(RegionConfig::from_configs(&server, &regions()).is_ok());
        server.set("Map", "GenerateMaptiles", "false");
        server.set("Startup", "GenerateMaptiles", "true");
        assert!(matches!(
            RegionConfig::from_configs(&server, &regions()),
            Err(RegionConfigError::Value {
                key: "GenerateMaptiles",
                ..
            })
        ));
        server.set("Startup", "GenerateMaptiles", "false");
        server.set(
            "Map",
            "MaptileStaticUUID",
            "00000000-0000-0000-0000-000000000000",
        );
        server.set(
            "Startup",
            "MaptileStaticUUID",
            "00000000-0000-0000-0000-000000000001",
        );
        assert!(matches!(
            RegionConfig::from_configs(&server, &regions()),
            Err(RegionConfigError::Value {
                key: "MaptileStaticUUID",
                ..
            })
        ));
    }

    #[test]
    fn unsupported_auth_is_refused_for_network_and_each_used_service_without_secrets() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/pin3/login-robustness.json"))
                .unwrap();
        for section in [
            "Network",
            "GridService",
            "PresenceService",
            "GridUserService",
            "InventoryService",
            "AssetService",
        ] {
            for entry in fixture["auth_types"]["network"].as_array().unwrap() {
                let auth = entry["auth"].as_str().unwrap();
                let mut config = server();
                config.set(section, "AuthType", auth);
                config.set(section, "HttpAuthPassword", "private-test-value");
                let result = RegionConfig::from_configs(&config, &regions());
                if entry["adds_authorization"].as_bool() == Some(true) {
                    let error = result.err().unwrap();
                    assert!(
                        matches!(
                            error,
                            RegionConfigError::Value {
                                key: "AuthType",
                                ..
                            }
                        ),
                        "{section} {auth}"
                    );
                    assert!(!error.to_string().contains("private-test-value"));
                } else {
                    assert!(result.is_ok(), "{section} {auth}");
                }
            }
            let mut config = server();
            config.set(section, "AuthType", "NotAServiceAuthValue");
            let error = RegionConfig::from_configs(&config, &regions())
                .err()
                .unwrap();
            assert!(matches!(
                error,
                RegionConfigError::Value {
                    key: "AuthType",
                    ..
                }
            ));
        }
    }

    #[test]
    fn required_presence_and_grid_user_uris_are_validated_without_credentials() {
        for (section, key) in [
            ("PresenceService", "PresenceServerURI"),
            ("GridUserService", "GridUserServerURI"),
        ] {
            for setting in [
                "",
                "not-a-uri",
                "http://test:private-test-value@localhost:8003",
            ] {
                let mut config = server();
                config.set(section, key, setting);
                let error = RegionConfig::from_configs(&config, &regions())
                    .err()
                    .unwrap();
                assert!(
                    matches!(error, RegionConfigError::Value { key: actual, .. } if actual == key)
                );
                assert!(!error.to_string().contains("private-test-value"));
            }
        }
        let mut config = server();
        config.set(
            "GridUserService",
            "GridUserServerURI",
            "http://127.0.0.1:8003/",
        );
        assert!(RegionConfig::from_configs(&config, &regions()).is_ok());
        config.set(
            "GridUserService",
            "GridUserServerURI",
            "http://127.0.0.1:8003/prefix",
        );
        assert!(matches!(
            RegionConfig::from_configs(&config, &regions()),
            Err(RegionConfigError::Value {
                key: "GridUserServerURI",
                ..
            })
        ));
    }

    #[test]
    fn all_ini_files_count_regions_separately_and_xml_is_refused() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.scratch")
            .join(format!("region-files-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        assert!(matches!(
            load_regions(&dir),
            Err(RegionConfigError::RegionCount)
        ));
        fs::write(dir.join("empty.ini"), "; no region here\n").unwrap();
        fs::write(dir.join("custom.ini"), "[Same Name]\nLocation=1000,1000\n").unwrap();
        assert_eq!(
            load_regions(&dir).unwrap().get("Same Name", "Location"),
            Some("1000,1000")
        );
        fs::write(dir.join("second.ini"), "[Same Name]\nLocation=1001,1000\n").unwrap();
        assert!(matches!(
            load_regions(&dir),
            Err(RegionConfigError::RegionCount)
        ));
        fs::remove_file(dir.join("second.ini")).unwrap();
        fs::write(dir.join("old-region.xml"), "<Region/>\n").unwrap();
        assert!(matches!(
            load_regions(&dir),
            Err(RegionConfigError::XmlRegions)
        ));
        fs::remove_dir_all(dir).unwrap();
    }
}
