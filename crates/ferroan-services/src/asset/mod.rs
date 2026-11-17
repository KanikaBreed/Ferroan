// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The asset service the Robust connectors call.
//!
//! Routes follow `Source/OpenSim.Server.Handlers/Asset/` at Tranquillity
//! `713857f10b8e91a639a967d4354a4682b080793c`. The store follows
//! `AssetService` plus the SQLite `assets` table. FSAssets (`XAssetService`),
//! the filesystem asset loader and MySQL are not in this pilot.

mod content_type;
mod http;
mod model;
mod open;
mod store;
mod xml;

pub use content_type::content_type;
pub use http::{AssetHttp, AssetOptions, DeleteMode};
pub use model::Asset;
pub use open::{AssetError, network_port};
pub use store::AssetStore;
pub use xml::{
    decode_asset, decode_metadata_type, decode_strings, encode_asset, encode_bool, encode_bools,
    encode_metadata, encode_string, encode_strings,
};
