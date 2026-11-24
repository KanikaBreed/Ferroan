// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Store, service and HTTP checks against the AuthStore fixture and, when the
//! CI variables are set, MySQL and PostgreSQL.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use ferroan_config::Config;
use ferroan_http::MAX_QUERY_BYTES;
use tower::ServiceExt;
use uuid::Uuid;

use super::hash::{self, ZERO_KEY};
use super::http::AuthHttp;
use super::model::{AuthKind, AuthRow};
use super::open::open_authorization;
use super::service::AuthService;
use super::store::AuthStore;
use super::{AuthError, authorization_router};

const PRINCIPAL: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const WEB_KEY: &str = "33333333-3333-4333-8333-333333333333";
const SALT: &str = "0123456789abcdef0123456789abcdef";

fn principal() -> Uuid {
    Uuid::parse_str(PRINCIPAL).unwrap()
}

fn scratch_auth() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ferroan-auth-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("AuthStore.db");
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/db/sqlite/AuthStore.db");
    std::fs::copy(&source, &path).unwrap();
    path
}

async fn sqlite_store() -> AuthStore {
    let path = scratch_auth();
    let pool = ferroan_db::open_sqlite_file(&path, &["AuthStore"])
        .await
        .unwrap();
    AuthStore::new(pool, "auth").unwrap()
}

fn password_service(store: AuthStore) -> AuthService {
    AuthService::new(store, AuthKind::Password)
}

fn http(service: AuthService) -> Router {
    AuthHttp::new(service, true, true, true, true).into_router()
}

