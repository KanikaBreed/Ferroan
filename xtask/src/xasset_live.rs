// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! XAsset live compare.
//!
//! `cargo xtask live assets --backend xasset --db mysql|pgsql` copies the
//! loaded library into `XAssetsMeta`, `XAssetsData` and the ordinary `assets`
//! table, then runs texture and inventory against C# and Ferroan on that
//! database. Neither service calls `ForEachDefaultXmlAsset`.
//!
//! The success line is `assets: match`. Pin6 carries the C# fixes for TB27,
//! TB28, TB29, TB30 and TB33, so that compare no longer names them. TB30
//! deletes only the meta row and leaves the data row, on both sides. A
//! rolled-back migrate and two concurrent stores of the same bytes are not
//! in this compare (TB58, TB59, D43). Any other HTTP difference fails the
//! command.

use std::process::ExitCode;

#[cfg(any(target_os = "linux", test))]
use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

#[cfg(target_os = "linux")]
use crate::live;
#[cfg(target_os = "linux")]
use crate::repo_root;

#[cfg(target_os = "linux")]
const SEED_QUERY: &str = "SELECT json_object(\
    'id', UUID, \
    'name', hex(Name), \
    'description', hex(Description), \
    'type', Type, \
    'local', Local, \
    'temporary', Temporary, \
    'flags', asset_flags, \
    'creator', hex(CreatorID), \
    'data', hex(Data)) FROM assets;";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XassetDb {
    Mysql,
    Pgsql,
}

impl XassetDb {
    #[cfg(target_os = "linux")]
    fn label(self) -> &'static str {
        match self {
            Self::Mysql => "mysql",
            Self::Pgsql => "pgsql",
        }
    }

    #[cfg(target_os = "linux")]
    fn env_name(self) -> &'static str {
        match self {
            Self::Mysql => "FERROAN_MYSQL_URL",
            Self::Pgsql => "FERROAN_POSTGRES_URL",
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn provider(self) -> &'static str {
        match self {
            Self::Mysql => "OpenSim.Data.MySQL.dll",
            Self::Pgsql => "OpenSim.Data.PGSQL.dll",
        }
    }
}

pub(crate) fn compare(db: XassetDb) -> ExitCode {
    match compare_inner(db) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("live: {error}");
            ExitCode::from(1)
        }
    }
}

fn compare_inner(db: XassetDb) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        compare_linux(db)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = db;
        Err("cargo xtask live assets runs on Linux".to_string())
    }
}

#[cfg(target_os = "linux")]
fn compare_linux(db: XassetDb) -> Result<String, String> {
    if let Some(pid) = live::standalone_is_up()? {
        return Err(format!(
            "the harness is already up (pid {pid}); run cargo xtask live down first"
        ));
    }
    let _state = crate::grid_state::claim()?;
    live::ensure_region_built()?;
    let target = database_target(db)?;
    let seed = crate::assets::seed_library_database()?;
    let rows = read_seed_rows(&seed)?;
    let sql = seed_sql(db, &rows, unix_now()?)?;
    let csharp = run_side(db, &target, "csharp", &sql)?;
    let ferroan = run_side(db, &target, "ferroan", &sql)?;
    let left = crate::assets::canonical_http(
        &fs::read(&csharp.capture)
            .map_err(|error| format!("could not read {}: {error}", csharp.capture.display()))?,
    )?;
    let right = crate::assets::canonical_http(
        &fs::read(&ferroan.capture)
            .map_err(|error| format!("could not read {}: {error}", ferroan.capture.display()))?,
    )?;
    if left != right {
        let diff = crate::assets::first_difference(&left, &right);
        return Err(format!(
            "asset HTTP differs\ncsharp: {}\nferroan: {}\n{diff}",
            csharp.capture.display(),
            ferroan.capture.display()
        ));
    }
    Ok(success_report(
        &csharp.report,
        &ferroan.report,
        left.lines().count(),
    ))
}

#[cfg(any(target_os = "linux", test))]
fn success_report(csharp: &str, ferroan: &str, records: usize) -> String {
    format!("{csharp}\n{ferroan}\nassets: match ({records} http records)")
}

#[cfg(any(target_os = "linux", test))]
struct DbTarget {
    user: String,
    password: String,
    host: String,
    port: u16,
    database: String,
}

