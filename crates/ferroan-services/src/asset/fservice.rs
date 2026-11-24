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
//   Source/OpenSim.Services.FSAssetService/FSAssetService.cs
//   Source/OpenSim.Framework.Serialization/External/ExternalRepresentationUtils.cs
//     SanitizeXml
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `FSAssetService` on the asset HTTP routes.
//!
//! `Get` reads the row and then the file. A local miss, or a row whose file
//! is missing or damaged, tries `FallbackService` and stores what it returned.
//! The fallback row is left in place. `GetData` does not fall back and does
//! not clean object XML. `GetMetadata` is the row only, so a missing file is
//! still metadata. `AssetsExist` is true only when that read would serve the
//! asset (TB25). It reads the hashes in one query and does not update
//! `access_time`. An id that does not parse is an error. A missing or
//! unreadable file is false. MySQL answers all false when the database will
//! not open. PostgreSQL returns that failure. A non-uuid `Get` is a miss, as the PostgreSQL plugin answers since
//! TB49 was fixed (#500).
//!
//! `Store` hashes the original bytes, truncates the name and description to
//! 64 UTF-16 code units, and writes an object as sanitized XML ending in a
//! NUL when the file is not already there. A failed spool write answers the
//! zero UUID and leaves no partial file (TB36). A failed row write answers
//! the zero UUID, or the id when the type is -2. `Delete` calls the plugin
//! and returns its answer (TB50, #501), so a PostgreSQL non-uuid is false
//! (TB39, #490). The console commands are `show assets`,
//! `show digest`, `delete asset`, `import` and `force import`, and only the
//! main instance registers them. The pin's PostgreSQL import page is
//! `LIMIT count OFFSET start` with `ORDER BY id` (TB40, #491). This import
//! seeks that start once and then reads the next id. `force import` updates
//! an existing row's hash (TB45). File reads and spool writes run on a
//! blocking thread. Import reads one source row and stores it before the
//! next read.
//! `ShowConsoleStats` is not started.
//!
//! Only the main instance runs `DefaultAssetLoader` and the writer. A later
//! instance in the process does not, and neither does `SecondaryInstance`.
//! The writer is aborted when this service is dropped. It does not exit
//! the process.

use std::time::Duration;

use uuid::Uuid;

use ferroan_db::{Backend, Pool};

use super::fsasset::{FsAsset, FsAssetStore, FsImportRow, FsMetadata};
use super::fscommands::{
    FsDigest, FsImportParse, connection_is_sqlx_url, delete_fs_asset_lines, import_syntax_line,
    parse_fs_import, registered_fs_commands, show_assets_lines, show_digest_lines,
    without_ado_pool_keys,
};
use super::fsfiles::{
    FsFiles, OBJECT_TYPE, WRITER_INTERVAL_MS, log_fsassets, next_writer_interval, sha256_hex,
};
use super::loader::LoadedAsset;
use super::model::{Asset, parse_id};
use super::store::{AssetStore, StoreError};

const NIL_ID: &str = "00000000-0000-0000-0000-000000000000";

pub struct FsAssetService {
    store: FsAssetStore,
    files: FsFiles,
    fallback: Option<AssetStore>,
    main: bool,
    writer: Option<tokio::task::JoinHandle<()>>,
}

/// Metadata `GetMetadata` serves. `created_unix` is the stored `create_time`.
pub struct FsMeta {
    pub asset: Asset,
    pub created_unix: i32,
}

impl Drop for FsAssetService {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            writer.abort();
        }
    }
}

impl FsAssetService {
    /// `main` starts nothing by itself. [`Self::start_writer`] is the main
    /// instance's writer thread, after the loader has stored.
    pub fn new(
        store: FsAssetStore,
        files: FsFiles,
        fallback: Option<AssetStore>,
        main: bool,
    ) -> Self {
        Self {
            store,
            files,
            fallback,
            main,
            writer: None,
        }
    }

    pub fn pool_occupancy(&self) -> (u32, u32) {
        if let Some(fallback) = &self.fallback {
            let _ = fallback.pool_occupancy();
        }
        self.store.pool_occupancy()
    }

    pub fn start_writer(&mut self) {
        if !self.main || self.writer.is_some() {
            return;
        }
        log_fsassets(
            "INFO",
            &format!(
                "Writer started with spooldir {} and basedir {}",
                self.files.spool().display(),
                self.files.base().display()
            ),
        );
        let files = self.files.clone();
        self.writer = Some(tokio::spawn(async move {
            let mut interval = WRITER_INTERVAL_MS;
            loop {
                let files = files.clone();
                let wrote = tokio::task::spawn_blocking(move || files.writer_pass())
                    .await
                    .unwrap_or(false);
                interval = next_writer_interval(interval, wrote);
                tokio::time::sleep(Duration::from_millis(interval)).await;
            }
        }));
    }

    /// `ForEachDefaultXmlAsset`'s callback: `Store` each asset. A spool
    /// failure is the zero id and the load continues. A sanitize failure is
    /// the constructor's exception.
    pub async fn load_defaults(&self, assets: &[LoadedAsset]) -> Result<(), StoreError> {
        for loaded in assets {
            let Some(data) = loaded.data.clone() else {
                continue;
            };
            let mut asset = Asset {
                id: loaded.id,
                name: loaded.name.clone(),
                description: loaded.description.clone(),
                type_code: loaded.type_code,
                local: loaded.local,
                temporary: loaded.temporary,
                creator_id: loaded.creator_id.clone(),
                flags: loaded.flags,
                data,
                upload_attempts: 0,
            };
            self.store_asset(&mut asset, &loaded.id.to_string())
                .await
                .map_err(|_| StoreError::Row("object xml".to_string()))?;
        }
        Ok(())
    }

    /// `Get`. `Err` is a plugin failure the handler does not catch.
    pub async fn get(&self, id: &str) -> Result<Option<Asset>, StoreError> {
        Ok(self.get_with_hash(id).await?.map(|(asset, _)| asset))
    }

    /// `Get` plus the hash `show digest` prints in `FS file`.
    async fn get_with_hash(&self, id: &str) -> Result<Option<(Asset, String)>, StoreError> {
        let Some(uuid) = parse_id(id) else {
            return Ok(None);
        };
        match self.store.get(uuid).await? {
            None => match self.take_fallback(id).await? {
                None => Ok(None),
                Some(asset) => {
                    let hash = self.row_hash(uuid).await?;
                    Ok(Some((asset, hash)))
                }
            },
            Some(row) => self.finish_local(id, row).await,
        }
    }

    async fn row_hash(&self, id: Uuid) -> Result<String, StoreError> {
        Ok(self
            .store
            .get(id)
            .await?
            .map(|row| row.hash)
            .unwrap_or_default())
    }

