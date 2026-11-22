// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Service, cache, console and `POST /griduser` behaviour.
//!
//! The memory rows exercise the service without a database. One SQLite copy
//! of the grid-user fixture checks a real store. The C# bytes are
//! `fixtures/griduser/service.json`. Login and logout in those XML bodies are
//! folded to one token: the fixture records the process-culture spelling, and
//! this service writes the invariant spelling (D37). Console text keeps the
//! stored integers.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ferroan_config::Config;
use ferroan_http::{Field, FieldMap};

use super::http::{self, GridUserHttp};
use super::model::GridUser;
use super::service::{CACHE_MS, GridClock, GridData, GridUserService, SystemClock};
use super::store::{GridUserStore, StoreError};
use super::wire::{self, FAILURE_XML, SUCCESS_XML, Vec3};

const USER: &str = "11111111-1111-1111-1111-111111111111";
const USER2: &str = "22222222-2222-2222-2222-222222222222";
const REGION: &str = "33333333-3333-3333-3333-333333333333";
const PREFIX36: &str = "44444444-4444-4444-4444-444444444444";
const UNIX: i32 = 1_700_000_000;

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

struct Mem {
    rows: Mutex<Vec<GridUser>>,
    fail: Mutex<HashSet<String>>,
}

struct TestClock {
    unix: AtomicI32,
    mono: AtomicU64,
}

impl TestClock {
    fn new(unix: i32) -> Self {
        Self {
            unix: AtomicI32::new(unix),
            mono: AtomicU64::new(0),
        }
    }

    fn set_unix(&self, unix: i32) {
        self.unix.store(unix, Ordering::Relaxed);
    }

    fn advance(&self, millis: u64) {
        self.mono.fetch_add(millis, Ordering::Relaxed);
    }
}

impl GridClock for TestClock {
    fn unix_time(&self) -> i32 {
        self.unix.load(Ordering::Relaxed)
    }

    fn mono_millis(&self) -> u64 {
        self.mono.load(Ordering::Relaxed)
    }
}

impl GridClock for Arc<TestClock> {
    fn unix_time(&self) -> i32 {
        (**self).unix_time()
    }

    fn mono_millis(&self) -> u64 {
        (**self).mono_millis()
    }
}

impl GridData for Arc<Mem> {
    async fn get_all(&self, prefix: &str) -> Result<Vec<GridUser>, StoreError> {
        let rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
        Ok(rows
            .iter()
            .filter(|row| row.user_id.starts_with(prefix))
            .cloned()
            .collect())
    }

    async fn get_online_in_region(&self, region_id: &str) -> Result<Vec<GridUser>, StoreError> {
        let rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
        Ok(rows
            .iter()
            .filter(|row| {
                row.field("LastRegionID") == Some(region_id) && row.field("Online") == Some("True")
            })
            .cloned()
            .collect())
    }

    async fn store(&self, user: &GridUser) -> Result<bool, StoreError> {
        let fail = self.fail.lock().unwrap_or_else(|error| error.into_inner());
        if fail.contains(&user.user_id) {
            return Ok(false);
        }
        drop(fail);
        let mut rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(found) = rows.iter_mut().find(|row| row.user_id == user.user_id) {
            for (key, value) in &user.data {
                found.data.insert(key.clone(), value.clone());
            }
        } else {
            rows.push(user.clone());
        }
        Ok(true)
    }
}

type World = (
    GridUserService<Arc<Mem>, Arc<TestClock>>,
    Arc<Mem>,
    Arc<TestClock>,
);

fn world() -> World {
    let mem = Arc::new(Mem {
        rows: Mutex::new(Vec::new()),
        fail: Mutex::new(HashSet::new()),
    });
    let clock = Arc::new(TestClock::new(UNIX));
    let service = GridUserService::new(Arc::clone(&mem), Arc::clone(&clock));
    (service, mem, clock)
}

