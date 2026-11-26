// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grid services. This crate holds the asset service, the user-account
//! service (including user aliases), authentication, the grid-user service,
//! the Null, MySQL and PostgreSQL Presence stores, and the raw avatar service.
//!
//! `ferroan-robust` serves a service when its ini section is present.
//! Inventory and the other Robust services are later work. Asset HTTP bodies
//! are compared with `fixtures/asset`. Account HTTP is compared with
//! `cargo xtask live compare accounts`. Authentication HTTP is compared with
//! `cargo xtask live compare auth`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod account;
pub mod alias;
pub mod asset;
pub mod auth;
pub mod avatar;
pub mod griduser;
pub mod presence;

#[cfg(test)]
mod shard_header;

pub use account::AccountHttp;
pub use alias::AliasHttp;
pub use auth::{AuthError, AuthHttp, authorization_router, open_authorization};
pub use avatar::AvatarHttp;
pub use griduser::GridUserHttp;

pub use asset::{
    Asset, AssetClock, AssetError, AssetHttp, AssetMetadata, AssetOptions, AssetStore, DeleteMode,
    StoreError, content_type, decode_asset, decode_metadata_type, decode_strings, encode_asset,
    encode_bool, encode_bools, encode_metadata, encode_string, encode_strings, listen_address,
    network_port,
};

/// A single table identifier accepted by the grid service stores.
pub(crate) fn sql_ident(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 64
        && bytes
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
