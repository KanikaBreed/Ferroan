// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local test-grid probes of AgentHandlers, Scene.VerifyUserPresence and Robust's
//! Presence/GridUser HTTP services at Tranquillity 858321883f5487a5d192d36ea9d7b90fa468310c.
//! Requests contain only committed fixture identities. Responses retain status,
//! content type and parsed JSON; only the test region name/UUID are folded.

use ferroan_http::{Field, FieldMap};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

const LIMIT: usize = 1024 * 1024;
const PIN: &str = "858321883f5487a5d192d36ea9d7b90fa468310c";
const AGENT: &str = "00000000-0000-4000-8000-000000000001";
const REGION: &str = "00000000-0000-4000-8000-000000000002";
const CASES: &[&str] = &[
    "unknown-session",
    "destination-mismatch",
    "query-numeric-strings",
    "query-non-array-features",
    "query-no-overlap",
    "query-legacy",
];

pub async fn offline(private_uri: &str) -> Result<(), String> {
    // ActivityDetector can finish after LogoutReply; observe bounded eventual
    // service state rather than accepting the viewer acknowledgement as proof.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let mut grid = fields("getgriduserinfo");
        grid.insert(
            "UserID".into(),
            Field::Str(crate::live::ESTATE_OWNER.into()),
        );
        let grid = post_before(deadline, &format!("{private_uri}/griduser"), &grid).await?;
        let mut presence = fields("getagents");
        presence.insert(
            "uuids".into(),
            Field::List(vec![crate::live::ESTATE_OWNER.into()]),
        );
        let presence = post_before(deadline, &format!("{private_uri}/presence"), &presence).await?;
        let grid_offline = matches!(grid.get("result"), Some(Field::Map(info)) if matches!(info.get("Online"), Some(Field::Str(value)) if value.eq_ignore_ascii_case("false")));
        let presence_empty =
            matches!(presence.get("result"), Some(Field::Str(value)) if value == "null");
        if grid_offline && presence_empty {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "logout service state did not settle: GridUser offline={grid_offline}, Presence empty={presence_empty}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn post_before(
    deadline: tokio::time::Instant,
    url: &str,
    fields: &FieldMap,
) -> Result<FieldMap, String> {
    tokio::time::timeout_at(deadline, async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "could not build logout state probe client".to_string())?;
        let body = ferroan_http::build_query_string(fields).map_err(|error| error.to_string())?;
        let mut response = client
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|_| "logout state probe transport failed".to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "logout state probe HTTP status {}",
                response.status().as_u16()
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "logout state probe body failed".to_string())?
        {
            if bytes.len().saturating_add(chunk.len()) > LIMIT {
                return Err("logout state probe response exceeds limit".to_string());
            }
            bytes.extend_from_slice(&chunk);
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "logout state probe response is not UTF-8".to_string())?;
        ferroan_http::parse_xml_response(text).map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "logout service-state probe exceeded its ten-second deadline".to_string())?
}

fn fields(method: &str) -> FieldMap {
    [("VERSIONMIN", "1"), ("VERSIONMAX", "1"), ("METHOD", method)]
        .into_iter()
        .map(|(key, value)| (key.to_string(), Field::Str(value.to_string())))
        .collect()
}

fn cases() -> Result<Vec<(&'static str, &'static str, Value)>, String> {
    let create: Value = serde_json::from_slice(include_bytes!(
        "../../fixtures/region-login/create-agent.request.json"
    ))
    .map_err(|error| error.to_string())?;
    let query: Value = serde_json::from_slice(include_bytes!(
        "../../fixtures/region-login/query-access.request.json"
    ))
    .map_err(|error| error.to_string())?;
    let mut create = create;
    create["destination_uuid"] = json!(crate::live::REGION_UUID);
    create["destination_name"] = json!(super::HOME_NAME);
    let mut missing = create.clone();
    missing["destination_uuid"] = json!("00000000-0000-4000-8000-000000000099");
    missing["destination_name"] = json!("Missing Fixture Region");
    let mut strings = query.clone();
    for field in [
        "simulation_service_supported_min",
        "simulation_service_supported_max",
        "simulation_service_accepted_min",
        "simulation_service_accepted_max",
    ] {
        strings[field] = json!("0.3");
    }
    let mut features = query.clone();
    features["features"] = json!("not an array");
    let mut overlap = query.clone();
    overlap["simulation_service_supported_min"] = json!(2.0);
    overlap["simulation_service_supported_max"] = json!(3.0);
    let mut legacy = query;
    let map = legacy
        .as_object_mut()
        .ok_or("query fixture is not an object")?;
    map.retain(|key, _| !key.starts_with("simulation_service_"));
    legacy["my_version"] = json!("SIMULATION/0.7");
    Ok(vec![
        (CASES[0], "POST", create),
        (CASES[1], "POST", missing),
        (CASES[2], "QUERYACCESS", strings),
        (CASES[3], "QUERYACCESS", features),
        (CASES[4], "QUERYACCESS", overlap),
        (CASES[5], "QUERYACCESS", legacy),
    ])
}

