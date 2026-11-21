// Copyright (c) 2006-2016, openmetaverse.co
// All rights reserved.
//
// - Redistribution and use in source and binary forms, with or without
//   modification, are permitted provided that the following conditions are met:
//
// - Redistributions of source code must retain the above copyright notice, this
//   list of conditions and the following disclaimer.
// - Neither the name of the openmetaverse.co nor the names
//   of its contributors may be used to endorse or promote products derived from
//   this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
//
// Ported to Rust for Ferroan from
//   OpenMetaverse/BitPack.cs (PackBits, PackFloat, PackBitsFromByte, BytePos)
//   libopenmetaverse @ the Tranquillity pin
//   c70fee205f64ee8672a8d607cef6a733931da951
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Safe `BitPack`. The zero-byte fast path keeps `bit_pos`, matching
//! `PackBitsFromByte` when `inbyte == 0`.

#[derive(Clone, Debug)]
pub(crate) struct BitPack {
    data: Vec<u8>,
    byte_pos: usize,
    bit_pos: u8,
}

impl BitPack {
    pub(crate) fn new() -> Self {
        Self {
            data: vec![0],
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        let end = if self.byte_pos != 0 && self.bit_pos == 0 {
            self.byte_pos - 1
        } else {
            self.byte_pos
        };
        self.data.get(..=end).unwrap_or(&self.data)
    }

    pub(crate) fn pack_bits(&mut self, mut data: u32, mut total: i32) {
        while total > 8 {
            self.pack_byte(data as u8);
            data >>= 8;
            total -= 8;
        }
        if total > 0 {
            self.pack_counted(data as u8, total);
        }
    }

    pub(crate) fn pack_float(&mut self, value: f32) {
        let bits = value.to_bits();
        self.pack_byte(bits as u8);
        self.pack_byte((bits >> 8) as u8);
        self.pack_byte((bits >> 16) as u8);
        self.pack_byte((bits >> 24) as u8);
    }

    /// `PackBitsFromByte(byte)` — a whole byte, low bits already aligned.
    pub(crate) fn pack_byte(&mut self, inbyte: u8) {
        if self.bit_pos == 0 {
            self.store(inbyte);
            self.byte_pos += 1;
            self.store(0);
            return;
        }
        if inbyte == 0 {
            self.byte_pos += 1;
            self.store(0);
            return;
        }
        let mixed = self.read() | (inbyte >> self.bit_pos);
        self.store(mixed);
        self.byte_pos += 1;
        self.store(inbyte << (8 - self.bit_pos));
    }

    /// `PackBitsFromByte(byte, count)`. `count` above 8 is clamped to 7, as C# does.
    pub(crate) fn pack_counted(&mut self, inbyte: u8, mut count: i32) {
        if count < 1 {
            return;
        }
        if count > 8 {
            count = 7;
        } else {
            count -= 1;
        }
        let mut cur = self.read();
        while count > 0 {
            if inbyte & (1 << count) != 0 {
                cur |= 0x80 >> self.bit_pos;
            }
            count -= 1;
            self.bit_pos += 1;
            if self.bit_pos >= 8 {
                self.store(cur);
                self.byte_pos += 1;
                cur = 0;
                self.bit_pos = 0;
            }
        }
        if inbyte & 0x01 != 0 {
            cur |= 0x80 >> self.bit_pos;
        }
        self.store(cur);
        self.bit_pos += 1;
        if self.bit_pos >= 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
            self.store(0);
        }
    }

    fn read(&self) -> u8 {
        self.data.get(self.byte_pos).copied().unwrap_or(0)
    }

    fn store(&mut self, value: u8) {
        if self.data.len() <= self.byte_pos {
            self.data.resize(self.byte_pos + 1, 0);
        }
        if let Some(slot) = self.data.get_mut(self.byte_pos) {
            *slot = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stride_packs_as_two_little_endian_bytes() {
        let mut bits = BitPack::new();
        bits.pack_bits(264, 16);
        assert_eq!(bits.bytes(), &[0x08, 0x01]);
    }

    #[test]
    fn group_header_is_stride_patch_and_land() {
        let mut bits = BitPack::new();
        bits.pack_bits(264, 16);
        bits.pack_byte(16);
        bits.pack_byte(0x4c);
        assert_eq!(bits.bytes(), &[0x08, 0x01, 0x10, 0x4c]);
    }
}
