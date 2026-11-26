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
// Ported from Source/OpenSim.Server.Handlers/BakedTextures/{XBakes,XBakesHandler}.cs
// and Source/OpenSim.Server.GridServer/GridServiceConnectorLoader.cs,
// Source/OpenSim.Framework.Servers.HttpServer/{BaseHttpServer,OSHttpServer/HttpRequestParser}.cs
// at c401bceecd1684832ae8ba86a31b5ee01a4ded7b.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Opaque per-avatar files served by the Robust XBakes connector.
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use ferroan_config::Config;
use ferroan_diag::{BAKES_BYTES, BAKES_ID_BYTES, Kind, refuse};
use uuid::Uuid;

use crate::auth::{AuthError, config_bool_either, refuse_transport};

const SECTION: &str = "BakedTextureService";
// HttpRequestParser.OnHeader checks this before HttpRequest's larger limit.
const MAX_BYTES: usize = 64 * 1024 * 1204;
const MAX_NAME: usize = 255;
const HTML: &str = "text/html;charset=UTF-8";

#[derive(Clone)]
pub struct Bakes {
    base: PathBuf,
    reject_shard: bool,
}

impl Bakes {
    pub const DELETE_HELP: &str = r#"<Level Name="delete"><Level Name="bakes"><Command><Module>fs</Module><Shared>False</Shared><HelpText>delete bakes &lt;ID&gt;</HelpText><LongHelp>Delete agent's baked textures from server</LongHelp><Description></Description></Command></Level></Level>"#;

    /// A full GridServer ini contains the service section even with the
    /// connector commented out. Section-only Ferroan configurations opt in directly.
    pub fn configured(config: &Config, listen_port: u16) -> Result<Option<Self>, AuthError> {
        let startup = config.get("Startup", "ServiceConnectors");
        let has_list = config.section_names().any(|s| s == "ServiceList");
        if startup.is_none() && !has_list {
            return config
                .section_names()
                .any(|s| s == SECTION)
                .then(|| Self::open(config, SECTION))
                .transpose();
        }
        let entries = startup
            .into_iter()
            .chain(config.keys("ServiceList").map(|(_, value)| value));
        let mut selected = None;
        for entry in entries.flat_map(|list| list.split([',', ' ', '\n', '\r', '\t'])) {
            let (prefix, module) = entry.split_once('/').unwrap_or(("", entry));
            if module != "OpenSim.Server.Handlers.dll:XBakesConnector" {
                continue;
            }
            let (section, port) = prefix.split_once('@').unwrap_or((SECTION, prefix));
            if !port.is_empty() {
                let port: u16 = port
                    .parse()
                    .map_err(|_| AuthError::Config("invalid XBakesConnector port"))?;
                if port != 0 && port != listen_port {
                    return Err(AuthError::Config(
                        "XBakesConnector port must match the Robust listener",
                    ));
                }
            }
            if selected.is_none() {
                selected = Some(Self::open(
                    config,
                    if section.is_empty() { SECTION } else { section },
                )?);
            }
        }
        Ok(selected)
    }

    fn open(config: &Config, section: &str) -> Result<Self, AuthError> {
        if !matches!(
            config.get(section, "LocalServiceModule"),
            Some("XBakes" | "OpenSim.Server.Handlers.dll:XBakes")
        ) {
            return Err(AuthError::Config(
                "BakedTextureService requires the XBakes LocalServiceModule",
            ));
        }
        let base = config
            .get(SECTION, "BaseDirectory")
            .filter(|s| !s.is_empty())
            .ok_or(AuthError::Config(
                "BakedTextureService requires BaseDirectory",
            ))?;
        refuse_transport(config, section)?;
        Ok(Self {
            base: PathBuf::from(base),
            reject_shard: !config_bool_either(config, section, "AllowllHTTPRequestIn", false)?,
        })
    }

    fn path(&self, id: &str) -> Option<PathBuf> {
        if id.len() > MAX_NAME {
            refuse(
                BAKES_ID_BYTES,
                Kind::Hard,
                MAX_NAME as u64,
                id.len() as u64,
                "bakes",
            );
            return None;
        }
        // Region callers use a UUID. Preserve the spelling of portable file
        // names and validate the individual shard names as well as the leaf.
        if id.len() < 10
            || !id.is_ascii()
            || ![&id[..2], &id[2..4], &id[4..6], &id[6..10], id]
                .into_iter()
                .all(portable_name)
        {
            return None;
        }
        Some(self.base.join(crate::asset::hash_to_file(id, false)))
    }

