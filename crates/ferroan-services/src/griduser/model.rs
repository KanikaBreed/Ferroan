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
//   Source/OpenSim.Data/IGridUserData.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! One `GridUser` row.
//!
//! `UserID` is the key. The other nine columns are the `Data` dictionary on
//! `GridUserData`. A missing key is omitted on store: SQLite and MySQL
//! `REPLACE` then apply the column default, and PostgreSQL leaves the old
//! value in place.

use std::collections::BTreeMap;

/// How many ids one `GetMany` statement carries. `GridUserBatch.Size`.
pub const BATCH_SIZE: usize = 500;

/// `true.ToString()` in the C# plugins. The column default is `'false'`.
pub const ONLINE_TRUE: &str = "True";

/// `LIKE` escape. The same character on SQLite, MySQL and PostgreSQL.
pub const LIKE_ESCAPE: char = '!';

/// Columns other than `UserID`, in schema order.
pub const DATA_COLUMNS: [&str; 9] = [
    "HomeRegionID",
    "HomePosition",
    "HomeLookAt",
    "LastRegionID",
    "LastPosition",
    "LastLookAt",
    "Online",
    "Login",
    "Logout",
];

/// Region columns are `uuid` on PostgreSQL and `char(36)` on the other two.
pub const REGION_COLUMNS: [&str; 2] = ["HomeRegionID", "LastRegionID"];

/// Default realm. PostgreSQL folds the unquoted name to `griduser`.
pub const DEFAULT_REALM: &str = "GridUser";

/// `character(n)` widths. PostgreSQL reads come back padded to these widths,
/// which is what Npgsql returns. `::text` strips the pad, and the store puts
/// it back. MySQL's connector trims, and SQLite does not pad.
const CHAR_WIDTH: &[(&str, usize)] = &[
    ("HomePosition", 64),
    ("HomeLookAt", 64),
    ("LastPosition", 64),
    ("LastLookAt", 64),
    ("Online", 5),
    ("Login", 16),
    ("Logout", 16),
];

/// The text a PostgreSQL read returns for this column. Other engines keep
/// `value` unchanged.
pub(crate) fn postgres_text(name: &str, value: &str) -> String {
    let Some(width) = CHAR_WIDTH
        .iter()
        .find(|(column, _)| *column == name)
        .map(|(_, width)| *width)
    else {
        return value.to_string();
    };
    if value.len() >= width {
        return value.to_string();
    }
    let mut text = String::with_capacity(width);
    text.push_str(value);
    text.extend(std::iter::repeat_n(' ', width - value.len()));
    text
}

/// A grid-user row. `data` holds only columns that are present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridUser {
    pub user_id: String,
    pub data: BTreeMap<String, String>,
}

impl GridUser {
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            data: BTreeMap::new(),
        }
    }

    pub fn field(&self, name: &str) -> Option<&str> {
        self.data.get(name).map(String::as_str)
    }
}

/// A literal prefix pattern: escape `!`, `%` and `_`, then append `%`.
pub fn like_prefix(id: &str) -> String {
    let mut out = String::with_capacity(id.len().saturating_add(1));
    for ch in id.chars() {
        if ch == LIKE_ESCAPE || ch == '%' || ch == '_' {
            out.push(LIKE_ESCAPE);
        }
        out.push(ch);
    }
    out.push('%');
    out
}

pub(crate) fn is_region(name: &str) -> bool {
    REGION_COLUMNS.contains(&name)
}
