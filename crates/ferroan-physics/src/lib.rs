// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Physics for Ferroan.
//!
//! P3.0b ports the Jolt module's substep accumulator. A heartbeat past 16
//! steps drops the extra time and reports `physics.steps-per-frame`. P3.2c
//! reads `[Jolt]` and warns once when `[Startup] physics` is not `Jolt`.
//! P3.1 binds Jolt through `oxijolt-sys` from this crate only. Unsafe is
//! allowed in `native`, and in `ferroan-scripting` once that crate exists.
//! The workspace forbids it everywhere else.

#![deny(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod jolt_config;
#[allow(unsafe_code)]
mod native;
mod substep;

pub use jolt_config::{
    BasicForces, JoltConfig, JoltLoad, MaterialCombine, VehicleBehaviour, VehiclePresetSet, load,
    startup_physics_warning,
};
pub use native::{
    JOLT_COMMIT, JOLT_VERSION, JOLTC_COMMIT, JOLTC_EXT_REVISION, NativeError, PhysicsSystem,
};
pub use substep::{MAX_STEPS_PER_FRAME, RateError, SubstepAccumulator};

#[cfg(test)]
mod jolt_config_oracle;
#[cfg(test)]
mod jolt_file_oracle;