fn row(user_id: &str, fields: &[(&str, &str)]) -> GridUser {
    let mut user = GridUser::new(user_id);
    for (key, value) in fields {
        user.data.insert((*key).to_string(), (*value).to_string());
    }
    user
}

#[test]
fn documents_use_the_invariant_date_and_the_indented_result() {
    assert_eq!(wire::unix_general(0), "01/01/1970 00:00:00");
    assert_eq!(wire::unix_general(UNIX), "11/14/2023 22:13:20");
    assert_eq!(wire::en_us_general(0), "1/1/1970 12:00:00 AM");
    assert_eq!(wire::en_us_general(UNIX), "11/14/2023 10:13:20 PM");
    assert_eq!(FAILURE_XML.len(), 83);
    assert!(FAILURE_XML.ends_with(b"</ServerResponse>"));
    assert!(contains(SUCCESS_XML, b"Success"));
    assert_eq!(
        wire::format_vector(Vec3::DEFAULT_POSITION),
        "<128, 128, 70>"
    );
    assert_eq!(wire::format_vector(Vec3::ZERO), "<0, 0, 0>");
    assert_eq!(
        wire::format_vector(Vec3 {
            x: 1.5,
            y: 0.25,
            z: 10.0
        }),
        "<1.5, 0.25, 10>"
    );
    let stored = "<0,0,0>";
    assert_eq!(
        wire::parse_vector(stored)
            .map(wire::format_vector)
            .as_deref(),
        Some("<0, 0, 0>")
    );
    assert_eq!(wire::parse_bool("True "), Some(true));
    assert_eq!(wire::parse_bool("false"), Some(false));
    assert_eq!(wire::parse_i32("1700000000      "), Some(UNIX));
}

#[tokio::test]
async fn loggedin_answers_the_row_and_a_later_read_uses_the_cache() {
    let (service, mem, clock) = world();
    let info = service.logged_in(USER).await.expect("login");
    assert_eq!(info.user_id, USER);
    assert!(info.online);
    assert_eq!(info.login, UNIX);
    assert_eq!(info.logout, 0);
    assert_eq!(info.home_position, "<0, 0, 0>");
    let body = http::dispatch(&service, &format!("METHOD=getgriduserinfo&UserID={USER}")).await;
    let text = String::from_utf8(body).expect("utf8");
    assert!(text.contains("<Online>True</Online>"));
    assert!(text.contains("<Login>11/14/2023 22:13:20</Login>"));
    assert!(text.contains("<Logout>01/01/1970 00:00:00</Logout>"));
    assert!(text.contains("&lt;0, 0, 0&gt;"));

    {
        let mut rows = mem.rows.lock().unwrap_or_else(|error| error.into_inner());
        rows[0]
            .data
            .insert("Online".to_string(), "False".to_string());
    }
    let cached = service.get_info(USER).await.expect("cached").expect("row");
    assert!(cached.online);
    clock.advance(CACHE_MS);
    let fresh = service.get_info(USER).await.expect("fresh").expect("row");
    assert!(!fresh.online);
}

#[tokio::test]
async fn a_cached_miss_hides_a_row_stored_past_the_service() {
    let (service, mem, clock) = world();
    assert!(service.get_info(USER).await.expect("miss").is_none());
    mem.store(&row(
        USER,
        &[("Online", "True"), ("Login", "10"), ("Logout", "0")],
    ))
    .await
    .expect("store");
    assert!(service.get_info(USER).await.expect("still").is_none());
    clock.advance(CACHE_MS);
    let found = service.get_info(USER).await.expect("later").expect("row");
    assert!(found.online);
    assert_eq!(found.login, 10);
}

#[tokio::test]
async fn a_short_id_stays_missing_on_the_service_that_logged_it_in() {
    let (service, mem, _) = world();
    let info = service.logged_in("short-id").await.expect("login");
    assert_eq!(info.user_id, "short-id");
    assert!(service.get_info("short-id").await.expect("get").is_none());
    let again = GridUserService::new(Arc::clone(&mem), TestClock::new(UNIX));
    let found = again.get_info("short-id").await.expect("new").expect("row");
    assert_eq!(found.user_id, "short-id");
}

