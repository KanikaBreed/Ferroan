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
//   Source/OpenSim.Framework.Serialization/External/LandDataSerializer.cs
//   (Deserialize, ProcessParcelAccessList, ProcessParcelEnvironment)
//   Source/OpenSim.Framework.Serialization/External/ExternalRepresentationUtils.cs
//   (ExecuteReadProcessors)
//   OpenSim-Tranquillity `dev-build` @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// and OpenMetaverse.Types/UUID.cs (Parse) at
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `LandDataSerializer.Deserialize`: one `landdata/` file into a parcel.
//!
//! The parcel starts as `new LandData()`: a random global id, the zero owner,
//! "Your Parcel", the default flags and a 512-byte zero bitmap. Each child of
//! `LandData` whose name has a reader sets its field. An element with no
//! reader is skipped, and so is `Dwell`, which the serializer writes but does
//! not read. A later element of the same name wins; access entries add up.
//!
//! A value that does not convert keeps the field as it was, and the read
//! goes on with the next element. The tenth such failure stops the read, and
//! fails it when anything but whitespace follows. Pin8's streaming reader
//! loses its place after a failure in the last element, or in an element with
//! child elements, and then stalls for 60 seconds and fails or drops the
//! elements after it (TB203). Ferroan reads every element in place (D139),
//! so a Pin8-written file whose last element, `Environment`, cannot be read
//! loads with the environment cleared. The tree parse drops comments and
//! leaves undeclared entities as text; D139 lists where that differs.
//!
//! An access entry is read until the first child of `ParcelAccessList` that
//! is not a `ParcelAccessEntry`. An entry's `Time` is read and ignored: every
//! entry loads with `Expires` 0.

use ferroan_diag::{OAR_PARCEL_ACCESS, OAR_PARCEL_BITMAP_BYTES};
use ferroan_llsd::xml::{Element, Node};
use uuid::Uuid;

use super::{MAX_ACCESS_ENTRIES, MAX_BITMAP_BYTES, OarParcel, OarParcelError, refuse_limit};
use crate::parcels::{LandAccess, Parcel, environment_version, vector_text};

/// `ExecuteReadProcessors` stops after this many failed elements.
const READ_ERRORS: u32 = 10;

/// `new LandData()`, as `Deserialize` starts it.
fn fresh() -> OarParcel {
    OarParcel {
        data: Parcel::estate_land(Uuid::nil(), 0),
        bitmap: vec![0; 512],
    }
}

/// One parcel file's text. `access_used` counts the access entries read so
/// far in this load, against `oar.parcel-access`.
pub(super) fn deserialize(
    text: &str,
    access_used: &mut usize,
) -> Result<OarParcel, OarParcelError> {
    let root = ferroan_llsd::xml::parse(text.as_bytes())
        .map_err(|error| OarParcelError::Xml(error.to_string()))?;
    // ReadStartElement("LandData").
    if root.name != "LandData" {
        return Err(OarParcelError::NotLandData(root.name));
    }
    // `<LandData/>` leaves the reader at the end of the document, where the
    // read loop spins until its 60-second timeout and ReadEndElement then
    // throws (TB203). Ferroan fails at once.
    if root.self_closing {
        return Err(OarParcelError::EmptyLandData);
    }
    let mut parcel = fresh();
    let mut errors = 0;
    for (index, node) in root.children.iter().enumerate() {
        let Node::Element(child) = node else {
            continue;
        };
        match read_field(&mut parcel, child, access_used)? {
            Field::Read | Field::Unknown => {}
            Field::Bad => {
                errors += 1;
                tracing::debug!(
                    "[ExternalRepresentationUtils]: Error while parsing element {}",
                    child.name
                );
                if errors == READ_ERRORS {
                    // The read stops here. ReadEndElement then needs the end tag.
                    if root.children[index + 1..].iter().any(is_content) {
                        return Err(OarParcelError::TooManyErrors);
                    }
                    break;
                }
            }
        }
    }
    Ok(parcel)
}

enum Field {
    Read,
    Bad,
    Unknown,
}

impl From<Option<()>> for Field {
    fn from(read: Option<()>) -> Self {
        match read {
            Some(()) => Field::Read,
            None => Field::Bad,
        }
    }
}

