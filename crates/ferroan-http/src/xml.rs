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
//   BuildXmlResponse, ParseXmlResponse
//   OpenSim-NGC/OpenSim-Tranquillity @ 713857f10b8e91a639a967d4354a4682b080793c
// Element names use XmlConvert.EncodeLocalName / DecodeName. That .NET source
// is not copied; the escapes are what those methods produce.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `ServerUtils.BuildXmlResponse` and `ParseXmlResponse`.
//!
//! A document Tranquillity cannot read becomes an empty map: `ParseXmlResponse`
//! swallows the exception. A document past the character cap or the depth cap
//! is an error. Tranquillity's cap is the character count; it has no depth cap.

use crate::error::HttpError;
use crate::value::{Field, FieldMap};
use crate::{MAX_XML_CHARS, MAX_XML_DEPTH};

enum Fail {
    Bad,
    Limit,
}

pub fn build_xml_response(fields: &FieldMap) -> Result<String, HttpError> {
    let mut out = String::from("<?xml version=\"1.0\"?>");
    write_map_element(&mut out, "ServerResponse", fields, false, 1)?;
    Ok(out)
}

pub fn parse_xml_response(data: &str) -> Result<FieldMap, HttpError> {
    parse_xml_response_limited(data, MAX_XML_CHARS, MAX_XML_DEPTH)
}

pub fn parse_xml_response_limited(
    data: &str,
    max_chars: usize,
    max_depth: usize,
) -> Result<FieldMap, HttpError> {
    let chars = data.chars().count();
    if chars > max_chars {
        return Err(HttpError::Limit {
            what: "xml",
            limit: max_chars,
            got: chars,
        });
    }
    if data.is_empty() {
        return Ok(FieldMap::new());
    }
    match parse_document(data, max_depth) {
        Ok(map) => Ok(map),
        Err(Fail::Bad) => Ok(FieldMap::new()),
        Err(Fail::Limit) => Err(HttpError::Limit {
            what: "xml depth",
            limit: max_depth,
            got: max_depth + 1,
        }),
    }
}

fn write_map_element(
    out: &mut String,
    name: &str,
    fields: &FieldMap,
    list: bool,
    depth: usize,
) -> Result<(), HttpError> {
    if depth > MAX_XML_DEPTH {
        return Err(HttpError::Limit {
            what: "xml depth",
            limit: MAX_XML_DEPTH,
            got: depth,
        });
    }
    let encoded = encode_local_name(name);
    if encoded.is_empty() {
        return Err(HttpError::Xml("empty element name"));
    }
    out.push('<');
    out.push_str(&encoded);
    if list {
        out.push_str(" type=\"List\"");
    }
    let mut body = String::new();
    let mut any = false;
    for (key, value) in fields {
        if matches!(value, Field::Null) {
            continue;
        }
        any = true;
        match value {
            Field::Map(nested) => write_map_element(&mut body, key, nested, true, depth + 1)?,
            Field::Str(_) | Field::Bool(_) | Field::Int(_) | Field::Uuid(_) => {
                let text = value.csharp_text().ok_or(HttpError::Xml("missing text"))?;
                write_text_element(&mut body, key, &text)?;
            }
            Field::List(_) => return Err(HttpError::Xml("a list is not an xml value")),
            Field::Null => {}
        }
    }
    if !any {
        out.push_str(" />");
        return Ok(());
    }
    out.push('>');
    out.push_str(&body);
    out.push_str("</");
    out.push_str(&encoded);
    out.push('>');
    Ok(())
}

fn write_text_element(out: &mut String, name: &str, text: &str) -> Result<(), HttpError> {
    let encoded = encode_local_name(name);
    if encoded.is_empty() {
        return Err(HttpError::Xml("empty element name"));
    }
    if !xml_chars(text) {
        return Err(HttpError::Xml("text is not xml"));
    }
    out.push('<');
    out.push_str(&encoded);
    out.push('>');
    push_escaped(out, text);
    out.push_str("</");
    out.push_str(&encoded);
    out.push('>');
    Ok(())
}

