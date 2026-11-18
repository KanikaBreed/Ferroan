// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Database access for Tranquillity's schemas.
//!
//! Pools are sqlx on tokio. Opening a service checks the `migrations` table
//! and refuses to start when a store's version is not the one Tranquillity
//! at [`TRANQUILLITY_REVISION`] would have migrated to. This crate does not
//! run migrations.
//!
//! The expected versions are read from Tranquillity's `*.migrations` resources
//! ([`store_versions_in_dir`]). SQLite files created by that C# migration code
//! live in `fixtures/db/sqlite/`.

mod backend;
mod connect;
mod error;
mod expected;
mod limits;
mod migrations;
mod pool;

pub use backend::Backend;
pub use error::DbError;
pub use expected::{StoreVersion, expected_stores, expected_version};
pub use limits::{MAX_POOL_CONNECTIONS, MIGRATIONS_STORE, MIGRATIONS_VERSION};
pub use migrations::store_versions_in_dir;
pub use pool::{Pool, open_sqlite_file};

/// Tranquillity `develop` revision whose migration resources this crate expects.
/// The pin is the `OpenSim-Tranquillity` row in `docs/REFERENCES.md`.
pub const TRANQUILLITY_REVISION: &str = "858321883f5487a5d192d36ea9d7b90fa468310c";
