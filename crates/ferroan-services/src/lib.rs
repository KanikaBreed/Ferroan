// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grid services. This crate holds the asset service, the user-account
//! service (including user aliases), authentication, the grid-user service,
//! the in-memory Presence store, and the raw avatar service.
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
pub mod bakes;
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
