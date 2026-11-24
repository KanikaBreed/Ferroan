// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! FSAssets live compare.
//!
//! `cargo xtask live assets --backend fsassets --db mysql|pgsql` copies the
//! loaded library into `fsassets` and gzip files under one `BaseDirectory`,
//! then runs texture and inventory against C# and Ferroan. Each service then
//! reads an asset the other stored on that same database and directory.
//!
//! A read of 10,000 assets is timed on each side with
//! `DaysBetweenAccessTimeUpdates` 365, so the interval does not also write
//! `access_time`. The texture run leaves the pin default of 0. The success
//! line is `assets: match`. TB45 is named only when this run's force probe
//! shows it. A miss status or an import page that differs fails the command.
//! Any other HTTP difference fails the command.

use std::process::ExitCode;

#[cfg(any(target_os = "linux", test))]
use sha2::{Digest, Sha256};
#[cfg(any(target_os = "linux", test))]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use flate2::Compression;
#[cfg(target_os = "linux")]
use flate2::write::GzEncoder;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

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
    'flags', asset_flags, \
    'data', hex(Data)) FROM assets;";

#[cfg(any(target_os = "linux", test))]
const BULK_COUNT: u32 = 10_000;
#[cfg(any(target_os = "linux", test))]
const TIMED_ACCESS_DAYS: i32 = 365;
#[cfg(any(target_os = "linux", test))]
const BULK_BODY: &[u8] = b"1";
#[cfg(any(target_os = "linux", test))]
const RUST_WROTE: &str = "f23e00c0-0000-4000-8000-000000000001";
#[cfg(any(target_os = "linux", test))]
const CSHARP_WROTE: &str = "f23e00c0-0000-4000-8000-000000000002";
#[cfg(any(target_os = "linux", test))]
const FORCE_ID: &str = "f23e00c0-0000-4000-8000-000000000003";
#[cfg(any(target_os = "linux", test))]
const RUST_BODY: &[u8] = b"fsasset-rust-wrote";
#[cfg(any(target_os = "linux", test))]
const CSHARP_BODY: &[u8] = b"fsasset-csharp-wrote";
#[cfg(any(target_os = "linux", test))]
const FORCE_OLD: &[u8] = b"fsasset-force-old";
#[cfg(any(target_os = "linux", test))]
const FORCE_NEW: &[u8] = b"fsasset-force-new";
#[cfg(any(target_os = "linux", test))]
const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FsassetDb {
    Mysql,
    Pgsql,
}

impl FsassetDb {
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

    #[cfg(any(target_os = "linux", test))]
    fn postgres(self) -> bool {
        self == Self::Pgsql
    }
}

pub(crate) fn compare(db: FsassetDb) -> ExitCode {
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

fn compare_inner(db: FsassetDb) -> Result<String, String> {
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
fn compare_linux(db: FsassetDb) -> Result<String, String> {
    if let Some(pid) = live::standalone_is_up()? {
        return Err(format!(
            "the harness is already up (pid {pid}); run cargo xtask live down first"
        ));
    }
    let _state = crate::grid_state::claim()?;
    live::ensure_region_built()?;
    let target = database_target(db)?;
    let run = live::scratch_run();
    fs::create_dir_all(&run)
        .map_err(|error| format!("could not create {}: {error}", run.display()))?;
    let base = run.join(format!("fsasset-base-{}", db.label()));
    if base.exists() {
        fs::remove_dir_all(&base)
            .map_err(|error| format!("could not reset {}: {error}", base.display()))?;
    }
    fs::create_dir_all(&base)
        .map_err(|error| format!("could not create {}: {error}", base.display()))?;
    let base_arg = base.to_str().ok_or("fsassets base path is not utf-8")?;
    let seed = crate::assets::seed_library_database()?;
    let rows = read_seed_rows(&seed)?;
    write_library_files(&base, &rows)?;
    let now = unix_now()?;
    let library = library_sql(db, &rows, now)?;
    let csharp = run_side(db, &target, "csharp", &library, base_arg, 0)?;
    let ferroan = run_side(db, &target, "ferroan", &library, base_arg, 0)?;
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
    let timed = time_and_cross(db, &target, base_arg, now)?;
    classify_missing(timed.csharp_miss, timed.ferroan_miss)?;
    let tb45 = classify_force(timed.csharp_kept_hash, timed.ferroan_wrote_hash)?;
    probe_import(db, &target, &run)?;
    Ok(success_report(
        &csharp.report,
        &ferroan.report,
        left.lines().count(),
        timed.csharp_ms,
        timed.ferroan_ms,
        &difference_ids(tb45),
    ))
}

#[cfg(any(target_os = "linux", test))]
fn success_report(
    csharp: &str,
    ferroan: &str,
    records: usize,
    csharp_ms: u128,
    ferroan_ms: u128,
    differences: &[&str],
) -> String {
    let mut text = format!(
        "{csharp}\n{ferroan}\nassets: match ({records} http records)\ncross-read ok\n\
         10000-asset read: csharp {csharp_ms} ms, ferroan {ferroan_ms} ms"
    );
    if !differences.is_empty() {
        text.push_str("\nknown differences: ");
        text.push_str(&differences.join(", "));
    }
    text
}

#[cfg(any(target_os = "linux", test))]
fn difference_ids(tb45: bool) -> Vec<&'static str> {
    if tb45 { vec!["TB45"] } else { Vec::new() }
}

/// A non-uuid get is a miss on both sides at Pin7. A status that differs is
/// a failure, not a listed difference.
#[cfg(any(target_os = "linux", test))]
fn classify_missing(csharp: u16, ferroan: u16) -> Result<(), String> {
    if csharp == ferroan {
        return Ok(());
    }
    Err(format!(
        "asset miss status differs: csharp {csharp}, ferroan {ferroan}"
    ))
}

/// C# `Store` ignores `force`, so an ordinary store leaves the hash. Ferroan's
/// force update writes the new hash and `access_time` only.
#[cfg(any(target_os = "linux", test))]
fn classify_force(csharp_kept_hash: bool, ferroan_wrote_hash: bool) -> Result<bool, String> {
    match (csharp_kept_hash, ferroan_wrote_hash) {
        (true, true) => Ok(true),
        (false, true) => Ok(false),
        (true, false) => Err("ferroan force update did not change the hash".to_string()),
        (false, false) => Err("force import changed neither hash".to_string()),
    }
}

/// Pin7 PostgreSQL import returns `count` rows from row `start`, ordered by
/// id. Ferroan uses that same page. MySQL is `LIMIT start, count` with no
/// order on the pin, so only the row count is compared.
#[cfg(any(target_os = "linux", test))]
fn pages_match(postgres: bool, pin: &str, ferroan: &str) -> Result<(), String> {
    if postgres {
        if page_ids(pin) == page_ids(ferroan) {
            return Ok(());
        }
        return Err(format!(
            "import page ids differ on postgresql: pin {} rows, ferroan {} rows",
            row_count(pin),
            row_count(ferroan)
        ));
    }
    if row_count(pin) == row_count(ferroan) {
        return Ok(());
    }
    Err(format!(
        "import page counts differ on mysql: pin {} rows, ferroan {} rows",
        row_count(pin),
        row_count(ferroan)
    ))
}

#[cfg(any(target_os = "linux", test))]
fn pin_page_sql(postgres: bool) -> &'static str {
    if postgres {
        "SELECT id::text FROM ferroan_q23e_import ORDER BY id LIMIT 2 OFFSET 0"
    } else {
        "SELECT id FROM ferroan_q23e_import LIMIT 0, 2"
    }
}

