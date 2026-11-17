// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `SLUtil.SLAssetTypeToContentType` at Tranquillity
//! `713857f10b8e91a639a967d4354a4682b080793c`.
//!
//! The first mapping for a type wins. A type with no mapping uses the
//! unknown type's content type, `application/octet-stream`.

/// Content type for an asset type code.
pub fn content_type(type_code: i8) -> &'static str {
    match type_code {
        -2 | 57 => "application/llsd+xml",
        0 => "image/x-j2c",
        1 => "audio/ogg",
        2 => "application/vnd.ll.callingcard",
        3 => "application/vnd.ll.landmark",
        5 => "application/vnd.ll.clothing",
        6 => "application/vnd.ll.primitive",
        7 => "application/vnd.ll.notecard",
        8 => "application/vnd.ll.folder",
        10 => "application/vnd.ll.lsltext",
        11 => "application/vnd.ll.lslbyte",
        12 | 18 => "image/tga",
        13 => "application/vnd.ll.bodypart",
        17 => "audio/x-wav",
        19 => "image/jpeg",
        20 => "application/vnd.ll.animation",
        21 => "application/vnd.ll.gesture",
        22 => "application/x-metaverse-simstate",
        24 => "application/vnd.ll.link",
        25 => "application/vnd.ll.linkfolder",
        49 => "application/vnd.ll.mesh",
        _ => "application/octet-stream",
    }
}

/// `SLUtil.ContentTypeToSLAssetType`, first mapping wins.
///
/// `None` means the setter leaves `Type` alone: the content type is unknown,
/// or it maps to `AssetType.Unknown` (`-1`), which that setter ignores.
pub fn asset_type_for_content(content: &str) -> Option<i8> {
    let type_code = match content {
        "image/x-j2c" | "image/jp2" => 0,
        "image/tga" => 12,
        "image/jpeg" => 19,
        "audio/ogg" | "application/ogg" => 1,
        "audio/x-wav" => 17,
        "application/vnd.ll.callingcard" | "application/x-metaverse-callingcard" => 2,
        "application/vnd.ll.landmark" | "application/x-metaverse-landmark" => 3,
        "application/vnd.ll.clothing" | "application/x-metaverse-clothing" => 5,
        "application/vnd.ll.primitive" | "application/x-metaverse-primitive" => 6,
        "application/vnd.ll.notecard" | "application/x-metaverse-notecard" => 7,
        "application/vnd.ll.lsltext" | "application/x-metaverse-lsl" => 10,
        "application/vnd.ll.lslbyte" | "application/x-metaverse-lso" => 11,
        "application/vnd.ll.bodypart" | "application/x-metaverse-bodypart" => 13,
        "application/vnd.ll.animation" | "application/x-metaverse-animation" => 20,
        "application/vnd.ll.gesture" | "application/x-metaverse-gesture" => 21,
        "application/x-metaverse-simstate" => 22,
        "application/vnd.ll.link" => 24,
        "application/vnd.ll.linkfolder" => 25,
        "application/vnd.ll.mesh" => 49,
        "application/llsd+xml" => 57,
        "application/vnd.ll.folder"
        | "application/vnd.ll.rootfolder"
        | "application/vnd.ll.trashfolder"
        | "application/vnd.ll.snapshotfolder"
        | "application/vnd.ll.lostandfoundfolder"
        | "application/vnd.ll.favoritefolder"
        | "application/vnd.ll.currentoutfitfolder"
        | "application/vnd.ll.outfitfolder"
        | "application/vnd.ll.myoutfitsfolder" => 8,
        _ => return None,
    };
    Some(type_code)
}
