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

//! Tar members the way `TarArchiveReader` returns them. The checksum is not read.

pub(super) const TYPE_UNKNOWN: &str = "TYPE_UNKNOWN";
pub(super) const TYPE_NORMAL_FILE: &str = "TYPE_NORMAL_FILE";
pub(super) const TYPE_HARD_LINK: &str = "TYPE_HARD_LINK";
pub(super) const TYPE_SYMBOLIC_LINK: &str = "TYPE_SYMBOLIC_LINK";
pub(super) const TYPE_CHAR_SPECIAL: &str = "TYPE_CHAR_SPECIAL";
pub(super) const TYPE_BLOCK_SPECIAL: &str = "TYPE_BLOCK_SPECIAL";
pub(super) const TYPE_DIRECTORY: &str = "TYPE_DIRECTORY";
pub(super) const TYPE_FIFO: &str = "TYPE_FIFO";
pub(super) const TYPE_CONTIGUOUS_FILE: &str = "TYPE_CONTIGUOUS_FILE";

pub(super) struct TarEntry {
    pub path: String,
    pub kind: &'static str,
    pub body: Vec<u8>,
}

pub(super) fn entries(bytes: &[u8]) -> Vec<TarEntry> {
    let mut reader = Reader { bytes, pos: 0 };
    let mut out = Vec::new();
    while let Some(entry) = reader.read_entry() {
        out.push(entry);
    }
    out
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn read_entry(&mut self) -> Option<TarEntry> {
        let mut header = self.take_block()?;
        let path = if header[156] == b'L' {
            let name_len = octal(&header, 124, 11);
            let name = self.read_data(name_len);
            let path = ascii(&name);
            header = self.take_block()?;
            path
        } else {
            ascii(&header[..100]).trim_matches('\0').to_string()
        };
        let size = octal(&header, 124, 11);
        let kind = kind_of(header[156]);
        let body = self.read_data(size);
        Some(TarEntry { path, kind, body })
    }

    fn take_block(&mut self) -> Option<Vec<u8>> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let end = (self.pos + 512).min(self.bytes.len());
        let block = self.bytes[self.pos..end].to_vec();
        self.pos = end;
        if block.is_empty() || block[0] == 0 || block.len() != 512 {
            return None;
        }
        Some(block)
    }

    fn read_data(&mut self, file_size: i32) -> Vec<u8> {
        if file_size <= 0 {
            return Vec::new();
        }
        let size = file_size as usize;
        let end = (self.pos + size).min(self.bytes.len());
        let data = self.bytes[self.pos..end].to_vec();
        self.pos = end;
        if !size.is_multiple_of(512) {
            let padding = 512 - (size % 512);
            let pad_end = (self.pos + padding).min(self.bytes.len());
            self.pos = pad_end;
        }
        data
    }
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| if *byte >= 128 { '?' } else { *byte as char })
        .collect()
}

/// `ConvertOctalBytesToDecimal`: trim leading spaces, then `d <<= 3; d |= c - '0'`.
fn octal(bytes: &[u8], start: usize, count: usize) -> i32 {
    let end = (start + count).min(bytes.len());
    let text = ascii(&bytes[start..end]);
    let mut value = 0i32;
    for ch in text.trim_start_matches(' ').chars() {
        value = value.wrapping_shl(3);
        value |= (ch as i32).wrapping_sub('0' as i32);
    }
    value
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
