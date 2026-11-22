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
// Ported to Rust for Ferroan from
//   Source/OpenSim.Server.Handlers/GridUser/GridUserServerPostHandler.cs
//   OpenSim-NGC/OpenSim-Tranquillity @ 1d7abb8337a477842fe1d7dd84beffe389ee52ac
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! `POST /griduser`.
//!
//! The handler answers 200 `application/xml`. Success and failure are the
//! indented `result` document. A grid-user document is `BuildXmlResponse`.
//! `loggedoutregion` is failure when `RegionID` is missing, not a uuid, or
//! the zero uuid. A body over [`ferroan_http::MAX_QUERY_BYTES`] is 413.
//! `X-SecondLife-Shard` is not checked.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ferroan_http::{Field, FieldMap, MAX_QUERY_BYTES, parse_query_string};

use super::service::{GridClock, GridData, GridUserService, SystemClock};
use super::store::GridUserStore;
use super::wire::{self, FAILURE_XML, SUCCESS_XML, Vec3};

#[derive(Clone)]
pub struct GridUserHttp {
    inner: Arc<GridUserService<GridUserStore, SystemClock>>,
}

impl GridUserHttp {
    pub fn new(service: GridUserService<GridUserStore, SystemClock>) -> Self {
        Self {
            inner: Arc::new(service),
        }
    }

    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.inner.pool_occupancy()
    }

    pub fn into_router(self) -> Router {
        Router::new()
            .route("/griduser", post(post_griduser))
            .with_state(self.inner)
    }
}

async fn post_griduser(
    State(service): State<Arc<GridUserService<GridUserStore, SystemClock>>>,
    body: Body,
) -> Response {
    let bytes = match axum::body::to_bytes(body, MAX_QUERY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return plain(StatusCode::PAYLOAD_TOO_LARGE, Vec::new()),
    };
    let text = String::from_utf8_lossy(&bytes);
    xml(dispatch(service.as_ref(), text.trim()).await)
}

pub async fn dispatch<D, C>(service: &GridUserService<D, C>, body: &str) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    let Ok(fields) = parse_query_string(body) else {
        return FAILURE_XML.to_vec();
    };
    response_for(service, &fields).await
}

pub async fn response_for<D, C>(service: &GridUserService<D, C>, fields: &FieldMap) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    let Some(method) = field_text(fields, "METHOD") else {
        return FAILURE_XML.to_vec();
    };
    match method.as_str() {
        "loggedin" => logged_in(service, fields).await,
        "loggedout" => logged_out(service, fields).await,
        "loggedoutregion" => logged_out_region(service, fields).await,
        "sethome" => set_home(service, fields).await,
        "setposition" => set_position(service, fields).await,
        "getgriduserinfo" => get_one(service, fields).await,
        "getgriduserinfos" => get_many(service, fields).await,
        _ => FAILURE_XML.to_vec(),
    }
}

async fn logged_in<D, C>(service: &GridUserService<D, C>, fields: &FieldMap) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    let Some(user) = field_text(fields, "UserID") else {
        return FAILURE_XML.to_vec();
    };
    match service.logged_in(&user).await {
        Ok(info) => wire::info_document(&info),
        Err(_) => FAILURE_XML.to_vec(),
    }
}

async fn logged_out<D, C>(service: &GridUserService<D, C>, fields: &FieldMap) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    let Some((user, region, position, look)) = unpack(fields) else {
        return FAILURE_XML.to_vec();
    };
    match service.logged_out(&user, &region, position, look).await {
        Ok(true) => SUCCESS_XML.to_vec(),
        _ => FAILURE_XML.to_vec(),
    }
}

async fn logged_out_region<D, C>(service: &GridUserService<D, C>, fields: &FieldMap) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    let Some(text) = field_text(fields, "RegionID") else {
        return FAILURE_XML.to_vec();
    };
    let Some(region) = wire::parse_uuid(&text) else {
        return FAILURE_XML.to_vec();
    };
    if region.is_nil() {
        return FAILURE_XML.to_vec();
    }
    match service.logged_out_region(&region.to_string()).await {
        Ok(true) => SUCCESS_XML.to_vec(),
        _ => FAILURE_XML.to_vec(),
    }
}

async fn set_home<D, C>(service: &GridUserService<D, C>, fields: &FieldMap) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    let Some((user, region, position, look)) = unpack(fields) else {
        return FAILURE_XML.to_vec();
    };
    match service.set_home(&user, &region, position, look).await {
        Ok(true) => SUCCESS_XML.to_vec(),
        _ => FAILURE_XML.to_vec(),
    }
}

async fn set_position<D, C>(service: &GridUserService<D, C>, fields: &FieldMap) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    if field_text(fields, "UserID").is_none() || field_text(fields, "RegionID").is_none() {
        return FAILURE_XML.to_vec();
    }
    let Some((user, region, position, look)) = unpack(fields) else {
        return FAILURE_XML.to_vec();
    };
    match service
        .set_last_position(&user, &region, position, look)
        .await
    {
        Ok(true) => SUCCESS_XML.to_vec(),
        _ => FAILURE_XML.to_vec(),
    }
}

async fn get_one<D, C>(service: &GridUserService<D, C>, fields: &FieldMap) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    let Some(user) = field_text(fields, "UserID") else {
        return FAILURE_XML.to_vec();
    };
    match service.get_info(&user).await {
        Ok(Some(info)) => wire::info_document(&info),
        _ => FAILURE_XML.to_vec(),
    }
}

async fn get_many<D, C>(service: &GridUserService<D, C>, fields: &FieldMap) -> Vec<u8>
where
    D: GridData,
    C: GridClock,
{
    let Some(ids) = field_list(fields, "AgentIDs") else {
        return FAILURE_XML.to_vec();
    };
    match service.get_infos(&ids).await {
        Ok(infos) => wire::infos_document(&infos),
        Err(_) => FAILURE_XML.to_vec(),
    }
}

fn unpack(fields: &FieldMap) -> Option<(String, String, Vec3, Vec3)> {
    let user = field_text(fields, "UserID")?;
    let region = wire::parse_uuid(&field_text(fields, "RegionID")?)?;
    // A missing position is the region default. A value that does not parse
    // is the zero vector.
    let position = match field_text(fields, "Position") {
        None => Vec3::DEFAULT_POSITION,
        Some(text) => wire::parse_vector(&text).unwrap_or(Vec3::ZERO),
    };
    let look = match field_text(fields, "LookAt") {
        None => Vec3::ZERO,
        Some(text) => wire::parse_vector(&text).unwrap_or(Vec3::ZERO),
    };
    Some((user, region.to_string(), position, look))
}

fn field_text(fields: &FieldMap, key: &str) -> Option<String> {
    match fields.get(key)? {
        Field::Str(text) | Field::Uuid(text) => Some(text.clone()),
        Field::Bool(true) => Some("True".to_string()),
        Field::Bool(false) => Some("False".to_string()),
        Field::Int(number) => Some(number.to_string()),
        Field::List(_) | Field::Map(_) | Field::Null => None,
    }
}

fn field_list(fields: &FieldMap, key: &str) -> Option<Vec<String>> {
    match fields.get(key)? {
        Field::List(items) => Some(items.clone()),
        _ => None,
    }
}

fn xml(body: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml")],
        body,
    )
        .into_response()
}

fn plain(status: StatusCode, body: Vec<u8>) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain")], body).into_response()
}
