// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grid services. This crate holds the asset service and the user-account
//! service, including user aliases.
//!
//! `ferroan-robust` serves a service when its ini section is present.
//! Inventory and the other Robust services are later work. Asset HTTP bodies
//! are compared with `fixtures/asset`. Account HTTP is compared with
//! `cargo xtask live compare accounts`.

pub mod account;
pub mod alias;
pub mod asset;

pub use account::AccountHttp;
pub use alias::AliasHttp;

pub use asset::{
    Asset, AssetError, AssetHttp, AssetOptions, AssetStore, DeleteMode, content_type, decode_asset,
    decode_metadata_type, decode_strings, encode_asset, encode_bool, encode_bools, encode_metadata,
    encode_string, encode_strings, listen_address, network_port,
};
