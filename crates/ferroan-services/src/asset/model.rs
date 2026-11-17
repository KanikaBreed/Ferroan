// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The fields `AssetBase` stores and the asset HTTP handlers read.
//!
//! Name and description are truncated to 64 UTF-16 code units on store,
//! which is what `SQLiteAssetData.StoreAsset` does with `Substring`.

use uuid::Uuid;

pub const MAX_ASSET_NAME: usize = 64;
pub const MAX_ASSET_DESC: usize = 64;
pub const MAPTILE: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub type_code: i8,
    pub local: bool,
    pub temporary: bool,
    pub creator_id: String,
    pub flags: i32,
    pub data: Vec<u8>,
    pub upload_attempts: i32,
}

impl Asset {
    pub fn stored_name(&self) -> String {
        truncate_utf16(&self.name, MAX_ASSET_NAME)
    }

    pub fn stored_description(&self) -> String {
        truncate_utf16(&self.description, MAX_ASSET_DESC)
    }
}

fn truncate_utf16(text: &str, units: usize) -> String {
    let mut count = 0;
    let mut end = 0;
    for ch in text.chars() {
        let width = ch.len_utf16();
        if count + width > units {
            break;
        }
        count += width;
        end += ch.len_utf8();
    }
    text[..end].to_string()
}

/// Hyphenated or 32-hex UUID text, with surrounding spaces ignored.
pub fn parse_id(text: &str) -> Option<Uuid> {
    Uuid::parse_str(text.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::truncate_utf16;

    #[test]
    fn truncation_counts_utf16_units_and_keeps_char_boundaries() {
        let ascii = "n".repeat(80);
        assert_eq!(truncate_utf16(&ascii, 64).chars().count(), 64);
        let emoji = format!("{}{}", "a".repeat(63), "😀");
        let stored = truncate_utf16(&emoji, 64);
        assert_eq!(stored, "a".repeat(63));
        assert!(stored.is_char_boundary(stored.len()));
    }
}
