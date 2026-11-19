// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `UserAccount` and `ToKeyValuePairs`.
//!
//! A field that is still null is omitted. `BuildXmlResponse` skips null, and
//! a freshly constructed account leaves `UserTitle`, `UserCountry` and
//! `DisplayName` null. A row read back from SQLite fills those with empty
//! strings, so a later get includes them. `LocalToGrid` is not a column and
//! is always true after a read.

use ferroan_http::{Field, FieldMap};
use uuid::Uuid;

use super::urls::service_urls_http;

pub const ZERO_ID: &str = "00000000-0000-0000-0000-000000000000";
pub const GOD_ID: &str = "6571e388-6218-4574-87db-f9379718315e";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub principal_id: Uuid,
    pub scope_id: Uuid,
    pub user_level: i32,
    pub user_flags: i32,
    /// `None` is the C# null that XML omits. `Some("")` is an empty element.
    pub user_title: Option<String>,
    pub user_country: Option<String>,
    pub local_to_grid: bool,
    pub service_urls: Vec<(String, String)>,
    pub created: i32,
    pub display_name: Option<String>,
    pub name_changed: u32,
}

impl Account {
    pub fn to_fields(&self) -> FieldMap {
        let mut fields = FieldMap::new();
        fields.insert("FirstName".to_string(), Field::Str(self.first_name.clone()));
        fields.insert("LastName".to_string(), Field::Str(self.last_name.clone()));
        fields.insert("Email".to_string(), Field::Str(self.email.clone()));
        fields.insert(
            "PrincipalID".to_string(),
            Field::Str(self.principal_id.to_string()),
        );
        fields.insert("ScopeID".to_string(), Field::Str(self.scope_id.to_string()));
        fields.insert("Created".to_string(), Field::Str(self.created.to_string()));
        fields.insert(
            "UserLevel".to_string(),
            Field::Str(self.user_level.to_string()),
        );
        fields.insert(
            "UserFlags".to_string(),
            Field::Str(self.user_flags.to_string()),
        );
        insert_optional(&mut fields, "UserTitle", &self.user_title);
        insert_optional(&mut fields, "UserCountry", &self.user_country);
        fields.insert(
            "LocalToGrid".to_string(),
            Field::Str(if self.local_to_grid { "True" } else { "False" }.to_string()),
        );
        insert_optional(&mut fields, "DisplayName", &self.display_name);
        fields.insert(
            "NameChanged".to_string(),
            Field::Str(self.name_changed.to_string()),
        );
        fields.insert(
            "ServiceURLs".to_string(),
            Field::Str(service_urls_http(&self.service_urls)),
        );
        fields
    }

    /// `UserAccount.FormattedName`.
    pub fn formatted_name(&self) -> String {
        let resident = self.last_name.eq_ignore_ascii_case("resident");
        match self.display_name.as_deref() {
            Some(name) if !name.trim().is_empty() => {
                let username = if resident {
                    self.first_name.clone()
                } else {
                    format!("{}.{}", self.first_name, self.last_name)
                };
                format!("{name} ({})", username.to_ascii_lowercase())
            }
            _ => {
                if resident {
                    self.first_name.clone()
                } else {
                    format!("{} {}", self.first_name, self.last_name)
                }
            }
        }
    }
}

fn insert_optional(fields: &mut FieldMap, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        fields.insert(key.to_string(), Field::Str(value.clone()));
    }
}
