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

//! The analytic box, sphere and cylinder `CookPrimShape` builds before the mesher.
//!
//! `JoltScene.cs` 1229–1259. An uncut square on a straight path is a box.
//! An uncut half-circle on curve1 is a sphere when the three sides match.
//! An uncut circle on a straight path is a cylinder when X matches Y, and
//! the axis correction is +90 degrees about X. A cut, a non-uniform sphere
//! and an elliptical cylinder are not analytic. Mesh and sculpt parts are
//! not inputs: the method rejects them before this test, and this row does
//! not call a mesher. The bounding-box fallback is not an analytic shape.

/// Profile and path cut fields `PrimHasNoCuts` reads (`JoltScene.cs` 2224–2228).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrimCuts {
    /// `ProfileBegin`.
    pub profile_begin: u16,
    /// `ProfileEnd`.
    pub profile_end: u16,
    /// `ProfileHollow`.
    pub profile_hollow: u16,
    /// `PathTwist`.
    pub path_twist: i8,
    /// `PathTwistBegin`.
    pub path_twist_begin: i8,
    /// `PathBegin`.
    pub path_begin: u16,
    /// `PathEnd`.
    pub path_end: u16,
    /// `PathTaperX`.
    pub path_taper_x: i8,
    /// `PathTaperY`.
    pub path_taper_y: i8,
    /// `PathScaleX`. 100 means no scale cut.
    pub path_scale_x: u8,
    /// `PathScaleY`. 100 means no scale cut.
    pub path_scale_y: u8,
    /// `PathShearX`.
    pub path_shear_x: u8,
    /// `PathShearY`.
    pub path_shear_y: u8,
}

/// `ProfileShape` (`PrimitiveBaseShape.cs` 40–47).
const PROFILE_CIRCLE: u8 = 0;
const PROFILE_SQUARE: u8 = 1;
const PROFILE_HALF_CIRCLE: u8 = 5;

/// `Extrusion` (`PrimitiveBaseShape.cs` 68–74).
const PATH_STRAIGHT: u8 = 16;
const PATH_CURVE1: u8 = 32;

/// `Quaternion.CreateFromAxisAngle(UnitX, MathF.PI * 0.5f)`, XYZW.
///
/// These are the bits Pin9's `CookPrimShape` stored for a cylinder. `f32::sin`
/// is not used: its result is not the same bits on every platform.
const CYLINDER_AXIS: [u32; 4] = [0x3F35_04F3, 0x0000_0000, 0x0000_0000, 0x3F35_04F3];

/// The constructor `CookPrimShape` chooses, and the dimensions it passes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnalyticShape {
    /// `CreateBoxShape(size / 2)`. Kind `box`.
    Box {
        /// Half extents, XYZ.
        half: [f32; 3],
    },
    /// `CreateSphereShape(size.X / 2)`. Kind `sphere`.
    Sphere {
        /// Radius.
        radius: f32,
    },
    /// `CreateCylinderShape(size.Z / 2, size.X / 2)`. Kind `cylinder`.
    Cylinder {
        /// Half height along the cylinder axis before the correction.
        half_height: f32,
        /// Radius.
        radius: f32,
        /// Axis correction, XYZW. Identity is not this variant.
        axis: [f32; 4],
    },
}

/// The fast path of `CookPrimShape`.
///
/// `profile` is `ProfileShape` and `path` is `PathCurve`. `size` is the prim
/// size in metres. The analytic path does not read the physical flag.
/// [`None`] is not a box: a rejected ellipsoid is cooked later as
/// `bbox(fallback)` only when the mesher returns nothing.
pub fn analytic_shape(
    profile: u8,
    path: u8,
    cuts: PrimCuts,
    size: [f32; 3],
) -> Option<AnalyticShape> {
    if !prim_has_no_cuts(cuts) {
        return None;
    }
    let hx = size[0] * 0.5;
    let hy = size[1] * 0.5;
    let hz = size[2] * 0.5;
    if profile == PROFILE_SQUARE && path == PATH_STRAIGHT {
        return Some(AnalyticShape::Box { half: [hx, hy, hz] });
    }
    if profile == PROFILE_HALF_CIRCLE
        && path == PATH_CURVE1
        && approx(size[0], size[1])
        && approx(size[1], size[2])
    {
        return Some(AnalyticShape::Sphere { radius: hx });
    }
    if profile == PROFILE_CIRCLE && path == PATH_STRAIGHT && approx(size[0], size[1]) {
        return Some(AnalyticShape::Cylinder {
            half_height: hz,
            radius: hx,
            axis: cylinder_axis(),
        });
    }
    None
}

