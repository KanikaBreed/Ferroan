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
// Ported to Rust for Ferroan from the `XmlTextReader` calls in
//   Source/OpenSim.Region.CoreModules/World/Archiver/ArchiveReadRequest.cs
//     (LoadControlFile)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A lazy XML cursor for the OAR control file, modelled on `XmlTextReader`.
//!
//! `LoadControlFile` walks `Read` and `ReadElementContentAsString`. Comments
//! and processing instructions are skipped. The XML declaration is skipped only
//! when it is the first bytes of the document, and only version 1.0 is accepted.
//! CDATA is character data. Character references and the five predefined
//! entities are decoded. A surrogate reference is stored as U+FFFD, which is
//! the id the fixture records for `&#xD800;`. The caller's loop reads the next
//! element. It does not skip the element that follows a content read.
//!
//! The fixture is what this reader is checked against. A missing root, a
//! character reference whose value fits in a code point, a DOCTYPE whose
//! internal subset is skipped, a DOCTYPE inside the root, a name's first
//! character, whitespace between attributes, `]]>` in text, `--` in a comment
//! and a raw character below U+0020 other than tab, LF and CR follow that
//! fixture. Other malformed input, a bare CR counted as a line end, and an
//! entity left in an attribute value do not (D146).

/// One start tag, including an empty element.
pub(super) struct StartEl {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub empty: bool,
    line: i32,
    pos: i32,
}

struct OpenEl {
    name: String,
    line: i32,
    pos: i32,
    prefixes: Vec<String>,
}

struct Attr {
    key: String,
    value: String,
    line: i32,
    pos: i32,
}

enum Node {
    Start(StartEl),
    End(String),
    Text(String),
}

pub(super) struct XmlText {
    src: Vec<u8>,
    index: usize,
    line: i32,
    col: i32,
    stack: Vec<OpenEl>,
    seen_root: bool,
}

impl XmlText {
    pub(super) fn new(src: Vec<u8>) -> Self {
        Self {
            src,
            index: 0,
            line: 1,
            col: 1,
            stack: Vec::new(),
            seen_root: false,
        }
    }

    /// The next element start. Whitespace, comments, text and end tags are skipped.
    pub(super) fn next_element(&mut self) -> Result<Option<StartEl>, String> {
        loop {
            match self.read_node()? {
                None => return Ok(None),
                Some(Node::Start(element)) => return Ok(Some(element)),
                Some(Node::End(_) | Node::Text(_)) => {}
            }
        }
    }

    /// `ReadElementContentAsString` for the element `next_element` just returned.
    pub(super) fn read_content(&mut self, element: &StartEl) -> Result<String, String> {
        if element.empty {
            return Ok(String::new());
        }
        let mut text = String::new();
        loop {
            match self.read_node()? {
                Some(Node::Text(value)) => text.push_str(&value),
                Some(Node::End(name)) if name == element.name => return Ok(text),
                Some(Node::Start(child)) => {
                    return Err(format!(
                        "ReadElementContentAs() methods cannot be called on an element that has \
                         child elements. Line {}, position {}.",
                        child.line, child.pos
                    ));
                }
                Some(Node::End(_)) | None => return Err(self.eof_error()),
            }
        }
    }

    fn read_node(&mut self) -> Result<Option<Node>, String> {
        loop {
            if self.index >= self.src.len() {
                if self.stack.is_empty() {
                    if !self.seen_root {
                        return Err("Root element is missing.".to_string());
                    }
                    return Ok(None);
                }
                return Err(self.eof_error());
            }
            let Some(byte) = self.peek() else {
                return Err(self.eof_error());
            };
            if byte != b'<' {
                if self.stack.is_empty() {
                    if is_space(byte) {
                        self.bump();
                        continue;
                    }
                    return Err(format!(
                        "Data at the root level is invalid. Line {}, position {}.",
                        self.line, self.col
                    ));
                }
                return Ok(Some(Node::Text(self.read_text()?)));
            }
            let at_start = self.index == 0;
            if let Some(node) = self.parse_markup(at_start)? {
                return Ok(Some(node));
            }
        }
    }

