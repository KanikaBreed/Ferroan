// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltConfig.cs
//   Source/OpenSim.Region.PhysicsModules.Jolt/JoltScene.cs (TestCommandsEnabled)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// This port remains available under MPL-2.0 and forms part of Ferroan's
// GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! `[Jolt]` as `JoltConfig.FromConfig` reads it.
//!
//! `JoltConfig.cs` 198–299 parses every key. `F`, `I`, `B` and `E` at 460–519
//! reject a value that does not parse or sits outside the inclusive range, and
//! the field keeps its default. `EffectivePhysicsStepRate` at 361–379 refuses
//! a rate below the heartbeat. Absent keys leave the field initializers, which
//! are the defaults P3.2a hardcodes.
//!
//! `JoltScene.TestCommandsEnabled` (`JoltScene.cs` 75–76, called from
//! `Initialise` at 362) reads `[Jolt] TestCommands` through Nini `GetBoolean`.
//! Anything but `true` or `false`, trimmed, any case, throws. Nothing catches
//! it until `RegionRuntime` (99–113) logs `[STARTUP]: [OPENSIM MAIN]: Fatal
//! error while starting the region runtime.` and the region server does not
//! start (TB161). That throw is against the module's own rule (`JoltConfig.cs`
//! 14–15). Ferroan warns and keeps false (D121). `FromConfig`'s `F`, `I`, `B`
//! and `E` still do not throw.
//!
//! When `[Startup] physics` is not `Jolt`, [`load`] logs one warning that names
//! the setting and the value, and still returns this configuration ([D110]).

use ferroan_config::Config;

const SECTION: &str = "Jolt";
const MAX_JOB_POOLS: i32 = 64;
const MAX_FRICTION: f32 = 255.0;
const MIN_TIMESCALE: f32 = 0.001;
const MAX_TIMESCALE: f32 = 1000.0;
const MAX_PHYSICS_STEP_RATE: f32 = 1000.0;
const MAX_RAY_CAST_HITS: i32 = 256;
const MIN_AVATAR_SPEED_FACTOR: f32 = 0.1;
const MAX_AVATAR_SPEED_FACTOR: f32 = 10.0;
const MIN_REGION_HEIGHT: f32 = -128.0;
const MAX_REGION_HEIGHT: f32 = 10_000.0;
const DEFAULT_FRAME_TIME: f32 = 0.0908;
const CORE_WALK_REQUEST: f32 = 4.096;
const CORE_FLY_REQUEST: f32 = CORE_WALK_REQUEST * 4.0;

/// Which values `llSetVehicleType` gives a vehicle type. `Jolt` `VehicleSettings.cs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VehiclePresetSet {
    Documented,
    Legacy,
}

/// Whose vehicle behaviour the region follows. `VehicleSettings.cs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VehicleBehaviour {
    SecondLife,
    InWorldz,
}

/// Whose basic-forces table a material uses. `JoltConfig.cs` 32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BasicForces {
    InWorldz,
    SecondLife,
}

/// How two surfaces combine. `IPhysicsBackend.cs` `MaterialCombine`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialCombine {
    Average,
    GeometricMean,
    Multiply,
    Min,
    Max,
}

const VEHICLE_PRESETS: &[&str] = &["Documented", "Legacy"];
const VEHICLE_BEHAVIOURS: &[&str] = &["SecondLife", "InWorldz"];
const BASIC_FORCES: &[&str] = &["InWorldz", "SecondLife"];
const MATERIAL_COMBINES: &[&str] = &["Average", "GeometricMean", "Multiply", "Min", "Max"];

/// One region's `[Jolt]` section after `FromConfig`.
#[derive(Clone, Debug, PartialEq)]
pub struct JoltConfig {
    pub gravity: f32,
    pub collision_steps: i32,
    pub position_iterations: i32,
    pub velocity_iterations: i32,
    pub thread_count: i32,
    pub deterministic_mode: bool,
    pub max_bodies: i32,
    pub max_body_pairs: i32,
    pub max_contact_constraints: i32,
    pub scale_caps_with_area: bool,
    pub body_update_buffer_max: i32,
    pub character_update_buffer_max: i32,
    pub contact_buffer_max: i32,
    pub avatar_jump_speed: f32,
    pub avatar_walk_speed_factor: f32,
    pub avatar_run_speed_factor: f32,
    pub avatar_fly_speed_factor: f32,
    pub avatar_fly_down_speed_factor: f32,
    pub capacity_log_interval_seconds: f32,
    pub metrics_log_interval_seconds: f32,
    pub job_pools: i32,
    pub job_pool_fair_handoff: bool,
    pub physics_step_rate: f32,
    pub physics_step_rate_set: bool,
    pub physics_step_collision_steps: i32,
    pub vehicle_ground_gravity_factor: f32,
    pub vehicle_presets: VehiclePresetSet,
    pub vehicle_presets_set: bool,
    pub vehicle_behaviour: VehicleBehaviour,
    pub vehicle_max_linear_speed: f32,
    pub vehicle_reference_speed: f32,
    pub vehicle_max_angular_speed: f32,
    pub vehicle_angular_limiter_floor: f32,
    pub vehicle_min_timescale: f32,
    pub vehicle_max_timescale: f32,
    pub vehicle_max_decay_timescale: f32,
    pub vehicle_max_decay_timescale_set: bool,
    pub vehicle_max_hover_timescale: f32,
    pub vehicle_max_attract_timescale: f32,
    pub vehicle_max_motor_offset: f32,
    pub vehicle_min_hover_height: f32,
    pub vehicle_max_hover_height: f32,
    pub vehicle_sled_assist: f32,
    pub vehicle_spike_mitigation: Option<bool>,
    pub vehicle_motor_target_on_undriven_axes: bool,
    pub vehicle_contact_friction: Option<f32>,
    pub sled_contact_friction: f32,
    pub basic_forces_behaviour: BasicForces,
    pub friction_combine: MaterialCombine,
    pub restitution_combine: MaterialCombine,
    pub terrain_friction: f32,
    pub terrain_restitution: f32,
    pub vehicle_rest_speed: f32,
    pub body_max_linear_speed: f32,
    pub body_max_angular_speed: f32,
    pub prim_linear_damping: f32,
    pub prim_angular_damping: f32,
    pub allow_unrecorded_native: bool,
    pub honour_convex_mesh: bool,
    pub max_physical_complexity: i32,
    pub body_enhanced_edge_removal: bool,
    pub avatar_enhanced_edge_removal: bool,
    pub two_sided_mesh_collision: bool,
    pub speculative_contact_distance: f32,
    pub sleep_velocity_threshold: f32,
    pub keyframed_motion_carries_riders: bool,
    pub moved_prims_carry_riders: bool,
    pub physics_sit: bool,
    pub ray_cast_budget_ms: f32,
    pub ray_cast_simulator_budget_ms: f32,
    pub ray_cast_max_tested_hits: i32,
    pub ray_cast_max_hits: i32,
    pub camera_ray_budget_ms: f32,
    pub avatar_push_max_speed: f32,
    pub avatar_push_recovery: f32,
    pub avatar_object_push_per_momentum: f32,
    pub avatar_object_push_max_force: f32,
}

