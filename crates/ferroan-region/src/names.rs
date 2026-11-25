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
//   Source/OpenSim.Region.CoreModules/Framework/UserManagement/UserManagementModule.cs
//     (GetUser, CheckUrl)
//   Source/OpenSim.Framework/Util.cs (ParseFullUniversalUserIdentifier)
//   Source/OpenSim.Framework/GridInfo.cs (GridInfo gatekeeper settings,
//     IsLocalGrid, OSHHTPHost)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// The gatekeeper is not resolved through DNS. C# refuses to start when it
// cannot resolve it; Ferroan compares host and port only, as IsLocalGrid does.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The grid lookups behind `UUIDNameRequest`: the account service, then
//! GridUser's universal identifier.

use std::sync::Arc;

use ferroan_client::{NameFuture, NameSource, UserName};
use ferroan_config::Config;
use uuid::Uuid;

use crate::grid::GridUserClient;

/// `OSHHTPHost`: a lower-case host and a port. A scheme is optional and a
/// path is not allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpHost {
    host: String,
    port: u16,
}

/// What `new OSHHTPHost(url)` makes of a string.
#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Empty,
    Invalid,
    Valid(HttpHost),
}

impl HttpHost {
    /// `OSHHTPHost`'s constructor without DNS.
    fn parse(url: &str) -> Parsed {
        if url.is_empty() {
            return Parsed::Empty;
        }
        let url = url.to_lowercase();
        let mut end = url.len();
        if url.ends_with('/') {
            end -= 1;
        }
        let (start, secure) = if url.starts_with("http") {
            match url.as_bytes().get(4) {
                Some(b's') => (8, true),
                Some(_) => (7, false),
                None => return Parsed::Invalid,
            }
        } else {
            (0, false)
        };
        let Some(text) = url.get(start..end) else {
            return Parsed::Invalid;
        };
        let (host, port) = match text.find(':') {
            Some(colon) if colon > 0 || start > 0 => {
                let host = &text[..colon];
                let Ok(port) = text[colon + 1..].trim().parse::<i64>() else {
                    return Parsed::Invalid;
                };
                let Ok(port) = u16::try_from(port) else {
                    return Parsed::Invalid;
                };
                (host, port)
            }
            Some(_) => return Parsed::Invalid,
            None => (text, if secure { 443 } else { 80 }),
        };
        if !host_name(host) {
            return Parsed::Invalid;
        }
        Parsed::Valid(Self {
            host: host.to_owned(),
            port,
        })
    }

    /// `HostAndPort`.
    fn host_and_port(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// `Uri.CheckHostName` is `Dns`, `IPv4` or `IPv6`: a dotted name of letters,
/// digits, hyphens and underscores, or an address.
fn host_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 255 {
        return false;
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    })
}

/// This grid's gatekeeper and its aliases, as `GridInfo` reads them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridHome {
    gatekeeper: Option<HttpHost>,
    aliases: Vec<HttpHost>,
}

const SECTIONS: [&str; 3] = ["Const", "Startup", "Hypergrid"];

/// `Util.GetConfigVarFromSections`: the last of the sections that sets it.
fn from_sections<'a>(config: &'a Config, key: &str) -> Option<&'a str> {
    SECTIONS
        .iter()
        .filter_map(|section| config.get(section, key))
        .next_back()
}

impl GridHome {
    /// `GatekeeperURI` from `[Const]`, `[Startup]` and `[Hypergrid]`, then
    /// `[GatekeeperService] ExternalName`, then `[GridService] Gatekeeper`,
    /// then the region's own server URI; `GatekeeperURIAlias` is a comma
    /// list from the same sections.
    pub fn from_config(config: &Config, server_uri: &str) -> Self {
        let gatekeeper = [
            from_sections(config, "GatekeeperURI"),
            config.get("GatekeeperService", "ExternalName"),
            config.get("GridService", "Gatekeeper"),
        ]
        .into_iter()
        .flatten()
        .find(|text| !text.is_empty())
        .unwrap_or(server_uri);
        let gatekeeper = match HttpHost::parse(gatekeeper) {
            Parsed::Valid(host) => Some(host),
            Parsed::Empty | Parsed::Invalid => None,
        };
        let aliases = from_sections(config, "GatekeeperURIAlias")
            .unwrap_or_default()
            .split(',')
            .filter_map(|alias| match HttpHost::parse(alias.trim()) {
                Parsed::Valid(host) => Some(host),
                Parsed::Empty | Parsed::Invalid => None,
            })
            .collect();
        Self {
            gatekeeper,
            aliases,
        }
    }

    /// `CheckUrl`: `None` for a URL that is not a valid host, otherwise
    /// whether it is this grid, and its host.
    fn check(&self, url: &str) -> Option<(bool, Option<HttpHost>)> {
        match HttpHost::parse(url) {
            Parsed::Empty => Some((true, None)),
            Parsed::Invalid => None,
            Parsed::Valid(host) => {
                let local = self.gatekeeper.as_ref() == Some(&host) || self.aliases.contains(&host);
                Some((local, Some(host)))
            }
        }
    }

    /// `GetUser`'s GridUser branch: a parsed identifier of this grid gives
    /// its first and last name, one of another grid `first.last` and
    /// `@host:port`. Anything else leaves the id unknown.
    pub fn name_from(&self, user_id: &str) -> Option<UserName> {
        let (url, first, last) = parse_universal_id(user_id)?;
        let (local, host) = self.check(&url)?;
        match (local, host) {
            (true, _) => Some(UserName::new(first, last)),
            (false, Some(host)) => Some(UserName::new(
                format!("{}.{}", first.replace(' ', "."), last.replace(' ', ".")),
                format!("@{}", host.host_and_port()),
            )),
            (false, None) => None,
        }
    }
}

