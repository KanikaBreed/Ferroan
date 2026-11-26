// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltCharacter.cs
//   Source/OpenSim.Region.PhysicsModules.Jolt/NonFiniteGuard.cs (Ok)
//   Source/OpenSim.Region.PhysicsModules.Jolt.Backend/IPhysicsBackend.cs (CharacterDesc.Default)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// This port remains available under MPL-2.0 and forms part of Ferroan's
// GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! Capsule, mass, jump latch, movement intent and the marker descriptor.
//!
//! `JoltCharacter.cs` 115–153 builds the capsule and the mass. 183–211 applies
//! one drain, including the latch release. 221–234 keeps the avatar inside the
//! region. 236–265 is the intent push. 364–407 and 499–518 reject a non-finite
//! target, size or buoyancy and keep the previous value.
//!
//! `CharacterDesc.Default` (`IPhysicsBackend.cs` 346–357) supplies the friction,
//! slope, step and push that the constructor does not overwrite. The descriptor
//! is what `CreateCharacter` receives. A ray that would see the query marker,
//! and a body-cap miss, need the native body. The gravity scale
//! `Gravity.Z * (1 - buoyancy)` sits inside `StepCharacter` and is not applied
//! here.

/// `DefaultAvatarMass`. Second Life's fixed avatar mass, in the same units as a prim.
pub const DEFAULT_AVATAR_MASS: f32 = 1.954922;

/// `DefaultAvatarSize`: the default appearance box, width, depth, height.
pub const DEFAULT_AVATAR_SIZE: [f32; 3] = [0.45, 0.6, 2.1];

/// `EdgeMargin`. The avatar stays this far inside the region edge.
pub const EDGE_MARGIN: f32 = 0.01;

const MIN_RADIUS: f32 = 0.2;
const CAP_CLEARANCE: f32 = 0.1;
const MIN_HALF_HEIGHT: f32 = 0.05;
const JUMP_RELEASE_VZ: f32 = 0.5;
const DEFAULT_FRICTION: f32 = 0.5;
const DEFAULT_STEP_HEIGHT: f32 = 0.45;
const DEFAULT_PUSH_STRENGTH: f32 = 1.0;
const MAX_SLOPE_DEGREES: f32 = 50.0;

/// Cylinder half-height and radius from [`capsule_from_size`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub half_height: f32,
    pub radius: f32,
}

/// `CapsuleFromSize`. A finite footprint keeps a radius of at least 0.2 m.
/// A NaN from `MathF.Min` or `MathF.Max` stays NaN, which is what the
/// constructor stores. The `Size` setter rejects that size before this runs.
#[must_use]
pub fn capsule_from_size(size: [f32; 3]) -> Capsule {
    let radius = at_least(MIN_RADIUS, lesser(size[0], size[1]) * 0.5);
    let total_half = at_least(radius + CAP_CLEARANCE, size[2] * 0.5);
    Capsule {
        radius,
        half_height: at_least(MIN_HALF_HEIGHT, total_half - radius),
    }
}

/// `MassFor`: the default mass scaled by capsule volume against the default size.
#[must_use]
pub fn mass_for(size: [f32; 3]) -> f32 {
    let volume = capsule_volume(size);
    let default_volume = capsule_volume(DEFAULT_AVATAR_SIZE);
    DEFAULT_AVATAR_MASS * (volume / default_volume)
}

/// `StandHalfFor`: cylinder half-height plus one cap.
#[must_use]
pub fn stand_half_for(size: [f32; 3]) -> f32 {
    let capsule = capsule_from_size(size);
    capsule.half_height + capsule.radius
}

fn capsule_volume(size: [f32; 3]) -> f32 {
    let capsule = capsule_from_size(size);
    let radius = capsule.radius;
    let half = capsule.half_height;
    // `4f / 3f * r` is a float division, then the sum, then `PI * r * r *` that.
    std::f32::consts::PI * radius * radius * (2.0 * half + 4.0f32 / 3.0f32 * radius)
}

