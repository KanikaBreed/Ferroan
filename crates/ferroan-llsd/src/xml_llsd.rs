// Copyright (c) 2006-2016, openmetaverse.co
// Copyright (c) OpenSimulator Contributors, http://opensimulator.org/
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
// XML binary decoding follows OpenMetaverse.StructuredData/LLSD/XmlLLSD.cs
//   OpenSim-NGC/libopenmetaverse @ a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b
//
// Copyright (c) 2026 the Aurora Viewer contributors
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Adapted from Aurora Viewer crates/aurora-llsd/src/xml_llsd.rs
//   Aurora-Viewer/Aurora-Viewer @ 48cb76e830a13965ac080128bb874afedda86d69
// Originally LGPL 2.1, from the Second Life viewer's llcommon/llsdserialize.cpp.
//
//! LLSD <-> XML (`application/llsd+xml`).

use crate::number::{self, format_date, parse_date};
use crate::xml::{self, Element};
use crate::{Llsd, LlsdError, Map};
use base64::Engine;
use uuid::Uuid;

fn decode_binary(el: &Element) -> Result<Vec<u8>, LlsdError> {
    // ParseLLSDXmlElement checks IsEmptyElement before examining encoding.
    if el.self_closing {
        return Ok(Vec::new());
    }
    if el.attr("encoding").is_some_and(|value| value != "base64") {
        return Err(LlsdError::InvalidBinary("unsupported encoding"));
    }
    // ReadString().Trim() accepts Unicode whitespace at the edges, whereas
    // Convert.FromBase64String ignores only these four ASCII characters inside.
    let text = el.text();
    let compact: Vec<u8> = text
        .trim()
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .collect();
    // .NET requires padding but accepts nonzero unused bits in the last sextet.
    let config = base64::engine::general_purpose::GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true);
    let engine = base64::engine::GeneralPurpose::new(&base64::alphabet::STANDARD, config);
    engine
        .decode(compact)
        .map_err(|_| LlsdError::InvalidBinary("invalid base64"))
}

/// Convert a parsed XML value element (e.g. `<map>`) into LLSD.
pub fn from_xml_element(el: &Element) -> Result<Llsd, LlsdError> {
    let text = || el.text();
    Ok(match el.name.as_str() {
        "llsd" => match el.elements().next() {
            Some(c) => from_xml_element(c)?,
            None => Llsd::Undef,
        },
        "undef" => Llsd::Undef,
        "boolean" => {
            let t = text();
            let t = t.trim();
            Llsd::Boolean(t == "1" || t.eq_ignore_ascii_case("true"))
        }
        "integer" => {
            let t = text();
            let t = t.trim();
            Llsd::Integer(
                t.parse::<i32>()
                    .or_else(|_| t.parse::<f64>().map(|f| f as i32))
                    .unwrap_or(0),
            )
        }
        "real" => {
            let t = text();
            let t = t.trim();
            Llsd::Real(match t {
                // Double.NaN is the quiet NaN with the sign bit set.
                "nan" | "NaN" => f64::from_bits(0xfff8_0000_0000_0000),
                "inf" | "+inf" | "infinity" => f64::INFINITY,
                "-inf" | "-infinity" => f64::NEG_INFINITY,
                _ => t.parse().unwrap_or(0.0),
            })
        }
        "string" => Llsd::String(text()),
        "uuid" => Llsd::Uuid(Uuid::parse_str(text().trim()).unwrap_or(Uuid::nil())),
        "date" => Llsd::Date(parse_date(&text())),
        "uri" => Llsd::Uri(text()),
        "binary" => Llsd::Binary(decode_binary(el)?),
        "array" => {
            let mut a = Vec::new();
            for c in el.elements() {
                a.push(from_xml_element(c)?);
            }
            Llsd::Array(a)
        }
        "map" => {
            let mut m = Map::new();
            let mut key: Option<String> = None;
            for c in el.elements() {
                if c.name == "key" {
                    key = Some(c.text());
                } else if let Some(k) = key.take() {
                    m.insert(k, from_xml_element(c)?);
                } else {
                    return Err(LlsdError::Invalid("map value without key"));
                }
            }
            Llsd::Map(m)
        }
        _ => return Err(LlsdError::Invalid("unknown LLSD element")),
    })
}

