// Copyright (c) 2015, InWorldz Halcyon Developers
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
//   * Redistributions of source code must retain the above copyright notice, this
//     list of conditions and the following disclaimer.
//
//   * Redistributions in binary form must reproduce the above copyright notice,
//     this list of conditions and the following disclaimer in the documentation
//     and/or other materials provided with the distribution.
//
//   * Neither the name of halcyon nor the names of its
//     contributors may be used to endorse or promote products derived from
//     this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
// FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
// DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
// CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
// OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Cite HalcyonBasicForces.cs 1-33. The 0.977 boundary is LegacyBuoyancy
// (lines 103-104). The full Halcyon terms are also in licenses/Halcyon.txt.
//
// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltPrim.cs
//   Source/OpenSim.Region.PhysicsModules.Jolt/HalcyonBasicForces.cs
//   KanikaBreed/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4.
// This port remains available under MPL-2.0 and forms part of Ferroan's
// GPL-3.0-or-later work. The buoyancy boundary remains BSD-3-Clause.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! Buoyancy, the script gravity factor, and the vehicle override.
//!
//! `HalcyonBasicForces.LegacyBuoyancy` (`HalcyonBasicForces.cs` 103–104) turns
//! a buoyancy from 0.977 up to, and not including, 1 into 1. `JoltPrim.Buoyancy`
//! (`JoltPrim.cs` 1784–1798) stores that value and `ApplyScriptGravity`
//! (1802–1808) writes `ScriptGravityFactor` (1813–1819). A linked child writes
//! the root. A non-finite value is rejected. An active vehicle writes factor 0
//! from `ApplyVehicleBodyParams` (2755–2767) and `RestoreVehicleBodyParams`
//! (2794–2810) puts the script factor back. `VehicleType` (2303–2332) is the
//! call that does both. `ProcessTypeChangeCore` (`VehicleController.cs`
//! 471–485) maps an unrecognised integer to none, so that code restores the
//! script factor. Water is not an input. The falling boxes are P3.65.

/// Scene defaults the factor's clamp reads. `BodyMaxLinearSpeed` is 500.
/// `JoltConfig.MaxPhysicsStepRate` is 1000. `gravity_length` is
/// `DefaultGravity.Length()` on `new JoltScene()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GravityScale {
    /// `JoltScene.BodyMaxLinearSpeed`.
    pub max_linear: f32,
    /// `JoltConfig.MaxPhysicsStepRate`.
    pub step_rate: f32,
    /// `JoltScene.DefaultGravity.Length()`.
    pub gravity_length: f32,
}

/// One prim's script gravity, without a body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GravityActor {
    /// Stored `_buoyancy`. The body feels `(1 - buoyancy)` of the region.
    pub buoyancy: f32,
    /// Stored `_gravityModifier`. The setter that clamps it is P3.50.
    pub modifier: f32,
    /// `_isPhysical`.
    pub physical: bool,
    /// `BodyId.Value` of the body the factor is written to.
    pub body: u32,
    /// `BodyId.IsValid`.
    pub body_valid: bool,
    /// `VehicleController.IsActive`. Type none is not active.
    pub vehicle_active: bool,
    /// `BasicForcesBehaviour` is InWorldz, so the 0.977 boundary applies.
    pub inworldz: bool,
}

/// One `SetBodyGravityFactor` write.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GravityWrite {
    /// `BodyId.Value`.
    pub body: u32,
    /// The factor passed to the backend.
    pub factor: f32,
}

/// `LegacyBuoyancy`. A value from 0.977 inclusive to 1 exclusive becomes 1.
pub fn legacy_buoyancy(buoyancy: f32) -> f32 {
    if (0.977f32..1.0f32).contains(&buoyancy) {
        1.0
    } else {
        buoyancy
    }
}

/// `ScriptGravityFactor`. The product is clamped to the speed the body can
/// reach in one 1 ms step.
pub fn script_gravity_factor(buoyancy: f32, modifier: f32, scale: GravityScale) -> f32 {
    let max = scale.max_linear * scale.step_rate / scale.gravity_length.max(1.0);
    ((1.0 - buoyancy) * modifier).clamp(-max, max)
}

/// `Buoyancy` set on a root. A non-finite value and an unchanged value do not
/// write. An inactive physical body with a valid id writes the script factor.
/// An active vehicle keeps the stored value and does not write.
pub fn set_buoyancy(
    actor: &mut GravityActor,
    value: f32,
    scale: GravityScale,
) -> Option<GravityWrite> {
    if !value.is_finite() {
        return None;
    }
    let stored = if actor.inworldz {
        legacy_buoyancy(value)
    } else {
        value
    };
    if actor.buoyancy == stored {
        return None;
    }
    actor.buoyancy = stored;
    write_script(actor, scale)
}