fn xml_chars(text: &str) -> bool {
    text.chars().all(is_xml_char)
}

fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&ch)
        || ('\u{E000}'..='\u{FFFD}').contains(&ch)
        || ('\u{10000}'..='\u{10FFFF}').contains(&ch)
}

fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            ch => out.push(ch),
        }
    }
}

fn encode_local_name(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = name.chars().collect();
    let escapes = escape_underscores(&chars);
    let mut escape_at = 0;
    let mut out = String::new();
    let mut index = 0;
    let next_escape = |escape_at: &mut usize| {
        if *escape_at < escapes.len() && escapes[*escape_at] == 0 {
            *escape_at += 1;
        }
    };
    if !is_name_start(chars[0]) || escapes.first() == Some(&0) {
        push_encoded(&mut out, chars[0]);
        index = 1;
        next_escape(&mut escape_at);
    }
    while index < chars.len() {
        let matched = escape_at < escapes.len() && escapes[escape_at] == index;
        if matched {
            escape_at += 1;
        }
        if !is_name_char(chars[index]) || matched {
            push_encoded(&mut out, chars[index]);
        } else {
            out.push(chars[index]);
        }
        index += 1;
    }
    if out.is_empty() {
        name.to_string()
    } else {
        out
    }
}

fn push_encoded(out: &mut String, ch: char) {
    out.push_str("_x");
    if (ch as u32) > 0xFFFF {
        push_hex(out, ch as u32, 8);
    } else {
        push_hex(out, ch as u32, 4);
    }
    out.push('_');
}

fn push_hex(out: &mut String, value: u32, digits: usize) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for shift in (0..digits).rev() {
        let nibble = (value >> (shift * 4)) & 0x0f;
        out.push(HEX[nibble as usize] as char);
    }
}

fn escape_underscores(chars: &[char]) -> Vec<usize> {
    let mut found = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if let Some(len) = escape_len(chars, index) {
            found.push(index);
            index += len;
        } else {
            index += 1;
        }
    }
    found
}

fn escape_len(chars: &[char], underscore: usize) -> Option<usize> {
    let rest = chars.get(underscore + 1..)?;
    if chars[underscore] != '_' || rest.len() < 6 {
        return None;
    }
    if rest[0] != 'X' && rest[0] != 'x' {
        return None;
    }
    if !rest[1..5].iter().copied().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    if rest[5] == '_' {
        return Some(7);
    }
    if rest.len() >= 10
        && rest[5..9].iter().copied().all(|ch| ch.is_ascii_hexdigit())
        && rest[9] == '_'
    {
        return Some(11);
    }
    None
}

fn is_name_start(ch: char) -> bool {
    matches!(
        ch,
        'A'..='Z'
            | '_'
            | 'a'..='z'
            | '\u{00C0}'..='\u{00D6}'
            | '\u{00D8}'..='\u{00F6}'
            | '\u{00F8}'..='\u{02FF}'
            | '\u{0370}'..='\u{037D}'
            | '\u{037F}'..='\u{1FFF}'
            | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}'
            | '\u{2C00}'..='\u{2FEF}'
            | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}'
            | '\u{FDF0}'..='\u{FFFD}'
    ) || ('\u{10000}'..='\u{EFFFF}').contains(&ch)
}

fn is_name_char(ch: char) -> bool {
    is_name_start(ch)
        || matches!(
            ch,
            '-' | '.' | '0'..='9' | '\u{00B7}' | '\u{0300}'..='\u{036F}' | '\u{203F}'..='\u{2040}'
        )
}

fn decode_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if let Some((scalar, len)) = encoded_scalar(&chars, index) {
            if let Some(ch) = char::from_u32(scalar) {
                out.push(ch);
            }
            index += len;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

fn encoded_scalar(chars: &[char], index: usize) -> Option<(u32, usize)> {
    let len = escape_len(chars, index)?;
    let hex: String = chars[index + 2..index + len - 1].iter().collect();
    let scalar = u32::from_str_radix(&hex, 16).ok()?;
    if scalar > 0x10_FFFF {
        return None;
    }
    Some((scalar, len))
}

struct Cursor<'a> {
    text: &'a str,
    byte: usize,
    depth: usize,
    max_depth: usize,
}

