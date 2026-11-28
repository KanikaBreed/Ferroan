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
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptCodeGen.cs
//     global slot walk and WriteOutGblAssignment
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptCompValu.cs
//     CompValuGlobalVar
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptTokenize.cs
//     AssignVarSlot for integer, float, string, key, vector, rotation and list
//   Source/OpenSim.Region.ScriptEngine.YEngine/MMRScriptReduce.cs
//     top-level variable, function and constant forms that fill variablesStack
//   OpenSim-NGC/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Slots for ordinary global variables.
//!
//! The walk is the one in `MMRScriptCodeGen` over `variablesStack`: a `constant`
//! is skipped, a function (`retType != null`) is skipped, and a plain variable
//! takes the next slot of its type array. `CompValuGlobalVar` calls
//! `AssignVarSlot` only when the declaration is not a property. `string` and
//! `key` share `iarStrings`. `quaternion` has already been spelled `rotation`.
//! A script-defined static field uses that same call, and this corpus has none:
//! that script is a later refusal, and the token stream has no class members.

use crate::object::Global;
use crate::tokenize::Token;

#[derive(Default)]
struct Counters {
    integers: i32,
    floats: i32,
    strings: i32,
    vectors: i32,
    rotations: i32,
    lists: i32,
}

/// Assign the slots `globalVarNames` records for ordinary globals in `tokens`.
#[must_use]
pub fn assign_global_slots(tokens: &[Token]) -> Vec<Global> {
    let mut counters = Counters::default();
    let mut globals = Vec::new();
    let mut index = 0;
    let mut braces = 0i32;
    let mut parens = 0i32;
    while index < tokens.len() {
        let kind = classify(&tokens[index]);
        match kind {
            Kind::OpenBrace => braces = braces.saturating_add(1),
            Kind::CloseBrace => braces = braces.saturating_sub(1),
            Kind::OpenParen => parens = parens.saturating_add(1),
            Kind::CloseParen => parens = parens.saturating_sub(1),
            _ => {}
        }
        if braces != 0 || parens != 0 {
            index += 1;
            continue;
        }
        if kind == Kind::Constant {
            index = skip_statement(tokens, index + 1);
            continue;
        }
        let Kind::Variable(type_name) = kind else {
            index += 1;
            continue;
        };
        let Some(name) = name_at(tokens, index + 1) else {
            index += 1;
            continue;
        };
        match sym_at(tokens, index + 2) {
            Some(";") => {
                push(&mut globals, &mut counters, &name, type_name);
                index += 3;
            }
            Some("=") => {
                push(&mut globals, &mut counters, &name, type_name);
                index = skip_statement(tokens, index + 3);
            }
            _ => index += 1,
        }
    }
    globals
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    Constant,
    Variable(&'static str),
    Other,
}

fn classify(token: &Token) -> Kind {
    match token {
        Token::Sym("{") => Kind::OpenBrace,
        Token::Sym("}") => Kind::CloseBrace,
        Token::Sym("(") => Kind::OpenParen,
        Token::Sym(")") => Kind::CloseParen,
        Token::Sym("constant") => Kind::Constant,
        Token::Sym("integer") => Kind::Variable("iarIntegers"),
        Token::Sym("float") => Kind::Variable("iarFloats"),
        Token::Sym("string" | "key") => Kind::Variable("iarStrings"),
        Token::Sym("vector") => Kind::Variable("iarVectors"),
        Token::Sym("rotation") => Kind::Variable("iarRotations"),
        Token::Sym("list") => Kind::Variable("iarLists"),
        _ => Kind::Other,
    }
}

fn name_at(tokens: &[Token], index: usize) -> Option<String> {
    match tokens.get(index) {
        Some(Token::Name(name)) => Some(name.clone()),
        _ => None,
    }
}

fn sym_at(tokens: &[Token], index: usize) -> Option<&'static str> {
    match tokens.get(index) {
        Some(Token::Sym(text)) => Some(*text),
        _ => None,
    }
}

fn push(globals: &mut Vec<Global>, counters: &mut Counters, name: &str, type_name: &'static str) {
    let index = match type_name {
        "iarIntegers" => next(&mut counters.integers),
        "iarFloats" => next(&mut counters.floats),
        "iarStrings" => next(&mut counters.strings),
        "iarVectors" => next(&mut counters.vectors),
        "iarRotations" => next(&mut counters.rotations),
        "iarLists" => next(&mut counters.lists),
        _ => return,
    };
    let Some(index) = index else {
        return;
    };
    globals.push(Global {
        name: name.to_string(),
        type_name: type_name.to_string(),
        index,
    });
}

fn next(slot: &mut i32) -> Option<i32> {
    let current = *slot;
    *slot = slot.checked_add(1)?;
    Some(current)
}

/// Move past the `;` that ends a top-level declaration, counting braces so a
/// later global is still visible.
fn skip_statement(tokens: &[Token], mut index: usize) -> usize {
    let mut braces = 0i32;
    while index < tokens.len() {
        match &tokens[index] {
            Token::Sym("{") => braces = braces.saturating_add(1),
            Token::Sym("}") => braces = braces.saturating_sub(1),
            Token::Sym(";") if braces == 0 => return index + 1,
            _ => {}
        }
        index += 1;
    }
    index
}
