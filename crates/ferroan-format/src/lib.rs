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
//   src/libraries/System.Private.CoreLib/src/Resources/Strings.resx
//     Format_InvalidStringWithOffsetAndReason, Format_UnexpectedClosingBrace,
//     Format_UnclosedFormatItem, Format_ExpectedAsciiDigit, Format_IndexOutOfRange
//   dotnet/runtime v10.0.14
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Composite format items from `string.Format` on .NET 10.0.14.
//!
//! [`parse`] splits a format into literal text and holes. [`accept`] applies
//! the same argument-count check `string.Format` applies while it walks.
//! Numeric and date specifiers are stored and not applied. Alignment is the
//! signed width `CompositeFormat` keeps: positive pads on the left, negative
//! on the right, and the pad length is a count of UTF-16 code units.

use std::fmt;

mod parse;

use parse::Walker;

/// Most UTF-16 code units [`parse`] will read. `string.Format` has no cap.
pub const FORMAT_CHAR_LIMIT: u32 = 65_536;

/// One piece of a format [`parse`] accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Text copied into the result. `{{` and `}}` are already one brace.
    Literal(String),
    /// A hole. `format` is absent when the item has no colon, and empty when
    /// the colon is followed by nothing (`{0:}`).
    Hole {
        /// Zero-based argument index.
        index: i32,
        /// Signed width. Zero means no padding.
        alignment: i32,
        /// The format specifier text, stored and not applied.
        format: Option<String>,
    },
}

/// A format [`parse`] accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pieces: Vec<Piece>,
    utf16_units: usize,
}

impl Parsed {
    /// Literals and holes, in order. An empty literal is kept before a hole,
    /// after a hole, and between two holes, including when the format is empty.
    #[must_use]
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// UTF-16 code units in the format. Offsets in a parse error use the same
    /// index .NET uses, which is not a count of Unicode scalar values.
    #[must_use]
    pub fn utf16_units(&self) -> usize {
        self.utf16_units
    }
}

/// Why [`parse`] or [`accept`] refused a format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// `string.Format` was given a null format. The message is
    /// `ArgumentNullException` for parameter `format`.
    Null,
    /// The text `FormatException.Message` carries for a bad format item.
    Invalid { message: String },
    /// A hole index is outside the argument list. When the rest of the format
    /// also parses, `parsed` is that full split. When a later item would not
    /// parse, `parsed` is absent: `string.Format` reports the index and stops.
    ArgIndex { parsed: Option<Parsed> },
    /// A Ferroan bound. This is not a .NET message.
    Limit { id: &'static str },
    /// The parser lost an invariant. A format the runtime accepts or rejects
    /// does not produce this.
    Internal,
}

const NULL_FORMAT: &str = "Value cannot be null. (Parameter 'format')";
const INDEX_RANGE: &str = "Index (zero based) must be greater than or equal to zero and less than the size of the argument list.";

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Null => f.write_str(NULL_FORMAT),
            FormatError::Invalid { message } => f.write_str(message),
            FormatError::ArgIndex { .. } => f.write_str(INDEX_RANGE),
            FormatError::Limit { id } => write!(f, "limit {id}"),
            FormatError::Internal => f.write_str("format parser fault"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Split `format` into literals and holes.
///
/// `None` is a null format. The walk does not look at an argument list.
///
/// # Errors
///
/// Returns [`FormatError::Null`], [`FormatError::Invalid`] with .NET's
/// message, [`FormatError::Limit`] when the format is longer than
/// [`FORMAT_CHAR_LIMIT`] UTF-16 code units, or [`FormatError::Internal`] if
/// the walk cannot continue.
pub fn parse(format: Option<&str>) -> Result<Parsed, FormatError> {
    drive(format, None)
}

/// Parse `format` and apply `string.Format`'s argument check in its order.
///
/// A hole whose index is outside `0..arg_count` fails at that hole. A later
/// item that would not parse is not reported.
///
/// # Errors
///
/// The same errors as [`parse`], plus [`FormatError::ArgIndex`].
pub fn accept(format: Option<&str>, arg_count: usize) -> Result<Parsed, FormatError> {
    match drive(format, Some(arg_count)) {
        Err(FormatError::ArgIndex { parsed: None }) => match drive(format, None) {
            Ok(parsed) => Err(FormatError::ArgIndex {
                parsed: Some(parsed),
            }),
            Err(FormatError::Invalid { .. }) => Err(FormatError::ArgIndex { parsed: None }),
            Err(error) => Err(error),
        },
        other => other,
    }
}

fn drive(format: Option<&str>, arg_count: Option<usize>) -> Result<Parsed, FormatError> {
    let Some(format) = format else {
        return Err(FormatError::Null);
    };
    let observed = utf16_len(format);
    if observed > u64::from(FORMAT_CHAR_LIMIT) {
        ferroan_diag::refuse(
            ferroan_diag::FORMAT_CHARS,
            ferroan_diag::Kind::Hard,
            u64::from(FORMAT_CHAR_LIMIT),
            observed,
            "format.parse",
        );
        return Err(FormatError::Limit {
            id: ferroan_diag::FORMAT_CHARS,
        });
    }
    let units: Vec<u16> = format.encode_utf16().collect();
    let mut walker = Walker::new(&units, arg_count);
    let pieces = walker.walk()?;
    let utf16_units = match usize::try_from(observed) {
        Ok(units) => units,
        Err(_) => return Err(FormatError::Internal),
    };
    Ok(Parsed {
        pieces,
        utf16_units,
    })
}

fn utf16_len(text: &str) -> u64 {
    let mut count = 0u64;
    for _ in text.encode_utf16() {
        count = count.saturating_add(1);
    }
    count
}