#[tokio::test]
async fn a_long_id_is_stored_whole_and_read_by_its_first_36_characters() {
    let (service, _, _) = world();
    let long = format!("{PREFIX36}extra");
    let info = service.logged_in(&long).await.expect("login");
    assert_eq!(info.user_id, long);
    let by_long = service.get_info(&long).await.expect("long").expect("row");
    let by_prefix = service
        .get_info(PREFIX36)
        .await
        .expect("prefix")
        .expect("row");
    assert_eq!(by_long.user_id, long);
    assert_eq!(by_prefix.user_id, long);
}

#[tokio::test]
async fn the_most_recent_row_wins_and_a_tie_or_a_bad_stamp_keeps_the_earlier_one() {
    let (service, mem, _) = world();
    mem.store(&row(
        "agent-old",
        &[("Login", "10"), ("Logout", "0"), ("Online", "False")],
    ))
    .await
    .expect("old");
    mem.store(&row(
        "agent-new",
        &[("Login", "40"), ("Logout", "20"), ("Online", "True")],
    ))
    .await
    .expect("new");
    let found = service.get_info("agent").await.expect("get").expect("row");
    assert_eq!(found.user_id, "agent-new");

    mem.store(&row(
        "tie-a",
        &[("Login", "5"), ("Logout", "5"), ("Online", "False")],
    ))
    .await
    .expect("a");
    mem.store(&row(
        "tie-b",
        &[("Login", "5"), ("Logout", "5"), ("Online", "True")],
    ))
    .await
    .expect("b");
    let tie = service.get_info("tie-").await.expect("tie").expect("row");
    assert_eq!(tie.user_id, "tie-a");

    mem.store(&row(
        "bad-a",
        &[("Login", "nope"), ("Logout", "0"), ("Online", "False")],
    ))
    .await
    .expect("bad");
    mem.store(&row(
        "bad-b",
        &[("Login", "90"), ("Logout", "0"), ("Online", "True")],
    ))
    .await
    .expect("later");
    let bad = service.get_info("bad-").await.expect("bad").expect("row");
    assert_eq!(bad.user_id, "bad-a");
}

#[tokio::test]
async fn loggedoutregion_keeps_going_when_one_store_fails_and_an_empty_region_succeeds() {
    let (service, mem, _) = world();
    service.logged_in(USER).await.expect("one");
    service.logged_in(USER2).await.expect("two");
    service
        .set_last_position(USER, REGION, Vec3::DEFAULT_POSITION, Vec3::ZERO)
        .await
        .expect("place");
    service
        .set_last_position(USER2, REGION, Vec3::ZERO, Vec3::ZERO)
        .await
        .expect("place two");
    assert!(
        service
            .logged_out_region("99999999-9999-9999-9999-999999999999")
            .await
            .expect("empty")
    );
    mem.fail
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(USER2.to_string());
    assert!(!service.logged_out_region(REGION).await.expect("aggregate"));
    let first = service.get_info(USER).await.expect("first").expect("row");
    let second = service.get_info(USER2).await.expect("second").expect("row");
    assert!(!first.online);
    assert_eq!(first.logout, UNIX);
    assert!(second.online);
}

#[tokio::test]
async fn loggedin_still_answers_when_the_store_refuses_the_row() {
    let (service, mem, _) = world();
    mem.fail
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(USER.to_string());
    let info = service.logged_in(USER).await.expect("login");
    assert!(info.online);
    let cached = service.get_info(USER).await.expect("cache").expect("row");
    assert_eq!(cached.user_id, USER);
    assert!(
        mem.rows
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
    );
}

