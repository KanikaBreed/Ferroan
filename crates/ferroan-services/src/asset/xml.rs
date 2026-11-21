// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `ServerUtils.SerializeResult` documents for asset HTTP bodies.
//!
//! Tranquillity writes these with `XmlSerializer` and `XmlTextWriter` at
//! `713857f10b8e91a639a967d4354a4682b080793c`. The bytes are the files under
//! `fixtures/asset`, produced by `cargo xtask asset`. On Linux that writer
//! emits a UTF-8 BOM, LF newlines, two-space indentation, and no newline
//! after the root end tag.
//!
//! `AssetBase` omits the get-only helpers and `[XmlIgnore] Metadata`.
//! `AssetMetadata` omits a null `SHA1`. A stored row has no hash column, and
//! `AssetService.GetMetadata` returns `asset.Metadata`, whose hash stays
//! null. `CreationDate` stays `DateTime.MinValue` because the SQLite row has
//! no creation column. Flag values are the `[Flags]` names (`Normal`,
//! `Maptile Rewritable`). `StoredAssetFlags.FromStored` keeps only
//! `Maptile | Rewritable | Collectable` before that text is chosen, so a
//! stored 8 is `Normal` and a stored 9 is `Maptile`. `ContentType` is derived
//! from `Type`.

use ferroan_diag::{
    self, ASSET_XML_DEPTH, ASSET_XML_ELEMENTS, ASSET_XML_NAME, HTTP_XML_CHARS, Kind,
};
use ferroan_http::MAX_XML_CHARS;
use uuid::Uuid;

use super::content_type::{asset_type_for_content, content_type};
use super::model::{Asset, parse_id};

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const DECL: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n";
const NS: &str = " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"";
const CREATION_MIN: &str = "0001-01-01T00:00:00";
const MAX_DEPTH: usize = 32;
const MAX_ELEMENTS: usize = 100_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum XmlError {
    #[error("asset xml is larger than {MAX_XML_CHARS} bytes")]
    Limit,
    #[error("asset xml is not utf-8")]
    Encoding,
    #[error("asset xml is not a document")]
    Syntax,
    #[error("asset xml element was not expected")]
    Shape,
}

/// An `AssetBase` document. `data_nil` is a missing or `xsi:nil` `Data`
/// element. The handlers answer 400 for that document: storing a NULL blob
/// makes the later get throw in C#, and a region connector always sends bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedAsset {
    pub asset: Asset,
    pub id_text: String,
    pub data_nil: bool,
}

pub fn encode_asset(asset: &Asset) -> Vec<u8> {
    let mut inner = String::new();
    inner.push_str(&elem(1, "Data", &encode_base64(&asset.data)));
    inner.push_str(&full_id(asset));
    inner.push_str(&elem(1, "ID", &asset.id.to_string()));
    inner.push_str(&elem(1, "Name", &asset.name));
    inner.push_str(&elem(1, "Description", &asset.description));
    inner.push_str(&elem(1, "Type", &asset.type_code.to_string()));
    inner.push_str(&elem(
        1,
        "UploadAttempts",
        &asset.upload_attempts.to_string(),
    ));
    inner.push_str(&elem(1, "Local", bool_word(asset.local)));
    inner.push_str(&elem(1, "Temporary", bool_word(asset.temporary)));
    inner.push_str(&elem(1, "CreatorID", &asset.creator_id));
    inner.push_str(&elem(1, "Flags", &flags_text(asset.flags)));
    document("AssetBase", true, &inner)
}

pub fn encode_metadata(asset: &Asset) -> Vec<u8> {
    let mut inner = String::new();
    inner.push_str(&full_id(asset));
    inner.push_str(&elem(1, "ID", &asset.id.to_string()));
    inner.push_str(&elem(1, "Name", &asset.name));
    inner.push_str(&elem(1, "Description", &asset.description));
    inner.push_str(&elem(1, "CreationDate", CREATION_MIN));
    inner.push_str(&elem(1, "Type", &asset.type_code.to_string()));
    inner.push_str(&elem(1, "ContentType", content_type(asset.type_code)));
    inner.push_str(&elem(1, "Local", bool_word(asset.local)));
    inner.push_str(&elem(1, "Temporary", bool_word(asset.temporary)));
    inner.push_str(&elem(1, "CreatorID", &asset.creator_id));
    inner.push_str(&elem(1, "Flags", &flags_text(asset.flags)));
    document("AssetMetadata", true, &inner)
}

