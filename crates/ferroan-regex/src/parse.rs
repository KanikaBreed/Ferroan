// Copyright (c) .NET Foundation and Contributors
//
// Licensed to the .NET Foundation under one or more agreements.
// The .NET Foundation licenses this file to you under the MIT license.
//
// Ported to Rust for Ferroan from
//   src/libraries/System.Text.RegularExpressions/src/System/Text/RegularExpressions/RegexParser.cs
//   dotnet/runtime v10.0.14
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Two-pass pattern walk from `RegexParser`.
//!
//! `CountCaptures` notes slots and can throw. `Reset` then `ScanRegex` walks
//! again. The count pass does not recurse into a subtraction that looked like
//! a range, and it does not throw for an undefined `\1` through `\9`. Those
//! differences are part of the offset .NET reports.

use std::collections::HashMap;

use crate::names::{is_boundary_word, known_property};
use crate::{CLASS_DEPTH_LIMIT, ParseError};

const IGNORE_CASE: u32 = 1;
const MULTILINE: u32 = 2;
const EXPLICIT_CAPTURE: u32 = 4;
const SINGLELINE: u32 = 16;
const IGNORE_WS: u32 = 32;
const RIGHT_TO_LEFT: u32 = 64;
const MAX_DIV_10: i32 = i32::MAX / 10;
const MAX_MOD_10: i32 = i32::MAX % 10;

const Q: u8 = 4;
const S: u8 = 3;
const W: u8 = 1;

/// `RegexParser.Category`, 128 bytes. Only indexes `0..=124` are read.
#[rustfmt::skip]
const CATEGORY: [u8; 128] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, W, W, 0, W, W, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    W, 0, 0, 2, S, 0, 0, 0, S, S, Q, Q, 0, 0, S, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, Q,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, S, S, 0, S, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, Q, S, 0, 0, 0,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Group,
    ExpressionConditional,
    BackreferenceConditional,
}

#[derive(Clone, Copy)]
struct Level {
    kind: Kind,
    children: u32,
}

