// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Failures from connection strings and from the migration-version check.
//!
//! Messages name the backend and the store. They do not include the connection
//! string, so a password in that string is not copied into a log.

use std::fmt;

use crate::Backend;

/// Why a database could not be opened or accepted.
#[derive(Debug)]
pub enum DbError {
    /// The connection string or the requested store list is not usable.
    Input(String),
    /// `store` is not a migration Tranquillity ships on `backend`.
    UnknownStore {
        /// Backend the caller asked to open.
        backend: Backend,
        /// Store name that is not in the catalog.
        store: String,
    },
    /// The server or file could not be opened.
    Connect {
        /// Backend that failed.
        backend: Backend,
        /// Driver message, with URL userinfo removed.
        message: String,
    },
    /// The version query failed. Tranquillity's `FindVersion` treats this as version -1.
    Unreadable {
        /// Store whose row was being read, or `migrations` for the table itself.
        store: String,
        /// Driver message.
        detail: String,
    },
    /// The stored version is missing or is not the version this build expects.
    Version {
        /// Store name, including the `migrations` meta row.
        store: String,
        /// `None` when the row is absent (Tranquillity would report 0).
        found: Option<i32>,
        /// Version Tranquillity's migration resources reach from an empty database.
        expected: i32,
    },
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(message) => write!(formatter, "{message}"),
            Self::UnknownStore { backend, store } => {
                write!(formatter, "{backend} has no migration store {store}")
            }
            Self::Connect { backend, message } => {
                write!(formatter, "could not connect to {backend}: {message}")
            }
            Self::Unreadable { store, detail } => {
                write!(
                    formatter,
                    "could not read migration version for {store} (Tranquillity reports -1): {detail}"
                )
            }
            Self::Version {
                store,
                found: None,
                expected,
            } => write!(
                formatter,
                "store {store} has no version row (Tranquillity reports 0); this build expects {expected}"
            ),
            Self::Version {
                store,
                found: Some(found),
                expected,
            } => write!(
                formatter,
                "store {store} is at version {found}; this build expects {expected}"
            ),
        }
    }
}

impl std::error::Error for DbError {}
