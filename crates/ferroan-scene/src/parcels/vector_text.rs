/*
 * Copyright (c) 2006-2016, openmetaverse.co
 * All rights reserved.
 *
 * - Redistribution and use in source and binary forms, with or without
 *   modification, are permitted provided that the following conditions are met:
 *
 * - Redistributions of source code must retain the above copyright notice, this
 *   list of conditions and the following disclaimer.
 * - Neither the name of the openmetaverse.co nor the names
 *   of its contributors may be used to endorse or promote products derived from
 *   this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 * ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
 * LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
 * CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
 * SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
 * INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
 * CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
 * ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
 * POSSIBILITY OF SUCH DAMAGE.
 */
// Ported from OpenMetaverse.Types/{Vector2,Vector3,Quaternion}.cs at
// OpenSim-NGC/libopenmetaverse a3596616c1c8cf23d2cfb9c71e0b0b4dd36d7d6b.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Acceptance of the string components consumed by the parcel environment.

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Components {
    Pair,
    Triple,
    Rotation,
}

fn number(component: &str) -> bool {
    // NumberStyles.Float uses the en-US infinity symbol and NaN. The special
    // symbols accept Unicode whitespace, ordinary numbers use ASCII space.
    let symbol = component.trim();
    let unsigned = symbol.strip_prefix(['+', '-']).unwrap_or(symbol);
    if unsigned == "∞" || unsigned.eq_ignore_ascii_case("nan") {
        return true;
    }
    let value = component
        .trim_end_matches('\0')
        .trim_matches(|c| matches!(c, ' ' | '\t'..='\r'));
    value.bytes().any(|b| b.is_ascii_digit()) && value.parse::<f32>().is_ok()
}

pub(super) fn accepts(text: &str, kind: Components) -> bool {
    let minimum = match kind {
        Components::Pair => 3,
        Components::Triple => 5,
        Components::Rotation => 7,
    };
    if text.encode_utf16().take(minimum).count() < minimum {
        return false;
    }
    // The first opening bracket can be preceded by arbitrary text. Without a
    // bracket, parsing starts at the beginning of the string.
    let mut rest = match text.find(['<', ',']) {
        Some(i) if text.as_bytes()[i] == b'<' => &text[i + 1..],
        Some(_) => text,
        None => return false,
    };
    let commas = if kind == Components::Pair { 1 } else { 2 };
    for _ in 0..commas {
        let Some((component, next)) = rest.split_once(',') else {
            return false;
        };
        if !number(component) {
            return false;
        }
        rest = next;
    }
    if kind == Components::Rotation {
        let Some(i) = rest.find([',', '>']) else {
            return false;
        };
        if !number(&rest[..i]) {
            return false;
        }
        if rest.as_bytes()[i] == b'>' {
            return true;
        }
        rest = &rest[i + 1..];
    }
    // The original search starts one character into the last component.
    // Vector3 and Quaternion also stop at an ASCII space. Vector2 does not.
    let end = rest
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == '>' || (kind != Components::Pair && *c == ' '))
        .map_or(rest.len(), |(i, _)| i);
    number(&rest[..end])
}