/// A child's `Buoyancy` set. The child rejects a non-finite value and does not
/// change its own field. A finite value is the root's set.
pub fn set_linked_buoyancy(
    root: &mut GravityActor,
    value: f32,
    scale: GravityScale,
) -> Option<GravityWrite> {
    if !value.is_finite() {
        return None;
    }
    set_buoyancy(root, value, scale)
}

/// `VehicleType`. The seven codes `ProcessTypeChangeCore` keeps are active
/// and, on a physical valid body, write factor 0. None and any other integer
/// restore `ScriptGravityFactor` when the body id is valid, including a body
/// that is not physical.
pub fn set_vehicle_type(
    actor: &mut GravityActor,
    code: i32,
    scale: GravityScale,
) -> Option<GravityWrite> {
    let active = recognised_vehicle(code);
    actor.vehicle_active = active;
    if active && actor.physical && actor.body_valid {
        return Some(GravityWrite {
            body: actor.body,
            factor: 0.0,
        });
    }
    if active || !actor.body_valid {
        return None;
    }
    Some(GravityWrite {
        body: actor.body,
        factor: script_gravity_factor(actor.buoyancy, actor.modifier, scale),
    })
}

/// `Vehicle` values that stay active. `TYPE_NONE` is 0. A param such as
/// `LINEAR_FRICTION_TIMESCALE` (16) and any other integer take the default
/// arm and become none.
fn recognised_vehicle(code: i32) -> bool {
    matches!(code, 1 | 2 | 3 | 4 | 5 | 10001 | 10002)
}

