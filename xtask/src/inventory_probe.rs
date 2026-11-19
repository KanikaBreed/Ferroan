// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Compare inventory and asset capabilities on the C# neighbour and the Ferroan
//! home region. Seed URLs differ by port, so the check is the name set and that
//! each value is an absolute HTTP URL. Inventory is equal LLSD. Asset bodies,
//! status, content type and `Content-Range` are compared byte for byte.
//! Mismatched bodies are written under the grid root and are not printed.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use ferroan_bot::login::{LoginRequest, http_client, login};
use ferroan_bot::session::{Agent, Session};
use ferroan_llsd::{Llsd, from_xml, to_xml};
use uuid::Uuid;

const ASK: &[&str] = &[
    "EventQueueGet",
    "FetchInventoryDescendents2",
    "FetchInventory2",
    "FetchLibDescendents2",
    "FetchLib2",
    "GetTexture",
    "GetMesh",
    "GetMesh2",
    "ViewerAsset",
    "NewFileAgentInventory",
];

const LIBRARY_OWNER: &str = "11111111-1111-0000-0000-000100bba000";
const LIBRARY_ROOT: &str = "00000112-000f-0000-0000-000100bba000";
const LIBRARY_ITEM: &str = "00000000-0000-2222-4444-100000001000";
const LIBRARY_TEXTURE: &str = "2a4880b6-b7a3-690a-2049-bfbe38eafb9f";
const MISSING_ASSET: &str = "ffffffff-ffff-ffff-ffff-ffffffffffff";
const MAX_BODY: usize = 16 * 1024 * 1024;

struct HttpReply {
    status: u16,
    content_type: String,
    content_range: Option<String>,
    body: Vec<u8>,
}

struct Evidence {
    names: BTreeSet<String>,
    folder: Llsd,
    library: Llsd,
    library_as_user: Llsd,
    library_item: Llsd,
    user_item: Llsd,
    texture: HttpReply,
    ranged: HttpReply,
    viewer: HttpReply,
    missing: HttpReply,
}

