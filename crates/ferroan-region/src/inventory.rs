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
//   Source/OpenSim.Services.Connectors/Inventory/XInventoryServicesConnector.cs
//   Source/OpenSim.Server.Handlers/Inventory/XInventoryInConnector.cs
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Robust `xinventory` client. The region keeps no inventory store of its own.
//! A non-success HTTP status is a transport failure. `CheckReturn` false is a
//! null service result. Reply bodies are not copied into errors.

use std::time::Duration;

use ferroan_diag::{self, INVENTORY_REPLY, Kind};
use ferroan_http::{Field, FieldMap, build_query_string, parse_xml_response};
use uuid::Uuid;

const MAX_REPLY: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("inventory service request failed")]
    Transport,
    #[error("inventory service reply could not be read")]
    Reply,
}

#[derive(Clone, Debug)]
pub struct InvFolder {
    pub id: Uuid,
    pub parent_id: Uuid,
    pub name: String,
    pub type_code: i32,
    pub version: i32,
    pub owner: Uuid,
}

#[derive(Clone, Debug)]
pub struct InvItem {
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
pub struct Collection {
    pub folder_id: Uuid,
    pub owner_id: Uuid,
    pub version: i32,
    pub folders: Vec<InvFolder>,
    pub items: Vec<InvItem>,
}

#[derive(Clone)]
pub struct InventoryClient {
    endpoint: String,
    http: reqwest::Client,
}

impl InventoryClient {
    pub fn endpoint(service_uri: &str) -> String {
        let trimmed = service_uri.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        if trimmed.ends_with('/') {
            format!("{trimmed}xinventory")
        } else {
            format!("{trimmed}/xinventory")
        }
    }

    pub fn new(service_uri: &str, http: reqwest::Client) -> Option<Self> {
        let endpoint = Self::endpoint(service_uri);
        if endpoint.is_empty() {
            None
        } else {
            Some(Self { endpoint, http })
        }
    }

    /// `None` is a null connector result (`CheckReturn` false). A transport
    /// error is `Err`; the caller treats that as one null collection per id.
    pub async fn folders_content(
        &self,
        principal: Uuid,
        ids: &[Uuid],
    ) -> Result<Option<Vec<Option<Collection>>>, InventoryError> {
        let folders = ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(
            "METHOD=GETMULTIPLEFOLDERSCONTENT&PRINCIPAL={principal}&FOLDERS={folders}&COUNT={}",
            ids.len()
        );
        let reply = self.post_raw(&body).await?;
        if !check_return(&reply) {
            return Ok(None);
        }
        let mut collections = Vec::with_capacity(ids.len());
        for id in ids {
            collections.push(collection_at(&reply, id));
        }
        Ok(Some(collections))
    }

