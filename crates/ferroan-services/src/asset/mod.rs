// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The asset service the Robust connectors call.
//!
//! Routes follow `Source/OpenSim.Server.Handlers/Asset/` at Tranquillity
//! `1d7abb8337a477842fe1d7dd84beffe389ee52ac`. The store follows
//! `AssetService` plus `SQLiteAssetData`, `MySQLAssetData` and
//! `PGSQLAssetData`. `XAssetStore` is the MySQL and PostgreSQL XAsset plugins.
//! `XAssetService` serves these HTTP routes on that store, including a chained
//! `AssetService`. `FsAssetStore` is the MySQL and PostgreSQL FSAssets plugins:
//! table rows only. `FsFiles` is the hash layout and the spool writer.
//! `FsAssetService` serves these HTTP routes on that store. The filesystem
//! loader runs when `DefaultAssetLoader` names it on `AssetService`, and on
//! the main `FSAssetService`. `XAssetService` does not seed the library.
//! `show asset`, `dump asset` and `delete asset` are functions here. The
//! FSAssets commands `show assets`, `show digest`, `delete asset`, `import`
//! and `force import` are too, and only a main `FsAssetService` registers
//! them. The remote console does not call any of them yet.

mod commands;
mod content_type;
mod decode;
mod fmysql;
mod fpostgres;
mod fsasset;
mod fscommands;
mod fservice;
mod fsfiles;
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
mod xservice;

pub use commands::{delete_asset, dump_asset, show_asset};
pub use content_type::content_type;
pub use fsasset::FsAssetStore;
pub use fsfiles::{FsFiles, WRITER_INTERVAL_MS, WRITER_MAX_RETRY_MS, next_writer_interval};
pub use http::{AssetHttp, AssetOptions, DeleteMode};
pub use model::{Asset, AssetMetadata};
pub use open::{AssetError, listen_address, network_port};
pub use store::{AssetClock, AssetStore, StoreError};
pub use xasset::XAssetStore;
pub use xml::{
    decode_asset, decode_metadata_type, decode_strings, encode_asset, encode_bool, encode_bools,
    encode_metadata, encode_string, encode_strings,
};