pub fn encode_bool(value: bool) -> Vec<u8> {
    primitive(&format!("<boolean>{}</boolean>", bool_word(value)))
}

pub fn encode_string(value: &str) -> Vec<u8> {
    primitive(&format!("<string>{}</string>", escape(value)))
}

pub fn encode_bools(values: &[bool]) -> Vec<u8> {
    let mut inner = String::new();
    for value in values {
        inner.push_str(&elem(1, "boolean", bool_word(*value)));
    }
    document("ArrayOfBoolean", true, &inner)
}

pub fn encode_strings(values: &[&str]) -> Vec<u8> {
    let mut inner = String::new();
    for value in values {
        inner.push_str(&elem(1, "string", value));
    }
    document("ArrayOfString", true, &inner)
}

pub fn decode_asset(bytes: &[u8]) -> Result<DecodedAsset, XmlError> {
    let root = parse_document(bytes)?;
    if root.name != "AssetBase" || root.nil {
        return Err(XmlError::Shape);
    }
    let mut asset = Asset {
        id: Uuid::nil(),
        name: String::new(),
        description: String::new(),
        type_code: -1,
        local: false,
        temporary: false,
        creator_id: String::new(),
        flags: 0,
        data: Vec::new(),
        upload_attempts: 0,
    };
    let mut id_text = Uuid::nil().to_string();
    let mut data_nil = true;
    for child in &root.children {
        match child.name.as_str() {
            "Data" => {
                if child.nil {
                    data_nil = true;
                    asset.data.clear();
                } else {
                    asset.data = decode_base64(&child.text).map_err(|_| XmlError::Shape)?;
                    data_nil = false;
                }
            }
            "FullID" => {
                if let Some(guid) = child.child("Guid") {
                    let id = parse_id(&guid.text).ok_or(XmlError::Shape)?;
                    asset.id = id;
                    id_text = id.to_string();
                }
            }
            "ID" => apply_id(&child.text, &mut asset, &mut id_text)?,
            "Name" => asset.name.clone_from(&child.text),
            "Description" => asset.description.clone_from(&child.text),
            "Type" => asset.type_code = parse_i8(&child.text)?,
            "UploadAttempts" => asset.upload_attempts = parse_i32(&child.text)?,
            "Local" => asset.local = parse_bool(&child.text)?,
            "Temporary" => asset.temporary = parse_bool(&child.text)?,
            "CreatorID" => asset.creator_id.clone_from(&child.text),
            "Flags" => asset.flags = parse_flags(&child.text)?,
            _ => {}
        }
    }
    Ok(DecodedAsset {
        asset,
        id_text,
        data_nil,
    })
}

pub fn decode_metadata_type(bytes: &[u8]) -> Result<(Uuid, i8, String), XmlError> {
    let root = parse_document(bytes)?;
    if root.name != "AssetMetadata" {
        return Err(XmlError::Shape);
    }
    let mut id = Uuid::nil();
    let mut id_text = id.to_string();
    let mut type_code = -1i8;
    let mut asset = Asset {
        id,
        name: String::new(),
        description: String::new(),
        type_code,
        local: false,
        temporary: false,
        creator_id: String::new(),
        flags: 0,
        data: Vec::new(),
        upload_attempts: 0,
    };
    for child in &root.children {
        match child.name.as_str() {
            "FullID" => {
                if let Some(guid) = child.child("Guid") {
                    id = parse_id(&guid.text).ok_or(XmlError::Shape)?;
                    asset.id = id;
                    id_text = id.to_string();
                }
            }
            "ID" => apply_id(&child.text, &mut asset, &mut id_text)?,
            "Type" => type_code = parse_i8(&child.text)?,
            "ContentType" => {
                if let Some(mapped) = asset_type_for_content(child.text.trim()) {
                    type_code = mapped;
                }
            }
            _ => {}
        }
    }
    Ok((asset.id, type_code, id_text))
}

