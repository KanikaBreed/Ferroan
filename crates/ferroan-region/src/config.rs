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
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The single region used by P2.0d. INI syntax, includes and substitutions
//! belong to ferroan-config. Interactive creation, XML regions, variable
//! region sizes and TLS listeners remain unsupported and are refused.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use ferroan_client::HandshakeConfig;
use ferroan_config::{Config, RegionLoad};
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
    #[error("Regions.ini must define exactly one region")]
    RegionCount,
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
    pub estate_owner: Uuid,
    pub handshake: HandshakeConfig,
    pub region_secret: Uuid,
}

impl RegionConfig {
    pub fn load(load: &RegionLoad) -> Result<Self, RegionConfigError> {
        let resolved = load.load()?;
        let directory = resolved
            .config
            .get("Startup", "regionload_regionsdir")
            .unwrap_or("Regions")
            .trim();
        let regions_path = load
            .config_dir
            .join(Path::new(directory))
            .join("Regions.ini");
        let regions = Config::parse_file(&regions_path)?;
        Self::from_configs(&resolved.config, &regions)
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
        let generate_maps = server
            .get("Map", "GenerateMaptiles")
            .or_else(|| server.get("Startup", "GenerateMaptiles"))
            .unwrap_or("true");
        if !generate_maps.eq_ignore_ascii_case("false") {
            return Err(value(
                "Map",
                "GenerateMaptiles",
                "P2.0d requires false; map generation is not implemented",
            ));
        }
        let map_section = if server.get("Map", "MaptileStaticUUID").is_some() {
            "Map"
        } else {
            "Startup"
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
        // RegionSettings and EstateSettings are not INI sections in C#.
        // Until their stores are implemented this region uses fresh settings;
        // only the genuine RegionInfo/estate-owner INI fields override them.
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
            estate_owner,
            handshake,
            region_secret: Uuid::new_v4(),
        })
    }

    pub fn handle(&self) -> u64 {
        (u64::from(self.location[0]) * 256) << 32 | (u64::from(self.location[1]) * 256)
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
        Config::parse("[Network]\nhttp_listener_port=9100\nbind_ip_address=127.0.0.2\n[GridService]\nGridServerURI=http://127.0.0.1:8003\n[Map]\nGenerateMaptiles=false\n").unwrap()
    }

    fn regions() -> Config {
        Config::parse("[Ferroan Test]\nRegionUUID=6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01\nLocation=1000,1001\nInternalAddress=127.0.0.1\nInternalPort=9100\nExternalHostName=127.0.0.1\n").unwrap()
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
        fs::write(dir.join("RegionServer.ini"), "[Const]\nServicePort=8003\n[GridService]\nGridServerURI=http://127.0.0.1:${Const|ServicePort}\n[Startup]\nInclude-Network=network.ini\n").unwrap();
        fs::write(
            dir.join("network.ini"),
            "[Network]\nhttp_listener_port=9100\nbind_ip_address=127.0.0.2\n[Map]\nGenerateMaptiles=false\n",
        )
        .unwrap();
        fs::write(dir.join("Regions/Regions.ini"), "[Ferroan Test]\nRegionUUID=6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01\nLocation=1000,1001\nInternalAddress=127.0.0.1\nInternalPort=9100\nExternalHostName=127.0.0.1\n").unwrap();
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
}
