// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Physics for Ferroan.
//!
//! P3.0b ports the Jolt module's substep accumulator. This crate forbids
//! unsafe code until P3.1 binds Jolt.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod substep;

pub use substep::{MAX_STEPS_PER_FRAME, RateError, SubstepAccumulator};
