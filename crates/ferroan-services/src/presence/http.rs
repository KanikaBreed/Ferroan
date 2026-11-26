/*
 * Copyright (c) Contributors, http://opensimulator.org/
 * See CONTRIBUTORS.TXT for a full list of copyright holders.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *     * Redistributions of source code must retain the above copyright
 *       notice, this list of conditions and the following disclaimer.
 *     * Redistributions in binary form must reproduce the above copyright
 *       notice, this list of conditions and the following disclaimer in the
 *       documentation and/or other materials provided with the distribution.
 *     * Neither the name of the OpenSimulator Project nor the
 *       names of its contributors may be used to endorse or promote products
 *       derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
 * EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
 * WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

// Ported from Tranquillity Source/OpenSim.Server.Handlers/Presence/PresenceServerPostHandler.cs and OpenSim.Services.Interfaces/IPresenceService.cs
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

use super::{PresenceInfo, PresenceService, parse_id};
use crate::griduser::{FAILURE_XML, SUCCESS_XML};

#[derive(Clone)]
pub struct PresenceHttp {
    pub(super) service: Arc<PresenceService>,
    pub(super) reject_shard: bool,
    pub(super) pool: Option<Arc<ferroan_db::Pool>>,
}

impl PresenceHttp {
    /// In-process login and the HTTP handler use the same configured service.
    pub fn service(&self) -> &PresenceService {
        &self.service
    }
    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.pool.as_ref().map_or((0, 0), |pool| pool.occupancy())
    }
    pub fn into_router(self) -> Router {
        Router::new()
            .route("/presence", post(post_presence))
            .with_state(self)
    }
}

async fn post_presence(
    State(http): State<PresenceHttp>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if http.reject_shard && headers.contains_key("X-SecondLife-Shard") {
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
            "presence",
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

async fn dispatch(service: &PresenceService, fields: FieldMap) -> Option<Vec<u8>> {
    let Field::Str(method) = fields.get("METHOD")? else {
        return None;
    };
    let uuid = |key| match fields.get(key)? {
        Field::Str(s) => parse_id(s),
        _ => None,
    };
    let success = match method.as_str() {
        "logout" => service.logout_agent(uuid("SessionID")?).await,
        "logoutregion" => service.logout_region_agents(uuid("RegionID")?).await,
        "report" => {
            service
                .report_agent(uuid("SessionID")?, uuid("RegionID")?)
                .await
        }
        "getagent" => {
            let value = service
                .get_agent(uuid("SessionID")?)
                .await
                .ok()?
                .map(info)
                .unwrap_or_else(|| Field::Str("null".into()));
            return xml(FieldMap::from([("result".into(), value)]));
        }
        "getagents" => {
            let Field::List(ids) = fields.get("uuids")? else {
                return None;
            };
            let mut result = FieldMap::new();
            for (i, row) in service.get_agents(ids).await.ok()?.into_iter().enumerate() {
                result.insert(format!("presence{i}"), info(row));
            }
            if result.is_empty() {
                result.insert("result".into(), Field::Str("null".into()));
            }
            return xml(result);
        }
        // LoginAgent is only callable in process.
        _ => return None,
    };
    success.ok()?.then(|| SUCCESS_XML.to_vec())
}
fn info(row: PresenceInfo) -> Field {
    Field::Map(FieldMap::from([
        ("UserID".into(), Field::Str(row.user_id)),
        ("RegionID".into(), Field::Str(row.region_id.to_string())),
    ]))
}
fn xml(fields: FieldMap) -> Option<Vec<u8>> {
    Some(build_xml_response(&fields).ok()?.into_bytes())
}
