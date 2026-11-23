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
//   Source/OpenSim.Services.UserAccountService/GridUserService.cs
//   Source/OpenSim.Framework.Console/ConsoleDisplayList.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `show grid user` and `show grid users online`.
//!
//! This process has no console prompt. The functions take the argv the C#
//! handlers see. `show grid user` needs four words, and the id is the fourth.
//! It prints each matching row, then `Entries`. `show grid users online`
//! counts rows that are online and whose login is less than five whole days
//! old. The text is the stored row, not the grid-user document.

use super::model::{DATA_COLUMNS, GridUser};
use super::service::{GridClock, GridData, GridUserService};
use super::wire;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    pub text: String,
}

impl<D, C> GridUserService<D, C>
where
    D: GridData,
    C: GridClock,
{
    pub async fn command(&self, argv: &[&str]) -> CommandOutcome {
        match argv {
            ["show", "grid", "user", ..] => self.show_user(argv).await,
            ["show", "grid", "users", "online", ..] => self.show_online().await,
            _ => CommandOutcome {
                text: String::new(),
            },
        }
    }

    async fn show_user(&self, argv: &[&str]) -> CommandOutcome {
        if argv.len() != 4 {
            return CommandOutcome {
                text: "Usage: show grid user <UUID>\n".to_string(),
            };
        }
        let Ok(rows) = self.data().get_all(argv[3]).await else {
            return CommandOutcome {
                text: "grid user command failed\n".to_string(),
            };
        };
        let mut text = String::new();
        for row in &rows {
            text.push_str(&display_row(row));
        }
        text.push_str(&format!("Entries: {}\n", rows.len()));
        CommandOutcome { text }
    }

    async fn show_online(&self) -> CommandOutcome {
        let Ok(rows) = self.data().get_all("").await else {
            return CommandOutcome {
                text: "grid user command failed\n".to_string(),
            };
        };
        let now = self.clock().unix_time();
        let mut count = 0_usize;
        for row in &rows {
            if !online_true(row) {
                continue;
            }
            let Some(login) = row.field("Login").and_then(wire::parse_i32) else {
                continue;
            };
            if within_five_days(now, login) {
                count = count.saturating_add(1);
            }
        }
        CommandOutcome {
            text: format!("Users online: {count}\n"),
        }
    }
}

fn online_true(row: &GridUser) -> bool {
    row.field("Online").and_then(wire::parse_bool) == Some(true)
}

fn within_five_days(now: i32, login: i32) -> bool {
    (i64::from(now) - i64::from(login)) / 86_400 < 5
}

fn display_row(row: &GridUser) -> String {
    let mut pairs = vec![("User ID".to_string(), row.user_id.clone())];
    for name in DATA_COLUMNS {
        if let Some(value) = row.field(name) {
            pairs.push((name.to_string(), value.to_string()));
        }
    }
    let width = pairs
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0);
    let mut text = String::new();
    for (key, value) in pairs {
        let pad = width.saturating_sub(key.chars().count());
        text.push_str(&key);
        text.extend(std::iter::repeat_n(' ', pad));
        text.push_str(" : ");
        text.push_str(&value);
        text.push('\n');
    }
    text.push('\n');
    text
}
