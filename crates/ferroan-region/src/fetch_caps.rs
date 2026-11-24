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
//   Source/OpenSim.Capabilities.Handlers/FetchInventory/FetchInvDescHandler.cs
//   Source/OpenSim.Capabilities.Handlers/FetchInventory/FetchLibDescHandler.cs
//   Source/OpenSim.Capabilities.Handlers/FetchInventory/FetchInventory2Handler.cs
//   Source/OpenSim.Capabilities.Handlers/FetchInventory/FetchLib2Handler.cs
//   Source/OpenSim.Capabilities.Handlers/GetAssets/GetAssetsHandler.cs
//   Source/OpenSim.Framework/InventoryItemBase.cs (ToLLSDxml)
//   Source/OpenSim.Framework/Util.cs (TryParseHttpRange)
//   Source/OpenSim.Framework/ScriptCompileTarget.cs (ForViewer)
//   Source/OpenSim.Region.ClientStack.LindenCaps/BunchOfCaps/BunchOfCaps.cs
//   KanikaBreed/OpenSim-Tranquillity @ 40432e0f83d3802434f898a4e28687bd977d5cf5
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Inventory and asset capabilities. User inventory and asset bytes come from
//! Robust. The library is the in-process Nini tree. Requests past the folder,
//! item, asset and LLSD bounds are refused rather than truncated. Each offered
//! uploader path stays until that upload is posted or its 120 second timer
//! fires. An agent may hold more than one. At most 128 waiting uploads are kept.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use ferroan_client::{CapAnswer, CapExchange, CapService, UploaderRoutes};
use ferroan_diag::{
    self, INVENTORY_FETCH_FOLDERS, INVENTORY_FETCH_ITEMS, INVENTORY_FETCH_LLSD, Kind,
};
use ferroan_llsd::{Llsd, from_xml, to_xml};
use ferroan_services::{Asset, content_type};
use uuid::Uuid;

use crate::config::RegionConfig;
use crate::inventory::{Collection, InvFolder, InvItem, InventoryClient};
use crate::library::{Library, LibraryItem};
use crate::remote_asset::AssetClient;

const MAX_FOLDERS: usize = 64;
const MAX_ITEM_IDS: usize = 256;
const MAX_LLSD: usize = 2 * 1024 * 1024;
const BAD_LIMIT: usize = 256;
const BAD_TTL: Duration = Duration::from_secs(30);
const UPLOAD_TTL: Duration = Duration::from_secs(120);
/// Waiting uploader paths across every agent. A further offer is refused.
const MAX_PENDING_UPLOADS: usize = 128;
const LSL_ON_LUAU: u32 = 0x0100_0000;
const FLAGS_FETCH2: u32 = 0xff;
const PERM_ALL: u32 = 0x8e000;
const PERM_EXPORT: u32 = 0x10000;
const PERM_TRANSFER: u32 = 0x2000;
const LLSD_MIME: &str = "application/llsd+xml";
const TEXT: &str = "text/plain";

const QUERY_TYPES: &[(&str, i8)] = &[
    ("texture_id", 0),
    ("sound_id", 1),
    ("callcard_id", 2),
    ("landmark_id", 3),
    ("script_id", 10),
    ("clothing_id", 5),
    ("object_id", 6),
    ("notecard_id", 7),
    ("lsltext_id", 10),
    ("lslbyte_id", 11),
    ("txtr_tga_id", 12),
    ("bodypart_id", 13),
    ("snd_wav_id", 17),
    ("img_tga_id", 18),
    ("jpeg_id", 19),
    ("animatn_id", 20),
    ("gesture_id", 21),
    ("mesh_id", 49),
    ("settings_id", 56),
    ("material_id", 57),
];

struct FolderRequest {
    folder_id: Uuid,
    owner_id: Uuid,
    fetch_folders: bool,
    fetch_items: bool,
}

struct PendingUpload {
    agent_id: Uuid,
    path: String,
    asset_id: Uuid,
    item_id: Uuid,
    name: String,
    description: String,
    folder: Uuid,
    asset_type: i8,
    inv_type: i32,
    next_owner: u32,
    group_mask: u32,
    everyone_mask: u32,
    expires: Instant,
    storing: bool,
}

pub struct FetchCaps {
    inventory: Option<InventoryClient>,
    assets: Option<AssetClient>,
    library: Library,
    bad: Mutex<Vec<(Uuid, Instant)>>,
    upload: Mutex<Vec<PendingUpload>>,
    routes: std::sync::OnceLock<UploaderRoutes>,
    public_base: String,
}

