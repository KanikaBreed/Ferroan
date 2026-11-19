// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! User-account and user-alias routes against a copy of the UserAccount fixture.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ferroan_db::open_sqlite_file;
use ferroan_services::account::{
    AccountHttp, AccountService, AccountStore, FixedClock, NewAccount,
};
use ferroan_services::alias::{AliasHttp, AliasService, AliasStore};
use tower::ServiceExt;
use uuid::Uuid;

static NEXT: AtomicU64 = AtomicU64::new(0);

const PRINCIPAL: &str = "11111111-1111-4111-8111-111111111111";
const FAILURE: &str =
    "<?xml version =\"1.0\"?><ServerResponse><result>Failure</result></ServerResponse>";

#[tokio::test]
async fn god_account_and_create_round_trip() {
    let app = account_router(true, true).await;
    let god = call(&app, "METHOD=getaccount&FirstName=GRID&LastName=SERVICES").await;
    assert_eq!(god.status, StatusCode::OK);
    assert_eq!(god.content_type, "application/xml");
    assert_eq!(
        god.body,
        "<?xml version=\"1.0\"?><ServerResponse><result type=\"List\"><FirstName>GRID</FirstName><LastName>SERVICES</LastName><Email></Email><PrincipalID>6571e388-6218-4574-87db-f9379718315e</PrincipalID><ScopeID>00000000-0000-0000-0000-000000000000</ScopeID><Created>1700000000</Created><UserLevel>240</UserLevel><UserFlags>0</UserFlags><UserTitle></UserTitle><UserCountry></UserCountry><LocalToGrid>True</LocalToGrid><DisplayName></DisplayName><NameChanged>0</NameChanged><ServiceURLs></ServiceURLs></result></ServerResponse>"
    );

    let created = call(
        &app,
        &format!(
            "METHOD=createuser&FirstName=Test&LastName=User&Password=harness-password&Email=test@example.com&PrincipalID={PRINCIPAL}"
        ),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK);
    assert!(!created.body.contains("harness-password"));
    assert_eq!(
        created.body,
        format!(
            "<?xml version=\"1.0\"?><ServerResponse><result type=\"List\"><FirstName>Test</FirstName><LastName>User</LastName><Email>test@example.com</Email><PrincipalID>{PRINCIPAL}</PrincipalID><ScopeID>00000000-0000-0000-0000-000000000000</ScopeID><Created>1700000000</Created><UserLevel>0</UserLevel><UserFlags>0</UserFlags><LocalToGrid>True</LocalToGrid><NameChanged>0</NameChanged><ServiceURLs>HomeURI*;InventoryServerURI*;AssetServerURI*;</ServiceURLs></result></ServerResponse>"
        )
    );

    let loaded = call(&app, &format!("METHOD=getaccount&UserID={PRINCIPAL}")).await;
    assert!(loaded.body.contains("<UserTitle></UserTitle>"));
    assert!(loaded.body.contains("<DisplayName></DisplayName>"));
    assert!(
        loaded
            .body
            .contains("HomeURI*;InventoryServerURI*;AssetServerURI*;")
    );

    let duplicate = call(
        &app,
        "METHOD=createuser&FirstName=Test&LastName=User&Password=harness-password&Email=other@example.com",
    )
    .await;
    assert_eq!(duplicate.body, FAILURE);

    let missing = call(&app, "METHOD=getaccount&FirstName=Missing&LastName=Person").await;
    assert_eq!(
        missing.body,
        "<?xml version=\"1.0\"?><ServerResponse><result>null</result></ServerResponse>"
    );
    let bad_scope = call(
        &app,
        "METHOD=getaccount&ScopeID=not-a-uuid&FirstName=Test&LastName=User",
    )
    .await;
    assert_eq!(bad_scope.body, missing.body);
    assert_eq!(call(&app, "METHOD=nope").await.body, FAILURE);
}