async fn call(app: Router, uri: &str, body: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), MAX_QUERY_BYTES)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn password_round_trip_matches_the_login_hash() {
    let service = password_service(sqlite_store().await);
    let id = principal();
    assert!(service.set_password(id, "harness-secret").await.unwrap());
    let row = service.get(id).await.unwrap().unwrap();
    let presented = hash::md5_text("harness-secret");
    assert_eq!(
        hash::presented_hash(&presented, &row.password_salt),
        row.password_hash
    );
    let token = service.authenticate(id, &presented, 30).await.unwrap();
    assert!(Uuid::parse_str(&token).is_ok());
    assert!(service.verify(id, &token, 30).await.unwrap());
    assert!(service.release(id, &token).await.unwrap());
    assert!(!service.verify(id, &token, 30).await.unwrap());
    assert!(
        service
            .authenticate(id, "00000000000000000000000000000000", 30)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn set_data_item_writes_one_known_column() {
    let store = sqlite_store().await;
    let id = principal();
    store
        .store(&AuthRow {
            principal_id: id,
            password_hash: hash::stored_hash("harness-secret", SALT),
            password_salt: SALT.to_string(),
            web_login_key: ZERO_KEY.to_string(),
            account_type: "UserAccount".to_string(),
        })
        .await
        .unwrap();
    assert!(
        store
            .set_data_item(id, "webLoginKey", WEB_KEY)
            .await
            .unwrap()
    );
    assert!(!store.set_data_item(id, "not-a-column", "x").await.unwrap());
    let row = store.get(id).await.unwrap().unwrap();
    assert_eq!(row.web_login_key, WEB_KEY);
}

#[tokio::test]
async fn http_plain_methods_and_authorization_match_the_captured_documents() {
    let app = http(password_service(sqlite_store().await));
    let set = format!("METHOD=setpassword&PRINCIPAL={PRINCIPAL}&PASSWORD=harness-secret");
    let (status, body) = call(app.clone(), "/auth/plain", &set).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<Result>Success</Result>"));
    assert!(!body.contains("harness-secret"));
    let stored = hash::stored_hash("harness-secret", SALT);
    let info = format!(
        "METHOD=setauthinfo&PRINCIPAL={PRINCIPAL}&AccountType=UserAccount&PasswordHash={stored}&PasswordSalt={SALT}&WebLoginKey={WEB_KEY}"
    );
    let (_, body) = call(app.clone(), "/auth/plain", &info).await;
    assert!(body.contains("<Result>Success</Result>"));
    let (_, got) = call(
        app.clone(),
        "/auth/plain",
        &format!("METHOD=getauthinfo&PRINCIPAL={PRINCIPAL}"),
    )
    .await;
    assert!(got.contains(&format!("<PrincipalID>{PRINCIPAL}</PrincipalID>")));
    assert!(got.contains(&format!("<PasswordHash>{stored}</PasswordHash>")));
    assert!(got.contains(&format!("<PasswordSalt>{SALT}</PasswordSalt>")));
    assert!(got.contains(&format!("<WebLoginKey>{WEB_KEY}</WebLoginKey>")));
    assert!(got.find("<PrincipalID>").unwrap() < got.find("<AccountType>").unwrap());
    assert!(got.find("<AccountType>").unwrap() < got.find("<PasswordHash>").unwrap());
    let presented = hash::md5_text("harness-secret");
    let (_, authed) = call(
        app.clone(),
        "/auth/plain",
        &format!("METHOD=authenticate&PRINCIPAL={PRINCIPAL}&PASSWORD={presented}"),
    )
    .await;
    assert!(authed.contains("<Token>"));
    let (status, _) = call(
        app.clone(),
        "/auth/plain",
        &format!("METHOD=authenticate&PRINCIPAL={PRINCIPAL}&PASSWORD={presented}&LIFETIME=nope"),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let (status, body) = call(app.clone(), "/auth/crypt", "ignored").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
    let (status, body) = call(app.clone(), "/auth", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/plain")
                .header("X-SecondLife-Shard", "OpenSim")
                .body(Body::from(format!(
                    "METHOD=authenticate&PRINCIPAL={PRINCIPAL}&PASSWORD=00000000000000000000000000000000"
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain")
    );
    let bytes = axum::body::to_bytes(response.into_body(), MAX_QUERY_BYTES)
        .await
        .unwrap();
    assert!(bytes.is_empty());

    let authz = authorization_router();
    let request = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<AuthorizationRequest xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n  <ID>11111111-1111-1111-1111-111111111111</ID>\n  <FirstName>Test</FirstName>\n  <SurName>User</SurName>\n  <Email>a@b.invalid</Email>\n  <RegionName>Region</RegionName>\n  <RegionID>22222222-2222-2222-2222-222222222222</RegionID>\n</AuthorizationRequest>";
    let (status, body) = call(authz.clone(), "/authorization", request).await;
    assert_eq!(status, StatusCode::OK);
    let shard = authz
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/authorization")
                .header("X-SecondLife-Shard", "OpenSim")
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(shard.status(), StatusCode::OK);
    assert!(body.starts_with('\u{feff}'));
    assert!(
        body.contains(
            "<Message>11111111-1111-1111-1111-111111111111 has been authorized</Message>"
        )
    );
}

#[tokio::test]
async fn webkey_consumes_the_key_and_or_password_accepts_the_hash() {
    let id = principal();
    let salt = SALT.to_string();
    let password_hash = hash::stored_hash("harness-secret", &salt);
    let row = AuthRow {
        principal_id: id,
        password_hash,
        password_salt: salt,
        web_login_key: WEB_KEY.to_string(),
        account_type: "UserAccount".to_string(),
    };
    let web_store = sqlite_store().await;
    web_store.store(&row).await.unwrap();
    let web = AuthService::new(web_store, AuthKind::Webkey);
    assert!(web.authenticate(id, "short", 30).await.unwrap().is_empty());
    assert!(web.authenticate(id, ZERO_KEY, 30).await.unwrap().is_empty());
    let token = web.authenticate(id, WEB_KEY, 30).await.unwrap();
    assert!(Uuid::parse_str(&token).is_ok());
    assert!(web.authenticate(id, WEB_KEY, 30).await.unwrap().is_empty());
    assert_eq!(web.get(id).await.unwrap().unwrap().web_login_key, ZERO_KEY);

    let both_store = sqlite_store().await;
    both_store.store(&row).await.unwrap();
    let both = AuthService::new(both_store, AuthKind::WebkeyOrPassword);
    let presented = hash::md5_text("harness-secret");
    let token = both.authenticate(id, &presented, 30).await.unwrap();
    assert!(Uuid::parse_str(&token).is_ok());
    assert_eq!(both.get(id).await.unwrap().unwrap().web_login_key, WEB_KEY);
}

#[test]
fn module_names_match_in_the_pin_order() {
    assert_eq!(
        AuthKind::from_module(
            "OpenSim.Services.AuthenticationService.dll:WebkeyOrPasswordAuthenticationService"
        ),
        Some(AuthKind::WebkeyOrPassword)
    );
    assert_eq!(
        AuthKind::from_module("WebkeyAuthenticationService"),
        Some(AuthKind::Webkey)
    );
    assert_eq!(
        AuthKind::from_module(
            "OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService"
        ),
        Some(AuthKind::Password)
    );
    assert_eq!(AuthKind::from_module("NoSuchService"), None);
}

#[tokio::test]
async fn open_reads_the_fixture_and_authorization_opens_no_database() {
    let path = scratch_auth();
    let ini = format!(
        "[AuthenticationService]\n    \
         LocalServiceModule = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source={};Version=3;\"\n    \
         AllowSetPassword = true\n",
        path.display()
    );
    let config = Config::parse(&ini).unwrap();
    let http = AuthHttp::open(&config).await.unwrap();
    assert_eq!(http.pool_occupancy().1, http.pool_occupancy().1);
    let app = http.into_router();
    let (status, _) = call(
        app,
        "/auth/plain",
        &format!("METHOD=setpassword&PRINCIPAL={PRINCIPAL}&PASSWORD=harness-secret"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let denied = Config::parse(
        "[Network]\n    AuthType = BasicHttpAuthentication\n\n\
         [AuthenticationService]\n    \
         LocalServiceModule = \"OpenSim.Services.AuthenticationService.dll:PasswordAuthenticationService\"\n    \
         StorageProvider = \"OpenSim.Data.SQLite.dll\"\n    \
         ConnectionString = \"Data Source=/tmp/unused.db;Version=3;\"\n",
    )
    .unwrap();
    assert!(matches!(
        AuthHttp::open(&denied).await,
        Err(AuthError::Message(_))
    ));

    let authorization = Config::parse(
        "[AuthorizationService]\n    \
         LocalServiceModule = \"OpenSim.Services.AuthorizationService.dll:AuthorizationService\"\n",
    )
    .unwrap();
    open_authorization(&authorization).unwrap();
    let missing = Config::parse("[AuthorizationService]\n").unwrap();
    assert!(open_authorization(&missing).is_err());
}

#[tokio::test]
async fn mysql_and_postgres_round_trip_when_configured() {
    if let Ok(url) = std::env::var("FERROAN_MYSQL_URL") {
        round_trip_sqlx(&url, false).await;
    }
    if let Ok(url) = std::env::var("FERROAN_POSTGRES_URL") {
        round_trip_sqlx(&url, true).await;
    }
}

async fn round_trip_sqlx(url: &str, postgres: bool) {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = LOCK.lock().await;
    let pool = ferroan_db::Pool::connect_sqlx_url(url).await.unwrap();
    if postgres {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS auth (
                uuid uuid NOT NULL,
                \"passwordHash\" varchar(32) NOT NULL,
                \"passwordSalt\" varchar(32) NOT NULL,
                \"webLoginKey\" varchar(255) NOT NULL,
                \"accountType\" varchar(32) NOT NULL DEFAULT 'UserAccount'
            )",
        )
        .execute(pool.postgres().unwrap())
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tokens (
                uuid uuid NOT NULL,
                token varchar(255) NOT NULL,
                validity timestamp NOT NULL
            )",
        )
        .execute(pool.postgres().unwrap())
        .await
        .unwrap();
        let _ = sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS auth_pkey ON auth (uuid)")
            .execute(pool.postgres().unwrap())
            .await;
        let _ = sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS uuid_token ON tokens (uuid, token)")
            .execute(pool.postgres().unwrap())
            .await;
        sqlx::query("DELETE FROM tokens WHERE uuid = $1 OR uuid = $2")
            .bind(principal())
            .bind(Uuid::parse_str(OTHER).unwrap())
            .execute(pool.postgres().unwrap())
            .await
            .unwrap();
        sqlx::query("DELETE FROM auth WHERE uuid = $1 OR uuid = $2")
            .bind(principal())
            .bind(Uuid::parse_str(OTHER).unwrap())
            .execute(pool.postgres().unwrap())
            .await
            .unwrap();
    } else {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS `auth` (
                `UUID` char(36) NOT NULL,
                `passwordHash` char(32) NOT NULL DEFAULT '',
                `passwordSalt` char(32) NOT NULL DEFAULT '',
                `webLoginKey` varchar(255) NOT NULL DEFAULT '',
                `accountType` varchar(32) NOT NULL DEFAULT 'UserAccount',
                PRIMARY KEY (`UUID`)
            ) ENGINE=InnoDB DEFAULT CHARSET=utf8",
        )
        .execute(pool.mysql().unwrap())
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS `tokens` (
                `UUID` char(36) NOT NULL,
                `token` varchar(255) NOT NULL,
                `validity` datetime NOT NULL,
                UNIQUE KEY `uuid_token` (`UUID`, `token`)
            ) ENGINE=InnoDB DEFAULT CHARSET=utf8",
        )
        .execute(pool.mysql().unwrap())
        .await
        .unwrap();
        sqlx::query("DELETE FROM tokens WHERE UUID = ? OR UUID = ?")
            .bind(PRINCIPAL)
            .bind(OTHER)
            .execute(pool.mysql().unwrap())
            .await
            .unwrap();
        sqlx::query("DELETE FROM auth WHERE UUID = ? OR UUID = ?")
            .bind(PRINCIPAL)
            .bind(OTHER)
            .execute(pool.mysql().unwrap())
            .await
            .unwrap();
    }
    let store = AuthStore::new(pool, "auth").unwrap();
    let service = AuthService::new(store, AuthKind::Password);
    let id = principal();
    assert!(service.set_password(id, "harness-secret").await.unwrap());
    let row = service.get(id).await.unwrap().unwrap();
    let presented = hash::md5_text("harness-secret");
    assert_eq!(
        hash::presented_hash(&presented, &row.password_salt),
        row.password_hash
    );
    let token = service.authenticate(id, &presented, 30).await.unwrap();
    assert!(service.verify(id, &token, 30).await.unwrap());
    assert!(service.release(id, &token).await.unwrap());
}
