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
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF
// THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Services.FSAssetService/FSAssetService.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `show assets`, `show digest`, `delete asset`, `import` and `force import`.
//!
//! `FSAssetService` registers these on the first main instance only. The
//! remote console does not call them yet. Each function is the text that
//! handler prints.

use super::commands::asset_flags_text;
use super::content_type::content_type;
use super::fsfiles::hash_to_file;

/// One command `FSAssetConnector` adds when this process's first asset
/// service is the main instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsConsoleCommand {
    pub category: &'static str,
    pub name: &'static str,
    pub help: &'static str,
    pub description: &'static str,
}

const FS_COMMANDS: &[FsConsoleCommand] = &[
    FsConsoleCommand {
        category: "fs",
        name: "show assets",
        help: "show assets",
        description: "Show asset stats",
    },
    FsConsoleCommand {
        category: "fs",
        name: "show digest",
        help: "show digest <ID>",
        description: "Show asset digest",
    },
    FsConsoleCommand {
        category: "fs",
        name: "delete asset",
        help: "delete asset <ID>",
        description: "Delete asset from database",
    },
    FsConsoleCommand {
        category: "fs",
        name: "import",
        help: "import <conn> <table> [<start> <count>]",
        description: "Import legacy assets",
    },
    FsConsoleCommand {
        category: "fs",
        name: "force import",
        help: "force import <conn> <table> [<start> <count>]",
        description: "Import legacy assets, overwriting current content",
    },
];

/// The commands the main instance registers. A secondary instance registers none.
pub fn registered_fs_commands(main: bool) -> &'static [FsConsoleCommand] {
    if main { FS_COMMANDS } else { &[] }
}

/// `HandleShowAssets`.
pub fn show_assets_lines(count: i64) -> Vec<String> {
    vec![format!("Total asset count: {count}")]
}

/// What `HandleShowDigest` prints after `Get`. Empty `data` is not found.
pub struct FsDigest<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub type_code: i8,
    pub flags: i32,
    pub hash: &'a str,
    pub osgrid: bool,
    pub data: &'a [u8],
}

/// `HandleShowDigest`. `None` is a miss or empty bytes.
pub fn show_digest_lines(args: &[&str], asset: Option<&FsDigest<'_>>) -> Vec<String> {
    if args.len() < 3 {
        return vec!["Syntax: show digest <ID>".to_string()];
    }
    let Some(asset) = asset.filter(|asset| !asset.data.is_empty()) else {
        return vec!["Asset not found".to_string()];
    };
    let path = hash_to_file(asset.hash, asset.osgrid);
    let mut lines = vec![
        format!("Name: {}", asset.name),
        format!("Description: {}", asset.description),
        format!("Type: {}", asset.type_code),
        format!("Content-type: {}", content_type(asset.type_code)),
        format!("Flags: {}", asset_flags_text(asset.flags)),
        format!("FS file: {}", path.display()),
    ];
    for index in 0..5 {
        let offset = index * 16;
        if asset.data.len() <= offset {
            break;
        }
        let end = (offset + 16).min(asset.data.len());
        let text = asset.data[offset..end]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join("-");
        lines.push(format!("{offset:04x}: {text}"));
    }
    lines
}

/// `HandleDeleteAsset` once the caller knows whether the row exists.
/// `syntax` is `args` shorter than three tokens.
pub fn delete_fs_asset_lines(args: &[&str], found: bool) -> Vec<String> {
    if args.len() < 3 {
        return vec!["Syntax: delete asset <ID>".to_string()];
    }
    if found {
        vec!["Asset deleted".to_string()]
    } else {
        vec!["Asset not found".to_string()]
    }
}

/// Parsed `import` / `force import`. `count == -1` is the rest of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsImportRequest {
    pub connection: String,
    pub table: String,
    pub start: i32,
    pub count: i32,
    pub force: bool,
}

/// `HandleImportAssets` before it opens the database.
#[derive(Debug, PartialEq, Eq)]
pub enum FsImportParse {
    Syntax,
    BadInteger,
    Ready(FsImportRequest),
}

const IMPORT_SYNTAX: &str = "Syntax: import <conn> <table> [<start> <count>]";

/// The same syntax line for `import` and `force import`.
pub fn import_syntax_line() -> String {
    IMPORT_SYNTAX.to_string()
}