#[tokio::test]
async fn flags_search_and_sqlite_multi_get() {
    let app = account_router(true, true).await;
    assert_eq!(
        call(&app, "METHOD=createuser&FirstName=Only").await.body,
        FAILURE
    );
    call(
        &app,
        &format!(
            "METHOD=createuser&FirstName=Ada&LastName=Lovelace&Password=harness-password&Email=ada@example.com&PrincipalID={PRINCIPAL}"
        ),
    )
    .await;
    let one = call(&app, "METHOD=getaccounts&query=ada").await;
    assert!(one.body.contains("account0"));
    assert!(one.body.contains("<FirstName>Ada</FirstName>"));
    let either = call(&app, "METHOD=getaccounts&query=Ada%20Zzz").await;
    assert!(either.body.contains("<LastName>Lovelace</LastName>"));
    let skipped = call(&app, "METHOD=getaccounts&query=ab%20cd%20Lovelace").await;
    assert!(skipped.body.contains("account0"));
    let too_many = call(&app, "METHOD=getaccounts&query=Aaa%20Bbb%20Ccc").await;
    assert!(too_many.body.contains("<result>null</result>"));
    let exact = call(&app, "METHOD=getaccount&FirstName=ada&LastName=Lovelace").await;
    assert!(exact.body.contains("<result>null</result>"));
    let multi = call(&app, &format!("METHOD=getmultiaccounts&IDS[]={PRINCIPAL}")).await;
    assert!(multi.body.contains("<result>null</result>"));
    assert_eq!(
        call(&app, &format!("METHOD=getmultiaccounts&IDS={PRINCIPAL}"))
            .await
            .body,
        FAILURE
    );

    let renamed = call(
        &app,
        &format!("METHOD=setdisplayname&PrincipalID={PRINCIPAL}&DisplayName=Shown"),
    )
    .await;
    assert_eq!(
        renamed.body,
        "<?xml version=\"1.0\"?><ServerResponse><result>success</result></ServerResponse>"
    );
    let after = call(&app, &format!("METHOD=getaccount&PrincipalID={PRINCIPAL}")).await;
    assert!(after.body.contains("<DisplayName>Shown</DisplayName>"));
    assert!(after.body.contains("<NameChanged>1700000000</NameChanged>"));

    let updated = call(
        &app,
        &format!("METHOD=setaccount&PrincipalID={PRINCIPAL}&Email=next@example.com&UserLevel=3"),
    )
    .await;
    assert!(updated.body.contains("<Email>next@example.com</Email>"));
    assert!(updated.body.contains("<UserLevel>3</UserLevel>"));
}

#[tokio::test]
async fn create_and_set_are_off_until_the_ini_says_so() {
    let app = account_router(false, false).await;
    assert_eq!(
        call(
            &app,
            "METHOD=createuser&FirstName=Test&LastName=User&Password=harness-password"
        )
        .await
        .body,
        FAILURE
    );
    assert_eq!(
        call(
            &app,
            "METHOD=setaccount&PrincipalID=6571e388-6218-4574-87db-f9379718315e&UserLevel=1"
        )
        .await
        .body,
        FAILURE
    );
}

#[tokio::test]
async fn shard_header_is_forbidden() {
    let app = account_router(true, true).await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/accounts")
                .header("X-SecondLife-Shard", "OpenSim")
                .body(Body::from(
                    "METHOD=getaccount&FirstName=GRID&LastName=SERVICES",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/plain"
    );
}

#[tokio::test]
async fn commands_change_the_row_and_do_not_keep_a_password() {
    let (service, _dir) = open_accounts().await;
    let created = service
        .command(
            &[
                "create",
                "user",
                "Pat",
                "Example",
                "harness-password",
                "pat@example.com",
                PRINCIPAL,
                "",
            ],
            &std::env::temp_dir(),
        )
        .await;
    assert!(created.error.is_none(), "{:?}", created.error);
    let shown = service
        .command(
            &["show", "account", "Pat", "Example"],
            &std::env::temp_dir(),
        )
        .await;
    assert!(shown.lines[0].contains("Pat Example"));
    assert_eq!(shown.lines[4], "Created: 11/14/2023 22:13:20");
    let reset = service
        .command(
            &[
                "reset",
                "user",
                "password",
                "Pat",
                "Example",
                "harness-password",
            ],
            &std::env::temp_dir(),
        )
        .await;
    assert_eq!(
        reset.lines,
        vec!["Unable to reset password for account Pat Example.".to_string()]
    );
    let level = service
        .command(
            &["set", "user", "level", "Pat", "Example", "7"],
            &std::env::temp_dir(),
        )
        .await;
    assert_eq!(
        level.lines,
        vec!["User level set for user Pat Example to 7".to_string()]
    );

    let root = scratch_dir();
    std::fs::create_dir_all(root.join("Imports")).unwrap();
    std::fs::write(
        root.join("Imports/users.csv"),
        format!(
            "short\n{PRINCIPAL},Imp,User,harness-password,imp@example.com\n{PRINCIPAL},Imp,User,harness-password,imp@example.com,1600000000\n"
        ),
    )
    .unwrap();
    let imported = service.command(&["import", "users"], &root).await;
    assert_eq!(
        imported.error.as_deref(),
        Some("import stopped: rezday is missing")
    );
    assert!(imported.lines[0].contains("System.String[]"));
    std::fs::write(
        root.join("Imports/users.csv"),
        "22222222-2222-4222-8222-222222222222,Imp,User,harness-password,imp@example.com,1600000000\n",
    )
    .unwrap();
    let imported = service
        .command(&["import", "users", "ignored.csv"], &root)
        .await;
    assert!(imported.error.is_none(), "{:?}", imported.error);
    let account = service
        .get_by_name(Uuid::nil(), "Imp", "User")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.created, 1_600_000_000);
    assert!(!format!("{account:?}").contains("harness-password"));
}