fn write_script(actor: &GravityActor, scale: GravityScale) -> Option<GravityWrite> {
    if !actor.physical || !actor.body_valid || actor.vehicle_active {
        return None;
    }
    Some(GravityWrite {
        body: actor.body,
        factor: script_gravity_factor(actor.buoyancy, actor.modifier, scale),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        GravityActor, GravityScale, legacy_buoyancy, script_gravity_factor, set_buoyancy,
        set_linked_buoyancy, set_vehicle_type,
    };

    #[test]
    fn gravity_commands_match_the_pinned_matrix() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jolt-file");
        let manifest = std::fs::read_to_string(root.join("manifest.txt")).expect("manifest");
        assert!(manifest.lines().any(|line| line == "gravity"));
        let text = std::fs::read_to_string(root.join("gravity/matrix.txt")).expect("matrix");
        assert!(text.contains("header HalcyonBasicForces.cs 1-33\n"));
        assert!(text.contains("water not_used\n"));
        assert!(text.contains("native not_called\n"));
        assert!(text.contains("source JoltPrim.cs 1784-1819\n"));
        assert!(
            text.contains("source JoltPrim.cs 2807\n")
                || text.contains("source JoltPrim.cs 2794-2810\n")
        );

        let mut parts = text.split("\ncase ");
        let header = parts.next().expect("header");
        assert_eq!(hex(field(header, "max_linear ")), 500f32.to_bits());
        assert_eq!(hex(field(header, "step_rate ")), 1000f32.to_bits());
        assert_eq!(hex(field(header, "gravity_z ")), (-9.80665f32).to_bits());
        assert_eq!(hex(field(header, "threshold ")), 0.977f32.to_bits());
        let gravity_length = f32::from_bits(hex(field(header, "gravity_len ")));
        assert!(gravity_length > 1.0);
        let scale = GravityScale {
            max_linear: 500.0,
            step_rate: 1000.0,
            gravity_length,
        };
        let clamp_modifier = f32::from_bits(hex(field(header, "clamp_modifier ")));
        assert_eq!(
            script_gravity_factor(0.0, clamp_modifier, scale).to_bits(),
            hex(field(header, "clamp_max "))
        );
        assert_eq!(
            script_gravity_factor(0.0, -clamp_modifier, scale).to_bits(),
            hex(field(header, "clamp_min "))
        );
        let mut functions = 0u32;
        for line in header.lines() {
            let Some(rest) = line.strip_prefix("in ") else {
                continue;
            };
            let (input, output) = rest.split_once(" out ").expect("function row");
            let value = f32::from_bits(hex(input));
            assert_eq!(legacy_buoyancy(value).to_bits(), hex(output), "{line}");
            functions += 1;
        }
        assert!(functions >= 13, "legacy rows");

        let cases: Vec<&str> = parts.collect();
        assert_eq!(cases.len(), 11);
        let unknown = cases
            .iter()
            .copied()
            .find(|case| case.starts_with("unknown-type\n"))
            .expect("unknown-type");
        assert!(unknown.contains("action buoyancy 3F000000\n"));
        assert!(unknown.contains("action type 2\n"));
        assert!(unknown.contains("action type 7\n"));
        for case_text in cases {
            check_case(case_text, scale);
        }
    }

    fn check_case(text: &str, scale: GravityScale) {
        let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
        let mut index = 1;
        let mut behaviour = "";
        let mut physical = false;
        let mut body = 0u32;
        let mut valid = false;
        let mut modifier = 0f32;
        let mut initial = 0f32;
        while index < lines.len() && !lines[index].starts_with("action ") {
            let (key, rest) = lines[index].split_once(' ').expect("header field");
            match key {
                "behaviour" => behaviour = rest,
                "physical" => physical = flag(rest),
                "body" => body = rest.parse().expect("body"),
                "valid" => valid = flag(rest),
                "modifier" => modifier = f32::from_bits(hex(rest)),
                "child" => {}
                "initial" => initial = f32::from_bits(hex(rest)),
                other => panic!("unknown case field {other}"),
            }
            index += 1;
        }
        let mut actor = GravityActor {
            buoyancy: initial,
            modifier,
            physical,
            body,
            body_valid: valid,
            vehicle_active: false,
            inworldz: behaviour == "InWorldz",
        };
        assert!(index < lines.len(), "no action");
        while index < lines.len() {
            let action = lines[index];
            index += 1;
            if let Some(bits) = action.strip_prefix("action buoyancy ") {
                let value = f32::from_bits(hex(bits));
                let write = set_buoyancy(&mut actor, value, scale);
                let stored = hex(take(&lines, &mut index, "stored "));
                let got = hex(take(&lines, &mut index, "get "));
                assert_eq!(stored, actor.buoyancy.to_bits(), "{action}");
                assert_eq!(got, actor.buoyancy.to_bits(), "{action}");
                expect_tail(&lines, &mut index, &actor, write, 0);
            } else if let Some(bits) = action.strip_prefix("action child ") {
                let value = f32::from_bits(hex(bits));
                let write = set_linked_buoyancy(&mut actor, value, scale);
                let root_stored = hex(take(&lines, &mut index, "root_stored "));
                let child_stored = hex(take(&lines, &mut index, "child_stored "));
                let child_get = hex(take(&lines, &mut index, "child_get "));
                assert_eq!(root_stored, actor.buoyancy.to_bits(), "{action}");
                assert_eq!(child_stored, 0, "{action}");
                assert_eq!(child_get, actor.buoyancy.to_bits(), "{action}");
                expect_tail(&lines, &mut index, &actor, write, 0);
            } else if let Some(code_text) = action.strip_prefix("action type ") {
                let code: i32 = code_text.parse().expect("type");
                let write = set_vehicle_type(&mut actor, code, scale);
                let stored = hex(take(&lines, &mut index, "stored "));
                let got = hex(take(&lines, &mut index, "get "));
                let type_get: i32 = take(&lines, &mut index, "type_get ")
                    .parse()
                    .expect("type_get");
                assert_eq!(stored, actor.buoyancy.to_bits(), "{action}");
                assert_eq!(got, actor.buoyancy.to_bits(), "{action}");
                let recognised = super::recognised_vehicle(code);
                if recognised {
                    assert_eq!(type_get, code);
                } else {
                    assert_eq!(type_get, 0);
                }
                let activates = if recognised && physical && valid {
                    1
                } else {
                    0
                };
                expect_tail(&lines, &mut index, &actor, write, activates);
            } else {
                panic!("unknown action {action}");
            }
        }
    }

    fn expect_tail(
        lines: &[&str],
        index: &mut usize,
        actor: &GravityActor,
        write: Option<super::GravityWrite>,
        activates: i32,
    ) {
        let active = flag(take(lines, index, "active "));
        assert_eq!(active, actor.vehicle_active);
        assert_eq!(
            take(lines, index, "activates ")
                .parse::<i32>()
                .expect("activates"),
            activates
        );
        let count: usize = take(lines, index, "writes ").parse().expect("writes");
        match write {
            None => assert_eq!(count, 0),
            Some(expected) => {
                assert_eq!(count, 1);
                let rest = take(lines, index, "write ");
                let (body, bits) = rest.split_once(' ').expect("write");
                assert_eq!(body.parse::<u32>().expect("write body"), expected.body);
                assert_eq!(hex(bits), expected.factor.to_bits());
            }
        }
    }

    fn take<'a>(lines: &[&'a str], index: &mut usize, prefix: &str) -> &'a str {
        let line = lines[*index];
        *index += 1;
        line.strip_prefix(prefix)
            .unwrap_or_else(|| panic!("expected {prefix} in {line}"))
    }

    fn field<'a>(text: &'a str, prefix: &str) -> &'a str {
        text.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("missing {prefix}"))
    }

    fn hex(text: &str) -> u32 {
        u32::from_str_radix(text, 16).unwrap_or_else(|_| panic!("bits {text}"))
    }

    fn flag(text: &str) -> bool {
        match text {
            "true" => true,
            "false" => false,
            other => panic!("flag {other}"),
        }
    }
}