impl FetchCaps {
    pub fn new(config: &RegionConfig) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build();
        let http = http.ok();
        let library = if !config.library_enabled {
            Library::absent()
        } else {
            match &config.library_file {
                Some(path) => match Library::open(path, &config.library_name) {
                    Ok(library) => library,
                    Err(error) => {
                        eprintln!("ferroan-region: {error}");
                        Library::root_only(&config.library_name)
                    }
                },
                None => Library::root_only(&config.library_name),
            }
        };
        let (inventory, assets) = match http {
            Some(http) => (
                InventoryClient::new(&config.inventory_server_uri, http.clone()),
                AssetClient::new(&config.asset_server_uri, http),
            ),
            None => (None, None),
        };
        Self {
            inventory,
            assets,
            library,
            bad: Mutex::new(Vec::new()),
            upload: Mutex::new(Vec::new()),
            routes: std::sync::OnceLock::new(),
            public_base: config.server_uri.trim_end_matches('/').to_owned(),
        }
    }

    /// Register uploader paths on the agent that asked for them.
    pub fn bind_uploader_routes(&self, routes: UploaderRoutes) {
        let _ = self.routes.set(routes);
    }

    fn routes(&self) -> UploaderRoutes {
        self.routes.get().cloned().unwrap_or_default()
    }

    pub fn service(self: &Arc<Self>) -> CapService {
        let this = Arc::clone(self);
        Arc::new(move |exchange| {
            let this = Arc::clone(&this);
            Box::pin(async move { this.handle(exchange).await })
        })
    }

    async fn handle(&self, exchange: CapExchange) -> CapAnswer {
        match exchange.name.as_str() {
            "FetchInventoryDescendents2" => self.descendents(&exchange, false).await,
            "FetchLibDescendents2" => self.descendents(&exchange, true).await,
            "FetchInventory2" => self.fetch_items(&exchange, false).await,
            "FetchLib2" => self.fetch_items(&exchange, true).await,
            "GetTexture" | "GetMesh" | "GetMesh2" | "ViewerAsset" => self.asset(&exchange).await,
            "NewFileAgentInventory" => self.upload_offer(&exchange),
            "" => self.upload_complete(&exchange).await,
            _ => CapAnswer::miss(),
        }
    }

    async fn descendents(&self, exchange: &CapExchange, library_only: bool) -> CapAnswer {
        if exchange.method != "POST" {
            return CapAnswer::miss();
        }
        let Ok(body) = from_xml(&exchange.body) else {
            ferroan_diag::log_malformed(&exchange.name, &exchange.body, "body is not llsd xml");
            return llsd_ok(empty_folders());
        };
        let folders = body.get("folders");
        if !folders.is_array() && body.get("folders").is_undef() {
            return llsd_ok(empty_folders());
        }
        if folders.as_array().len() > MAX_FOLDERS {
            let agent = exchange.agent_id.to_string();
            ferroan_diag::hit(&ferroan_diag::Hit {
                id: INVENTORY_FETCH_FOLDERS,
                kind: Kind::Hard,
                configured: MAX_FOLDERS as u64,
                observed: folders.as_array().len() as u64,
                action: ferroan_diag::Action::Refuse,
                route: &exchange.name,
                agent_id: Some(&agent),
            });
            return refused();
        }
        let mut requested = Vec::new();
        let mut bad = Vec::new();
        for folder in folders.as_array() {
            if !folder.is_map() {
                continue;
            }
            let id = folder.get("folder_id").as_uuid();
            if self.is_bad(id) {
                bad.push(id);
                continue;
            }
            requested.push(FolderRequest {
                folder_id: id,
                owner_id: folder.get("owner_id").as_uuid(),
                fetch_folders: folder.get("fetch_folders").as_bool(),
                fetch_items: folder.get("fetch_items").as_bool(),
            });
        }
        if requested.is_empty() {
            return self.finish_folders(Vec::new(), bad);
        }
        if library_only {
            return self.library_descendents(requested, bad, exchange.agent_id);
        }
        self.user_descendents(requested, bad).await
    }

    fn library_descendents(
        &self,
        requested: Vec<FolderRequest>,
        mut bad: Vec<Uuid>,
        agent_id: Uuid,
    ) -> CapAnswer {
        let mut seen = Vec::new();
        let mut collections = Vec::new();
        let mut zero = false;
        for request in requested {
            if request.folder_id.is_nil() {
                if zero {
                    continue;
                }
                zero = true;
                // FetchLibDesc encodes the logged-in agent and the request owner.
                collections.push(folder_llsd(
                    Uuid::nil(),
                    agent_id,
                    request.owner_id,
                    -1,
                    0,
                    Llsd::new_array(),
                    Llsd::new_array(),
                ));
                continue;
            }
            if request.owner_id != self.library.owner() || seen.contains(&request.folder_id) {
                continue;
            }
            seen.push(request.folder_id);
            match self.library.folder(request.folder_id) {
                Some(folder) => {
                    collections.push(library_collection(&self.library, folder, agent_id))
                }
                None => bad.push(request.folder_id),
            }
        }
        self.finish_folders(collections, bad)
    }

    async fn user_descendents(
        &self,
        requested: Vec<FolderRequest>,
        mut bad: Vec<Uuid>,
    ) -> CapAnswer {
        let mut seen_library = Vec::new();
        let mut seen_other = Vec::new();
        let mut library_requests = Vec::new();
        let mut other = Vec::new();
        let mut collections = Vec::new();
        let mut zero = false;
        for request in requested {
            if request.folder_id.is_nil() {
                if zero {
                    continue;
                }
                zero = true;
                collections.push(zero_collection(request.owner_id, 0));
                continue;
            }
            if self.library.is_loaded() && request.owner_id == self.library.owner() {
                if seen_library.contains(&request.folder_id) {
                    continue;
                }
                seen_library.push(request.folder_id);
                library_requests.push(request);
                continue;
            }
            if seen_other.contains(&request.folder_id) {
                continue;
            }
            seen_other.push(request.folder_id);
            other.push(request);
        }
        if !other.is_empty() {
            let owner = other[0].owner_id;
            let ids: Vec<_> = other.iter().map(|request| request.folder_id).collect();
            match self.inventory_folders(owner, &ids).await {
                None => return self.finish_folders(Vec::new(), bad),
                Some(fetched) if fetched.is_empty() => {
                    for request in &other {
                        bad.push(request.folder_id);
                    }
                }
                Some(fetched) => {
                    for (request, contents) in other.iter().zip(fetched) {
                        let Some(mut contents) = contents else {
                            bad.push(request.folder_id);
                            continue;
                        };
                        if contents.folder_id.is_nil() {
                            match self.inventory_folder(owner, request.folder_id).await {
                                Some(folder) => {
                                    contents.folder_id = folder.id;
                                    contents.owner_id = folder.owner;
                                    contents.version = folder.version;
                                }
                                None => {
                                    bad.push(request.folder_id);
                                    continue;
                                }
                            }
                        }
                        if !request.fetch_folders {
                            contents.folders.clear();
                        }
                        if !request.fetch_items {
                            contents.items.clear();
                        }
                        let descendents =
                            i32::try_from(contents.items.len() + contents.folders.len())
                                .unwrap_or(i32::MAX);
                        self.expand_links(owner, &mut contents).await;
                        collections.push(user_collection(contents, descendents));
                    }
                }
            }
        }
        for request in library_requests {
            if let Some(folder) = self.library.folder(request.folder_id) {
                // FetchInvDesc writes the collection owner into both agent and owner.
                collections.push(library_collection(
                    &self.library,
                    folder,
                    self.library.owner(),
                ));
            }
        }
        self.finish_folders(collections, bad)
    }

    async fn inventory_folders(
        &self,
        owner: Uuid,
        ids: &[Uuid],
    ) -> Option<Vec<Option<Collection>>> {
        let Some(client) = &self.inventory else {
            return Some(vec![None; ids.len()]);
        };
        match client.folders_content(owner, ids).await {
            Ok(Some(collections)) => Some(collections),
            Ok(None) => None,
            Err(_) => Some(vec![None; ids.len()]),
        }
    }

    async fn inventory_folder(&self, owner: Uuid, id: Uuid) -> Option<InvFolder> {
        self.inventory
            .as_ref()?
            .folder(owner, id)
            .await
            .ok()
            .flatten()
    }

    async fn expand_links(&self, owner: Uuid, contents: &mut Collection) {
        let links: Vec<Uuid> = contents
            .items
            .iter()
            .filter(|item| item.asset_type == 24)
            .map(|item| item.asset_id)
            .collect();
        if links.is_empty() {
            return;
        }
        let Some(client) = &self.inventory else {
            return;
        };
        let Ok(Some(linked)) = client.items(owner, &links).await else {
            return;
        };
        let mut prefix = Vec::new();
        for item in linked.into_iter().flatten() {
            if item.asset_type != 24 {
                prefix.push(item);
            }
        }
        if !prefix.is_empty() {
            prefix.append(&mut contents.items);
            contents.items = prefix;
        }
    }

    fn finish_folders(&self, collections: Vec<Llsd>, bad: Vec<Uuid>) -> CapAnswer {
        let mut folders = Llsd::new_array();
        for collection in collections {
            folders.push(collection);
        }
        let mut map = Llsd::new_map();
        map.insert("folders", folders);
        if !bad.is_empty() {
            let mut listed = Llsd::new_array();
            for id in bad {
                self.remember_bad(id);
                let mut one = Llsd::new_map();
                one.insert("folder_id", id);
                one.insert("error", "Unknown");
                listed.push(one);
            }
            map.insert("bad_folders", listed);
        }
        llsd_ok(map)
    }

    async fn fetch_items(&self, exchange: &CapExchange, library: bool) -> CapAnswer {
        if exchange.method != "POST" {
            return CapAnswer::miss();
        }
        if library && (!self.library.is_loaded() || exchange.agent_id.is_nil()) {
            return plain(StatusCode::NOT_FOUND, Vec::new());
        }
        let Ok(body) = from_xml(&exchange.body) else {
            ferroan_diag::log_malformed(&exchange.name, &exchange.body, "body is not llsd xml");
            return refused();
        };
        let items = body.get("items");
        if !items.is_array() {
            return refused();
        }
        if items.as_array().len() > MAX_ITEM_IDS {
            let agent = exchange.agent_id.to_string();
            ferroan_diag::hit(&ferroan_diag::Hit {
                id: INVENTORY_FETCH_ITEMS,
                kind: Kind::Hard,
                configured: MAX_ITEM_IDS as u64,
                observed: items.as_array().len() as u64,
                action: ferroan_diag::Action::Refuse,
                route: &exchange.name,
                agent_id: Some(&agent),
            });
            return refused();
        }
        let mut ids = Vec::new();
        for item in items.as_array() {
            let id = item.get("item_id").as_uuid();
            if !self.is_bad(id) {
                ids.push(id);
            }
        }
        let mask = if library { u32::MAX } else { FLAGS_FETCH2 };
        let shown = if library {
            self.library
                .items_by_id(&ids)
                .unwrap_or_default()
                .into_iter()
                .map(library_as_item)
                .collect()
        } else {
            match self.user_items(exchange.agent_id, &ids).await {
                Some(items) => items.into_iter().flatten().collect(),
                None => Vec::new(),
            }
        };
        let mut array = Llsd::new_array();
        for item in &shown {
            array.push(item_llsd(item, mask));
        }
        let mut map = Llsd::new_map();
        map.insert("agent_id", exchange.agent_id);
        map.insert("items", array);
        llsd_ok(map)
    }

    async fn user_items(&self, agent: Uuid, ids: &[Uuid]) -> Option<Vec<Option<InvItem>>> {
        if ids.is_empty() {
            return Some(Vec::new());
        }
        let client = self.inventory.as_ref()?;
        client.items(agent, ids).await.unwrap_or(None)
    }

    async fn asset(&self, exchange: &CapExchange) -> CapAnswer {
        if !matches!(exchange.method.as_str(), "GET" | "POST") {
            return CapAnswer::miss();
        }
        let Some(client) = &self.assets else {
            return plain(StatusCode::SERVICE_UNAVAILABLE, Vec::new());
        };
        if exchange.query.is_empty() {
            return plain(StatusCode::BAD_REQUEST, Vec::new());
        }
        let Ok(query) = ferroan_http::parse_query_string(&exchange.query) else {
            return plain(StatusCode::BAD_REQUEST, Vec::new());
        };
        let mut found = None;
        for (key, value) in &query {
            if let Some((_, type_code)) = QUERY_TYPES.iter().find(|(name, _)| name == key) {
                let text = match value {
                    ferroan_http::Field::Str(text) | ferroan_http::Field::Uuid(text) => {
                        text.as_str()
                    }
                    _ => "",
                };
                found = Some((*type_code, text));
                break;
            }
        }
        let Some((type_code, asset_text)) = found else {
            return plain(StatusCode::NOT_FOUND, Vec::new());
        };
        if asset_text.is_empty() {
            return plain(StatusCode::BAD_REQUEST, Vec::new());
        }
        let Ok(asset_id) = Uuid::parse_str(asset_text.trim()) else {
            return plain(StatusCode::BAD_REQUEST, Vec::new());
        };
        let asset = match client.get(asset_id).await {
            Ok(Some(asset)) => asset,
            Ok(None) => return plain(StatusCode::NOT_FOUND, Vec::new()),
            Err(_) => return plain(StatusCode::NOT_FOUND, Vec::new()),
        };
        if type_code == 57 && asset.type_code == 0 {
            return plain(StatusCode::NOT_FOUND, Vec::new());
        }
        let (status, body, content_range) = slice_range(exchange.range.as_deref(), &asset.data);
        CapAnswer {
            handled: true,
            status,
            content_type: content_type(asset.type_code).to_owned(),
            body: body.to_vec(),
            content_range,
        }
    }

    fn upload_offer(&self, exchange: &CapExchange) -> CapAnswer {
        if exchange.method != "POST" {
            return CapAnswer::miss();
        }
        let Ok(body) = from_xml(&exchange.body) else {
            return upload_error("Could not parse the upload request");
        };
        let asset_type = body.get("asset_type").as_str();
        let inventory_type = body.get("inventory_type").as_str();
        let mesh_fee = asset_type == "mesh";
        let kind = upload_kind(inventory_type, asset_type);
        let Some((asset_code, inv_code)) = kind else {
            return upload_error("this upload type is not served by this capability");
        };
        let path = format!("/{}", Uuid::new_v4());
        let asset_id = Uuid::new_v4();
        let item_id = Uuid::new_v4();
        let pending = PendingUpload {
            agent_id: exchange.agent_id,
            path: path.clone(),
            asset_id,
            item_id,
            name: body.get("name").as_str().to_owned(),
            description: body.get("description").as_str().to_owned(),
            folder: body.get("folder_id").as_uuid(),
            asset_type: asset_code,
            inv_type: i32::from(inv_code),
            next_owner: body.get("next_owner_mask").as_i32() as u32,
            group_mask: body.get("group_mask").as_i32() as u32,
            everyone_mask: body.get("everyone_mask").as_i32() as u32,
            expires: Instant::now() + UPLOAD_TTL,
            storing: false,
        };
        let mut slots = self
            .upload
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.drop_expired(&mut slots);
        if slots
            .iter()
            .any(|slot| slot.agent_id == exchange.agent_id && slot.storing)
        {
            return upload_error("Uploader busy processing previous request");
        }
        if slots.len() >= MAX_PENDING_UPLOADS {
            let agent = exchange.agent_id.to_string();
            ferroan_diag::hit(&ferroan_diag::Hit {
                id: ferroan_diag::REGION_PENDING_UPLOADS,
                kind: Kind::Hard,
                configured: MAX_PENDING_UPLOADS as u64,
                observed: slots.len() as u64 + 1,
                action: ferroan_diag::Action::Refuse,
                route: "NewFileAgentInventory",
                agent_id: Some(&agent),
            });
            return upload_error("uploader limit reached");
        }
        if !self.routes().grant(exchange.agent_id, &path) {
            return upload_error("uploader path was not registered");
        }
        slots.push(pending);
        drop(slots);
        let mut map = Llsd::new_map();
        map.insert("uploader", format!("{}{path}", self.public_base));
        map.insert("state", "upload");
        map.insert("upload_price", 0);
        if mesh_fee {
            map.insert("data", mesh_fee_data());
        }
        llsd_ok(map)
    }

    async fn upload_complete(&self, exchange: &CapExchange) -> CapAnswer {
        if exchange.method != "POST" {
            return CapAnswer::miss();
        }
        let pending = {
            let mut slots = self
                .upload
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(index) = slots.iter().position(|pending| {
                pending.path == exchange.path && pending.agent_id == exchange.agent_id
            }) else {
                return CapAnswer::miss();
            };
            if slots[index].storing {
                return CapAnswer::miss();
            }
            if slots[index].expires <= Instant::now() {
                let expired = slots.remove(index);
                self.routes().revoke(expired.agent_id, &expired.path);
                return CapAnswer::miss();
            }
            slots[index].storing = true;
            slots[index].expires = Instant::now() + UPLOAD_TTL;
            let pending = clone_pending(&slots[index]);
            // The bytes are in flight. A second POST on this path misses, and a
            // new offer while it stores is busy. Another waiting path stays.
            self.routes().revoke(pending.agent_id, &pending.path);
            pending
        };
        if exchange.body.len() > crate::remote_asset::MAX_ASSET_BODY {
            self.clear_upload(&pending.path);
            return upload_failed(&pending, "upload is too large");
        }
        let perms = upload_permissions(
            pending.next_owner,
            pending.group_mask,
            pending.everyone_mask,
        );
        let asset = Asset {
            id: pending.asset_id,
            name: pending.name.clone(),
            description: pending.description.clone(),
            type_code: pending.asset_type,
            local: false,
            temporary: false,
            creator_id: exchange.agent_id.to_string(),
            flags: 0,
            data: exchange.body.clone(),
            upload_attempts: 0,
        };
        let stored = match &self.assets {
            Some(client) => client.store(&asset).await.unwrap_or(false),
            None => false,
        };
        if !stored {
            self.clear_upload(&pending.path);
            return upload_failed(&pending, "asset store failed");
        }
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i32)
            .unwrap_or(0);
        let item = InvItem {
            id: pending.item_id,
            asset_id: pending.asset_id,
            folder: pending.folder,
            name: pending.name.clone(),
            description: pending.description.clone(),
            asset_type: i32::from(pending.asset_type),
            inv_type: pending.inv_type,
            current_permissions: perms.0,
            next_permissions: perms.3,
            everyone_permissions: perms.2,
            base_permissions: perms.0,
            group_permissions: perms.1,
            flags: 0,
            creator_id: exchange.agent_id.to_string(),
            owner: exchange.agent_id,
            group_id: Uuid::nil(),
            group_owned: false,
            sale_price: 0,
            sale_type: 0,
            creation_date: created,
        };
        let added = match &self.inventory {
            Some(client) => client.add_item(&item).await.unwrap_or(false),
            None => false,
        };
        self.clear_upload(&pending.path);
        if !added {
            return upload_failed(&pending, "inventory item was not created");
        }
        let mut map = Llsd::new_map();
        map.insert("new_asset", pending.asset_id.to_string());
        map.insert("new_inventory_item", pending.item_id);
        map.insert("state", "complete");
        map.insert("new_next_owner_mask", perms.3 as i32);
        map.insert("new_group_mask", perms.1 as i32);
        map.insert("new_everyone_mask", perms.2 as i32);
        llsd_ok(map)
    }

    fn clear_upload(&self, path: &str) {
        let mut slots = self
            .upload
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(index) = slots.iter().position(|pending| pending.path == path) {
            let pending = slots.remove(index);
            self.routes().revoke(pending.agent_id, &pending.path);
        }
    }

    fn drop_expired(&self, slots: &mut Vec<PendingUpload>) {
        let now = Instant::now();
        let expired = slots
            .iter()
            .filter(|pending| !pending.storing && pending.expires <= now)
            .map(|pending| (pending.agent_id, pending.path.clone()))
            .collect::<Vec<_>>();
        slots.retain(|pending| pending.storing || pending.expires > now);
        for (agent_id, path) in expired {
            self.routes().revoke(agent_id, &path);
        }
    }

    fn is_bad(&self, id: Uuid) -> bool {
        let mut cache = self.bad.lock().unwrap_or_else(|poison| poison.into_inner());
        let now = Instant::now();
        cache.retain(|(_, at)| now.duration_since(*at) < BAD_TTL);
        cache.iter().any(|(found, _)| *found == id)
    }

    fn remember_bad(&self, id: Uuid) {
        let mut cache = self.bad.lock().unwrap_or_else(|poison| poison.into_inner());
        let now = Instant::now();
        cache.retain(|(_, at)| now.duration_since(*at) < BAD_TTL);
        if cache.iter().any(|(found, _)| *found == id) {
            return;
        }
        if cache.len() >= BAD_LIMIT {
            cache.remove(0);
        }
        cache.push((id, now));
    }
}