#[tokio::test]
async fn the_handler_rejects_a_bad_region_logout_and_formats_the_other_methods() {
    let (service, _, _) = world();
    assert_eq!(
        http::dispatch(&service, "METHOD=loggedoutregion").await,
        FAILURE_XML
    );
    assert_eq!(
        http::dispatch(&service, "METHOD=loggedoutregion&RegionID=nope").await,
        FAILURE_XML
    );
    assert_eq!(
        http::dispatch(
            &service,
            "METHOD=loggedoutregion&RegionID=00000000-0000-0000-0000-000000000000"
        )
        .await,
        FAILURE_XML
    );
    assert_eq!(
        http::dispatch(&service, "METHOD=loggedin").await,
        FAILURE_XML
    );
    assert_eq!(http::dispatch(&service, "METHOD=nope").await, FAILURE_XML);
    assert_eq!(http::dispatch(&service, "").await, FAILURE_XML);

    let login = http::dispatch(&service, &format!("METHOD=loggedin&UserID={USER}")).await;
    assert!(contains(&login, b"<Online>True</Online>"));

    let home = http::dispatch(
        &service,
        &format!(
            "METHOD=sethome&UserID={USER}&RegionID={REGION}&Position=%3C10%2C%2020%2C%2030%3E"
        ),
    )
    .await;
    assert_eq!(home, SUCCESS_XML);
    let placed = http::dispatch(
        &service,
        &format!("METHOD=setposition&UserID={USER}&RegionID={REGION}"),
    )
    .await;
    assert_eq!(placed, SUCCESS_XML);
    let read = String::from_utf8(
        http::dispatch(&service, &format!("METHOD=getgriduserinfo&UserID={USER}")).await,
    )
    .expect("utf8");
    assert!(read.contains(&format!("<HomeRegionID>{REGION}</HomeRegionID>")));
    assert!(read.contains("&lt;10, 20, 30&gt;"));
    assert!(read.contains("&lt;128, 128, 70&gt;"));

    let gone = http::dispatch(
        &service,
        &format!(
            "METHOD=loggedout&UserID={USER}&RegionID={REGION}&Position=nope&LookAt=%3C1%2C%200%2C%200%3E"
        ),
    )
    .await;
    assert_eq!(gone, SUCCESS_XML);

    let many = String::from_utf8(
        http::dispatch(
            &service,
            &format!(
                "METHOD=getgriduserinfos&AgentIDs[]={USER}&AgentIDs[]=99999999-9999-9999-9999-999999999999"
            ),
        )
        .await,
    )
    .expect("utf8");
    assert!(many.contains("<griduser0 type=\"List\">"));
    assert!(!many.contains("griduser1"));
    assert!(many.contains(&format!("<UserID>{USER}</UserID>")));

    let mut fields = FieldMap::new();
    fields.insert(
        "METHOD".to_string(),
        Field::Str("getgriduserinfos".to_string()),
    );
    fields.insert("AgentIDs".to_string(), Field::List(Vec::new()));
    let empty = http::response_for(&service, &fields).await;
    assert_eq!(
        empty,
        b"<?xml version=\"1.0\"?><ServerResponse><result>null</result></ServerResponse>"
    );

    let missing = String::from_utf8(
        http::dispatch(
            &service,
            "METHOD=getgriduserinfos&AgentIDs[]=99999999-9999-9999-9999-999999999999",
        )
        .await,
    )
    .expect("utf8");
    assert_eq!(missing, "<?xml version=\"1.0\"?><ServerResponse />");
    assert_eq!(
        http::dispatch(&service, "METHOD=getgriduserinfos&AgentIDs=one").await,
        FAILURE_XML
    );
}

