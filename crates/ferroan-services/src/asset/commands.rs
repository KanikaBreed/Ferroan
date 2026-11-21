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
//   Source/OpenSim.Server.Handlers/Asset/AssetServerConnector.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// The remote console is not wired. Each command is the function the handler
// calls, and the lines match that handler's output.

//! `show asset`, `dump asset` and `delete asset`.
//!
//! These are the command bodies. Nothing in `ferroan-robust` registers them
//! on the remote console yet.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::content_type::content_type;

/// One asset `show asset` prints. Empty `data` is "Asset not found", as the
/// handler treats `Data.Length == 0`.
pub struct ShownAsset {
    pub name: String,
    pub description: String,
    pub type_code: i8,
    pub temporary: bool,
    pub flags: i32,
    pub data: Vec<u8>,
}

/// The id and name `delete asset` prints after a row with bytes was found.
pub struct DeletedAsset {
    pub id: String,
    pub name: String,
}

/// `HandleShowAsset`. `None` is a missing row.
pub fn show_asset(args: &[&str], asset: Option<&ShownAsset>) -> Vec<String> {
    if args.len() < 3 {
        return vec!["Syntax: show asset <ID>".to_string()];
    }
    let Some(asset) = asset.filter(|asset| !asset.data.is_empty()) else {
        return vec!["Asset not found".to_string()];
    };
    let mut lines = vec![
        format!("Name: {}", asset.name),
        format!("Description: {}", asset.description),
        format!(
            "Type: {} (type number = {})",
            asset_type_name(asset.type_code),
            asset.type_code
        ),
        format!("Content-type: {}", content_type(asset.type_code)),
        format!("Size: {} bytes", asset.data.len()),
        format!("Temporary: {}", if asset.temporary { "yes" } else { "no" }),
        format!("Flags: {}", asset_flags_text(asset.flags)),
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

/// `HandleDeleteAsset`. `None` is a missing row or a row with no bytes.
/// `deleted` is `AssetService.Delete`: false prints the error line.
pub fn delete_asset(args: &[&str], asset: Option<&DeletedAsset>, deleted: bool) -> Vec<String> {
    if args.len() < 3 {
        return vec!["Syntax: delete asset <ID>".to_string()];
    }
    let Some(asset) = asset else {
        return vec![format!("Could not find asset with ID {}", args[2])];
    };
    if deleted {
        vec![format!("Deleted asset {} {}", asset.id, asset.name)]
    } else {
        vec![format!(
            "ERROR: Could not delete asset {} {}",
            asset.id, asset.name
        )]
    }
}

/// `HandleDumpAsset`. The file name is the raw id token. `None` is a missing
/// row; empty bytes are still written. An existing file is left untouched.
pub fn dump_asset(
    args: &[&str],
    asset: Option<&[u8]>,
    directory: &Path,
) -> std::io::Result<Vec<String>> {
    if args.len() < 3 {
        return Ok(vec!["Usage is dump asset <ID>".to_string()]);
    }
    let raw = args[2];
    let Some(id) = console_uuid(raw) else {
        return Ok(vec![format!("ERROR: {raw} is not a valid ID format")]);
    };
    let Some(data) = asset else {
        return Ok(vec![format!("ERROR: No asset found with ID {id}")]);
    };
    let path = dump_path(directory, raw);
    if path.is_file() {
        return Ok(vec![format!(
            "File {raw} already exists.  Please move or remove it."
        )]);
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(data)?;
    Ok(vec![format!("Asset dumped to file {raw}")])
}

/// `OpenMetaverse.UUID.TryParse`: leading and trailing spaces, a hyphenated
/// 36-character id, or 32 hex characters. Braces and `urn:uuid:` are rejected.
pub fn console_uuid(raw: &str) -> Option<Uuid> {
    let bytes = raw.as_bytes();
    let mut start = 0;
    while start < bytes.len() && bytes[start] == b' ' {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && bytes[end - 1] == b' ' {
        end -= 1;
    }
    if !raw.is_char_boundary(start) || !raw.is_char_boundary(end) {
        return None;
    }
    let text = &raw[start..end];
    let mut chars = text.chars();
    if text.len() == 36
        && chars.nth(8) == Some('-')
        && text.chars().nth(13) == Some('-')
        && text.chars().nth(18) == Some('-')
        && text.chars().nth(23) == Some('-')
    {
        return Uuid::parse_str(text).ok();
    }
    if text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Uuid::parse_str(text).ok();
    }
    None
}

fn dump_path(directory: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        directory.join(path)
    }
}

/// `[Flags] AssetFlags.ToString`. A value with a bit outside 1, 2 and 4 is
/// the whole decimal. Zero is `Normal`.
pub fn asset_flags_text(flags: i32) -> String {
    const NAMED: i32 = 1 | 2 | 4;
    if flags & !NAMED != 0 {
        return flags.to_string();
    }
    if flags == 0 {
        return "Normal".to_string();
    }
    let mut parts = Vec::new();
    if flags & 1 != 0 {
        parts.push("Maptile");
    }
    if flags & 2 != 0 {
        parts.push("Rewritable");
    }
    if flags & 4 != 0 {
        parts.push("Collectable");
    }
    parts.join(", ")
}

fn asset_type_name(type_code: i8) -> String {
    let name = match type_code {
        -2 => "OSMaterial",
        -1 => "Unknown",
        0 => "Texture",
        1 => "Sound",
        2 => "CallingCard",
        3 => "Landmark",
        5 => "Clothing",
        6 => "Object",
        7 => "Notecard",
        8 => "Folder",
        10 => "LSLText",
        11 => "LSLBytecode",
        12 => "TextureTGA",
        13 => "Bodypart",
        17 => "SoundWAV",
        18 => "ImageTGA",
        19 => "ImageJPEG",
        20 => "Animation",
        21 => "Gesture",
        22 => "Simstate",
        24 => "Link",
        25 => "LinkFolder",
        26 => "MarketplaceFolder",
        49 => "Mesh",
        56 => "Settings",
        57 => "Material",
        58 => "GLTF",
        59 => "GLTF_bin",
        _ => return type_code.to_string(),
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/asset/commands.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        serde_json::from_str(&text).expect("commands.json")
    }

    fn lines_of(value: &serde_json::Value) -> Vec<String> {
        value["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .map(|line| line.as_str().expect("line").to_string())
            .collect()
    }

    fn args_of(value: &serde_json::Value) -> Vec<String> {
        value["args"]
            .as_array()
            .expect("args")
            .iter()
            .map(|arg| arg.as_str().expect("arg").to_string())
            .collect()
    }

    fn decode_hex(text: &str) -> Vec<u8> {
        assert!(text.len().is_multiple_of(2), "{text}");
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    fn encode_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn pilot_commands_match_csharp() {
        let document = fixture();
        for item in document["try_parse"].as_array().expect("try_parse") {
            let raw = item["raw"].as_str().expect("raw");
            let parsed = console_uuid(raw);
            assert_eq!(parsed.is_some(), item["ok"].as_bool().expect("ok"), "{raw}");
            if let Some(id) = parsed {
                assert_eq!(id.to_string(), item["id"].as_str().expect("id"));
            }
        }
        for item in document["flags"].as_array().expect("flags") {
            let value = i32::try_from(item["value"].as_i64().expect("value")).expect("flag");
            assert_eq!(
                asset_flags_text(value),
                item["text"].as_str().expect("text"),
                "{value}"
            );
        }
        for item in document["types"].as_array().expect("types") {
            let value = i8::try_from(item["value"].as_i64().expect("value")).expect("type");
            assert_eq!(asset_type_name(value), item["text"].as_str().expect("text"));
            assert_eq!(
                content_type(value),
                item["content_type"].as_str().expect("content")
            );
        }
        for item in document["show"].as_array().expect("show") {
            let args = args_of(item);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let shown = if item["asset"]["present"].as_bool().expect("present") {
                Some(ShownAsset {
                    name: item["asset"]["name"].as_str().expect("name").to_string(),
                    description: item["asset"]["description"]
                        .as_str()
                        .expect("description")
                        .to_string(),
                    type_code: i8::try_from(item["asset"]["type"].as_i64().expect("type"))
                        .expect("type"),
                    temporary: item["asset"]["temporary"].as_bool().expect("temporary"),
                    flags: i32::try_from(item["asset"]["flags"].as_i64().expect("flags"))
                        .expect("flags"),
                    data: decode_hex(item["asset"]["hex"].as_str().expect("hex")),
                })
            } else {
                None
            };
            assert_eq!(show_asset(&refs, shown.as_ref()), lines_of(item));
        }
        for item in document["delete"].as_array().expect("delete") {
            let args = args_of(item);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let asset = if item["asset"]["present"].as_bool().expect("present") {
                Some(DeletedAsset {
                    id: item["asset"]["id"].as_str().expect("id").to_string(),
                    name: item["asset"]["name"].as_str().expect("name").to_string(),
                })
            } else {
                None
            };
            assert_eq!(
                delete_asset(
                    &refs,
                    asset.as_ref(),
                    item["deleted"].as_bool().expect("deleted")
                ),
                lines_of(item)
            );
        }
        let directory = std::env::temp_dir().join(format!(
            "ferroan-dump-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&directory).expect("temp");
        for item in document["dump"].as_array().expect("dump") {
            let case = directory.join(Uuid::new_v4().simple().to_string());
            std::fs::create_dir_all(&case).expect("case");
            let args = args_of(item);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let bytes = decode_hex(item["hex"].as_str().expect("hex"));
            if item["file_exists"].as_bool().expect("exists") {
                std::fs::write(case.join(refs[2]), b"sentinel").expect("sentinel");
            }
            let asset = if item["found"].as_bool().expect("found") {
                Some(bytes.as_slice())
            } else {
                None
            };
            let lines = dump_asset(&refs, asset, &case).expect("dump");
            assert_eq!(lines, lines_of(item));
            if item["write"].as_bool().expect("write") {
                let written = std::fs::read(case.join(refs[2])).expect("written");
                assert_eq!(
                    encode_hex(&written),
                    item["write_hex"].as_str().expect("hex")
                );
            }
        }
        let _ = std::fs::remove_dir_all(directory);
    }
}