#[cfg(any(target_os = "linux", test))]
fn parse_database_url(raw: &str) -> Result<DbTarget, String> {
    let rest = raw
        .strip_prefix("mysql://")
        .or_else(|| raw.strip_prefix("postgres://"))
        .or_else(|| raw.strip_prefix("postgresql://"))
        .ok_or("the database url has an unknown scheme")?;
    let (userinfo, after) = rest.split_once('@').ok_or("the database url has no user")?;
    let (user, password) = userinfo
        .split_once(':')
        .ok_or("the database url has no password")?;
    let (hostport, db_and_query) = after
        .split_once('/')
        .ok_or("the database url has no database")?;
    let database = db_and_query.split('?').next().unwrap_or("");
    let (host, port_text) = hostport
        .split_once(':')
        .ok_or("the database url has no port")?;
    let port = port_text
        .parse::<u16>()
        .map_err(|_| "the database url has a bad port")?;
    if user.is_empty()
        || password.is_empty()
        || host.is_empty()
        || database.is_empty()
        || user.contains([' ', '"', '\'', ';', '#', '\\', '\n', '\r'])
        || password.contains([' ', '"', '\'', ';', '#', '\\', '\n', '\r'])
        || host.contains([' ', '"', ';', '\n', '\r'])
        || database.contains(['/', ' ', '"', ';', '\n', '\r'])
    {
        return Err("the database url has a character this harness cannot quote".to_string());
    }
    Ok(DbTarget {
        user: user.to_string(),
        password: password.to_string(),
        host: host.to_string(),
        port,
        database: database.to_string(),
    })
}

#[cfg(target_os = "linux")]
fn database_target(db: XassetDb) -> Result<DbTarget, String> {
    let name = db.env_name();
    let raw = std::env::var(name).map_err(|_| format!("{name} is unset"))?;
    if raw.is_empty() {
        return Err(format!("{name} is unset"));
    }
    parse_database_url(&raw).map_err(|error| format!("{name}: {error}"))
}

/// C# keeps `Pooling=false`. Ferroan rejects that key and uses its own pool.
#[cfg(any(target_os = "linux", test))]
fn ado(db: XassetDb, target: &DbTarget, pooling_false: bool) -> String {
    let mut text = match db {
        XassetDb::Mysql => format!(
            "Server={};Port={};Database={};User ID={};Password={};SslMode=None",
            target.host, target.port, target.database, target.user, target.password
        ),
        XassetDb::Pgsql => format!(
            "Server={};Port={};Database={};User Id={};Password={};SSL Mode=Disable",
            target.host, target.port, target.database, target.user, target.password
        ),
    };
    if pooling_false {
        text.push_str(";Pooling=false");
    }
    text
}

#[cfg(any(target_os = "linux", test))]
fn service_ini(port: u16, provider: &str, ado: &str, grid: bool) -> String {
    let asset = format!(
        "[AssetService]\n    \
         LocalServiceModule = \"OpenSim.Services.AssetService.dll:XAssetService\"\n    \
         StorageProvider = \"{provider}\"\n    \
         ConnectionString = \"{ado}\"\n"
    );
    if !grid {
        return format!(
            "[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n{asset}"
        );
    }
    format!(
        "[Const]\n    BaseHostname = \"127.0.0.1\"\n    PublicPort = \"{port}\"\n    \
         PrivatePort = \"{port}\"\n\n[Startup]\n    RegistryLocation = \".\"\n    \
         ConsoleHistoryFileEnabled = false\n    \
         EnableRobustSelfsignedCertSupport = false\n\n[Network]\n    port = {port}\n    \
         bind_ip_address = \"127.0.0.1\"\n\n[ServiceList]\n    \
         AssetServiceConnector = \"{port}/OpenSim.Server.Handlers.dll:AssetServiceConnector\"\n\n\
         [DatabaseService]\n    StorageProvider = \"{provider}\"\n    \
         ConnectionString = \"{ado}\"\n\n{asset}"
    )
}

#[cfg(any(target_os = "linux", test))]
struct SeedAsset {
    id: String,
    name_hex: String,
    description_hex: String,
    type_code: i64,
    local: bool,
    temporary: bool,
    flags: i64,
    creator_hex: String,
    data_hex: String,
    hash_hex: String,
}

