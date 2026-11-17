// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use indexmap::IndexMap;

/// One `Dictionary<string, object>` entry from `ServerUtils`.
///
/// Insertion order is the order Tranquillity's dictionary enumerates, which
/// is the order keys were added.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Str(String),
    List(Vec<String>),
    Map(FieldMap),
    /// `BuildXmlResponse` skips a null. `BuildQueryString` throws on one.
    Null,
    Bool(bool),
    Int(i32),
    Uuid(String),
}

pub type FieldMap = IndexMap<String, Field>;

impl Field {
    pub(crate) fn csharp_text(&self) -> Option<String> {
        match self {
            Self::Str(text) => Some(text.clone()),
            Self::Bool(true) => Some("True".to_string()),
            Self::Bool(false) => Some("False".to_string()),
            Self::Int(number) => Some(number.to_string()),
            Self::Uuid(text) => Some(text.clone()),
            Self::List(_) | Self::Map(_) | Self::Null => None,
        }
    }
}
