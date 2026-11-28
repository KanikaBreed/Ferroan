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
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptObjCode.cs
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptCodeGen.cs
//   Source/OpenSim.Region.ScriptEngine.YEngine/XMRInstAbstract.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The header of a YEngine object file, through the global declaration triples.

use ferroan_diag::{Kind, YENGINE_HEADER_STRING, YENGINE_OBJECT_BYTES, refuse};

use crate::error::Failure;

pub const MAGIC: &str = "YObjectCode";
pub const VERSION: i32 = -11;
pub const OBJECT_BYTES: u64 = 8 * 1024 * 1024;
pub const HEADER_STRING: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Global {
    pub name: String,
    pub type_name: String,
    pub index: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: i32,
    pub source_hash: String,
    pub sizes: [i32; 11],
    pub states: Vec<String>,
    pub globals: Vec<Global>,
}

pub fn read_header(bytes: &[u8]) -> Result<Header, Failure> {
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > OBJECT_BYTES {
        refuse(
            YENGINE_OBJECT_BYTES,
            Kind::Hard,
            OBJECT_BYTES,
            len,
            "yengine.object",
        );
        return Err(Failure::Limit {
            id: YENGINE_OBJECT_BYTES,
        });
    }
    let mut cur = Cursor { bytes, at: 0 };
    let magic = cur.chars(MAGIC.encode_utf16().count())?;
    if magic != MAGIC {
        return Err(Failure::Object("bad magic".to_string()));
    }
    let version = cur.i32()?;
    // The pinned reader rejects a foreign version before ReadString of the hash.
    if version != VERSION {
        return Err(Failure::Version {
            found: version,
            accepted: VERSION,
        });
    }
    let source_hash = cur.string()?;
    let mut sizes = [0; 11];
    for slot in &mut sizes {
        *slot = cur.i32()?;
    }
    let n_states = cur.i32()?;
    if n_states < 0 {
        return Err(Failure::Object("state count".to_string()));
    }
    let n_states = n_states as usize;
    if n_states > cur.remaining() {
        return Err(Failure::Object("state count".to_string()));
    }
    let mut states = Vec::new();
    for _ in 0..n_states {
        states.push(cur.string()?);
    }
    let mut globals = Vec::new();
    loop {
        let name = cur.string()?;
        if name.is_empty() {
            break;
        }
        let type_name = cur.string()?;
        let index = cur.i32()?;
        globals.push(Global {
            name,
            type_name,
            index,
        });
    }
    Ok(Header {
        version,
        source_hash,
        sizes,
        states,
        globals,
    })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Failure> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| Failure::Object("truncated".to_string()))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    /// `BinaryReader.ReadChars` with the UTF-8 writer `new BinaryWriter(stream)` uses.
    fn chars(&mut self, n: usize) -> Result<String, Failure> {
        let mut out = String::new();
        for _ in 0..n {
            if self.remaining() == 0 {
                return Err(Failure::Object("truncated".to_string()));
            }
            let width = utf8_char_width(self.bytes[self.at]);
            if width == 0 || self.remaining() < width {
                return Err(Failure::Object("truncated".to_string()));
            }
            let raw = self.take(width)?;
            let text =
                std::str::from_utf8(raw).map_err(|_| Failure::Object("string".to_string()))?;
            let mut chars = text.chars();
            let Some(ch) = chars.next() else {
                return Err(Failure::Object("string".to_string()));
            };
            if chars.next().is_some() {
                return Err(Failure::Object("string".to_string()));
            }
            out.push(ch);
        }
        Ok(out)
    }

    fn i32(&mut self) -> Result<i32, Failure> {
        let raw = self.take(4)?;
        Ok(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    fn string(&mut self) -> Result<String, Failure> {
        let len = self.seven_bit()?;
        if len > HEADER_STRING {
            refuse(
                YENGINE_HEADER_STRING,
                Kind::Hard,
                HEADER_STRING,
                len,
                "yengine.object",
            );
            return Err(Failure::Limit {
                id: YENGINE_HEADER_STRING,
            });
        }
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        let raw = self.take(len)?;
        String::from_utf8(raw.to_vec()).map_err(|_| Failure::Object("string".to_string()))
    }

    /// `BinaryReader.Read7BitEncodedInt` on .NET 10.
    fn seven_bit(&mut self) -> Result<u64, Failure> {
        let mut result: u32 = 0;
        for shift in (0..28).step_by(7) {
            let byte = self.take(1)?[0];
            result |= u32::from(byte & 0x7f) << shift;
            if byte <= 0x7f {
                return Ok(u64::from(result));
            }
        }
        let byte = self.take(1)?[0];
        if byte > 0b_1111 {
            return Err(Failure::Object("string length".to_string()));
        }
        result |= u32::from(byte) << 28;
        Ok(u64::from(result))
    }
}

fn utf8_char_width(byte: u8) -> usize {
    match byte {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 0,
    }
}