#[cfg(any(target_os = "linux", test))]
fn seed_sql(db: XassetDb, rows: &[SeedAsset], now: i64) -> Result<String, String> {
    if i32::try_from(now).is_err() {
        return Err("the clock does not fit the asset time column".to_string());
    }
    let mut sql = String::new();
    match db {
        XassetDb::Mysql => {
            sql.push_str(
                "START TRANSACTION;\nDELETE FROM `XAssetsMeta`;\nDELETE FROM `XAssetsData`;\n\
                 DELETE FROM `assets`;\n",
            );
        }
        XassetDb::Pgsql => {
            sql.push_str(
                "BEGIN;\nDELETE FROM xassetsmeta;\nDELETE FROM xassetsdata;\nDELETE FROM assets;\n",
            );
        }
    }
    for row in rows {
        validate_row(row)?;
        match db {
            XassetDb::Mysql => push_mysql_row(&mut sql, row, now),
            XassetDb::Pgsql => push_postgres_row(&mut sql, row, now),
        }
    }
    sql.push_str("COMMIT;\n");
    Ok(sql)
}

#[cfg(any(target_os = "linux", test))]
fn validate_row(row: &SeedAsset) -> Result<(), String> {
    if !is_uuid(&row.id) {
        return Err("a library row has an id that is not a uuid".to_string());
    }
    for hex in [
        &row.name_hex,
        &row.description_hex,
        &row.creator_hex,
        &row.data_hex,
        &row.hash_hex,
    ] {
        if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) || !hex.len().is_multiple_of(2) {
            return Err("a library value is not hex".to_string());
        }
    }
    if row.hash_hex.len() != 64 {
        return Err("a library hash is not 32 bytes".to_string());
    }
    if !(-128..=127).contains(&row.type_code) || i32::try_from(row.flags).is_err() {
        return Err("a library row has a value this table cannot store".to_string());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn push_mysql_row(sql: &mut String, row: &SeedAsset, now: i64) {
    let local = i32::from(row.local);
    let temporary = i32::from(row.temporary);
    let name = mysql_text(&row.name_hex);
    let description = mysql_text(&row.description_hex);
    let creator = mysql_text(&row.creator_hex);
    let data = mysql_blob(&row.data_hex);
    sql.push_str(&format!(
        "INSERT IGNORE INTO `XAssetsData` (`Hash`, `Data`) VALUES (UNHEX('{}'), {data});\n\
         INSERT INTO `XAssetsMeta` (`ID`, `Hash`, `Name`, `Description`, `AssetType`, `Local`, \
         `Temporary`, `CreateTime`, `AccessTime`, `AssetFlags`, `CreatorID`) VALUES ('{}', \
         UNHEX('{}'), {name}, {description}, {}, {local}, {temporary}, {now}, {now}, {}, \
         {creator});\n\
         INSERT INTO `assets` (`id`, `name`, `description`, `assetType`, `local`, `temporary`, \
         `data`, `create_time`, `access_time`, `asset_flags`, `CreatorID`) VALUES ('{}', \
         {name}, {description}, {}, {local}, {temporary}, {data}, {now}, {now}, {}, {creator});\n",
        row.hash_hex,
        row.id,
        row.hash_hex,
        row.type_code,
        row.flags,
        row.id,
        row.type_code,
        row.flags
    ));
}

#[cfg(any(target_os = "linux", test))]
fn push_postgres_row(sql: &mut String, row: &SeedAsset, now: i64) {
    let local = if row.local { "true" } else { "false" };
    let temporary = if row.temporary { "true" } else { "false" };
    let name = postgres_text(&row.name_hex);
    let description = postgres_text(&row.description_hex);
    let creator = postgres_text(&row.creator_hex);
    let data = postgres_blob(&row.data_hex);
    sql.push_str(&format!(
        "INSERT INTO xassetsdata (hash, data) VALUES (decode('{}','hex'), {data}) \
         ON CONFLICT (hash) DO NOTHING;\n\
         INSERT INTO xassetsmeta (id, hash, name, description, \"AssetType\", local, temporary, \
         create_time, access_time, asset_flags, creatorid) VALUES ('{}'::uuid, \
         decode('{}','hex'), {name}, {description}, {}, {local}, {temporary}, {now}, {now}, {}, \
         {creator});\n\
         INSERT INTO assets (id, name, description, \"assetType\", local, temporary, data, \
         create_time, access_time, asset_flags, creatorid) VALUES ('{}'::uuid, {name}, \
         {description}, {}, {local}, {temporary}, {data}, {now}, {now}, {}, {creator});\n",
        row.hash_hex,
        row.id,
        row.hash_hex,
        row.type_code,
        row.flags,
        row.id,
        row.type_code,
        row.flags
    ));
}

#[cfg(any(target_os = "linux", test))]
fn mysql_text(hex: &str) -> String {
    if hex.is_empty() {
        "''".to_string()
    } else {
        format!("CONVERT(UNHEX('{hex}') USING utf8mb3)")
    }
}

#[cfg(any(target_os = "linux", test))]
fn mysql_blob(hex: &str) -> String {
    if hex.is_empty() {
        "''".to_string()
    } else {
        format!("UNHEX('{hex}')")
    }
}

#[cfg(any(target_os = "linux", test))]
fn postgres_text(hex: &str) -> String {
    if hex.is_empty() {
        "''".to_string()
    } else {
        format!("convert_from(decode('{hex}','hex'), 'UTF8')")
    }
}

#[cfg(any(target_os = "linux", test))]
fn postgres_blob(hex: &str) -> String {
    if hex.is_empty() {
        "''::bytea".to_string()
    } else {
        format!("decode('{hex}','hex')")
    }
}

#[cfg(target_os = "linux")]
fn read_seed_rows(database: &Path) -> Result<Vec<SeedAsset>, String> {
    let output = Command::new("sqlite3")
        .arg(database)
        .arg(SEED_QUERY)
        .output()
        .map_err(|error| format!("could not read the library seed: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "could not read the library seed: {}",
            stderr.trim()
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "the library seed is not utf-8".to_string())?;
    let mut rows = Vec::new();
    for line in text.lines() {
        if !line.is_empty() {
            rows.push(seed_asset(line)?);
        }
    }
    if rows.is_empty() {
        return Err("the library seed has no assets".to_string());
    }
    if !rows.iter().any(|row| row.id == crate::assets::KELP) {
        return Err("the library seed has no kelp texture".to_string());
    }
    Ok(rows)
}

#[cfg(any(target_os = "linux", test))]
fn seed_asset(line: &str) -> Result<SeedAsset, String> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|_| "a library row is not json".to_string())?;
    let id = value
        .get("id")
        .and_then(|item| item.as_str())
        .ok_or("a library row has no id")?
        .trim()
        .to_ascii_lowercase();
    let data_hex = hex_field(&value, "data")?;
    let row = SeedAsset {
        id,
        name_hex: hex_field(&value, "name")?,
        description_hex: hex_field(&value, "description")?,
        type_code: int_field(&value, "type")?,
        local: int_field(&value, "local")? != 0,
        temporary: int_field(&value, "temporary")? != 0,
        flags: int_field(&value, "flags")?,
        creator_hex: hex_field(&value, "creator")?,
        hash_hex: sha256_hex(&data_hex)?,
        data_hex,
    };
    validate_row(&row)?;
    Ok(row)
}

