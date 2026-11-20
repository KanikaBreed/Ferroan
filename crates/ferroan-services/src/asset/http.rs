// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Asset HTTP routes.
//!
//! `AssetServerGetHandler`, `AssetServerPostHandler`,
//! `AssetServerDeleteHandler` and `AssetsExistHandler` at Tranquillity
//! `713857f10b8e91a639a967d4354a4682b080793c`.
//!
//! GET of an asset is `text/xml`. GET of metadata uses the asset type's
//! content type. GET of bytes is `application/octet-stream`. POST, DELETE and
//! exists leave the handler default, `application/xml`. A missing asset is
//! 404 `text/plain` with an empty body. When `RedirectURL` is set, that 404
//! becomes 302 and the location is the configured text plus the request path.
//! The C# constructor's trailing-slash trim does not change the text: it only
//! runs when the value does not already end in `/`.
//!
//! `UpdateContent` returns false. Delete runs only for the configured type
//! set, and a parsed id still answers true when the row was already gone,
//! which is what `SQLiteAssetData.Delete` returns.
//!
//! Requests that carry `X-SecondLife-Shard` are 403 on get, post and delete.
//! That is `DisallowLlHttpRequest`, installed unless `AllowllHTTPRequestIn`
//! is true. Exists does not use that check: its handler is constructed
//! without auth.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, OriginalUri, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use ferroan_http::MAX_XML_CHARS;

use super::model::{MAPTILE, parse_id};
use super::store::AssetStore;
use super::xml;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteMode {
    None,
    Maptile,
    All,
}

#[derive(Clone, Debug)]
pub struct AssetOptions {
    pub delete: DeleteMode,
    pub redirect_url: Option<String>,
    pub reject_shard_header: bool,
}

impl Default for AssetOptions {
    fn default() -> Self {
        Self {
            delete: DeleteMode::None,
            redirect_url: None,
            reject_shard_header: true,
        }
    }
}

#[derive(Clone)]
pub struct AssetHttp {
    inner: Arc<Inner>,
}

struct Inner {
    store: AssetStore,
    options: AssetOptions,
    /// Passed to `axum::body::to_bytes`. `DefaultBodyLimit` only applies to
    /// the `Bytes` and `String` extractors, and these handlers take `Body`.
    body_limit: usize,
}

impl AssetHttp {
    pub fn new(store: AssetStore, options: AssetOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                options,
                body_limit: MAX_XML_CHARS,
            }),
        }
    }

    pub fn pool_occupancy(&self) -> (u32, u32) {
        self.inner.store.pool_occupancy()
    }

    pub fn into_router(self) -> Router {
        let limit = self.inner.body_limit;
        self.into_router_limited(limit)
    }

    pub fn into_router_limited(mut self, limit: usize) -> Router {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.body_limit = limit;
        }
        Router::new()
            .route("/assets/{id}/data", get(get_data))
            .route("/assets/{id}/metadata", get(get_metadata))
            .route("/assets/{id}/{cmd}", get(get_unknown))
            .route(
                "/assets/{id}",
                get(get_asset).post(post_update).delete(delete_asset),
            )
            .route(
                "/assets",
                get(get_bare).post(post_store).delete(delete_bare),
            )
            .route("/assets/", post(post_store))
            .route("/get_assets_exist", post(post_exists))
            .layer(DefaultBodyLimit::max(limit))
            .with_state(self)
    }
}

async fn get_bare() -> Response {
    xml_bytes(StatusCode::OK, "application/xml", Vec::new())
}

async fn get_asset(
    State(app): State<AssetHttp>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = app.shard(&headers) {
        return response;
    }
    match app.inner.store.get_parsed(&id).await {
        Some(asset) => xml_bytes(StatusCode::OK, "text/xml", xml::encode_asset(&asset)),
        None => app.missing(uri.path(), true),
    }
}

async fn get_data(
    State(app): State<AssetHttp>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = app.shard(&headers) {
        return response;
    }
    match app.inner.store.get_parsed(&id).await {
        Some(asset) => xml_bytes(StatusCode::OK, "application/octet-stream", asset.data),
        None => app.missing(uri.path(), true),
    }
}

async fn get_metadata(
    State(app): State<AssetHttp>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = app.shard(&headers) {
        return response;
    }
    match app.inner.store.get_parsed(&id).await {
        Some(asset) => {
            let content = super::content_type::content_type(asset.type_code);
            xml_bytes(StatusCode::OK, content, xml::encode_metadata(&asset))
        }
        None => app.missing(uri.path(), true),
    }
}

async fn get_unknown(State(app): State<AssetHttp>, headers: HeaderMap) -> Response {
    if let Some(response) = app.shard(&headers) {
        return response;
    }
    xml_bytes(StatusCode::BAD_REQUEST, "text/plain", Vec::new())
}

