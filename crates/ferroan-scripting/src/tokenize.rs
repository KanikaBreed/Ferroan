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
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptTokenize.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! YEngine's raw token stream and the uppercase MD5 of each token's text.

use ferroan_diag::{Kind, YENGINE_NAME_CHARS, YENGINE_SOURCE_CHARS, YENGINE_STRING_CHARS, refuse};
use md5::{Digest, Md5};

use crate::error::Failure;
use crate::number::format_f64;

pub const SOURCE_CHARS: u64 = 1_048_576;
pub const MAX_NAME_LEN: usize = 255;
pub const MAX_STRING_LEN: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Name(String),
    Int(i32),
    Float(f64),
    Str(String),
    Char(u16),
    Sym(&'static str),
}

impl Token {
    pub fn text(&self) -> String {
        match self {
            Token::Name(value) => value.clone(),
            Token::Int(value) => value.to_string(),
            Token::Float(value) => format_f64(*value),
            Token::Str(value) => quote_string(value),
            Token::Char(value) => quote_char(*value),
            Token::Sym(value) => (*value).to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub tokens: Vec<Token>,
    pub warnings: Vec<String>,
}

pub fn tokenize(source: &str) -> Result<Stream, Failure> {
    let mut count = 0u64;
    for _ in source.encode_utf16() {
        count = count.saturating_add(1);
        if count > SOURCE_CHARS {
            refuse(
                YENGINE_SOURCE_CHARS,
                Kind::Hard,
                SOURCE_CHARS,
                count,
                "yengine.tokenize",
            );
            return Err(Failure::Limit {
                id: YENGINE_SOURCE_CHARS,
            });
        }
    }
    let units: Vec<u16> = source.encode_utf16().collect();
    let mut parser = Parser {
        units,
        i: 0,
        line: 0,
        bol: 0,
        tokens: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        opt: Options::default(),
    };
    parser.run();
    if !parser.errors.is_empty() {
        return Err(Failure::Invalid {
            messages: parser.errors,
        });
    }
    Ok(Stream {
        tokens: parser.tokens,
        warnings: parser.warnings,
    })
}

pub fn source_hash(stream: &Stream) -> String {
    let mut hasher = Md5::new();
    for token in &stream.tokens {
        hasher.update([0]);
        hasher.update(token.text().as_bytes());
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02X}"));
    }
    out
}

fn quote_string(val: &str) -> String {
    if !val.contains(['"', '\\', '\n', '\t']) {
        return format!("\"{val}\"");
    }
    let mut out = String::from("\"");
    for ch in val.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn quote_char(value: u16) -> String {
    match value {
        0x27 => "'\\''".to_string(),
        0x5c => "'\\\\'".to_string(),
        0x0a => "'\\n'".to_string(),
        0x09 => "'\\t'".to_string(),
        other => format!(
            "'{}'",
            char::from_u32(u32::from(other)).unwrap_or('\u{FFFD}')
        ),
    }
}

#[derive(Clone, Copy, Default)]
struct Options {
    arrays: bool,
    adv: bool,
    try_catch: bool,
    objects: bool,
    chars: bool,
    /// Stored by YEngine. It does not change the token text.
    #[allow(dead_code)]
    no_right: bool,
    dollars: bool,
}

struct Parser {
    units: Vec<u16>,
    i: usize,
    line: i32,
    bol: usize,
    tokens: Vec<Token>,
    errors: Vec<String>,
    warnings: Vec<String>,
    opt: Options,
}

enum FloatParse {
    Not,
    Value(f64, usize),
    /// The message is already recorded. Do not parse an integer.
    Stop(usize),
    Bad(String, usize),
}

impl Parser {
    fn run(&mut self) {
        while self.i < self.units.len() {
            let c = self.units[self.i];
            if c == u16::from(b'\n') {
                self.on_newline();
                self.i += 1;
                continue;
            }
            if c <= u16::from(b' ') {
                self.i += 1;
                continue;
            }
            if self.starts_with(self.i, "//") {
                while self.i < self.units.len() && self.units[self.i] != u16::from(b'\n') {
                    self.i += 1;
                }
                self.line += 1;
                self.bol = self.i + 1;
                self.i += 1;
                continue;
            }
            if self.starts_with(self.i, "/*") {
                self.i += 2;
                while self.i + 1 < self.units.len()
                    && !(self.units[self.i] == u16::from(b'*')
                        && self.units[self.i + 1] == u16::from(b'/'))
                {
                    if self.units[self.i] == u16::from(b'\n') {
                        self.line += 1;
                        self.bol = self.i + 1;
                    }
                    self.i += 1;
                }
                self.i += 1;
                self.i += 1;
                continue;
            }
            if is_digit(c) {
                let start = self.i;
                self.take_number(start, false);
                continue;
            }
            if c == u16::from(b'.')
                && self.i + 1 < self.units.len()
                && is_digit(self.units[self.i + 1])
            {
                let start = self.i;
                self.take_number(start, true);
                continue;
            }
            if c == u16::from(b'"') {
                self.string_token();
                continue;
            }
            if c == u16::from(b'\'') {
                self.char_token();
                continue;
            }
            if is_name_start(c, self.opt.dollars) {
                self.name_token();
                continue;
            }
            if c == u16::from(b';') && self.take_yoption() {
                self.i += 1;
                continue;
            }
            if let Some(text) = self.delim_at(self.i) {
                self.push(Token::Sym(text));
                self.i += text.len();
                continue;
            }
            let shown = char::from_u32(u32::from(c)).unwrap_or('\u{FFFD}');
            self.fail(format!("unknown character '{shown}'"));
            self.i += 1;
        }
    }

    fn take_number(&mut self, start: usize, dotted: bool) {
        match self.try_float(start) {
            FloatParse::Not => {
                if dotted {
                    self.i += 1;
                } else {
                    let (end, value, message) = self.try_int(start);
                    self.finish_int(end, value, message);
                }
            }
            FloatParse::Value(value, end) => {
                self.push(Token::Float(value));
                self.i = end;
            }
            FloatParse::Stop(end) => {
                self.i = end;
            }
            FloatParse::Bad(message, end) => {
                self.fail(message);
                self.i = end;
            }
        }
    }

    fn finish_int(&mut self, end: usize, value: Option<i32>, message: Option<String>) {
        if let Some(message) = message {
            self.fail(message);
        } else if let Some(value) = value {
            self.push(Token::Int(value));
        }
        if end == 0 {
            self.i += 1;
        } else {
            self.i = end;
        }
    }

    fn on_newline(&mut self) {
        self.line += 1;
        self.bol = self.i + 1;
        let n = self.tokens.len();
        if n >= 3
            && matches!(self.tokens[n - 1], Token::Str(_))
            && matches!(self.tokens[n - 2], Token::Int(_))
            && matches!(self.tokens[n - 3], Token::Sym("#"))
        {
            if let Token::Int(line) = self.tokens[n - 2] {
                self.line = line;
            }
            self.tokens.truncate(n - 3);
        }
    }

    fn string_token(&mut self) {
        let start = self.i;
        let mut backslash = false;
        let mut buf = Vec::new();
        let mut j = start;
        loop {
            j += 1;
            if j >= self.units.len() {
                break;
            }
            let mut c = self.units[j];
            if c == u16::from(b'\\') && !backslash {
                backslash = true;
                continue;
            }
            if c == u16::from(b'\n') {
                self.line += 1;
                self.bol = j + 1;
            } else {
                if !backslash && c == u16::from(b'"') {
                    break;
                }
                if backslash && c == u16::from(b'n') {
                    c = u16::from(b'\n');
                }
                if backslash && c == u16::from(b't') {
                    buf.extend_from_slice(&[b' ' as u16, b' ' as u16, b' ' as u16]);
                    c = u16::from(b' ');
                }
            }
            backslash = false;
            buf.push(c);
        }
        let span = j.saturating_sub(start);
        if span > MAX_STRING_LEN {
            let observed = u64::try_from(span).unwrap_or(u64::MAX);
            refuse(
                YENGINE_STRING_CHARS,
                Kind::Hard,
                u64::try_from(MAX_STRING_LEN).unwrap_or(u64::MAX),
                observed,
                "yengine.tokenize",
            );
            self.fail(format!("string too long, max {MAX_STRING_LEN}"));
        } else {
            self.push(Token::Str(String::from_utf16_lossy(&buf)));
        }
        self.i = j + 1;
    }

    fn char_token(&mut self) {
        let start = self.i;
        let mut backslash = false;
        let mut overflow = false;
        let mut underflow = true;
        let mut value = 0u16;
        let mut j = start;
        loop {
            j += 1;
            if j >= self.units.len() {
                break;
            }
            let mut c = self.units[j];
            if c == u16::from(b'\\') && !backslash {
                backslash = true;
                continue;
            }
            if c == u16::from(b'\n') {
                self.line += 1;
                self.bol = j + 1;
            } else {
                if !backslash && c == u16::from(b'\'') {
                    break;
                }
                if backslash && c == u16::from(b'n') {
                    c = u16::from(b'\n');
                }
                if backslash && c == u16::from(b't') {
                    c = u16::from(b'\t');
                }
            }
            backslash = false;
            overflow = !underflow;
            underflow = false;
            value = c;
        }
        if underflow || overflow {
            self.fail("character must be exactly one character".to_string());
        } else {
            self.push(Token::Char(value));
        }
        self.i = j + 1;
    }

    fn name_token(&mut self) {
        let start = self.i;
        let mut j = start;
        loop {
            j += 1;
            if j >= self.units.len() {
                break;
            }
            let c = self.units[j];
            if is_name_cont(c, self.opt.dollars) {
                continue;
            }
            break;
        }
        let span = j - start;
        if span > MAX_NAME_LEN {
            let observed = u64::try_from(span).unwrap_or(u64::MAX);
            refuse(
                YENGINE_NAME_CHARS,
                Kind::Hard,
                u64::try_from(MAX_NAME_LEN).unwrap_or(u64::MAX),
                observed,
                "yengine.tokenize",
            );
            self.fail(format!("name too long, max {MAX_NAME_LEN}"));
        } else {
            let raw = &self.units[start..j];
            let mut name = String::from_utf16_lossy(raw);
            if name == "quaternion" {
                name = "rotation".to_string();
            }
            if let Some(text) = spell(&name, self.opt) {
                self.push(Token::Sym(text));
            } else {
                self.push(Token::Name(name));
            }
        }
        self.i = j;
    }

    fn take_yoption(&mut self) -> bool {
        let Some(Token::Name(last)) = self.tokens.last().cloned() else {
            return false;
        };
        if last.eq_ignore_ascii_case("yoptions") {
            self.opt.adv = true;
            self.opt.try_catch = true;
            self.tokens.pop();
            return true;
        }
        if self.tokens.len() < 2 {
            return false;
        }
        let prev_is = matches!(&self.tokens[self.tokens.len() - 2], Token::Name(prev) if prev.eq_ignore_ascii_case("yoption"));
        if !prev_is {
            return false;
        }
        let opt = last;
        if opt.eq_ignore_ascii_case("allowall") {
            self.opt.arrays = true;
            self.opt.adv = true;
            self.opt.try_catch = true;
            self.opt.objects = true;
            self.opt.chars = true;
            self.opt.dollars = true;
        } else if opt.eq_ignore_ascii_case("arrays") {
            self.opt.arrays = true;
        } else if opt.eq_ignore_ascii_case("advflowctl") {
            self.opt.adv = true;
        } else if opt.eq_ignore_ascii_case("trycatch") {
            self.opt.try_catch = true;
        } else if opt.eq_ignore_ascii_case("objects") {
            self.opt.objects = true;
        } else if opt.eq_ignore_ascii_case("chars") {
            self.opt.chars = true;
        } else if opt.eq_ignore_ascii_case("norighttoleft") {
            self.opt.no_right = true;
        } else if opt.eq_ignore_ascii_case("dollarsigns") {
            self.opt.dollars = true;
        } else {
            self.warnings.push("unknown YOption".to_string());
        }
        self.tokens.truncate(self.tokens.len() - 2);
        true
    }

    fn note_float_overflow(&mut self, error: &mut bool) {
        if !*error {
            self.fail("floating exponent overflow".to_string());
        }
        *error = true;
    }

    fn try_float(&mut self, start: usize) -> FloatParse {
        let mut decimals = false;
        let mut error = false;
        let mut exponent: i32 = 0;
        let mut mantissa: u64 = 0;
        let mut j = start;
        while j < self.units.len() {
            let mut c = self.units[j];
            if is_digit(c) {
                let digit = u64::from(c - u16::from(b'0'));
                let m = mantissa.wrapping_mul(10).wrapping_add(digit);
                if m / 10 != mantissa {
                    if !decimals {
                        exponent = exponent.wrapping_add(1);
                    }
                } else {
                    mantissa = m;
                    if decimals {
                        exponent = exponent.wrapping_sub(1);
                    }
                }
                j += 1;
                continue;
            }
            if c == u16::from(b'.') {
                if decimals {
                    return FloatParse::Bad("more than one decimal point".to_string(), j);
                }
                decimals = true;
                j += 1;
                continue;
            }
            if c == u16::from(b'E') || c == u16::from(b'e') {
                j += 1;
                if j >= self.units.len() {
                    return FloatParse::Bad("floating exponent off end of source".to_string(), j);
                }
                c = self.units[j];
                let negexp = c == u16::from(b'-');
                if negexp || c == u16::from(b'+') {
                    j += 1;
                }
                let mut y: i32 = 0;
                let mut nulexp = true;
                while j < self.units.len() {
                    c = self.units[j];
                    if !is_digit(c) {
                        break;
                    }
                    let x = y
                        .wrapping_mul(10)
                        .wrapping_add(i32::from(c - u16::from(b'0')));
                    if x / 10 != y {
                        self.note_float_overflow(&mut error);
                    }
                    y = x;
                    nulexp = false;
                    j += 1;
                }
                if nulexp {
                    return FloatParse::Bad("bad or missing floating exponent".to_string(), j);
                }
                let x = if negexp {
                    let x = exponent.wrapping_sub(y);
                    if x > exponent {
                        self.note_float_overflow(&mut error);
                    }
                    x
                } else {
                    let x = exponent.wrapping_add(y);
                    if x < exponent {
                        self.note_float_overflow(&mut error);
                    }
                    x
                };
                exponent = x;
            }
            if c == u16::from(b'F') || c == u16::from(b'f') {
                j += 1;
                if j >= self.units.len() {
                    return FloatParse::Bad("f at end of source".to_string(), j);
                }
                c = self.units[j];
                if is_digit(c)
                    || c == u16::from(b'.')
                    || c == u16::from(b'E')
                    || c == u16::from(b'e')
                    || c == u16::from(b'F')
                    || c == u16::from(b'f')
                {
                    return FloatParse::Bad("Syntax error".to_string(), j);
                }
                break;
            }
            break;
        }
        if !decimals {
            return FloatParse::Not;
        }
        let mut f = mantissa as f64;
        if exponent != 0 && mantissa != 0 && !error {
            let mut exp = exponent;
            let mut f10 = 10.0f64;
            if exp < 0 {
                exp = exp.wrapping_neg();
                while exp > 0 {
                    if exp & 1 != 0 {
                        f /= f10;
                    }
                    exp /= 2;
                    f10 *= f10;
                }
            } else {
                while exp > 0 {
                    if exp & 1 != 0 {
                        f *= f10;
                    }
                    exp /= 2;
                    f10 *= f10;
                }
            }
        }
        if error {
            FloatParse::Stop(j)
        } else {
            FloatParse::Value(f, j)
        }
    }

    fn try_int(&self, start: usize) -> (usize, Option<i32>, Option<String>) {
        let mut base: u32 = 10;
        let mut error = false;
        let mut mantissa: u32 = 0;
        let mut j = start;
        while j < self.units.len() {
            let c = self.units[j];
            if is_digit(c) {
                let digit = u32::from(c - u16::from(b'0'));
                let m = mantissa.wrapping_mul(base).wrapping_add(digit);
                if m / base != mantissa {
                    error = true;
                }
                mantissa = m;
                j += 1;
                continue;
            }
            if base == 16 && (u16::from(b'A')..=u16::from(b'F')).contains(&c) {
                let digit = u32::from(c - u16::from(b'A')) + 10;
                let m = mantissa.wrapping_mul(base).wrapping_add(digit);
                if m / base != mantissa {
                    error = true;
                }
                mantissa = m;
                j += 1;
                continue;
            }
            if base == 16 && (u16::from(b'a')..=u16::from(b'f')).contains(&c) {
                let digit = u32::from(c - u16::from(b'a')) + 10;
                let m = mantissa.wrapping_mul(base).wrapping_add(digit);
                if m / base != mantissa {
                    error = true;
                }
                mantissa = m;
                j += 1;
                continue;
            }
            if (c == u16::from(b'x') || c == u16::from(b'X')) && mantissa == 0 && base == 10 {
                base = 16;
                j += 1;
                continue;
            }
            break;
        }
        if error {
            (j, None, Some("integer overflow".to_string()))
        } else {
            (j, Some(mantissa as i32), None)
        }
    }

    fn delim_at(&self, at: usize) -> Option<&'static str> {
        const DELIMS: &[&str] = &[
            "...", "&&&", "|||", "<<=", ">>=", "<=", ">=", "==", "!=", "++", "--", "&&", "||",
            "+=", "&=", "-=", "*=", "/=", "%=", "|=", "^=", "<<", ">>", "~", "!", "@", "%", "^",
            "&", "*", "(", ")", "-", "+", "=", "{", "}", "[", "]", ";", ":", "<", ">", ",", ".",
            "?", "/", "|", "#",
        ];
        DELIMS
            .iter()
            .copied()
            .find(|text| self.starts_with(at, text))
    }

    fn starts_with(&self, at: usize, text: &str) -> bool {
        let units: Vec<u16> = text.encode_utf16().collect();
        if at + units.len() > self.units.len() {
            return false;
        }
        self.units[at..at + units.len()] == units
    }

    fn push(&mut self, token: Token) {
        self.tokens.push(token);
    }

    fn fail(&mut self, message: String) {
        self.errors.push(message);
    }
}

fn is_digit(c: u16) -> bool {
    (u16::from(b'0')..=u16::from(b'9')).contains(&c)
}

fn is_name_start(c: u16, dollars: bool) -> bool {
    (u16::from(b'a')..=u16::from(b'z')).contains(&c)
        || (u16::from(b'A')..=u16::from(b'Z')).contains(&c)
        || c == u16::from(b'_')
        || (dollars && c == u16::from(b'$'))
}

fn is_name_cont(c: u16, dollars: bool) -> bool {
    is_name_start(c, dollars) || is_digit(c)
}

fn spell(name: &str, opt: Options) -> Option<&'static str> {
    let mapped = if name == "quaternion" {
        "rotation"
    } else {
        name
    };
    let basic = match mapped {
        "default" => Some("default"),
        "do" => Some("do"),
        "else" => Some("else"),
        "float" => Some("float"),
        "for" => Some("for"),
        "if" => Some("if"),
        "integer" => Some("integer"),
        "list" => Some("list"),
        "jump" => Some("jump"),
        "key" => Some("key"),
        "return" => Some("return"),
        "rotation" => Some("rotation"),
        "state" => Some("state"),
        "string" => Some("string"),
        "vector" => Some("vector"),
        "while" => Some("while"),
        _ => None,
    };
    if basic.is_some() {
        return basic;
    }
    if opt.arrays {
        let hit = match mapped {
            "array" => Some("array"),
            "foreach" => Some("foreach"),
            "in" => Some("in"),
            "is" => Some("is"),
            "object" => Some("object"),
            "undef" => Some("undef"),
            _ => None,
        };
        if hit.is_some() {
            return hit;
        }
    }
    if opt.adv {
        let hit = match mapped {
            "break" => Some("break"),
            "case" => Some("case"),
            "constant" => Some("constant"),
            "continue" => Some("continue"),
            "switch" => Some("switch"),
            _ => None,
        };
        if hit.is_some() {
            return hit;
        }
    }
    if opt.try_catch {
        let hit = match mapped {
            "catch" => Some("catch"),
            "exception" | "scriptexception" => Some("exception"),
            "finally" => Some("finally"),
            "throw" => Some("throw"),
            "try" => Some("try"),
            _ => None,
        };
        if hit.is_some() {
            return hit;
        }
    }
    if opt.objects {
        let hit = match mapped {
            "abstract" => Some("abstract"),
            "base" => Some("base"),
            "class" => Some("class"),
            "constructor" => Some("constructor"),
            "delegate" => Some("delegate"),
            "destructor" => Some("destructor"),
            "final" => Some("final"),
            "get" => Some("get"),
            "interface" => Some("interface"),
            "new" => Some("new"),
            "override" => Some("override"),
            "partial" => Some("partial"),
            "private" => Some("private"),
            "protected" => Some("protected"),
            "public" => Some("public"),
            "set" => Some("set"),
            "static" => Some("static"),
            "this" => Some("this"),
            "typedef" => Some("typedef"),
            "virtual" => Some("virtual"),
            _ => None,
        };
        if hit.is_some() {
            return hit;
        }
    }
    if opt.chars && mapped == "char" {
        return Some("char");
    }
    None
}