/// `m_ldProcessors`.
fn read_field(
    parcel: &mut OarParcel,
    el: &Element,
    access_used: &mut usize,
) -> Result<Field, OarParcelError> {
    let data = &mut parcel.data;
    let read = match el.name.as_str() {
        "Area" => set(&mut data.area, el, int32),
        "AuctionID" => set(&mut data.auction, el, uint32),
        "AuthBuyerID" => set(&mut data.auth_buyer, el, omv_uuid),
        "Category" => set(&mut data.category, el, |text| sbyte(text).map(i32::from)),
        "ClaimDate" => set(&mut data.claim_date, el, int32),
        "ClaimPrice" => set(&mut data.claim_price, el, int32),
        "GlobalID" => set(&mut data.id, el, omv_uuid),
        "GroupID" => set(&mut data.group, el, omv_uuid),
        "IsGroupOwned" => set(&mut data.group_owned, el, boolean),
        "Bitmap" => {
            let Some(bitmap) = simple_text(el).and_then(|text| base64(&text)) else {
                return Ok(Field::Bad);
            };
            refuse_limit(
                OAR_PARCEL_BITMAP_BYTES,
                MAX_BITMAP_BYTES,
                bitmap.len(),
                OarParcelError::Limit(OAR_PARCEL_BITMAP_BYTES),
            )?;
            parcel.bitmap = bitmap;
            Some(())
        }
        "Description" => set(&mut data.description, el, |text| Some(text.to_owned())),
        "Flags" => set(&mut data.flags, el, uint32),
        "LandingType" => set(&mut data.landing_type, el, byte),
        "Name" => set(&mut data.name, el, |text| Some(text.to_owned())),
        "Status" => set(&mut data.status, el, |text| sbyte(text).map(i32::from)),
        "LocalID" => set(&mut data.local_id, el, int32),
        "MediaAutoScale" => set(&mut data.media_auto_scale, el, byte),
        "MediaID" => set(&mut data.media_id, el, omv_uuid),
        "MediaURL" => set(&mut data.media_url, el, |text| Some(text.to_owned())),
        "MusicURL" => set(&mut data.music_url, el, |text| Some(text.to_owned())),
        "MediaType" => set(&mut data.media_type, el, |text| Some(text.to_owned())),
        "MediaDesc" => set(&mut data.media_description, el, |text| {
            Some(text.to_owned())
        }),
        "MediaH" => set(&mut data.media_size[1], el, int32),
        "MediaW" => set(&mut data.media_size[0], el, int32),
        "MediaLoop" => set(&mut data.media_loop, el, boolean),
        "ObscureMOAP" => set(&mut data.obscure_media, el, boolean),
        "OwnerID" => set(&mut data.owner, el, omv_uuid),
        "ParcelAccessList" => {
            access_list(&mut data.access, el, access_used)?;
            Some(())
        }
        "Environment" => environment(data, el),
        "PassHours" => set(&mut data.pass_hours, el, single),
        "PassPrice" => set(&mut data.pass_price, el, int32),
        "SalePrice" => set(&mut data.sale_price, el, int32),
        "SnapshotID" => set(&mut data.snapshot, el, omv_uuid),
        "UserLocation" => set(&mut data.location, el, vector_text::vector3),
        "UserLookAt" => set(&mut data.look_at, el, vector_text::vector3),
        "OtherCleanTime" => set(&mut data.other_clean_time, el, int32),
        "SeeAVs" => set(&mut data.see_avs, el, |text| Some(text == "1")),
        "AnyAVSnds" => set(&mut data.any_av_sounds, el, |text| Some(text == "1")),
        "GrpAVSnds" => set(&mut data.group_av_sounds, el, |text| Some(text == "1")),
        _ => return Ok(Field::Unknown),
    };
    Ok(read.into())
}

/// `field = convert(ReadElementString(name))`. A failure leaves the field.
fn set<T>(field: &mut T, el: &Element, convert: impl Fn(&str) -> Option<T>) -> Option<()> {
    *field = convert(&simple_text(el)?)?;
    Some(())
}

/// `ProcessParcelEnvironment`. `FromOSDString` answers null for text it
/// cannot read, and reading that null's version throws after the null is
/// stored: the environment is cleared and the version stays.
fn environment(data: &mut Parcel, el: &Element) -> Option<()> {
    let text = simple_text(el)?;
    match environment_version(&text) {
        Some(version) => {
            data.environment = Some(text);
            data.environment_version = version;
            Some(())
        }
        None => {
            data.environment = None;
            None
        }
    }
}

/// `ProcessParcelAccessList`. Entries are read while the next content is a
/// `ParcelAccessEntry`. An empty `<ParcelAccessEntry/>` adds nothing; an
/// entry written as a start and end tag adds an entry with the zero agent.
fn access_list(
    access: &mut Vec<LandAccess>,
    el: &Element,
    access_used: &mut usize,
) -> Result<(), OarParcelError> {
    if el.self_closing {
        return Ok(());
    }
    for node in &el.children {
        let entry = match node {
            Node::Text(text) if is_whitespace(text) => continue,
            Node::Element(entry) if entry.name == "ParcelAccessEntry" => entry,
            _ => break,
        };
        if entry.self_closing {
            continue;
        }
        *access_used += 1;
        refuse_limit(
            OAR_PARCEL_ACCESS,
            MAX_ACCESS_ENTRIES,
            *access_used,
            OarParcelError::Limit(OAR_PARCEL_ACCESS),
        )?;
        access.push(access_entry(entry));
    }
    Ok(())
}

