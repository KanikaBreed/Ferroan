// Copyright (c) 2025 Utopia Skye LLC
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Server.Handlers/Base/ControlPlaneAccess.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 858321883f5487a5d192d36ea9d7b90fa468310c.
// The source has no file header; the repository LICENSE.txt assigns new
// Tranquillity files the copyright and MPL notice above. This port remains
// available under MPL-2.0 and forms part of Ferroan's GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! The general `Authorize` address policy used by AgentSimpleHandler.POST.
//! The separate explicit-IP mutation policy is not used by that endpoint.
//! DNS is resolved once before listeners start; request handling never resolves
//! names or trusts forwarding headers. Configuration and resolution are bounded.

use std::collections::HashSet;
use std::future::Future;
use std::io;
use std::net::IpAddr;
use std::time::Duration;

use ferroan_config::Config;

const MAX_HOSTS: usize = 256;
const MAX_HOST_BYTES: usize = 2048;
const MAX_ADDRESSES: usize = 4096;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum ControlPlaneError {
    #[error("control-plane trust configuration exceeds its host or address limit")]
    Limit,
    #[error("control-plane host resolution exceeded the startup timeout")]
    ResolveTimeout,
    #[error(
        "control-plane host URI has no hostname; use an explicit HTTP URI or bracketed IPv6 address"
    )]
    EmptyUriHost,
}

#[derive(Clone, Default)]
pub struct ControlPlaneAccess {
    addresses: HashSet<IpAddr>,
    pending_hosts: Vec<String>,
}

impl ControlPlaneAccess {
    pub fn from_config(config: &Config) -> Result<Self, ControlPlaneError> {
        let mut policy = Self::default();
        let configured = ["Security", "Network"]
            .into_iter()
            .flat_map(|section| {
                ["ControlPlaneTrustedHosts", "TrustedControlPlaneHosts"]
                    .into_iter()
                    .filter_map(move |key| config.get(section, key))
            })
            .find(|value| !value.trim().is_empty())
            .unwrap_or("");
        let mut count = 0;
        for host in configured.split([',', ';', '|', ' ', '\t', '\r', '\n']) {
            if host.is_empty() {
                continue;
            }
            count += 1;
            if count > MAX_HOSTS {
                return Err(ControlPlaneError::Limit);
            }
            policy.add_host(host.trim())?;
        }
        if let Some(host) = config.get("Network", "hostname") {
            policy.add_host(host.trim())?;
        }
        Ok(policy)
    }

    fn add_host(&mut self, text: &str) -> Result<(), ControlPlaneError> {
        if text.is_empty() {
            return Ok(());
        }
        if text.len() > MAX_HOST_BYTES {
            return Err(ControlPlaneError::Limit);
        }
        let host = if let Ok(uri) = reqwest::Url::parse(text) {
            // C# resolves an empty URI host as the machine's own addresses.
            // Refuse this ambiguous configuration rather than broadening trust.
            uri.host_str()
                .filter(|host| !host.is_empty())
                .ok_or(ControlPlaneError::EmptyUriHost)?
                .trim_matches(['[', ']'])
                .to_owned()
        } else if let Some(tail) = text.strip_prefix('[') {
            tail.split_once(']')
                .map_or(text, |(host, _)| host)
                .to_owned()
        } else if text.matches(':').count() == 1 {
            text.rsplit_once(':')
                .map_or(text, |(host, _)| host)
                .to_owned()
        } else {
            text.to_owned()
        };
        if host.is_empty() {
            return Ok(());
        }
        if let Ok(address) = host.parse() {
            self.addresses.insert(normalize_address(address));
        } else if !self.pending_hosts.contains(&host) {
            if self.pending_hosts.len() >= MAX_HOSTS {
                return Err(ControlPlaneError::Limit);
            }
            self.pending_hosts.push(host);
        }
        Ok(())
    }

    pub fn allows(&self, address: IpAddr) -> bool {
        let address = normalize_address(address);
        address.is_loopback() || self.addresses.contains(&address)
    }

    /// Resolve configured names before exposing the control-plane listener.
    /// Failed DNS lookups are ignored like C#; exceeding the total startup bound
    /// fails configuration rather than starting with a partially resolved policy.
    pub async fn resolve(&mut self) -> Result<(), ControlPlaneError> {
        self.resolve_with(|host| async move {
            tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map(|addresses| {
                    addresses
                        .take(MAX_ADDRESSES + 1)
                        .map(|address| address.ip())
                        .collect()
                })
        })
        .await
    }