async fn post_store(State(app): State<AssetHttp>, headers: HeaderMap, body: Body) -> Response {
    if let Some(response) = app.shard(&headers) {
        return response;
    }
    let Some(bytes) = read_body(app.inner.body_limit, body).await else {
        return too_large();
    };
    let decoded = match xml::decode_asset(&bytes) {
        Ok(decoded) => decoded,
        Err(_) => return bad_xml(),
    };
    if decoded.data_nil {
        return bad_xml();
    }
    let id_text = decoded.id_text.clone();
    if app.inner.store.store_new(&decoded.asset).await.is_err() {
        return xml_bytes(
            StatusCode::INTERNAL_SERVER_ERROR,
            "application/xml",
            Vec::new(),
        );
    }
    xml_bytes(
        StatusCode::OK,
        "application/xml",
        xml::encode_string(&id_text),
    )
}

async fn post_update(State(app): State<AssetHttp>, headers: HeaderMap, body: Body) -> Response {
    if let Some(response) = app.shard(&headers) {
        return response;
    }
    let Some(bytes) = read_body(app.inner.body_limit, body).await else {
        return too_large();
    };
    if xml::decode_asset(&bytes).is_err() {
        return bad_xml();
    }
    // AssetService.UpdateContent returns false for every id.
    xml_bytes(StatusCode::OK, "application/xml", xml::encode_bool(false))
}

async fn delete_bare(State(app): State<AssetHttp>, headers: HeaderMap) -> Response {
    if let Some(response) = app.shard(&headers) {
        return response;
    }
    xml_bytes(StatusCode::OK, "application/xml", xml::encode_bool(false))
}

async fn delete_asset(
    State(app): State<AssetHttp>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = app.shard(&headers) {
        return response;
    }
    let deleted = app.delete_one(&id).await;
    xml_bytes(StatusCode::OK, "application/xml", xml::encode_bool(deleted))
}

async fn post_exists(State(app): State<AssetHttp>, body: Body) -> Response {
    let Some(bytes) = read_body(app.inner.body_limit, body).await else {
        return too_large();
    };
    let ids = match xml::decode_strings(&bytes) {
        Ok(ids) => ids,
        Err(_) => return bad_xml(),
    };
    let found = match exists_flags(&app, &ids).await {
        Ok(found) => found,
        Err(()) => vec![false; ids.len()],
    };
    xml_bytes(StatusCode::OK, "application/xml", xml::encode_bools(&found))
}

impl AssetHttp {
    fn shard(&self, headers: &HeaderMap) -> Option<Response> {
        if self.inner.options.reject_shard_header && headers.get("x-secondlife-shard").is_some() {
            Some(xml_bytes(StatusCode::FORBIDDEN, "text/plain", Vec::new()))
        } else {
            None
        }
    }

    fn missing(&self, path: &str, id_present: bool) -> Response {
        if id_present && let Some(url) = &self.inner.options.redirect_url {
            return redirect(url, path);
        }
        xml_bytes(StatusCode::NOT_FOUND, "text/plain", Vec::new())
    }

    async fn delete_one(&self, id: &str) -> bool {
        if self.inner.options.delete == DeleteMode::None {
            return false;
        }
        let Some(asset) = self.inner.store.get_parsed(id).await else {
            return false;
        };
        let allowed = self.inner.options.delete == DeleteMode::All || asset.flags & MAPTILE != 0;
        if !allowed {
            return false;
        }
        self.inner.store.delete(asset.id).await.is_ok()
    }
}

async fn exists_flags(app: &AssetHttp, ids: &[String]) -> Result<Vec<bool>, ()> {
    let mut parsed = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(uuid) = parse_id(id) else {
            return Ok(vec![false; ids.len()]);
        };
        parsed.push(uuid);
    }
    app.inner.store.exists(&parsed).await.map_err(|_| ())
}

fn redirect(url: &str, path: &str) -> Response {
    let mut location = url.to_string();
    if !path.starts_with('/') {
        location.push('/');
    }
    location.push_str(path);
    let mut response = xml_bytes(StatusCode::FOUND, "text/plain", Vec::new());
    if let Ok(value) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

fn bad_xml() -> Response {
    xml_bytes(StatusCode::BAD_REQUEST, "application/xml", Vec::new())
}

fn xml_bytes(status: StatusCode, content_type: &'static str, body: Vec<u8>) -> Response {
    (status, [(header::CONTENT_TYPE, content_type)], body).into_response()
}

fn too_large() -> Response {
    xml_bytes(StatusCode::PAYLOAD_TOO_LARGE, "text/plain", Vec::new())
}

async fn read_body(limit: usize, body: Body) -> Option<axum::body::Bytes> {
    // `None` means the body exceeded `limit`. A `Result` whose error is a
    // `Response` trips clippy 1.99 `result_large_err`.
    axum::body::to_bytes(body, limit).await.ok()
}

impl AssetStore {
    async fn get_parsed(&self, id: &str) -> Option<super::model::Asset> {
        let id = parse_id(id)?;
        self.get(id).await.unwrap_or_default()
    }
}
