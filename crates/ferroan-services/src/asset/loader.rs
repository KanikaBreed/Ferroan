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
//   Source/OpenSim.Framework.AssetLoader.Filesystem/AssetLoaderFileSystem.cs
//   Source/OpenSim.Services.AssetService/AssetService.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// Nini's XmlConfigSource is not copied. A malformed set file is skipped, as
// the loader's XmlException catch does. A duplicate key, a bad asset id or a
// bad assetType fails the load before any row is written. A repeated section
// name is dropped: Nini calls Configs.Add before LoadKeys, and Add keeps the
// first IConfig.

//! The filesystem default-asset loader.
//!
//! `DefaultAssetLoader = OpenSim.Framework.AssetLoader.Filesystem.dll` reads
//! `AssetLoaderArgs` (an `AssetSets.xml` path) and stores each library asset.
//! An id that is already stored is left alone when the SHA1 of its bytes
//! matches, and replaced through `StoreAsset` when it does not. A duplicate
//! key is Nini's `ArgumentException`, so the load fails before any row is
//! written. A repeated section name keeps the first section. HTTP
//! `UpdateContent` is a different path and still returns false.

use std::fs;
use std::path::{Component, Path, PathBuf};

use ferroan_diag::{Kind, refuse};
use sha1::{Digest, Sha1};
use uuid::Uuid;

use super::model::Asset;
use super::store::{AssetStore, StoreError};

const LIBRARY_OWNER: &str = "11111111-1111-0000-0000-000100bba000";
const MAX_FILES: u64 = 10_000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

