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
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// The fixed read and write follow the maintainer's branches
//   fix/fsassets-damaged-file, fix/fsassets-writer-errors and
//   fix/fsassets-store-spool-errors, except that a file blocking a directory
//   is renamed aside (D31) and is not deleted.
// QR14 follows FSAssetService.cs (ReadFsFile, Store, WriteSpoolFile, WriteStoreFile)
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b, with spool repair per D76.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! FSAssets file layout and spool writer. No database, no HTTP and no service.
//!
//! `HashToPath` is `hash[0..2]/[2..4]/[4..6]/[6..10]`, or `hash[0..3]/[3..6]`
//! when `UseOsgridFormat` is set. A hash shorter than 10 characters is
//! `junkyard`. The file is that path plus the hash. The writer stores
//! `BaseDirectory/<path>/<hash>.gz`.
//!
//! `Store` writes `SpoolDirectory/spool/<hash>.asset.<id>.tmp`, forces it to
//! disk and renames it to `SpoolDirectory/<hash>.asset` (TB36). A failed
//! write leaves no file and is an error. The writer reads only `*.asset`
//! files in `SpoolDirectory`, not the nested `spool/` directory. It gzip-writes
//! a temporary file beside the destination, forces it to disk and renames it
//! onto the `.gz` (TB24). A pass that cannot write keeps the spool file and
//! returns, so the caller can try again. It does not end the process. The
//! pause starts at 1000 ms and doubles up to 60000 ms after I/O failures.
//! Large spools resume on the ordinary interval.
//!
//! A missing, empty (unless the hash is of no bytes), unreadable or
//! mismatching file is missing. Storing the same bytes writes the file again
//! (TB23). An object also matches the hash of its bytes without a trailing
//! NUL. Fetch accepts a whole object as Pin8 does. Store can also compare
//! a spooled object with the caller's sanitized XML (TB91).
//!
//! A file that occupies a directory component is renamed aside in the same
//! directory with a `.blocked-<time>` suffix and a warning through
//! `ferroan_diag` (D31). It is not deleted, and the new name is not an asset.
//! A damaged spool falls back to the base copy. A repeat store replaces it
//! (D76). Unix syncs directory entries as well as file contents.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::store::StoreError;

/// `AssetType.Object`. Store may write an object with a trailing NUL.
pub(super) const OBJECT_TYPE: i8 = 6;

/// How many directory obstacles one write will move aside.
const BLOCK_RENAMES: u32 = 16;
const SPOOL_ENTRIES_PER_PASS: usize = 256;

// Coordinate a repair with a writer of that hash, including other FsFiles
// instances in this process. Different hash prefixes can publish concurrently.
fn publication_lock(hash: &str) -> std::sync::MutexGuard<'static, ()> {
    static LOCKS: [Mutex<()>; 16] = [const { Mutex::new(()) }; 16];
    let index = hash
        .chars()
        .next()
        .and_then(|c| c.to_digit(16))
        .unwrap_or(0);
    LOCKS[index as usize]
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The writer's pause between passes, in milliseconds.
pub const WRITER_INTERVAL_MS: u64 = 1000;

/// The longest pause after a pass that left files in the spool.
pub const WRITER_MAX_RETRY_MS: u64 = 60_000;

/// One FSAssets directory pair: `BaseDirectory` and `SpoolDirectory`.
#[derive(Debug, Clone)]
pub struct FsFiles {
    base: PathBuf,
    spool: PathBuf,
    osgrid: bool,
    scan: Arc<Mutex<Option<std::iter::Peekable<fs::ReadDir>>>>,
}

#[derive(Clone, Copy)]
enum FileUse<'a> {
    Fetch,
    Store(&'a dyn Fn(&[u8]) -> bool),
}

