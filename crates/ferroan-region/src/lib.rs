// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A region registered with C# Robust; login remains a grid service.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod avatar;
pub mod caps_policy;
pub mod child_drive;
pub mod children;
pub mod config;
pub mod control_plane;
pub mod fetch_caps;
pub mod grid;
pub mod groups;
pub mod inventory;
pub mod land_properties;
pub mod library;
pub mod names;
pub mod objects;
pub mod remote_asset;
pub mod settings;
pub mod simulation;
pub mod simulation_client;
pub mod source_hop;
pub mod staff;
pub mod status;