impl Default for JoltConfig {
    fn default() -> Self {
        Self {
            gravity: -9.80665,
            collision_steps: 6,
            position_iterations: 2,
            velocity_iterations: 20,
            thread_count: 0,
            deterministic_mode: false,
            max_bodies: 0,
            max_body_pairs: 65_536,
            max_contact_constraints: 16_384,
            scale_caps_with_area: false,
            body_update_buffer_max: 65_536,
            character_update_buffer_max: 1024,
            contact_buffer_max: 0,
            avatar_jump_speed: 4.0,
            avatar_walk_speed_factor: 3.20 / CORE_WALK_REQUEST,
            avatar_run_speed_factor: 5.13 / CORE_WALK_REQUEST,
            avatar_fly_speed_factor: 16.00 / CORE_FLY_REQUEST,
            avatar_fly_down_speed_factor: 22.87 / CORE_FLY_REQUEST,
            capacity_log_interval_seconds: 10.0,
            metrics_log_interval_seconds: 30.0,
            job_pools: 1,
            job_pool_fair_handoff: false,
            physics_step_rate: 45.0,
            physics_step_rate_set: false,
            physics_step_collision_steps: 2,
            vehicle_ground_gravity_factor: 1.0,
            vehicle_presets: VehiclePresetSet::Documented,
            vehicle_presets_set: false,
            vehicle_behaviour: VehicleBehaviour::InWorldz,
            vehicle_max_linear_speed: 200.0,
            vehicle_reference_speed: 30.0,
            vehicle_max_angular_speed: (std::f64::consts::PI * 4.0) as f32,
            vehicle_angular_limiter_floor: 0.1,
            vehicle_min_timescale: 0.0156,
            vehicle_max_timescale: MAX_TIMESCALE,
            vehicle_max_decay_timescale: 120.0,
            vehicle_max_decay_timescale_set: false,
            vehicle_max_hover_timescale: 300.0,
            vehicle_max_attract_timescale: 500.0,
            vehicle_max_motor_offset: 100.0,
            vehicle_min_hover_height: MIN_REGION_HEIGHT,
            vehicle_max_hover_height: MAX_REGION_HEIGHT,
            vehicle_sled_assist: 0.045,
            vehicle_spike_mitigation: None,
            vehicle_motor_target_on_undriven_axes: false,
            vehicle_contact_friction: None,
            sled_contact_friction: 0.0,
            basic_forces_behaviour: BasicForces::InWorldz,
            friction_combine: MaterialCombine::Average,
            restitution_combine: MaterialCombine::Average,
            terrain_friction: 0.4,
            terrain_restitution: 0.05,
            vehicle_rest_speed: 0.1,
            body_max_linear_speed: 500.0,
            body_max_angular_speed: 0.25 * std::f32::consts::PI * 60.0,
            prim_linear_damping: 0.02,
            prim_angular_damping: 0.05,
            allow_unrecorded_native: false,
            honour_convex_mesh: false,
            max_physical_complexity: 0,
            body_enhanced_edge_removal: false,
            avatar_enhanced_edge_removal: false,
            two_sided_mesh_collision: false,
            speculative_contact_distance: 0.02,
            sleep_velocity_threshold: 0.03,
            keyframed_motion_carries_riders: true,
            moved_prims_carry_riders: false,
            physics_sit: true,
            ray_cast_budget_ms: 5.0,
            ray_cast_simulator_budget_ms: 5.0,
            ray_cast_max_tested_hits: 1024,
            ray_cast_max_hits: MAX_RAY_CAST_HITS,
            camera_ray_budget_ms: 30.0,
            avatar_push_max_speed: 100.0,
            avatar_push_recovery: 5.0,
            avatar_object_push_per_momentum: 100.0,
            avatar_object_push_max_force: 2000.0,
        }
    }
}

impl JoltConfig {
    /// `EffectivePresets` (`JoltConfig.cs` 305–308).
    pub fn effective_presets(&self) -> VehiclePresetSet {
        if self.vehicle_presets_set {
            self.vehicle_presets
        } else if self.vehicle_behaviour == VehicleBehaviour::InWorldz {
            VehiclePresetSet::Legacy
        } else {
            VehiclePresetSet::Documented
        }
    }