/// Parse an `<llsd>` XML document.
///
/// A NUL, or any other XML 1.0 forbidden control byte, makes
/// `XmlTextReader` throw. `DeserializeLLSDXml` catches that and returns
/// undef. Infinity is written as the single byte 0x1E, which is one of
/// those bytes, so it comes back as undef too.
/// Invalid binary values likewise invalidate the entire document, including
/// when nested in an array or map. Other existing structural errors remain
/// errors in this API; the C# public decoder catches all parsing exceptions.
pub fn from_xml(src: &[u8]) -> Result<Llsd, LlsdError> {
    if src
        .iter()
        .any(|b| *b < 0x20 && !matches!(*b, b'\t' | b'\n' | b'\r'))
    {
        return Ok(Llsd::Undef);
    }
    let root = xml::parse(src)?;
    match from_xml_element(&root) {
        Err(LlsdError::InvalidBinary(_)) => Ok(Llsd::Undef),
        result => result,
    }
}

fn write_value(v: &Llsd, out: &mut String) {
    write_value_in(v, false, out);
}

/// OpenMetaverse's byte encoder (`SerializeLLSDXmlElement` on `osUTF8`).
///
/// Empty maps and arrays are `<map></map>` and `<array></array>`. Undef is
/// `<undef/>`. Informal binary has no encoding attribute. Real text is the
/// low 8 bits of each character, so U+221E becomes the byte 0x1E. Map keys
/// are not escaped.
fn write_value_in(v: &Llsd, formal: bool, out: &mut String) {
    match v {
        Llsd::Undef => out.push_str("<undef/>"),
        Llsd::Boolean(b) => {
            out.push_str(if *b {
                "<boolean>1</boolean>"
            } else {
                "<boolean>0</boolean>"
            });
        }
        Llsd::Integer(i) => {
            out.push_str("<integer>");
            out.push_str(&i.to_string());
            out.push_str("</integer>");
        }
        Llsd::Real(r) => {
            out.push_str("<real>");
            number::push_ascii_low(&number::format_real(*r), out);
            out.push_str("</real>");
        }
        Llsd::String(s) => {
            out.push_str("<string>");
            xml::escape(s, out);
            out.push_str("</string>");
        }
        Llsd::Uuid(u) => {
            out.push_str("<uuid>");
            out.push_str(&u.to_string());
            out.push_str("</uuid>");
        }
        Llsd::Date(d) => {
            out.push_str("<date>");
            number::push_ascii_low(&format_date(*d), out);
            out.push_str("</date>");
        }
        Llsd::Uri(s) => {
            out.push_str("<uri>");
            xml::escape(s, out);
            out.push_str("</uri>");
        }
        Llsd::Binary(b) => {
            if formal {
                out.push_str("<binary encoding=\"base64\">");
            } else {
                out.push_str("<binary>");
            }
            out.push_str(&base64::engine::general_purpose::STANDARD.encode(b));
            out.push_str("</binary>");
        }
        Llsd::Array(a) => {
            out.push_str("<array>");
            for e in a {
                write_value_in(e, formal, out);
            }
            out.push_str("</array>");
        }
        Llsd::Map(m) => {
            out.push_str("<map>");
            for (k, e) in m {
                out.push_str("<key>");
                out.push_str(k);
                out.push_str("</key>");
                write_value_in(e, formal, out);
            }
            out.push_str("</map>");
        }
    }
}

/// Informal LLSD XML: `<llsd>` and the value, with no XML declaration.
/// This is `SerializeLLSDXmlToBytes(value, false)`.
pub fn to_xml_string(v: &Llsd) -> String {
    let mut s = String::from("<llsd>");
    write_value(v, &mut s);
    s.push_str("</llsd>");
    s
}

pub fn to_xml(v: &Llsd) -> Vec<u8> {
    to_xml_string(v).into_bytes()
}

/// `SerializeLLSDXmlToBytes(value, true)`.
pub fn to_xml_formal(v: &Llsd) -> Vec<u8> {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><llsd>");
    write_value_in(v, true, &mut s);
    s.push_str("</llsd>");
    s.into_bytes()
}

/// LLSDSerialize::serialize(..., LLSD_XML), with the stream format header
/// (indra/llcommon/llsdserialize.cpp and llsdserialize_xml.cpp, originally
/// LGPL 2.1). Inventory buckets in group notices use this envelope; HTTP
/// capabilities use `to_xml` instead. The transmitted bytes exclude a NUL.
pub fn to_serialized_xml(v: &Llsd) -> Vec<u8> {
    let mut s = String::from("<? LLSD/XML ?>\n<llsd>");
    write_value(v, &mut s);
    s.push_str("</llsd>\n");
    s.into_bytes()
}