#[cfg(any(target_os = "linux", test))]
fn ferroan_page_sql(postgres: bool) -> &'static str {
    if postgres {
        "SELECT id::text FROM ferroan_q23e_import ORDER BY id LIMIT 2 OFFSET 0"
    } else {
        "SELECT id FROM ferroan_q23e_import ORDER BY id LIMIT 0, 2"
    }
}

#[cfg(any(target_os = "linux", test))]
fn force_sql(postgres: bool, id: &str, hash: &str, now: i64) -> String {
    if postgres {
        format!(
            "UPDATE fsassets SET \"hash\" = '{hash}', \"access_time\" = {now} \
             WHERE \"id\" = '{id}'::uuid"
        )
    } else {
        format!(
            "UPDATE fsassets SET hash = '{hash}', access_time = UNIX_TIMESTAMP() WHERE id = '{id}'"
        )
    }
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
fn database_target(db: FsassetDb) -> Result<DbTarget, String> {
    let name = db.env_name();
    let raw = std::env::var(name).map_err(|_| format!("{name} is unset"))?;
    if raw.is_empty() {
        return Err(format!("{name} is unset"));
    }
    parse_database_url(&raw).map_err(|error| format!("{name}: {error}"))
}

/// C# keeps `Pooling=false`. Ferroan rejects that key and uses its own pool.
#[cfg(any(target_os = "linux", test))]
fn ado(db: FsassetDb, target: &DbTarget, pooling_false: bool) -> String {
    let mut text = match db {
        FsassetDb::Mysql => format!(
            "Server={};Port={};Database={};User ID={};Password={};SslMode=None",
            target.host, target.port, target.database, target.user, target.password
        ),
        FsassetDb::Pgsql => format!(
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
struct IniSpec<'a> {
    port: u16,
    provider: &'a str,
    ado: &'a str,
    grid: bool,
    base: &'a str,
    spool: &'a str,
    access_days: i32,
}

#[cfg(any(target_os = "linux", test))]
fn service_ini(spec: &IniSpec<'_>) -> String {
    let asset = format!(
        "[AssetService]\n    \
         LocalServiceModule = \"OpenSim.Services.FSAssetService.dll:FSAssetConnector\"\n    \
         StorageProvider = \"{provider}\"\n    \
         ConnectionString = \"{ado}\"\n    \
         BaseDirectory = \"{base}\"\n    \
         SpoolDirectory = \"{spool}\"\n    \
         DaysBetweenAccessTimeUpdates = {days}\n    \
         ShowConsoleStats = false\n",
        provider = spec.provider,
        ado = spec.ado,
        base = spec.base,
        spool = spec.spool,
        days = spec.access_days,
    );
    if !spec.grid {
        return format!(
            "[Network]\n    port = {port}\n    bind_ip_address = \"127.0.0.1\"\n\n{asset}",
            port = spec.port
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
         ConnectionString = \"{ado}\"\n\n{asset}",
        port = spec.port,
        provider = spec.provider,
        ado = spec.ado,
    )
}

#[cfg(any(target_os = "linux", test))]
struct SeedAsset {
    id: String,
    name_hex: String,
    description_hex: String,
    type_code: i64,
    flags: i64,
    hash_hex: String,
    data: Vec<u8>,
}

#[cfg(any(target_os = "linux", test))]
fn library_sql(db: FsassetDb, rows: &[SeedAsset], now: i64) -> Result<String, String> {
    if i32::try_from(now).is_err() {
        return Err("the clock does not fit the asset time column".to_string());
    }
    let mut sql = String::from(match db {
        FsassetDb::Mysql => "START TRANSACTION;\nDELETE FROM `fsassets`;\n",
        FsassetDb::Pgsql => "BEGIN;\nDELETE FROM fsassets;\n",
    });
    for row in rows {
        validate_row(row)?;
        push_row(&mut sql, db, row, now);
    }
    sql.push_str("COMMIT;\n");
    Ok(sql)
}

#[cfg(any(target_os = "linux", test))]
fn timing_sql(
    db: FsassetDb,
    now: i64,
    bulk_hash: &str,
    force_hash: &str,
) -> Result<String, String> {
    if !is_hash(bulk_hash) || !is_hash(force_hash) || i32::try_from(now).is_err() {
        return Err("the timed seed has a bad hash".to_string());
    }
    let mut sql = String::from(match db {
        FsassetDb::Mysql => "START TRANSACTION;\nDELETE FROM `fsassets`;\n",
        FsassetDb::Pgsql => "BEGIN;\nDELETE FROM fsassets;\n",
    });
    for index in 0..BULK_COUNT {
        let row = SeedAsset {
            id: bulk_id(index),
            name_hex: String::new(),
            description_hex: String::new(),
            type_code: 0,
            flags: 0,
            hash_hex: bulk_hash.to_string(),
            data: Vec::new(),
        };
        validate_row(&row)?;
        push_row(&mut sql, db, &row, now);
    }
    let force = SeedAsset {
        id: FORCE_ID.to_string(),
        name_hex: String::new(),
        description_hex: String::new(),
        type_code: 0,
        flags: 0,
        hash_hex: force_hash.to_string(),
        data: Vec::new(),
    };
    validate_row(&force)?;
    push_row(&mut sql, db, &force, now);
    sql.push_str("COMMIT;\n");
    Ok(sql)
}

#[cfg(any(target_os = "linux", test))]
fn push_row(sql: &mut String, db: FsassetDb, row: &SeedAsset, now: i64) {
    match db {
        FsassetDb::Mysql => {
            let name = mysql_text(&row.name_hex);
            let description = mysql_text(&row.description_hex);
            sql.push_str(&format!(
                "INSERT INTO `fsassets` (`id`, `name`, `description`, `type`, `hash`, \
                 `create_time`, `access_time`, `asset_flags`) VALUES ('{id}', {name}, \
                 {description}, {type_code}, '{hash}', {now}, {now}, {flags});\n",
                id = row.id,
                type_code = row.type_code,
                hash = row.hash_hex,
                flags = row.flags,
            ));
        }
        FsassetDb::Pgsql => {
            sql.push_str(&format!(
                "INSERT INTO fsassets (id, type, hash, create_time, access_time, asset_flags) \
                 VALUES ('{id}'::uuid, {type_code}, '{hash}', {now}, {now}, {flags});\n",
                id = row.id,
                type_code = row.type_code,
                hash = row.hash_hex,
                flags = row.flags,
            ));
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn validate_row(row: &SeedAsset) -> Result<(), String> {
    if !is_uuid(&row.id) {
        return Err("an fsassets row has an id that is not a uuid".to_string());
    }
    if !is_hash(&row.hash_hex) {
        return Err("an fsassets hash is not 32 bytes".to_string());
    }
    for hex in [&row.name_hex, &row.description_hex] {
        if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) || !hex.len().is_multiple_of(2) {
            return Err("an fsassets name is not hex".to_string());
        }
    }
    if !(-128..=127).contains(&row.type_code) || i32::try_from(row.flags).is_err() {
        return Err("an fsassets row has a value this table cannot store".to_string());
    }
    Ok(())
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
fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
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

#[cfg(any(target_os = "linux", test))]
fn bulk_id(index: u32) -> String {
    format!("f23e0000-0000-4000-8000-{index:012x}")
}

#[cfg(any(target_os = "linux", test))]
fn hash_to_file(hash: &str) -> PathBuf {
    let chars: Vec<char> = hash.chars().collect();
    if chars.len() < 10 {
        return PathBuf::from("junkyard").join(hash);
    }
    let take = |start: usize, len: usize| chars[start..start + len].iter().collect::<String>();
    PathBuf::from(take(0, 2))
        .join(take(2, 2))
        .join(take(4, 2))
        .join(take(6, 4))
        .join(hash)
}

#[cfg(any(target_os = "linux", test))]
fn gzip_file(disk: &Path) -> PathBuf {
    let mut name = disk.as_os_str().to_os_string();
    name.push(".gz");
    PathBuf::from(name)
}

#[cfg(target_os = "linux")]
fn write_library_files(base: &Path, rows: &[SeedAsset]) -> Result<(), String> {
    for row in rows {
        if row.hash_hex.eq_ignore_ascii_case(EMPTY_HASH) {
            continue;
        }
        write_gzip(base, &row.hash_hex, &row.data)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_gzip(base: &Path, hash: &str, data: &[u8]) -> Result<(), String> {
    let gz = gzip_file(&base.join(hash_to_file(hash)));
    if let Some(parent) = gz.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let file = fs::File::create(&gz)
        .map_err(|error| format!("could not create {}: {error}", gz.display()))?;
    let mut gzip = GzEncoder::new(file, Compression::default());
    gzip.write_all(data)
        .map_err(|error| format!("could not compress an asset: {error}"))?;
    let file = gzip
        .finish()
        .map_err(|error| format!("could not finish an asset file: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("could not store an asset file: {error}"))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn wait_gzip(base: &str, hash: &str) -> Result<(), String> {
    let path = gzip_file(&Path::new(base).join(hash_to_file(hash)));
    for _ in 0..40 {
        if path.is_file() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("the asset writer did not publish the file".to_string())
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
    let data = hex_decode(&hex_field(&value, "data")?)?;
    let row = SeedAsset {
        id,
        name_hex: mysql_column_hex(&hex_field(&value, "name")?)?,
        description_hex: mysql_column_hex(&hex_field(&value, "description")?)?,
        type_code: int_field(&value, "type")?,
        flags: int_field(&value, "flags")?,
        hash_hex: sha256_bytes(&data),
        data,
    };
    validate_row(&row)?;
    Ok(row)
}

#[cfg(any(target_os = "linux", test))]
fn mysql_column_hex(raw_hex: &str) -> Result<String, String> {
    let bytes = hex_decode(raw_hex)?;
    let text = String::from_utf8(bytes).map_err(|_| "a library name is not utf-8".to_string())?;
    let mut kept = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        if u32::from(ch) > 0xFFFF {
            continue;
        }
        if count == 64 {
            break;
        }
        kept.push(ch);
        count += 1;
    }
    Ok(hex_encode(kept.as_bytes()))
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
fn sha256_bytes(data: &[u8]) -> String {
    hex_encode(&Sha256::digest(data))
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

#[cfg(target_os = "linux")]
struct SideReport {
    report: String,
    capture: PathBuf,
}

#[cfg(target_os = "linux")]
fn run_side(
    db: FsassetDb,
    target: &DbTarget,
    role: &str,
    sql: &str,
    base: &str,
    access_days: i32,
) -> Result<SideReport, String> {
    let run = live::scratch_run();
    apply_sql(db, target, sql, &run)?;
    let port = live::allocate_port()?;
    let name = format!("{}-{role}", db.label());
    let log = run.join(format!("fsasset-{name}.log"));
    let spool = run.join(format!("fsasset-spool-{name}"));
    fs::create_dir_all(&spool)
        .map_err(|error| format!("could not create {}: {error}", spool.display()))?;
    let spool_arg = spool.to_str().ok_or("fsassets spool path is not utf-8")?;
    let mut server = start_side(&StartSpec {
        db,
        target,
        name: &name,
        port,
        log: &log,
        base,
        spool: spool_arg,
        access_days,
    })?;
    crate::assets::wait_service(&mut server, &name, port, &log)?;
    let capture = run.join(format!("fsasset-{name}.cap"));
    let upstream = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let (proxy_port, proxy) = crate::assets::spawn_proxy(upstream, &capture)?;
    let uri = format!("http://127.0.0.1:{proxy_port}");
    println!("live: assets {name} via {uri}");
    let outcome = live::run_against_assets(&uri, "assets");
    let proxy_result = proxy.finish();
    drop(server);
    proxy_result?;
    let report = outcome?;
    Ok(SideReport {
        report: format!("assets {name}:\n{report}"),
        capture,
    })
}

#[cfg(target_os = "linux")]
struct StartSpec<'a> {
    db: FsassetDb,
    target: &'a DbTarget,
    name: &'a str,
    port: u16,
    log: &'a Path,
    base: &'a str,
    spool: &'a str,
    access_days: i32,
}

#[cfg(target_os = "linux")]
fn start_side(spec: &StartSpec<'_>) -> Result<crate::assets::Owned, String> {
    let run = live::scratch_run();
    let csharp = spec.name.ends_with("csharp");
    let ini_path = run.join(format!("fsasset-{}.ini", spec.name));
    let ini = service_ini(&IniSpec {
        port: spec.port,
        provider: spec.db.provider(),
        ado: &ado(spec.db, spec.target, csharp),
        grid: csharp,
        base: spec.base,
        spool: spec.spool,
        access_days: spec.access_days,
    });
    live::write_secret(&ini_path, &ini)?;
    let empty = run.join(format!("fsasset-empty-{}", spec.name));
    fs::create_dir_all(&empty)
        .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
    let ini_arg = ini_path.to_str().ok_or("fsassets ini path is not utf-8")?;
    let empty_arg = empty.to_str().ok_or("fsassets config path is not utf-8")?;
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
            spec.log,
            true,
            true,
        )
    } else {
        let binary = live::build_binary("ferroan-robust")?;
        crate::assets::spawn_owned(
            &binary,
            &["--inimaster", ini_arg, "--inidirectory", empty_arg],
            &repo_root(),
            spec.log,
            false,
            true,
        )
    }
}

#[cfg(target_os = "linux")]
struct TimedCross {
    csharp_ms: u128,
    ferroan_ms: u128,
    csharp_miss: u16,
    ferroan_miss: u16,
    csharp_kept_hash: bool,
    ferroan_wrote_hash: bool,
}

#[cfg(target_os = "linux")]
fn time_and_cross(
    db: FsassetDb,
    target: &DbTarget,
    base: &str,
    now: i64,
) -> Result<TimedCross, String> {
    let bulk_hash = sha256_bytes(BULK_BODY);
    let old_hash = sha256_bytes(FORCE_OLD);
    let new_hash = sha256_bytes(FORCE_NEW);
    let sql = timing_sql(db, now, &bulk_hash, &old_hash)?;
    let run = live::scratch_run();
    apply_sql(db, target, &sql, &run)?;
    write_gzip(Path::new(base), &bulk_hash, BULK_BODY)?;
    write_gzip(Path::new(base), &old_hash, FORCE_OLD)?;
    let rust_port = live::allocate_port()?;
    let csharp_port = live::allocate_port()?;
    let rust_log = run.join(format!("fsasset-{}-timed-ferroan.log", db.label()));
    let csharp_log = run.join(format!("fsasset-{}-timed-csharp.log", db.label()));
    let rust_spool = run.join(format!("fsasset-timed-spool-{}-ferroan", db.label()));
    let csharp_spool = run.join(format!("fsasset-timed-spool-{}-csharp", db.label()));
    fs::create_dir_all(&rust_spool)
        .map_err(|error| format!("could not create {}: {error}", rust_spool.display()))?;
    fs::create_dir_all(&csharp_spool)
        .map_err(|error| format!("could not create {}: {error}", csharp_spool.display()))?;
    let rust_spool_arg = rust_spool
        .to_str()
        .ok_or("fsassets spool path is not utf-8")?;
    let csharp_spool_arg = csharp_spool
        .to_str()
        .ok_or("fsassets spool path is not utf-8")?;
    let ferroan_name = format!("{}-timed-ferroan", db.label());
    let csharp_name = format!("{}-timed-csharp", db.label());
    let mut ferroan = start_side(&StartSpec {
        db,
        target,
        name: &ferroan_name,
        port: rust_port,
        log: &rust_log,
        base,
        spool: rust_spool_arg,
        access_days: TIMED_ACCESS_DAYS,
    })?;
    let started = start_side(&StartSpec {
        db,
        target,
        name: &csharp_name,
        port: csharp_port,
        log: &csharp_log,
        base,
        spool: csharp_spool_arg,
        access_days: TIMED_ACCESS_DAYS,
    });
    let mut csharp = match started {
        Ok(server) => server,
        Err(error) => {
            drop(ferroan);
            return Err(error);
        }
    };
    let ready = crate::assets::wait_service(&mut ferroan, "fsasset-ferroan", rust_port, &rust_log)
        .and_then(|()| {
            crate::assets::wait_service(&mut csharp, "fsasset-csharp", csharp_port, &csharp_log)
        });
    let outcome = ready.and_then(|()| {
        cross_store(rust_port, base, RUST_WROTE, RUST_BODY, csharp_port)?;
        cross_store(csharp_port, base, CSHARP_WROTE, CSHARP_BODY, rust_port)?;
        println!("live: fsassets reading {BULK_COUNT} assets");
        let csharp_ms = time_reads(csharp_port)?;
        let ferroan_ms = time_reads(rust_port)?;
        let csharp_miss = http_status(csharp_port, "/assets/nope")?;
        let ferroan_miss = http_status(rust_port, "/assets/nope")?;
        post_asset(csharp_port, FORCE_ID, "force", FORCE_NEW)?;
        Ok(TimedCross {
            csharp_ms,
            ferroan_ms,
            csharp_miss,
            ferroan_miss,
            csharp_kept_hash: false,
            ferroan_wrote_hash: false,
        })
    });
    drop(csharp);
    drop(ferroan);
    let mut timed = outcome?;
    let stored = select_hash(db, target, &run, FORCE_ID)?;
    timed.csharp_kept_hash = stored.eq_ignore_ascii_case(&old_hash);
    if !is_hash(&new_hash) || !is_uuid(FORCE_ID) {
        return Err("the force update is not a uuid and hash".to_string());
    }
    apply_sql(
        db,
        target,
        &force_sql(db.postgres(), FORCE_ID, &new_hash, now),
        &run,
    )?;
    let updated = select_hash(db, target, &run, FORCE_ID)?;
    timed.ferroan_wrote_hash = updated.eq_ignore_ascii_case(&new_hash);
    Ok(timed)
}

#[cfg(target_os = "linux")]
fn cross_store(writer: u16, base: &str, id: &str, body: &[u8], reader: u16) -> Result<(), String> {
    post_asset(writer, id, "cross", body)?;
    wait_gzip(base, &sha256_bytes(body))?;
    let reply = http_get(reader, &format!("/assets/{id}/data"))?;
    if reply.status != 200 || reply.body != body {
        return Err(format!(
            "cross-read of {id} status {} length {}",
            reply.status,
            reply.body.len()
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn post_asset(port: u16, id: &str, name: &str, body: &[u8]) -> Result<(), String> {
    let xml = asset_xml(id, name, body);
    let reply = http_exchange(port, &http_request("POST", "/assets", port, &xml), false)?;
    if reply.status != 200 {
        return Err(format!("asset store of {id} status {}", reply.status));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn time_reads(port: u16) -> Result<u128, String> {
    let started = Instant::now();
    let mut conn = Conn::connect(port)?;
    for index in 0..BULK_COUNT {
        if index.is_multiple_of(2000) {
            println!("live: fsassets read {index}");
        }
        let path = format!("/assets/{}/data", bulk_id(index));
        let reply = conn.get(&http_request("GET", &path, port, &[]))?;
        if reply.status != 200 || reply.body.as_slice() != BULK_BODY {
            return Err(format!(
                "asset {index} read status {} ({} bytes)",
                reply.status,
                reply.body.len()
            ));
        }
    }
    Ok(started.elapsed().as_millis())
}

#[cfg(target_os = "linux")]
fn http_status(port: u16, path: &str) -> Result<u16, String> {
    Ok(http_get(port, path)?.status)
}

#[cfg(target_os = "linux")]
fn http_get(port: u16, path: &str) -> Result<HttpReply, String> {
    http_exchange(port, &http_request("GET", path, port, &[]), true)
}

#[cfg(target_os = "linux")]
fn http_exchange(port: u16, request: &[u8], retry_get: bool) -> Result<HttpReply, String> {
    let mut conn = Conn::connect(port)?;
    match conn.send(request) {
        Ok(reply) => Ok(reply),
        Err(error) if retry_get => {
            let mut again = Conn::connect(port)?;
            again.send(request).map_err(|_| error)
        }
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
    dead: bool,
}

#[cfg(target_os = "linux")]
impl Conn {
    fn connect(port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .map_err(|_| "could not connect to the asset service".to_string())?;
        let _ = stream.set_nodelay(true);
        let timeout = Some(Duration::from_secs(20));
        stream
            .set_read_timeout(timeout)
            .map_err(|_| "could not time the asset read".to_string())?;
        stream
            .set_write_timeout(timeout)
            .map_err(|_| "could not time the asset write".to_string())?;
        Ok(Self {
            stream,
            buf: Vec::new(),
            dead: false,
        })
    }

    fn get(&mut self, request: &[u8]) -> Result<HttpReply, String> {
        match self.send(request) {
            Ok(reply) => Ok(reply),
            Err(_) => {
                self.reconnect()?;
                self.send(request)
            }
        }
    }

    fn send(&mut self, request: &[u8]) -> Result<HttpReply, String> {
        if self.dead {
            self.reconnect()?;
        }
        self.stream
            .write_all(request)
            .map_err(|_| "could not write to the asset service".to_string())?;
        let reply = self.read_reply()?;
        if reply.close {
            self.dead = true;
        }
        Ok(reply)
    }

    fn reconnect(&mut self) -> Result<(), String> {
        let port = self
            .stream
            .peer_addr()
            .map_err(|_| "could not reconnect to the asset service".to_string())?
            .port();
        *self = Self::connect(port)?;
        Ok(())
    }

    fn read_reply(&mut self) -> Result<HttpReply, String> {
        loop {
            if let Some(taken) = take_response(&self.buf)? {
                self.buf.drain(..taken.consumed);
                return Ok(HttpReply {
                    status: taken.status,
                    body: taken.body,
                    close: taken.close,
                });
            }
            let mut tmp = [0u8; 8192];
            let read = self.stream.read(&mut tmp);
            let count = match read {
                Ok(0) => return Err("the asset service closed the connection".to_string()),
                Ok(count) => count,
                Err(_) => return Err("the asset service did not answer".to_string()),
            };
            self.buf.extend_from_slice(&tmp[..count]);
            if self.buf.len() > 2 * 1024 * 1024 {
                return Err("the asset response is too large".to_string());
            }
        }
    }
}

#[cfg(any(target_os = "linux", test))]
struct HttpReply {
    status: u16,
    body: Vec<u8>,
    close: bool,
}

#[cfg(any(target_os = "linux", test))]
struct Taken {
    status: u16,
    body: Vec<u8>,
    close: bool,
    consumed: usize,
}

#[cfg(any(target_os = "linux", test))]
fn take_response(buf: &[u8]) -> Result<Option<Taken>, String> {
    let Some(split) = find_crlf_crlf(buf) else {
        if buf.len() > 64 * 1024 {
            return Err("the asset response header is too large".to_string());
        }
        return Ok(None);
    };
    let header = std::str::from_utf8(&buf[..split])
        .map_err(|_| "the asset response header is not utf-8".to_string())?;
    let mut lines = header.split("\r\n");
    let status = parse_status(lines.next().unwrap_or(""))?;
    let mut length = None;
    let mut close = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "the asset response has a bad length".to_string())?,
            );
        } else if name.eq_ignore_ascii_case("connection")
            && value.trim().eq_ignore_ascii_case("close")
        {
            close = true;
        }
    }
    let Some(length) = length else {
        return Err("the asset response has no content length".to_string());
    };
    if length > 1024 * 1024 {
        return Err("the asset response is too large".to_string());
    }
    let start = split + 4;
    let end = start + length;
    if buf.len() < end {
        return Ok(None);
    }
    Ok(Some(Taken {
        status,
        body: buf[start..end].to_vec(),
        close,
        consumed: end,
    }))
}

#[cfg(any(target_os = "linux", test))]
fn parse_status(line: &str) -> Result<u16, String> {
    let mut parts = line.split_whitespace();
    let _version = parts.next();
    parts
        .next()
        .and_then(|text| text.parse::<u16>().ok())
        .ok_or_else(|| "the asset response has no status".to_string())
}

#[cfg(any(target_os = "linux", test))]
fn find_crlf_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

#[cfg(any(target_os = "linux", test))]
fn http_request(method: &str, path: &str, port: u16, body: &[u8]) -> Vec<u8> {
    let mut text =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: keep-alive\r\n");
    if method == "POST" {
        text.push_str("Content-Type: text/xml\r\n");
        text.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    text.push_str("\r\n");
    let mut bytes = text.into_bytes();
    if method == "POST" {
        bytes.extend_from_slice(body);
    }
    bytes
}

#[cfg(any(target_os = "linux", test))]
fn asset_xml(id: &str, name: &str, data: &[u8]) -> Vec<u8> {
    let encoded = base64(data);
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <AssetBase xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n  \
         <Data>{encoded}</Data>\n  <FullID>\n    <Guid>{id}</Guid>\n  </FullID>\n  \
         <ID>{id}</ID>\n  <Name>{name}</Name>\n  <Description>cross</Description>\n  \
         <Type>0</Type>\n  <UploadAttempts>0</UploadAttempts>\n  \
         <Local>false</Local>\n  <Temporary>false</Temporary>\n  \
         <CreatorID></CreatorID>\n  <Flags>Normal</Flags>\n</AssetBase>"
    );
    let mut out = vec![0xEF, 0xBB, 0xBF];
    out.extend_from_slice(text.as_bytes());
    out
}

#[cfg(any(target_os = "linux", test))]
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= data.len() {
        let word = (u32::from(data[index]) << 16)
            | (u32::from(data[index + 1]) << 8)
            | u32::from(data[index + 2]);
        out.push(TABLE[((word >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((word >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((word >> 6) & 0x3f) as usize] as char);
        out.push(TABLE[(word & 0x3f) as usize] as char);
        index += 3;
    }
    let rest = data.len() - index;
    if rest == 1 {
        let word = u32::from(data[index]) << 16;
        out.push(TABLE[((word >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((word >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let word = (u32::from(data[index]) << 16) | (u32::from(data[index + 1]) << 8);
        out.push(TABLE[((word >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((word >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((word >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(target_os = "linux")]
fn probe_import(db: FsassetDb, target: &DbTarget, run: &Path) -> Result<(), String> {
    let create = import_seed_sql(db.postgres());
    apply_sql(db, target, create, run)?;
    let pin = query_sql(db, target, run, pin_page_sql(db.postgres()));
    let ferroan = query_sql(db, target, run, ferroan_page_sql(db.postgres()));
    let dropped = apply_sql(
        db,
        target,
        "DROP TABLE IF EXISTS ferroan_q23e_import;\n",
        run,
    );
    let pin = pin?;
    let ferroan = ferroan?;
    dropped?;
    pages_match(db.postgres(), &pin, &ferroan)
}

#[cfg(any(target_os = "linux", test))]
fn import_seed_sql(postgres: bool) -> &'static str {
    if postgres {
        "DROP TABLE IF EXISTS ferroan_q23e_import;\n\
         CREATE TABLE ferroan_q23e_import (id uuid PRIMARY KEY);\n\
         INSERT INTO ferroan_q23e_import (id) VALUES \
         ('f23e00a0-0000-4000-8000-000000000001'), \
         ('f23e00a0-0000-4000-8000-000000000002'), \
         ('f23e00a0-0000-4000-8000-000000000003');\n"
    } else {
        "DROP TABLE IF EXISTS ferroan_q23e_import;\n\
         CREATE TABLE ferroan_q23e_import (id char(36) PRIMARY KEY);\n\
         INSERT INTO ferroan_q23e_import (id) VALUES \
         ('f23e00a0-0000-4000-8000-000000000001'), \
         ('f23e00a0-0000-4000-8000-000000000002'), \
         ('f23e00a0-0000-4000-8000-000000000003');\n"
    }
}

#[cfg(any(target_os = "linux", test))]
fn row_count(text: &str) -> usize {
    page_ids(text).len()
}

#[cfg(any(target_os = "linux", test))]
fn page_ids(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(target_os = "linux")]
fn select_hash(db: FsassetDb, target: &DbTarget, run: &Path, id: &str) -> Result<String, String> {
    if !is_uuid(id) {
        return Err("the force id is not a uuid".to_string());
    }
    let sql = if db.postgres() {
        format!("SELECT TRIM(hash) FROM fsassets WHERE id = '{id}'::uuid")
    } else {
        format!("SELECT TRIM(hash) FROM fsassets WHERE id = '{id}'")
    };
    let text = query_sql(db, target, run, &sql)?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let hash = lines
        .next()
        .ok_or("the force row is missing")?
        .trim()
        .to_ascii_lowercase();
    if lines.next().is_some() || !is_hash(&hash) {
        return Err("the force row hash is not one sha-256".to_string());
    }
    Ok(hash)
}

#[cfg(target_os = "linux")]
fn apply_sql(db: FsassetDb, target: &DbTarget, sql: &str, run: &Path) -> Result<(), String> {
    let _ = query_sql(db, target, run, sql)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn query_sql(db: FsassetDb, target: &DbTarget, run: &Path, sql: &str) -> Result<String, String> {
    // The library seed is the asset bytes as hex. Passing that as an argument
    // exceeds the command-line limit, so the client reads a file.
    let sql_path = run.join(format!("fsasset-{}-statement.sql", db.label()));
    fs::write(&sql_path, sql).map_err(|error| format!("could not write the statement: {error}"))?;
    let result = match db {
        FsassetDb::Mysql => query_mysql(target, run, &sql_path),
        FsassetDb::Pgsql => query_postgres(target, &sql_path),
    };
    let _ = fs::remove_file(&sql_path);
    result.map_err(|error| format!("{error}. Load the schema dump into this database first"))
}

#[cfg(target_os = "linux")]
fn query_mysql(target: &DbTarget, run: &Path, sql_path: &Path) -> Result<String, String> {
    let defaults = run.join("fsasset-mysql-client.cnf");
    live::write_secret(
        &defaults,
        &format!("[client]\npassword={}\n", target.password),
    )?;
    let input = fs::File::open(sql_path)
        .map_err(|error| format!("could not read the statement: {error}"))?;
    let defaults_arg = defaults
        .to_str()
        .ok_or("mysql defaults path is not utf-8")?;
    let port = target.port.to_string();
    let output = Command::new("mysql")
        .args([
            &format!("--defaults-extra-file={defaults_arg}"),
            "--protocol=TCP",
            "-h",
            &target.host,
            "-P",
            &port,
            "-u",
            &target.user,
            "--ssl-mode=DISABLED",
            "--batch",
            "--skip-column-names",
            &target.database,
        ])
        .stdin(Stdio::from(input))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let _ = fs::remove_file(&defaults);
    let output = output.map_err(|error| format!("could not start mysql: {error}"))?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|_| "mysql output is not utf-8".to_string());
    }
    Err(format!(
        "could not update fsassets: {}",
        redact(&tail_text(&output.stderr), &target.password)
    ))
}

#[cfg(target_os = "linux")]
fn query_postgres(target: &DbTarget, sql_path: &Path) -> Result<String, String> {
    let port = target.port.to_string();
    let sql_arg = sql_path
        .to_str()
        .ok_or("postgres statement path is not utf-8")?;
    let output = Command::new("psql")
        .env("PGPASSWORD", &target.password)
        .args([
            "-h",
            &target.host,
            "-p",
            &port,
            "-U",
            &target.user,
            "-d",
            &target.database,
            "-v",
            "ON_ERROR_STOP=1",
            "-q",
            "-t",
            "-A",
            "-f",
            sql_arg,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("could not start psql: {error}"))?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|_| "psql output is not utf-8".to_string());
    }
    Err(format!(
        "could not update fsassets: {}",
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

    fn sample() -> SeedAsset {
        SeedAsset {
            id: "2a4880b6-b7a3-690a-2049-bfbe38eafb9f".to_string(),
            name_hex: "6e".to_string(),
            description_hex: String::new(),
            type_code: 0,
            flags: 1,
            hash_hex: "ab".repeat(32),
            data: vec![0, 255],
        }
    }

    fn target() -> DbTarget {
        DbTarget {
            user: "user".to_string(),
            password: "secret".to_string(),
            host: "127.0.0.1".to_string(),
            port: 1,
            database: "assets".to_string(),
        }
    }

    #[test]
    fn fsasset_success_line_names_only_the_ids_this_run_shows() {
        let mysql = success_report("csharp", "ferroan", 12, 10, 12, &difference_ids(true));
        assert!(mysql.contains("assets: match (12 http records)"));
        assert!(mysql.contains("cross-read ok"));
        assert!(mysql.contains("10000-asset read: csharp 10 ms, ferroan 12 ms"));
        assert!(mysql.contains("known differences: TB45"));
        assert!(!mysql.contains("TB39"));
        assert!(!mysql.contains("TB40"));
        let clean = success_report("csharp", "ferroan", 1, 1, 1, &difference_ids(false));
        assert!(!clean.contains("known differences"));
    }

    #[test]
    fn fsasset_probes_fail_a_miss_or_a_page_that_differs() {
        assert!(classify_missing(404, 404).is_ok());
        assert!(classify_missing(500, 404).is_err());
        assert!(classify_force(true, true).expect("shown"));
        assert!(!classify_force(false, true).expect("pin overwrites"));
        assert!(classify_force(true, false).is_err());
        assert!(pages_match(false, "a\nb\n", "b\na\n").is_ok());
        assert!(pages_match(false, "a\n", "a\nb\n").is_err());
        assert!(pages_match(true, "a\nb\n", "a\nb\n").is_ok());
        assert!(pages_match(true, "a\nb\n", "b\na\n").is_err());
    }

    #[test]
    fn fsasset_import_sql_matches_the_pin_page() {
        assert_eq!(pin_page_sql(true), ferroan_page_sql(true));
        assert!(pin_page_sql(true).contains("ORDER BY id LIMIT 2 OFFSET 0"));
        assert!(!pin_page_sql(true).contains("LIMIT 0 OFFSET 2"));
        assert!(pin_page_sql(false).contains("LIMIT 0, 2"));
        assert!(!pin_page_sql(false).contains("ORDER BY"));
        assert!(ferroan_page_sql(false).contains("ORDER BY id LIMIT 0, 2"));
        let force = force_sql(true, FORCE_ID, &"ab".repeat(32), 5);
        assert!(force.contains("SET \"hash\""));
        assert!(force.contains("access_time"));
        assert!(!force.contains("name"));
        assert_eq!(row_count("one\n\ntwo\n"), 2);
    }

    #[test]
    fn fsasset_seed_touches_only_fsassets_and_uses_the_hash_path() {
        let mysql = library_sql(FsassetDb::Mysql, &[sample()], 9).expect("mysql");
        assert!(mysql.contains("DELETE FROM `fsassets`"));
        assert!(mysql.contains("CONVERT(UNHEX('6e') USING utf8mb3)"));
        assert!(!mysql.contains("DELETE FROM `assets`"));
        assert!(!mysql.contains("XAssets"));
        let postgres = library_sql(FsassetDb::Pgsql, &[sample()], 9).expect("postgres");
        assert!(postgres.contains("DELETE FROM fsassets"));
        assert!(postgres.contains("'2a4880b6-b7a3-690a-2049-bfbe38eafb9f'::uuid"));
        assert!(!postgres.contains("name"));
        let hash = format!("0123456789{EMPTY_HASH}");
        let short = &hash[..20];
        assert_eq!(
            hash_to_file(short),
            PathBuf::from("01/23/45/6789").join(short)
        );
        assert_eq!(
            gzip_file(&hash_to_file(short)),
            PathBuf::from(format!("01/23/45/6789/{short}.gz"))
        );
        assert_eq!(hash_to_file("short"), PathBuf::from("junkyard/short"));
        assert_eq!(sha256_bytes(&[]), EMPTY_HASH);
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert!(is_uuid(&bulk_id(9_999)));
        assert_ne!(bulk_id(0), FORCE_ID);
    }

    #[test]
    fn fsasset_ado_omits_pooling_for_ferroan_and_the_response_parser_stops_at_the_body() {
        let csharp = ado(FsassetDb::Mysql, &target(), true);
        let ferroan = ado(FsassetDb::Pgsql, &target(), false);
        assert!(csharp.contains("Pooling=false"));
        assert!(csharp.contains("SslMode=None"));
        assert!(!ferroan.contains("Pooling"));
        assert!(ferroan.contains("SSL Mode=Disable"));
        let ini = service_ini(&IniSpec {
            port: 9,
            provider: FsassetDb::Mysql.provider(),
            ado: "hidden",
            grid: false,
            base: "/tmp/fs-base",
            spool: "/tmp/fs-spool",
            access_days: TIMED_ACCESS_DAYS,
        });
        assert!(ini.contains("FSAssetConnector"));
        assert!(ini.contains("BaseDirectory = \"/tmp/fs-base\""));
        assert!(ini.contains("DaysBetweenAccessTimeUpdates = 365"));
        assert!(!ini.contains("ServiceList"));
        let grid = service_ini(&IniSpec {
            port: 9,
            provider: "OpenSim.Data.PGSQL.dll",
            ado: "hidden",
            grid: true,
            base: "/tmp/fs-base",
            spool: "/tmp/fs-spool",
            access_days: 0,
        });
        assert!(grid.contains("AssetServiceConnector"));
        let raw =
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\nhelloMORE";
        let taken = take_response(raw).expect("parse").expect("complete");
        assert_eq!(taken.status, 200);
        assert_eq!(taken.body, b"hello");
        assert!(!taken.close);
        assert_eq!(&raw[taken.consumed..], b"MORE");
        assert!(
            take_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhel")
                .expect("partial")
                .is_none()
        );
        let closed = take_response(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .expect("closed")
        .expect("done");
        assert_eq!(closed.status, 404);
        assert!(closed.close);
        assert!(closed.body.is_empty());
        let request = http_request("GET", "/assets/nope", 9, b"");
        let text = String::from_utf8(request).expect("request");
        assert!(text.starts_with("GET /assets/nope HTTP/1.1\r\n"));
        assert!(!text.contains("Content-Length"));
    }

    #[test]
    fn fsasset_library_json_keeps_a_truncated_mysql_name() {
        let line = r#"{"id":"2a4880b6-b7a3-690a-2049-bfbe38eafb9f","name":"6e","description":null,"type":0,"flags":1,"data":"00ff"}"#;
        let row = seed_asset(line).expect("row");
        assert_eq!(row.hash_hex, sha256_bytes(&[0x00, 0xff]));
        assert_eq!(row.name_hex, "6e");
        assert!(row.description_hex.is_empty());
        let long = format!("\"name\":\"{}\"", hex_encode("é".repeat(70).as_bytes()));
        let text = format!(
            r#"{{"id":"2a4880b6-b7a3-690a-2049-bfbe38eafb9f",{long},"description":"","type":0,"flags":0,"data":""}}"#
        );
        let truncated = seed_asset(&text).expect("truncated");
        let name = String::from_utf8(hex_decode(&truncated.name_hex).expect("hex")).expect("utf8");
        assert_eq!(name.chars().count(), 64);
    }
}
