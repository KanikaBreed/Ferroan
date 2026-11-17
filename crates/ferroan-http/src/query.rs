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

// Ported to Rust for Ferroan from
//   Source/OpenSim.Server.Base/ServerUtils.cs
//   ParseQueryString, BuildQueryString
//   OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c
// The percent-encoding is what System.Web.HttpUtility does when those methods
// call it (UTF-8, space as '+', lowercase hex). That .NET source is not copied.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `ServerUtils.ParseQueryString` and `BuildQueryString`.

use crate::error::HttpError;
use crate::value::{Field, FieldMap};
use crate::{MAX_FIELDS, MAX_QUERY_BYTES};

const MAX_LIST_ITEMS: usize = 4096;
const MAX_BUILT_BYTES: usize = 2 * 1024 * 1024;

/// Safe bytes from `HttpUtility.UrlEncode`: RFC 1738 section 4, minus `+`.
fn is_url_safe(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'(' | b')' | b'*' | b'-' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'_' | b'a'..=b'z'
    )
}

fn url_encode(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if is_url_safe(byte) {
            out.push(byte as char);
        } else if byte == b' ' {
            out.push('+');
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

fn hex_value(ch: char) -> Option<u32> {
    match ch {
        '0'..='9' => Some(u32::from(ch) - u32::from('0')),
        'a'..='f' => Some(u32::from(ch) - u32::from('a') + 10),
        'A'..='F' => Some(u32::from(ch) - u32::from('A') + 10),
        _ => None,
    }
}

/// `HttpUtility.UrlDecode` on a string: `+` is a space, `%HH` is a UTF-8 byte,
/// `%uXXXX` is a UTF-16 code unit. Invalid `%` sequences stay as text.
fn url_decode(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut bytes: Vec<u8> = Vec::new();
    let mut out = String::new();
    let mut pending_high: Option<u16> = None;
    let mut pos = 0;

    let flush_bytes = |bytes: &mut Vec<u8>, out: &mut String| {
        if !bytes.is_empty() {
            out.push_str(&String::from_utf8_lossy(bytes));
            bytes.clear();
        }
    };
    let push_unit =
        |unit: u16, bytes: &mut Vec<u8>, out: &mut String, pending: &mut Option<u16>| {
            flush_bytes(bytes, out);
            if let Some(high) = pending.take() {
                if (0xDC00..=0xDFFF).contains(&unit) {
                    let scalar = 0x1_0000
                        + (((u32::from(high) - 0xD800) << 10) | (u32::from(unit) - 0xDC00));
                    out.push(char::from_u32(scalar).unwrap_or('\u{FFFD}'));
                    return;
                }
                out.push('\u{FFFD}');
            }
            if (0xD800..=0xDBFF).contains(&unit) {
                *pending = Some(unit);
            } else if (0xDC00..=0xDFFF).contains(&unit) {
                out.push('\u{FFFD}');
            } else {
                out.push(char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}'));
            }
        };

    while pos < chars.len() {
        let ch = chars[pos];
        if ch == '+' {
            bytes.push(b' ');
            pos += 1;
            continue;
        }
        if ch == '%' && pos + 2 < chars.len() {
            if chars[pos + 1] == 'u' && pos + 5 < chars.len() {
                let digits = [
                    hex_value(chars[pos + 2]),
                    hex_value(chars[pos + 3]),
                    hex_value(chars[pos + 4]),
                    hex_value(chars[pos + 5]),
                ];
                if digits.iter().all(Option::is_some) {
                    let unit = ((digits[0].unwrap() << 12)
                        | (digits[1].unwrap() << 8)
                        | (digits[2].unwrap() << 4)
                        | digits[3].unwrap()) as u16;
                    pos += 6;
                    push_unit(unit, &mut bytes, &mut out, &mut pending_high);
                    continue;
                }
            } else if let (Some(high), Some(low)) =
                (hex_value(chars[pos + 1]), hex_value(chars[pos + 2]))
            {
                bytes.push(((high << 4) | low) as u8);
                pos += 3;
                continue;
            }
        }
        if (ch as u32) < 0x80 {
            bytes.push(ch as u8);
        } else {
            push_unit(ch as u32 as u16, &mut bytes, &mut out, &mut pending_high);
        }
        pos += 1;
    }
    flush_bytes(&mut bytes, &mut out);
    if pending_high.is_some() {
        out.push('\u{FFFD}');
    }
    out
}

fn blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

pub fn parse_query_string(query: &str) -> Result<FieldMap, HttpError> {
    parse_query_string_limited(query, MAX_QUERY_BYTES, MAX_FIELDS)
}

pub fn parse_query_string_limited(
    query: &str,
    max_bytes: usize,
    max_fields: usize,
) -> Result<FieldMap, HttpError> {
    if query.len() > max_bytes {
        return Err(HttpError::Limit {
            what: "query",
            limit: max_bytes,
            got: query.len(),
        });
    }
    let mut result = FieldMap::new();
    if query.is_empty() {
        return Ok(result);
    }
    for term in query.split('&') {
        let parts: Vec<&str> = term.split('=').collect();
        if parts.is_empty() || blank(parts[0]) {
            continue;
        }
        let name = url_decode(parts[0]);
        let item = if parts.len() > 1 && !blank(parts[1]) {
            url_decode(parts[1])
        } else {
            String::new()
        };
        if let Some(bare) = name.strip_suffix("[]") {
            if bare.is_empty() {
                continue;
            }
            match result.get_mut(bare) {
                Some(Field::List(list)) => {
                    if list.len() >= MAX_LIST_ITEMS {
                        return Err(HttpError::Limit {
                            what: "query list",
                            limit: MAX_LIST_ITEMS,
                            got: list.len() + 1,
                        });
                    }
                    list.push(item);
                }
                Some(_) => {}
                None => {
                    if result.len() >= max_fields {
                        return Err(HttpError::Limit {
                            what: "query fields",
                            limit: max_fields,
                            got: result.len() + 1,
                        });
                    }
                    result.insert(bare.to_string(), Field::List(vec![item]));
                }
            }
        } else if !result.contains_key(&name) {
            if result.len() >= max_fields {
                return Err(HttpError::Limit {
                    what: "query fields",
                    limit: max_fields,
                    got: result.len() + 1,
                });
            }
            result.insert(name, Field::Str(item));
        }
    }
    Ok(result)
}

pub fn build_query_string(fields: &FieldMap) -> Result<String, HttpError> {
    let mut out = String::new();
    for (key, value) in fields {
        match value {
            Field::List(items) => {
                let encoded_key = url_encode(key);
                for item in items {
                    if !out.is_empty() {
                        out.push('&');
                    }
                    out.push_str(&encoded_key);
                    out.push_str("[]=");
                    out.push_str(&url_encode(item));
                }
            }
            Field::Map(map) => {
                let json = write_json_map(map);
                if !out.is_empty() {
                    out.push('&');
                }
                out.push_str(&url_encode(key));
                out.push('=');
                out.push_str(&url_encode(&json));
            }
            Field::Null => return Err(HttpError::Query("null value")),
            other => {
                let text = other
                    .csharp_text()
                    .ok_or(HttpError::Query("unsupported value"))?;
                if !out.is_empty() {
                    out.push('&');
                }
                out.push_str(&url_encode(key));
                if !text.is_empty() {
                    out.push('=');
                    out.push_str(&url_encode(&text));
                }
            }
        }
        if out.len() > MAX_BUILT_BYTES {
            return Err(HttpError::Limit {
                what: "built query",
                limit: MAX_BUILT_BYTES,
                got: out.len(),
            });
        }
    }
    Ok(out)
}

/// Compact JSON for a nested dictionary. LitJson's `ToJson` is what
/// `BuildQueryString` calls. Spaces are not inserted. Strings escape quotes,
/// backslashes and controls; other characters are written as themselves.
fn write_json_map(map: &FieldMap) -> String {
    let mut out = String::from("{");
    let mut first = true;
    for (key, value) in map {
        if !first {
            out.push(',');
        }
        first = false;
        write_json_string(&mut out, key);
        out.push(':');
        write_json_field(&mut out, value);
    }
    out.push('}');
    out
}

fn write_json_field(out: &mut String, value: &Field) {
    match value {
        Field::Null => out.push_str("null"),
        Field::Bool(true) => out.push_str("true"),
        Field::Bool(false) => out.push_str("false"),
        Field::Int(number) => out.push_str(&number.to_string()),
        Field::Str(text) | Field::Uuid(text) => write_json_string(out, text),
        Field::List(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_json_string(out, item);
            }
            out.push(']');
        }
        Field::Map(map) => out.push_str(&write_json_map(map)),
    }
}

fn write_json_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                out.push_str(&format!("\\u{code:04x}", code = ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}
