// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The region side of one LLUDP circuit, the seed capability and an event queue.
//!
//! Throttles are still planned. Empty event polls wait for the C# hold deadline.

mod arrival;
mod caps;
mod circuit;
mod event_queue;
mod handshake;

pub use arrival::ArrivalConfig;
pub use caps::{CapsError, CapsRegistry, CapsServer};
pub use circuit::{CircuitError, CircuitExpect, CircuitServer, CircuitTiming, RootArrival};
pub use event_queue::EventQueueError;
pub use handshake::HandshakeConfig;