    async fn resolve_with<F, Fut>(&mut self, mut lookup: F) -> Result<(), ControlPlaneError>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = io::Result<Vec<IpAddr>>>,
    {
        let deadline = tokio::time::Instant::now() + RESOLVE_TIMEOUT;
        for host in std::mem::take(&mut self.pending_hosts) {
            if let Ok(addresses) = tokio::time::timeout_at(deadline, lookup(host))
                .await
                .map_err(|_| ControlPlaneError::ResolveTimeout)?
            {
                if addresses.len() > MAX_ADDRESSES {
                    return Err(ControlPlaneError::Limit);
                }
                for address in addresses {
                    self.addresses.insert(normalize_address(address));
                    if self.addresses.len() > MAX_ADDRESSES {
                        return Err(ControlPlaneError::Limit);
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn normalize_address(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map_or(IpAddr::V6(address), IpAddr::V4),
        address => address,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn trust_alias_precedence_delimiters_localhost_and_mapped_addresses_match_csharp() {
        let config = Config::parse("[Security]\nControlPlaneTrustedHosts=\nTrustedControlPlaneHosts=\"192.0.2.1;192.0.2.2|[::ffff:192.0.2.3]:9000 2001:db8::1\"\n[Network]\nControlPlaneTrustedHosts=192.0.2.9\nhostname=http://192.0.2.4:9000/path\n").unwrap();
        let policy = ControlPlaneAccess::from_config(&config).unwrap();
        for address in [
            "127.0.0.9",
            "::1",
            "::ffff:127.0.0.9",
            "192.0.2.1",
            "192.0.2.2",
            "192.0.2.3",
            "::ffff:192.0.2.3",
            "192.0.2.4",
            "2001:db8::1",
        ] {
            assert!(policy.allows(ip(address)), "{address}");
        }
        for address in ["192.0.2.9", "192.0.2.10", "2001:db8::2"] {
            assert!(!policy.allows(ip(address)), "{address}");
        }
    }

    #[test]
    fn network_alias_is_used_when_security_is_unset_and_limits_do_not_echo_values() {
        let config = Config::parse("[Network]\nTrustedControlPlaneHosts=192.0.2.6:8003\n").unwrap();
        assert!(
            ControlPlaneAccess::from_config(&config)
                .unwrap()
                .allows(ip("192.0.2.6"))
        );
        let mut config = Config::default();
        config.set(
            "Security",
            "ControlPlaneTrustedHosts",
            &"192.0.2.1,".repeat(MAX_HOSTS + 1),
        );
        assert!(matches!(
            ControlPlaneAccess::from_config(&config),
            Err(ControlPlaneError::Limit)
        ));
        config.set(
            "Security",
            "ControlPlaneTrustedHosts",
            &"x".repeat(MAX_HOST_BYTES + 1),
        );
        let error = ControlPlaneAccess::from_config(&config).err().unwrap();
        assert!(!error.to_string().contains(&"x".repeat(100)));
    }

    #[test]
    fn opaque_uri_hosts_are_refused_instead_of_trusting_machine_addresses() {
        // .NET10 oracle: these are absolute URIs whose Host is empty. C# then
        // calls Dns.GetHostAddresses("") and trusts the machine-local results.
        for host in ["robust:8003", "robust.example.invalid:8003", "fd00::1"] {
            let mut config = Config::default();
            config.set("Network", "ControlPlaneTrustedHosts", host);
            assert!(matches!(
                ControlPlaneAccess::from_config(&config),
                Err(ControlPlaneError::EmptyUriHost)
            ));
        }
        let config = Config::parse(
            "[Network]\nControlPlaneTrustedHosts=[fd00::1]:9000,http://[fd00::2]:9000/\n",
        )
        .unwrap();
        let policy = ControlPlaneAccess::from_config(&config).unwrap();
        assert!(policy.allows(ip("fd00::1")));
        assert!(policy.allows(ip("fd00::2")));
    }

    #[tokio::test]
    async fn dns_is_resolved_once_normalized_and_failures_are_ignored() {
        let config = Config::parse("[Network]\nControlPlaneTrustedHosts=http://robust.example.invalid:8003,absent.example.invalid\n").unwrap();
        let mut policy = ControlPlaneAccess::from_config(&config).unwrap();
        policy
            .resolve_with(|host| async move {
                if host == "robust.example.invalid" {
                    Ok(vec![ip("::ffff:192.0.2.7"), ip("2001:db8::7")])
                } else {
                    Err(io::Error::from(io::ErrorKind::NotFound))
                }
            })
            .await
            .unwrap();
        assert!(policy.allows(ip("192.0.2.7")));
        assert!(policy.allows(ip("2001:db8::7")));
        assert!(policy.pending_hosts.is_empty());
        policy
            .resolve_with(|_| async { panic!("must not resolve twice") })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dns_results_and_startup_wait_are_bounded() {
        let config =
            Config::parse("[Network]\nControlPlaneTrustedHosts=robust.example.invalid\n").unwrap();
        let mut policy = ControlPlaneAccess::from_config(&config).unwrap();
        let result = policy
            .resolve_with(|_| async { Ok(vec![ip("192.0.2.1"); MAX_ADDRESSES + 1]) })
            .await;
        assert!(matches!(result, Err(ControlPlaneError::Limit)));
        let mut policy = ControlPlaneAccess::from_config(&config).unwrap();
        let result = tokio::time::timeout(
            RESOLVE_TIMEOUT + Duration::from_secs(1),
            policy.resolve_with(|_| std::future::pending()),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(ControlPlaneError::ResolveTimeout)));
    }
}