    /// `EffectivePhysicsStepRate` (`JoltConfig.cs` 361–379).
    ///
    /// The rate is this config's `physics_step_rate`, or 0 when that rate is
    /// off or below the heartbeat. A rate the operator set fills `warning`.
    /// An unset default that is too slow fills `note`.
    pub fn effective_physics_step_rate(
        &self,
        heartbeat_seconds: f32,
    ) -> (f32, Option<String>, Option<String>) {
        if self.physics_step_rate <= 0.0 {
            return (0.0, None, None);
        }
        // NaN is not greater than zero, matching `!(heartbeat > 0f)`.
        let below = heartbeat_seconds.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            || self.physics_step_rate * heartbeat_seconds < 1.0_f32 - 1e-3_f32;
        if !below {
            return (self.physics_step_rate, None, None);
        }
        let rate = csharp_g(self.physics_step_rate);
        let hertz = if heartbeat_seconds > 0.0 {
            1.0 / heartbeat_seconds
        } else {
            0.0
        };
        let heartbeat = format_hash2(hertz);
        if self.physics_step_rate_set {
            (
                0.0,
                Some(format!(
                    "[Jolt] PhysicsStepRate = {rate} is below the heartbeat's own rate ({heartbeat} Hz, [Startup] FrameTime); using 0, one physics step per heartbeat."
                )),
                None,
            )
        } else {
            (
                0.0,
                None,
                Some(format!(
                    "[Jolt] PhysicsStepRate is not set, and its default of {rate} Hz is below the heartbeat's own rate ({heartbeat} Hz, [Startup] FrameTime); running one physics step per heartbeat."
                )),
            )
        }
    }
}

/// What [`load`] read, including the warnings it logged.
#[derive(Clone, Debug, PartialEq)]
pub struct JoltLoad {
    pub config: JoltConfig,
    /// `FromConfig`'s warnings, in that method's order.
    pub warnings: Vec<String>,
    pub step_warning: Option<String>,
    pub step_note: Option<String>,
    /// One line when `[Startup] physics` is not `Jolt`. Logged once.
    pub engine_warning: Option<String>,
    /// A bad `TestCommands` value. Logged once, and the flag stays false.
    pub test_commands_warning: Option<String>,
    pub effective_step_rate: f32,
    pub test_commands: bool,
    /// `[Startup] FrameTime`, or `0.0908` when the key is absent or unparsable.
    /// `Scene.cs` sets `FrameTime` to `0.0908f` and then `GetFloat("FrameTime", FrameTime)`.
    pub heartbeat_seconds: f32,
}

/// The warning [`load`] logs when `physics` is not the exact string `Jolt`.
pub fn startup_physics_warning(value: &str) -> Option<String> {
    if value == "Jolt" {
        None
    } else {
        Some(format!(
            "[Startup] physics = \"{value}\" is not Jolt; running Jolt."
        ))
    }
}

/// Reads `[Jolt]` and `[Startup] physics` / `FrameTime`.
///
/// Logs the engine warning once when the physics value is not `Jolt`, then
/// each `FromConfig` warning, then the step-rate warning. The step-rate note
/// is information. The returned config is Jolt's either way.
pub fn load(config: &Config) -> JoltLoad {
    let physics = config.get("Startup", "physics").unwrap_or("");
    let engine_warning = startup_physics_warning(physics);
    if let Some(warning) = &engine_warning {
        tracing::warn!("{warning}");
    }
    let heartbeat_seconds = frame_time(config);
    let (parsed, warnings, test_commands, test_commands_warning) = from_config(config);
    for warning in &warnings {
        tracing::warn!("{warning}");
    }
    if let Some(warning) = &test_commands_warning {
        tracing::warn!("{warning}");
    }
    let (effective_step_rate, step_warning, step_note) =
        parsed.effective_physics_step_rate(heartbeat_seconds);
    if let Some(warning) = &step_warning {
        tracing::warn!("{warning}");
    }
    if let Some(note) = &step_note {
        tracing::info!("{note}");
    }
    JoltLoad {
        config: parsed,
        warnings,
        step_warning,
        step_note,
        engine_warning,
        test_commands_warning,
        effective_step_rate,
        test_commands,
        heartbeat_seconds,
    }
}

fn frame_time(config: &Config) -> f32 {
    match config.get("Startup", "FrameTime") {
        Some(raw) => try_f(raw, f32::MIN, f32::MAX).unwrap_or(DEFAULT_FRAME_TIME),
        None => DEFAULT_FRAME_TIME,
    }
}

