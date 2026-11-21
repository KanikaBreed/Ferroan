// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pools and the migration-version gate.
//!
//! `open` connects and then reads `migrations`. A mismatch drops the pool and
//! returns an error, which is the refusal to start. Nothing in this module
//! executes a migration script.

use std::path::Path;

use sqlx::mysql::{MySqlPool, MySqlPoolOptions};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

use crate::backend::Backend;
use crate::connect::{
    mysql_options, mysql_target, postgres_options, postgres_target, scrub_driver_message,
    sqlite_options, sqlite_target,
};
use crate::error::DbError;
use crate::expected::expected_version;
use ferroan_diag::{self, DB_POOL};

use crate::limits::{
    ACQUIRE_TIMEOUT, MAX_POOL_CONNECTIONS, MAX_STORES_PER_OPEN, MIGRATIONS_STORE,
    MIGRATIONS_VERSION,
};

enum Inner {
    Sqlite(SqlitePool),
    Mysql(MySqlPool),
    Postgres(PgPool),
}

/// A bounded pool for one Tranquillity connection string.
pub struct Pool {
    backend: Backend,
    inner: Inner,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pool")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

impl Pool {
    /// Connections in the pool, then how many of those are idle.
    pub fn occupancy(&self) -> (u32, u32) {
        let (open, idle) = match &self.inner {
            Inner::Sqlite(pool) => (pool.size(), pool.num_idle()),
            Inner::Mysql(pool) => (pool.size(), pool.num_idle()),
            Inner::Postgres(pool) => (pool.size(), pool.num_idle()),
        };
        ferroan_diag::observe(
            DB_POOL,
            u64::from(MAX_POOL_CONNECTIONS),
            u64::from(open),
            "db",
        );
        (open, u32::try_from(idle).unwrap_or(u32::MAX))
    }

    /// The SQLite pool, when this connection is SQLite.
    pub fn sqlite(&self) -> Option<&sqlx::sqlite::SqlitePool> {
        match &self.inner {
            Inner::Sqlite(pool) => Some(pool),
            _ => None,
        }
    }

    /// The MySQL pool, when this connection is MySQL.
    pub fn mysql(&self) -> Option<&sqlx::mysql::MySqlPool> {
        match &self.inner {
            Inner::Mysql(pool) => Some(pool),
            _ => None,
        }
    }

    /// The PostgreSQL pool, when this connection is PostgreSQL.
    pub fn postgres(&self) -> Option<&sqlx::postgres::PgPool> {
        match &self.inner {
            Inner::Postgres(pool) => Some(pool),
            _ => None,
        }
    }

    /// Open `connection_string` and require every named store, plus the
    /// `migrations` meta row, to be at the version this build expects.
    ///
    /// `stores` is the set that service uses. A region asset file is checked
    /// for `AssetStore` only; it is not required to contain every other store.
    pub async fn open(
        backend: Backend,
        connection_string: &str,
        stores: &[&str],
    ) -> Result<Self, DbError> {
        let pool = Self::connect(backend, connection_string).await?;
        pool.require_stores(stores).await?;
        Ok(pool)
    }