/// `MathF.Max`. A NaN on either side is returned. `Max(-0, +0)` is `+0`.
fn at_least(floor: f32, value: f32) -> f32 {
    if floor != value {
        if !floor.is_nan() {
            if value < floor { floor } else { value }
        } else {
            floor
        }
    } else if value.is_sign_negative() {
        floor
    } else {
        value
    }
}

/// `MathF.Min`. A NaN on either side is returned. `Min(-0, +0)` is `-0`.
fn lesser(left: f32, right: f32) -> f32 {
    if left != right {
        if !left.is_nan() {
            if left < right { left } else { right }
        } else {
            left
        }
    } else if left.is_sign_negative() {
        left
    } else {
        right
    }
}

/// `[Jolt]` speed factors `DesiredVelocity` reads. Jump speed is not one of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedFactors {
    pub walk: f32,
    pub run: f32,
    pub fly: f32,
    pub fly_down: f32,
}

/// One `DesiredVelocity` call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intent {
    pub target: [f32; 3],
    pub flying: bool,
    pub always_run: bool,
    pub factors: SpeedFactors,
}

/// `DesiredVelocity`. Walk and run scale X and Y only. Flight scales all three,
/// and a negative target Z uses the fly-down factor.
#[must_use]
pub fn desired_velocity(intent: Intent) -> [f32; 3] {
    let target = intent.target;
    if intent.flying {
        let mut velocity = scale(target, intent.factors.fly);
        if target[2] < 0.0 {
            velocity[2] = target[2] * intent.factors.fly_down;
        }
        velocity
    } else {
        let factor = if intent.always_run {
            intent.factors.run
        } else {
            intent.factors.walk
        };
        [target[0] * factor, target[1] * factor, target[2]]
    }
}

fn scale(value: [f32; 3], factor: f32) -> [f32; 3] {
    [value[0] * factor, value[1] * factor, value[2] * factor]
}

/// `AvatarJump` sets the latch and ignores `force_z`.
#[must_use]
pub fn jump_latch_on_press(_force_z: f32) -> bool {
    true
}

/// `PushMovement` sends the current latch and does not clear it.
#[must_use]
pub fn movement_sends_latch(latched: bool) -> bool {
    latched
}

/// `ApplyCharacterState` clears the latch when vertical velocity is above 0.5
/// or the avatar is not supported. 0.5 exactly stays latched.
#[must_use]
pub fn jump_latch_after_drain(latched: bool, velocity_z: f32, supported: bool) -> bool {
    if latched && (velocity_z > JUMP_RELEASE_VZ || !supported) {
        false
    } else {
        latched
    }
}

/// Support flags `ApplyCharacterState` writes for one drain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroundFlags {
    pub colliding: bool,
    pub ground: bool,
    pub object: bool,
}

/// `IsColliding`, `CollidingGround` and `CollidingObj`.
/// An invalid ground body counts as terrain-as-ground.
#[must_use]
pub fn ground_flags(supported: bool, ground_valid: bool, ground_is_terrain: bool) -> GroundFlags {
    GroundFlags {
        colliding: supported,
        ground: supported && (ground_is_terrain || !ground_valid),
        object: supported && ground_valid && !ground_is_terrain,
    }
}

/// Position after `KeepInRegion`, and whether that wrote a transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionClamp {
    pub position: [f32; 3],
    pub wrote_transform: bool,
}

/// `KeepInRegion`. `Math.Clamp` throws when the margin is at least the region
/// extent; this is the comparison it uses when the minimum is at most the
/// maximum, which a 256 m region satisfies. Z is left as it arrived.
#[must_use]
pub fn keep_in_region(
    position: [f32; 3],
    region_x: f32,
    region_y: f32,
    allow_outside: bool,
    character_valid: bool,
) -> RegionClamp {
    if allow_outside {
        return RegionClamp {
            position,
            wrote_transform: false,
        };
    }
    let x = clamp_axis(position[0], EDGE_MARGIN, region_x - EDGE_MARGIN);
    let y = clamp_axis(position[1], EDGE_MARGIN, region_y - EDGE_MARGIN);
    let moved = x != position[0] || y != position[1];
    RegionClamp {
        position: [x, y, position[2]],
        wrote_transform: moved && character_valid,
    }
}

