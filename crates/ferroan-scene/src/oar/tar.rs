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
//   Source/OpenSim.Framework.Serialization/TarArchiveReader.cs
//   (ReadEntry, ReadHeader, ReadData, ConvertOctalBytesToDecimal)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Streaming tar members the way `TarArchiveReader` returns them.
//!
//! `next_member` yields the path, type and declared size before the body is
//! read, so a caller can skip or cap a member. The checksum is not read. A
//! GNU long-name header's following header is not tested for a zero block or
//! an empty read. A negative size throws and aborts. A member above
//! `oar.member-bytes` is refused before its body, whether the caller keeps or
//! skips it. A kept member can also hit a path-class cap. There is no
//! archive-wide decompressed total. Each bound is one row in `docs/LIMITS.md`.

use std::io::Read;

use ferroan_diag::{
    Kind, OAR_CONTROL_BYTES, OAR_LANDDATA_BYTES, OAR_MEMBER_BYTES, OAR_MEMBERS, OAR_OBJECT_BYTES,
    OAR_PATH_BYTES, OAR_SETTINGS_BYTES, OAR_TERRAIN_BYTES,
};

pub(super) const TYPE_UNKNOWN: &str = "TYPE_UNKNOWN";
pub(super) const TYPE_NORMAL_FILE: &str = "TYPE_NORMAL_FILE";
pub(super) const TYPE_HARD_LINK: &str = "TYPE_HARD_LINK";
pub(super) const TYPE_SYMBOLIC_LINK: &str = "TYPE_SYMBOLIC_LINK";
pub(super) const TYPE_CHAR_SPECIAL: &str = "TYPE_CHAR_SPECIAL";
pub(super) const TYPE_BLOCK_SPECIAL: &str = "TYPE_BLOCK_SPECIAL";
pub(super) const TYPE_DIRECTORY: &str = "TYPE_DIRECTORY";
pub(super) const TYPE_FIFO: &str = "TYPE_FIFO";
pub(super) const TYPE_CONTIGUOUS_FILE: &str = "TYPE_CONTIGUOUS_FILE";

/// The limit counters are process-wide. Tests that read them take this lock.
#[cfg(test)]
pub(super) static LIMIT_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

const INDEX_ERROR: &str = "Index was outside the bounds of the array.";
const BUFFER_ERROR: &str =
    "Index and count must refer to a location within the buffer. (Parameter 'bytes')";
const CHUNK: usize = 8192;

/// 262144 members. A late control file opens a new session, so each pass has
/// this budget. Pin8 does not cap the member count.
const MAX_MEMBERS: u64 = 262_144;
/// 1536 MiB. Above the garbage size a 200-byte header declares. Pin8 allocates
/// the declared size. Ferroan refuses before allocating it.
pub(super) const MAX_MEMBER_BYTES: u64 = 1_610_612_736;
/// Kept `archive.xml`. Checked after `oar.member-bytes`.
pub(super) const MAX_CONTROL_BYTES: u64 = 8 * 1024 * 1024;
/// Kept `objects/` member.
pub(super) const MAX_OBJECT_BYTES: u64 = 64 * 1024 * 1024;
/// Kept `terrains/` member. Above Q216b's later 75,759,615-byte `.r32`/`.f32` rule.
pub(super) const MAX_TERRAIN_BYTES: u64 = 128 * 1024 * 1024;
/// Kept `settings/` member.
pub(super) const MAX_SETTINGS_BYTES: u64 = 4 * 1024 * 1024;
/// Kept `landdata/` member.
pub(super) const MAX_LANDDATA_BYTES: u64 = 32 * 1024 * 1024;
/// 1024 bytes after the ASCII replacement Pin8 applies to a path.
const MAX_PATH_BYTES: u64 = 1024;

#[derive(Debug)]
pub(super) struct Member {
    pub path: String,
    pub kind: &'static str,
    pub size: i32,
}

enum Pending {
    Clear,
    Waiting { size: i32 },
}

pub(super) struct Session<R> {
    reader: R,
    members: u64,
    pending: Pending,
}