fn from_config(config: &Config) -> (JoltConfig, Vec<String>, bool, Option<String>) {
    let mut parsed = JoltConfig::default();
    let mut warnings = Vec::new();
    if !config.section_names().any(|name| name == SECTION) {
        return (parsed, warnings, false, None);
    }
    parsed.gravity = f(
        config,
        "Gravity",
        parsed.gravity,
        -1000.0,
        1000.0,
        &mut warnings,
    );
    parsed.collision_steps = i(
        config,
        "CollisionSteps",
        parsed.collision_steps,
        1,
        64,
        &mut warnings,
    );
    parsed.position_iterations = i(
        config,
        "PositionIterations",
        parsed.position_iterations,
        1,
        255,
        &mut warnings,
    );
    parsed.velocity_iterations = i(
        config,
        "VelocityIterations",
        parsed.velocity_iterations,
        1,
        255,
        &mut warnings,
    );
    parsed.thread_count = i(
        config,
        "ThreadCount",
        parsed.thread_count,
        0,
        256,
        &mut warnings,
    );
    parsed.deterministic_mode = b(
        config,
        "DeterministicMode",
        parsed.deterministic_mode,
        &mut warnings,
    );
    parsed.max_bodies = i(
        config,
        "MaxBodies",
        parsed.max_bodies,
        0,
        8_388_607,
        &mut warnings,
    );
    parsed.max_body_pairs = i(
        config,
        "MaxBodyPairs",
        parsed.max_body_pairs,
        1,
        8_388_607,
        &mut warnings,
    );
    parsed.max_contact_constraints = i(
        config,
        "MaxContactConstraints",
        parsed.max_contact_constraints,
        1,
        8_388_607,
        &mut warnings,
    );
    parsed.scale_caps_with_area = b(
        config,
        "ScaleCapsWithArea",
        parsed.scale_caps_with_area,
        &mut warnings,
    );
    parsed.allow_unrecorded_native = b(
        config,
        "AllowUnrecordedNative",
        parsed.allow_unrecorded_native,
        &mut warnings,
    );
    parsed.honour_convex_mesh = b(
        config,
        "HonourConvexMesh",
        parsed.honour_convex_mesh,
        &mut warnings,
    );
    parsed.body_enhanced_edge_removal = b(
        config,
        "BodyEnhancedEdgeRemoval",
        parsed.body_enhanced_edge_removal,
        &mut warnings,
    );
    parsed.avatar_enhanced_edge_removal = b(
        config,
        "AvatarEnhancedEdgeRemoval",
        parsed.avatar_enhanced_edge_removal,
        &mut warnings,
    );
    parsed.two_sided_mesh_collision = b(
        config,
        "TwoSidedMeshCollision",
        parsed.two_sided_mesh_collision,
        &mut warnings,
    );
    parsed.speculative_contact_distance = f(
        config,
        "SpeculativeContactDistance",
        parsed.speculative_contact_distance,
        0.0,
        0.5,
        &mut warnings,
    );
    parsed.sleep_velocity_threshold = f(
        config,
        "SleepVelocityThreshold",
        parsed.sleep_velocity_threshold,
        0.001,
        1.0,
        &mut warnings,
    );
    parsed.keyframed_motion_carries_riders = b(
        config,
        "KeyframedMotionCarriesRiders",
        parsed.keyframed_motion_carries_riders,
        &mut warnings,
    );
    parsed.moved_prims_carry_riders = b(
        config,
        "MovedPrimsCarryRiders",
        parsed.moved_prims_carry_riders,
        &mut warnings,
    );
    parsed.physics_sit = b(config, "PhysicsSit", parsed.physics_sit, &mut warnings);
    parsed.body_update_buffer_max = i(
        config,
        "BodyUpdateBufferMax",
        parsed.body_update_buffer_max,
        1,
        8_388_607,
        &mut warnings,
    );
    parsed.character_update_buffer_max = i(
        config,
        "CharacterUpdateBufferMax",
        parsed.character_update_buffer_max,
        1,
        65_536,
        &mut warnings,
    );
    parsed.contact_buffer_max = i(
        config,
        "ContactBufferMax",
        parsed.contact_buffer_max,
        0,
        16_777_216,
        &mut warnings,
    );
    parsed.avatar_jump_speed = f(
        config,
        "AvatarJumpSpeed",
        parsed.avatar_jump_speed,
        0.0,
        100.0,
        &mut warnings,
    );
    parsed.avatar_walk_speed_factor = f(
        config,
        "AvatarWalkSpeedFactor",
        parsed.avatar_walk_speed_factor,
        MIN_AVATAR_SPEED_FACTOR,
        MAX_AVATAR_SPEED_FACTOR,
        &mut warnings,
    );
    parsed.avatar_run_speed_factor = f(
        config,
        "AvatarRunSpeedFactor",
        parsed.avatar_run_speed_factor,
        MIN_AVATAR_SPEED_FACTOR,
        MAX_AVATAR_SPEED_FACTOR,
        &mut warnings,
    );
    parsed.avatar_fly_speed_factor = f(
        config,
        "AvatarFlySpeedFactor",
        parsed.avatar_fly_speed_factor,
        MIN_AVATAR_SPEED_FACTOR,
        MAX_AVATAR_SPEED_FACTOR,
        &mut warnings,
    );
    parsed.avatar_fly_down_speed_factor = f(
        config,
        "AvatarFlyDownSpeedFactor",
        parsed.avatar_fly_down_speed_factor,
        MIN_AVATAR_SPEED_FACTOR,
        MAX_AVATAR_SPEED_FACTOR,
        &mut warnings,
    );
    parsed.capacity_log_interval_seconds = f(
        config,
        "CapacityLogIntervalSeconds",
        parsed.capacity_log_interval_seconds,
        0.1,
        86_400.0,
        &mut warnings,
    );
    parsed.metrics_log_interval_seconds = f(
        config,
        "MetricsLogIntervalSeconds",
        parsed.metrics_log_interval_seconds,
        0.0,
        86_400.0,
        &mut warnings,
    );
    parsed.job_pools = i(
        config,
        "JobPools",
        parsed.job_pools,
        1,
        MAX_JOB_POOLS,
        &mut warnings,
    );
    parsed.job_pool_fair_handoff = b(
        config,
        "JobPoolFairHandoff",
        parsed.job_pool_fair_handoff,
        &mut warnings,
    );
    parsed.physics_step_rate = f(
        config,
        "PhysicsStepRate",
        parsed.physics_step_rate,
        0.0,
        MAX_PHYSICS_STEP_RATE,
        &mut warnings,
    );
    parsed.physics_step_rate_set = config
        .get(SECTION, "PhysicsStepRate")
        .is_some_and(|raw| try_f(raw, 0.0, MAX_PHYSICS_STEP_RATE).is_some());
    parsed.physics_step_collision_steps = i(
        config,
        "PhysicsStepCollisionSteps",
        parsed.physics_step_collision_steps,
        1,
        64,
        &mut warnings,
    );
    parsed.vehicle_ground_gravity_factor = f(
        config,
        "VehicleGroundGravityFactor",
        parsed.vehicle_ground_gravity_factor,
        0.0,
        1.0,
        &mut warnings,
    );
    let (presets, presets_valid) = e(
        config,
        "VehiclePresets",
        parsed.vehicle_presets,
        VEHICLE_PRESETS,
        &mut warnings,
    );
    parsed.vehicle_presets = presets;
    parsed.vehicle_presets_set = presets_valid && config.get(SECTION, "VehiclePresets").is_some();
    parsed.vehicle_behaviour = e(
        config,
        "VehicleBehaviour",
        parsed.vehicle_behaviour,
        VEHICLE_BEHAVIOURS,
        &mut warnings,
    )
    .0;
    parsed.vehicle_max_linear_speed = f(
        config,
        "VehicleMaxLinearSpeed",
        parsed.vehicle_max_linear_speed,
        1.0,
        10_000.0,
        &mut warnings,
    );
    parsed.vehicle_reference_speed = f(
        config,
        "VehicleReferenceSpeed",
        parsed.vehicle_reference_speed,
        0.1,
        10_000.0,
        &mut warnings,
    );
    parsed.vehicle_max_angular_speed = f(
        config,
        "VehicleMaxAngularSpeed",
        parsed.vehicle_max_angular_speed,
        0.1,
        1000.0,
        &mut warnings,
    );
    parsed.vehicle_angular_limiter_floor = f(
        config,
        "VehicleAngularLimiterFloor",
        parsed.vehicle_angular_limiter_floor,
        0.0,
        1.0,
        &mut warnings,
    );
    parsed.vehicle_min_timescale = f(
        config,
        "VehicleMinTimescale",
        parsed.vehicle_min_timescale,
        MIN_TIMESCALE,
        1.0,
        &mut warnings,
    );
    parsed.vehicle_max_timescale = f(
        config,
        "VehicleMaxTimescale",
        parsed.vehicle_max_timescale,
        1.0,
        MAX_TIMESCALE,
        &mut warnings,
    );
    parsed.vehicle_max_decay_timescale = f(
        config,
        "VehicleMaxDecayTimescale",
        parsed.vehicle_max_decay_timescale,
        1.0,
        MAX_TIMESCALE,
        &mut warnings,
    );
    parsed.vehicle_max_decay_timescale_set = config
        .get(SECTION, "VehicleMaxDecayTimescale")
        .is_some_and(|raw| try_f(raw, 1.0, MAX_TIMESCALE).is_some());
    parsed.vehicle_max_hover_timescale = f(
        config,
        "VehicleMaxHoverTimescale",
        parsed.vehicle_max_hover_timescale,
        1.0,
        MAX_TIMESCALE,
        &mut warnings,
    );
    parsed.vehicle_max_attract_timescale = f(
        config,
        "VehicleMaxAttractTimescale",
        parsed.vehicle_max_attract_timescale,
        1.0,
        MAX_TIMESCALE,
        &mut warnings,
    );
    parsed.vehicle_max_motor_offset = f(
        config,
        "VehicleMaxMotorOffset",
        parsed.vehicle_max_motor_offset,
        0.0,
        10_000.0,
        &mut warnings,
    );
    parsed.vehicle_min_hover_height = f(
        config,
        "VehicleMinHoverHeight",
        parsed.vehicle_min_hover_height,
        -100_000.0,
        100_000.0,
        &mut warnings,
    );
    parsed.vehicle_max_hover_height = f(
        config,
        "VehicleMaxHoverHeight",
        parsed.vehicle_max_hover_height,
        -100_000.0,
        100_000.0,
        &mut warnings,
    );
    if parsed.vehicle_min_hover_height > parsed.vehicle_max_hover_height {
        warnings.push(format!(
            "[Jolt] VehicleMinHoverHeight ({}) is above VehicleMaxHoverHeight ({}); using the defaults for both.",
            csharp_g(parsed.vehicle_min_hover_height),
            csharp_g(parsed.vehicle_max_hover_height)
        ));
        parsed.vehicle_min_hover_height = MIN_REGION_HEIGHT;
        parsed.vehicle_max_hover_height = MAX_REGION_HEIGHT;
    }
    parsed.vehicle_sled_assist = f(
        config,
        "VehicleSledAssist",
        parsed.vehicle_sled_assist,
        0.0,
        10.0,
        &mut warnings,
    );
    if let Some(raw) = config.get(SECTION, "VehicleSpikeMitigation") {
        if let Some(value) = try_b(raw) {
            parsed.vehicle_spike_mitigation = Some(value);
        } else {
            warnings.push(format!(
                "[Jolt] VehicleSpikeMitigation = \"{raw}\" is invalid (expected true or false); using the behaviour's own choice."
            ));
        }
    }
    parsed.vehicle_motor_target_on_undriven_axes = b(
        config,
        "VehicleMotorTargetOnUndrivenAxes",
        parsed.vehicle_motor_target_on_undriven_axes,
        &mut warnings,
    );
    if let Some(raw) = config.get(SECTION, "VehicleContactFriction")
        && !raw.trim().is_empty()
    {
        if let Some(value) = try_f(raw, 0.0, 10.0) {
            parsed.vehicle_contact_friction = Some(value);
        } else {
            warnings.push(format!(
                "[Jolt] VehicleContactFriction = \"{raw}\" is invalid (expected a finite number in [0, 10], or nothing for the material's value); using the material's."
            ));
        }
    }
    parsed.sled_contact_friction = f(
        config,
        "SledContactFriction",
        parsed.sled_contact_friction,
        0.0,
        10.0,
        &mut warnings,
    );
    parsed.basic_forces_behaviour = e(
        config,
        "BasicForcesBehaviour",
        parsed.basic_forces_behaviour,
        BASIC_FORCES,
        &mut warnings,
    )
    .0;
    parsed.friction_combine = e(
        config,
        "FrictionCombine",
        parsed.friction_combine,
        MATERIAL_COMBINES,
        &mut warnings,
    )
    .0;
    parsed.restitution_combine = e(
        config,
        "RestitutionCombine",
        parsed.restitution_combine,
        MATERIAL_COMBINES,
        &mut warnings,
    )
    .0;
    parsed.terrain_friction = f(
        config,
        "TerrainFriction",
        parsed.terrain_friction,
        0.0,
        MAX_FRICTION,
        &mut warnings,
    );
    parsed.terrain_restitution = f(
        config,
        "TerrainRestitution",
        parsed.terrain_restitution,
        0.0,
        1.0,
        &mut warnings,
    );
    parsed.vehicle_rest_speed = f(
        config,
        "VehicleRestSpeed",
        parsed.vehicle_rest_speed,
        0.0,
        1.0,
        &mut warnings,
    );
    parsed.body_max_linear_speed = f(
        config,
        "BodyMaxLinearSpeed",
        parsed.body_max_linear_speed,
        1.0,
        100_000.0,
        &mut warnings,
    );
    parsed.body_max_angular_speed = f(
        config,
        "BodyMaxAngularSpeed",
        parsed.body_max_angular_speed,
        0.1,
        10_000.0,
        &mut warnings,
    );
    parsed.max_physical_complexity = i(
        config,
        "MaxPhysicalComplexity",
        parsed.max_physical_complexity,
        0,
        1_000_000,
        &mut warnings,
    );
    parsed.prim_linear_damping = f(
        config,
        "PrimLinearDamping",
        parsed.prim_linear_damping,
        0.0,
        100.0,
        &mut warnings,
    );
    parsed.prim_angular_damping = f(
        config,
        "PrimAngularDamping",
        parsed.prim_angular_damping,
        0.0,
        100.0,
        &mut warnings,
    );
    parsed.ray_cast_budget_ms = f(
        config,
        "RayCastBudgetMs",
        parsed.ray_cast_budget_ms,
        0.1,
        1000.0,
        &mut warnings,
    );
    parsed.ray_cast_simulator_budget_ms = f(
        config,
        "RayCastSimulatorBudgetMs",
        parsed.ray_cast_simulator_budget_ms,
        0.1,
        1000.0,
        &mut warnings,
    );
    parsed.ray_cast_max_tested_hits = i(
        config,
        "RayCastMaxTestedHits",
        parsed.ray_cast_max_tested_hits,
        1,
        1_000_000,
        &mut warnings,
    );
    parsed.ray_cast_max_hits = i(
        config,
        "RayCastMaxHits",
        parsed.ray_cast_max_hits,
        1,
        MAX_RAY_CAST_HITS,
        &mut warnings,
    );
    parsed.camera_ray_budget_ms = f(
        config,
        "CameraRayBudgetMs",
        parsed.camera_ray_budget_ms,
        0.0,
        1000.0,
        &mut warnings,
    );
    parsed.avatar_push_max_speed = f(
        config,
        "AvatarPushMaxSpeed",
        parsed.avatar_push_max_speed,
        0.0,
        1000.0,
        &mut warnings,
    );
    parsed.avatar_push_recovery = f(
        config,
        "AvatarPushRecovery",
        parsed.avatar_push_recovery,
        0.0,
        1000.0,
        &mut warnings,
    );
    parsed.avatar_object_push_per_momentum = f(
        config,
        "AvatarObjectPushPerMomentum",
        parsed.avatar_object_push_per_momentum,
        0.0,
        100_000.0,
        &mut warnings,
    );
    parsed.avatar_object_push_max_force = f(
        config,
        "AvatarObjectPushMaxForce",
        parsed.avatar_object_push_max_force,
        0.0,
        1_000_000.0,
        &mut warnings,
    );
    let (test_commands, test_commands_warning) = test_commands(config);
    (parsed, warnings, test_commands, test_commands_warning)
}

