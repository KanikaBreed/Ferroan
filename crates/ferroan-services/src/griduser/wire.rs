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
//   Source/OpenSim.Services.Interfaces/IGridUserService.cs
//   Source/OpenSim.Services.UserAccountService/GridUserService.cs
//   Source/OpenSim.Framework/Util.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grid-user values on the wire.
//!
//! A document reformats the stored row: region ids are hyphenated, positions
//! use `<x, y, z>` in the en-US general spelling, online is `True` or `False`,
//! and login and logout are the invariant general date (D37). A value that
//! does not parse becomes the zero region, the zero vector, offline, or the
//! epoch. `∞` and `-∞` parse as infinities.

use ferroan_http::{Field, FieldMap, build_xml_response};
use uuid::Uuid;

use super::model::GridUser;

pub const ZERO_REGION: &str = "00000000-0000-0000-0000-000000000000";

/// `Util.DocToBytes` of `<result>Failure</result>`.
pub const FAILURE_XML: &[u8] =
    b"<?xml version=\"1.0\"?>\n<ServerResponse>\n  <result>Failure</result>\n</ServerResponse>";

/// `Util.DocToBytes` of `<result>Success</result>`.
pub const SUCCESS_XML: &[u8] =
    b"<?xml version=\"1.0\"?>\n<ServerResponse>\n  <result>Success</result>\n</ServerResponse>";