/// One asset from `ForEachDefaultXmlAsset`, before the service stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedAsset {
    pub id: Uuid,
    pub name: String,
    pub type_code: i8,
    pub creator_id: String,
    pub description: String,
    pub local: bool,
    pub temporary: bool,
    pub flags: i32,
    /// `None` only when a caller builds a row the file loader did not.
    /// A missing file is `FileNotFoundException` in `LoadAsset` (`FileInfo.Length`
    /// throws before the exists check), so `load_asset_sets` returns an error
    /// instead of a null payload.
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoaderError {
    #[error("duplicate key {name} in {path}")]
    DuplicateKey { path: String, name: String },
    #[error("assetType {value} in {path} is not an integer")]
    Type { path: String, value: String },
    #[error("assetID {value} in {path} is not a UUID")]
    Id { path: String, value: String },
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Limit(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Read the asset-set control file the way `AssetLoaderFileSystem` does.
///
/// A missing control file or a control file that is not XML yields an empty
/// list. A bad set file is skipped. A repeated section name keeps the first
/// section. A duplicate key, a bad id or a bad type fails the whole load.
pub fn load_asset_sets(path: &Path) -> Result<Vec<LoadedAsset>, LoaderError> {
    if !path.is_file() {
        tracing::error!(
            "asset loader: control file {} does not exist",
            path.display()
        );
        return Ok(Vec::new());
    }
    let root = absolute(path)?;
    let parent = root.parent().unwrap_or(Path::new(".")).to_path_buf();
    let sections = match parse_config(path)? {
        Ok(sections) => sections,
        Err(()) => return Ok(Vec::new()),
    };
    let mut assets = Vec::new();
    let mut total = 0u64;
    for section in sections {
        let file = section.value("file").unwrap_or("");
        if file.is_empty() {
            tracing::error!("asset loader: set file is empty");
            continue;
        }
        let set_path = join_path(&parent, file);
        if !set_path.is_file() {
            tracing::error!(
                "asset loader: set file {} does not exist",
                set_path.display()
            );
            continue;
        }
        let set_sections = match parse_config(&set_path)? {
            Ok(sections) => sections,
            Err(()) => continue,
        };
        let dir = set_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        for item in set_sections {
            let loaded = asset_from_section(&set_path, &dir, &item, &mut total)?;
            if assets.len() as u64 >= MAX_FILES {
                refuse(
                    ferroan_diag::ASSET_LOADER_FILES,
                    Kind::Hard,
                    MAX_FILES,
                    assets.len() as u64 + 1,
                    "asset-loader",
                );
                return Err(LoaderError::Limit(format!(
                    "asset loader stopped at {MAX_FILES} assets"
                )));
            }
            assets.push(loaded);
        }
    }
    Ok(assets)
}

/// `AssetService`'s loader callback: store when the id is new or the bytes differ.
pub async fn seed_assets(store: &AssetStore, assets: &[LoadedAsset]) -> Result<(), LoaderError> {
    for asset in assets {
        let existing = store.get(asset.id).await?;
        let Some(data) = asset.data.as_ref() else {
            // SHA1Hash(null) throws once a row exists. A new id still calls
            // StoreAsset; this store has no NULL bytes, so the load stops
            // rather than writing a different row.
            return Err(LoaderError::Io(format!("asset {} has no bytes", asset.id)));
        };
        let replace = match &existing {
            None => true,
            Some(row) => sha1_hex(&row.data) != sha1_hex(data),
        };
        if replace {
            store.store_data(&asset.as_asset(data.clone())).await?;
        }
    }
    Ok(())
}

impl LoadedAsset {
    fn as_asset(&self, data: Vec<u8>) -> Asset {
        Asset {
            id: self.id,
            name: self.name.clone(),
            description: self.description.clone(),
            type_code: self.type_code,
            local: self.local,
            temporary: self.temporary,
            creator_id: self.creator_id.clone(),
            flags: self.flags,
            data,
            upload_attempts: 0,
        }
    }
}

/// Lowercase SHA1 hex. `Util.bytesToHexString` uses the lowercase nibble
/// helpers for both the upper and lower flag.
pub fn sha1_hex(data: &[u8]) -> String {
    let digest = Sha1::digest(data);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        text.push(char::from(nibble(byte >> 4)));
        text.push(char::from(nibble(byte & 0x0f)));
    }
    text
}

fn nibble(value: u8) -> u8 {
    if value > 9 {
        value + 0x57
    } else {
        value + b'0'
    }
}

fn asset_from_section(
    set_path: &Path,
    dir: &Path,
    section: &Section,
    total: &mut u64,
) -> Result<LoadedAsset, LoaderError> {
    let path_text = set_path.display().to_string();
    let id = match section.value("assetID") {
        None => Uuid::new_v4(),
        Some(value) => Uuid::parse_str(value).map_err(|_| LoaderError::Id {
            path: path_text.clone(),
            value: value.to_string(),
        })?,
    };
    let type_code = match section.value("assetType") {
        None => 0,
        Some(value) => parse_type(value).map_err(|_| LoaderError::Type {
            path: path_text.clone(),
            value: value.to_string(),
        })?,
    };
    let name = section.value("name").unwrap_or("").to_string();
    let file_name = section.value("fileName").unwrap_or("");
    let data = if file_name.is_empty() {
        Some(Vec::new())
    } else {
        let file = join_path(dir, file_name);
        if !file.is_file() {
            return Err(LoaderError::Io(format!(
                "asset loader: file {} not found",
                file.display()
            )));
        }
        Some(read_bounded(&file, total)?)
    };
    Ok(LoadedAsset {
        id,
        name,
        type_code,
        creator_id: LIBRARY_OWNER.to_string(),
        description: String::new(),
        local: false,
        temporary: false,
        flags: 0,
        data,
    })
}

fn parse_type(value: &str) -> Result<i8, ()> {
    let value = value.trim();
    let parsed: i32 = value.parse().map_err(|_| ())?;
    Ok(parsed as i8)
}

fn read_bounded(path: &Path, total: &mut u64) -> Result<Vec<u8>, LoaderError> {
    let meta = fs::metadata(path).map_err(|error| LoaderError::Io(error.to_string()))?;
    let len = meta.len();
    if len > MAX_FILE_BYTES {
        refuse(
            ferroan_diag::ASSET_LOADER_FILE_BYTES,
            Kind::Hard,
            MAX_FILE_BYTES,
            len,
            "asset-loader",
        );
        return Err(LoaderError::Limit(format!(
            "{} is {len} bytes",
            path.display()
        )));
    }
    let next = total.saturating_add(len);
    if next > MAX_TOTAL_BYTES {
        refuse(
            ferroan_diag::ASSET_LOADER_TOTAL_BYTES,
            Kind::Hard,
            MAX_TOTAL_BYTES,
            next,
            "asset-loader",
        );
        return Err(LoaderError::Limit(
            "asset loader byte total exceeded".to_string(),
        ));
    }
    let bytes = fs::read(path).map_err(|error| LoaderError::Io(error.to_string()))?;
    *total = next;
    Ok(bytes)
}

/// `Ok(Err(()))` is an XML failure the loader swallows.
fn parse_config(path: &Path) -> Result<Result<Vec<Section>, ()>, LoaderError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => return Err(LoaderError::Io(error.to_string())),
    };
    if text.len() as u64 > MAX_FILE_BYTES {
        refuse(
            ferroan_diag::ASSET_LOADER_FILE_BYTES,
            Kind::Hard,
            MAX_FILE_BYTES,
            text.len() as u64,
            "asset-loader",
        );
        return Err(LoaderError::Limit(format!(
            "{} is too large",
            path.display()
        )));
    }
    match parse_sections(&text) {
        Ok(sections) => {
            let mut kept = Vec::new();
            for section in sections {
                if kept
                    .iter()
                    .any(|earlier: &Section| earlier.name == section.name)
                {
                    continue;
                }
                let mut keys = Vec::new();
                for (key, _) in &section.keys {
                    if keys.iter().any(|name| name == key) {
                        return Err(LoaderError::DuplicateKey {
                            path: path.display().to_string(),
                            name: key.clone(),
                        });
                    }
                    keys.push(key.clone());
                }
                kept.push(section);
            }
            Ok(Ok(kept))
        }
        Err(error) => {
            tracing::error!("asset loader: error loading {}: {error}", path.display());
            Ok(Err(()))
        }
    }
}

