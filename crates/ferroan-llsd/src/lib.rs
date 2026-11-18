//
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

// OSD string conversion follows OpenMetaverse.StructuredData/StructuredData.cs
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-llsd/src/lib.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// Originally LGPL 2.1, from the Second Life viewer's llcommon/llsd.cpp.
//
//! LLSD (Linden Lab Structured Data) value type with XML and binary codecs, and a notation parser.
//!
//! Port of the semantics of `llcommon/llsd.cpp` and `llsdserialize*.cpp`
//! from the Second Life viewer (originally LGPL 2.1).

pub mod binary;
pub mod notation;
mod number;
pub mod xml;
mod xml_llsd;

use indexmap::IndexMap;
use uuid::Uuid;

pub use binary::{from_binary, from_binary_local, to_binary, to_binary_framed};
pub use notation::{from_notation, to_notation};
pub use xml_llsd::{
    from_xml, from_xml_element, to_pretty_xml_string, to_serialized_xml, to_xml, to_xml_formal,
    to_xml_string,
};

#[derive(Debug, thiserror::Error)]
pub enum LlsdError {
    #[error("xml: {0}")]
    Xml(#[from] xml::XmlError),
    #[error("invalid llsd: {0}")]
    Invalid(&'static str),
    #[error("unexpected end of data")]
    Eof,
    #[error("parsed llsd larger than its budget")]
    TooLarge,
}

/// Insertion order, matching OpenMetaverse. Aurora's `BTreeMap` sorted keys
/// and would re-encode a map differently from the C# bytes.
pub type Map = IndexMap<String, Llsd>;

/// Upper bound on the estimated heap a binary or notation parse may build,
/// whatever the input size. Real mesh sections are mostly binary blobs and
/// stay near their own size, far below this. Files the viewer wrote itself
/// ([`binary::from_binary_local`]) keep only the per-byte rule: a large
/// inventory cache is well over it.
pub const MAX_PARSED_BYTES: usize = 128 * 1024 * 1024;
/// Estimated heap allowed per input byte. Real LLSD (blobs, strings, reals,
/// maps with named keys) stays under 12. An `IndexMap` is larger than the
/// sorted map Aurora used, and an inventory-shaped document lands near 9.
const PARSED_BYTES_PER_INPUT_BYTE: usize = 16;
/// Small documents are never refused by the per-byte rule.
const PARSED_BYTES_FLOOR: usize = 64 * 1024;
/// Inline size of one value, in its parent's array slot or map entry.
pub(crate) const VALUE_COST: usize = size_of::<Llsd>();
/// One map entry: its key and value, twice over for the map's storage.
pub(crate) const MAP_ENTRY_COST: usize = 2 * (size_of::<String>() + size_of::<Llsd>());

/// Estimated heap left to a parse, charged before each allocation it makes,
/// so the tree stays proportional to the input (with a hard cap, except for
/// the viewer's own files).
pub(crate) struct Budget {
    pub(crate) left: usize,
}

impl Budget {
    pub(crate) fn for_input(len: usize) -> Self {
        let left = Self::for_local_input(len).left.min(MAX_PARSED_BYTES);
        Budget { left }
    }

    /// The per-byte rule alone, for files the viewer wrote itself.
    pub(crate) fn for_local_input(len: usize) -> Self {
        let left = len
            .saturating_mul(PARSED_BYTES_PER_INPUT_BYTE)
            .saturating_add(PARSED_BYTES_FLOOR);
        Budget { left }
    }

    pub(crate) fn charge(&mut self, bytes: usize) -> Result<(), LlsdError> {
        self.left = self.left.checked_sub(bytes).ok_or(LlsdError::TooLarge)?;
        Ok(())
    }

    /// `n` values reserved at once (an array's slots).
    pub(crate) fn charge_values(&mut self, n: usize) -> Result<(), LlsdError> {
        self.charge(n.checked_mul(VALUE_COST).ok_or(LlsdError::TooLarge)?)
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn osd_capability_names_use_non_string_conversions() {
        assert_eq!(Llsd::Boolean(false).to_osd_string(), "0");
        assert_eq!(Llsd::Boolean(true).to_osd_string(), "1");
        assert_eq!(Llsd::Integer(-17).to_osd_string(), "-17");
        assert_eq!(Llsd::Real(f64::INFINITY).to_osd_string(), "∞");
        assert_eq!(Llsd::Real(1e20).to_osd_string(), "1e+20");
        assert_eq!(
            Llsd::Uri("EventQueueGet".into()).to_osd_string(),
            "EventQueueGet"
        );
        assert_eq!(
            Llsd::Binary(b"EventQueueGet".to_vec()).to_osd_string(),
            "RXZlbnRRdWV1ZUdldA=="
        );
        assert_eq!(Llsd::Date(0).to_osd_string(), "1970-01-01T00:00:00Z");
        assert_eq!(
            Llsd::Uuid(Uuid::nil()).to_osd_string(),
            Uuid::nil().to_string()
        );
        for value in [Llsd::Undef, Llsd::new_map(), Llsd::new_array()] {
            assert!(value.to_osd_string().is_empty());
        }
        assert_eq!(Llsd::Boolean(false).to_string_value(), "");
    }

    #[test]
    fn budget_grows_with_the_input_up_to_the_cap() {
        assert_eq!(Budget::for_input(0).left, PARSED_BYTES_FLOOR);
        assert_eq!(Budget::for_input(1000).left, 16_000 + PARSED_BYTES_FLOOR);
        // a whole decompressed mesh section (32 MB) gets the cap, not 512 MB
        assert_eq!(Budget::for_input(32 << 20).left, MAX_PARSED_BYTES);
        assert_eq!(Budget::for_input(usize::MAX).left, MAX_PARSED_BYTES);
        // the viewer's own files keep the per-byte rule only
        assert_eq!(
            Budget::for_local_input(32 << 20).left,
            (512 << 20) + PARSED_BYTES_FLOOR
        );
        let mut b = Budget::for_input(0);
        let slots = PARSED_BYTES_FLOOR / VALUE_COST;
        assert!(b.charge_values(slots).is_ok());
        let leftover = PARSED_BYTES_FLOOR - slots * VALUE_COST;
        if leftover > 0 {
            assert!(b.charge(leftover).is_ok());
        }
        assert!(matches!(b.charge(1), Err(LlsdError::TooLarge)));
        assert!(matches!(
            Budget::for_input(0).charge_values(usize::MAX),
            Err(LlsdError::TooLarge)
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Llsd {
    #[default]
    Undef,
    Boolean(bool),
    Integer(i32),
    Real(f64),
    String(String),
    Uuid(Uuid),
    /// 100-nanosecond ticks since the Unix epoch.
    ///
    /// OpenMetaverse keeps a `DateTime`. The binary form is `TotalSeconds`
    /// as an `f64`, and reading that double back does not restore the same
    /// millisecond, so the value here is the tick count rather than the double.
    Date(i64),
    Uri(String),
    Binary(Vec<u8>),
    Array(Vec<Llsd>),
    Map(Map),
}

static UNDEF: Llsd = Llsd::Undef;

impl Llsd {
    pub fn new_map() -> Self {
        Llsd::Map(Map::new())
    }

    pub fn new_array() -> Self {
        Llsd::Array(Vec::new())
    }

    pub fn is_undef(&self) -> bool {
        matches!(self, Llsd::Undef)
    }

    pub fn is_map(&self) -> bool {
        matches!(self, Llsd::Map(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Llsd::Array(_))
    }

    /// Map lookup; returns `Undef` when missing or not a map.
    pub fn get(&self, key: &str) -> &Llsd {
        match self {
            Llsd::Map(m) => m.get(key).unwrap_or(&UNDEF),
            _ => &UNDEF,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        matches!(self, Llsd::Map(m) if m.contains_key(key))
    }

    /// Array lookup; returns `Undef` when missing or not an array.
    pub fn at(&self, idx: usize) -> &Llsd {
        match self {
            Llsd::Array(a) => a.get(idx).unwrap_or(&UNDEF),
            _ => &UNDEF,
        }
    }

    pub fn as_array(&self) -> &[Llsd] {
        match self {
            Llsd::Array(a) => a,
            _ => &[],
        }
    }

    pub fn as_map(&self) -> Option<&Map> {
        match self {
            Llsd::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Llsd::Array(a) => a.len(),
            Llsd::Map(m) => m.len(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Insert into a map (converts `Undef` into a map first).
    pub fn insert(&mut self, key: impl Into<String>, v: impl Into<Llsd>) {
        if self.is_undef() {
            *self = Llsd::new_map();
        }
        if let Llsd::Map(m) = self {
            m.insert(key.into(), v.into());
        }
    }

    /// Append to an array (converts `Undef` into an array first).
    pub fn push(&mut self, v: impl Into<Llsd>) {
        if self.is_undef() {
            *self = Llsd::new_array();
        }
        if let Llsd::Array(a) = self {
            a.push(v.into());
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Llsd::Boolean(b) => *b,
            Llsd::Integer(i) => *i != 0,
            Llsd::Real(r) => *r != 0.0,
            Llsd::String(s) => !(s.is_empty() || s == "0" || s.eq_ignore_ascii_case("false")),
            _ => false,
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            Llsd::Boolean(b) => *b as i32,
            Llsd::Integer(i) => *i,
            Llsd::Real(r) => r.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32,
            Llsd::String(s) => s
                .trim()
                .parse::<i64>()
                .map(|v| v as i32)
                .or_else(|_| s.trim().parse::<f64>().map(|v| v as i32))
                .unwrap_or(0),
            Llsd::Binary(b) if b.len() >= 4 => i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            _ => 0,
        }
    }

    /// Unsigned interpretation (used for U32 values packed into integers).
    pub fn as_u32(&self) -> u32 {
        match self {
            Llsd::Real(r) => r.max(0.0).min(u32::MAX as f64) as u32,
            Llsd::String(s) => s.trim().parse::<u64>().map(|v| v as u32).unwrap_or(0),
            other => other.as_i32() as u32,
        }
    }

    /// U64 values are transported as 8-byte binaries by convention.
    pub fn as_u64(&self) -> u64 {
        match self {
            Llsd::Binary(b) if b.len() >= 8 => {
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
            Llsd::String(s) => s.trim().parse().unwrap_or(0),
            Llsd::Real(r) => *r as u64,
            other => other.as_i32() as u32 as u64,
        }
    }

    pub fn as_f64(&self) -> f64 {
        match self {
            Llsd::Boolean(b) => *b as i32 as f64,
            Llsd::Integer(i) => *i as f64,
            Llsd::Real(r) => *r,
            Llsd::Date(d) => number::ticks_to_seconds(*d),
            Llsd::String(s) => s.trim().parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub fn as_f32(&self) -> f32 {
        self.as_f64() as f32
    }

    pub fn as_str(&self) -> &str {
        match self {
            Llsd::String(s) | Llsd::Uri(s) => s,
            _ => "",
        }
    }

    /// String conversion following LLSD rules.
    pub fn to_string_value(&self) -> String {
        match self {
            Llsd::Undef => String::new(),
            Llsd::Boolean(b) => if *b { "true" } else { "" }.to_owned(),
            Llsd::Integer(i) => i.to_string(),
            Llsd::Real(r) => r.to_string(),
            Llsd::String(s) | Llsd::Uri(s) => s.clone(),
            Llsd::Uuid(u) => u.to_string(),
            Llsd::Date(d) => number::format_date(*d),
            Llsd::Binary(b) => String::from_utf8_lossy(b).into_owned(),
            Llsd::Array(_) | Llsd::Map(_) => String::new(),
        }
    }

    /// OpenMetaverse OSD.AsString conversion for capability-name lookup.
    /// URI text is retained as supplied; absolute URI canonicalization is not
    /// needed for a capability name, which is a relative URI.
    pub fn to_osd_string(&self) -> String {
        use base64::Engine;
        match self {
            Llsd::Boolean(value) => if *value { "1" } else { "0" }.to_owned(),
            Llsd::Real(value) => number::format_real(*value),
            Llsd::Binary(value) => base64::engine::general_purpose::STANDARD.encode(value),
            _ => self.to_string_value(),
        }
    }

    pub fn as_uuid(&self) -> Uuid {
        match self {
            Llsd::Uuid(u) => *u,
            Llsd::String(s) => Uuid::parse_str(s.trim()).unwrap_or(Uuid::nil()),
            Llsd::Binary(b) if b.len() == 16 => Uuid::from_slice(b).unwrap_or(Uuid::nil()),
            _ => Uuid::nil(),
        }
    }

    pub fn as_binary(&self) -> &[u8] {
        match self {
            Llsd::Binary(b) => b,
            Llsd::String(s) => s.as_bytes(),
            _ => &[],
        }
    }

    /// Interpret an LLSD array of 3 reals as a vector.
    pub fn as_vec3(&self) -> [f32; 3] {
        [
            self.at(0).as_f32(),
            self.at(1).as_f32(),
            self.at(2).as_f32(),
        ]
    }

    pub fn as_vec4(&self) -> [f32; 4] {
        [
            self.at(0).as_f32(),
            self.at(1).as_f32(),
            self.at(2).as_f32(),
            self.at(3).as_f32(),
        ]
    }
}

impl std::ops::Index<&str> for Llsd {
    type Output = Llsd;
    fn index(&self, key: &str) -> &Llsd {
        self.get(key)
    }
}

impl std::ops::Index<usize> for Llsd {
    type Output = Llsd;
    fn index(&self, idx: usize) -> &Llsd {
        self.at(idx)
    }
}

impl From<bool> for Llsd {
    fn from(v: bool) -> Self {
        Llsd::Boolean(v)
    }
}
impl From<i32> for Llsd {
    fn from(v: i32) -> Self {
        Llsd::Integer(v)
    }
}
impl From<u32> for Llsd {
    fn from(v: u32) -> Self {
        Llsd::Integer(v as i32)
    }
}
impl From<f64> for Llsd {
    fn from(v: f64) -> Self {
        Llsd::Real(v)
    }
}
impl From<f32> for Llsd {
    fn from(v: f32) -> Self {
        Llsd::Real(v as f64)
    }
}
impl From<&str> for Llsd {
    fn from(v: &str) -> Self {
        Llsd::String(v.to_owned())
    }
}
impl From<String> for Llsd {
    fn from(v: String) -> Self {
        Llsd::String(v)
    }
}
impl From<Uuid> for Llsd {
    fn from(v: Uuid) -> Self {
        Llsd::Uuid(v)
    }
}
impl From<Vec<Llsd>> for Llsd {
    fn from(v: Vec<Llsd>) -> Self {
        Llsd::Array(v)
    }
}
impl From<Map> for Llsd {
    fn from(v: Map) -> Self {
        Llsd::Map(v)
    }
}
impl From<Vec<u8>> for Llsd {
    fn from(v: Vec<u8>) -> Self {
        Llsd::Binary(v)
    }
}

/// Build an LLSD map literal: `llsd_map! { "a" => 1, "b" => "x" }`.
#[macro_export]
macro_rules! llsd_map {
    ($($k:expr => $v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut m = $crate::Map::new();
        $( m.insert(($k).to_string(), $crate::Llsd::from($v)); )*
        $crate::Llsd::Map(m)
    }};
}
