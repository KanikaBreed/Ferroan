// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounds that do not come from the database or the connection string.

use std::time::Duration;

/// Maximum connections in one pool. Tranquillity's `Maximum Pool Size` is not honoured.
pub const MAX_POOL_CONNECTIONS: u32 = 8;

/// How long [`crate::Pool::open`] waits for a connection from the pool.
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

/// Row Tranquillity inserts when it creates the `migrations` table.
pub const MIGRATIONS_STORE: &str = "migrations";

/// Version of that meta row. Tranquillity does not migrate it past 1.
pub const MIGRATIONS_VERSION: i32 = 1;

/// A service opens at most this many stores on one connection.
pub const MAX_STORES_PER_OPEN: usize = 32;