fn test_commands(config: &Config) -> (bool, Option<String>) {
    let Some(raw) = config.get(SECTION, "TestCommands") else {
        return (false, None);
    };
    if let Some(value) = try_b(raw) {
        (value, None)
    } else {
        (
            false,
            Some(format!(
                "[Jolt] TestCommands = \"{raw}\" is invalid (expected true or false); using the default False."
            )),
        )
    }
}

fn f(
    config: &Config,
    key: &str,
    default: f32,
    min: f32,
    max: f32,
    warnings: &mut Vec<String>,
) -> f32 {
    let Some(raw) = config.get(SECTION, key) else {
        return default;
    };
    if let Some(value) = try_f(raw, min, max) {
        value
    } else {
        warnings.push(format!(
            "[Jolt] {key} = \"{raw}\" is invalid (expected a finite number in [{}, {}]); using the default {}.",
            csharp_g(min),
            csharp_g(max),
            csharp_g(default)
        ));
        default
    }
}

fn i(
    config: &Config,
    key: &str,
    default: i32,
    min: i32,
    max: i32,
    warnings: &mut Vec<String>,
) -> i32 {
    let Some(raw) = config.get(SECTION, key) else {
        return default;
    };
    if let Some(value) = try_i(raw, min, max) {
        value
    } else {
        warnings.push(format!(
            "[Jolt] {key} = \"{raw}\" is invalid (expected an integer in [{min}, {max}]); using the default {default}."
        ));
        default
    }
}