pub fn decode_strings(bytes: &[u8]) -> Result<Vec<String>, XmlError> {
    let root = parse_document(bytes)?;
    if root.name != "ArrayOfString" {
        return Err(XmlError::Shape);
    }
    let mut ids = Vec::new();
    for child in &root.children {
        if child.name != "string" {
            return Err(XmlError::Shape);
        }
        ids.push(child.text.clone());
    }
    Ok(ids)
}

fn apply_id(text: &str, asset: &mut Asset, id_text: &mut String) -> Result<(), XmlError> {
    if let Some(id) = parse_id(text) {
        asset.id = id;
        *id_text = id.to_string();
    } else {
        *id_text = text.to_string();
    }
    Ok(())
}

fn full_id(asset: &Asset) -> String {
    format!("  <FullID>\n    <Guid>{}</Guid>\n  </FullID>\n", asset.id)
}

fn elem(indent: usize, name: &str, text: &str) -> String {
    let pad = "  ".repeat(indent);
    if text.is_empty() {
        format!("{pad}<{name} />\n")
    } else {
        format!("{pad}<{name}>{}</{name}>\n", escape(text))
    }
}

fn document(name: &str, namespaced: bool, inner: &str) -> Vec<u8> {
    let mut text = String::from(DECL);
    text.push('<');
    text.push_str(name);
    if namespaced {
        text.push_str(NS);
    }
    if inner.is_empty() {
        text.push_str(" />");
    } else {
        text.push_str(">\n");
        text.push_str(inner);
        text.push_str("</");
        text.push_str(name);
        text.push('>');
    }
    with_bom(&text)
}

fn primitive(body: &str) -> Vec<u8> {
    let mut text = String::from(DECL);
    text.push_str(body);
    with_bom(&text)
}

fn with_bom(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(BOM.len() + text.len());
    out.extend_from_slice(&BOM);
    out.extend_from_slice(text.as_bytes());
    out
}

fn bool_word(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Named bits of `AssetFlags`. `StoredAssetFlags.Known` is the same mask.
const KNOWN_ASSET_FLAGS: i32 = 1 | 2 | 4;

fn flags_text(flags: i32) -> String {
    // StoredAssetFlags.FromStored drops bits no version names.
    let flags = flags & KNOWN_ASSET_FLAGS;
    if flags == 0 {
        return "Normal".to_string();
    }
    const NAMED: [(i32, &str); 3] = [(1, "Maptile"), (2, "Rewritable"), (4, "Collectable")];
    let mut left = flags;
    let mut parts = Vec::new();
    for (bit, name) in NAMED {
        if flags & bit == bit {
            parts.push(name);
            left &= !bit;
        }
    }
    if left != 0 || parts.is_empty() {
        return flags.to_string();
    }
    parts.join(" ")
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

fn parse_bool(text: &str) -> Result<bool, XmlError> {
    match text.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(XmlError::Shape),
    }
}

fn parse_i8(text: &str) -> Result<i8, XmlError> {
    text.trim().parse().map_err(|_| XmlError::Shape)
}

fn parse_i32(text: &str) -> Result<i32, XmlError> {
    text.trim().parse().map_err(|_| XmlError::Shape)
}

fn parse_flags(text: &str) -> Result<i32, XmlError> {
    let text = text.trim();
    if let Ok(value) = text.parse::<i32>() {
        return Ok(value);
    }
    let mut flags = 0i32;
    if text.is_empty() {
        return Err(XmlError::Shape);
    }
    for part in text.split_whitespace() {
        flags |= match part {
            "Normal" => 0,
            "Maptile" => 1,
            "Rewritable" => 2,
            "Collectable" => 4,
            _ => return Err(XmlError::Shape),
        };
    }
    Ok(flags)
}

struct Node {
    name: String,
    nil: bool,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }
}

struct Parser<'a> {
    text: &'a str,
    index: usize,
    depth: usize,
    elements: usize,
}

