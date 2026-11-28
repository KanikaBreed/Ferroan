// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Services.AvatarService/AppearanceService.cs (GetBake)
//   OpenSim-NGC/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4.
// This port remains available under MPL-2.0 and forms part of Ferroan's
// GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! Resolve one bake channel to asset bytes. This path only reads.

use uuid::Uuid;

use crate::asset::AssetHttp;
use crate::avatar::AvatarService;

use super::channels::{bake_key, parse_wire_uuid};

/// The asset bytes for this agent, channel and path uuid.
///
/// `None` is the handler's 404. A plugin error from either read is `None` as
/// well: the pin catches it and does not answer 500. The path uuid is compared
/// with the stored id. It is not the lookup key, and this function does not
/// write the avatar index.
pub(crate) async fn get_bake(
    avatars: &AvatarService,
    assets: &AssetHttp,
    agent: Uuid,
    channel: &str,
    expected: Uuid,
) -> Option<Vec<u8>> {
    if agent.is_nil() {
        return None;
    }
    let key = bake_key(channel)?;
    let data = avatars.get_avatar(agent).await.ok()?;
    let stored = data
        .fields
        .iter()
        .find(|(name, _)| name == &key)
        .map(|(_, value)| value.as_str())?;
    let stored_id = parse_wire_uuid(stored)?;
    if stored_id.is_nil() || stored_id != expected {
        return None;
    }
    assets.bake_bytes(&stored_id.to_string()).await
}
