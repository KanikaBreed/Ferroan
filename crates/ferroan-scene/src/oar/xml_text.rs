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

//! A lazy XML cursor for the OAR control file.
//!
//! `LoadControlFile` constructs an `XmlTextReader` over the ASCII text and
//! walks it with `Read`. Comments, processing instructions and the declaration
//! are nodes. A second root, a child element inside `ReadElementContentAsString`,
//! and a truncated document produce Pin8's exception text. The caller's loop
//! reads the next element. It does not skip the element that follows a content
//! read.

/// One start tag, including an empty element.
pub(super) struct StartEl {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub empty: bool,
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
    stack: Vec<String>,
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
            if self.peek() != Some(b'<') {
                return Ok(Some(Node::Text(self.read_text())));
            }
            if let Some(node) = self.parse_markup()? {
                return Ok(Some(node));
            }
        }
    }

    fn parse_markup(&mut self) -> Result<Option<Node>, String> {
        self.bump();
        match self.peek() {
            Some(b'/') => {
                self.bump();
                self.parse_end()
            }
            Some(b'!') => {
                self.skip_bang()?;
                Ok(None)
            }
            Some(b'?') => {
                self.skip_until(b"?>")?;
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
        let (attrs, empty) = self.parse_attrs()?;
        if self.stack.is_empty() {
            self.seen_root = true;
        }
        if !empty {
            self.stack.push(name.clone());
        }
        Ok(Some(Node::Start(StartEl {
            name,
            attrs,
            empty,
            line,
            pos,
        })))
    }

    fn parse_end(&mut self) -> Result<Option<Node>, String> {
        let name = self.read_name()?;
        self.skip_space();
        if self.peek() != Some(b'>') {
            return Err(self.eof_error());
        }
        self.bump();
        self.stack.pop();
        Ok(Some(Node::End(name)))
    }

    fn parse_attrs(&mut self) -> Result<(Vec<(String, String)>, bool), String> {
        let mut attrs = Vec::new();
        loop {
            self.skip_space();
            match self.peek() {
                None => return Err(self.eof_error()),
                Some(b'>') => {
                    self.bump();
                    return Ok((attrs, false));
                }
                Some(b'/') => {
                    self.bump();
                    if self.peek() != Some(b'>') {
                        return Err(self.eof_error());
                    }
                    self.bump();
                    return Ok((attrs, true));
                }
                Some(_) => {
                    let key = self.read_name()?;
                    self.skip_space();
                    if self.peek() != Some(b'=') {
                        return Err(self.eof_error());
                    }
                    self.bump();
                    self.skip_space();
                    let quote = self.peek().ok_or_else(|| self.eof_error())?;
                    if quote != b'"' && quote != b'\'' {
                        return Err(self.eof_error());
                    }
                    self.bump();
                    let start = self.index;
                    while self.peek().is_some_and(|byte| byte != quote) {
                        self.bump();
                    }
                    if self.peek().is_none() {
                        return Err(self.eof_error());
                    }
                    let raw = self.src[start..self.index].to_vec();
                    self.bump();
                    attrs.push((key, unescape(&raw)));
                }
            }
        }
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

    fn read_text(&mut self) -> String {
        let start = self.index;
        while self.peek().is_some_and(|byte| byte != b'<') {
            self.bump();
        }
        unescape(&self.src[start..self.index])
    }

    fn skip_bang(&mut self) -> Result<(), String> {
        self.bump();
        if self.starts_with(b"--") {
            self.bump();
            self.bump();
            self.skip_until(b"-->")?;
            return Ok(());
        }
        self.skip_until(b">")
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
            .cloned()
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

fn unescape(raw: &[u8]) -> String {
    if !raw.contains(&b'&') {
        return String::from_utf8_lossy(raw).into_owned();
    }
    let mut out = String::new();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] != b'&' {
            out.push(raw[index] as char);
            index += 1;
            continue;
        }
        let rest = &raw[index..];
        if let Some(name) = rest.strip_prefix(b"&amp;") {
            out.push('&');
            index = raw.len() - name.len();
        } else if let Some(name) = rest.strip_prefix(b"&lt;") {
            out.push('<');
            index = raw.len() - name.len();
        } else if let Some(name) = rest.strip_prefix(b"&gt;") {
            out.push('>');
            index = raw.len() - name.len();
        } else if let Some(name) = rest.strip_prefix(b"&quot;") {
            out.push('"');
            index = raw.len() - name.len();
        } else if let Some(name) = rest.strip_prefix(b"&apos;") {
            out.push('\'');
            index = raw.len() - name.len();
        } else {
            out.push('&');
            index += 1;
        }
    }
    out
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