fn prim_has_no_cuts(cuts: PrimCuts) -> bool {
    cuts.profile_begin == 0
        && cuts.profile_end == 0
        && cuts.profile_hollow == 0
        && cuts.path_twist == 0
        && cuts.path_twist_begin == 0
        && cuts.path_begin == 0
        && cuts.path_end == 0
        && cuts.path_taper_x == 0
        && cuts.path_taper_y == 0
        && cuts.path_scale_x == 100
        && cuts.path_scale_y == 100
        && cuts.path_shear_x == 0
        && cuts.path_shear_y == 0
}

/// `JoltScene.Approx` (`JoltScene.cs` 2230–2231).
fn approx(a: f32, b: f32) -> bool {
    let scale = 1f32.max(a.abs().max(b.abs()));
    (a - b).abs() <= 1e-4f32 * scale
}

fn cylinder_axis() -> [f32; 4] {
    [
        f32::from_bits(CYLINDER_AXIS[0]),
        f32::from_bits(CYLINDER_AXIS[1]),
        f32::from_bits(CYLINDER_AXIS[2]),
        f32::from_bits(CYLINDER_AXIS[3]),
    ]
}

#[cfg(test)]
mod tests {
    use super::{AnalyticShape, PrimCuts, analytic_shape};

    #[test]
    fn analytic_constructors_match_cook_prim_shape() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let text =
            std::fs::read_to_string(root.join("jolt-file/analytic/shape.txt")).expect("shape");
        let assets =
            std::fs::read_to_string(root.join("jolt-record/drop/assets.txt")).expect("assets");
        assert_eq!(line(&text, "method "), "CookPrimShape");
        assert_eq!(line(&text, "source "), "JoltScene.cs 1229-1259");
        assert_eq!(line(&text, "cuts "), "JoltScene.cs 2224-2228");
        assert_eq!(line(&text, "box "), "JoltPhysicsBackend.cs 1712-1718");
        assert_eq!(line(&text, "sphere "), "JoltPhysicsBackend.cs 1720-1723");
        assert_eq!(line(&text, "cylinder "), "JoltPhysicsBackend.cs 1731-1741");
        assert_eq!(line(&text, "access "), "internal");
        assert_eq!(line(&text, "initialise "), "not_called");
        assert_eq!(line(&text, "native_proof "), "after_calls");
        assert_eq!(line(&text, "mesher "), "null");
        assert_eq!(line(&text, "stock_source "), "jolt-record/drop");
        assert_eq!(line(&text, "stock_asset_kind "), "analytic-box");
        assert_eq!(line(&assets, "kind "), "analytic-box");