    /// `GetData`. No fallback and no object cleanup.
    pub async fn get_data(&self, id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let Some(uuid) = parse_id(id) else {
            return Ok(None);
        };
        let Some(row) = self.store.get(uuid).await? else {
            return Ok(None);
        };
        match read_checked_off(&self.files, &row.hash, row.metadata.type_code).await {
            Ok(done) => Ok(done.value),
            Err(error) => {
                log_fsassets("ERROR", &format!("Could not get asset {id}: {error}"));
                Ok(None)
            }
        }
    }

    /// `GetMetadata`. The file is not opened.
    pub async fn get_metadata(&self, id: &str) -> Result<Option<FsMeta>, StoreError> {
        let Some(uuid) = parse_id(id) else {
            return Ok(None);
        };
        let Some(row) = self.store.get(uuid).await? else {
            return Ok(None);
        };
        Ok(Some(FsMeta {
            created_unix: row.create_time,
            asset: asset_from_row(&row, Vec::new()),
        }))
    }

    /// `AssetsExist`. A bad id is `Err`, which the handler answers as 500.
    /// One hash query, then the file check. The access time is not updated.
    /// A file that cannot be read is false. MySQL answers all false when the
    /// connection cannot be acquired. A statement error after it is open is
    /// returned. PostgreSQL returns either failure.
    pub async fn assets_exist(&self, ids: &[String]) -> Result<Vec<bool>, StoreError> {
        let mut parsed = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(uuid) = parse_id(id) else {
                return Err(StoreError::Row(format!("asset id {id} is not a uuid")));
            };
            parsed.push(uuid);
        }
        let rows = self.store.hashes(&parsed).await?;
        let mut found = Vec::with_capacity(rows.len());
        for row in rows {
            let present = match row {
                Some(row) => read_checked_off(&self.files, &row.hash, row.type_code)
                    .await
                    .ok()
                    .and_then(|done| done.value)
                    .is_some(),
                None => false,
            };
            found.push(present);
        }
        Ok(found)
    }

    /// `Store`. `Err` is the object cleanup throwing. `Ok` is the id text,
    /// including the zero UUID when the asset was not stored.
    pub async fn store_asset(&self, asset: &mut Asset, id_text: &str) -> Result<String, ()> {
        self.store_asset_inner(asset, id_text, false).await
    }

    async fn store_asset_inner(
        &self,
        asset: &mut Asset,
        id_text: &str,
        force: bool,
    ) -> Result<String, ()> {
        let original = asset.data.clone();
        let hash = sha256_hex(&original);
        truncate_name(asset);
        truncate_description(asset);
        let mut stored = original;
        if read_checked_off(&self.files, &hash, asset.type_code)
            .await
            .ok()
            .and_then(|done| done.value)
            .is_none()
        {
            let final_file = self.files.spool().join(format!("{hash}.asset"));
            if !final_file.is_file() {
                if asset.type_code == OBJECT_TYPE && rewrite_object(&mut stored).is_err() {
                    return Err(());
                }
                match write_spool_off(self.files.clone(), hash.clone(), stored).await {
                    SpoolHandoff::Stored { data, .. } => stored = data,
                    SpoolHandoff::Failed(data) => {
                        asset.data = data;
                        return Ok(NIL_ID.to_string());
                    }
                    SpoolHandoff::Stopped => return Ok(NIL_ID.to_string()),
                }
            }
        }
        asset.data = stored;
        let Some((id, id_text)) = resolve_id(asset.id, id_text) else {
            return Ok(NIL_ID.to_string());
        };
        asset.id = id;
        let meta = FsMetadata {
            id,
            name: asset.name.clone(),
            description: asset.description.clone(),
            type_code: asset.type_code,
            flags: asset.flags,
        };
        let written = if force {
            self.store.store_forced(&meta, &hash).await
        } else {
            self.store.store(&meta, &hash).await
        };
        match written {
            Ok(true) => Ok(id_text),
            Ok(false) | Err(_) => {
                if asset.type_code == -2 {
                    Ok(id_text)
                } else {
                    Ok(NIL_ID.to_string())
                }
            }
        }
    }

    /// `Delete`. Answers what the plugin answers (TB50, #501): a PostgreSQL
    /// non-uuid is false. `Err` is a statement that failed.
    pub async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        self.store.delete(id).await
    }

    #[cfg(test)]
    pub(crate) fn store_for_test(&self) -> &FsAssetStore {
        &self.store
    }

    /// Commands the main instance registers. A secondary instance registers none.
    /// The remote console does not call this yet.
    #[allow(dead_code)]
    pub fn console_commands(&self) -> &'static [super::fscommands::FsConsoleCommand] {
        registered_fs_commands(self.main)
    }

    /// `HandleShowAssets`. The remote console does not call this yet.
    #[allow(dead_code)]
    pub async fn console_show_assets(&self) -> Result<Vec<String>, StoreError> {
        Ok(show_assets_lines(self.store.count().await?))
    }

    /// `HandleShowDigest`. A missing file is not found, because this is `Get`.
    /// The remote console does not call this yet.
    #[allow(dead_code)]
    pub async fn console_show_digest(&self, args: &[&str]) -> Result<Vec<String>, StoreError> {
        if args.len() < 3 {
            return Ok(show_digest_lines(args, None));
        }
        let Some((asset, hash)) = self.get_with_hash(args[2]).await? else {
            return Ok(show_digest_lines(args, None));
        };
        let digest = FsDigest {
            name: &asset.name,
            description: &asset.description,
            type_code: asset.type_code,
            flags: asset.flags,
            hash: &hash,
            osgrid: self.files.osgrid(),
            data: &asset.data,
        };
        Ok(show_digest_lines(args, Some(&digest)))
    }

    /// `HandleDeleteAsset`. The row is what is deleted, so a missing file
    /// still deletes. A non-uuid is not found and `Delete` is not called (TB39).
    /// The remote console does not call this yet.
    #[allow(dead_code)]
    pub async fn console_delete_asset(&self, args: &[&str]) -> Result<Vec<String>, StoreError> {
        if args.len() < 3 {
            return Ok(delete_fs_asset_lines(args, false));
        }
        if self.get_metadata(args[2]).await?.is_none() {
            return Ok(delete_fs_asset_lines(args, false));
        }
        let _deleted = self.store.delete(args[2]).await?;
        Ok(delete_fs_asset_lines(args, true))
    }

    /// `HandleImportAssets`. A bad integer does not open the database.
    /// A failed connection logs and returns no `Import done` line.
    /// The connection string is not logged. The remote console does not call this yet.
    #[allow(dead_code)]
    pub async fn console_import(&self, args: &[&str]) -> Result<Vec<String>, StoreError> {
        let request = match parse_fs_import(args) {
            FsImportParse::Syntax => return Ok(vec![import_syntax_line()]),
            FsImportParse::BadInteger => {
                return Ok(vec!["ERROR: start and count must be integers".to_string()]);
            }
            FsImportParse::Ready(request) => request,
        };
        if request.start < 0 || request.count < -1 {
            return Ok(vec!["ERROR: start and count must be integers".to_string()]);
        }
        let pool = match open_import_pool(self.store.backend(), &request.connection).await {
            Ok(pool) => pool,
            Err(()) => return Ok(Vec::new()),
        };
        let backend = self.store.backend();
        let force = request.force;
        let table = request.table.clone();
        let start = request.start;
        let count = request.count;
        let mut lines = vec!["Querying database".to_string(), "Reading data".to_string()];
        let mut imported = 0u64;
        let walked = super::fsasset::import_walk(&pool, &table, start, count, async |row| {
            if imported.is_multiple_of(100) {
                lines.push(format!("{imported} assets imported so far"));
            }
            if self.import_one(row, force).await.is_err() {
                return Err(ImportStop::Store);
            }
            imported += 1;
            Ok(())
        })
        .await;
        pool.close().await;
        match walked {
            Ok(()) => {
                lines.push(format!("Import done, {imported} assets imported"));
                Ok(lines)
            }
            Err(ImportStop::Store) => {
                log_fsassets("ERROR", "[FSASSETS]: Error importing assets");
                Ok(lines)
            }
            Err(ImportStop::Query(error)) => {
                let bound = matches!(error, StoreError::Bound);
                if !bound {
                    log_import_failure(backend);
                }
                if bound || imported > 0 {
                    Ok(lines)
                } else {
                    Ok(Vec::new())
                }
            }
        }
    }

    async fn import_one(&self, row: FsImportRow, force: bool) -> Result<(), ()> {
        let id_text = row.id.to_string();
        let mut asset = Asset {
            id: row.id,
            name: row.name,
            description: row.description,
            type_code: row.type_code,
            local: false,
            temporary: false,
            creator_id: String::new(),
            flags: 0,
            data: row.data,
            upload_attempts: 0,
        };
        self.store_asset_inner(&mut asset, &id_text, force).await?;
        Ok(())
    }

    async fn finish_local(
        &self,
        id: &str,
        row: FsAsset,
    ) -> Result<Option<(Asset, String)>, StoreError> {
        let stored_hash = row.hash.clone();
        let data = match read_checked_off(&self.files, &stored_hash, row.metadata.type_code)
            .await
            .map(|done| done.value)
        {
            Ok(Some(data)) => data,
            Ok(None) => {
                let Some(mut asset) = self.take_fallback(id).await? else {
                    return Ok(None);
                };
                if asset.type_code == OBJECT_TYPE && rewrite_object(&mut asset.data).is_err() {
                    log_fsassets("ERROR", &format!("Could not get asset {id}"));
                    return Ok(None);
                }
                let hash = match parse_id(id) {
                    Some(uuid) => self.row_hash(uuid).await?,
                    None => String::new(),
                };
                return Ok(Some((asset, hash)));
            }
            Err(error) => {
                log_fsassets("ERROR", &format!("Could not get asset {id}: {error}"));
                return Ok(None);
            }
        };
        let mut asset = asset_from_row(&row, data);
        if asset.type_code == OBJECT_TYPE && rewrite_object(&mut asset.data).is_err() {
            log_fsassets("ERROR", &format!("Could not get asset {id}"));
            return Ok(None);
        }
        Ok(Some((asset, stored_hash)))
    }

    /// Fallback `Get`, then `Store`. The returned asset keeps `Store`'s data.
    /// `Store`'s id is ignored, including the zero UUID. A throw is a miss.
    async fn take_fallback(&self, id: &str) -> Result<Option<Asset>, StoreError> {
        let Some(fallback) = &self.fallback else {
            return Ok(None);
        };
        let Some(uuid) = parse_id(id) else {
            return Ok(None);
        };
        let Ok(Some(mut asset)) = fallback.get(uuid).await else {
            return Ok(None);
        };
        log_fsassets(
            "INFO",
            &format!("Added asset {id} from fallback to local store"),
        );
        let stored_id = asset.id.to_string();
        if self.store_asset(&mut asset, &stored_id).await.is_err() {
            log_fsassets("ERROR", &format!("Could not get asset {id}"));
            return Ok(None);
        }
        Ok(Some(asset))
    }
}

