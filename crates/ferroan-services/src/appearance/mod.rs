// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Viewer bake read. `GET /texture/<agent>/<channel>/<uuid>` returns the asset
//! bytes only when the avatar index still names that uuid for the channel.

mod channels;
mod http;
mod open;
mod service;

pub use http::AppearanceHttp;
pub use open::AppearanceRoute;

#[cfg(test)]
mod tests;