pub async fn probe(home_uri: &str, directory: &Path, ferroan: bool) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let mut exchanges = serde_json::Map::new();
    for (name, method, request) in cases()? {
        let path = if method == "POST" {
            format!("/agent/{AGENT}/")
        } else {
            format!("/agent/{AGENT}/{}/", crate::live::REGION_UUID)
        };
        let mut body = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        let mut builder = client
            .request(
                reqwest::Method::from_bytes(method.as_bytes())
                    .map_err(|error| error.to_string())?,
                format!("{home_uri}{path}"),
            )
            .header("Content-Type", "application/json");
        let encoding = if method == "POST" {
            let mut gzip =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gzip.write_all(&body).map_err(|error| error.to_string())?;
            body = gzip.finish().map_err(|error| error.to_string())?;
            builder = builder.header("X-Content-Encoding", "gzip");
            "gzip"
        } else {
            ""
        };
        let mut response = builder
            .body(body)
            .send()
            .await
            .map_err(|_| format!("region parity probe {name} transport failed"))?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| format!("region parity probe {name} body failed"))?
        {
            if bytes.len().saturating_add(chunk.len()) > LIMIT {
                return Err(format!("region parity probe {name} exceeds response limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let response: Value = serde_json::from_slice(&bytes).map_err(|_| {
            format!("region parity probe {name} did not return JSON (status {status})")
        })?;
        let exchange = json!({"method":method,"path":path,"request":request,"request_content_type":"application/json","request_x_content_encoding":encoding,"status":status,"content_type":content_type,"response":response});
        // All other strings are public fixture inputs or source-defined replies.
        let folded = serde_json::to_string(&exchange)
            .map_err(|error| error.to_string())?
            .replace(crate::live::REGION_UUID, REGION)
            .replace(super::HOME_NAME, "Fixture Region");
        exchanges.insert(
            name.into(),
            serde_json::from_str(&folded).map_err(|error| error.to_string())?,
        );
    }
    let report = json!({"source": if ferroan {"ferroan"} else {"csharp"}, "revision":PIN,"exchanges":exchanges});
    fs::write(
        directory.join("region-parity.json"),
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if ferroan {
        let expected = read(&crate::repo_root().join("fixtures/region-parity/exchanges.json"))?;
        if report["exchanges"] != expected["exchanges"] {
            return Err("region parity answers differ from C#; inspect .scratch/live/grid/region-parity.json".into());
        }
    }
    Ok(format!(
        "region parity: {} captured HTTP answers {}",
        CASES.len(),
        if ferroan {
            "match C#"
        } else {
            "recorded from C#"
        }
    ))
}

fn read(path: &Path) -> Result<Value, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(LIMIT as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() > LIMIT {
        return Err("region parity report exceeds limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub fn generate(directory: &Path) -> Result<String, String> {
    let mut report = read(&directory.join("region-parity.json"))?;
    if report["source"] != "csharp" || report["revision"] != PIN {
        return Err("region parity fixtures require a capture from pinned C#".into());
    }
    if report["exchanges"].as_object().is_none_or(|exchanges| {
        exchanges.len() != CASES.len() || CASES.iter().any(|case| !exchanges.contains_key(*case))
    }) {
        return Err("region parity capture is incomplete".into());
    }
    report["services"] = super::login_capture::service_exchanges(directory)?;
    let target = crate::repo_root().join("fixtures/region-parity");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    fs::write(
        target.join("exchanges.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok("region parity fixtures: six C# HTTP answers captured".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_keep_the_captured_request_shapes_and_double_precision() {
        let fixture: Value = serde_json::from_slice(include_bytes!(
            "../../fixtures/region-parity/exchanges.json"
        ))
        .unwrap();
        for (name, method, request) in cases().unwrap() {
            let folded = serde_json::to_string(&request)
                .unwrap()
                .replace(crate::live::REGION_UUID, REGION)
                .replace(super::super::HOME_NAME, "Fixture Region");
            assert_eq!(
                serde_json::from_str::<Value>(&folded).unwrap(),
                fixture["exchanges"][name]["request"]
            );
            assert_eq!(method, fixture["exchanges"][name]["method"]);
        }
        assert_eq!(
            fixture["exchanges"]["query-numeric-strings"]["response"]["negotiated_inbound_version"]
                .as_f64(),
            Some(f64::from(0.3_f32))
        );
    }

    #[test]
    fn fixture_generator_rejects_rust_and_incomplete_captures() {
        let directory = crate::repo_root()
            .join(".scratch")
            .join(format!("parity-fixture-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        for report in [
            json!({"source":"ferroan","revision":PIN}),
            json!({"source":"csharp","revision":PIN,"exchanges":{}}),
        ] {
            fs::write(
                directory.join("region-parity.json"),
                serde_json::to_vec(&report).unwrap(),
            )
            .unwrap();
            assert!(generate(&directory).is_err());
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
