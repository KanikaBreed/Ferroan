// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The region side of one LLUDP circuit.
//!
//! Throttles, CAPS and the event queue are later rows.

mod circuit;

pub use circuit::{CircuitError, CircuitExpect, CircuitServer};