/// `Util.ParseFullUniversalUserIdentifier(value, out uuid, out url, out
/// first, out last)`: `uuid;url;first last[;secret]`. The url is trimmed
/// and lower-cased.
fn parse_universal_id(value: &str) -> Option<(String, String, String)> {
    if value.encode_utf16().count() < 38 {
        return None;
    }
    Uuid::parse_str(value.get(..36)?).ok()?;
    let mut seps = value
        .char_indices()
        .skip(36)
        .filter(|(_, c)| *c == ';')
        .map(|(at, _)| at)
        .take(3);
    let first_sep = seps.next()?;
    let second_sep = seps.next()?;
    let third_sep = seps.next();
    let url = value[first_sep + 1..second_sep].trim().to_lowercase();
    let names_at = second_sep + 1;
    if names_at >= value.len() {
        return None;
    }
    let names = match third_sep {
        Some(end) => &value[names_at..end],
        None => &value[names_at..],
    };
    let mut parts = names.split(' ').filter(|part| !part.is_empty());
    let first = parts.next()?.to_owned();
    let last = parts.next().unwrap_or_default().to_owned();
    Some((url, first, last))
}

/// The account and GridUser services of one region.
pub struct GridNames {
    account_uri: String,
    scope: Uuid,
    grid_user: GridUserClient,
    home: GridHome,
}

impl GridNames {
    pub fn new(account_uri: &str, scope: Uuid, grid_user_uri: &str, home: GridHome) -> Arc<Self> {
        Arc::new(Self {
            account_uri: account_uri.to_owned(),
            scope,
            grid_user: GridUserClient::new(grid_user_uri),
            home,
        })
    }
}

impl NameSource for GridNames {
    fn account(&self, id: Uuid) -> NameFuture {
        let uri = self.account_uri.clone();
        let scope = self.scope;
        Box::pin(async move {
            crate::grid::account_name(&uri, scope, id)
                .await
                .map(|(first, last)| UserName::new(first, last))
        })
    }

    fn grid_user(&self, id: Uuid) -> NameFuture {
        let client = self.grid_user.clone();
        let home = self.home.clone();
        Box::pin(async move {
            let user_id = client.user_id(id).await.ok()??;
            home.name_from(&user_id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "11111111-2222-3333-4444-555555555555";

    fn home(gatekeeper: &str) -> GridHome {
        let config =
            Config::parse(&format!("[Hypergrid]\nGatekeeperURI = {gatekeeper}\n")).unwrap();
        GridHome::from_config(&config, "http://region.example:9000/")
    }

    #[test]
    fn a_resident_row_is_not_a_universal_id() {
        assert_eq!(home("http://grid.example:8002").name_from(ID), None);
    }

    #[test]
    fn this_grid_gives_first_and_last() {
        let home = home("http://grid.example:8002");
        assert_eq!(
            home.name_from(&format!("{ID};HTTP://Grid.Example:8002/;Ann Smith")),
            Some(UserName::new("Ann", "Smith"))
        );
        assert_eq!(
            home.name_from(&format!("{ID};;Ann Smith;secret")),
            Some(UserName::new("Ann", "Smith"))
        );
    }

    #[test]
    fn another_grid_gives_dotted_first_and_its_host() {
        assert_eq!(
            home("http://grid.example:8002")
                .name_from(&format!("{ID};https://other.example/;Bob Jones")),
            Some(UserName::new("Bob.Jones", "@other.example:443"))
        );
    }

    #[test]
    fn a_bad_url_or_name_leaves_the_id_unknown() {
        let home = home("http://grid.example:8002");
        for value in [
            format!("{ID};http://other.example:8002/path;Bob Jones"),
            format!("{ID};http://other.example:99999;Bob Jones"),
            format!("{ID};http://other.example:8002"),
            format!("{ID};http://other.example:8002;"),
            format!("{ID};http://other.example:8002;   "),
            "not-a-uuid-not-a-uuid-not-a-uuid-xx;;Bob Jones".to_string(),
        ] {
            assert_eq!(home.name_from(&value), None, "{value}");
        }
    }

    #[test]
    fn the_server_uri_and_aliases_are_this_grid_without_a_gatekeeper() {
        let config =
            Config::parse("[Startup]\nGatekeeperURIAlias = grid-alias.example:8002, bad host\n")
                .unwrap();
        let home = GridHome::from_config(&config, "http://region.example:9000/");
        assert_eq!(
            home.name_from(&format!("{ID};http://region.example:9000;Ann Smith")),
            Some(UserName::new("Ann", "Smith"))
        );
        assert_eq!(
            home.name_from(&format!("{ID};http://grid-alias.example:8002/;Ann Smith")),
            Some(UserName::new("Ann", "Smith"))
        );
        assert_eq!(home.aliases.len(), 1);
    }

    #[test]
    fn the_last_section_that_sets_the_gatekeeper_wins() {
        let config = Config::parse(
            "[Const]\nGatekeeperURI = http://first.example:1\n[Hypergrid]\nGatekeeperURI = http://last.example:2\n",
        )
        .unwrap();
        let home = GridHome::from_config(&config, "");
        assert_eq!(
            home.gatekeeper,
            Some(HttpHost {
                host: "last.example".into(),
                port: 2
            })
        );
    }
}