struct Section {
    name: String,
    keys: Vec<(String, String)>,
}

impl Section {
    fn value(&self, key: &str) -> Option<&str> {
        self.keys
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

struct Tag {
    name: String,
    close: bool,
    self_closing: bool,
    attrs: Vec<(String, String)>,
}

fn parse_sections(text: &str) -> Result<Vec<Section>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let tags = tokenize(text)?;
    if tags.is_empty() {
        return Err("empty document".to_string());
    }
    let mut index = 0;
    if !tags[0].close {
        index = 1;
    }
    let mut sections = Vec::new();
    while index < tags.len() {
        let tag = &tags[index];
        if tag.close && tag.name.eq_ignore_ascii_case(&tags[0].name) {
            break;
        }
        if tag.close || !tag.name.eq_ignore_ascii_case("Section") {
            return Err(format!("unexpected <{}>", tag.name));
        }
        let name = attr(tag, "Name").unwrap_or("").to_string();
        index += 1;
        let mut keys = Vec::new();
        if !tag.self_closing {
            while index < tags.len() {
                let inner = &tags[index];
                if inner.close && inner.name.eq_ignore_ascii_case("Section") {
                    index += 1;
                    break;
                }
                if inner.close || !inner.name.eq_ignore_ascii_case("Key") {
                    return Err(format!("unexpected <{}> in section {name}", inner.name));
                }
                let key = attr(inner, "Name").unwrap_or("").to_string();
                let value = attr(inner, "Value").unwrap_or("").to_string();
                keys.push((key, value));
                index += 1;
                if inner.self_closing {
                    continue;
                }
                if index >= tags.len()
                    || !tags[index].close
                    || !tags[index].name.eq_ignore_ascii_case("Key")
                {
                    return Err(format!("section {name} has an unclosed key"));
                }
                index += 1;
            }
        }
        sections.push(Section { name, keys });
    }
    Ok(sections)
}

fn attr<'a>(tag: &'a Tag, name: &str) -> Option<&'a str> {
    tag.attrs
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn tokenize(text: &str) -> Result<Vec<Tag>, String> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut tags = Vec::new();
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        if text[index..].starts_with("<!--") {
            let Some(end) = text[index + 4..].find("-->") else {
                return Err("unclosed comment".to_string());
            };
            index += 4 + end + 3;
            continue;
        }
        if text[index..].starts_with("<?") {
            let Some(end) = text[index + 2..].find("?>") else {
                return Err("unclosed declaration".to_string());
            };
            index += 2 + end + 2;
            continue;
        }
        let mut end = index + 1;
        let mut quote = None;
        while end < bytes.len() {
            let byte = bytes[end];
            if let Some(mark) = quote {
                if byte == mark {
                    quote = None;
                }
            } else if byte == b'"' || byte == b'\'' {
                quote = Some(byte);
            } else if byte == b'>' {
                break;
            }
            end += 1;
        }
        if end >= bytes.len() {
            return Err("unclosed tag".to_string());
        }
        let body = &text[index + 1..end];
        index = end + 1;
        if body.starts_with('!') || body.starts_with('/') && body[1..].contains('<') {
            return Err("bad tag".to_string());
        }
        tags.push(parse_tag(body)?);
    }
    Ok(tags)
}