#[tokio::test]
async fn replace_resets_active_and_country_fails() {
    let (service, path) = open_accounts().await;
    service
        .create_user(
            Uuid::nil(),
            NewAccount {
                principal_id: Uuid::parse_str(PRINCIPAL).unwrap(),
                first_name: "Ada".to_string(),
                last_name: "Lovelace".to_string(),
                password: "harness-password".to_string(),
                email: "ada@example.com".to_string(),
                model: String::new(),
            },
        )
        .await
        .unwrap();
    let pool = open_sqlite_file(&path, &["UserAccount"]).await.unwrap();
    let sqlite = pool.sqlite().unwrap();
    sqlx::query("UPDATE UserAccounts SET active = 0 WHERE PrincipalID = ?")
        .bind(PRINCIPAL)
        .execute(sqlite)
        .await
        .unwrap();
    let mut account = service
        .get_by_principal(Uuid::nil(), Uuid::parse_str(PRINCIPAL).unwrap())
        .await
        .unwrap()
        .unwrap();
    account.email = "next@example.com".to_string();
    assert!(service.store_user(&account).await.unwrap());
    let active: i64 = sqlx::query_scalar("SELECT active FROM UserAccounts WHERE PrincipalID = ?")
        .bind(PRINCIPAL)
        .fetch_one(sqlite)
        .await
        .unwrap();
    assert_eq!(active, 1);
    account.user_country = Some("XX".to_string());
    assert!(!service.store_user(&account).await.unwrap());
}

#[tokio::test]
async fn alias_routes_round_trip() {
    let (app, _dir) = alias_router().await;
    let alias = "44444444-4444-4444-8444-444444444444";
    let missing = call_at(
        &app,
        "/useralias",
        &format!("METHOD=getuserforalias&AliasID={alias}"),
    )
    .await;
    assert!(missing.body.contains("<result>null</result>"));
    assert_eq!(
        call_at(&app, "/useralias", "METHOD=nope").await.body,
        FAILURE
    );
}

#[tokio::test]
async fn alias_command_stores_and_deletes() {
    let (service, _dir) = open_alias().await;
    let user = "33333333-3333-4333-8333-333333333333";
    let alias = "44444444-4444-4444-8444-444444444444";
    let created = service
        .command(&["create", "alias", user, alias, "home"])
        .await;
    assert!(created.error.is_none(), "{:?}", created.error);
    assert!(created.lines[0].contains(alias));
    let shown = service.command(&["show", "alias", user]).await;
    assert!(shown.lines[0].contains("home"));
    let deleted = service.command(&["delete", "alias", alias]).await;
    assert_eq!(deleted.lines, vec![format!("Alias for ID {alias} deleted")]);
    let gone = service.command(&["show", "alias", user]).await;
    assert!(gone.lines[0].contains("No aliases"));
}

struct Answer {
    status: StatusCode,
    content_type: String,
    body: String,
}

async fn call(app: &Router, body: &str) -> Answer {
    call_at(app, "/accounts", body).await
}

async fn call_at(app: &Router, path: &str, body: &str) -> Answer {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .unwrap();
    Answer {
        status,
        content_type,
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

async fn account_router(allow_create: bool, allow_set: bool) -> Router {
    let (service, _dir) = open_accounts().await;
    AccountHttp::new(service, allow_create, allow_set, true).into_router()
}

async fn open_accounts() -> (AccountService, PathBuf) {
    let path = scratch_db("UserAccount.db");
    let pool = open_sqlite_file(&path, &["UserAccount"]).await.unwrap();
    let store = AccountStore::new(pool, "UserAccounts").unwrap();
    let service = AccountService::open(
        store,
        Arc::new(FixedClock {
            unix_time: 1_700_000_000,
        }),
    )
    .await
    .unwrap();
    (service, path)
}

async fn alias_router() -> (Router, PathBuf) {
    let (service, path) = open_alias().await;
    (AliasHttp::new(service, true).into_router(), path)
}

async fn open_alias() -> (AliasService, PathBuf) {
    let path = scratch_db("UserAccount.db");
    let pool = ferroan_db::Pool::connect(
        ferroan_db::Backend::Sqlite,
        &format!("Data Source={};Version=3", path.display()),
    )
    .await
    .unwrap();
    AliasStore::create_table(&pool, "UserAlias").await.unwrap();
    let store = AliasStore::new(pool, "UserAlias").await.unwrap();
    (AliasService::new(store), path)
}

fn scratch_db(name: &str) -> PathBuf {
    let dir = scratch_dir();
    let path = dir.join(name);
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/db/sqlite/UserAccount.db");
    std::fs::copy(&source, &path).unwrap();
    path
}

fn scratch_dir() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ferroan-account-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