    /// Aligned with `ids`. A null slot is a missing item. `Ok(None)` means the
    /// connector returned null.
    pub async fn items(
        &self,
        principal: Uuid,
        ids: &[Uuid],
    ) -> Result<Option<Vec<Option<InvItem>>>, InventoryError> {
        if ids.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let listed = ids
            .iter()
            .map(Uuid::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(
            "METHOD=GETMULTIPLEITEMS&PRINCIPAL={principal}&ITEMS={listed}&COUNT={}",
            ids.len()
        );
        let reply = self.post_raw(&body).await?;
        if !check_return(&reply) {
            return Ok(None);
        }
        let mut slots = Vec::new();
        for (key, value) in &reply {
            if !key.starts_with("item_") {
                continue;
            }
            slots.push(match value {
                Field::Map(map) => Some(build_item(map)),
                _ => None,
            });
        }
        Ok(Some(slots))
    }

    pub async fn folder(
        &self,
        principal: Uuid,
        folder: Uuid,
    ) -> Result<Option<InvFolder>, InventoryError> {
        let body = format!("METHOD=GETFOLDER&ID={folder}&PRINCIPAL={principal}");
        let reply = self.post_raw(&body).await?;
        if !check_return(&reply) {
            return Ok(None);
        }
        Ok(match reply.get("folder") {
            Some(Field::Map(map)) => Some(build_folder(map)),
            _ => None,
        })
    }

    pub async fn add_item(&self, item: &InvItem) -> Result<bool, InventoryError> {
        let mut fields = FieldMap::new();
        let mut put = |key: &str, value: String| {
            fields.insert(key.to_owned(), Field::Str(value));
        };
        put("METHOD", "ADDITEM".to_owned());
        put("AssetID", item.asset_id.to_string());
        put("AssetType", item.asset_type.to_string());
        put("Name", item.name.clone());
        put("Owner", item.owner.to_string());
        put("ID", item.id.to_string());
        put("InvType", item.inv_type.to_string());
        put("Folder", item.folder.to_string());
        put("CreatorId", item.creator_id.clone());
        put("CreatorData", String::new());
        put("Description", item.description.clone());
        put("NextPermissions", item.next_permissions.to_string());
        put("CurrentPermissions", item.current_permissions.to_string());
        put("BasePermissions", item.base_permissions.to_string());
        put("EveryOnePermissions", item.everyone_permissions.to_string());
        put("GroupPermissions", item.group_permissions.to_string());
        put("GroupID", item.group_id.to_string());
        put(
            "GroupOwned",
            if item.group_owned { "True" } else { "False" }.to_owned(),
        );
        put("SalePrice", item.sale_price.to_string());
        put("SaleType", item.sale_type.to_string());
        put("Flags", item.flags.to_string());
        put("CreationDate", item.creation_date.to_string());
        put("ExperienceID", Uuid::nil().to_string());
        let body = build_query_string(&fields).map_err(|_| InventoryError::Transport)?;
        let reply = self.post_raw(&body).await?;
        Ok(check_return(&reply))
    }

    async fn post_raw(&self, body: &str) -> Result<FieldMap, InventoryError> {
        let response = self
            .http
            .post(&self.endpoint)
            .timeout(Duration::from_secs(30))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body.to_owned())
            .send()
            .await
            .map_err(|_| InventoryError::Transport)?;
        if !response.status().is_success() {
            return Err(InventoryError::Transport);
        }
        let bytes = read_limited(response).await?;
        let text = std::str::from_utf8(&bytes).map_err(|_| InventoryError::Reply)?;
        parse_xml_response(text).map_err(|_| InventoryError::Reply)
    }
}

async fn read_limited(mut response: reqwest::Response) -> Result<Vec<u8>, InventoryError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| InventoryError::Reply)? {
        let next = bytes.len().saturating_add(chunk.len());
        if next > MAX_REPLY {
            ferroan_diag::refuse(
                INVENTORY_REPLY,
                Kind::Hard,
                MAX_REPLY as u64,
                next as u64,
                "inventory",
            );
            return Err(InventoryError::Reply);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) fn check_return(reply: &FieldMap) -> bool {
    if reply.is_empty() {
        return false;
    }
    match reply.get("RESULT") {
        Some(Field::Str(text)) => matches!(text.to_ascii_lowercase().as_str(), "true"),
        Some(_) => true,
        None => true,
    }
}

fn collection_at(reply: &FieldMap, id: &Uuid) -> Option<Collection> {
    let Field::Map(folder) = reply.get(&format!("F_{id}"))? else {
        return None;
    };
    let folder_id = uuid_field(folder, "FID")?;
    let owner_id = uuid_field(folder, "OWNER")?;
    let version = match folder.get("VERSION") {
        Some(Field::Str(text)) => text.trim().parse().unwrap_or(-1),
        _ => -1,
    };
    let mut folders = Vec::new();
    if let Some(Field::Map(children)) = folder.get("FOLDERS") {
        for value in children.values() {
            if let Field::Map(child) = value {
                folders.push(build_folder(child));
            }
        }
    }
    let mut items = Vec::new();
    if let Some(Field::Map(children)) = folder.get("ITEMS") {
        for value in children.values() {
            if let Field::Map(child) = value {
                items.push(build_item(child));
            }
        }
    }
    Some(Collection {
        folder_id,
        owner_id,
        version,
        folders,
        items,
    })
}

fn build_folder(data: &FieldMap) -> InvFolder {
    InvFolder {
        parent_id: uuid_field(data, "ParentID").unwrap_or(Uuid::nil()),
        type_code: i32::from(int_field(data, "Type").unwrap_or(0) as i16),
        version: i32::from(int_field(data, "Version").unwrap_or(0) as u16),
        name: text_field(data, "Name").unwrap_or_default().to_owned(),
        owner: uuid_field(data, "Owner").unwrap_or(Uuid::nil()),
        id: uuid_field(data, "ID").unwrap_or(Uuid::nil()),
    }
}

fn build_item(data: &FieldMap) -> InvItem {
    InvItem {
        asset_id: uuid_field(data, "AssetID").unwrap_or(Uuid::nil()),
        asset_type: int_field(data, "AssetType").unwrap_or(0),
        name: text_field(data, "Name").unwrap_or_default().to_owned(),
        owner: uuid_field(data, "Owner").unwrap_or(Uuid::nil()),
        id: uuid_field(data, "ID").unwrap_or(Uuid::nil()),
        inv_type: int_field(data, "InvType").unwrap_or(0),
        folder: uuid_field(data, "Folder").unwrap_or(Uuid::nil()),
        creator_id: text_field(data, "CreatorId").unwrap_or_default().to_owned(),
        next_permissions: uint_field(data, "NextPermissions"),
        current_permissions: uint_field(data, "CurrentPermissions"),
        base_permissions: uint_field(data, "BasePermissions"),
        everyone_permissions: uint_field(data, "EveryOnePermissions"),
        group_permissions: uint_field(data, "GroupPermissions"),
        group_id: uuid_field(data, "GroupID").unwrap_or(Uuid::nil()),
        group_owned: matches!(text_field(data, "GroupOwned"), Some(text) if text.eq_ignore_ascii_case("true")),
        sale_price: int_field(data, "SalePrice").unwrap_or(0),
        sale_type: int_field(data, "SaleType").unwrap_or(0),
        flags: uint_field(data, "Flags"),
        creation_date: int_field(data, "CreationDate").unwrap_or(0),
        description: text_field(data, "Description")
            .unwrap_or_default()
            .to_owned(),
    }
}

fn text_field<'a>(map: &'a FieldMap, key: &str) -> Option<&'a str> {
    match map.get(key) {
        Some(Field::Str(text) | Field::Uuid(text)) => Some(text),
        _ => None,
    }
}