impl FsFiles {
    /// `base` is `BaseDirectory`. `spool` is `SpoolDirectory`.
    /// `osgrid` is `UseOsgridFormat`.
    pub fn new(base: impl Into<PathBuf>, spool: impl Into<PathBuf>, osgrid: bool) -> Self {
        let files = Self {
            base: base.into(),
            spool: spool.into(),
            osgrid,
            scan: Arc::default(),
        };
        warn_if_spool_is_temporary(&files.spool);
        files
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    pub fn spool(&self) -> &Path {
        &self.spool
    }

    pub const fn osgrid(&self) -> bool {
        self.osgrid
    }

    /// Relative path of the file, without `.gz`. This is `HashToFile`.
    pub fn relative_file(&self, hash: &str) -> PathBuf {
        hash_to_file(hash, self.osgrid)
    }

    /// The gzip path under `BaseDirectory`.
    pub fn gzip_path(&self, hash: &str) -> PathBuf {
        gzip_path(&self.base, &self.relative_file(hash))
    }

    /// Publish complete spool bytes. A Unix directory sync error is returned
    /// even if the rename already made the complete file visible.
    pub fn write_spool(&self, hash: &str, data: &[u8]) -> Result<(), StoreError> {
        let folder = self.spool.join("spool");
        let dest = self.spool.join(format!("{hash}.asset"));
        let _publish = publication_lock(hash);
        create_directories(&folder)
            .and_then(|()| {
                publish_file(&folder, &dest, |mut file| {
                    file.write_all(data)?;
                    file.sync_all()
                })
            })
            .inspect_err(|error| {
                report_fsassets(
                    "ERROR",
                    &format!(
                        "could not write {} to the spool, the asset is not stored: {error}",
                        dest.display()
                    ),
                )
            })
            .map_err(Into::into)
    }

    /// Process one bounded batch. Only I/O failures increase the retry interval.
    pub fn write_once(&self) -> Result<bool, StoreError> {
        let mut cursor = self.scan.lock().unwrap_or_else(|e| e.into_inner());
        if cursor.is_none() {
            match fs::read_dir(&self.spool) {
                Ok(entries) => *cursor = Some(entries.peekable()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
                Err(error) => return Err(error.into()),
            }
        }
        let Some(entries) = cursor.as_mut() else {
            return Ok(true);
        };
        let succeeded = self.flush_batch(entries.by_ref().take(SPOOL_ENTRIES_PER_PASS));
        if entries.peek().is_some() {
            ferroan_diag::hit(&ferroan_diag::Hit {
                id: ferroan_diag::ASSET_FSASSETS_SPOOL_PASS,
                kind: ferroan_diag::Kind::Soft,
                configured: SPOOL_ENTRIES_PER_PASS as u64,
                observed: SPOOL_ENTRIES_PER_PASS as u64 + 1,
                action: ferroan_diag::Action::Delay,
                route: "spool writer",
                agent_id: None,
            });
        } else {
            *cursor = None;
        }
        Ok(succeeded)
    }

    fn flush_batch(&self, entries: impl Iterator<Item = io::Result<fs::DirEntry>>) -> bool {
        let started = std::time::Instant::now();
        let mut written = 0;
        let mut failed = 0;
        for entry in entries {
            match entry.and_then(|entry| self.flush_entry(entry)) {
                Ok(true) => written += 1,
                Ok(false) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => {
                    if failed == 0 {
                        report_fsassets(
                            "ERROR",
                            &format!("spool entry in {}: {error}", self.spool.display()),
                        );
                    }
                    failed += 1;
                }
            }
        }
        if failed > 0 {
            report_fsassets(
                "ERROR",
                &format!(
                    "write cycle wrote {written} files; {failed} entries failed and will be retried"
                ),
            );
        } else if written > 0 {
            let elapsed = started.elapsed().as_millis();
            report_fsassets(
                "INFO",
                &format!("write cycle complete, {written} files, {elapsed} ms"),
            );
        }
        failed == 0
    }

    fn flush_entry(&self, entry: fs::DirEntry) -> io::Result<bool> {
        let file = entry.path();
        if file.extension().and_then(|s| s.to_str()) != Some("asset") {
            return Ok(false);
        }
        if !entry.file_type()?.is_file() {
            return Ok(false);
        }
        let Some(hash) = file.file_stem().and_then(|s| s.to_str()) else {
            return Ok(false);
        };
        let relative = self.relative_file(hash);
        if !relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
        {
            warn_fsassets(&format!(
                "skipping invalid spool filename {}",
                file.display()
            ));
            return Ok(false);
        }
        let _publish = publication_lock(hash);
        write_store_file(&file, &gzip_path(&self.base, &relative), hash)
            .map(|()| true)
            .map_err(|error| {
                io::Error::other(format!("could not flush {}: {error}", file.display()))
            })
    }

    /// One writer pass that cannot end the process. `false` means try again.
    pub fn writer_pass(&self) -> bool {
        match self.write_once() {
            Ok(written) => written,
            Err(error) => {
                report_fsassets(
                    "ERROR",
                    &format!("writer pass over {} failed: {error}", self.spool.display()),
                );
                false
            }
        }
    }

    /// A store must replace a damaged spool even if a good gzip can serve reads.
    pub(super) fn needs_spool(
        &self,
        hash: &str,
        type_code: i8,
        matches_rewritten_object: impl Fn(&[u8]) -> bool,
    ) -> Result<bool, StoreError> {
        Ok(self
            .checked_content(hash, type_code, FileUse::Store(&matches_rewritten_object))?
            .is_none())
    }

    /// Return verified spool or base bytes. A genuine empty asset needs no file.
    pub fn read_checked(&self, hash: &str, type_code: i8) -> Result<Option<Vec<u8>>, StoreError> {
        self.checked_content(hash, type_code, FileUse::Fetch)
    }

    fn checked_content(
        &self,
        hash: &str,
        type_code: i8,
        purpose: FileUse<'_>,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        if hash.is_empty() {
            return Ok(None);
        }
        if hash_eq(hash, &sha256_hex(&[])) {
            return Ok(Some(Vec::new()));
        }
        let spool = self.spool.join(format!("{hash}.asset"));
        match fs::read(&spool) {
            Ok(data) => {
                // For a fetch, keep Pin8's whole-object exception. A store also
                // has the original bytes and can verify the sanitized object.
                let matches = content_matches(&data, hash, type_code, || match purpose {
                    FileUse::Fetch => true,
                    FileUse::Store(matches_rewritten) => matches_rewritten(&data),
                });
                if matches {
                    if matches!(purpose, FileUse::Store(_)) {
                        // A previous rename may have succeeded but its directory sync failed.
                        sync_directory(&self.spool)?;
                    }
                    return Ok(Some(data));
                }
                warn_mismatch(&spool, hash, data.len());
                if matches!(purpose, FileUse::Store(_)) {
                    return Ok(None);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => {
                warn_fsassets(&format!("could not read {}: {error}", spool.display()));
                if matches!(purpose, FileUse::Store(_)) {
                    return Ok(None);
                }
            }
        }
        let Some((data, file, whole)) = self.read_base(hash)? else {
            return Ok(None);
        };
        if content_matches(&data, hash, type_code, || whole) {
            return Ok(Some(data));
        }
        warn_mismatch(&file, hash, data.len());
        Ok(None)
    }

    fn read_base(&self, hash: &str) -> Result<Option<(Vec<u8>, PathBuf, bool)>, StoreError> {
        let disk = self.base.join(hash_to_file(hash, self.osgrid));
        let gzip = gzip_path_of(&disk);
        if gzip.is_file() {
            return Ok(read_gzip(&gzip).map(|(data, whole)| (data, gzip, whole)));
        }
        if disk.is_file()
            && let Some(data) = read_optional(&disk)?
        {
            return Ok(Some((data, disk, true)));
        }
        Ok(None)
    }
}

/// The next pause. A pass that wrote every file returns to 1000 ms. A pass
/// that left files behind doubles the pause, up to 60000 ms. `current_ms` of
/// 0 is the starting pause.
pub fn next_writer_interval(current_ms: u64, wrote_all: bool) -> u64 {
    if wrote_all {
        return WRITER_INTERVAL_MS;
    }
    current_ms
        .max(WRITER_INTERVAL_MS)
        .saturating_mul(2)
        .min(WRITER_MAX_RETRY_MS)
}

/// SHA-256 hex, lowercase, as `Util.SHA256Hash` writes it.
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// `HashToPath`: the directory under `BaseDirectory`, without the hash.
pub fn hash_to_path(hash: &str, osgrid: bool) -> PathBuf {
    let chars: Vec<char> = hash.chars().collect();
    if chars.len() < 10 {
        return PathBuf::from("junkyard");
    }
    let take = |start: usize, len: usize| chars[start..start + len].iter().collect::<String>();
    if osgrid {
        PathBuf::from(take(0, 3)).join(take(3, 3))
    } else {
        PathBuf::from(take(0, 2))
            .join(take(2, 2))
            .join(take(4, 2))
            .join(take(6, 4))
    }
}

/// `HashToFile`.
pub fn hash_to_file(hash: &str, osgrid: bool) -> PathBuf {
    hash_to_path(hash, osgrid).join(hash)
}

fn gzip_path(base: &Path, relative: &Path) -> PathBuf {
    gzip_path_of(&base.join(relative))
}

fn gzip_path_of(disk: &Path) -> PathBuf {
    let mut name = disk.as_os_str().to_os_string();
    name.push(".gz");
    PathBuf::from(name)
}

fn content_matches(
    data: &[u8],
    hash: &str,
    type_code: i8,
    accept_rewritten: impl FnOnce() -> bool,
) -> bool {
    if hash_eq(&sha256_hex(data), hash) {
        return true;
    }
    if type_code != OBJECT_TYPE || data.is_empty() {
        return false;
    }
    if data[data.len() - 1] == 0 && hash_eq(&sha256_hex(&data[..data.len() - 1]), hash) {
        return true;
    }
    accept_rewritten()
}

fn hash_eq(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

/// Directory entries need their own flush on Unix. Rust's standard library
/// has no portable directory flush on Windows; file contents are still synced.
fn sync_directory(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        fs::File::open(dir)?.sync_all()?;
        #[cfg(test)]
        tests::record_sync(dir)?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn create_directories(dir: &Path) -> io::Result<()> {
    if dir.as_os_str().is_empty() || dir.is_dir() {
        return Ok(());
    }
    let parent = dir
        .parent()
        .ok_or_else(|| io::Error::other("directory has no parent"))?;
    create_directories(parent)?;
    match fs::create_dir(dir) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => {
            return sync_directory(parent);
        }
        Err(e) => return Err(e),
    }
    if let Err(error) = sync_directory(parent) {
        // An empty unpublished directory must be created and synced on retry.
        let _ = fs::remove_dir(dir);
        return Err(error);
    }
    Ok(())
}

fn warn_mismatch(file: &Path, hash: &str, bytes: usize) {
    warn_fsassets(&format!(
        "{} holds {bytes} bytes that do not match hash {hash}",
        file.display()
    ));
}

fn write_store_file(spool_file: &Path, store_file: &Path, hash: &str) -> io::Result<()> {
    let data = match fs::read(spool_file) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    // A writer has no type metadata. Rewritten objects may legitimately have a
    // different hash. Preserve a verifiably good gzip over mismatching spool bytes.
    let plain = store_file.with_extension("");
    let keep_base = if !store_file.is_file() && !plain.is_file() || hash_matches(&data, hash) {
        false
    } else if store_file.is_file() {
        read_gzip(store_file).is_some_and(|(old, _)| hash_matches(&old, hash))
    } else {
        fs::read(plain).is_ok_and(|old| hash_matches(&old, hash))
    };
    let parent = store_file
        .parent()
        .ok_or_else(|| io::Error::other("missing gzip parent"))?;
    if keep_base {
        sync_directory(parent)?;
    } else {
        make_directory_for(store_file)?;
        publish_file(parent, store_file, |file| gzip_to(file, &data))?;
    }
    remove_spool(spool_file)
}

/// The callback writes and syncs the contents. Only the creator cleans up the
/// temporary file. A successful rename is durable on Unix before we return.
fn publish_file(
    temp_dir: &Path,
    dest: &Path,
    write: impl FnOnce(fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::other("missing destination parent"))?;
    let mut name = dest
        .file_name()
        .ok_or_else(|| io::Error::other("missing destination name"))?
        .to_os_string();
    name.push(format!(".{}.tmp", Uuid::new_v4().simple()));
    let temp = temp_dir.join(name);
    let file = fs::File::create_new(&temp)?;
    if let Err(error) = write(file).and_then(|()| fs::rename(&temp, dest)) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    sync_directory(parent)?;
    if temp_dir != parent {
        sync_directory(temp_dir)?;
    }
    Ok(())
}

fn remove_spool(file: &Path) -> io::Result<()> {
    match fs::remove_file(file) {
        Ok(()) => sync_directory(
            file.parent()
                .ok_or_else(|| io::Error::other("missing spool parent"))?,
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn hash_matches(data: &[u8], hash: &str) -> bool {
    hash_eq(&sha256_hex(data), hash)
}

fn gzip_to(file: fs::File, data: &[u8]) -> io::Result<()> {
    let mut gzip = GzEncoder::new(file, Compression::default());
    gzip.write_all(data)?;
    let file = gzip.finish()?;
    file.sync_all()?;
    Ok(())
}

fn read_gzip(path: &Path) -> Option<(Vec<u8>, bool)> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            warn_fsassets(&format!("could not read {}: {error}", path.display()));
            return None;
        }
    };
    let mut decoder = GzDecoder::new(bytes.as_slice());
    let mut data = Vec::new();
    if let Err(error) = io::Read::read_to_end(&mut decoder, &mut data) {
        warn_fsassets(&format!("could not read {}: {error}", path.display()));
        return None;
    }
    let whole = gzip_trailer_matches(&bytes, data.len());
    Some((data, whole))
}

fn gzip_trailer_matches(bytes: &[u8], length: usize) -> bool {
    if bytes.len() < 18 {
        return false;
    }
    let start = bytes.len() - 4;
    let trailer = u32::from_le_bytes([
        bytes[start],
        bytes[start + 1],
        bytes[start + 2],
        bytes[start + 3],
    ]);
    trailer == length as u32
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            warn_fsassets(&format!("could not read {}: {error}", path.display()));
            Ok(None)
        }
    }
}

/// Make the hash path, preserving a file that obstructs it (D31).
fn make_directory_for(disk_file: &Path) -> io::Result<()> {
    let dir = disk_file
        .parent()
        .ok_or_else(|| io::Error::other("missing gzip parent"))?;
    for _ in 0..BLOCK_RENAMES {
        match create_directories(dir) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let Some(obstacle) = dir.ancestors().find(|path| path.is_file()) else {
                    return Err(error);
                };
                rename_aside(obstacle)?;
            }
        }
    }
    Err(io::Error::other(
        "too many blocking files in the asset directory",
    ))
}