impl<R: Read> Session<R> {
    pub(super) fn new(reader: R) -> Self {
        Self {
            reader,
            members: 0,
            pending: Pending::Clear,
        }
    }

    pub(super) fn next_member(&mut self) -> Result<Option<Member>, String> {
        if matches!(self.pending, Pending::Waiting { .. }) {
            return Err("a tar member body is still unread".to_string());
        }
        let Some(member) = self.read_header()? else {
            return Ok(None);
        };
        if self.members >= MAX_MEMBERS {
            return Err(refuse(
                OAR_MEMBERS,
                MAX_MEMBERS,
                self.members.saturating_add(1),
            ));
        }
        self.members += 1;
        self.pending = Pending::Waiting { size: member.size };
        Ok(Some(member))
    }

    pub(super) fn read_body(&mut self) -> Result<Vec<u8>, String> {
        let size = self.take_pending()?;
        self.consume(size, true)
    }

    pub(super) fn skip_body(&mut self) -> Result<(), String> {
        let size = self.take_pending()?;
        self.consume(size, false)?;
        Ok(())
    }

    fn take_pending(&mut self) -> Result<i32, String> {
        match std::mem::replace(&mut self.pending, Pending::Clear) {
            Pending::Waiting { size } => Ok(size),
            Pending::Clear => Err("a tar member body was already consumed".to_string()),
        }
    }

    fn read_header(&mut self) -> Result<Option<Member>, String> {
        let header = self.read_block()?;
        if header.is_empty() || header[0] == 0 {
            return Ok(None);
        }
        if header.len() <= 156 {
            return Err(INDEX_ERROR.to_string());
        }
        let (path, header) = if header[156] == b'L' {
            let name_len = octal(&header, 124, 11)?;
            let path = self.read_long_name(name_len)?;
            (path, self.read_block()?)
        } else {
            let path = ascii(&header[..100]).trim_matches('\0').to_string();
            check_path(&path)?;
            (path, header)
        };
        let size = octal(&header, 124, 11)?;
        if header.len() <= 156 {
            return Err(INDEX_ERROR.to_string());
        }
        Ok(Some(Member {
            path,
            kind: kind_of(header[156]),
            size,
        }))
    }

    /// The long name is one `ReadData`. Its length is refused before that read
    /// when it cannot fit the path cap or the member cap.
    fn read_long_name(&mut self, name_len: i32) -> Result<String, String> {
        if name_len < 0 {
            return Err(negative_count(name_len));
        }
        let declared = u64::from(name_len as u32);
        if declared > MAX_MEMBER_BYTES {
            return Err(refuse(OAR_MEMBER_BYTES, MAX_MEMBER_BYTES, declared));
        }
        if declared > MAX_PATH_BYTES {
            return Err(refuse(OAR_PATH_BYTES, MAX_PATH_BYTES, declared));
        }
        let raw = self.consume(name_len, true)?;
        let path = ascii(&raw);
        check_path(&path)?;
        Ok(path)
    }

    fn consume(&mut self, size: i32, keep: bool) -> Result<Vec<u8>, String> {
        if size < 0 {
            return Err(negative_count(size));
        }
        let declared = u64::from(size as u32);
        if declared > MAX_MEMBER_BYTES {
            return Err(refuse(OAR_MEMBER_BYTES, MAX_MEMBER_BYTES, declared));
        }
        let data = self.take_bytes(size as usize, keep)?;
        if size % 512 != 0 {
            let padding = 512 - (size as usize % 512);
            self.take_bytes(padding, false)?;
        }
        Ok(data)
    }

    fn read_block(&mut self) -> Result<Vec<u8>, String> {
        self.take_bytes(512, true)
    }

    fn take_bytes(&mut self, want: usize, keep: bool) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        if keep {
            // One reservation. Doubling would hold the member twice for a moment.
            // A garbage declared size cannot reserve past the member cap.
            let bound = usize::try_from(MAX_MEMBER_BYTES).unwrap_or(usize::MAX);
            out.reserve(want.min(bound));
        }
        let mut left = want;
        let mut buf = [0u8; CHUNK];
        while left > 0 {
            let chunk = left.min(buf.len());
            let n = self.read_chunk(&mut buf[..chunk])?;
            if n == 0 {
                break;
            }
            if keep {
                out.extend_from_slice(&buf[..n]);
            }
            left -= n;
        }
        Ok(out)
    }

    fn read_chunk(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.reader.read(buf).map_err(|error| error.to_string())
    }
}