    fn parse_markup(&mut self, at_start: bool) -> Result<Option<Node>, String> {
        self.bump();
        match self.peek() {
            Some(b'/') => {
                self.bump();
                self.parse_end()
            }
            Some(b'!') => self.read_bang(),
            Some(b'?') => {
                self.parse_pi(at_start)?;
                Ok(None)
            }
            Some(_) => self.parse_start(),
            None => Err(self.eof_error()),
        }
    }

    fn parse_start(&mut self) -> Result<Option<Node>, String> {
        let line = self.line;
        let pos = self.col;
        if self.stack.is_empty() && self.seen_root {
            return Err(format!(
                "There are multiple root elements. Line {line}, position {pos}."
            ));
        }
        let name = self.read_name()?;
        let (attrs, prefixes, empty) = self.parse_attrs()?;
        let at_root = self.stack.is_empty();
        self.stack.push(OpenEl {
            name: name.clone(),
            line,
            pos,
            prefixes,
        });
        if at_root {
            self.seen_root = true;
        }
        self.prefix_ok(&name, line, pos)?;
        for attr in &attrs {
            self.prefix_ok(&attr.key, attr.line, attr.pos)?;
        }
        if empty {
            self.stack.pop();
        }
        Ok(Some(Node::Start(StartEl {
            name,
            attrs: attrs
                .into_iter()
                .map(|attr| (attr.key, attr.value))
                .collect(),
            empty,
            line,
            pos,
        })))
    }

    fn parse_end(&mut self) -> Result<Option<Node>, String> {
        let line = self.line;
        let pos = self.col;
        let name = self.read_name()?;
        self.skip_space();
        if self.peek() != Some(b'>') {
            return Err(self.eof_error());
        }
        self.bump();
        self.prefix_ok(&name, line, pos)?;
        let Some(open) = self.stack.pop() else {
            return Err(format!("Unexpected end tag. Line {line}, position {pos}."));
        };
        if open.name != name {
            return Err(format!(
                "The '{}' start tag on line {} position {} does not match the end tag of \
                 '{name}'. Line {line}, position {pos}.",
                open.name, open.line, open.pos
            ));
        }
        Ok(Some(Node::End(name)))
    }

    fn parse_attrs(&mut self) -> Result<(Vec<Attr>, Vec<String>, bool), String> {
        let mut attrs = Vec::new();
        let mut prefixes = Vec::new();
        let mut saw_attr = false;
        loop {
            let before = self.index;
            self.skip_space();
            let spaced = self.index != before;
            match self.peek() {
                None => return Err(self.eof_error()),
                Some(b'>') => {
                    self.bump();
                    return Ok((attrs, prefixes, false));
                }
                Some(b'/') => {
                    self.bump();
                    if self.peek() != Some(b'>') {
                        return Err(self.eof_error());
                    }
                    self.bump();
                    return Ok((attrs, prefixes, true));
                }
                Some(_) => {
                    if saw_attr && !spaced {
                        let line = self.line;
                        let pos = self.col;
                        let key = self.read_name()?;
                        return Err(format!(
                            "'{key}' is an unexpected token. Expecting whitespace. Line {line}, \
                             position {pos}."
                        ));
                    }
                    let key_line = self.line;
                    let key_pos = self.col;
                    let key = self.read_name()?;
                    saw_attr = true;
                    if attrs.iter().any(|attr| attr.key == key) {
                        return Err(format!(
                            "'{key}' is a duplicate attribute name. Line {key_line}, position \
                             {key_pos}."
                        ));
                    }
                    if let Some(prefix) = key.strip_prefix("xmlns:")
                        && !prefix.is_empty()
                    {
                        prefixes.push(prefix.to_string());
                    }
                    self.skip_space();
                    if self.peek() != Some(b'=') {
                        return Err(self.eof_error());
                    }
                    self.bump();
                    self.skip_space();
                    let value = self.read_quoted()?;
                    attrs.push(Attr {
                        key,
                        value,
                        line: key_line,
                        pos: key_pos,
                    });
                }
            }
        }
    }