fn rename_aside(path: &Path) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("the blocking path has no file name"))?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("the blocking path has no directory"))?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let name = name.to_string_lossy();
    for extra in 0..1000u32 {
        let suffix = if extra == 0 {
            format!("{name}.blocked-{millis}")
        } else {
            format!("{name}.blocked-{millis}-{extra}")
        };
        let dest = parent.join(suffix);
        if dest.exists() {
            continue;
        }
        fs::rename(path, &dest)?;
        sync_directory(parent)?;
        warn_fsassets(&format!(
            "renamed blocking file {} aside to {}; it is not an asset",
            path.display(),
            dest.display()
        ));
        return Ok(());
    }
    Err(io::Error::other(
        "could not choose a name for the blocking file",
    ))
}

fn warn_if_spool_is_temporary(spool: &Path) {
    let Ok(spool) = prefixed(spool) else {
        return;
    };
    let mut temps = vec![PathBuf::from("/tmp")];
    temps.push(std::env::temp_dir());
    for temp in temps {
        let Ok(root) = prefixed(&temp) else {
            continue;
        };
        if spool.starts_with(&root) {
            warn_fsassets(&format!(
                "SpoolDirectory {} is in the temporary directory {}. Assets wait there until they are written to BaseDirectory, and are lost if it is cleared, as at a restart. Set SpoolDirectory to a directory on the same disk as BaseDirectory.",
                spool.display(),
                root.display()
            ));
            return;
        }
    }
}

fn prefixed(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut text = absolute.into_os_string();
    if !text.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR) {
        text.push(std::path::MAIN_SEPARATOR_STR);
    }
    Ok(PathBuf::from(text))
}

fn warn_fsassets(message: &str) {
    report_fsassets("WARN", message);
}

pub(super) fn log_fsassets(level: &str, message: &str) {
    report_fsassets(level, message);
}

fn report_fsassets(level: &str, message: &str) {
    match level {
        "INFO" => tracing::info!(target: "ferroan::fsassets", "{message}"),
        "ERROR" => tracing::error!(target: "ferroan::fsassets", "{message}"),
        _ => tracing::warn!(target: "ferroan::fsassets", "{message}"),
    }
    ferroan_diag::push_log(level, "ferroan::fsassets", message);
}

#[cfg(test)]
#[path = "fsfiles_tests.rs"]
mod tests;
