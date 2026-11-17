// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grid services. This crate currently holds the asset service.
//!
//! `ferroan-robust` serves it. Inventory, accounts and the other Robust
//! services are later work. The asset HTTP bodies are compared with
//! `fixtures/asset`, which `cargo xtask asset` writes from Tranquillity's
//! `XmlSerializer`.

pub mod asset;

pub use asset::{
    Asset, AssetError, AssetHttp, AssetOptions, AssetStore, DeleteMode, content_type, decode_asset,
    decode_metadata_type, decode_strings, encode_asset, encode_bool, encode_bools, encode_metadata,
    encode_string, encode_strings, listen_address, network_port,
};
