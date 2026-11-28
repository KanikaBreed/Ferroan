// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Services.Interfaces/AppearanceChannels.cs
//   Source/OpenSim.Services.Interfaces/IAvatarService.cs (BakeIndexPrefix)
//   OpenSim-NGC/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4.
// This port remains available under MPL-2.0 and forms part of Ferroan's
// GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! The eleven viewer channel tokens and the avatar-index keys they name.

use uuid::Uuid;

/// Token as the viewer writes it, then the `Bake:<Index>` name.
const CHANNELS: &[(&str, &str)] = &[
    ("head", "Head"),
    ("upper", "Upper"),
    ("lower", "Lower"),
    ("eyes", "Eyes"),
    ("skirt", "Skirt"),
    ("hair", "Hair"),
    ("leftarm", "LeftArm"),
    ("leftleg", "LeftLeg"),
    ("aux1", "Aux1"),
    ("aux2", "Aux2"),
    ("aux3", "Aux3"),
];

/// Avatar-service key for a viewer channel token.
///
/// Matching is ASCII case-insensitive. A numeric token, an empty token and any
/// other spelling return `None`.
pub(crate) fn bake_key(token: &str) -> Option<String> {
    if token.is_empty() {
        return None;
    }
    CHANNELS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(token))
        .map(|(_, index)| format!("Bake:{index}"))
}

/// Hyphenated or 32-hex UUID text, with surrounding spaces ignored.
///
/// Braces and a `urn:uuid:` prefix are rejected. That is `UUID.TryParse` at
/// the pin, which is narrower than [`Uuid::parse_str`].
pub(crate) fn parse_wire_uuid(text: &str) -> Option<Uuid> {
    let text = text.trim();
    let bytes = text.as_bytes();
    let hyphenated = bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit());
    let raw = bytes.len() == 32 && bytes.iter().all(u8::is_ascii_hexdigit);
    if !hyphenated && !raw {
        return None;
    }
    Uuid::parse_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::{CHANNELS, bake_key, parse_wire_uuid};

    #[test]
    fn eleven_tokens_use_the_viewer_spelling() {
        assert_eq!(CHANNELS.len(), 11);
        assert_eq!(bake_key("HEAD").as_deref(), Some("Bake:Head"));
        assert_eq!(bake_key("leftarm").as_deref(), Some("Bake:LeftArm"));
        assert_eq!(bake_key("aux3").as_deref(), Some("Bake:Aux3"));
        assert_eq!(bake_key("torso"), None);
        assert_eq!(bake_key("0"), None);
        assert_eq!(bake_key("head-baked"), None);
        assert_eq!(bake_key(""), None);
    }

    #[test]
    fn wire_uuids_follow_the_pin_parser() {
        assert!(parse_wire_uuid("  B7D2FF2E-DC32-44D8-AA61-3D22070A4964 ").is_some());
        assert!(parse_wire_uuid("b7d2ff2edc3244d8aa613d22070a4964").is_some());
        assert!(parse_wire_uuid("{b7d2ff2e-dc32-44d8-aa61-3d22070a4964}").is_none());
        assert!(parse_wire_uuid("urn:uuid:b7d2ff2e-dc32-44d8-aa61-3d22070a4964").is_none());
        assert!(parse_wire_uuid("not-a-uuid").is_none());
    }
}
