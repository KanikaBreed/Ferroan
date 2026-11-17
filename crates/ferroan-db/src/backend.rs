// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The three database backends Tranquillity ships.

use std::fmt;

/// Which Tranquillity data provider a connection uses.
///
/// The migration number for a store is not the same on each backend. Callers
/// name the backend explicitly; a SQLite file and a MySQL database that both
/// hold `AssetStore` are at different versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// `OpenSim.Data.SQLite`.
    Sqlite,
    /// `OpenSim.Data.MySQL` (MySqlConnector).
    Mysql,
    /// `OpenSim.Data.PGSQL` (Npgsql).
    Postgres,
}

impl Backend {
    /// Short name used in errors and the fixture manifest.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