fn clone_pending(pending: &PendingUpload) -> PendingUpload {
    PendingUpload {
        agent_id: pending.agent_id,
        path: pending.path.clone(),
        asset_id: pending.asset_id,
        item_id: pending.item_id,
        name: pending.name.clone(),
        description: pending.description.clone(),
        folder: pending.folder,
        asset_type: pending.asset_type,
        inv_type: pending.inv_type,
        next_owner: pending.next_owner,
        group_mask: pending.group_mask,
        everyone_mask: pending.everyone_mask,
        expires: pending.expires,
        storing: pending.storing,
    }
}

fn upload_kind(inventory_type: &str, asset_type: &str) -> Option<(i8, i8)> {
    // A mesh fee's later POST is an object item. The posted body is stored as
    // that asset; this region does not build the linkset from it.
    if asset_type == "mesh" || inventory_type == "mesh" {
        return Some((6, 6));
    }
    if inventory_type == "sound" || asset_type == "sound" && inventory_type.is_empty() {
        return Some((1, 1));
    }
    if inventory_type == "animation"
        || asset_type == "animatn"
        || asset_type == "animation" && inventory_type.is_empty()
    {
        return Some((20, 19));
    }
    if inventory_type == "texture"
        || asset_type == "texture"
        || inventory_type.is_empty() && asset_type.is_empty()
    {
        return Some((0, 0));
    }
    None
}

