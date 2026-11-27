// Copyright (c) 2026 Legion Builds
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported to Rust for Ferroan from
//   Source/OpenSim.Server.Handlers/Appearance/AppearanceServerConnector.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 41f18f983bb2bfc82e033fed410d361ba57d5aa4.
// This port remains available under MPL-2.0 and forms part of Ferroan's
// GPL-3.0-or-later work.
// Ferroan changes: Copyright (c) 2026 Ferroan contributors.
// SPDX-License-Identifier: MPL-2.0

//! `GET /texture/<agent>/<channel>/<uuid>`.
//!
//! A hit is `image/x-j2c` and the asset bytes. The handler's own misses, a
//! method other than GET, and a shard refusal leave the body empty and use
//! the simple-stream default `text/html;charset=UTF-8`. `/texture` with no
//! second segment never enters the handler: that is the server's HTML 404.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;

use crate::asset::AssetHttp;
use crate::avatar::AvatarService;

use super::channels::parse_wire_uuid;
use super::service::get_bake;

const HTML: &str = "text/html;charset=UTF-8";

#[derive(Clone)]
pub struct AppearanceHttp {
    avatars: Arc<AvatarService>,
    assets: AssetHttp,
    reject_shard: bool,
}

impl AppearanceHttp {
    pub(crate) fn from_services(
        avatars: Arc<AvatarService>,
        assets: AssetHttp,
        reject_shard: bool,
    ) -> Self {
        Self {
            avatars,
            assets,
            reject_shard,
        }
    }

    pub fn into_router(self) -> Router {
        Router::new()
            .route("/texture", any(bare))
            .route("/texture/", any(bare))
            .route("/texture/{*rest}", any(handle))
            .with_state(self)
    }
}

async fn bare() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/html")],
        ferroan_http::HTTP_404,
    )
        .into_response()
}

fn empty(status: StatusCode) -> Response {
    (status, [(header::CONTENT_TYPE, HTML)]).into_response()
}

async fn handle(State(http): State<AppearanceHttp>, request: Request) -> Response {
    if http.reject_shard && request.headers().contains_key("X-SecondLife-Shard") {
        return empty(StatusCode::FORBIDDEN);
    }
    if request.method() != Method::GET {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let Some(rest) = request.uri().path().strip_prefix("/texture") else {
        return empty(StatusCode::NOT_FOUND);
    };
    let mut parts = rest.split(['/', '?', '&']).filter(|part| !part.is_empty());
    let (Some(agent), Some(channel), Some(asset)) = (parts.next(), parts.next(), parts.next())
    else {
        return empty(StatusCode::NOT_FOUND);
    };
    let (Some(agent_id), Some(expected)) = (parse_wire_uuid(agent), parse_wire_uuid(asset)) else {
        return empty(StatusCode::NOT_FOUND);
    };
    match get_bake(&http.avatars, &http.assets, agent_id, channel, expected).await {
        Some(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "image/x-j2c")],
            bytes,
        )
            .into_response(),
        None => empty(StatusCode::NOT_FOUND),
    }
}
