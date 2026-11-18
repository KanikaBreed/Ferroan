// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The region side of one LLUDP circuit, the seed capability and an event queue.
//!
//! Throttles are still planned. An empty event poll is answered at once.

mod caps;
mod circuit;
mod handshake;

pub use caps::{CapsError, CapsRegistry, CapsServer};
pub use circuit::{CircuitError, CircuitExpect, CircuitServer};
pub use handshake::HandshakeConfig;