pub(crate) struct Parser<'a> {
    pattern: &'a str,
    units: Vec<u16>,
    pos: usize,
    options: u32,
    option_stack: Vec<u32>,
    autocap: i32,
    capcount: i32,
    captop: i32,
    caps: HashMap<i32, i32>,
    capnames: Option<HashMap<String, i32>>,
    capnamelist: Option<Vec<String>>,
    ignore_next_paren: bool,
    stack: Vec<Level>,
    current: Level,
    has_unit: bool,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(pattern: &'a str, units: Vec<u16>) -> Self {
        Self {
            pattern,
            units,
            pos: 0,
            options: 0,
            option_stack: Vec::new(),
            autocap: 0,
            capcount: 0,
            captop: 0,
            caps: HashMap::new(),
            capnames: None,
            capnamelist: None,
            ignore_next_paren: false,
            stack: Vec::new(),
            current: Level {
                kind: Kind::Group,
                children: 0,
            },
            has_unit: false,
        }
    }

    pub(crate) fn utf16_len(&self) -> usize {
        self.units.len()
    }

    pub(crate) fn parse(&mut self) -> Result<(), ParseError> {
        self.count_captures()?;
        self.reset();
        self.scan_regex()
    }

    fn reset(&mut self) {
        self.pos = 0;
        self.autocap = 1;
        self.ignore_next_paren = false;
        self.option_stack.clear();
        self.options = 0;
        self.stack.clear();
        self.current = Level {
            kind: Kind::Group,
            children: 0,
        };
        self.has_unit = false;
    }

    fn fail(&self, detail: &str) -> ParseError {
        ParseError::Invalid {
            message: format!(
                "Invalid pattern '{}' at offset {}. {detail}",
                self.pattern, self.pos
            ),
        }
    }

    fn depth_limit(&self, depth: u32) -> Result<(), ParseError> {
        if depth < CLASS_DEPTH_LIMIT {
            return Ok(());
        }
        let observed = u64::from(depth) + 1;
        ferroan_diag::refuse(
            ferroan_diag::REGEX_CLASS_DEPTH,
            ferroan_diag::Kind::Hard,
            u64::from(CLASS_DEPTH_LIMIT),
            observed,
            "regex.parse",
        );
        Err(ParseError::Limit {
            id: ferroan_diag::REGEX_CLASS_DEPTH,
        })
    }

    fn at(&self, index: usize) -> Result<u16, ParseError> {
        self.units.get(index).copied().ok_or(ParseError::Internal)
    }

    fn bump(&mut self) -> Result<u16, ParseError> {
        let ch = self.at(self.pos)?;
        self.pos += 1;
        Ok(ch)
    }

    fn is_capture_slot(&self, slot: i32) -> bool {
        self.caps.contains_key(&slot)
    }

    fn note_capture_slot(&mut self, slot: i32, at: i32) {
        if self.caps.contains_key(&slot) {
            return;
        }
        self.caps.insert(slot, at);
        self.capcount += 1;
        if self.captop <= slot {
            self.captop = if slot == i32::MAX { slot } else { slot + 1 };
        }
    }

    fn note_capture_name(&mut self, name: String, at: i32) {
        if self.capnames.is_none() {
            self.capnames = Some(HashMap::new());
            self.capnamelist = Some(Vec::new());
        }
        let Some(map) = self.capnames.as_mut() else {
            return;
        };
        if map.contains_key(&name) {
            return;
        }
        map.insert(name.clone(), at);
        if let Some(list) = self.capnamelist.as_mut() {
            list.push(name);
        }
    }

    fn count_captures(&mut self) -> Result<(), ParseError> {
        self.note_capture_slot(0, 0);
        self.autocap = 1;
        while self.pos < self.units.len() {
            let ch = self.bump()?;
            match ch {
                ch if ch == u16::from(b'\\') => {
                    if self.pos < self.units.len() {
                        self.scan_backslash(true)?;
                    }
                }
                ch if ch == u16::from(b'#') => {
                    if self.options & IGNORE_WS != 0 {
                        self.pos -= 1;
                        self.scan_blank()?;
                    }
                }
                ch if ch == u16::from(b'[') => self.scan_char_class(true, 0)?,
                ch if ch == u16::from(b')') => {
                    if let Some(saved) = self.option_stack.pop() {
                        self.options = saved;
                    }
                }
                ch if ch == u16::from(b'(') => self.count_open()?,
                _ => {}
            }
        }
        self.assign_name_slots()
    }

    fn count_open(&mut self) -> Result<(), ParseError> {
        let len = self.units.len();
        if self.pos + 1 < len
            && self.at(self.pos)? == u16::from(b'?')
            && self.at(self.pos + 1)? == u16::from(b'#')
        {
            self.pos -= 1;
            return self.scan_blank();
        }
        self.option_stack.push(self.options);
        let mut keep_ignore = false;
        if self.pos < len && self.at(self.pos)? == u16::from(b'?') {
            self.pos += 1;
            if self.pos + 1 < len
                && (self.at(self.pos)? == u16::from(b'<') || self.at(self.pos)? == u16::from(b'\''))
            {
                self.pos += 1;
                if self.pos < len {
                    let ch = self.at(self.pos)?;
                    if ch != u16::from(b'0') && is_boundary_word(ch) {
                        if (u16::from(b'1')..=u16::from(b'9')).contains(&ch) {
                            let slot = self.scan_decimal()?;
                            self.note_capture_slot(slot, 0);
                        } else {
                            let name = self.scan_capname()?;
                            self.note_capture_name(name, 0);
                        }
                    }
                }
            } else {
                self.scan_options();
                if self.pos < len {
                    let ch = self.at(self.pos)?;
                    if ch == u16::from(b')') {
                        self.pos += 1;
                        self.option_stack.pop();
                    } else if ch == u16::from(b'(') {
                        self.ignore_next_paren = true;
                        keep_ignore = true;
                    }
                }
            }
        } else if self.options & EXPLICIT_CAPTURE == 0 && !self.ignore_next_paren {
            let slot = self.autocap;
            self.autocap = self.autocap.wrapping_add(1);
            self.note_capture_slot(slot, 0);
        }
        if !keep_ignore {
            self.ignore_next_paren = false;
        }
        Ok(())
    }

    fn assign_name_slots(&mut self) -> Result<(), ParseError> {
        if self.capnamelist.is_some() {
            let names = self.capnamelist.clone().ok_or(ParseError::Internal)?;
            for name in names {
                while self.is_capture_slot(self.autocap) {
                    self.autocap = self.autocap.wrapping_add(1);
                }
                let noted = self
                    .capnames
                    .as_ref()
                    .and_then(|map| map.get(&name).copied())
                    .ok_or(ParseError::Internal)?;
                if let Some(map) = self.capnames.as_mut() {
                    map.insert(name, self.autocap);
                }
                self.note_capture_slot(self.autocap, noted);
                self.autocap = self.autocap.wrapping_add(1);
            }
        }
        let capnumlist = if self.capcount < self.captop {
            let mut keys: Vec<i32> = self.caps.keys().copied().collect();
            keys.sort_unstable();
            if i32::try_from(keys.len()).ok() != Some(self.capcount) {
                return Err(ParseError::Internal);
            }
            Some(keys)
        } else {
            None
        };
        if self.capnames.is_some() || capnumlist.is_some() {
            self.merge_names(capnumlist)?;
        }
        Ok(())
    }

    fn merge_names(&mut self, capnumlist: Option<Vec<i32>>) -> Result<(), ParseError> {
        let mut next: i32;
        let mut old = if self.capnames.is_none() {
            self.capnames = Some(HashMap::new());
            self.capnamelist = Some(Vec::new());
            next = -1;
            Vec::new()
        } else {
            let list = self.capnamelist.clone().ok_or(ParseError::Internal)?;
            let first = list.first().ok_or(ParseError::Internal)?;
            next = self
                .capnames
                .as_ref()
                .and_then(|map| map.get(first).copied())
                .ok_or(ParseError::Internal)?;
            self.capnamelist = Some(Vec::new());
            list
        };
        let mut k = 0usize;
        let count = usize::try_from(self.capcount).unwrap_or(0);
        for i in 0..count {
            let j = match &capnumlist {
                Some(list) => *list.get(i).ok_or(ParseError::Internal)?,
                None => i32::try_from(i).unwrap_or(0),
            };
            if next == j {
                let name = old.get(k).ok_or(ParseError::Internal)?.clone();
                k += 1;
                next = if k == old.len() {
                    -1
                } else {
                    let upcoming = old.get(k).ok_or(ParseError::Internal)?;
                    self.capnames
                        .as_ref()
                        .and_then(|map| map.get(upcoming).copied())
                        .ok_or(ParseError::Internal)?
                };
                if let Some(list) = self.capnamelist.as_mut() {
                    list.push(name);
                }
            } else {
                let text = j.to_string();
                if let Some(map) = self.capnames.as_mut() {
                    map.insert(text.clone(), j);
                }
                if let Some(list) = self.capnamelist.as_mut() {
                    list.push(text);
                }
            }
        }
        let _ = &mut old;
        Ok(())
    }

    fn scan_regex(&mut self) -> Result<(), ParseError> {
        let mut is_quantifier = false;
        while self.pos < self.units.len() {
            let mut was_prev = is_quantifier;
            is_quantifier = false;
            self.scan_blank()?;
            let startpos = self.pos;
            self.scan_ordinary()?;
            let endpos = self.pos;
            self.scan_blank()?;
            let ch = if self.pos == self.units.len() {
                u16::from(b'!')
            } else {
                let ch = self.at(self.pos)?;
                if is_special(ch) {
                    is_quantifier = is_quantifier_char(ch);
                    self.pos += 1;
                    ch
                } else {
                    u16::from(b' ')
                }
            };
            if startpos < endpos {
                let unquantified = endpos - startpos - usize::from(is_quantifier);
                was_prev = false;
                if is_quantifier && unquantified < endpos {
                    self.has_unit = true;
                }
            }
            match ch {
                ch if ch == u16::from(b'!') => break,
                ch if ch == u16::from(b' ') => continue,
                ch if ch == u16::from(b'[') => {
                    self.scan_char_class(false, 0)?;
                    self.has_unit = true;
                }
                ch if ch == u16::from(b'(') => {
                    self.option_stack.push(self.options);
                    if let Some(kind) = self.scan_group_open()? {
                        self.stack.push(self.current);
                        self.current = Level { kind, children: 0 };
                    } else {
                        self.option_stack.pop();
                    }
                    continue;
                }
                ch if ch == u16::from(b'|') => {
                    self.add_alternate();
                    continue;
                }
                ch if ch == u16::from(b')') => {
                    if !self.on_close()? {
                        continue;
                    }
                }
                ch if ch == u16::from(b'\\') => {
                    if self.pos == self.units.len() {
                        return Err(self.fail("Illegal \\ at end of pattern."));
                    }
                    self.scan_backslash(false)?;
                }
                ch if ch == u16::from(b'^') || ch == u16::from(b'$') || ch == u16::from(b'.') => {
                    self.has_unit = true;
                }
                ch if ch == u16::from(b'{')
                    || ch == u16::from(b'*')
                    || ch == u16::from(b'+')
                    || ch == u16::from(b'?') =>
                {
                    if !self.has_unit {
                        let shown = display_unit(ch)?;
                        let detail = if was_prev {
                            format!("Nested quantifier '{shown}'.")
                        } else {
                            format!("Quantifier '{shown}' following nothing.")
                        };
                        return Err(self.fail(&detail));
                    }
                    self.pos -= 1;
                }
                _ => return Err(ParseError::Internal),
            }
            // .NET assigns isQuantifier = IsTrueQuantifier() here, and that
            // value is wasPrevQuantifier on the next atom. The end-of-pattern
            // test short-circuits, so a quantifier character that ends the
            // pattern keeps the flag set by IsQuantifier. A brace that only
            // looks like a quantifier and is later rewound stays true, which
            // is what makes ^?* and \p{L}?* report a nested quantifier.
            self.scan_blank()?;
            let at_end = self.pos == self.units.len();
            if !at_end {
                is_quantifier = self.is_true_quantifier();
            }
            if at_end || !is_quantifier {
                if !self.has_unit {
                    return Err(ParseError::Internal);
                }
                self.has_unit = false;
                continue;
            }
            let qch = self.bump()?;
            self.apply_quantifier(qch)?;
        }
        if !self.stack.is_empty() {
            return Err(self.fail("Not enough )'s."));
        }
        self.finish_group()?;
        Ok(())
    }

    fn scan_ordinary(&mut self) -> Result<(), ParseError> {
        let x_mode = self.options & IGNORE_WS != 0;
        while self.pos < self.units.len() {
            let ch = self.at(self.pos)?;
            let stop = if x_mode {
                is_special_or_space(ch)
            } else {
                is_special(ch)
            };
            if !stop || (ch == u16::from(b'{') && !self.is_true_quantifier()) {
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn apply_quantifier(&mut self, qch: u16) -> Result<(), ParseError> {
        let (min, max, rewound) = self.quantifier_bounds(qch)?;
        if rewound {
            self.has_unit = false;
            return Ok(());
        }
        self.scan_blank()?;
        if self.pos < self.units.len() && self.at(self.pos)? == u16::from(b'?') {
            self.pos += 1;
        }
        if min > max {
            return Err(self.fail("Illegal {x,y} with x > y."));
        }
        self.has_unit = false;
        Ok(())
    }

    fn quantifier_bounds(&mut self, qch: u16) -> Result<(i32, i32, bool), ParseError> {
        match qch {
            ch if ch == u16::from(b'*') => Ok((0, i32::MAX, false)),
            ch if ch == u16::from(b'?') => Ok((0, 1, false)),
            ch if ch == u16::from(b'+') => Ok((1, i32::MAX, false)),
            ch if ch == u16::from(b'{') => {
                let startpos = self.pos;
                let mut max = self.scan_decimal()?;
                let min = max;
                if startpos < self.pos
                    && self.pos < self.units.len()
                    && self.at(self.pos)? == u16::from(b',')
                {
                    self.pos += 1;
                    if self.pos == self.units.len() || self.at(self.pos)? == u16::from(b'}') {
                        max = i32::MAX;
                    } else {
                        max = self.scan_decimal()?;
                    }
                }
                if startpos == self.pos
                    || self.pos == self.units.len()
                    || self.bump()? != u16::from(b'}')
                {
                    if startpos == 0 {
                        return Err(ParseError::Internal);
                    }
                    self.pos = startpos - 1;
                    return Ok((0, 0, true));
                }
                Ok((min, max, false))
            }
            _ => Err(ParseError::Internal),
        }
    }

    fn add_alternate(&mut self) {
        if self.current.kind == Kind::ExpressionConditional
            || self.current.kind == Kind::BackreferenceConditional
        {
            self.current.children += 1;
        }
    }

    fn finish_group(&mut self) -> Result<(), ParseError> {
        if self.current.kind == Kind::ExpressionConditional
            || self.current.kind == Kind::BackreferenceConditional
        {
            self.current.children += 1;
            let children = self.current.children;
            if (self.current.kind == Kind::BackreferenceConditional && children > 2) || children > 3
            {
                return Err(self.fail("Too many | in (?()|)."));
            }
        }
        self.has_unit = true;
        Ok(())
    }

    /// Close one group. `false` means the caller skips the quantifier, as the
    /// C# `)` arm does when `_unit` is null.
    fn on_close(&mut self) -> Result<bool, ParseError> {
        if self.stack.is_empty() {
            return Err(self.fail("Too many )'s."));
        }
        self.finish_group()?;
        self.current = self.stack.pop().ok_or(ParseError::Internal)?;
        self.options = self.option_stack.pop().ok_or(ParseError::Internal)?;
        if self.current.kind == Kind::ExpressionConditional && self.current.children == 0 {
            if !self.has_unit {
                return Err(self.fail("Illegal conditional (?(...)) expression."));
            }
            self.current.children += 1;
            self.has_unit = false;
        }
        Ok(self.has_unit)
    }

    fn scan_group_open(&mut self) -> Result<Option<Kind>, ParseError> {
        let len = self.units.len();
        if self.pos == len
            || self.at(self.pos)? != u16::from(b'?')
            || (self.pos + 1 < len && self.at(self.pos + 1)? == u16::from(b')'))
        {
            if self.options & EXPLICIT_CAPTURE != 0 || self.ignore_next_paren {
                self.ignore_next_paren = false;
                return Ok(Some(Kind::Group));
            }
            self.autocap = self.autocap.wrapping_add(1);
            return Ok(Some(Kind::Group));
        }
        self.pos += 1;
        if self.pos == len {
            return Err(self.fail("Unrecognized grouping construct."));
        }
        let mut close = u16::from(b'>');
        let mut ch = self.bump()?;
        if ch == u16::from(b'\'') {
            close = u16::from(b'\'');
            ch = u16::from(b'<');
        }
        match ch {
            ch if ch == u16::from(b':')
                || ch == u16::from(b'=')
                || ch == u16::from(b'!')
                || ch == u16::from(b'>') =>
            {
                if ch == u16::from(b'=') || ch == u16::from(b'!') {
                    self.options &= !RIGHT_TO_LEFT;
                }
                Ok(Some(Kind::Group))
            }
            ch if ch == u16::from(b'<') => self.scan_angle(close),
            ch if ch == u16::from(b'(') => self.scan_conditional(),
            _ => {
                self.pos -= 1;
                if self.current.kind != Kind::ExpressionConditional {
                    self.scan_options();
                }
                if self.pos == self.units.len() {
                    return Err(self.fail("Unrecognized grouping construct."));
                }
                let next = self.bump()?;
                if next == u16::from(b')') {
                    return Ok(None);
                }
                if next == u16::from(b':') {
                    return Ok(Some(Kind::Group));
                }
                Err(self.fail("Unrecognized grouping construct."))
            }
        }
    }

    fn scan_angle(&mut self, close: u16) -> Result<Option<Kind>, ParseError> {
        if self.pos == self.units.len() {
            return Err(self.fail("Unrecognized grouping construct."));
        }
        let mut ch = self.bump()?;
        if ch == u16::from(b'=') || ch == u16::from(b'!') {
            if close == u16::from(b'\'') {
                return Err(self.fail("Unrecognized grouping construct."));
            }
            self.options |= RIGHT_TO_LEFT;
            return Ok(Some(Kind::Group));
        }
        self.pos -= 1;
        let mut capnum: i32 = -1;
        let mut uncapnum: i32 = -1;
        let mut proceed = false;
        if ch.wrapping_sub(u16::from(b'0')) <= 9 {
            capnum = self.scan_decimal()?;
            if !self.is_capture_slot(capnum) {
                capnum = -1;
            }
            if self.pos < self.units.len() {
                let next = self.at(self.pos)?;
                if next != close && next != u16::from(b'-') {
                    return Err(self.fail(
                        "Invalid group name: Group names must begin with a word character.",
                    ));
                }
            }
            if capnum == 0 {
                return Err(self.fail("Capture number cannot be zero."));
            }
        } else if is_boundary_word(ch) {
            let name = self.scan_capname()?;
            if let Some(slot) = self
                .capnames
                .as_ref()
                .and_then(|map| map.get(&name).copied())
            {
                capnum = slot;
            }
            if self.pos < self.units.len() {
                let next = self.at(self.pos)?;
                if next != close && next != u16::from(b'-') {
                    return Err(self.fail(
                        "Invalid group name: Group names must begin with a word character.",
                    ));
                }
            }
        } else if ch == u16::from(b'-') {
            proceed = true;
        } else {
            return Err(
                self.fail("Invalid group name: Group names must begin with a word character.")
            );
        }
        if (capnum != -1 || proceed)
            && self.pos + 1 < self.units.len()
            && self.at(self.pos)? == u16::from(b'-')
        {
            self.pos += 1;
            ch = self.at(self.pos)?;
            if ch.wrapping_sub(u16::from(b'0')) <= 9 {
                uncapnum = self.scan_decimal()?;
                if !self.is_capture_slot(uncapnum) {
                    return Err(
                        self.fail(&format!("Reference to undefined group number {uncapnum}."))
                    );
                }
                if self.pos < self.units.len() && self.at(self.pos)? != close {
                    return Err(self.fail(
                        "Invalid group name: Group names must begin with a word character.",
                    ));
                }
            } else if is_boundary_word(ch) {
                let name = self.scan_capname()?;
                match self
                    .capnames
                    .as_ref()
                    .and_then(|map| map.get(&name).copied())
                {
                    Some(slot) => uncapnum = slot,
                    None => {
                        return Err(
                            self.fail(&format!("Reference to undefined group name '{name}'."))
                        );
                    }
                }
                if self.pos < self.units.len() && self.at(self.pos)? != close {
                    return Err(self.fail(
                        "Invalid group name: Group names must begin with a word character.",
                    ));
                }
            } else {
                return Err(
                    self.fail("Invalid group name: Group names must begin with a word character.")
                );
            }
        }
        if (capnum != -1 || uncapnum != -1) && self.pos < self.units.len() && self.bump()? == close
        {
            return Ok(Some(Kind::Group));
        }
        Err(self.fail("Unrecognized grouping construct."))
    }

    fn scan_conditional(&mut self) -> Result<Option<Kind>, ParseError> {
        let paren_pos = self.pos;
        if self.pos < self.units.len() {
            let ch = self.at(self.pos)?;
            if (u16::from(b'0')..=u16::from(b'9')).contains(&ch) {
                let capnum = self.scan_decimal()?;
                if self.pos < self.units.len() && self.bump()? == u16::from(b')') {
                    if self.is_capture_slot(capnum) {
                        return Ok(Some(Kind::BackreferenceConditional));
                    }
                    return Err(self.fail(&format!(
                        "Conditional alternation refers to an undefined group number {capnum}."
                    )));
                }
                return Err(self.fail(&format!(
                    "Conditional alternation is missing a closing parenthesis after the group number {capnum}."
                )));
            } else if is_boundary_word(ch) {
                let name = self.scan_capname()?;
                if let Some(slot) = self
                    .capnames
                    .as_ref()
                    .and_then(|map| map.get(&name).copied())
                    && self.pos < self.units.len()
                    && self.bump()? == u16::from(b')')
                {
                    let _ = slot;
                    return Ok(Some(Kind::BackreferenceConditional));
                }
            }
        }
        if paren_pos == 0 {
            return Err(ParseError::Internal);
        }
        self.pos = paren_pos - 1;
        self.ignore_next_paren = true;
        if self.pos + 2 < self.units.len() && self.at(self.pos + 1)? == u16::from(b'?') {
            let marker = self.at(self.pos + 2)?;
            if marker == u16::from(b'#') {
                return Err(self.fail("Alternation conditions cannot be comments."));
            }
            if marker == u16::from(b'\'')
                || (self.pos + 3 < self.units.len()
                    && marker == u16::from(b'<')
                    && self.at(self.pos + 3)? != u16::from(b'!')
                    && self.at(self.pos + 3)? != u16::from(b'='))
            {
                return Err(self.fail("Alternation conditions do not capture and cannot be named."));
            }
        }
        Ok(Some(Kind::ExpressionConditional))
    }

    fn scan_blank(&mut self) -> Result<(), ParseError> {
        loop {
            if self.options & IGNORE_WS != 0 {
                while self.pos < self.units.len() && is_space(self.at(self.pos)?) {
                    self.pos += 1;
                }
            }
            if self.options & IGNORE_WS != 0
                && self.pos < self.units.len()
                && self.at(self.pos)? == u16::from(b'#')
            {
                match self.units[self.pos..]
                    .iter()
                    .position(|unit| *unit == u16::from(b'\n'))
                {
                    Some(rel) => self.pos += rel,
                    None => self.pos = self.units.len(),
                }
            } else if self.pos + 2 < self.units.len()
                && self.at(self.pos + 2)? == u16::from(b'#')
                && self.at(self.pos + 1)? == u16::from(b'?')
                && self.at(self.pos)? == u16::from(b'(')
            {
                match self.units[self.pos..]
                    .iter()
                    .position(|unit| *unit == u16::from(b')'))
                {
                    Some(rel) => self.pos += rel + 1,
                    None => {
                        self.pos = self.units.len();
                        return Err(self.fail("Unterminated (?#...) comment."));
                    }
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn scan_backslash(&mut self, scan_only: bool) -> Result<(), ParseError> {
        if self.pos >= self.units.len() {
            return Err(ParseError::Internal);
        }
        let ch = self.at(self.pos)?;
        match ch {
            _ if ch == u16::from(b'b')
                || ch == u16::from(b'B')
                || ch == u16::from(b'A')
                || ch == u16::from(b'G')
                || ch == u16::from(b'Z')
                || ch == u16::from(b'z')
                || ch == u16::from(b'w')
                || ch == u16::from(b'W')
                || ch == u16::from(b's')
                || ch == u16::from(b'S')
                || ch == u16::from(b'd')
                || ch == u16::from(b'D') =>
            {
                self.pos += 1;
                if !scan_only {
                    self.has_unit = true;
                }
                Ok(())
            }
            _ if ch == u16::from(b'p') || ch == u16::from(b'P') => {
                self.pos += 1;
                if scan_only {
                    return Ok(());
                }
                let name = self.parse_property()?;
                self.require_property(&name)?;
                self.has_unit = true;
                Ok(())
            }
            _ => self.scan_basic_backslash(scan_only),
        }
    }

    fn scan_basic_backslash(&mut self, scan_only: bool) -> Result<(), ParseError> {
        let backpos = self.pos;
        let mut close = 0u16;
        let mut angled = false;
        let mut ch = self.at(self.pos)?;
        if ch == u16::from(b'k') {
            if self.pos + 1 < self.units.len() {
                self.pos += 1;
                ch = self.bump()?;
                if ch == u16::from(b'<') || ch == u16::from(b'\'') {
                    angled = true;
                    close = if ch == u16::from(b'\'') {
                        u16::from(b'\'')
                    } else {
                        u16::from(b'>')
                    };
                }
            }
            if !angled || self.pos == self.units.len() {
                return Err(self.fail("Malformed \\k<...> named back reference."));
            }
            ch = self.at(self.pos)?;
        } else if (ch == u16::from(b'<') || ch == u16::from(b'\''))
            && self.pos + 1 < self.units.len()
        {
            angled = true;
            close = if ch == u16::from(b'\'') {
                u16::from(b'\'')
            } else {
                u16::from(b'>')
            };
            self.pos += 1;
            ch = self.at(self.pos)?;
        }
        if angled && (u16::from(b'0')..=u16::from(b'9')).contains(&ch) {
            let capnum = self.scan_decimal()?;
            if self.pos < self.units.len() && self.bump()? == close {
                if scan_only {
                    return Ok(());
                }
                if self.is_capture_slot(capnum) {
                    self.has_unit = true;
                    return Ok(());
                }
                return Err(self.fail(&format!("Reference to undefined group number {capnum}.")));
            }
        } else if !angled && (u16::from(b'1')..=u16::from(b'9')).contains(&ch) {
            if self.options & 256 == 0 {
                let capnum = self.scan_decimal()?;
                if scan_only {
                    return Ok(());
                }
                if self.is_capture_slot(capnum) {
                    self.has_unit = true;
                    return Ok(());
                }
                if capnum <= 9 {
                    return Err(
                        self.fail(&format!("Reference to undefined group number {capnum}."))
                    );
                }
            }
        } else if angled && is_boundary_word(ch) {
            let name = self.scan_capname()?;
            if self.pos < self.units.len() && self.bump()? == close {
                if scan_only {
                    return Ok(());
                }
                if self
                    .capnames
                    .as_ref()
                    .and_then(|map| map.get(&name).copied())
                    .is_some()
                {
                    self.has_unit = true;
                    return Ok(());
                }
                return Err(self.fail(&format!("Reference to undefined group name '{name}'.")));
            }
        }
        self.pos = backpos;
        let _escaped = self.scan_char_escape()?;
        if !scan_only {
            self.has_unit = true;
        }
        Ok(())
    }

    fn scan_capname(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        while self.pos < self.units.len() {
            if !is_boundary_word(self.at(self.pos)?) {
                break;
            }
            self.pos += 1;
        }
        String::from_utf16(&self.units[start..self.pos]).map_err(|_| ParseError::Internal)
    }

    fn scan_decimal(&mut self) -> Result<i32, ParseError> {
        let mut value: i32 = 0;
        while self.pos < self.units.len() {
            let digit = self.at(self.pos)?.wrapping_sub(u16::from(b'0'));
            if digit > 9 {
                break;
            }
            self.pos += 1;
            if value > MAX_DIV_10 || (value == MAX_DIV_10 && i32::from(digit) > MAX_MOD_10) {
                return Err(self.fail(
                    "Quantifier and capture group numbers must be less than or equal to Int32.MaxValue.",
                ));
            }
            value = value * 10 + i32::from(digit);
        }
        Ok(value)
    }

    fn scan_hex(&mut self, mut digits: i32) -> Result<u16, ParseError> {
        let mut value = 0i32;
        let width = usize::try_from(digits).unwrap_or(usize::MAX);
        if self.pos.saturating_add(width) <= self.units.len() {
            while digits > 0 {
                let ch = self.bump()?;
                let result = hex_value(ch);
                if result == 0xFF {
                    break;
                }
                value = value * 16 + result;
                digits -= 1;
            }
        }
        if digits > 0 {
            return Err(self.fail("Insufficient or invalid hexadecimal digits."));
        }
        u16::try_from(value).map_err(|_| ParseError::Internal)
    }

    fn scan_control(&mut self) -> Result<u16, ParseError> {
        if self.pos == self.units.len() {
            return Err(self.fail("Missing control character."));
        }
        let mut ch = self.bump()?;
        if (u16::from(b'a')..=u16::from(b'z')).contains(&ch) {
            ch -= u16::from(b'a' - b'A');
        }
        let folded = ch.wrapping_sub(u16::from(b'@'));
        if folded < u16::from(b' ') {
            return Ok(folded);
        }
        Err(self.fail("Unrecognized control character."))
    }

    fn scan_options(&mut self) {
        let mut off = false;
        while self.pos < self.units.len() {
            let ch = match self.units.get(self.pos).copied() {
                Some(ch) => ch,
                None => return,
            };
            if ch == u16::from(b'-') {
                off = true;
            } else if ch == u16::from(b'+') {
                off = false;
            } else {
                let folded = if ch < 128 { ch | 0x20 } else { ch };
                let bit = match folded {
                    ch if ch == u16::from(b'i') => IGNORE_CASE,
                    ch if ch == u16::from(b'm') => MULTILINE,
                    ch if ch == u16::from(b'n') => EXPLICIT_CAPTURE,
                    ch if ch == u16::from(b's') => SINGLELINE,
                    ch if ch == u16::from(b'x') => IGNORE_WS,
                    _ => 0,
                };
                if bit == 0 {
                    return;
                }
                if off {
                    self.options &= !bit;
                } else {
                    self.options |= bit;
                }
            }
            self.pos += 1;
        }
    }

    fn scan_char_escape(&mut self) -> Result<u16, ParseError> {
        let ch = self.bump()?;
        if (u16::from(b'0')..=u16::from(b'7')).contains(&ch) {
            self.pos -= 1;
            return Ok(self.scan_octal());
        }
        match ch {
            ch if ch == u16::from(b'x') => self.scan_hex(2),
            ch if ch == u16::from(b'u') => self.scan_hex(4),
            ch if ch == u16::from(b'a') => Ok(0x0007),
            ch if ch == u16::from(b'b') => Ok(0x0008),
            ch if ch == u16::from(b'e') => Ok(0x001B),
            ch if ch == u16::from(b'f') => Ok(0x000C),
            ch if ch == u16::from(b'n') => Ok(0x000A),
            ch if ch == u16::from(b'r') => Ok(0x000D),
            ch if ch == u16::from(b't') => Ok(0x0009),
            ch if ch == u16::from(b'v') => Ok(0x000B),
            ch if ch == u16::from(b'c') => self.scan_control(),
            _ => {
                if self.options & 256 == 0 && is_boundary_word(ch) {
                    let shown = display_unit(ch)?;
                    return Err(self.fail(&format!("Unrecognized escape sequence \\{shown}.")));
                }
                Ok(ch)
            }
        }
    }

    fn scan_octal(&mut self) -> u16 {
        let mut remaining = self.units.len().saturating_sub(self.pos).min(3);
        let mut value = 0i32;
        while remaining > 0 {
            let Some(unit) = self.units.get(self.pos).copied() else {
                break;
            };
            let digit = unit.wrapping_sub(u16::from(b'0'));
            if digit > 7 {
                break;
            }
            self.pos += 1;
            value = value * 8 + i32::from(digit);
            remaining -= 1;
        }
        u16::try_from(value & 0xFF).unwrap_or(0)
    }

    fn parse_property(&mut self) -> Result<String, ParseError> {
        if self.pos + 2 >= self.units.len() {
            return Err(self.fail("Incomplete \\p{X} character escape."));
        }
        if self.bump()? != u16::from(b'{') {
            return Err(self.fail("Malformed \\p{X} character escape."));
        }
        let start = self.pos;
        while self.pos < self.units.len() {
            let ch = self.at(self.pos)?;
            if !(is_boundary_word(ch) || ch == u16::from(b'-')) {
                break;
            }
            self.pos += 1;
        }
        let name =
            String::from_utf16(&self.units[start..self.pos]).map_err(|_| ParseError::Internal)?;
        if self.pos == self.units.len() || self.bump()? != u16::from(b'}') {
            return Err(self.fail("Incomplete \\p{X} character escape."));
        }
        Ok(name)
    }

    fn require_property(&self, name: &str) -> Result<(), ParseError> {
        if known_property(name) {
            return Ok(());
        }
        Err(self.fail(&format!("Unknown property '{name}'.")))
    }

    fn scan_char_class(&mut self, scan_only: bool, depth: u32) -> Result<(), ParseError> {
        let mut in_range = false;
        let mut first = true;
        let mut closed = false;
        let mut prev = 0u16;
        if self.pos < self.units.len() && self.at(self.pos)? == u16::from(b'^') {
            self.pos += 1;
            if self.options & 256 != 0
                && self.pos < self.units.len()
                && self.at(self.pos)? == u16::from(b']')
            {
                first = false;
            }
        }
        while self.pos < self.units.len() {
            let mut translated = false;
            let mut ch = self.bump()?;
            if ch == u16::from(b']') {
                if !first {
                    closed = true;
                    break;
                }
            } else if ch == u16::from(b'\\') && self.pos < self.units.len() {
                ch = self.bump()?;
                if matches_class_shorthand(ch) {
                    if !scan_only && in_range {
                        let shown = display_unit(ch)?;
                        return Err(self.fail(&format!(
                            "Cannot include class \\{shown} in character range."
                        )));
                    }
                    if ch == u16::from(b'p') || ch == u16::from(b'P') {
                        let name = self.parse_property()?;
                        if !scan_only {
                            self.require_property(&name)?;
                        }
                    }
                    first = false;
                    continue;
                } else if ch == u16::from(b'-') {
                    if !scan_only && in_range {
                        if prev > ch {
                            return Err(self.fail("[x-y] range in reverse order."));
                        }
                        in_range = false;
                        prev = 0;
                    }
                    first = false;
                    continue;
                } else {
                    self.pos -= 1;
                    ch = self.scan_char_escape()?;
                    translated = true;
                }
            }
            if in_range {
                in_range = false;
                if !scan_only {
                    if ch == u16::from(b'[') && !translated && !first {
                        self.depth_limit(depth)?;
                        self.scan_char_class(false, depth + 1)?;
                        if self.pos < self.units.len() && self.at(self.pos)? != u16::from(b']') {
                            return Err(self.fail(
                                "A subtraction must be the last element in a character class.",
                            ));
                        }
                    } else if prev > ch {
                        return Err(self.fail("[x-y] range in reverse order."));
                    }
                }
            } else if self.pos + 1 < self.units.len()
                && self.at(self.pos)? == u16::from(b'-')
                && self.at(self.pos + 1)? != u16::from(b']')
            {
                prev = ch;
                in_range = true;
                self.pos += 1;
            } else if self.pos < self.units.len()
                && ch == u16::from(b'-')
                && !translated
                && self.at(self.pos)? == u16::from(b'[')
                && !first
            {
                self.pos += 1;
                self.depth_limit(depth)?;
                self.scan_char_class(scan_only, depth + 1)?;
                if !scan_only
                    && self.pos < self.units.len()
                    && self.at(self.pos)? != u16::from(b']')
                {
                    return Err(
                        self.fail("A subtraction must be the last element in a character class.")
                    );
                }
            }
            first = false;
        }
        if !closed {
            return Err(self.fail("Unterminated [] set."));
        }
        Ok(())
    }

    fn is_true_quantifier(&self) -> bool {
        let Some(start) = self.units.get(self.pos).copied() else {
            return false;
        };
        if start != u16::from(b'{') {
            return start <= u16::from(b'{') && category(start) >= Q;
        }
        let mut pos = self.pos;
        let mut n_chars = self.units.len() - self.pos;
        let mut ch = start;
        loop {
            if n_chars == 0 {
                break;
            }
            n_chars -= 1;
            if n_chars == 0 {
                break;
            }
            pos += 1;
            ch = self.units[pos];
            if ch.wrapping_sub(u16::from(b'0')) > 9 {
                break;
            }
        }
        if n_chars == 0 || pos - self.pos == 1 {
            return false;
        }
        if ch == u16::from(b'}') {
            return true;
        }
        if ch != u16::from(b',') {
            return false;
        }
        loop {
            if n_chars == 0 {
                break;
            }
            n_chars -= 1;
            if n_chars == 0 {
                break;
            }
            pos += 1;
            ch = self.units[pos];
            if ch.wrapping_sub(u16::from(b'0')) > 9 {
                break;
            }
        }
        n_chars > 0 && ch == u16::from(b'}')
    }
}

fn category(ch: u16) -> u8 {
    if usize::from(ch) < CATEGORY.len() && ch <= u16::from(b'|') {
        CATEGORY[usize::from(ch)]
    } else {
        0
    }
}

fn is_special(ch: u16) -> bool {
    ch <= u16::from(b'|') && category(ch) >= S
}

fn is_special_or_space(ch: u16) -> bool {
    ch <= u16::from(b'|') && category(ch) >= W
}

fn is_quantifier_char(ch: u16) -> bool {
    ch <= u16::from(b'{') && category(ch) == Q
}

fn is_space(ch: u16) -> bool {
    ch <= u16::from(b' ') && category(ch) == W
}

fn matches_class_shorthand(ch: u16) -> bool {
    ch == u16::from(b'D')
        || ch == u16::from(b'd')
        || ch == u16::from(b'S')
        || ch == u16::from(b's')
        || ch == u16::from(b'W')
        || ch == u16::from(b'w')
        || ch == u16::from(b'p')
        || ch == u16::from(b'P')
}

fn display_unit(ch: u16) -> Result<char, ParseError> {
    char::from_u32(u32::from(ch)).ok_or(ParseError::Internal)
}

fn hex_value(ch: u16) -> i32 {
    if (u16::from(b'0')..=u16::from(b'9')).contains(&ch) {
        i32::from(ch - u16::from(b'0'))
    } else if (u16::from(b'a')..=u16::from(b'f')).contains(&ch) {
        i32::from(ch - u16::from(b'a')) + 10
    } else if (u16::from(b'A')..=u16::from(b'F')).contains(&ch) {
        i32::from(ch - u16::from(b'A')) + 10
    } else {
        0xFF
    }
}