fn clamp_axis(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Fields the constructor writes over `CharacterDesc.Default`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkerRequest {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
    pub size: [f32; 3],
    pub jump_speed: f32,
    pub subscribed_ms: i32,
    pub user_data: u32,
}

/// The `CharacterDesc` `CreateCharacter` receives. This is the descriptor, not
/// a native kinematic body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkerDesc {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
    pub half_height: f32,
    pub radius: f32,
    pub mass: f32,
    pub friction: f32,
    pub max_slope_angle: f32,
    pub step_height: f32,
    pub push_strength: f32,
    pub jump_speed: f32,
    pub wants_contact_events: bool,
    pub user_data: u32,
}

/// `CreateCharacterInternal`. Contact events are wanted only when `subscribed_ms`
/// is already above zero, which it is not at construction.
#[must_use]
pub fn marker_descriptor(request: MarkerRequest) -> MarkerDesc {
    let capsule = capsule_from_size(request.size);
    MarkerDesc {
        position: request.position,
        orientation: request.orientation,
        half_height: capsule.half_height,
        radius: capsule.radius,
        mass: mass_for(request.size),
        friction: DEFAULT_FRICTION,
        max_slope_angle: MAX_SLOPE_DEGREES * (std::f32::consts::PI / 180.0),
        step_height: DEFAULT_STEP_HEIGHT,
        push_strength: DEFAULT_PUSH_STRENGTH,
        jump_speed: request.jump_speed,
        wants_contact_events: request.subscribed_ms > 0,
        user_data: request.user_data,
    }
}

/// Size, capsule and mass the `Size` setter stores.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeState {
    pub size: [f32; 3],
    pub half_height: f32,
    pub radius: f32,
    pub mass: f32,
}

impl ShapeState {
    /// The constructor's size path. The size is stored as given, including a
    /// negative, zero or non-finite axis. `MathF.Max` and `MathF.Min` pass a
    /// NaN through, so one NaN axis makes the capsule and the mass NaN.
    #[must_use]
    pub fn new(size: [f32; 3]) -> Self {
        let capsule = capsule_from_size(size);
        Self {
            size,
            half_height: capsule.half_height,
            radius: capsule.radius,
            mass: mass_for(size),
        }
    }

    /// `Size` setter. Returns whether mass and shape were forwarded.
    /// A non-finite component, or the same size again, forwards nothing.
    pub fn set_size(&mut self, next: [f32; 3]) -> bool {
        if !finite3(next) || self.size == next {
            return false;
        }
        let capsule = capsule_from_size(next);
        self.size = next;
        self.half_height = capsule.half_height;
        self.radius = capsule.radius;
        self.mass = mass_for(next);
        true
    }
}

/// `Buoyancy` setter. A finite value is stored and forwarded. A non-finite
/// value leaves `stored` in place. The gravity multiply is not this setter.
#[must_use]
pub fn store_buoyancy(stored: f32, value: f32) -> (bool, f32) {
    if value.is_finite() {
        (true, value)
    } else {
        (false, stored)
    }
}

/// `TargetVelocity` setter. `None` means the previous value stays and no
/// movement is pushed.
#[must_use]
pub fn store_target(next: [f32; 3]) -> Option<[f32; 3]> {
    if finite3(next) { Some(next) } else { None }
}

/// `SubscribeEvents`: the backend wants contact events when `ms` is above zero.
#[must_use]
pub fn contact_wanted_after_subscribe(ms: i32) -> bool {
    ms > 0
}

/// `UnSubscribeEvents` always clears the contact gate.
#[must_use]
pub fn contact_wanted_after_unsubscribe() -> bool {
    false
}

fn finite3(value: [f32; 3]) -> bool {
    value[0].is_finite() && value[1].is_finite() && value[2].is_finite()
}
