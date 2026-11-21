// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The asset service the Robust connectors call.
//!
//! Routes follow `Source/OpenSim.Server.Handlers/Asset/` at Tranquillity
//! `c70fee205f64ee8672a8d607cef6a733931da951`. The store follows
//! `AssetService` plus `SQLiteAssetData`, `MySQLAssetData` and
//! `PGSQLAssetData`. `XAssetStore` is the MySQL and PostgreSQL XAsset plugins.
//! `XAssetService`, its chained fallback and these HTTP routes are not that
//! store. The filesystem loader runs when `DefaultAssetLoader` names it.
//! `show asset`, `dump asset` and `delete asset` are functions here; the
//! remote console does not call them yet.

mod commands;
mod content_type;
mod decode;
mod http;
mod loader;
mod model;
mod mysql;
mod open;
mod postgres;
mod sqlite;
mod store;
mod time;
mod xasset;
mod xml;
mod xmysql;
mod xpostgres;

pub use commands::{delete_asset, dump_asset, show_asset};
pub use content_type::content_type;
pub use http::{AssetHttp, AssetOptions, DeleteMode};
pub use model::{Asset, AssetMetadata};
pub use open::{AssetError, listen_address, network_port};
pub use store::{AssetClock, AssetStore, StoreError};
pub use xasset::XAssetStore;
pub use xml::{
    decode_asset, decode_metadata_type, decode_strings, encode_asset, encode_bool, encode_bools,
    encode_metadata, encode_string, encode_strings,
};
