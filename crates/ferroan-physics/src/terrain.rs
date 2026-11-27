// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs
//   KanikaBreed/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4.
// This port remains available under MPL-2.0 and forms part of Ferroan's
// GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! The padded heightfield `JoltScene.SetTerrain` stores before it cooks.
//!
//! The region map is `height * width` floats in order `y * width + x`. The
//! stored field is `max(width, height) + 1` on a side, so a 256 m region
//! becomes 257 samples and the far edge is a copy of the last real sample.
//! A non-square map repeats the last row or column until that square is
//! full. This is the array the method keeps. It is not the cooked surface
//! and it is not `TerrainHeightAt`.
//!
//! A side past [`REGION_EDGE_M`] is `terrain.edge`. The square is not
//! reserved. Reservation of an accepted square is fallible.

use ferroan_diag::{Kind, TERRAIN_EDGE};

/// `Constants.MaximumRegionSize`. `docs/LIMITS.md` lists this as `terrain.edge`.
const REGION_EDGE_M: i32 = 4096;

/// Pad `samples` the way `SetTerrain` does.
///
/// `width` and `height` are the region sizes in metres. `samples` holds at
/// least `height * width` floats. A non-positive size or a short map returns
/// [`None`]. A side past the 4096 m region edge reports `terrain.edge` and
/// returns [`None`] before the square is reserved. A square that does not fit
/// in memory returns [`None`]. Sample bits are copied, including `-0`,
/// infinities and a NaN payload.
pub fn pad_heightfield(width: i32, height: i32, samples: &[f32]) -> Option<Vec<f32>> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let long = width.max(height);
    if long > REGION_EDGE_M {
        ferroan_diag::refuse(
            TERRAIN_EDGE,
            Kind::Hard,
            u64::try_from(REGION_EDGE_M).unwrap_or(u64::MAX),
            u64::try_from(long).unwrap_or(u64::MAX),
            "physics.heightfield",
        );
        return None;
    }
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    let need = width.checked_mul(height)?;
    if samples.len() < need {
        return None;
    }
    let m = width.max(height).checked_add(1)?;
    let cells = m.checked_mul(m)?;
    let mut field = Vec::new();
    field.try_reserve_exact(cells).ok()?;
    for y in 0..m {
        let src_y = y.min(height - 1);
        let src_row = src_y.checked_mul(width)?;
        for x in 0..m {
            let src_x = x.min(width - 1);
            let sample = samples.get(src_row.checked_add(src_x)?)?;
            field.push(*sample);
        }
    }
    if field.len() != cells {
        return None;
    }
    Some(field)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{REGION_EDGE_M, pad_heightfield};

    #[test]
    fn padded_samples_match_set_terrain() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let text =
            std::fs::read_to_string(root.join("jolt-file/heightfield/field.txt")).expect("field");
        let config =
            std::fs::read_to_string(root.join("jolt-record/drop/config.txt")).expect("config");
        let provenance = std::fs::read_to_string(root.join("jolt-record/drop/provenance.txt"))
            .expect("provenance");
        assert_eq!(line(&text, "stock_gravity "), line(&config, "Gravity "));
        assert_eq!(
            line(&text, "stock_prim_linear_damping "),
            line(&config, "PrimLinearDamping ")
        );
        assert_eq!(
            line(&text, "stock_prim_angular_damping "),
            line(&config, "PrimAngularDamping ")
        );
        assert_eq!(line(&text, "stock_dt_f64 "), line(&provenance, "dt_f64 "));
        assert_eq!(
            line(&text, "stock_simulate_dt_f32 "),
            line(&provenance, "simulate_dt_f32 ")
        );
        assert_eq!(
            line(&text, "compiled_gravity "),
            line(&text, "stock_gravity ")
        );
        assert_eq!(
            line(&text, "compiled_prim_linear_damping "),
            line(&text, "stock_prim_linear_damping ")
        );
        assert_eq!(
            line(&text, "compiled_prim_angular_damping "),
            line(&text, "stock_prim_angular_damping ")
        );
        assert_eq!(
            line(&text, "compiled_dt_f64 "),
            line(&text, "stock_dt_f64 ")
        );
        assert_eq!(
            line(&text, "compiled_simulate_dt_f32 "),
            line(&text, "stock_simulate_dt_f32 ")
        );
        assert_eq!(line(&text, "stock_match "), "true");
        assert_eq!(line(&text, "method "), "SetTerrain");
        assert_eq!(line(&text, "source "), "JoltScene.cs 3552-3613");
        assert_eq!(line(&text, "layout "), "y*m+x");

        let cases = cases(&text);
        let names: Vec<&str> = cases.iter().map(|case| case.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "square-1",
                "square-3",
                "rect-5x3",
                "rect-2x4",
                "square-256",
                "extra-tail",
                "special-bits",
                "reject-short",
                "reject-null",
                "reject-zero-x",
                "reject-zero-y",
            ]
        );
        for case in &cases {
            if case.digest.is_some() {
                assert_digest(case);
                continue;
            }
            match &case.field {
                None => {
                    assert!(!case.cooked, "{}", case.name);
                    assert_eq!(case.calls, 0, "{}", case.name);
                    let samples = case.input.as_deref().unwrap_or(&[]);
                    assert!(pad_heightfield(case.width, case.height, samples).is_none());
                }
                Some(field) => {
                    assert!(case.cooked, "{}", case.name);
                    assert_eq!(case.calls, 1, "{}", case.name);
                    assert_eq!(case.scale, Some([0x3F80_0000, 0x3F80_0000, 0x3F80_0000]));
                    assert_eq!(case.position, Some([0, 0, 0]));
                    let samples = case.input.as_deref().expect("cooked input");
                    let padded = pad_heightfield(case.width, case.height, samples).expect("pad");
                    let bits: Vec<u32> = padded.iter().map(|sample| sample.to_bits()).collect();
                    assert_eq!(&bits, field, "{}", case.name);
                    assert_far_edge(case, field);
                }
            }
        }
        let wide = cases
            .iter()
            .find(|case| case.name == "square-256")
            .expect("256");
        assert_eq!(
            (wide.width, wide.height, wide.field_m),
            (256, 256, Some(257))
        );
        assert_eq!(wide.field_count, Some(257 * 257));
        assert!(wide.field.is_none());
        let special = cases
            .iter()
            .find(|case| case.name == "special-bits")
            .expect("bits");
        let input = special.input.as_ref().expect("bits input");
        assert_eq!(
            input
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>(),
            [0x8000_0000, 0x7F80_0000, 0xFF80_0000, 0x7FC0_0001]
        );
        let tail = cases
            .iter()
            .find(|case| case.name == "extra-tail")
            .expect("tail");
        let tail_input = tail.input.as_ref().expect("tail input");
        assert_eq!(tail_input.len(), 6);
        assert_eq!(tail_input[4].to_bits(), 0x42F6_0000);
        assert_eq!(tail_input[5].to_bits(), 0xC2F6_0000);
        let tail_field = tail.field.as_ref().expect("tail field");
        assert!(!tail_field.contains(&0x42F6_0000));
        assert!(!tail_field.contains(&0xC2F6_0000));
    }

    #[test]
    fn a_rectangle_past_the_region_edge_is_refused() {
        let _guard = limits_lock();
        let before = limit_hits(ferroan_diag::TERRAIN_EDGE);
        let samples = vec![1.0f32; 4097 * 2];
        assert!(pad_heightfield(4097, 2, &samples).is_none());
        assert!(pad_heightfield(3, 4097, &samples).is_none());
        assert_eq!(limit_hits(ferroan_diag::TERRAIN_EDGE), before + 2);
        assert!(pad_heightfield(0, 4, &[]).is_none());
        assert_eq!(limit_hits(ferroan_diag::TERRAIN_EDGE), before + 2);
    }

    #[test]
    fn the_region_edge_is_accepted() {
        assert_eq!(REGION_EDGE_M, 4096);
        let mut samples = vec![0.0f32; 4096];
        samples[0] = f32::from_bits(0x4000_0000);
        samples[4095] = f32::from_bits(0x4000_0FFF);
        let padded = pad_heightfield(4096, 1, &samples).expect("edge");
        let m = 4097;
        assert_eq!(padded.len(), m * m);
        assert_eq!(padded[0].to_bits(), 0x4000_0000);
        assert_eq!(padded[4095].to_bits(), 0x4000_0FFF);
        assert_eq!(padded[4096].to_bits(), 0x4000_0FFF);
        assert_eq!(padded[(m - 1) * m].to_bits(), 0x4000_0000);
        assert_eq!(padded[(m - 1) * m + (m - 1)].to_bits(), 0x4000_0FFF);
    }

    fn assert_digest(case: &Case) {
        assert!(case.cooked, "{}", case.name);
        assert_eq!(case.calls, 1, "{}", case.name);
        assert_eq!(case.formula.as_deref(), Some("origin-plus-index"));
        assert_eq!(case.byte_order.as_deref(), Some("little-endian-float32"));
        let origin = case.origin.expect("origin");
        let count = case.input_count.expect("count");
        assert_eq!(count, 256 * 256, "{}", case.name);
        let samples: Vec<f32> = (0..count)
            .map(|index| f32::from_bits(origin.wrapping_add(u32::try_from(index).expect("index"))))
            .collect();
        let padded = pad_heightfield(case.width, case.height, &samples).expect("pad");
        let m = usize::try_from(case.field_m.expect("m")).expect("m");
        assert_eq!(padded.len(), m * m, "{}", case.name);
        assert_eq!(Some(padded.len()), case.field_count);
        let mut bytes = Vec::with_capacity(padded.len() * 4);
        for sample in &padded {
            bytes.extend_from_slice(&sample.to_bits().to_le_bytes());
        }
        let digest = hex(&Sha256::digest(&bytes));
        assert_eq!(digest, case.digest.as_deref().expect("digest"));
        assert_eq!(case.points.len(), 4, "{}", case.name);
        for point in &case.points {
            assert_eq!(
                padded[point.y * m + point.x].to_bits(),
                point.bits,
                "{} {},{}",
                case.name,
                point.x,
                point.y
            );
        }
        let far = 0x4000_0000 + 255 * 256 + 255;
        assert_eq!(point_bits(case, 256, 256), far);
        assert_eq!(point_bits(case, 256, 0), 0x4000_00FF);
        assert_eq!(point_bits(case, 0, 256), 0x4000_FF00);
        assert_eq!(point_bits(case, 0, 0), 0x4000_0000);
    }

    fn point_bits(case: &Case, x: usize, y: usize) -> u32 {
        case.points
            .iter()
            .find(|point| point.x == x && point.y == y)
            .expect("point")
            .bits
    }

    fn assert_far_edge(case: &Case, field: &[u32]) {
        let width = usize::try_from(case.width).expect("width");
        let height = usize::try_from(case.height).expect("height");
        let m = usize::try_from(case.field_m.expect("m")).expect("m");
        assert_eq!(m, width.max(height) + 1, "{}", case.name);
        assert_eq!(field.len(), m * m, "{}", case.name);
        let input = case.input.as_ref().expect("input");
        for y in 0..m {
            let src_y = y.min(height - 1);
            let copied = input[src_y * width + (width - 1)].to_bits();
            assert_eq!(field[y * m + (m - 1)], copied, "{} column {y}", case.name);
        }
        for x in 0..m {
            let src_x = x.min(width - 1);
            let copied = input[(height - 1) * width + src_x].to_bits();
            assert_eq!(field[(m - 1) * m + x], copied, "{} row {x}", case.name);
        }
    }

    fn line<'a>(text: &'a str, key: &str) -> &'a str {
        text.lines()
            .find_map(|row| row.strip_prefix(key))
            .unwrap_or_else(|| panic!("missing {key}"))
    }

    struct Point {
        x: usize,
        y: usize,
        bits: u32,
    }

    struct Case {
        name: String,
        width: i32,
        height: i32,
        cooked: bool,
        calls: u32,
        field_m: Option<i32>,
        scale: Option<[u32; 3]>,
        position: Option<[u32; 3]>,
        formula: Option<String>,
        origin: Option<u32>,
        input_count: Option<usize>,
        byte_order: Option<String>,
        field_count: Option<usize>,
        digest: Option<String>,
        points: Vec<Point>,
        input: Option<Vec<f32>>,
        field: Option<Vec<u32>>,
    }

    fn cases(text: &str) -> Vec<Case> {
        let mut lines = text.lines().peekable();
        let mut out = Vec::new();
        while let Some(row) = lines.next() {
            let Some(name) = row.strip_prefix("case ") else {
                continue;
            };
            let mut case = Case {
                name: name.to_string(),
                width: 0,
                height: 0,
                cooked: false,
                calls: u32::MAX,
                field_m: None,
                scale: None,
                position: None,
                formula: None,
                origin: None,
                input_count: None,
                byte_order: None,
                field_count: None,
                digest: None,
                points: Vec::new(),
                input: None,
                field: None,
            };
            while let Some(row) = lines.next() {
                if row.is_empty() {
                    break;
                }
                if row.starts_with("case ") {
                    panic!("case {} ended early at {row}", case.name);
                }
                if let Some(rest) = row.strip_prefix("region ") {
                    let mut parts = rest.split(' ');
                    case.width = parts.next().unwrap().parse().unwrap();
                    case.height = parts.next().unwrap().parse().unwrap();
                } else if let Some(rest) = row.strip_prefix("cooked ") {
                    case.cooked = rest == "true";
                } else if let Some(rest) = row.strip_prefix("heightfield_calls ") {
                    case.calls = rest.parse().unwrap();
                } else if let Some(rest) = row.strip_prefix("field_m ") {
                    case.field_m = Some(rest.parse().unwrap());
                } else if let Some(rest) = row.strip_prefix("scale ") {
                    case.scale = Some(hex3(rest));
                } else if let Some(rest) = row.strip_prefix("position ") {
                    case.position = Some(hex3(rest));
                } else if row == "input null" {
                    case.input = None;
                } else if let Some(rest) = row.strip_prefix("input_formula ") {
                    case.formula = Some(rest.to_string());
                } else if let Some(rest) = row.strip_prefix("input_origin ") {
                    case.origin = Some(u32::from_str_radix(rest, 16).unwrap());
                } else if let Some(rest) = row.strip_prefix("input_count ") {
                    let count: usize = rest.parse().unwrap();
                    if lines.peek() == Some(&"input") {
                        lines.next();
                        case.input = Some(read_floats(&mut lines, count));
                    } else {
                        case.input_count = Some(count);
                    }
                } else if let Some(rest) = row.strip_prefix("field_encoding ") {
                    assert_eq!(rest, "digest");
                } else if let Some(rest) = row.strip_prefix("byte_order ") {
                    case.byte_order = Some(rest.to_string());
                } else if let Some(rest) = row.strip_prefix("field_count ") {
                    let count: usize = rest.parse().unwrap();
                    if lines.peek() == Some(&"field") {
                        lines.next();
                        case.field = Some(read_bits(&mut lines, count));
                        break;
                    }
                    case.field_count = Some(count);
                } else if let Some(rest) = row.strip_prefix("field_sha256 ") {
                    case.digest = Some(rest.to_string());
                } else if let Some(rest) = row
                    .strip_prefix("corner ")
                    .or_else(|| row.strip_prefix("edge "))
                {
                    let mut parts = rest.split(' ');
                    let x: usize = parts.next().unwrap().parse().unwrap();
                    let y: usize = parts.next().unwrap().parse().unwrap();
                    let bits = u32::from_str_radix(parts.next().unwrap(), 16).unwrap();
                    assert!(parts.next().is_none());
                    case.points.push(Point { x, y, bits });
                } else if row == "field null" {
                    case.field = None;
                    break;
                } else if row.starts_with("terrain_sets ")
                    || row.starts_with("sample_count ")
                    || row.starts_with("shape ")
                    || row.starts_with("stored_matches_span ")
                {
                } else {
                    panic!("case {} unexpected {row}", case.name);
                }
            }
            assert_ne!(case.calls, u32::MAX, "{}", case.name);
            out.push(case);
        }
        out
    }

    fn read_floats<'a>(lines: &mut impl Iterator<Item = &'a str>, count: usize) -> Vec<f32> {
        read_bits(lines, count)
            .into_iter()
            .map(f32::from_bits)
            .collect()
    }

    fn read_bits<'a>(lines: &mut impl Iterator<Item = &'a str>, count: usize) -> Vec<u32> {
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            let row = lines.next().expect("sample");
            values.push(u32::from_str_radix(row, 16).expect("hex"));
        }
        values
    }

    fn hex3(text: &str) -> [u32; 3] {
        let mut parts = text.split(' ');
        let mut out = [0; 3];
        for slot in &mut out {
            *slot = u32::from_str_radix(parts.next().unwrap(), 16).unwrap();
        }
        assert!(parts.next().is_none());
        out
    }

    fn hex(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }

    fn limit_hits(id: &str) -> u64 {
        let needle = format!("ferroan_limit_hits_total{{limit=\"{id}\",action=\"refuse\"}} ");
        ferroan_diag::metrics()
            .lines()
            .find_map(|line| line.strip_prefix(&needle)?.parse().ok())
            .unwrap_or(0)
    }

    fn limits_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}