fn parse_document(data: &str, max_depth: usize) -> Result<FieldMap, Fail> {
    let mut cursor = Cursor {
        text: data,
        byte: 0,
        depth: 1,
        max_depth,
    };
    cursor.find_server_response()
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.byte..]
    }

    fn eof(&self) -> bool {
        self.byte >= self.text.len()
    }

    fn bump(&mut self) -> Option<char> {
        let mut chars = self.rest().chars();
        let ch = chars.next()?;
        self.byte += ch.len_utf8();
        Some(ch)
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.rest().starts_with(prefix)
    }

    fn skip_misc(&mut self) -> Result<(), Fail> {
        loop {
            self.skip_ws();
            if self.starts_with("<!--") {
                self.skip_comment()?;
                continue;
            }
            if self.starts_with("<?") {
                self.skip_pi()?;
                continue;
            }
            if self.starts_with("<!DOCTYPE") {
                return Err(Fail::Bad);
            }
            break;
        }
        Ok(())
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.rest().chars().next() {
            if !matches!(ch, ' ' | '\t' | '\n' | '\r') {
                break;
            }
            self.bump();
        }
    }

    fn skip_comment(&mut self) -> Result<(), Fail> {
        if !self.starts_with("<!--") {
            return Err(Fail::Bad);
        }
        self.byte += 4;
        while !self.eof() {
            if self.starts_with("-->") {
                self.byte += 3;
                return Ok(());
            }
            self.bump();
        }
        Err(Fail::Bad)
    }

    fn skip_pi(&mut self) -> Result<(), Fail> {
        if !self.starts_with("<?") {
            return Err(Fail::Bad);
        }
        self.byte += 2;
        while !self.eof() {
            if self.starts_with("?>") {
                self.byte += 2;
                return Ok(());
            }
            self.bump();
        }
        Err(Fail::Bad)
    }

    fn find_server_response(&mut self) -> Result<FieldMap, Fail> {
        loop {
            self.skip_misc()?;
            if self.eof() {
                return Ok(FieldMap::new());
            }
            if !self.starts_with("<") {
                return Err(Fail::Bad);
            }
            let saved = self.byte;
            let (name, _, empty) = self.read_start()?;
            if name == "ServerResponse" {
                if empty {
                    return Ok(FieldMap::new());
                }
                let map = self.read_children()?;
                self.expect_end("ServerResponse")?;
                return Ok(map);
            }
            if empty {
                continue;
            }
            // Not the response. Step back and skip the whole element, then keep looking.
            self.byte = saved;
            self.skip_element()?;
        }
    }

    fn skip_element(&mut self) -> Result<(), Fail> {
        let (name, _, empty) = self.read_start()?;
        if empty {
            return Ok(());
        }
        loop {
            self.skip_misc()?;
            if self.starts_with("</") {
                self.expect_end(&name)?;
                return Ok(());
            }
            if self.eof() || !self.starts_with("<") {
                return Err(Fail::Bad);
            }
            self.skip_element()?;
        }
    }

    fn read_children(&mut self) -> Result<FieldMap, Fail> {
        let mut map = FieldMap::new();
        loop {
            self.skip_misc()?;
            if self.starts_with("</") || self.eof() {
                break;
            }
            let (name, is_list, empty) = self.read_start()?;
            let value = if empty {
                if is_list {
                    Field::Map(FieldMap::new())
                } else {
                    Field::Str(String::new())
                }
            } else if is_list {
                self.depth += 1;
                if self.depth > self.max_depth {
                    return Err(Fail::Limit);
                }
                let nested = self.read_children()?;
                self.expect_end(&name)?;
                self.depth -= 1;
                Field::Map(nested)
            } else {
                let text = self.read_text()?;
                self.expect_end(&name)?;
                Field::Str(text)
            };
            map.insert(decode_name(&name), value);
        }
        Ok(map)
    }

    fn read_start(&mut self) -> Result<(String, bool, bool), Fail> {
        if self.bump() != Some('<') {
            return Err(Fail::Bad);
        }
        let name = self.read_name()?;
        if name.is_empty() {
            return Err(Fail::Bad);
        }
        let mut is_list = false;
        loop {
            self.skip_ws();
            if self.starts_with("/>") {
                self.byte += 2;
                return Ok((name, is_list, true));
            }
            if self.starts_with(">") {
                self.byte += 1;
                return Ok((name, is_list, false));
            }
            let attr = self.read_name()?;
            self.skip_ws();
            if self.bump() != Some('=') {
                return Err(Fail::Bad);
            }
            self.skip_ws();
            let value = self.read_attr()?;
            if attr == "type" && value == "List" {
                is_list = true;
            }
        }
    }

    fn read_name(&mut self) -> Result<String, Fail> {
        let mut name = String::new();
        while let Some(ch) = self.rest().chars().next() {
            if is_name_char(ch) || (name.is_empty() && is_name_start(ch)) {
                name.push(ch);
                self.bump();
            } else {
                break;
            }
        }
        Ok(name)
    }

    fn read_attr(&mut self) -> Result<String, Fail> {
        let quote = self.bump().ok_or(Fail::Bad)?;
        if quote != '"' && quote != '\'' {
            return Err(Fail::Bad);
        }
        let mut value = String::new();
        while let Some(ch) = self.bump() {
            if ch == quote {
                return Ok(value);
            }
            if ch == '&' {
                value.push(self.read_entity()?);
            } else {
                value.push(ch);
            }
        }
        Err(Fail::Bad)
    }

    fn read_text(&mut self) -> Result<String, Fail> {
        let mut text = String::new();
        while !self.eof() {
            if self.starts_with("</") {
                return Ok(normalize_newlines(&text));
            }
            if self.starts_with("<![CDATA[") {
                self.byte += 9;
                while !self.eof() {
                    if self.starts_with("]]>") {
                        self.byte += 3;
                        break;
                    }
                    text.push(self.bump().ok_or(Fail::Bad)?);
                }
                continue;
            }
            if self.starts_with("<!--") {
                self.skip_comment()?;
                continue;
            }
            if self.starts_with("<") {
                return Err(Fail::Bad);
            }
            let ch = self.bump().ok_or(Fail::Bad)?;
            if ch == '&' {
                text.push(self.read_entity()?);
            } else {
                text.push(ch);
            }
        }
        Err(Fail::Bad)
    }

    fn read_entity(&mut self) -> Result<char, Fail> {
        let start = self.rest();
        let Some(end) = start.find(';') else {
            return Err(Fail::Bad);
        };
        let body = &start[..end];
        self.byte += end + 1;
        let ch = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
            char::from_u32(u32::from_str_radix(hex, 16).map_err(|_| Fail::Bad)?).ok_or(Fail::Bad)?
        } else if let Some(dec) = body.strip_prefix('#') {
            char::from_u32(dec.parse().map_err(|_| Fail::Bad)?).ok_or(Fail::Bad)?
        } else {
            match body {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => return Err(Fail::Bad),
            }
        };
        if !is_xml_char(ch) {
            return Err(Fail::Bad);
        }
        Ok(ch)
    }

    fn expect_end(&mut self, name: &str) -> Result<(), Fail> {
        self.skip_ws();
        let closing = format!("</{name}>");
        if self.starts_with(&closing) {
            self.byte += closing.len();
            return Ok(());
        }
        // Whitespace before '>' is accepted by XmlReader.
        let marked = format!("</{name}");
        if self.starts_with(&marked) {
            self.byte += marked.len();
            self.skip_ws();
            if self.bump() == Some('>') {
                return Ok(());
            }
        }
        Err(Fail::Bad)
    }
}

fn normalize_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}