/// `Convert.ToInt32` that does not leave the process. Spaces are ignored.
/// A value that does not fit in `i32` is refused.
pub fn parse_fs_import(args: &[&str]) -> FsImportParse {
    if args.is_empty() {
        return FsImportParse::Syntax;
    }
    let mut rest = args.to_vec();
    let mut force = false;
    if rest[0] == "force" {
        force = true;
        rest.remove(0);
    }
    if rest.len() < 3 {
        return FsImportParse::Syntax;
    }
    let mut start = 0i32;
    let mut count = -1i32;
    if rest.len() > 3 {
        let Some(value) = parse_console_i32(rest[3]) else {
            return FsImportParse::BadInteger;
        };
        start = value;
    }
    if rest.len() > 4 {
        let Some(value) = parse_console_i32(rest[4]) else {
            return FsImportParse::BadInteger;
        };
        count = value;
    }
    FsImportRequest {
        connection: rest[1].to_string(),
        table: rest[2].to_string(),
        start,
        count,
        force,
    }
    .into()
}

impl From<FsImportRequest> for FsImportParse {
    fn from(request: FsImportRequest) -> Self {
        Self::Ready(request)
    }
}

fn parse_console_i32(text: &str) -> Option<i32> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    text.parse().ok()
}

/// The lines `Import` prints for `imported` stored rows, including the
/// `0`, `100`, `200`, ... line before each of those stores.
#[cfg(test)]
pub(super) fn import_done_lines(imported: u64) -> Vec<String> {
    let mut lines = vec!["Querying database".to_string(), "Reading data".to_string()];
    let mut seen = 0u64;
    while seen < imported {
        if seen.is_multiple_of(100) {
            lines.push(format!("{seen} assets imported so far"));
        }
        seen += 1;
    }
    lines.push(format!("Import done, {imported} assets imported"));
    lines
}

/// Drop ADO pool keys. Ferroan opens its own short-lived pool and closes it.
/// A sqlx URL is returned unchanged. The string is not logged.
pub fn without_ado_pool_keys(connection: &str) -> String {
    if connection_is_sqlx_url(connection) {
        return connection.to_string();
    }
    let mut out = String::new();
    let mut rest = connection;
    while !rest.is_empty() {
        let (pair, next) = split_connection_pair(rest);
        rest = next;
        let key = pair.split('=').next().unwrap_or("").trim();
        let normalized: String = key
            .chars()
            .filter(|ch| !ch.is_whitespace() && *ch != '_' && *ch != '-')
            .flat_map(|ch| ch.to_lowercase())
            .collect();
        if normalized == "pooling" || normalized == "maximumpoolsize" {
            continue;
        }
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(pair);
    }
    out
}

pub(crate) fn connection_is_sqlx_url(connection: &str) -> bool {
    let scheme = connection
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .unwrap_or("");
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "mysql" | "postgres" | "postgresql"
    )
}