/// Refuse a declaration above `oar.member-bytes` before the body is read.
/// A negative size is left for the read, which reports Pin8's count text.
pub(super) fn member_limit(size: i32) -> Result<(), String> {
    if size < 0 {
        return Ok(());
    }
    let declared = u64::from(size as u32);
    if declared > MAX_MEMBER_BYTES {
        return Err(refuse(OAR_MEMBER_BYTES, MAX_MEMBER_BYTES, declared));
    }
    Ok(())
}

/// Path-class cap for a body the caller is keeping. A skipped member does not
/// call this. A negative size is left for the read. `assets/`, `inventory/`,
/// `userprofiles/` and any other path have no class cap.
pub(super) fn kept_limit(relative: &str, size: i32) -> Result<(), String> {
    if size < 0 {
        return Ok(());
    }
    let declared = u64::from(size as u32);
    let Some((id, cap)) = class_cap(relative) else {
        return Ok(());
    };
    if declared > cap {
        return Err(refuse(id, cap, declared));
    }
    Ok(())
}

fn class_cap(relative: &str) -> Option<(&'static str, u64)> {
    if relative == "archive.xml" {
        Some((OAR_CONTROL_BYTES, MAX_CONTROL_BYTES))
    } else if relative.starts_with("objects/") {
        Some((OAR_OBJECT_BYTES, MAX_OBJECT_BYTES))
    } else if relative.starts_with("terrains/") {
        Some((OAR_TERRAIN_BYTES, MAX_TERRAIN_BYTES))
    } else if relative.starts_with("settings/") {
        Some((OAR_SETTINGS_BYTES, MAX_SETTINGS_BYTES))
    } else if relative.starts_with("landdata/") {
        Some((OAR_LANDDATA_BYTES, MAX_LANDDATA_BYTES))
    } else {
        None
    }
}

fn check_path(path: &str) -> Result<(), String> {
    let len = path.len() as u64;
    if len > MAX_PATH_BYTES {
        return Err(refuse(OAR_PATH_BYTES, MAX_PATH_BYTES, len));
    }
    Ok(())
}

fn refuse(id: &'static str, configured: u64, observed: u64) -> String {
    ferroan_diag::refuse(id, Kind::Hard, configured, observed, "oar");
    format!("oar limit {id}")
}

fn negative_count(count: i32) -> String {
    format!(
        "count ('{count}') must be a non-negative value. (Parameter 'count')\nActual value was {count}."
    )
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| if *byte >= 128 { '?' } else { *byte as char })
        .collect()
}

/// `ConvertOctalBytesToDecimal`: trim leading spaces, then `d <<= 3; d |= c - '0'`.
fn octal(bytes: &[u8], start: usize, count: usize) -> Result<i32, String> {
    let Some(end) = start.checked_add(count) else {
        return Err(BUFFER_ERROR.to_string());
    };
    if end > bytes.len() {
        return Err(BUFFER_ERROR.to_string());
    }
    let text = ascii(&bytes[start..end]);
    let mut value = 0i32;
    for ch in text.trim_start_matches(' ').chars() {
        value = value.wrapping_shl(3);
        value |= (ch as i32).wrapping_sub('0' as i32);
    }
    Ok(value)
}

