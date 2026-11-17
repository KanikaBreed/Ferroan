// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tranquillity connection strings, translated into sqlx options.
//!
//! SQLite strings follow System.Data.SQLite (`URI=file:...` and `Data Source`).
//! MySQL strings follow MySqlConnector 2.6.2. PostgreSQL strings follow Npgsql
//! 10.0.3. Those are the packages `OpenSim.Data.MySQL` and `OpenSim.Data.PGSQL`
//! reference at the pinned revision.
//!
//! The pool never grows past [`MAX_POOL_CONNECTIONS`]. A `Maximum Pool Size`
//! key in the string is rejected so a config file cannot lift that bound.
//! SQLite files are opened only when they already exist: Ferroan does not
//! create a database and does not apply migrations.

use std::path::PathBuf;

use sqlx::mysql::{MySqlConnectOptions, MySqlSslMode};
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::sqlite::SqliteConnectOptions;

use crate::backend::Backend;
use crate::error::DbError;
use crate::limits::MAX_POOL_CONNECTIONS;

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
    let options = SqliteConnectOptions::new()
        .filename(&target.filename)
        .create_if_missing(false)
        .read_only(target.read_only);
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
    for (key, value) in &pairs {
        match key.as_str() {
            "datasource" | "server" | "host" => host = Some(value.clone()),
            "port" => port = Some(parse_port(value)?),
            "database" | "db" => database = Some(value.clone()),
            "userid" | "username" | "uid" | "user" => username = Some(value.clone()),
            "password" | "pwd" => password = value.clone(),
            "sslmode" => ssl = parse_postgres_ssl(value)?,
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
        },
        password,
    ))
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