#[tokio::test]
async fn console_commands_print_the_stored_row_and_the_five_day_count() {
    let (service, mem, clock) = world();
    assert_eq!(
        service.command(&["show", "grid", "user"]).await.text,
        "Usage: show grid user <UUID>\n"
    );
    service.logged_in(USER).await.expect("login");
    service.logged_in(USER2).await.expect("login two");
    let shown = service.command(&["show", "grid", "user", USER]).await.text;
    assert!(shown.contains(&format!("User ID : {USER}\n")));
    assert!(shown.contains("Online  : True\n"));
    assert!(shown.contains(&format!("Login   : {UNIX}\n")));
    assert!(shown.contains("Entries: 1\n"));
    assert_eq!(
        service
            .command(&["show", "grid", "users", "online"])
            .await
            .text,
        "Users online: 2\n"
    );
    clock.set_unix(UNIX + 5 * 86_400);
    assert_eq!(
        service
            .command(&["show", "grid", "users", "online"])
            .await
            .text,
        "Users online: 0\n"
    );
    clock.set_unix(UNIX + 5 * 86_400 - 1);
    assert_eq!(
        service
            .command(&["show", "grid", "users", "online"])
            .await
            .text,
        "Users online: 2\n"
    );
    mem.store(&row(
        "future",
        &[("Online", "True"), ("Login", "2000000000"), ("Logout", "0")],
    ))
    .await
    .expect("future");
    clock.set_unix(UNIX);
    let counted = service
        .command(&["show", "grid", "users", "online"])
        .await
        .text;
    assert_eq!(counted, "Users online: 3\n");
}

#[tokio::test]
async fn sqlite_login_logout_and_console_use_the_store() {
    let (store, _path) = open_sqlite().await;
    store.wipe().await.expect("wipe");
    let service = GridUserService::new(store, TestClock::new(UNIX));
    let body = http::dispatch(&service, &format!("METHOD=loggedin&UserID={USER}")).await;
    assert!(contains(&body, b"<Online>True</Online>"));
    let text = service.command(&["show", "grid", "user", USER]).await.text;
    assert!(text.contains("HomePosition : <0,0,0>\n"));
    assert!(text.contains("Online       : True\n"));
    assert!(text.contains(&format!("Login        : {UNIX}\n")));
    assert!(text.contains("Logout       : 0\n"));
    assert!(text.ends_with("Entries: 1\n"));
    let logged_out = http::dispatch(
        &service,
        &format!(
            "METHOD=loggedout&UserID={USER}&RegionID={REGION}&Position=%3C1.5%2C%200.25%2C%2010%3E"
        ),
    )
    .await;
    assert_eq!(logged_out, SUCCESS_XML);
    let read = String::from_utf8(
        http::dispatch(&service, &format!("METHOD=getgriduserinfo&UserID={USER}")).await,
    )
    .expect("utf8");
    assert!(read.contains("<Online>False</Online>"));
    assert!(read.contains(&format!("<LastRegionID>{REGION}</LastRegionID>")));
    assert!(read.contains("&lt;1.5, 0.25, 10&gt;"));
}

#[tokio::test]
async fn open_reads_the_grid_user_section_and_refuses_a_bad_one() {
    let missing =
        Config::parse("[GridUserService]\nLocalServiceModule = GridUserService\n").expect("ini");
    let error = GridUserHttp::open(&missing).await.err().expect("missing");
    assert_eq!(error.to_string(), "No StorageProvider configured");

    let bad_module = Config::parse(
        "[GridUserService]\n\
         LocalServiceModule = OtherService\n\
         StorageProvider = OpenSim.Data.SQLite.dll\n\
         ConnectionString = Data Source=:memory:\n",
    )
    .expect("ini");
    let error = GridUserHttp::open(&bad_module).await.err().expect("module");
    assert!(error.to_string().contains("GridUserService"));

    let https = Config::parse(
        "[Network]\nhttps_main = true\n\
         [GridUserService]\n\
         LocalServiceModule = OpenSim.Services.UserAccountService.dll:GridUserService\n\
         StorageProvider = OpenSim.Data.SQLite.dll\n\
         ConnectionString = Data Source=:memory:\n",
    )
    .expect("ini");
    let error = GridUserHttp::open(&https).await.err().expect("https");
    assert!(error.to_string().contains("https_main"));
}