/// `m_laeProcessors`, through `ExecuteReadProcessors`. `Time` is read and
/// dropped: Pin8 treats every entry as permanent.
fn access_entry(el: &Element) -> LandAccess {
    let mut entry = LandAccess {
        agent: Uuid::nil(),
        flags: 0,
        expires: 0,
    };
    let mut errors = 0;
    for child in el.elements() {
        let read = match child.name.as_str() {
            "AgentID" => set(&mut entry.agent, child, omv_uuid),
            "Time" => simple_text(child).map(drop),
            "AccessList" => set(&mut entry.flags, child, uint32),
            _ => continue,
        };
        if read.is_none() {
            errors += 1;
            if errors == READ_ERRORS {
                break;
            }
        }
    }
    entry
}

/// `ReadElementString`: the element's text. Child elements throw.
fn simple_text(el: &Element) -> Option<String> {
    if el.elements().next().is_some() {
        return None;
    }
    Some(el.text())
}

/// What `MoveToContent` stops at: an element, or text that is not whitespace.
fn is_content(node: &Node) -> bool {
    match node {
        Node::Element(_) => true,
        Node::Text(text) => !is_whitespace(text),
    }
}

/// An XML whitespace node: space, tab, carriage return and line feed.
fn is_whitespace(text: &str) -> bool {
    text.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
}

/// `NumberStyles.Integer`: white space (U+0009 to U+000D and space) around an
/// optional sign and the digits, then any trailing NULs.
fn integer_text(text: &str) -> &str {
    text.trim_end_matches('\0')
        .trim_matches(|c| matches!(c, ' ' | '\t'..='\r'))
}

/// `Convert.ToInt32(string)`.
pub(super) fn int32(text: &str) -> Option<i32> {
    integer_text(text).parse().ok()
}

/// `Convert.ToSByte(string)`.
pub(super) fn sbyte(text: &str) -> Option<i8> {
    integer_text(text).parse().ok()
}

/// `Convert.ToUInt32(string)`. A negative zero ("-0", "-00") parses as 0.
pub(super) fn uint32(text: &str) -> Option<u32> {
    let text = integer_text(text);
    if let Some(digits) = text.strip_prefix('-') {
        return (!digits.is_empty() && digits.bytes().all(|b| b == b'0')).then_some(0);
    }
    text.parse().ok()
}

/// `Convert.ToByte(string)`.
pub(super) fn byte(text: &str) -> Option<u8> {
    u8::try_from(uint32(text)?).ok()
}

/// `Convert.ToBoolean(string)`: "True" or "False" in any case, with white
/// space and NULs trimmed.
pub(super) fn boolean(text: &str) -> Option<bool> {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

/// `Convert.ToSingle(string)`: `NumberStyles.Float` with group separators
/// allowed in the integral digits.
pub(super) fn single(text: &str) -> Option<f32> {
    let trimmed = integer_text(text);
    let unsigned = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    let integral = unsigned
        .find(['.', 'e', 'E'])
        .map_or(unsigned, |end| &unsigned[..end]);
    if !integral.contains(',') {
        return vector_text::number(text);
    }
    if integral.starts_with(',') || !integral.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    let sign = &trimmed[..trimmed.len() - unsigned.len()];
    let joined = format!(
        "{sign}{}{}",
        integral.replace(',', ""),
        &unsigned[integral.len()..]
    );
    vector_text::number(&joined)
}

/// `Convert.FromBase64String`. Space, tab, carriage return and line feed are
/// ignored. Padding may only end the text.
pub(super) fn base64(text: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = text
        .bytes()
        .filter(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .collect();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let (quads, _) = bytes.as_chunks::<4>();
    for quad in quads {
        let mut word = 0u32;
        let mut pads = 0;
        for &byte in quad {
            word <<= 6;
            if byte == b'=' {
                pads += 1;
            } else if pads > 0 {
                return None;
            } else {
                word |= value(byte)?;
            }
        }
        let [_, a, b, c] = word.to_be_bytes();
        out.push(a);
        if pads < 2 {
            out.push(b);
        }
        if pads < 1 {
            out.push(c);
        }
    }
    // Only the last quad may hold padding.
    let padded_quads = quads.iter().filter(|quad| quad.contains(&b'=')).count();
    if padded_quads > 1 || (padded_quads == 1 && padding == 0) {
        return None;
    }
    Some(out)
}

/// libopenmetaverse `UUID.Parse`: leading spaces are skipped, then 36
/// hyphenated or 32 plain hex digits. Anything after them is ignored.
pub(super) fn omv_uuid(text: &str) -> Option<Uuid> {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() < 32 {
        return None;
    }
    let start = units
        .iter()
        .take_while(|unit| **unit == u16::from(b' '))
        .count();
    let rest = &units[start..];
    if rest.len() < 32 {
        return None;
    }
    let length = if rest[8] == u16::from(b'-') { 36 } else { 32 };
    let digits = String::from_utf16(rest.get(..length)?).ok()?;
    if !digits.bytes().all(|b| b == b'-' || b.is_ascii_hexdigit()) {
        return None;
    }
    Uuid::try_parse(&digits).ok()
}