fn parse_tag(body: &str) -> Result<Tag, String> {
    let mut body = body.trim();
    let close = body.starts_with('/');
    if close {
        body = body[1..].trim();
    }
    let self_closing = body.ends_with('/');
    if self_closing {
        body = body[..body.len() - 1].trim();
    }
    let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
    let name = body[..name_end].to_string();
    if name.is_empty() || name.chars().any(|ch| ch == '<' || ch == '>') {
        return Err("bad tag name".to_string());
    }
    let mut rest = body[name_end..].trim();
    let mut attrs = Vec::new();
    while !rest.is_empty() {
        let eq = rest
            .find('=')
            .ok_or_else(|| format!("attribute in <{name}>"))?;
        let key = rest[..eq].trim();
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            return Err(format!("attribute name in <{name}>"));
        }
        rest = rest[eq + 1..].trim_start();
        let quote = rest
            .chars()
            .next()
            .ok_or_else(|| format!("attribute {key}"))?;
        if quote != '"' && quote != '\'' {
            return Err(format!("attribute {key} is not quoted"));
        }
        rest = &rest[quote.len_utf8()..];
        let value_end = rest.find(quote).ok_or_else(|| format!("attribute {key}"))?;
        let value = decode_entities(&rest[..value_end])?;
        attrs.push((key.to_string(), value));
        rest = rest[value_end + quote.len_utf8()..].trim_start();
    }
    Ok(Tag {
        name,
        close,
        self_closing,
        attrs,
    })
}

fn decode_entities(value: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let Some(end) = rest.find(';') else {
            return Err("unclosed entity".to_string());
        };
        let entity = &rest[..end];
        rest = &rest[end + 1..];
        let decoded = if let Some(hex) = entity
            .strip_prefix("#x")
            .or_else(|| entity.strip_prefix("#X"))
        {
            char_from(u32::from_str_radix(hex, 16).map_err(|_| "bad entity".to_string())?)?
        } else if let Some(decimal) = entity.strip_prefix('#') {
            char_from(decimal.parse().map_err(|_| "bad entity".to_string())?)?
        } else {
            match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => return Err(format!("unknown entity {entity}")),
            }
        };
        out.push(decoded);
    }
    out.push_str(rest);
    Ok(out)
}

fn char_from(value: u32) -> Result<char, String> {
    char::from_u32(value).ok_or_else(|| "bad entity".to_string())
}

