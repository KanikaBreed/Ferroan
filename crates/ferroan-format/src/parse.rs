// Copyright (c) .NET Foundation and Contributors
//
// Licensed to the .NET Foundation under one or more agreements.
// The .NET Foundation licenses this file to you under the MIT license.
//
// Ported to Rust for Ferroan from
//   src/libraries/System.Private.CoreLib/src/System/Text/StringBuilder.cs
//     AppendFormat, IndexLimit and WidthLimit
//   src/libraries/System.Private.CoreLib/src/System/Text/CompositeFormat.cs
//     TryParseLiterals segment list
//   dotnet/runtime v10.0.14
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `AppendFormat` item walk. Digit loops stop once the value is no longer
//! below one million, which is the condition in `StringBuilder`, so a further
//! digit is an unclosed item. A value that is still below the limit may become
//! 9_999_999. `CompositeFormat` keeps multiplying and is not that rule.

use crate::{FormatError, Piece};

const INDEX_LIMIT: i32 = 1_000_000;
const WIDTH_LIMIT: i32 = 1_000_000;
const SPACE: u16 = b' ' as u16;
const OPEN: u16 = b'{' as u16;
const CLOSE: u16 = b'}' as u16;

enum Reason {
    Unexpected,
    Unclosed,
    Digit,
}

pub(crate) struct Walker<'a> {
    units: &'a [u16],
    pos: usize,
    literal: Vec<u16>,
    pieces: Vec<Piece>,
    arg_count: Option<usize>,
}

impl<'a> Walker<'a> {
    pub(crate) fn new(units: &'a [u16], arg_count: Option<usize>) -> Self {
        Self {
            units,
            pos: 0,
            literal: Vec::new(),
            pieces: Vec::new(),
            arg_count,
        }
    }

    pub(crate) fn walk(&mut self) -> Result<Vec<Piece>, FormatError> {
        loop {
            let ch = self.scan_to_hole()?;
            let Some(ch) = ch else {
                return Ok(std::mem::take(&mut self.pieces));
            };
            self.hole(ch)?;
        }
    }

    /// `Ok(None)` is the end of the format. `Ok(Some(ch))` is the first
    /// character of a hole, already read.
    fn scan_to_hole(&mut self) -> Result<Option<u16>, FormatError> {
        loop {
            if self.pos >= self.units.len() {
                self.push_literal()?;
                return Ok(None);
            }
            let remainder = &self.units[self.pos..];
            let Some(count) = index_of_brace(remainder) else {
                self.literal.extend_from_slice(remainder);
                self.push_literal()?;
                return Ok(None);
            };
            self.literal.extend_from_slice(&remainder[..count]);
            self.pos += count;
            let brace = self.units[self.pos];
            let ch = self.move_next()?;
            if brace == ch {
                self.literal.push(ch);
                self.bump()?;
                continue;
            }
            if brace != OPEN {
                return Err(self.invalid(Reason::Unexpected));
            }
            self.push_literal()?;
            return Ok(Some(ch));
        }
    }

    fn hole(&mut self, mut ch: u16) -> Result<(), FormatError> {
        let mut index = match ascii_digit(ch) {
            Some(digit) => digit,
            None => return Err(self.invalid(Reason::Digit)),
        };
        ch = self.move_next()?;
        let mut alignment = 0i32;
        let mut item_format: Option<String> = None;
        if ch != CLOSE {
            while index < INDEX_LIMIT {
                let Some(digit) = ascii_digit(ch) else {
                    break;
                };
                index = match index
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(digit))
                {
                    Some(value) => value,
                    None => return Err(FormatError::Internal),
                };
                ch = self.move_next()?;
            }
            while ch == SPACE {
                ch = self.move_next()?;
            }
            if ch == b',' as u16 {
                alignment = self.alignment(&mut ch)?;
            }
            if ch != CLOSE {
                item_format = Some(self.item_format(&mut ch)?);
            }
        }
        self.bump()?;
        if let Some(arg_count) = self.arg_count
            && !index_in_range(index, arg_count)
        {
            return Err(FormatError::ArgIndex { parsed: None });
        }
        self.pieces.push(Piece::Hole {
            index,
            alignment,
            format: item_format,
        });
        Ok(())
    }

    fn alignment(&mut self, ch: &mut u16) -> Result<i32, FormatError> {
        loop {
            *ch = self.move_next()?;
            if *ch != SPACE {
                break;
            }
        }
        let left = *ch == b'-' as u16;
        if left {
            *ch = self.move_next()?;
        }
        let mut width = match ascii_digit(*ch) {
            Some(digit) => digit,
            None => return Err(self.invalid(Reason::Digit)),
        };
        *ch = self.move_next()?;
        while width < WIDTH_LIMIT {
            let Some(digit) = ascii_digit(*ch) else {
                break;
            };
            width = match width
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
            {
                Some(value) => value,
                None => return Err(FormatError::Internal),
            };
            *ch = self.move_next()?;
        }
        while *ch == SPACE {
            *ch = self.move_next()?;
        }
        if left {
            match width.checked_neg() {
                Some(value) => Ok(value),
                None => Err(FormatError::Internal),
            }
        } else {
            Ok(width)
        }
    }

    fn item_format(&mut self, ch: &mut u16) -> Result<String, FormatError> {
        if *ch != b':' as u16 {
            return Err(self.invalid(Reason::Unclosed));
        }
        let start = self.pos;
        loop {
            *ch = self.move_next()?;
            if *ch == CLOSE {
                break;
            }
            if *ch == OPEN {
                return Err(self.invalid(Reason::Unclosed));
            }
        }
        let Some(from) = start.checked_add(1) else {
            return Err(FormatError::Internal);
        };
        if from > self.pos {
            return Err(FormatError::Internal);
        }
        string_from_units(&self.units[from..self.pos])
    }

    fn push_literal(&mut self) -> Result<(), FormatError> {
        let text = string_from_units(&self.literal)?;
        self.literal.clear();
        self.pieces.push(Piece::Literal(text));
        Ok(())
    }

    fn move_next(&mut self) -> Result<u16, FormatError> {
        self.bump()?;
        if self.pos >= self.units.len() {
            return Err(self.invalid(Reason::Unclosed));
        }
        Ok(self.units[self.pos])
    }

    fn bump(&mut self) -> Result<(), FormatError> {
        self.pos = self.pos.checked_add(1).ok_or(FormatError::Internal)?;
        Ok(())
    }

    fn invalid(&self, reason: Reason) -> FormatError {
        let detail = match reason {
            Reason::Unexpected => "Unexpected closing brace without a corresponding opening brace.",
            Reason::Unclosed => "Format item ends prematurely.",
            Reason::Digit => "Expected an ASCII digit.",
        };
        FormatError::Invalid {
            message: format!(
                "Input string was not in a correct format. Failure to parse near offset {pos}. {detail}",
                pos = self.pos
            ),
        }
    }
}

fn index_of_brace(units: &[u16]) -> Option<usize> {
    units.iter().position(|ch| *ch == OPEN || *ch == CLOSE)
}

fn ascii_digit(ch: u16) -> Option<i32> {
    if (b'0' as u16..=b'9' as u16).contains(&ch) {
        Some(i32::from(ch - b'0' as u16))
    } else {
        None
    }
}

fn index_in_range(index: i32, arg_count: usize) -> bool {
    match usize::try_from(index) {
        Ok(index) => index < arg_count,
        Err(_) => false,
    }
}

fn string_from_units(units: &[u16]) -> Result<String, FormatError> {
    match String::from_utf16(units) {
        Ok(text) => Ok(text),
        Err(_) => Err(FormatError::Internal),
    }
}
