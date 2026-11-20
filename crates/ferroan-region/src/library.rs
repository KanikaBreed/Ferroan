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
//   Source/OpenSim.Services.InventoryService/LibraryService.cs
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The in-process inventory library. Tranquillity reads Nini XML here; it is
//! not an HTTP Robust service. A missing file keeps the root folder.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use ferroan_diag::{self, Kind, LIBRARY_FILE_BYTES, LIBRARY_FILES, LIBRARY_TOTAL_BYTES};
use uuid::Uuid;

const OWNER_TEXT: &str = "11111111-1111-0000-0000-000100bba000";
const ROOT_TEXT: &str = "00000112-000f-0000-0000-000100bba000";

fn library_uuid(text: &str) -> Uuid {
    // INFALLIBLE: OWNER_TEXT and ROOT_TEXT are the fixed library UUIDs in this file.
    #[allow(clippy::expect_used)]
    Uuid::parse_str(text).expect("library uuid")
}
const BASE_PERMISSIONS: u32 = 0x9e000;
const EVERYONE_PERMISSIONS: u32 = 0x9a000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FILES: usize = 64;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("library control file is missing")]
    Missing,
    #[error("library file is too large")]
    TooLarge,
    #[error("library file could not be read")]
    Io,
}

#[derive(Clone, Debug)]
pub struct LibraryFolder {
    pub id: Uuid,
    pub parent_id: Uuid,
    pub name: String,
    pub type_code: i32,
    pub version: i32,
    pub owner: Uuid,
    pub children: Vec<Uuid>,
    pub items: Vec<Uuid>,
}

#[derive(Clone, Debug)]
pub struct LibraryItem {
    pub id: Uuid,
    pub asset_id: Uuid,
    pub folder: Uuid,
    pub name: String,
    pub description: String,
    pub asset_type: i32,
    pub inv_type: i32,
    pub current_permissions: u32,
    pub next_permissions: u32,
    pub everyone_permissions: u32,
    pub base_permissions: u32,
    pub group_permissions: u32,
    pub flags: u32,
    pub creator_id: String,
    pub owner: Uuid,
    pub group_id: Uuid,
    pub group_owned: bool,
    pub sale_price: i32,
    pub sale_type: i32,
    pub creation_date: i32,
}

#[derive(Clone, Debug)]
pub struct Library {
    loaded: bool,
    owner: Uuid,
    folders: Vec<LibraryFolder>,
    items: Vec<LibraryItem>,
    folder_at: HashMap<Uuid, usize>,
    item_at: HashMap<Uuid, usize>,
}