/// Log into the neighbour, then the home region, and compare one folder, one
/// library folder, one library item, one user item, and texture responses.
pub async fn compare(
    login_uri: &str,
    password: &str,
    private_uri: &str,
    scratch: &Path,
) -> Result<String, String> {
    let neighbour = visit(login_uri, password, "uri:Ferroan Neighbour&128&128&25").await?;
    crate::grid::login_probe::offline(private_uri).await?;
    let home = visit(login_uri, password, "uri:Ferroan Test&128&128&25").await?;
    crate::grid::login_probe::offline(private_uri).await?;
    if neighbour.names != home.names {
        return Err(format!(
            "seed names differ: csharp [{}] ferroan [{}]",
            neighbour
                .names
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            home.names.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    for name in ASK {
        if !home.names.contains(*name) {
            return Err(format!("the seed did not grant {name}"));
        }
    }
    same_llsd(scratch, "inventory-folder", &neighbour.folder, &home.folder)?;
    same_llsd(scratch, "library-folder", &neighbour.library, &home.library)?;
    same_llsd(
        scratch,
        "library-folder-user-cap",
        &neighbour.library_as_user,
        &home.library_as_user,
    )?;
    same_llsd(
        scratch,
        "library-item",
        &neighbour.library_item,
        &home.library_item,
    )?;
    same_llsd(scratch, "user-item", &neighbour.user_item, &home.user_item)?;
    same_http(scratch, "texture", &neighbour.texture, &home.texture)?;
    same_http(scratch, "texture-range", &neighbour.ranged, &home.ranged)?;
    same_http(scratch, "viewer-asset", &neighbour.viewer, &home.viewer)?;
    same_http(scratch, "missing-asset", &neighbour.missing, &home.missing)?;
    if home.texture.status != 200 || media_type(&home.texture.content_type) != "image/x-j2c" {
        return Err(format!(
            "full texture status {} type {}",
            home.texture.status, home.texture.content_type
        ));
    }
    if home.ranged.status != 206 || home.ranged.content_range.is_none() {
        return Err(format!(
            "ranged texture status {} range {:?}",
            home.ranged.status, home.ranged.content_range
        ));
    }
    if home.missing.status != 404 {
        return Err(format!("missing asset status {}", home.missing.status));
    }
    Ok(format!(
        "inventory caps: seed names {}, folder, library, item and texture bodies match the C# neighbour",
        home.names.len()
    ))
}

async fn visit(login_uri: &str, password: &str, start: &str) -> Result<Evidence, String> {
    let http = http_client().map_err(|error| error.to_string())?;
    let response = login(
        &http,
        &LoginRequest {
            login_uri: login_uri.to_owned(),
            username: "Test User".to_owned(),
            password: password.to_owned(),
            start: start.to_owned(),
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut session = Session::connect(
        SocketAddr::from((response.sim_ip, response.sim_port)),
        Agent {
            agent_id: response.agent_id,
            session_id: response.session_id,
            circuit_code: response.circuit_code,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    session
        .run_until_ready(&http, &response.seed_capability, Duration::from_secs(30))
        .await
        .map_err(|error| error.to_string())?;
    let seed = post_seed(&http, &response.seed_capability).await?;
    session
        .drive(Duration::from_millis(200))
        .await
        .map_err(|error| error.to_string())?;
    let folder_id = textures_folder(&response)?;
    let owner = Uuid::parse_str(LIBRARY_OWNER).map_err(|_| "library owner is not a uuid")?;
    let root = Uuid::parse_str(LIBRARY_ROOT).map_err(|_| "library root is not a uuid")?;
    let library_item = Uuid::parse_str(LIBRARY_ITEM).map_err(|_| "library item is not a uuid")?;
    let texture = Uuid::parse_str(LIBRARY_TEXTURE).map_err(|_| "library texture is not a uuid")?;
    let missing = Uuid::parse_str(MISSING_ASSET).map_err(|_| "missing asset is not a uuid")?;
    let folder = post_cap(
        &http,
        cap(&seed, "FetchInventoryDescendents2")?,
        &folder_request(folder_id, response.agent_id),
    )
    .await?;
    let user_item_id = first_item(&folder)?;
    let library = post_cap(
        &http,
        cap(&seed, "FetchLibDescendents2")?,
        &folder_request(root, owner),
    )
    .await?;
    let library_as_user = post_cap(
        &http,
        cap(&seed, "FetchInventoryDescendents2")?,
        &folder_request(root, owner),
    )
    .await?;
    let library_item = post_cap(
        &http,
        cap(&seed, "FetchLib2")?,
        &items_request(response.agent_id, &[library_item]),
    )
    .await?;
    let user_item = post_cap(
        &http,
        cap(&seed, "FetchInventory2")?,
        &items_request(response.agent_id, &[user_item_id]),
    )
    .await?;
    let texture_reply = get_asset(
        &http,
        cap(&seed, "GetTexture")?,
        "texture_id",
        texture,
        None,
    )
    .await?;
    let ranged = get_asset(
        &http,
        cap(&seed, "GetTexture")?,
        "texture_id",
        texture,
        Some("bytes=0-31"),
    )
    .await?;
    let viewer = get_asset(
        &http,
        cap(&seed, "ViewerAsset")?,
        "texture_id",
        texture,
        None,
    )
    .await?;
    let missing = get_asset(
        &http,
        cap(&seed, "GetTexture")?,
        "texture_id",
        missing,
        None,
    )
    .await?;
    session
        .logout(Duration::from_secs(30))
        .await
        .map_err(|error| error.to_string())?;
    Ok(Evidence {
        names: seed.keys().cloned().collect(),
        folder,
        library,
        library_as_user,
        library_item,
        user_item,
        texture: texture_reply,
        ranged,
        viewer,
        missing,
    })
}

fn textures_folder(login: &ferroan_bot::login::LoginResponse) -> Result<Uuid, String> {
    for folder in login.raw.get("inventory-skeleton").as_array() {
        let name = folder.get("name").as_str().trim_matches('"');
        if name == "Textures" {
            let id = folder.get("folder_id").as_uuid();
            if !id.is_nil() {
                return Ok(id);
            }
        }
    }
    Err("the login reply has no Textures folder".to_string())
}

fn first_item(folder: &Llsd) -> Result<Uuid, String> {
    for item in folder.get("folders").at(0).get("items").as_array() {
        let id = item.get("item_id").as_uuid();
        if !id.is_nil() {
            return Ok(id);
        }
    }
    Err("the Textures folder has no item to fetch".to_string())
}

fn cap<'a>(
    seed: &'a std::collections::HashMap<String, String>,
    name: &str,
) -> Result<&'a str, String> {
    seed.get(name)
        .map(String::as_str)
        .filter(|url| absolute_http(url))
        .ok_or_else(|| format!("{name} is missing or is not an absolute http URL"))
}

fn absolute_http(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn folder_request(folder: Uuid, owner: Uuid) -> Llsd {
    let mut one = Llsd::new_map();
    one.insert("folder_id", folder);
    one.insert("owner_id", owner);
    one.insert("fetch_folders", true);
    one.insert("fetch_items", true);
    one.insert("sort_order", 0);
    let mut folders = Llsd::new_array();
    folders.push(one);
    let mut request = Llsd::new_map();
    request.insert("folders", folders);
    request
}

fn items_request(agent: Uuid, ids: &[Uuid]) -> Llsd {
    let mut items = Llsd::new_array();
    for id in ids {
        let mut one = Llsd::new_map();
        one.insert("item_id", *id);
        items.push(one);
    }
    let mut request = Llsd::new_map();
    request.insert("agent_id", agent);
    request.insert("items", items);
    request
}

async fn post_seed(
    http: &reqwest::Client,
    seed: &str,
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut names = Llsd::new_array();
    for name in ASK {
        names.push(*name);
    }
    let reply = post_llsd(http, seed, &names).await?;
    let Some(map) = reply.as_map() else {
        return Err("the seed reply is not an LLSD map".to_string());
    };
    let mut caps = std::collections::HashMap::new();
    for (name, value) in map {
        let text = value.to_string_value();
        if text.is_empty() {
            continue;
        }
        if !absolute_http(&text) {
            return Err(format!("{name} is not an absolute http URL"));
        }
        caps.insert(name.clone(), text);
    }
    Ok(caps)
}

async fn post_cap(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<Llsd, String> {
    post_llsd(http, url, body).await
}

async fn post_llsd(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<Llsd, String> {
    let xml = to_xml(body);
    let response = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/llsd+xml")
        .body(xml)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let bytes = read_body(response).await?;
    if !status.is_success() {
        return Err(format!("capability POST status {}", status.as_u16()));
    }
    from_xml(&bytes).map_err(|_| format!("capability reply is not LLSD ({} bytes)", bytes.len()))
}

async fn get_asset(
    http: &reqwest::Client,
    url: &str,
    key: &str,
    id: Uuid,
    range: Option<&str>,
) -> Result<HttpReply, String> {
    let target = if url.contains('?') {
        format!("{url}&{key}={id}")
    } else {
        format!("{url}?{key}={id}")
    };
    let mut request = http.get(target);
    if let Some(range) = range {
        request = request.header(reqwest::header::RANGE, range);
    }
    let response = request
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = read_body(response).await?;
    Ok(HttpReply {
        status,
        content_type,
        content_range,
        body,
    })
}

async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| error.without_url().to_string())?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err(format!("response exceeds {MAX_BODY} bytes"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn same_llsd(scratch: &Path, name: &str, left: &Llsd, right: &Llsd) -> Result<(), String> {
    if left == right {
        return Ok(());
    }
    let left_xml = to_xml(left);
    let right_xml = to_xml(right);
    let _ = std::fs::write(scratch.join(format!("{name}.csharp.xml")), &left_xml);
    let _ = std::fs::write(scratch.join(format!("{name}.ferroan.xml")), &right_xml);
    Err(format!(
        "{name} LLSD differs ({} and {} bytes); wrote the bodies under the grid root",
        left_xml.len(),
        right_xml.len()
    ))
}

fn same_http(
    scratch: &Path,
    name: &str,
    left: &HttpReply,
    right: &HttpReply,
) -> Result<(), String> {
    let same = left.status == right.status
        && media_type(&left.content_type) == media_type(&right.content_type)
        && left.content_range == right.content_range
        && left.body == right.body;
    if same {
        return Ok(());
    }
    let _ = std::fs::write(scratch.join(format!("{name}.csharp.bin")), &left.body);
    let _ = std::fs::write(scratch.join(format!("{name}.ferroan.bin")), &right.body);
    Err(format!(
        "{name} differs (status {}/{}, {} and {} bytes); wrote the bodies under the grid root",
        left.status,
        right.status,
        left.body.len(),
        right.body.len()
    ))
}

fn media_type(header: &str) -> &str {
    header.split(';').next().unwrap_or(header).trim()
}