    /// An element's own `xmlns:` declarations are already on the stack.
    fn prefix_ok(&self, name: &str, line: i32, pos: i32) -> Result<(), String> {
        let Some((prefix, local)) = name.split_once(':') else {
            return Ok(());
        };
        if prefix.is_empty() || local.is_empty() {
            return Err(format!(
                "'{prefix}' is an undeclared prefix. Line {line}, position {pos}."
            ));
        }
        if prefix == "xml"
            || prefix == "xmlns"
            || self
                .stack
                .iter()
                .any(|open| open.prefixes.iter().any(|declared| declared == prefix))
        {
            return Ok(());
        }
        Err(format!(
            "'{prefix}' is an undeclared prefix. Line {line}, position {pos}."
        ))
    }

    fn read_name(&mut self) -> Result<String, String> {
        let line = self.line;
        let pos = self.col;
        let Some(first) = self.peek() else {
            return Err(self.eof_error());
        };
        if !is_name_start(first) {
            if is_name(first) {
                let shown = first as char;
                return Err(format!(
                    "Name cannot begin with the '{shown}' character, hexadecimal value \
                     0x{first:X}. Line {line}, position {pos}."
                ));
            }
            return Err(self.eof_error());
        }
        let start = self.index;
        while self.peek().is_some_and(is_name) {
            self.bump();
        }
        Ok(String::from_utf8_lossy(&self.src[start..self.index]).into_owned())
    }

    fn read_text(&mut self) -> Result<String, String> {
        let mut out = String::new();
        while self.peek().is_some_and(|byte| byte != b'<') {
            if self.starts_with(b"]]>") {
                return Err(format!(
                    "']]>' is not allowed in character data. Line {}, position {}.",
                    self.line, self.col
                ));
            }
            if self.peek() == Some(b'&') {
                out.push(self.read_reference()?);
            } else {
                out.push(self.read_raw()?);
            }
        }
        Ok(out)
    }

    /// One raw character. Tab, LF and CR are the only bytes below U+0020.
    fn read_raw(&mut self) -> Result<char, String> {
        let line = self.line;
        let pos = self.col;
        let Some(byte) = self.bump() else {
            return Err(self.eof_error());
        };
        if let Some(error) = raw_char_error(byte, line, pos) {
            return Err(error);
        }
        Ok(byte as char)
    }