/// The indented form people read and keep in files: one element per line,
/// four spaces per level below `<llsd>`. Port of LLSDXMLFormatter::format
/// with OPTIONS_PRETTY (indra/llcommon/llsdserialize_xml.cpp, originally
/// LGPL 2.1), byte for byte, so that files written here and by the Legacy
/// Viewer look the same.
pub fn to_pretty_xml_string(v: &Llsd) -> String {
    let mut s = String::from("<?xml version=\"1.0\" ?>\n<llsd>\n");
    write_pretty(v, 1, &mut s);
    s.push_str("</llsd>\n");
    s
}

fn write_pretty(v: &Llsd, level: usize, out: &mut String) {
    let pre = "    ".repeat(level);
    match v {
        Llsd::Map(m) if !m.is_empty() => {
            out.push_str(&pre);
            out.push_str("<map>\n");
            for (k, e) in m {
                out.push_str(&pre);
                out.push_str("<key>");
                xml::escape(k, out);
                out.push_str("</key>\n");
                write_pretty(e, level + 1, out);
            }
            out.push_str(&pre);
            out.push_str("</map>\n");
        }
        Llsd::Array(a) if !a.is_empty() => {
            out.push_str(&pre);
            out.push_str("<array>\n");
            for e in a {
                write_pretty(e, level + 1, out);
            }
            out.push_str(&pre);
            out.push_str("</array>\n");
        }
        // LL writes these empty values as empty elements
        Llsd::String(s) if s.is_empty() => {
            out.push_str(&pre);
            out.push_str("<string />\n");
        }
        Llsd::Uuid(u) if u.is_nil() => {
            out.push_str(&pre);
            out.push_str("<uuid />\n");
        }
        _ => {
            out.push_str(&pre);
            write_value(v, out);
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llsd_map;

    #[test]
    fn invalid_binary_propagates_before_public_undef_mapping() {
        let doc = b"<llsd><map><key>nested</key><array><binary>!</binary></array></map></llsd>";
        let root = xml::parse(doc).unwrap();
        assert!(matches!(
            from_xml_element(&root),
            Err(LlsdError::InvalidBinary("invalid base64"))
        ));
        assert_eq!(from_xml(doc).unwrap(), Llsd::Undef);
        // The binary change does not alter the API's other structural errors.
        assert!(from_xml(b"<llsd><map><integer>1</integer></map></llsd>").is_err());
    }

    #[test]
    fn roundtrip() {
        let mut v = llsd_map! {
            "a" => 1, "b" => "x<y", "c" => 2.5, "d" => true,
            "e" => Uuid::from_u128(0x1234), "f" => vec![1u8, 2, 3],
        };
        v.insert("g", Llsd::Array(vec![Llsd::Undef, Llsd::Integer(5)]));
        let xml = to_xml(&v);
        let back = from_xml(&xml).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn pretty_matches_ll_and_reads_back() {
        let mut v = llsd_map! { "version" => 1, "name" => "" };
        v.insert(
            "list",
            Llsd::Array(vec![Llsd::from("a&b"), Llsd::new_map()]),
        );
        let xml = to_pretty_xml_string(&v);
        let expected = "<?xml version=\"1.0\" ?>\n<llsd>\n    <map>\n    <key>version</key>\n        <integer>1</integer>\n    <key>name</key>\n        <string />\n    <key>list</key>\n        <array>\n            <string>a&amp;b</string>\n            <map></map>\n        </array>\n    </map>\n</llsd>\n";
        assert_eq!(xml, expected);
        assert_eq!(from_xml(xml.as_bytes()).unwrap(), v);
    }

    #[test]
    fn parses_ll_style() {
        let doc = b"<?xml version=\"1.0\" ?>\n<llsd>\n<map>\n  <key>events</key>\n  <array>\n    <map><key>message</key><string>Foo</string></map>\n  </array>\n  <key>id</key><integer>3</integer>\n</map>\n</llsd>";
        let v = from_xml(doc).unwrap();
        assert_eq!(v["id"].as_i32(), 3);
        assert_eq!(v["events"][0]["message"].as_str(), "Foo");
    }
}
