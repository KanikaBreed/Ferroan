// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Physics for Ferroan.
//!
//! P3.0b ports the Jolt module's substep accumulator. P3.1 binds Jolt through
//! `oxijolt-sys` from this crate only. Unsafe is allowed here and in
//! `ferroan-scripting` once that crate exists. The workspace forbids it
//! everywhere else.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod native;
mod substep;

pub use native::{
    JOLT_COMMIT, JOLT_VERSION, JOLTC_COMMIT, JOLTC_EXT_REVISION, NativeError, PhysicsSystem,
};
pub use substep::{MAX_STEPS_PER_FRAME, RateError, SubstepAccumulator};
