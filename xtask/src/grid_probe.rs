// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local test-grid probes of AgentHandlers, Scene.VerifyUserPresence and Robust's
//! Presence/GridUser HTTP services at Tranquillity f67db54399c027fd74e5aefcb2193905b4dc1646.
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
/// Overall budget for one service-state observation. A healthy GridUser or
/// Presence handler returns in milliseconds. This stays ten seconds.
const SERVICE_DEADLINE: Duration = Duration::from_secs(10);
/// One private POST. `probe.service-attempt`. A socket that does not answer
/// is abandoned so the next attempt opens a new connection.
const SERVICE_ATTEMPT: Duration = Duration::from_secs(1);
const PIN: &str = "f67db54399c027fd74e5aefcb2193905b4dc1646";
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

pub async fn online(
    private_uri: &str,
    directory: &Path,
    ferroan: bool,
    arrival: ferroan_bot::scenario::RootArrival,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + SERVICE_DEADLINE;
    loop {
        let mut grid = fields("getgriduserinfo");
        grid.insert("UserID".into(), Field::Str(arrival.agent_id.to_string()));
        let grid = post_before(deadline, &format!("{private_uri}/griduser"), &grid).await?;
        let mut presence = fields("getagent");
        presence.insert(
            "SessionID".into(),
            Field::Str(arrival.session_id.to_string()),
        );
        let presence = post_before(deadline, &format!("{private_uri}/presence"), &presence).await?;
        if let (Some(Field::Map(grid)), Some(Field::Map(presence))) =
            (grid.get("result"), presence.get("result"))
        {
            let position = match grid.get("LastPosition") {
                Some(Field::Str(value)) => vector(value),
                _ => None,
            };
            let look = match grid.get("LastLookAt") {
                Some(Field::Str(value)) => vector(value),
                _ => None,
            };
            if online_matches(grid, presence, &arrival) {
                let observation = json!({"source":if ferroan {"ferroan"} else {"csharp"},"revision":PIN,"online":true,"presence_region":REGION,"last_region":REGION,"last_position":position,"last_look_at":look,"arrival_position":arrival.position,"arrival_look_at":arrival.look_at,"arrival_handle":arrival.region_handle});
                fs::write(
                    directory.join("root-arrival-online.json"),
                    serde_json::to_vec_pretty(&observation).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                if ferroan {
                    let baseline =
                        read(&crate::repo_root().join("fixtures/root-arrival/exchanges.json"))?;
                    for key in [
                        "online",
                        "presence_region",
                        "last_region",
                        "last_look_at",
                        "arrival_look_at",
                        "arrival_handle",
                    ] {
                        if observation[key] != baseline["online"][key] {
                            return Err(format!("root-arrival {key} differs from C#"));
                        }
                    }
                    for axis in 0..2 {
                        if observation["last_position"][axis]
                            != baseline["online"]["last_position"][axis]
                        {
                            return Err("root-arrival XY differs from C#".into());
                        }
                    }
                }
                return Ok("root arrival: Presence region and GridUser online/last region/position match current AgentMovementComplete; stored look is zero".into());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("root-arrival services did not settle to the current online circuit region and position".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn online_matches(
    grid: &FieldMap,
    presence: &FieldMap,
    arrival: &ferroan_bot::scenario::RootArrival,
) -> bool {
    let text = |map: &FieldMap, key: &str, expected: &str| matches!(map.get(key), Some(Field::Str(value)) if value == expected);
    let position = match grid.get("LastPosition") {
        Some(Field::Str(value)) => vector(value),
        _ => None,
    };
    let look = match grid.get("LastLookAt") {
        Some(Field::Str(value)) => vector(value),
        _ => None,
    };
    // Vector3.ToString uses Single.ToString(EnUsCulture). At the pinned .NET
    // runtime those values round trip; compare parsed singles without epsilon.
    text(grid, "Online", "True")
        && text(grid, "UserID", &arrival.agent_id.to_string())
        && text(presence, "UserID", &arrival.agent_id.to_string())
        && text(presence, "RegionID", &arrival.region_id.to_string())
        && text(grid, "LastRegionID", &arrival.region_id.to_string())
        && position == Some(arrival.position)
        && look == Some([0.0; 3])
}

pub(super) fn vector(text: &str) -> Option<[f32; 3]> {
    let text = text.strip_prefix('<')?.strip_suffix('>')?;
    let mut parts = text.split(',');
    let mut vector = [0.0_f32; 3];
    for value in &mut vector {
        *value = parts.next()?.trim().parse().ok()?;
        if !value.is_finite() {
            return None;
        }
    }
    parts.next().is_none().then_some(vector)
}

/// Grid scenarios reuse accounts. A wire LogoutReply precedes asynchronous
/// service cleanup, so never let the next scenario log in before both services
/// settle. Standalone keeps using the bot's ordinary scenario runner.
pub async fn run_scenarios<'a, F, Fut>(
    scenario: &'a str,
    private_uri: &str,
    mut run: F,
) -> Result<String, String>
where
    F: FnMut(&'a str) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let mut names =
        ferroan_bot::scenario::selection(scenario).map_or_else(|| vec![scenario], <[_]>::to_vec);
    if scenario == "all" {
        names.push("neighbour");
    }
    let mut lines = Vec::new();
    for name in names {
        let result = async {
            let line = run(name).await?;
            offline(private_uri).await?;
            if name == "chat" || name == "roots" {
                offline_user(private_uri, crate::live::CHAT_USER).await?;
            }
            Ok::<_, String>(line)
        }
        .await;
        match result {
            Ok(line) => lines.push(
                line.strip_suffix("\nlive: run passed")
                    .unwrap_or(&line)
                    .to_owned(),
            ),
            Err(error) => {
                lines.push(format!("{name}: fail ({error})"));
                return Err(lines.join("\n"));
            }
        }
    }
    lines.push("live: run passed".into());
    Ok(lines.join("\n"))
}

pub async fn offline(private_uri: &str) -> Result<(), String> {
    offline_user(private_uri, crate::live::ESTATE_OWNER).await
}

async fn offline_user(private_uri: &str, user: &str) -> Result<(), String> {
    await_settled(private_uri, user).await
}

/// Wait until GridUser is offline and Presence has no session, or fail with
/// the last state. The login is the caller's; this function never logs in.
pub async fn await_settled(private_uri: &str, user: &str) -> Result<(), String> {
    require_principal(user)?;
    // ActivityDetector can finish after LogoutReply; observe bounded eventual
    // service state rather than accepting the viewer acknowledgement as proof.
    let deadline = tokio::time::Instant::now() + SERVICE_DEADLINE;
    let mut grid_offline = false;
    let mut presence_empty = false;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(settle_error(grid_offline, presence_empty));
        }
        let mut grid = fields("getgriduserinfo");
        grid.insert("UserID".into(), Field::Str(user.into()));
        let grid = match post_before(deadline, &service_url(private_uri, "griduser"), &grid).await {
            Ok(value) => value,
            Err(error) => {
                return Err(format!(
                    "{error}; {}",
                    settle_error(grid_offline, presence_empty)
                ));
            }
        };
        let mut presence = fields("getagents");
        presence.insert("uuids".into(), Field::List(vec![user.into()]));
        let presence =
            match post_before(deadline, &service_url(private_uri, "presence"), &presence).await {
                Ok(value) => value,
                Err(error) => {
                    return Err(format!(
                        "{error}; {}",
                        settle_error(grid_offline, presence_empty)
                    ));
                }
            };
        grid_offline = grid_user_is_offline(&grid);
        presence_empty =
            matches!(presence.get("result"), Some(Field::Str(value)) if value == "null");
        if grid_offline && presence_empty {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(settle_error(grid_offline, presence_empty));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The one login every `xtask` caller uses. A refused or unfinished settle
/// returns before `login_to_simulator` is posted.
pub async fn login_when_settled(
    probe: &str,
    principal: &str,
    http: &reqwest::Client,
    request: &ferroan_bot::login::LoginRequest,
) -> Result<ferroan_bot::login::LoginResponse, String> {
    await_settled(probe, principal).await?;
    ferroan_bot::login(http, request)
        .await
        .map_err(|error| error.to_string())
}

fn require_principal(user: &str) -> Result<(), String> {
    let id =
        uuid::Uuid::parse_str(user).map_err(|_| "the login principal is not a uuid".to_string())?;
    if id.is_nil() {
        return Err("the login principal is nil".to_string());
    }
    Ok(())
}

/// A present row is offline only when `Online` is false. No row is also
/// offline: `GetGridUserInfo` answers `Failure` (`GridUserServerPostHandler`),
/// and the first login has no griduser row until login creates one.
fn grid_user_is_offline(grid: &FieldMap) -> bool {
    match grid.get("result") {
        Some(Field::Map(info)) => matches!(
            info.get("Online"),
            Some(Field::Str(value)) if value.eq_ignore_ascii_case("false")
        ),
        Some(Field::Str(value)) if value == "Failure" || value == "null" => true,
        _ => false,
    }
}

fn settle_error(grid_offline: bool, presence_empty: bool) -> String {
    format!(
        "logout service state did not settle: GridUser offline={grid_offline}, Presence empty={presence_empty}"
    )
}

fn service_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/{path}")
}

fn service_name(url: &str) -> &str {
    match url.rsplit('/').next() {
        Some(name) if !name.is_empty() => name,
        _ => url,
    }
}

fn deadline_error(service: &str) -> String {
    format!("logout service-state probe exceeded its ten-second deadline ({service})")
}

fn abandon_attempt(service: &str, waited: Duration) {
    let configured = u64::try_from(SERVICE_ATTEMPT.as_millis()).unwrap_or(u64::MAX);
    let observed = u64::try_from(waited.as_millis()).unwrap_or(u64::MAX);
    ferroan_diag::hit(&ferroan_diag::Hit {
        id: ferroan_diag::PROBE_SERVICE_ATTEMPT,
        kind: ferroan_diag::Kind::Hard,
        configured,
        observed,
        action: ferroan_diag::Action::Delay,
        route: service,
        agent_id: None,
    });
}

/// Post one service-state form. Each attempt waits at most [`SERVICE_ATTEMPT`].
///
/// On Q16c `live services` (run 44963949219) `online` posted through the
/// private capture proxy and that one socket did not return for the whole
/// ten-second deadline. Robust logged no slow HTTP line. Logout then opened
/// a new connection, and GridUser and Presence answered at once. The client
/// timeout used to be that same deadline, so the first unanswered POST failed
/// the step. A timed-out attempt is dropped and retried until `deadline`.
/// A transport or HTTP error is returned as before.
async fn post_before(
    deadline: tokio::time::Instant,
    url: &str,
    fields: &FieldMap,
) -> Result<FieldMap, String> {
    let service = service_name(url);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(deadline_error(service));
        }
        let bound = SERVICE_ATTEMPT.min(deadline.saturating_duration_since(now));
        let started = tokio::time::Instant::now();
        match tokio::time::timeout(bound, post_once(url, fields)).await {
            Ok(result) => return result,
            Err(_elapsed) => {
                abandon_attempt(service, started.elapsed());
                if tokio::time::Instant::now() >= deadline {
                    return Err(deadline_error(service));
                }
            }
        }
    }
}

async fn post_once(url: &str, fields: &FieldMap) -> Result<FieldMap, String> {
    // Longer than SERVICE_ATTEMPT so the outer timeout, not this client
    // timeout, is what abandons a stuck socket and retries.
    let client = reqwest::Client::builder()
        .timeout(SERVICE_ATTEMPT + Duration::from_millis(500))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "could not build logout state probe client".to_string())?;
    let body = ferroan_http::build_query_string(fields).map_err(|error| error.to_string())?;
    let mut response = client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("X-Ferroan-Probe", "harness")
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
}

fn fields(method: &str) -> FieldMap {
    [("VERSIONMIN", "1"), ("VERSIONMAX", "1"), ("METHOD", method)]
        .into_iter()
        .map(|(key, value)| (key.to_string(), Field::Str(value.to_string())))
        .collect()
}

/// A deliberate query for a fixed nonexistent test session. A dedicated proxy
/// records it separately from CreateAgent's concurrent inventory prefetch.
pub async fn missing_presence(private_uri: &str) -> Result<(), String> {
    let mut request = fields("getagent");
    request.insert(
        "SessionID".into(),
        Field::Str("00000000-0000-4000-8000-000000000004".into()),
    );
    let reply = post_before(
        tokio::time::Instant::now() + SERVICE_DEADLINE,
        &service_url(private_uri, "presence"),
        &request,
    )
    .await?;
    if !matches!(reply.get("result"), Some(Field::Str(value)) if value == "null") {
        return Err("isolated missing-session probe unexpectedly found a Presence".into());
    }
    Ok(())
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
            .header("Content-Type", "application/json")
            .header("X-Ferroan-Probe", "harness");
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
    // The probe writes `source` itself. The C#-only proof is the pinned channel
    // on this directory's login circuit, the same marker login fixtures require.
    let provenance = crate::arrival_fixtures::csharp_provenance(&super::login_capture::records(
        &directory.join("capture-udp.cap"),
    )?)?;
    if report["source"] != "csharp" || report["revision"] != PIN {
        return Err("region parity fixtures require a capture from pinned C#".into());
    }
    report["provenance"] = provenance;
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

pub fn generate_root_arrival(directory: &Path) -> Result<String, String> {
    let online = read(&directory.join("root-arrival-online.json"))?;
    if online["source"] != "csharp" || online["revision"] != PIN {
        return Err("root-arrival fixtures require pinned C# observations".into());
    }
    let marker = super::login_capture::root_arrival_marker(directory)?;
    for key in ["arrival_position", "arrival_look_at", "arrival_handle"] {
        if marker[key] != online[key] {
            return Err(format!(
                "online {key} does not match the captured C# arrival"
            ));
        }
    }
    let root_arrival = json!({"source":"csharp", "revision":PIN,"channel_version":marker["channel_version"],"services":super::login_capture::root_arrival_exchanges(directory)?,"online":online});
    let target = crate::repo_root().join("fixtures/root-arrival");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    fs::write(
        target.join("exchanges.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&root_arrival).map_err(|error| error.to_string())?
        ),
    )
    .map_err(|error| error.to_string())?;
    Ok(
        "root-arrival fixtures: two C# HTTP exchanges and online service observation captured"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn next_scenario_login_waits_for_griduser_and_presence_to_settle() {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let (waiting, held) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let service = tokio::spawn(async move {
            let mut waiting = Some(waiting);
            let mut released = Some(released);
            // First only Presence is clear; then only GridUser is offline.
            // The third round is held until the test allows both to settle.
            for index in 0..6 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 4096];
                    let n = socket.read(&mut bytes).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&bytes[..n]);
                    let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") else {
                        continue;
                    };
                    let headers = std::str::from_utf8(&request[..end]).unwrap();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.contains(if index % 2 == 0 {
                    "METHOD=getgriduserinfo"
                } else {
                    "METHOD=getagents"
                }));
                if index == 5 {
                    waiting.take().unwrap().send(()).unwrap();
                    released.take().unwrap().await.unwrap();
                }
                let result = match index {
                    0 => "<result type=\"List\"><Online>True</Online></result>",
                    2 | 4 => "<result type=\"List\"><Online>False</Online></result>",
                    3 => "<result type=\"List\"><UserID>still-present</UserID></result>",
                    _ => "<result>null</result>",
                };
                let body = format!("<ServerResponse>{result}</ServerResponse>");
                socket.write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                ).as_bytes()).await.unwrap();
            }
        });
        let logins = Arc::new(Mutex::new(Vec::new()));
        let attempted = logins.clone();
        let runner = tokio::spawn(async move {
            run_scenarios("all", &uri, |name| {
                attempted.lock().unwrap().push(name.to_owned());
                // Each successful scenario includes its strict LogoutReply.
                // Stop at the next login attempt: no virtual world is needed
                // to prove the production scenario scheduler's ordering.
                std::future::ready(if name == "login" {
                    Ok("login: pass\nlive: run passed".into())
                } else {
                    Err("stopped at next login attempt".into())
                })
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(3), held)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*logins.lock().unwrap(), ["login"]);
        release.send(()).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), runner)
            .await
            .unwrap()
            .unwrap();
        assert!(
            result
                .unwrap_err()
                .contains("walk: fail (stopped at next login attempt)")
        );
        assert_eq!(*logins.lock().unwrap(), ["login", "walk"]);
        service.await.unwrap();
    }

    #[tokio::test]
    async fn nil_principal_does_not_log_in() {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let error = login_when_settled(
            "http://127.0.0.1:9/",
            "00000000-0000-0000-0000-000000000000",
            &http,
            &ferroan_bot::login::LoginRequest {
                login_uri: "http://127.0.0.1:9/".into(),
                username: "Test User".into(),
                password: "unused".into(),
                start: "last".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("nil"), "{error}");
    }

    #[tokio::test]
    async fn a_missing_griduser_row_does_not_block_the_first_login() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let owner = crate::live::ESTATE_OWNER;
        let logins = Arc::new(AtomicUsize::new(0));
        let login_count = logins.clone();
        let service = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http(&mut socket).await;
                if request.contains("login_to_simulator") {
                    login_count.fetch_add(1, Ordering::SeqCst);
                    socket
                        .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                    break;
                }
                assert!(request.contains(owner), "probe omitted the asset account");
                let body = if request.contains("METHOD=getgriduserinfo") {
                    "<ServerResponse><result>Failure</result></ServerResponse>"
                } else {
                    "<ServerResponse><result>null</result></ServerResponse>"
                };
                let bytes = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(bytes.as_bytes()).await.unwrap();
            }
        });
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let started = tokio::time::Instant::now();
        let result = login_when_settled(
            &uri,
            owner,
            &http,
            &ferroan_bot::login::LoginRequest {
                login_uri: format!("{uri}/"),
                username: "Test User".into(),
                password: "unused".into(),
                start: "last".into(),
            },
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(result.is_err(), "the fake login reply is not a session");
        assert_eq!(logins.load(Ordering::SeqCst), 1);
        service.await.unwrap();
    }

    #[tokio::test]
    async fn asset_login_waits_while_presence_is_held() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let owner = crate::live::ESTATE_OWNER;
        let released = Arc::new(AtomicBool::new(false));
        let logins = Arc::new(AtomicUsize::new(0));
        let (waiting, held) = tokio::sync::oneshot::channel();
        let release_flag = released.clone();
        let login_count = logins.clone();
        let service = tokio::spawn(async move {
            let mut waiting = Some(waiting);
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http(&mut socket).await;
                if request.contains("login_to_simulator") {
                    assert!(
                        release_flag.load(Ordering::SeqCst),
                        "login_to_simulator was posted while Presence was held"
                    );
                    login_count.fetch_add(1, Ordering::SeqCst);
                    socket
                        .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                    break;
                }
                assert!(request.contains(owner), "probe omitted the asset account");
                let grid = request.contains("METHOD=getgriduserinfo");
                let held_presence = !release_flag.load(Ordering::SeqCst);
                if !grid
                    && held_presence
                    && let Some(sender) = waiting.take()
                {
                    sender.send(()).unwrap();
                }
                let result = if grid {
                    "<result type=\"List\"><Online>False</Online></result>"
                } else if held_presence {
                    "<result type=\"List\"><UserID>held</UserID></result>"
                } else {
                    "<result>null</result>"
                };
                let body = format!("<ServerResponse>{result}</ServerResponse>");
                let bytes = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(bytes.as_bytes()).await.unwrap();
            }
        });
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let login_uri = format!("{uri}/");
        let probe = uri.clone();
        let runner = tokio::spawn(async move {
            login_when_settled(
                &probe,
                owner,
                &http,
                &ferroan_bot::login::LoginRequest {
                    login_uri,
                    username: "Test User".into(),
                    password: "unused".into(),
                    start: "last".into(),
                },
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(3), held)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(logins.load(Ordering::SeqCst), 0);
        released.store(true, Ordering::SeqCst);
        let result = tokio::time::timeout(Duration::from_secs(3), runner)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err(), "the fake login reply is not a session");
        assert_eq!(logins.load(Ordering::SeqCst), 1);
        service.await.unwrap();
    }

    #[tokio::test]
    async fn login_is_not_attempted_when_the_presence_bound_runs_out() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let owner = crate::live::ESTATE_OWNER;
        let logins = Arc::new(AtomicUsize::new(0));
        let login_count = logins.clone();
        let service = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let request = read_http(&mut socket).await;
                if request.contains("login_to_simulator") {
                    login_count.fetch_add(1, Ordering::SeqCst);
                    let _ = socket
                        .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    continue;
                }
                assert!(request.contains(owner), "probe omitted the asset account");
                let grid = request.contains("METHOD=getgriduserinfo");
                let result = if grid {
                    "<result type=\"List\"><Online>True</Online></result>"
                } else {
                    "<result type=\"List\"><UserID>held</UserID></result>"
                };
                let body = format!("<ServerResponse>{result}</ServerResponse>");
                let bytes = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(bytes.as_bytes()).await;
            }
        });
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let error = login_when_settled(
            &uri,
            owner,
            &http,
            &ferroan_bot::login::LoginRequest {
                login_uri: format!("{uri}/"),
                username: "Test User".into(),
                password: "unused".into(),
                start: "last".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("GridUser offline=false"), "{error}");
        assert!(error.contains("Presence empty=false"), "{error}");
        assert_eq!(logins.load(Ordering::SeqCst), 0);
        service.abort();
    }

    #[test]
    fn online_probe_requires_current_identity_region_exact_position_and_live_state() {
        let fixture: Value =
            serde_json::from_slice(include_bytes!("../../fixtures/root-arrival/exchanges.json"))
                .unwrap();
        let form = ferroan_http::parse_query_string(
            fixture["services"]["griduser-setposition"]["request"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let Field::Str(position) = &form["Position"] else {
            panic!("fixture position");
        };
        let arrival = ferroan_bot::scenario::RootArrival {
            agent_id: uuid::Uuid::parse_str(AGENT).unwrap(),
            session_id: uuid::Uuid::from_u128(4),
            region_id: uuid::Uuid::parse_str(REGION).unwrap(),
            position: [128.0, 128.0, f32::from_bits(0x41cf_ae15)],
            look_at: [0.99, 0.042, 0.0],
            region_handle: 1_099_511_628_032_000,
        };
        let grid: FieldMap = [
            ("UserID", AGENT),
            ("LastRegionID", REGION),
            ("Online", "True"),
            ("LastPosition", position),
            ("LastLookAt", "<0, 0, 0>"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), Field::Str(v.into())))
        .collect();
        let presence: FieldMap = [("UserID", AGENT), ("RegionID", REGION)]
            .into_iter()
            .map(|(k, v)| (k.into(), Field::Str(v.into())))
            .collect();
        assert!(online_matches(&grid, &presence, &arrival));
        for (key, value) in [
            ("UserID", "wrong"),
            ("LastRegionID", "00000000-0000-0000-0000-000000000000"),
            ("Online", "False"),
            ("LastPosition", "<128, 128, 25.96>"),
            ("LastLookAt", "<0.99, 0.042, 0>"),
        ] {
            let mut stale = grid.clone();
            stale.insert(key.into(), Field::Str(value.into()));
            assert!(!online_matches(&stale, &presence, &arrival), "{key}");
        }
        for key in ["UserID", "RegionID"] {
            let mut wrong = presence.clone();
            wrong.insert(key.into(), Field::Str("wrong".into()));
            assert!(!online_matches(&grid, &wrong, &arrival));
        }
        for invalid in ["128, 128, 0", "<0, 0>", "<0, 0, 0, 1>", "<NaN, 0, 0>"] {
            assert!(vector(invalid).is_none());
        }
        assert_eq!(
            fixture["online"]["last_position"],
            fixture["online"]["arrival_position"]
        );
        assert_ne!(
            fixture["online"]["last_look_at"],
            fixture["online"]["arrival_look_at"]
        );
    }

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
        let mut complete = serde_json::Map::new();
        for case in CASES {
            complete.insert((*case).to_string(), json!({}));
        }
        for report in [
            json!({"source":"ferroan","revision":PIN}),
            json!({"source":"csharp","revision":PIN,"exchanges":{}}),
            json!({"source":"csharp","revision":PIN,"exchanges":complete}),
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

    #[tokio::test]
    async fn an_expired_probe_deadline_names_the_service() {
        let deadline = tokio::time::Instant::now();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let started = tokio::time::Instant::now();
        let error = post_before(deadline, "http://127.0.0.1:9/presence", &fields("getagent"))
            .await
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(500), "{error}");
        assert!(error.contains("ten-second deadline"), "{error}");
        assert!(error.contains("presence"), "{error}");
        assert!(!error.contains("127.0.0.1"), "{error}");
    }

    #[tokio::test]
    async fn a_stalled_service_post_is_abandoned_inside_one_second() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let accepts = Arc::new(AtomicUsize::new(0));
        let count = accepts.clone();
        let service = tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let index = count.fetch_add(1, Ordering::SeqCst);
                let request = read_http(&mut socket).await;
                assert!(request.contains("METHOD=getagent"), "{request}");
                if index == 0 {
                    held.push(socket);
                    continue;
                }
                let body = "<ServerResponse><result>null</result></ServerResponse>";
                let bytes = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(bytes.as_bytes()).await.unwrap();
                break;
            }
            drop(held);
        });
        let started = tokio::time::Instant::now();
        missing_presence(&uri).await.unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(800),
            "the first socket was not abandoned: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(4),
            "one stuck socket still spent the ten-second budget: {elapsed:?}"
        );
        assert!(accepts.load(Ordering::SeqCst) >= 2);
        let metrics = ferroan_diag::metrics();
        assert!(
            metrics.contains("limit=\"probe.service-attempt\",action=\"delay\""),
            "{metrics}"
        );
        service.await.unwrap();
    }

    async fn read_http(socket: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut request = Vec::new();
        loop {
            let mut bytes = [0; 4096];
            let n = socket.read(&mut bytes).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&bytes[..n]);
            let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let headers = std::str::from_utf8(&request[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                break;
            }
        }
        String::from_utf8(request).unwrap()
    }
}