impl Library {
    pub fn absent() -> Self {
        Self {
            loaded: false,
            owner: Uuid::nil(),
            folders: Vec::new(),
            items: Vec::new(),
            folder_at: HashMap::new(),
            item_at: HashMap::new(),
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn owner(&self) -> Uuid {
        self.owner
    }

    pub fn root_id(&self) -> Uuid {
        self.folders
            .first()
            .map(|folder| folder.id)
            .unwrap_or(Uuid::nil())
    }

    /// Load `DefaultLibrary`. Included files stay inside that file's directory.
    pub fn open(path: &Path, name: &str) -> Result<Self, LibraryError> {
        let owner = library_uuid(OWNER_TEXT);
        let root_id = library_uuid(ROOT_TEXT);
        let mut library = Self {
            loaded: true,
            owner,
            folders: vec![LibraryFolder {
                id: root_id,
                parent_id: Uuid::nil(),
                name: name.to_owned(),
                type_code: 8,
                version: 1,
                owner,
                children: Vec::new(),
                items: Vec::new(),
            }],
            items: Vec::new(),
            folder_at: HashMap::from([(root_id, 0)]),
            item_at: HashMap::new(),
        };
        if !path.is_file() {
            return Err(LibraryError::Missing);
        }
        let mut files = 0usize;
        let mut total = 0u64;
        library.read_control(path, &mut files, &mut total)?;
        Ok(library)
    }

    pub fn root_only(name: &str) -> Self {
        let owner = library_uuid(OWNER_TEXT);
        let root_id = library_uuid(ROOT_TEXT);
        Self {
            loaded: true,
            owner,
            folders: vec![LibraryFolder {
                id: root_id,
                parent_id: Uuid::nil(),
                name: name.to_owned(),
                type_code: 8,
                version: 1,
                owner,
                children: Vec::new(),
                items: Vec::new(),
            }],
            items: Vec::new(),
            folder_at: HashMap::from([(root_id, 0)]),
            item_at: HashMap::new(),
        }
    }

    pub fn folder(&self, id: Uuid) -> Option<&LibraryFolder> {
        self.folder_at.get(&id).map(|index| &self.folders[*index])
    }

    pub fn item(&self, id: Uuid) -> Option<&LibraryItem> {
        self.item_at.get(&id).map(|index| &self.items[*index])
    }

    pub fn child_folders(&self, id: Uuid) -> Vec<&LibraryFolder> {
        self.folder(id)
            .map(|folder| {
                folder
                    .children
                    .iter()
                    .filter_map(|child| self.folder(*child))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn child_items(&self, id: Uuid) -> Vec<&LibraryItem> {
        self.folder(id)
            .map(|folder| {
                folder
                    .items
                    .iter()
                    .filter_map(|item| self.item(*item))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Found items only, in request order. `None` when none of the ids exist.
    pub fn items_by_id(&self, ids: &[Uuid]) -> Option<Vec<&LibraryItem>> {
        let found: Vec<_> = ids.iter().filter_map(|id| self.item(*id)).collect();
        if found.is_empty() { None } else { Some(found) }
    }

    fn read_control(
        &mut self,
        path: &Path,
        files: &mut usize,
        total: &mut u64,
    ) -> Result<(), LibraryError> {
        let base = path.parent().unwrap_or(Path::new("."));
        for section in read_sections(path, files, total)? {
            if section.contains_key("RootVersion") {
                if let Some(root) = self.folders.first_mut() {
                    root.version = ushort_value(section.get("RootVersion"), root.version);
                }
                continue;
            }
            if let Some(folders) = contained(&section, "foldersFile", base) {
                self.read_folders(&folders, files, total)?;
            }
            if let Some(items) = contained(&section, "itemsFile", base) {
                self.read_items(&items, files, total)?;
            }
        }
        Ok(())
    }

    fn read_folders(
        &mut self,
        path: &Path,
        files: &mut usize,
        total: &mut u64,
    ) -> Result<(), LibraryError> {
        let sections = match read_sections(path, files, total) {
            Ok(sections) => sections,
            Err(LibraryError::Missing) => return Ok(()),
            Err(error) => return Err(error),
        };
        for section in sections {
            let id = uuid_value(section.get("folderID"), ROOT_TEXT);
            let parent = uuid_value(section.get("parentFolderID"), ROOT_TEXT);
            if self.folder_at.contains_key(&id) || !self.folder_at.contains_key(&parent) {
                continue;
            }
            let index = self.folders.len();
            self.folders.push(LibraryFolder {
                id,
                parent_id: parent,
                name: text_value(section.get("name"), "unknown"),
                type_code: short_value(section.get("type"), 8),
                version: ushort_value(section.get("version"), 1),
                owner: self.owner,
                children: Vec::new(),
                items: Vec::new(),
            });
            self.folder_at.insert(id, index);
            if let Some(parent_index) = self.folder_at.get(&parent).copied() {
                self.folders[parent_index].children.push(id);
            }
        }
        Ok(())
    }

    fn read_items(
        &mut self,
        path: &Path,
        files: &mut usize,
        total: &mut u64,
    ) -> Result<(), LibraryError> {
        let sections = match read_sections(path, files, total) {
            Ok(sections) => sections,
            Err(LibraryError::Missing) => return Ok(()),
            Err(error) => return Err(error),
        };
        for section in sections {
            let id = uuid_value(section.get("inventoryID"), ROOT_TEXT);
            let folder = uuid_value(section.get("folderID"), ROOT_TEXT);
            if self.item_at.contains_key(&id) || !self.folder_at.contains_key(&folder) {
                continue;
            }
            let name = text_value(section.get("name"), "");
            let inv_type = int_value(section.get("inventoryType"), 0);
            // GroupPermissions reads the basePermissions key, with default 0.
            let group = long_value(section.get("basePermissions"), 0);
            let item = LibraryItem {
                id,
                asset_id: uuid_value(section.get("assetID"), &id.to_string()),
                folder,
                description: text_value(section.get("description"), &name),
                name,
                inv_type,
                asset_type: int_value(section.get("assetType"), inv_type),
                current_permissions: long_value(
                    section.get("currentPermissions"),
                    BASE_PERMISSIONS,
                ),
                next_permissions: long_value(section.get("nextPermissions"), BASE_PERMISSIONS),
                everyone_permissions: long_value(
                    section.get("everyonePermissions"),
                    EVERYONE_PERMISSIONS,
                ),
                base_permissions: long_value(section.get("basePermissions"), BASE_PERMISSIONS),
                group_permissions: group,
                flags: int_value(section.get("flags"), 0) as u32,
                creator_id: self.owner.to_string(),
                owner: self.owner,
                group_id: Uuid::nil(),
                group_owned: false,
                sale_price: 0,
                sale_type: 0,
                // InventoryItemBase initializes CreationDate to unix seconds at
                // construction. The library XML does not store a date.
                creation_date: unix_now(),
            };
            let index = self.items.len();
            self.items.push(item);
            self.item_at.insert(id, index);
            if let Some(folder_index) = self.folder_at.get(&folder).copied() {
                self.folders[folder_index].items.push(id);
            }
        }
        Ok(())
    }
}

fn contained(section: &HashMap<String, String>, key: &str, base: &Path) -> Option<PathBuf> {
    let relative = section.get(key)?.trim();
    if relative.is_empty() {
        return None;
    }
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return None;
    }
    Some(base.join(path))
}

fn read_sections(
    path: &Path,
    files: &mut usize,
    total: &mut u64,
) -> Result<Vec<HashMap<String, String>>, LibraryError> {
    if *files >= MAX_FILES {
        ferroan_diag::refuse(
            LIBRARY_FILES,
            Kind::Hard,
            MAX_FILES as u64,
            (*files as u64).saturating_add(1),
            "library",
        );
        return Err(LibraryError::TooLarge);
    }
    let meta = fs::metadata(path).map_err(|_| LibraryError::Missing)?;
    if !meta.is_file() {
        return Err(LibraryError::Missing);
    }
    if meta.len() > MAX_FILE_BYTES {
        ferroan_diag::refuse(
            LIBRARY_FILE_BYTES,
            Kind::Hard,
            MAX_FILE_BYTES,
            meta.len(),
            "library",
        );
        return Err(LibraryError::TooLarge);
    }
    let next_total = total.saturating_add(meta.len());
    if next_total > MAX_TOTAL_BYTES {
        ferroan_diag::refuse(
            LIBRARY_TOTAL_BYTES,
            Kind::Hard,
            MAX_TOTAL_BYTES,
            next_total,
            "library",
        );
        return Err(LibraryError::TooLarge);
    }
    let bytes = fs::read(path).map_err(|_| LibraryError::Io)?;
    *files += 1;
    *total = total.saturating_add(bytes.len() as u64);
    let text = String::from_utf8_lossy(&bytes);
    Ok(parse_nini(&strip_comments(&text)))
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start + 4..].find("-->") {
            Some(end) => &rest[start + 4 + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn parse_nini(text: &str) -> Vec<HashMap<String, String>> {
    let mut sections = Vec::new();
    let mut current: Option<HashMap<String, String>> = None;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        let rest = &text[index..];
        if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("</Section") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            index += 9;
            continue;
        }
        if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case("<Section") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(HashMap::new());
            index += 8;
            continue;
        }
        if rest.len() >= 4
            && rest[..4].eq_ignore_ascii_case("<Key")
            && let Some(end) = rest.find('>')
        {
            let tag = &rest[..end];
            if let Some(section) = current.as_mut()
                && let (Some(name), Some(value)) = (attr(tag, "Name"), attr(tag, "Value"))
            {
                section.insert(name, decode_entities(&value));
            }
            index += end + 1;
            continue;
        }
        index += 1;
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = name.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(&key) {
        let name_at = from + found;
        let after = name_at + key.len();
        let tail = lower[after..].trim_start();
        if tail.starts_with('=') {
            let equals = after + (lower[after..].len() - tail.len());
            let value = tag.get(equals + 1..)?.trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let rest = &value[quote.len_utf8()..];
                let end = rest.find(quote)?;
                return Some(rest[..end].to_owned());
            }
        }
        from = after;
    }
    None
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn text_value(value: Option<&String>, default: &str) -> String {
    value.cloned().unwrap_or_else(|| default.to_owned())
}

fn short_value(value: Option<&String>, default: i32) -> i32 {
    i32::from(int_value(value, default) as i16)
}

fn ushort_value(value: Option<&String>, default: i32) -> i32 {
    i32::from(int_value(value, default) as u16)
}

fn int_value(value: Option<&String>, default: i32) -> i32 {
    value
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(default)
}

fn long_value(value: Option<&String>, default: u32) -> u32 {
    value
        .and_then(|text| text.trim().parse::<i64>().ok())
        .map(|number| number as u32)
        .unwrap_or(default)
}

fn unix_now() -> i32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i32::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

fn uuid_value(value: Option<&String>, default: &str) -> Uuid {
    value
        .and_then(|text| Uuid::parse_str(text.trim()).ok())
        .or_else(|| Uuid::parse_str(default).ok())
        .unwrap_or(Uuid::nil())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_version_and_group_permissions_follow_the_nini_quirk() {
        let dir = std::env::temp_dir().join(format!("ferroan-library-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("Libraries.xml"),
            r#"<Nini>
  <Section Name="OpenSim Standard Library">
    <Key Name="RootVersion" Value="429"/>
  </Section>
  <!-- <Section Name="Ignored"><Key Name="foldersFile" Value="nope.xml"/></Section> -->
  <Section Name="Textures Library">
    <Key Name="foldersFile" Value="folders.xml"/>
    <Key Name="itemsFile" Value="items.xml"/>
  </Section>
</Nini>"#,
        )
        .unwrap();
        fs::write(
            dir.join("folders.xml"),
            r#"<Nini><Section Name="Texture Library">
    <Key Name="folderID" Value="00000112-000f-0000-0000-000100bba001"/>
    <Key Name="parentFolderID" Value="00000112-000f-0000-0000-000100bba000"/>
    <Key Name="name" Value="Texture Library"/>
    <Key Name="type" Value="0"/>
  </Section></Nini>"#,
        )
        .unwrap();
        fs::write(
            dir.join("items.xml"),
            r#"<Nini>
  <Section Name="plain">
    <Key Name="inventoryID" Value="00000000-0000-2222-4444-100000001000"/>
    <Key Name="assetID" Value="00000000-0000-2222-3333-100000001000"/>
    <Key Name="folderID" Value="00000112-000f-0000-0000-000100bba001"/>
    <Key Name="name" Value="4-tile2"/>
    <Key Name="description" Value="4-tile2 texture"/>
  </Section>
  <Section Name="quirk">
    <Key Name="inventoryID" Value="00000000-0000-2222-4444-100000001001"/>
    <Key Name="folderID" Value="00000112-000f-0000-0000-000100bba001"/>
    <Key Name="name" Value="quirk"/>
    <Key Name="basePermissions" Value="123"/>
  </Section>
</Nini>"#,
        )
        .unwrap();
        let library = Library::open(&dir.join("Libraries.xml"), "OpenSim Library").unwrap();
        assert_eq!(library.folder(library.root_id()).unwrap().version, 429);
        let folder = Uuid::parse_str("00000112-000f-0000-0000-000100bba001").unwrap();
        assert_eq!(library.child_folders(library.root_id()).len(), 1);
        assert_eq!(library.child_items(folder).len(), 2);
        let plain = library
            .item(Uuid::parse_str("00000000-0000-2222-4444-100000001000").unwrap())
            .unwrap();
        assert_eq!(plain.base_permissions, BASE_PERMISSIONS);
        assert_eq!(plain.group_permissions, 0);
        assert_eq!(plain.description, "4-tile2 texture");
        let quirk = library
            .item(Uuid::parse_str("00000000-0000-2222-4444-100000001001").unwrap())
            .unwrap();
        assert_eq!(quirk.base_permissions, 123);
        assert_eq!(quirk.group_permissions, 123);
        assert_eq!(quirk.asset_id, quirk.id);
        assert!(quirk.creation_date > 1_700_000_000);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_control_file_is_an_error_without_its_path() {
        let error = Library::open(Path::new("/no/such/ferroan-library.xml"), "OpenSim Library");
        assert!(matches!(error, Err(LibraryError::Missing)));
        assert!(!error.unwrap_err().to_string().contains('/'));
    }

    /// The shipped library, when the pinned checkout is present. Ordinary CI
    /// skips this; the live harness compares the same file through the region.
    #[test]
    fn the_pinned_library_loads_when_the_checkout_is_present() {
        const PIN: &str = "c70fee205f64ee8672a8d607cef6a733931da951";
        let Some(root) = reference_root() else {
            eprintln!("reference checkout not found; the shipped library was not loaded");
            return;
        };
        let checkout = root.join("OpenSim-Tranquillity");
        match checkout_head(&checkout) {
            Some(head) if head == PIN => {}
            Some(head) => {
                eprintln!(
                    "OpenSim-Tranquillity is at {head}, not {PIN}; the shipped library was not loaded"
                );
                return;
            }
            None => {
                eprintln!("could not read the revision; the shipped library was not loaded");
                return;
            }
        }
        let path = checkout
            .join("Source/OpenSim.Services.InventoryService/AppData/inventory/Libraries.xml");
        let library = Library::open(&path, "OpenSim Library").unwrap_or_else(|error| {
            panic!("shipped library did not load: {error}");
        });
        assert_eq!(library.folder(library.root_id()).unwrap().version, 429);
        let item = library
            .item(Uuid::parse_str("00000000-0000-2222-4444-100000001000").unwrap())
            .expect("4-tile2");
        assert_eq!(item.name, "4-tile2");
        assert_eq!(
            item.asset_id,
            Uuid::parse_str("00000000-0000-2222-3333-100000001000").unwrap()
        );
        assert!(library.child_folders(library.root_id()).len() > 1);
        assert!(library.items.len() > 10);
    }

    fn reference_root() -> Option<PathBuf> {
        let set = std::env::var("FERROAN_REF")
            .ok()
            .filter(|value| !value.is_empty());
        let path = if let Some(value) = set {
            if let Some(rest) = value.strip_prefix("~/") {
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .map(PathBuf::from)?
                    .join(rest)
            } else {
                PathBuf::from(value)
            }
        } else {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            PathBuf::from(home).join("Git/reference")
        };
        path.is_dir().then_some(path)
    }

    fn checkout_head(checkout: &Path) -> Option<String> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?;
        let head = text.trim();
        if head.len() == 40 && head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Some(head.to_string())
        } else {
            None
        }
    }
}