    fn read_quoted(&mut self) -> Result<String, String> {
        let quote = self.peek().ok_or_else(|| self.eof_error())?;
        if quote != b'"' && quote != b'\'' {
            return Err(self.eof_error());
        }
        self.bump();
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error()),
                Some(byte) if byte == quote => {
                    self.bump();
                    return Ok(out);
                }
                Some(b'&') => out.push(self.read_reference()?),
                Some(_) => out.push(self.read_raw()?),
            }
        }
    }

    fn read_reference(&mut self) -> Result<char, String> {
        self.bump();
        if self.peek() == Some(b'#') {
            self.bump();
            return self.read_char_ref();
        }
        let line = self.line;
        let pos = self.col;
        if !self.peek().is_some_and(is_name_start) {
            return Err(format!(
                "An error occurred while parsing EntityName. Line {line}, position {pos}."
            ));
        }
        let name = self.read_name()?;
        if self.peek() != Some(b';') {
            return Err(format!(
                "An error occurred while parsing EntityName. Line {line}, position {pos}."
            ));
        }
        self.bump();
        match name.as_str() {
            "amp" => Ok('&'),
            "lt" => Ok('<'),
            "gt" => Ok('>'),
            "quot" => Ok('"'),
            "apos" => Ok('\''),
            _ => Err(format!(
                "Reference to undeclared entity '{name}'. Line {line}, position {pos}."
            )),
        }
    }

    fn read_char_ref(&mut self) -> Result<char, String> {
        let hex = matches!(self.peek(), Some(b'x' | b'X'));
        if hex {
            self.bump();
        }
        let radix: u64 = if hex { 16 } else { 10 };
        let mut value: u64 = 0;
        let mut digits = 0u32;
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.numeric_syntax(hex));
            };
            let digit = if hex {
                hex_digit(byte).map(u64::from)
            } else if byte.is_ascii_digit() {
                Some(u64::from(byte - b'0'))
            } else {
                None
            };
            let Some(digit) = digit else {
                if digits == 0 || byte != b';' {
                    return Err(self.numeric_syntax(hex));
                }
                break;
            };
            let Some(next) = value
                .checked_mul(radix)
                .and_then(|next| next.checked_add(digit))
            else {
                return Err(self.numeric_syntax(hex));
            };
            if next > 0x10_FFFF {
                return Err(self.numeric_syntax(hex));
            }
            self.bump();
            value = next;
            digits += 1;
        }
        self.bump();
        // Pin8's reader does not reject a surrogate. The recorded id is U+FFFD.
        if (0xD800..=0xDFFF).contains(&value) {
            return Ok('\u{FFFD}');
        }
        match u32::try_from(value).ok().and_then(char::from_u32) {
            Some(ch) => Ok(ch),
            None => Err(format!(
                "'{value:X}', hexadecimal value {value:X}, is an invalid character. Line {}, \
                 position {}.",
                self.line, self.col
            )),
        }
    }

    fn numeric_syntax(&self, hex: bool) -> String {
        let kind = if hex { "hexadecimal" } else { "decimal" };
        format!(
            "Invalid syntax for a {kind} numeric entity reference. Line {}, position {}.",
            self.line, self.col
        )
    }

    fn read_bang(&mut self) -> Result<Option<Node>, String> {
        self.bump();
        if self.starts_with(b"--") {
            self.bump();
            self.bump();
            return self.skip_comment();
        }
        if self.starts_with(b"[CDATA[") {
            return self.read_cdata();
        }
        let line = self.line;
        let pos = self.col;
        if self.starts_with(b"DOCTYPE") {
            if !self.stack.is_empty() {
                return Err(format!(
                    "Unexpected DTD declaration. Line {line}, position {pos}."
                ));
            }
            self.skip_doctype()?;
            return Ok(None);
        }
        self.skip_until(b">")?;
        Ok(None)
    }

    /// `DtdProcessing.Ignore` drops an internal subset, including a `>` inside it.
    fn skip_doctype(&mut self) -> Result<(), String> {
        let mut subset = false;
        while self.index < self.src.len() {
            if !subset && self.peek() == Some(b'[') {
                subset = true;
                self.bump();
                continue;
            }
            if subset && self.starts_with(b"]>") {
                self.bump();
                self.bump();
                return Ok(());
            }
            if !subset && self.peek() == Some(b'>') {
                self.bump();
                return Ok(());
            }
            self.bump();
        }
        Err(self.eof_error())
    }

    fn skip_comment(&mut self) -> Result<Option<Node>, String> {
        loop {
            if self.starts_with(b"-->") {
                self.bump();
                self.bump();
                self.bump();
                return Ok(None);
            }
            if self.starts_with(b"--") {
                return Err(format!(
                    "An XML comment cannot contain '--', and '-' cannot be the last character. \
                     Line {}, position {}.",
                    self.line, self.col
                ));
            }
            if self.bump().is_none() {
                return Err(self.eof_error());
            }
        }
    }

    fn read_cdata(&mut self) -> Result<Option<Node>, String> {
        for _ in 0..7 {
            self.bump();
        }
        let mut out = String::new();
        loop {
            if self.starts_with(b"]]>") {
                self.bump();
                self.bump();
                self.bump();
                break;
            }
            let line = self.line;
            let pos = self.col;
            let Some(byte) = self.bump() else {
                return Err(self.eof_error());
            };
            if let Some(error) = raw_char_error(byte, line, pos) {
                return Err(error);
            }
            if self.stack.is_empty() && !is_space(byte) {
                return Err(format!(
                    "Data at the root level is invalid. Line {line}, position {pos}."
                ));
            }
            if !self.stack.is_empty() {
                out.push(byte as char);
            }
        }
        if self.stack.is_empty() {
            return Ok(None);
        }
        Ok(Some(Node::Text(out)))
    }

    fn parse_pi(&mut self, at_start: bool) -> Result<(), String> {
        self.bump();
        let line = self.line;
        let pos = self.col;
        if !self.peek().is_some_and(is_name_start) {
            return Err(self.eof_error());
        }
        let target = self.read_name()?;
        if target.eq_ignore_ascii_case("xml") && target != "xml" {
            return Err(format!(
                "'{target}' is an invalid name for processing instructions. Line {line}, \
                 position {pos}."
            ));
        }
        if target == "xml" && !at_start {
            return Err(format!(
                "Unexpected XML declaration. The XML declaration must be the first node in the \
                 document, and no whitespace characters are allowed to appear before it. Line \
                 {line}, position {pos}."
            ));
        }
        if target == "xml" && at_start {
            self.check_version()?;
        }
        self.skip_until(b"?>")
    }

    /// `version` must be `1.0` when the declaration carries one.
    fn check_version(&mut self) -> Result<(), String> {
        self.skip_space();
        if !self.starts_with(b"version") {
            return Ok(());
        }
        for _ in 0..b"version".len() {
            self.bump();
        }
        self.skip_space();
        if self.peek() != Some(b'=') {
            return Ok(());
        }
        self.bump();
        self.skip_space();
        let Some(quote) = self.peek() else {
            return Err(self.eof_error());
        };
        if quote != b'"' && quote != b'\'' {
            return Ok(());
        }
        self.bump();
        let line = self.line;
        let pos = self.col;
        let mut value = String::new();
        loop {
            match self.peek() {
                None => return Err(self.eof_error()),
                Some(byte) if byte == quote => {
                    self.bump();
                    break;
                }
                Some(_) => value.push(self.read_raw()?),
            }
        }
        if value != "1.0" {
            return Err(format!(
                "Version number '{value}' is invalid. Line {line}, position {pos}."
            ));
        }
        Ok(())
    }

    fn skip_until(&mut self, marker: &[u8]) -> Result<(), String> {
        while self.index < self.src.len() {
            if self.starts_with(marker) {
                for _ in marker {
                    self.bump();
                }
                return Ok(());
            }
            self.bump();
        }
        Err(self.eof_error())
    }

    fn skip_space(&mut self) {
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.bump();
        }
    }

    fn starts_with(&self, marker: &[u8]) -> bool {
        self.src
            .get(self.index..)
            .is_some_and(|rest| rest.starts_with(marker))
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.index).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.index += 1;
        if byte == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(byte)
    }

    fn eof_error(&self) -> String {
        let names = self
            .stack
            .iter()
            .rev()
            .map(|open| open.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if names.is_empty() {
            return format!(
                "Unexpected end of file has occurred. Line {}, position {}.",
                self.line, self.col
            );
        }
        format!(
            "Unexpected end of file has occurred. The following elements are not closed: {names}. \
             Line {}, position {}.",
            self.line, self.col
        )
    }
}

