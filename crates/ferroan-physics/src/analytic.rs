// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltPrim.cs
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

/// `DensityScaleFactor` (`JoltPrim.cs` 64). Simulator density 1000 becomes this.
const DENSITY_SCALE: f32 = 0.01;
/// Default `SceneObjectPart` density, before the scale.
const SIM_DENSITY: f32 = 1000.0;
/// `LoneMass` floor (`JoltPrim.cs` 1322).
const MIN_MASS: f32 = 1.0e-3;

/// Volume times the physical density. `JoltPrim.Mass` for a lone analytic prim
/// (`JoltPrim.cs` 1228–1236 and 1322).
#[must_use]
pub fn analytic_mass(shape: AnalyticShape) -> f32 {
    let volume = match shape {
        AnalyticShape::Box { half } => 8.0 * half[0] * half[1] * half[2],
        AnalyticShape::Sphere { radius } => {
            (4.0 / 3.0) * std::f32::consts::PI * radius * radius * radius
        }
        AnalyticShape::Cylinder {
            half_height,
            radius,
            ..
        } => std::f32::consts::PI * radius * radius * (2.0 * half_height),
    };
    (volume * SIM_DENSITY * DENSITY_SCALE).max(MIN_MASS)
}

/// One free-flight sample. Position is metres and velocity is metres per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    /// World position after `heartbeats` heartbeats.
    pub position: [f32; 3],
    /// Linear velocity after those heartbeats.
    pub velocity: [f32; 3],
}

/// Symplectic free flight before contact.
///
/// Each solver step adds gravity, then applies `v *= max(0, 1 - c dt)`, then
/// moves by the updated velocity (`MotionProperties.inl` 133–144 and
/// `PhysicsSystem.cpp` 1613–1682). `collision_steps` slices one heartbeat.
/// A body that is not dynamic stays where it is: pass `heartbeats` 0.
#[must_use]
pub fn free_flight(
    mut position: [f32; 3],
    mut velocity: [f32; 3],
    gravity: [f32; 3],
    linear_damping: f32,
    heartbeat: f32,
    collision_steps: u32,
    heartbeats: u32,
) -> Flight {
    if collision_steps == 0 || heartbeats == 0 || !heartbeat.is_finite() || heartbeat <= 0.0 {
        return Flight { position, velocity };
    }
    let dt = heartbeat / collision_steps as f32;
    if !dt.is_finite() || dt <= 0.0 {
        return Flight { position, velocity };
    }
    let damp = (1.0 - linear_damping * dt).max(0.0);
    let total = heartbeats.saturating_mul(collision_steps);
    for _ in 0..total {
        velocity[0] += dt * gravity[0];
        velocity[1] += dt * gravity[1];
        velocity[2] += dt * gravity[2];
        velocity[0] *= damp;
        velocity[1] *= damp;
        velocity[2] *= damp;
        position[0] += velocity[0] * dt;
        position[1] += velocity[1] * dt;
        position[2] += velocity[2] * dt;
    }
    Flight { position, velocity }
}

/// A ray hit. `distance` is the parameter along a unit direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// World position of the hit.
    pub position: [f32; 3],
    /// Outward normal.
    pub normal: [f32; 3],
    /// Distance from the start along the unit direction.
    pub distance: f32,
}

/// Closest hit of a unit ray against an analytic shape centred at `center`.
///
/// The cylinder's axis correction stands it on Z, which is the orientation
/// `CookPrimShape` stores. A miss, a zero length, or a non-unit direction
/// returns none.
#[must_use]
pub fn analytic_ray(
    shape: AnalyticShape,
    center: [f32; 3],
    start: [f32; 3],
    direction: [f32; 3],
    length: f32,
) -> Option<RayHit> {
    if !length.is_finite() || length <= 0.0 || !unit(direction) {
        return None;
    }
    match shape {
        AnalyticShape::Box { half } => ray_box(center, half, start, direction, length),
        AnalyticShape::Sphere { radius } => ray_sphere(center, radius, start, direction, length),
        AnalyticShape::Cylinder {
            half_height,
            radius,
            ..
        } => ray_cylinder_z(center, radius, half_height, start, direction, length),
    }
}

fn unit(direction: [f32; 3]) -> bool {
    let length_sq =
        direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2];
    (length_sq - 1.0).abs() <= 1.0e-6
}