#[tokio::test]
async fn post_answers_xml_and_a_huge_body_is_413() {
    let (store, _path) = open_sqlite().await;
    store.wipe().await.expect("wipe");
    let http = GridUserHttp::new(GridUserService::new(store, SystemClock));
    let app = http.into_router();
    let response = tower::ServiceExt::oneshot(
        app.clone(),
        axum::http::Request::builder()
            .method("POST")
            .uri("/griduser")
            .header("X-SecondLife-Shard", "OpenSim")
            .body(axum::body::Body::from(format!(
                "METHOD=loggedin&UserID={USER}"
            )))
            .expect("request"),
    )
    .await
    .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/xml")
    );
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    assert!(contains(&bytes, b"<Online>True</Online>"));

    let huge = "x".repeat(ferroan_http::MAX_QUERY_BYTES + 1);
    let too_big = tower::ServiceExt::oneshot(
        app,
        axum::http::Request::builder()
            .method("POST")
            .uri("/griduser")
            .body(axum::body::Body::from(huge))
            .expect("request"),
    )
    .await
    .expect("response");
    assert_eq!(too_big.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn service_fixture_matches_the_recorded_sequence() {
    replay_service_fixture(&load_service_fixture()).await;
}

#[tokio::test]
async fn griduser_service_matches_the_pin() {
    let oracle = std::env::var("FERROAN_GRIDUSER_ORACLE").unwrap_or_default();
    if oracle.is_empty() || std::env::var("FERROAN_GRIDUSER_SERVICE").ok().as_deref() != Some("1") {
        eprintln!("FERROAN_GRIDUSER_SERVICE is unset; the grid user service capture was not run");
        return;
    }
    let request = service_request(&load_service_fixture());
    let captured = run_service_oracle(&oracle, &request);
    replay_service_fixture(&captured).await;
}

fn load_service_fixture() -> serde_json::Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/griduser/service.json");
    let text = std::fs::read_to_string(&path).expect("fixture");
    serde_json::from_str(&text).expect("json")
}

fn service_request(doc: &serde_json::Value) -> serde_json::Value {
    let steps = doc["steps"].as_array().expect("steps");
    let steps: Vec<serde_json::Value> = steps
        .iter()
        .map(|step| {
            let mut row = serde_json::Map::new();
            for key in [
                "op",
                "body",
                "time_user",
                "time_field",
                "user_id",
                "data",
                "argv",
            ] {
                if let Some(value) = step.get(key) {
                    row.insert(key.to_string(), value.clone());
                }
            }
            serde_json::Value::Object(row)
        })
        .collect();
    serde_json::json!({ "steps": steps })
}

fn run_service_oracle(oracle: &str, request: &serde_json::Value) -> serde_json::Value {
    let mut child = std::process::Command::new("dotnet")
        .arg(oracle)
        .arg("service")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("dotnet");
    serde_json::to_writer(child.stdin.as_mut().expect("stdin"), request).expect("request");
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "grid user service oracle failed: {}",
        scrub_secret(&String::from_utf8_lossy(&output.stderr))
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    serde_json::from_str(line)
        .unwrap_or_else(|error| panic!("json {error}: {}", scrub_secret(&stdout)))
}

