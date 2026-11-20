// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading Tranquillity's INI files the way Tranquillity reads them.
//!
//! Parsing and `${Section|key}` substitution follow Nini 0.9.2.16, which is
//! the package Tranquillity references. The region and grid loaders follow
//! `ConfigurationLoader` and `GridServerConfigSource` at the pinned
//! Tranquillity revision. Fixtures from `tools/config-oracle` are the check:
//! that program calls Nini and writes the resolved key/value set.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod error;
mod expand;
mod glob;
mod load;
mod parse;

pub use error::ConfigError;
pub use load::{GridLoad, LoadWarning, RegionLoad, Resolved};
pub use parse::Config;