fn ray_box(
    center: [f32; 3],
    half: [f32; 3],
    start: [f32; 3],
    direction: [f32; 3],
    length: f32,
) -> Option<RayHit> {
    let mut t_min = 0.0f32;
    let mut t_max = length;
    let mut normal = [0.0f32; 3];
    for axis in 0..3 {
        let origin = start[axis] - center[axis];
        if direction[axis].abs() <= 1.0e-12 {
            if origin.abs() > half[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / direction[axis];
        let mut t1 = (-half[axis] - origin) * inv;
        let mut t2 = (half[axis] - origin) * inv;
        let mut face = -1.0f32;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
            face = 1.0;
        }
        if t1 > t_min {
            t_min = t1;
            normal = [0.0; 3];
            normal[axis] = face;
        }
        if t2 < t_max {
            t_max = t2;
        }
        if t_min > t_max {
            return None;
        }
    }
    if t_min < 0.0 || t_min > length {
        return None;
    }
    Some(hit_at(start, direction, t_min, normal))
}

fn ray_sphere(
    center: [f32; 3],
    radius: f32,
    start: [f32; 3],
    direction: [f32; 3],
    length: f32,
) -> Option<RayHit> {
    let offset = [
        start[0] - center[0],
        start[1] - center[1],
        start[2] - center[2],
    ];
    let b = 2.0 * (offset[0] * direction[0] + offset[1] * direction[1] + offset[2] * direction[2]);
    let c = offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2] - radius * radius;
    let disc = b * b - 4.0 * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t = (-b - root) * 0.5;
    if t < 0.0 || t > length {
        return None;
    }
    let position = [
        start[0] + direction[0] * t,
        start[1] + direction[1] * t,
        start[2] + direction[2] * t,
    ];
    let scale = 1.0 / radius;
    let normal = [
        (position[0] - center[0]) * scale,
        (position[1] - center[1]) * scale,
        (position[2] - center[2]) * scale,
    ];
    Some(RayHit {
        position,
        normal,
        distance: t,
    })
}

fn ray_cylinder_z(
    center: [f32; 3],
    radius: f32,
    half_height: f32,
    start: [f32; 3],
    direction: [f32; 3],
    length: f32,
) -> Option<RayHit> {
    let ox = start[0] - center[0];
    let oy = start[1] - center[1];
    let oz = start[2] - center[2];
    let a = direction[0] * direction[0] + direction[1] * direction[1];
    let mut best: Option<RayHit> = None;
    if a > 1.0e-12 {
        let b = 2.0 * (ox * direction[0] + oy * direction[1]);
        let c = ox * ox + oy * oy - radius * radius;
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let root = disc.sqrt();
            let t = (-b - root) / (2.0 * a);
            if t >= 0.0 && t <= length {
                let z = oz + direction[2] * t;
                if z.abs() <= half_height {
                    let position = [
                        start[0] + direction[0] * t,
                        start[1] + direction[1] * t,
                        start[2] + direction[2] * t,
                    ];
                    let scale = 1.0 / radius;
                    best = Some(RayHit {
                        position,
                        normal: [
                            (position[0] - center[0]) * scale,
                            (position[1] - center[1]) * scale,
                            0.0,
                        ],
                        distance: t,
                    });
                }
            }
        }
    }
    if direction[2].abs() > 1.0e-12 {
        for sign in [-1.0f32, 1.0] {
            let t = (sign * half_height - oz) / direction[2];
            if t < 0.0 || t > length {
                continue;
            }
            let x = ox + direction[0] * t;
            let y = oy + direction[1] * t;
            if x * x + y * y > radius * radius {
                continue;
            }
            if best.is_some_and(|hit| hit.distance <= t) {
                continue;
            }
            best = Some(hit_at(start, direction, t, [0.0, 0.0, sign]));
        }
    }
    best
}