    /// Open a pool without checking schema versions.
    ///
    /// Services should use [`Self::open`]. This exists for tools that inspect
    /// a database before deciding which stores to require.
    pub async fn connect(backend: Backend, connection_string: &str) -> Result<Self, DbError> {
        let inner = match backend {
            Backend::Sqlite => {
                let target = sqlite_target(connection_string)?;
                let options = sqlite_options(&target)?;
                let pool = SqlitePoolOptions::new()
                    .max_connections(MAX_POOL_CONNECTIONS)
                    .acquire_timeout(ACQUIRE_TIMEOUT)
                    .connect_with(options)
                    .await
                    .map_err(|error| connect_error(backend, &error))?;
                Inner::Sqlite(pool)
            }
            Backend::Mysql => {
                let (target, password) = mysql_target(connection_string)?;
                let options = mysql_options(&target, &password);
                let pool = MySqlPoolOptions::new()
                    .max_connections(MAX_POOL_CONNECTIONS)
                    .min_connections(0)
                    .acquire_timeout(ACQUIRE_TIMEOUT)
                    .connect_with(options)
                    .await
                    .map_err(|error| connect_error(backend, &error))?;
                Inner::Mysql(pool)
            }
            Backend::Postgres => {
                let (target, password) = postgres_target(connection_string)?;
                let options = postgres_options(&target, &password);
                let pool = PgPoolOptions::new()
                    .max_connections(MAX_POOL_CONNECTIONS)
                    .min_connections(0)
                    .acquire_timeout(ACQUIRE_TIMEOUT)
                    .connect_with(options)
                    .await
                    .map_err(|error| connect_error(backend, &error))?;
                Inner::Postgres(pool)
            }
        };
        Ok(Self { backend, inner })
    }

    /// Open a sqlx URL (`sqlite:`, `mysql:` or `postgres:`).
    ///
    /// SQLite files are not created. Prefer [`Self::connect`] for a Tranquillity
    /// connection string. Tests use this for `FERROAN_MYSQL_URL` and
    /// `FERROAN_POSTGRES_URL`.
    pub async fn connect_sqlx_url(url: &str) -> Result<Self, DbError> {
        if url.len() > 4096 || url.as_bytes().contains(&0) {
            return Err(DbError::Input(
                "database URL is empty or too long".to_string(),
            ));
        }
        let backend = crate::connect::backend_from_sqlx_url(url)?;
        let inner = match backend {
            Backend::Sqlite => {
                if url.contains(":memory:") {
                    return Err(DbError::Input(
                        "SQLite memory databases are not accepted".to_string(),
                    ));
                }
                let options = url
                    .parse::<sqlx::sqlite::SqliteConnectOptions>()
                    .map_err(|error| DbError::Input(scrub_driver_message(&error.to_string())))?
                    .create_if_missing(false);
                let pool = SqlitePoolOptions::new()
                    .max_connections(MAX_POOL_CONNECTIONS)
                    .acquire_timeout(ACQUIRE_TIMEOUT)
                    .connect_with(options)
                    .await
                    .map_err(|error| connect_error(backend, &error))?;
                Inner::Sqlite(pool)
            }
            Backend::Mysql => {
                let pool = MySqlPoolOptions::new()
                    .max_connections(MAX_POOL_CONNECTIONS)
                    .acquire_timeout(ACQUIRE_TIMEOUT)
                    .connect(url)
                    .await
                    .map_err(|error| connect_error(backend, &error))?;
                Inner::Mysql(pool)
            }
            Backend::Postgres => {
                let pool = PgPoolOptions::new()
                    .max_connections(MAX_POOL_CONNECTIONS)
                    .acquire_timeout(ACQUIRE_TIMEOUT)
                    .connect(url)
                    .await
                    .map_err(|error| connect_error(backend, &error))?;
                Inner::Postgres(pool)
            }
        };
        Ok(Self { backend, inner })
    }

    /// Backend this pool was opened with.
    pub const fn backend(&self) -> Backend {
        self.backend
    }

    /// Close every connection. SQLite tests do this before another process opens the file.
    pub async fn close(self) {
        match self.inner {
            Inner::Sqlite(pool) => pool.close().await,
            Inner::Mysql(pool) => pool.close().await,
            Inner::Postgres(pool) => pool.close().await,
        }
    }