fn b(config: &Config, key: &str, default: bool, warnings: &mut Vec<String>) -> bool {
    let Some(raw) = config.get(SECTION, key) else {
        return default;
    };
    if let Some(value) = try_b(raw) {
        value
    } else {
        warnings.push(format!(
            "[Jolt] {key} = \"{raw}\" is invalid (expected true or false); using the default {}.",
            csharp_bool(default)
        ));
        default
    }
}

fn e<T>(
    config: &Config,
    key: &str,
    default: T,
    names: &[&str],
    warnings: &mut Vec<String>,
) -> (T, bool)
where
    T: Copy + Named,
{
    let Some(raw) = config.get(SECTION, key) else {
        return (default, false);
    };
    if let Some(name) = try_e(raw, names) {
        (T::from_name(name), true)
    } else {
        warnings.push(format!(
            "[Jolt] {key} = \"{raw}\" is invalid (expected one of {}); using the default {}.",
            names.join(", "),
            default.name()
        ));
        (default, false)
    }
}

trait Named: Copy {
    fn name(self) -> &'static str;
    fn from_name(name: &str) -> Self;
}

impl Named for VehiclePresetSet {
    fn name(self) -> &'static str {
        match self {
            Self::Documented => "Documented",
            Self::Legacy => "Legacy",
        }
    }
    fn from_name(name: &str) -> Self {
        match name {
            "Legacy" => Self::Legacy,
            _ => Self::Documented,
        }
    }
}

