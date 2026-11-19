// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! User aliases.
//!
//! Routes follow `Source/OpenSim.Server.Handlers/UserAlias/` at Tranquillity
//! `40432e0f83d3802434f898a4e28687bd977d5cf5`. The service follows
//! `UserAliasService`. The pin ships a MySQL `UserAlias` migration and no
//! SQLite plugin, so this store requires a table the caller has already
//! created from that column list. It does not invent a migrations version.

mod commands;
mod http;
mod model;
mod open;
mod service;
mod store;

pub use http::AliasHttp;
pub use model::Alias;
pub use open::AliasError;
pub use service::AliasService;
pub use store::{AliasStore, SQLITE_ALIAS_DDL};
