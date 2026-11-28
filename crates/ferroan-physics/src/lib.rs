// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Physics for Ferroan.
//!
//! P3.0b ports the Jolt module's substep accumulator. A heartbeat past 16
//! steps drops the extra time and reports `physics.steps-per-frame`. P3.2c
//! reads `[Jolt]` and warns once when `[Startup] physics` is not `Jolt`.
//! P3.1 binds Jolt through `oxijolt-sys` from this crate only. P3.3 runs the
//! heartbeat on a physics thread. The frame posts one heartbeat and reads the
//! pose after the reply. P3.3b replays the dilation window and the stats
//! transform. A successful simulate contributes 1, and `SimPhysicsStepMs`
//! stays 20. P3.4 chooses an analytic box, sphere or cylinder. The dimensions
//! are the ones `CookPrimShape` passes. A non-uniform sphere is not analytic.
//! Mass, a ray and free flight before contact use that shape.
//! Unsafe is allowed in `native`, and in `ferroan-scripting` once that crate
//! exists. The workspace forbids it everywhere else.

#![deny(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod analytic;
mod character;
mod dilation;
mod jolt_config;
#[allow(unsafe_code)]
mod native;
mod substep;
mod thread;

pub use analytic::{
    AnalyticShape, Flight, PrimCuts, RayHit, analytic_mass, analytic_ray, analytic_shape,
    free_flight,
};
pub use character::{
    Capsule, DEFAULT_AVATAR_MASS, DEFAULT_AVATAR_SIZE, EDGE_MARGIN, GroundFlags, Intent,
    MarkerDesc, MarkerRequest, RegionClamp, ShapeState, SpeedFactors, capsule_from_size,
    contact_wanted_after_subscribe, contact_wanted_after_unsubscribe, desired_velocity,
    ground_flags, jump_latch_after_drain, jump_latch_on_press, keep_in_region,
    marker_body_user_data, marker_descriptor, mass_for, movement_sends_latch, stand_half_for,
    store_buoyancy, store_target,
};
pub use dilation::{
    DILATION_WINDOW, DilationWindow, PHYSICS_STEP_MS, StatsInputs, StatsReport, accumulate_frame,
    report_stats, statistics_fps_factor,
};
pub use jolt_config::{
    BasicForces, JoltConfig, JoltLoad, MaterialCombine, VehicleBehaviour, VehiclePresetSet, load,
    startup_physics_warning,
};
pub use native::{
    JOLT_COMMIT, JOLT_VERSION, JOLTC_COMMIT, JOLTC_EXT_REVISION, NativeError, PhysicsSystem,
};
pub use substep::{MAX_STEPS_PER_FRAME, RateError, SubstepAccumulator};
pub use thread::{BodyReply, Physics, Pose, REQUEST_QUEUE, RayReply, Refusal, SpawnError, Step};

#[cfg(test)]
mod character_oracle;
#[cfg(test)]
mod jolt_config_oracle;
#[cfg(test)]
mod jolt_file_oracle;
#[cfg(test)]
mod jolt_matched_oracle;
#[cfg(test)]
mod jolt_record_oracle;
