// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pin8 reads Presence and password-token rows Ferroan wrote, on a non-UTC server.
//!
//! `cargo xtask session-zone` sets the environment and starts the servers.
//! The MySQL Presence store itself remains Q26c2: this test writes `LastSeen`
//! with the same `UPDATE ... NOW()` as `MySQLPresenceData.ReportAgent`.

use std::process::Command;

use chrono::NaiveDateTime;
use ferroan_db::{Backend, Pool};
use uuid::Uuid;

use super::store::AuthStore;
use crate::presence::{PgPresenceData, PresenceStore};

const LIFE_MINUTES: i32 = 30;

struct ZoneEnv {
    dll: std::ffi::OsString,
    ado: String,
    engine: String,
    name: String,
    offset: i64,
}

#[tokio::test]
async fn session_zone_csharp_reads_ferroan_rows() {
    if std::env::var("FERROAN_ZONE_CATALOG_ONLY").is_ok() {
        let ado = required("FERROAN_ZONE_CATALOG_ADO");
        assert_postgres_zone(&ado, "UTC", 0).await;
        println!("session zone: a login without SELECT on pg_db_role_setting still connects");
        return;
    }
    let Some(dll) = std::env::var_os("FERROAN_ZONE_ORACLE") else {
        eprintln!(
            "FERROAN_ZONE_ORACLE is unset; run cargo xtask session-zone for the Pin8 comparison"
        );
        return;
    };
    let env = ZoneEnv {
        dll,
        ado: required("FERROAN_ZONE_ADO"),
        engine: required("FERROAN_ZONE_ENGINE"),
        name: required("FERROAN_ZONE_NAME"),
        offset: required("FERROAN_ZONE_OFFSET")
            .parse()
            .unwrap_or_else(|_| panic!("FERROAN_ZONE_OFFSET is not an integer")),
    };
    let backend = match env.engine.as_str() {
        "mysql" => Backend::Mysql,
        "postgres" => Backend::Postgres,
        other => panic!("FERROAN_ZONE_ENGINE must be mysql or postgres, not {other}"),
    };
    let db = connect(backend, &env.ado).await;
    assert_session(&db, &env).await;
    let pg = db.postgres().cloned();
    let my = db.mysql().cloned();
    oracle(&env, &["migrate"]);
    let auth = AuthStore::new(db, "auth").unwrap();
    auth_both_ways(&auth, &env, pg.as_ref(), my.as_ref()).await;
    presence_both_ways(&env, pg.as_ref(), my.as_ref()).await;
    if let Ok(ado) = std::env::var("FERROAN_ZONE_UNPRIV_ADO")
        && !ado.is_empty()
    {
        assert_postgres_zone(&ado, "UTC", 0).await;
        println!("session zone: an unprivileged PostgreSQL login stays on UTC");
    }
    if let Ok(ado) = std::env::var("FERROAN_ZONE_UNPRIV_KEY_ADO")
        && !ado.is_empty()
    {
        assert_postgres_zone(&ado, &env.name, env.offset).await;
        println!(
            "session zone: Timezone= selects {} for that login",
            env.name
        );
    }
    if let Ok(ado) = std::env::var("FERROAN_ZONE_BAD_ADO")
        && !ado.is_empty()
    {
        refuse_bad_timezone(&ado).await;
    }
    println!(
        "session zone: {} {} offset {}: Pin8 read Ferroan's LastSeen and token",
        env.engine, env.name, env.offset
    );
}

fn required(name: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value,
        _ => panic!("{name} is unset"),
    }
}

async fn connect(backend: Backend, ado: &str) -> Pool {
    match Pool::connect(backend, ado).await {
        Ok(pool) => pool,
        Err(error) => panic!("connect failed: {}", public_error(&error.to_string())),
    }
}

async fn assert_session(db: &Pool, env: &ZoneEnv) {
    if let Some(pool) = db.postgres() {
        let name: String = sqlx::query_scalar("SELECT current_setting('TimeZone')")
            .fetch_one(pool)
            .await
            .unwrap();
        let offset: i32 = sqlx::query_scalar("SELECT (EXTRACT(TIMEZONE FROM now()) / 60)::integer")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(name, env.name);
        assert_eq!(i64::from(offset), env.offset);
        assert_ne!(name, "UTC");
    } else if let Some(pool) = db.mysql() {
        let name: String = sqlx::query_scalar("SELECT @@session.time_zone")
            .fetch_one(pool)
            .await
            .unwrap();
        let global: String = sqlx::query_scalar("SELECT @@global.time_zone")
            .fetch_one(pool)
            .await
            .unwrap();
        let offset: i64 =
            sqlx::query_scalar("SELECT TIMESTAMPDIFF(MINUTE, UTC_TIMESTAMP(), NOW())")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(name, global);
        assert_eq!(name, env.name);
        assert_eq!(offset, env.offset);
        assert_ne!(name, "+00:00");
    } else {
        panic!("session zone pool is neither MySQL nor PostgreSQL");
    }
}

