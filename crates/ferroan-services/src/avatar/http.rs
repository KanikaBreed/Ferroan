// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Ported from OpenSim.Server.Handlers/Avatar/AvatarServerPostHandler.cs and OpenSim.Services.Connectors/Avatar/AvatarServicesConnector.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use ferroan_http::{Field, FieldMap, MAX_QUERY_BYTES, build_xml_response, parse_query_string};
use std::sync::Arc;
use uuid::Uuid;

use super::{AvatarData, AvatarService};
use crate::griduser::{FAILURE_XML, SUCCESS_XML};

#[derive(Clone)]
pub struct AvatarHttp {
    service: Arc<AvatarService>,
    reject_shard_header: bool,
}
impl AvatarHttp {
    pub fn new(service: AvatarService, reject_shard_header: bool) -> Self {
        Self {
            service: Arc::new(service),
            reject_shard_header,
        }
    }
    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.service.pool_occupancy()
    }
    pub fn into_router(self) -> Router {
        Router::new()
            .route("/avatar", post(post_avatar))
            .with_state(self)
    }
}

async fn post_avatar(State(http): State<AvatarHttp>, headers: HeaderMap, body: Body) -> Response {
    if http.reject_shard_header && headers.contains_key("X-SecondLife-Shard") {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "text/plain")],
            Vec::<u8>::new(),
        )
            .into_response();
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_QUERY_BYTES).await else {
        ferroan_diag::refuse(
            ferroan_diag::HTTP_QUERY_BYTES,
            ferroan_diag::Kind::Hard,
            MAX_QUERY_BYTES as u64,
            MAX_QUERY_BYTES as u64 + 1,
            "avatar",
        );
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes));
    let reply = match parse_query_string(text.trim()) {
        Ok(fields) => dispatch(&http.service, fields).await,
        Err(_) => None,
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        reply.unwrap_or_else(|| FAILURE_XML.to_vec()),
    )
        .into_response()
}

async fn dispatch(service: &AvatarService, mut fields: FieldMap) -> Option<Vec<u8>> {
    let Field::Str(user) = fields.shift_remove("UserID")? else {
        return None;
    };
    let user = user.trim_matches(' ');
    if !matches!(user.len(), 32 | 36) {
        return None;
    }
    let id = Uuid::parse_str(user).ok()?;
    let Field::Str(method) = fields.shift_remove("METHOD")? else {
        return None;
    };
    let success = match method.as_str() {
        "getavatar" => {
            let data = service.get_avatar(id).await.ok()?;
            let mut row = FieldMap::from([(
                "AvatarType".into(),
                Field::Str(data.avatar_type.to_string()),
            )]);
            row.extend(
                data.fields
                    .into_iter()
                    .map(|(name, value)| (name, Field::Str(value))),
            );
            return Some(
                build_xml_response(&FieldMap::from([("result".into(), Field::Map(row))]))
                    .ok()?
                    .into_bytes(),
            );
        }
        "setavatar" => {
            fields.shift_remove("VERSIONMIN");
            fields.shift_remove("VERSIONMAX");
            let mut data = AvatarData {
                avatar_type: 0,
                fields: Vec::with_capacity(fields.len()),
            };
            for (name, field) in fields {
                let value = match field {
                    Field::Str(text) => text,
                    Field::List(_) => "System.Collections.Generic.List`1[System.String]".into(),
                    _ => return None,
                };
                if name == "AvatarType" {
                    data.avatar_type = super::service::numeric_text(&value)
                        .parse()
                        .unwrap_or_default();
                }
                data.fields.push((name, value));
            }
            service.set_avatar(id, &data).await
        }
        "resetavatar" => service.reset_avatar(id).await,
        "setitems" => {
            let (Field::List(names), Field::List(values)) =
                (fields.get("Names")?, fields.get("Values")?)
            else {
                return None;
            };
            service.set_items(id, names, values).await
        }
        "removeitems" => {
            let Field::List(names) = fields.get("Names")? else {
                return None;
            };
            service.remove_items(id, names).await
        }
        _ => return None,
    };
    success.ok()?.then(|| SUCCESS_XML.to_vec())
}