fn upload_permissions(
    next_owner: u32,
    group_mask: u32,
    everyone_mask: u32,
) -> (u32, u32, u32, u32) {
    let base = PERM_ALL | PERM_EXPORT;
    let group = base & group_mask;
    let everyone = base & everyone_mask;
    let mut next = base & next_owner;
    if next & PERM_ALL == 0 {
        next = PERM_TRANSFER;
    }
    (base, group, everyone, next)
}

fn mesh_fee_data() -> Llsd {
    let mut breakdown = Llsd::new_map();
    breakdown.insert("mesh_streaming", 0);
    breakdown.insert("mesh_physics", 0);
    breakdown.insert("mesh_instance", 0);
    breakdown.insert("texture", 0);
    breakdown.insert("model", 0);
    let mut data = Llsd::new_map();
    data.insert("resource_cost", 0.0);
    data.insert("model_streaming_cost", 0.0);
    data.insert("simulation_cost", 0.0);
    data.insert("physics_cost", 0.0);
    data.insert("upload_price_breakdown", breakdown);
    data
}

fn upload_error(message: &str) -> CapAnswer {
    let mut error = Llsd::new_map();
    error.insert("message", message);
    error.insert("identifier", Uuid::nil());
    let mut map = Llsd::new_map();
    map.insert("uploader", "");
    map.insert("state", "error");
    map.insert("error", error);
    llsd_ok(map)
}

fn upload_failed(pending: &PendingUpload, message: &str) -> CapAnswer {
    let mut error = Llsd::new_map();
    error.insert("message", message);
    error.insert("identifier", pending.item_id);
    let mut map = Llsd::new_map();
    map.insert("state", "failed");
    map.insert("new_inventory_item", pending.item_id);
    map.insert("error", error);
    llsd_ok(map)
}

fn zero_collection(owner: Uuid, version: i32) -> Llsd {
    folder_llsd(
        Uuid::nil(),
        owner,
        owner,
        version,
        0,
        Llsd::new_array(),
        Llsd::new_array(),
    )
}

fn library_collection(
    library: &Library,
    folder: &crate::library::LibraryFolder,
    agent_id: Uuid,
) -> Llsd {
    let categories = library
        .child_folders(folder.id)
        .into_iter()
        .map(|child| {
            category_llsd(
                child.id,
                child.parent_id,
                &child.name,
                child.type_code,
                child.version,
            )
        })
        .collect::<Vec<_>>();
    let items = library
        .child_items(folder.id)
        .into_iter()
        .map(|item| item_llsd(&library_as_item(item), u32::MAX))
        .collect::<Vec<_>>();
    let descendents = i32::try_from(categories.len() + items.len()).unwrap_or(i32::MAX);
    folder_llsd(
        folder.id,
        agent_id,
        library.owner(),
        folder.version,
        descendents,
        array(categories),
        array(items),
    )
}

fn user_collection(contents: Collection, descendents: i32) -> Llsd {
    let categories = contents
        .folders
        .iter()
        .map(|folder| {
            category_llsd(
                folder.id,
                folder.parent_id,
                &folder.name,
                folder.type_code,
                folder.version,
            )
        })
        .collect::<Vec<_>>();
    let items = contents
        .items
        .iter()
        .map(|item| item_llsd(item, u32::MAX))
        .collect::<Vec<_>>();
    folder_llsd(
        contents.folder_id,
        contents.owner_id,
        contents.owner_id,
        contents.version,
        descendents,
        array(categories),
        array(items),
    )
}

fn library_as_item(item: &LibraryItem) -> InvItem {
    InvItem {
        id: item.id,
        asset_id: item.asset_id,
        folder: item.folder,
        name: item.name.clone(),
        description: item.description.clone(),
        asset_type: item.asset_type,
        inv_type: item.inv_type,
        current_permissions: item.current_permissions,
        next_permissions: item.next_permissions,
        everyone_permissions: item.everyone_permissions,
        base_permissions: item.base_permissions,
        group_permissions: item.group_permissions,
        flags: item.flags,
        creator_id: item.creator_id.clone(),
        owner: item.owner,
        group_id: item.group_id,
        group_owned: item.group_owned,
        sale_price: item.sale_price,
        sale_type: item.sale_type,
        creation_date: item.creation_date,
    }
}