enum ImportStop {
    Query(StoreError),
    Store,
}

impl From<StoreError> for ImportStop {
    fn from(error: StoreError) -> Self {
        Self::Query(error)
    }
}

struct BlockingDone<T> {
    value: T,
    /// The worker that ran the closure. Production ignores it. The off-thread
    /// test reads this value, which is the closure's own thread.
    #[cfg_attr(not(test), allow(dead_code))]
    thread: std::thread::ThreadId,
}

async fn read_checked_off(
    files: &super::fsfiles::FsFiles,
    hash: &str,
    type_code: i8,
) -> Result<BlockingDone<Option<Vec<u8>>>, StoreError> {
    let files = files.clone();
    let hash = hash.to_owned();
    match tokio::task::spawn_blocking(move || {
        let thread = std::thread::current().id();
        BlockingDone {
            value: files.read_checked(&hash, type_code),
            thread,
        }
    })
    .await
    {
        Ok(BlockingDone {
            value: Ok(value),
            thread,
        }) => Ok(BlockingDone { value, thread }),
        Ok(BlockingDone {
            value: Err(error),
            thread: _,
        }) => Err(error),
        Err(_) => Err(StoreError::Io(std::io::Error::other(
            "fsassets file read stopped",
        ))),
    }
}

enum SpoolHandoff {
    Stored {
        data: Vec<u8>,
        #[cfg_attr(not(test), allow(dead_code))]
        thread: std::thread::ThreadId,
    },
    Failed(Vec<u8>),
    Stopped,
}

async fn write_spool_off(
    files: super::fsfiles::FsFiles,
    hash: String,
    data: Vec<u8>,
) -> SpoolHandoff {
    let written = tokio::task::spawn_blocking(move || {
        let thread = std::thread::current().id();
        let result = files.write_spool(&hash, &data);
        (result, data, thread)
    })
    .await;
    match written {
        Ok((Ok(()), data, thread)) => SpoolHandoff::Stored { data, thread },
        Ok((Err(_), data, _)) => SpoolHandoff::Failed(data),
        Err(_) => SpoolHandoff::Stopped,
    }
}

fn log_import_failure(backend: Backend) {
    let message = match backend {
        Backend::Postgres => "[PGSQL FSASSETS]: Error importing assets",
        _ => "[FSASSETS]: Error importing assets",
    };
    log_fsassets("ERROR", message);
}

fn log_import_connect(backend: Backend) {
    let message = match backend {
        Backend::Postgres => "[PGSQL FSASSETS]: Error importing assets",
        _ => "[FSASSETS]: Can't connect to database",
    };
    log_fsassets("ERROR", message);
}

async fn open_import_pool(backend: Backend, connection: &str) -> Result<Pool, ()> {
    let opened = if connection_is_sqlx_url(connection) {
        Pool::connect_sqlx_url(connection).await
    } else {
        let stripped = without_ado_pool_keys(connection);
        Pool::connect(backend, &stripped).await
    };
    match opened {
        Ok(pool) if pool.backend() == backend => Ok(pool),
        Ok(pool) => {
            pool.close().await;
            log_import_connect(backend);
            Err(())
        }
        Err(_) => {
            log_import_connect(backend);
            Err(())
        }
    }
}

