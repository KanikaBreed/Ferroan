// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `UserAlias.ToKeyValuePairs`.
//!
//! `Description` null is omitted. An empty string is an empty element.
//! `Id` is not on the wire.

use ferroan_http::{Field, FieldMap};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub alias_id: Uuid,
    pub user_id: Uuid,
    pub description: Option<String>,
}

impl Alias {
    pub fn to_fields(&self) -> FieldMap {
        let mut fields = FieldMap::new();
        fields.insert("AliasID".to_string(), Field::Str(self.alias_id.to_string()));
        fields.insert("UserID".to_string(), Field::Str(self.user_id.to_string()));
        if let Some(description) = &self.description {
            fields.insert("Description".to_string(), Field::Str(description.clone()));
        }
        fields
    }
}
