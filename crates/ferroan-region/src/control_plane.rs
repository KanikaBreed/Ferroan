// Copyright (c) 2025 Utopia Skye LLC
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Server.Handlers/Base/ControlPlaneAccess.cs
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5.
// The source has no file header; the repository LICENSE.txt assigns new
// Tranquillity files the copyright and MPL notice above. This port remains
// available under MPL-2.0 and forms part of Ferroan's GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! General `Authorize` address policy used by AgentSimpleHandler.POST.
//! Names refresh after untrusted calls at most once a minute. Readers use the
//! last snapshot during refresh. Failed/empty lookups retain that name's old
//! addresses; successful lookups replace them. The separate explicit-IP
//! mutation policy is not used by this endpoint. Resource bounds and private
//! warning text are Ferroan additions; the matching policy follows C#.

use ferroan_config::Config;
use std::collections::HashSet;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::time::Instant;

const MAX_HOSTS: usize = 256;
const MAX_HOST_BYTES: usize = 2048;
const MAX_ADDRESSES: usize = 4096;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum ControlPlaneError {
    #[error("control-plane trust configuration exceeds its host or address limit")]
    Limit,
    #[error("control-plane host resolution exceeded its time limit")]
    ResolveTimeout,
}
#[derive(Default)]
struct Resolution {
    by_name: Vec<Vec<IpAddr>>,
    next: Option<Instant>,
}
#[derive(Default)]
struct Policy {
    addresses: HashSet<IpAddr>,
    names: Vec<(String, usize)>,
    trusted: RwLock<HashSet<IpAddr>>,
    resolution: tokio::sync::Mutex<Resolution>,
    warned: Mutex<HashSet<(usize, &'static str)>>,
}
#[derive(Clone, Default)]
pub struct ControlPlaneAccess(Arc<Policy>);

impl ControlPlaneAccess {
    pub fn from_config(config: &Config) -> Result<Self, ControlPlaneError> {
        let mut policy = Policy::default();
        let mut count = 0;
        for section in ["Security", "Network"] {
            for key in ["ControlPlaneTrustedHosts", "TrustedControlPlaneHosts"] {
                for entry in config
                    .get(section, key)
                    .unwrap_or("")
                    .split([',', ';', '|', ' ', '\t', '\r', '\n'])
                {
                    if entry.is_empty() {
                        continue;
                    }
                    count += 1;
                    if count > MAX_HOSTS {
                        return Err(ControlPlaneError::Limit);
                    }
                    policy.add_host(entry, count)?;
                }
            }
        }
        if let Some(entry) = config.get("Network", "hostname") {
            policy.add_host(entry, count + 1)?;
        }
        policy.trusted = RwLock::new(policy.addresses.clone());
        Ok(Self(Arc::new(policy)))
    }
    fn cached(&self, address: IpAddr) -> bool {
        // No user code runs under these short snapshot locks. Recover their
        // intact values if another internal operation ever poisons a lock.
        address.is_loopback()
            || self
                .0
                .trusted
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&address)
    }
    pub async fn allows(&self, address: IpAddr) -> bool {
        self.allows_with(address, lookup).await
    }
    async fn allows_with<F, Fut>(&self, address: IpAddr, lookup: F) -> bool
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = io::Result<Vec<IpAddr>>>,
    {
        let address = normalize_address(address);
        if self.cached(address) {
            return true;
        }
        if self.0.names.is_empty() {
            return false;
        }
        // C# does not queue other requests behind the DNS winner.
        let Ok(mut state) = self.0.resolution.try_lock() else {
            return self.cached(address);
        };
        if state.next.is_some_and(|next| Instant::now() < next) {
            return self.cached(address);
        }
        if self.resolve_locked(&mut state, lookup).await.is_err() {
            self.0.warn(
                0,
                "lookup batch exceeded its resource bound; retaining previous addresses",
            );
        }
        state.next = Some(Instant::now() + REFRESH_INTERVAL);
        self.cached(address)
    }
    /// Startup ignores failed DNS lookups, but fails on resource-bound errors.
    pub async fn resolve(&mut self) -> Result<(), ControlPlaneError> {
        self.resolve_with(lookup).await
    }
    async fn resolve_with<F, Fut>(&self, lookup: F) -> Result<(), ControlPlaneError>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = io::Result<Vec<IpAddr>>>,
    {
        let mut state = self.0.resolution.lock().await;
        if state.next.is_some() && state.by_name.len() == self.0.names.len() {
            return Ok(());
        }
        self.resolve_locked(&mut state, lookup).await
    }
    async fn resolve_locked<F, Fut>(
        &self,
        state: &mut Resolution,
        mut lookup: F,
    ) -> Result<(), ControlPlaneError>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = io::Result<Vec<IpAddr>>>,
    {
        // Reserve before awaiting: cancellation cannot multiply DNS batches.
        state.next = Some(Instant::now() + REFRESH_INTERVAL);
        let mut by_name = state.by_name.clone();
        by_name.resize(self.0.names.len(), Vec::new());
        let deadline = Instant::now() + RESOLVE_TIMEOUT;
        for (index, (host, entry)) in self.0.names.iter().enumerate() {
            let result = tokio::time::timeout_at(deadline, lookup(host.clone()))
                .await
                .map_err(|_| ControlPlaneError::ResolveTimeout)?;
            match result {
                Ok(addresses) if !addresses.is_empty() => {
                    if addresses.len() > MAX_ADDRESSES {
                        return Err(ControlPlaneError::Limit);
                    }
                    let found: HashSet<_> = addresses.into_iter().map(normalize_address).collect();
                    by_name[index] = found.into_iter().collect();
                    if self.0.addresses.len() + by_name.iter().map(Vec::len).sum::<usize>()
                        > MAX_ADDRESSES
                    {
                        return Err(ControlPlaneError::Limit);
                    }
                }
                Ok(_) => self
                    .0
                    .warn(*entry, "has no address; retaining previous addresses"),
                Err(_) => self.0.warn(
                    *entry,
                    "could not be resolved; retaining previous addresses",
                ),
            }
        }
        let mut trusted = self.0.addresses.clone();
        trusted.extend(by_name.iter().flatten().copied());
        *self.0.trusted.write().unwrap_or_else(|e| e.into_inner()) = trusted;
        state.by_name = by_name;
        state.next = Some(Instant::now() + REFRESH_INTERVAL);
        Ok(())
    }
}
impl Policy {
    fn add_host(&mut self, entry: &str, index: usize) -> Result<(), ControlPlaneError> {
        let entry = entry.trim();
        if entry.is_empty() {
            return Ok(());
        }
        if entry.len() > MAX_HOST_BYTES {
            return Err(ControlPlaneError::Limit);
        }
        let host = host_of(entry);
        if let Some(address) = parse_address(&host) {
            self.addresses.insert(normalize_address(address));
        } else if host.is_empty() {
            self.warn(index, "names no host; ignoring entry");
        } else if host.contains('/') {
            self.warn(
                index,
                "contains an unsupported address range; list individual addresses",
            );
        } else if !self
            .names
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(&host))
        {
            if self.names.len() >= MAX_HOSTS {
                return Err(ControlPlaneError::Limit);
            }
            self.names.push((host, index));
        }
        Ok(())
    }
    fn warn(&self, entry: usize, problem: &'static str) {
        if self
            .warned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((entry, problem))
        {
            eprintln!("ferroan-region: control-plane trusted-host entry {entry} {problem}");
        }
    }
}
async fn lookup(host: String) -> io::Result<Vec<IpAddr>> {
    tokio::net::lookup_host((host.as_str(), 0))
        .await
        .map(|addresses| {
            addresses
                .take(MAX_ADDRESSES + 1)
                .map(|address| address.ip())
                .collect()
        })
}
fn host_of(entry: &str) -> String {
    let entry = entry.trim();
    if entry.is_empty() || parse_address(entry).is_some() {
        return entry.to_owned();
    }
    if let Ok(uri) = reqwest::Url::parse(entry)
        && let Some(host) = uri.host_str().filter(|host| !host.is_empty())
    {
        return host.to_owned();
    }
    // .NET Uri accepts scoped IPv6 URL literals; the URL crate rejects them.
    if let Some((_, authority)) = entry.split_once("://")
        && let Some(tail) = authority.strip_prefix('[')
        && let Some((address, _)) = tail.split_once(']')
        && let Some(IpAddr::V6(address)) = parse_address(address)
    {
        return format!("[{address}]");
    }
    if let Some(tail) = entry.strip_prefix('[') {
        return tail
            .split_once(']')
            .map_or_else(String::new, |(host, _)| host.to_owned());
    }
    if let Some((host, _)) = entry.rsplit_once(':')
        && !host.contains(':')
    {
        return host.to_owned();
    }
    entry.to_owned()
}
fn parse_address(text: &str) -> Option<IpAddr> {
    let text = if let Some(tail) = text.strip_prefix('[') {
        let (address, suffix) = tail.split_once(']')?;
        // .NET accepts an IPv6 literal followed by a decimal port, without a
        // u16 port range check. Other suffixes and bracketed IPv4 are not IPs.
        if !address.contains(':')
            || !(suffix.is_empty()
                || suffix.strip_prefix(':').is_some_and(|port| {
                    !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
                }))
        {
            return None;
        }
        address
    } else {
        text
    };
    if text.contains(':') {
        // IpAddr and SocketAddr::ip omit the incoming interface scope.
        return text
            .split('%')
            .next()?
            .parse::<std::net::Ipv6Addr>()
            .ok()
            .map(IpAddr::V6);
    }
    if let Ok(address) = text.parse() {
        return Some(address);
    }
    // IPAddress.TryParse accepts traditional IPv4 decimal/octal/hex forms.
    let parts: Vec<_> = text.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut address = 0u32;
    for (index, part) in parts.iter().enumerate() {
        let (digits, radix) =
            if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                (hex, 16)
            } else if part.len() > 1 && part.starts_with('0') {
                (&part[1..], 8)
            } else {
                (*part, 10)
            };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(digits, radix).ok()?;
        if index + 1 == parts.len() {
            let remaining = 32 - 8 * index;
            if remaining < 32 && value >= (1u32 << remaining) {
                return None;
            }
            address |= value;
        } else {
            if value > 255 {
                return None;
            }
            address |= value << (24 - 8 * index);
        }
    }
    Some(IpAddr::V4(Ipv4Addr::from(address)))
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
#[path = "control_plane_tests.rs"]
mod tests;