#[cfg(any(target_os = "linux", test))]
fn hex_field(value: &serde_json::Value, key: &str) -> Result<String, String> {
    match value.get(key) {
        Some(serde_json::Value::Null) | None => Ok(String::new()),
        Some(serde_json::Value::String(text))
            if text.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            Ok(text.to_ascii_lowercase())
        }
        _ => Err("a library value is not hex".to_string()),
    }
}

#[cfg(any(target_os = "linux", test))]
fn int_field(value: &serde_json::Value, key: &str) -> Result<i64, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| format!("a library row has no {key}"))
}

#[cfg(any(target_os = "linux", test))]
fn sha256_hex(data_hex: &str) -> Result<String, String> {
    let bytes = hex_decode(data_hex)?;
    Ok(hex_encode(&Sha256::digest(&bytes)))
}

#[cfg(any(target_os = "linux", test))]
fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("a library value is not hex".to_string());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let high = hex_nibble(bytes[index])?;
        let low = hex_nibble(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Ok(out)
}

#[cfg(any(target_os = "linux", test))]
fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("a library value is not hex".to_string()),
    }
}

#[cfg(any(target_os = "linux", test))]
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(any(target_os = "linux", test))]
fn is_uuid(text: &str) -> bool {
    let groups = [8usize, 4, 4, 4, 12];
    let mut parts = text.split('-');
    for length in groups {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != length || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
    }
    parts.next().is_none()
}

#[cfg(target_os = "linux")]
struct SideReport {
    report: String,
    capture: PathBuf,
}