fn folder_llsd(
    folder_id: Uuid,
    agent_id: Uuid,
    owner_id: Uuid,
    version: i32,
    descendents: i32,
    categories: Llsd,
    items: Llsd,
) -> Llsd {
    let mut map = Llsd::new_map();
    map.insert("folder_id", folder_id);
    map.insert("agent_id", agent_id);
    map.insert("owner_id", owner_id);
    map.insert("descendents", descendents);
    map.insert("version", version);
    map.insert("categories", categories);
    map.insert("items", items);
    map
}

fn category_llsd(id: Uuid, parent: Uuid, name: &str, type_code: i32, version: i32) -> Llsd {
    let mut map = Llsd::new_map();
    map.insert("category_id", id);
    map.insert("parent_id", parent);
    map.insert("name", name);
    map.insert("type_default", type_code);
    map.insert("version", version);
    map
}

fn item_llsd(item: &InvItem, flags_mask: u32) -> Llsd {
    let mut permissions = Llsd::new_map();
    permissions.insert("creator_id", creator_uuid(&item.creator_id));
    permissions.insert("owner_id", item.owner);
    permissions.insert("group_id", item.group_id);
    permissions.insert("base_mask", item.current_permissions as i32);
    permissions.insert("owner_mask", item.current_permissions as i32);
    permissions.insert("group_mask", item.group_permissions as i32);
    permissions.insert("everyone_mask", item.everyone_permissions as i32);
    permissions.insert("next_owner_mask", item.next_permissions as i32);
    permissions.insert("is_owner_group", item.group_owned);
    let mut sale = Llsd::new_map();
    sale.insert("sale_price", item.sale_price);
    sale.insert("sale_type", item.sale_type);
    let mut map = Llsd::new_map();
    map.insert("parent_id", item.folder);
    map.insert("asset_id", item.asset_id);
    map.insert("item_id", item.id);
    map.insert("permissions", permissions);
    map.insert("type", item.asset_type);
    map.insert("inv_type", item.inv_type);
    map.insert("flags", viewer_flags(item.flags, flags_mask));
    map.insert("sale_info", sale);
    map.insert("name", item.name.as_str());
    map.insert("desc", item.description.as_str());
    map.insert("created_at", item.creation_date);
    map
}

fn creator_uuid(text: &str) -> Uuid {
    Uuid::parse_str(text.trim()).unwrap_or(Uuid::nil())
}

fn viewer_flags(flags: u32, mask: u32) -> i32 {
    (flags & mask & !LSL_ON_LUAU) as i32
}

fn array(values: Vec<Llsd>) -> Llsd {
    let mut array = Llsd::new_array();
    for value in values {
        array.push(value);
    }
    array
}

fn empty_folders() -> Llsd {
    let mut map = Llsd::new_map();
    map.insert("folders", Llsd::new_array());
    map
}

fn llsd_ok(body: Llsd) -> CapAnswer {
    let bytes = to_xml(&body);
    if bytes.len() > MAX_LLSD {
        ferroan_diag::refuse(
            INVENTORY_FETCH_LLSD,
            Kind::Hard,
            MAX_LLSD as u64,
            bytes.len() as u64,
            "inventory",
        );
        return refused();
    }
    CapAnswer {
        handled: true,
        status: StatusCode::OK,
        content_type: LLSD_MIME.to_owned(),
        body: bytes,
        content_range: None,
    }
}

fn refused() -> CapAnswer {
    plain(StatusCode::BAD_REQUEST, Vec::new())
}

fn plain(status: StatusCode, body: Vec<u8>) -> CapAnswer {
    CapAnswer {
        handled: true,
        status,
        content_type: TEXT.to_owned(),
        body,
        content_range: None,
    }
}

/// `TryParseHttpRange`, then the full-body and 206 rules in `GetAssetsHandler`.
pub(crate) fn slice_range<'a>(
    header: Option<&str>,
    data: &'a [u8],
) -> (StatusCode, &'a [u8], Option<String>) {
    let Some((mut start, mut end)) = parse_range(header) else {
        return (StatusCode::OK, data, None);
    };
    let len = data.len();
    let Ok(start_at) = usize::try_from(start) else {
        return (StatusCode::OK, data, None);
    };
    if start_at >= len {
        return (StatusCode::OK, data, None);
    }
    if end == -1 {
        end = len.saturating_sub(1) as i32;
    } else {
        end = end.clamp(0, len.saturating_sub(1) as i32);
    }
    start = start.clamp(0, end);
    let from = start as usize;
    let to = end as usize;
    (
        StatusCode::PARTIAL_CONTENT,
        &data[from..=to],
        Some(format!("bytes {from}-{to}/{len}")),
    )
}