fn split_connection_pair(input: &str) -> (&str, &str) {
    let mut quote = None;
    for (index, ch) in input.char_indices() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == ';' => return (&input[..index], &input[index + 1..]),
            None => {}
        }
    }
    (input, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_instance_registers_the_five_commands() {
        assert!(registered_fs_commands(false).is_empty());
        let commands = registered_fs_commands(true);
        assert_eq!(commands.len(), 5);
        assert!(commands.iter().all(|command| command.category == "fs"));
        assert_eq!(
            commands
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            vec![
                "show assets",
                "show digest",
                "delete asset",
                "import",
                "force import"
            ]
        );
        assert_eq!(commands[3].help, "import <conn> <table> [<start> <count>]");
        assert_eq!(
            commands[4].description,
            "Import legacy assets, overwriting current content"
        );
    }

    #[test]
    fn show_assets_prints_the_count() {
        assert_eq!(show_assets_lines(0), vec!["Total asset count: 0"]);
        assert_eq!(show_assets_lines(12), vec!["Total asset count: 12"]);
    }

    #[test]
    fn show_digest_matches_the_pin_lines() {
        assert_eq!(
            show_digest_lines(&["show", "digest"], None),
            vec!["Syntax: show digest <ID>"]
        );
        assert_eq!(
            show_digest_lines(&["show", "digest", "abc"], None),
            vec!["Asset not found"]
        );
        let empty = FsDigest {
            name: "n",
            description: "d",
            type_code: 0,
            flags: 0,
            hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            osgrid: false,
            data: b"",
        };
        assert_eq!(
            show_digest_lines(&["show", "digest", "abc"], Some(&empty)),
            vec!["Asset not found"]
        );
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let data: Vec<u8> = (0..17).collect();
        let digest = FsDigest {
            name: "prim",
            description: "a mesh",
            type_code: 6,
            flags: 3,
            hash,
            osgrid: false,
            data: &data,
        };
        let lines = show_digest_lines(&["show", "digest", "abc"], Some(&digest));
        assert_eq!(
            lines,
            vec![
                "Name: prim".to_string(),
                "Description: a mesh".to_string(),
                "Type: 6".to_string(),
                "Content-type: application/vnd.ll.primitive".to_string(),
                "Flags: Maptile, Rewritable".to_string(),
                format!("FS file: {}", hash_to_file(hash, false).display()),
                "0000: 00-01-02-03-04-05-06-07-08-09-0A-0B-0C-0D-0E-0F".to_string(),
                "0010: 10".to_string(),
            ]
        );
        let osgrid = FsDigest {
            osgrid: true,
            ..digest
        };
        let os_lines = show_digest_lines(&["show", "digest", "abc"], Some(&osgrid));
        assert_eq!(
            os_lines[5],
            format!("FS file: {}", hash_to_file(hash, true).display())
        );
    }

    #[test]
    fn delete_asset_lines_follow_the_row() {
        assert_eq!(
            delete_fs_asset_lines(&["delete", "asset"], false),
            vec!["Syntax: delete asset <ID>"]
        );
        assert_eq!(
            delete_fs_asset_lines(&["delete", "asset", "nope"], false),
            vec!["Asset not found"]
        );
        assert_eq!(
            delete_fs_asset_lines(&["delete", "asset", "id"], true),
            vec!["Asset deleted"]
        );
    }

    #[test]
    fn import_parse_drops_force_and_refuses_a_bad_integer() {
        assert_eq!(parse_fs_import(&[]), FsImportParse::Syntax);
        assert_eq!(parse_fs_import(&["import", "c"]), FsImportParse::Syntax);
        assert_eq!(parse_fs_import(&["force", "import"]), FsImportParse::Syntax);
        assert_eq!(
            parse_fs_import(&["import", "conn", "assets", "nope"]),
            FsImportParse::BadInteger
        );
        assert_eq!(
            parse_fs_import(&["import", "conn", "assets", "1", "lots"]),
            FsImportParse::BadInteger
        );
        assert_eq!(
            parse_fs_import(&["import", "conn", "assets", "1", "2147483648"]),
            FsImportParse::BadInteger
        );
        match parse_fs_import(&["force", "import", "conn", "assets", " 2 ", "3"]) {
            FsImportParse::Ready(request) => {
                assert!(request.force);
                assert_eq!(request.connection, "conn");
                assert_eq!(request.table, "assets");
                assert_eq!(request.start, 2);
                assert_eq!(request.count, 3);
            }
            other => panic!("ready, got {other:?}"),
        }
        match parse_fs_import(&["import", "conn", "assets"]) {
            FsImportParse::Ready(request) => {
                assert!(!request.force);
                assert_eq!(request.start, 0);
                assert_eq!(request.count, -1);
            }
            other => panic!("ready, got {other:?}"),
        }
    }

    #[test]
    fn import_progress_prints_before_each_hundred_including_zero() {
        assert_eq!(
            import_done_lines(0),
            vec![
                "Querying database",
                "Reading data",
                "Import done, 0 assets imported"
            ]
        );
        assert_eq!(
            import_done_lines(1),
            vec![
                "Querying database",
                "Reading data",
                "0 assets imported so far",
                "Import done, 1 assets imported"
            ]
        );
        let hundred = import_done_lines(100);
        assert_eq!(
            hundred
                .iter()
                .filter(|line| line.contains("so far"))
                .count(),
            1
        );
        assert_eq!(hundred[2], "0 assets imported so far");
        assert_eq!(
            hundred.last().map(String::as_str),
            Some("Import done, 100 assets imported")
        );
        let next = import_done_lines(101);
        assert!(next.iter().any(|line| line == "100 assets imported so far"));
    }

    #[test]
    fn ado_pool_keys_are_dropped_and_a_url_is_kept() {
        let stripped = without_ado_pool_keys(
            "Server=127.0.0.1;Database=ferroan;User ID=ferroan;Password=x;Pooling=false",
        );
        assert!(!stripped.to_ascii_lowercase().contains("pooling"));
        assert!(stripped.contains("Password=x"));
        assert!(stripped.contains("Server=127.0.0.1"));
        let url = "mysql://ferroan:x@127.0.0.1:3306/ferroan";
        assert_eq!(without_ado_pool_keys(url), url);
    }
}