fn parse_document(bytes: &[u8]) -> Result<Node, XmlError> {
    if bytes.len() > MAX_XML_CHARS {
        ferroan_diag::refuse(
            HTTP_XML_CHARS,
            Kind::Hard,
            MAX_XML_CHARS as u64,
            bytes.len() as u64,
            "asset",
        );
        return Err(XmlError::Limit);
    }
    let bytes = bytes.strip_prefix(&BOM).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| XmlError::Encoding)?;
    let mut parser = Parser {
        text,
        index: 0,
        depth: 0,
        elements: 0,
    };
    parser.skip_misc()?;
    let node = parser.element()?;
    parser.skip_misc()?;
    if parser.index != parser.text.len() {
        return Err(XmlError::Syntax);
    }
    Ok(node)
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.text[self.index..].chars().next()
    }

    fn bump(&mut self) -> Result<char, XmlError> {
        let mut chars = self.text[self.index..].chars();
        let ch = chars.next().ok_or(XmlError::Syntax)?;
        self.index += ch.len_utf8();
        Ok(ch)
    }

    fn skip_misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_whitespace();
            if self.text[self.index..].starts_with("<?") {
                self.skip_until("?>")?;
                continue;
            }
            if self.text[self.index..].starts_with("<!--") {
                self.skip_until("-->")?;
                continue;
            }
            break;
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
            let _ = self.bump();
        }
    }

    fn skip_until(&mut self, marker: &str) -> Result<(), XmlError> {
        if let Some(found) = self.text[self.index..].find(marker) {
            self.index += found + marker.len();
            Ok(())
        } else {
            Err(XmlError::Syntax)
        }
    }

    fn element(&mut self) -> Result<Node, XmlError> {
        if self.depth >= MAX_DEPTH {
            ferroan_diag::refuse(
                ASSET_XML_DEPTH,
                Kind::Hard,
                MAX_DEPTH as u64,
                self.depth as u64,
                "asset",
            );
            return Err(XmlError::Limit);
        }
        if self.elements >= MAX_ELEMENTS {
            ferroan_diag::refuse(
                ASSET_XML_ELEMENTS,
                Kind::Hard,
                MAX_ELEMENTS as u64,
                self.elements as u64,
                "asset",
            );
            return Err(XmlError::Limit);
        }
        self.depth += 1;
        self.elements += 1;
        if self.bump()? != '<' {
            return Err(XmlError::Syntax);
        }
        let name = self.read_name()?;
        let local = local_name(&name);
        let mut nil = false;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some('/') => {
                    self.bump()?;
                    if self.bump()? != '>' {
                        return Err(XmlError::Syntax);
                    }
                    self.depth -= 1;
                    return Ok(Node {
                        name: local,
                        nil,
                        text: String::new(),
                        children: Vec::new(),
                    });
                }
                Some('>') => {
                    self.bump()?;
                    break;
                }
                Some(_) => {
                    let attr = self.read_name()?;
                    self.skip_whitespace();
                    if self.bump()? != '=' {
                        return Err(XmlError::Syntax);
                    }
                    self.skip_whitespace();
                    let value = self.read_attr()?;
                    if local_name(&attr) == "nil" && value == "true" {
                        nil = true;
                    }
                }
                None => return Err(XmlError::Syntax),
            }
        }
        let mut text = String::new();
        let mut children = Vec::new();
        loop {
            if self.text[self.index..].starts_with("</") {
                self.index += 2;
                let end = self.read_name()?;
                self.skip_whitespace();
                if self.bump()? != '>' || local_name(&end) != local {
                    return Err(XmlError::Syntax);
                }
                break;
            }
            if self.peek() == Some('<') {
                if text.chars().any(|ch| !ch.is_whitespace()) {
                    return Err(XmlError::Syntax);
                }
                text.clear();
                children.push(self.element()?);
            } else {
                text.push_str(&self.read_text()?);
            }
        }
        self.depth -= 1;
        Ok(Node {
            name: local,
            nil,
            text,
            children,
        })
    }

    fn read_name(&mut self) -> Result<String, XmlError> {
        let start = self.index;
        let first = self.peek().ok_or(XmlError::Syntax)?;
        if !is_name_start(first) {
            return Err(XmlError::Syntax);
        }
        self.bump()?;
        while matches!(self.peek(), Some(ch) if is_name_char(ch)) {
            self.bump()?;
        }
        if self.index - start > 128 {
            ferroan_diag::refuse(
                ASSET_XML_NAME,
                Kind::Hard,
                128,
                (self.index - start) as u64,
                "asset",
            );
            return Err(XmlError::Limit);
        }
        Ok(self.text[start..self.index].to_string())
    }

    fn read_attr(&mut self) -> Result<String, XmlError> {
        let quote = self.bump()?;
        if quote != '"' && quote != '\'' {
            return Err(XmlError::Syntax);
        }
        let mut value = String::new();
        loop {
            let ch = self.bump()?;
            if ch == quote {
                break;
            }
            if ch == '&' {
                value.push(self.read_entity()?);
            } else {
                value.push(ch);
            }
        }
        Ok(value)
    }

    fn read_text(&mut self) -> Result<String, XmlError> {
        let mut value = String::new();
        loop {
            match self.peek() {
                None => return Err(XmlError::Syntax),
                Some('<') => break,
                Some('&') => {
                    self.bump()?;
                    value.push(self.read_entity()?);
                }
                Some(_) => value.push(self.bump()?),
            }
        }
        Ok(value)
    }

    fn read_entity(&mut self) -> Result<char, XmlError> {
        let start = self.index;
        let end = self.text[self.index..].find(';').ok_or(XmlError::Syntax)?;
        let body = &self.text[start..start + end];
        self.index = start + end + 1;
        if body.len() > 16 {
            return Err(XmlError::Syntax);
        }
        let ch = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
            let code = u32::from_str_radix(hex, 16).map_err(|_| XmlError::Syntax)?;
            char::from_u32(code).ok_or(XmlError::Syntax)?
        } else if let Some(dec) = body.strip_prefix('#') {
            let code = dec.parse::<u32>().map_err(|_| XmlError::Syntax)?;
            char::from_u32(code).ok_or(XmlError::Syntax)?
        } else {
            match body {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => return Err(XmlError::Syntax),
            }
        };
        Ok(ch)
    }
}

