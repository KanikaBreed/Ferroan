// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! User accounts.
//!
//! Routes follow `Source/OpenSim.Server.Handlers/UserAccounts/` at Tranquillity
//! `40432e0f83d3802434f898a4e28687bd977d5cf5`. The service follows
//! `Source/OpenSim.Services.UserAccountService/UserAccountService.cs`. The
//! store follows `SQLiteUserAccountData` and `SQLiteGenericTableHandler`.
//! Inventory, appearance, authentication, grid and grid-user plugins are not
//! loaded. MySQL and PostgreSQL stores wait for P1.A1.

mod commands;
mod http;
mod model;
mod open;
mod search;
mod service;
mod store;
mod urls;

pub use commands::CommandOutcome;
pub use http::AccountHttp;
pub use model::Account;
pub use open::AccountError;
pub use service::{AccountService, Clock, FixedClock, NewAccount, SystemClock};
pub use store::AccountStore;