const DEFAULT_X: f32 = 128.0;
const DEFAULT_Y: f32 = 128.0;
const DEFAULT_Z: f32 = 70.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const DEFAULT_POSITION: Self = Self {
        x: DEFAULT_X,
        y: DEFAULT_Y,
        z: DEFAULT_Z,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridUserInfo {
    pub user_id: String,
    pub home_region: String,
    pub home_position: String,
    pub home_look: String,
    pub last_region: String,
    pub last_position: String,
    pub last_look: String,
    pub online: bool,
    pub login: i32,
    pub logout: i32,
}

pub fn to_info(row: &GridUser) -> GridUserInfo {
    GridUserInfo {
        user_id: row.user_id.clone(),
        home_region: region_text(row.field("HomeRegionID")),
        home_position: vector_text(row.field("HomePosition")),
        home_look: vector_text(row.field("HomeLookAt")),
        last_region: region_text(row.field("LastRegionID")),
        last_position: vector_text(row.field("LastPosition")),
        last_look: vector_text(row.field("LastLookAt")),
        online: row.field("Online").and_then(parse_bool).unwrap_or(false),
        login: row.field("Login").and_then(parse_i32).unwrap_or(0),
        logout: row.field("Logout").and_then(parse_i32).unwrap_or(0),
    }
}

pub fn info_document(info: &GridUserInfo) -> Vec<u8> {
    let mut outer = FieldMap::new();
    outer.insert("result".to_string(), Field::Map(info_fields(info)));
    document(&outer)
}

pub fn infos_document(infos: &[Option<GridUserInfo>]) -> Vec<u8> {
    let mut outer = FieldMap::new();
    if infos.is_empty() {
        outer.insert("result".to_string(), Field::Str("null".to_string()));
        return document(&outer);
    }
    let mut index = 0_u32;
    for info in infos {
        let Some(info) = info else {
            continue;
        };
        outer.insert(format!("griduser{index}"), Field::Map(info_fields(info)));
        index = index.saturating_add(1);
    }
    document(&outer)
}

pub fn parse_uuid(text: &str) -> Option<Uuid> {
    let trimmed = text.trim();
    let bare = trimmed
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(trimmed);
    if let Ok(id) = Uuid::parse_str(bare) {
        return Some(id);
    }
    if bare.len() == 32 && bare.chars().all(|ch| ch.is_ascii_hexdigit()) {
        let hyphenated = format!(
            "{}-{}-{}-{}-{}",
            &bare[0..8],
            &bare[8..12],
            &bare[12..16],
            &bare[16..20],
            &bare[20..32]
        );
        return Uuid::parse_str(&hyphenated).ok();
    }
    None
}

pub fn parse_vector(text: &str) -> Option<Vec3> {
    let inner = text.trim().trim_matches(|ch| ch == '<' || ch == '>');
    let mut parts = inner.split(',');
    let x = parse_coordinate(parts.next()?.trim())?;
    let y = parse_coordinate(parts.next()?.trim())?;
    let z = parse_coordinate(parts.next()?.trim())?;
    if parts.next().is_some() {
        return None;
    }
    Some(Vec3 { x, y, z })
}

fn parse_coordinate(text: &str) -> Option<f32> {
    match text {
        "∞" | "+∞" => Some(f32::INFINITY),
        "-∞" => Some(f32::NEG_INFINITY),
        _ => text.parse().ok(),
    }
}

pub fn format_vector(value: Vec3) -> String {
    format!(
        "<{}, {}, {}>",
        format_f32(value.x),
        format_f32(value.y),
        format_f32(value.z)
    )
}

pub fn parse_bool(text: &str) -> Option<bool> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

pub fn parse_i32(text: &str) -> Option<i32> {
    text.trim().parse().ok()
}

/// Invariant general date, the same spelling as the account console.
pub fn unix_general(seconds: i32) -> String {
    let (year, month, day, hour, minute, second) = clock_parts(seconds);
    format!("{month:02}/{day:02}/{year:04} {hour:02}:{minute:02}:{second:02}")
}

/// en-US general date. The hour has no leading zero, and midnight is 12.
/// This spelling uses a normal space before `AM`/`PM`. The captured process
/// culture uses a narrow no-break space there. The fixture fold accepts both.
#[cfg(test)]
pub fn en_us_general(seconds: i32) -> String {
    let (year, month, day, hour, minute, second) = clock_parts(seconds);
    let (hour12, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{month}/{day}/{year} {hour12}:{minute:02}:{second:02} {suffix}")
}

fn clock_parts(seconds: i32) -> (i32, u32, u32, i32, i32, i32) {
    let days = i64::from(seconds.div_euclid(86_400));
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = time / 3600;
    let minute = (time % 3600) / 60;
    let second = time % 60;
    (year, month, day, hour, minute, second)
}

fn info_fields(info: &GridUserInfo) -> FieldMap {
    let mut fields = FieldMap::new();
    fields.insert("UserID".to_string(), Field::Str(info.user_id.clone()));
    fields.insert(
        "HomeRegionID".to_string(),
        Field::Str(info.home_region.clone()),
    );
    fields.insert(
        "HomePosition".to_string(),
        Field::Str(info.home_position.clone()),
    );
    fields.insert("HomeLookAt".to_string(), Field::Str(info.home_look.clone()));
    fields.insert(
        "LastRegionID".to_string(),
        Field::Str(info.last_region.clone()),
    );
    fields.insert(
        "LastPosition".to_string(),
        Field::Str(info.last_position.clone()),
    );
    fields.insert("LastLookAt".to_string(), Field::Str(info.last_look.clone()));
    fields.insert(
        "Online".to_string(),
        Field::Str(if info.online {
            "True".to_string()
        } else {
            "False".to_string()
        }),
    );
    fields.insert("Login".to_string(), Field::Str(unix_general(info.login)));
    fields.insert("Logout".to_string(), Field::Str(unix_general(info.logout)));
    fields
}

fn region_text(value: Option<&str>) -> String {
    value
        .and_then(parse_uuid)
        .map(|id| id.to_string())
        .unwrap_or_else(|| ZERO_REGION.to_string())
}

fn vector_text(value: Option<&str>) -> String {
    value
        .and_then(parse_vector)
        .map(format_vector)
        .unwrap_or_else(|| format_vector(Vec3::ZERO))
}

fn document(fields: &FieldMap) -> Vec<u8> {
    build_xml_response(fields)
        .map(|text| text.into_bytes())
        .unwrap_or_else(|_| FAILURE_XML.to_vec())
}

/// en-US general spelling, including non-finite coordinates.
pub fn format_f32(value: f32) -> String {
    super::float_text::format_f32(value)
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day as u32)
}
