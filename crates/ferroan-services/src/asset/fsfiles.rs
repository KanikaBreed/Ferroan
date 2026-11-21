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
//   OpenSim-NGC/OpenSim-Tranquillity @ c70fee205f64ee8672a8d607cef6a733931da951
// The fixed read and write follow the maintainer's branches
//   fix/fsassets-damaged-file, fix/fsassets-writer-errors and
//   fix/fsassets-store-spool-errors, except that a file blocking a directory
//   is renamed aside (D31) and is not deleted.
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
//! pause starts at 1000 ms and doubles up to 60000 ms while files remain.
//!
//! A missing, empty (unless the hash is of no bytes), unreadable or
//! mismatching file is missing. Storing the same bytes writes the file again
//! (TB23). An object also matches the hash of its bytes without a trailing
//! NUL, and a whole object file is kept, as the C# fix does.
//!
//! A file that occupies a directory component is renamed aside in the same
//! directory with a `.blocked-<time>` suffix and a warning through
//! `ferroan_diag` (D31). It is not deleted, and the new name is not an asset.
//!
//! This does not change TB19 or TB20 on `AssetService`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::store::StoreError;

/// `AssetType.Object`. Store may write an object with a trailing NUL.
const OBJECT_TYPE: i8 = 6;

/// How many directory obstacles one write will move aside.
const BLOCK_RENAMES: u32 = 16;

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
}

impl FsFiles {
    /// `base` is `BaseDirectory`. `spool` is `SpoolDirectory`.
    /// `osgrid` is `UseOsgridFormat`.
    pub fn new(base: impl Into<PathBuf>, spool: impl Into<PathBuf>, osgrid: bool) -> Self {
        let files = Self {
            base: base.into(),
            spool: spool.into(),
            osgrid,
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

    /// Relative path of the file, without `.gz`. This is `HashToFile`.
    pub fn relative_file(&self, hash: &str) -> PathBuf {
        hash_to_file(hash, self.osgrid)
    }

    /// The gzip path under `BaseDirectory`.
    pub fn gzip_path(&self, hash: &str) -> PathBuf {
        gzip_path(&self.base, &self.relative_file(hash))
    }

    /// Writes the spool file. An error leaves no partial file (TB36).
    pub fn write_spool(&self, hash: &str, data: &[u8]) -> Result<(), StoreError> {
        let folder = self.spool.join("spool");
        fs::create_dir_all(&folder)?;
        let final_file = self.spool.join(format!("{hash}.asset"));
        let part = folder.join(format!("{hash}.asset.{}.tmp", Uuid::new_v4().simple()));
        if let Err(error) = write_synced(&part, data) {
            let _ = fs::remove_file(&part);
            report_fsassets(
                "ERROR",
                &format!(
                    "could not write {} to the spool, the asset is not stored: {error}",
                    final_file.display()
                ),
            );
            return Err(error.into());
        }
        if let Err(error) = rename_over(&part, &final_file) {
            let _ = fs::remove_file(&part);
            report_fsassets(
                "ERROR",
                &format!(
                    "could not write {} to the spool, the asset is not stored: {error}",
                    final_file.display()
                ),
            );
            return Err(error.into());
        }
        Ok(())
    }

    /// One writer pass. `Ok(true)` means every `*.asset` file was written.
    /// `Ok(false)` means at least one stayed in the spool (TB24).
    pub fn write_once(&self) -> Result<bool, StoreError> {
        let entries = match fs::read_dir(&self.spool) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error.into()),
        };
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".asset") {
                continue;
            }
            if entry.file_type()?.is_file() {
                files.push(entry.path());
            }
        }
        if files.is_empty() {
            return Ok(true);
        }
        let started = std::time::Instant::now();
        let mut failed = 0usize;
        for file in &files {
            let hash = file
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            let store = gzip_path(&self.base, &hash_to_file(&hash, self.osgrid));
            if let Err(error) = write_store_file(file, &store) {
                if failed == 0 {
                    report_fsassets(
                        "ERROR",
                        &format!(
                            "could not write {} to {}, it stays in the spool to be written again: {error}",
                            file.display(),
                            store.display()
                        ),
                    );
                }
                failed = failed.saturating_add(1);
            }
        }
        let elapsed = started.elapsed().as_millis();
        if failed > 0 {
            report_fsassets(
                "ERROR",
                &format!(
                    "write cycle wrote {} of {} files; the others stay in the spool to be written again",
                    files.len() - failed,
                    files.len()
                ),
            );
        } else if elapsed > 0 {
            let average = elapsed as f64 / files.len() as f64;
            report_fsassets(
                "INFO",
                &format!(
                    "write cycle complete, {} files, {elapsed} ticks, avg {average:.2}",
                    files.len()
                ),
            );
        }
        Ok(failed == 0)
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

    /// Writes the file when the stored bytes are missing or do not match.
    /// A spool write that fails is an error and leaves no partial file.
    /// `Ok(false)` means the spool file is still waiting for another pass.
    pub fn store_bytes(&self, hash: &str, type_code: i8, data: &[u8]) -> Result<bool, StoreError> {
        if self.read_checked(hash, type_code)?.is_some() {
            return Ok(true);
        }
        self.write_spool(hash, data)?;
        self.write_once()
    }

    /// The file bytes when they are the content for `hash`, or `None` when
    /// the file is missing, empty, unreadable or mismatching (TB23).
    pub fn read_checked(&self, hash: &str, type_code: i8) -> Result<Option<Vec<u8>>, StoreError> {
        if hash.is_empty() {
            return Ok(None);
        }
        if hash_eq(hash, &sha256_hex(&[])) {
            return Ok(Some(Vec::new()));
        }
        let Some((data, file, whole)) = self.read_file(hash)? else {
            return Ok(None);
        };
        if content_matches(&data, hash, type_code, whole) {
            return Ok(Some(data));
        }
        warn_fsassets(&format!(
            "{} holds {} bytes that do not match hash {hash}; assets with that hash are answered as missing until their content is stored again",
            file.display(),
            data.len()
        ));
        Ok(None)
    }

    fn read_file(&self, hash: &str) -> Result<Option<(Vec<u8>, PathBuf, bool)>, StoreError> {
        let spool_file = self.spool.join(format!("{hash}.asset"));
        if let Some(data) = read_optional(&spool_file)? {
            return Ok(Some((data, spool_file, true)));
        }
        let disk = self.base.join(hash_to_file(hash, self.osgrid));
        let gzip = gzip_path_of(&disk);
        if gzip.is_file() {
            return Ok(read_gzip(&gzip)?.map(|(data, whole)| (data, gzip, whole)));
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

fn content_matches(data: &[u8], hash: &str, type_code: i8, whole: bool) -> bool {
    if hash_eq(&sha256_hex(data), hash) {
        return true;
    }
    if type_code != OBJECT_TYPE || data.is_empty() {
        return false;
    }
    if data[data.len() - 1] == 0 && hash_eq(&sha256_hex(&data[..data.len() - 1]), hash) {
        return true;
    }
    whole
}

fn hash_eq(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn write_synced(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}

fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(to)?;
            fs::rename(from, to)
        }
        Err(error) => Err(error),
    }
}

