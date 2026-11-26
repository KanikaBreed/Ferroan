// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tranquillity connection strings, translated into sqlx options.
//!
//! SQLite strings follow System.Data.SQLite (`URI=file:...` and `Data Source`).
//! MySQL strings follow MySqlConnector 2.6.2. PostgreSQL strings follow Npgsql
//! 10.0.3. Those are the packages `OpenSim.Data.MySQL` and `OpenSim.Data.PGSQL`
//! reference at the pinned revision.
//!
//! MySqlConnector leaves the session time zone alone. Npgsql sends `Timezone=`
//! when the string sets it, and otherwise sends `PGTZ` when that variable is
//! set. When neither is set, PostgreSQL applies the role and database zone,
//! then the server zone. sqlx does neither on its own: MySQL defaults to
//! `+00:00`, and PostgreSQL's startup packet forces `TimeZone=UTC`. The pool
//! corrects both after parsing.
//!
//! The pool never grows past [`MAX_POOL_CONNECTIONS`]. A `Maximum Pool Size`
//! key in the string is rejected so a config file cannot lift that bound.
//! SQLite files are opened only when they already exist: Ferroan does not
//! create a database and does not apply migrations.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use sqlx::mysql::{MySqlConnectOptions, MySqlSslMode};
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::sqlite::SqliteConnectOptions;

use crate::backend::Backend;
use crate::error::DbError;
use crate::limits::{MAX_POOL_CONNECTIONS, TIMEZONE_BYTES};

const MAX_CONNECTION_STRING: usize = 4096;
const MAX_PATH: usize = 4096;

/// Parsed SQLite target. The file is not created.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SqliteTarget {
    pub(crate) filename: PathBuf,
    pub(crate) read_only: bool,
    /// `UseUTF16Encoding` was present. The connection stays UTF-8; SQLite
    /// converts stored UTF-16 text when it is read.
    pub(crate) utf16_requested: bool,
}

/// Parsed MySQL target, without the password (tests compare that separately).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MysqlTarget {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) database: String,
    pub(crate) ssl: MysqlSsl,
}

/// SSL modes Ferroan maps onto MySqlConnector's `MySqlSslMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MysqlSsl {
    Disabled,
    Preferred,
    Required,
    VerifyCa,
    VerifyIdentity,
}

/// Parsed PostgreSQL target, without the password.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PostgresTarget {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) database: String,
    pub(crate) ssl: PostgresSsl,
    /// Npgsql `Timezone=`. Absent means the server zone, when it can be read.
    pub(crate) timezone: Option<String>,
}

/// SSL modes Ferroan maps onto Npgsql's `SslMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostgresSsl {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

pub(crate) fn sqlite_target(connection_string: &str) -> Result<SqliteTarget, DbError> {
    let pairs = parse_pairs(connection_string)?;
    let mut data_source = None;
    let mut uri = None;
    let mut utf16_requested = false;
    for (key, value) in &pairs {
        match key.as_str() {
            "datasource" => data_source = Some(value.clone()),
            "uri" => uri = Some(value.clone()),
            "version" => {
                if value != "3" {
                    return Err(DbError::Input(format!(
                        "SQLite Version must be 3, not {value}"
                    )));
                }
            }
            "useutf16encoding" => {
                parse_bool(value)?;
                utf16_requested = true;
            }
            "binaryguid" | "datetimeformat" | "foreignkeys" => {}
            other => {
                return Err(DbError::Input(format!(
                    "unsupported SQLite connection key {other}"
                )));
            }
        }
    }
    let (filename, read_only) = match (data_source, uri) {
        (Some(source), None) => (PathBuf::from(source), false),
        (None, Some(uri)) => parse_sqlite_uri(&uri)?,
        (Some(_), Some(_)) => {
            return Err(DbError::Input(
                "SQLite connection string sets both Data Source and URI".to_string(),
            ));
        }
        (None, None) => {
            return Err(DbError::Input(
                "SQLite connection string needs Data Source or URI".to_string(),
            ));
        }
    };
    if filename.as_os_str().is_empty() || filename.as_os_str().len() > MAX_PATH {
        return Err(DbError::Input(
            "SQLite file path is empty or too long".to_string(),
        ));
    }
    Ok(SqliteTarget {
        filename,
        read_only,
        utf16_requested,
    })
}

pub(crate) fn sqlite_options(target: &SqliteTarget) -> Result<SqliteConnectOptions, DbError> {
    // `--rust auth` keeps C# login's connection on this file. Wait for that
    // lock instead of failing the query. This does not change journal mode.
    let options = SqliteConnectOptions::new()
        .filename(&target.filename)
        .create_if_missing(false)
        .read_only(target.read_only)
        .busy_timeout(std::time::Duration::from_secs(15));
    Ok(options)
}

pub(crate) fn mysql_target(connection_string: &str) -> Result<(MysqlTarget, String), DbError> {
    let pairs = parse_pairs(connection_string)?;
    let mut host = None;
    let mut port = None;
    let mut username = None;
    let mut password = String::new();
    let mut database = None;
    let mut ssl = MysqlSsl::Preferred;
    for (key, value) in &pairs {
        match key.as_str() {
            "datasource" | "server" | "host" | "address" | "addr" | "networkaddress" => {
                host = Some(value.clone());
            }
            "port" => port = Some(parse_port(value)?),
            "database" | "initialcatalog" => database = Some(value.clone()),
            "userid" | "username" | "uid" | "user" => username = Some(value.clone()),
            "password" | "pwd" => password = value.clone(),
            "sslmode" => ssl = parse_mysql_ssl(value)?,
            "charset" | "characterset" | "oldguids" | "treattinyasboolean"
            | "connectiontimeout" | "connecttimeout" => {}
            "maximumpoolsize" | "pooling" => {
                return Err(DbError::Input(format!(
                    "Ferroan uses its own pool of at most {MAX_POOL_CONNECTIONS} connections and does not accept {key}"
                )));
            }
            other => {
                return Err(DbError::Input(format!(
                    "unsupported MySQL connection key {other}"
                )));
            }
        }
    }
    let host = require_field(host, "MySQL host")?;
    reject_host_tricks(&host)?;
    Ok((
        MysqlTarget {
            host,
            port: port.unwrap_or(3306),
            username: require_field(username, "MySQL user")?,
            database: require_field(database, "MySQL database")?,
            ssl,
        },
        password,
    ))
}

