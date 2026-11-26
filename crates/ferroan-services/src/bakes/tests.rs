// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use axum::body::{Body, to_bytes};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use tower::ServiceExt;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("bakes-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config(base: &Path) -> Config {
    let mut c = Config::default();
    c.set(
        SECTION,
        "LocalServiceModule",
        "OpenSim.Server.Handlers.dll:XBakes",
    );
    c.set(SECTION, "BaseDirectory", base.to_str().unwrap());
    c
}

fn files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            files(&p, found);
        } else {
            found.push(p);
        }
    }
}

#[tokio::test]
async fn xbakes_replies_commands_and_files_match_pin8() {
    let dir = Directory::new();
    let cache = Bakes::configured(&config(&dir.0), 8003).unwrap().unwrap();
    let router = cache.clone().into_router();
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../fixtures/avatar/bakes.json")).unwrap();
    for case in fixture["observations"].as_array().unwrap() {
        if let Some(command) = case["command"].as_str() {
            let args: Vec<_> = command.split_whitespace().skip(2).collect();
            assert_eq!(cache.delete(&args), case["output"].as_str().unwrap());
        } else {
            let label = &case["label"];
            let mut request = Request::builder()
                .method(case["method"].as_str().unwrap())
                .uri(case["target"].as_str().unwrap());
            if case["shard"] == true {
                request = request.header("X-SecondLife-Shard", "fixture");
            }
            if let Some(len) = case["declaredLength"].as_u64() {
                request = request.header(header::CONTENT_LENGTH, len);
            }
            let bytes = STANDARD
                .decode(case["data"].as_str().unwrap_or_default())
                .unwrap();
            let response = router
                .clone()
                .oneshot(request.body(Body::from(bytes)).unwrap())
                .await
                .unwrap();
            assert_eq!(
                u64::from(response.status().as_u16()),
                case["status"].as_u64().unwrap(),
                "{label}"
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .map(|v| v.to_str().unwrap()),
                case["contentType"].as_str(),
                "{label}"
            );
            let reply = to_bytes(response.into_body(), MAX_BYTES).await.unwrap();
            assert_eq!(
                STANDARD.encode(reply),
                case["reply"].as_str().unwrap(),
                "{label}"
            );
        }
        let mut actual = Vec::new();
        files(&dir.0, &mut actual);
        let stored = case["stored"].as_array().unwrap();
        assert_eq!(actual.len(), stored.len());
        for f in stored {
            let path = dir.0.join(f["path"].as_str().unwrap());
            assert!(actual.contains(&path));
            assert_eq!(
                STANDARD.encode(fs::read(path).unwrap()),
                f["bytes"].as_str().unwrap()
            );
        }
    }
}

#[test]
fn full_ini_keeps_xbakes_opt_in_and_honours_transport_settings() {
    let mut c = config(Path::new("bakes"));
    assert!(Bakes::configured(&c, 8003).unwrap().is_some());
    c.set("ServiceList", "Other", "another connector");
    assert!(Bakes::configured(&c, 8003).unwrap().is_none());
    c.set(
        "ServiceList",
        SECTION,
        "8003/OpenSim.Server.Handlers.dll:XBakesConnector",
    );
    assert!(Bakes::configured(&c, 8003).unwrap().is_some());
    assert!(Bakes::configured(&c, 8003).unwrap().unwrap().reject_shard);
    assert!(Bakes::configured(&c, 8002).is_err());
    c.set("Network", "AllowllHTTPRequestIn", "true");
    assert!(!Bakes::configured(&c, 8003).unwrap().unwrap().reject_shard);
    c.set(SECTION, "AllowllHTTPRequestIn", "false");
    assert!(Bakes::configured(&c, 8003).unwrap().unwrap().reject_shard);
    c.set(SECTION, "AuthType", "BasicHttpAuthentication");
    assert!(Bakes::configured(&c, 8003).is_err());
    c.set(SECTION, "AuthType", "None");
    c.set(SECTION, "BaseDirectory", "");
    assert!(Bakes::configured(&c, 8003).is_err());
}

#[tokio::test]
async fn an_oversized_stream_preserves_the_previous_file() {
    use axum::body::Bytes;
    let dir = Directory::new();
    let cache = Bakes::configured(&config(&dir.0), 8003).unwrap().unwrap();
    let id = "abcdef0123456789";
    let path = cache.path(id).unwrap();
    replace_file(&path, b"previous").unwrap();
    // Reuse one allocation. The stream supplies no Content-Length.
    let chunk = Bytes::from(vec![0; 1024 * 1024]);
    let count = MAX_BYTES / chunk.len();
    let last = chunk.slice(..MAX_BYTES % chunk.len() + 1);
    let chunks = std::iter::repeat_n(chunk, count)
        .chain([last])
        .map(Ok::<_, io::Error>);
    let request = Request::post(format!("/bakes/{id}"))
        .body(Body::from_stream(futures_util::stream::iter(chunks)))
        .unwrap();
    let reply = cache.into_router().oneshot(request).await.unwrap();
    assert_eq!(reply.status(), StatusCode::BAD_REQUEST);
    assert_eq!(fs::read(path).unwrap(), b"previous");
}

#[test]
fn the_read_bound_also_applies_to_files_left_by_another_writer() {
    let dir = Directory::new();
    let file = dir.0.join("large");
    File::create(&file)
        .unwrap()
        .set_len(MAX_BYTES as u64 + 1)
        .unwrap();
    assert_eq!(read_file(&file).status(), StatusCode::INTERNAL_SERVER_ERROR);
}