async fn assert_postgres_zone(ado: &str, name: &str, offset: i64) {
    let db = connect(Backend::Postgres, ado).await;
    let pool = db.postgres().unwrap();
    let found: String = sqlx::query_scalar("SELECT current_setting('TimeZone')")
        .fetch_one(pool)
        .await
        .unwrap();
    let found_offset: i32 =
        sqlx::query_scalar("SELECT (EXTRACT(TIMEZONE FROM now()) / 60)::integer")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(found, name);
    assert_eq!(i64::from(found_offset), offset);
    db.close().await;
}

async fn refuse_bad_timezone(ado: &str) {
    match Pool::connect(Backend::Postgres, ado).await {
        Ok(pool) => {
            pool.close().await;
            panic!("invalid Timezone connected");
        }
        Err(error) => {
            let text = error.to_string();
            assert!(
                !text.to_ascii_lowercase().contains("password"),
                "connect error included a secret"
            );
            assert!(
                !text.contains("127.0.0.1"),
                "connect error included the target"
            );
            let lower = text.to_ascii_lowercase();
            assert!(
                lower.contains("time zone")
                    || lower.contains("timezone")
                    || lower.contains("invalid"),
                "invalid Timezone was not reported: {}",
                public_error(&text)
            );
            println!("session zone: invalid Timezone is refused without the connection string");
        }
    }
}

async fn auth_both_ways(
    auth: &AuthStore,
    env: &ZoneEnv,
    pg: Option<&sqlx::PgPool>,
    my: Option<&sqlx::MySqlPool>,
) {
    let principal = Uuid::new_v4();
    let token = format!("zone-{}", Uuid::new_v4().simple());
    assert!(
        auth.set_token(principal, &token, LIFE_MINUTES)
            .await
            .unwrap()
    );
    let (validity, local_due, utc_due) = token_clocks(env, pg, my, principal, &token).await;
    close_stamps("Ferroan token", &validity, &local_due);
    away_from_utc(&validity, &utc_due, env.offset);
    let read = oracle(env, &["auth-read", &principal.to_string(), &token]);
    close_stamps(
        "Pin8 read of Ferroan's token",
        &field(&read, "validity"),
        &local_due,
    );
    let check = oracle(
        env,
        &[
            "auth-check",
            &principal.to_string(),
            &token,
            &LIFE_MINUTES.to_string(),
        ],
    );
    assert_eq!(field(&check, "check"), "true");

    let principal = Uuid::new_v4();
    let token = format!("zone-{}", Uuid::new_v4().simple());
    let set = oracle(
        env,
        &[
            "auth-set",
            &principal.to_string(),
            &token,
            &LIFE_MINUTES.to_string(),
        ],
    );
    assert_eq!(field(&set, "set"), "true");
    let (validity, local_due, utc_due) = token_clocks(env, pg, my, principal, &token).await;
    close_stamps("Pin8 token", &validity, &local_due);
    away_from_utc(&validity, &utc_due, env.offset);
    assert!(
        auth.check_token(principal, &token, LIFE_MINUTES)
            .await
            .unwrap()
    );
}

async fn token_clocks(
    env: &ZoneEnv,
    pg: Option<&sqlx::PgPool>,
    my: Option<&sqlx::MySqlPool>,
    principal: Uuid,
    token: &str,
) -> (String, String, String) {
    if env.engine == "mysql" {
        let pool = my.unwrap();
        let row: (String, String, String) = sqlx::query_as(
            "SELECT DATE_FORMAT(validity, '%Y-%m-%d %H:%i:%s'), \
             DATE_FORMAT(DATE_ADD(NOW(), INTERVAL 30 MINUTE), '%Y-%m-%d %H:%i:%s'), \
             DATE_FORMAT(DATE_ADD(UTC_TIMESTAMP(), INTERVAL 30 MINUTE), '%Y-%m-%d %H:%i:%s') \
             FROM tokens WHERE UUID = ? AND token = ?",
        )
        .bind(principal.to_string())
        .bind(token)
        .fetch_one(pool)
        .await
        .unwrap();
        row
    } else {
        let pool = pg.unwrap();
        let row: (String, String, String) = sqlx::query_as(
            "SELECT to_char(validity, 'YYYY-MM-DD HH24:MI:SS'), \
             to_char(localtimestamp + interval '30 minutes', 'YYYY-MM-DD HH24:MI:SS'), \
             to_char((current_timestamp AT TIME ZONE 'UTC') + interval '30 minutes', \
                     'YYYY-MM-DD HH24:MI:SS') \
             FROM tokens WHERE uuid = $1 AND token = $2",
        )
        .bind(principal)
        .bind(token)
        .fetch_one(pool)
        .await
        .unwrap();
        row
    }
}