fn write_store_file(spool_file: &Path, store_file: &Path) -> io::Result<()> {
    if !make_directory_for(store_file)? {
        return Err(io::Error::other(
            "could not create the directory for the asset file",
        ));
    }
    let data = fs::read(spool_file)?;
    let mut temp_name = store_file.as_os_str().to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = PathBuf::from(temp_name);
    let written = gzip_to(&temp, &data);
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    if let Err(error) = rename_over(&temp, store_file) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    fs::remove_file(spool_file)?;
    Ok(())
}

fn gzip_to(path: &Path, data: &[u8]) -> io::Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    let mut gzip = GzEncoder::new(file, Compression::default());
    gzip.write_all(data)?;
    let file = gzip.finish()?;
    file.sync_all()?;
    Ok(())
}

fn read_gzip(path: &Path) -> Result<Option<(Vec<u8>, bool)>, StoreError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            warn_fsassets(&format!("could not read {}: {error}", path.display()));
            return Ok(None);
        }
    };
    let mut decoder = GzDecoder::new(bytes.as_slice());
    let mut data = Vec::new();
    if let Err(error) = io::Read::read_to_end(&mut decoder, &mut data) {
        warn_fsassets(&format!("could not read {}: {error}", path.display()));
        return Ok(None);
    }
    let whole = gzip_trailer_matches(&bytes, data.len());
    Ok(Some((data, whole)))
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

/// Makes the directory that holds `disk_file`. A file in the way is renamed
/// aside (D31). `Ok(false)` means the directory could not be made.
fn make_directory_for(disk_file: &Path) -> io::Result<bool> {
    for _ in 0..BLOCK_RENAMES {
        let Some(dir) = disk_file.parent() else {
            return Ok(false);
        };
        if fs::create_dir_all(dir).is_ok() {
            return Ok(true);
        }
        let Some(bone) = blocking_component(dir) else {
            warn_fsassets(&format!(
                "could not resolve path creation error for {}",
                disk_file.display()
            ));
            return Ok(false);
        };
        let Ok(meta) = fs::metadata(&bone) else {
            warn_fsassets(&format!(
                "could not resolve path creation error for {}",
                disk_file.display()
            ));
            return Ok(false);
        };
        if !meta.is_file() {
            warn_fsassets(&format!(
                "could not resolve path creation error for {}",
                disk_file.display()
            ));
            return Ok(false);
        }
        rename_aside(&bone)?;
    }
    warn_fsassets(&format!(
        "could not resolve path creation error for {}",
        disk_file.display()
    ));
    Ok(false)
}

fn blocking_component(dir: &Path) -> Option<PathBuf> {
    let mut current = dir.to_path_buf();
    loop {
        let parent = current.parent()?;
        if parent.as_os_str().is_empty() {
            return None;
        }
        if fs::create_dir_all(parent).is_ok() {
            return Some(current);
        }
        current = parent.to_path_buf();
    }
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