pub(crate) fn mysql_options(target: &MysqlTarget, password: &str) -> MySqlConnectOptions {
    MySqlConnectOptions::new()
        .host(&target.host)
        .port(target.port)
        .username(&target.username)
        .password(password)
        .database(&target.database)
        .ssl_mode(match target.ssl {
            MysqlSsl::Disabled => MySqlSslMode::Disabled,
            MysqlSsl::Preferred => MySqlSslMode::Preferred,
            MysqlSsl::Required => MySqlSslMode::Required,
            MysqlSsl::VerifyCa => MySqlSslMode::VerifyCa,
            MysqlSsl::VerifyIdentity => MySqlSslMode::VerifyIdentity,
        })
        // sqlx's default sends time_zone='+00:00'. MySqlConnector 2.6.2 does not.
        .timezone(None::<String>)
}

pub(crate) fn postgres_target(
    connection_string: &str,
) -> Result<(PostgresTarget, String), DbError> {
    let pairs = parse_pairs(connection_string)?;
    let mut host = None;
    let mut port = None;
    let mut username = None;
    let mut password = String::new();
    let mut database = None;
    let mut ssl = PostgresSsl::Prefer;
    let mut timezone = None;
    for (key, value) in &pairs {
        match key.as_str() {
            "datasource" | "server" | "host" => host = Some(value.clone()),
            "port" => port = Some(parse_port(value)?),
            "database" | "db" => database = Some(value.clone()),
            "userid" | "username" | "uid" | "user" => username = Some(value.clone()),
            "password" | "pwd" => password = value.clone(),
            "sslmode" => ssl = parse_postgres_ssl(value)?,
            "timezone" => timezone = Some(require_timezone(value)?),
            "timeout" | "commandtimeout" | "searchpath" => {}
            "maximumpoolsize" | "pooling" => {
                return Err(DbError::Input(format!(
                    "Ferroan uses its own pool of at most {MAX_POOL_CONNECTIONS} connections and does not accept {key}"
                )));
            }
            other => {
                return Err(DbError::Input(format!(
                    "unsupported PostgreSQL connection key {other}"
                )));
            }
        }
    }
    let host = require_field(host, "PostgreSQL host")?;
    reject_host_tricks(&host)?;
    Ok((
        PostgresTarget {
            host,
            port: port.unwrap_or(5432),
            username: require_field(username, "PostgreSQL user")?,
            database: require_field(database, "PostgreSQL database")?,
            ssl,
            timezone,
        },
        password,
    ))
}

/// A sqlx MySQL URL. An explicit `timezone` query is kept. Otherwise the
/// session uses the server zone, as `mysql_options` does for ADO strings.
pub(crate) fn mysql_url_options(url: &str) -> Result<MySqlConnectOptions, DbError> {
    let options = url
        .parse::<MySqlConnectOptions>()
        .map_err(|error| DbError::Input(scrub_driver_message(&error.to_string())))?;
    if url_sets_timezone(url) {
        Ok(options)
    } else {
        Ok(options.timezone(None::<String>))
    }
}

/// A sqlx PostgreSQL URL. The session zone is applied after connect.
pub(crate) fn postgres_url_options(url: &str) -> Result<PgConnectOptions, DbError> {
    url.parse::<PgConnectOptions>()
        .map_err(|error| DbError::Input(scrub_driver_message(&error.to_string())))
}

/// True when the URL query uses a key sqlx's MySQL parser honours.
///
/// sqlx matches `timezone` and `time-zone` only, and the match is
/// case-sensitive. Any other spelling leaves sqlx's `+00:00` default in
/// place, so the caller clears it and the session uses the server zone.
fn url_sets_timezone(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else {
        return false;
    };
    query.split('&').any(|pair| {
        let key = pair.split_once('=').map(|(key, _)| key).unwrap_or(pair);
        matches!(key, "timezone" | "time-zone")
    })
}

fn require_timezone(value: &str) -> Result<String, DbError> {
    if zone_within_limit(value) {
        Ok(value.to_string())
    } else {
        Err(DbError::Input(format!(
            "PostgreSQL Timezone is empty or longer than {}",
            ferroan_diag::DB_TIMEZONE
        )))
    }
}

/// Length bound only. The value is a bound parameter, so a character filter
/// adds no safety and rejects zones PostgreSQL accepts (`5.5`, POSIX strings,
/// `<+0330>-03:30`). Empty is not a zone. A longer value reports `db.timezone`.
fn zone_within_limit(value: &str) -> bool {
    if value.len() > TIMEZONE_BYTES {
        let observed = u64::try_from(value.len()).unwrap_or(u64::MAX);
        let configured = u64::try_from(TIMEZONE_BYTES).unwrap_or(u64::MAX);
        ferroan_diag::refuse(
            ferroan_diag::DB_TIMEZONE,
            ferroan_diag::Kind::Hard,
            configured,
            observed,
            "db",
        );
        return false;
    }
    !value.is_empty()
}