    pub fn into_router(self) -> Router {
        Router::new()
            .route("/bakes", any(no_id))
            .route("/bakes/", any(no_id))
            .route("/bakes/{*params}", any(handle))
            .with_state(self)
    }

    /// Arguments after the two registered command words.
    pub fn delete(&self, args: &[&str]) -> &'static str {
        let Some(id) = args.first() else {
            return "Syntax: delete bakes <ID>";
        };
        let Some(path) = self.path(id) else {
            return "Bakes not found";
        };
        match fs::remove_file(path) {
            Ok(()) => "Bakes deleted",
            Err(err) if err.kind() == io::ErrorKind::NotFound => "Bakes not found",
            Err(_) => "Bakes delete failed",
        }
    }
}

fn portable_name(name: &str) -> bool {
    if name.ends_with([' ', '.'])
        || name
            .bytes()
            .any(|b| b.is_ascii_control() || b"<>:\"/\\|?*".contains(&b))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !(matches!(stem.get(..3), Some("COM" | "LPT"))
        && matches!(stem.as_bytes().get(3..), Some([b'1'..=b'9'])))
}

async fn no_id() -> Response {
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

fn oversized(observed: u64) -> Response {
    refuse(BAKES_BYTES, Kind::Hard, MAX_BYTES as u64, observed, "bakes");
    // Pin8's HTTP parser refuses before creating the XBakes response.
    StatusCode::BAD_REQUEST.into_response()
}

async fn handle(State(cache): State<Bakes>, request: Request) -> Response {
    if let Some(length) = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > MAX_BYTES as u64)
    {
        return oversized(length);
    }
    if cache.reject_shard && request.headers().contains_key("X-SecondLife-Shard") {
        return empty(StatusCode::FORBIDDEN);
    }
    let method = request.method();
    if method != Method::POST && method != Method::GET {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let Some(id) = request
        .uri()
        .path()
        .strip_prefix("/bakes/")
        .unwrap_or_default()
        .split(['/', '?', '&'])
        .find(|s| !s.is_empty())
    else {
        return empty(StatusCode::OK);
    };
    let Some(path) = cache.path(id) else {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    };
    if method == Method::GET {
        return tokio::task::spawn_blocking(move || read_file(&path))
            .await
            .unwrap_or_else(|_| empty(StatusCode::INTERNAL_SERVER_ERROR));
    }
    let bytes = match ferroan_http::read_body(request.into_body(), MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(ferroan_http::HttpError::Limit { got, .. }) => return oversized(got as u64),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match tokio::task::spawn_blocking(move || replace_file(&path, &bytes)).await {
        Ok(Ok(())) => empty(StatusCode::OK),
        _ => empty(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn read_file(path: &Path) -> Response {
    let Ok(file) = File::open(path) else {
        return empty(StatusCode::OK);
    };
    let mut bytes = Vec::new();
    let read = match file.metadata() {
        Ok(metadata) if metadata.len() > MAX_BYTES as u64 => return oversized_file(metadata.len()),
        _ => file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes),
    };
    match read {
        Err(_) => empty(StatusCode::OK),
        Ok(n) if n > MAX_BYTES => oversized_file(n as u64),
        Ok(_) => ([(header::CONTENT_TYPE, HTML)], bytes).into_response(),
    }
}

fn oversized_file(length: u64) -> Response {
    refuse(
        BAKES_BYTES,
        Kind::Hard,
        MAX_BYTES as u64,
        length,
        "bakes-get",
    );
    empty(StatusCode::INTERNAL_SERVER_ERROR)
}

fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing bake directory"))?;
    fs::create_dir_all(parent)?;
    // Publish one complete file even if two uploads overlap or a write fails.
    let tmp = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    let mut file = File::create_new(&tmp)?;
    let written = file.write_all(bytes);
    drop(file);
    if let Err(error) = written.and_then(|()| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(tmp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