impl Named for VehicleBehaviour {
    fn name(self) -> &'static str {
        match self {
            Self::SecondLife => "SecondLife",
            Self::InWorldz => "InWorldz",
        }
    }
    fn from_name(name: &str) -> Self {
        match name {
            "SecondLife" => Self::SecondLife,
            _ => Self::InWorldz,
        }
    }
}

impl Named for BasicForces {
    fn name(self) -> &'static str {
        match self {
            Self::InWorldz => "InWorldz",
            Self::SecondLife => "SecondLife",
        }
    }
    fn from_name(name: &str) -> Self {
        match name {
            "SecondLife" => Self::SecondLife,
            _ => Self::InWorldz,
        }
    }
}

impl Named for MaterialCombine {
    fn name(self) -> &'static str {
        match self {
            Self::Average => "Average",
            Self::GeometricMean => "GeometricMean",
            Self::Multiply => "Multiply",
            Self::Min => "Min",
            Self::Max => "Max",
        }
    }
    fn from_name(name: &str) -> Self {
        match name {
            "GeometricMean" => Self::GeometricMean,
            "Multiply" => Self::Multiply,
            "Min" => Self::Min,
            "Max" => Self::Max,
            _ => Self::Average,
        }
    }
}

fn try_f(raw: &str, min: f32, max: f32) -> Option<f32> {
    let value: f32 = raw.trim().parse().ok()?;
    if value.is_finite() && value >= min && value <= max {
        Some(value)
    } else {
        None
    }
}

fn try_i(raw: &str, min: i32, max: i32) -> Option<i32> {
    let value: i32 = raw.trim().parse().ok()?;
    if (min..=max).contains(&value) {
        Some(value)
    } else {
        None
    }
}

fn try_b(raw: &str) -> Option<bool> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        Some(true)
    } else if trimmed.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

/// `E<T>` (`JoltConfig.cs` 496–509) after `Enum.TryParse(ignoreCase)` and
/// `Enum.IsDefined`.
///
/// A leading Unicode digit or `-` is rejected before the parse. A leading `+`
/// is one signed integer (`+1`, `+0`, `+01`). Any other string is a comma
/// list of names: split, trim, match ignoring case, and OR the values. A name
/// list does not accept numbers (`Average, +1` is invalid). The OR must be
/// exactly one named value.
fn try_e<'a>(raw: &str, names: &'a [&'a str]) -> Option<&'a str> {
    let name = raw.trim();
    let first = name.chars().next()?;
    if first.is_digit(10) || first == '-' {
        return None;
    }
    let combined = if first == '+' {
        match name.parse::<i32>() {
            Ok(value) => value,
            Err(_) => parse_enum_names(name, names)?,
        }
    } else {
        parse_enum_names(name, names)?
    };
    names.get(usize::try_from(combined).ok()?).copied()
}

/// Comma-separated names, as `Enum.TryParse` ORs them. An empty part fails.
fn parse_enum_names(name: &str, names: &[&str]) -> Option<i32> {
    if name.is_empty() {
        return None;
    }
    let mut combined = 0i32;
    for part in name.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        let index = names
            .iter()
            .position(|item| item.eq_ignore_ascii_case(part))?;
        combined |= i32::try_from(index).ok()?;
    }
    Some(combined)
}

fn csharp_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

/// `float.ToString(CultureInfo.InvariantCulture)` for the values this reader formats.
pub(crate) fn csharp_g(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    format!("{value}")
}