async fn presence_both_ways(
    env: &ZoneEnv,
    pg: Option<&sqlx::PgPool>,
    my: Option<&sqlx::MySqlPool>,
) {
    let user = Uuid::new_v4();
    let session = Uuid::new_v4();
    let home = Uuid::new_v4();
    let away = Uuid::new_v4();
    let secure = Uuid::new_v4();
    seed(env, user, session, home, secure);
    if let Some(pool) = pg {
        let store = PgPresenceData::open(pool.clone()).unwrap();
        assert!(store.report_agent(session, away).await.unwrap());
    } else {
        let changed = sqlx::query(
            "UPDATE `Presence` SET `RegionID` = ?, `LastSeen` = NOW() WHERE `SessionID` = ?",
        )
        .bind(away.to_string())
        .bind(session.to_string())
        .execute(my.unwrap())
        .await
        .unwrap()
        .rows_affected();
        assert_eq!(changed, 1);
    }
    let got = oracle(env, &["presence-get", &session.to_string()]);
    let (local, utc) = session_clocks(env, pg, my).await;
    close_stamps(
        "Pin8 read of Ferroan's LastSeen",
        &field(&got, "lastseen"),
        &local,
    );
    away_from_utc(&field(&got, "lastseen"), &utc, env.offset);
    assert_eq!(field(&got, "plugin"), "1");

    let session = Uuid::new_v4();
    let secure = Uuid::new_v4();
    seed(env, user, session, home, secure);
    let report = oracle(
        env,
        &["presence-report", &session.to_string(), &away.to_string()],
    );
    assert_eq!(field(&report, "report"), "true");
    let stored = stored_last_seen(env, pg, my, session).await;
    let (local, utc) = session_clocks(env, pg, my).await;
    close_stamps("Ferroan read of Pin8 LastSeen", &stored, &local);
    away_from_utc(&stored, &utc, env.offset);
}

fn seed(env: &ZoneEnv, user: Uuid, session: Uuid, region: Uuid, secure: Uuid) {
    let seeded = oracle(
        env,
        &[
            "presence-seed",
            &user.to_string(),
            &session.to_string(),
            &region.to_string(),
            &secure.to_string(),
        ],
    );
    assert_eq!(field(&seeded, "seed"), "true");
}

async fn session_clocks(
    env: &ZoneEnv,
    pg: Option<&sqlx::PgPool>,
    my: Option<&sqlx::MySqlPool>,
) -> (String, String) {
    if env.engine == "mysql" {
        let row: (String, String) = sqlx::query_as(
            "SELECT DATE_FORMAT(NOW(), '%Y-%m-%d %H:%i:%s'), \
             DATE_FORMAT(UTC_TIMESTAMP(), '%Y-%m-%d %H:%i:%s')",
        )
        .fetch_one(my.unwrap())
        .await
        .unwrap();
        row
    } else {
        let row: (String, String) = sqlx::query_as(
            "SELECT to_char(localtimestamp, 'YYYY-MM-DD HH24:MI:SS'), \
             to_char(current_timestamp AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS')",
        )
        .fetch_one(pg.unwrap())
        .await
        .unwrap();
        row
    }
}

async fn stored_last_seen(
    env: &ZoneEnv,
    pg: Option<&sqlx::PgPool>,
    my: Option<&sqlx::MySqlPool>,
    session: Uuid,
) -> String {
    if env.engine == "mysql" {
        sqlx::query_scalar(
            "SELECT DATE_FORMAT(`LastSeen`, '%Y-%m-%d %H:%i:%s') FROM `Presence` WHERE `SessionID` = ?",
        )
        .bind(session.to_string())
        .fetch_one(my.unwrap())
        .await
        .unwrap()
    } else {
        sqlx::query_scalar(
            r#"SELECT to_char("LastSeen", 'YYYY-MM-DD HH24:MI:SS') FROM presence WHERE "SessionID" = $1"#,
        )
        .bind(session)
        .fetch_one(pg.unwrap())
        .await
        .unwrap()
    }
}

fn oracle(env: &ZoneEnv, args: &[&str]) -> String {
    let output = Command::new("dotnet")
        .arg(&env.dll)
        .arg("zone")
        .arg(&env.engine)
        .args(args)
        .env("FERROAN_ZONE_ADO", &env.ado)
        .output()
        .unwrap_or_else(|error| panic!("could not run the zone oracle: {error}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        panic!(
            "zone oracle failed: {}",
            public_error(&String::from_utf8_lossy(&output.stderr))
        );
    }
    stdout
}

fn field(text: &str, key: &str) -> String {
    let prefix = format!("{key}=");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix).map(str::to_string))
        .unwrap_or_else(|| panic!("zone oracle omitted {key}"))
}

fn stamp(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M:%S")
        .unwrap_or_else(|error| panic!("unreadable stamp {text}: {error}"))
}

fn close_stamps(label: &str, left: &str, right: &str) {
    let delta = (stamp(left) - stamp(right)).num_seconds().abs();
    assert!(
        delta <= 15,
        "{label}: {left} and {right} differ by {delta}s"
    );
}

fn away_from_utc(local: &str, utc: &str, offset_minutes: i64) {
    let minutes = (stamp(local) - stamp(utc)).num_minutes();
    assert!(
        (minutes - offset_minutes).abs() <= 1,
        "stamp {local} against UTC {utc} is {minutes} minutes, not {offset_minutes}"
    );
}

fn public_error(text: &str) -> String {
    if text.to_ascii_lowercase().contains("password") || text.contains("://") {
        return "the message included a secret".to_string();
    }
    text.chars().take(300).collect()
}