fn absolute(path: &Path) -> Result<PathBuf, LoaderError> {
    if path.is_absolute() {
        Ok(normalize(path))
    } else {
        let cwd = std::env::current_dir().map_err(|error| LoaderError::Io(error.to_string()))?;
        Ok(normalize(&cwd.join(path)))
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn join_path(base: &Path, relative: &str) -> PathBuf {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        normalize(relative_path)
    } else {
        normalize(&base.join(relative_path))
    }
}

/// The configured loader name is the filesystem plugin, optionally with its class.
///
/// `LoadPlugin` splits a class suffix on the last colon. A Windows drive
/// prefix is not a class name: the text after that colon contains a slash.
pub fn filesystem_loader(name: &str) -> bool {
    let trimmed = name.trim();
    let dll = match trimmed.rsplit_once(':') {
        Some((path, class))
            if !path.is_empty()
                && !class.is_empty()
                && !class.contains(['\\', '/'])
                && !path.ends_with(':') =>
        {
            path
        }
        _ => trimmed,
    };
    Path::new(dll).file_name().and_then(|file| file.to_str())
        == Some("OpenSim.Framework.AssetLoader.Filesystem.dll")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_of_empty_and_hello_is_lowercase() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            sha1_hex(b"hello"),
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        );
    }

    #[test]
    fn a_missing_control_file_loads_nothing() {
        let missing = std::env::temp_dir().join("ferroan-no-such-asset-sets.xml");
        let _ = fs::remove_file(&missing);
        let loaded = load_asset_sets(&missing).expect("missing file");
        assert!(loaded.is_empty());
    }

    #[test]
    fn entities_and_a_missing_id_follow_the_set_file() {
        let dir = std::env::temp_dir().join(format!("ferroan-loader-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(dir.join("One")).unwrap();
        fs::write(dir.join("One/hello.dat"), b"hello").unwrap();
        fs::write(
            dir.join("One/one.xml"),
            r#"<Nini>
  <Section Name="Hello">
    <Key Name="assetID" Value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
    <Key Name="name" Value="a &amp; b"/>
    <Key Name="assetType" Value=" 20 "/>
    <Key Name="fileName" Value="hello.dat"/>
  </Section>
  <Section Name="Random">
    <Key Name="name" Value="Random"/>
    <Key Name="assetType" Value="-2"/>
  </Section>
</Nini>
"#,
        )
        .unwrap();
        fs::write(
            dir.join("AssetSets.xml"),
            r#"<Nini>
  <Section Name="One"><Key Name="file" Value="One/one.xml"/></Section>
</Nini>
"#,
        )
        .unwrap();
        let loaded = load_asset_sets(&dir.join("AssetSets.xml")).expect("load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "a & b");
        assert_eq!(loaded[0].type_code, 20);
        assert_eq!(loaded[0].data.as_deref(), Some(&b"hello"[..]));
        assert_eq!(loaded[0].creator_id, LIBRARY_OWNER);
        assert_eq!(loaded[1].name, "Random");
        assert_eq!(loaded[1].type_code, -2);
        assert_eq!(loaded[1].data.as_deref(), Some(&b""[..]));
        assert_ne!(loaded[1].id, Uuid::nil());
        let again = load_asset_sets(&dir.join("AssetSets.xml")).expect("again");
        assert_ne!(loaded[1].id, again[1].id);
        let _ = fs::remove_dir_all(dir);
    }

    fn asset_fixture(name: &str) -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/asset")
            .join(name);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    fn fixture_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/asset")
    }

    fn expect_i8(value: &serde_json::Value, key: &str) -> i8 {
        i8::try_from(value[key].as_i64().unwrap_or_else(|| panic!("{key}"))).expect(key)
    }

    #[test]
    fn pilot_loader_sample_matches_csharp() {
        let document = asset_fixture("loader-sample.json");
        assert!(
            document["missing_asset_id_is_random"]
                .as_bool()
                .expect("random")
        );
        let control = fixture_root().join(document["sample"].as_str().expect("sample"));
        let loaded = load_asset_sets(&control).expect("sample");
        assert_loaded(&loaded, document["assets"].as_array().expect("assets"));
        for failure in document["failures"].as_array().expect("failures") {
            let path = fixture_root().join(failure["file"].as_str().expect("file"));
            let name = failure["name"].as_str().expect("name");
            let result = load_asset_sets(&path);
            if failure["threw"].as_bool().expect("threw") {
                assert!(result.is_err(), "{name}");
            } else {
                let loaded = result.unwrap_or_else(|error| panic!("{name}: {error}"));
                assert_loaded(&loaded, failure["assets"].as_array().expect("assets"));
            }
        }
    }

    fn assert_loaded(loaded: &[LoadedAsset], assets: &[serde_json::Value]) {
        assert_eq!(loaded.len(), assets.len());
        for (got, expected) in loaded.iter().zip(assets) {
            assert_eq!(got.id.to_string(), expected["id"].as_str().expect("id"));
            assert_eq!(got.name, expected["name"].as_str().expect("name"));
            assert_eq!(got.type_code, expect_i8(expected, "type"));
            assert_eq!(
                got.creator_id,
                expected["creator"].as_str().expect("creator")
            );
            assert_eq!(
                got.description,
                expected["description"].as_str().expect("description")
            );
            assert_eq!(got.local, expected["local"].as_bool().expect("local"));
            assert_eq!(
                got.temporary,
                expected["temporary"].as_bool().expect("temporary")
            );
            assert_eq!(got.flags, expected["flags"].as_i64().expect("flags") as i32);
            if expected["data_null"].as_bool().expect("null") {
                assert!(got.data.is_none());
            } else {
                let data = got.data.as_deref().expect("bytes");
                assert_eq!(
                    data.len() as i64,
                    expected["length"].as_i64().expect("length")
                );
                assert_eq!(sha1_hex(data), expected["sha1"].as_str().expect("sha1"));
            }
        }
    }

    #[tokio::test]
    async fn pilot_seed_matches_csharp() {
        let document = asset_fixture("seed.json");
        let dir = std::env::temp_dir().join(format!(
            "ferroan-seed-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).expect("temp");
        let path = dir.join("AssetStore.db");
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/db/sqlite/AssetStore.db");
        fs::copy(&source, &path).expect("copy");
        let pool = ferroan_db::open_sqlite_file(&path, &["AssetStore"])
            .await
            .expect("open");
        let store = AssetStore::new(pool).expect("store");
        for case in document["cases"].as_array().expect("cases") {
            let id = Uuid::parse_str(case["id"].as_str().expect("id")).expect("id");
            if case["before"]["present"].as_bool().expect("before") {
                store
                    .store_data(&row_asset(id, &case["before"]))
                    .await
                    .expect("before");
            }
            let input = &case["input"];
            let loaded = LoadedAsset {
                id,
                name: input["name"].as_str().expect("name").to_string(),
                type_code: expect_i8(input, "type"),
                creator_id: input["creator"].as_str().expect("creator").to_string(),
                description: input["description"]
                    .as_str()
                    .expect("description")
                    .to_string(),
                local: input["local"].as_bool().expect("local"),
                temporary: input["temporary"].as_bool().expect("temporary"),
                flags: input["flags"].as_i64().expect("flags") as i32,
                data: if input["data_null"].as_bool().expect("null") {
                    None
                } else {
                    Some(decode_hex(input["hex"].as_str().expect("hex")))
                },
            };
            let seeded = seed_assets(&store, &[loaded]).await;
            let row = store.get(id).await.expect("get");
            if input["data_null"].as_bool().expect("null") {
                assert!(seeded.is_err(), "{}", case["name"]);
                if case["before"]["present"].as_bool().expect("before") {
                    assert_row(
                        case["name"].as_str().expect("name"),
                        row.as_ref(),
                        &case["before"],
                    );
                } else {
                    assert!(row.is_none(), "{}", case["name"]);
                }
                continue;
            }
            assert!(seeded.is_ok(), "{}: {seeded:?}", case["name"]);
            assert_row(
                case["name"].as_str().expect("name"),
                row.as_ref(),
                &case["after"],
            );
        }
        drop(store);
        let _ = fs::remove_dir_all(dir);
    }

    fn row_asset(id: Uuid, row: &serde_json::Value) -> Asset {
        Asset {
            id,
            name: row["name"].as_str().expect("name").to_string(),
            description: row["description"]
                .as_str()
                .expect("description")
                .to_string(),
            type_code: expect_i8(row, "type"),
            local: row["local"].as_bool().expect("local"),
            temporary: row["temporary"].as_bool().expect("temporary"),
            creator_id: row["creator"].as_str().expect("creator").to_string(),
            flags: row["flags"].as_i64().expect("flags") as i32,
            data: decode_hex(row["hex"].as_str().expect("hex")),
            upload_attempts: 0,
        }
    }

    fn assert_row(label: &str, row: Option<&Asset>, expected: &serde_json::Value) {
        assert!(expected["present"].as_bool().expect("present"), "{label}");
        let row = row.unwrap_or_else(|| panic!("{label} missing"));
        assert!(!expected["data_null"].as_bool().expect("null"), "{label}");
        assert_eq!(
            row.name,
            expected["name"].as_str().expect("name"),
            "{label}"
        );
        assert_eq!(row.type_code, expect_i8(expected, "type"), "{label}");
        assert_eq!(
            row.data,
            decode_hex(expected["hex"].as_str().expect("hex")),
            "{label}"
        );
        assert_eq!(
            row.creator_id,
            expected["creator"].as_str().expect("creator"),
            "{label}"
        );
        assert_eq!(
            row.flags,
            expected["flags"].as_i64().expect("flags") as i32,
            "{label}"
        );
    }

    fn decode_hex(text: &str) -> Vec<u8> {
        assert!(text.len().is_multiple_of(2), "{text}");
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    fn library_control() -> Option<std::path::PathBuf> {
        let root = std::env::var_os("FERROAN_REF")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join("Git/reference"))
            })?;
        let path = root.join(
            "OpenSim-Tranquillity/Source/OpenSim.Services.InventoryService/AppData/assets/AssetSets.xml",
        );
        path.is_file().then_some(path)
    }

    #[test]
    fn pilot_library_matches_the_csharp_manifest() {
        let forced = std::env::var("FERROAN_ASSET_LIBRARY").ok().as_deref() == Some("1");
        let Some(path) = library_control() else {
            if forced {
                panic!("FERROAN_ASSET_LIBRARY=1 but the pinned AssetSets.xml is missing");
            }
            eprintln!("pinned AssetSets.xml is not available; library manifest was not compared");
            return;
        };
        let document = asset_fixture("library-manifest.json");
        let loaded = load_asset_sets(&path).expect("library");
        let assets = document["assets"].as_array().expect("assets");
        assert_eq!(loaded.len(), assets.len());
        assert_eq!(
            loaded.len() as i64,
            document["count"].as_i64().expect("count")
        );
        for (got, expected) in loaded.iter().zip(assets) {
            let id = expected["id"].as_str().expect("id");
            assert_eq!(got.id.to_string(), id);
            assert_eq!(got.name, expected["name"].as_str().expect("name"), "{id}");
            assert_eq!(got.type_code, expect_i8(expected, "type"), "{id}");
            assert_eq!(
                got.creator_id,
                expected["creator"].as_str().expect("creator"),
                "{id}"
            );
            assert_eq!(
                got.description,
                expected["description"].as_str().expect("description"),
                "{id}"
            );
            assert_eq!(
                got.local,
                expected["local"].as_bool().expect("local"),
                "{id}"
            );
            assert_eq!(
                got.temporary,
                expected["temporary"].as_bool().expect("temporary"),
                "{id}"
            );
            assert_eq!(
                got.flags,
                expected["flags"].as_i64().expect("flags") as i32,
                "{id}"
            );
            assert!(!expected["data_null"].as_bool().expect("null"), "{id}");
            let data = got.data.as_deref().unwrap_or_else(|| panic!("{id}"));
            assert_eq!(
                data.len() as i64,
                expected["length"].as_i64().expect("length"),
                "{id}"
            );
            assert_eq!(
                sha1_hex(data),
                expected["sha1"].as_str().expect("sha1"),
                "{id}"
            );
        }
    }
}