#[cfg(target_os = "linux")]
fn run_side(db: XassetDb, target: &DbTarget, role: &str, sql: &str) -> Result<SideReport, String> {
    let run = live::scratch_run();
    fs::create_dir_all(&run)
        .map_err(|error| format!("could not create {}: {error}", run.display()))?;
    apply_sql(db, target, sql, &run)?;
    let port = live::allocate_port()?;
    let name = format!("{}-{role}", db.label());
    let log = run.join(format!("xasset-{name}.log"));
    let mut server = start_side(db, target, &name, port, &log)?;
    crate::assets::wait_service(&mut server, &name, port, &log)?;
    let capture = run.join(format!("xasset-{name}.cap"));
    let upstream = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let (proxy_port, proxy) = crate::assets::spawn_proxy(upstream, &capture)?;
    let uri = format!("http://127.0.0.1:{proxy_port}");
    println!("live: assets {name} via {uri}");
    let report = live::run_against_assets(&uri, "assets");
    let proxy_result = proxy.finish();
    drop(server);
    proxy_result?;
    let report = report?;
    Ok(SideReport {
        report: format!("assets {name}:\n{report}"),
        capture,
    })
}

#[cfg(target_os = "linux")]
fn start_side(
    db: XassetDb,
    target: &DbTarget,
    name: &str,
    port: u16,
    log: &Path,
) -> Result<crate::assets::Owned, String> {
    let run = live::scratch_run();
    let csharp = name.ends_with("csharp");
    let ini_path = run.join(format!("xasset-{name}.ini"));
    let ini = service_ini(port, db.provider(), &ado(db, target, csharp), csharp);
    live::write_secret(&ini_path, &ini)?;
    let empty = run.join("xasset-empty");
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_arg = ini_path.to_str().ok_or("xasset ini path is not utf-8")?;
    let empty_arg = empty.to_str().ok_or("xasset config path is not utf-8")?;
    if csharp {
        let binary = live::grid_server_binary();
        if !binary.is_file() {
            return Err("run cargo xtask live build first".to_string());
        }
        crate::assets::spawn_owned(
            &binary,
            &[
                "--inimaster",
                ini_arg,
                "--inidirectory",
                empty_arg,
                "--console",
                "basic",
            ],
            binary.parent().unwrap_or(Path::new(".")),
            log,
            true,
            true,
        )
    } else {
        let binary = live::build_binary("ferroan-robust")?;
        crate::assets::spawn_owned(
            &binary,
            &["--inimaster", ini_arg, "--inidirectory", empty_arg],
            &repo_root(),
            log,
            false,
            true,
        )
    }
}

#[cfg(target_os = "linux")]
fn apply_sql(db: XassetDb, target: &DbTarget, sql: &str, run: &Path) -> Result<(), String> {
    let sql_path = run.join(format!("xasset-{}-seed.sql", db.label()));
    fs::write(&sql_path, sql).map_err(|error| format!("could not write the seed: {error}"))?;
    match db {
        XassetDb::Mysql => apply_mysql(target, &sql_path, run),
        XassetDb::Pgsql => apply_postgres(target, &sql_path),
    }
    .map_err(|error| format!("{error}. Load the schema dump into this database first"))
}

#[cfg(target_os = "linux")]
fn apply_mysql(target: &DbTarget, sql_path: &Path, run: &Path) -> Result<(), String> {
    let defaults = run.join("xasset-mysql-client.cnf");
    live::write_secret(
        &defaults,
        &format!("[client]\npassword={}\n", target.password),
    )?;
    let input =
        fs::File::open(sql_path).map_err(|error| format!("could not read the seed: {error}"))?;
    let port = target.port.to_string();
    let output = crate::sql_client::run_mysql(
        &defaults,
        &target.host,
        &port,
        &target.user,
        &target.database,
        &[],
        Stdio::from(input),
    );
    let _ = fs::remove_file(&defaults);
    let output = output?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "could not seed XAsset tables: {}",
        redact(&tail_text(&output.stderr), &target.password)
    ))
}

#[cfg(target_os = "linux")]
fn apply_postgres(target: &DbTarget, sql_path: &Path) -> Result<(), String> {
    let port = target.port.to_string();
    let output = crate::sql_client::run_psql(
        &target.password,
        &target.host,
        &port,
        &target.user,
        &target.database,
        sql_path,
        &[],
    )?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "could not seed XAsset tables: {}",
        redact(&tail_text(&output.stderr), &target.password)
    ))
}