async fn replay_service_fixture(doc: &serde_json::Value) {
    let steps = doc["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 35);
    let spellings = date_spellings(doc);
    let (store, _path) = open_sqlite().await;
    store.wipe().await.expect("wipe");
    let clock = Arc::new(TestClock::new(0));
    let service = GridUserService::new(store, Arc::clone(&clock));
    for (index, step) in steps.iter().enumerate() {
        let unix = i32::try_from(step["unix"].as_i64().expect("unix")).expect("unix fits");
        clock.set_unix(unix);
        match step["op"].as_str().expect("op") {
            "post" => {
                let body = step["body"].as_str().expect("body");
                let got = String::from_utf8(http::dispatch(&service, body).await).expect("utf8");
                let expected = step["response"].as_str().expect("response");
                assert_eq!(
                    fold_dates(&got, &spellings),
                    fold_dates(expected, &spellings),
                    "step {index}"
                );
            }
            "store" => {
                let mut user = GridUser::new(step["user_id"].as_str().expect("user"));
                let data = step["data"].as_object().expect("data");
                for (key, value) in data {
                    user.data
                        .insert(key.clone(), value.as_str().expect("value").to_string());
                }
                let stored = service.data().store(&user).await.expect("store");
                assert_eq!(
                    stored,
                    step["stored"].as_bool().expect("stored"),
                    "step {index}"
                );
            }
            "console" => {
                let argv = step["argv"].as_array().expect("argv");
                let words: Vec<String> = argv
                    .iter()
                    .map(|word| word.as_str().expect("word").to_string())
                    .collect();
                let refs: Vec<&str> = words.iter().map(String::as_str).collect();
                let text = service.command(&refs).await.text;
                assert_eq!(text, step["text"].as_str().expect("text"), "step {index}");
            }
            other => panic!("unknown service step {other}"),
        }
    }
}

fn date_spellings(doc: &serde_json::Value) -> Vec<String> {
    let mut unixes = HashSet::new();
    unixes.insert(0);
    for step in doc["steps"].as_array().expect("steps") {
        if let Some(unix) = step["unix"].as_i64() {
            unixes.insert(i32::try_from(unix).expect("unix fits"));
        }
        if let Some(data) = step.get("data").and_then(serde_json::Value::as_object) {
            for key in ["Login", "Logout"] {
                if let Some(text) = data.get(key).and_then(serde_json::Value::as_str)
                    && let Some(unix) = wire::parse_i32(text)
                {
                    unixes.insert(unix);
                }
            }
        }
    }
    let mut spellings: Vec<String> = unixes
        .into_iter()
        .flat_map(|unix| {
            let en = wire::en_us_general(unix);
            let narrow = narrow_ampm(&en);
            [wire::unix_general(unix), en, narrow]
        })
        .collect();
    spellings.sort_by(|left, right| right.len().cmp(&left.len()).then(right.cmp(left)));
    spellings.dedup();
    spellings
}

fn narrow_ampm(spelling: &str) -> String {
    let Some(at) = spelling.rfind(' ') else {
        return spelling.to_string();
    };
    let mut out = String::new();
    out.push_str(&spelling[..at]);
    out.push('\u{202f}');
    out.push_str(&spelling[at + 1..]);
    out
}

fn fold_dates(text: &str, spellings: &[String]) -> String {
    let mut folded = text.to_string();
    for spelling in spellings {
        folded = folded.replace(spelling, "DATE");
    }
    folded
}

fn scrub_secret(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(at) = lower.find("password") else {
            result.push_str(rest);
            break;
        };
        result.push_str(&rest[..at + "password".len()]);
        rest = &rest[at + "password".len()..];
        let trimmed = rest.trim_start();
        result.push_str(&rest[..rest.len() - trimmed.len()]);
        rest = trimmed;
        if let Some(stripped) = rest.strip_prefix('=') {
            result.push_str("=redacted");
            let end = stripped
                .find(|ch: char| ch == ';' || ch.is_whitespace())
                .unwrap_or(stripped.len());
            rest = &stripped[end..];
        }
    }
    result
}

async fn open_sqlite() -> (GridUserStore, std::path::PathBuf) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ferroan-griduser-service-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp");
    let path = dir.join("GridUserStore.db");
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/db/sqlite/GridUserStore.db");
    std::fs::copy(&source, &path).expect("copy");
    let pool = ferroan_db::open_sqlite_file(&path, &["GridUserStore"])
        .await
        .expect("open");
    (GridUserStore::open(pool).expect("store"), path)
}