/// `float.ToString("0.##", CultureInfo.InvariantCulture)`.
fn format_hash2(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if !value.is_finite() {
        return csharp_g(value);
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    let negative = value.is_sign_negative();
    let scaled = (value.abs() as f64) * 100.0;
    let rounded = scaled.round() as u64;
    let whole = rounded / 100;
    let frac = (rounded % 100) as u8;
    let mut text = if negative {
        format!("-{whole}")
    } else {
        whole.to_string()
    };
    if frac != 0 {
        text.push('.');
        if frac.is_multiple_of(10) {
            text.push(char::from(b'0' + frac / 10));
        } else {
            text.push(char::from(b'0' + frac / 10));
            text.push(char::from(b'0' + frac % 10));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(hex: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(hex, 16).unwrap())
    }

    #[test]
    fn invariant_general_matches_the_pin_defaults() {
        assert_eq!(csharp_g(bits("C11CE80A")), "-9.80665");
        assert_eq!(csharp_g(bits("3F480000")), "0.78125");
        assert_eq!(csharp_g(bits("3FA05000")), "1.2524414");
        assert_eq!(csharp_g(bits("3F79FFFF")), "0.97656244");
        assert_eq!(csharp_g(bits("3FB2AC00")), "1.395874");
        assert_eq!(csharp_g(bits("3D3851EC")), "0.045");
        assert_eq!(csharp_g(bits("3C7F9724")), "0.0156");
        assert_eq!(csharp_g(bits("41490FDB")), "12.566371");
        assert_eq!(csharp_g(bits("423C7EDD")), "47.12389");
        assert_eq!(csharp_g(bits("41300482")), "11.001101");
        assert_eq!(csharp_g(bits("40800000")), "4");
        assert_eq!(csharp_g(bits("80000000")), "-0");
        assert_eq!(format_hash2(bits("41300482")), "11");
        assert_eq!(format_hash2(100.0), "100");
        assert_eq!(
            JoltConfig::default().avatar_fly_speed_factor.to_bits(),
            bits("3F79FFFF").to_bits()
        );
        assert_eq!(
            JoltConfig::default().body_max_angular_speed.to_bits(),
            bits("423C7EDD").to_bits()
        );
        assert_eq!(
            JoltConfig::default().vehicle_max_angular_speed.to_bits(),
            bits("41490FDB").to_bits()
        );
    }

    #[test]
    fn enum_try_parse_matches_the_pin() {
        assert_eq!(try_e("+1", MATERIAL_COMBINES), Some("GeometricMean"));
        assert_eq!(try_e("Average, Min", MATERIAL_COMBINES), Some("Min"));
        assert_eq!(try_e("Min,Max", MATERIAL_COMBINES), None);
        assert_eq!(try_e("1", MATERIAL_COMBINES), None);
        assert_eq!(try_e("-Max", MATERIAL_COMBINES), None);
        assert_eq!(try_e("Average, +1", MATERIAL_COMBINES), None);
        assert_eq!(try_e("+0", VEHICLE_BEHAVIOURS), Some("SecondLife"));
        assert_eq!(
            try_e("SecondLife,InWorldz", VEHICLE_BEHAVIOURS),
            Some("InWorldz")
        );
        assert_eq!(try_e("+1", VEHICLE_PRESETS), Some("Legacy"));
        assert_eq!(try_e("+2", VEHICLE_PRESETS), None);
        assert_eq!(try_e("+0", BASIC_FORCES), Some("InWorldz"));
        assert_eq!(try_e("+1", BASIC_FORCES), Some("SecondLife"));
        assert_eq!(
            try_e("InWorldz, SecondLife", BASIC_FORCES),
            Some("SecondLife")
        );
        assert_eq!(try_e("Documented, Legacy", VEHICLE_PRESETS), Some("Legacy"));
    }

    #[test]
    fn a_non_jolt_engine_logs_one_warning_and_still_runs_jolt() {
        for (value, expected) in [
            (
                "ubODE",
                "[Startup] physics = \"ubODE\" is not Jolt; running Jolt.",
            ),
            (
                "BulletSim",
                "[Startup] physics = \"BulletSim\" is not Jolt; running Jolt.",
            ),
            (
                "basicphysics",
                "[Startup] physics = \"basicphysics\" is not Jolt; running Jolt.",
            ),
        ] {
            let text = format!("[Startup]\nphysics = {value}\nFrameTime = 0.0909\n");
            let config = Config::parse(&text).unwrap();
            let logged = warn_lines(&config);
            let loaded = load(&config);
            assert_eq!(loaded.engine_warning.as_deref(), Some(expected), "{value}");
            assert_eq!(logged, vec![expected.to_string()], "{value}");
            assert_eq!(loaded.config.gravity.to_bits(), bits("C11CE80A").to_bits());
            assert_eq!(loaded.effective_step_rate.to_bits(), 45.0f32.to_bits());
        }
        let jolt = Config::parse("[Startup]\nphysics = Jolt\nFrameTime = 0.0909\n").unwrap();
        assert!(load(&jolt).engine_warning.is_none());
        assert!(warn_lines(&jolt).is_empty());
        let lower = Config::parse("[Startup]\nphysics = jolt\nFrameTime = 0.0909\n").unwrap();
        assert_eq!(
            load(&lower).engine_warning.as_deref(),
            Some("[Startup] physics = \"jolt\" is not Jolt; running Jolt.")
        );
        let empty = Config::parse("").unwrap();
        assert_eq!(
            load(&empty).engine_warning.as_deref(),
            Some("[Startup] physics = \"\" is not Jolt; running Jolt.")
        );
        assert_eq!(warn_lines(&empty).len(), 1);
        let both = Config::parse("[Startup]\nphysics = ubODE\n[Jolt]\nGravity = nope\n").unwrap();
        assert_eq!(
            warn_lines(&both),
            vec![
                "[Startup] physics = \"ubODE\" is not Jolt; running Jolt.".to_string(),
                "[Jolt] Gravity = \"nope\" is invalid (expected a finite number in [-1000, 1000]); using the default -9.80665.".to_string(),
            ]
        );
        assert_eq!(
            load(&both).config.gravity.to_bits(),
            bits("C11CE80A").to_bits()
        );
    }

    fn warn_lines(config: &Config) -> Vec<String> {
        let store = WarnStore(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));
        tracing::subscriber::with_default(store.clone(), || {
            let _ = load(config);
        });
        store.0.lock().unwrap().clone()
    }

    #[derive(Clone)]
    struct WarnStore(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl tracing::Subscriber for WarnStore {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() == tracing::Level::WARN
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut message = String::new();
            event.record(&mut Message(&mut message));
            if let Ok(mut guard) = self.0.lock() {
                guard.push(message);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    struct Message<'a>(&'a mut String);

    impl tracing::field::Visit for Message<'_> {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "message" {
                self.0.push_str(value);
            }
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0.push_str(&format!("{value:?}"));
            }
        }
    }
}
