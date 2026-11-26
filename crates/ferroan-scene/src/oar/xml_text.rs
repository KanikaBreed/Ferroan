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
//! when it is the first bytes of the document. CDATA is character data.
//! Character references and the five predefined entities are decoded. A second
//! root, a child element inside a content read, a truncated document, an
//! end-tag name that does not match, non-whitespace text at the root, an
//! undeclared entity, a duplicate attribute, a late XML declaration and an
//! undeclared prefix produce Pin8's exception text, including the line and the
//! position. The caller's loop reads the next element. It does not skip the
//! element that follows a content read.

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
        loop {
            self.skip_space();
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
                    let key_line = self.line;
                    let key_pos = self.col;
                    let key = self.read_name()?;
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
        let start = self.index;
        while self.peek().is_some_and(is_name) {
            self.bump();
        }
        if start == self.index {
            return Err(self.eof_error());
        }
        Ok(String::from_utf8_lossy(&self.src[start..self.index]).into_owned())
    }

    fn read_text(&mut self) -> Result<String, String> {
        let mut out = String::new();
        while self.peek().is_some_and(|byte| byte != b'<') {
            if self.peek() == Some(b'&') {
                out.push(self.read_reference()?);
            } else if let Some(byte) = self.bump() {
                out.push(byte as char);
            }
        }
        Ok(out)
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
                Some(_) => {
                    if let Some(byte) = self.bump() {
                        out.push(byte as char);
                    }
                }
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
        let mut value: u32 = 0;
        let mut digits = 0u32;
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.numeric_syntax(hex));
            };
            let digit = if hex {
                hex_digit(byte)
            } else if byte.is_ascii_digit() {
                Some(u32::from(byte - b'0'))
            } else {
                None
            };
            let Some(digit) = digit else {
                if digits == 0 || byte != b';' {
                    return Err(self.numeric_syntax(hex));
                }
                break;
            };
            self.bump();
            value = value
                .saturating_mul(if hex { 16 } else { 10 })
                .saturating_add(digit);
            digits += 1;
            if digits > 6 {
                return Err(self.numeric_syntax(hex));
            }
        }
        self.bump();
        char::from_u32(value).ok_or_else(|| {
            format!(
                "'{value:X}', hexadecimal value {value:X}, is an invalid character. Line {}, \
                 position {}.",
                self.line, self.col
            )
        })
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
            self.skip_until(b"-->")?;
            return Ok(None);
        }
        if self.starts_with(b"[CDATA[") {
            return self.read_cdata();
        }
        self.skip_until(b">")?;
        Ok(None)
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
        self.skip_until(b"?>")
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
        format!(
            "Unexpected end of file has occurred. The following elements are not closed: {names}. \
             Line {}, position {}.",
            self.line, self.col
        )
    }
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
