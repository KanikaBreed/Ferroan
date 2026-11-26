// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Authentication and authorization.
//!
//! Routes follow `Source/OpenSim.Server.Handlers/Authentication/` and
//! `Source/OpenSim.Server.Handlers/Authorization/` at Tranquillity
//! `1d7abb8337a477842fe1d7dd84beffe389ee52ac`. The password is the salted
//! double MD5 `AuthenticationServiceBase` stores. Region-flag authorization
//! and OpenId are not this service.

mod authorize;
mod hash;
mod http;
mod model;
mod open;
mod service;
mod store;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod zone;

pub use authorize::router as authorization_router;
pub use http::AuthHttp;
pub use open::{AuthError, open_authorization};
pub(crate) use open::{config_bool_either, refuse_transport};