fn hit_at(start: [f32; 3], direction: [f32; 3], t: f32, normal: [f32; 3]) -> RayHit {
    RayHit {
        position: [
            start[0] + direction[0] * t,
            start[1] + direction[1] * t,
            start[2] + direction[2] * t,
        ],
        normal,
        distance: t,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnalyticShape, PATH_CURVE1, PATH_STRAIGHT, PROFILE_CIRCLE, PROFILE_HALF_CIRCLE,
        PROFILE_SQUARE, PrimCuts, analytic_mass, analytic_ray, analytic_shape, free_flight,
    };

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

    #[test]
    fn envelope_matches_mass_ray_and_free_flight() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/jolt-record/analytic-envelope.txt");
        let text = std::fs::read_to_string(path).expect("envelope");
        let gravity = bits3(header(&text, "gravity "));
        let damping = bits1(header(&text, "linear_damping "));
        let heartbeat = bits1(header(&text, "heartbeat "));
        let collision_steps: u32 = header(&text, "collision_steps ").parse().expect("steps");
        let position_tol: f32 = header(&text, "position_tolerance_m ").parse().expect("pos");
        let velocity_tol: f32 = header(&text, "velocity_tolerance_m_s ")
            .parse()
            .expect("vel");
        let mass_decimals: u32 = header(&text, "mass_decimals ").parse().expect("mass");
        let mut shapes = Vec::new();
        for line in text.lines().filter(|line| line.starts_with("shape ")) {
            let words: Vec<_> = line.split_whitespace().collect();
            let name = words[1];
            let local = words[5].parse::<u32>().expect("local");
            let position = [bits1(words[7]), bits1(words[8]), bits1(words[9])];
            let size = [bits1(words[11]), bits1(words[12]), bits1(words[13])];
            let mass = bits1(words[15]);
            let shape = shape_for(name, size).expect(name);
            let got = analytic_mass(shape);
            assert!(
                mass_near(got, mass, mass_decimals),
                "{name} mass {got} vs {mass}"
            );
            shapes.push((name.to_string(), position, shape, local));
        }
        let mut rays = 0u32;
        for line in text
            .lines()
            .filter(|line| line.starts_with("ray ") && line.contains(" heartbeats "))
        {
            let words: Vec<_> = line.split_whitespace().collect();
            let name = words[1];
            let start = [bits1(words[5]), bits1(words[6]), bits1(words[7])];
            let direction = [bits1(words[9]), bits1(words[10]), bits1(words[11])];
            let length = bits1(words[13]);
            assert_eq!(words[15], "1");
            let consumer: u32 = words[17].parse().expect("consumer");
            let hit_pos = [bits1(words[19]), bits1(words[20]), bits1(words[21])];
            let depth = bits1(words[27]);
            let found = shapes.iter().find(|item| item.0 == name).expect(name);
            let center = found.1;
            let shape = found.2;
            let local = found.3;
            assert_eq!(local, consumer);
            let hit = analytic_ray(shape, center, start, direction, length).expect(name);
            assert!(
                distance(hit.position, hit_pos) <= position_tol,
                "{name} ray {}",
                distance(hit.position, hit_pos)
            );
            assert!((hit.distance - depth).abs() <= position_tol);
            rays += 1;
        }
        assert_eq!(rays, 3);
        assert_eq!(shapes.len(), 6);
        let mut flights = 0u32;
        for line in text.lines().filter(|line| line.starts_with("flight ")) {
            let words: Vec<_> = line.split_whitespace().collect();
            let name = words[1];
            let step: u32 = words[3].parse().expect("step");
            let position = [bits1(words[5]), bits1(words[6]), bits1(words[7])];
            let velocity = [bits1(words[9]), bits1(words[10]), bits1(words[11])];
            let (start_name, start, shape, _) =
                shapes.iter().find(|item| item.0 == name).expect(name);
            assert_eq!(start_name, name);
            let _ = shape;
            let flight = free_flight(
                *start,
                [0.0; 3],
                gravity,
                damping,
                heartbeat,
                collision_steps,
                step,
            );
            assert!(
                distance(flight.position, position) <= position_tol,
                "{name} step {step} pos {}",
                distance(flight.position, position)
            );
            assert!(
                distance(flight.velocity, velocity) <= velocity_tol,
                "{name} step {step} vel {}",
                distance(flight.velocity, velocity)
            );
            flights += 1;
        }
        assert_eq!(flights, 6);
    }

    fn shape_for(name: &str, size: [f32; 3]) -> Option<AnalyticShape> {
        let (profile, path) = match name {
            "box-static" | "box-dynamic" => (PROFILE_SQUARE, PATH_STRAIGHT),
            "sphere-static" | "sphere-dynamic" => (PROFILE_HALF_CIRCLE, PATH_CURVE1),
            "cylinder-static" | "cylinder-dynamic" => (PROFILE_CIRCLE, PATH_STRAIGHT),
            _ => return None,
        };
        analytic_shape(profile, path, uncut_cuts(), size)
    }

    fn uncut_cuts() -> PrimCuts {
        PrimCuts {
            profile_begin: 0,
            profile_end: 0,
            profile_hollow: 0,
            path_twist: 0,
            path_twist_begin: 0,
            path_begin: 0,
            path_end: 0,
            path_taper_x: 0,
            path_taper_y: 0,
            path_scale_x: 100,
            path_scale_y: 100,
            path_shear_x: 0,
            path_shear_y: 0,
        }
    }

    fn header<'a>(text: &'a str, key: &str) -> &'a str {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .unwrap_or_else(|| panic!("missing {key}"))
    }

    fn bits1(text: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(text, 16).expect("hex"))
    }

    fn mass_near(got: f32, expected: f32, decimals: u32) -> bool {
        let scale = 10f64.powi(i32::try_from(decimals).unwrap_or(3));
        let left = (f64::from(got) * scale).round_ties_even();
        let right = (f64::from(expected) * scale).round_ties_even();
        left == right
    }

    fn distance(left: [f32; 3], right: [f32; 3]) -> f32 {
        let d = [left[0] - right[0], left[1] - right[1], left[2] - right[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
}