/// The last `timezone` or `TimeZone` assignment in PostgreSQL's `postmaster.opts`.
///
/// The file is one shell-quoted command line. The value is taken as stored.
/// An empty or over-long value is ignored so the caller can try the configuration file.
pub(crate) fn timezone_from_postmaster_opts(text: &str) -> Option<String> {
    let tokens = shell_tokens(text)?;
    let mut found = None;
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if let Some(value) = assignment_after(token, "-c") {
            found = accepted_zone(value);
        } else if let Some(value) = assignment_after(token, "--") {
            found = accepted_zone(value);
        } else if token == "-c"
            && let Some(next) = tokens.get(index + 1)
            && let Some(value) = zone_assignment(next)
        {
            found = accepted_zone(value);
        }
        index += 1;
    }
    found
}

/// `timezone=America/New_York` after a `-c` or `--` prefix on the same token.
fn assignment_after<'a>(token: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = token.strip_prefix(prefix)?;
    zone_assignment(rest)
}

fn zone_assignment(token: &str) -> Option<&str> {
    let (key, value) = token.split_once('=')?;
    if key.eq_ignore_ascii_case("timezone") {
        Some(value)
    } else {
        None
    }
}

fn accepted_zone(value: &str) -> Option<String> {
    if zone_within_limit(value) {
        Some(value.to_string())
    } else {
        None
    }
}

/// Split a `postmaster.opts` line. An unclosed quote means the line is not usable.
fn shell_tokens(text: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut in_token = false;
    let mut quote = None;
    while let Some(ch) = chars.next() {
        if let Some(opened) = quote {
            if ch == opened {
                quote = None;
            } else if ch == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            in_token = true;
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if in_token {
                tokens.push(std::mem::take(&mut current));
                in_token = false;
            }
            continue;
        }
        in_token = true;
        current.push(ch);
    }
    if quote.is_some() {
        return None;
    }
    if in_token {
        tokens.push(current);
    }
    Some(tokens)
}

/// One warning per pool when the session has to stay on UTC (D104).
const UTC_FALLBACK_WARNING: &str = "PostgreSQL session stays on UTC because this login cannot read the server zone (D104). Set Timezone= to that zone";

