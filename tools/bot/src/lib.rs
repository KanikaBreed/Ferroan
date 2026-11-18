// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A headless viewer for the live harness.
//!
//! XML-RPC login, one LLUDP circuit (acks, resends, the region handshake),
//! the seed capability, and the scenarios `cargo xtask live run` drives.
//! Nothing here starts a region.

mod body;
pub mod caps;
pub mod circuit;
pub mod login;
pub mod scenario;
pub mod session;
pub mod throttle;
pub mod trace;
mod world;
mod xmlrpc;

pub use caps::{SEED_CAPS, request_seed};
pub use circuit::{Circuit, Resend};
pub use login::{CHANNEL, LOGIN_OPTIONS, LoginRequest, LoginResponse, hash_password, login};
pub use session::{Agent, Session};