#[cfg(target_os = "linux")]
fn tail_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let start = text.len().saturating_sub(800);
    text[start..].trim().to_string()
}

#[cfg(target_os = "linux")]
fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "***")
    }
}

#[cfg(target_os = "linux")]
fn unix_now() -> Result<i64, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "the clock is before the unix epoch".to_string())?
        .as_secs();
    i64::try_from(now).map_err(|_| "the clock does not fit a signed integer".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(local: bool) -> SeedAsset {
        SeedAsset {
            id: "2a4880b6-b7a3-690a-2049-bfbe38eafb9f".to_string(),
            name_hex: "6e".to_string(),
            description_hex: String::new(),
            type_code: 0,
            local,
            temporary: false,
            flags: 1,
            creator_hex: String::new(),
            data_hex: "00ff".to_string(),
            hash_hex: "ab".repeat(32),
        }
    }

    fn target() -> DbTarget {
        DbTarget {
            user: "user".to_string(),
            password: "pw".to_string(),
            host: "127.0.0.1".to_string(),
            port: 1,
            database: "assets".to_string(),
        }
    }

    #[test]
    fn the_success_line_is_a_match_without_a_known_difference_list() {
        let text = success_report("csharp", "ferroan", 3);
        assert!(text.contains("assets: match (3 http records)"));
        assert!(!text.contains("known differences"));
        for id in ["TB27", "TB28", "TB29", "TB30", "TB33"] {
            assert!(!text.contains(id), "{id}");
        }
    }

    #[test]
    fn seed_sql_uses_the_xasset_tables_and_keeps_values_as_hex() {
        let mysql = seed_sql(XassetDb::Mysql, &[sample(true)], 1).expect("mysql");
        assert!(mysql.contains("DELETE FROM `XAssetsMeta`"));
        assert!(mysql.contains("INSERT IGNORE INTO `XAssetsData`"));
        assert!(mysql.contains("UNHEX('00ff')"));
        assert!(mysql.contains("2a4880b6-b7a3-690a-2049-bfbe38eafb9f"));
        assert!(!mysql.contains("00ff,"));
        let postgres = seed_sql(XassetDb::Pgsql, &[sample(false)], 1).expect("postgres");
        assert!(postgres.contains("DELETE FROM xassetsmeta"));
        assert!(postgres.contains("ON CONFLICT (hash) DO NOTHING"));
        assert!(postgres.contains("\"AssetType\""));
        assert!(postgres.contains("\"assetType\""));
        assert!(postgres.contains("false"));
    }

    #[test]
    fn ado_omits_pooling_for_ferroan_and_sets_it_for_csharp() {
        let target = target();
        let ferroan = ado(XassetDb::Mysql, &target, false);
        let csharp = ado(XassetDb::Pgsql, &target, true);
        assert!(!ferroan.contains("Pooling"));
        assert!(csharp.contains("Pooling=false"));
        assert!(csharp.contains("SSL Mode=Disable"));
        let ini = service_ini(8000, XassetDb::Mysql.provider(), &ferroan, false);
        assert!(ini.contains("XAssetService"));
        assert!(!ini.contains("DefaultAssetLoader"));
        assert!(!ini.contains("Pooling"));
    }

    #[test]
    fn a_library_json_row_hashes_the_bytes() {
        let line = r#"{"id":"2A4880B6-B7A3-690A-2049-BFBE38EAFB9F","name":"6E","description":null,"type":0,"local":0,"temporary":1,"flags":0,"creator":"","data":"00ff"}"#;
        let row = seed_asset(line).expect("row");
        assert_eq!(row.id, "2a4880b6-b7a3-690a-2049-bfbe38eafb9f");
        assert!(row.temporary);
        assert!(!row.local);
        assert_eq!(row.hash_hex, hex_encode(&Sha256::digest([0x00, 0xff])));
    }

    #[test]
    fn the_database_url_parser_rejects_a_quote_and_keeps_the_port() {
        let parsed =
            parse_database_url("postgres://user:pw@127.0.0.1:15432/assets?sslmode=disable")
                .expect("url");
        assert_eq!(parsed.port, 15432);
        assert_eq!(parsed.database, "assets");
        assert!(parse_database_url("mysql://user:p\"w@127.0.0.1:1/db").is_err());
        assert!(parse_database_url("not a url").is_err());
    }
}