fn parse_range(header: Option<&str>) -> Option<(i32, i32)> {
    let header = header?;
    if header.trim().is_empty() {
        return None;
    }
    let header = header.trim();
    let rest = header.strip_prefix("bytes=")?;
    let (raw_start, raw_end) = rest.split_once('-')?;
    if rest.split('-').nth(2).is_some() {
        return None;
    }
    let start = if raw_start.trim().is_empty() {
        0
    } else {
        raw_start.trim().parse::<i32>().ok()?
    };
    if start < 0 {
        return None;
    }
    let raw_end = raw_end.trim();
    if raw_end.is_empty() {
        return Some((start, -1));
    }
    let end = raw_end.parse::<i32>().ok()?;
    if end > 0 { Some((start, end)) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_follow_the_asset_handler() {
        let data = b"abcdefghijklmnopqrstuvwxyz";
        assert_eq!(slice_range(None, data).0, StatusCode::OK);
        assert_eq!(slice_range(Some("bytes=0-0"), data).0, StatusCode::OK);
        assert_eq!(slice_range(Some("bytes=0-0"), data).1.len(), data.len());
        let (status, body, range) = slice_range(Some("bytes=1-3"), data);
        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(body, b"bcd");
        assert_eq!(range.as_deref(), Some("bytes 1-3/26"));
        let (status, body, range) = slice_range(Some("bytes=24-"), data);
        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(body, b"yz");
        assert_eq!(range.as_deref(), Some("bytes 24-25/26"));
        let (status, body, range) = slice_range(Some("bytes=40-50"), data);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.len(), data.len());
        assert!(range.is_none());
    }

    #[test]
    fn texture_upload_permissions_keep_the_viewers_next_owner_mask() {
        let (base, group, everyone, next) = upload_permissions(0x0008_e000, 0, 0);
        assert_eq!(base, 0x9e000);
        assert_eq!(group, 0);
        assert_eq!(everyone, 0);
        assert_eq!(next, 0x8e000);
    }

    #[tokio::test]
    async fn a_library_folder_fetch_uses_the_root_version_and_owner() {
        let dir = std::env::temp_dir().join(format!("ferroan-fetch-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Libraries.xml"),
            r#"<Nini>
              <Section Name="root"><Key Name="RootVersion" Value="429"/></Section>
              <Section Name="textures">
                <Key Name="foldersFile" Value="folders.xml"/>
                <Key Name="itemsFile" Value="items.xml"/>
              </Section>
            </Nini>"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("folders.xml"),
            r#"<Nini><Section Name="Texture Library">
              <Key Name="folderID" Value="00000112-000f-0000-0000-000100bba001"/>
              <Key Name="parentFolderID" Value="00000112-000f-0000-0000-000100bba000"/>
              <Key Name="name" Value="Texture Library"/>
              <Key Name="type" Value="0"/>
            </Section></Nini>"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("items.xml"),
            r#"<Nini><Section Name="item">
              <Key Name="inventoryID" Value="00000000-0000-2222-4444-100000001000"/>
              <Key Name="assetID" Value="00000000-0000-2222-3333-100000001000"/>
              <Key Name="folderID" Value="00000112-000f-0000-0000-000100bba001"/>
              <Key Name="name" Value="4-tile2"/>
              <Key Name="description" Value="4-tile2 texture"/>
              <Key Name="assetType" Value="0"/>
              <Key Name="inventoryType" Value="0"/>
            </Section></Nini>"#,
        )
        .unwrap();
        let library = Library::open(&dir.join("Libraries.xml"), "OpenSim Library").unwrap();
        let caps = FetchCaps {
            inventory: None,
            assets: None,
            library,
            bad: Mutex::new(Vec::new()),
            upload: Mutex::new(Vec::new()),
            routes: std::sync::OnceLock::new(),
            public_base: "http://127.0.0.1:9".into(),
        };
        let root = Uuid::parse_str("00000112-000f-0000-0000-000100bba000").unwrap();
        let owner = Uuid::parse_str("11111111-1111-0000-0000-000100bba000").unwrap();
        let agent = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let mut one = Llsd::new_map();
        one.insert("folder_id", root);
        one.insert("owner_id", owner);
        one.insert("fetch_folders", true);
        one.insert("fetch_items", true);
        let mut folders = Llsd::new_array();
        folders.push(one);
        let mut request = Llsd::new_map();
        request.insert("folders", folders);
        let answer = caps
            .descendents(
                &CapExchange {
                    name: "FetchLibDescendents2".into(),
                    path: "/x".into(),
                    method: "POST".into(),
                    query: String::new(),
                    range: None,
                    body: to_xml(&request),
                    agent_id: agent,
                },
                true,
            )
            .await;
        let parsed = from_xml(&answer.body).unwrap();
        let folder = parsed.get("folders").at(0);
        assert_eq!(folder.get("version").as_i32(), 429);
        assert_eq!(folder.get("agent_id").as_uuid(), agent);
        assert_eq!(folder.get("owner_id").as_uuid(), owner);
        assert_eq!(folder.get("categories").as_array().len(), 1);
        assert_eq!(
            folder.get("categories").at(0).get("name").as_str(),
            "Texture Library"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn offer_body(name: &str) -> Vec<u8> {
        let mut map = Llsd::new_map();
        map.insert("asset_type", "texture");
        map.insert("inventory_type", "texture");
        map.insert("name", name);
        map.insert("description", "synthetic");
        map.insert("folder_id", Uuid::nil());
        map.insert("next_owner_mask", 0);
        map.insert("group_mask", 0);
        map.insert("everyone_mask", 0);
        to_xml(&map)
    }

    fn mesh_fee_body(name: &str) -> Vec<u8> {
        let mut map = Llsd::new_map();
        map.insert("asset_type", "mesh");
        map.insert("inventory_type", "object");
        map.insert("name", name);
        map.insert("description", "synthetic model");
        map.insert("folder_id", Uuid::nil());
        map.insert("next_owner_mask", 0);
        map.insert("group_mask", 0);
        map.insert("everyone_mask", 0);
        to_xml(&map)
    }

    fn exchange(agent: Uuid, name: &str, path: &str, body: Vec<u8>) -> CapExchange {
        CapExchange {
            name: name.to_owned(),
            path: path.to_owned(),
            method: "POST".to_owned(),
            query: String::new(),
            range: None,
            body,
            agent_id: agent,
        }
    }

    #[tokio::test]
    async fn two_agents_keep_their_own_uploads() {
        let stored = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let recorded = Arc::clone(&stored);
        let assets = axum::Router::new()
            .route(
                "/assets/",
                axum::routing::post(move |body: axum::body::Bytes| {
                    let recorded = Arc::clone(&recorded);
                    async move {
                        let text = String::from_utf8_lossy(&body);
                        let id = text
                            .split("<ID>")
                            .nth(1)
                            .and_then(|rest| rest.split("</ID>").next())
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        let creator = text
                            .split("<CreatorID>")
                            .nth(1)
                            .and_then(|rest| rest.split("</CreatorID>").next())
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        recorded.lock().unwrap().push(format!("{creator} {id}"));
                        (
                            StatusCode::OK,
                            format!(
                                "<?xml version=\"1.0\" encoding=\"utf-8\"?><string xmlns=\"http://schemas.microsoft.com/2003/10/Serialization/\">{id}</string>"
                            ),
                        )
                    }
                }),
            )
            .route(
                "/xinventory",
                axum::routing::post(|| async {
                    r#"<?xml version="1.0"?><ServerResponse><RESULT>True</RESULT></ServerResponse>"#
                }),
            );
        let asset_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let asset_base = format!("http://{}", asset_listener.local_addr().unwrap());
        let assets_task = tokio::spawn(async move {
            axum::serve(asset_listener, assets).await.unwrap();
        });
        let caps_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let caps_base = format!("http://{}", caps_listener.local_addr().unwrap());
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let caps = Arc::new(FetchCaps {
            inventory: crate::inventory::InventoryClient::new(&asset_base, http.clone()),
            assets: crate::remote_asset::AssetClient::new(&asset_base, http.clone()),
            library: Library::absent(),
            bad: Mutex::new(Vec::new()),
            upload: Mutex::new(Vec::new()),
            routes: std::sync::OnceLock::new(),
            public_base: caps_base.clone(),
        });
        let mut grants = ferroan_client::CapGrantSet::default();
        grants.grant("NewFileAgentInventory", ferroan_client::CapTarget::Local);
        let registry = ferroan_client::CapsRegistry::with_grants(
            &format!("{caps_base}/"),
            grants,
            Some(caps.service()),
        )
        .unwrap();
        caps.bind_uploader_routes(registry.uploader_routes());
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let first_seed = "/CAPS/0000000000000000000000000000000a0000/";
        let second_seed = "/CAPS/0000000000000000000000000000000b0000/";
        registry.register_agent(first_seed, first).unwrap();
        registry.register_agent(second_seed, second).unwrap();
        let app = registry.router();
        let caps_task = tokio::spawn(async move { axum::serve(caps_listener, app).await.unwrap() });

        let first_cap = seed_cap(&http, &caps_base, first_seed).await;
        let second_cap = seed_cap(&http, &caps_base, second_seed).await;
        let first_offer = post_llsd(&http, &first_cap, &offer_body("left")).await;
        let second_offer = post_llsd(&http, &second_cap, &offer_body("right")).await;
        assert_eq!(first_offer.get("state").as_str(), "upload");
        assert_eq!(second_offer.get("state").as_str(), "upload");
        let first_uploader = first_offer.get("uploader").as_str().to_owned();
        let second_uploader = second_offer.get("uploader").as_str().to_owned();
        assert_ne!(first_uploader, second_uploader);
        let stranger = format!("{caps_base}/{}", Uuid::new_v4());
        assert_eq!(post_bytes(&http, &stranger, b"no").await.0, 404);

        let (left, right) = tokio::join!(
            post_bytes(&http, &first_uploader, b"left-bytes"),
            post_bytes(&http, &second_uploader, b"right-bytes"),
        );
        let left_body = from_xml(&left.1).unwrap();
        let right_body = from_xml(&right.1).unwrap();
        assert_eq!(left.0, 200);
        assert_eq!(right.0, 200);
        assert_eq!(left_body.get("state").as_str(), "complete");
        assert_eq!(right_body.get("state").as_str(), "complete");
        assert_ne!(
            left_body.get("new_asset").as_str(),
            right_body.get("new_asset").as_str()
        );
        assert_ne!(
            left_body.get("new_inventory_item").as_uuid(),
            right_body.get("new_inventory_item").as_uuid()
        );
        assert_eq!(post_bytes(&http, &first_uploader, b"again").await.0, 404);
        assert_eq!(post_bytes(&http, &second_uploader, b"again").await.0, 404);

        let replaced = post_llsd(&http, &first_cap, &offer_body("left-again")).await;
        assert_eq!(replaced.get("state").as_str(), "upload");
        let replaced_uploader = replaced.get("uploader").as_str().to_owned();
        assert_ne!(replaced_uploader, first_uploader);
        assert_eq!(post_bytes(&http, &first_uploader, b"stale").await.0, 404);
        let again = post_bytes(&http, &replaced_uploader, b"fresh").await;
        assert_eq!(
            from_xml(&again.1).unwrap().get("state").as_str(),
            "complete"
        );
        let right_again = post_llsd(&http, &second_cap, &offer_body("right-again")).await;
        assert_eq!(right_again.get("state").as_str(), "upload");

        let seen = stored.lock().unwrap().clone();
        assert!(seen.iter().any(|line| line.starts_with(&first.to_string())));
        assert!(
            seen.iter()
                .any(|line| line.starts_with(&second.to_string()))
        );
        caps_task.abort();
        assets_task.abort();
    }

    #[tokio::test]
    async fn a_storing_agent_does_not_block_the_other_agents_offer() {
        let started = Arc::new(tokio::sync::Notify::new());
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
        let started_gate = Arc::clone(&started);
        let assets = axum::Router::new()
            .route(
                "/assets/",
                axum::routing::post(move |body: axum::body::Bytes| {
                    let started_gate = Arc::clone(&started_gate);
                    let release_rx = Arc::clone(&release_rx);
                    async move {
                        let text = String::from_utf8_lossy(&body);
                        let id = text
                            .split("<ID>")
                            .nth(1)
                            .and_then(|rest| rest.split("</ID>").next())
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        let gate = release_rx.lock().await.take();
                        started_gate.notify_one();
                        if let Some(gate) = gate {
                            let _ = gate.await;
                        }
                        (
                            StatusCode::OK,
                            format!(
                                "<?xml version=\"1.0\" encoding=\"utf-8\"?><string xmlns=\"http://schemas.microsoft.com/2003/10/Serialization/\">{id}</string>"
                            ),
                        )
                    }
                }),
            )
            .route(
                "/xinventory",
                axum::routing::post(|| async {
                    r#"<?xml version="1.0"?><ServerResponse><RESULT>True</RESULT></ServerResponse>"#
                }),
            );
        let asset_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let asset_base = format!("http://{}", asset_listener.local_addr().unwrap());
        let assets_task = tokio::spawn(async move {
            axum::serve(asset_listener, assets).await.unwrap();
        });
        let caps_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let caps_base = format!("http://{}", caps_listener.local_addr().unwrap());
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let caps = Arc::new(FetchCaps {
            inventory: crate::inventory::InventoryClient::new(&asset_base, http.clone()),
            assets: crate::remote_asset::AssetClient::new(&asset_base, http.clone()),
            library: Library::absent(),
            bad: Mutex::new(Vec::new()),
            upload: Mutex::new(Vec::new()),
            routes: std::sync::OnceLock::new(),
            public_base: caps_base.clone(),
        });
        let mut grants = ferroan_client::CapGrantSet::default();
        grants.grant("NewFileAgentInventory", ferroan_client::CapTarget::Local);
        let registry = ferroan_client::CapsRegistry::with_grants(
            &format!("{caps_base}/"),
            grants,
            Some(caps.service()),
        )
        .unwrap();
        caps.bind_uploader_routes(registry.uploader_routes());
        let first = Uuid::from_u128(3);
        let second = Uuid::from_u128(4);
        let first_seed = "/CAPS/0000000000000000000000000000000c0000/";
        let second_seed = "/CAPS/0000000000000000000000000000000d0000/";
        registry.register_agent(first_seed, first).unwrap();
        registry.register_agent(second_seed, second).unwrap();
        let app = registry.router();
        let caps_task = tokio::spawn(async move { axum::serve(caps_listener, app).await.unwrap() });
        let first_cap = seed_cap(&http, &caps_base, first_seed).await;
        let second_cap = seed_cap(&http, &caps_base, second_seed).await;
        let first_offer = post_llsd(&http, &first_cap, &offer_body("busy")).await;
        let uploader = first_offer.get("uploader").as_str().to_owned();
        let http_complete = http.clone();
        let completing =
            tokio::spawn(async move { post_bytes(&http_complete, &uploader, b"blocked").await });
        tokio::time::timeout(Duration::from_secs(5), started.notified())
            .await
            .unwrap();
        let busy = post_llsd(&http, &first_cap, &offer_body("busy-again")).await;
        assert_eq!(
            busy.get("error").get("message").as_str(),
            "Uploader busy processing previous request"
        );
        let other = post_llsd(&http, &second_cap, &offer_body("other")).await;
        assert_eq!(other.get("state").as_str(), "upload");
        release_tx.send(()).unwrap();
        let finished = tokio::time::timeout(Duration::from_secs(5), completing)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            from_xml(&finished.1).unwrap().get("state").as_str(),
            "complete"
        );
        let other_done = post_bytes(&http, other.get("uploader").as_str(), b"other-bytes").await;
        assert_eq!(
            from_xml(&other_done.1).unwrap().get("state").as_str(),
            "complete"
        );
        caps_task.abort();
        assets_task.abort();
    }

    #[tokio::test]
    async fn a_new_agent_past_the_upload_bound_is_refused() {
        let caps_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let caps_base = format!("http://{}", caps_listener.local_addr().unwrap());
        let caps = Arc::new(FetchCaps {
            inventory: None,
            assets: None,
            library: Library::absent(),
            bad: Mutex::new(Vec::new()),
            upload: Mutex::new(Vec::new()),
            routes: std::sync::OnceLock::new(),
            public_base: caps_base.clone(),
        });
        let registry = ferroan_client::CapsRegistry::new(&format!("{caps_base}/")).unwrap();
        caps.bind_uploader_routes(registry.uploader_routes());
        for n in 1..=MAX_PENDING_UPLOADS {
            let agent = Uuid::from_u128(n as u128);
            registry
                .register_agent(&format!("/CAPS/{n:08x}0000/"), agent)
                .unwrap();
            let answer = caps.upload_offer(&exchange(
                agent,
                "NewFileAgentInventory",
                "/seed",
                offer_body("n"),
            ));
            assert_eq!(
                from_xml(&answer.body).unwrap().get("state").as_str(),
                "upload"
            );
        }
        let extra = Uuid::from_u128(10_000);
        registry
            .register_agent("/CAPS/000027100000/", extra)
            .unwrap();
        let refused = caps.upload_offer(&exchange(
            extra,
            "NewFileAgentInventory",
            "/seed",
            offer_body("over"),
        ));
        let body = from_xml(&refused.body).unwrap();
        assert_eq!(body.get("state").as_str(), "error");
        assert_eq!(
            body.get("error").get("message").as_str(),
            "uploader limit reached"
        );
        assert!(ferroan_diag::metrics().contains("region.pending-uploads"));
        let again = caps.upload_offer(&exchange(
            Uuid::from_u128(1),
            "NewFileAgentInventory",
            "/seed",
            offer_body("second"),
        ));
        let again_body = from_xml(&again.body).unwrap();
        assert_eq!(again_body.get("state").as_str(), "error");
        assert_eq!(
            again_body.get("error").get("message").as_str(),
            "uploader limit reached"
        );
        let slots = caps
            .upload
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(slots.len(), MAX_PENDING_UPLOADS);
        assert!(slots.iter().any(|slot| slot.agent_id == Uuid::from_u128(1)));
        drop(slots);
        drop(caps_listener);
    }

    #[tokio::test]
    async fn an_expired_waiting_upload_is_dropped_when_the_next_offer_arrives() {
        let caps_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let caps_base = format!("http://{}", caps_listener.local_addr().unwrap());
        let caps = Arc::new(FetchCaps {
            inventory: None,
            assets: None,
            library: Library::absent(),
            bad: Mutex::new(Vec::new()),
            upload: Mutex::new(Vec::new()),
            routes: std::sync::OnceLock::new(),
            public_base: caps_base.clone(),
        });
        let registry = ferroan_client::CapsRegistry::new(&format!("{caps_base}/")).unwrap();
        caps.bind_uploader_routes(registry.uploader_routes());
        let agent = Uuid::from_u128(7);
        registry
            .register_agent("/CAPS/000000070000/", agent)
            .unwrap();
        let first = caps.upload_offer(&exchange(
            agent,
            "NewFileAgentInventory",
            "/seed",
            offer_body("waiting"),
        ));
        let first_body = from_xml(&first.body).unwrap();
        assert_eq!(first_body.get("state").as_str(), "upload");
        let first_path = {
            let mut slots = caps
                .upload
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            assert_eq!(slots.len(), 1);
            slots[0].expires = Instant::now();
            slots[0].path.clone()
        };
        let second = caps.upload_offer(&exchange(
            agent,
            "NewFileAgentInventory",
            "/seed",
            offer_body("after"),
        ));
        let second_body = from_xml(&second.body).unwrap();
        assert_eq!(second_body.get("state").as_str(), "upload");
        assert_ne!(
            second_body.get("uploader").as_str(),
            first_body.get("uploader").as_str()
        );
        {
            let slots = caps
                .upload
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            assert_eq!(slots.len(), 1);
            assert_ne!(slots[0].path, first_path);
        }
        let missed = caps
            .upload_complete(&exchange(agent, "", &first_path, b"late".to_vec()))
            .await;
        assert!(!missed.handled);
        drop(caps_listener);
    }

    #[tokio::test]
    async fn mesh_fee_then_texture_then_the_first_uploader_completes() {
        let stored = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let recorded = Arc::clone(&stored);
        let assets = axum::Router::new()
            .route(
                "/assets/",
                axum::routing::post(move |body: axum::body::Bytes| {
                    let recorded = Arc::clone(&recorded);
                    async move {
                        let text = String::from_utf8_lossy(&body);
                        let name = text
                            .split("<Name>")
                            .nth(1)
                            .and_then(|rest| rest.split("</Name>").next())
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        let kind = text
                            .split("<Type>")
                            .nth(1)
                            .and_then(|rest| rest.split("</Type>").next())
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        let id = text
                            .split("<ID>")
                            .nth(1)
                            .and_then(|rest| rest.split("</ID>").next())
                            .unwrap_or("")
                            .trim()
                            .to_owned();
                        recorded
                            .lock()
                            .unwrap()
                            .push(format!("{name} {kind} {id}"));
                        (
                            StatusCode::OK,
                            format!(
                                "<?xml version=\"1.0\" encoding=\"utf-8\"?><string xmlns=\"http://schemas.microsoft.com/2003/10/Serialization/\">{id}</string>"
                            ),
                        )
                    }
                }),
            )
            .route(
                "/xinventory",
                axum::routing::post(|| async {
                    r#"<?xml version="1.0"?><ServerResponse><RESULT>True</RESULT></ServerResponse>"#
                }),
            );
        let asset_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let asset_base = format!("http://{}", asset_listener.local_addr().unwrap());
        let assets_task = tokio::spawn(async move {
            axum::serve(asset_listener, assets).await.unwrap();
        });
        let caps_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let caps_base = format!("http://{}", caps_listener.local_addr().unwrap());
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let caps = Arc::new(FetchCaps {
            inventory: crate::inventory::InventoryClient::new(&asset_base, http.clone()),
            assets: crate::remote_asset::AssetClient::new(&asset_base, http.clone()),
            library: Library::absent(),
            bad: Mutex::new(Vec::new()),
            upload: Mutex::new(Vec::new()),
            routes: std::sync::OnceLock::new(),
            public_base: caps_base.clone(),
        });
        let mut grants = ferroan_client::CapGrantSet::default();
        grants.grant("NewFileAgentInventory", ferroan_client::CapTarget::Local);
        let registry = ferroan_client::CapsRegistry::with_grants(
            &format!("{caps_base}/"),
            grants,
            Some(caps.service()),
        )
        .unwrap();
        caps.bind_uploader_routes(registry.uploader_routes());
        let agent = Uuid::from_u128(9);
        let seed = "/CAPS/000000000000000000000000000000090000/";
        registry.register_agent(seed, agent).unwrap();
        let app = registry.router();
        let caps_task = tokio::spawn(async move { axum::serve(caps_listener, app).await.unwrap() });

        let cap = seed_cap(&http, &caps_base, seed).await;
        let fee = post_llsd(&http, &cap, &mesh_fee_body("model")).await;
        assert_eq!(fee.get("state").as_str(), "upload");
        assert_eq!(fee.get("upload_price").as_i32(), 0);
        assert_eq!(fee.get("data").get("resource_cost").as_f64(), 0.0);
        assert_eq!(
            fee.get("data")
                .get("upload_price_breakdown")
                .get("model")
                .as_i32(),
            0
        );
        let mesh_uploader = fee.get("uploader").as_str().to_owned();
        let texture = post_llsd(&http, &cap, &offer_body("cloth")).await;
        assert_eq!(texture.get("state").as_str(), "upload");
        let texture_uploader = texture.get("uploader").as_str().to_owned();
        assert_ne!(texture_uploader, mesh_uploader);
        let texture_done = post_bytes(&http, &texture_uploader, b"texture-bytes").await;
        assert_eq!(texture_done.0, 200);
        assert_eq!(
            from_xml(&texture_done.1).unwrap().get("state").as_str(),
            "complete"
        );
        let mesh_done = post_bytes(&http, &mesh_uploader, b"mesh-bytes").await;
        assert_eq!(mesh_done.0, 200);
        let mesh_body = from_xml(&mesh_done.1).unwrap();
        assert_eq!(mesh_body.get("state").as_str(), "complete");
        assert_ne!(
            mesh_body.get("new_asset").as_str(),
            from_xml(&texture_done.1).unwrap().get("new_asset").as_str()
        );
        assert_eq!(post_bytes(&http, &mesh_uploader, b"again").await.0, 404);
        let seen = stored.lock().unwrap().clone();
        assert!(seen.iter().any(|line| line.starts_with("cloth 0 ")));
        assert!(seen.iter().any(|line| line.starts_with("model 6 ")));
        caps_task.abort();
        assets_task.abort();
    }

    async fn seed_cap(http: &reqwest::Client, base: &str, seed: &str) -> String {
        let (status, bytes) = post_bytes_type(
            http,
            &format!("{base}{seed}"),
            "application/llsd+xml",
            &to_xml(&{
                let mut names = Llsd::new_array();
                names.push("NewFileAgentInventory");
                names
            }),
        )
        .await;
        assert_eq!(status, 200);
        from_xml(&bytes)
            .unwrap()
            .get("NewFileAgentInventory")
            .as_str()
            .to_owned()
    }

    async fn post_llsd(http: &reqwest::Client, url: &str, body: &[u8]) -> Llsd {
        let (status, bytes) = post_bytes_type(http, url, "application/llsd+xml", body).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
        from_xml(&bytes).unwrap()
    }

    async fn post_bytes(http: &reqwest::Client, url: &str, body: &[u8]) -> (u16, Vec<u8>) {
        post_bytes_type(http, url, "application/octet-stream", body).await
    }

    async fn post_bytes_type(
        http: &reqwest::Client,
        url: &str,
        content_type: &str,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        let response = http
            .post(url)
            .header("content-type", content_type)
            .body(body.to_vec())
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = response.bytes().await.unwrap().to_vec();
        (status, bytes)
    }
}