/// Set the PostgreSQL session zone. sqlx has already forced `TimeZone=UTC`
/// in the startup packet, and `RESET` keeps that value, so `pg_settings` does
/// not reveal the server default.
///
/// Npgsql sends `Timezone=` from the connection string, otherwise `PGTZ`.
/// When neither is set, PostgreSQL applies `pg_db_role_setting` and then the
/// command line and configuration file. `explicit` is the connection-string
/// zone and is sent as a bound parameter. A login that still cannot read a
/// zone leaves the UTC session in place (D104).
pub(crate) async fn apply_postgres_timezone(
    conn: &mut sqlx::postgres::PgConnection,
    explicit: Option<String>,
    warned: &AtomicBool,
) -> Result<(), sqlx::Error> {
    let zone = match explicit {
        Some(zone) => zone,
        None => match pgtz_from_env()? {
            Some(zone) => zone,
            None => match server_timezone(conn).await? {
                ServerZone::Found(zone) => zone,
                ServerZone::Unreadable => {
                    warn_utc_fallback(warned);
                    return Ok(());
                }
                ServerZone::Absent => return Ok(()),
            },
        },
    };
    sqlx::query("SELECT set_config('TimeZone', $1, false)")
        .bind(zone)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Npgsql's `PostgresEnvironment.TimeZone`: the `PGTZ` variable.
///
/// An unset variable is not a zone. The value is sent as given, with no trim.
/// An empty or over-long value refuses the connection instead of being sent.
pub(crate) fn pgtz_from_env() -> Result<Option<String>, sqlx::Error> {
    match std::env::var("PGTZ") {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(unsupported_pgtz("PGTZ is not Unicode")),
        Ok(value) => pgtz_value(&value).map(Some),
    }
}

/// `PGTZ` as given. No trim. Empty and over-long values refuse the connection.
fn pgtz_value(value: &str) -> Result<String, sqlx::Error> {
    if zone_within_limit(value) {
        Ok(value.to_string())
    } else {
        Err(unsupported_pgtz(&format!(
            "PGTZ is empty or longer than {}",
            ferroan_diag::DB_TIMEZONE
        )))
    }
}

fn unsupported_pgtz(message: &str) -> sqlx::Error {
    sqlx::Error::Configuration(Box::new(UnsupportedZone(message.to_string())))
}

#[derive(Debug)]
struct UnsupportedZone(String);

impl std::fmt::Display for UnsupportedZone {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for UnsupportedZone {}

#[derive(Debug, PartialEq, Eq)]
enum ServerZone {
    Found(String),
    /// Role, database, command line and configuration file named no zone.
    Absent,
    /// No role or database zone could be read, and this login cannot read
    /// the command line or `pg_file_settings` either (D104).
    Unreadable,
}

/// PostgreSQL applies `pg_db_role_setting` above the command line, and the
/// command line above the configuration file. The first zone in that order
/// is the one a client that sent no `TimeZone` parameter receives.
async fn server_timezone(
    conn: &mut sqlx::postgres::PgConnection,
) -> Result<ServerZone, sqlx::Error> {
    if let Some(zone) = role_database_timezone(conn).await? {
        return Ok(ServerZone::Found(zone));
    }
    let (opts_readable, command_line) = command_line_timezone(conn).await?;
    // Skip the file when the command line already named a zone, and when
    // `postmaster.opts` was unreadable. `applied` stays true when the command
    // line overrides the file, so the file cannot be trusted alone.
    let (file_readable, file_zone) = if command_line.is_some() || !opts_readable {
        (false, None)
    } else {
        configured_timezone(conn).await?
    };
    Ok(server_zone_from_probes(
        command_line,
        opts_readable,
        file_readable,
        file_zone,
    ))
}

/// A file zone, or `Absent`, only when both `postmaster.opts` and
/// `pg_file_settings` were readable. The file names a zone for `Found` and
/// names none for `Absent`. A readable `postmaster.opts` that names no zone
/// does not by itself decide `Absent`. An unread command line does not yield
/// to the file.
fn server_zone_from_probes(
    command_line: Option<String>,
    opts_readable: bool,
    file_readable: bool,
    file_zone: Option<String>,
) -> ServerZone {
    if let Some(zone) = command_line {
        return ServerZone::Found(zone);
    }
    if !opts_readable || !file_readable {
        return ServerZone::Unreadable;
    }
    match file_zone {
        Some(zone) => ServerZone::Found(zone),
        None => ServerZone::Absent,
    }
}

/// Highest precedence first: this database and role, this role, this
/// database, then a setting for every database and role. Within one row the
/// last `timezone` assignment wins. The role oid keeps the session name
/// exactly, and an unknown name is NULL rather than an error. The database
/// oid is this backend's `datid`, so the read does not use `pg_database`.
const ROLE_DATABASE_ZONE: &str = "\
SELECT source_rank, setting FROM ( \
  SELECT 1 AS source_rank, entry.setting, entry.ord \
  FROM pg_db_role_setting AS setting, \
       LATERAL unnest(setting.setconfig) WITH ORDINALITY AS entry(setting, ord) \
  WHERE setting.setdatabase = (SELECT datid FROM pg_stat_get_activity(pg_backend_pid())) \
    AND setting.setrole = to_regrole(quote_ident(session_user)) \
  UNION ALL \
  SELECT 2, entry.setting, entry.ord \
  FROM pg_db_role_setting AS setting, \
       LATERAL unnest(setting.setconfig) WITH ORDINALITY AS entry(setting, ord) \
  WHERE setting.setdatabase = 0 \
    AND setting.setrole = to_regrole(quote_ident(session_user)) \
  UNION ALL \
  SELECT 3, entry.setting, entry.ord \
  FROM pg_db_role_setting AS setting, \
       LATERAL unnest(setting.setconfig) WITH ORDINALITY AS entry(setting, ord) \
  WHERE setting.setdatabase = (SELECT datid FROM pg_stat_get_activity(pg_backend_pid())) \
    AND setting.setrole = 0 \
  UNION ALL \
  SELECT 4, entry.setting, entry.ord \
  FROM pg_db_role_setting AS setting, \
       LATERAL unnest(setting.setconfig) WITH ORDINALITY AS entry(setting, ord) \
  WHERE setting.setdatabase = 0 AND setting.setrole = 0 \
) AS levels \
ORDER BY source_rank, ord";

const ROLE_SETTINGS_PRIVILEGE: &str = "\
SELECT has_table_privilege(session_user, 'pg_catalog.pg_db_role_setting', 'SELECT')";

async fn role_database_timezone(
    conn: &mut sqlx::postgres::PgConnection,
) -> Result<Option<String>, sqlx::Error> {
    // Ask before reading. A refused SELECT is written to the server log and
    // sqlx retries it until the acquire timeout. No privilege falls through.
    let allowed: bool = sqlx::query_scalar(ROLE_SETTINGS_PRIVILEGE)
        .fetch_one(&mut *conn)
        .await?;
    if !allowed {
        return Ok(None);
    }
    let rows: Vec<(i32, String)> = match sqlx::query_as(ROLE_DATABASE_ZONE)
        .fetch_all(&mut *conn)
        .await
    {
        Ok(rows) => rows,
        Err(error) if insufficient_privilege(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(timezone_from_ranked_settings(
        rows.iter().map(|(rank, setting)| (*rank, setting.as_str())),
    ))
}

/// The last in-limit `timezone` assignment. A lower rank does not replace a
/// higher one. `entries` are in the order PostgreSQL applies them. The value
/// is taken as stored.
pub(crate) fn timezone_from_ranked_settings<'a>(
    entries: impl IntoIterator<Item = (i32, &'a str)>,
) -> Option<String> {
    let mut best: Option<(i32, String)> = None;
    for (rank, entry) in entries {
        let Some(value) = timezone_from_setting(entry) else {
            continue;
        };
        match &best {
            Some((kept, _)) if rank > *kept => {}
            _ => best = Some((rank, value.to_string())),
        }
    }
    best.map(|(_, zone)| zone)
}

fn timezone_from_setting(entry: &str) -> Option<&str> {
    let (key, value) = entry.split_once('=')?;
    if !key.eq_ignore_ascii_case("timezone") {
        return None;
    }
    if zone_within_limit(value) {
        Some(value)
    } else {
        None
    }
}

const READ_FILE_PRIVILEGE: &str =
    "SELECT has_function_privilege(session_user, 'pg_read_file(text)', 'EXECUTE')";

const POSTMASTER_OPTS: &str = "SELECT pg_read_file('postmaster.opts')";

const FILE_SETTINGS_PRIVILEGE: &str = "\
SELECT has_table_privilege(session_user, 'pg_file_settings', 'SELECT') \
AND has_function_privilege(session_user, 'pg_show_all_file_settings()', 'EXECUTE')";

const FILE_TIMEZONE: &str = "\
SELECT setting FROM pg_file_settings \
WHERE lower(name) = 'timezone' AND applied AND (error IS NULL OR error = '') \
ORDER BY seqno DESC LIMIT 1";

/// `(readable, zone)`. A denied read is not a command line that named no
/// zone: only a readable `postmaster.opts` may yield to `pg_file_settings`.
async fn command_line_timezone(
    conn: &mut sqlx::postgres::PgConnection,
) -> Result<(bool, Option<String>), sqlx::Error> {
    match probe_text(conn, READ_FILE_PRIVILEGE, POSTMASTER_OPTS).await? {
        Probe::Denied => Ok((false, None)),
        Probe::Value(text) => Ok((
            true,
            text.as_deref().and_then(timezone_from_postmaster_opts),
        )),
    }
}

/// Readable is true only when this probe itself was allowed. The setting is
/// taken as stored.
async fn configured_timezone(
    conn: &mut sqlx::postgres::PgConnection,
) -> Result<(bool, Option<String>), sqlx::Error> {
    match probe_text(conn, FILE_SETTINGS_PRIVILEGE, FILE_TIMEZONE).await? {
        Probe::Denied => Ok((false, None)),
        Probe::Value(zone) => Ok((true, zone.filter(|zone| zone_within_limit(zone)))),
    }
}

enum Probe {
    Denied,
    Value(Option<String>),
}

/// Ask before reading. A refused `pg_read_file` or `pg_file_settings` query
/// writes an error to the server log on every new connection. Only SQLSTATE
/// 42501 counts as unreadable; a dropped connection or any other error is
/// returned so the pool does not keep it.
async fn probe_text(
    conn: &mut sqlx::postgres::PgConnection,
    privilege_sql: &str,
    read_sql: &str,
) -> Result<Probe, sqlx::Error> {
    let allowed: bool = sqlx::query_scalar(privilege_sql)
        .fetch_one(&mut *conn)
        .await?;
    if !allowed {
        return Ok(Probe::Denied);
    }
    match sqlx::query_scalar::<_, String>(read_sql)
        .fetch_optional(&mut *conn)
        .await
    {
        Ok(value) => Ok(Probe::Value(value)),
        Err(error) if insufficient_privilege(&error) => Ok(Probe::Denied),
        Err(error) => Err(error),
    }
}

fn insufficient_privilege(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code == "42501")
}

/// Returns whether this call logged. Later calls on the same flag do not.
fn warn_utc_fallback(warned: &AtomicBool) -> bool {
    if warned
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        tracing::warn!("{UTC_FALLBACK_WARNING}");
        true
    } else {
        false
    }
}

pub(crate) fn postgres_options(target: &PostgresTarget, password: &str) -> PgConnectOptions {
    PgConnectOptions::new()
        .host(&target.host)
        .port(target.port)
        .username(&target.username)
        .password(password)
        .database(&target.database)
        .ssl_mode(match target.ssl {
            PostgresSsl::Disable => PgSslMode::Disable,
            PostgresSsl::Prefer => PgSslMode::Prefer,
            PostgresSsl::Require => PgSslMode::Require,
            PostgresSsl::VerifyCa => PgSslMode::VerifyCa,
            PostgresSsl::VerifyFull => PgSslMode::VerifyFull,
        })
}

pub(crate) fn scrub_driver_message(message: &str) -> String {
    let mut cleaned = String::with_capacity(message.len().min(500));
    let bytes = message.as_bytes();
    let mut index = 0;
    while index < bytes.len() && cleaned.len() < 500 {
        if bytes[index..].starts_with(b"://") {
            cleaned.push_str("://");
            index += 3;
            while index < bytes.len() && bytes[index] != b'@' && bytes[index] != b'/' {
                index += 1;
            }
            if index < bytes.len() && bytes[index] == b'@' {
                index += 1;
            }
            continue;
        }
        cleaned.push(bytes[index] as char);
        index += 1;
    }
    cleaned
}

fn parse_sqlite_uri(uri: &str) -> Result<(PathBuf, bool), DbError> {
    let Some(rest) = strip_ascii_prefix(uri, "file:") else {
        return Err(DbError::Input(
            "SQLite URI must start with file:".to_string(),
        ));
    };
    if rest.starts_with("//") {
        return Err(DbError::Input(
            "SQLite URI host form (file://...) is not accepted".to_string(),
        ));
    }
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    if path.is_empty() {
        return Err(DbError::Input("SQLite URI has an empty path".to_string()));
    }
    let mut read_only = false;
    if let Some(query) = query {
        for part in query.split('&') {
            if part.is_empty() {
                continue;
            }
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            match key.to_ascii_lowercase().as_str() {
                "mode" => match value.to_ascii_lowercase().as_str() {
                    "ro" => read_only = true,
                    "rw" | "rwc" => read_only = false,
                    "memory" => {
                        return Err(DbError::Input(
                            "SQLite memory databases are not accepted".to_string(),
                        ));
                    }
                    other => {
                        return Err(DbError::Input(format!(
                            "unsupported SQLite URI mode {other}"
                        )));
                    }
                },
                other => {
                    return Err(DbError::Input(format!(
                        "unsupported SQLite URI parameter {other}"
                    )));
                }
            }
        }
    }
    Ok((PathBuf::from(path), read_only))
}

fn parse_mysql_ssl(value: &str) -> Result<MysqlSsl, DbError> {
    match normalize_token(value).as_str() {
        "none" | "disabled" | "disable" => Ok(MysqlSsl::Disabled),
        "preferred" | "prefer" => Ok(MysqlSsl::Preferred),
        "required" | "require" => Ok(MysqlSsl::Required),
        "verifyca" => Ok(MysqlSsl::VerifyCa),
        "verifyfull" | "verifyidentity" => Ok(MysqlSsl::VerifyIdentity),
        other => Err(DbError::Input(format!("unsupported MySQL SslMode {other}"))),
    }
}

fn parse_postgres_ssl(value: &str) -> Result<PostgresSsl, DbError> {
    match normalize_token(value).as_str() {
        "disable" | "disabled" | "none" => Ok(PostgresSsl::Disable),
        "prefer" | "preferred" | "allow" => Ok(PostgresSsl::Prefer),
        "require" | "required" => Ok(PostgresSsl::Require),
        "verifyca" => Ok(PostgresSsl::VerifyCa),
        "verifyfull" => Ok(PostgresSsl::VerifyFull),
        other => Err(DbError::Input(format!(
            "unsupported PostgreSQL SSL Mode {other}"
        ))),
    }
}

fn parse_pairs(input: &str) -> Result<Vec<(String, String)>, DbError> {
    if input.len() > MAX_CONNECTION_STRING {
        return Err(DbError::Input(
            "connection string is longer than 4096 bytes".to_string(),
        ));
    }
    if input.as_bytes().contains(&0) {
        return Err(DbError::Input(
            "connection string contains a NUL byte".to_string(),
        ));
    }
    let mut pairs = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        let (pair, next) = split_pair(rest)?;
        rest = next;
        if pair.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = pair.split_once('=') else {
            return Err(DbError::Input(
                "connection string entry has no '='".to_string(),
            ));
        };
        let key = normalize_token(key);
        if key.is_empty() {
            return Err(DbError::Input(
                "connection string has an empty key".to_string(),
            ));
        }
        if key.len() > 64 || value.len() > MAX_CONNECTION_STRING {
            return Err(DbError::Input(
                "connection string key or value is too long".to_string(),
            ));
        }
        pairs.push((key, unquote(value.trim())));
    }
    if pairs.is_empty() {
        return Err(DbError::Input("connection string is empty".to_string()));
    }
    if pairs.len() > 32 {
        return Err(DbError::Input(
            "connection string has too many keys".to_string(),
        ));
    }
    Ok(pairs)
}

fn split_pair(input: &str) -> Result<(&str, &str), DbError> {
    let mut quote: Option<char> = None;
    for (index, ch) in input.char_indices() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == ';' => return Ok((&input[..index], &input[index + 1..])),
            None => {}
        }
    }
    if quote.is_some() {
        return Err(DbError::Input(
            "connection string has an unterminated quote".to_string(),
        ));
    }
    Ok((input, ""))
}