    /// Require `stores` and the `migrations` meta row.
    pub async fn require_stores(&self, stores: &[&str]) -> Result<(), DbError> {
        if stores.is_empty() {
            return Err(DbError::Input(
                "open requires at least one migration store".to_string(),
            ));
        }
        if stores.len() > MAX_STORES_PER_OPEN {
            return Err(DbError::Input(format!(
                "open accepts at most {MAX_STORES_PER_OPEN} stores"
            )));
        }
        let mut seen = Vec::with_capacity(stores.len());
        for store in stores {
            if store.is_empty() || store.len() > 100 {
                return Err(DbError::Input(
                    "store name is empty or longer than 100 bytes".to_string(),
                ));
            }
            if seen.contains(store) {
                return Err(DbError::Input(format!("store {store} was requested twice")));
            }
            expected_version(self.backend, store)?;
            seen.push(*store);
        }
        self.require_version(MIGRATIONS_STORE, MIGRATIONS_VERSION)
            .await?;
        for store in stores {
            let expected = expected_version(self.backend, store)?;
            self.require_version(store, expected).await?;
        }
        Ok(())
    }

    async fn require_version(&self, store: &str, expected: i32) -> Result<(), DbError> {
        match self.read_version(store).await? {
            Some(found) if found == expected => Ok(()),
            found => Err(DbError::Version {
                store: store.to_string(),
                found,
                expected,
            }),
        }
    }

    async fn read_version(&self, store: &str) -> Result<Option<i32>, DbError> {
        let sql_question =
            "SELECT version FROM migrations WHERE name = ? ORDER BY version DESC LIMIT 1";
        let sql_dollar =
            "SELECT version FROM migrations WHERE name = $1 ORDER BY version DESC LIMIT 1";
        let value = match &self.inner {
            Inner::Sqlite(pool) => {
                let row: Option<(i64,)> = sqlx::query_as(sql_question)
                    .bind(store)
                    .fetch_optional(pool)
                    .await
                    .map_err(|error| unreadable(self.backend, store, &error))?;
                row.map(|(version,)| version)
            }
            Inner::Mysql(pool) => {
                let row: Option<(i32,)> = sqlx::query_as(sql_question)
                    .bind(store)
                    .fetch_optional(pool)
                    .await
                    .map_err(|error| unreadable(self.backend, store, &error))?;
                row.map(|(version,)| i64::from(version))
            }
            Inner::Postgres(pool) => {
                let row: Option<(i32,)> = sqlx::query_as(sql_dollar)
                    .bind(store)
                    .fetch_optional(pool)
                    .await
                    .map_err(|error| unreadable(self.backend, store, &error))?;
                row.map(|(version,)| i64::from(version))
            }
        };
        match value {
            None => Ok(None),
            Some(version) => match i32::try_from(version) {
                Ok(version) => Ok(Some(version)),
                Err(_) => Err(DbError::Unreadable {
                    store: store.to_string(),
                    detail: "version does not fit in a 32-bit integer".to_string(),
                }),
            },
        }
    }
}

/// Open a SQLite file that a test or the schema tool already created.
pub async fn open_sqlite_file(path: &Path, stores: &[&str]) -> Result<Pool, DbError> {
    let connection_string = sqlite_data_source(path)?;
    Pool::open(Backend::Sqlite, &connection_string, stores).await
}

fn sqlite_data_source(path: &Path) -> Result<String, DbError> {
    let text = path
        .to_str()
        .ok_or_else(|| DbError::Input("SQLite path is not valid Unicode".to_string()))?;
    if text.is_empty()
        || text.len() > 4096
        || text.as_bytes().contains(&0)
        || text.contains(';')
        || text.contains('=')
    {
        return Err(DbError::Input(
            "SQLite path cannot be placed in a connection string".to_string(),
        ));
    }
    Ok(format!("Data Source={text};Version=3"))
}

fn connect_error(backend: Backend, error: &sqlx::Error) -> DbError {
    DbError::Connect {
        backend,
        message: scrub_driver_message(&error.to_string()),
    }
}

fn unreadable(backend: Backend, store: &str, error: &sqlx::Error) -> DbError {
    match error {
        sqlx::Error::PoolTimedOut => DbError::Connect {
            backend,
            message: "timed out waiting for a connection".to_string(),
        },
        other => DbError::Unreadable {
            store: store.to_string(),
            detail: scrub_driver_message(&other.to_string()),
        },
    }
}
