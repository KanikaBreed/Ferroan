// Copyright (c) .NET Foundation and Contributors
//
// Licensed to the .NET Foundation under one or more agreements.
// The .NET Foundation licenses this file to you under the MIT license.
//
// Ported to Rust for Ferroan from
//   src/libraries/System.Text.RegularExpressions/src/System/Text/RegularExpressions/RegexParser.cs
//   src/libraries/System.Text.RegularExpressions/src/System/Text/RegularExpressions/RegexCharClass.cs
//   dotnet/runtime v10.0.14
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Accept or reject a .NET regular-expression pattern.
//!
//! [`parse`] follows `RegexParser.Parse` for `RegexOptions.None` on the .NET
//! 10.0.14 runtime: a pattern that runtime accepts parses, and a pattern it
//! rejects produces the same `RegexParseException` text. The capture-group
//! tree, the character-class set, matching, replacement and the match timeout
//! are later rows. A null pattern is not representable here; the script wrapper
//! turns a null pattern into an empty one before it compiles.

use std::fmt;

mod names;
mod parse;

use parse::Parser;

/// Most UTF-16 code units `parse` will read. .NET has no pattern-length cap.
pub const PATTERN_CHAR_LIMIT: u32 = 65_536;

/// Most nested class subtractions `parse` will walk. .NET recurses until the
/// stack runs out.
pub const CLASS_DEPTH_LIMIT: u32 = 64;

/// A pattern [`parse`] accepted. Group slots are not part of this row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parsed {
    utf16_units: usize,
}

impl Parsed {
    /// UTF-16 code units in the pattern. Offsets in a parse error use the same
    /// index .NET uses, which is not a count of Unicode scalar values.
    #[must_use]
    pub fn utf16_units(self) -> usize {
        self.utf16_units
    }
}

/// Why [`parse`] refused a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The text `new Regex(pattern)` puts in `RegexParseException.Message`.
    Invalid { message: String },
    /// A Ferroan bound. This is not a .NET message.
    Limit { id: &'static str },
    /// The parser lost an invariant. A pattern the runtime accepts or rejects
    /// does not produce this.
    Internal,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Invalid { message } => f.write_str(message),
            ParseError::Limit { id } => write!(f, "limit {id}"),
            ParseError::Internal => f.write_str("regex parser fault"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse `pattern` the way `new Regex(pattern)` does under `RegexOptions.None`.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] with .NET's message, [`ParseError::Limit`]
/// when the pattern is longer than [`PATTERN_CHAR_LIMIT`] UTF-16 code units or
/// a class subtraction nests deeper than [`CLASS_DEPTH_LIMIT`], or
/// [`ParseError::Internal`] if the walk cannot continue.
pub fn parse(pattern: &str) -> Result<Parsed, ParseError> {
    let observed = u64::try_from(pattern.encode_utf16().count()).unwrap_or(u64::MAX);
    if observed > u64::from(PATTERN_CHAR_LIMIT) {
        ferroan_diag::refuse(
            ferroan_diag::REGEX_PATTERN_CHARS,
            ferroan_diag::Kind::Hard,
            u64::from(PATTERN_CHAR_LIMIT),
            observed,
            "regex.parse",
        );
        return Err(ParseError::Limit {
            id: ferroan_diag::REGEX_PATTERN_CHARS,
        });
    }
    let units: Vec<u16> = pattern.encode_utf16().collect();
    let mut parser = Parser::new(pattern, units);
    parser.parse()?;
    Ok(Parsed {
        utf16_units: parser.utf16_len(),
    })
}
