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
//   Source/OpenSim.Region.ClientStack.LindenCaps/WebFetchInvDescModule.cs
//   Source/OpenSim.Region.ClientStack.LindenCaps/FetchLibDescModule.cs
//   Source/OpenSim.Region.ClientStack.LindenCaps/FetchInventory2Module.cs
//   Source/OpenSim.Region.ClientStack.LindenCaps/GetAssetsModule.cs
//   Source/OpenSim.Region.ClientStack.LindenCaps/BunchOfCaps/BunchOfCapsModule.cs
//   Source/OpenSim.Region.ClientStack.LindenCaps/BunchOfCaps/BunchOfCaps.cs
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Which capability names a region grants, following each module's Initialise
//! and RegisterCaps. The seed still returns a name only when the client asks.

use ferroan_client::{CapGrantSet, CapTarget};
use ferroan_config::Config;

const SECTION: &str = "ClientStack.LindenCaps";

pub fn cap_grants(config: &Config) -> CapGrantSet {
    let mut grants = CapGrantSet::default();
    // BunchOfCaps registers this on every region whose config exists. The
    // seed omits it unless the client lists the name.
    grants.grant("NewFileAgentInventory", CapTarget::Local);
    if !config.section_names().any(|name| name == SECTION) {
        return grants;
    }
    let descendents = cap_setting(config, "Cap_FetchInventoryDescendents2", "");
    if !descendents.is_empty() {
        local_or_external(&mut grants, "FetchInventoryDescendents2", descendents);
    }
    let library_descendents = cap_setting(config, "Cap_FetchLibDescendents2", "");
    if !library_descendents.is_empty() {
        local_or_external(&mut grants, "FetchLibDescendents2", library_descendents);
    }
    // RegisterCaps calls the local helper only. The helper's external branch
    // is unreachable, so a configured external URL is not granted.
    let fetch_items = cap_setting(config, "Cap_FetchInventory2", "");
    if fetch_items == "localhost" {
        grants.grant("FetchInventory2", CapTarget::Local);
        if cap_setting(config, "Cap_FetchLib2", "localhost") == "localhost" {
            grants.grant("FetchLib2", CapTarget::Local);
        }
    }
    let texture = cap_setting(config, "Cap_GetTexture", "");
    let mesh = cap_setting(config, "Cap_GetMesh", "");
    let mesh2 = cap_setting(config, "Cap_GetMesh2", "");
    let asset = cap_setting(config, "Cap_GetAsset", "");
    if texture.is_empty() && mesh.is_empty() && mesh2.is_empty() && asset.is_empty() {
        return grants;
    }
    // GetTexture's external branch registers the string even when it is empty,
    // once another asset cap has enabled the module.
    if texture == "localhost" {
        grants.grant("GetTexture", CapTarget::Local);
    } else {
        grants.grant("GetTexture", CapTarget::External(texture.to_owned()));
    }
    optional_asset(&mut grants, "GetMesh", mesh);
    optional_asset(&mut grants, "GetMesh2", mesh2);
    optional_asset(&mut grants, "ViewerAsset", asset);
    grants
}

fn cap_setting<'a>(config: &'a Config, key: &str, default: &'a str) -> &'a str {
    config.get(SECTION, key).unwrap_or(default)
}

fn local_or_external(grants: &mut CapGrantSet, name: &str, url: &str) {
    if url == "localhost" {
        grants.grant(name, CapTarget::Local);
    } else {
        grants.grant(name, CapTarget::External(url.to_owned()));
    }
}

fn optional_asset(grants: &mut CapGrantSet, name: &str, url: &str) {
    if url == "localhost" {
        grants.grant(name, CapTarget::Local);
    } else if !url.is_empty() {
        grants.grant(name, CapTarget::External(url.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grants(text: &str) -> CapGrantSet {
        cap_grants(&Config::parse(text).unwrap())
    }

    #[test]
    fn a_missing_section_grants_only_the_upload_cap() {
        let grants = grants("[Startup]\n");
        assert!(grants.contains("NewFileAgentInventory"));
        assert!(!grants.contains("FetchInventoryDescendents2"));
        assert!(!grants.contains("GetTexture"));
    }

    #[test]
    fn shipped_defaults_are_local_handlers() {
        let text = "\
[ClientStack.LindenCaps]
Cap_FetchInventoryDescendents2 = localhost
Cap_FetchInventory2 = localhost
Cap_FetchLibDescendents2 = localhost
Cap_FetchLib2 = localhost
Cap_GetTexture = localhost
Cap_GetMesh = localhost
Cap_GetMesh2 = localhost
Cap_GetAsset = localhost
";
        let grants = grants(text);
        for name in [
            "FetchInventoryDescendents2",
            "FetchInventory2",
            "FetchLibDescendents2",
            "FetchLib2",
            "GetTexture",
            "GetMesh",
            "GetMesh2",
            "ViewerAsset",
            "NewFileAgentInventory",
        ] {
            assert!(grants.contains(name), "{name}");
        }
    }

    #[test]
    fn an_external_fetch_inventory2_url_is_not_registered() {
        let grants = grants(
            "[ClientStack.LindenCaps]\nCap_FetchInventory2 = http://assets.example/inv\nCap_FetchLib2 = localhost\n",
        );
        assert!(!grants.contains("FetchInventory2"));
        assert!(!grants.contains("FetchLib2"));
    }

    #[test]
    fn fetch_lib2_waits_for_fetch_inventory2() {
        let grants = grants("[ClientStack.LindenCaps]\nCap_FetchLib2 = localhost\n");
        assert!(!grants.contains("FetchLib2"));
    }

    #[test]
    fn an_empty_get_texture_is_registered_when_another_asset_cap_enables_the_module() {
        let grants = grants("[ClientStack.LindenCaps]\nCap_GetMesh = localhost\n");
        assert!(grants.contains("GetMesh"));
        assert!(grants.contains("GetTexture"));
        assert!(!grants.contains("ViewerAsset"));
    }
}