fn asset_from_row(row: &FsAsset, data: Vec<u8>) -> Asset {
    Asset {
        id: row.metadata.id,
        name: row.metadata.name.clone(),
        description: row.metadata.description.clone(),
        type_code: row.metadata.type_code,
        local: false,
        temporary: false,
        creator_id: String::new(),
        flags: row.metadata.flags,
        data,
        upload_attempts: 0,
    }
}

fn truncate_name(asset: &mut Asset) {
    let stored = asset.stored_name();
    if stored != asset.name {
        log_fsassets(
            "WARN",
            &format!(
                "Name for asset {} truncated from {} to {} characters on add",
                asset.id,
                utf16_len(&asset.name),
                utf16_len(&stored)
            ),
        );
        asset.name = stored;
    }
}

fn truncate_description(asset: &mut Asset) {
    let stored = asset.stored_description();
    if stored != asset.description {
        log_fsassets(
            "WARN",
            &format!(
                "Description for asset {} truncated from {} to {} characters on add",
                asset.id,
                utf16_len(&asset.description),
                utf16_len(&stored)
            ),
        );
        asset.description = stored;
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `None` is an id this store cannot write. An empty id is a new UUID.
fn resolve_id(id: Uuid, id_text: &str) -> Option<(Uuid, String)> {
    if id_text.is_empty() {
        let id = if id.is_nil() { Uuid::new_v4() } else { id };
        return Some((id, id.to_string()));
    }
    if let Some(parsed) = parse_id(id_text) {
        return Some((parsed, parsed.to_string()));
    }
    if id.is_nil() {
        return None;
    }
    Some((id, id.to_string()))
}

/// `Utils.BytesToString`: UTF-8, stopping at the first NUL.
fn bytes_to_string(data: &[u8]) -> String {
    let end = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
}

/// `Utils.StringToBytes`: empty stays empty. Any other string is UTF-8 plus a NUL.
fn string_to_bytes(text: &str) -> Vec<u8> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(text.len().saturating_add(1));
    out.extend_from_slice(text.as_bytes());
    out.push(0);
    out
}

fn rewrite_object(data: &mut Vec<u8>) -> Result<(), ()> {
    let clean = sanitize_xml(&bytes_to_string(data))?;
    *data = string_to_bytes(&clean);
    Ok(())
}

/// `ExternalRepresentationUtils.SanitizeXml`. An index past the end is the
/// C# throw. A match at index 0 is left as it is.
fn sanitize_xml(xml: &str) -> Result<String, ()> {
    if xml.trim().is_empty() {
        return Ok(xml.to_string());
    }
    let chars: Vec<char> = xml.chars().collect();
    let needle: Vec<char> = "xmlns:xmlns:".chars().collect();
    let Some(indx) = find_chars(&chars, &needle) else {
        return Ok(xml.to_string());
    };
    if indx == 0 {
        return Ok(xml.to_string());
    }
    let mut indx2 = indx + needle.len();
    loop {
        let Some(probe) = indx2.checked_add(5) else {
            return Err(());
        };
        if probe >= chars.len() {
            return Err(());
        }
        if chars[probe] != ':' {
            break;
        }
        indx2 = indx2.saturating_add(6);
    }
    if indx2 > chars.len() {
        return Err(());
    }
    let bad: String = chars[indx..indx2].iter().collect();
    let text: String = chars.into_iter().collect();
    Ok(text.replace(&bad, "xmlns:"))
}

fn find_chars(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    for start in 0..=last {
        if hay[start..start + needle.len()] == *needle {
            return Some(start);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test(flavor = "current_thread")]
    async fn file_reads_and_spool_writes_run_off_the_async_task() {
        let dir = std::env::temp_dir().join(format!(
            "ferroan-fsassets-block-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("base")).expect("base");
        std::fs::create_dir_all(dir.join("spool")).expect("spool");
        let files = super::super::fsfiles::FsFiles::new(dir.join("base"), dir.join("spool"), false);
        let bytes = b"blocking-bytes".to_vec();
        let hash = super::sha256_hex(&bytes);
        let here = std::thread::current().id();
        let stored = super::write_spool_off(files.clone(), hash.clone(), bytes.clone()).await;
        let super::SpoolHandoff::Stored {
            thread: wrote_on, ..
        } = stored
        else {
            panic!("spool");
        };
        assert_ne!(wrote_on, here, "spool write stayed on the async task");
        let read = super::read_checked_off(&files, &hash, 0)
            .await
            .expect("read");
        assert_eq!(read.value.expect("bytes"), bytes);
        assert_ne!(read.thread, here, "file read stayed on the async task");
        assert!(
            super::read_checked_off(&files, "", 0)
                .await
                .expect("empty hash")
                .value
                .is_none()
        );
        assert!(
            super::read_checked_off(&files, "not-a-stored-hash", 0)
                .await
                .expect("missing")
                .value
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_rewrites_a_doubled_xmlns_and_stops_at_the_colon() {
        let xml = r#"<a xmlns:xmlns:os="urn:x">hi</a>"#;
        let clean = sanitize_xml(xml).expect("xml");
        assert_eq!(clean, r#"<a xmlns:os="urn:x">hi</a>"#);
        // Six characters after the extra `xmlns:` so the next probe does not throw.
        let extra = "zz xmlns:xmlns:xmlns:restXY";
        assert_eq!(sanitize_xml(extra).expect("extra"), "zz xmlns:restXY");
    }

    #[test]
    fn sanitize_leaves_a_match_at_the_start_and_throws_off_the_end() {
        assert_eq!(
            sanitize_xml("xmlns:xmlns:os").expect("start"),
            "xmlns:xmlns:os"
        );
        assert_eq!(sanitize_xml("   ").expect("blank"), "   ");
        assert!(sanitize_xml("x xmlns:xmlns:").is_err());
    }

    #[test]
    fn object_bytes_gain_one_trailing_nul_and_lose_it_on_read() {
        let text = "<root/>";
        let bytes = string_to_bytes(text);
        assert_eq!(bytes.last().copied(), Some(0));
        assert_eq!(bytes_to_string(&bytes), text);
        assert!(string_to_bytes("").is_empty());
        let mut data = b"zz xmlns:xmlns:os=\"u\"".to_vec();
        rewrite_object(&mut data).expect("clean");
        assert_eq!(bytes_to_string(&data), "zz xmlns:os=\"u\"");
        assert_eq!(*data.last().expect("nul"), 0);
    }

    #[test]
    fn an_empty_id_is_generated_and_a_bad_id_is_not_stored_under_another() {
        let (id, text) = resolve_id(Uuid::nil(), "").expect("new");
        assert!(!id.is_nil());
        assert_eq!(text, id.to_string());
        assert!(resolve_id(Uuid::nil(), "not-a-uuid").is_none());
    }

    const ZERO_ID: &str = "00000000-0000-0000-0000-000000000000";

    #[tokio::test]
    async fn fsasset_service_serves_the_asset_routes_when_a_database_is_configured() {
        if let Some(url) = db_url("FERROAN_MYSQL_URL", "MySQL") {
            let _fs = super::super::fsasset::FSASSET_MYSQL.lock().await;
            let _assets = super::super::store::ASSET_MYSQL.lock().await;
            check_service(
                &url,
                "MySQL",
                ferroan_db::Backend::Mysql,
                "OpenSim.Data.MySQL.dll",
            )
            .await;
        }
        if let Some(url) = db_url("FERROAN_POSTGRES_URL", "PostgreSQL") {
            let _fs = super::super::fsasset::FSASSET_POSTGRES.lock().await;
            let _assets = super::super::store::ASSET_POSTGRES.lock().await;
            check_service(
                &url,
                "PostgreSQL",
                ferroan_db::Backend::Postgres,
                "OpenSim.Data.PGSQL.dll",
            )
            .await;
        }
    }

    fn db_url(var: &str, label: &str) -> Option<String> {
        match std::env::var(var) {
            Ok(url) if !url.is_empty() => Some(url),
            Ok(_) => {
                eprintln!("{var} is empty; {label} FSAsset service was not run");
                None
            }
            Err(_) => {
                eprintln!("{var} is unset; {label} FSAsset service was not run");
                None
            }
        }
    }

    async fn check_service(url: &str, label: &str, backend: ferroan_db::Backend, provider: &str) {
        let ado = ado_from_sqlx_url(url).expect("ado");
        let dir =
            std::env::temp_dir().join(format!("ferroan-q23c-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("base")).unwrap();
        std::fs::create_dir_all(dir.join("spool")).unwrap();
        let look = open_fs(backend, &ado, 0).await;
        look.wipe().await.expect("wipe");
        let stock = open_stock(backend, &ado).await;
        let files = super::super::fsfiles::FsFiles::new(dir.join("base"), dir.join("spool"), false);
        let fallback = open_stock(backend, &ado).await;
        let service = FsAssetService::new(
            open_fs(backend, &ado, 0).await,
            files.clone(),
            Some(fallback),
            true,
        );
        assert!(service.main, "{label} direct service is the main instance");
        let app = super::super::http::AssetHttp::from_fsasset(
            service,
            super::super::http::AssetOptions {
                delete: super::super::http::DeleteMode::All,
                ..super::super::http::AssetOptions::default()
            },
        )
        .into_router();
        let mysql = backend == ferroan_db::Backend::Mysql;
        serve_texture(&app, &look, &files, mysql).await;
        serve_object(&app, &files).await;
        serve_names_and_edges(&app, &look, &files, mysql).await;
        serve_fallback(&app, &look, &stock).await;
        serve_delete(&app, &look, &files).await;
        serve_access_interval(backend, &ado, &dir).await;
        serve_open(backend, &ado, provider, &look, &dir).await;
        let _ = stock.delete(fid(5)).await;
        look.wipe().await.expect("wipe");
        look.close().await;
        stock.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn serve_texture(
        app: &axum::Router,
        look: &super::super::fsasset::FsAssetStore,
        files: &super::super::fsfiles::FsFiles,
        mysql: bool,
    ) {
        let id = fid(1);
        let bytes = b"hello-texture";
        let posted = call(
            app,
            "POST",
            "/assets/",
            &super::super::xml::encode_asset(&sample(id, "texture", 0, bytes)),
        )
        .await;
        assert_eq!(posted.status, axum::http::StatusCode::OK);
        assert_eq!(posted.content_type, "application/xml");
        assert_eq!(
            posted.body,
            super::super::xml::encode_string(&id.to_string())
        );
        let asset = call(app, "GET", &format!("/assets/{id}"), &[]).await;
        assert_eq!(asset.status, axum::http::StatusCode::OK);
        assert_eq!(asset.content_type, "text/xml");
        let asset_text = text(&asset.body);
        assert!(!asset_text.contains("CreatorID"));
        assert!(!asset_text.contains("CreationDate"));
        let row = look.get(id).await.expect("row").expect("stored");
        let meta = call(app, "GET", &format!("/assets/{id}/metadata"), &[]).await;
        assert_eq!(meta.status, axum::http::StatusCode::OK);
        assert_eq!(meta.content_type, "image/x-j2c");
        let meta_text = text(&meta.body);
        assert!(meta_text.contains(&super::super::xml::xml_unix_date(row.create_time)));
        assert!(!meta_text.contains("CreatorID"));
        if mysql {
            assert!(meta_text.contains("<Name>texture</Name>"));
        } else {
            assert!(meta_text.contains("<Name />"));
        }
        let data = call(app, "GET", &format!("/assets/{id}/data"), &[]).await;
        assert_eq!(data.status, axum::http::StatusCode::OK);
        assert_eq!(data.content_type, "application/octet-stream");
        assert_eq!(data.body, bytes);
        // A same-second rewrite of access_time still compares equal. Pin a
        // sentinel so existence fails unless it leaves the column alone.
        // Days are 0, so a get would move this value.
        pin_access(look, id, 50).await;
        let exists = exist_call(app, &[&id.to_string()]).await;
        assert_eq!(exists.status, axum::http::StatusCode::OK);
        assert_eq!(exists.body, super::super::xml::encode_bools(&[true]));
        assert_eq!(access_of(look, id).await, 50);
        let hash = super::sha256_hex(bytes);
        let spool_file = files.spool().join(format!("{hash}.asset"));
        std::fs::remove_file(&spool_file).expect("spool");
        let missing = exist_call(app, &[&id.to_string()]).await;
        assert_eq!(missing.body, super::super::xml::encode_bools(&[false]));
        assert_eq!(access_of(look, id).await, 50);
        let gone = call(app, "GET", &format!("/assets/{id}"), &[]).await;
        assert_eq!(gone.status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(gone.content_type, "text/plain");
        let meta_kept = call(app, "GET", &format!("/assets/{id}/metadata"), &[]).await;
        assert_eq!(meta_kept.status, axum::http::StatusCode::OK);
        assert!(look.get(id).await.expect("row").is_some());
        let again = call(
            app,
            "POST",
            "/assets/",
            &super::super::xml::encode_asset(&sample(id, "texture", 0, bytes)),
        )
        .await;
        assert_eq!(
            again.body,
            super::super::xml::encode_string(&id.to_string())
        );
        assert_eq!(
            exist_call(app, &[&id.to_string()]).await.body,
            super::super::xml::encode_bools(&[true])
        );
        assert!(files.writer_pass());
        assert!(files.gzip_path(&hash).is_file());
        assert!(!spool_file.exists());
        let reread = call(app, "GET", &format!("/assets/{id}/data"), &[]).await;
        assert_eq!(reread.body, bytes);
        let blocked = call_shard(app, "GET", &format!("/assets/{id}")).await;
        assert_eq!(blocked.status, axum::http::StatusCode::FORBIDDEN);
    }

    async fn serve_object(app: &axum::Router, files: &super::super::fsfiles::FsFiles) {
        let id = fid(2);
        let original = b"zz xmlns:xmlns:os=\"u\"";
        let posted = call(
            app,
            "POST",
            "/assets/",
            &super::super::xml::encode_asset(&sample(id, "obj", OBJECT_TYPE, original)),
        )
        .await;
        assert_eq!(posted.status, axum::http::StatusCode::OK);
        let hash = super::sha256_hex(original);
        let spool_file = files.spool().join(format!("{hash}.asset"));
        let stored = std::fs::read(&spool_file).expect("object spool");
        assert_eq!(stored.last().copied(), Some(0));
        assert_eq!(super::bytes_to_string(&stored), "zz xmlns:os=\"u\"");
        let data = call(app, "GET", &format!("/assets/{id}/data"), &[]).await;
        assert_eq!(data.body, stored);
        let overwritten = [original.as_slice(), &[0]].concat();
        std::fs::write(&spool_file, &overwritten).unwrap();
        let raw = call(app, "GET", &format!("/assets/{id}/data"), &[]).await;
        assert_eq!(raw.status, axum::http::StatusCode::OK);
        assert_eq!(raw.body, overwritten);
        let full = call(app, "GET", &format!("/assets/{id}"), &[]).await;
        let decoded = super::super::xml::decode_asset(&full.body).expect("asset xml");
        assert_eq!(
            super::bytes_to_string(&decoded.asset.data),
            "zz xmlns:os=\"u\""
        );
        let thrown = call(
            app,
            "POST",
            "/assets/",
            &super::super::xml::encode_asset(&sample(
                fid(12),
                "badxml",
                OBJECT_TYPE,
                b"x xmlns:xmlns:",
            )),
        )
        .await;
        assert_eq!(thrown.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(thrown.body.is_empty());
    }

    async fn serve_names_and_edges(
        app: &axum::Router,
        look: &super::super::fsasset::FsAssetStore,
        files: &super::super::fsfiles::FsFiles,
        mysql: bool,
    ) {
        let id = fid(3);
        let long = "n".repeat(65);
        let posted = call(
            app,
            "POST",
            "/assets/",
            &super::super::xml::encode_asset(&sample(id, &long, 0, b"named")),
        )
        .await;
        assert_eq!(
            posted.body,
            super::super::xml::encode_string(&id.to_string())
        );
        let meta = text(
            &call(app, "GET", &format!("/assets/{id}/metadata"), &[])
                .await
                .body,
        );
        if mysql {
            assert!(meta.contains(&format!("<Name>{}</Name>", "n".repeat(64))));
            assert!(!meta.contains(&format!("<Name>{long}</Name>")));
        } else {
            assert!(meta.contains("<Name />"));
        }
        let empty_id = fid(4);
        let empty = call(
            app,
            "POST",
            "/assets/",
            &super::super::xml::encode_asset(&sample(empty_id, "empty", 0, b"")),
        )
        .await;
        assert_eq!(
            empty.body,
            super::super::xml::encode_string(&empty_id.to_string())
        );
        assert_eq!(
            exist_call(app, &[&empty_id.to_string()]).await.body,
            super::super::xml::encode_bools(&[true])
        );
        let empty_hash = super::sha256_hex(b"");
        assert!(!files.spool().join(format!("{empty_hash}.asset")).exists());
        let before = look.count().await.expect("count");
        let generated = call(app, "POST", "/assets/", &raw_asset("", "aGVsbG8=")).await;
        assert_eq!(generated.status, axum::http::StatusCode::OK);
        let generated_text = text(&generated.body);
        assert!(!generated_text.contains(ZERO_ID));
        assert_eq!(look.count().await.expect("count"), before + 1);
        let bad = call(app, "POST", "/assets/", &raw_asset("not-a-uuid", "YmFk")).await;
        assert_eq!(bad.status, axum::http::StatusCode::OK);
        assert_eq!(bad.body, super::super::xml::encode_string(ZERO_ID));
        assert_eq!(look.count().await.expect("count"), before + 1);
        assert!(
            files
                .spool()
                .join(format!("{}.asset", super::sha256_hex(b"bad")))
                .is_file()
        );
        let missed = call(app, "GET", "/assets/not-a-uuid", &[]).await;
        assert_eq!(missed.status, axum::http::StatusCode::NOT_FOUND);
        let bad_exists = exist_call(app, &["not-a-uuid"]).await;
        assert_eq!(
            bad_exists.status,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(bad_exists.body.is_empty());
        let updated = call(
            app,
            "POST",
            &format!("/assets/{id}"),
            &super::super::xml::encode_asset(&sample(id, "texture", 0, b"named")),
        )
        .await;
        assert_eq!(updated.body, super::super::xml::encode_bool(false));
    }

    async fn serve_fallback(
        app: &axum::Router,
        look: &super::super::fsasset::FsAssetStore,
        stock: &super::super::store::AssetStore,
    ) {
        let id = fid(5);
        let _ = stock.delete(id).await;
        stock
            .store_new(&sample(id, "from-stock", 0, b"fallback-bytes"))
            .await
            .expect("stock");
        assert!(look.get(id).await.expect("fs").is_none());
        let first = call(app, "GET", &format!("/assets/{id}"), &[]).await;
        assert_eq!(first.status, axum::http::StatusCode::OK);
        assert!(text(&first.body).contains("from-stock"));
        assert!(stock.get(id).await.expect("stock").is_some());
        assert!(look.get(id).await.expect("fs").is_some());
        let data = call(app, "GET", &format!("/assets/{id}/data"), &[]).await;
        assert_eq!(data.body, b"fallback-bytes");
        let second = text(&call(app, "GET", &format!("/assets/{id}"), &[]).await.body);
        if look.backend() == ferroan_db::Backend::Mysql {
            assert!(second.contains("from-stock"));
        } else {
            assert!(second.contains("<Name />"));
        }
    }

    async fn serve_delete(
        app: &axum::Router,
        look: &super::super::fsasset::FsAssetStore,
        files: &super::super::fsfiles::FsFiles,
    ) {
        let id = fid(6);
        let bytes = b"delete-me";
        let posted = call(
            app,
            "POST",
            "/assets/",
            &super::super::xml::encode_asset(&sample(id, "del", 0, bytes)),
        )
        .await;
        assert_eq!(posted.status, axum::http::StatusCode::OK);
        let hash = super::sha256_hex(bytes);
        std::fs::remove_file(files.spool().join(format!("{hash}.asset"))).unwrap();
        let kept = call(app, "DELETE", &format!("/assets/{id}"), &[]).await;
        assert_eq!(kept.status, axum::http::StatusCode::OK);
        assert_eq!(kept.body, super::super::xml::encode_bool(false));
        assert!(look.get(id).await.expect("row").is_some());
        let present = fid(1);
        let removed = call(app, "DELETE", &format!("/assets/{present}"), &[]).await;
        assert_eq!(removed.body, super::super::xml::encode_bool(true));
        assert!(look.get(present).await.expect("row").is_none());
        let bad = call(app, "DELETE", "/assets/not-a-uuid", &[]).await;
        assert_eq!(bad.status, axum::http::StatusCode::OK);
        assert_eq!(bad.body, super::super::xml::encode_bool(false));
    }

    async fn serve_access_interval(backend: ferroan_db::Backend, ado: &str, dir: &std::path::Path) {
        let id = fid(8);
        let files = super::super::fsfiles::FsFiles::new(
            dir.join("aged-base"),
            dir.join("aged-spool"),
            false,
        );
        std::fs::create_dir_all(files.base()).unwrap();
        std::fs::create_dir_all(files.spool()).unwrap();
        let service = FsAssetService::new(open_fs(backend, ado, 30).await, files, None, true);
        assert!(service.main);
        let mut asset = sample(id, "aged", 0, b"aged");
        let answered = service
            .store_asset(&mut asset, &id.to_string())
            .await
            .expect("store");
        assert_eq!(answered, id.to_string());
        let first = service.store.clock(id).await.expect("clock").expect("row");
        service
            .get(&id.to_string())
            .await
            .expect("get")
            .expect("asset");
        service
            .get(&id.to_string())
            .await
            .expect("get")
            .expect("asset");
        let second = service.store.clock(id).await.expect("clock").expect("row");
        assert_eq!(first.access_time, second.access_time);
        let bad = FsAssetService::new(
            open_fs(backend, ado, 0).await,
            super::super::fsfiles::FsFiles::new(dir.join("bad-base"), dir.join("not-a-dir"), false),
            None,
            false,
        );
        assert!(!bad.main);
        std::fs::create_dir_all(dir.join("bad-base")).unwrap();
        std::fs::write(dir.join("not-a-dir"), b"x").unwrap();
        let mut missed = sample(fid(9), "spool", 0, b"no-room");
        let answered = bad
            .store_asset(&mut missed, &fid(9).to_string())
            .await
            .expect("nil");
        assert_eq!(answered, ZERO_ID);
        let http = super::super::http::AssetHttp::from_fsasset(
            bad,
            super::super::http::AssetOptions::default(),
        )
        .into_router();
        // The service was moved. Post through a fresh one built the same way below.
        drop(http);
        let again = FsAssetService::new(
            open_fs(backend, ado, 0).await,
            super::super::fsfiles::FsFiles::new(dir.join("bad-base"), dir.join("not-a-dir"), false),
            None,
            false,
        );
        let app = super::super::http::AssetHttp::from_fsasset(
            again,
            super::super::http::AssetOptions::default(),
        )
        .into_router();
        let posted = call(
            &app,
            "POST",
            "/assets/",
            &super::super::xml::encode_asset(&sample(fid(9), "spool", 0, b"no-room")),
        )
        .await;
        assert_eq!(posted.status, axum::http::StatusCode::OK);
        assert_eq!(posted.body, super::super::xml::encode_string(ZERO_ID));
        assert!(service.store.get(fid(9)).await.expect("row").is_none());
    }

    async fn serve_open(
        _backend: ferroan_db::Backend,
        ado: &str,
        provider: &str,
        look: &super::super::fsasset::FsAssetStore,
        dir: &std::path::Path,
    ) {
        super::super::open::reset_fsasset_main_for_test();
        let mut config = fs_config(provider, ado);
        let Err(err) = super::super::http::AssetHttp::open(&config).await else {
            panic!("a missing BaseDirectory was accepted");
        };
        assert!(err.to_string().contains("Configuration error"), "{err}");
        let control = write_loader(&dir.join("load-a"), fid(20));
        config.set(
            "AssetService",
            "BaseDirectory",
            &dir.join("open-a-base").display().to_string(),
        );
        config.set(
            "AssetService",
            "SpoolDirectory",
            &dir.join("open-a-spool").display().to_string(),
        );
        config.set(
            "AssetService",
            "DefaultAssetLoader",
            "OpenSim.Framework.AssetLoader.Filesystem.dll",
        );
        config.set("AssetService", "AssetLoaderArgs", &control);
        config.set("AssetService", "AssetLoaderEnabled", "false");
        drop(
            super::super::http::AssetHttp::open(&config)
                .await
                .expect("loader"),
        );
        assert!(look.get(fid(20)).await.expect("row").is_some());

        super::super::open::reset_fsasset_main_for_test();
        let mut config = fs_config(provider, ado);
        config.set(
            "AssetService",
            "BaseDirectory",
            &dir.join("open-b-base").display().to_string(),
        );
        config.set(
            "AssetService",
            "SpoolDirectory",
            &dir.join("open-b-spool").display().to_string(),
        );
        config.set("AssetService", "DefaultAssetLoader", "Nope.dll");
        let Err(err) = super::super::http::AssetHttp::open(&config).await else {
            panic!("an unknown loader was accepted");
        };
        assert!(err.to_string().contains("could not be loaded"), "{err}");
        config.set(
            "AssetService",
            "DefaultAssetLoader",
            "OpenSim.Framework.AssetLoader.Filesystem.dll",
        );
        config.set(
            "AssetService",
            "AssetLoaderArgs",
            &write_loader(&dir.join("load-b"), fid(21)),
        );
        drop(
            super::super::http::AssetHttp::open(&config)
                .await
                .expect("claimed"),
        );
        assert!(look.get(fid(21)).await.expect("row").is_none());

        super::super::open::reset_fsasset_main_for_test();
        let mut config = fs_config(provider, ado);
        config.set(
            "AssetService",
            "BaseDirectory",
            &dir.join("open-c-base").display().to_string(),
        );
        config.set(
            "AssetService",
            "SpoolDirectory",
            &dir.join("open-c-spool").display().to_string(),
        );
        config.set("AssetService", "SecondaryInstance", "true");
        config.set(
            "AssetService",
            "DefaultAssetLoader",
            "OpenSim.Framework.AssetLoader.Filesystem.dll",
        );
        config.set(
            "AssetService",
            "AssetLoaderArgs",
            &write_loader(&dir.join("load-c"), fid(22)),
        );
        drop(
            super::super::http::AssetHttp::open(&config)
                .await
                .expect("secondary"),
        );
        assert!(look.get(fid(22)).await.expect("row").is_none());
        config.set("AssetService", "SecondaryInstance", "false");
        config.set(
            "AssetService",
            "AssetLoaderArgs",
            &write_loader(&dir.join("load-d"), fid(23)),
        );
        drop(
            super::super::http::AssetHttp::open(&config)
                .await
                .expect("later"),
        );
        assert!(look.get(fid(23)).await.expect("row").is_none());

        super::super::open::reset_fsasset_main_for_test();
        let mut config = fs_config(provider, ado);
        config.set(
            "AssetService",
            "BaseDirectory",
            &dir.join("open-e-base").display().to_string(),
        );
        config.set(
            "AssetService",
            "SpoolDirectory",
            &dir.join("open-e-spool").display().to_string(),
        );
        config.set(
            "AssetService",
            "FallbackService",
            "OpenSim.Services.AssetService.dll:XAssetService",
        );
        drop(
            super::super::http::AssetHttp::open(&config)
                .await
                .expect("fallback"),
        );
        super::super::open::reset_fsasset_main_for_test();
    }

    async fn open_fs(
        backend: ferroan_db::Backend,
        ado: &str,
        days: i32,
    ) -> super::super::fsasset::FsAssetStore {
        let pool = ferroan_db::Pool::open(backend, ado, &["FSAssetStore"])
            .await
            .expect("fs pool");
        let realm = if backend == ferroan_db::Backend::Postgres {
            "not_used"
        } else {
            "fsassets"
        };
        super::super::fsasset::FsAssetStore::with_settings(pool, realm, days).expect("fs")
    }

    async fn open_stock(
        backend: ferroan_db::Backend,
        ado: &str,
    ) -> super::super::store::AssetStore {
        let pool = ferroan_db::Pool::open(backend, ado, &["AssetStore"])
            .await
            .expect("asset pool");
        super::super::store::AssetStore::new(pool).expect("asset")
    }

    fn fs_config(provider: &str, ado: &str) -> ferroan_config::Config {
        let mut config = ferroan_config::Config::default();
        config.set(
            "AssetService",
            "LocalServiceModule",
            "OpenSim.Services.FSAssetService.dll:FSAssetConnector",
        );
        config.set("AssetService", "StorageProvider", provider);
        config.set("AssetService", "ConnectionString", ado);
        config.set("Network", "port", "8003");
        config
    }

    fn write_loader(dir: &std::path::Path, id: Uuid) -> String {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("set.xml"),
            format!(
                "<Nini><Section Name=\"Item\"><Key Name=\"assetID\" Value=\"{id}\"/><Key Name=\"name\" Value=\"lib\"/><Key Name=\"assetType\" Value=\"0\"/><Key Name=\"fileName\" Value=\"\"/></Section></Nini>"
            ),
        )
        .unwrap();
        let control = dir.join("AssetSets.xml");
        std::fs::write(
            &control,
            "<Nini><Section Name=\"One\"><Key Name=\"file\" Value=\"set.xml\"/></Section></Nini>",
        )
        .unwrap();
        control.display().to_string()
    }

    fn sample(id: Uuid, name: &str, type_code: i8, data: &[u8]) -> super::super::model::Asset {
        super::super::model::Asset {
            id,
            name: name.to_string(),
            description: String::new(),
            type_code,
            local: false,
            temporary: false,
            creator_id: String::new(),
            flags: 0,
            data: data.to_vec(),
            upload_attempts: 0,
        }
    }

    fn fid(n: u32) -> Uuid {
        Uuid::parse_str(&format!("f523c000-0000-4000-8000-{n:012x}")).unwrap()
    }

    fn raw_asset(id_text: &str, data_b64: &str) -> Vec<u8> {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<AssetBase><Data>{data_b64}</Data><ID>{id_text}</ID><Name>edge</Name><Type>0</Type><Local>false</Local><Temporary>false</Temporary><Flags>Normal</Flags></AssetBase>"
        )
        .into_bytes()
    }

    struct Answer {
        status: axum::http::StatusCode,
        content_type: String,
        body: Vec<u8>,
    }

    async fn call(app: &axum::Router, method: &str, uri: &str, body: &[u8]) -> Answer {
        send(app, method, uri, body, false).await
    }

    async fn call_shard(app: &axum::Router, method: &str, uri: &str) -> Answer {
        send(app, method, uri, &[], true).await
    }

    async fn send(app: &axum::Router, method: &str, uri: &str, body: &[u8], shard: bool) -> Answer {
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        if shard {
            builder = builder.header("X-SecondLife-Shard", "region");
        }
        let response = app
            .clone()
            .oneshot(builder.body(axum::body::Body::from(body.to_vec())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = axum::body::to_bytes(response.into_body(), 2_000_000)
            .await
            .unwrap()
            .to_vec();
        Answer {
            status,
            content_type,
            body,
        }
    }

    async fn access_of(look: &super::super::fsasset::FsAssetStore, id: Uuid) -> i32 {
        look.clock(id)
            .await
            .expect("clock")
            .expect("row")
            .access_time
    }

    async fn pin_access(look: &super::super::fsasset::FsAssetStore, id: Uuid, access: i32) {
        match look.backend() {
            ferroan_db::Backend::Mysql => {
                sqlx::query("UPDATE fsassets SET access_time = ? WHERE id = ?")
                    .bind(access)
                    .bind(id.to_string())
                    .execute(look.pool().mysql().expect("mysql"))
                    .await
                    .expect("access");
            }
            ferroan_db::Backend::Postgres => {
                sqlx::query("UPDATE fsassets SET access_time = $2 WHERE id = $1::uuid")
                    .bind(id.to_string())
                    .bind(access)
                    .execute(look.pool().postgres().expect("postgres"))
                    .await
                    .expect("access");
            }
            ferroan_db::Backend::Sqlite => panic!("sqlite"),
        }
    }

    async fn exist_call(app: &axum::Router, ids: &[&str]) -> Answer {
        call(
            app,
            "POST",
            "/get_assets_exist",
            &super::super::xml::encode_strings(ids),
        )
        .await
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("utf-8")
    }

    fn ado_from_sqlx_url(url: &str) -> Option<String> {
        let mysql = url.starts_with("mysql://");
        let rest = url
            .strip_prefix("mysql://")
            .or_else(|| url.strip_prefix("postgres://"))
            .or_else(|| url.strip_prefix("postgresql://"))?;
        let (userinfo, hostpart) = rest.rsplit_once('@')?;
        let (user, pass) = userinfo.split_once(':')?;
        if user.is_empty()
            || pass.is_empty()
            || user.contains([';', '\n', '\r'])
            || pass.contains([';', '\n', '\r'])
        {
            return None;
        }
        let (hostport, dbquery) = hostpart.split_once('/')?;
        let db = dbquery.split(['?', '#']).next().unwrap_or("");
        if db.is_empty() || db.contains([';', '\n', '\r']) {
            return None;
        }
        let (host, port) = match hostport.split_once(':') {
            Some((host, port)) => (host, port),
            None => (hostport, if mysql { "3306" } else { "5432" }),
        };
        if host.is_empty() || port.parse::<u16>().is_err() {
            return None;
        }
        if mysql {
            Some(format!(
                "Server={host};Port={port};Database={db};User ID={user};Password={pass};SslMode=None"
            ))
        } else {
            Some(format!(
                "Server={host};Port={port};Database={db};User Id={user};Password={pass};SSL Mode=Disable"
            ))
        }
    }
}