fn kind_of(byte: u8) -> &'static str {
    match byte {
        0 | b'0' => TYPE_NORMAL_FILE,
        b'1' => TYPE_HARD_LINK,
        b'2' => TYPE_SYMBOLIC_LINK,
        b'3' => TYPE_CHAR_SPECIAL,
        b'4' => TYPE_BLOCK_SPECIAL,
        b'5' => TYPE_DIRECTORY,
        b'6' => TYPE_FIFO,
        b'7' => TYPE_CONTIGUOUS_FILE,
        _ => TYPE_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn hits(id: &str) -> u64 {
        let prefix = format!("ferroan_limit_hits_total{{limit=\"{id}\",action=\"refuse\"}} ");
        ferroan_diag::metrics()
            .lines()
            .find_map(|line| line.strip_prefix(&prefix)?.parse().ok())
            .unwrap_or(0)
    }

    fn header(name: &str, size_field: &[u8], kind: u8) -> Vec<u8> {
        let mut block = vec![0u8; 512];
        let bytes = name.as_bytes();
        block[..bytes.len()].copy_from_slice(bytes);
        block[124..124 + size_field.len()].copy_from_slice(size_field);
        block[156] = kind;
        block
    }

    fn octal_field(size: i32) -> String {
        format!("{size:011o}")
    }

    #[test]
    fn a_negative_size_throws_and_aborts() {
        let block = header("notes/after.txt", b"0000000000/", b'0');
        let mut session = Session::new(Cursor::new(block));
        let member = session.next_member().unwrap().unwrap();
        assert_eq!(member.size, -1);
        let error = session.read_body().unwrap_err();
        assert_eq!(
            error,
            "count ('-1') must be a non-negative value. (Parameter 'count')\nActual value was -1."
        );
    }

    #[test]
    fn a_short_final_block_longer_than_156_is_an_entry() {
        let mut session = Session::new(Cursor::new(vec![b'x'; 200]));
        let member = session.next_member().unwrap().unwrap();
        assert_eq!(member.kind, TYPE_UNKNOWN);
        assert_eq!(member.path.len(), 100);
        assert!(member.path.chars().all(|ch| ch == 'x'));
        assert_eq!(member.size, 1_227_133_512);
        assert!(session.read_body().unwrap().is_empty());
    }

    #[test]
    fn a_block_of_156_bytes_throws_the_index_error() {
        let mut session = Session::new(Cursor::new(vec![b'a'; 156]));
        assert_eq!(
            session.next_member().unwrap_err(),
            "Index was outside the bounds of the array."
        );
    }

    #[test]
    fn a_zero_block_ends_the_archive() {
        let mut session = Session::new(Cursor::new(vec![0u8; 512]));
        assert!(session.next_member().unwrap().is_none());
    }

    fn long_name(name: &[u8]) -> Vec<u8> {
        let field = format!("{:011o}", name.len());
        let mut bytes = header("././@LongLink", field.as_bytes(), b'L');
        bytes.extend(name);
        if !name.len().is_multiple_of(512) {
            bytes.extend(vec![0u8; 512 - (name.len() % 512)]);
        }
        bytes
    }

    #[test]
    fn an_empty_header_after_a_gnu_long_name_throws() {
        let mut session = Session::new(Cursor::new(long_name(b"name")));
        assert_eq!(
            session.next_member().unwrap_err(),
            "Index and count must refer to a location within the buffer. (Parameter 'bytes')"
        );
    }

    #[test]
    fn a_zero_header_after_a_gnu_long_name_is_an_entry() {
        let mut bytes = long_name(b"name");
        bytes.extend(vec![0u8; 512]);
        let mut session = Session::new(Cursor::new(bytes));
        let member = session.next_member().unwrap().unwrap();
        assert_eq!(member.path, "name");
        assert_eq!(member.kind, TYPE_NORMAL_FILE);
        // Eleven NUL size bytes stay at -48: each step is `(d << 3) | (0 - '0')`.
        assert_eq!(member.size, -48);
        let error = session.read_body().unwrap_err();
        assert!(error.contains("Actual value was -48."), "{error}");
    }

    #[test]
    fn a_second_header_stops_before_the_type_byte() {
        let mut bytes = long_name(b"name");
        bytes.extend(vec![b'0'; 150]);
        let mut session = Session::new(Cursor::new(bytes));
        assert_eq!(
            session.next_member().unwrap_err(),
            "Index was outside the bounds of the array."
        );
    }

    #[test]
    fn an_unread_body_is_an_error() {
        let block = header("a.txt", octal_field(0).as_bytes(), b'0');
        let mut session = Session::new(Cursor::new(block));
        session.next_member().unwrap().unwrap();
        let error = session.next_member().unwrap_err();
        assert_eq!(error, "a tar member body is still unread");
    }

    #[test]
    fn the_member_after_the_cap_is_refused() {
        let _guard = LIMIT_TEST.lock().unwrap();
        let block = header("a.txt", octal_field(0).as_bytes(), b'0');
        let mut session = Session::new(Repeat {
            block,
            left: usize::try_from(MAX_MEMBERS).unwrap() + 1,
            offset: 0,
        });
        for _ in 0..MAX_MEMBERS {
            session.next_member().unwrap().unwrap();
            session.skip_body().unwrap();
        }
        let before = hits(OAR_MEMBERS);
        let error = session.next_member().unwrap_err();
        assert!(error.contains(OAR_MEMBERS), "{error}");
        assert_eq!(hits(OAR_MEMBERS), before + 1);
    }

    #[test]
    fn a_long_name_past_the_path_cap_is_refused_before_the_body() {
        let _guard = LIMIT_TEST.lock().unwrap();
        let field = octal_field(i32::try_from(MAX_PATH_BYTES).unwrap() + 1);
        let block = header("././@LongLink", field.as_bytes(), b'L');
        let mut session = Session::new(Cursor::new(block));
        let before = hits(OAR_PATH_BYTES);
        let error = session.next_member().unwrap_err();
        assert!(error.contains(OAR_PATH_BYTES), "{error}");
        assert_eq!(hits(OAR_PATH_BYTES), before + 1);
    }

    #[test]
    fn a_declared_size_past_the_member_cap_is_refused_before_allocating() {
        let _guard = LIMIT_TEST.lock().unwrap();
        let field = octal_field(i32::try_from(MAX_MEMBER_BYTES).unwrap() + 1);
        let block = header("big.bin", field.as_bytes(), b'0');
        let mut session = Session::new(Cursor::new(block));
        session.next_member().unwrap().unwrap();
        let before = hits(OAR_MEMBER_BYTES);
        let error = session.read_body().unwrap_err();
        assert!(error.contains(OAR_MEMBER_BYTES), "{error}");
        assert_eq!(hits(OAR_MEMBER_BYTES), before + 1);
    }

    #[test]
    fn a_kept_path_hits_its_class_cap_and_a_negative_size_does_not() {
        let _guard = LIMIT_TEST.lock().unwrap();
        assert!(kept_limit("objects/a.xml", -1).is_ok());
        assert!(
            kept_limit(
                "archive.xml.bak",
                i32::try_from(MAX_CONTROL_BYTES).unwrap() + 1
            )
            .is_ok()
        );
        assert!(kept_limit("assets/a.jp2", i32::try_from(MAX_OBJECT_BYTES).unwrap() + 1).is_ok());
        // Q216b's later .r32 rule starts at 75,759,616. This cap stays above it.
        assert!(kept_limit("terrains/r.r32", 75_759_616).is_ok());
        let cases = [
            ("archive.xml", MAX_CONTROL_BYTES, OAR_CONTROL_BYTES),
            ("objects/a.xml", MAX_OBJECT_BYTES, OAR_OBJECT_BYTES),
            ("terrains/r.r32", MAX_TERRAIN_BYTES, OAR_TERRAIN_BYTES),
            ("settings/a.ini", MAX_SETTINGS_BYTES, OAR_SETTINGS_BYTES),
            ("landdata/a.bin", MAX_LANDDATA_BYTES, OAR_LANDDATA_BYTES),
        ];
        for (path, cap, id) in cases {
            let before = hits(id);
            let error = kept_limit(path, i32::try_from(cap).unwrap() + 1).unwrap_err();
            assert!(error.contains(id), "{path}: {error}");
            assert_eq!(hits(id), before + 1, "{path}");
        }
    }

    struct Repeat {
        block: Vec<u8>,
        left: usize,
        offset: usize,
    }

    impl Read for Repeat {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.left == 0 || buf.is_empty() {
                return Ok(0);
            }
            let n = (self.block.len() - self.offset).min(buf.len());
            buf[..n].copy_from_slice(&self.block[self.offset..self.offset + n]);
            self.offset += n;
            if self.offset == self.block.len() {
                self.offset = 0;
                self.left -= 1;
            }
            Ok(n)
        }
    }
}