        let cases = cases(&text);
        let names: Vec<&str> = cases.iter().map(|case| case.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "box-static",
                "box-dynamic",
                "sphere-static",
                "sphere-dynamic",
                "cylinder-static",
                "cylinder-dynamic",
                "ellipsoid-static",
                "ellipsoid-dynamic",
            ]
        );
        for pair in [0, 2, 4, 6] {
            assert!(!cases[pair].physical);
            assert!(cases[pair + 1].physical);
            assert_eq!(cases[pair].same_cook(), cases[pair + 1].same_cook());
        }
        for case in &cases {
            assert_eq!(case.hulls, 1);
            assert_eq!(case.mass_volume, "0000000000000000");
            assert_eq!(case.shape, 1);
            let cooked = analytic_shape(case.profile, case.path, case.cuts, case.size);
            match case.kind.as_str() {
                "box" => {
                    assert!(case.analytic);
                    assert_eq!(case.box_calls, 1);
                    assert_eq!(case.sphere_calls, 0);
                    assert_eq!(case.cylinder_calls, 0);
                    assert_eq!(case.radius, None);
                    assert_eq!(case.cylinder_half, None);
                    assert_eq!(case.cylinder_radius, None);
                    assert_eq!(case.axis, identity());
                    match cooked {
                        Some(AnalyticShape::Box { half }) => {
                            assert_eq!(hex3(half), case.half.clone().expect("half"));
                        }
                        other => panic!("box cooked {other:?}"),
                    }
                }
                "sphere" => {
                    assert!(case.analytic);
                    assert_eq!(case.box_calls, 0);
                    assert_eq!(case.sphere_calls, 1);
                    assert_eq!(case.cylinder_calls, 0);
                    assert_eq!(case.half, None);
                    assert_eq!(case.cylinder_half, None);
                    assert_eq!(case.cylinder_radius, None);
                    assert_eq!(case.axis, identity());
                    match cooked {
                        Some(AnalyticShape::Sphere { radius }) => {
                            assert_eq!(hex(radius), case.radius.clone().expect("radius"));
                        }
                        other => panic!("sphere cooked {other:?}"),
                    }
                }
                "cylinder" => {
                    assert!(case.analytic);
                    assert_eq!(case.box_calls, 0);
                    assert_eq!(case.sphere_calls, 0);
                    assert_eq!(case.cylinder_calls, 1);
                    assert_eq!(case.half, None);
                    assert_eq!(case.radius, None);
                    match cooked {
                        Some(AnalyticShape::Cylinder {
                            half_height,
                            radius,
                            axis,
                        }) => {
                            assert_eq!(
                                hex(half_height),
                                case.cylinder_half.clone().expect("cylinder half")
                            );
                            assert_eq!(
                                hex(radius),
                                case.cylinder_radius.clone().expect("cylinder radius")
                            );
                            assert_eq!(bits4(axis), case.axis);
                        }
                        other => panic!("cylinder cooked {other:?}"),
                    }
                }
                "bbox(fallback)" => {
                    assert!(!case.analytic);
                    assert_eq!(case.box_calls, 1);
                    assert_eq!(case.sphere_calls, 0);
                    assert_eq!(case.cylinder_calls, 0);
                    assert_eq!(case.radius, None);
                    assert_eq!(case.cylinder_half, None);
                    assert_eq!(case.cylinder_radius, None);
                    assert_eq!(case.axis, identity());
                    assert!(cooked.is_none());
                    let half = case.half.clone().expect("fallback half");
                    assert_eq!(half[0], hex(case.size[0] * 0.5));
                    assert_eq!(half[1], hex(case.size[1] * 0.5));
                    assert_eq!(half[2], hex(case.size[2] * 0.5));
                }
                other => panic!("unexpected kind {other}"),
            }
        }
    }

    struct Case {
        name: String,
        physical: bool,
        profile: u8,
        path: u8,
        cuts: PrimCuts,
        size: [f32; 3],
        kind: String,
        analytic: bool,
        box_calls: u32,
        sphere_calls: u32,
        cylinder_calls: u32,
        half: Option<Vec<String>>,
        radius: Option<String>,
        cylinder_half: Option<String>,
        cylinder_radius: Option<String>,
        axis: Vec<String>,
        hulls: i32,
        mass_volume: String,
        shape: u32,
    }

    impl Case {
        fn same_cook(&self) -> String {
            format!(
                "{:?} {} {} {} {} {} {} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {} {} {} {}",
                self.cuts,
                self.profile,
                self.path,
                hex(self.size[0]),
                hex(self.size[1]),
                hex(self.size[2]),
                self.kind,
                self.analytic,
                self.box_calls,
                self.sphere_calls,
                self.cylinder_calls,
                self.half,
                self.radius,
                self.cylinder_half,
                self.cylinder_radius,
                self.axis.join(" "),
                self.hulls,
                self.mass_volume,
                self.shape,
            )
        }
    }

    fn cases(text: &str) -> Vec<Case> {
        text.split("\ncase ")
            .skip(1)
            .map(|chunk| {
                let (name, body) = chunk.split_once('\n').unwrap_or((chunk, ""));
                let block: Vec<String> = body
                    .lines()
                    .take_while(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect();
                parse_case(name, &block)
            })
            .collect()
    }

    fn parse_case(name: &str, block: &[String]) -> Case {
        let size = bits3(&field(block, "size"));
        let cuts = cuts_from(&field(block, "cuts"));
        Case {
            name: name.to_string(),
            physical: field(block, "physical") == "true",
            profile: field(block, "profile").parse().expect("profile"),
            path: field(block, "path").parse().expect("path"),
            cuts,
            size,
            kind: field(block, "kind"),
            analytic: field(block, "analytic") == "true",
            box_calls: field(block, "box_calls").parse().expect("box_calls"),
            sphere_calls: field(block, "sphere_calls").parse().expect("sphere_calls"),
            cylinder_calls: field(block, "cylinder_calls")
                .parse()
                .expect("cylinder_calls"),
            half: optional_bits(&field(block, "half")),
            radius: optional_one(&field(block, "radius")),
            cylinder_half: optional_one(&field(block, "cylinder_half")),
            cylinder_radius: optional_one(&field(block, "cylinder_radius")),
            axis: field(block, "axis")
                .split(' ')
                .map(str::to_string)
                .collect(),
            hulls: field(block, "hulls").parse().expect("hulls"),
            mass_volume: field(block, "mass_volume"),
            shape: field(block, "shape").parse().expect("shape"),
        }
    }

    fn cuts_from(text: &str) -> PrimCuts {
        let parts: Vec<&str> = text.split(' ').collect();
        assert_eq!(parts.len(), 13, "cuts");
        PrimCuts {
            profile_begin: parts[0].parse().expect("profile begin"),
            profile_end: parts[1].parse().expect("profile end"),
            profile_hollow: parts[2].parse().expect("hollow"),
            path_twist: parts[3].parse().expect("twist"),
            path_twist_begin: parts[4].parse().expect("twist begin"),
            path_begin: parts[5].parse().expect("path begin"),
            path_end: parts[6].parse().expect("path end"),
            path_taper_x: parts[7].parse().expect("taper x"),
            path_taper_y: parts[8].parse().expect("taper y"),
            path_scale_x: parts[9].parse().expect("scale x"),
            path_scale_y: parts[10].parse().expect("scale y"),
            path_shear_x: parts[11].parse().expect("shear x"),
            path_shear_y: parts[12].parse().expect("shear y"),
        }
    }

    fn bits3(text: &str) -> [f32; 3] {
        let parts: Vec<&str> = text.split(' ').collect();
        assert_eq!(parts.len(), 3);
        [
            f32::from_bits(u32::from_str_radix(parts[0], 16).expect("x")),
            f32::from_bits(u32::from_str_radix(parts[1], 16).expect("y")),
            f32::from_bits(u32::from_str_radix(parts[2], 16).expect("z")),
        ]
    }

    fn optional_bits(text: &str) -> Option<Vec<String>> {
        if text == "none" {
            None
        } else {
            Some(text.split(' ').map(str::to_string).collect())
        }
    }

    fn optional_one(text: &str) -> Option<String> {
        if text == "none" {
            None
        } else {
            Some(text.to_string())
        }
    }

    fn field(block: &[String], key: &str) -> String {
        let prefix = format!("{key} ");
        block
            .iter()
            .find_map(|line| line.strip_prefix(&prefix).map(str::to_string))
            .unwrap_or_else(|| panic!("missing {key}"))
    }

    fn line(text: &str, key: &str) -> String {
        text.lines()
            .find_map(|line| line.strip_prefix(key).map(str::to_string))
            .unwrap_or_else(|| panic!("missing {key}"))
    }

    fn hex(value: f32) -> String {
        format!("{:08X}", value.to_bits())
    }

    fn hex3(value: [f32; 3]) -> Vec<String> {
        value.into_iter().map(hex).collect()
    }

    fn bits4(value: [f32; 4]) -> Vec<String> {
        value.into_iter().map(hex).collect()
    }

    fn identity() -> Vec<String> {
        vec![
            "00000000".to_string(),
            "00000000".to_string(),
            "00000000".to_string(),
            "3F800000".to_string(),
        ]
    }
}