fn unquote(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    if (first == '"' || first == '\'') && value.ends_with(first) && value.chars().count() >= 2 {
        let inner: String = value.chars().skip(1).collect();
        return inner
            .chars()
            .take(inner.chars().count().saturating_sub(1))
            .collect();
    }
    value.to_string()
}

fn normalize_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '_' && *ch != '-')
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn parse_bool(value: &str) -> Result<bool, DbError> {
    match normalize_token(value).as_str() {
        "true" | "yes" | "1" => Ok(true),
        "false" | "no" | "0" => Ok(false),
        _ => Err(DbError::Input(format!(
            "expected a boolean connection value, got {value}"
        ))),
    }
}

fn parse_port(value: &str) -> Result<u16, DbError> {
    let port: u16 = value
        .trim()
        .parse()
        .map_err(|_| DbError::Input(format!("invalid port {value}")))?;
    if port == 0 {
        return Err(DbError::Input(
            "port must be between 1 and 65535".to_string(),
        ));
    }
    Ok(port)
}

fn require_field(value: Option<String>, what: &str) -> Result<String, DbError> {
    match value {
        Some(text) if !text.is_empty() && text.len() <= 256 => Ok(text),
        Some(_) => Err(DbError::Input(format!("{what} is empty or too long"))),
        None => Err(DbError::Input(format!("connection string needs {what}"))),
    }
}