fn local_name(name: &str) -> String {
    name.rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(name)
        .to_string()
}

fn is_name_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch == ':'
}

fn is_name_char(ch: char) -> bool {
    is_name_start(ch) || ch.is_ascii_digit() || ch == '-' || ch == '.'
}

fn encode_base64(data: &[u8]) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= data.len() {
        let n = (u32::from(data[index]) << 16)
            | (u32::from(data[index + 1]) << 8)
            | u32::from(data[index + 2]);
        push_base64(&mut out, n, 4);
        index += 3;
    }
    match data.len() - index {
        1 => {
            let n = u32::from(data[index]) << 16;
            push_base64(&mut out, n, 2);
            out.push_str("==");
        }
        2 => {
            let n = (u32::from(data[index]) << 16) | (u32::from(data[index + 1]) << 8);
            push_base64(&mut out, n, 3);
            out.push('=');
        }
        _ => {}
    }
    out
}

fn push_base64(out: &mut String, n: u32, count: usize) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let shifts = [18, 12, 6, 0];
    for shift in shifts.into_iter().take(count) {
        out.push(TABLE[((n >> shift) & 63) as usize] as char);
    }
}

fn decode_base64(text: &str) -> Result<Vec<u8>, ()> {
    fn value(byte: u8) -> Result<u8, ()> {
        match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(()),
        }
    }
    let mut cleaned = Vec::new();
    for byte in text.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        cleaned.push(byte);
    }
    let (chunks, rest) = cleaned.as_chunks::<4>();
    if !rest.is_empty() {
        return Err(());
    }
    let mut out = Vec::new();
    for chunk in chunks {
        let pad = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if pad > 2 {
            return Err(());
        }
        let mut n = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            if *byte == b'=' {
                if index < 4 - pad {
                    return Err(());
                }
                continue;
            }
            n = (n << 6) | u32::from(value(*byte)?);
        }
        n <<= 6 * pad as u32;
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad == 0 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unnamed_flags_encode_as_normal() {
        let asset = Asset {
            id: Uuid::parse_str("33333333-4444-5555-6666-777777777777").unwrap(),
            name: "unnamed-flags".to_string(),
            description: String::new(),
            type_code: 0,
            local: false,
            temporary: false,
            creator_id: "creator".to_string(),
            flags: 8,
            data: Vec::new(),
            upload_attempts: 0,
        };
        let text = String::from_utf8(encode_asset(&asset)).unwrap();
        assert!(text.contains("<Flags>Normal</Flags>"), "{text}");
    }
}