fn raw_char_error(byte: u8, line: i32, pos: i32) -> Option<String> {
    if byte >= 0x20 || matches!(byte, b'\t' | b'\n' | b'\r') {
        return None;
    }
    let shown = if byte == 0 {
        ".".to_string()
    } else {
        (byte as char).to_string()
    };
    Some(format!(
        "'{shown}', hexadecimal value 0x{byte:02X}, is an invalid character. Line {line}, \
         position {pos}."
    ))
}

fn is_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.')
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':')
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn hex_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'f' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'F' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single() -> String {
        "<?xml version=\"1.0\" encoding=\"utf-16\"?>\n\
         <archive major_version=\"0\" minor_version=\"8\">\n  \
         <creation_info>\n    \
         <datetime>1700000000</datetime>\n    \
         <id>11111111-1111-1111-1111-111111111111</id>\n  \
         </creation_info>\n  \
         <assets_included>True</assets_included>\n  \
         <region_info>\n    \
         <size_in_meters>256,256</size_in_meters>\n  \
         </region_info>\n\
         </archive>"
            .to_string()
    }

    fn drive(xml: &str) -> Result<Vec<String>, String> {
        let mut reader = XmlText::new(xml.as_bytes().to_vec());
        let mut seen = Vec::new();
        while let Some(element) = reader.next_element()? {
            if matches!(
                element.name.as_str(),
                "datetime" | "id" | "dir" | "size_in_meters" | "assets_included"
            ) {
                let text = reader.read_content(&element)?;
                seen.push(format!("{}={text}", element.name));
            }
        }
        Ok(seen)
    }

    #[test]
    fn child_element_id_matches_pin8() {
        let xml = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id><b/>abc</id>",
        );
        let error = drive(&xml).unwrap_err();
        assert_eq!(
            error,
            "ReadElementContentAs() methods cannot be called on an element that has child \
             elements. Line 5, position 10."
        );
    }

    #[test]
    fn a_second_root_matches_pin8() {
        let error = drive(&format!("{}<extra/>", single())).unwrap_err();
        assert_eq!(
            error,
            "There are multiple root elements. Line 11, position 12."
        );
    }

    #[test]
    fn a_truncated_control_file_matches_pin8() {
        let xml = single();
        let cut = &xml[..xml.len() - 12];
        let error = drive(cut).unwrap_err();
        assert_eq!(
            error,
            "Unexpected end of file has occurred. The following elements are not closed: \
             region_info, archive. Line 10, position 16."
        );
    }

    #[test]
    fn a_comment_between_elements_leaves_the_id() {
        let xml = compact(&single()).replace("</datetime>", "</datetime><!--c-->");
        let seen = drive(&xml).unwrap();
        assert!(seen.iter().any(|row| row.starts_with("id=11111111")));
    }

    #[test]
    fn an_end_tag_mismatch_matches_pin8() {
        let xml = single().replace("</region_info>", "</regionx>");
        assert_eq!(
            drive(&xml).unwrap_err(),
            "The 'region_info' start tag on line 8 position 4 does not match the end tag of \
             'regionx'. Line 10, position 5."
        );
    }

    #[test]
    fn text_at_the_root_matches_pin8() {
        assert_eq!(
            drive(&format!("{}junk", single())).unwrap_err(),
            "Data at the root level is invalid. Line 11, position 11."
        );
        let leading = single().replacen("<?xml version=\"1.0\" encoding=\"utf-16\"?>\n", "junk", 1);
        assert_eq!(
            drive(&leading).unwrap_err(),
            "Data at the root level is invalid. Line 1, position 1."
        );
        // `ascii_bytes` turns a UTF-8 BOM into three question marks before this reader.
        assert_eq!(
            drive(&format!("???{}", single())).unwrap_err(),
            "Data at the root level is invalid. Line 1, position 1."
        );
    }

    #[test]
    fn an_undeclared_entity_matches_pin8() {
        let xml = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>&foo;</id>",
        );
        assert_eq!(
            drive(&xml).unwrap_err(),
            "Reference to undeclared entity 'foo'. Line 5, position 10."
        );
    }

    #[test]
    fn cdata_and_a_character_reference_are_the_region_id() {
        let cdata = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id><![CDATA[11111111-1111-1111-1111-111111111111]]></id>",
        );
        assert!(
            drive(&cdata)
                .unwrap()
                .iter()
                .any(|row| row == "id=11111111-1111-1111-1111-111111111111")
        );
        let chars = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>&#50;1111111-1111-1111-1111-111111111111</id>",
        );
        assert!(
            drive(&chars)
                .unwrap()
                .iter()
                .any(|row| row == "id=21111111-1111-1111-1111-111111111111")
        );
        let hex = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>&#x32;1111111-1111-1111-1111-111111111111</id>",
        );
        assert!(
            drive(&hex)
                .unwrap()
                .iter()
                .any(|row| row == "id=21111111-1111-1111-1111-111111111111")
        );
    }

    #[test]
    fn a_comment_inside_content_is_skipped() {
        let xml = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>1111<!--c-->1111-1111-1111-1111-111111111111</id>",
        );
        assert!(
            drive(&xml)
                .unwrap()
                .iter()
                .any(|row| row == "id=11111111-1111-1111-1111-111111111111")
        );
    }

    #[test]
    fn a_duplicate_attribute_matches_pin8() {
        let xml = single().replace(
            "<archive major_version=\"0\" minor_version=\"8\">",
            "<archive major_version=\"0\" major_version=\"2\" minor_version=\"8\">",
        );
        assert_eq!(
            drive(&xml).unwrap_err(),
            "'major_version' is a duplicate attribute name. Line 2, position 28."
        );
    }

    #[test]
    fn a_late_declaration_matches_pin8() {
        let xml = single().replace(
            "  <creation_info>",
            "  <?xml version=\"1.0\"?><creation_info>",
        );
        assert_eq!(
            drive(&xml).unwrap_err(),
            "Unexpected XML declaration. The XML declaration must be the first node in the \
             document, and no whitespace characters are allowed to appear before it. Line 3, \
             position 5."
        );
    }

    #[test]
    fn an_undeclared_prefix_matches_pin8() {
        let xml = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<a:id>x</a:id><id>11111111-1111-1111-1111-111111111111</id>",
        );
        assert_eq!(
            drive(&xml).unwrap_err(),
            "'a' is an undeclared prefix. Line 5, position 6."
        );
        let declared = "<a:archive xmlns:a=\"u\" major_version=\"0\" minor_version=\"8\"/>";
        let mut reader = XmlText::new(declared.as_bytes().to_vec());
        let element = reader.next_element().unwrap().unwrap();
        assert_eq!(element.name, "a:archive");
    }

    #[test]
    fn a_missing_root_matches_pin8() {
        assert_eq!(drive("").unwrap_err(), "Root element is missing.");
        assert_eq!(
            drive("<?xml version=\"1.0\" encoding=\"utf-16\"?>").unwrap_err(),
            "Root element is missing."
        );
        assert_eq!(drive("<!-- c -->").unwrap_err(), "Root element is missing.");
    }

    #[test]
    fn a_seven_digit_reference_and_a_surrogate_load() {
        let digits = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>&#0000050;1111111-1111-1111-1111-111111111111</id>",
        );
        assert!(
            drive(&digits)
                .unwrap()
                .iter()
                .any(|row| row == "id=21111111-1111-1111-1111-111111111111")
        );
        let surrogate = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>&#xD800;</id>",
        );
        assert!(
            drive(&surrogate)
                .unwrap()
                .iter()
                .any(|row| row == "id=\u{FFFD}")
        );
        let zero = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>&#0;</id>",
        );
        assert!(drive(&zero).unwrap().iter().any(|row| row == "id=\u{0}"));
    }

    #[test]
    fn a_doctype_subset_is_skipped_and_one_inside_the_root_is_refused() {
        let body = single().replacen("<?xml version=\"1.0\" encoding=\"utf-16\"?>\n", "", 1);
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"utf-16\"?>\n<!DOCTYPE archive [<!ENTITY e \
             \"x\">]>\n{body}"
        );
        assert!(
            drive(&xml)
                .unwrap()
                .iter()
                .any(|row| row.starts_with("id=11111111"))
        );
        let inside = single().replace("<creation_info>", "<!DOCTYPE x><creation_info>");
        assert_eq!(
            drive(&inside).unwrap_err(),
            "Unexpected DTD declaration. Line 3, position 5."
        );
    }

    #[test]
    fn a_name_must_begin_with_a_name_start_character() {
        let digit = single().replace("<creation_info>", "<creation_info><1x/>");
        assert_eq!(
            drive(&digit).unwrap_err(),
            "Name cannot begin with the '1' character, hexadecimal value 0x31. Line 3, position \
             19."
        );
        let dash = single().replace("<creation_info>", "<creation_info><-x/>");
        assert_eq!(
            drive(&dash).unwrap_err(),
            "Name cannot begin with the '-' character, hexadecimal value 0x2D. Line 3, position \
             19."
        );
    }

    #[test]
    fn attributes_require_whitespace_between_them() {
        let xml = single().replace(
            "major_version=\"0\" minor_version",
            "major_version=\"0\"minor_version",
        );
        assert_eq!(
            drive(&xml).unwrap_err(),
            "'minor_version' is an unexpected token. Expecting whitespace. Line 2, position 27."
        );
    }

    #[test]
    fn markup_in_text_and_in_a_comment_matches_pin8() {
        let text = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>1111]]>1111-1111-1111-1111-111111111111</id>",
        );
        assert_eq!(
            drive(&text).unwrap_err(),
            "']]>' is not allowed in character data. Line 5, position 13."
        );
        let nested = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id><![CDATA[<![CDATA[x]]>]]></id>",
        );
        assert_eq!(
            drive(&nested).unwrap_err(),
            "']]>' is not allowed in character data. Line 5, position 31."
        );
        let comment = single().replace("<creation_info>", "<!-- a -- b --><creation_info>");
        assert_eq!(
            drive(&comment).unwrap_err(),
            "An XML comment cannot contain '--', and '-' cannot be the last character. Line 3, \
             position 10."
        );
    }

    #[test]
    fn a_raw_control_and_a_bad_version_match_pin8() {
        let nul = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>a\u{0}b</id>",
        );
        assert_eq!(
            drive(&nul).unwrap_err(),
            "'.', hexadecimal value 0x00, is an invalid character. Line 5, position 10."
        );
        let ctrl = single().replace(
            "<id>11111111-1111-1111-1111-111111111111</id>",
            "<id>a\u{1}b</id>",
        );
        assert_eq!(
            drive(&ctrl).unwrap_err(),
            "'\u{1}', hexadecimal value 0x01, is an invalid character. Line 5, position 10."
        );
        let version = single().replace("version=\"1.0\"", "version=\"2.0\"");
        assert_eq!(
            drive(&version).unwrap_err(),
            "Version number '2.0' is invalid. Line 1, position 16."
        );
    }

    #[test]
    fn eof_with_an_empty_stack_names_no_element() {
        let xml = single();
        let at = xml.find("minor_version=\"8").unwrap() + 16;
        let error = drive(&xml[..at]).unwrap_err();
        assert!(!error.contains("not closed"), "{error}");
        assert!(
            error.starts_with("Unexpected end of file has occurred. Line "),
            "{error}"
        );
    }

    fn compact(xml: &str) -> String {
        let bytes = xml.as_bytes();
        let mut out = String::new();
        let mut index = 0;
        while index < bytes.len() {
            out.push(bytes[index] as char);
            if bytes[index] == b'>' {
                let mut next = index + 1;
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                if next < bytes.len() && bytes[next] == b'<' {
                    index = next;
                    continue;
                }
            }
            index += 1;
        }
        out
    }
}