fn uuid_field(map: &FieldMap, key: &str) -> Option<Uuid> {
    text_field(map, key).and_then(|text| Uuid::parse_str(text.trim()).ok())
}

fn int_field(map: &FieldMap, key: &str) -> Option<i32> {
    text_field(map, key).and_then(|text| text.trim().parse().ok())
}

fn uint_field(map: &FieldMap, key: &str) -> u32 {
    text_field(map, key)
        .and_then(|text| text.trim().parse::<u64>().ok())
        .map(|value| value as u32)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_return_matches_the_connector() {
        let mut empty = FieldMap::new();
        assert!(!check_return(&empty));
        empty.insert("VERSION".into(), Field::Str("1".into()));
        assert!(check_return(&empty));
        empty.insert("RESULT".into(), Field::Str("true".into()));
        assert!(check_return(&empty));
        empty.insert("RESULT".into(), Field::Str("no".into()));
        assert!(!check_return(&empty));
        empty.insert("RESULT".into(), Field::Int(0));
        assert!(check_return(&empty));
    }

    #[test]
    fn folder_content_is_looked_up_by_the_requested_id() {
        let id = Uuid::parse_str("00000000-0000-4000-8000-000000000010").unwrap();
        let xml = format!(
            r#"<?xml version="1.0"?><ServerResponse>
            <RESULT>True</RESULT>
            <F_{id} type="List">
              <FID>{id}</FID>
              <OWNER>00000000-0000-4000-8000-000000000001</OWNER>
              <VERSION>3</VERSION>
              <FOLDERS type="List">
                <folder_0 type="List">
                  <ParentID>{id}</ParentID>
                  <Type>0</Type>
                  <Version>1</Version>
                  <Name>Textures</Name>
                  <Owner>00000000-0000-4000-8000-000000000001</Owner>
                  <ID>00000000-0000-4000-8000-000000000011</ID>
                </folder_0>
              </FOLDERS>
              <ITEMS type="List" />
            </F_{id}>
            </ServerResponse>"#
        );
        let reply = parse_xml_response(&xml).unwrap();
        let collection = collection_at(&reply, &id).unwrap();
        assert_eq!(collection.version, 3);
        assert_eq!(collection.folders.len(), 1);
        assert_eq!(collection.folders[0].name, "Textures");
        assert!(collection.items.is_empty());
    }
}