fn reject_host_tricks(host: &str) -> Result<(), DbError> {
    if host.contains("://") || host.contains('@') || host.contains('/') || host.contains('\\') {
        return Err(DbError::Input(
            "database host must be a host name, not a URL".to_string(),
        ));
    }
    Ok(())
}

fn strip_ascii_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let mut text_bytes = text.bytes();
    for expected in prefix.bytes() {
        if text_bytes.next()?.to_ascii_lowercase() != expected {
            return None;
        }
    }
    Some(&text[prefix.len()..])
}

/// Backend named by a sqlx URL. Used by tests and by operators who already have a URL.
pub fn backend_from_sqlx_url(url: &str) -> Result<Backend, DbError> {
    if url.len() > MAX_CONNECTION_STRING || url.as_bytes().contains(&0) {
        return Err(DbError::Input(
            "database URL is empty or too long".to_string(),
        ));
    }
    let scheme = url.split_once(':').map(|(scheme, _)| scheme).unwrap_or("");
    match scheme.to_ascii_lowercase().as_str() {
        "sqlite" => Ok(Backend::Sqlite),
        "mysql" => Ok(Backend::Mysql),
        "postgres" | "postgresql" => Ok(Backend::Postgres),
        _ => Err(DbError::Input(
            "database URL must start with sqlite:, mysql: or postgres:".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_uri_from_the_shipped_standalone_config() {
        let target = sqlite_target("URI=file:Asset.db").unwrap();
        assert_eq!(target.filename, PathBuf::from("Asset.db"));
        assert!(!target.read_only);
        assert!(!target.utf16_requested);

        let encoded = sqlite_target("URI=file:OpenSim.db;UseUTF16Encoding=True").unwrap();
        assert_eq!(encoded.filename, PathBuf::from("OpenSim.db"));
        assert!(encoded.utf16_requested);
    }

    #[test]
    fn sqlite_uri_mode_and_absolute_path() {
        let target = sqlite_target("URI=file:/tmp/region.db?mode=ro").unwrap();
        assert_eq!(target.filename, PathBuf::from("/tmp/region.db"));
        assert!(target.read_only);
        assert!(sqlite_target("URI=file:mem.db?mode=memory").is_err());
        assert!(sqlite_target("URI=file://localhost/mem.db").is_err());
    }

    #[test]
    fn mysql_grid_server_example_disables_ssl() {
        let (target, password) = mysql_target(
            "Data Source=localhost;Database=opensim;User ID=opensim;Password=*****;Old Guids=true;SslMode=None;",
        )
        .unwrap();
        assert_eq!(target.host, "localhost");
        assert_eq!(target.port, 3306);
        assert_eq!(target.username, "opensim");
        assert_eq!(target.database, "opensim");
        assert_eq!(password, "*****");
        assert_eq!(target.ssl, MysqlSsl::Disabled);
    }

    #[test]
    fn mysql_without_ssl_mode_prefers_encryption() {
        let (target, password) =
            mysql_target("Server=localhost;Database=opensim;User Id=opensim; password=secret")
                .unwrap();
        assert_eq!(target.ssl, MysqlSsl::Preferred);
        assert_eq!(password, "secret");
        assert_eq!(target.username, "opensim");
    }

    #[test]
    fn postgres_example_disables_ssl() {
        let (target, password) = postgres_target(
            "Server=localhost;Database=opensim;User Id=opensim; password=***; SSL Mode=Disable",
        )
        .unwrap();
        assert_eq!(target.port, 5432);
        assert_eq!(target.ssl, PostgresSsl::Disable);
        assert_eq!(password, "***");
        assert_eq!(target.database, "opensim");
    }

    #[test]
    fn quoted_password_keeps_an_embedded_semicolon() {
        let (target, password) =
            mysql_target("Server=localhost;Database=opensim;User ID=opensim;Password=\"sec;ret\";")
                .unwrap();
        assert_eq!(password, "sec;ret");
        assert_eq!(target.database, "opensim");
    }

    #[test]
    fn unknown_key_is_named_and_its_value_is_not() {
        let error =
            mysql_target("Server=localhost;Database=db;User ID=u;Secret=hunter2").unwrap_err();
        let text = error.to_string();
        assert!(text.contains("secret"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
    }

    #[test]
    fn postgres_timezone_is_kept_and_not_rewritten() {
        let (target, _) = postgres_target(
            "Server=localhost;Database=db;User Id=u;Password=p;Timezone=America/New_York",
        )
        .unwrap();
        assert_eq!(target.timezone.as_deref(), Some("America/New_York"));
        let error = postgres_target("Server=localhost;Database=db;User Id=u;Password=p;Timezone=")
            .unwrap_err();
        let text = error.to_string();
        assert!(text.contains("Timezone"), "{text}");
        assert!(text.contains(ferroan_diag::DB_TIMEZONE), "{text}");
        let (spaced, _) = postgres_target(
            "Server=localhost;Database=db;User Id=u;Password=p;Timezone=America/New York",
        )
        .unwrap();
        assert_eq!(spaced.timezone.as_deref(), Some("America/New York"));
        let long = "A".repeat(TIMEZONE_BYTES + 1);
        let too_long = postgres_target(&format!(
            "Server=localhost;Database=db;User Id=u;Password=p;Timezone={long}"
        ))
        .unwrap_err();
        let too_long = too_long.to_string();
        assert!(too_long.contains(ferroan_diag::DB_TIMEZONE), "{too_long}");
        assert!(!too_long.contains(&long));
        let mysql = mysql_target("Server=localhost;Database=db;User ID=u;Password=p;Timezone=UTC")
            .unwrap_err();
        assert!(mysql.to_string().contains("unsupported MySQL"));
    }

    #[test]
    fn mysql_url_timezone_key_matches_sqlx() {
        assert!(url_sets_timezone("mysql://u@h/db?timezone=%2B08%3A00"));
        assert!(url_sets_timezone(
            "mysql://u@h/db?ssl-mode=disabled&time-zone=%2B08%3A00"
        ));
        assert!(!url_sets_timezone("mysql://u@h/db?time_zone=%2B08%3A00"));
        assert!(!url_sets_timezone("mysql://u@h/db?Timezone=%2B08%3A00"));
        assert!(!url_sets_timezone("mysql://u@h/db?TIMEZONE=UTC"));
        assert!(!url_sets_timezone("mysql://u@h/db"));
    }

    #[test]
    fn ranked_settings_keep_postgresql_order() {
        let specific = [
            (1, "search_path=public"),
            (1, "TimeZone=Europe/Berlin"),
            (1, "TimeZone=Asia/Tokyo"),
            (2, "TimeZone=America/New_York"),
            (3, "timezone=UTC"),
        ];
        assert_eq!(
            timezone_from_ranked_settings(specific).as_deref(),
            Some("Asia/Tokyo")
        );
        let role_over_database = [
            (3, "TimeZone=Europe/Berlin"),
            (2, "TimeZone=Asia/Tokyo"),
            (4, "TimeZone=UTC"),
        ];
        assert_eq!(
            timezone_from_ranked_settings(role_over_database).as_deref(),
            Some("Asia/Tokyo")
        );
        let database_when_the_role_row_names_no_zone = [
            (1, "search_path=public"),
            (2, "TimeZone="),
            (3, "TimeZone=Europe/Berlin"),
        ];
        assert_eq!(
            timezone_from_ranked_settings(database_when_the_role_row_names_no_zone).as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(
            timezone_from_ranked_settings([(2, "TimeZone=not a zone")]).as_deref(),
            Some("not a zone")
        );
        assert_eq!(
            timezone_from_ranked_settings([(4, "TimeZone=UTC")]).as_deref(),
            Some("UTC")
        );
        assert!(timezone_from_ranked_settings([(1, "listen_addresses=*")]).is_none());
    }

    #[test]
    fn utc_fallback_warning_is_once_per_pool() {
        assert!(UTC_FALLBACK_WARNING.contains("D104"));
        assert!(UTC_FALLBACK_WARNING.contains("Timezone="));
        let warned = AtomicBool::new(false);
        assert!(warn_utc_fallback(&warned));
        assert!(!warn_utc_fallback(&warned));
    }

    #[test]
    fn postmaster_opts_timezone_is_the_last_assignment() {
        let line = "/usr/lib/postgresql/16/bin/postgres \"-c\" \"timezone=UTC\" \"-c\" \"timezone=America/New_York\"\n";
        assert_eq!(
            timezone_from_postmaster_opts(line).as_deref(),
            Some("America/New_York")
        );
        assert_eq!(
            timezone_from_postmaster_opts("postgres -ctimezone=Europe/Berlin").as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(
            timezone_from_postmaster_opts("postgres --timezone=+01:00").as_deref(),
            Some("+01:00")
        );
        assert!(timezone_from_postmaster_opts("postgres -c listen_addresses=*").is_none());
        assert!(timezone_from_postmaster_opts("postgres \"-c").is_none());
        assert_eq!(
            timezone_from_postmaster_opts("postgres \"-ctimezone=5.5 \"").as_deref(),
            Some("5.5 ")
        );
    }

    #[test]
    fn numeric_and_posix_zones_are_sent_as_given() {
        let samples = ["5.5", "EST5EDT,M3.2.0,M11.1.0", "<+0330>-03:30"];
        for sample in samples {
            let (target, _) = postgres_target(&format!(
                "Server=localhost;Database=db;User Id=u;Password=p;Timezone={sample}"
            ))
            .unwrap();
            assert_eq!(target.timezone.as_deref(), Some(sample));
            assert_eq!(require_timezone(sample).unwrap(), sample);
            let ranked = format!("TimeZone={sample}");
            assert_eq!(
                timezone_from_ranked_settings([
                    (1, ranked.as_str()),
                    (2, "TimeZone=Europe/Berlin")
                ])
                .as_deref(),
                Some(sample)
            );
            let command = format!("postgres -ctimezone={sample}");
            assert_eq!(
                timezone_from_postmaster_opts(&command).as_deref(),
                Some(sample)
            );
            let setting = format!("timezone={sample}");
            assert_eq!(timezone_from_setting(&setting), Some(sample));
        }
        assert_eq!(require_timezone(" 5.5").unwrap(), " 5.5");
        assert_eq!(timezone_from_setting("TimeZone= 5.5"), Some(" 5.5"));
        assert_eq!(
            timezone_from_setting("TimeZone=<+0330>-03:30"),
            Some("<+0330>-03:30")
        );
        let too_long = format!("TimeZone={}", "B".repeat(TIMEZONE_BYTES + 1));
        assert!(timezone_from_setting(&too_long).is_none());
        assert_eq!(
            timezone_from_ranked_settings([(1, too_long.as_str()), (2, "TimeZone=Europe/Berlin")])
                .as_deref(),
            Some("Europe/Berlin")
        );
    }

    #[test]
    fn pgtz_keeps_surrounding_characters() {
        for sample in ["5.5", "EST5EDT,M3.2.0,M11.1.0", "<+0330>-03:30", " 5.5"] {
            assert_eq!(pgtz_value(sample).unwrap(), sample);
        }
        let empty = pgtz_value("").unwrap_err().to_string();
        assert!(empty.contains(ferroan_diag::DB_TIMEZONE), "{empty}");
        let long = "C".repeat(TIMEZONE_BYTES + 1);
        let too_long = pgtz_value(&long).unwrap_err().to_string();
        assert!(too_long.contains(ferroan_diag::DB_TIMEZONE), "{too_long}");
        assert!(!too_long.contains(&long));
    }

    #[test]
    fn absent_requires_readable_command_line_and_file() {
        assert_eq!(
            server_zone_from_probes(None, false, false, None),
            ServerZone::Unreadable
        );
        assert_eq!(
            server_zone_from_probes(None, false, true, Some("Europe/Berlin".to_string())),
            ServerZone::Unreadable
        );
        assert_eq!(
            server_zone_from_probes(None, false, true, None),
            ServerZone::Unreadable
        );
        assert_eq!(
            server_zone_from_probes(None, true, false, None),
            ServerZone::Unreadable
        );
        assert_eq!(
            server_zone_from_probes(None, true, true, None),
            ServerZone::Absent
        );
        assert_eq!(
            server_zone_from_probes(None, true, true, Some("America/New_York".to_string())),
            ServerZone::Found("America/New_York".to_string())
        );
        assert_eq!(
            server_zone_from_probes(Some("America/New_York".to_string()), true, false, None),
            ServerZone::Found("America/New_York".to_string())
        );
    }

    #[test]
    fn role_catalog_is_checked_before_the_read() {
        assert!(ROLE_SETTINGS_PRIVILEGE.contains("has_table_privilege"));
        assert!(ROLE_SETTINGS_PRIVILEGE.contains("pg_catalog.pg_db_role_setting"));
        assert!(!ROLE_SETTINGS_PRIVILEGE.contains("pg_database"));
        assert!(ROLE_DATABASE_ZONE.contains("to_regrole(quote_ident(session_user))"));
        assert!(ROLE_DATABASE_ZONE.contains("pg_stat_get_activity(pg_backend_pid())"));
        assert!(!ROLE_DATABASE_ZONE.contains("session_user::regrole"));
        assert!(!ROLE_DATABASE_ZONE.contains("pg_roles"));
        assert!(!ROLE_DATABASE_ZONE.contains("pg_database"));
        assert!(FILE_SETTINGS_PRIVILEGE.contains("has_function_privilege"));
        assert!(FILE_SETTINGS_PRIVILEGE.contains("pg_show_all_file_settings()"));
    }

    #[test]
    fn pool_size_key_is_rejected() {
        let error = mysql_target(
            "Server=localhost;Database=db;User ID=u;Password=p;Maximum Pool Size=1000;",
        )
        .unwrap_err();
        assert!(error.to_string().contains("at most"));
    }

    #[test]
    fn last_duplicate_key_wins_and_nul_is_rejected() {
        let (target, _) =
            mysql_target("Server=one;Server=two;Database=db;User ID=u;Password=p;Port=3307")
                .unwrap();
        assert_eq!(target.host, "two");
        assert_eq!(target.port, 3307);
        assert!(sqlite_target("URI=file:a.db\0").is_err());
    }

    #[test]
    fn scrub_removes_url_userinfo() {
        let cleaned = scrub_driver_message("failed mysql://opensim:secret@localhost/opensim now");
        assert!(!cleaned.contains("secret"), "{cleaned}");
        assert!(cleaned.contains("localhost"), "{cleaned}");
    }
}
